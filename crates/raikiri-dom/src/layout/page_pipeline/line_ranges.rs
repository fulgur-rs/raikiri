//! Line ranges for page slices of one immutable paragraph.

use super::{PageFragmentLineRange, line_range_for_page};

/// Borrows a source's metrics for all of its intersecting page slices.
pub(super) struct LineRanges<'a> {
    metrics: &'a [(f32, f32)],
    abs_y: f32,
    monotone: bool,
}

impl<'a> LineRanges<'a> {
    pub(super) fn new(metrics: &'a [(f32, f32)], abs_y: f32) -> Self {
        let mut previous = f32::NEG_INFINITY;
        let monotone = metrics.iter().copied().all(|metric| {
            let center = line_center(metric, abs_y);
            let ordered = center.is_finite() && center >= previous;
            previous = center;
            ordered
        });
        Self {
            metrics,
            abs_y,
            monotone,
        }
    }

    pub(super) fn range(&self, start: f32, end: f32) -> Option<PageFragmentLineRange> {
        if !self.monotone {
            return line_range_for_page(self.metrics, self.abs_y, start, end);
        }
        let lower = start - 0.001;
        let upper = end - 0.001;
        if lower.is_nan() || upper.is_nan() || lower >= upper {
            return None;
        }
        let first = self
            .metrics
            .partition_point(|&metric| line_center(metric, self.abs_y) < lower);
        let end = self
            .metrics
            .partition_point(|&metric| line_center(metric, self.abs_y) < upper);
        (first < end)
            .then(|| PageFragmentLineRange::new(first as u32, ((end - 1) as u32).saturating_add(1)))
    }
}

fn line_center((top, bottom): (f32, f32), abs_y: f32) -> f32 {
    #[cfg(test)]
    super::tests::LINE_CENTER_EVALUATIONS.with(|count| count.set(count.get() + 1));
    let top = abs_y + top;
    let bottom = abs_y + bottom;
    (top + bottom) * 0.5
}
