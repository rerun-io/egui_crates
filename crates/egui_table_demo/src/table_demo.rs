use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, Context, Id, NumExt as _, Sense, Ui, Vec2};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct TableDemo {
    num_columns: usize,
    num_rows: u64,
    num_sticky_cols: usize,
    default_column: egui_table::Column,
    auto_size_mode: egui_table::AutoSizeMode,
    top_row_height: f32,
    row_height: f32,
    highlight_hovered_row: bool,
    is_row_expanded: BTreeMap<u64, bool>,
    selected_rows: BTreeSet<u64>,
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
            highlight_hovered_row: true,
            is_row_expanded: Default::default(),
            selected_rows: Default::default(),
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

        if *row_nr == 0 {
            if 0 < col_range.start {
                // Our special grouped column, with its title kept in view while scrolling:
                let text = format!("This is group {group_index}");
                let font_id = egui::TextStyle::Heading.resolve(ui.style());
                let text_color = ui.visuals().text_color();
                let galley = ui
                    .painter()
                    .layout(text, font_id, text_color, f32::INFINITY);

                // Put the text leftmost in the clip rect (so it is always visible)
                let mut pos = Align2::LEFT_CENTER
                    .anchor_size(
                        (ui.clip_rect() - ui.visuals().table.cell_margin).left_center(),
                        galley.size(),
                    )
                    .min;

                // … but not so far to the right that it doesn't fit.
                pos.x = pos.x.at_most(ui.max_rect().right() - galley.size().x);

                ui.put(
                    egui::Rect::from_min_size(pos, galley.size()),
                    egui::Label::new(galley),
                );
            }
        } else if col_range.start == 0 {
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

    // You can use row_ui to add some style or interaction to the entire row.
    fn row_ui(&mut self, ui: &mut Ui, row_nr: u64) {
        let response = ui.response().interact(Sense::click());
        if response.clicked() {
            // Click to select a row, and Cmd/Ctrl-click to select several:
            if ui.input(|i| i.modifiers.command) {
                if !self.selected_rows.remove(&row_nr) {
                    self.selected_rows.insert(row_nr);
                }
            } else {
                self.selected_rows = BTreeSet::from([row_nr]);
            }
        }
    }

    fn is_row_selected(&self, row_nr: u64) -> bool {
        self.selected_rows.contains(&row_nr)
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell_info: &egui_table::CellInfo) {
        let egui_table::CellInfo { row_nr, col_nr, .. } = *cell_info;

        self.cell_content_ui(row_nr, col_nr, ui);
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
        let id_salt = egui::IdSalt::new("table_demo");
        let state_id = egui_table::Table::new().id_salt(id_salt).get_id(ui); // Note: must be here (in the correct outer `ui` scope) to be correct.

        let mut scroll_to = ScrollTo::default();

        egui::Panel::left("table_demo_settings")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.settings_ui(ui, state_id, &mut scroll_to);
                });
            });

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
            .auto_size_mode(self.auto_size_mode)
            .highlight_hovered_row(self.highlight_hovered_row);

        if let Some(column) = scroll_to.column {
            table = table.scroll_to_column(column, None);
        }
        if let Some(row) = scroll_to.row {
            table = table.scroll_to_row(row, None);
        }

        table.show(ui, self);
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui, state_id: Id, scroll_to: &mut ScrollTo) {
        egui::CollapsingHeader::new("Layout")
            .default_open(true)
            .show(ui, |ui| self.layout_ui(ui));

        egui::CollapsingHeader::new("Rows")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.highlight_hovered_row, "Highlight hovered row");
                ui.horizontal(|ui| {
                    ui.label(format!("{} selected", self.selected_rows.len()));
                    if ui.button("Clear").clicked() {
                        self.selected_rows.clear();
                    }
                });
                ui.weak("Click a row to select it, and Cmd/Ctrl-click to select several.");
            });

        egui::CollapsingHeader::new("Style")
            .default_open(true)
            .show(ui, |ui| {
                let mut table_style = ui.visuals().table;
                table_style.ui(ui);
                if table_style != ui.visuals().table {
                    set_table_style(ui.ctx(), table_style);
                }
            });

        egui::CollapsingHeader::new("Scroll to")
            .default_open(true)
            .show(ui, |ui| self.scroll_to_ui(ui, scroll_to));

        ui.separator();

        ui.horizontal(|ui| {
            if ui.button("Reset settings").clicked() {
                *self = Self::default();
                set_table_style(ui.ctx(), demo_table_style());
            }
            if ui.button("Reset state").clicked() {
                debug_assert!(
                    egui_table::TableState::load(ui, state_id).is_some(),
                    "Wrong state_id"
                );
                egui_table::TableState::reset(ui, state_id);
            }
        });

        for info in &self.prefetched {
            ui.weak(format!(
                "Visible columns {}..{}, rows {}..{}",
                info.visible_columns.start,
                info.visible_columns.end,
                info.visible_rows.start,
                info.visible_rows.end
            ));
        }
        self.prefetched.clear();
    }

    fn layout_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("layout").num_columns(2).show(ui, |ui| {
            let label = ui.label("Columns");
            ui.add(egui::DragValue::new(&mut self.num_columns).range(12..=usize::MAX))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Sticky columns");
            ui.add(egui::DragValue::new(&mut self.num_sticky_cols))
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

            let label = ui.label("Header row height");
            ui.add(egui::DragValue::new(&mut self.top_row_height).range(0.0..=100.0))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Row height");
            ui.add(egui::DragValue::new(&mut self.row_height).range(0.0..=100.0))
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Column width");
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

            let label = ui.label("Auto-size columns");
            egui::ComboBox::from_id_salt("auto_size_mode")
                .selected_text(format!("{:?}", self.auto_size_mode))
                .show_ui(ui, |ui| {
                    for mode in [
                        egui_table::AutoSizeMode::Never,
                        egui_table::AutoSizeMode::Always,
                        egui_table::AutoSizeMode::OnParentResize,
                    ] {
                        ui.selectable_value(&mut self.auto_size_mode, mode, format!("{mode:?}"));
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();
        });
    }

    fn scroll_to_ui(&self, ui: &mut egui::Ui, scroll_to: &mut ScrollTo) {
        let first_col = self.num_sticky_cols;
        let last_col = self.num_columns.saturating_sub(1);
        let last_row = self.num_rows.saturating_sub(1);

        egui::Grid::new("scroll_to").num_columns(2).show(ui, |ui| {
            ui.label("Column");
            ui.horizontal(|ui| {
                if ui.button("First").clicked() {
                    scroll_to.column = Some(first_col);
                }
                if ui.button("Middle").clicked() {
                    scroll_to.column = Some(usize::midpoint(first_col, last_col));
                }
                if ui.button("Last").clicked() {
                    scroll_to.column = Some(last_col);
                }
            });
            ui.end_row();

            ui.label("Row");
            ui.horizontal(|ui| {
                if ui.button("First").clicked() {
                    scroll_to.row = Some(0);
                }
                if ui.button("Middle").clicked() {
                    scroll_to.row = Some(last_row / 2);
                }
                if ui.button("Last").clicked() {
                    scroll_to.row = Some(last_row);
                }
            });
            ui.end_row();
        });
    }
}

/// Where the user asked the table to scroll to this frame.
#[derive(Default)]
struct ScrollTo {
    column: Option<usize>,
    row: Option<u64>,
}

/// The table style the demo starts with.
pub fn demo_table_style() -> egui::style::TableStyle {
    egui::style::TableStyle {
        striped: true,
        ..Default::default()
    }
}

/// Use this table style in both the dark and the light theme.
pub fn set_table_style(ctx: &Context, table_style: egui::style::TableStyle) {
    ctx.all_styles_mut(|style| style.visuals.table = table_style);
}
