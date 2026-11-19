use demo::DemoApp;
use egui_kittest::kittest::Queryable as _;
use egui_kittest::{Harness, SnapshotResults};

#[test]
pub fn table_snapshot() {
    let mut results = SnapshotResults::new();

    let mut harness = Harness::builder().build_eframe(|cc| DemoApp::new(cc));
    results.add(harness.try_snapshot("table_demo"));

    harness.get_by_label("Scroll").click();
    harness.run();

    results.add(harness.try_snapshot("scroll_demo"));
}
