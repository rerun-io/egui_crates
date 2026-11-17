use egui::{
    NumExt as _, Rect, ResizeState,
    emath::{GuiRounding as _, Rangef},
    pos2, vec2,
};
use itertools::Itertools as _;

use crate::behavior::{EditAction, LayoutContext};
use crate::{
    Behavior, ContainerInsertion, DropContext, InsertionPoint, SimplifyAction, TileId, Tiles, Tree,
};

/// How to lay out the children of a grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum GridLayout {
    /// Place children in a grid, with a dynamic number of columns and rows.
    /// Resizing the window may change the number of columns and rows.
    #[default]
    Auto,

    /// Place children in a grid with this many columns,
    /// and as many rows as needed.
    Columns(usize),
}

/// A grid of tiles.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Grid {
    /// The order of the children, row-major.
    ///
    /// We allow holes (for easier drag-dropping).
    /// We collapse all holes if they become too numerous.
    children: Vec<Option<TileId>>,

    /// Determines the number of columns.
    pub layout: GridLayout,

    /// Share of the available width assigned to each column.
    pub col_shares: Vec<f32>,

    /// Share of the available height assigned to each row.
    pub row_shares: Vec<f32>,

    /// ui point x ranges for each column, recomputed during layout
    #[cfg_attr(feature = "serde", serde(skip))]
    col_ranges: Vec<Rangef>,

    /// ui point y ranges for each row, recomputed during layout
    #[cfg_attr(feature = "serde", serde(skip))]
    row_ranges: Vec<Rangef>,
}

impl PartialEq for Grid {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            children,
            layout,
            col_shares,
            row_shares,
            col_ranges: _, // ignored because they are recomputed each frame
            row_ranges: _, // ignored because they are recomputed each frame
        } = self;

        layout == &other.layout
            && children == &other.children
            && col_shares == &other.col_shares
            && row_shares == &other.row_shares
    }
}

impl Grid {
    pub fn new(children: Vec<TileId>) -> Self {
        Self {
            children: children.into_iter().map(Some).collect(),
            ..Default::default()
        }
    }

    pub fn num_children(&self) -> usize {
        self.children().count()
    }

    /// Includes invisible children.
    pub fn children(&self) -> impl Iterator<Item = &TileId> {
        self.children.iter().filter_map(|c| c.as_ref())
    }

    pub fn add_child(&mut self, child: TileId) {
        self.children.push(Some(child));
    }

    pub fn insert_at(&mut self, index: usize, child: TileId) {
        if let Some(slot) = self.children.get_mut(index) {
            if slot.is_none() {
                // put it in the empty hole
                slot.replace(child);
            } else {
                // put it before
                log::trace!("Inserting {child:?} into Grid at {index}");
                self.children.insert(index, Some(child));
            }
        } else {
            // put it last
            log::trace!("Pushing {child:?} last in Grid");
            self.children.push(Some(child));
        }
    }

    /// Returns the child already at the given index, if any.
    #[must_use]
    pub fn replace_at(&mut self, index: usize, child: TileId) -> Option<TileId> {
        if let Some(slot) = self.children.get_mut(index) {
            slot.replace(child)
        } else {
            // put it last
            self.children.push(Some(child));
            None
        }
    }

    /// Swap out one child for another, keeping its cell.
    ///
    /// The column and row shares are positional, so they need no fixing up.
    ///
    /// Returns the index of the cell that was swapped,
    /// or `None` if `old` was not a child of this grid.
    #[must_use]
    pub(super) fn replace_child(&mut self, old: TileId, new: TileId) -> Option<usize> {
        let index = self.children.iter().position(|child| *child == Some(old))?;
        self.children[index] = Some(new);
        Some(index)
    }

    fn collapse_holes(&mut self) {
        log::trace!("Collaping grid holes");
        self.children.retain(|child| child.is_some());
    }

    fn visible_children_and_holes<Pane>(&self, tiles: &Tiles<Pane>) -> Vec<Option<TileId>> {
        self.children
            .iter()
            .filter(|id| id.is_none_or(|id| tiles.is_visible_in_layout(id)))
            .copied()
            .collect()
    }

    pub(super) fn layout<Pane>(
        &mut self,
        tiles: &mut Tiles<Pane>,
        layout: &LayoutContext<'_>,
        rect: Rect,
    ) {
        // clean up any empty holes at the end
        while self.children.last() == Some(&None) {
            self.children.pop();
        }

        let gap = layout.gap_width;

        let visible_children_and_holes = self.visible_children_and_holes(tiles);

        // Calculate grid dimensions:
        let (num_cols, num_rows) = {
            let num_visible_children = visible_children_and_holes.len();

            let num_cols = match self.layout {
                GridLayout::Auto => {
                    (layout.grid_auto_column_count)(num_visible_children, rect, gap)
                }
                GridLayout::Columns(num_columns) => num_columns,
            };
            let num_cols = num_cols.at_least(1);
            let num_rows = num_visible_children.div_ceil(num_cols);
            (num_cols, num_rows)
        };

        debug_assert!(
            visible_children_and_holes.len() <= num_cols * num_rows,
            "Bug in egui_tiles::Grid::layout"
        );

        // Figure out where each column and row goes:
        self.col_shares.resize(num_cols, 1.0);
        self.row_shares.resize(num_rows, 1.0);

        let col_widths = sizes_from_shares(&self.col_shares, rect.width(), gap);
        let row_heights = sizes_from_shares(&self.row_shares, rect.height(), gap);

        debug_assert_eq!(
            col_widths.len(),
            num_cols,
            "Bug in egui_tiles::Grid::layout"
        );
        debug_assert_eq!(
            row_heights.len(),
            num_rows,
            "Bug in egui_tiles::Grid::layout"
        );

        {
            let mut x = rect.left();
            self.col_ranges.clear();
            for &width in &col_widths {
                self.col_ranges.push(Rangef::new(x, x + width));
                x += width + gap;
            }
        }
        {
            let mut y = rect.top();
            self.row_ranges.clear();
            for &height in &row_heights {
                self.row_ranges.push(Rangef::new(y, y + height));
                y += height + gap;
            }
        }

        debug_assert_eq!(
            self.col_ranges.len(),
            num_cols,
            "Bug in egui_tiles::Grid::layout"
        );
        debug_assert_eq!(
            self.row_ranges.len(),
            num_rows,
            "Bug in egui_tiles::Grid::layout"
        );

        // Layout each child:
        for (i, &child) in visible_children_and_holes.iter().enumerate() {
            if let Some(child) = child {
                let col = i % num_cols;
                let row = i / num_cols;
                let child_rect = Rect::from_x_y_ranges(self.col_ranges[col], self.row_ranges[row]);
                tiles.layout_tile(layout, child_rect, child);
            }
        }

        // Check if we should collapse some holes:
        {
            let num_holes = visible_children_and_holes
                .iter()
                .filter(|c| c.is_none())
                .count()
                + (num_cols * num_rows - visible_children_and_holes.len());

            if num_cols.min(num_rows) <= num_holes {
                // More holes than there are columns or rows - let's collapse all holes
                // so that we can shrink for next frame:
                self.collapse_holes();
            }
        }
    }

    pub(super) fn ui<Pane>(
        &mut self,
        tree: &mut Tree<Pane>,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &mut DropContext,
        ui: &egui::Ui,
        tile_id: TileId,
    ) {
        for &child in &self.children {
            if let Some(child) = child
                && tree.is_visible_in_layout(child)
            {
                tree.tile_ui(behavior, drop_context, ui, child);
                crate::cover_tile_if_dragged(tree, behavior, ui, child);
            }
        }

        // Register drop-zones:
        for i in 0..(self.col_ranges.len() * self.row_ranges.len()) {
            let col = i % self.col_ranges.len();
            let row = i / self.col_ranges.len();
            let child_rect = Rect::from_x_y_ranges(self.col_ranges[col], self.row_ranges[row]);
            drop_context.suggest_rect(
                InsertionPoint::new(tile_id, ContainerInsertion::Grid(i)),
                child_rect,
            );
        }

        self.resize_columns(&tree.tiles, behavior, ui, tile_id);
        self.resize_rows(&tree.tiles, behavior, ui, tile_id);
    }

    fn resize_columns<Pane>(
        &mut self,
        tiles: &Tiles<Pane>,
        behavior: &mut dyn Behavior<Pane>,
        ui: &egui::Ui,
        parent_id: TileId,
    ) {
        let resizable = behavior.is_container_resizable(tiles, parent_id);

        let parent_rect = tiles.rect_or_die(parent_id);
        for (i, (left, right)) in self.col_ranges.iter().copied().tuple_windows().enumerate() {
            let resize_id = ui.scope_id().with((parent_id, "resize_col", i));

            let x = egui::lerp(left.max..=right.min, 0.5);

            if !resizable {
                let stroke = behavior.resize_stroke(ui, resize_id, ResizeState::Idle);
                ui.painter().vline(x, parent_rect.y_range(), stroke);
                continue;
            }

            let mut resize_state = ResizeState::Idle;
            let line_rect = Rect::from_center_size(
                pos2(x, parent_rect.center().y),
                vec2(
                    2.0 * ui.style().interaction.resize_grab_radius_side,
                    parent_rect.height(),
                ),
            );
            let response = ui.interact(line_rect, resize_id, egui::Sense::click_and_drag());
            // NOTE: Check for interaction with line_rect BEFORE entering the 'IF block' below,
            // otherwise we miss the start of a drag event in certain cases (e.g. touchscreens).
            if let Some(pointer) = ui.pointer_interact_pos() {
                resize_state = resize_interaction(
                    behavior,
                    &self.col_ranges,
                    &mut self.col_shares,
                    &response,
                    pointer.round_to_pixels(ui.pixels_per_point()).x - x,
                    i,
                );

                if resize_state != ResizeState::Idle {
                    ui.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }

            let stroke = behavior.resize_stroke(ui, resize_id, resize_state);
            ui.painter().vline(x, parent_rect.y_range(), stroke);
        }
    }

    fn resize_rows<Pane>(
        &mut self,
        tiles: &Tiles<Pane>,
        behavior: &mut dyn Behavior<Pane>,
        ui: &egui::Ui,
        parent_id: TileId,
    ) {
        let resizable = behavior.is_container_resizable(tiles, parent_id);

        let parent_rect = tiles.rect_or_die(parent_id);
        for (i, (top, bottom)) in self.row_ranges.iter().copied().tuple_windows().enumerate() {
            let resize_id = ui.scope_id().with((parent_id, "resize_row", i));

            let y = egui::lerp(top.max..=bottom.min, 0.5);

            if !resizable {
                let stroke = behavior.resize_stroke(ui, resize_id, ResizeState::Idle);
                ui.painter().hline(parent_rect.x_range(), y, stroke);
                continue;
            }

            let mut resize_state = ResizeState::Idle;
            let line_rect = Rect::from_center_size(
                pos2(parent_rect.center().x, y),
                vec2(
                    parent_rect.width(),
                    2.0 * ui.style().interaction.resize_grab_radius_side,
                ),
            );
            let response = ui.interact(line_rect, resize_id, egui::Sense::click_and_drag());
            // NOTE: Check for interaction with line_rect BEFORE entering the 'IF block' below,
            // otherwise we miss the start of a drag event in certain cases (e.g. touchscreens).
            if let Some(pointer) = ui.pointer_interact_pos() {
                resize_state = resize_interaction(
                    behavior,
                    &self.row_ranges,
                    &mut self.row_shares,
                    &response,
                    pointer.round_to_pixels(ui.pixels_per_point()).y - y,
                    i,
                );

                if resize_state != ResizeState::Idle {
                    ui.set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
            }

            let stroke = behavior.resize_stroke(ui, resize_id, resize_state);
            ui.painter().hline(parent_rect.x_range(), y, stroke);
        }
    }

    pub(super) fn simplify_children(&mut self, mut simplify: impl FnMut(TileId) -> SimplifyAction) {
        for child_opt in &mut self.children {
            if let Some(child) = *child_opt {
                match simplify(child) {
                    SimplifyAction::Remove => {
                        *child_opt = None;
                    }
                    SimplifyAction::Keep => {}
                    SimplifyAction::Replace(new) => {
                        *child_opt = Some(new);
                    }
                }
            }
        }
    }

    pub(super) fn retain(&mut self, mut retain: impl FnMut(TileId) -> bool) {
        for child_opt in &mut self.children {
            if let Some(child) = *child_opt
                && !retain(child)
            {
                *child_opt = None;
            }
        }
    }

    /// Returns child index, if found.
    pub(crate) fn remove_child(&mut self, needle: TileId) -> Option<usize> {
        let index = self
            .children
            .iter()
            .position(|&child| child == Some(needle))?;
        self.children[index] = None;
        Some(index)
    }
}

fn resize_interaction<Pane>(
    behavior: &mut dyn Behavior<Pane>,
    ranges: &[Rangef],
    shares: &mut [f32],
    splitter_response: &egui::Response,
    dx: f32,
    i: usize,
) -> ResizeState {
    assert_eq!(ranges.len(), shares.len(), "Bug in egui_tiles::Grid");
    let num = ranges.len();
    let tile_width = |i: usize| ranges[i].span();

    let left = i;
    let right = i + 1;

    if splitter_response.double_clicked() {
        behavior.on_edit(EditAction::TileResized);

        // double-click to center the split between left and right:
        let mean = 0.5 * (shares[left] + shares[right]);
        shares[left] = mean;
        shares[right] = mean;
        ResizeState::Hovered
    } else if splitter_response.dragged() {
        behavior.on_edit(EditAction::TileResized);

        if dx < 0.0 {
            // Expand right, shrink stuff to the left:
            shares[right] += shrink_shares(
                behavior,
                shares,
                &(0..=i).rev().collect_vec(),
                dx.abs(),
                tile_width,
            );
        } else {
            // Expand the left, shrink stuff to the right:
            shares[left] += shrink_shares(
                behavior,
                shares,
                &(i + 1..num).collect_vec(),
                dx.abs(),
                tile_width,
            );
        }
        ResizeState::Dragged
    } else if splitter_response.hovered() {
        ResizeState::Hovered
    } else {
        ResizeState::Idle
    }
}

/// Try shrink the children by a total of `target_in_points`,
/// making sure no child gets smaller than its minimum size.
fn shrink_shares<Pane>(
    behavior: &dyn Behavior<Pane>,
    shares: &mut [f32],
    children: &[usize],
    target_in_points: f32,
    size_in_point: impl Fn(usize) -> f32,
) -> f32 {
    if children.is_empty() {
        return 0.0;
    }

    let mut total_shares = 0.0;
    let mut total_points = 0.0;
    for &child in children {
        total_shares += shares[child];
        total_points += size_in_point(child);
    }

    let shares_per_point = total_shares / total_points;

    let min_size_in_shares = shares_per_point * behavior.min_size();

    let target_in_shares = shares_per_point * target_in_points;
    let mut total_shares_lost = 0.0;

    for &child in children {
        let share = &mut shares[child];
        let spare_share = (*share - min_size_in_shares).at_least(0.0);
        let shares_needed = (target_in_shares - total_shares_lost).at_least(0.0);
        let shrink_by = f32::min(spare_share, shares_needed);

        *share -= shrink_by;
        total_shares_lost += shrink_by;
    }

    total_shares_lost
}

fn sizes_from_shares(shares: &[f32], available_size: f32, gap_width: f32) -> Vec<f32> {
    if shares.is_empty() {
        return vec![];
    }

    let available_size = available_size - gap_width * (shares.len() - 1) as f32;
    let available_size = available_size.at_least(0.0);

    let total_share: f32 = shares.iter().sum();
    if total_share <= 0.0 {
        vec![available_size / shares.len() as f32; shares.len()]
    } else {
        shares
            .iter()
            .map(|&share| share / total_share * available_size)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Container, Tile};

    use super::*;

    #[test]
    fn test_grid_with_chaos_monkey() {
        #[derive(Debug)]
        struct Pane {}

        struct TestBehavior {}

        impl Behavior<Pane> for TestBehavior {
            fn pane_ui(
                &mut self,
                _ui: &mut egui::Ui,
                _tile_id: TileId,
                _pane: &mut Pane,
            ) -> crate::UiResponse {
                panic!()
            }

            fn tab_title_for_pane(&mut self, _pane: &Pane) -> egui::WidgetText {
                panic!()
            }

            fn is_tab_closable(&self, _tiles: &Tiles<Pane>, _tile_id: TileId) -> bool {
                panic!()
            }

            fn on_tab_close(&mut self, _tiles: &mut Tiles<Pane>, _tile_id: TileId) -> bool {
                panic!()
            }
        }

        let mut tree = {
            let mut tiles = Tiles::default();
            let panes: Vec<TileId> = vec![tiles.insert_pane(Pane {}), tiles.insert_pane(Pane {})];
            let root: TileId = tiles.insert_grid_tile(panes);
            Tree::new(egui::Id::unique("test_tree"), root, tiles)
        };

        let style = egui::Style::default();
        let behavior = TestBehavior {};
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1024.0, 768.0));

        // Go crazy on it to make sure we never crash:
        let mut rng = Pcg64::new_seed(123_456_789_012);

        for _ in 0..1000 {
            let root = tree.root.unwrap();
            crate::behavior::layout_tiles(&mut tree.tiles, Some(root), &behavior, &style, area);

            // Add some tiles:
            for _ in 0..rng.rand_u64() % 3 {
                if tree.tiles.len() < 100 {
                    let pane = tree.tiles.insert_pane(Pane {});
                    if let Some(Tile::Container(Container::Grid(grid))) = tree.tiles.get_mut(root) {
                        grid.add_child(pane);
                    } else {
                        panic!()
                    }
                }
            }

            // Move a random child to then end of the grid:
            for _ in 0..rng.rand_u64() % 2 {
                if let Some(Tile::Container(Container::Grid(grid))) = tree.tiles.get_mut(root) {
                    if !grid.children.is_empty() {
                        let child_idx = rng.rand_usize() % grid.children.len();
                        let child = grid.children[child_idx].take();
                        grid.children.push(child);
                    }
                } else {
                    panic!()
                }
            }

            // Flip some visibilities:
            for _ in 0..rng.rand_u64() % 2 {
                let children =
                    if let Some(Tile::Container(Container::Grid(grid))) = tree.tiles.get(root) {
                        grid.visible_children_and_holes(&tree.tiles)
                            .iter()
                            .copied()
                            .flatten()
                            .collect_vec()
                    } else {
                        panic!()
                    };

                if !children.is_empty() {
                    let child_idx = rng.rand_usize() % children.len();
                    tree.tiles.toggle_visibility(children[child_idx]);
                }
            }

            // Remove some tiles:
            for _ in 0..rng.rand_u64() % 2 {
                let children =
                    if let Some(Tile::Container(Container::Grid(grid))) = tree.tiles.get(root) {
                        grid.visible_children_and_holes(&tree.tiles)
                            .iter()
                            .copied()
                            .flatten()
                            .collect_vec()
                    } else {
                        panic!()
                    };

                if !children.is_empty() {
                    let child_id = children[rng.rand_usize() % children.len()];
                    let (parent, _) = tree.remove_tile_id_from_parent(child_id).unwrap();
                    assert_eq!(parent, root);
                    tree.tiles.remove(child_id).unwrap();
                }
            }
        }
    }

    // We want a simple RNG, but don't want to pull in any deps just for a test.
    // Code from adapted from https://docs.rs/nanorand/latest/src/nanorand/rand/pcg64.rs.html#15-19
    pub struct Pcg64 {
        seed: u128,
        state: u128,
        inc: u128,
    }

    impl Pcg64 {
        pub const fn new_seed(seed: u128) -> Self {
            Self {
                seed,
                inc: 0,
                state: 0,
            }
        }

        fn step(&mut self) {
            const PCG_DEFAULT_MULTIPLIER_128: u128 = 47026247687942121848144207491837523525;

            self.state = self
                .state
                .wrapping_mul(PCG_DEFAULT_MULTIPLIER_128)
                .wrapping_add(self.inc);
        }

        #[expect(
            clippy::cast_possible_truncation,
            reason = "Truncation is intentional in a PRNG"
        )]
        fn rand_u64(&mut self) -> u64 {
            self.state = 0;
            self.inc = self.seed.wrapping_shl(1) | 1;
            self.step();
            self.state = self.state.wrapping_add(self.seed);
            self.step();
            self.step();
            self.state.wrapping_shr(64) as u64 ^ self.state as u64
        }

        #[expect(
            clippy::cast_possible_truncation,
            reason = "Truncation is intentional in a PRNG"
        )]
        fn rand_usize(&mut self) -> usize {
            self.rand_u64() as usize
        }
    }
}
