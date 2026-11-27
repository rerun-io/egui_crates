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
            table.striped = true;
            table.header_fill = Color32::from_rgb(40, 40, 80);
            table.selected_row_fill = Some(Color32::from_rgb(0, 80, 0));
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
                    .show(ui, &mut Delegate);
            });
        });
    harness.run();
    harness.snapshot("table_style");
}

/// Double-clicking the resize line of a column sizes the column to fit its contents.
#[test]
pub fn double_click_resize_line_auto_sizes_column() {
    use std::{cell::Cell, rc::Rc};

    struct Delegate {
        wide_cell: bool,
    }

    impl egui_table::TableDelegate for Delegate {
        fn header_cell_ui(&mut self, ui: &mut egui::Ui, _cell: &egui_table::HeaderCellInfo) {
            ui.label("H");
        }

        fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::CellInfo) {
            if self.wide_cell && cell.col_nr == 0 && cell.row_nr == 1 {
                ui.label("A cell that is much wider than the column");
            } else {
                ui.label("x");
            }
        }
    }

    let wide_cell = Rc::new(Cell::new(true));
    let width = Rc::new(Cell::new(0.0));
    let line_pos = Rc::new(Cell::new(egui::Pos2::ZERO));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .with_step_dt(1.0 / 60.0) // Fast enough for a double-click
        .build_ui({
            let wide_cell = Rc::clone(&wide_cell);
            let width = Rc::clone(&width);
            let line_pos = Rc::clone(&line_pos);
            move |ui| {
                let table = egui_table::Table::new()
                    .num_rows(3)
                    .columns(vec![egui_table::Column::new(60.0).range(10.0..=500.0); 2])
                    .headers([egui_table::HeaderRow::new(20.0)]);
                let id = table.get_id(ui);
                let top_left = ui.cursor().min;
                table.show(
                    ui,
                    &mut Delegate {
                        wide_cell: wide_cell.get(),
                    },
                );

                let state = egui_table::TableState::load(ui.ctx(), id).expect("table state");
                let column_width = state
                    .col_widths
                    .get(&egui_table::Column::default().id_for(0))
                    .copied()
                    .unwrap_or_default();
                width.set(column_width);
                line_pos.set(top_left + egui::vec2(column_width, 30.0));
            }
        });
    harness.run();

    // Columns don't shrink by themselves when their contents get narrower:
    wide_cell.set(false);
    harness.run();
    let before = width.get();
    assert!(
        150.0 < before,
        "the wide cell should widen the column, but it is {before}"
    );

    harness.hover_at(line_pos.get());
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: line_pos.get(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
            harness.step();
        }
    }
    harness.run();

    let after = width.get();
    assert!(
        after < 50.0,
        "double-click should shrink the column to fit: {before} -> {after}"
    );
}

/// A styled table with a sticky column, scrolled sideways.
///
/// The line on the edge of the sticky column stays, and the corners stay rounded.
#[test]
pub fn table_style_scrolled_snapshot() {
    use egui::{Color32, CornerRadius, Stroke};

    struct Delegate;

    impl egui_table::TableDelegate for Delegate {
        fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::HeaderCellInfo) {
            ui.strong(format!("Col {}", cell.group_index));
        }

        fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &egui_table::CellInfo) {
            ui.label(format!("{}, {}", cell.row_nr, cell.col_nr));
        }
    }

    let mut harness = Harness::builder()
        .with_size(egui::vec2(220.0, 140.0))
        .with_pixels_per_point(2.0)
        .build_ui({
            let mut first_frame = true;
            move |ui| {
                let table = &mut ui.visuals_mut().table;
                table.striped = true;
                table.header_fill = Color32::from_rgb(40, 40, 80);
                table.outer_stroke = Stroke::new(1.0, Color32::RED);
                table.corner_radius = CornerRadius::same(6);
                table.header_stroke = Stroke::new(1.0, Color32::YELLOW);
                table.column_stroke = Stroke::new(1.0, Color32::LIGHT_BLUE);

                egui::Frame::new().inner_margin(8).show(ui, |ui| {
                    let mut table = egui_table::Table::new()
                        .num_rows(4)
                        .num_sticky_cols(1)
                        .columns(vec![egui_table::Column::new(60.0).resizable(false); 6])
                        .headers([egui_table::HeaderRow::new(20.0)]);
                    if first_frame {
                        table = table.scroll_to_column(5, Some(egui::Align::Max));
                        first_frame = false;
                    }
                    table.show(ui, &mut Delegate);
                });
            }
        });
    harness.run();
    harness.snapshot("table_style_scrolled");
}
