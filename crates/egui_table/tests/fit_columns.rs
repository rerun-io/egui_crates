//! Columns are fitted to their content, even when the content arrives late.

use egui_kittest::Harness;
use egui_table::{Column, HeaderCellInfo, HeaderRow, Table, TableDelegate, TableState};

const CELL_TEXT: &str = "a fairly long cell text";

struct State {
    num_rows: u64,
    num_columns: usize,
    table_id: Option<egui::Id>,
}

struct Delegate;

impl TableDelegate for Delegate {
    fn header_cell_ui(&mut self, ui: &mut egui::Ui, _cell: &HeaderCellInfo) {
        ui.label("H");
    }

    /// Like most truncating content, this only reports its full width when measured,
    /// so it never makes its column grow outside of a sizing pass.
    fn cell_ui(&mut self, ui: &mut egui::Ui, _cell: &egui_table::CellInfo) {
        if ui.is_sizing_pass() {
            ui.label(CELL_TEXT);
        } else {
            ui.add(egui::Label::new(CELL_TEXT).truncate());
        }
    }
}

fn harness(num_rows: u64, num_columns: usize) -> Harness<'static, State> {
    Harness::builder()
        .with_size([2000.0, 400.0])
        .build_ui_state(
            |ui, state: &mut State| {
                let table = Table::new()
                    .id_salt("table")
                    .num_rows(state.num_rows)
                    .columns(vec![Column::new(100.0); state.num_columns])
                    .headers([HeaderRow::new(20.0)]);
                state.table_id = Some(table.get_id(ui));
                table.show(ui, &mut Delegate);
            },
            State {
                num_rows,
                num_columns,
                table_id: None,
            },
        )
}

fn column_widths(harness: &Harness<'_, State>) -> Vec<f32> {
    let id = harness.state().table_id.expect("table was shown");
    let state = TableState::load(&harness.ctx, id).expect("table state is stored");
    (0..harness.state().num_columns)
        .map(|i| state.col_widths[&Column::default().id_for(i)])
        .collect()
}

fn fitted_widths(num_columns: usize) -> Vec<f32> {
    let mut harness = harness(10, num_columns);
    harness.run();
    column_widths(&harness)
}

#[test]
fn columns_are_fitted_when_rows_arrive() {
    let mut harness = harness(0, 2);
    harness.run();

    harness.state_mut().num_rows = 10;
    harness.run();

    let expected = fitted_widths(2);
    assert_ne!(
        expected,
        vec![100.0; 2],
        "Column::new width must not match the fit"
    );
    assert_eq!(column_widths(&harness), expected);
}

#[test]
fn added_column_is_fitted() {
    let mut harness = harness(10, 1);
    harness.run();

    harness.state_mut().num_columns = 2;
    harness.run();

    assert_eq!(column_widths(&harness), fitted_widths(2));
}
