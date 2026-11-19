use egui::Rect;

use crate::Tree;

use super::{Behavior, DropContext, SimplifyAction, TileId, Tiles};
use crate::behavior::LayoutContext;

mod grid;
mod linear;
mod tabs;

pub use grid::{Grid, GridLayout};
pub use linear::{Linear, LinearDir, Shares};
pub use tabs::Tabs;

// ----------------------------------------------------------------------------

/// The layout type of a [`Container`].
///
/// This is used to describe a [`Container`], and to change it to a different layout type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ContainerKind {
    /// Each child in an individual tab.
    #[default]
    Tabs,

    /// Left-to-right
    Horizontal,

    /// Top-down
    Vertical,

    /// In a grid, laied out row-wise, left-to-right, top-down.
    Grid,
}

impl ContainerKind {
    pub const ALL: [Self; 4] = [Self::Tabs, Self::Horizontal, Self::Vertical, Self::Grid];
}

// ----------------------------------------------------------------------------

/// A container of several [`super::Tile`]s.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Container {
    Tabs(Tabs),
    Linear(Linear),
    Grid(Grid),
}

impl From<Tabs> for Container {
    #[inline]
    fn from(tabs: Tabs) -> Self {
        Self::Tabs(tabs)
    }
}

impl From<Linear> for Container {
    #[inline]
    fn from(linear: Linear) -> Self {
        Self::Linear(linear)
    }
}

impl From<Grid> for Container {
    #[inline]
    fn from(grid: Grid) -> Self {
        Self::Grid(grid)
    }
}

impl Container {
    pub fn new(kind: ContainerKind, children: Vec<TileId>) -> Self {
        match kind {
            ContainerKind::Tabs => Self::new_tabs(children),
            ContainerKind::Horizontal => Self::new_horizontal(children),
            ContainerKind::Vertical => Self::new_vertical(children),
            ContainerKind::Grid => Self::new_grid(children),
        }
    }

    pub fn new_linear(dir: LinearDir, children: Vec<TileId>) -> Self {
        Self::Linear(Linear::new(dir, children))
    }

    pub fn new_horizontal(children: Vec<TileId>) -> Self {
        Self::new_linear(LinearDir::Horizontal, children)
    }

    pub fn new_vertical(children: Vec<TileId>) -> Self {
        Self::new_linear(LinearDir::Vertical, children)
    }

    pub fn new_tabs(children: Vec<TileId>) -> Self {
        Self::Tabs(Tabs::new(children))
    }

    pub fn new_grid(children: Vec<TileId>) -> Self {
        Self::Grid(Grid::new(children))
    }

    pub fn is_empty(&self) -> bool {
        self.num_children() == 0
    }

    pub fn num_children(&self) -> usize {
        match self {
            Self::Tabs(tabs) => tabs.children.len(),
            Self::Linear(linear) => linear.children.len(),
            Self::Grid(grid) => grid.num_children(),
        }
    }

    /// All the childrens of this container.
    pub fn children(&self) -> impl Iterator<Item = &TileId> {
        match self {
            Self::Tabs(tabs) => itertools::Either::Left(tabs.children.iter()),
            Self::Linear(linear) => itertools::Either::Left(linear.children.iter()),
            Self::Grid(grid) => itertools::Either::Right(grid.children()),
        }
    }

    /// All the active childrens of this container.
    ///
    /// For tabs, this is just the active tab.
    /// For other containers, it is all children.
    pub fn active_children<Pane>(&self, tiles: &Tiles<Pane>) -> impl Iterator<Item = TileId> {
        match self {
            Self::Tabs(tabs) => itertools::Either::Left(itertools::Either::Left(
                tabs.next_active(tiles).into_iter(),
            )),
            Self::Linear(linear) => {
                itertools::Either::Left(itertools::Either::Right(linear.children.iter().copied()))
            }
            Self::Grid(grid) => itertools::Either::Right(grid.children().copied()),
        }
    }

    /// If we have exactly one child, return it
    pub fn only_child(&self) -> Option<TileId> {
        let mut only_child = None;
        for &child in self.children() {
            if only_child.is_none() {
                only_child = Some(child);
            } else {
                return None;
            }
        }
        only_child
    }

    pub fn children_vec(&self) -> Vec<TileId> {
        self.children().copied().collect()
    }

    pub fn has_child(&self, needle: TileId) -> bool {
        self.children().any(|&t| t == needle)
    }

    pub fn add_child(&mut self, child: TileId) {
        match self {
            Self::Tabs(tabs) => tabs.add_child(child),
            Self::Linear(linear) => linear.add_child(child),
            Self::Grid(grid) => grid.add_child(child),
        }
    }

    /// Iterate through all children in order, and keep only those for which the closure returns `true`.
    pub fn retain(&mut self, mut retain: impl FnMut(TileId) -> bool) {
        match self {
            Self::Tabs(tabs) => tabs.children.retain(|tile_id: &TileId| retain(*tile_id)),
            Self::Linear(linear) => linear.children.retain(|tile_id: &TileId| retain(*tile_id)),
            Self::Grid(grid) => grid.retain(retain),
        }
    }

    /// Swap out one child for another, keeping its place and its share of the space.
    ///
    /// Returns the child index that was swapped, mirroring [`Self::remove_child`],
    /// or `None` if `old` was not a child of this container.
    #[must_use]
    pub fn replace_child(&mut self, old: TileId, new: TileId) -> Option<usize> {
        match self {
            Self::Tabs(tabs) => tabs.replace_child(old, new),
            Self::Linear(linear) => linear.replace_child(old, new),
            Self::Grid(grid) => grid.replace_child(old, new),
        }
    }

    /// Returns child index, if found.
    pub fn remove_child(&mut self, child: TileId) -> Option<usize> {
        match self {
            Self::Tabs(tabs) => tabs.remove_child(child),
            Self::Linear(linear) => linear.remove_child(child),
            Self::Grid(grid) => grid.remove_child(child),
        }
    }

    pub fn kind(&self) -> ContainerKind {
        match self {
            Self::Tabs(_) => ContainerKind::Tabs,
            Self::Linear(linear) => match linear.dir {
                LinearDir::Horizontal => ContainerKind::Horizontal,
                LinearDir::Vertical => ContainerKind::Vertical,
            },
            Self::Grid(_) => ContainerKind::Grid,
        }
    }

    pub fn set_kind(&mut self, kind: ContainerKind) {
        if kind == self.kind() {
            return;
        }

        *self = match kind {
            ContainerKind::Tabs => Self::Tabs(Tabs::new(self.children_vec())),
            ContainerKind::Horizontal => {
                Self::Linear(Linear::new(LinearDir::Horizontal, self.children_vec()))
            }
            ContainerKind::Vertical => {
                Self::Linear(Linear::new(LinearDir::Vertical, self.children_vec()))
            }
            ContainerKind::Grid => Self::Grid(Grid::new(self.children_vec())),
        };
    }

    pub(super) fn simplify_children(&mut self, simplify: impl FnMut(TileId) -> SimplifyAction) {
        match self {
            Self::Tabs(tabs) => tabs.simplify_children(simplify),
            Self::Linear(linear) => linear.simplify_children(simplify),
            Self::Grid(grid) => grid.simplify_children(simplify),
        }
    }

    pub(super) fn layout<Pane>(
        &mut self,
        tiles: &mut Tiles<Pane>,
        layout: &LayoutContext<'_>,
        rect: Rect,
        tile_id: TileId,
    ) {
        if self.is_empty() {
            return;
        }

        match self {
            Self::Tabs(tabs) => tabs.layout(tiles, layout, rect, tile_id),
            Self::Linear(linear) => {
                linear.layout(tiles, layout, rect);
            }
            Self::Grid(grid) => grid.layout(tiles, layout, rect),
        }
    }

    pub(super) fn ui<Pane>(
        &mut self,
        tree: &mut Tree<Pane>,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &mut DropContext,
        ui: &mut egui::Ui,
        rect: Rect,
        tile_id: TileId,
    ) {
        match self {
            Self::Tabs(tabs) => {
                tabs.ui(tree, behavior, drop_context, ui, rect, tile_id);
            }
            Self::Linear(linear) => {
                linear.ui(tree, behavior, drop_context, ui, tile_id);
            }
            Self::Grid(grid) => {
                grid.ui(tree, behavior, drop_context, ui, tile_id);
            }
        }
    }
}
