//! Pages that a range of normal-flow positions can land on.

use super::records::PageFragment;

/// Pages with a flow range, ordered by their start, so a paragraph that is not
/// repeated on every page visits only the pages its lines can land on instead
/// of every page.
pub(super) struct FlowPages<'p> {
    by_flow: Vec<(f32, f32, &'p PageFragment)>,
    ends_ordered: bool,
}

impl<'p> FlowPages<'p> {
    pub(super) fn new(pages: &'p [PageFragment]) -> Self {
        let mut by_flow: Vec<_> = pages
            .iter()
            .filter_map(|page| page.flow_range.map(|(start, end)| (start, end, page)))
            .collect();
        by_flow.sort_by(|left, right| left.0.total_cmp(&right.0));
        let ends_ordered = by_flow.windows(2).all(|pair| pair[0].1 <= pair[1].1);
        Self {
            by_flow,
            ends_ordered,
        }
    }

    /// The pages whose flow range can hold a line centered between `low` and
    /// `high`, in flow order. A page without a flow range cannot own a
    /// normal-flow line, so it is never a candidate.
    ///
    /// Callers compute each center with the arithmetic of
    /// [`crate::layout::line_center_on_page`], which still decides which page
    /// owns each line; the slack keeps the pruning conservative against it.
    pub(super) fn around(
        &self,
        low: f32,
        high: f32,
    ) -> impl Iterator<Item = &'p PageFragment> + '_ {
        let last = self
            .by_flow
            .partition_point(|&(start, _, _)| start - 1.0 <= high);
        let first = if self.ends_ordered {
            self.by_flow[..last].partition_point(|&(_, end, _)| end + 1.0 <= low)
        } else {
            0
        };
        self.by_flow[first..last].iter().map(|&(_, _, page)| page)
    }
}

#[cfg(test)]
mod tests;
