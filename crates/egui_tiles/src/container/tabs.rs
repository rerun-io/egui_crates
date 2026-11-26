use egui::{NumExt as _, Rect, Vec2, scroll_area::ScrollBarVisibility, vec2};

use crate::behavior::{EditAction, LayoutContext, TabState};
use crate::{
    Behavior, ContainerInsertion, DropContext, InsertionPoint, SimplifyAction, TileId, Tiles, Tree,
    is_being_dragged,
};

/// A container with tabs. Only one tab is open (active) at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Tabs {
    /// The tabs, in order.
    pub children: Vec<TileId>,

    /// The currently open tab.
    pub active: Option<TileId>,
}

/// The current tab scrolling state
#[derive(Clone, Copy, Debug, Default)]
struct ScrollState {
    /// The current horizontal scroll offset.
    ///
    /// Positive: scroll right.
    /// Negatie: scroll left.
    pub offset: f32,

    /// Outstanding offset to apply smoothly over the next few frames.
    /// This is what the buttons update.
    pub offset_debt: f32,

    /// The size of all the tabs last frame.
    pub content_size: Vec2,

    /// The available size for the tabs.
    pub available: Vec2,

    /// Show the left scroll-arrow this frame?
    pub show_left_arrow: bool,

    /// Show the right scroll-arrow this frame?
    pub show_right_arrow: bool,

    /// Did we show the left scroll-arrow last frame?
    pub showed_left_arrow_prev: bool,

    /// Did we show the right scroll-arrow last frame?
    pub showed_right_arrow_prev: bool,
}

impl ScrollState {
    /// Returns the space left for the tabs after the scroll arrows.
    pub fn update(&mut self, ui: &egui::Ui, arrow_size: Vec2) -> f32 {
        let mut scroll_area_width = ui.available_width();

        let button_and_spacing_width = arrow_size.x + ui.spacing().item_spacing.x;

        let margin = 0.1;

        self.show_left_arrow = arrow_size.x < self.offset;

        if self.show_left_arrow {
            scroll_area_width -= button_and_spacing_width;
        }

        self.show_right_arrow = self.offset + scroll_area_width + margin < self.content_size.x;

        // Compensate for showing/hiding of arrow:
        self.offset += button_and_spacing_width
            * ((self.show_left_arrow as i32 as f32) - (self.showed_left_arrow_prev as i32 as f32));

        if self.show_right_arrow {
            scroll_area_width -= button_and_spacing_width;
        }

        self.showed_left_arrow_prev = self.show_left_arrow;
        self.showed_right_arrow_prev = self.show_right_arrow;

        if self.offset_debt != 0.0 {
            const SPEED: f32 = 500.0;

            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let max_movement = dt * SPEED;
            if self.offset_debt.abs() <= max_movement {
                self.offset += self.offset_debt;
                self.offset_debt = 0.0;
            } else {
                let movement = self.offset_debt.signum() * max_movement;
                self.offset += movement;
                self.offset_debt -= movement;
                ui.request_repaint();
            }
        }

        scroll_area_width
    }

    fn scroll_increment(&self) -> f32 {
        (self.available.x / 3.0).at_least(20.0)
    }

    /// `rotation` is in radians, as in [`egui::Shape::rotated_triangle`]:
    /// `TAU / 4.0` points left, `-TAU / 4.0` right.
    ///
    /// The arrow is painted rather than written, so it does not depend on a font
    /// that carries the arrow glyphs.
    fn arrow_button(
        ui: &mut egui::Ui,
        arrow_size: Vec2,
        id: egui::Id,
        rotation: f32,
        label: &str,
    ) -> bool {
        // Same size and proportions as the egui submenu arrow.
        let icon_width = ui.spacing().icon_width;
        let triangle = egui::Atom::paint(Vec2::splat(icon_width), move |ui, args| {
            let rect = egui::Rect::from_center_size(
                args.rect.center(),
                egui::vec2(args.rect.width() * 0.55, args.rect.height() * 0.35),
            );
            ui.painter().add(egui::Shape::rotated_triangle(
                rect,
                rotation,
                args.fallback_text_color,
            ));
        });

        let response = ui
            .scope_builder(egui::UiBuilder::new().scope_id(id), |ui| {
                ui.add_sized(arrow_size, egui::Button::new(triangle))
            })
            .inner;

        // The button has no text, so give screen readers something to announce.
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));

        response.clicked()
    }

    fn hidden_arrow_marker(ui: &egui::Ui, arrow_size: Vec2, id: egui::Id) {
        let rect = ui
            .layout()
            .align_size_within_rect(arrow_size, ui.available_rect_before_wrap());
        ui.interact(rect, id, egui::Sense::hover());
    }

    pub fn left_arrow(&mut self, ui: &mut egui::Ui, arrow_size: Vec2, id: egui::Id) {
        if !self.show_left_arrow {
            Self::hidden_arrow_marker(ui, arrow_size, id);
            return;
        }

        if Self::arrow_button(
            ui,
            arrow_size,
            id,
            core::f32::consts::TAU / 4.0,
            "Scroll tabs left",
        ) {
            self.offset_debt -= self.scroll_increment();
        }
    }

    pub fn right_arrow(&mut self, ui: &mut egui::Ui, arrow_size: Vec2, id: egui::Id) {
        if !self.show_right_arrow {
            Self::hidden_arrow_marker(ui, arrow_size, id);
            return;
        }

        if Self::arrow_button(
            ui,
            arrow_size,
            id,
            -core::f32::consts::TAU / 4.0,
            "Scroll tabs right",
        ) {
            self.offset_debt += self.scroll_increment();
        }
    }
}

impl Tabs {
    pub fn new(children: Vec<TileId>) -> Self {
        let active = children.first().copied();
        Self { children, active }
    }

    pub fn add_child(&mut self, child: TileId) {
        self.children.push(child);
    }

    /// Swap out one tab for another, keeping its position and whether it was the open one.
    ///
    /// Returns the index of the tab that was swapped,
    /// or `None` if `old` was not a tab of this container.
    #[must_use]
    pub(super) fn replace_child(&mut self, old: TileId, new: TileId) -> Option<usize> {
        let index = self.children.iter().position(|child| *child == old)?;
        self.children[index] = new;
        if self.active == Some(old) {
            self.active = Some(new);
        }
        Some(index)
    }

    pub fn set_active(&mut self, child: TileId) {
        self.active = Some(child);
    }

    pub fn is_active(&self, child: TileId) -> bool {
        Some(child) == self.active
    }

    /// Make the tab after (or before) the active one the active tab, wrapping around at the ends.
    ///
    /// Hidden tabs are skipped.
    /// If no tab is active, the first (or last) visible tab is activated.
    ///
    /// Returns `true` if the active tab changed.
    pub fn cycle_active<Pane>(&mut self, tiles: &Tiles<Pane>, forward: bool) -> bool {
        let visible: Vec<TileId> = self
            .children
            .iter()
            .copied()
            .filter(|&child_id| tiles.is_visible_in_layout(child_id))
            .collect();

        let Some(last) = visible.len().checked_sub(1) else {
            return false;
        };

        let current = self
            .active
            .and_then(|active| visible.iter().position(|&child_id| child_id == active));

        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(index), true) => (index + 1) % visible.len(),
            (Some(index), false) => index.checked_sub(1).unwrap_or(last),
        };

        let new_active = Some(visible[next]);
        let changed = new_active != self.active;
        self.active = new_active;
        changed
    }

    pub(super) fn layout<Pane>(
        &mut self,
        tiles: &mut Tiles<Pane>,
        layout: &LayoutContext<'_>,
        rect: Rect,
        tile_id: TileId,
    ) {
        let prev_active = self.active;
        self.ensure_active(tiles);
        if prev_active != self.active {
            layout.tab_auto_selected.set(true);
        }

        let mut active_rect = rect;
        if (layout.is_tab_bar_visible)(tile_id, self) {
            active_rect.min.y += layout.tab_bar_height;
        }

        if let Some(active) = self.active {
            // Only lay out the active tab (saves CPU):
            tiles.layout_tile(layout, active_rect, active);
        }
    }

    pub fn next_active<Pane>(&self, tiles: &Tiles<Pane>) -> Option<TileId> {
        self.active
            .filter(|active| self.children.contains(active) && tiles.is_visible_in_layout(*active))
            .or_else(|| {
                self.children
                    .iter()
                    .copied()
                    .find(|&child_id| tiles.is_visible_in_layout(child_id))
            })
    }

    /// Make sure we have an active tab (or no visible tabs).
    pub fn ensure_active<Pane>(&mut self, tiles: &Tiles<Pane>) {
        self.active = self.next_active(tiles);
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
        let next_active = if behavior.is_tab_bar_visible(tile_id, self) {
            self.tab_bar_ui(tree, behavior, ui, rect, drop_context, tile_id)
        } else {
            self.active
        };

        if let Some(active) = self.active {
            tree.tile_ui(behavior, drop_context, ui, active);
            crate::cover_tile_if_dragged(tree, behavior, ui, active);
        }

        // We have only laid out the active tab, so we need to switch active tab _after_ the ui pass above.
        //
        // Mid-drag the tab bar draws the tabs this container _would_ have, so `next_active` can
        // name a tile that is on its way in but is not a child yet. Committing that would leave
        // this container pointing at a tile it does not contain.
        if next_active.is_none_or(|active| self.children.contains(&active)) {
            self.active = next_active;
        }
    }

    /// Returns the next active tab (e.g. the one clicked, or the current).
    fn tab_bar_ui<Pane>(
        &self,
        tree: &mut Tree<Pane>,
        behavior: &mut dyn Behavior<Pane>,
        ui: &mut egui::Ui,
        rect: Rect,
        drop_context: &mut DropContext,
        tile_id: TileId,
    ) -> Option<TileId> {
        let mut next_active = self.active;

        // Mid-drag, show this container as it _would_ be if the tile were dropped now.
        let shown = tree.preview_tabs(tile_id).cloned().unwrap_or_else(|| {
            // Either nothing is being dragged, or this container is absent from the previewed
            // tree because dropping would collapse it. In the latter case still drop the tile
            // that is leaving, so the remaining tabs close the gap rather than holding a slot
            // open for it.
            let mut shown = self.clone();
            if let Some(dragged) = drop_context.dragged_tile_id {
                shown.children.retain(|&child| child != dragged);
                shown.ensure_active(&tree.tiles);
            }
            shown
        });
        let tab_children = shown.children.clone();

        let tab_bar_height = behavior.tab_bar_height(ui.style());
        let arrow_size = egui::Vec2::splat(tab_bar_height);
        let tab_bar_rect = rect.split_top_bottom_at_y(rect.top() + tab_bar_height).0;
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(tab_bar_rect));

        let mut button_rects = ahash::HashMap::default();
        let mut dragged_index = None;

        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, behavior.tab_bar_color(ui.visuals()));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let scroll_state_id = ui.make_persistent_id(tile_id);
            let mut scroll_state = ui.memory_mut(|m| {
                m.data
                    .get_temp::<ScrollState>(scroll_state_id)
                    .unwrap_or_default()
            });

            // Allow user to add buttons such as "add new tab".
            // They can also read and modify the scroll state if they want.
            behavior.top_bar_right_ui(&tree.tiles, ui, tile_id, self, &mut scroll_state.offset);

            let scroll_area_width = scroll_state.update(ui, arrow_size);

            // We're in a right-to-left layout, so start with the right scroll-arrow:
            let right_arrow_id = ui.make_persistent_id((tile_id, "right_scroll_arrow"));
            scroll_state.right_arrow(ui, arrow_size, right_arrow_id);

            ui.allocate_ui_with_layout(
                ui.available_size(),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    // Left custom slot — first call in this LTR child layout
                    // means leftmost on screen, so it sits to the left of the
                    // left scroll-arrow.
                    behavior.tab_bar_left_ui(&tree.tiles, ui, tile_id, self);

                    let left_arrow_id = ui.make_persistent_id((tile_id, "left_scroll_arrow"));
                    scroll_state.left_arrow(ui, arrow_size, left_arrow_id);

                    // Clamp the precomputed width so it can't exceed what's
                    // left after the leading slot + left arrow consumed space
                    // inside this LTR child ui.
                    let scroll_area_width = scroll_area_width.min(ui.available_width()).max(0.0);

                    // Prepare to show the scroll area with the tabs:

                    scroll_state.offset = scroll_state
                        .offset
                        .at_most(scroll_state.content_size.x - ui.available_width());
                    scroll_state.offset = scroll_state.offset.at_least(0.0);

                    let scroll_area = egui::ScrollArea::horizontal()
                        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                        .max_width(scroll_area_width)
                        .auto_shrink([false; 2])
                        .horizontal_scroll_offset(scroll_state.offset);

                    let output = scroll_area.show(ui, |ui| {
                        if !tree.is_root(tile_id)
                            && behavior.is_tile_draggable(&tree.tiles, tile_id)
                        {
                            // Make the background behind the buttons draggable (to drag the parent container tile).
                            // We also sense clicks to avoid eager-dragging on mouse-down.
                            let sense = egui::Sense::click_and_drag();
                            if ui
                                .interact(ui.max_rect(), ui.scope_id().with("background"), sense)
                                .on_hover_cursor(egui::CursorIcon::Grab)
                                .drag_started()
                            {
                                behavior.on_edit(EditAction::TileDragged);
                                ui.set_dragged_id(tile_id.egui_id(tree.id));
                            }
                        }

                        ui.spacing_mut().item_spacing.x = 0.0; // Tabs have spacing built-in

                        for (i, &child_id) in tab_children.iter().enumerate() {
                            if !tree.is_visible_in_layout(child_id) {
                                continue;
                            }

                            let is_being_dragged = is_being_dragged(ui, tree.id, child_id);

                            let selected = shown.is_active(child_id);
                            let id = child_id.egui_id(tree.id);
                            let tab_state = TabState {
                                active: selected,
                                is_being_dragged,
                                closable: behavior.is_tab_closable(&tree.tiles, child_id),
                            };

                            let response =
                                behavior.tab_ui(&mut tree.tiles, ui, id, child_id, &tab_state);

                            if response.clicked() {
                                behavior.on_edit(EditAction::TabSelected);
                                next_active = Some(child_id);
                            }

                            if let Some(mouse_pos) = drop_context.mouse_pos
                                && drop_context.dragged_tile_id.is_some()
                                && response.rect.contains(mouse_pos)
                            {
                                // Expand this tab - maybe the user wants to drop something into it!
                                behavior.on_edit(EditAction::TabSelected);
                                next_active = Some(child_id);
                            }

                            button_rects.insert(child_id, response.rect);
                            if is_being_dragged {
                                dragged_index = Some(i);
                            }
                        }

                        // Allow the user to add a trailing widget after the last tab
                        // (e.g. a "➕" button), inside the tab scroll area's flow.
                        behavior.tab_bar_trailing_ui(&tree.tiles, ui, tile_id, self);
                    });

                    scroll_state.offset = output.state.clamped_offset().x;
                    scroll_state.content_size = output.content_size;
                    scroll_state.available = output.inner_rect.size();
                },
            );

            ui.data_mut(|data| data.insert_temp(scroll_state_id, scroll_state));
        });

        // -----------
        // Drop zones:

        let preview_thickness = 6.0;
        let after_rect = |rect: Rect| {
            let dragged_size = if let Some(dragged_index) = dragged_index {
                tab_children
                    .get(dragged_index)
                    .and_then(|id| button_rects.get(id))
                    .map(|r| r.size())
                    .unwrap_or_else(|| rect.size())
            } else {
                rect.size() // guess that the size is the same as the last button
            };
            Rect::from_min_size(
                rect.right_top() + vec2(ui.spacing().item_spacing.x, 0.0),
                dragged_size,
            )
        };
        super::linear::drop_zones(
            preview_thickness,
            &tab_children,
            dragged_index,
            super::LinearDir::Horizontal,
            |tile_id| button_rects.get(&tile_id).copied(),
            |rect, i| {
                drop_context.suggest_rect(
                    InsertionPoint::new(tile_id, ContainerInsertion::Tabs(i)),
                    rect,
                );
            },
            after_rect,
        );

        next_active
    }

    pub(super) fn simplify_children(&mut self, mut simplify: impl FnMut(TileId) -> SimplifyAction) {
        self.children.retain_mut(|child| match simplify(*child) {
            SimplifyAction::Remove => {
                // The tab being removed may be the open one, and this is the only place that
                // still knows it happened. The `Replace` arm below already carries `active`
                // across; leaving it out here means `simplify` can return a container whose open
                // tab is a tile that no longer exists anywhere in the tree.
                //
                // `None` rather than "the next tab": which tab to open instead is a question for
                // whoever shows the container (`ensure_active` answers it at layout time from
                // what is visible), while "the open tab is gone" is a fact this pass knows.
                if self.active == Some(*child) {
                    self.active = None;
                }
                false
            }
            SimplifyAction::Keep => true,
            SimplifyAction::Replace(new) => {
                if self.active == Some(*child) {
                    self.active = Some(new);
                }
                *child = new;
                true
            }
        });
    }

    /// Returns child index, if found.
    pub(crate) fn remove_child(&mut self, needle: TileId) -> Option<usize> {
        let index = self.children.iter().position(|&child| child == needle)?;
        self.children.remove(index);
        Some(index)
    }
}

// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::{Container, SimplificationOptions, Tile, Tiles, Tree};

    /// `simplify` must let go of a tab it removes.
    ///
    /// Pruning an empty container is routine, and it can be the container's *open* tab.
    /// [`Tabs::simplify_children`] carries `active` across a `Replace` but not across a `Remove`,
    /// so `simplify` can hand back a tab container whose open tab is a tile that is no longer in
    /// the tree. `Tabs::layout` papers over it on the next frame, but everything that looks at the
    /// tree in between - deciding which pane to reveal, taking a snapshot, writing a save - sees
    /// the dangling id, and a save puts it on disk.
    #[test]
    fn simplify_lets_go_of_a_tab_it_removes() {
        let mut tiles = Tiles::default();
        let empty = tiles.insert_horizontal_tile(vec![]);
        let pane = tiles.insert_pane("keep");
        // Two survivors, not one: with a single child left, `prune_single_child_tabs` would
        // dissolve the tab container itself and take the damaged field with it - a scene that
        // passes whether or not the bug is there.
        let other = tiles.insert_pane("keep too");
        let root = tiles.insert_tab_tile(vec![empty, pane, other]);
        let mut tree = Tree::new(egui::Id::unique("simplify_active"), root, tiles);

        match tree.tiles.get(root) {
            Some(Tile::Container(Container::Tabs(tabs))) => assert_eq!(
                tabs.active,
                Some(empty),
                "setup: the container about to be pruned is the open tab"
            ),
            other => panic!("expected a tab container, got {other:?}"),
        }

        tree.simplify(&SimplificationOptions::default());

        assert!(
            tree.tiles.get(empty).is_none(),
            "the empty container should have been pruned"
        );
        match tree.tiles.get(root) {
            Some(Tile::Container(Container::Tabs(tabs))) => {
                if let Some(active) = tabs.active {
                    assert!(
                        tree.tiles.get(active).is_some(),
                        "the open tab must be a tile that still exists"
                    );
                    assert!(
                        tabs.children.contains(&active),
                        "the open tab must be one of the container's own tabs"
                    );
                }
            }
            other => panic!("expected a tab container, got {other:?}"),
        }
    }
}
