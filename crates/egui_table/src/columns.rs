//! Logic for constrained column auto-sizing.

use egui::Rangef;

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
pub struct Column {
    pub current: f32,
    pub range: Rangef,
    pub id: Option<egui::Id>,
    pub resizable: bool,
    pub auto_size_this_frame: bool,
}

impl Default for Column {
    fn default() -> Self {
        Self {
            current: 100.0,
            range: Rangef::new(4.0, f32::INFINITY),
            id: None,
            resizable: true,
            auto_size_this_frame: false,
        }
    }
}

impl Column {
    /// Current and/or initial column width.
    ///
    /// To avoid rounding error you should keep this to a precise value, e.g. a multiple of `0.25`.
    #[inline]
    pub fn new(current: f32) -> Self {
        Self {
            current,
            ..Default::default()
        }
    }

    /// Allowed width range.
    ///
    /// To avoid rounding error you should keep this to a precise value, e.g. a multiple of `0.25`.
    #[inline]
    pub fn range(mut self, range: impl Into<Rangef>) -> Self {
        self.range = range.into();
        self
    }

    /// Optional unique id within the parent table.
    ///
    /// If not set, the column index is used.
    #[inline]
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    /// Can the user resize this column?
    #[inline]
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// If set, we should accurately measure the size of this column this frame
    /// so that we can correctly auto-size it.
    ///
    /// This is done as an egui `sizing_pass`.
    pub fn auto_size_this_frame(mut self, auto_size_this_frame: bool) -> Self {
        self.auto_size_this_frame = auto_size_this_frame;
        self
    }

    #[inline]
    pub fn id_for(&self, col_idx: usize) -> egui::Id {
        self.id.unwrap_or_else(|| egui::Id::unique(col_idx))
    }

    /// Resize columns to fit the total width.
    pub fn auto_size(columns: &mut [Self], target_width: f32) {
        if columns.is_empty() {
            return;
        }

        // Make sure all columns have a valid range.
        let mut min_width = 0.0;
        let mut max_width = 0.0;
        let mut current_width = 0.0;
        for column in columns.iter_mut() {
            column.current = column.range.clamp(column.current);
            min_width += column.range.min;
            max_width += column.range.max;
            current_width += column.current;
        }

        if current_width == target_width {
            return; // We're good
        }

        let wants_to_grow = current_width < target_width;
        let sign = if wants_to_grow { 1.0 } else { -1.0 };

        if wants_to_grow && max_width <= current_width {
            return; // Can't grow
        }
        if !wants_to_grow && current_width <= min_width {
            return; // Can't shrink
        }

        // Which columns has room to change, and by how much (abs)?
        let mut can_change: Vec<(f32, usize)> = columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if wants_to_grow && c.current < c.range.max {
                    return Some((c.range.max - c.current, i));
                }
                if !wants_to_grow && c.range.min < c.current {
                    return Some((c.current - c.range.min, i));
                }
                None
            })
            .collect();

        if can_change.is_empty() {
            return; // No columns can change
        }

        // Put the columns that has the most room to change first:
        can_change.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("NaN or Inf in column code"));
        debug_assert!(
            can_change[0].0 >= can_change.last().expect("Can't be empty").0,
            "The sort is broken"
        );

        let mut remaining_abs = (target_width - current_width).abs();

        while let Some((room_in_least, least_idx)) = can_change.pop() {
            let evenly_distributed = remaining_abs / (can_change.len() as f32 + 1.0);

            if evenly_distributed <= room_in_least {
                // Distribute evenly, and we're done:
                columns[least_idx].current += sign * evenly_distributed;
                for (_, i) in can_change {
                    columns[i].current += sign * evenly_distributed;
                }
                return;
            }

            // Put as much as we can in the least column, then continue:
            if wants_to_grow {
                columns[least_idx].current = columns[least_idx].range.max;
            } else {
                columns[least_idx].current = columns[least_idx].range.min;
            }
            remaining_abs -= room_in_least;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(c: i32, range: std::ops::RangeInclusive<i32>) -> Column {
        Column::new(c as f32).range(Rangef::new(*range.start() as f32, *range.end() as f32))
    }

    fn widths(columns: &[Column]) -> Vec<f32> {
        columns.iter().map(|c| c.current).collect()
    }

    #[test]
    fn test_single_column() {
        let mut columns = [col(0, 100..=200)];
        Column::auto_size(&mut columns, 50.0);
        assert_eq!(widths(&columns), [100.0]);
        Column::auto_size(&mut columns, 132.0);
        assert_eq!(widths(&columns), [132.0]);
        Column::auto_size(&mut columns, 500.0);
        assert_eq!(widths(&columns), [200.0]);
    }

    #[test]
    fn test_many_columns() {
        let mut columns = [col(15, 10..=20), col(25, 10..=100), col(150, 100..=200)];

        Column::auto_size(&mut columns, 190.0);
        assert_eq!(
            widths(&columns),
            [15.0, 25.0, 150.0],
            "They have no need to grow"
        );

        Column::auto_size(&mut columns, 193.0);
        assert_eq!(
            widths(&columns),
            [16.0, 26.0, 151.0],
            "They should grow equally"
        );

        Column::auto_size(&mut columns, 187.0);
        assert_eq!(
            widths(&columns),
            [14.0, 24.0, 149.0],
            "They should shrink equally"
        );

        Column::auto_size(&mut columns, 207.0);
        assert_eq!(
            widths(&columns),
            [20.0, 31.0, 156.0],
            "They should saturate the first column, then spread equally"
        );
    }
}
