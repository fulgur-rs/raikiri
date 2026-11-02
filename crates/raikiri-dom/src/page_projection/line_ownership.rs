//! Center-owner geometry shared by candidates in one projection preparation.

use super::records::{FlowAxis, PageFragment};
use std::collections::HashMap;

/// Memoizes the first center-owning range in the original page-slice order.
/// Only intersecting Y-flow candidates outside the center page need a lookup.
/// Entries end with this preparation, and hold no document or page mutation.
pub(super) struct LineOwnership<'p> {
    pages: &'p [PageFragment],
    center_heights: HashMap<(u32, u32), Option<f32>>,
}

impl<'p> LineOwnership<'p> {
    pub(super) fn new(pages: &'p [PageFragment]) -> Self {
        Self {
            pages,
            center_heights: HashMap::new(),
        }
    }

    /// Whether the page owns this transformed line extent, including overflow.
    /// Center ownership, flow-axis and intersection checks precede memoization.
    pub(super) fn owns(&mut self, page: &PageFragment, top: f32, bottom: f32) -> bool {
        page.flow_range.is_some_and(|(start, end)| {
            crate::layout::line_center_on_page(top, bottom, start, end)
                || matches!(page.flow_axis, FlowAxis::Y)
                    && top < end - 0.01
                    && bottom > start + 0.01
                    && self
                        .center_height(top, bottom)
                        .is_some_and(|height| bottom - top > height)
        })
    }

    fn center_height(&mut self, top: f32, bottom: f32) -> Option<f32> {
        let pages = self.pages;
        *self
            .center_heights
            .entry((top.to_bits(), bottom.to_bits()))
            .or_insert_with(|| {
                pages.iter().find_map(|page| {
                    #[cfg(test)]
                    super::work_tests::line_owner_probe();
                    page.flow_range
                        .filter(|&(start, end)| {
                            crate::layout::line_center_on_page(top, bottom, start, end)
                        })
                        .map(|(start, end)| end - start)
                })
            })
    }
}

#[cfg(test)]
mod tests;
