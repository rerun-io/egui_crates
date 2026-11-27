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

/// A table that uses every field of [`egui::style::TableStyle`].
#[test]
pub fn table_style_snapshot() {
    use egui::{Color32, CornerRadius, Margin, Stroke};

    struct Delegate;

    impl egui_table::TableDelegate for Delegate {
        fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::HeaderCellInfo) {
            ui.strong(["Name", "Kind", "Size"][cell.group_index]);
        }

        fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::CellInfo) {
            let egui_table::CellInfo { row_nr, col_nr, .. } = *cell;
            ui.label(match col_nr {
                0 => format!("row {row_nr}"),
                1 => "file".to_owned(),
                _ => "42 kB".to_owned(),
            });
        }

        fn is_row_selected(&self, row_nr: u64) -> bool {
            row_nr == 2
        }
    }

    let mut harness = Harness::builder()
        .with_size(egui::vec2(180.0, 160.0))
        .with_pixels_per_point(2.0)
        .build_ui(|ui| {
            let table = &mut ui.visuals_mut().table;
            table.header_fill = Color32::from_rgb(40, 40, 80);
            table.selected_row_fill = Color32::from_rgb(0, 80, 0);
            table.outer_stroke = Stroke::new(1.0, Color32::RED);
            table.corner_radius = CornerRadius::same(4);
            table.header_stroke = Stroke::new(1.0, Color32::YELLOW);
            table.row_stroke = Stroke::new(1.0, Color32::GRAY);
            table.column_stroke = Stroke::new(1.0, Color32::LIGHT_BLUE);
            table.cell_margin = Margin::symmetric(6, 2);

            egui::Frame::new().inner_margin(8).show(ui, |ui| {
                egui_table::Table::new()
                    .num_rows(4)
                    .columns(vec![egui_table::Column::new(50.0).resizable(false); 3])
                    .headers([egui_table::HeaderRow::new(20.0)])
                    .striped(true)
                    .show(ui, &mut Delegate);
            });
        });
    harness.run();
    harness.snapshot("table_style");
}
