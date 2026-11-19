//! Small example test for iterating on the inspector itself.
//!
//! Run normally to exercise the test as a regular kittest test:
//!
//! ```sh
//! cargo test --test example_app
//! ```
//!
//! Run with the inspector attached to debug the inspector UI against a real harness:
//!
//! ```sh
//! cargo build
//! KITTEST_INSPECTOR=1 \
//!   KITTEST_INSPECTOR_PATH=target/debug/kittest_inspector \
//!   cargo test --test example_app -- --nocapture
//! cargo build && KITTEST_INSPECTOR=1 KITTEST_INSPECTOR_PATH=target/debug/kittest_inspector cargo test --test example_app
//! ```
//!
//! The UI is a single-line `TextEdit` plus a 4×4 grid of labelled buttons — enough to
//! produce a handful of harness steps (type, click, click, …) so you can walk through
//! history, try Control mode, inspect widget bounds, etc.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

const GRID_SIZE: usize = 4;

#[derive(Default)]
struct ExampleState {
    text: String,
    /// Flat bit-set indexed by `row * GRID_SIZE + col`; flipped on each click.
    clicked: [bool; GRID_SIZE * GRID_SIZE],
}

fn button_label(row: usize, col: usize) -> String {
    format!("Button {row},{col}")
}

fn build_ui(ui: &mut egui::Ui, state: &mut ExampleState) {
    ui.heading("Example app");
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.add(egui::TextEdit::singleline(&mut state.text).hint_text("type here"));
    });
    ui.separator();
    egui::Grid::new("button_grid")
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for row in 0..GRID_SIZE {
                for col in 0..GRID_SIZE {
                    let idx = row * GRID_SIZE + col;
                    // `selectable_label` so the cell stays visibly "on" after a click — makes
                    // the inspector's history scrubber much more useful when walking through.
                    if ui
                        .selectable_label(state.clicked[idx], button_label(row, col))
                        .clicked()
                    {
                        state.clicked[idx] = !state.clicked[idx];
                    }
                }
                ui.end_row();
            }
        });
    ui.separator();
    ui.label(format!(
        "Clicked so far: {}",
        state.clicked.iter().filter(|b| **b).count()
    ));
}

#[test]
fn type_text_and_click_every_button() {
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(360.0, 300.0))
        .build_ui_state(build_ui, ExampleState::default());

    // Focus the text edit and type. Each `type_text` + `run` produces one inspector step,
    // so the source panel's event highlight moves line-by-line as you step.
    // (There's only one `TextInput` in the UI, so role alone uniquely identifies it.)
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .focus();
    harness.run();
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text("Hello, kittest!");
    harness.run();

    // Click each button in row-major order. Separate `run` per click so each button press
    // lands in its own harness step — easier to scrub through in the inspector.
    for row in 0..GRID_SIZE {
        for col in 0..GRID_SIZE {
            let label = button_label(row, col);
            harness.get_by_label(label.as_str()).click();
            harness.run();
        }
    }

    let state = harness.state();
    assert_eq!(
        state.text, "Hello, kittest!",
        "TextEdit did not receive the typed text"
    );
    assert!(
        state.clicked.iter().all(|b| *b),
        "Not every grid button was clicked: {:?}",
        state.clicked,
    );
}
