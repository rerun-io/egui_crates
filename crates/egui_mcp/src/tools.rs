//! The MCP face of [`egui_inspection_tools`]: its tools served through `rmcp`, plus the TCP
//! connection lifecycle of the `egui-mcp` binary.
//!
//! The tools are split across two types so the app-driving half can be reused:
//!
//! - [`UiServer`] serves the UI/inspection tools (`click`, `type_text`, `screenshot`,
//!   `widget_tree`, `batch`, …). It holds a single [`Bridge`] and nothing else — constructing
//!   one *is* the "connected" state. It is transport-agnostic: pair it with the router from
//!   [`UiServer::router`] to list and dispatch the tools, so another MCP server can embed
//!   it next to *its own* connection tools, driving whatever [`Bridge`] it built.
//! - [`Server`] is egui-mcp's own server: it owns the UI tools' [`ToolRouter`] plus an
//!   `Option<UiServer>` that is `Some` only while attached, and adds the TCP connection
//!   lifecycle (`attach` / `disconnect` / `status`). Because the router lives on the server
//!   rather than the (absent) [`UiServer`], the app-driving tools are always listed — MCP
//!   clients that cache the initial tool list discover them before `attach` — while calls made
//!   before `attach` return `no app connected`.
//!
//! The tools themselves live in [`egui_inspection_tools`]. MCP adds one thing to them:
//! `screenshot` takes a `save_path` to also write the PNG to disk, on the machine running the
//! server.
//!
//! Recoverable failures (no app connected, node not found, etc.) become a tool result with
//! `isError: true` — not a JSON-RPC error, per MCP spec.

use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use base64::Engine as _;
use egui_inspection_tools::{BatchArgs, Content, Effect, JsonObject, ToolDef, ToolOutput};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{
        router::tool::{ToolRoute, ToolRouter},
        tool::ToolCallContext,
        wrapper::Parameters,
    },
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        Tool, ToolAnnotations,
    },
    service::{RequestContext, RoleServer},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::bridge::{BoxFuture, Bridge};

pub use egui_inspection_tools::{
    BatchAction, ClickArgs, DragArgs, GetWidgetArgs, GetWidgetResult, HoverArgs, PointerButtonArg,
    Pos2Lit, PressKeyArgs, PressKeyModifiers, ResizeArgs, ScrollArgs, Target, TypeTextArgs,
    WaitForArgs, WidgetTreeResult,
};

// ---------------------------------------------------------------------------------------
// UiServer + Server
// ---------------------------------------------------------------------------------------

/// The reusable core: the egui UI/inspection tools (`click`, `type_text`, `screenshot`,
/// `widget_tree`, `batch`, …) bound to one live [`Bridge`].
///
/// It holds *only* the bridge — so a `UiServer` always has an app to drive, and its mere
/// existence is the "connected" state (a host represents "disconnected" as the absence of a
/// `UiServer`, e.g. [`Server`]'s `Option<UiServer>`).
///
/// It owns no connection logic and no router. A host builds the bridge however it likes (TCP,
/// a tunnelled transport, …), wraps it with [`UiServer::new`], and pairs it with the router
/// from [`UiServer::router`]: `router.list_all()` lists the commands for the host's
/// `list_tools`, and [`UiServer::dispatch`] runs one — so another MCP server can offer its own
/// connection tools alongside this shared command set.
pub struct UiServer {
    bridge: Bridge,
}

impl UiServer {
    /// Wrap a live [`Bridge`] as a UI server.
    pub fn new(bridge: Bridge) -> Self {
        Self { bridge }
    }

    /// Build the router over the UI/inspection tools.
    ///
    /// The router is independent of any `UiServer` instance — list its tools while
    /// disconnected, then dispatch against a `UiServer` once one exists (this is exactly what
    /// [`Server`] does, and what an embedding server should do too).
    pub fn router() -> ToolRouter<Self> {
        let mut router = ToolRouter::new();
        for def in ui_tools() {
            router.add_route(ToolRoute::new_dyn(mcp_tool(def), route_call));
        }
        router
    }

    /// Run one UI tool against this server's bridge, via a router from [`UiServer::router`].
    ///
    /// For an embedding server's `call_tool` to delegate to.
    ///
    /// # Errors
    /// If the tool name is unknown (recoverable failures are reported in the `CallToolResult`).
    pub async fn dispatch(
        &self,
        router: &ToolRouter<Self>,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tcc = ToolCallContext::new(self, request, context);
        Ok(complete(router.call(tcc).await?))
    }

    /// The bridge this server drives.
    fn bridge(&self) -> &Bridge {
        &self.bridge
    }

    /// Run one UI tool by name. A `batch` step comes back through here too, so it reaches the
    /// MCP-only `save_path` of `screenshot`.
    async fn call(&self, name: &str, arguments: JsonObject) -> ToolOutput {
        if name != "batch" {
            return self.call_one(name, arguments).await;
        }
        let args: BatchArgs = match ui_tool("batch").parse_arguments(arguments) {
            Ok(args) => args,
            Err(err) => return ToolOutput::error(err),
        };
        egui_inspection_tools::run_batch(args, |name, arguments| {
            Box::pin(async move { self.call_one(&name, arguments).await })
        })
        .await
    }

    /// [`Self::call`] for any tool but `batch`.
    async fn call_one(&self, name: &str, arguments: JsonObject) -> ToolOutput {
        if name == "screenshot" {
            return self
                .screenshot(arguments)
                .await
                .unwrap_or_else(ToolOutput::error);
        }
        egui_inspection_tools::call(self.bridge.transport(), &tokio_sleep, name, arguments).await
    }

    /// `screenshot`, plus writing the PNG to `save_path`.
    async fn screenshot(&self, arguments: JsonObject) -> Result<ToolOutput, String> {
        let args: ScreenshotArgs = ui_tool("screenshot").parse_arguments(arguments)?;
        let png = self.bridge.screenshot(args.pixels_per_point).await?;
        let saved = match &args.save_path {
            Some(path) => {
                std::fs::write(path, &png.bytes)
                    .map_err(|err| format!("save to `{path}`: {err}"))?;
                Some(path.as_str())
            }
            None => None,
        };
        let meta = json!({ "width": png.size[0], "height": png.size[1], "saved_to": saved });
        Ok(ToolOutput::success(vec![
            Content::Text(meta.to_string()),
            Content::Image {
                bytes: png.bytes,
                mime_type: "image/png".to_owned(),
            },
        ]))
    }
}

/// The [`ToolRoute`] handler of every UI tool.
fn route_call(
    context: ToolCallContext<'_, UiServer>,
) -> Pin<Box<dyn Future<Output = Result<CallToolResponse, McpError>> + Send + '_>> {
    let ToolCallContext {
        service,
        name,
        arguments,
        ..
    } = context;
    Box::pin(async move {
        let output = service.call(&name, arguments.unwrap_or_default()).await;
        Ok(call_tool_result(output).into())
    })
}

/// The timer `wait_for` polls with.
fn tokio_sleep(duration: Duration) -> BoxFuture<'static, ()> {
    Box::pin(tokio::time::sleep(duration))
}

/// egui-mcp's own MCP server: the TCP connection lifecycle plus the UI tools.
///
/// It owns the UI tools' [`ToolRouter`] directly and the attached [`UiServer`] (if any) behind
/// a lock. The router is independent of the connection, so the app-driving tools stay
/// discoverable while disconnected; calling one before `attach` returns a recoverable
/// `no app connected` error.
#[derive(Clone)]
pub struct Server {
    /// The attached UI server, `Some` only while connected. Behind a shared lock so the
    /// `&self` handler methods (and clones rmcp may make) can attach/detach it.
    ui: Arc<Mutex<Option<UiServer>>>,
    ui_router: ToolRouter<UiServer>,
    lifecycle_router: ToolRouter<Self>,
}

impl Server {
    pub fn new() -> Self {
        Self {
            ui: Arc::new(Mutex::new(None)),
            ui_router: UiServer::router(),
            lifecycle_router: Self::lifecycle_router(),
        }
    }

    fn tools(&self) -> Vec<Tool> {
        let mut tools = self.lifecycle_router.list_all();
        tools.extend(self.ui_router.list_all());
        tools
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------
// The UI tools, as MCP sees them
// ---------------------------------------------------------------------------------------

/// The `screenshot` description, with the MCP-only `save_path`.
const SCREENSHOT_DESCRIPTION: &str = "Capture the current frame as a PNG screenshot.
Defaults to logical-point resolution (`pixels_per_point: 1.0`) so pixels align with `click`/`widget_tree` coordinates; pass a higher `pixels_per_point` for detail, or `save_path` to also write it to disk.
Requires the app window to be visible — a fully-occluded or minimized window can't render a frame to capture (notably on macOS), so the call times out; bring the window to the foreground first.";

/// The `batch` description, which also says the connection tools can't be batched.
const BATCH_DESCRIPTION: &str = "Execute a sequence of app-driving tool calls in one round trip (the connection tools `attach`/`disconnect`/`status` are not available here).
Stops on the first error.
Results are emitted in execution order, interleaved: each step contributes one JSON text item followed by any image items it produced (e.g. screenshots).
`batch` cannot be nested.
Use this to act and observe in one call, e.g. a `click` then a `widget_tree` or `screenshot`.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    /// Output resolution in pixels per logical point.
    /// Defaults to `1.0`, which makes screenshot pixels line up 1:1 with the logical coordinates used by `click`/`widget_tree`.
    /// Higher values give a sharper image, capped at the display's native scale (no upscaling).
    #[serde(default = "default_pixels_per_point")]
    pub pixels_per_point: f32,

    /// If set, also write the PNG to this path on the machine running the MCP server (in addition to returning it inline).
    #[serde(default)]
    pub save_path: Option<String>,
}

fn default_pixels_per_point() -> f32 {
    1.0
}

/// The UI tools of [`egui_inspection_tools`], with `screenshot` and `batch` adapted to MCP.
fn ui_tools() -> &'static [ToolDef] {
    static TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| {
        egui_inspection_tools::tools()
            .iter()
            .map(|def| match def.name.as_str() {
                "screenshot" => ToolDef {
                    description: SCREENSHOT_DESCRIPTION.to_owned(),
                    input_schema: egui_inspection_tools::input_schema::<ScreenshotArgs>(),
                    // It writes a file when given `save_path`.
                    effect: Effect::Idempotent,
                    ..def.clone()
                },
                "batch" => ToolDef {
                    description: BATCH_DESCRIPTION.to_owned(),
                    ..def.clone()
                },
                _ => def.clone(),
            })
            .collect()
    });
    &TOOLS
}

/// One of [`ui_tools`], by a name known to exist.
fn ui_tool(name: &str) -> &'static ToolDef {
    #[expect(clippy::unwrap_used)] // Only called with the names of tools that exist.
    ui_tools().iter().find(|def| def.name == name).unwrap()
}

/// The MCP description of a tool.
fn mcp_tool(def: &ToolDef) -> Tool {
    let ToolDef {
        name,
        title,
        description,
        input_schema,
        output_schema,
        effect,
    } = def;
    let annotations = if effect.is_read_only() {
        ToolAnnotations::new().read_only(true).open_world(false)
    } else {
        ToolAnnotations::new()
            .read_only(false)
            .destructive(*effect == Effect::Destructive)
            .idempotent(*effect == Effect::Idempotent)
            .open_world(false)
    };
    let mut tool = Tool::new(
        name.clone(),
        description.clone(),
        Arc::new(input_schema.clone()),
    )
    .with_title(*title)
    .with_annotations(annotations);
    if let Some(output_schema) = output_schema {
        tool = tool.with_raw_output_schema(Arc::new(output_schema.clone()));
    }
    tool
}

fn call_tool_result(output: ToolOutput) -> CallToolResult {
    let ToolOutput {
        content,
        structured,
        is_error,
    } = output;
    let content = content
        .into_iter()
        .map(|item| match item {
            Content::Text(text) => ContentBlock::text(text),
            Content::Image { bytes, mime_type } => ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                mime_type,
            ),
        })
        .collect();
    let mut result = if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = structured;
    result
}

// ---------------------------------------------------------------------------------------
// General helpers / utils
// ---------------------------------------------------------------------------------------

fn text_error(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

/// Collapses a router response to its completed result.
/// None of the UI tools use multi-round-trip input or tasks, so any other variant is a bug.
fn complete(response: CallToolResponse) -> CallToolResult {
    match response {
        CallToolResponse::Complete(result) => result,
        other => text_error(format!("unexpected tool response: {other:?}")),
    }
}

// ---------------------------------------------------------------------------------------
// Lifecycle tools
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachArgs {
    /// Host the app's inspection port is on.
    /// Defaults to `127.0.0.1`.
    #[serde(default = "default_host")]
    pub host: String,

    /// Port the app is listening on.
    /// Defaults to `5719` (`egui_inspection`'s default).
    #[serde(default = "default_port")]
    pub port: u16,

    /// How long to keep retrying the connection, in seconds.
    /// Defaults to 10.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn default_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_port() -> u16 {
    5719
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyArgs {}

/// Lifecycle tools — connection management, served directly by [`Server`].
#[tool_router(router = lifecycle_router)]
impl Server {
    /// Connect to a running egui app's inspection port (an app built with eframe's `inspection` feature, launched with `EGUI_INSPECTION` set).
    /// Defaults to 127.0.0.1:5719.
    /// Retries until `timeout_secs` elapses.
    /// On success the app-driving tools start working (they are always listed, but error until an app is attached).
    #[tool(
        title = "Attach to app",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn attach(
        &self,
        Parameters(args): Parameters<AttachArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut guard = self.ui.lock().await;
        if guard.is_some() {
            return Ok(text_error(
                "already connected — call `disconnect` first before attaching again",
            ));
        }
        let timeout = args.timeout_secs.map(Duration::from_secs);
        let bridge = match Bridge::connect(&args.host, args.port, timeout).await {
            Ok(b) => b,
            Err(e) => return Ok(text_error(format!("attach failed: {e}"))),
        };
        let info = bridge.peer_info.clone();
        *guard = Some(UiServer::new(bridge));
        Ok(CallToolResult::structured(
            json!({ "ok": true, "attached": info }),
        ))
    }

    /// Disconnect from the attached app.
    /// App-driving tools remain available but return an error until `attach` is called again.
    #[tool(
        title = "Disconnect",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn disconnect(&self, _p: Parameters<EmptyArgs>) -> Result<CallToolResult, McpError> {
        if self.ui.lock().await.take().is_some() {
            Ok(CallToolResult::structured(json!({ "ok": true })))
        } else {
            Ok(text_error("no app connected"))
        }
    }

    /// Report whether an app is connected and its peer info.
    #[tool(
        title = "Connection status",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn status(&self, _p: Parameters<EmptyArgs>) -> Result<CallToolResult, McpError> {
        let guard = self.ui.lock().await;
        let body = match guard.as_ref() {
            None => json!({ "state": "idle" }),
            Some(ui) => json!({ "state": "connected", "peer": ui.bridge().peer_info }),
        };
        Ok(CallToolResult::structured(body))
    }
}

/// Operating guidance sent to clients at initialize (the MCP `instructions` field). The
/// per-tool descriptions cover each command in isolation; this establishes the cross-cutting
/// workflow — the observe→act→verify loop and the prefer-locators convention — that an agent
/// otherwise has to infer.
const INSTRUCTIONS: &str = r#"This mcp drives a live egui app: it reads the app's accessibility tree and synthesizes real input events. Work in an observe → act → verify loop.

Getting oriented:
- Call `attach` first (check `status` if unsure); the app-driving tools return "no app connected" until then.
- Start most tasks with `widget_tree` to discover widgets and their ids, and/or `screenshot` to see the rendered frame.
- When a widget reports `omitted_children`, query `widget_tree` again with that widget's `id` as `root` to walk just that part of the app.
- `widget_tree` returns an abridged tree: no `bounds`, and text over 100 characters cut short. A trailing ` […]` in a `label` or `value` means exactly that — the text goes on, and `get_widget` on that widget's `id` returns all of it, along with its `bounds` in logical points and its parent. Filters always match the full text, so searching for a phrase from past the cut still finds the widget.

Targeting widgets:
- Prefer locators — an `id` from `widget_tree`, a `role`, or a text match — over a raw `pos`. Locators resolve to the widget's current position and survive layout changes; reach for `pos` only when nothing matches.
- For text matches prefer `content_contains`: it matches a widget's `label` (accessible name) OR its `value`. This matters because many widgets (`Label`, monospace text, counters) carry their text in `value` with an empty `label`, so `label_contains` alone silently misses them. Use `label_contains`/`value_contains` only when you specifically need to match one field.
- A locator in an action must match exactly one widget. If it matches several, the call errors and lists the candidates — narrow the filter (add a `role`, or switch to a more specific text match) or target a specific `id`. Use `widget_tree` when you want every match.

Acting and verifying:
- After an action that changes the UI, confirm it landed: `widget_tree` for the expected state, `screenshot` to look, or `wait_for` to poll until async or animated UI settles.
- Use `batch` to act and observe in one round trip (e.g. `click` then `screenshot`), avoiding an extra turn.

Conventions:
- Everything is in logical points, one shared coordinate frame: raw `pos`, `resize` dimensions, the `bounds` from `get_widget`, and a default (`pixels_per_point: 1.0`) `screenshot`. So a widget's `bounds` center is exactly where to `click`, and a pixel in the screenshot is a logical point. There is no fixed screen size; use `resize` to set the viewport."#;

/// How long a client may keep the tool list without asking again.
const TOOL_LIST_TTL: Duration = Duration::from_mins(5);

/// The `tools/list` result, with the cache hints MCP 2026-07-28 (SEP-2549) requires.
///
/// Both `ttlMs` and `cacheScope` are mandatory from that protocol version on: a client that
/// negotiated it rejects the whole list when either is missing, leaving the server with no tools.
/// The list is fixed for the life of the process and holds nothing user-specific, so it may be
/// cached and shared freely.
fn tool_list(tools: Vec<Tool>) -> ListToolsResult {
    ListToolsResult::with_all_items(tools)
        .with_ttl_ms(u64::try_from(TOOL_LIST_TTL.as_millis()).unwrap_or(u64::MAX))
        .with_cache_scope(CacheScope::Public)
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("egui-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(tool_list(self.tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        // Lifecycle tools run on `self`; everything else is delegated to the attached UI server.
        if self.lifecycle_router.has_route(&request.name) {
            if let Some(tool) = self.lifecycle_router.get(&request.name)
                && let Some(arguments) = request.arguments.as_ref()
                && let Err(err) =
                    egui_inspection_tools::check_arguments(&tool.input_schema, arguments)
            {
                return Ok(text_error(err).into());
            }
            let tcc = ToolCallContext::new(self, request, context);
            return self.lifecycle_router.call(tcc).await;
        }
        let guard = self.ui.lock().await;
        let Some(ui) = guard.as_ref() else {
            return Ok(text_error("no app connected — call `attach` first").into());
        };
        ui.dispatch(&self.ui_router, request, context)
            .await
            .map(Into::into)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if let Some(tool) = self.lifecycle_router.get(name) {
            return Some(tool.clone());
        }
        self.ui_router.get(name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use egui_inspection::protocol::{EncodedPng, Request, Response};

    use super::*;
    use crate::bridge::{PeerInfo, Transport};

    /// An app that answers every screenshot with the same tiny image.
    struct ScreenshotApp;

    impl Transport for ScreenshotApp {
        fn request(&self, _req: Request) -> BoxFuture<'_, Result<Response, String>> {
            Box::pin(std::future::ready(Ok(Response::Screenshot(EncodedPng {
                size: [4, 3],
                bytes: b"png".to_vec(),
            }))))
        }
    }

    /// `save_path` is MCP's own, so a batch must reach MCP's `screenshot`, not the core one.
    #[tokio::test]
    async fn a_batched_screenshot_still_saves_to_disk() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("screenshot.png");
        let path_str = path.to_str().expect("utf8 path");
        let peer_info = PeerInfo {
            transport: "test".to_owned(),
            protocol_version: 0,
            label: None,
        };
        let ui = UiServer::new(Bridge::with_transport(ScreenshotApp, peer_info));
        let args =
            json!({ "actions": [{ "name": "screenshot", "args": { "save_path": path_str } }] });
        let output = ui
            .call("batch", args.as_object().expect("object").clone())
            .await;
        assert!(!output.is_error, "{output:?}");
        assert_eq!(std::fs::read(&path).expect("saved"), b"png");
    }

    /// Snapshot the entire agent-facing surface: the server `instructions` plus every tool's
    /// name, description, and input/output schemas — exactly what an MCP client is shown on
    /// connect. Guards descriptions, schemas, and structured-output shapes against accidental
    /// change. Run `INSTA_UPDATE=always cargo nextest run -p egui_mcp` to accept intended edits.
    ///
    /// The server version is intentionally excluded so the snapshot is stable across releases.
    #[test]
    fn tool_list_carries_both_cache_hints() {
        let result = serde_json::to_value(tool_list(Server::new().tools())).unwrap();
        assert!(
            result["ttlMs"].is_u64(),
            "ttlMs missing: {}",
            result["ttlMs"]
        );
        assert_eq!(result["cacheScope"], "public");
    }

    #[test]
    fn agent_surface_snapshot() {
        let server = Server::new();

        let mut surface = String::new();
        surface.push_str("# Server instructions\n\n");
        surface.push_str(
            server
                .get_info()
                .instructions
                .as_deref()
                .unwrap_or("(none)"),
        );
        surface.push_str("\n\n# Tools\n");

        let mut tools = server.tools();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        for tool in &tools {
            write!(surface, "\n## {}\n\n", tool.name).unwrap();
            surface.push_str(&serde_json::to_string_pretty(tool).expect("serialize tool"));
            surface.push('\n');
        }

        insta::assert_snapshot!("agent_surface", surface);
    }
}
