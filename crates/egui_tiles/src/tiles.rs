use egui::{Pos2, Rect};

use crate::behavior::LayoutContext;

use super::{
    Behavior, Container, ContainerInsertion, ContainerKind, GcAction, InsertionPoint, Linear,
    LinearDir, SimplificationOptions, SimplifyAction, Tabs, Tile, TileId,
};

/// Contains all tile state, but no root.
///
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
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Tiles<Pane> {
    next_tile_id: u64,

    tiles: ahash::HashMap<TileId, Tile<Pane>>,

    /// Tiles are visible by default, so we only store the invisible ones.
    invisible: ahash::HashSet<TileId>,

    /// Filled in by the layout step at the start of each frame.
    #[cfg_attr(feature = "serde", serde(default, skip))]
    pub(super) rects: ahash::HashMap<TileId, Rect>,
}

impl<Pane: PartialEq> PartialEq for Tiles<Pane> {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            next_tile_id: _, // ignored
            tiles,
            invisible,
            rects: _, // ignore transient state
        } = self;
        tiles == &other.tiles && invisible == &other.invisible
    }
}

impl<Pane> Default for Tiles<Pane> {
    fn default() -> Self {
        Self {
            next_tile_id: 1,
            tiles: Default::default(),
            invisible: Default::default(),
            rects: Default::default(),
        }
    }
}

// ----------------------------------------------------------------------------

impl<Pane> Tiles<Pane> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The number of tiles, including invisible tiles.
    #[inline]
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn get(&self, tile_id: TileId) -> Option<&Tile<Pane>> {
        self.tiles.get(&tile_id)
    }

    /// Get the pane instance for a given [`TileId`]
    pub fn get_pane(&self, tile_id: &TileId) -> Option<&Pane> {
        match self.tiles.get(tile_id)? {
            Tile::Pane(pane) => Some(pane),
            Tile::Container(_) => None,
        }
    }

    /// Get the container instance for a given [`TileId`]
    pub fn get_container(&self, tile_id: TileId) -> Option<&Container> {
        match self.tiles.get(&tile_id)? {
            Tile::Container(container) => Some(container),
            Tile::Pane(_) => None,
        }
    }

    pub fn get_mut(&mut self, tile_id: TileId) -> Option<&mut Tile<Pane>> {
        self.tiles.get_mut(&tile_id)
    }

    /// Get the screen-space rectangle of where a tile is shown.
    ///
    /// This is updated by [`crate::Tree::ui`], so you need to call that first.
    ///
    /// If the tile isn't visible, or is in an inactive tab, this return `None`.
    pub fn rect(&self, tile_id: TileId) -> Option<Rect> {
        if self.is_visible_in_layout(tile_id) {
            self.rects.get(&tile_id).copied()
        } else {
            None
        }
    }

    pub(super) fn rect_or_die(&self, tile_id: TileId) -> Rect {
        let rect = self.rect(tile_id);
        debug_assert!(rect.is_some(), "Failed to find rect for {tile_id:?}");
        rect.unwrap_or(egui::Rect::from_min_max(Pos2::ZERO, Pos2::ZERO))
    }

    /// All tiles, in arbitrary order
    pub fn iter(&self) -> impl Iterator<Item = (&TileId, &Tile<Pane>)> + '_ {
        self.tiles.iter()
    }

    /// All tiles, in arbitrary order
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&TileId, &mut Tile<Pane>)> + '_ {
        self.tiles.iter_mut()
    }

    /// All [`TileId`]s, in arbitrary order
    pub fn tile_ids(&self) -> impl Iterator<Item = TileId> + '_ {
        self.tiles.keys().copied()
    }

    /// All [`Tile`]s in arbitrary order
    pub fn tiles(&self) -> impl Iterator<Item = &Tile<Pane>> + '_ {
        self.tiles.values()
    }

    /// All [`Tile`]s in arbitrary order
    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<Pane>> + '_ {
        self.tiles.values_mut()
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn is_visible(&self, tile_id: TileId) -> bool {
        !self.invisible.contains(&tile_id)
    }

    /// Is this tile given any space in the layout?
    ///
    /// Like [`Self::is_visible`], except a container whose children are all invisible is
    /// skipped too: it would take up space to show nothing.
    ///
    /// An empty container hides nothing and is still a drop target, so it keeps its space.
    pub fn is_visible_in_layout(&self, tile_id: TileId) -> bool {
        if !self.is_visible(tile_id) {
            return false;
        }

        let Some(Tile::Container(container)) = self.get(tile_id) else {
            return true;
        };

        let mut has_children = false;
        for &child_id in container.children() {
            has_children = true;
            if self.is_visible_in_layout(child_id) {
                return true;
            }
        }
        !has_children
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn set_visible(&mut self, tile_id: TileId, visible: bool) {
        if visible {
            self.invisible.remove(&tile_id);
        } else {
            self.invisible.insert(tile_id);
        }
    }

    pub fn toggle_visibility(&mut self, tile_id: TileId) {
        self.set_visible(tile_id, !self.is_visible(tile_id));
    }

    /// This excludes all tiles that are invisible or are inactive tabs, recursively.
    pub(crate) fn collect_active_tiles(&self, tile_id: TileId, tiles: &mut Vec<TileId>) {
        if !self.is_visible_in_layout(tile_id) {
            return;
        }
        tiles.push(tile_id);

        if let Some(Tile::Container(container)) = self.get(tile_id) {
            for child_id in container.active_children(self) {
                self.collect_active_tiles(child_id, tiles);
            }
        }
    }

    pub fn insert(&mut self, id: TileId, tile: Tile<Pane>) {
        self.tiles.insert(id, tile);
    }

    /// Remove the tile with the given id from the tiles container.
    ///
    /// Note that this does not actually remove the tile from the tree and may
    /// leave dangling references. If you want to permanently remove the tile
    /// consider calling [`crate::Tree::remove_recursively`].
    pub fn remove(&mut self, id: TileId) -> Option<Tile<Pane>> {
        self.tiles.remove(&id)
    }

    pub fn next_free_id(&mut self) -> TileId {
        let mut id = TileId::from_u64(self.next_tile_id);

        // Make sure it doesn't collide with an existing id
        while self.tiles.contains_key(&id) {
            self.next_tile_id += 1;
            id = TileId::from_u64(self.next_tile_id);
        }

        // Final increment the next_id
        self.next_tile_id += 1;

        id
    }
    /// Recomputes `next_tile_id` so that newly allocated tiles
    /// will not collide with any existing `TileId` or any holes
    /// left in the numbering.
    ///
    /// Note: this sets `next_tile_id` to the maximum existing `TileId`+1.
    pub fn recompute_next_tile_id(&mut self) {
        self.next_tile_id = self
            .tiles
            .keys()
            .map(|tile_id| tile_id.0 + 1)
            .max()
            .unwrap_or(self.next_tile_id);
    }

    #[must_use]
    pub fn insert_new(&mut self, tile: Tile<Pane>) -> TileId {
        let id = self.next_free_id();
        self.tiles.insert(id, tile);
        id
    }

    #[must_use]
    pub fn insert_pane(&mut self, pane: Pane) -> TileId {
        self.insert_new(Tile::Pane(pane))
    }

    #[must_use]
    pub fn insert_container(&mut self, container: impl Into<Container>) -> TileId {
        self.insert_new(Tile::Container(container.into()))
    }

    #[must_use]
    pub fn insert_tab_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_tabs(children)))
    }

    #[must_use]
    pub fn insert_horizontal_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_linear(
            LinearDir::Horizontal,
            children,
        )))
    }

    #[must_use]
    pub fn insert_vertical_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_linear(
            LinearDir::Vertical,
            children,
        )))
    }

    #[must_use]
    pub fn insert_grid_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_grid(children)))
    }

    pub fn parent_of(&self, child_id: TileId) -> Option<TileId> {
        #[expect(clippy::iter_over_hash_type)] // Each tile can only have one parent
        for (tile_id, tile) in &self.tiles {
            if let Tile::Container(container) = tile
                && container.has_child(child_id)
            {
                return Some(*tile_id);
            }
        }
        None
    }

    pub fn is_root(&self, tile_id: TileId) -> bool {
        self.parent_of(tile_id).is_none()
    }

    /// Insert `inserted_id` into the tree at `insertion_point`.
    ///
    /// When the insertion point's parent is not already a container of the required kind, that
    /// parent gets wrapped in a new container holding both it and `inserted_id`. The parent keeps
    /// its own [`TileId`] and the *new* container is the one given a freshly allocated id, so a
    /// `TileId` always keeps referring to the same tile. Applications that key their own state off
    /// `TileId`s depend on that.
    ///
    /// Returns the id of the new container, when one was needed. Since [`Tiles`] does not know
    /// what the tree's root is, the caller must re-point the root at it if the wrapped tile
    /// happened to be the root.
    #[must_use]
    pub(super) fn insert_at(
        &mut self,
        insertion_point: InsertionPoint,
        inserted_id: TileId,
    ) -> Option<TileId> {
        let InsertionPoint {
            parent_id,
            insertion,
        } = insertion_point;

        let Some(mut parent_tile) = self.tiles.remove(&parent_id) else {
            log::debug!("Failed to insert: could not find parent {parent_id:?}");
            return None;
        };

        // Can the parent take the tile as-is, or does it need wrapping in a new container?
        let inserted_into_parent = match (&mut parent_tile, insertion) {
            (Tile::Container(Container::Tabs(tabs)), ContainerInsertion::Tabs(index)) => {
                let index = index.min(tabs.children.len());
                tabs.children.insert(index, inserted_id);
                tabs.set_active(inserted_id);
                true
            }

            (
                Tile::Container(Container::Linear(
                    linear @ Linear {
                        dir: LinearDir::Horizontal,
                        ..
                    },
                )),
                ContainerInsertion::Horizontal(index),
            )
            | (
                Tile::Container(Container::Linear(
                    linear @ Linear {
                        dir: LinearDir::Vertical,
                        ..
                    },
                )),
                ContainerInsertion::Vertical(index),
            ) => {
                let index = index.min(linear.children.len());
                linear.children.insert(index, inserted_id);
                true
            }

            (Tile::Container(Container::Grid(grid)), ContainerInsertion::Grid(index)) => {
                grid.insert_at(index, inserted_id);
                true
            }

            _ => false,
        };

        // Either way the parent goes back exactly where it was, under its original id.
        self.tiles.insert(parent_id, parent_tile);

        if inserted_into_parent {
            return None;
        }

        // Look up the grandparent _before_ creating the wrapper, or `parent_of` would find the
        // wrapper itself.
        let grandparent_id = self.parent_of(parent_id);

        let mut children = vec![parent_id];
        children.insert(insertion.index().min(1), inserted_id);

        let container = match insertion {
            ContainerInsertion::Tabs(_) => {
                let mut tabs = Tabs::new(children);
                tabs.set_active(inserted_id);
                Container::Tabs(tabs)
            }
            ContainerInsertion::Horizontal(_) => {
                Container::new_linear(LinearDir::Horizontal, children)
            }
            ContainerInsertion::Vertical(_) => Container::new_linear(LinearDir::Vertical, children),
            ContainerInsertion::Grid(_) => Container::new_grid(children),
        };
        let wrapper_id = self.insert_new(Tile::Container(container));

        // Whoever referred to the parent now refers to the container wrapping it.
        // `grandparent_id` is `None` when the parent is the root, which the caller handles.
        if let Some(grandparent_id) = grandparent_id
            && let Some(Tile::Container(grandparent)) = self.get_mut(grandparent_id)
            && grandparent.replace_child(parent_id, wrapper_id).is_none()
        {
            // `grandparent_id` came from `parent_of`, so this should be unreachable.
            log::warn!(
                "Bug: {grandparent_id:?} was the parent of {parent_id:?}, \
                 but does not list it as a child"
            );
        }

        Some(wrapper_id)
    }

    /// Detect cycles, duplications, and other invalid state, and fix it.
    ///
    /// Will also call [`Behavior::retain_pane`] to check if a users wants to remove a pane.
    ///
    /// Finally free up any tiles that are no longer reachable from the root.
    ///
    /// Returns whether the root survived.
    #[must_use]
    pub(super) fn gc_root(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        root_id: Option<TileId>,
    ) -> bool {
        let mut visited = Default::default();

        let root_kept = match root_id {
            Some(root_id) => self.gc_tile_id(behavior, &mut visited, root_id) == GcAction::Keep,
            None => true,
        };

        if visited.len() < self.tiles.len() {
            // This should only happen if the user set up the tree in a bad state,
            // or if it was restored from a bad state via serde.
            // …or if there is a bug somewhere 😜
            log::debug!(
                "GC collecting tiles: {:?}",
                self.tiles
                    .keys()
                    .filter(|id| !visited.contains(id))
                    .collect::<Vec<_>>()
            );
        }

        self.invisible.retain(|tile_id| visited.contains(tile_id));
        self.tiles.retain(|tile_id, _| visited.contains(tile_id));

        root_kept
    }

    /// Detect cycles, duplications, and other invalid state, and remove them.
    fn gc_tile_id(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        visited: &mut ahash::HashSet<TileId>,
        tile_id: TileId,
    ) -> GcAction {
        let Some(mut tile) = self.tiles.remove(&tile_id) else {
            return GcAction::Remove;
        };
        if !visited.insert(tile_id) {
            log::warn!("Cycle or duplication detected");
            // Put the tile back before telling the caller to drop it.
            //
            // What is duplicated is the *reference*, not the tile: we got here through a second
            // parent, and the tile is still alive under the first one. Returning without the
            // re-insert deletes it from the arena outright, and then the first parent names a
            // child that no longer exists - damage strictly worse than the sharing it was meant
            // to repair. `simplify` unravels it from there: the parent looks empty, gets pruned,
            // its parent looks empty, and a tree that was merely ill-formed ends up gone.
            self.tiles.insert(tile_id, tile);
            return GcAction::Remove;
        }

        match &mut tile {
            Tile::Pane(pane) => {
                if !behavior.retain_pane(pane) {
                    return GcAction::Remove;
                }
            }
            Tile::Container(container) => {
                container
                    .retain(|child| self.gc_tile_id(behavior, visited, child) == GcAction::Keep);
            }
        }
        self.tiles.insert(tile_id, tile);
        GcAction::Keep
    }

    pub(super) fn layout_tile(&mut self, layout: &LayoutContext<'_>, rect: Rect, tile_id: TileId) {
        let Some(mut tile) = self.tiles.remove(&tile_id) else {
            log::debug!("Failed to find tile {tile_id:?} during layout");
            return;
        };
        self.rects.insert(tile_id, rect);

        if let Tile::Container(container) = &mut tile {
            container.layout(self, layout, rect, tile_id);
        }

        self.tiles.insert(tile_id, tile);
    }

    /// Simplify the tree, perhaps culling empty containers,
    /// and/or merging single-child containers into their parent.
    ///
    /// Drag-dropping tiles can often leave containers empty, or with only a single child.
    /// This is often undesired, so this function can be used to clean up the tree.
    ///
    /// What simplifications are allowed is controlled by the [`SimplificationOptions`].
    pub(super) fn simplify(
        &mut self,
        options: &SimplificationOptions,
        it: TileId,
        parent_kind: Option<ContainerKind>,
    ) -> SimplifyAction {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::debug!("Failed to find tile {it:?} during simplify");
            return SimplifyAction::Remove;
        };

        if let Tile::Container(container) = &mut tile {
            let kind = container.kind();
            container.simplify_children(|child| self.simplify(options, child, Some(kind)));

            if kind == ContainerKind::Tabs {
                if options.prune_empty_tabs && container.is_empty() {
                    log::trace!("Simplify: removing empty tabs container");
                    return SimplifyAction::Remove;
                }

                if options.prune_single_child_tabs
                    && let Some(only_child) = container.only_child()
                {
                    let child_is_pane = matches!(self.get(only_child), Some(Tile::Pane(_)));

                    if options.all_panes_must_have_tabs
                        && child_is_pane
                        && parent_kind != Some(ContainerKind::Tabs)
                    {
                        // Keep it, even though we only have one child
                    } else {
                        log::trace!("Simplify: collapsing single-child tabs container");
                        return SimplifyAction::Replace(only_child);
                    }
                }

                if options.flatten_tabs_in_tabs {
                    let was_active = match container {
                        Container::Tabs(tabs) => tabs.active,
                        Container::Linear(_) | Container::Grid(_) => None,
                    };

                    let mut found = false;
                    let mut new_children = Vec::new();
                    let mut new_active = None;

                    for &child_id in container.children() {
                        if let Some(Tile::Container(Container::Tabs(child_tabs))) =
                            self.get(child_id)
                        {
                            if was_active == Some(child_id) {
                                // The open tab is the container being flattened away, so the
                                // user is looking at whichever of _its_ tabs was open.
                                new_active = child_tabs.active;
                            }
                            new_children.extend(child_tabs.children.iter().copied());
                            found = true;

                            // Its tabs now belong to this container, so the child itself is gone.
                            self.tiles.remove(&child_id);
                        } else {
                            if was_active == Some(child_id) {
                                new_active = Some(child_id);
                            }
                            new_children.push(child_id);
                        }
                    }

                    if found {
                        // Keep this container's own id: flattening changes what it holds,
                        // not which container it is. For the same reason, keep showing the tab
                        // the user had open.
                        let mut tabs = Tabs::new(new_children);
                        if let Some(new_active) = new_active {
                            tabs.set_active(new_active);
                        }
                        *container = Container::Tabs(tabs);
                    }
                }
            } else {
                if options.join_nested_linear_containers
                    && let Container::Linear(parent) = container
                {
                    let mut new_children = Vec::with_capacity(parent.children.len());
                    for child_id in parent.children.drain(..) {
                        if let Some(Tile::Container(Container::Linear(child))) =
                            &mut self.get_mut(child_id)
                        {
                            if parent.dir == child.dir {
                                // absorb the child
                                log::trace!(
                                    "Simplify: absorbing nested linear container with {} children",
                                    child.children.len()
                                );

                                let mut child_share_sum = 0.0;
                                for &grandchild in &child.children {
                                    child_share_sum += child.shares[grandchild];
                                }
                                let share_normalizer = parent.shares[child_id] / child_share_sum;
                                for &grandchild in &child.children {
                                    new_children.push(grandchild);
                                    parent.shares[grandchild] =
                                        child.shares[grandchild] * share_normalizer;
                                }

                                self.tiles.remove(&child_id);
                            } else {
                                // keep the child
                                new_children.push(child_id);
                            }
                        } else {
                            new_children.push(child_id);
                        }
                    }
                    parent.children = new_children;
                }

                if options.prune_empty_containers && container.is_empty() {
                    log::trace!("Simplify: removing empty container tile");
                    return SimplifyAction::Remove;
                }
                if options.prune_single_child_containers
                    && let Some(only_child) = container.only_child()
                {
                    log::trace!("Simplify: collapsing single-child container tile");
                    return SimplifyAction::Replace(only_child);
                }
            }
        }

        self.tiles.insert(it, tile);
        SimplifyAction::Keep
    }

    /// Returns the id of a new container that should take `it`'s place in its parent, if `it` was
    /// a pane that had to be wrapped in one.
    ///
    /// The pane keeps its own [`TileId`]; it is the new container that gets a fresh one. See
    /// [`Self::insert_at`] for why that matters.
    #[must_use]
    pub(super) fn make_all_panes_children_of_tabs(
        &mut self,
        parent_is_tabs: bool,
        it: TileId,
    ) -> Option<TileId> {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::debug!("Failed to find tile {it:?} during make_all_panes_children_of_tabs");
            return None;
        };

        match &mut tile {
            Tile::Pane(_) => {
                if !parent_is_tabs {
                    // Add tabs to this pane:
                    log::trace!("Auto-adding Tabs-parent to pane {it:?}");
                    self.tiles.insert(it, tile);
                    let tabs = Container::new_tabs(vec![it]);
                    return Some(self.insert_new(Tile::Container(tabs)));
                }
            }
            Tile::Container(container) => {
                let is_tabs = container.kind() == ContainerKind::Tabs;
                let children: Vec<TileId> = container.children().copied().collect();
                for child in children {
                    if let Some(new_child) = self.make_all_panes_children_of_tabs(is_tabs, child)
                        && container.replace_child(child, new_child).is_none()
                    {
                        log::warn!("Bug: {child:?} is no longer a child of {it:?}");
                    }
                }
            }
        }

        self.tiles.insert(it, tile);
        None
    }

    /// Returns true if the active tile was found in this tree.
    pub(super) fn make_active(
        &mut self,
        it: TileId,
        should_activate: &mut dyn FnMut(TileId, &Tile<Pane>) -> bool,
    ) -> bool {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::debug!("Failed to find tile {it:?} during make_active");
            return false;
        };

        let mut activate = should_activate(it, &tile);

        if let Tile::Container(container) = &mut tile {
            let mut active_child = None;
            for &child in container.children() {
                if self.make_active(child, should_activate) {
                    active_child = Some(child);
                }
            }

            if let Some(active_child) = active_child
                && let Container::Tabs(tabs) = container
            {
                tabs.set_active(active_child);
            }

            activate |= active_child.is_some();
        }

        self.tiles.insert(it, tile);
        activate
    }
}

impl<Pane: PartialEq> Tiles<Pane> {
    /// Find the tile with the given pane.
    pub fn find_pane(&self, needle: &Pane) -> Option<TileId> {
        self.tiles
            .iter()
            .find(|(_, tile)| {
                if let Tile::Pane(pane) = *tile {
                    pane == needle
                } else {
                    false
                }
            })
            .map(|(tile_id, _)| *tile_id)
    }
}

// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Grid, GridLayout, Shares, Tree};

    fn insertion(parent_id: TileId, insertion: ContainerInsertion) -> InsertionPoint {
        InsertionPoint {
            parent_id,
            insertion,
        }
    }

    /// Wrapping a tile in a new container must not disturb the wrapped tile's own [`TileId`].
    ///
    /// Applications may key their own state off `TileId`s (Rerun does), so an id has to keep
    /// referring to the same tile for as long as that tile exists.
    #[test]
    fn wrapping_a_tile_keeps_its_id() {
        for insertion_kind in [
            ContainerInsertion::Tabs(1),
            ContainerInsertion::Horizontal(1),
            ContainerInsertion::Vertical(1),
            ContainerInsertion::Grid(1),
        ] {
            let mut tiles = Tiles::default();
            let a = tiles.insert_pane("a");
            let b = tiles.insert_pane("b");
            let root = tiles.insert_horizontal_tile(vec![a, b]);
            let dropped = tiles.insert_pane("dropped");

            // Drop `dropped` onto pane `a`, which has to wrap `a` in a new container:
            let wrapper = tiles
                .insert_at(insertion(a, insertion_kind), dropped)
                .expect("wrapping a pane should have created a container");

            assert_eq!(
                tiles.get(a),
                Some(&Tile::Pane("a")),
                "{insertion_kind:?}: pane `a` must still be a pane, under its original id"
            );
            assert_ne!(
                wrapper, a,
                "{insertion_kind:?}: the new container needs its own id"
            );

            let wrapper_children = tiles
                .get_container(wrapper)
                .expect("the wrapper should be a container")
                .children_vec();
            assert!(
                wrapper_children.contains(&a) && wrapper_children.contains(&dropped),
                "{insertion_kind:?}: the wrapper should hold both tiles, but holds \
                 {wrapper_children:?}"
            );

            // ...and the grandparent should now point at the wrapper rather than at `a`:
            assert_eq!(
                tiles
                    .get_container(root)
                    .expect("root should be a container")
                    .children_vec(),
                vec![wrapper, b],
                "{insertion_kind:?}: the root should hold the wrapper in `a`'s old place"
            );
            assert_eq!(
                tiles.parent_of(a),
                Some(wrapper),
                "{insertion_kind:?}: `a` should now live inside the wrapper"
            );
        }
    }

    /// The wrapper takes over the wrapped tile's place in the layout, so it must inherit its
    /// share of the parent's space -- otherwise the layout jumps on drop.
    #[test]
    fn the_wrapper_inherits_the_wrapped_tile_s_share() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let root = tiles.insert_horizontal_tile(vec![a, b]);
        let dropped = tiles.insert_pane("dropped");

        // Give `a` a distinctly lopsided share:
        let mut shares = Shares::default();
        shares.set_share(a, 3.0);
        shares.set_share(b, 1.0);
        if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(root) {
            linear.shares = shares;
        }

        let wrapper = tiles
            .insert_at(insertion(a, ContainerInsertion::Vertical(1)), dropped)
            .expect("wrapping a pane should have created a container");

        let Some(Tile::Container(Container::Linear(linear))) = tiles.get(root) else {
            panic!("root should still be a linear container");
        };
        assert_eq!(
            linear.shares[wrapper], 3.0,
            "the wrapper should have taken over `a`'s share"
        );
        assert_eq!(linear.shares[b], 1.0, "`b`'s share should be untouched");
    }

    /// If the wrapped tile was the open tab, the container replacing it should be the open tab.
    #[test]
    fn the_wrapper_inherits_being_the_active_tab() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let root = tiles.insert_tab_tile(vec![a, b]);
        let dropped = tiles.insert_pane("dropped");

        if let Some(Tile::Container(Container::Tabs(tabs))) = tiles.get_mut(root) {
            tabs.set_active(a);
        }

        let wrapper = tiles
            .insert_at(insertion(a, ContainerInsertion::Vertical(1)), dropped)
            .expect("wrapping a pane should have created a container");

        let Some(Tile::Container(Container::Tabs(tabs))) = tiles.get(root) else {
            panic!("root should still be a tabs container");
        };
        assert_eq!(
            tabs.active,
            Some(wrapper),
            "the wrapper took `a`'s place, so it should be the open tab"
        );
    }

    /// A grid's shares are positional, so the wrapper has to land in the wrapped tile's cell.
    #[test]
    fn the_wrapper_takes_the_wrapped_tile_s_grid_cell() {
        let mut tiles = Tiles::default();
        let panes: Vec<TileId> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|pane| tiles.insert_pane(pane))
            .collect();
        let mut grid = Grid::new(panes.clone());
        grid.layout = GridLayout::Columns(2);
        let root = tiles.insert_new(Tile::Container(Container::Grid(grid)));
        let dropped = tiles.insert_pane("dropped");

        // Wrap the third pane, i.e. the one in the bottom-left cell:
        let wrapper = tiles
            .insert_at(
                insertion(panes[2], ContainerInsertion::Vertical(1)),
                dropped,
            )
            .expect("wrapping a pane should have created a container");

        assert_eq!(
            tiles
                .get_container(root)
                .expect("root should be a container")
                .children_vec(),
            vec![panes[0], panes[1], wrapper, panes[3]],
            "the wrapper should sit in the cell the wrapped pane occupied"
        );
    }

    /// Wrapping a pane in the tab container that `all_panes_must_have_tabs` requires must not
    /// disturb the pane's own [`TileId`], for the same reason [`Tiles::insert_at`] must not.
    #[test]
    fn auto_adding_tabs_keeps_the_pane_s_id() {
        let options = SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        };

        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let root = tiles.insert_horizontal_tile(vec![a, b]);
        let mut tree = Tree::new(egui::Id::unique("test"), root, tiles);

        tree.simplify(&options);

        for pane in [a, b] {
            assert!(
                matches!(tree.tiles.get(pane), Some(Tile::Pane(_))),
                "pane {pane:?} should still be a pane under its own id, but is {:?}",
                tree.tiles.get(pane)
            );
            let parent = tree.tiles.parent_of(pane).expect("the pane needs a parent");
            assert_eq!(
                tree.tiles.get_container(parent).map(Container::kind),
                Some(ContainerKind::Tabs),
                "the pane should have gained a tab container as its parent"
            );
            assert_ne!(parent, pane, "the tab container needs its own id");
        }

        assert_eq!(tree.root, Some(root), "the root container is untouched");

        // Running it again must be a no-op — otherwise it would wrap forever.
        let before = tree.clone();
        tree.simplify(&options);
        assert_eq!(before, tree, "simplifying twice should change nothing");
    }

    /// The same, for a tree whose root *is* a bare pane: there the new tab container becomes the
    /// root, which only [`Tree`] can arrange.
    #[test]
    fn auto_adding_tabs_to_a_root_pane_makes_the_tabs_the_root() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let mut tree = Tree::new(egui::Id::unique("test"), a, tiles);

        tree.simplify(&SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        });

        let root = tree.root.expect("the tree should still have a root");
        assert_ne!(root, a, "the new tab container should be the root");
        assert!(
            matches!(tree.tiles.get(a), Some(Tile::Pane(_))),
            "`a` is still a pane"
        );
        assert_eq!(
            tree.tiles.get_container(root).map(Container::children_vec),
            Some(vec![a]),
            "the root tab container should hold the pane"
        );
    }

    /// Flattening tabs-in-tabs rearranges what a container holds, so it should keep its id.
    #[test]
    fn flattening_tabs_in_tabs_keeps_the_container_s_id() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let inner = tiles.insert_tab_tile(vec![a, b]);
        let c = tiles.insert_pane("c");
        let outer = tiles.insert_tab_tile(vec![inner, c]);
        let mut tree = Tree::new(egui::Id::unique("test"), outer, tiles);

        tree.simplify(&SimplificationOptions {
            flatten_tabs_in_tabs: true,
            ..SimplificationOptions::OFF
        });

        assert_eq!(
            tree.root,
            Some(outer),
            "the outer container should still be the root"
        );
        assert_eq!(
            tree.tiles.get_container(outer).map(Container::children_vec),
            Some(vec![a, b, c]),
            "the inner container's tabs should have been folded into the outer one"
        );
    }

    /// The flattened-away container must be removed, or it would still claim `a` and `b` as its
    /// children, and [`Tiles::parent_of`] would return either container depending on hash order.
    #[test]
    fn flattening_tabs_in_tabs_removes_the_inner_container() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let b = tiles.insert_pane("b");
        let inner = tiles.insert_tab_tile(vec![a, b]);
        let c = tiles.insert_pane("c");
        let outer = tiles.insert_tab_tile(vec![inner, c]);
        let mut tree = Tree::new(egui::Id::unique("test"), outer, tiles);

        tree.simplify(&SimplificationOptions {
            flatten_tabs_in_tabs: true,
            ..SimplificationOptions::OFF
        });

        assert_eq!(
            tree.tiles.get(inner),
            None,
            "the flattened-away inner container should be gone"
        );
        assert_eq!(tree.tiles.len(), 4, "`outer`, `a`, `b` and `c` are left");
    }

    /// Flattening keeps the container's id, so it should also keep showing whatever tab the user
    /// had open — including when the open tab is the inner container being flattened away, where
    /// what they are really looking at is one of *its* tabs.
    #[test]
    fn flattening_tabs_in_tabs_keeps_the_open_tab() {
        // `outer` is `Tabs[ Tabs[a, b], c ]`, and flattens to `Tabs[a, b, c]`.
        let build = || {
            let mut tiles = Tiles::default();
            let a = tiles.insert_pane("a");
            let b = tiles.insert_pane("b");
            let inner = tiles.insert_tab_tile(vec![a, b]);
            let c = tiles.insert_pane("c");
            let outer = tiles.insert_tab_tile(vec![inner, c]);
            (
                Tree::new(egui::Id::unique("test"), outer, tiles),
                a,
                b,
                inner,
                c,
                outer,
            )
        };
        let options = SimplificationOptions {
            flatten_tabs_in_tabs: true,
            ..SimplificationOptions::OFF
        };
        let active_of = |tree: &Tree<&'static str>, id: TileId| match tree.tiles.get_container(id) {
            Some(Container::Tabs(tabs)) => tabs.active,
            _ => panic!("{id:?} should be a tabs container"),
        };

        // The open tab is `c`, which survives flattening untouched:
        {
            let (mut tree, _a, _b, _inner, c, outer) = build();
            if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(outer) {
                tabs.set_active(c);
            }
            tree.simplify(&options);
            assert_eq!(
                active_of(&tree, outer),
                Some(c),
                "`c` was open, and still is"
            );
        }

        // The open tab is the inner container, which was itself showing `b`:
        {
            let (mut tree, _a, b, inner, _c, outer) = build();
            if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(inner) {
                tabs.set_active(b);
            }
            if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(outer) {
                tabs.set_active(inner);
            }
            tree.simplify(&options);
            assert_eq!(
                active_of(&tree, outer),
                Some(b),
                "the user was looking at `b` through the flattened container, so `b` stays open"
            );
        }
    }

    /// A tab container whose only pane is hidden has nothing left to show, so it should get no
    /// space of its own. Showing the pane again should bring the container back.
    #[test]
    fn a_container_with_nothing_to_show_is_not_laid_out() {
        let mut tiles = Tiles::default();
        let pane = tiles.insert_pane("a");
        let tabs = tiles.insert_tab_tile(vec![pane]);

        assert!(tiles.is_visible_in_layout(tabs));

        tiles.set_visible(pane, false);
        assert!(
            !tiles.is_visible_in_layout(tabs),
            "the tab container has only a hidden pane left to show"
        );
        assert!(
            tiles.is_visible(tabs),
            "the container itself was never hidden, and asking for the flag should say so"
        );

        tiles.set_visible(pane, true);
        assert!(
            tiles.is_visible_in_layout(tabs),
            "showing the pane again should bring back the container around it"
        );
    }

    /// Nothing visible anywhere in the subtree means nothing to lay out, however deep it is.
    #[test]
    fn hiding_the_only_pane_hides_every_container_above_it() {
        let mut tiles = Tiles::default();
        let pane = tiles.insert_pane("a");
        let tabs = tiles.insert_tab_tile(vec![pane]);
        let inner = tiles.insert_horizontal_tile(vec![tabs]);
        let outer = tiles.insert_vertical_tile(vec![inner]);

        tiles.set_visible(pane, false);

        for tile_id in [tabs, inner, outer] {
            assert!(
                !tiles.is_visible_in_layout(tile_id),
                "{tile_id:?} has nothing but the hidden pane below it"
            );
        }
    }

    /// An empty container is not hiding anything, and it is still a place to drop tiles into,
    /// so it keeps its space.
    #[test]
    fn an_empty_container_keeps_its_space() {
        let mut tiles = Tiles::<&str>::default();
        let empty = tiles.insert_tab_tile(vec![]);
        assert!(tiles.is_visible_in_layout(empty));

        tiles.set_visible(empty, false);
        assert!(
            !tiles.is_visible_in_layout(empty),
            "hiding it explicitly still hides it"
        );
    }

    /// Inserting into a container that already accepts the tile must not wrap anything.
    #[test]
    fn inserting_into_a_matching_container_creates_nothing() {
        let mut tiles = Tiles::default();
        let a = tiles.insert_pane("a");
        let root = tiles.insert_horizontal_tile(vec![a]);
        let dropped = tiles.insert_pane("dropped");

        let wrapper = tiles.insert_at(insertion(root, ContainerInsertion::Horizontal(0)), dropped);

        assert_eq!(wrapper, None, "a horizontal container takes it directly");
        assert_eq!(
            tiles
                .get_container(root)
                .expect("root should be a container")
                .children_vec(),
            vec![dropped, a]
        );
    }
}
