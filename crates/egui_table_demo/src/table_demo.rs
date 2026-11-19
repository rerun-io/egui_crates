use std::collections::BTreeMap;

use egui::{Align2, Context, Id, Margin, NumExt as _, Sense, Ui, Vec2};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct TableDemo {
    num_columns: usize,
    num_rows: u64,
    num_sticky_cols: usize,
    default_column: egui_table::Column,
    auto_size_mode: egui_table::AutoSizeMode,
    top_row_height: f32,
    row_height: f32,
    is_row_expanded: BTreeMap<u64, bool>,
    prefetched: Vec<egui_table::PrefetchInfo>,
}

impl Default for TableDemo {
    fn default() -> Self {
        Self {
            num_columns: 20,
            num_rows: 10_000,
            num_sticky_cols: 1,
            default_column: egui_table::Column::new(100.0)
                .range(10.0..=500.0)
                .resizable(true),
            auto_size_mode: egui_table::AutoSizeMode::default(),
            top_row_height: 24.0,
            row_height: 18.0,
            is_row_expanded: Default::default(),
            prefetched: vec![],
        }
    }
}

impl TableDemo {
    fn was_row_prefetched(&self, row_nr: u64) -> bool {
        self.prefetched
            .iter()
            .any(|info| info.visible_rows.contains(&row_nr))
    }

    fn cell_content_ui(&mut self, row_nr: u64, col_nr: usize, ui: &mut egui::Ui) {
        assert!(
            self.was_row_prefetched(row_nr),
            "Was asked to show row {row_nr} which was not prefetched! This is a bug in egui_table."
        );

        let is_expanded = self
            .is_row_expanded
            .get(&row_nr)
            .copied()
            .unwrap_or_default();
        let expandedness = ui.animate_bool(Id::unique(row_nr), is_expanded);

        ui.vertical(|ui| {
            if col_nr == 0 {
                ui.horizontal(|ui| {
                    // Button to expand/collapse row:
                    let (_, response) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::click());
                    egui::collapsing_header::paint_default_icon(ui, expandedness, &response);
                    if response.clicked() {
                        // Toggle.
                        // Note: we use a map instead of a set so that we can animate opening and closing of each column.
                        self.is_row_expanded.insert(row_nr, !is_expanded);
                    }

                    ui.label(row_nr.to_string());
                });
            } else {
                ui.horizontal(|ui| {
                    ui.label(format!("({row_nr}, {col_nr})"));

                    if (row_nr + col_nr as u64).is_multiple_of(27) {
                        if !ui.is_sizing_pass() {
                            // During a sizing pass we don't truncate!
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        }
                        ui.label("Extra long cell that will be truncated with an ellipsis character because it is so long");
                    }
                });

                if 0.0 < expandedness {
                    ui.label("Expanded content");
                    ui.label("Blah blah blah…");
                }
            }
        });
    }
}

impl egui_table::TableDelegate for TableDemo {
    fn prepare(&mut self, info: &egui_table::PrefetchInfo) {
        assert!(
            info.visible_rows.end <= self.num_rows,
            "Was asked to prefetch rows {:?}, but we only have {} rows. This is a bug in egui_table.",
            info.visible_rows,
            self.num_rows
        );
        self.prefetched.push(info.clone());
    }

    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell_inf: &egui_table::HeaderCellInfo) {
        let egui_table::HeaderCellInfo {
            group_index,
            col_range,
            row_nr,
            ..
        } = cell_inf;

        let margin = 4;

        egui::Frame::NONE
            .inner_margin(Margin::symmetric(margin, 0))
            .show(ui, |ui| {
                #[expect(clippy::collapsible_else_if)]
                if *row_nr == 0 {
                    if 0 < col_range.start {
                        // Our special grouped column.
                        let sticky = true;
                        let text = format!("This is group {group_index}");
                        if sticky {
                            let font_id = egui::TextStyle::Heading.resolve(ui.style());
                            let text_color = ui.visuals().text_color();
                            let galley =
                                ui.painter()
                                    .layout(text, font_id, text_color, f32::INFINITY);

                            // Put the text leftmost in the clip rect (so it is always visible)
                            let mut pos = Align2::LEFT_CENTER
                                .anchor_size(
                                    ui.clip_rect().shrink(margin as _).left_center(),
                                    galley.size(),
                                )
                                .min;

                            // … but not so far to the right that it doesn't fit.
                            pos.x = pos.x.at_most(ui.max_rect().right() - galley.size().x);

                            ui.put(
                                egui::Rect::from_min_size(pos, galley.size()),
                                egui::Label::new(galley),
                            );
                        } else {
                            ui.heading(text);
                        }
                    }
                } else {
                    if col_range.start == 0 {
                        egui::Sides::new().height(ui.available_height()).show(
                            ui,
                            |ui| {
                                ui.heading("Row");
                            },
                            |ui| {
                                ui.label("⬇");
                            },
                        );
                    } else {
                        ui.heading(format!("Column {group_index}"));
                    }
                }
            });
    }

    // You can use row_ui to add some style or interaction to the entire row.
    fn row_ui(&mut self, ui: &mut Ui, _row_nr: u64) {
        if ui.rect_contains_pointer(ui.max_rect()) {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, ui.visuals().code.bg_color);
        }

        if ui.response().interact(Sense::click()).clicked() {
            // Handle row clicks
        }
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell_info: &egui_table::CellInfo) {
        let egui_table::CellInfo { row_nr, col_nr, .. } = *cell_info;

        if row_nr % 2 == 1 {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, ui.visuals().faint_bg_color);
        }

        egui::Frame::NONE
            .inner_margin(Margin::symmetric(4, 0))
            .show(ui, |ui| {
                self.cell_content_ui(row_nr, col_nr, ui);
            });
    }

    fn row_top_offset(&self, ctx: &Context, _table_id: Id, row_nr: u64) -> f32 {
        let fully_expanded_row_height = 48.0;

        self.is_row_expanded
            .range(0..row_nr)
            .map(|(expanded_row_nr, expanded)| {
                let how_expanded = ctx.animate_bool(Id::unique(expanded_row_nr), *expanded);
                how_expanded * fully_expanded_row_height
            })
            .sum::<f32>()
            + row_nr as f32 * self.row_height
    }
}

impl TableDemo {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("settings").show(ui, |ui| {
            let label = ui.label("Columns");
            ui.add(egui::DragValue::new(&mut self.num_columns).range(12..=usize::MAX))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Rows");
            let speed = 1.0 + 0.05 * self.num_rows as f32;
            ui.add(
                egui::DragValue::new(&mut self.num_rows)
                    .speed(speed)
                    .range(0..=10_000),
            )
            .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Height of top row");
            ui.add(egui::DragValue::new(&mut self.top_row_height).range(0.0..=100.0))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Height of other rows");
            ui.add(egui::DragValue::new(&mut self.row_height).range(0.0..=100.0))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Sticky columns");
            ui.add(egui::DragValue::new(&mut self.num_sticky_cols))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Default column width");
            ui.add(egui::DragValue::new(&mut self.default_column.current))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Column width range");
            ui.horizontal(|ui| {
                let range = &mut self.default_column.range;
                ui.add(egui::DragValue::new(&mut range.min).range(0.0..=range.max))
                    .labelled_by(label.id);
                ui.label("to");
                ui.add(egui::DragValue::new(&mut range.max).range(range.min..=1000.0))
                    .labelled_by(label.id);
            });
            ui.end_row();

            ui.label("Auto-size mode");
            ui.horizontal(|ui| {
                ui.radio_value(
                    &mut self.auto_size_mode,
                    egui_table::AutoSizeMode::Never,
                    "Never",
                );
                ui.radio_value(
                    &mut self.auto_size_mode,
                    egui_table::AutoSizeMode::Always,
                    "Always",
                );
                ui.radio_value(
                    &mut self.auto_size_mode,
                    egui_table::AutoSizeMode::OnParentResize,
                    "OnParentResize",
                );
            });
            ui.end_row();
        });

        let id_salt = egui::IdSalt::new("table_demo");
        let state_id = egui_table::Table::new().id_salt(id_salt).get_id(ui); // Note: must be here (in the correct outer `ui` scope) to be correct.

        ui.horizontal(|ui| {
            if ui.button("Reset settings").clicked() {
                *self = Self::default();
            }
            if ui.button("Reset state").clicked() {
                debug_assert!(
                    egui_table::TableState::load(ui, state_id).is_some(),
                    "Wrong state_id"
                );
                egui_table::TableState::reset(ui, state_id);
            }
        });

        ui.separator();

        ui.horizontal(|ui| {
            for info in &self.prefetched {
                ui.label("Visible columns:");
                ui.label(format!(
                    "{}..{}",
                    info.visible_columns.start, info.visible_columns.end
                ));

                ui.label("rows:");
                ui.label(format!(
                    "{}..{}",
                    info.visible_rows.start, info.visible_rows.end
                ));
            }
            self.prefetched.clear();
        });

        let first_scrollable_col = self.num_sticky_cols;
        let last_col = self.num_columns.saturating_sub(1);
        let mid_col = usize::midpoint(first_scrollable_col, last_col);
        let mut scroll_to_column = None;
        ui.horizontal(|ui| {
            ui.label("Scroll horizontally to…");
            if ui.button("left").clicked() {
                scroll_to_column = Some(first_scrollable_col);
            }
            if ui.button("middle").clicked() {
                scroll_to_column = Some(mid_col);
            }
            if ui.button("right").clicked() {
                scroll_to_column = Some(last_col);
            }
            if ui.button(format!("column {mid_col}")).clicked() {
                scroll_to_column = Some(mid_col);
            }
        });

        let mut scroll_to_row = None;
        ui.horizontal(|ui| {
            ui.label("Scroll vertically to…");
            if ui.button("top").clicked() {
                scroll_to_row = Some(0);
            }
            if ui.button("middle").clicked() {
                scroll_to_row = Some(self.num_rows / 2);
            }
            if ui.button("bottom").clicked() {
                scroll_to_row = Some(self.num_rows.saturating_sub(1));
            }
            let mid_row = self.num_rows / 2;
            if ui.button(format!("row {mid_row}")).clicked() {
                scroll_to_row = Some(mid_row);
            }
        });

        ui.separator();

        let mut table = egui_table::Table::new()
            .id_salt(id_salt)
            .num_rows(self.num_rows)
            .columns(vec![self.default_column; self.num_columns])
            .num_sticky_cols(self.num_sticky_cols)
            .headers([
                egui_table::HeaderRow {
                    height: self.top_row_height,
                    groups: vec![0..1, 1..4, 4..8, 8..12],
                },
                egui_table::HeaderRow::new(self.top_row_height),
            ])
            .auto_size_mode(self.auto_size_mode);

        if let Some(scroll_to_column) = scroll_to_column {
            table = table.scroll_to_column(scroll_to_column, None);
        }
        if let Some(scroll_to_row) = scroll_to_row {
            table = table.scroll_to_row(scroll_to_row, None);
        }

        table.show(ui, self);
    }
}
