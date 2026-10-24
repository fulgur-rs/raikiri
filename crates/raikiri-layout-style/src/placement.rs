use crate::LayoutStyleView;
use raikiri_style::property::GridLineValue;

impl LayoutStyleView<'_> {
    /// Whether the box names a grid line or span in any of `grid-row-start`,
    /// `grid-row-end`, `grid-column-start` and `grid-column-end`, rather than
    /// leaving all four to auto-placement.
    pub fn has_grid_placement(&self) -> bool {
        let values = &self.prepared.values;
        [
            &values.grid_row_start,
            &values.grid_row_end,
            &values.grid_column_start,
            &values.grid_column_end,
        ]
        .into_iter()
        .any(names_a_line)
    }
}

/// Whether the value names a line or a span, as opposed to `auto`.
fn names_a_line(value: &GridLineValue) -> bool {
    matches!(
        value,
        GridLineValue::Line(_)
            | GridLineValue::Named(_)
            | GridLineValue::NamedLine(..)
            | GridLineValue::Span(_)
            | GridLineValue::SpanNamed(..)
    )
}
