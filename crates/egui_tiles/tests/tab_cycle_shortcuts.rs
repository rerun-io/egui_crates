//! `Ctrl+Tab` and `Ctrl+Shift+Tab` cycle the active tab of the hovered tab container.

use egui::{Key, Modifiers};
use egui_tiles::{Behavior, Container, Tile, TileId, Tiles, Tree, UiResponse};

struct TestBehavior;

impl Behavior<&'static str> for TestBehavior {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _id: TileId, pane: &mut &'static str) -> UiResponse {
        ui.label(*pane);
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
        (*pane).into()
    }
}

const VIEWPORT: egui::Vec2 = egui::vec2(800.0, 600.0);

/// A tab container with three panes, `a` active, shown in a viewport of [`VIEWPORT`] size.
fn harness() -> egui_kittest::Harness<'static, Tree<&'static str>> {
    let mut tiles = Tiles::default();
    let a = tiles.insert_pane("a");
    let b = tiles.insert_pane("b");
    let c = tiles.insert_pane("c");
    let root = tiles.insert_tab_tile(vec![a, b, c]);
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

/// The name of the active tab of the root tab container.
fn active_tab(tree: &Tree<&'static str>) -> &'static str {
    let root = tree.root().expect("the tree should have a root");
    let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get(root) else {
        panic!("the root should be a tab container");
    };
    let active = tabs.active.expect("a tab should be active");
    match tree.tiles.get(active) {
        Some(Tile::Pane(pane)) => pane,
        other => panic!("expected a pane, got {other:?}"),
    }
}

fn pane(tree: &Tree<&'static str>, name: &str) -> TileId {
    tree.tiles
        .iter()
        .find(|(_, tile)| matches!(tile, Tile::Pane(pane) if *pane == name))
        .map(|(tile_id, _)| *tile_id)
        .expect("the pane should be in the tree")
}

#[test]
fn ctrl_tab_cycles_forward_and_wraps() {
    let mut harness = harness();
    harness.hover_at(VIEWPORT.to_pos2() / 2.0);
    harness.run();
    assert_eq!(active_tab(harness.state()), "a");

    for expected in ["b", "c", "a"] {
        harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
        harness.run();
        assert_eq!(active_tab(harness.state()), expected);
    }
}

#[test]
fn ctrl_shift_tab_cycles_backward_and_wraps() {
    let mut harness = harness();
    harness.hover_at(VIEWPORT.to_pos2() / 2.0);
    harness.run();

    for expected in ["c", "b", "a"] {
        harness.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab);
        harness.run();
        assert_eq!(active_tab(harness.state()), expected);
    }
}

#[test]
fn cycling_skips_hidden_tabs() {
    let mut harness = harness();
    let hidden = pane(harness.state(), "b");
    harness.state_mut().set_visible(hidden, false);
    harness.hover_at(VIEWPORT.to_pos2() / 2.0);
    harness.run();

    harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    harness.run();
    assert_eq!(
        active_tab(harness.state()),
        "c",
        "the hidden `b` should be skipped"
    );
}

#[test]
fn plain_tab_leaves_the_active_tab_alone() {
    let mut harness = harness();
    harness.hover_at(VIEWPORT.to_pos2() / 2.0);
    harness.run();

    harness.key_press(Key::Tab);
    harness.run();
    assert_eq!(active_tab(harness.state()), "a");
}

#[test]
fn ctrl_tab_does_nothing_when_the_pointer_is_elsewhere() {
    let mut harness = harness();
    harness.hover_at(egui::pos2(-10.0, -10.0));
    harness.run();

    harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    harness.run();
    assert_eq!(active_tab(harness.state()), "a");
}
