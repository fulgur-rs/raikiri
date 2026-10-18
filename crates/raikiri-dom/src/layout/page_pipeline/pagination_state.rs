//! What pagination carries from one candidate of the flow to the next.

use super::{
    CascadeResult, CheckedParagraphs, ColumnContexts, DeferredShifts, EnteredParagraphs,
    FloatShifts, FootnoteAreas, HeaderRepeats, IfcBoxShifts, LastNamedBox, PageCandidate,
    PageNames, PageOrigins, PageReach, PendingBreak, PrecedingFlexItems, SiblingMargins,
    SubtreeShifts, TextLineIndex, TrailingFlexChildren, grow_ancestors_for_shift,
};
use crate::layout::context::LayoutSession;

/// Where pagination stands while the candidates of the flow are placed in
/// source order, and what it has to remember from the ones placed so far.
pub(super) struct PaginationState {
    /// How far the flow after the candidates placed so far has moved down.
    pub(super) flow_shift: f32,
    /// The page the flow is on.
    pub(super) current_page: u32,
    pub(super) deferred_shifts: DeferredShifts,
    pub(super) subtree_shifts: SubtreeShifts,
    pub(super) float_shifts: FloatShifts,
    pub(super) reach: PageReach,
    pub(super) page_names: PageNames,
    pub(super) pending_break: PendingBreak,
    pub(super) sibling_margins: SiblingMargins,
    pub(super) last_named: LastNamedBox,
    /// Whether any box was placed yet.
    pub(super) saw_child: bool,
    pub(super) checked_paragraphs: CheckedParagraphs,
    /// Footnotes are collected in source order and stacked upward from the
    /// bottom edge of their anchor page. The normal-flow height is removed
    /// from `flow_shift`, leaving the box only in this page-local area.
    pub(super) footnote_areas: FootnoteAreas,
    pub(super) column_contexts: ColumnContexts,
    pub(super) preceding_flex_items: PrecedingFlexItems,
    pub(super) trailing_flex_children: TrailingFlexChildren,
    pub(super) entered_paragraphs: EnteredParagraphs,
    pub(super) table_headers: HeaderRepeats,
}

impl PaginationState {
    /// The state before the first candidate: the flow is on the first page,
    /// unshifted, and nothing is pending. `page_names` starts with the page
    /// type of the first page, and `column_contexts` with the nodes of the
    /// document.
    pub(super) fn new(
        table_headers: HeaderRepeats,
        page_names: PageNames,
        column_contexts: ColumnContexts,
    ) -> Self {
        Self {
            flow_shift: 0.0,
            current_page: 0,
            deferred_shifts: DeferredShifts::default(),
            subtree_shifts: SubtreeShifts::default(),
            float_shifts: FloatShifts::default(),
            reach: PageReach::default(),
            page_names,
            pending_break: PendingBreak::default(),
            sibling_margins: SiblingMargins::default(),
            last_named: LastNamedBox::default(),
            saw_child: false,
            checked_paragraphs: CheckedParagraphs::default(),
            footnote_areas: FootnoteAreas::default(),
            column_contexts,
            preceding_flex_items: PrecedingFlexItems::default(),
            trailing_flex_children: TrailingFlexChildren::default(),
            entered_paragraphs: EnteredParagraphs::default(),
            table_headers,
        }
    }

    /// Settles the shifts that end or begin at `candidate`, before it is
    /// placed: those it has reached, those of the subtrees and the moved
    /// float it is outside of, and the clearance below the moved floats of
    /// its formatting context.
    pub(super) fn settle_shifts(
        &mut self,
        document: &mut LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        candidate: &PageCandidate,
        text_lines: &TextLineIndex,
        page_origins: &PageOrigins,
    ) {
        let node_id = candidate.node_id;
        self.deferred_shifts.apply_reached(
            document,
            text_lines,
            node_id,
            candidate.ifc_root,
            &mut self.flow_shift,
        );
        if let Some(page) = self
            .subtree_shifts
            .leave(document, node_id, &mut self.flow_shift)
        {
            self.current_page = page;
        }
        if let Some((delta, flow_page)) = self.float_shifts.end_pending_outside(document, node_id) {
            self.flow_shift -= delta;
            self.current_page = flow_page;
        }
        if let Some(clearance) =
            self.float_shifts
                .clearance_for(document, cascade, body_id, candidate, self.flow_shift)
        {
            self.flow_shift += clearance.shift;
            if clearance.floating {
                self.float_shifts.hold_cleared_float(
                    document,
                    cascade,
                    node_id,
                    &clearance,
                    self.flow_shift,
                    self.current_page,
                );
            } else if !candidate.context.is_direct_body_element {
                // The ancestors of a nested clearing box grow to hold it.
                grow_ancestors_for_shift(
                    document,
                    cascade,
                    node_id,
                    clearance.shift,
                    body_id,
                    &mut self.subtree_shifts,
                    self.current_page,
                    &IfcBoxShifts::default(),
                );
            }
            // The box may now start on a later page than the flow.
            self.current_page = self
                .current_page
                .max(page_origins.page_index_for_y(candidate.raw_y + self.flow_shift));
        }
    }
}
