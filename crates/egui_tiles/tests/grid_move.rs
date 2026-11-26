//! [`Tree::move_tile_to_container`] into a grid that has a hole.
//!
//! Dragging a tile out of a grid leaves a hole behind, and the hole stays until the grid decides
//! to collapse it. The insertion index must not be thrown off by it.

use egui_tiles::{Container, Tile, TileId, Tiles, Tree};

struct Setup {
    tree: Tree<&'static str>,
    root: TileId,
    grid: TileId,
    a: TileId,
    b: TileId,
    c: TileId,
    d: TileId,
    e: TileId,
}

/// A horizontal root holding a grid `[a, b, c, d]` and a pane `e`.
fn setup() -> Setup {
    let mut tiles = Tiles::default();
    let a = tiles.insert_pane("a");
    let b = tiles.insert_pane("b");
    let c = tiles.insert_pane("c");
    let d = tiles.insert_pane("d");
    let e = tiles.insert_pane("e");
    let grid = tiles.insert_grid_tile(vec![a, b, c, d]);
    let root = tiles.insert_horizontal_tile(vec![grid, e]);
    let tree = Tree::new(egui::Id::unique("grid_move"), root, tiles);
    Setup {
        tree,
        root,
        grid,
        a,
        b,
        c,
        d,
        e,
    }
}

fn grid_children(tree: &Tree<&'static str>, grid: TileId) -> Vec<TileId> {
    match tree.tiles.get(grid) {
        Some(Tile::Container(Container::Grid(grid))) => grid.children().copied().collect(),
        _ => panic!("Expected a grid"),
    }
}

/// Move `a` out of the grid, so that the grid is `[hole, b, c, d]`.
fn setup_with_hole() -> Setup {
    let mut setup = setup();
    setup
        .tree
        .move_tile_to_container(setup.a, setup.root, 0, true);
    assert_eq!(
        grid_children(&setup.tree, setup.grid),
        [setup.b, setup.c, setup.d],
        "Moving `a` out should leave the other three behind"
    );
    setup
}

#[test]
fn append_to_grid_with_hole() {
    for reflow_grid in [true, false] {
        let Setup {
            mut tree,
            grid,
            b,
            c,
            d,
            e,
            ..
        } = setup_with_hole();

        tree.move_tile_to_container(e, grid, usize::MAX, reflow_grid);

        assert_eq!(
            grid_children(&tree, grid),
            [b, c, d, e],
            "reflow_grid: {reflow_grid}"
        );
    }
}

#[test]
fn insert_after_last_child_of_grid_with_hole() {
    let Setup {
        mut tree,
        grid,
        b,
        c,
        d,
        e,
        ..
    } = setup_with_hole();

    // In a list of the children, "after the last one" is the number of children.
    tree.move_tile_to_container(e, grid, 3, true);

    assert_eq!(grid_children(&tree, grid), [b, c, d, e]);
}

#[test]
fn insert_between_children_of_grid_with_hole() {
    let Setup {
        mut tree,
        grid,
        b,
        c,
        d,
        e,
        ..
    } = setup_with_hole();

    // Before the second child, as seen in a list of the children.
    tree.move_tile_to_container(e, grid, 1, true);

    assert_eq!(grid_children(&tree, grid), [b, e, c, d]);
}

#[test]
fn reorder_within_grid_with_hole() {
    let Setup {
        mut tree,
        grid,
        b,
        c,
        d,
        ..
    } = setup_with_hole();

    // Move the first child to the end.
    tree.move_tile_to_container(b, grid, 3, true);
    assert_eq!(grid_children(&tree, grid), [c, d, b]);

    // And the last one back to the front.
    tree.move_tile_to_container(b, grid, 0, true);
    assert_eq!(grid_children(&tree, grid), [b, c, d]);
}

#[test]
fn grid_without_hole_is_unaffected() {
    for (index, expected) in [
        (0, "eabcd"),
        (2, "abecd"),
        (4, "abcde"),
        (usize::MAX, "abcde"),
    ] {
        let Setup {
            mut tree, grid, e, ..
        } = setup();

        tree.move_tile_to_container(e, grid, index, true);

        let names: String = grid_children(&tree, grid)
            .into_iter()
            .map(|id| match tree.tiles.get(id) {
                Some(Tile::Pane(name)) => *name,
                _ => panic!("Expected a pane"),
            })
            .collect();
        assert_eq!(names, expected, "index: {index}");
    }
}
