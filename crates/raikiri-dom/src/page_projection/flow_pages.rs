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
mod tests {
    use super::FlowPages;
    use crate::page_projection::records::PageFragment;

    fn pages(ranges: &[Option<(f32, f32)>]) -> Vec<PageFragment> {
        ranges
            .iter()
            .enumerate()
            .map(|(index, &range)| {
                let mut page = PageFragment::new();
                page.page_index = index as u32;
                page.flow_range = range;
                page
            })
            .collect()
    }

    fn indices<'p>(found: impl Iterator<Item = &'p PageFragment>) -> Vec<u32> {
        found.map(|page| page.page_index).collect()
    }

    #[test]
    fn pages_around_a_range_skip_pages_without_flow_and_far_pages() {
        let pages = pages(&[
            Some((0.0, 100.0)),
            None,
            Some((100.0, 200.0)),
            Some((200.0, 300.0)),
        ]);
        let flow = FlowPages::new(&pages);
        assert_eq!(indices(flow.around(150.0, 150.0)), [2]);
        assert_eq!(indices(flow.around(50.0, 250.0)), [0, 2, 3]);
        // An empty range, as left by lines without a finite center.
        assert!(indices(flow.around(f32::INFINITY, f32::NEG_INFINITY)).is_empty());
    }

    #[test]
    fn pages_with_unordered_ends_are_searched_from_the_first_page() {
        // The first range ends after the second, so ends cannot be
        // binary-searched and every page starting before the range is kept.
        let pages = pages(&[
            Some((0.0, 500.0)),
            Some((100.0, 200.0)),
            Some((600.0, 700.0)),
        ]);
        let flow = FlowPages::new(&pages);
        assert_eq!(indices(flow.around(300.0, 300.0)), [0, 1]);
    }

    #[test]
    fn pages_around_a_center_keep_its_owner_at_large_coordinates() {
        // Above 2^26 the spacing of f32 values is 8, wider than the slack.
        let start = 67_108_864.0_f32;
        let pages = pages(&[Some((start - 8.0, start)), Some((start, start + 8.0))]);
        let flow = FlowPages::new(&pages);
        let (top, height) = (start, 10.0);
        let bottom = top + height;
        let center = (top + bottom) * 0.5;
        let owners: Vec<_> = pages
            .iter()
            .filter(|page| {
                let (start, end) = page.flow_range.unwrap();
                crate::layout::line_center_on_page(top, bottom, start, end)
            })
            .map(|page| page.page_index)
            .collect();
        assert_eq!(owners, [1]);
        assert!(indices(flow.around(center, center)).contains(&1));
    }
}
