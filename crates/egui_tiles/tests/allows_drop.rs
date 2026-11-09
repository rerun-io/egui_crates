//! A [`Behavior`] can turn down drop targets with `allows_drop`, and a turned-down target has to
//! act as if it were not there: the closest target that is allowed takes its place, and with
//! none allowed a release drops nothing.
//!
//! The drag goes through the real widgets - press the pane's drag handle, move the pointer,
//! release - so this covers the whole path from the drop zones to `move_tile`. It also checks
//! the part that is easy to get wrong: the behavior is asked once the whole tree has been shown,
//! when every tile is back in [`Tiles`], so it can look at the target.

use egui_kittest::kittest::Queryable as _;
use egui_tiles::{
    Behavior, ContainerKind, DropTarget, EditAction, Tile, TileId, Tiles, Tree, UiResponse,
};

/// The pane the behavior below has rules for.
const STRIP: &str = "strip";

// ----------------------------------------------------------------------------

/// Lets [`STRIP`] be dropped only into one kind of container - or nowhere at all - and counts
/// the drops `egui_tiles` actually committed, so the assertions below can tell a successful
/// drag from one that quietly did nothing.
struct StripRule {
    allowed_kind: Option<ContainerKind>,
    tiles_dropped: usize,
}

/// The accessibility label of the drag handle inside a pane, used to find it from the test.
fn drag_handle_label(pane: &str) -> String {
    format!("drag {pane}")
}

impl Behavior<&'static str> for StripRule {
    fn on_edit(&mut self, action: EditAction) {
        if action == EditAction::TileDropped {
            self.tiles_dropped += 1;
        }
    }

    fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
        (*pane).into()
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut &'static str,
    ) -> UiResponse {
        let handle = egui::Button::new(drag_handle_label(pane)).sense(egui::Sense::drag());
        if ui.add(handle).drag_started() {
            UiResponse::DragStarted
        } else {
            UiResponse::None
        }
    }

    fn allows_drop(
        &self,
        tiles: &Tiles<&'static str>,
        dragged_tile_id: TileId,
        target: &DropTarget,
    ) -> bool {
        assert!(
            tiles.get(target.parent_id).is_some(),
            "the target has to be in the tree when the behavior is asked about it"
        );
        let is_strip = matches!(tiles.get(dragged_tile_id), Some(Tile::Pane(STRIP)));
        !is_strip || self.allowed_kind == Some(target.kind)
    }
}

// ----------------------------------------------------------------------------

/// The tree under test, and the behavior that keeps count of the drops.
struct App {
    tree: Tree<&'static str>,
    behavior: StripRule,
}

/// Build a harness around pane `a` with the strip to its right.
fn harness(allowed_kind: Option<ContainerKind>) -> egui_kittest::Harness<'static, App> {
    let mut tiles = Tiles::default();
    let a = tiles.insert_pane("a");
    let strip = tiles.insert_pane(STRIP);
    let root = tiles.insert_horizontal_tile(vec![a, strip]);
    let tree = Tree::new(egui::Id::unique("test"), root, tiles);

    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_ui_state(
            |ui, app: &mut App| {
                app.tree.ui(&mut app.behavior, ui);
            },
            App {
                tree,
                behavior: StripRule {
                    allowed_kind,
                    tiles_dropped: 0,
                },
            },
        );
    harness.run();
    harness
}

/// The pane with the given name, or `None` if it is gone.
fn pane(tree: &Tree<&'static str>, name: &'static str) -> Option<TileId> {
    tree.tiles.find_pane(&name)
}

/// Drag the strip by its handle, release it at `to`, and let the tree settle.
///
/// The drag goes through the real widgets - press, move the pointer, release - rather than by
/// poking `egui_tiles` internals, so this exercises the path a user actually takes.
fn drag_strip_to(harness: &mut egui_kittest::Harness<'_, App>, to: egui::Pos2) {
    let from = harness.get_by_label(&drag_handle_label(STRIP)).rect();
    harness.drag_at(from.center());
    harness.step();
    harness.hover_at(to);
    // The drop zone is only settled once the tile has been hovered for a frame or two.
    for _ in 0..4 {
        harness.step();
    }
    harness.drop_at(to);
    harness.step();
    harness.run();
}

// ----------------------------------------------------------------------------

/// A rejected target does not block the drop: the closest target that is allowed takes it.
#[test]
fn a_rejected_target_gives_way_to_the_closest_allowed_one() {
    let mut harness = harness(Some(ContainerKind::Vertical));
    let a = pane(&harness.state().tree, "a").expect("pane `a`");
    let a_rect = harness
        .state()
        .tree
        .tiles
        .rect(a)
        .expect("`a` should be laid out");

    // The left edge of `a` is a horizontal target, which the strip may not take. The closest
    // allowed one is the top half of `a`, so aim a little above the middle to make that clear:
    drag_strip_to(
        &mut harness,
        egui::pos2(a_rect.left() + 8.0, a_rect.center().y - 8.0),
    );

    let app = harness.state();
    assert_eq!(
        app.behavior.tiles_dropped, 1,
        "the strip should have been dropped once"
    );
    let parent = app
        .tree
        .tiles
        .parent_of(a)
        .and_then(|parent_id| app.tree.tiles.get_container(parent_id))
        .expect("`a` should have ended up in a container");
    assert_eq!(
        parent.kind(),
        ContainerKind::Vertical,
        "the strip should have been stacked on `a` instead of put beside it"
    );
    assert_eq!(
        parent.children_vec(),
        vec![pane(&app.tree, STRIP).expect("the strip"), a],
        "the strip should sit above `a`"
    );
}

/// With every target rejected, releasing the tile must leave the tree exactly as it was.
#[test]
fn nothing_is_dropped_when_every_target_is_rejected() {
    let mut harness = harness(None);
    let a = pane(&harness.state().tree, "a").expect("pane `a`");
    let root = harness
        .state()
        .tree
        .root()
        .expect("the tree should have a root");
    let a_rect = harness
        .state()
        .tree
        .tiles
        .rect(a)
        .expect("`a` should be laid out");

    drag_strip_to(
        &mut harness,
        egui::pos2(a_rect.center().x, a_rect.top() + 8.0),
    );

    let app = harness.state();
    assert_eq!(
        app.behavior.tiles_dropped, 0,
        "nothing should have been dropped"
    );
    assert_eq!(
        app.tree.root(),
        Some(root),
        "the root should be the same tile as before"
    );
    assert_eq!(
        app.tree
            .tiles
            .get_container(root)
            .expect("the root should still be a container")
            .children_vec(),
        vec![a, pane(&app.tree, STRIP).expect("the strip")],
        "the strip should still sit to the right of `a`"
    );
}
