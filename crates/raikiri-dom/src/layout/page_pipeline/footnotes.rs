use super::{Document, PageOrigins, materialize_y};
use std::collections::HashMap;

/// The footnote area at the bottom of each page, filled upward in source order.
#[derive(Default)]
pub(super) struct FootnoteAreas {
    /// The height of the notes already stacked on each page.
    used: HashMap<u32, f32>,
}

impl FootnoteAreas {
    /// Finds the page that owns a footnote and retains its ordinary-flow height.
    pub(super) fn candidate(
        &self,
        raw_y: f32,
        height: f32,
        flow_shift: f32,
        current_page: u32,
        pages: &PageOrigins,
    ) -> FootnotePlacement {
        let effective_y = raw_y + flow_shift;
        let anchor_page = pages
            .page_index_in_flow(effective_y)
            .unwrap_or(current_page);
        FootnotePlacement {
            page: current_page.max(anchor_page),
            height,
        }
    }

    /// Places a footnote at the bottom of its page, above earlier notes.
    pub(super) fn place(
        &mut self,
        document: &mut Document,
        node_id: usize,
        placement: FootnotePlacement,
        pages: &PageOrigins,
    ) {
        let page_start = pages.origin(placement.page);
        let page_end = page_start + pages.step_at(placement.page);
        let used = self.used.entry(placement.page).or_insert(0.0);
        let top = (page_end - *used - placement.height).max(page_start);
        *used += placement.height;
        materialize_y(document, node_id, top);
    }
}

/// A footnote's page and normal-flow height before placement.
#[derive(Clone, Copy)]
pub(super) struct FootnotePlacement {
    pub(super) page: u32,
    pub(super) height: f32,
}
