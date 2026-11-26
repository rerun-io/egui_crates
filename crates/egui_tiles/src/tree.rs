use egui::{Key, Modifiers, NumExt as _, Rect, Ui};

use crate::behavior::{EditAction, layout_tiles};
use crate::{ContainerInsertion, ContainerKind, PreviewOptions, UiResponse};

use super::{
    Behavior, Container, DropContext, InsertionPoint, SimplificationOptions, SimplifyAction, Tabs,
    Tile, TileId, Tiles,
};

/// Rects closer than this (in points, summed over both corners) count as converged.
const RECT_CONVERGENCE_THRESHOLD: f32 = 0.5;

/// The ids of every pane in the tree, sorted.
fn sorted_pane_ids<Pane>(tiles: &Tiles<Pane>) -> Vec<u64> {
    let mut ids: Vec<u64> = tiles
        .iter()
        .filter_map(|(id, tile)| matches!(tile, Tile::Pane(_)).then_some(id.0))
        .collect();
    ids.sort_unstable();
    ids
}

/// The _original_ ids of every pane in a [`Tiles::skeleton`], sorted.
///
/// A skeleton stores each pane's original [`TileId`] as its payload, so this must always agree
/// with [`sorted_pane_ids`] of the tree it was built from: speculative edits move panes around,
/// but must never lose or duplicate one.
fn sorted_skeleton_pane_ids(tiles: &Tiles<TileId>) -> Vec<u64> {
    let mut ids: Vec<u64> = tiles
        .tiles()
        .filter_map(|tile| match tile {
            Tile::Pane(original_id) => Some(original_id.0),
            Tile::Container(_) => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// How far to move a smoothed rect towards its target this frame.
///
/// [`egui::Style::animation_time`] sets the pace, so the drag preview animates in step with the
/// rest of the ui.
fn smoothing_factor(ui: &Ui, options: &PreviewOptions) -> f32 {
    let dt = ui.input(|input| input.stable_dt).at_most(0.1);
    egui::emath::exponential_smooth_factor(
        options.reach_this_fraction,
        ui.style().animation_time,
        dt,
    )
}

/// Are `a` and `b` close enough to be considered the same rect?
fn rects_close_enough(a: Rect, b: Rect) -> bool {
    a.min.distance(b.min) + a.max.distance(b.max) < RECT_CONVERGENCE_THRESHOLD
}

/// The parts of the animated drag preview that must survive to the next frame.
///
/// Kept in [`egui::Memory`] rather than in the [`Tree`], because some applications
/// (e.g. Rerun) re-create their [`Tree`] from scratch every frame, which would otherwise
/// reset this before it was ever read, silently disabling the preview.
#[derive(Clone, Default)]
struct PreviewMemory {
    /// The best insertion point from the previous frame's [`DropContext`].
    ///
    /// Where a dragged tile would land is only known at the _end_ of a frame,
    /// so the speculative layout always works off the previous frame's answer.
    insertion: Option<InsertionPoint>,

    /// Where each tile is drawn right now, on its way to its target rect.
    smoothed_rects: ahash::HashMap<TileId, Rect>,
}

impl PreviewMemory {
    fn data_id(tree_id: egui::Id) -> egui::Id {
        tree_id.with("egui_tiles_preview_state")
    }

    fn load(ctx: &egui::Context, tree_id: egui::Id) -> Self {
        ctx.data_mut(|data| data.get_temp(Self::data_id(tree_id)).unwrap_or_default())
    }

    fn store(self, ctx: &egui::Context, tree_id: egui::Id) {
        let data_id = Self::data_id(tree_id);
        ctx.data_mut(|data| {
            if self.is_idle() {
                data.remove::<Self>(data_id);
            } else {
                data.insert_temp(data_id, self);
            }
        });
    }

    /// Is there nothing worth remembering until the next frame?
    fn is_idle(&self) -> bool {
        let Self {
            insertion,
            smoothed_rects,
        } = self;

        insertion.is_none() && smoothed_rects.is_empty()
    }
}

/// Frame-local drag-preview state, owned by the [`Tree`] for the duration of [`Tree::ui`].
#[derive(Clone, Default)]
struct Preview {
    /// Carried over from the previous frame, and stored back at the end of this one.
    memory: PreviewMemory,

    /// What the tree would look like if the dragged tile were dropped right now: a laid-out
    /// [`Tiles::skeleton`] with the pending move already applied.
    ///
    /// Every tile keeps its [`TileId`] through a move and through simplification, so this is read
    /// with the real tree's ids: a tile's target rect is `tiles.rect(id)`, and how a `Tabs`
    /// container would look is `tiles.get_container(id)`.
    ///
    /// Just the [`Tiles`], not a whole [`Tree`] — a `Tree` owns a `Preview`, so that would be
    /// infinitely recursive, and nothing here needs the speculative root anyway.
    ///
    /// Derived fresh each frame from the real tree plus [`PreviewMemory::insertion`], so it can
    /// never go stale. Never persisted.
    tiles: Option<Tiles<TileId>>,
}

/// The top level type. Contains all persistent state, including layouts and sizes.
///
/// You'll usually construct this once and then store it, calling [`Tree::ui`] each frame.
///
/// See [the crate-level documentation](crate) for a complete example.
///
/// ## How to construct a [`Tree`]
/// ```
/// use egui_tiles::{Tiles, TileId, Tree};
///
/// struct Pane { } // put some state here
///
/// let mut tiles = Tiles::default();
/// let tabs: Vec<TileId> = vec![tiles.insert_pane(Pane { }), tiles.insert_pane(Pane { })];
/// let root: TileId = tiles.insert_tab_tile(tabs);
///
/// let tree = Tree::new(egui::Id::unique("my_tree"), root, tiles);
/// ```
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Tree<Pane> {
    /// The constant, globally unique id of this tree.
    pub(crate) id: egui::Id,

    /// None = empty tree
    pub root: Option<TileId>,

    /// All the tiles in the tree.
    pub tiles: Tiles<Pane>,

    /// When finite, this values contains the exact height of this tree
    #[cfg_attr(
        feature = "serde",
        serde(serialize_with = "serialize_f32_infinity_as_null"),
        serde(deserialize_with = "deserialize_f32_null_as_infinity")
    )]
    height: f32,

    /// When finite, this values contains the exact width of this tree
    #[cfg_attr(
        feature = "serde",
        serde(serialize_with = "serialize_f32_infinity_as_null"),
        serde(deserialize_with = "deserialize_f32_null_as_infinity")
    )]
    width: f32,

    /// Transient drag-preview state, only meaningful during [`Tree::ui`].
    ///
    /// The part of it that outlives the frame lives in [`egui::Memory`]; see [`PreviewMemory`].
    #[cfg_attr(feature = "serde", serde(skip))]
    preview: Preview,
}

// Workaround for JSON which doesn't support infinity, because JSON is stupid.
#[cfg(feature = "serde")]
fn serialize_f32_infinity_as_null<S: serde::Serializer>(
    t: &f32,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if t.is_infinite() {
        serializer.serialize_none()
    } else {
        serializer.serialize_some(t)
    }
}

#[cfg(feature = "serde")]
fn deserialize_f32_null_as_infinity<'de, D: serde::Deserializer<'de>>(
    des: D,
) -> Result<f32, D::Error> {
    use serde::Deserialize as _;
    Ok(Option::<f32>::deserialize(des)?.unwrap_or(f32::INFINITY))
}

impl<Pane: PartialEq> PartialEq for Tree<Pane> {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            id,
            root,
            tiles,
            height,
            width,
            preview: _, // transient, excluded
        } = self;

        *id == other.id
            && *root == other.root
            && *tiles == other.tiles
            && *height == other.height
            && *width == other.width
    }
}

impl<Pane: std::fmt::Debug> std::fmt::Debug for Tree<Pane> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print a hierarchical view of the tree:
        fn format_tile<Pane: std::fmt::Debug>(
            f: &mut std::fmt::Formatter<'_>,
            tiles: &Tiles<Pane>,
            indent: usize,
            tile_id: TileId,
        ) -> std::fmt::Result {
            write!(f, "{} {tile_id:?}: ", "  ".repeat(indent))?;
            if let Some(tile) = tiles.get(tile_id) {
                match tile {
                    Tile::Pane(pane) => writeln!(f, "Pane {pane:?}"),
                    Tile::Container(container) => {
                        writeln!(
                            f,
                            "{}",
                            match container {
                                Container::Tabs(_) => "Tabs",
                                Container::Linear(_) => "Linear",
                                Container::Grid(_) => "Grid",
                            }
                        )?;
                        for &child in container.children() {
                            format_tile(f, tiles, indent + 1, child)?;
                        }
                        Ok(())
                    }
                }
            } else {
                writeln!(f, "DANGLING")
            }
        }

        let Self {
            id,
            root,
            tiles,
            width,
            height,
            ..
        } = self;

        if let Some(root) = root {
            writeln!(f, "Tree {{")?;
            writeln!(f, "    id: {id:?}")?;
            writeln!(f, "    width: {width:?}")?;
            writeln!(f, "    height: {height:?}")?;
            format_tile(f, tiles, 1, *root)?;
            write!(f, "}}")
        } else {
            writeln!(f, "Tree {{ }}")
        }
    }
}

// ----------------------------------------------------------------------------

impl<Pane> Tree<Pane> {
    /// Construct an empty tree.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn empty(id: impl Into<egui::Id>) -> Self {
        Self {
            id: id.into(),
            root: None,
            tiles: Default::default(),
            width: f32::INFINITY,
            height: f32::INFINITY,
            preview: Default::default(),
        }
    }

    /// The most flexible constructor, allowing you to set up the tiles
    /// however you want.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn new(id: impl Into<egui::Id>, root: TileId, tiles: Tiles<Pane>) -> Self {
        Self {
            id: id.into(),
            root: Some(root),
            tiles,
            width: f32::INFINITY,
            height: f32::INFINITY,
            preview: Default::default(),
        }
    }

    /// Create a top-level [`crate::Tabs`] container with the given panes.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn new_tabs(id: impl Into<egui::Id>, panes: Vec<Pane>) -> Self {
        Self::new_container(id, ContainerKind::Tabs, panes)
    }

    /// Create a top-level horizontal [`crate::Linear`] container with the given panes.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn new_horizontal(id: impl Into<egui::Id>, panes: Vec<Pane>) -> Self {
        Self::new_container(id, ContainerKind::Horizontal, panes)
    }

    /// Create a top-level vertical [`crate::Linear`] container with the given panes.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn new_vertical(id: impl Into<egui::Id>, panes: Vec<Pane>) -> Self {
        Self::new_container(id, ContainerKind::Vertical, panes)
    }

    /// Create a top-level [`crate::Grid`] container with the given panes.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn new_grid(id: impl Into<egui::Id>, panes: Vec<Pane>) -> Self {
        Self::new_container(id, ContainerKind::Grid, panes)
    }

    /// Create a top-level container with the given panes.
    ///
    /// The `id` must be _globally_ unique (!).
    /// This is so that the same tree can be added to different [`egui::Ui`]s (if you want).
    pub fn new_container(id: impl Into<egui::Id>, kind: ContainerKind, panes: Vec<Pane>) -> Self {
        let mut tiles = Tiles::default();
        let tile_ids = panes
            .into_iter()
            .map(|pane| tiles.insert_pane(pane))
            .collect();
        let root = tiles.insert_new(Tile::Container(Container::new(kind, tile_ids)));
        Self::new(id, root, tiles)
    }

    /// Remove the given tile and all child tiles, recursively.
    ///
    /// This also removes the tile id from the parent's list of children.
    ///
    /// All removed tiles are returned in unspecified order.
    pub fn remove_recursively(&mut self, id: TileId) -> Vec<Tile<Pane>> {
        // Remove the top-most tile_id from its parent
        self.remove_tile_id_from_parent(id);

        let mut removed_tiles = vec![];
        self.remove_recursively_impl(id, &mut removed_tiles);
        removed_tiles
    }

    fn remove_recursively_impl(&mut self, id: TileId, removed_tiles: &mut Vec<Tile<Pane>>) {
        // We can safely use the raw `tiles.remove` API here because either the parent was cleaned
        // up explicitly from `remove_recursively` or the parent is also being removed so there's
        // no reason to clean it up.
        if let Some(tile) = self.tiles.remove(id) {
            if let Tile::Container(container) = &tile {
                for &child_id in container.children() {
                    self.remove_recursively_impl(child_id, removed_tiles);
                }
            }
            removed_tiles.push(tile);
        }
    }

    /// The globally unique id used by this `Tree`.
    #[inline]
    pub fn id(&self) -> egui::Id {
        self.id
    }

    /// Check if [`Self::root`] is [`None`].
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    #[inline]
    pub fn root(&self) -> Option<TileId> {
        self.root
    }

    #[inline]
    pub fn is_root(&self, tile: TileId) -> bool {
        self.root == Some(tile)
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn is_visible(&self, tile_id: TileId) -> bool {
        self.tiles.is_visible(tile_id)
    }

    /// Is this tile given any space in the layout?
    ///
    /// See [`Tiles::is_visible_in_layout`].
    pub fn is_visible_in_layout(&self, tile_id: TileId) -> bool {
        self.tiles.is_visible_in_layout(tile_id)
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn set_visible(&mut self, tile_id: TileId, visible: bool) {
        self.tiles.set_visible(tile_id, visible);
    }

    /// All visible tiles.
    ///
    /// This excludes all tiles that are invisible or are inactive tabs, recursively,
    /// as well as containers left with nothing visible to show.
    ///
    /// The order of the returned tiles is arbitrary.
    pub fn active_tiles(&self) -> Vec<TileId> {
        let mut tiles = vec![];
        if let Some(root) = self.root {
            self.tiles.collect_active_tiles(root, &mut tiles);
        }
        tiles
    }

    /// All non-visible tiles.
    ///
    /// This includes all tiles that are invisible or are inactive tabs. Uses `active_tiles`.
    ///
    /// The order of the returned tiles is arbitrary.
    pub fn inactive_tiles(&self) -> Vec<TileId> {
        let active_tiles = self.active_tiles();
        self.tiles
            .tile_ids()
            .filter(|id| !active_tiles.contains(id))
            .collect()
    }

    /// Show the tree in the given [`Ui`].
    ///
    /// The tree will use upp all the available space - nothing more, nothing less.
    pub fn ui(&mut self, behavior: &mut dyn Behavior<Pane>, ui: &mut Ui) {
        self.simplify(&behavior.simplification_options());

        self.gc(behavior);

        self.tiles.rects.clear();

        // Check if anything is being dragged:
        let dragged_id = self.dragged_id(ui);
        let mut drop_context = DropContext {
            enabled: true,
            dragged_tile_id: dragged_id,
            mouse_pos: ui.input(|i| i.pointer.interact_pos()),
            best_dist_sq: f32::INFINITY,
            best_insertion: None,
            preview_rect: None,
        };

        let mut rect = ui.available_rect_before_wrap();
        if self.height.is_finite() {
            rect.set_height(self.height);
        }
        if self.width.is_finite() {
            rect.set_width(self.width);
        }

        if layout_tiles(&mut self.tiles, self.root, behavior, ui.style(), rect) {
            behavior.on_edit(EditAction::TabSelected);
        }

        let preview_options = behavior.preview_options();
        self.preview.memory = PreviewMemory::load(ui.ctx(), self.id);

        if dragged_id.is_none() || !preview_options.enabled {
            // Forget where the tile would have landed, but keep `smoothed_rects`
            // so that the tiles animate back into place.
            self.preview.memory.insertion = None;
        }

        // Derived, never stored: worst case the preview is missing for a frame.
        self.preview.tiles = self.preview.memory.insertion.and_then(|insertion| {
            let dragged_id = dragged_id?;
            Some(self.speculate(dragged_id, insertion, behavior, ui.style(), rect))
        });

        self.update_smoothed_rects(ui, dragged_id, &preview_options);

        if let Some(root) = self.root {
            self.tile_ui(behavior, &mut drop_context, ui, root);
        }

        if dragged_id.is_some() {
            // Where the dragged tile would land is only known now that every tile has
            // registered its drop zones. Remember it, so the next frame can speculate on it.
            self.preview.memory.insertion = drop_context.best_insertion;
        }

        // NOTE: this commits the drop, and clears the preview when it does.
        self.preview_dragged_tile(behavior, &drop_context, ui, &preview_options);

        std::mem::take(&mut self.preview)
            .memory
            .store(ui.ctx(), self.id);

        self.handle_tab_cycle_shortcuts(behavior, ui);

        ui.advance_cursor_after_rect(rect);
    }

    /// `Ctrl+Tab` / `Ctrl+Shift+Tab` cycles the active tab of the innermost hovered tab container.
    ///
    /// Must run after the tiles have been shown, so the new active tab takes effect next frame,
    /// same as when a tab is clicked.
    fn handle_tab_cycle_shortcuts(&mut self, behavior: &mut dyn Behavior<Pane>, ui: &Ui) {
        if !behavior.cycle_tabs_with_keyboard() {
            return;
        }

        let hovered_tabs = self
            .tiles
            .iter()
            .filter(|(_, tile)| matches!(tile, Tile::Container(Container::Tabs(_))))
            .filter_map(|(&tile_id, _)| Some((tile_id, self.tiles.rect(tile_id)?)))
            .filter(|(_, rect)| ui.rect_contains_pointer(*rect))
            .min_by(|(_, a), (_, b)| a.area().total_cmp(&b.area()))
            .map(|(tile_id, _)| tile_id);

        let Some(tabs_id) = hovered_tabs else {
            return;
        };

        // Check the more specific shortcut first: `consume_key` ignores extra Shift.
        let forward =
            if ui.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab)) {
                false
            } else if ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab)) {
                true
            } else {
                return;
            };

        let Some(mut tile) = self.tiles.remove(tabs_id) else {
            return;
        };
        if let Tile::Container(Container::Tabs(tabs)) = &mut tile
            && tabs.cycle_active(&self.tiles, forward)
        {
            behavior.on_edit(EditAction::TabSelected);
        }
        self.tiles.insert(tabs_id, tile);
    }

    /// Sets the exact height that can be used by the tree.
    ///
    /// Determines the height that will be used by the tree component.
    /// By default, the tree occupies all the available space in the parent container.
    pub fn set_height(&mut self, height: f32) {
        if height.is_sign_positive() && height.is_finite() {
            self.height = height;
        } else {
            self.height = f32::INFINITY;
        }
    }

    /// Sets the exact width that can be used by the tree.
    ///
    /// Determines the width that will be used by the tree component.
    /// By default, the tree occupies all the available space in the parent container.
    pub fn set_width(&mut self, width: f32) {
        if width.is_sign_positive() && width.is_finite() {
            self.width = width;
        } else {
            self.width = f32::INFINITY;
        }
    }

    pub(super) fn tile_ui(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &mut DropContext,
        ui: &Ui,
        tile_id: TileId,
    ) {
        if !self.is_visible_in_layout(tile_id) {
            return;
        }
        // NOTE: important that we get the rect and tile in two steps,
        // otherwise we could loose the tile when there is no rect.
        let Some(rect) = self.display_rect(tile_id) else {
            log::debug!("Failed to find rect for tile {tile_id:?} during ui");
            return;
        };
        let Some(mut tile) = self.tiles.remove(tile_id) else {
            log::debug!("Failed to find tile {tile_id:?} during ui");
            return;
        };

        let drop_context_was_enabled = drop_context.enabled;
        if Some(tile_id) == drop_context.dragged_tile_id {
            // Can't drag a tile onto self or any children
            drop_context.enabled = false;
        }
        // Use actual (non-animated) rect for drop zones to prevent a feedback loop
        let drop_rect = self.tiles.rect(tile_id).unwrap_or(rect);
        drop_context.on_tile(behavior, ui.style(), tile_id, drop_rect, &tile);

        // Each tile gets its own `Ui`, nested inside each other, with proper clip rectangles.
        let enabled = ui.is_enabled();
        let mut ui = egui::Ui::new(
            ui.ctx().clone(),
            ui.scope_id().with(tile_id),
            egui::UiBuilder::new()
                .layer_id(ui.layer_id())
                .accessibility_parent(ui.unique_id())
                .max_rect(rect),
        );

        let is_being_dragged_tile = Some(tile_id) == drop_context.dragged_tile_id;

        if is_being_dragged_tile && self.is_previewing() {
            self.tiles.insert(tile_id, tile);
            drop_context.enabled = drop_context_was_enabled;
        } else {
            ui.add_enabled_ui(enabled, |ui| {
                match &mut tile {
                    Tile::Pane(pane) => {
                        if behavior.pane_ui(ui, tile_id, pane) == UiResponse::DragStarted
                            && behavior.is_tile_draggable(&self.tiles, tile_id)
                        {
                            ui.set_dragged_id(tile_id.egui_id(self.id));
                        }
                    }
                    Tile::Container(container) => {
                        container.ui(self, behavior, drop_context, ui, rect, tile_id);
                    }
                }

                behavior.paint_on_top_of_tile(ui.painter(), ui.style(), tile_id, rect);

                self.tiles.insert(tile_id, tile);
                drop_context.enabled = drop_context_was_enabled;
            });
        }
    }

    /// Recursively "activate" the ancestors of the tiles that matches the given predicate.
    ///
    /// This means making the matching tiles and its ancestors the active tab in any tab layout.
    ///
    /// Returns `true` if a tab was made active.
    pub fn make_active(
        &mut self,
        mut should_activate: impl FnMut(TileId, &Tile<Pane>) -> bool,
    ) -> bool {
        if let Some(root) = self.root {
            self.tiles.make_active(root, &mut should_activate)
        } else {
            false
        }
    }

    fn preview_dragged_tile(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &DropContext,
        ui: &mut Ui,
        preview_options: &PreviewOptions,
    ) {
        let (Some(mouse_pos), Some(dragged_tile_id)) =
            (drop_context.mouse_pos, drop_context.dragged_tile_id)
        else {
            return;
        };

        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);

        // Preview what is being dragged:
        egui::Area::new(ui.scope_id().with((dragged_tile_id, "preview")))
            .pivot(egui::Align2::CENTER_CENTER)
            .current_pos(mouse_pos)
            .interactable(false)
            .show(ui, |ui| {
                behavior.drag_ui(&self.tiles, ui, dragged_tile_id);
            });

        // Highlight where the tile would land. With the animated preview that is the dragged
        // tile's own smoothed rect, so the highlight lines up with the layout sliding around
        // underneath it. Otherwise — preview disabled, or the very first frame of a drag,
        // before the speculative layout has had an insertion point to work from — smooth the
        // drop zone directly instead.
        let preview_rect = match self.smoothed_rect(dragged_tile_id) {
            Some(smoothed) => Some(smoothed),
            None => drop_context
                .preview_rect
                .map(|rect| smooth_preview_rect(ui, dragged_tile_id, rect, preview_options)),
        };

        if let Some(preview_rect) = preview_rect {
            let parent_rect = drop_context
                .best_insertion
                .and_then(|insertion_point| self.display_rect(insertion_point.parent_id));

            behavior.paint_drag_preview(ui.visuals(), ui.painter(), parent_rect, preview_rect);

            if behavior.preview_dragged_panes() {
                // TODO(emilk): add support for previewing containers too.
                if preview_rect.width() > 32.0
                    && preview_rect.height() > 32.0
                    && let Some(Tile::Pane(pane)) = self.tiles.get_mut(dragged_tile_id)
                {
                    // Intentionally ignore the response, since the user cannot possibly
                    // begin a drag on the preview pane.
                    let _ignored: UiResponse = behavior.pane_ui(
                        &mut ui.new_child(egui::UiBuilder::new().max_rect(preview_rect)),
                        dragged_tile_id,
                        pane,
                    );
                }
            }
        }

        if ui.input(|i| i.pointer.any_released()) {
            if let Some(insertion_point) = drop_context.best_insertion {
                behavior.on_edit(EditAction::TileDropped);
                self.move_tile(dragged_tile_id, insertion_point, false);
            }
            clear_smooth_preview_rect(ui, dragged_tile_id);
            self.preview = Preview::default();
        }
    }

    /// Simplify and normalize the tree using the given options.
    ///
    /// This is also called at the start of [`Self::ui`].
    pub fn simplify(&mut self, options: &SimplificationOptions) {
        if let Some(root) = self.root {
            match self.tiles.simplify(options, root, None) {
                SimplifyAction::Keep => {}
                SimplifyAction::Remove => {
                    self.root = None;
                }
                SimplifyAction::Replace(new_root) => {
                    self.root = Some(new_root);
                }
            }

            if options.all_panes_must_have_tabs
                && let Some(tile_id) = self.root
                && let Some(new_root) = self.tiles.make_all_panes_children_of_tabs(false, tile_id)
            {
                // The root was a bare pane, and is now wrapped in a tab container.
                self.root = Some(new_root);
            }
        }
    }

    /// Simplify all of the children of the given container tile recursively.
    pub fn simplify_children_of_tile(&mut self, tile_id: TileId, options: &SimplificationOptions) {
        if let Some(Tile::Container(mut container)) = self.tiles.remove(tile_id) {
            let kind = container.kind();
            container.simplify_children(|child| self.tiles.simplify(options, child, Some(kind)));
            self.tiles.insert(tile_id, Tile::Container(container));
        }
    }

    /// Garbage-collect tiles that are no longer reachable from the root tile.
    ///
    /// This is also called by [`Self::ui`], so usually you don't need to call this yourself.
    pub fn gc(&mut self, behavior: &mut dyn Behavior<Pane>) {
        if !self.tiles.gc_root(behavior, self.root) {
            self.root = None;
        }
    }

    /// Move a tile to a new container, at the specified insertion index.
    ///
    /// If the insertion index is greater than the current number of children, the tile is appended at the end.
    ///
    /// The grid layout needs a special treatment because it can have holes. When dragging a tile away from a grid, it
    /// leaves behind it a hole. As a result, if the tile is the dropped in the same grid, it there is no need to account
    /// for an insertion index shift (the hole can still occupy the original place of the dragged tile). However, if the
    /// tiles are reordered in a separate, linear representation of the grid (such as the Rerun blueprint tree), the
    /// expectation is that the grid is properly reordered and thus the insertion index must be shifted in case the tile
    /// is moved inside the same grid. The `reflow_grid` parameter controls this behavior.
    ///
    /// TL;DR:
    /// - when drag-and-dropping from a 2D representation of the grid, set `reflow_grid = false`
    /// - when drag-and-dropping from a 1D representation of the grid, set `reflow_grid = true`
    pub fn move_tile_to_container(
        &mut self,
        moved_tile_id: TileId,
        destination_container: TileId,
        mut insertion_index: usize,
        reflow_grid: bool,
    ) {
        // find target container
        if let Some(Tile::Container(target_container)) = self.tiles.get(destination_container) {
            let num_children = target_container.num_children();
            if insertion_index > num_children {
                insertion_index = num_children;
            }

            let container_insertion = match target_container.kind() {
                ContainerKind::Tabs => ContainerInsertion::Tabs(insertion_index),
                ContainerKind::Horizontal => ContainerInsertion::Horizontal(insertion_index),
                ContainerKind::Vertical => ContainerInsertion::Vertical(insertion_index),
                ContainerKind::Grid => ContainerInsertion::Grid(insertion_index),
            };

            self.move_tile(
                moved_tile_id,
                InsertionPoint {
                    parent_id: destination_container,
                    insertion: container_insertion,
                },
                reflow_grid,
            );
        } else {
            log::warn!(
                "Failed to find destination container {destination_container:?} during `move_tile_to_container()`"
            );
        }
    }

    /// Move the given tile to the given insertion point.
    ///
    /// See [`Self::move_tile_to_container()`] for details on `reflow_grid`.
    pub(super) fn move_tile(
        &mut self,
        moved_tile_id: TileId,
        insertion_point: InsertionPoint,
        reflow_grid: bool,
    ) {
        log::trace!(
            "Moving {moved_tile_id:?} into {:?}",
            insertion_point.insertion
        );

        if let Some((prev_parent_id, source_index)) = self.remove_tile_id_from_parent(moved_tile_id)
        {
            // Check to see if we are moving a tile within the same container:
            if prev_parent_id == insertion_point.parent_id {
                let parent_tile = self.tiles.get_mut(prev_parent_id);

                if let Some(Tile::Container(container)) = parent_tile
                    && container.kind() == insertion_point.insertion.kind()
                {
                    let dest_index = insertion_point.insertion.index();
                    log::trace!("Moving within the same parent: {source_index} -> {dest_index}");
                    // lets swap the two indices

                    let adjusted_index = if source_index < dest_index {
                        // We removed an earlier element, so we need to adjust the index:
                        dest_index - 1
                    } else {
                        dest_index
                    };

                    match container {
                        Container::Tabs(tabs) => {
                            let insertion_index = adjusted_index.min(tabs.children.len());
                            tabs.children.insert(insertion_index, moved_tile_id);
                            tabs.active = Some(moved_tile_id);
                        }
                        Container::Linear(linear) => {
                            let insertion_index = adjusted_index.min(linear.children.len());
                            linear.children.insert(insertion_index, moved_tile_id);
                        }
                        Container::Grid(grid) => {
                            if reflow_grid {
                                self.insert_at(insertion_point, moved_tile_id);
                            } else {
                                let dest_tile = grid.replace_at(dest_index, moved_tile_id);
                                if let Some(dest) = dest_tile {
                                    grid.insert_at(source_index, dest);
                                }
                            }
                        }
                    }
                    return; // done
                }
            }
        }

        // Moving to a new parent
        self.insert_at(insertion_point, moved_tile_id);
    }

    /// [`Tiles::insert_at`], plus the root fix-up that only the [`Tree`] can do.
    fn insert_at(&mut self, insertion_point: InsertionPoint, inserted_id: TileId) {
        if let Some(wrapper_id) = self.tiles.insert_at(insertion_point, inserted_id)
            && self.root == Some(insertion_point.parent_id)
        {
            // The root got wrapped in a new container, which is now the root.
            self.root = Some(wrapper_id);
        }
    }

    /// Find the currently dragged tile, if any.
    pub fn dragged_id(&self, ctx: &egui::Context) -> Option<TileId> {
        for tile_id in self.tiles.tile_ids() {
            if self.is_root(tile_id) {
                continue; // not allowed to drag root
            }

            let is_tile_being_dragged = crate::is_being_dragged(ctx, self.id, tile_id);
            if is_tile_being_dragged {
                // Abort drags on escape:
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ctx.stop_dragging();
                    return None;
                }

                return Some(tile_id);
            }
        }
        None
    }

    /// This removes the given tile from the parents list of children.
    ///
    /// The [`Tile`] itself is not removed from [`Self::tiles`].
    ///
    /// Performs no simplifications.
    ///
    /// If found, the parent tile and the child's index is returned.
    pub(super) fn remove_tile_id_from_parent(
        &mut self,
        remove_me: TileId,
    ) -> Option<(TileId, usize)> {
        let mut result = None;

        for (parent_id, parent) in self.tiles.iter_mut() {
            if let Tile::Container(container) = parent
                && let Some(child_index) = container.remove_child(remove_me)
            {
                result = Some((*parent_id, child_index));
            }
        }

        // Make sure that if we drag away the active some tabs,
        // that the tab container gets assigned another active tab.
        // If the tab is dragged to the same container, then it will become active again,
        // since all tabs become active when dragged, wherever they end up.
        if let Some((parent_id, _)) = result
            && let Some(mut tile) = self.tiles.remove(parent_id)
        {
            if let Tile::Container(Container::Tabs(tabs)) = &mut tile {
                tabs.ensure_active(&self.tiles);
            }
            self.tiles.insert(parent_id, tile);
        }

        result
    }

    /// Work out what the tree would look like if the dragged tile were dropped right now.
    ///
    /// The move, the simplification and the layout all run against a [`Tiles::skeleton`] — a copy
    /// of the tree's structure in which every pane is replaced by its own [`TileId`]. The real
    /// tree is only ever read, so no amount of trouble in here can lose a pane.
    ///
    /// Tiles keep their ids throughout, so the result is readable with the real tree's ids.
    fn speculate(
        &self,
        dragged_id: TileId,
        insertion: InsertionPoint,
        behavior: &dyn Behavior<Pane>,
        style: &egui::Style,
        rect: Rect,
    ) -> Tiles<TileId> {
        let simplification_options = behavior.simplification_options();

        let mut skeleton = Tree {
            id: self.id,
            root: self.root,
            tiles: self.tiles.skeleton(),
            width: self.width,
            height: self.height,
            preview: Preview::default(),
        };

        skeleton.move_tile(dragged_id, insertion, false);
        skeleton.simplify(&simplification_options);
        layout_tiles(&mut skeleton.tiles, skeleton.root, behavior, style, rect);

        // `Grid` collapses trailing holes at the _start_ of its layout pass, which can open up a
        // simplification that wasn't available before. A second round settles it.
        skeleton.simplify(&simplification_options);
        layout_tiles(&mut skeleton.tiles, skeleton.root, behavior, style, rect);

        // Speculating on a skeleton means panes cannot get lost. Assert it, so that a future tree
        // edit which drops, duplicates or relocates one is caught here rather than by a puzzled
        // user -- the latter would silently break every lookup below, which goes by id.
        debug_assert!(
            skeleton.tiles.iter().all(|(&id, tile)| match tile {
                Tile::Pane(real_id) => *real_id == id,
                Tile::Container(_) => true,
            }),
            "the speculative pass moved a pane to a different id"
        );
        debug_assert_eq!(
            sorted_skeleton_pane_ids(&skeleton.tiles),
            sorted_pane_ids(&self.tiles),
            "the speculative pass lost or duplicated a pane"
        );

        skeleton.tiles
    }

    /// Exponentially smooth every animated tile's rect towards where it should be.
    fn update_smoothed_rects(
        &mut self,
        ui: &Ui,
        dragged_id: Option<TileId>,
        options: &PreviewOptions,
    ) {
        let Self { tiles, preview, .. } = self;
        let Preview {
            memory,
            tiles: preview_tiles,
        } = preview;

        let no_targets = ahash::HashMap::default();
        let targets = preview_tiles
            .as_ref()
            .map_or(&no_targets, |preview| &preview.rects);

        if targets.is_empty() && memory.smoothed_rects.is_empty() {
            return;
        }

        let t = smoothing_factor(ui, options);

        // Start animating any tile that has a target but isn't animating yet:
        #[expect(clippy::iter_over_hash_type)] // Each tile animates independently.
        for (&tile_id, &target) in targets {
            // The dragged tile appears in its new home right away; the rest slide there.
            let start = if Some(tile_id) == dragged_id {
                Some(target)
            } else {
                tiles.rect(tile_id)
            };
            if let Some(start) = start {
                memory.smoothed_rects.entry(tile_id).or_insert(start);
            }
        }

        // Animate, and stop tracking whatever has arrived with nowhere further to go:
        let mut any_animating = false;
        memory.smoothed_rects.retain(|tile_id, smoothed| {
            let Some(target) = targets
                .get(tile_id)
                .copied()
                .or_else(|| tiles.rect(*tile_id))
            else {
                return false;
            };

            *smoothed = smoothed.lerp_towards(&target, t);

            if rects_close_enough(*smoothed, target) {
                *smoothed = target;
                targets.contains_key(tile_id) // still has somewhere to be
            } else {
                any_animating = true;
                true
            }
        });

        if any_animating {
            ui.ctx().request_repaint();
        }
    }

    /// Where a tile is drawn right now.
    ///
    /// During a drag preview this is part-way between where the tile is and where it would end
    /// up if the dragged tile were dropped. Use [`Tiles::rect`] instead for anything that must
    /// not feed back into the animation, such as hit-testing drop zones.
    pub(crate) fn display_rect(&self, tile_id: TileId) -> Option<Rect> {
        let rect = self.tiles.rect(tile_id)?;
        Some(self.smoothed_rect(tile_id).unwrap_or(rect))
    }

    /// Same as [`Self::display_rect`], but complains in debug builds if the tile has no rect.
    pub(crate) fn display_rect_or_die(&self, tile_id: TileId) -> Rect {
        let rect = self.tiles.rect_or_die(tile_id);
        self.smoothed_rect(tile_id).unwrap_or(rect)
    }

    fn smoothed_rect(&self, tile_id: TileId) -> Option<Rect> {
        self.preview.memory.smoothed_rects.get(&tile_id).copied()
    }

    /// Are any tiles currently animating for a drag preview?
    pub(crate) fn is_previewing(&self) -> bool {
        !self.preview.memory.smoothed_rects.is_empty()
    }

    /// How a [`Tabs`] container should be drawn mid-drag, if a drag is under way.
    pub(crate) fn preview_tabs(&self, tile_id: TileId) -> Option<&Tabs> {
        match self.preview.tiles.as_ref()?.get(tile_id)? {
            Tile::Container(Container::Tabs(tabs)) => Some(tabs),

            // The container was simplified away, or replaced by one of a different kind, in the
            // tree the drop would produce. Nothing sensible to preview.
            Tile::Container(_) | Tile::Pane(_) => None,
        }
    }
}

// ----------------------------------------------------------------------------

/// We store the preview rect in egui temp storage so that it is not serialized,
/// and so that a user could re-create the [`Tree`] each frame and still get smooth previews.
fn smooth_preview_rect_id(dragged_tile_id: TileId) -> egui::Id {
    egui::Id::unique((dragged_tile_id, "smoothed_preview_rect"))
}

fn clear_smooth_preview_rect(ctx: &egui::Context, dragged_tile_id: TileId) {
    let data_id = smooth_preview_rect_id(dragged_tile_id);
    ctx.data_mut(|data| data.remove::<Rect>(data_id));
}

/// Take the preview rectangle and smooth it over time.
fn smooth_preview_rect(
    ui: &Ui,
    dragged_tile_id: TileId,
    new_rect: Rect,
    options: &PreviewOptions,
) -> Rect {
    let data_id = smooth_preview_rect_id(dragged_tile_id);
    let t = smoothing_factor(ui, options);

    let mut requires_repaint = false;

    let smoothed = ui.data_mut(|data| {
        let smoothed: &mut Rect = data.get_temp_mut_or(data_id, new_rect);

        *smoothed = smoothed.lerp_towards(&new_rect, t);

        if rects_close_enough(*smoothed, new_rect) {
            *smoothed = new_rect;
        } else {
            requires_repaint = true;
        }
        *smoothed
    });

    if requires_repaint {
        ui.ctx().request_repaint();
    }

    smoothed
}

// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use egui::Pos2;

    use super::*;

    struct TestBehavior;

    impl Behavior<&'static str> for TestBehavior {
        fn pane_ui(
            &mut self,
            _ui: &mut Ui,
            _tile_id: TileId,
            _pane: &mut &'static str,
        ) -> UiResponse {
            UiResponse::None
        }

        fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
            (*pane).into()
        }
    }

    const TREE_ID: &str = "test_tree";

    /// Deterministic, so re-creating it yields the same [`TileId`]s.
    fn create_tree() -> (Tree<&'static str>, Vec<TileId>) {
        let mut tiles = Tiles::default();
        let panes: Vec<TileId> = ["a", "b", "c"]
            .into_iter()
            .map(|pane| tiles.insert_pane(pane))
            .collect();
        let root = tiles.insert_horizontal_tile(panes.clone());
        (Tree::new(egui::Id::unique(TREE_ID), root, tiles), panes)
    }

    /// Returns the actual (non-animated) rects the tiles ended up with.
    fn run_frame(
        ctx: &egui::Context,
        pointer: Pos2,
        dragged: Option<TileId>,
        pointer_down: bool,
    ) -> ahash::HashMap<TileId, Rect> {
        let mut rects = Default::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events: vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: pointer_down,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        };

        let mut full_output = ctx.run_ui(raw_input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                // Simulate an app (like Rerun) that re-creates the tree from
                // its own source of truth every single frame:
                let (mut tree, _) = create_tree();

                if let Some(dragged) = dragged {
                    ui.ctx().set_dragged_id(dragged.egui_id(tree.id));
                }

                tree.ui(&mut TestBehavior, ui);

                rects = tree.tiles.rects.clone();
            });
        });
        full_output.textures_delta.clear();

        rects
    }

    /// The drag preview needs state that outlives the frame.
    /// It must therefore survive an application re-creating its [`Tree`] every frame.
    #[test]
    fn preview_state_survives_tree_recreation() {
        let ctx = egui::Context::default();
        let tree_id = egui::Id::unique(TREE_ID);
        let (_, panes) = create_tree();
        let dragged = panes[0];

        // Warm-up frame, so the tiles have rects:
        let actual_rects = run_frame(&ctx, Pos2::new(50.0, 300.0), None, false);
        assert!(
            PreviewMemory::load(&ctx, tree_id).is_idle(),
            "no drag, so nothing to remember"
        );

        // Drag the first pane over the last one:
        let pointer = Pos2::new(800.0, 300.0);

        // First drag frame: the insertion point is only known at the _end_ of the frame,
        // so all we can do is remember it for the next frame.
        run_frame(&ctx, pointer, Some(dragged), true);
        let state = PreviewMemory::load(&ctx, tree_id);
        assert!(
            state.insertion.is_some(),
            "the insertion point should be remembered for the next frame"
        );
        assert!(
            state.smoothed_rects.is_empty(),
            "nothing to animate yet: the speculative layout has not run"
        );

        // Second drag frame: the remembered insertion point drives the speculative layout.
        run_frame(&ctx, pointer, Some(dragged), true);
        let state = PreviewMemory::load(&ctx, tree_id);
        assert!(
            !state.smoothed_rects.is_empty(),
            "the speculative layout should have produced rects to animate towards"
        );

        // The dragged pane is previewed somewhere other than where it currently is:
        let actual_rect = actual_rects[&dragged];
        let preview_rect = state.smoothed_rects.get(&dragged).copied();
        assert!(
            preview_rect.is_some_and(|preview| !rects_close_enough(preview, actual_rect)),
            "the dragged pane should be previewed in its new home, \
             but was previewed at {preview_rect:?} and actually is at {actual_rect:?}"
        );

        // Dropping the pane forgets the preview:
        run_frame(&ctx, pointer, Some(dragged), false);
        assert!(
            PreviewMemory::load(&ctx, tree_id).is_idle(),
            "the preview state should be forgotten once the tile is dropped"
        );
    }

    /// Speculating must never disturb the tree the user is looking at — not the panes it holds,
    /// and not its structure. Hovering a drag around is not an edit.
    #[test]
    fn hovering_a_drag_never_touches_the_real_tree() {
        for all_panes_must_have_tabs in [false, true] {
            let ctx = egui::Context::default();
            let (mut tree, panes) = create_tree();
            let dragged = panes[0];
            let before = tree.clone();

            let mut behavior = TabsBehavior {
                all_panes_must_have_tabs,
            };

            // Warm-up frame, so the tiles have rects:
            run_frame_with(
                &ctx,
                &mut tree,
                &mut behavior,
                Pos2::new(50.0, 300.0),
                None,
                false,
            );
            let settled = tree.clone();

            // Sweep the pointer over every region of every tile, so that every kind of
            // insertion point (tabs, horizontal, vertical, and onto panes vs. containers)
            // gets speculated on. Each position is held for several frames, because the
            // speculative layout always runs off the _previous_ frame's insertion point.
            let mut speculated = 0;
            for y in [20.0, 100.0, 300.0, 500.0, 580.0] {
                for x in [20.0, 200.0, 450.0, 700.0, 880.0] {
                    for _ in 0..3 {
                        run_frame_with(
                            &ctx,
                            &mut tree,
                            &mut behavior,
                            Pos2::new(x, y),
                            Some(dragged),
                            true,
                        );
                        // `Tree::ui` hands the surviving state back to egui, so read it there:
                        let memory = PreviewMemory::load(&ctx, tree.id);
                        speculated += usize::from(!memory.smoothed_rects.is_empty());

                        assert_eq!(
                            tree, settled,
                            "hovering a drag at ({x}, {y}) changed the tree \
                             (all_panes_must_have_tabs = {all_panes_must_have_tabs})"
                        );
                    }
                }
            }

            // Guard against the test passing simply because nothing ever happened:
            assert!(
                50 < speculated,
                "the sweep only speculated on {speculated} frames, so it proves little \
                 (all_panes_must_have_tabs = {all_panes_must_have_tabs})"
            );

            assert_eq!(
                pane_names(&before),
                pane_names(&tree),
                "no pane may be lost by hovering a drag \
                 (all_panes_must_have_tabs = {all_panes_must_have_tabs})"
            );
        }
    }

    /// A drag that is actually dropped *should* edit the tree — the preview must not eat the drop.
    #[test]
    fn dropping_a_drag_does_edit_the_real_tree() {
        let ctx = egui::Context::default();
        let (mut tree, panes) = create_tree();
        let dragged = panes[0];
        let mut behavior = TabsBehavior {
            all_panes_must_have_tabs: false,
        };

        run_frame_with(
            &ctx,
            &mut tree,
            &mut behavior,
            Pos2::new(50.0, 300.0),
            None,
            false,
        );
        let before = tree.clone();

        // Drag the leftmost pane onto the top edge of the rightmost one, then let go:
        let pointer = Pos2::new(800.0, 80.0);
        for _ in 0..3 {
            run_frame_with(&ctx, &mut tree, &mut behavior, pointer, Some(dragged), true);
        }
        run_frame_with(
            &ctx,
            &mut tree,
            &mut behavior,
            pointer,
            Some(dragged),
            false,
        );

        assert_ne!(before, tree, "the drop should have moved the pane");
        assert_eq!(
            pane_names(&before),
            pane_names(&tree),
            "the drop moved a pane, it must not have lost one"
        );
    }

    /// The panes a tree holds, by name and sorted — ids change when a tile is legitimately
    /// relocated, the set of panes must not.
    fn pane_names(tree: &Tree<&'static str>) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = tree
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                Tile::Pane(pane) => Some(*pane),
                Tile::Container(_) => None,
            })
            .collect();
        names.sort_unstable();
        names
    }

    /// While a tile is dragged over a tab bar, that tab bar should already show the incoming
    /// tab, selected. This exercises mapping the speculative tree's ids back onto the real
    /// ones: the containers involved get relocated during the speculative move.
    #[test]
    fn tab_bar_previews_the_incoming_tab() {
        let ctx = egui::Context::default();

        // `Tabs[a, b]` on the left, pane `c` on the right:
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let tabs = tiles.insert_tab_tile(vec![a, b]);
        let c = tiles.insert_pane("c");
        let root = tiles.insert_horizontal_tile(vec![tabs, c]);
        let mut tree = Tree::new(egui::Id::unique(TREE_ID), root, tiles);

        let mut behavior = TabRecorder::default();

        // Warm-up, so the tiles have rects:
        run_frame_with(
            &ctx,
            &mut tree,
            &mut behavior,
            Pos2::new(200.0, 300.0),
            None,
            false,
        );
        assert_eq!(
            behavior.take_drawn_tabs(),
            vec![(a, true), (b, false)],
            "sanity: the tab bar shows both of its tabs, with the first one active"
        );

        // Drag pane `c` onto that tab bar and hold it there. The speculative layout runs off
        // the previous frame's insertion point, so give it a few frames to settle, then look
        // only at the last one.
        let over_tab_bar = Pos2::new(200.0, 8.0);
        for _ in 0..3 {
            run_frame_with(&ctx, &mut tree, &mut behavior, over_tab_bar, Some(c), true);
        }
        behavior.take_drawn_tabs();
        run_frame_with(&ctx, &mut tree, &mut behavior, over_tab_bar, Some(c), true);

        let drawn = behavior.take_drawn_tabs();
        let drawn_ids: Vec<TileId> = drawn.iter().map(|&(tile_id, _)| tile_id).collect();
        assert!(
            drawn_ids.contains(&c),
            "the tab bar should preview the incoming tab, but only drew {drawn_ids:?}"
        );
        assert_eq!(
            drawn
                .iter()
                .filter(|&&(_, active)| active)
                .map(|&(tile_id, _)| tile_id)
                .collect::<Vec<_>>(),
            vec![c],
            "the incoming tab should be the one shown as selected"
        );

        // Nothing about the real tree may have changed yet:
        assert_eq!(
            tree.tiles.get_container(tabs).map(Container::children_vec),
            Some(vec![a, b])
        );

        // Letting go actually moves it in:
        run_frame_with(&ctx, &mut tree, &mut behavior, over_tab_bar, Some(c), false);
        let children = tree
            .tiles
            .get_container(tabs)
            .map(Container::children_vec)
            .expect("the tabs container should still be there");
        assert!(
            children.contains(&c),
            "dropping should have moved the pane into the tabs container, but it holds {children:?}"
        );
    }

    /// Dragging a tab out of a container that will collapse as a result should still show that
    /// container without the tab that is leaving — the remaining tabs close the gap.
    #[test]
    fn tab_bar_previews_a_tab_leaving() {
        let ctx = egui::Context::default();

        // `Tabs[a, b]` on the left, pane `c` on the right. Dragging `a` out leaves `Tabs[b]`,
        // which simplification collapses, so the tabs container is absent from the preview.
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let tabs = tiles.insert_tab_tile(vec![a, b]);
        let c = tiles.insert_pane("c");
        let root = tiles.insert_horizontal_tile(vec![tabs, c]);
        let mut tree = Tree::new(egui::Id::unique(TREE_ID), root, tiles);

        let mut behavior = TabRecorder::default();

        run_frame_with(
            &ctx,
            &mut tree,
            &mut behavior,
            Pos2::new(200.0, 300.0),
            None,
            false,
        );
        assert_eq!(
            behavior.take_drawn_tabs(),
            vec![(a, true), (b, false)],
            "sanity: the tab bar shows both tabs"
        );

        // Drag `a` over pane `c`, and hold it there:
        let over_c = Pos2::new(700.0, 300.0);
        for _ in 0..4 {
            run_frame_with(&ctx, &mut tree, &mut behavior, over_c, Some(a), true);
        }
        behavior.take_drawn_tabs();
        run_frame_with(&ctx, &mut tree, &mut behavior, over_c, Some(a), true);

        let drawn: Vec<TileId> = behavior
            .take_drawn_tabs()
            .into_iter()
            .map(|(tile_id, _)| tile_id)
            .collect();
        assert_eq!(
            drawn,
            vec![b],
            "the tab bar should already show itself without the tab being dragged out"
        );
    }

    /// A [`Behavior`] that records which tabs the tab bar drew, and which looked selected.
    #[derive(Default)]
    struct TabRecorder {
        drawn_tabs: Vec<(TileId, bool)>,
    }

    impl TabRecorder {
        fn take_drawn_tabs(&mut self) -> Vec<(TileId, bool)> {
            std::mem::take(&mut self.drawn_tabs)
        }
    }

    impl Behavior<&'static str> for TabRecorder {
        fn pane_ui(
            &mut self,
            _ui: &mut Ui,
            _tile_id: TileId,
            _pane: &mut &'static str,
        ) -> UiResponse {
            UiResponse::None
        }

        fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
            (*pane).into()
        }

        fn tab_ui(
            &mut self,
            _tiles: &mut Tiles<&'static str>,
            ui: &mut Ui,
            id: egui::Id,
            tile_id: TileId,
            state: &crate::TabState,
        ) -> egui::Response {
            self.drawn_tabs.push((tile_id, state.active));
            let (_, rect) = ui.allocate_space(egui::vec2(64.0, ui.available_height()));
            ui.interact(rect, id, egui::Sense::click_and_drag())
        }
    }

    struct TabsBehavior {
        all_panes_must_have_tabs: bool,
    }

    impl Behavior<&'static str> for TabsBehavior {
        fn pane_ui(
            &mut self,
            _ui: &mut Ui,
            _tile_id: TileId,
            _pane: &mut &'static str,
        ) -> UiResponse {
            UiResponse::None
        }

        fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
            (*pane).into()
        }

        fn simplification_options(&self) -> SimplificationOptions {
            SimplificationOptions {
                all_panes_must_have_tabs: self.all_panes_must_have_tabs,
                ..Default::default()
            }
        }
    }

    /// Run one frame against an existing tree, i.e. an app that keeps its [`Tree`] around.
    fn run_frame_with(
        ctx: &egui::Context,
        tree: &mut Tree<&'static str>,
        behavior: &mut dyn Behavior<&'static str>,
        pointer: Pos2,
        dragged: Option<TileId>,
        pointer_down: bool,
    ) {
        let mut full_output = ctx.run_ui(raw_input(pointer, pointer_down), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(dragged) = dragged {
                    ui.ctx().set_dragged_id(dragged.egui_id(tree.id));
                }
                tree.ui(behavior, ui);
            });
        });
        full_output.textures_delta.clear();
    }

    fn raw_input(pointer: Pos2, pointer_down: bool) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events: vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: pointer_down,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        }
    }

    /// Wrapping the root leaves the wrapped tile with its own id, so the _new_ container has to
    /// become the root. Only the [`Tree`] knows what the root is, so [`Tiles`] hands it back.
    #[test]
    fn wrapping_the_root_makes_the_new_container_the_root() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let root = tiles.insert_horizontal_tile(vec![a, b]);
        let dropped = tiles.insert_pane("dropped");
        let mut tree = Tree::new(egui::Id::unique("test"), root, tiles);

        // A vertical insertion into a horizontal container has to wrap it:
        tree.move_tile(
            dropped,
            InsertionPoint {
                parent_id: root,
                insertion: ContainerInsertion::Vertical(1),
            },
            false,
        );

        let new_root = tree.root.expect("the tree should still have a root");
        assert_ne!(
            new_root, root,
            "the wrapping container should be the new root"
        );
        assert_eq!(
            tree.tiles
                .get_container(root)
                .expect("the old root should still be a container under its own id")
                .children_vec(),
            vec![a, b],
            "the wrapped container should be untouched"
        );
        assert_eq!(
            tree.tiles
                .get_container(new_root)
                .expect("the new root should be a container")
                .children_vec(),
            vec![root, dropped],
        );
        assert_eq!(tree.tiles.parent_of(root), Some(new_root));
    }

    /// Drops one specific pane, the way an application closes a document.
    struct DropPane(&'static str);

    impl Behavior<&'static str> for DropPane {
        fn pane_ui(
            &mut self,
            _ui: &mut egui::Ui,
            _tile_id: TileId,
            _pane: &mut &'static str,
        ) -> UiResponse {
            UiResponse::None
        }

        fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
            (*pane).into()
        }

        fn retain_pane(&mut self, pane: &&'static str) -> bool {
            *pane != self.0
        }
    }

    /// Keeps every pane; `gc` needs a [`Behavior`], these tests are not about `retain_pane`.
    struct KeepEverything;

    impl Behavior<&'static str> for KeepEverything {
        fn pane_ui(
            &mut self,
            _ui: &mut egui::Ui,
            _tile_id: TileId,
            _pane: &mut &'static str,
        ) -> UiResponse {
            UiResponse::None
        }

        fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
            (*pane).into()
        }
    }

    /// A root the collector had to drop must stop being the root.
    ///
    /// A single pane can be the root - `simplify` produces exactly that once a tab container is
    /// down to one child - and `retain_pane` is how an application closes it. `gc` then removed
    /// the tile and left `root` naming it, so the tree rendered nothing while
    /// `is_empty()` still said `false`: no way for the caller to notice, and no way to recover
    /// except by writing `tree.root = None` by hand.
    #[test]
    fn gc_clears_a_root_it_had_to_drop() {
        let mut tiles = Tiles::default();
        let only = tiles.insert_pane("doomed");
        let mut tree = Tree::new(egui::Id::unique("root_pane"), only, tiles);

        tree.gc(&mut DropPane("doomed"));

        assert_eq!(
            tree.root, None,
            "the root tile is gone, so the tree must not still name it"
        );
        assert!(tree.is_empty(), "an empty tree has to say that it is empty");
    }

    /// A tile named as a child by two containers must lose the second *reference*, not itself.
    ///
    /// `gc` takes a tile out of the arena on the way in and puts it back on the way out, but the
    /// early return for a tile it has already visited skips the putting back — so the tile is
    /// deleted, while the container that legitimately owns it keeps naming it. `simplify` then
    /// prunes that container as empty, and its parent, and the whole tree can disappear.
    #[test]
    fn gc_drops_the_second_reference_to_a_shared_tile_and_not_the_tile() {
        let mut tiles = Tiles::default();
        let pane = tiles.insert_pane("shared");
        let first = tiles.insert_tab_tile(vec![pane]);
        let second = tiles.insert_tab_tile(vec![pane]); // the same tile, a second parent
        let root = tiles.insert_horizontal_tile(vec![first, second]);
        let mut tree = Tree::new(egui::Id::unique("shared"), root, tiles);

        tree.gc(&mut KeepEverything);

        assert!(
            tree.tiles.get(pane).is_some(),
            "the shared tile itself must survive - only one of the two references is bogus"
        );
        assert_eq!(
            tree.tiles
                .get_container(first)
                .expect("the first parent should still be a container")
                .children_vec(),
            vec![pane],
            "the first parent to reach the tile keeps it"
        );
        assert!(
            tree.tiles
                .get_container(second)
                .expect("the second parent should still be a container")
                .children_vec()
                .is_empty(),
            "the second reference is the one that has to go"
        );

        // And the point of it all: the rest of the frame must not unravel the tree.
        tree.simplify(&SimplificationOptions::default());
        assert!(
            tree.root.is_some(),
            "the tree must still have a root after gc+simplify"
        );
        assert!(
            tree.tiles.get(pane).is_some(),
            "the pane must still be in the tree after gc+simplify"
        );
    }
}
