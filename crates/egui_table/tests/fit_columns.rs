//! Columns are fitted to their content, even when the content arrives late.

use egui_kittest::Harness;
use egui_table::{Column, HeaderCellInfo, HeaderRow, Table, TableDelegate, TableState};

const CELL_TEXT: &str = "a fairly long cell text";

/// Narrower than [`CELL_TEXT`].
const DEFAULT_WIDTH: f32 = 100.0;

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
                    .columns(vec![Column::new(DEFAULT_WIDTH); state.num_columns])
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
        .map(|i| state.columns[&Column::default().id_for(i)].width)
        .collect()
}

fn fitted_widths(num_columns: usize) -> Vec<f32> {
    let mut harness = harness(10, num_columns);
    harness.run();
    column_widths(&harness)
}

#[test]
fn columns_are_fitted_when_rows_arrive() {
    let fitted = fitted_widths(2);

    let mut harness = harness(0, 2);
    harness.run();
    for (empty, fitted) in std::iter::zip(column_widths(&harness), &fitted) {
        assert!(empty < *fitted, "With no rows, only the header is measured");
    }

    harness.state_mut().num_rows = 10;
    harness.run();
    assert_eq!(column_widths(&harness), fitted);
}

#[test]
fn added_column_is_fitted() {
    let mut harness = harness(10, 1);
    harness.run();

    let fitted = fitted_widths(2);
    assert!(
        DEFAULT_WIDTH < fitted[1],
        "The test needs a column that grows when fitted"
    );

    harness.state_mut().num_columns = 2;
    harness.run();
    assert_eq!(column_widths(&harness), fitted);
}
