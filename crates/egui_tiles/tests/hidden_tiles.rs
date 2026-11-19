//! Hiding a tile should free up the space it took, all the way up through the containers that
//! held nothing else.
//!
//! `all_panes_must_have_tabs` is what makes this easy to get wrong: every pane sits alone in a
//! tab container, so hiding the pane leaves the tab container as the thing occupying the slot.

use egui_tiles::{Behavior, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};

struct TestBehavior;

impl Behavior<&'static str> for TestBehavior {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _id: TileId, pane: &mut &'static str) -> UiResponse {
        ui.label(*pane);
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
        (*pane).into()
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

const VIEWPORT: egui::Vec2 = egui::vec2(800.0, 600.0);

/// What `TestBehavior` leaves between two tiles, since it keeps the default `Behavior::gap_width`.
const GAP_WIDTH: f32 = 1.0;

/// Two panes side by side, each wrapped in its own tab container by `all_panes_must_have_tabs`.
fn harness() -> egui_kittest::Harness<'static, Tree<&'static str>> {
    let mut tiles = Tiles::default();
    let a = tiles.insert_pane("a");
    let b = tiles.insert_pane("b");
    let root = tiles.insert_horizontal_tile(vec![a, b]);
    let tree = Tree::new(egui::Id::unique("test"), root, tiles);

    let mut harness = egui_kittest::Harness::builder()
        .with_size(VIEWPORT)
        .build_ui_state(
            |ui, tree: &mut Tree<&'static str>| {
                tree.ui(&mut TestBehavior, ui);
            },
            tree,
        );
    harness.run();
    harness
}

/// The pane with the given name, or `None` if it is gone.
fn pane(tree: &Tree<&'static str>, name: &str) -> Option<TileId> {
    tree.tiles
        .iter()
        .find(|(_, tile)| matches!(tile, Tile::Pane(pane) if *pane == name))
        .map(|(tile_id, _)| *tile_id)
}

/// The rect the pane's slot in the root container was laid out in, tab bar and all,
/// or `None` if it was never laid out.
fn slot_rect(tree: &Tree<&'static str>, name: &str) -> Option<egui::Rect> {
    let pane_id = pane(tree, name).expect("the pane should still be in the tree");
    let slot = tree
        .tiles
        .parent_of(pane_id)
        .expect("`all_panes_must_have_tabs` should have given the pane a tab container");
    tree.tiles.rect(slot)
}

/// The width the pane's slot takes up in the root container, tab bar and all.
fn slot_width(tree: &Tree<&'static str>, name: &str) -> f32 {
    slot_rect(tree, name)
        .expect("the slot should have been laid out")
        .width()
}

/// The width the whole tree was laid out in.
fn tree_width(tree: &Tree<&'static str>) -> f32 {
    tree.tiles
        .rect(tree.root().expect("the tree should have a root"))
        .expect("the root should have been laid out")
        .width()
}

#[test]
fn hiding_a_pane_gives_its_space_to_its_sibling() {
    let mut harness = harness();

    let whole_width = tree_width(harness.state());
    let split_width = (whole_width - GAP_WIDTH) / 2.0;
    assert_eq!(
        slot_width(harness.state(), "b"),
        split_width,
        "the two panes should split the {whole_width} the tree was laid out in"
    );

    let hidden = pane(harness.state(), "a").expect("pane `a`");
    harness.state_mut().set_visible(hidden, false);
    harness.run();

    assert_eq!(
        slot_width(harness.state(), "b"),
        whole_width,
        "`b` is the only thing left to show, so it should have the whole width"
    );
    assert_eq!(
        slot_rect(harness.state(), "a"),
        None,
        "the tab container around the hidden pane should not be laid out"
    );

    harness.state_mut().set_visible(hidden, true);
    harness.run();

    assert_eq!(
        slot_width(harness.state(), "b"),
        split_width,
        "showing the pane again should restore the split"
    );
}

/// A tile that shows nothing is not among the active tiles either, so an app asking what the
/// user can see does not hear about the container wrapped around a hidden pane.
#[test]
fn a_hidden_pane_leaves_no_active_tiles_behind() {
    let mut harness = harness();

    let hidden = pane(harness.state(), "a").expect("pane `a`");
    harness.state_mut().set_visible(hidden, false);
    harness.run();

    let tree = harness.state();
    let active = tree.active_tiles();
    let visible_panes: Vec<_> = active
        .iter()
        .filter_map(|&tile_id| match tree.tiles.get(tile_id) {
            Some(Tile::Pane(pane)) => Some(*pane),
            _ => None,
        })
        .collect();

    assert_eq!(visible_panes, vec!["b"]);
    assert!(
        !active.contains(&hidden)
            && !active.contains(&tree.tiles.parent_of(hidden).expect("the tab container")),
        "neither the hidden pane nor the container around it should be active: {active:?}"
    );
}
