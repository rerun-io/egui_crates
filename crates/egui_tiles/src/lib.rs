//! # [egui](https://github.com/emilk/egui) hierarchial tile manager
//! Tiles that can be arranges in horizontal, vertical, and grid-layouts, or put in tabs.
//! The tiles can be resized and re-arranged by drag-and-drop.
//!
//! ## Overview
//! The fundamental unit is the [`Tile`] which is either a [`Container`] or a `Pane` (a leaf).
//! The [`Tile`]s are put into a [`Tree`].
//! Everything is generic over the type of panes, leaving up to the user what to store in the tree.
//!
//! Each [`Tile`] is identified by a [`TileId`], handed out in sequence by the [`Tiles`] it lives in.
//! The tiles are stored in [`Tiles`].
//!
//! The entire state is stored in a single [`Tree`] struct which consists of a [`Tiles`] and a root [`TileId`].
//!
//! The behavior and the look of the [`Tree`] is controlled by the [`Behavior`] `trait`.
//! The user needs to implement this in order to specify the `ui` of each `Pane` and
//! the tab name of panes (if there are tab tiles).
//!
//! ## Example
//! See [`Tree`] for how to construct a tree.
//!
//! ```
//! // This specifies how you want to represent your panes in memory.
//! // Implementing serde is optional, but will make the entire tree serializable.
//! #[derive(serde::Serialize, serde::Deserialize)]
//! enum Pane {
//!     Settings,
//!     Text(String),
//! }
//!
//! fn tree_ui(ui: &mut egui::Ui, tree: &mut egui_tiles::Tree<Pane>, settings: &mut Settings) {
//!     let mut behavior = MyBehavior { settings };
//!     tree.ui(&mut behavior, ui);
//! }
//!
//! struct MyBehavior<'a> {
//!     settings: &'a mut Settings
//! }
//!
//! impl<'a> egui_tiles::Behavior<Pane> for MyBehavior<'a> {
//!     fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
//!         match pane {
//!             Pane::Settings => "Settings".into(),
//!             Pane::Text(text) => text.clone().into(),
//!         }
//!     }
//!
//!     fn pane_ui(
//!         &mut self,
//!         ui: &mut egui::Ui,
//!         _tile_id: egui_tiles::TileId,
//!         pane: &mut Pane,
//!     ) -> egui_tiles::UiResponse {
//!         match pane {
//!             Pane::Settings => self.settings.ui(ui),
//!             Pane::Text(text) => {
//!                 ui.text_edit_singleline(text);
//!             },
//!         }
//!
//!         Default::default()
//!     }
//!
//!     // you can override more methods to customize the behavior further
//! }
//!
//! struct Settings {
//!     checked: bool,
//! }
//!
//! impl Settings {
//!     fn ui(&mut self, ui: &mut egui::Ui) {
//!         ui.checkbox(&mut self.checked, "Checked");
//!     }
//! }
//! ```
//!
//! ## Invisible tiles
//! Tiles can be made invisible with [`Tree::set_visible`] and [`Tiles::set_visible`].
//! Invisible tiles still retain their ordering in the container their in until
//! they are made visible again.
//!
//! A container whose children are all invisible is not laid out either.
//! [`Tiles::is_visible`] reports the flag you set yourself, while
//! [`Tiles::is_visible_in_layout`] reports what actually gets space.
//!
//! ## Shares
//! The relative sizes of linear layout (horizontal or vertical) and grid columns and rows are specified by _shares_.
//! If the shares are `1,2,3` it means the first element gets `1/6` of the space, the second `2/6`, and the third `3/6`.
//! The default share size is `1`, and when resizing the shares are restributed so that
//! the total shares are always approximately the same as the number of rows/columns.
//! This makes it easy to add new rows/columns.
//!
//! ## Shortcomings
//! The implementation is recursive, so if your trees get too deep you will get a stack overflow.
//!
//! ## Future improvements
//! * Easy per-tab close-buttons
//! * Scrolling of tab-bar
//! * Vertical tab bar

// ## Implementation notes
// In many places we want to recursively visit all tiles, while also mutating them.
// In order to not get into trouble with the borrow checker a trick is used:
// each [`Tile`] is removed, mutated, recursed, and then re-added.
// You'll see this pattern many times reading the following code.
//
// Each frame consists of two passes: layout, and ui.
// The layout pass figures out where each tile should be placed.
// The ui pass does all the painting.
// These two passes could be combined into one pass if we wanted to,
// but having them split up makes the code slightly simpler, and
// leaves the door open for more complex layout (e.g. min/max sizes per tile).
//
// Everything is quite dynamic, so we have a bunch of defensive coding that call `warn!` on failure.
// These situations should not happen in normal use, but could happen if the user messes with
// the internals of the tree, putting it in an invalid state.

#![forbid(unsafe_code)]

use egui::{Pos2, Rect};

mod behavior;
mod container;
mod tile;
mod tiles;
mod tree;

pub use behavior::{Behavior, EditAction, TabState};
pub use container::{Container, ContainerKind, Grid, GridLayout, Linear, LinearDir, Shares, Tabs};
pub use tile::{Tile, TileId};
pub use tiles::Tiles;
pub use tree::Tree;

// ----------------------------------------------------------------------------

/// The response from [`Behavior::pane_ui`] for a pane.
#[must_use]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiResponse {
    #[default]
    None,

    /// The viewer is being dragged via some element in the Pane
    DragStarted,
}

/// What are the rules for simplifying the tree?
///
/// Drag-dropping tiles can often leave containers empty, or with only a single child.
/// The [`SimplificationOptions`] specifies what simplifications are allowed.
///
/// The [`Tree`] will run a simplification pass each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimplificationOptions {
    /// Remove empty [`Tabs`] containers?
    pub prune_empty_tabs: bool,

    /// Remove empty containers (that aren't [`Tabs`])?
    pub prune_empty_containers: bool,

    /// Remove [`Tabs`] containers with only a single child?
    ///
    /// Even if `true`, [`Self::all_panes_must_have_tabs`] will be respected.
    pub prune_single_child_tabs: bool,

    /// Prune containers (that aren't [`Tabs`]) with only a single child?
    pub prune_single_child_containers: bool,

    /// If true, each pane will have a [`Tabs`] container as a parent.
    ///
    /// This will win out over [`Self::prune_single_child_tabs`].
    pub all_panes_must_have_tabs: bool,

    /// If a horizontal container contain another horizontal container, join them?
    /// Same for vertical containers. Does NOT apply to grid container or tab containers.
    pub join_nested_linear_containers: bool,

    // Flatten tabs in tabs?
    pub flatten_tabs_in_tabs: bool,
}

impl SimplificationOptions {
    /// [`SimplificationOptions`] with all simplifications turned off.
    ///
    /// This makes it easy to run a single simplification type on a tree:
    /// ```
    /// # use egui_tiles::*;
    /// # let mut tree: Tree<()> = Tree::empty(egui::Id::unique("tree"));
    /// tree.simplify(&SimplificationOptions {
    ///     prune_empty_tabs: true,
    ///     ..SimplificationOptions::OFF
    /// });
    ///
    pub const OFF: Self = Self {
        prune_empty_tabs: false,
        prune_empty_containers: false,
        prune_single_child_tabs: false,
        prune_single_child_containers: false,
        all_panes_must_have_tabs: false,
        join_nested_linear_containers: false,
        flatten_tabs_in_tabs: false,
    };
}

impl Default for SimplificationOptions {
    fn default() -> Self {
        Self {
            prune_empty_tabs: true,
            prune_single_child_tabs: true,
            prune_empty_containers: true,
            prune_single_child_containers: true,
            all_panes_must_have_tabs: false,
            join_nested_linear_containers: true,
            flatten_tabs_in_tabs: false,
        }
    }
}

/// The current state of a resize handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeState {
    Idle,

    /// The user is hovering over the resize handle.
    Hovering,

    /// The user is dragging the resize handle.
    Dragging,
}

// ----------------------------------------------------------------------------

/// An insertion point in a specific container.
///
/// Specifies the expected container layout type, and where to insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainerInsertion {
    Tabs(usize),
    Horizontal(usize),
    Vertical(usize),
    Grid(usize),
}

impl ContainerInsertion {
    /// Where in the parent (in what order among its children).
    fn index(self) -> usize {
        match self {
            Self::Tabs(index)
            | Self::Horizontal(index)
            | Self::Vertical(index)
            | Self::Grid(index) => index,
        }
    }

    fn kind(self) -> ContainerKind {
        match self {
            Self::Tabs(_) => ContainerKind::Tabs,
            Self::Horizontal(_) => ContainerKind::Horizontal,
            Self::Vertical(_) => ContainerKind::Vertical,
            Self::Grid(_) => ContainerKind::Grid,
        }
    }
}

/// A place a dragged tile could be dropped, as offered to [`Behavior::allows_drop`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropTarget {
    /// The tile the dragged tile would be inserted next to, or into.
    ///
    /// A pane, or a container of another kind than `kind`, gets wrapped in a new
    /// container of that kind. A container of the same kind gets the dragged tile as a child.
    pub parent_id: TileId,

    /// The kind of container the dragged tile would end up in.
    pub kind: ContainerKind,

    /// Where the dragged tile would be previewed if this target is picked.
    pub preview_rect: Rect,
}

/// Where in the tree to insert a tile.
#[derive(Clone, Copy, Debug)]
struct InsertionPoint {
    pub parent_id: TileId,

    /// Where in the parent?
    pub insertion: ContainerInsertion,
}

impl InsertionPoint {
    pub fn new(parent_id: TileId, insertion: ContainerInsertion) -> Self {
        Self {
            parent_id,
            insertion,
        }
    }
}

#[derive(PartialEq, Eq)]
enum GcAction {
    Keep,
    Remove,
}

#[must_use]
enum SimplifyAction {
    Remove,
    Keep,
    Replace(TileId),
}

pub(crate) fn is_being_dragged(ctx: &egui::Context, tree_id: egui::Id, tile_id: TileId) -> bool {
    let dragged_id = ctx.dragged_id().or_else(|| ctx.drag_stopped_id());
    dragged_id == Some(tile_id.egui_id(tree_id))
}

/// If this tile is currently being dragged, cover it with a semi-transparent overlay ([`Behavior::dragged_overlay_color`]).
fn cover_tile_if_dragged<Pane>(
    tree: &Tree<Pane>,
    behavior: &dyn Behavior<Pane>,
    ui: &egui::Ui,
    tile_id: TileId,
) {
    if is_being_dragged(ui, tree.id, tile_id)
        && let Some(child_rect) = tree.tiles.rect(tile_id)
    {
        let overlay_color = behavior.dragged_overlay_color(ui.visuals());
        ui.painter().rect_filled(child_rect, 0.0, overlay_color);
    }
}

// ----------------------------------------------------------------------------

/// Context used for drag-and-dropping of tiles.
///
/// This is passed down during the `ui` pass.
/// Each tile registers itself with this context.
struct DropContext {
    enabled: bool,
    dragged_tile_id: Option<TileId>,
    mouse_pos: Option<Pos2>,

    /// Every drop target offered during the `ui` pass.
    ///
    /// Resolved into `best_insertion` by [`Self::pick_best`] once the whole tree has been shown.
    candidates: Vec<DropCandidate>,

    best_insertion: Option<InsertionPoint>,
    preview_rect: Option<Rect>,
}

#[derive(Clone, Copy, Debug)]
struct DropCandidate {
    insertion: InsertionPoint,
    preview_rect: Rect,
    dist_sq: f32,
}

impl DropContext {
    fn on_tile<Pane>(
        &mut self,
        behavior: &dyn Behavior<Pane>,
        style: &egui::Style,
        parent_id: TileId,
        rect: Rect,
        tile: &Tile<Pane>,
    ) {
        if !self.enabled {
            return;
        }

        if tile.kind() != Some(ContainerKind::Horizontal) {
            self.suggest_rect(
                InsertionPoint::new(parent_id, ContainerInsertion::Horizontal(0)),
                rect.split_left_right_at_fraction(0.5).0,
            );
            self.suggest_rect(
                InsertionPoint::new(parent_id, ContainerInsertion::Horizontal(usize::MAX)),
                rect.split_left_right_at_fraction(0.5).1,
            );
        }

        if tile.kind() != Some(ContainerKind::Vertical) {
            self.suggest_rect(
                InsertionPoint::new(parent_id, ContainerInsertion::Vertical(0)),
                rect.split_top_bottom_at_fraction(0.5).0,
            );
            self.suggest_rect(
                InsertionPoint::new(parent_id, ContainerInsertion::Vertical(usize::MAX)),
                rect.split_top_bottom_at_fraction(0.5).1,
            );
        }

        self.suggest_rect(
            InsertionPoint::new(parent_id, ContainerInsertion::Tabs(usize::MAX)),
            rect.split_top_bottom_at_y(rect.top() + behavior.tab_bar_height(style))
                .1,
        );
    }

    fn suggest_rect(&mut self, insertion: InsertionPoint, preview_rect: Rect) {
        if !self.enabled || self.dragged_tile_id.is_none() {
            return;
        }
        if let Some(mouse_pos) = self.mouse_pos {
            self.candidates.push(DropCandidate {
                insertion,
                preview_rect,
                dist_sq: mouse_pos.distance_sq(preview_rect.center()),
            });
        }
    }

    /// Pick the closest target that the behavior accepts.
    ///
    /// Must run after the whole tree has been shown: while a tile is being shown it is taken
    /// out of `tiles`, so only now can the behavior look at the target and its ancestors.
    fn pick_best<Pane>(&mut self, behavior: &dyn Behavior<Pane>, tiles: &Tiles<Pane>) {
        let Some(dragged_tile_id) = self.dragged_tile_id else {
            return;
        };
        self.candidates
            .sort_by(|a, b| a.dist_sq.total_cmp(&b.dist_sq));
        let best = self.candidates.iter().find(|candidate| {
            let target = DropTarget {
                parent_id: candidate.insertion.parent_id,
                kind: candidate.insertion.insertion.kind(),
                preview_rect: candidate.preview_rect,
            };
            behavior.allows_drop(tiles, dragged_tile_id, &target)
        });
        self.best_insertion = best.map(|candidate| candidate.insertion);
        self.preview_rect = best.map(|candidate| candidate.preview_rect);
    }
}
