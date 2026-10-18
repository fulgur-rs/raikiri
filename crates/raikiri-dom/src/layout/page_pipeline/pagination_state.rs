//! What pagination carries from one candidate of the flow to the next.

use super::candidates::FlowRoots;
use super::{
    CascadeResult, CheckedParagraphs, ColumnContexts, DeferredShifts, EnteredParagraphs,
    FloatShifts, FootnoteAreas, HeaderRepeats, IfcBoxShifts, LastNamedBox, LayoutError,
    LineBreakFix, LineMove, PageCandidate, PageLayoutControl, PageNames, PageOrigins, PageReach,
    ParagraphLineFit, PendingBreak, PrecedingFlexItems, SiblingMargins, SubtreeShifts,
    TextLineIndex, TrailingFlexChildren, block_effective_y, block_fits_page_step,
    flex_line_paragraph, forced_target_page, grow_ancestors_for_shift,
    has_explicit_positioned_block_ancestor, has_fixed_height, has_footnote_ancestor,
    line_block_parent, lines_can_shift_inside, move_line_block, move_paragraph_lines,
    page_type_leaves_local_flow, paragraph_line_bounds, participates_in_margin_adjacency,
    place_text_candidate, positioned_text_start_y, reaches_line, record_candidate_pages,
    shift_text_candidate, text_changes_page_name, text_sets_page_name,
};
use crate::layout::context::LayoutSession;

/// What is the same for every candidate of the flow.
pub(super) struct FlowEnv<'a> {
    pub(super) cascade: &'a CascadeResult,
    pub(super) body_id: usize,
    pub(super) control: &'a PageLayoutControl<'a>,
    pub(super) page_origins: &'a PageOrigins,
    pub(super) text_lines: &'a TextLineIndex,
    pub(super) roots: FlowRoots,
    pub(super) root_flow_offset: f32,
}

/// Whether pagination goes on with the next candidate after one was placed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Flow {
    Next,
    /// The page limit ends pagination.
    Stop,
}

/// Returns [`Flow::Stop`] from the enclosing method when `$page_index` is
/// past the page limit of a control that truncates there.
macro_rules! check_page_limit {
    ($control:expr, $page_index:expr) => {
        if !$control.within_page_limit($page_index)? {
            return Ok(Flow::Stop);
        }
    };
}

/// What the start of a candidate settled for its placement.
pub(super) struct CandidateEntry {
    /// Whether content already placed in the candidate's flow lets a named
    /// page change open a new page.
    pub(super) name_context_seen: bool,
    /// The page type the candidate compares its own against to detect that
    /// change.
    pub(super) comparison_page_name: Option<String>,
    /// Whether leaving a flex item's internal named flow keeps the last
    /// opened physical page until an outer boundary or natural overflow opens
    /// one.
    pub(super) keep_local_page_type: bool,
    /// The page the flow was on before the candidate.
    pub(super) page_before_candidate: u32,
    /// The root of the candidate's paragraph and its offset, when the
    /// candidate is the first inside all the paragraphs around it and so
    /// moves the root.
    pub(super) ifc_root: Option<(usize, f32)>,
    /// Whether the margins of the candidate adjoin those of its siblings.
    pub(super) participates_in_margin_adjacency: bool,
    /// Whether the candidate follows its containing box through explicit
    /// positioned insets, rather than the normal flow.
    pub(super) uses_positioned_coordinates: bool,
}

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

    /// Starts the placement of `candidate`: skips the lines of a repeated
    /// table header and candidates that are not source nodes, settles the
    /// shifts that end or begin at it, and decides the page type it compares
    /// against and the paragraph it moves. `None` when the candidate is not
    /// placed.
    pub(super) fn begin_candidate(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
    ) -> Result<Option<CandidateEntry>, LayoutError> {
        let node_id = candidate.node_id;
        // Header lines stay together and are placed from the table's rows,
        // rather than acquiring independent text-page shifts.
        if self.table_headers.owner(node_id).is_some() {
            return Ok(None);
        }
        let name_participates_in_flow = candidate.context.page_name_in_flow;
        if name_participates_in_flow
            && candidate.context.is_flex_item
            && let Some(context) = candidate.named_flex_context
            && self
                .preceding_flex_items
                .record(context, candidate.height, self.current_page)
        {
            self.column_contexts
                .occupy(Some(context), self.current_page, document, env.cascade);
        }
        let (name_context_seen, comparison_page_name) = self.page_names.comparison(
            candidate,
            self.saw_child,
            &mut self.column_contexts,
            document,
            env.cascade,
            self.current_page,
        );
        // Leaving a flex item's internal named flow keeps the last opened
        // physical page until an outer boundary or natural overflow opens one.
        let keep_local_page_type = candidate.named_flex_context.is_none()
            && self.page_names.current != comparison_page_name;
        let page_before_candidate = self.current_page;
        let moves_ifc_root = self
            .entered_paragraphs
            .enter(document, candidate.ifc_root, node_id);
        let ifc_root = candidate.ifc_root.filter(|_| moves_ifc_root);
        self.settle_shifts(document, env, candidate);
        if !document.is_source_node(node_id) {
            return Ok(None);
        }
        let participates_in_margin_adjacency =
            participates_in_margin_adjacency(document, env.cascade, candidate);
        // Explicit positioned insets follow their containing box, not margin
        // changes in a parallel normal flow. Both-auto boxes keep static placement.
        let uses_positioned_coordinates = candidate.context.is_out_of_flow_descendant
            && has_explicit_positioned_block_ancestor(document, node_id);
        Ok(Some(CandidateEntry {
            name_context_seen,
            comparison_page_name,
            keep_local_page_type,
            page_before_candidate,
            ifc_root,
            participates_in_margin_adjacency,
            uses_positioned_coordinates,
        }))
    }

    /// Places the in-flow text `candidate` after its start: moves the
    /// paragraph or lines that would break badly at a page edge, opens the
    /// page a pending break or a named page change asks for, and records the
    /// pages the text occupies.
    pub(super) fn place_text(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
    ) -> Result<Flow, LayoutError> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            text_lines,
            roots,
            root_flow_offset,
        } = *env;
        let CandidateEntry {
            name_context_seen,
            ref comparison_page_name,
            keep_local_page_type,
            page_before_candidate,
            ifc_root,
            participates_in_margin_adjacency,
            uses_positioned_coordinates,
        } = *entry;
        let node_id = candidate.node_id;
        let name_participates_in_flow = candidate.context.page_name_in_flow;
        let page_origin = |page_index: u32| page_origins.origin(page_index);
        let page_step_at = |page_index: u32| page_origins.step_at(page_index);
        let page_index_in_flow = |y: f32| page_origins.page_index_in_flow(y);
        // The inline layout root whose lines hold this text, if any.
        let text_ifc_root = candidate.ifc_root.map(|(root, _)| root);
        // In-flow text between two blocks separates them: the next block's
        // gap is no longer a margin adjoining the previous block.
        if participates_in_margin_adjacency {
            self.sibling_margins
                .end_run_at_text(document, node_id, text_ifc_root);
        }
        // A footnote's descendants follow the moved footnote box. They are
        // deliberately not paginated independently, otherwise the stale
        // pre-pagination coordinates would move them back into normal flow.
        if candidate.context.is_float_descendant
            && has_footnote_ancestor(cascade, node_id, document)
        {
            return Ok(Flow::Next);
        }
        // Text is not itself a class-A box, but non-empty text can be the
        // content that follows a nested named box.  Carry the containing
        // box's page type so that content after `page:b` inside a
        // `page:a` box resumes on an `a` page rather than becoming an
        // anonymous page.  Only direct body text consumes a pending
        // break-after; descendants must wait until their containing box
        // has finished.
        let raw_y = candidate.raw_y;
        let mut height = candidate.height;
        let candidate_page_name = candidate.page_name.clone();
        let mut effective_y = if uses_positioned_coordinates {
            positioned_text_start_y(document, text_lines, node_id, text_ifc_root)
        } else {
            raw_y + self.flow_shift
        };
        // A fixed-height block whose first line would straddle the
        // current fragmentainer is pushed as a unit. This matches normal
        // block fragmentation for monolithic content while leaving tall
        // blocks and already-forced page transitions to the existing
        // fragment logic.
        // A block inside an inline element of an inline engine paragraph
        // is laid out from the paragraph's root, as a direct child is.
        // A block nested in plain block flow is moved the same way; its
        // ancestors then grow to hold it.
        // The block of text in an IFC paragraph is its root, including nested inline elements.
        let paragraph_root = candidate
            .ifc_root
            .map(|(root, _)| root)
            .filter(|&root| document.is_source_node(root) && root != node_id);
        let line_block_parent = line_block_parent(
            document,
            cascade,
            &roots,
            candidate,
            paragraph_root,
            &self.checked_paragraphs,
            uses_positioned_coordinates,
        );
        if let Some((block_id, nested)) = line_block_parent
            && self.checked_paragraphs.block_text.insert(block_id)
            && has_fixed_height(document, block_id)
        {
            let block_height = document.nodes[block_id].unrounded_layout.size.height;
            let block_effective_y = block_effective_y(
                document,
                block_id,
                raw_y,
                effective_y,
                root_flow_offset,
                self.flow_shift,
            );
            if let Some(target_page) = page_origins.first_line_push_page(
                effective_y,
                height,
                block_effective_y,
                block_height,
                self.current_page,
            ) {
                check_page_limit!(control, target_page);
                let target_y = page_origin(target_page);
                let delta = move_line_block(
                    document,
                    cascade,
                    body_id,
                    block_id,
                    nested,
                    block_effective_y,
                    target_y,
                    &mut self.subtree_shifts,
                    page_before_candidate,
                );
                self.flow_shift += delta;
                effective_y += delta;
            }
        }
        // `orphans` and `widows` constrain breaks between line boxes, not
        // breaks between block-level children.  When the paragraph has
        // lines enough for both, a break that leaves too few widows moves
        // up between earlier lines; otherwise a fitting block is moved as
        // a unit when its natural split would violate either constraint.
        // Oversized blocks that cannot break earlier keep their natural
        // break.
        // A paragraph that is a flex item alone on its line cannot move
        // as a unit, but its later lines can move down inside it. It is
        // looked for only when no line block parent was found.
        let flex_line_paragraph = flex_line_paragraph(
            document,
            cascade,
            body_id,
            candidate,
            paragraph_root.filter(|_| line_block_parent.is_none()),
            uses_positioned_coordinates,
        );
        if let Some((block_id, nested)) =
            line_block_parent.or(flex_line_paragraph.map(|(root, _)| (root, true)))
            && self.checked_paragraphs.orphans_widows.insert(block_id)
            && let Some(line_metrics) = paragraph_line_bounds(document, node_id, text_lines)
        {
            let page_start = page_origin(self.current_page);
            let page_end = page_start + page_step_at(self.current_page);
            let line_fit = ParagraphLineFit::measure(
                &line_metrics,
                effective_y,
                page_start,
                page_end,
                cascade.computed[block_id].orphans,
                cascade.computed[block_id].widows,
            );
            let block_height = document.nodes[block_id].unrounded_layout.size.height;
            let can_move_block = line_block_parent.is_some()
                && block_fits_page_step(block_height, page_step_at(self.current_page));
            let fix = line_fit.fix(
                &line_metrics,
                effective_y,
                self.current_page,
                page_origins,
                can_move_block,
                // Lines move down inside the paragraph only when it is
                // its own inline layout root, has no earlier split
                // pending, and can shift its lines.
                || {
                    paragraph_root == Some(block_id)
                        && self.deferred_shifts.line_splits.is_empty()
                        && lines_can_shift_inside(document, block_id)
                },
            );
            match fix {
                LineBreakFix::MoveLines(moves) => {
                    for (line, shift, page) in moves.into_iter().flatten() {
                        check_page_limit!(control, page);
                        move_paragraph_lines(
                            document,
                            cascade,
                            body_id,
                            &LineMove {
                                block_id,
                                flex_line: flex_line_paragraph,
                                line,
                                shift,
                            },
                            &mut self.subtree_shifts,
                            page_before_candidate,
                        );
                        if reaches_line(text_lines.get(node_id), line) {
                            height += shift;
                        }
                        self.deferred_shifts
                            .line_splits
                            .push((block_id, line, shift));
                    }
                }
                LineBreakFix::MoveBlock => {
                    // The move is measured from where the block is now.
                    let block_effective_y = block_effective_y(
                        document,
                        block_id,
                        raw_y,
                        effective_y,
                        root_flow_offset,
                        self.flow_shift,
                    );
                    let target_page = self.current_page.saturating_add(1);
                    check_page_limit!(control, target_page);
                    let delta = move_line_block(
                        document,
                        cascade,
                        body_id,
                        block_id,
                        nested,
                        block_effective_y,
                        page_origin(target_page),
                        &mut self.subtree_shifts,
                        page_before_candidate,
                    );
                    self.flow_shift += delta;
                    effective_y += delta;
                }
                LineBreakFix::Keep => {}
            }
        }
        place_text_candidate(
            document,
            text_lines,
            node_id,
            ifc_root,
            text_ifc_root,
            effective_y,
        );
        let named_page_change = text_changes_page_name(
            name_context_seen,
            name_participates_in_flow,
            height,
            &candidate_page_name,
            comparison_page_name,
        );
        let pending_break_applies = self.pending_break.applies_to(document, node_id);
        let consumes_pending_break = pending_break_applies && name_participates_in_flow;
        if self.saw_child {
            if consumes_pending_break || named_page_change {
                let natural_page = page_index_in_flow(effective_y).unwrap_or(self.current_page);
                let target_page = forced_target_page(self.current_page, natural_page);
                check_page_limit!(control, target_page);
                let target_y = page_origin(target_page);
                // The candidate was materialized to `effective_y` above,
                // so this is the additional movement for this boundary.
                // The same amount advances `flow_shift`, which also moves
                // later candidates whose ancestor is not moved here.
                let delta = target_y - effective_y;
                if delta.is_finite() {
                    shift_text_candidate(
                        document,
                        text_lines,
                        node_id,
                        ifc_root.map(|(root, _)| root),
                        text_ifc_root,
                        delta,
                    );
                    self.flow_shift += delta;
                    effective_y += delta;
                }
                self.current_page = target_page;
                self.page_names.current = candidate_page_name.clone();
            } else if let Some(natural_page) = page_index_in_flow(effective_y) {
                check_page_limit!(control, natural_page);
                self.current_page = self.current_page.max(natural_page);
            }
        } else {
            self.saw_child = true;
            self.current_page = 0;
            if name_participates_in_flow {
                self.page_names.current = candidate_page_name.clone();
            }
        }
        if text_sets_page_name(
            height,
            name_participates_in_flow,
            named_page_change,
            consumes_pending_break,
            page_type_leaves_local_flow(
                keep_local_page_type,
                self.current_page,
                page_before_candidate,
            ),
        ) {
            self.page_names.current = candidate_page_name;
        }
        check_page_limit!(control, self.current_page);
        if name_participates_in_flow && height > 0.0 {
            self.column_contexts
                .occupy_enclosing(node_id, self.current_page, document, cascade);
        }
        let end_page = page_origins.end_page(effective_y, height);
        let in_flow = !candidate.context.is_out_of_flow_descendant;
        self.record_pages(control, end_page, in_flow)?;
        if consumes_pending_break {
            self.pending_break.clear();
        }
        self.pending_break.note_after(
            document,
            cascade,
            node_id,
            candidate.context.deferred_named_break_after,
        );

        Ok(Flow::Next)
    }

    /// Records the pages the candidate placed on the current page occupies,
    /// through `end_page` when it runs on.
    fn record_pages(
        &mut self,
        control: &PageLayoutControl<'_>,
        end_page: Option<u32>,
        in_flow: bool,
    ) -> Result<(), LayoutError> {
        record_candidate_pages(
            &mut self.reach,
            &mut self.page_names,
            control,
            self.current_page,
            end_page,
            in_flow,
        )
    }

    /// Settles the shifts that end or begin at `candidate`, before it is
    /// placed: those it has reached, those of the subtrees and the moved
    /// float it is outside of, and the clearance below the moved floats of
    /// its formatting context.
    fn settle_shifts(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
    ) {
        let FlowEnv {
            cascade,
            body_id,
            page_origins,
            text_lines,
            ..
        } = *env;
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
