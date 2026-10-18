//! What pagination carries from one candidate of the flow to the next.

use super::{
    CascadeResult, DeferredShifts, FloatShifts, IfcBoxShifts, PageCandidate, PageOrigins,
    SubtreeShifts, TextLineIndex, grow_ancestors_for_shift,
};
use crate::layout::context::LayoutSession;

/// Where the flow stands while its candidates are placed in source order: how
/// far it has moved down, the page it is on, and the shifts waiting for it to
/// leave a subtree or reach a moved line.
#[derive(Default)]
pub(super) struct PaginationState {
    /// How far the flow after the candidates placed so far has moved down.
    pub(super) flow_shift: f32,
    /// The page the flow is on.
    pub(super) current_page: u32,
    pub(super) deferred_shifts: DeferredShifts,
    pub(super) subtree_shifts: SubtreeShifts,
    pub(super) float_shifts: FloatShifts,
}

impl PaginationState {
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
