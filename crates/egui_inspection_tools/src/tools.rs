//! The tool definitions and their dispatch.
//!
//! [`tools()`] lists every tool with its JSON Schema; [`call`] runs one by name.
//!
//! Locator resolution happens here, not in the app: a tool fetches a fresh `AccessKit` tree
//! (`GetTree`), resolves the locator to a logical-point position, synthesizes the matching
//! `egui::Event`s, and sends them via `ApplyEvents`. The app-side handler stays low-level.

use std::pin::pin;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use egui::Event;
use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::args::{
    BatchArgs, ClickArgs, DragArgs, GetWidgetArgs, GetWidgetResult, HoverArgs, PressKeyArgs,
    ResizeArgs, ScreenshotArgs, ScrollArgs, Target, TypeTextArgs, WaitForArgs, WidgetTreeResult,
};
use crate::output::{Content, ToolOutput};
use crate::transport::{self, BoxFuture, Sleep, Transport, TreeSnapshot};
use crate::tree::{self, Id, Locator, QueryFilter, Widget};

/// A JSON object, as tool arguments and JSON Schemas are.
pub type JsonObject = serde_json::Map<String, Value>;

/// How long `wait_for` waits between two looks at the widget tree.
const WAIT_FOR_POLL_INTERVAL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------------------

/// What calling a tool does to the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Only observes the app.
    ReadOnly,

    /// Changes the app, but calling it again with the same arguments changes nothing more.
    Idempotent,

    /// Changes the app, and each call adds to the last, but loses no state.
    Cumulative,

    /// Changes the app in a way that may lose state, e.g. a click or typed text.
    Destructive,
}

impl Effect {
    /// The tool only observes the app.
    pub fn is_read_only(self) -> bool {
        self == Self::ReadOnly
    }
}

/// A tool an agent can call: its name, what it does, and the arguments it takes.
#[derive(Clone, Debug)]
pub struct ToolDef {
    /// The unique name to [`call`] it by, e.g. `click`.
    pub name: &'static str,

    /// A short human-readable name, e.g. `Click`.
    pub title: &'static str,

    /// What the tool does, for the agent. One sentence per line.
    pub description: &'static str,

    /// The JSON Schema (draft 2020-12) of the arguments: always an object.
    pub input_schema: JsonObject,

    /// The JSON Schema of [`ToolOutput::structured`], for tools that return one.
    pub output_schema: Option<JsonObject>,

    /// What calling the tool does to the app.
    pub effect: Effect,
}

impl ToolDef {
    fn new<Args: JsonSchema>(
        name: &'static str,
        title: &'static str,
        description: &'static str,
        effect: Effect,
    ) -> Self {
        Self {
            name,
            title,
            description,
            input_schema: input_schema::<Args>(),
            output_schema: None,
            effect,
        }
    }

    fn with_output<Output: JsonSchema>(mut self) -> Self {
        self.output_schema = Some(output_schema::<Output>());
        self
    }

    /// Check `arguments` against [`Self::input_schema`], then deserialize them.
    ///
    /// # Errors
    /// On an argument the schema doesn't declare, or one that doesn't deserialize.
    pub fn parse_arguments<Args: DeserializeOwned>(
        &self,
        arguments: JsonObject,
    ) -> Result<Args, String> {
        check_arguments(&self.input_schema, &arguments)?;
        serde_json::from_value(Value::Object(arguments))
            .map_err(|err| format!("failed to deserialize parameters: {err}"))
    }
}

/// The JSON Schema of a tool's arguments: draft 2020-12, without the top-level `title` and
/// `description` (the Rust type's name and doc, which are noise to an agent).
pub fn input_schema<Args: JsonSchema>() -> JsonObject {
    schema_object::<Args>(SchemaSettings::draft2020_12())
}

/// The JSON Schema of a tool's structured output, describing what `Serialize` writes.
fn output_schema<Output: JsonSchema>() -> JsonObject {
    schema_object::<Output>(SchemaSettings::draft2020_12().for_serialize())
}

fn schema_object<T: JsonSchema>(settings: SchemaSettings) -> JsonObject {
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let mut object = match serde_json::to_value(schema) {
        Ok(Value::Object(object)) => object,
        _ => JsonObject::new(),
    };
    object.remove("title");
    object.remove("description");
    object
}

/// Every tool, in a stable order.
pub fn tools() -> &'static [ToolDef] {
    static TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| {
        vec![
            ToolDef::new::<QueryFilter>(
                "widget_tree",
                "Widget tree",
                WIDGET_TREE_DESCRIPTION,
                Effect::ReadOnly,
            )
            .with_output::<WidgetTreeResult>(),
            ToolDef::new::<GetWidgetArgs>(
                "get_widget",
                "Widget details",
                GET_WIDGET_DESCRIPTION,
                Effect::ReadOnly,
            )
            .with_output::<GetWidgetResult>(),
            ToolDef::new::<ScreenshotArgs>(
                "screenshot",
                "Screenshot",
                SCREENSHOT_DESCRIPTION,
                Effect::ReadOnly,
            ),
            ToolDef::new::<WaitForArgs>(
                "wait_for",
                "Wait for widgets",
                WAIT_FOR_DESCRIPTION,
                Effect::ReadOnly,
            ),
            ToolDef::new::<ClickArgs>("click", "Click", CLICK_DESCRIPTION, Effect::Destructive),
            ToolDef::new::<HoverArgs>("hover", "Hover", HOVER_DESCRIPTION, Effect::Idempotent),
            ToolDef::new::<ScrollArgs>("scroll", "Scroll", SCROLL_DESCRIPTION, Effect::Cumulative),
            ToolDef::new::<DragArgs>("drag", "Drag", DRAG_DESCRIPTION, Effect::Destructive),
            ToolDef::new::<TypeTextArgs>(
                "type_text",
                "Type text",
                TYPE_TEXT_DESCRIPTION,
                Effect::Destructive,
            ),
            ToolDef::new::<PressKeyArgs>(
                "press_key",
                "Press key",
                PRESS_KEY_DESCRIPTION,
                Effect::Destructive,
            ),
            ToolDef::new::<ResizeArgs>(
                "resize",
                "Resize viewport",
                RESIZE_DESCRIPTION,
                Effect::Idempotent,
            ),
            BATCH.clone(),
        ]
    });
    &TOOLS
}

static BATCH: LazyLock<ToolDef> = LazyLock::new(|| {
    ToolDef::new::<BatchArgs>("batch", "Batch", BATCH_DESCRIPTION, Effect::Destructive)
});

/// The tool called `name`, if there is one.
pub fn tool(name: &str) -> Option<&'static ToolDef> {
    tools().iter().find(|tool| tool.name == name)
}

const WIDGET_TREE_DESCRIPTION: &str = "Walk the app's widget tree and return the widgets matching the filter, nested by ancestry: each one carries the matches from its own subtree in `children`.
Widgets in between that don't match are skipped, so a `children` entry is a descendant, not necessarily a direct child.
`role`, if given, is a role name (e.g. `Button`, `Label`), matched case-insensitively; an unknown role errors with the roles present in the tree.
`limit` (200 by default) caps how many widgets come back: whole levels are kept from the top down, so the app's structure survives and the leaves go first, and any widget whose children were cut reports how many in `omitted_children`.
`root` starts the walk at one widget instead of the app's root — pass the `id` of a widget that reported `omitted_children` to see that part of the tree in full.
`exclude` leaves a subtree out — pass a widget `id`, or the same constraints the filter takes — and takes everything below it with it; use it to skip a panel whose text would otherwise answer every query.
Use the returned `id` with `click`, `type_text`, or `get_widget`.
The widgets are abridged — call `get_widget` with an `id` for one widget's full detail, including its `bounds` (logical points, its center is where `click` lands), its parent, and its text in full.
`label` and `value` are cut short at 100 characters, marked with a trailing ` […]`; the filters still match against the whole text, so a phrase past the cut still finds its widget.
Empty scaffolding is dropped: a widget with no children, no `label`, no `value`, and no `role` (a `role` of `Unknown` is reported as none) never appears.";

const GET_WIDGET_DESCRIPTION: &str = "Return a single widget by id (from `widget_tree`), in full detail: its `bounds` in logical points, its `parent_id`, its state, and its `label`/`value` untruncated.
This is the follow-up to `widget_tree`, which omits `bounds` and cuts long text short to stay compact.";

const SCREENSHOT_DESCRIPTION: &str = "Capture the current frame as a PNG screenshot.
Defaults to logical-point resolution (`pixels_per_point: 1.0`) so pixels align with `click`/`widget_tree` coordinates; pass a higher `pixels_per_point` for detail.
Requires the app window to be visible — a fully-occluded or minimized window can't render a frame to capture (notably on macOS), so the call times out; bring the window to the foreground first.";

const CLICK_DESCRIPTION: &str = "Click the center of a widget's bounding box, or a raw `pos` in logical points.
Specify either a locator (`id` from `widget_tree`, `role`, or a text match — prefer `content_contains`, which matches `label` or `value`; `label_contains`/`value_contains` match just one field) or `pos: { x, y }`.
`button` defaults to `primary` (accepts `primary`/`secondary`/`middle`/`extra1`/`extra2`, or aliases `left`/`right`).
`count: 2` → double-click, `3` → triple.";

const HOVER_DESCRIPTION: &str = "Move the pointer over a widget (or raw `pos`) without clicking.
Tooltips and hover popups only appear after a short delay — follow with `wait_for` (e.g. its `min_steps`) to let them settle before reading the tree or screenshotting.";

const SCROLL_DESCRIPTION: &str = "Send a mouse wheel scroll over a widget (or raw `pos`).
`delta` is in logical points: positive Y scrolls down (reveals content below); positive X scrolls right.";

const DRAG_DESCRIPTION: &str = "Primary-button drag from `start` to `end`.
Each target accepts the same fields as `click`: locator (`id`/`content_contains`/`role`/`label_contains`/`value_contains`) or `pos: {x, y}`.
`steps` controls how many intermediate pointer-move events are emitted between press and release.";

const RESIZE_DESCRIPTION: &str = "Resize the app's viewport to the given logical-point dimensions.";

const WAIT_FOR_DESCRIPTION: &str = "Poll the widget tree until its conditions hold, or until `timeout_secs` elapses.
Waits until at least `min_matches` visible widgets match the filter (when one is given) *and* at least `min_steps` frames have rendered since the call began.
The text filter is `role` and/or one of `content_contains` (matches `label` or `value` — prefer this; e.g. monospace/`Label` text lives in `value`), `label_contains`, `value_contains`.
Requires a filter (`content_contains`/`role`/`label_contains`/`value_contains`) or a non-zero `min_steps`.";

const TYPE_TEXT_DESCRIPTION: &str = "Type text into the currently focused widget.
Optionally focus a widget first (by `id`, `role`, or a text match — `content_contains`/`label_contains`/`value_contains`) — this uses an `AccessKit` focus request, not a click, so it won't move the cursor or clear an existing text selection.";

const PRESS_KEY_DESCRIPTION: &str = "Send a key press (down + up) to the focused widget.
`key` is an egui key name such as `Backspace`, `Delete`, `Enter`, `Tab`, `A`–`Z`, `ArrowLeft`, `ArrowRight`, `Home`, `End`, `Escape`.";

const BATCH_DESCRIPTION: &str = "Execute a sequence of tool calls in one round trip.
Stops on the first error.
Results are emitted in execution order, interleaved: each step contributes one JSON text item followed by any image items it produced (e.g. screenshots).
`batch` cannot be nested.
Use this to act and observe in one call, e.g. a `click` then a `widget_tree` or `screenshot`.";

// ---------------------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------------------

/// Run the tool called `name` against the app behind `transport`.
///
/// Failures the agent can recover from — an unknown tool or argument, a widget that isn't
/// there, a transport error — come back as a [`ToolOutput`] with `is_error` set.
pub async fn call(
    transport: &dyn Transport,
    sleep: &dyn Sleep,
    name: &str,
    arguments: JsonObject,
) -> ToolOutput {
    if name == "batch" {
        let args = match BATCH.parse_arguments(arguments) {
            Ok(args) => args,
            Err(err) => return ToolOutput::error(err),
        };
        // Each step is a call of its own, by name: the same path as a top-level call, so it
        // checks and parses its arguments exactly like one.
        return run_batch(args, |name, arguments| {
            Box::pin(async move { call_one(transport, sleep, &name, arguments).await })
        })
        .await;
    }
    call_one(transport, sleep, name, arguments).await
}

/// [`call`] for any tool but `batch`, which can't be nested.
async fn call_one(
    transport: &dyn Transport,
    sleep: &dyn Sleep,
    name: &str,
    arguments: JsonObject,
) -> ToolOutput {
    let Some(def) = tool(name) else {
        return ToolOutput::error(format!("unknown tool `{name}`"));
    };
    run(transport, sleep, def, arguments)
        .await
        .unwrap_or_else(ToolOutput::error)
}

async fn run(
    transport: &dyn Transport,
    sleep: &dyn Sleep,
    def: &ToolDef,
    arguments: JsonObject,
) -> Result<ToolOutput, String> {
    match def.name {
        "widget_tree" => widget_tree(transport, def.parse_arguments(arguments)?).await,
        "get_widget" => get_widget(transport, def.parse_arguments(arguments)?).await,
        "screenshot" => screenshot(transport, def.parse_arguments(arguments)?).await,
        "wait_for" => wait_for(transport, sleep, def.parse_arguments(arguments)?).await,
        "click" => click(transport, def.parse_arguments(arguments)?).await,
        "hover" => hover(transport, def.parse_arguments(arguments)?).await,
        "scroll" => scroll(transport, def.parse_arguments(arguments)?).await,
        "drag" => drag(transport, def.parse_arguments(arguments)?).await,
        "type_text" => type_text(transport, def.parse_arguments(arguments)?).await,
        "press_key" => press_key(transport, def.parse_arguments(arguments)?).await,
        "resize" => resize(transport, def.parse_arguments(arguments)?).await,
        name => Err(format!("unknown tool `{name}`")),
    }
}

/// Run a `batch`: call `step` for each action, by its unprefixed tool name, until one fails.
///
/// [`call`] runs each step as a call of its own. A host that serves more tools, or overrides
/// some, passes its own dispatch as `step`, so a batch reaches those too.
pub async fn run_batch<'a>(
    args: BatchArgs,
    mut step: impl FnMut(String, JsonObject) -> BoxFuture<'a, ToolOutput>,
) -> ToolOutput {
    if args.actions.iter().any(|action| action.name == "batch") {
        return ToolOutput::error("nested `batch` is not allowed");
    }
    let mut content = Vec::new();
    let mut any_error = false;
    for action in args.actions {
        let arguments = match action.args {
            Value::Object(map) => map,
            _ => JsonObject::new(),
        };
        let result = step(action.name.clone(), arguments).await;
        let mut step_texts = Vec::new();
        let mut step_images = Vec::new();
        for item in result.content {
            match item {
                Content::Text(text) => step_texts.push(text),
                image @ Content::Image { .. } => step_images.push(image),
            }
        }
        let entry = json!({
            "name": action.name,
            "isError": result.is_error,
            "content": step_texts,
        });
        content.push(Content::Text(entry.to_string()));
        content.extend(step_images);
        if result.is_error {
            any_error = true;
            break;
        }
    }
    ToolOutput {
        content,
        structured: None,
        is_error: any_error,
    }
}

/// Reject an argument the tool's schema doesn't declare.
///
/// Serde ignores unknown keys, and `#[serde(flatten)]` rules out `deny_unknown_fields`, so
/// without this a misspelled argument is silently ignored.
/// [`ToolDef::parse_arguments`] does this for you.
///
/// # Errors
/// Names the first unknown argument, and lists the ones the schema takes.
pub fn check_arguments(schema: &JsonObject, arguments: &JsonObject) -> Result<(), String> {
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

// ---------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------

/// Resolve a [`Target`] to an optional widget id + a logical-point position, fetching a fresh
/// tree first. Use [`resolve_in_tree`] directly when resolving several targets against one
/// snapshot (e.g. a drag's start and end).
async fn resolve_target(
    transport: &dyn Transport,
    target: &Target,
) -> Result<(Option<Id>, egui::Pos2), String> {
    // A raw `pos` target needs no tree.
    if let Some(p) = target.pos {
        return Ok((None, egui::Pos2::new(p.x, p.y)));
    }
    let snap = transport::fetch_tree(transport).await?;
    resolve_in_tree(&snap, target)
}

/// Resolve a [`Target`] against an already-fetched tree snapshot.
fn resolve_in_tree(
    snap: &TreeSnapshot,
    target: &Target,
) -> Result<(Option<Id>, egui::Pos2), String> {
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

/// Hold `modifiers` down for the duration of `action`, then release them.
async fn with_modifiers<T>(
    transport: &dyn Transport,
    modifiers: egui::Modifiers,
    action: impl AsyncFnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if modifiers == egui::Modifiers::NONE {
        return action().await;
    }
    transport::apply_events(transport, vec![Event::ModifiersChanged(modifiers)]).await?;
    let result = action().await;
    // Release even when the action failed, so the next call doesn't inherit a stuck modifier.
    let released = transport::apply_events(
        transport,
        vec![Event::ModifiersChanged(egui::Modifiers::NONE)],
    )
    .await;
    let value = result?;
    released?;
    Ok(value)
}

/// Run `main` to completion while also driving `background`, which is dropped once `main` is
/// done.
///
/// Unlike a race, this never abandons `main` halfway — a request it has sent always gets its
/// response read.
async fn with_background<T>(
    main: impl Future<Output = T>,
    background: impl Future<Output = ()>,
) -> T {
    let mut main = pin!(main);
    let mut background = pin!(background);
    let mut background_done = false;
    std::future::poll_fn(|cx| {
        if !background_done && background.as_mut().poll(cx).is_ready() {
            background_done = true;
        }
        main.as_mut().poll(cx)
    })
    .await
}

// ---------------------------------------------------------------------------------------
// The tools
// ---------------------------------------------------------------------------------------

async fn screenshot(transport: &dyn Transport, args: ScreenshotArgs) -> Result<ToolOutput, String> {
    let png = transport::screenshot(transport, args.pixels_per_point).await?;
    let meta = json!({ "width": png.size[0], "height": png.size[1] });
    Ok(ToolOutput::success(vec![
        Content::Text(meta.to_string()),
        Content::Image {
            bytes: png.bytes,
            mime_type: "image/png".to_owned(),
        },
    ]))
}

async fn widget_tree(transport: &dyn Transport, filter: QueryFilter) -> Result<ToolOutput, String> {
    let snap = transport::fetch_tree(transport).await?;
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
    structured(&WidgetTreeResult { nodes })
}

async fn get_widget(transport: &dyn Transport, args: GetWidgetArgs) -> Result<ToolOutput, String> {
    let locator = Locator::Id { id: args.id };
    let snap = transport::fetch_tree(transport).await?;
    let ppp = snap.pixels_per_point;
    // `get_widget` is a lookup, not an action: a missing id is `null`, not an error.
    let node = match snap.tree {
        Some(tree) => tree::resolve_unique(&tree, &locator, ppp)
            .ok()
            .map(|n| tree::widget_detail(&n, ppp)),
        None => None,
    };
    structured(&GetWidgetResult { node })
}

fn structured(value: &impl serde::Serialize) -> Result<ToolOutput, String> {
    serde_json::to_value(value)
        .map(ToolOutput::structured)
        .map_err(|err| format!("Failed to serialize structured content: {err}"))
}

async fn click(transport: &dyn Transport, args: ClickArgs) -> Result<ToolOutput, String> {
    let button = args.button.to_egui();
    let count = args.count.max(1);
    let modifiers = args.modifiers.to_egui();
    let (node_id, center) = resolve_target(transport, &args.target).await?;

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
    with_modifiers(transport, modifiers, async || {
        transport::apply_events(transport, events).await
    })
    .await?;
    Ok(ToolOutput::structured(json!({
        "ok": true,
        "clicked_id": node_id,
        "pos": [center.x, center.y],
        "button": args.button,
        "count": count,
    })))
}

async fn hover(transport: &dyn Transport, args: HoverArgs) -> Result<ToolOutput, String> {
    let (node_id, pos) = resolve_target(transport, &args.target).await?;
    transport::apply_events(transport, vec![Event::PointerMoved(pos)]).await?;
    Ok(ToolOutput::structured(json!({
        "ok": true,
        "hovered_id": node_id,
        "pos": [pos.x, pos.y],
    })))
}

async fn scroll(transport: &dyn Transport, args: ScrollArgs) -> Result<ToolOutput, String> {
    let (node_id, pos) = resolve_target(transport, &args.target).await?;
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
    with_modifiers(transport, modifiers, async || {
        transport::apply_events(transport, events).await
    })
    .await?;
    Ok(ToolOutput::structured(json!({
        "ok": true,
        "scrolled_id": node_id,
        "pos": [pos.x, pos.y],
        "delta": [args.delta.x, args.delta.y],
    })))
}

async fn drag(transport: &dyn Transport, args: DragArgs) -> Result<ToolOutput, String> {
    // Resolve both endpoints against one tree snapshot — no input happens between them.
    let snap = transport::fetch_tree(transport).await?;
    let (start_id, start_pos) = resolve_in_tree(&snap, &args.start)?;
    let (end_id, end_pos) = resolve_in_tree(&snap, &args.end)?;
    let modifiers = args.modifiers.to_egui();
    let steps = args.steps.max(1);

    // One hold spans the whole gesture: the modifier state persists across the frames the
    // press, the waypoints, and the release are spread over.
    with_modifiers(transport, modifiers, async || {
        transport::apply_events(
            transport,
            vec![
                Event::PointerMoved(start_pos),
                Event::PointerButton {
                    pos: start_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
        )
        .await?;

        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let waypoint = egui::Pos2::new(
                start_pos.x + (end_pos.x - start_pos.x) * t,
                start_pos.y + (end_pos.y - start_pos.y) * t,
            );
            transport::apply_events(transport, vec![Event::PointerMoved(waypoint)]).await?;
        }

        transport::apply_events(
            transport,
            vec![Event::PointerButton {
                pos: end_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
        )
        .await
    })
    .await?;
    Ok(ToolOutput::structured(json!({
        "ok": true,
        "start_id": start_id,
        "end_id": end_id,
        "start_pos": [start_pos.x, start_pos.y],
        "end_pos": [end_pos.x, end_pos.y],
        "steps": steps,
    })))
}

async fn resize(transport: &dyn Transport, args: ResizeArgs) -> Result<ToolOutput, String> {
    transport::resize(transport, args.width, args.height).await?;
    Ok(ToolOutput::structured(
        json!({ "ok": true, "width": args.width, "height": args.height }),
    ))
}

async fn wait_for(
    transport: &dyn Transport,
    sleep: &dyn Sleep,
    args: WaitForArgs,
) -> Result<ToolOutput, String> {
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

    // Set by a timer running next to the polling, which checks it after each look at the tree.
    // A timer rather than a clock, because the host supplies the timer and `std::time::Instant`
    // panics on the web.
    let timed_out = AtomicBool::new(args.timeout_secs == 0);
    let timer = async {
        sleep.sleep(Duration::from_secs(args.timeout_secs)).await;
        timed_out.store(true, Ordering::Relaxed);
    };

    let poll = async {
        let mut start_step = None;
        loop {
            let snap = transport::fetch_tree(transport).await?;
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
            if matched_ok && args.min_steps <= steps_waited {
                return Ok(ToolOutput::structured(
                    json!({ "ok": true, "matched": matches, "steps_waited": steps_waited }),
                ));
            }
            if timed_out.load(Ordering::Relaxed) {
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
            sleep.sleep(WAIT_FOR_POLL_INTERVAL).await;
        }
    };

    with_background(poll, timer).await
}

async fn type_text(transport: &dyn Transport, args: TypeTextArgs) -> Result<ToolOutput, String> {
    // Optionally focus a target first. Unlike a click, an AccessKit focus request doesn't
    // move the text cursor or reset the selection.
    let focused_id = match Locator::from_fields(args.id, args.query.clone()) {
        Some(locator) => {
            let snap = transport::fetch_tree(transport).await?;
            if let Some(role) = &args.query.role {
                tree::validate_role(role, snap.tree.as_ref())?;
            }
            let tree = snap.tree.as_ref().ok_or("no accesskit tree yet")?;
            let node = tree::resolve_unique(tree, &locator, snap.pixels_per_point)?;
            let id = tree::accesskit_id(&node);
            transport::apply_events(
                transport,
                vec![Event::AccessKitActionRequest(accesskit::ActionRequest {
                    action: accesskit::Action::Focus,
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: accesskit::NodeId(id.0),
                    data: None,
                })],
            )
            .await?;
            Some(id)
        }
        None => None,
    };

    if !args.text.is_empty() {
        transport::apply_events(transport, vec![Event::Text(args.text)]).await?;
    }

    Ok(ToolOutput::structured(json!({
        "ok": true,
        "focused_id": focused_id,
    })))
}

async fn press_key(transport: &dyn Transport, args: PressKeyArgs) -> Result<ToolOutput, String> {
    let key =
        egui::Key::from_name(&args.key).ok_or_else(|| format!("unknown key `{}`", args.key))?;
    let modifiers = args.modifiers.to_egui();
    with_modifiers(transport, modifiers, async || {
        transport::apply_events(
            transport,
            vec![
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
            ],
        )
        .await
    })
    .await?;
    Ok(ToolOutput::structured(
        json!({ "ok": true, "key": args.key }),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Check `args` the way a call to `name` would.
    #[track_caller]
    fn check(name: &str, args: &Value) -> Result<(), String> {
        let tool = tool(name).expect("a tool by that name");
        check_arguments(&tool.input_schema, args.as_object().expect("an object"))
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
}
