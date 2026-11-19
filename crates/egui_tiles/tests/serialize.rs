#![cfg(feature = "serde")]

use egui_tiles::{Tiles, Tree};

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Pane {
    nr: usize,
}

fn create_tree() -> Tree<Pane> {
    let mut next_view_nr = 0;
    let mut gen_pane = || {
        let pane = Pane { nr: next_view_nr };
        next_view_nr += 1;
        pane
    };

    let mut tiles = Tiles::default();

    let mut tabs = vec![];
    tabs.push({
        let children = (0..7).map(|_| tiles.insert_pane(gen_pane())).collect();
        tiles.insert_horizontal_tile(children)
    });
    tabs.push({
        let cells = (0..11).map(|_| tiles.insert_pane(gen_pane())).collect();
        tiles.insert_grid_tile(cells)
    });
    tabs.push(tiles.insert_pane(gen_pane()));

    let root = tiles.insert_tab_tile(tabs);

    Tree::new(egui::Id::unique("my_tree"), root, tiles)
}

#[test]
fn test_serialize_json() {
    let original = create_tree();
    let json = serde_json::to_string(&original).expect("json serialize");
    let restored = serde_json::from_str(&json).expect("json deserialize");
    assert_eq!(original, restored, "JSON did not round-trip");
}

#[test]
fn test_serialize_ron() {
    let original = create_tree();
    let ron = ron::to_string(&original).expect("ron serialize");
    let restored = ron::from_str(&ron).expect("ron deserialize");
    assert_eq!(original, restored, "RON did not round-trip");
}
