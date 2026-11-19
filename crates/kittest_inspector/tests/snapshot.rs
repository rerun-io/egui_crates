//! Snapshot test that launches `InspectorApp` through `egui_kittest`'s eframe harness and
//! feeds it a synthetic `Frame` over the worker channel — the same channel that
//! `kittest_inspector`'s IO thread uses in production.
//!
//! No real harness is attached, so nothing is listening on the release channel; we keep a
//! reference to the receiver alive for the duration of the test so `release_tx.send(...)`
//! stays a no-op rather than failing (which would stall `send_release`).

use std::sync::mpsc;

use egui::accesskit::{Node, NodeId, Rect as AkRect, Role, Tree, TreeId, TreeUpdate};
use egui_kittest::Harness;
use egui_kittest::inspector_api::{Frame, SourceView};
use kittest_inspector::{InspectorApp, WorkerEvent};

/// Build a representative `Frame` — solid-ish RGBA pixels, a fake `AccessKit` tree with three
/// nodes, and a short `SourceView` so the source/frame/accesskit panels all have something
/// to render.
fn make_synthetic_frame() -> Frame {
    let width: u32 = 200;
    let height: u32 = 120;
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            // Subtle diagonal gradient so the snapshot actually contains an image, not a flat fill.
            // The division bounds the value to 0..=255.
            #[expect(clippy::cast_possible_truncation)]
            let shade = ((x + y) * 255 / (width + height)) as u8;
            rgba[i] = shade;
            rgba[i + 1] = 90;
            rgba[i + 2] = 160;
            rgba[i + 3] = 255;
        }
    }

    let root_id = NodeId(1);
    let button_id = NodeId(2);
    let label_id = NodeId(3);

    let mut root = Node::new(Role::Window);
    root.set_children([button_id, label_id]);

    let mut button = Node::new(Role::Button);
    button.set_label("Click me");
    button.set_bounds(AkRect {
        x0: 10.0,
        y0: 10.0,
        x1: 110.0,
        y1: 40.0,
    });

    let mut label = Node::new(Role::Label);
    label.set_label("Hello, kittest!");
    label.set_bounds(AkRect {
        x0: 10.0,
        y0: 60.0,
        x1: 180.0,
        y1: 90.0,
    });

    let accesskit = TreeUpdate {
        nodes: vec![(root_id, root), (button_id, button), (label_id, label)],
        tree: Some(Tree::new(root_id)),
        tree_id: TreeId::ROOT,
        focus: root_id,
    };

    let source_contents = "\
fn test_inspector() {
    let mut harness = Harness::new_eframe(|cc| InspectorApp::new(cc, rx, tx));
    harness.run();
    harness.snapshot(\"inspector_renders_frame\");
}
";

    Frame {
        step: 0,
        width,
        height,
        pixels_per_point: 1.0,
        rgba,
        accesskit: Some(accesskit),
        label: Some("synthetic_snapshot".to_owned()),
        source: Some(SourceView {
            path: "tests/snapshot.rs".to_owned(),
            contents: Some(source_contents.to_owned()),
            call_site_line: Some(3),
            event_lines: vec![],
        }),
    }
}

#[test]
fn inspector_renders_frame() {
    let (worker_tx, worker_rx) = mpsc::channel::<WorkerEvent>();
    let (release_tx, release_rx) = mpsc::channel::<Vec<egui::Event>>();

    // Push the frame before constructing the harness so it's ready to consume on the first
    // `pump_worker()` call. `send` into an unbounded mpsc channel can't block.
    worker_tx
        .send(WorkerEvent::Frame(Box::new(make_synthetic_frame())))
        .expect("worker channel should accept the frame");

    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1100.0, 750.0))
        .build_eframe(|cc| InspectorApp::new(cc, worker_rx, release_tx));

    // `step` (one frame), not `run` — the app calls `request_repaint_after` unconditionally
    // every frame so it never "settles" in the way `run` expects.
    harness.step();

    // Hold the release rx alive for the test duration — nothing reads from it, but dropping
    // it early would make future `release_tx.send` calls error (they don't fire here, but
    // this keeps the channel semantics identical to the production IO thread).
    let _release_rx = release_rx;

    harness.snapshot("inspector_renders_frame");
}
