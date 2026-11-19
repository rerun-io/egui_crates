//! The argument and result types of the tools.
//!
//! Their doc comments are part of each tool's JSON Schema, so they are written for the agent.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tree::{Id, Locator, Query, Widget, WidgetDetail};

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
    pub(crate) fn as_locator(&self) -> Option<Locator> {
        Locator::from_fields(self.id, self.query.clone())
    }
}

// ---------------------------------------------------------------------------------------
// Args structs
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    /// Output resolution in pixels per logical point.
    /// Defaults to `1.0`, which makes screenshot pixels line up 1:1 with the logical coordinates used by `click`/`widget_tree`.
    /// Higher values give a sharper image, capped at the display's native scale (no upscaling).
    #[serde(default = "default_pixels_per_point")]
    pub pixels_per_point: f32,
}

fn default_pixels_per_point() -> f32 {
    1.0
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
pub struct GetWidgetArgs {
    /// Widget id, as returned by `widget_tree`.
    pub id: Id,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
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
    pub(crate) fn to_egui(self) -> egui::Modifiers {
        egui::Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: self.mac_cmd,
            command: self.command,
        }
    }
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
    pub(crate) fn to_egui(self) -> egui::PointerButton {
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

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
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
