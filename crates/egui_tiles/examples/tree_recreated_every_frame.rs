//! Reproduces how Rerun (and similar apps) drive `egui_tiles`:
//!
//! The tree is NOT the source of truth. Instead the app keeps its own model (here `Blueprint`)
//! and **re-creates the [`egui_tiles::Tree`] from scratch every single frame**, assigning each
//! tile a stable [`TileId`] derived from a hash of the app's own id — never from
//! `Tiles::insert_new`'s running counter. After [`Tree::ui`], any edit is read back out of the
//! tree and folded into the `Blueprint`, so it persists into the next frame's freshly-built tree.
//!
//! This setup is easy for `egui_tiles` to get wrong: anything that edits the tree mid-frame and
//! expects to put it back the way it was has to survive the tree being thrown away and rebuilt
//! underneath it. Run this, drag panes around, and watch the "panes" counter in the top bar —
//! it must stay constant.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};

use egui_tiles::{Container, ContainerKind, Tile, TileId, Tiles, Tree};

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

    fn count_panes(&self) -> usize {
        match self {
            Self::Pane { .. } => 1,
            Self::Container { children, .. } => children.iter().map(Self::count_panes).sum(),
        }
    }
}

struct Blueprint {
    root: Node,
    /// Monotonic source for fresh ids when the tree grows a container we've never seen.
    next_generated_id: u64,
}

impl Default for Blueprint {
    fn default() -> Self {
        // Nested containers of three different kinds, so that dragging things around exercises
        // the interesting cases: splitting a pane, joining a linear container, moving a tile
        // between containers, and dropping into a tab bar.
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
    /// Build a fresh [`Tree`] from the blueprint, and a `TileId -> app id` reverse map.
    ///
    /// Mirrors Rerun: tiles get hash-based ids via [`Tiles::insert`], so `next_tile_id` stays 0.
    fn to_tree(&self) -> (Tree<String>, HashMap<TileId, String>) {
        let mut tiles = Tiles::default();
        let mut reverse = HashMap::new();

        fn insert(
            node: &Node,
            tiles: &mut Tiles<String>,
            reverse: &mut HashMap<TileId, String>,
        ) -> TileId {
            match node {
                Node::Pane { id } => {
                    let id_ = tile_id(id);
                    tiles.insert(id_, Tile::Pane(id.clone()));
                    // Deliberately NOT in `reverse`: a pane carries its own app id as its
                    // payload, and `reverse` is only consulted for containers. Putting panes in
                    // here is a trap -- see the note on `sync_from_tree`.
                    id_
                }
                Node::Container { id, kind, children } => {
                    let child_ids = children.iter().map(|c| insert(c, tiles, reverse)).collect();
                    let container = Container::new(*kind, child_ids);
                    let id_ = tile_id(id);
                    tiles.insert(id_, Tile::Container(container));
                    reverse.insert(id_, id.clone());
                    id_
                }
            }
        }

        let root = insert(&self.root, &mut tiles, &mut reverse);
        (
            Tree::new(egui::Id::unique("rerun_style_tree"), root, tiles),
            reverse,
        )
    }

    /// Fold an edited tree back into the blueprint, minting fresh ids for any newly-created
    /// containers (tiles whose id isn't in `reverse`) — exactly what Rerun does on a drop.
    ///
    /// ## Watch out
    ///
    /// `reverse` maps a `TileId` back to an app id, and it deliberately contains **containers
    /// only**. Panes are identified by their own payload instead.
    ///
    /// That matters because a `TileId` does not necessarily keep referring to the same *kind* of
    /// tile. `egui_tiles` inserts containers of its own accord — with `all_panes_must_have_tabs`
    /// it wraps every pane in a tab container, re-using the pane's id for the container and
    /// moving the pane itself to a fresh id. So an id that meant `"pane_a"` one frame can mean
    /// "the container holding `pane_a`" the next.
    ///
    /// If panes were in `reverse`, such a container would be handed the app id `"pane_a"`, and
    /// the next `to_tree()` would insert both a pane and a container at `tile_id("pane_a")` — the
    /// second overwriting the first, silently losing a pane.
    fn sync_from_tree(&mut self, tree: &Tree<String>, reverse: &HashMap<TileId, String>) {
        fn rebuild(
            tile_id: TileId,
            tree: &Tree<String>,
            reverse: &HashMap<TileId, String>,
            next: &mut u64,
        ) -> Option<Node> {
            match tree.tiles.get(tile_id)? {
                Tile::Pane(app_id) => Some(Node::Pane { id: app_id.clone() }),
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
                    Some(Node::Container {
                        id,
                        kind: container.kind(),
                        children,
                    })
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

struct TreeBehavior;

impl egui_tiles::Behavior<String> for TreeBehavior {
    fn tab_title_for_pane(&mut self, pane: &String) -> egui::WidgetText {
        pane.clone().into()
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut String,
    ) -> egui_tiles::UiResponse {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        pane.hash(&mut hasher);
        let hue = (hasher.finish() % 360) as f32 / 360.0;
        let color = egui::epaint::Hsva::new(hue, 0.5, 0.5, 1.0);
        ui.painter().rect_filled(ui.max_rect(), 0.0, color);

        ui.heading(pane.as_str());
        if ui
            .add(egui::Button::new("Drag me!").sense(egui::Sense::drag()))
            .drag_started()
        {
            egui_tiles::UiResponse::DragStarted
        } else {
            egui_tiles::UiResponse::None
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    env_logger::init(); // `Tile #N not found`-style warnings show up here with `RUST_LOG=warn`.

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([700.0, 500.0]),
        ..Default::default()
    };

    let mut blueprint = Blueprint::default();

    eframe::run_ui_native(
        "egui_tiles: tree re-created every frame",
        options,
        move |ui, _frame| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("panes: {}", blueprint.root.count_panes()));
                    ui.label("← must stay constant while you drag panes around");
                    if ui.button("Reset").clicked() {
                        blueprint = Blueprint::default();
                    }
                });
                ui.separator();

                // Rebuild the tree from the blueprint every frame:
                let (mut tree, reverse) = blueprint.to_tree();

                let mut behavior = TreeBehavior;
                tree.ui(&mut behavior, ui);

                // Fold any edit back into the blueprint so it survives the next rebuild:
                blueprint.sync_from_tree(&tree, &reverse);
            });
        },
    )
}
