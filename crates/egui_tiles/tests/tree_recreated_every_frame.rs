//! Tests the pattern in `examples/tree_recreated_every_frame.rs`: an app that owns its own layout
//! model and re-creates the [`Tree`] from scratch every frame, with `TileId`s derived from its own
//! stable ids rather than from `Tiles::insert_new`'s counter.
//!
//! Two constraints fall out of that, and both are easy to break:
//!
//! 1. Any state that has to outlive a frame cannot live in the `Tree` — it gets thrown away.
//! 2. Anything that edits the tree mid-frame and expects to put it back has to cope with the tree
//!    being discarded and rebuilt underneath it.
//!
//! Breaking either tends to show up as a silently lost pane rather than a compile error, so: drag
//! panes around and check that none go missing.

use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};

use egui_kittest::{Harness, kittest::Queryable as _};

use egui_tiles::{
    Behavior, Container, ContainerKind, EditAction, Tile, TileId, Tiles, Tree, UiResponse,
};

// ----------------------------------------------------------------------------
// The app's own source of truth — analogous to Rerun's blueprint.

/// A node in the app's own layout model, identified by a stable string id.
#[derive(Clone)]
enum Node {
    Pane {
        id: String,
    },
    Container {
        id: String,
        kind: ContainerKind,
        children: Vec<Self>,
    },
}

impl Node {
    fn pane(id: impl Into<String>) -> Self {
        Self::Pane { id: id.into() }
    }

    fn container(id: impl Into<String>, kind: ContainerKind, children: Vec<Self>) -> Self {
        Self::Container {
            id: id.into(),
            kind,
            children,
        }
    }

    /// The panes this layout holds, sorted — the order changes as tiles get rearranged,
    /// but the set of panes must not.
    fn sorted_pane_ids(&self) -> Vec<&str> {
        fn collect<'a>(node: &'a Node, out: &mut Vec<&'a str>) {
            match node {
                Node::Pane { id } => out.push(id.as_str()),
                Node::Container { children, .. } => {
                    for child in children {
                        collect(child, out);
                    }
                }
            }
        }

        let mut ids = Vec::new();
        collect(self, &mut ids);
        ids.sort_unstable();
        ids
    }

    /// A compact rendering of the layout, for assertion messages.
    fn describe(&self) -> String {
        match self {
            Self::Pane { id } => id.clone(),
            Self::Container { kind, children, .. } => format!(
                "{kind:?}[{}]",
                children
                    .iter()
                    .map(Self::describe)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Everything that has to survive from one frame to the next. Notably *not* the [`Tree`].
#[derive(Default)]
struct App {
    blueprint: Blueprint,
    behavior: TestBehavior,
}

struct Blueprint {
    root: Node,
    /// Monotonic source of fresh ids for containers the tree grew that we've never seen.
    next_generated_id: u64,
}

impl Default for Blueprint {
    fn default() -> Self {
        // Nested containers of three different kinds, so the drops below land in meaningfully
        // different places:
        //
        //   Horizontal[ pane_a, Vertical[ pane_b, Tabs[ pane_c, pane_d ] ] ]
        Self {
            root: Node::container(
                "root",
                ContainerKind::Horizontal,
                vec![
                    Node::pane("pane_a"),
                    Node::container(
                        "right_column",
                        ContainerKind::Vertical,
                        vec![
                            Node::pane("pane_b"),
                            Node::container(
                                "bottom_tabs",
                                ContainerKind::Tabs,
                                vec![Node::pane("pane_c"), Node::pane("pane_d")],
                            ),
                        ],
                    ),
                ],
            ),
            next_generated_id: 0,
        }
    }
}

/// Stable [`TileId`] from an app id — deterministic, so re-creating the tree yields the same ids.
fn tile_id(app_id: &str) -> TileId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app_id.hash(&mut hasher);
    TileId::from_u64(hasher.finish())
}

impl Blueprint {
    /// Build a fresh [`Tree`] from the blueprint, plus a `TileId -> app id` reverse map.
    fn to_tree(&self) -> (Tree<String>, HashMap<TileId, String>) {
        fn insert(
            node: &Node,
            tiles: &mut Tiles<String>,
            reverse: &mut HashMap<TileId, String>,
        ) -> TileId {
            match node {
                Node::Pane { id } => {
                    let tile = tile_id(id);
                    tiles.insert(tile, Tile::Pane(id.clone()));
                    // Deliberately NOT in `reverse`; see the note on `sync_from_tree`.
                    tile
                }
                Node::Container { id, kind, children } => {
                    let child_ids = children.iter().map(|c| insert(c, tiles, reverse)).collect();
                    let tile = tile_id(id);
                    tiles.insert(tile, Tile::Container(Container::new(*kind, child_ids)));
                    reverse.insert(tile, id.clone());
                    tile
                }
            }
        }

        let mut tiles = Tiles::default();
        let mut reverse = HashMap::new();
        let root = insert(&self.root, &mut tiles, &mut reverse);
        (
            Tree::new(egui::Id::unique("test_tree"), root, tiles),
            reverse,
        )
    }

    /// Fold an edited tree back into the blueprint, minting fresh ids for any newly-created
    /// containers — exactly what the app does after a drop.
    ///
    /// `reverse` deliberately holds **containers only**: a `TileId` does not keep referring to
    /// the same kind of tile. Dropping a pane onto another pane makes `egui_tiles` reuse the
    /// target's id for the container it wraps them in, so an id that meant `"pane_a"` one frame
    /// means `"the container holding pane_a"` the next. Handing that container the app id
    /// `"pane_a"` would make the next `to_tree()` insert a pane and a container at the same id,
    /// silently losing a pane. See `examples/tree_recreated_every_frame.rs`.
    fn sync_from_tree(&mut self, tree: &Tree<String>, reverse: &HashMap<TileId, String>) {
        fn rebuild(
            tile_id: TileId,
            tree: &Tree<String>,
            reverse: &HashMap<TileId, String>,
            next: &mut u64,
        ) -> Option<Node> {
            match tree.tiles.get(tile_id)? {
                Tile::Pane(app_id) => Some(Node::pane(app_id.clone())),
                Tile::Container(container) => {
                    let id = reverse.get(&tile_id).cloned().unwrap_or_else(|| {
                        let generated = format!("generated_container_{next}");
                        *next += 1;
                        generated
                    });
                    let children = container
                        .children()
                        .filter_map(|&c| rebuild(c, tree, reverse, next))
                        .collect();
                    Some(Node::container(id, container.kind(), children))
                }
            }
        }

        if let Some(root) = tree.root
            && let Some(node) = rebuild(root, tree, reverse, &mut self.next_generated_id)
        {
            self.root = node;
        }
    }
}

// ----------------------------------------------------------------------------

/// Counts the drops `egui_tiles` actually committed, so the assertions below can tell a
/// successful drag from one that quietly did nothing.
#[derive(Default)]
struct TestBehavior {
    tiles_dropped: usize,
}

/// The accessibility label of the drag handle inside a pane, used to find it from the test.
fn drag_handle_label(pane: &str) -> String {
    format!("drag {pane}")
}

impl Behavior<String> for TestBehavior {
    fn on_edit(&mut self, action: EditAction) {
        if action == EditAction::TileDropped {
            self.tiles_dropped += 1;
        }
    }

    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        pane.clone().into()
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut String) -> UiResponse {
        let handle = egui::Button::new(drag_handle_label(pane)).sense(egui::Sense::drag());
        if ui.add(handle).drag_started() {
            UiResponse::DragStarted
        } else {
            UiResponse::None
        }
    }
}

// ----------------------------------------------------------------------------

/// Dragging a pane and dropping it must never lose a pane, however the tree gets rearranged.
///
/// The drag is driven through the real widgets — press the pane's drag handle, move the pointer,
/// release — rather than by poking `egui_tiles` internals, so this exercises the same path a user
/// would take.
/// Build a harness around a fresh app.
fn harness() -> egui_kittest::Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            |ui, app: &mut App| {
                // Re-create the tree from the app's own model every frame:
                let (mut tree, reverse) = app.blueprint.to_tree();
                tree.ui(&mut app.behavior, ui);
                // Persist any edit back, so it survives the next rebuild:
                app.blueprint.sync_from_tree(&tree, &reverse);
            },
            App::default(),
        );
    harness.run();
    harness
}

/// The widgets a pane can be grabbed by, or dropped onto: the drag handle inside the pane, and
/// the pane's tab when it sits in a tab bar.
///
/// Both are found by accessibility label, which is why a tab needs one.
fn grab_points(pane: &str) -> [String; 2] {
    [drag_handle_label(pane), pane.to_owned()]
}

/// Drag whatever is labelled `source_label` onto whatever is labelled `target_label`, and let go.
///
/// The drag goes through the real widgets — find them by accessibility label, press, move the
/// pointer, release — rather than by poking `egui_tiles` internals, so this exercises the path a
/// user actually takes. Returns `false` if either widget is not on screen, e.g. the drag handle
/// inside an inactive tab.
fn drag_onto(
    harness: &mut egui_kittest::Harness<'_, App>,
    source_label: &str,
    target_label: &str,
) -> bool {
    let Some(from) = harness.query_by_label(source_label).map(|node| node.rect()) else {
        return false;
    };
    let Some(to) = harness.query_by_label(target_label).map(|node| node.rect()) else {
        return false;
    };

    harness.drag_at(from.center());
    harness.step();
    harness.hover_at(to.center());
    // The drop zone is only settled once the tile has been hovered for a frame or two.
    for _ in 0..4 {
        harness.step();
    }
    harness.drop_at(to.center());
    harness.step();
    harness.run();

    true
}

const PANES: [&str; 4] = ["pane_a", "pane_b", "pane_c", "pane_d"];

fn all_panes() -> String {
    PANES.join(", ")
}

/// Every way of dropping one pane onto another must leave all four panes in place.
///
/// Each pane can be grabbed, and aimed at, in two ways: by the drag handle inside it, and by its
/// tab when it sits in a tab bar. Dragging *by the tab* is the only way to move a pane that is not
/// the open tab, so without it a quarter of the tree is untouchable from a test.
#[test]
fn dropping_a_pane_never_loses_it() {
    let mut dragged = 0;
    let mut skipped = 0;

    for source in PANES {
        for source_label in grab_points(source) {
            for target in PANES {
                if source == target {
                    continue;
                }

                for target_label in grab_points(target) {
                    let mut harness = harness();
                    let before = harness.state().blueprint.root.describe();

                    if !drag_onto(&mut harness, &source_label, &target_label) {
                        // Not on screen: the drag handle inside a tab that isn't open.
                        skipped += 1;
                        continue;
                    }
                    dragged += 1;

                    let app = harness.state();

                    assert_eq!(
                        app.blueprint.root.sorted_pane_ids().join(", "),
                        all_panes(),
                        "dragging {source_label} onto {target_label} lost a pane: {before} -> {}",
                        app.blueprint.root.describe()
                    );

                    // Otherwise the check above would pass by never having dragged anything.
                    assert_eq!(
                        app.behavior.tiles_dropped, 1,
                        "dragging {source_label} onto {target_label} committed no drop"
                    );
                }
            }
        }
    }

    assert!(
        0 < dragged,
        "no drag ran at all ({skipped} target(s) were off screen)"
    );
}

/// The same, but on one long-lived app, so that whatever a drop leaves behind is carried into the
/// next one. Losing a pane only on the fifth drag still loses a pane.
#[test]
fn many_drags_in_a_row_never_lose_a_pane() {
    let mut harness = harness();
    let mut drags = 0;
    let mut dropped_before = 0;

    for round in 0..PANES.len() {
        for (i, source) in PANES.iter().enumerate() {
            let target = PANES[(i + 1 + round) % PANES.len()];
            if *source == target {
                continue;
            }

            for source_label in grab_points(source) {
                for target_label in grab_points(target) {
                    let before = harness.state().blueprint.root.describe();
                    if !drag_onto(&mut harness, &source_label, &target_label) {
                        continue;
                    }
                    drags += 1;

                    let app = harness.state();
                    assert_eq!(
                        app.blueprint.root.sorted_pane_ids().join(", "),
                        all_panes(),
                        "drag {drags} ({source_label} onto {target_label}) lost a pane: \
                         {before} -> {}",
                        app.blueprint.root.describe()
                    );
                    assert_eq!(
                        app.behavior.tiles_dropped,
                        dropped_before + 1,
                        "drag {drags} ({source_label} onto {target_label}) committed no drop"
                    );
                    dropped_before = app.behavior.tiles_dropped;
                }
            }
        }
    }

    assert!(0 < drags, "no drag ran at all");
}
