use egui::{Rect, Ui, Vec2, Vec2b, pos2, vec2};

use egui_table::{SplitScroll, SplitScrollDelegate};

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub struct SplitScrollDemo {}

impl SplitScrollDemo {
    pub fn ui(&self, ui: &mut egui::Ui) {
        _ = self;
        let mut delegate = DemoScrollDelegate {};

        SplitScroll {
            scroll_enabled: Vec2b::new(true, true),
            fixed_size: vec2(123.0, 37.0),
            scroll_outer_size: vec2(600.0, 400.0),
            scroll_content_size: vec2(10_000.0, 10_000.0),
            stick_to_bottom: false,
        }
        .show(ui, &mut delegate);
    }
}

struct DemoScrollDelegate {}

// TODO: unified coordinate system
impl SplitScrollDelegate for DemoScrollDelegate {
    fn left_top_ui(&mut self, ui: &mut Ui) {
        checkerboard(ui);
        ui.label("Fixed region");
    }

    fn right_top_ui(&mut self, ui: &mut Ui, _scroll_offset: Vec2) {
        checkerboard(ui);
        ui.label("Horizontally scrollable. This is where the fixed rows of a table view will go.");
    }

    fn left_bottom_ui(&mut self, ui: &mut Ui, _scroll_offset: Vec2) {
        checkerboard(ui);
        ui.label("Vertically scrollable. This is where the fixed columns of a table view will go, for instance the row number.");
    }

    fn right_bottom_ui(&mut self, ui: &mut Ui, _scroll_offset: Vec2) {
        checkerboard(ui);
        ui.label("Fully scrollable. This is where the bulk of the table view will go.");
    }
}

fn checkerboard(ui: &Ui) {
    let rect = ui.max_rect();
    // ui.painter()
    //     .rect_stroke(rect.shrink(0.5), 1.0, (1.0, ui.visuals().text_color()));

    let fill_color = ui.visuals().faint_bg_color;

    let mut x = rect.left();
    while x < rect.right() {
        let column = Rect::from_min_size(pos2(x, rect.top()), vec2(40.0, rect.height()));
        ui.painter().rect_filled(column, 0.0, fill_color);
        x += 91.0;
    }

    let mut y = rect.top();
    while y < rect.bottom() {
        let row = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), 20.0));
        ui.painter().rect_filled(row, 0.0, fill_color);
        y += 43.0;
    }
}
