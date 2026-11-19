//! Accessibility-tree tests.

use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable as _};
use egui_tiles::{Behavior, TileId, Tree, UiResponse};

/// Renders each pane as a label.
struct TestBehavior;

impl Behavior<&'static str> for TestBehavior {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut &'static str,
    ) -> UiResponse {
        ui.label(*pane);
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &&'static str) -> egui::WidgetText {
        (*pane).into()
    }
}

/// Tile widgets remain descendants of the `Ui` that contains the tree.
#[test]
fn tile_accessibility_nodes_stay_under_parent_ui() {
    let tree = Tree::new_tabs(egui::Id::unique("test"), vec!["Pane content"]);
    let mut harness = Harness::builder().build_ui_state(
        |ui, tree: &mut Tree<&'static str>| {
            ui.scope(|ui| {
                ui.response().widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Pane, true, "Outer panel")
                });
                tree.ui(&mut TestBehavior, ui);
            });
        },
        tree,
    );
    harness.run();

    let panel = harness.get_by_role_and_label(Role::Pane, "Outer panel");
    panel.get_by_label("Pane content");
}
