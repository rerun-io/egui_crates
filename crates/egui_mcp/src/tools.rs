//! MCP tool definitions + dispatch, built on the official `rmcp` SDK.
//!
//! Each tool is an async method annotated with `#[tool]`. The macro derives the input schema
//! from the typed parameter struct (via `schemars::JsonSchema`) and wires the method into a
//! [`ToolRouter`].
//!
//! The tools are split across two types so the app-driving half can be reused:
//!
//! - [`UiServer`] owns the core UI/inspection commands (`click`, `type_text`, `screenshot`,
//!   `widget_tree`, `batch`, …). It holds a single [`Bridge`] and nothing else — constructing
//!   one *is* the "connected" state. It is transport-agnostic: pair it with the router from
//!   [`UiServer::router`] to list and dispatch the commands, so another MCP server can embed
//!   it next to *its own* connection tools, driving whatever [`Bridge`] it built.
//! - [`Server`] is egui-mcp's own server: it owns the UI tools' [`ToolRouter`] plus an
//!   `Option<UiServer>` that is `Some` only while attached, and adds the TCP connection
//!   lifecycle (`attach` / `disconnect` / `status`). Because the router lives on the server
//!   rather than the (absent) [`UiServer`], the app-driving tools are always listed — MCP
//!   clients that cache the initial tool list discover them before `attach` — while calls made
//!   before `attach` return `no app connected`.
//!
//! Locator resolution is done here, MCP-side: a tool fetches a fresh `AccessKit` tree
//! (`GetTree`), resolves the locator to a logical-point position, synthesizes the matching
//! `egui::Event`s, and sends them via `ApplyEvents`. The app-side plugin stays low-level.
//!
//! Recoverable failures (no app connected, node not found, etc.) are returned as a plain
//! `ToolError`, which `rmcp` renders into a tool result with `isError: true` — not as a
//! JSON-RPC error, per MCP spec.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use egui::Event;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{
        router::tool::ToolRouter,
        tool::ToolCallContext,
        wrapper::{Json, Parameters},
    },
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        Implementation, JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerConfig, Tool,
    },
    schemars,
    service::{RequestContext, RoleServer},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::bridge::{Bridge, TreeSnapshot};
use crate::tree::{self, Id, Locator, Query, QueryFilter, Widget, WidgetDetail};

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
        Self::tool_router()
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
        if let Some(tool) = router.get(&request.name)
            && let Err(err) = check_arguments(&tool.input_schema, request.arguments.as_ref())
        {
            return Ok(text_error(err));
        }
        let tcc = ToolCallContext::new(self, request, context);
        Ok(complete(router.call(tcc).await?))
    }

    /// The bridge this server drives.
    fn bridge(&self) -> &Bridge {
        &self.bridge
    }
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
// General helpers / utils
// ---------------------------------------------------------------------------------------

fn text_error(msg: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.into())])
}

fn content_as_text(c: &ContentBlock) -> Option<&str> {
    match c {
        ContentBlock::Text(t) => Some(t.text.as_str()),
        _ => None,
    }
}

fn content_is_image(c: &ContentBlock) -> bool {
    matches!(c, ContentBlock::Image(_))
}

/// Reject an argument the tool's schema doesn't declare.
///
/// Serde ignores unknown keys, and `#[serde(flatten)]` rules out `deny_unknown_fields`, so
/// without this a misspelled argument is silently ignored.
fn check_arguments(schema: &JsonObject, arguments: Option<&JsonObject>) -> Result<(), String> {
    let Some(arguments) = arguments else {
        return Ok(());
    };
    let empty = JsonObject::new();
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    check_against(schema, arguments, defs, "")
}

/// The body of [`check_arguments`], recursing into nested objects and array items.
///
/// Anything the schema doesn't pin down — a free-form object, a shape we can't resolve — is
/// left alone, so this only ever rejects a key the tool positively doesn't have.
fn check_against(
    schema: &JsonObject,
    arguments: &JsonObject,
    defs: &JsonObject,
    path: &str,
) -> Result<(), String> {
    if schema.get("additionalProperties") == Some(&Value::Bool(true)) {
        return Ok(());
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };

    for (key, value) in arguments {
        let Some(property) = properties.get(key) else {
            let mut known: Vec<&str> = properties.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(format!(
                "unknown argument `{path}{key}` — this takes: {}",
                known.join(", ")
            ));
        };

        let nested_path = format!("{path}{key}.");
        if let Some(nested) = value.as_object()
            && let Some(nested_schema) = object_schema(property, defs)
        {
            check_against(nested_schema, nested, defs, &nested_path)?;
        }
        if let Some(items) = value.as_array()
            && let Some(item_schema) = property
                .as_object()
                .and_then(|p| p.get("items"))
                .and_then(|items| object_schema(items, defs))
        {
            for item in items.iter().filter_map(Value::as_object) {
                check_against(item_schema, item, defs, &nested_path)?;
            }
        }
    }
    Ok(())
}

/// Follow `$ref` and `anyOf` until an object schema with `properties`, if there is one.
fn object_schema<'a>(schema: &'a Value, defs: &'a JsonObject) -> Option<&'a JsonObject> {
    let schema = schema.as_object()?;
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.strip_prefix("#/$defs/")?;
        return object_schema(defs.get(name)?, defs);
    }
    if schema.contains_key("properties") {
        return Some(schema);
    }
    schema
        .get("anyOf")?
        .as_array()?
        .iter()
        .find_map(|variant| object_schema(variant, defs))
}

/// Collapses a router response to its completed result.
/// None of the UI tools use multi-round-trip input or tasks, so any other variant is a bug.
fn complete(response: CallToolResponse) -> CallToolResult {
    match response {
        CallToolResponse::Complete(result) => result,
        other => text_error(format!("unexpected tool response: {other:?}")),
    }
}

/// A recoverable tool failure (no app connected, widget not found, bad argument, a bridge I/O
/// error, …), carried as a plain message string.
///
/// It is *not* a JSON-RPC protocol error: a `String` already implements `rmcp`'s `IntoContents`,
/// so when a `#[tool]` method returns `Err(ToolError)`, `rmcp` renders it into a `CallToolResult`
/// with `isError: true` (per the MCP spec). `String` is also the [`Bridge`]'s error type, so the
/// inner handlers `?`-propagate bridge failures with no conversion.
type ToolError = String;

/// The result of an app-driving tool handler — see [`ToolError`].
type ToolResult<T> = Result<T, ToolError>;

// ---------------------------------------------------------------------------------------
// Target — locator OR raw position, shared by click / hover / scroll / drag.
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Target {
    /// Widget id from `widget_tree`.
    #[serde(default)]
    pub id: Option<Id>,

    /// Widget-matching constraints (`role` and/or a text predicate); see `widget_tree`.
    #[serde(flatten)]
    pub query: Query,

    /// Raw position in logical points (use instead of locator fields).
    #[serde(default)]
    pub pos: Option<Pos2Lit>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub struct Pos2Lit {
    pub x: f32,
    pub y: f32,
}

impl Target {
    fn as_locator(&self) -> Option<Locator> {
        Locator::from_fields(self.id, self.query.clone())
    }
}

/// Resolve a [`Target`] to an optional widget id + a logical-point position, fetching a fresh
/// tree first. Use [`resolve_in_tree`] directly when resolving several targets against one
/// snapshot (e.g. a drag's start and end).
async fn resolve_target(bridge: &Bridge, target: &Target) -> ToolResult<(Option<Id>, egui::Pos2)> {
    // A raw `pos` target needs no tree.
    if let Some(p) = target.pos {
        return Ok((None, egui::Pos2::new(p.x, p.y)));
    }
    let snap = bridge.fetch_tree().await?;
    resolve_in_tree(&snap, target)
}

/// Resolve a [`Target`] against an already-fetched tree snapshot.
fn resolve_in_tree(snap: &TreeSnapshot, target: &Target) -> ToolResult<(Option<Id>, egui::Pos2)> {
    if let Some(p) = target.pos {
        return Ok((None, egui::Pos2::new(p.x, p.y)));
    }
    if let Some(role) = &target.query.role {
        tree::validate_role(role, snap.tree.as_ref())?;
    }
    let locator = target.as_locator().ok_or(
        "target requires `id`, `content_contains`, `role`, `label_contains`, `value_contains`, or `pos`",
    )?;
    let tree = snap.tree.as_ref().ok_or("no accesskit tree yet")?;
    let node = tree::resolve_unique(tree, &locator, snap.pixels_per_point)?;
    let view = tree::widget_detail(&node, snap.pixels_per_point);
    let bounds = view.bounds.ok_or("node has no bounds — can't target")?;
    // `widget_detail` already returns logical-point bounds, so the center needs no further scaling.
    let (cx, cy) = bounds.center();
    // AccessKit measures in f64, egui in f32; a screen coordinate fits either way.
    #[expect(clippy::cast_possible_truncation)]
    let center = egui::Pos2::new(cx as f32, cy as f32);
    Ok((Some(view.id), center))
}

// ---------------------------------------------------------------------------------------
// Args structs
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyArgs {}

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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWidgetArgs {
    /// Widget id, as returned by `widget_tree`.
    pub id: Id,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PressKeyModifiers {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub mac_cmd: bool,

    /// = Cmd on Mac / Ctrl on Win+Linux.
    #[serde(default)]
    pub command: bool,
}

impl PressKeyModifiers {
    fn to_egui(&self) -> egui::Modifiers {
        egui::Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: self.mac_cmd,
            command: self.command,
        }
    }
}

/// Hold `modifiers` down for the duration of `action`, then release them.
async fn with_modifiers<T>(
    bridge: &Bridge,
    modifiers: egui::Modifiers,
    action: impl AsyncFnOnce() -> ToolResult<T>,
) -> ToolResult<T> {
    if modifiers == egui::Modifiers::NONE {
        return action().await;
    }
    bridge
        .apply_events(vec![Event::ModifiersChanged(modifiers)])
        .await?;
    let result = action().await;
    // Release even when the action failed, so the next call doesn't inherit a stuck modifier.
    let released = bridge
        .apply_events(vec![Event::ModifiersChanged(egui::Modifiers::NONE)])
        .await;
    let value = result?;
    released?;
    Ok(value)
}

/// Which mouse button a `click`/`drag` uses. `left`/`right` are accepted as aliases for
/// `primary`/`secondary`.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PointerButtonArg {
    #[default]
    #[serde(alias = "left")]
    Primary,
    #[serde(alias = "right")]
    Secondary,
    Middle,
    Extra1,
    Extra2,
}

impl PointerButtonArg {
    fn to_egui(self) -> egui::PointerButton {
        match self {
            Self::Primary => egui::PointerButton::Primary,
            Self::Secondary => egui::PointerButton::Secondary,
            Self::Middle => egui::PointerButton::Middle,
            Self::Extra1 => egui::PointerButton::Extra1,
            Self::Extra2 => egui::PointerButton::Extra2,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickArgs {
    #[serde(flatten)]
    pub target: Target,

    /// `primary`/`secondary`/`middle`/`extra1`/`extra2` (or aliases `left`/`right`). Defaults to `primary`.
    #[serde(default)]
    pub button: PointerButtonArg,

    /// `2` → double-click; `3` → triple-click.
    #[serde(default = "default_one")]
    pub count: u32,
    #[serde(default)]
    pub modifiers: PressKeyModifiers,
}

fn default_one() -> u32 {
    1
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HoverArgs {
    #[serde(flatten)]
    pub target: Target,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScrollArgs {
    #[serde(flatten)]
    pub target: Target,

    /// Logical points.
    /// Positive Y scrolls down (reveals content below); positive X scrolls right.
    pub delta: Pos2Lit,
    #[serde(default)]
    pub modifiers: PressKeyModifiers,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DragArgs {
    pub start: Target,
    pub end: Target,
    #[serde(default = "default_drag_steps")]
    pub steps: u32,
    #[serde(default)]
    pub modifiers: PressKeyModifiers,
}

fn default_drag_steps() -> u32 {
    8
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResizeArgs {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitForArgs {
    /// Which widgets to wait for (`content_contains` and/or `role`/`label_contains`/`value_contains`);
    /// see `widget_tree`. Omit every constraint to wait on `min_steps` alone.
    #[serde(flatten)]
    pub query: Query,
    #[serde(default = "default_wait_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_min_matches")]
    pub min_matches: u32,

    /// Also wait until at least this many frames have rendered since the call began.
    /// Use it to let animations, tooltips, or other time/frame-driven UI settle (e.g. after a `hover`); `0` means don't wait for frames.
    #[serde(default)]
    pub min_steps: u64,
}

fn default_wait_timeout() -> u64 {
    5
}

fn default_min_matches() -> u32 {
    1
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeTextArgs {
    pub text: String,

    /// Optional focus target: widget id from `widget_tree` to focus before typing.
    /// Omit all locator fields to type into whatever is currently focused.
    #[serde(default)]
    pub id: Option<Id>,

    /// Optional focus target by match (`content_contains` and/or `role`/`label_contains`/`value_contains`);
    /// see `widget_tree`. Omit all locator fields to type into whatever is currently focused.
    #[serde(flatten)]
    pub query: Query,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PressKeyArgs {
    pub key: String,
    #[serde(default)]
    pub modifiers: PressKeyModifiers,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchArgs {
    pub actions: Vec<BatchAction>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchAction {
    pub name: String,
    #[serde(default)]
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub args: Value,
}

// ---------------------------------------------------------------------------------------
// Structured tool outputs — MCP requires `structuredContent` to be a JSON object, so
// collection/optional results are wrapped in a named field rather than returned bare.
// ---------------------------------------------------------------------------------------

/// `widget_tree` result: the matching widgets, nested by ancestry.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WidgetTreeResult {
    pub nodes: Vec<Widget>,
}

/// `get_widget` result: the resolved widget, or `null` if the id didn't match.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetWidgetResult {
    pub node: Option<WidgetDetail>,
}

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

/// App-driving tools — each needs an attached app.
#[tool_router]
impl UiServer {
    /// Capture the current frame as a PNG screenshot.
    /// Defaults to logical-point resolution (`pixels_per_point: 1.0`) so pixels align with `click`/`widget_tree` coordinates; pass a higher `pixels_per_point` for detail, or `save_path` to also write it to disk.
    /// Requires the app window to be visible — a fully-occluded or minimized window can't render a frame to capture (notably on macOS), so the call times out; bring the window to the foreground first.
    #[tool(
        title = "Screenshot",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn screenshot(
        &self,
        Parameters(args): Parameters<ScreenshotArgs>,
    ) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        let png = bridge.screenshot(args.pixels_per_point).await?;
        let saved = match &args.save_path {
            Some(path) => {
                std::fs::write(path, &png.bytes).map_err(|e| format!("save to `{path}`: {e}"))?;
                Some(path.as_str())
            }
            None => None,
        };
        let png_b64 = base64::engine::general_purpose::STANDARD.encode(&png.bytes);
        let meta = json!({ "width": png.size[0], "height": png.size[1], "saved_to": saved });
        Ok(CallToolResult::success(vec![
            ContentBlock::text(meta.to_string()),
            ContentBlock::image(png_b64, "image/png"),
        ]))
    }

    // The return type is spelled `Result<Json<…>, ToolError>` rather than the `ToolResult` alias
    // on purpose: `#[tool]` derives the output schema by syntactically matching `Json<T>` /
    // `Result<Json<T>, _>`, and the alias would hide it, silently dropping the schema.
    /// Walk the app's widget tree and return the widgets matching the filter, nested by ancestry: each one carries the matches from its own subtree in `children`.
    /// Widgets in between that don't match are skipped, so a `children` entry is a descendant, not necessarily a direct child.
    /// `role`, if given, is a role name (e.g. `Button`, `Label`), matched case-insensitively; an unknown role errors with the roles present in the tree.
    /// `limit` (200 by default) caps how many widgets come back: whole levels are kept from the top down, so the app's structure survives and the leaves go first, and any widget whose children were cut reports how many in `omitted_children`.
    /// `root` starts the walk at one widget instead of the app's root — pass the `id` of a widget that reported `omitted_children` to see that part of the tree in full.
    /// `exclude` leaves a subtree out — pass a widget `id`, or the same constraints the filter takes — and takes everything below it with it; use it to skip a panel whose text would otherwise answer every query.
    /// Use the returned `id` with `click`, `type_text`, or `get_widget`.
    /// The widgets are abridged — call `get_widget` with an `id` for one widget's full detail, including its `bounds` (logical points, its center is where `click` lands), its parent, and its text in full.
    /// `label` and `value` are cut short at 100 characters, marked with a trailing ` […]`; the filters still match against the whole text, so a phrase past the cut still finds its widget.
    /// Empty scaffolding is dropped: a widget with no children, no `label`, no `value`, and no `role` (a `role` of `Unknown` is reported as none) never appears.
    #[tool(
        title = "Widget tree",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn widget_tree(
        &self,
        Parameters(filter): Parameters<QueryFilter>,
    ) -> Result<Json<WidgetTreeResult>, ToolError> {
        let bridge = self.bridge();
        let snap = bridge.fetch_tree().await?;
        let roles = [
            filter.query.role.as_deref(),
            filter
                .exclude
                .as_ref()
                .and_then(|e| e.query.role.as_deref()),
        ];
        for role in roles.into_iter().flatten() {
            tree::validate_role(role, snap.tree.as_ref())?;
        }
        let nodes = match snap.tree {
            Some(tree) => tree::query(&tree, &filter, snap.pixels_per_point)?,
            None => Vec::new(),
        };
        Ok(Json(WidgetTreeResult { nodes }))
    }

    /// Return a single widget by id (from `widget_tree`), in full detail: its `bounds` in logical points, its `parent_id`, its state, and its `label`/`value` untruncated.
    /// This is the follow-up to `widget_tree`, which omits `bounds` and cuts long text short to stay compact.
    // Spelled-out `Result<Json<…>, ToolError>` (not the `ToolResult` alias) so `#[tool]` derives
    // the output schema — see `widget_tree`.
    #[tool(
        title = "Widget details",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_widget(
        &self,
        Parameters(args): Parameters<GetWidgetArgs>,
    ) -> Result<Json<GetWidgetResult>, ToolError> {
        let bridge = self.bridge();
        let locator = Locator::Id { id: args.id };
        let snap = bridge.fetch_tree().await?;
        let ppp = snap.pixels_per_point;
        // `get_widget` is a lookup, not an action: a missing id is `null`, not an error.
        let node = match snap.tree {
            Some(tree) => tree::resolve_unique(&tree, &locator, ppp)
                .ok()
                .map(|n| tree::widget_detail(&n, ppp)),
            None => None,
        };
        Ok(Json(GetWidgetResult { node }))
    }

    /// Click the center of a widget's bounding box, or a raw `pos` in logical points.
    /// Specify either a locator (`id` from `widget_tree`, `role`, or a text match — prefer `content_contains`, which matches `label` or `value`; `label_contains`/`value_contains` match just one field) or `pos: { x, y }`.
    /// `button` defaults to `primary` (accepts `primary`/`secondary`/`middle`/`extra1`/`extra2`, or aliases `left`/`right`).
    /// `count: 2` → double-click, `3` → triple.
    #[tool(
        title = "Click",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn click(&self, Parameters(args): Parameters<ClickArgs>) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        let button = args.button.to_egui();
        let count = args.count.max(1);
        let modifiers = args.modifiers.to_egui();
        let (node_id, center) = resolve_target(bridge, &args.target).await?;

        // Press/release pairs share one frame's input time, so egui's multi-click detection
        // turns `count` into double/triple clicks.
        let mut events = vec![Event::PointerMoved(center)];
        for _ in 0..count {
            events.push(Event::PointerButton {
                pos: center,
                button,
                pressed: true,
                modifiers,
            });
            events.push(Event::PointerButton {
                pos: center,
                button,
                pressed: false,
                modifiers,
            });
        }
        with_modifiers(bridge, modifiers, async || {
            bridge.apply_events(events).await
        })
        .await?;
        Ok(CallToolResult::structured(json!({
            "ok": true,
            "clicked_id": node_id,
            "pos": [center.x, center.y],
            "button": args.button,
            "count": count,
        })))
    }

    /// Move the pointer over a widget (or raw `pos`) without clicking.
    /// Tooltips and hover popups only appear after a short delay — follow with `wait_for` (e.g. its `min_steps`) to let them settle before reading the tree or screenshotting.
    #[tool(
        title = "Hover",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn hover(&self, Parameters(args): Parameters<HoverArgs>) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        let (node_id, pos) = resolve_target(bridge, &args.target).await?;
        bridge.apply_events(vec![Event::PointerMoved(pos)]).await?;
        Ok(CallToolResult::structured(json!({
            "ok": true,
            "hovered_id": node_id,
            "pos": [pos.x, pos.y],
        })))
    }

    /// Send a mouse wheel scroll over a widget (or raw `pos`).
    /// `delta` is in logical points: positive Y scrolls down (reveals content below); positive X scrolls right.
    #[tool(
        title = "Scroll",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn scroll(&self, Parameters(args): Parameters<ScrollArgs>) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        let (node_id, pos) = resolve_target(bridge, &args.target).await?;
        let modifiers = args.modifiers.to_egui();
        let events = vec![
            Event::PointerMoved(pos),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                // Match scroll direction to the convention of playwright and other tools by inverting it
                delta: egui::Vec2::new(-args.delta.x, -args.delta.y),
                phase: egui::TouchPhase::Move,
                modifiers,
                source: egui::MouseWheelSource::Unknown,
            },
        ];
        with_modifiers(bridge, modifiers, async || {
            bridge.apply_events(events).await
        })
        .await?;
        Ok(CallToolResult::structured(json!({
            "ok": true,
            "scrolled_id": node_id,
            "pos": [pos.x, pos.y],
            "delta": [args.delta.x, args.delta.y],
        })))
    }

    /// Primary-button drag from `start` to `end`.
    /// Each target accepts the same fields as `click`: locator (`id`/`content_contains`/`role`/`label_contains`/`value_contains`) or `pos: {x, y}`.
    /// `steps` controls how many intermediate pointer-move events are emitted between press and release.
    #[tool(
        title = "Drag",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn drag(&self, Parameters(args): Parameters<DragArgs>) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        // Resolve both endpoints against one tree snapshot — no input happens between them.
        let snap = bridge.fetch_tree().await?;
        let (start_id, start_pos) = resolve_in_tree(&snap, &args.start)?;
        let (end_id, end_pos) = resolve_in_tree(&snap, &args.end)?;
        let modifiers = args.modifiers.to_egui();
        let steps = args.steps.max(1);

        // One hold spans the whole gesture: the modifier state persists across the frames the
        // press, the waypoints, and the release are spread over.
        with_modifiers(bridge, modifiers, async || {
            bridge
                .apply_events(vec![
                    Event::PointerMoved(start_pos),
                    Event::PointerButton {
                        pos: start_pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers,
                    },
                ])
                .await?;

            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                let waypoint = egui::Pos2::new(
                    start_pos.x + (end_pos.x - start_pos.x) * t,
                    start_pos.y + (end_pos.y - start_pos.y) * t,
                );
                bridge
                    .apply_events(vec![Event::PointerMoved(waypoint)])
                    .await?;
            }

            bridge
                .apply_events(vec![Event::PointerButton {
                    pos: end_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                }])
                .await
        })
        .await?;
        Ok(CallToolResult::structured(json!({
            "ok": true,
            "start_id": start_id,
            "end_id": end_id,
            "start_pos": [start_pos.x, start_pos.y],
            "end_pos": [end_pos.x, end_pos.y],
            "steps": steps,
        })))
    }

    /// Resize the app's viewport to the given logical-point dimensions.
    #[tool(
        title = "Resize viewport",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn resize(&self, Parameters(args): Parameters<ResizeArgs>) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        bridge.resize(args.width, args.height).await?;
        Ok(CallToolResult::structured(
            json!({ "ok": true, "width": args.width, "height": args.height }),
        ))
    }

    /// Poll the widget tree until its conditions hold, or until `timeout_secs` elapses.
    /// Waits until at least `min_matches` visible widgets match the filter (when one is given) *and* at least `min_steps` frames have rendered since the call began.
    /// The text filter is `role` and/or one of `content_contains` (matches `label` or `value` — prefer this; e.g. monospace/`Label` text lives in `value`), `label_contains`, `value_contains`.
    /// Requires a filter (`content_contains`/`role`/`label_contains`/`value_contains`) or a non-zero `min_steps`.
    #[tool(
        title = "Wait for widgets",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn wait_for(
        &self,
        Parameters(args): Parameters<WaitForArgs>,
    ) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        let has_filter = !args.query.is_empty();
        if !has_filter && args.min_steps == 0 {
            return Err(
                "wait_for requires a filter (`content_contains`/`role`/`label_contains`/`value_contains`) or a non-zero `min_steps`".to_owned(),
            );
        }
        let filter = QueryFilter {
            query: args.query.clone(),
            visible_only: true,
            limit: args.min_matches as usize,
            ..Default::default()
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout_secs);
        let mut start_step = None;
        loop {
            let snap = bridge.fetch_tree().await?;
            // Baseline off the first observed frame, so `min_steps` counts frames since the call.
            let steps_waited = snap
                .step
                .saturating_sub(*start_step.get_or_insert(snap.step));
            // Fail fast on a typo'd role rather than polling until timeout, listing the roles
            // currently in the tree. A valid-but-absent role passes and keeps polling.
            if let Some(role) = &filter.query.role {
                tree::validate_role(role, snap.tree.as_ref())?;
            }
            let matches: Vec<Widget> = match (has_filter, snap.tree) {
                (true, Some(tree)) => tree::query(&tree, &filter, snap.pixels_per_point)?,
                _ => Vec::new(),
            };
            let num_matches = tree::count(&matches);
            #[expect(clippy::cast_possible_truncation)]
            // a tree with >4 billion matches is not a thing
            let matched_ok = !has_filter || num_matches as u32 >= args.min_matches;
            if matched_ok && steps_waited >= args.min_steps {
                return Ok(CallToolResult::structured(
                    json!({ "ok": true, "matched": matches, "steps_waited": steps_waited }),
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "wait_for timed out after {}s (content_contains={:?}, role={:?}, label_contains={:?}, value_contains={:?}, found {}, min_steps={}, steps_waited={})",
                    args.timeout_secs,
                    args.query.content_contains,
                    args.query.role,
                    args.query.label_contains,
                    args.query.value_contains,
                    num_matches,
                    args.min_steps,
                    steps_waited,
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Type text into the currently focused widget.
    /// Optionally focus a widget first (by `id`, `role`, or a text match — `content_contains`/`label_contains`/`value_contains`) — this uses an `AccessKit` focus request, not a click, so it won't move the cursor or clear an existing text selection.
    #[tool(
        title = "Type text",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn type_text(
        &self,
        Parameters(args): Parameters<TypeTextArgs>,
    ) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        // Optionally focus a target first. Unlike a click, an AccessKit focus request doesn't
        // move the text cursor or reset the selection.
        let focused_id = match Locator::from_fields(args.id, args.query.clone()) {
            Some(locator) => {
                let snap = bridge.fetch_tree().await?;
                if let Some(role) = &args.query.role {
                    tree::validate_role(role, snap.tree.as_ref())?;
                }
                let tree = snap.tree.as_ref().ok_or("no accesskit tree yet")?;
                let node = tree::resolve_unique(tree, &locator, snap.pixels_per_point)?;
                let id = tree::accesskit_id(&node);
                bridge
                    .apply_events(vec![Event::AccessKitActionRequest(
                        accesskit::ActionRequest {
                            action: accesskit::Action::Focus,
                            target_tree: accesskit::TreeId::ROOT,
                            target_node: accesskit::NodeId(id.0),
                            data: None,
                        },
                    )])
                    .await?;
                Some(id)
            }
            None => None,
        };

        if !args.text.is_empty() {
            bridge.apply_events(vec![Event::Text(args.text)]).await?;
        }

        Ok(CallToolResult::structured(json!({
            "ok": true,
            "focused_id": focused_id,
        })))
    }

    /// Send a key press (down + up) to the focused widget.
    /// `key` is an egui key name such as `Backspace`, `Delete`, `Enter`, `Tab`, `A`–`Z`, `ArrowLeft`, `ArrowRight`, `Home`, `End`, `Escape`.
    #[tool(
        title = "Press key",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn press_key(
        &self,
        Parameters(args): Parameters<PressKeyArgs>,
    ) -> ToolResult<CallToolResult> {
        let bridge = self.bridge();
        let key =
            egui::Key::from_name(&args.key).ok_or_else(|| format!("unknown key `{}`", args.key))?;
        let modifiers = args.modifiers.to_egui();
        with_modifiers(bridge, modifiers, async || {
            bridge
                .apply_events(vec![
                    Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers,
                    },
                ])
                .await
        })
        .await?;
        Ok(CallToolResult::structured(
            json!({ "ok": true, "key": args.key }),
        ))
    }

    /// Execute a sequence of app-driving tool calls in one round trip (the connection tools `attach`/`disconnect`/`status` are not available here).
    /// Stops on the first error.
    /// Results are emitted in execution order, interleaved: each step contributes one JSON text item followed by any image items it produced (e.g. screenshots).
    /// `batch` cannot be nested.
    /// Use this to act and observe in one call, e.g. a `click` then a `widget_tree` or `screenshot`.
    #[tool(
        title = "Batch",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn batch(
        &self,
        Parameters(args): Parameters<BatchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if args.actions.iter().any(|a| a.name == "batch") {
            return Ok(text_error("nested `batch` is not allowed"));
        }
        let mut content: Vec<ContentBlock> = Vec::new();
        let mut any_error = false;
        // Re-enter the UI router by name — same path as a top-level call, so each step parses
        // its args and runs exactly like a direct invocation.
        let router = Self::router();
        for action in args.actions {
            let mut request = CallToolRequestParams::new(action.name.clone());
            if let Value::Object(map) = action.args {
                request = request.with_arguments(map);
            }
            let result = self
                .dispatch(&router, request, ctx.clone())
                .await
                .unwrap_or_else(|e| text_error(e.message.to_string()));
            let mut step_texts: Vec<String> = Vec::new();
            let mut step_images: Vec<ContentBlock> = Vec::new();
            for item in &result.content {
                if let Some(text) = content_as_text(item) {
                    step_texts.push(text.to_owned());
                } else if content_is_image(item) {
                    step_images.push(item.clone());
                }
            }
            let entry = json!({
                "name": action.name,
                "isError": result.is_error.unwrap_or(false),
                "content": step_texts,
            });
            let entry_text = match serde_json::to_string(&entry) {
                Ok(s) => s,
                Err(e) => return Ok(text_error(format!("serialize batch step: {e}"))),
            };
            content.push(ContentBlock::text(entry_text));
            content.extend(step_images);
            if result.is_error.unwrap_or(false) {
                any_error = true;
                break;
            }
        }
        let mut result = if any_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        };
        result.is_error = Some(any_error);
        Ok(result)
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
                && let Err(err) = check_arguments(&tool.input_schema, request.arguments.as_ref())
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

    use serde_json::json;

    use super::*;

    /// Check `args` the way a call to `name` would.
    #[track_caller]
    fn check(name: &str, args: &Value) -> Result<(), String> {
        let tool = Server::new().get_tool(name).expect("a tool by that name");
        let args = args.as_object().expect("an object").clone();
        check_arguments(&tool.input_schema, Some(&args))
    }

    /// A misspelled filter used to read as "no filter", which quietly matches every widget.
    #[test]
    fn a_misspelled_argument_is_an_error() {
        let err = check("widget_tree", &json!({ "content_contain": "Save" }))
            .expect_err("`content_contain` is not an argument");
        assert!(
            err.starts_with("unknown argument `content_contain`"),
            "{err}"
        );
        assert!(
            err.contains("content_contains"),
            "it lists what is taken: {err}"
        );

        // Flattened fields belong to the tool just as much as the declared ones.
        check(
            "widget_tree",
            &json!({ "content_contains": "Save", "limit": 5 }),
        )
        .expect("both are real arguments");
    }

    /// Nested argument objects are checked too — `exclude` takes the same shape as the filter.
    #[test]
    fn a_misspelled_argument_inside_a_nested_object_is_an_error() {
        let err = check("widget_tree", &json!({ "exclude": { "rol": "Button" } }))
            .expect_err("`rol` is not a field of `exclude`");
        assert!(err.starts_with("unknown argument `exclude.rol`"), "{err}");

        check("widget_tree", &json!({ "exclude": { "role": "Button" } })).expect("`role` is one");
    }

    /// A batch step carries another tool's arguments, which the schema leaves free-form; the
    /// step itself still runs through its own tool's check when it is dispatched.
    #[test]
    fn a_free_form_argument_object_is_left_alone() {
        check(
            "batch",
            &json!({ "actions": [{ "name": "click", "args": { "anything": 1 } }] }),
        )
        .expect("`args` is free-form");

        let err = check("batch", &json!({ "actions": [{ "tool": "click" }] }))
            .expect_err("a step has a fixed shape");
        assert!(err.contains("`actions.tool`"), "{err}");
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
