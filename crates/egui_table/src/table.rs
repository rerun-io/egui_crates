use std::{
    collections::{BTreeMap, btree_map::Entry},
    ops::{Range, RangeInclusive},
};

use egui::{
    Align, Context, Id, IdMap, IdSalt, Layout, NumExt as _, Rangef, Rect, Response, Ui, UiBuilder,
    Vec2, Vec2b,
};
use vec1::Vec1;

use crate::{SplitScroll, SplitScrollDelegate, columns::Column};

// TODO: fix the functionality of this
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum AutoSizeMode {
    /// Never auto-size the columns.
    #[default]
    Never,

    /// Always auto-size the columns
    Always,

    /// Auto-size the columns if the parents' width changes
    OnParentResize,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct TableState {
    // Maps columns ids to their widths.
    pub col_widths: IdMap<f32>,

    pub parent_width: Option<f32>,
}

impl TableState {
    pub fn load(ctx: &egui::Context, id: Id) -> Option<Self> {
        ctx.data_mut(|d| d.get_persisted(id))
    }

    pub fn store(self, ctx: &egui::Context, id: Id) {
        ctx.data_mut(|d| d.insert_persisted(id, self));
    }

    pub fn id(ui: &Ui, id_salt: IdSalt) -> Id {
        ui.make_persistent_id(id_salt)
    }

    pub fn reset(ctx: &egui::Context, id: Id) {
        ctx.data_mut(|d| {
            d.remove::<Self>(id);
        });
    }
}

/// Describes one of potentially many header rows.
///
/// Each header row has a fixed height.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct HeaderRow {
    pub height: f32,

    /// If empty, it is ignored.
    ///
    /// Contains non-overlapping ranges of column indices to group together.
    /// For instance: `vec![(0..3), (3..5), (5..6)]`.
    pub groups: Vec<Range<usize>>,
}

impl HeaderRow {
    pub fn new(height: f32) -> Self {
        Self {
            height,
            groups: Default::default(),
        }
    }
}

/// A table viewer.
///
/// Designed to be fast when there are millions of rows, but only hundreds of columns.
///
/// ## Sticky columns and rows
/// You can designate a certain number of column and rows as being "sticky".
/// These won't scroll with the rest of the table.
///
/// The sticky rows are always the first ones at the top, and are usually used for the column headers.
/// The sticky columns are always the first ones on the left, useful for special columns like
/// table row number or similar.
/// A sticky column is sometimes called a "gutter".
///
/// ## Batteries not included
/// * You need to specify the `Table` size beforehand
/// * Does not add any margins to cells. Add it yourself with [`egui::Frame`].
/// * Does not wrap cells in scroll areas. Do that yourself.
/// * Doesn't paint any guide-lines for the rows. Paint them yourself.
pub struct Table {
    /// The columns of the table.
    columns: Vec<Column>,

    /// Salt added to the parent [`Ui::id`] to produce an [`Id`] that is unique
    /// within the parent [`Ui`].
    ///
    /// You need to set this to something unique if you have multiple tables in the same ui.
    id_salt: IdSalt,

    /// Which columns are sticky (non-scrolling)?
    num_sticky_cols: usize,

    /// The count and parameters of the sticky (non-scrolling) header rows.
    headers: Vec<HeaderRow>,

    /// Total number of rows (sticky + non-sticky).
    num_rows: u64,

    /// How to do auto-sizing of columns, if at all.
    auto_size_mode: AutoSizeMode,

    scroll_to_columns: Option<(RangeInclusive<usize>, Option<Align>)>,
    scroll_to_rows: Option<(RangeInclusive<u64>, Option<Align>)>,

    /// If true, the vertical scrollbar will stick to the bottom as the content grows.
    ///
    /// Useful for log views or terminal emulation.
    stick_to_bottom: bool,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            columns: vec![],
            id_salt: IdSalt::new("table"),
            num_sticky_cols: 0,
            headers: vec![HeaderRow::new(16.0)],
            num_rows: 0,
            auto_size_mode: AutoSizeMode::default(),
            scroll_to_columns: None,
            scroll_to_rows: None,
            stick_to_bottom: false,
        }
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CellInfo {
    pub col_nr: usize,

    pub row_nr: u64,

    /// The unique [`Id`] of this table.
    pub table_id: Id,
    // We could add more stuff here, like a reference to the column
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HeaderCellInfo {
    pub group_index: usize,

    pub col_range: Range<usize>,

    /// Header row
    pub row_nr: usize,

    /// The unique [`Id`] of this table.
    pub table_id: Id,
}

/// Data given to the delegate containing information about what is about to be rendered.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct PrefetchInfo {
    /// The sticky columns are always visible.
    pub num_sticky_columns: usize,

    /// This range of columns are currently visible, in addition to the sticky ones.
    pub visible_columns: Range<usize>,

    /// These rows are currently visible.
    pub visible_rows: Range<u64>,

    /// The unique [`Id`] of this table.
    pub table_id: Id,
}

/// The interface that the user needs to implement to display a table.
///
/// The [`Table`] calls functions on the delegate to render the table.
pub trait TableDelegate {
    /// Called before any call to [`Self::cell_ui`] to communicate the range of visible columns and rows.
    ///
    /// You can use this to only load the data required to be viewed.
    fn prepare(&mut self, _info: &PrefetchInfo) {}

    /// The contents of a header cell in the table.
    ///
    /// The [`CellInfo::row_nr`] is which header row (usually 0).
    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &HeaderCellInfo);

    /// The contents of a row.
    ///
    /// Individual cell [`Ui`]s will be children of the ui passed to this fn, so you can e.g. use
    /// [`Ui::style_mut`] to style the whole row.
    ///
    /// This might be called multiple times per row (e.g. for sticky and non-sticky columns).
    fn row_ui(&mut self, _ui: &mut Ui, _row_nr: u64) {}

    /// The contents of a cell in the table.
    ///
    /// The [`CellInfo::row_nr`] is ignoring header rows.
    fn cell_ui(&mut self, ui: &mut Ui, cell: &CellInfo);

    /// Compute the offset for the top of the given row.
    ///
    /// Implement this for arbitrary row heights. The default implementation uses
    /// [`Self::default_row_height`].
    ///
    /// Note: must always return 0.0 for `row_nr = 0`.
    fn row_top_offset(&self, _ctx: &Context, _table_id: Id, row_nr: u64) -> f32 {
        row_nr as f32 * self.default_row_height()
    }

    /// Default row height.
    ///
    /// This is used by the default implementation of [`Self::row_top_offset`].
    fn default_row_height(&self) -> f32 {
        20.0
    }
}

impl Table {
    /// Create a new table, with no columns and no headers, and zero rows.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Salt added to the parent [`Ui::id`] to produce an [`Id`] that is unique
    /// within the parent [`Ui`].
    ///
    /// You need to set this to something unique if you have multiple tables in the same ui.
    #[inline]
    pub fn id_salt(mut self, id_salt: impl egui::AsIdSalt) -> Self {
        self.id_salt = IdSalt::new(id_salt);
        self
    }

    /// Total number of rows (sticky + non-sticky).
    #[inline]
    pub fn num_rows(mut self, num_rows: u64) -> Self {
        self.num_rows = num_rows;
        self
    }

    /// The columns of the table.
    #[inline]
    pub fn columns(mut self, columns: impl Into<Vec<Column>>) -> Self {
        self.columns = columns.into();
        self
    }

    /// How many columns are sticky (non-scrolling)?
    ///
    /// Default is 0.
    #[inline]
    pub fn num_sticky_cols(mut self, num_sticky_cols: usize) -> Self {
        self.num_sticky_cols = num_sticky_cols;
        self
    }

    /// The count and parameters of the sticky (non-scrolling) header rows.
    #[inline]
    pub fn headers(mut self, headers: impl Into<Vec<HeaderRow>>) -> Self {
        self.headers = headers.into();
        self
    }

    /// How to do auto-sizing of columns, if at all.
    #[inline]
    pub fn auto_size_mode(mut self, auto_size_mode: AutoSizeMode) -> Self {
        self.auto_size_mode = auto_size_mode;
        self
    }

    /// The scroll handle will stick to the bottom position even while the content size
    /// changes dynamically.
    ///
    /// This can be useful to simulate terminal UIs or log/info scrollers.
    /// The scroll handle remains stuck until user manually changes position. Once "unstuck"
    /// it will remain focused on whatever content viewport the user left it on.
    #[inline]
    pub fn stick_to_bottom(mut self, stick: bool) -> Self {
        self.stick_to_bottom = stick;
        self
    }

    /// Read the globally unique id, based on the current [`Self::id_salt`]
    /// and the parent id.
    #[inline]
    pub fn get_id(&self, ui: &Ui) -> Id {
        TableState::id(ui, self.id_salt)
    }

    /// Set a row to scroll to.
    ///
    /// `align` specifies if the row should be positioned in the top, center, or bottom of the view
    /// (using [`Align::TOP`], [`Align::Center`] or [`Align::BOTTOM`]).
    /// If `align` is `None`, the table will scroll just enough to bring the cursor into view.
    ///
    /// See also: [`Self::scroll_to_column`].
    #[inline]
    pub fn scroll_to_row(self, row: u64, align: Option<Align>) -> Self {
        self.scroll_to_rows(row..=row, align)
    }

    /// Scroll to a range of rows.
    ///
    /// See [`Self::scroll_to_row`] for details.
    #[inline]
    pub fn scroll_to_rows(mut self, rows: RangeInclusive<u64>, align: Option<Align>) -> Self {
        self.scroll_to_rows = Some((rows, align));
        self
    }

    /// Set a column to scroll to.
    ///
    /// `align` specifies if the column should be positioned in the left, center, or right of the view
    /// (using [`Align::LEFT`], [`Align::Center`] or [`Align::RIGHT`]).
    /// If `align` is `None`, the table will scroll just enough to bring the cursor into view.
    ///
    /// See also: [`Self::scroll_to_row`].
    #[inline]
    pub fn scroll_to_column(self, column: usize, align: Option<Align>) -> Self {
        self.scroll_to_columns(column..=column, align)
    }

    /// Scroll to a range of columns.
    ///
    /// See [`Self::scroll_to_column`] for details.
    #[inline]
    pub fn scroll_to_columns(
        mut self,
        columns: RangeInclusive<usize>,
        align: Option<Align>,
    ) -> Self {
        self.scroll_to_columns = Some((columns, align));
        self
    }

    /// The top y coordinate offset of a specific row nr.
    ///
    /// `get_row_top_offset(0)` should always return 0.0.
    #[expect(clippy::unused_self)] // for uniformity
    fn get_row_top_offset(
        &self,
        ctx: &Context,
        table_id: Id,
        table_delegate: &dyn TableDelegate,
        row_nr: u64,
    ) -> f32 {
        table_delegate.row_top_offset(ctx, table_id, row_nr)
    }

    /// Which row contains the given y offset (from the top)?
    fn get_row_nr_at_y_offset(
        &self,
        ctx: &Context,
        table_id: Id,
        table_delegate: &dyn TableDelegate,
        y_offset: f32,
    ) -> u64 {
        partition_point(0..=self.num_rows, |row_nr| {
            y_offset <= self.get_row_top_offset(ctx, table_id, table_delegate, row_nr)
        })
        .saturating_sub(1)
    }

    pub fn show(mut self, ui: &mut Ui, table_delegate: &mut dyn TableDelegate) -> Response {
        self.num_sticky_cols = self.num_sticky_cols.at_most(self.columns.len());

        let id = TableState::id(ui, self.id_salt);
        let state = TableState::load(ui, id);
        let is_new = state.is_none();
        let do_full_sizing_pass = is_new;
        let mut state = state.unwrap_or_default();

        for (i, column) in self.columns.iter_mut().enumerate() {
            let column_id = column.id_for(i);
            if let Some(existing_width) = state.col_widths.get(&column_id) {
                column.current = *existing_width;
            }
            column.current = column.range.clamp(column.current);

            if do_full_sizing_pass {
                column.auto_size_this_frame = true;
            }
        }

        let parent_width = ui.available_width();
        let auto_size = match self.auto_size_mode {
            AutoSizeMode::Never => false,
            AutoSizeMode::Always => true,
            AutoSizeMode::OnParentResize => state.parent_width != Some(parent_width),
        };
        if auto_size {
            Column::auto_size(&mut self.columns, parent_width);
        }
        state.parent_width = Some(parent_width);

        let col_x = {
            let mut x = ui.cursor().min.x;
            let mut col_x = Vec1::with_capacity(x, self.columns.len() + 1);
            for column in &self.columns {
                x += column.current;
                col_x.push(x);
            }
            col_x
        };

        let header_row_y = {
            let mut y = ui.cursor().min.y;
            let mut sticky_row_y = Vec1::with_capacity(y, self.headers.len() + 1);
            for header in &self.headers {
                y += header.height;
                sticky_row_y.push(y);
            }
            sticky_row_y
        };

        let sticky_size = Vec2::new(
            self.columns[..self.num_sticky_cols]
                .iter()
                .map(|c| c.current)
                .sum(),
            self.headers.iter().map(|h| h.height).sum(),
        );

        let mut ui_builder = UiBuilder::new().layout(Layout::top_down(Align::Min));
        if do_full_sizing_pass {
            ui_builder = ui_builder.sizing_pass().invisible();
            ui.request_discard("Full egui_table sizing");
        }
        let response = ui
            .scope_builder(ui_builder, |ui| {
                // Don't wrap text in the table cells.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); // TODO: I think this is default for horizontal layouts anyway?

                let num_columns = self.columns.len();

                for (col_nr, column) in self.columns.iter_mut().enumerate() {
                    if column.resizable {
                        let column_resize_id = id.with(column.id_for(col_nr)).with("resize");
                        if let Some(response) = ui.read_response(column_resize_id)
                            && response.double_clicked()
                        {
                            column.auto_size_this_frame = true;
                        }
                    }
                    if column.auto_size_this_frame {
                        ui.request_discard("egui_table column sizing");
                    }
                }

                SplitScroll {
                    scroll_enabled: Vec2b::new(true, true),
                    fixed_size: sticky_size,
                    scroll_outer_size: (ui.available_size() - sticky_size).at_least(Vec2::ZERO),
                    scroll_content_size: Vec2::new(
                        self.columns[self.num_sticky_cols..]
                            .iter()
                            .map(|c| c.current)
                            .sum(),
                        self.get_row_top_offset(ui, id, table_delegate, self.num_rows),
                    ),
                    stick_to_bottom: self.stick_to_bottom,
                }
                .show(
                    ui,
                    &mut TableSplitScrollDelegate {
                        id,
                        table_delegate,
                        state: &mut state,
                        table: &mut self,
                        col_x,
                        header_row_y,
                        max_column_widths: vec![0.0; num_columns],
                        visible_column_lines: Default::default(),
                        do_full_sizing_pass,
                        has_prefetched: false,
                        egui_ctx: ui.clone(),
                    },
                );
            })
            .response;

        state.store(ui, id);
        response
    }
}

#[derive(Clone, Copy, Debug)]
struct ColumnResizer {
    scroll_offset: Vec2,

    top: f32,
}

fn update(map: &mut BTreeMap<usize, ColumnResizer>, key: usize, value: ColumnResizer) {
    match map.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(value);
        }
        Entry::Occupied(mut entry) => {
            entry.get_mut().top = entry.get_mut().top.min(value.top);
        }
    }
}

struct TableSplitScrollDelegate<'a> {
    id: Id,
    table_delegate: &'a mut dyn TableDelegate,
    table: &'a mut Table,
    state: &'a mut TableState,

    /// The x coordinate for the start of each column, plus the end of the last column.
    col_x: Vec1<f32>,

    /// The y coordinate for the start of each header row, plus the end of the last header row.
    header_row_y: Vec1<f32>,

    /// Actual width of the widest element in each column
    max_column_widths: Vec<f32>,

    /// Key is column number. The resizer is to the right of the column.
    visible_column_lines: BTreeMap<usize, ColumnResizer>,

    do_full_sizing_pass: bool,

    has_prefetched: bool,

    egui_ctx: Context,
}

impl TableSplitScrollDelegate<'_> {
    /// Helper wrapper around [`Table::get_row_top_offset`].
    fn get_row_top_offset(&self, row_nr: u64) -> f32 {
        self.table
            .get_row_top_offset(&self.egui_ctx, self.id, self.table_delegate, row_nr)
    }

    /// Helper wrapper around [`Table::get_row_nr_at_y_offset`].
    fn get_row_nr_at_y_offset(&self, y_offset: f32) -> u64 {
        self.table
            .get_row_nr_at_y_offset(&self.egui_ctx, self.id, self.table_delegate, y_offset)
    }

    /// The columns that are stuck to the left side.
    fn sticky_columns(&self) -> Range<usize> {
        0..self.table.num_sticky_cols
    }

    /// The columns that scroll horizontally.
    fn scrolling_columns(&self) -> Range<usize> {
        self.table.num_sticky_cols..self.table.columns.len()
    }

    /// The scroll offset that applies to the given column.
    fn column_scroll_offset(&self, col_nr: usize, scroll_offset: Vec2) -> Vec2 {
        if col_nr < self.table.num_sticky_cols {
            Vec2::ZERO
        } else {
            scroll_offset
        }
    }

    /// Show all header groups.
    ///
    /// The sticky columns stay in place, and the other columns move by `scroll_offset`.
    /// A group with a sticky column is never narrower than its sticky part.
    /// Other groups are clipped at the sticky columns, so they slide under them.
    fn header_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        let num_sticky_cols = self.table.num_sticky_cols;

        for (row_nr, header_row) in self.table.headers.iter().enumerate() {
            let groups = if header_row.groups.is_empty() {
                (0..self.table.columns.len()).map(|i| i..i + 1).collect()
            } else {
                header_row.groups.clone()
            };

            let y_range = Rangef::new(self.header_row_y[row_nr], self.header_row_y[row_nr + 1]);

            for (group_index, col_range) in groups.into_iter().enumerate() {
                let start = col_range.start;
                let end = col_range.end;

                let left = self.col_x[start] - self.column_scroll_offset(start, scroll_offset).x;
                let mut right =
                    self.col_x[end] - self.column_scroll_offset(end - 1, scroll_offset).x;
                let mut visible_rect = ui.clip_rect();
                if start < num_sticky_cols {
                    right = right.at_least(self.col_x[end.at_most(num_sticky_cols)]);
                } else {
                    visible_rect.min.x = visible_rect.min.x.at_least(self.col_x[num_sticky_cols]);
                }
                let mut header_rect = Rect::from_x_y_ranges(left..=right, y_range);

                if !self.do_full_sizing_pass
                    && !visible_rect.x_range().intersects(header_rect.x_range())
                {
                    continue; // Not visible
                }

                if 0 < start
                    && self.table.columns[start - 1].resizable
                    && visible_rect.x_range().contains(header_rect.left())
                {
                    // The previous column is resizable, so make sure the resize line goes to above this heading:
                    let scroll_offset = self.column_scroll_offset(start - 1, scroll_offset);
                    update(
                        &mut self.visible_column_lines,
                        start - 1,
                        ColumnResizer {
                            scroll_offset,
                            top: header_rect.top(),
                        },
                    );
                }

                let clip_rect = header_rect.intersect(visible_rect);

                let last_column = &self.table.columns[end - 1];
                let auto_size_this_frame = last_column.auto_size_this_frame; // TODO: correct?

                if auto_size_this_frame {
                    // Note: we shrink the cell rect when auto-sizing, but not the clip rect! This is to avoid flicker.
                    header_rect.max.x = header_rect.min.x
                        + self.table.columns[start..end]
                            .iter()
                            .map(|column| column.range.min)
                            .sum::<f32>();
                }

                let mut ui_builder = UiBuilder::new()
                    .max_rect(header_rect)
                    .id_salt(("header", row_nr, group_index))
                    .layout(egui::Layout::left_to_right(egui::Align::Center));
                if auto_size_this_frame {
                    ui_builder = ui_builder.sizing_pass();
                }
                let mut cell_ui = ui.new_child(ui_builder);
                cell_ui.shrink_clip_rect(clip_rect);

                self.table_delegate.header_cell_ui(
                    &mut cell_ui,
                    &HeaderCellInfo {
                        group_index,
                        col_range,
                        row_nr,
                        table_id: self.id,
                    },
                );

                if start + 1 == end {
                    // normal single-column group
                    let col_nr = start;
                    let column = &self.table.columns[start];
                    let width = &mut self.max_column_widths[col_nr];
                    *width = width.max(cell_ui.min_size().x);

                    // Save column lines for later interaction:
                    if column.resizable && visible_rect.x_range().contains(header_rect.right()) {
                        let scroll_offset = self.column_scroll_offset(col_nr, scroll_offset);
                        update(
                            &mut self.visible_column_lines,
                            col_nr,
                            ColumnResizer {
                                scroll_offset,
                                top: header_rect.top(),
                            },
                        );
                    }
                }
            }
        }
    }

    /// Show the cells of the visible rows in `region_cols`.
    fn region_ui(
        &mut self,
        ui: &mut Ui,
        scroll_offset: Vec2,
        region_cols: Range<usize>,
        do_prefetch: bool,
    ) {
        // Used to find the visible range of columns and rows:
        let viewport = ui.clip_rect().translate(scroll_offset);

        let col_range = if region_cols.is_empty() || viewport.left() == viewport.right() {
            0..0
        } else if self.do_full_sizing_pass {
            // We do the UI for all columns during a sizing pass, so we can auto-size ALL columns
            region_cols
        } else {
            // Only paint the visible columns:
            let col_idx_at = |x: f32| -> usize {
                self.col_x
                    .partition_point(|&col_x| col_x < x)
                    .saturating_sub(1)
                    .at_most(self.table.columns.len() - 1)
            };

            // Clamp to the region, so a column at the border is not shown in both regions:
            let end = (col_idx_at(viewport.max.x) + 1).at_most(region_cols.end);
            col_idx_at(viewport.min.x).clamp(region_cols.start, end)..end
        };

        let row_range = if self.table.num_rows == 0 || viewport.top() == viewport.bottom() {
            0..0
        } else {
            // Only paint the visible rows:
            let row_idx_at = |y: f32| -> u64 {
                let row_nr = self.get_row_nr_at_y_offset(y - self.header_row_y.last());
                row_nr.at_most(self.table.num_rows.saturating_sub(1))
            };

            let margin = if do_prefetch {
                1.0 // Handle possible rounding errors in the syncing of the scroll offsets
            } else {
                0.0
            };

            row_idx_at(viewport.min.y - margin)..row_idx_at(viewport.max.y + margin) + 1
        };

        if do_prefetch {
            self.table_delegate.prepare(&PrefetchInfo {
                num_sticky_columns: self.table.num_sticky_cols,
                visible_columns: col_range.clone(),
                visible_rows: row_range.clone(),
                table_id: self.id,
            });
            self.has_prefetched = true;
        } else {
            debug_assert!(
                self.has_prefetched,
                "SplitScroll delegate methods called in unexpected order"
            );
        }

        for row_nr in row_range {
            let y_range = Rangef::new(
                self.header_row_y.last() + self.get_row_top_offset(row_nr),
                self.header_row_y.last() + self.get_row_top_offset(row_nr + 1),
            );

            let row_x_range = self.col_x[0]..=self.col_x[self.col_x.len() - 1];
            let row_rect = Rect::from_x_y_ranges(row_x_range, y_range).translate(-scroll_offset);

            let mut row_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(row_rect)
                    .id_salt(("row", row_nr))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            row_ui.set_min_size(row_rect.size());

            self.table_delegate.row_ui(&mut row_ui, row_nr);

            for col_nr in col_range.clone() {
                let column = &self.table.columns[col_nr];
                let mut cell_rect =
                    Rect::from_x_y_ranges(self.col_x[col_nr]..=self.col_x[col_nr + 1], y_range)
                        .translate(-scroll_offset);
                let clip_rect = cell_rect;
                if column.auto_size_this_frame {
                    // Note: we shrink the cell rect when auto-sizing, but not the clip rect! This is to avoid flicker.
                    cell_rect.max.x = cell_rect.min.x + column.range.min;
                }

                let mut ui_builder = UiBuilder::new()
                    .max_rect(cell_rect)
                    .id_salt((row_nr, col_nr))
                    .layout(egui::Layout::left_to_right(egui::Align::Center));
                if column.auto_size_this_frame {
                    ui_builder = ui_builder.sizing_pass();
                }
                let mut cell_ui = row_ui.new_child(ui_builder);
                cell_ui.shrink_clip_rect(clip_rect);

                self.table_delegate.cell_ui(
                    &mut cell_ui,
                    &CellInfo {
                        col_nr,
                        row_nr,
                        table_id: self.id,
                    },
                );

                let width = &mut self.max_column_widths[col_nr];
                *width = width.max(cell_ui.min_size().x);
            }
        }

        // Save column lines for later interaction:
        for col_nr in col_range.clone() {
            let column = &self.table.columns[col_nr];
            if column.resizable {
                update(
                    &mut self.visible_column_lines,
                    col_nr,
                    ColumnResizer {
                        scroll_offset,
                        top: *self.header_row_y.last(),
                    },
                );
            }
        }
    }
}

impl SplitScrollDelegate for TableSplitScrollDelegate<'_> {
    // First to be called
    fn right_bottom_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        if self.table.scroll_to_columns.is_some() || self.table.scroll_to_rows.is_some() {
            let mut target_rect = ui.clip_rect(); // no scrolling
            let mut target_align = None;

            if let Some((column_range, align)) = &self.table.scroll_to_columns {
                // Use the first scrollable column as the base, so that offsets start
                // at 0 for the first non-sticky column — mirroring how row_top_offset
                // starts at 0 for the first data row.
                let scrollable_col_x_base = self.col_x[self.table.num_sticky_cols];
                let x_from_column_nr = |col_nr: usize| -> f32 {
                    ui.min_rect().left() + (self.col_x[col_nr] - scrollable_col_x_base)
                };

                let sticky_width = scrollable_col_x_base - self.col_x.first();

                // Subtract sticky_width from the left of the target rect so that when
                // scroll_to_rect aligns the left of the target to the viewport left, the
                // actual column lands just right of the sticky columns (not behind them).
                target_rect.min.x = x_from_column_nr(*column_range.start()) - sticky_width;
                target_rect.max.x = x_from_column_nr(*column_range.end() + 1);
                target_align = target_align.or(*align);
            }

            if let Some((row_range, align)) = &self.table.scroll_to_rows {
                let y_from_row_nr =
                    |row_nr: u64| -> f32 { ui.min_rect().top() + self.get_row_top_offset(row_nr) };

                let sticky_height = self.header_row_y.last() - self.header_row_y.first();

                // Subtract sticky_height from the top of the target rect so that when
                // scroll_to_rect aligns the top of the target to the viewport top, the
                // actual row lands just below the sticky header (not behind it).
                target_rect.min.y = y_from_row_nr(*row_range.start()) - sticky_height;
                target_rect.max.y = y_from_row_nr(*row_range.end() + 1);
                target_align = target_align.or(*align);
            }

            ui.scroll_to_rect(target_rect, target_align);
        }

        self.region_ui(ui, scroll_offset, self.scrolling_columns(), true);
    }

    // We show the whole header in `top_ui`, because a header group can span
    // both the sticky and the scrolling columns.
    fn top_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        self.header_ui(ui, scroll_offset);
    }

    fn left_bottom_ui(&mut self, ui: &mut Ui, scroll_offset: Vec2) {
        self.region_ui(ui, scroll_offset, self.sticky_columns(), false);
    }

    fn finish(&mut self, ui: &mut Ui) {
        // Paint column resize lines

        for (col_nr, ColumnResizer { scroll_offset, top }) in &self.visible_column_lines {
            let col_nr = *col_nr;
            let Some(column) = self.table.columns.get(col_nr) else {
                continue;
            };
            if !column.resizable {
                continue;
            }

            let column_id = column.id_for(col_nr);
            let used_width = column.range.clamp(self.max_column_widths[col_nr]);

            let column_width = self
                .state
                .col_widths
                .entry(column_id)
                .or_insert(column.current);

            let layout_width = *column_width; // Width used when computing col_x

            if ui.is_sizing_pass() || column.auto_size_this_frame {
                // Shrink to fit the widest element in the column:
                *column_width = used_width;
            } else {
                // Grow to fit the widest element in the column:
                *column_width = column_width.max(used_width);
            }

            let column_resize_id = self.id.with(column.id_for(col_nr)).with("resize");

            // Right side of the column, adjusted for any width change since layout:
            let mut x = self.col_x[col_nr + 1] - scroll_offset.x + (*column_width - layout_width);
            let yrange = Rangef::new(*top, ui.clip_rect().bottom());
            let line_rect = egui::Rect::from_x_y_ranges(x..=x, yrange)
                .expand(ui.style().interaction.resize_grab_radius_side);

            let resize_response =
                ui.interact(line_rect, column_resize_id, egui::Sense::click_and_drag());

            if resize_response.dragged()
                && let Some(pointer) = ui.pointer_latest_pos()
            {
                // Drag-to-resize.
                // TODO: use `ui.intrinsic_size` (once it exist) to prevent
                // resizing below what the content can fit within.
                let new_width = *column_width + pointer.x - x;
                let new_width = column.range.clamp(new_width);
                x += new_width - *column_width;
                *column_width = new_width;
            }

            let dragging_something_else =
                ui.input(|i| i.pointer.any_down() || i.pointer.any_pressed());
            let resize_hover = resize_response.hovered() && !dragging_something_else;

            if resize_hover || resize_response.dragged() {
                ui.set_cursor_icon(egui::CursorIcon::ResizeColumn);
            }

            let stroke = if resize_response.dragged() {
                ui.style().visuals.widgets.active.bg_stroke
            } else if resize_hover {
                ui.style().visuals.widgets.hovered.bg_stroke
            } else {
                // ui.visuals().widgets.inactive.bg_stroke
                ui.visuals().widgets.noninteractive.bg_stroke
            };

            ui.painter().vline(x, yrange, stroke);
        }
    }
}

/// Returns the index of the first element that returns `true` using binary search.
fn partition_point(range: RangeInclusive<u64>, second_partition: impl Fn(u64) -> bool) -> u64 {
    let mut min = *range.start();
    let mut max = *range.end();

    debug_assert!(min < max, "Bad call to partition_point");

    while min < max {
        let mid = min + (max - min) / 2;

        if second_partition(mid) {
            max = mid;
        } else {
            min = mid + 1;
        }
    }

    min
}

#[cfg(test)]
mod tests {
    use crate::table::partition_point;

    #[test]
    fn test_partition_point() {
        assert_eq!(partition_point(0..=17, |i| 8 <= i), 8);
        assert_eq!(partition_point(0..=17, |i| 9 <= i), 9);
        assert_eq!(partition_point(10..=17, |_| true), 10);
        assert_eq!(partition_point(10..=17, |_| false), 17);
    }
}
