//! What pagination carries from one candidate of the flow to the next.

use super::candidates::FlowRoots;
use super::{
    AvoidedBreaks, BoxFit, BoxOverflow, BoxParentScope, BoxTree, CascadeResult, CheckedParagraphs,
    ColumnContexts, DeferredShifts, EnteredParagraphs, FloatShifts, FloatTransition, FloatValue,
    FootnoteAreas, HeaderRepeats, IfcBoxShifts, LastNamedBox, LayoutError, LineBreakFix,
    LineBreakMove, LineMove, MarginBreak, MarginTruncation, PageBox, PageCandidate,
    PageContentInsets, PageLayoutControl, PageNames, PageOrigins, PageReach, ParagraphLineFit,
    PendingBreak, PositionValue, PrecedingFlexItems, SiblingMargins, SubtreeShifts, TextLineIndex,
    TrailingFlexChildren, TransitionLanding, TransitionMove, accumulated_relative_offset_y,
    block_effective_y, block_fits_page_step, box_changes_page_name, box_occupies_column,
    box_sets_page_name, break_margins, closing_edges_line_move, closing_edges_move,
    collapses_through, collapses_with_previous_block, content_overflow_end, current_abs_y,
    first_box_ancestor_at_start, first_line_judged_height, first_owned_line_height,
    fits_decorated_paper, flex_line_paragraph, follows_flow_content, forced_line_breaks,
    forced_line_moves, forced_target_page, grow_ancestors_for_shift,
    has_explicit_positioned_block_ancestor, has_fixed_height, has_footnote_ancestor,
    in_row_flex_container, is_floating_box_for_pagination, lead_below_ancestor, leading_edges_move,
    line_block_parent, lines_between_blocks, lines_can_shift_between_blocks,
    lines_can_shift_inside, make_room_for_table_header, margin_box_starts_page, materialize_y,
    move_box_by_truncated_margin, move_box_for_transition, move_flex_line_siblings,
    move_line_block, move_paragraph_lines, page_break_is_forced, page_type_leaves_local_flow,
    paragraph_line_bounds, participates_in_margin_adjacency, place_first_table_header,
    place_table_footer, place_text_candidate, positioned_text_start_y, reaches_line,
    record_candidate_pages, reset_inline_x_for_page_transition, run_tail_move,
    shift_text_candidate, straddling_line, text_changes_page_name, text_sets_page_name,
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
    pub(super) content_width: f32,
    pub(super) avoided_breaks: &'a AvoidedBreaks,
    pub(super) insets: PageContentInsets,
    pub(super) page_box: PageBox,
}

/// Why placing a candidate ends pagination instead of going on with the next
/// one.
#[derive(Debug)]
pub(super) enum Halt {
    /// The page limit truncates pagination: the candidates placed so far are
    /// all there is.
    PageLimit,
    /// Layout failed.
    Failed(LayoutError),
}

impl From<LayoutError> for Halt {
    fn from(error: LayoutError) -> Self {
        Self::Failed(error)
    }
}

impl Halt {
    /// What pagination returns once it stopped here: nothing for the page
    /// limit, which only truncates it, and the error otherwise.
    pub(super) fn finish(self) -> Result<(), LayoutError> {
        match self {
            Self::PageLimit => Ok(()),
            Self::Failed(error) => Err(error),
        }
    }
}

/// Lets placement reach `page_index`, or halts it there when `page_index` is
/// past the page limit of a control that truncates at it.
fn reach_page(control: &PageLayoutControl<'_>, page_index: u32) -> Result<(), Halt> {
    if control.within_page_limit(page_index)? {
        Ok(())
    } else {
        Err(Halt::PageLimit)
    }
}

/// The relative offsets that paint `candidate` away from its place in the flow,
/// or `None` for a box outside the flow, which has no place there.
fn flow_relative_offset_y(
    document: &LayoutSession<'_>,
    env: &FlowEnv<'_>,
    candidate: &PageCandidate,
    entry: &CandidateEntry,
) -> Option<f32> {
    let node_id = candidate.node_id;
    let in_flow = !entry.uses_positioned_coordinates
        && !candidate.context.is_out_of_flow_descendant()
        && !matches!(
            document.nodes[node_id].position(),
            PositionValue::Absolute | PositionValue::Fixed
        );
    in_flow.then(|| accumulated_relative_offset_y(document, env.cascade, env.body_id, node_id))
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

/// Where in-flow text sits while its lines are kept together at the page
/// edges.
pub(super) struct TextPosition {
    /// Where the text starts in the flow.
    pub(super) effective_y: f32,
    /// How tall the text is.
    pub(super) height: f32,
}

/// What a box meets at the start of its placement: the break it starts
/// after or on, and the margins around it.
pub(super) struct BoxBreak {
    /// The box's block-start margin as a page break sees it.
    pub(super) margin_top: f32,
    /// How far relative positioning offsets the box and its ancestors.
    pub(super) relative_offset: f32,
    /// How the box would cross the page edge.
    pub(super) overflow: BoxOverflow,
    /// The page the box fits: the current page, or the later page the flow
    /// has reached when the box before it ran over a page edge.
    pub(super) fit_page: u32,
    /// Whether the box has a forced break before it.
    pub(super) forced_before: bool,
    /// Whether a break pending after the previous box, a forced break before
    /// the box that is not at a page edge, or a change of the page type opens
    /// a page for the box.
    pub(super) forced_transition: bool,
    /// Whether the margins of the box collapse through it.
    pub(super) margins_collapse_through: bool,
    /// Whether the margins of the box collapse with those of the previous
    /// block.
    pub(super) collapses_with_previous_block: bool,
    /// Whether a break pending after the previous box applies to the box.
    pub(super) pending_break_applies: bool,
}

impl BoxBreak {
    /// Whether the box starts a page of its own: a forced break, a change of
    /// the page type or a break inside the box that it avoids opens the page.
    pub(super) fn is_page_transition(&self) -> bool {
        self.forced_transition || self.overflow.moves_box()
    }
}

/// Where a box is placed while the page it starts on is settled.
pub(super) struct BoxPosition {
    /// Where the box starts in the flow.
    pub(super) effective_y: f32,
    /// Set when a forced break starts this box's page, which makes the box the
    /// owner of the margins after the break.
    pub(super) starts_after_forced_break: bool,
}

/// Why the page of in-flow text was settled as it was.
pub(super) struct TextPageStep {
    /// Whether the text names another page type than the flow it follows, so
    /// that it opens a page.
    pub(super) named_page_change: bool,
    /// Whether a break pending after the previous box applies to the text.
    pub(super) consumes_pending_break: bool,
}

/// The blocks whose lines hold the text of a candidate: what keeping those
/// lines together can move.
pub(super) struct LineBlocks {
    /// The inline layout root of the candidate's paragraph, unless the
    /// candidate is that root itself.
    pub(super) paragraph_root: Option<usize>,
    /// The block the paragraph checks move as a unit, with whether it is
    /// nested in plain block flow; see [`line_block_parent`].
    pub(super) block: Option<(usize, bool)>,
    /// The paragraph and its flex container, for a paragraph that is a flex
    /// item alone on its line; see [`flex_line_paragraph`].
    pub(super) flex_line: Option<(usize, usize)>,
}

impl LineBlocks {
    /// Looks the blocks of `candidate` up. The flex line is looked for only
    /// when no line block was found.
    pub(super) fn find(
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        uses_positioned_coordinates: bool,
        checked_paragraphs: &CheckedParagraphs,
    ) -> Self {
        // The block of text in an IFC paragraph is its root, including nested
        // inline elements. A block inside an inline element of an inline
        // engine paragraph is laid out from the paragraph's root, as a direct
        // child is.
        let paragraph_root = candidate
            .ifc_root
            .map(|(root, _)| root)
            .filter(|&root| document.is_source_node(root) && root != candidate.node_id);
        let block = line_block_parent(
            document,
            env.cascade,
            &env.roots,
            candidate,
            paragraph_root,
            checked_paragraphs,
            uses_positioned_coordinates,
        );
        let flex_line = flex_line_paragraph(
            document,
            env.cascade,
            env.body_id,
            candidate,
            paragraph_root.filter(|_| block.is_none()),
            uses_positioned_coordinates,
        );
        Self {
            paragraph_root,
            block,
            flex_line,
        }
    }
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
        if let Some(group) = self.table_headers.owner(node_id) {
            // The footer closes the table below the rows placed before it,
            // and what follows the table follows the footer.
            if self
                .table_headers
                .headers
                .get(&group)
                .is_some_and(|footer| footer.footer && footer.visit_shift.is_none())
            {
                self.flow_shift += place_table_footer(
                    document,
                    &mut self.table_headers,
                    group,
                    self.flow_shift,
                    env.page_origins,
                );
            }
            return Ok(None);
        }
        let name_participates_in_flow = candidate.context.page_name_in_flow();
        if name_participates_in_flow
            && candidate.context.is_flex_item()
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
        let uses_positioned_coordinates = candidate.context.is_out_of_flow_descendant()
            && has_explicit_positioned_block_ancestor(document, env.cascade, node_id);
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
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            control,
            page_origins,
            text_lines,
            ..
        } = *env;
        let CandidateEntry {
            keep_local_page_type,
            page_before_candidate,
            ifc_root,
            participates_in_margin_adjacency,
            uses_positioned_coordinates,
            ..
        } = *entry;
        let node_id = candidate.node_id;
        let name_participates_in_flow = candidate.context.page_name_in_flow();
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
        if candidate.context.is_float_descendant()
            && has_footnote_ancestor(cascade, node_id, document)
        {
            return Ok(());
        }
        let mut text = TextPosition {
            effective_y: if uses_positioned_coordinates {
                positioned_text_start_y(document, text_lines, node_id, text_ifc_root)
            } else {
                candidate.raw_y + self.flow_shift
            },
            height: candidate.height,
        };
        let blocks = LineBlocks::find(
            document,
            env,
            candidate,
            uses_positioned_coordinates,
            &self.checked_paragraphs,
        );
        self.push_first_line_block(document, env, candidate, entry, &blocks, &mut text)?;
        self.keep_lines_together(document, env, candidate, entry, &blocks, &mut text)?;
        let first_line_shift =
            self.keep_lines_after_block(document, env, candidate, entry, &blocks, &mut text)?;
        // A root that this text moves is placed from where the text's first
        // line was; the lines moved above it are already in the line shifts.
        let ifc_root = ifc_root.map(|(root, offset)| (root, offset + first_line_shift));
        place_text_candidate(
            document,
            text_lines,
            node_id,
            ifc_root,
            text_ifc_root,
            text.effective_y,
        );
        let step = self.step_text_page(document, env, candidate, entry, &mut text)?;
        if text_sets_page_name(
            text.height,
            name_participates_in_flow,
            step.named_page_change,
            step.consumes_pending_break,
            page_type_leaves_local_flow(
                keep_local_page_type,
                self.current_page,
                page_before_candidate,
            ),
        ) {
            self.page_names.current = candidate.page_name.clone();
        }
        reach_page(control, self.current_page)?;
        if name_participates_in_flow && text.height > 0.0 {
            self.column_contexts
                .occupy_enclosing(node_id, self.current_page, document, cascade);
        }
        let end_page = page_origins.end_page(text.effective_y, text.height);
        let in_flow = !candidate.context.is_out_of_flow_descendant();
        self.record_pages(control, end_page, in_flow)?;
        if step.consumes_pending_break {
            self.pending_break.clear();
        }
        self.pending_break.note_after(
            document,
            cascade,
            node_id,
            candidate.context.deferred_named_break_after(),
        );

        Ok(())
    }

    /// Places the box `candidate` after its start: keeps a footnote in the
    /// page-local area of its anchor, or begins the box, starts it on its page,
    /// records it among its siblings, makes room for the header of the table
    /// it holds, names the page it opens, and finishes with the pages it
    /// occupies.
    pub(super) fn place_box(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            page_origins,
            ..
        } = *env;
        let CandidateEntry {
            keep_local_page_type,
            page_before_candidate,
            ..
        } = *entry;
        let node_id = candidate.node_id;

        if matches!(cascade.computed[node_id].float, FloatValue::Footnote) {
            return self.place_footnote(document, env, candidate);
        }
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        let (page_break, mut position) = self.begin_box(document, env, candidate, entry);
        let page_transition =
            self.start_box_page(document, env, candidate, entry, &page_break, &mut position)?;
        let BoxPosition {
            mut effective_y,
            starts_after_forced_break,
        } = position;
        self.record_box_among_siblings(
            document,
            env,
            candidate,
            entry,
            &page_break,
            starts_after_forced_break,
        );

        let header_delta = make_room_for_table_header(
            document,
            &mut self.table_headers,
            node_id,
            effective_y,
            height,
            self.current_page,
            page_origins,
        );
        effective_y += header_delta;
        self.flow_shift += header_delta;

        if box_sets_page_name(
            &candidate.context,
            page_transition,
            page_type_leaves_local_flow(
                keep_local_page_type,
                self.current_page,
                page_before_candidate,
            ),
        ) {
            self.page_names.current = candidate.page_name.clone();
            if candidate.context.is_direct_body_element() || candidate.named_flex_context.is_some()
            {
                self.last_named = LastNamedBox::new(document, candidate, node_id, raw_y, height);
            }
        }
        self.finish_box(document, env, candidate, effective_y)
    }

    /// Finishes the placement of the box `candidate` at `effective_y` on the
    /// page it starts on: records the pages it runs through, the columns of
    /// its page type it occupies, the flex item it is among the items of its
    /// named flow, and the break pending after it.
    pub(super) fn finish_box(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        effective_y: f32,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            control,
            page_origins,
            insets,
            page_box,
            ..
        } = *env;
        let node_id = candidate.node_id;
        let height = candidate.height;
        reach_page(control, self.current_page)?;
        // A fixed-height direct block that fits on the physical paper is
        // monolithic in this minimal paginator.
        let monolithic_paper_fit = || {
            candidate.context.is_direct_body_element()
                && fits_decorated_paper(
                    document,
                    node_id,
                    insets,
                    page_origins.origin(self.current_page),
                    page_box.height,
                    effective_y,
                    height,
                )
        };
        let end_page = page_origins
            .end_page(effective_y, height)
            .filter(|_| !monolithic_paper_fit());
        let in_flow = !candidate.context.is_out_of_flow_descendant();
        self.record_pages(control, end_page, in_flow)?;

        if box_occupies_column(
            &candidate.context,
            candidate.context.page_name_in_flow(),
            height,
        ) {
            self.column_contexts
                .occupy_enclosing(node_id, self.current_page, document, cascade);
        }
        if candidate.context.is_flex_item()
            && let Some(context) = candidate.named_flex_context
        {
            self.preceding_flex_items
                .record(context, height, self.current_page);
        }
        self.pending_break.note_after(
            document,
            cascade,
            node_id,
            candidate.context.deferred_named_break_after(),
        );

        Ok(())
    }

    /// Places the footnote box `candidate`. `float: footnote` is a page-local
    /// out-of-flow placement: it keeps the anchor's page, stacks multiple notes
    /// from the bottom, and removes the note's ordinary-flow footprint so
    /// following content can continue.
    pub(super) fn place_footnote(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
    ) -> Result<(), Halt> {
        let FlowEnv {
            control,
            page_origins,
            ..
        } = *env;
        let placement = self.footnote_areas.candidate(
            candidate.raw_y,
            candidate.height,
            self.flow_shift,
            self.current_page,
            page_origins,
        );
        reach_page(control, placement.page)?;
        self.footnote_areas
            .place(document, candidate.node_id, placement, page_origins);
        self.flow_shift -= placement.height;
        self.reach.include(placement.page, false);
        self.saw_child = true;
        Ok(())
    }

    /// Records the box `candidate` among the siblings whose margins adjoin
    /// its own: passes a forced break on to the margin after the box unless
    /// the box starts after one, and remembers where the box ends for the
    /// sibling that follows it.
    pub(super) fn record_box_among_siblings(
        &mut self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        page_break: &BoxBreak,
        starts_after_forced_break: bool,
    ) {
        let node_id = candidate.node_id;
        // Parallel flows do not interrupt adjoining normal-flow siblings.
        if entry.participates_in_margin_adjacency
            && let Some(parent_id) =
                BoxTree::for_layout(document).parent(node_id, BoxParentScope::Document)
        {
            self.sibling_margins.break_taken_by_float = None;
            if !starts_after_forced_break {
                self.sibling_margins.pass_on_forced_break(
                    document,
                    env.cascade,
                    parent_id,
                    node_id,
                    page_break.collapses_with_previous_block && page_break.margins_collapse_through,
                );
            }
            self.sibling_margins.record_bottom(
                parent_id,
                node_id,
                page_break.collapses_with_previous_block,
                candidate.raw_y + candidate.height,
            );
        }
    }

    /// Starts the box `candidate` on a page: the first box of the flow starts
    /// on the first page, and a later one opens the page its transition asks
    /// for, or starts on the page its position falls on. Returns whether the
    /// box met a page transition.
    pub(super) fn start_box_page(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        page_break: &BoxBreak,
        position: &mut BoxPosition,
    ) -> Result<bool, Halt> {
        let node_id = candidate.node_id;
        let page_transition = self.saw_child && page_break.is_page_transition();
        if self.saw_child {
            // A break-after on the preceding box and a break-before (or named
            // page transition) on this box describe the same boundary, not two
            // blank pages.
            if page_transition {
                self.open_page_for_box(document, env, candidate, entry, page_break, position)?;
            } else if position.effective_y.is_finite() && position.effective_y >= 0.0 {
                let relative_offset =
                    flow_relative_offset_y(document, env, candidate, entry).unwrap_or(0.0);
                self.start_box_on_natural_page(
                    document,
                    env,
                    candidate,
                    page_break,
                    (position, relative_offset),
                )?;
            }
            reset_inline_x_for_page_transition(
                document,
                node_id,
                page_transition,
                candidate.context.is_direct_body_element(),
            );
        } else {
            // A forced break before the first class-A box does not manufacture
            // a leading blank page. The box starts on the page its top falls
            // on, which is the second when the schedule opens with a short
            // page that ends where the flow begins, as the one a relayout
            // after delivered pages starts with: a forced break after the box
            // then opens the third page, not the one the box is on. A
            // relative offset only paints the box elsewhere, so the page is
            // found from its position in the flow. A box outside the flow
            // does not open the flow on its page.
            self.saw_child = true;
            let flow_y = flow_relative_offset_y(document, env, candidate, entry)
                .map(|relative_offset| position.effective_y - relative_offset);
            self.current_page = match flow_y {
                Some(flow_y) if flow_y.is_finite() && flow_y >= 0.0 => {
                    env.page_origins.start_page(flow_y, candidate.height)
                }
                _ => 0,
            };
        }
        Ok(page_transition)
    }

    /// Begins the placement of the box `candidate`: places the first header of
    /// the table it holds, fixes the box at its position in the flow, and works
    /// out what it meets there: the break it starts after or on, the page type
    /// it names, and the margins around it.
    pub(super) fn begin_box(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
    ) -> (BoxBreak, BoxPosition) {
        let FlowEnv {
            cascade,
            body_id,
            page_origins,
            content_width,
            avoided_breaks,
            ..
        } = *env;
        let CandidateEntry {
            name_context_seen,
            ref comparison_page_name,
            participates_in_margin_adjacency,
            uses_positioned_coordinates,
            ..
        } = *entry;
        let node_id = candidate.node_id;
        let computed = &cascade.computed[node_id];
        // Copy layout data before any break adjustment so the immutable node
        // borrow does not overlap the in-place location update below.
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        place_first_table_header(
            document,
            &mut self.table_headers,
            node_id,
            self.flow_shift,
            page_origins,
        );
        let effective_y = if uses_positioned_coordinates {
            current_abs_y(document, node_id)
        } else {
            raw_y + self.flow_shift
        };
        materialize_y(document, node_id, effective_y);
        let forced_before = page_break_is_forced(document, node_id, computed.break_before);
        // A block box with no block size and no in-flow content lets its
        // block-start and block-end margins collapse through it.
        let margins_collapse_through = collapses_through(document, cascade, node_id, height);
        let (margin_top, trailing_margin) =
            break_margins(document, cascade, node_id, content_width);
        let page_start = page_origins.origin(self.current_page);
        // Whether the box fits is decided at its normal-flow position,
        // before any relative offset.
        let relative_offset = accumulated_relative_offset_y(document, cascade, body_id, node_id);
        // Content before the box that runs over a page edge leaves the page the
        // box before it started on behind: the box fits the page the flow has
        // reached.
        let fit_page = page_origins
            .page_index_in_flow(effective_y - relative_offset)
            .filter(|&page| page > self.current_page && self.reach.flow_max >= page)
            .unwrap_or(self.current_page);
        let fit_page_start = page_origins.origin(fit_page);
        let fit = BoxFit {
            effective_y,
            flow_y: effective_y - relative_offset,
            margin_top,
            trailing_margin,
            page_start: fit_page_start,
            page_end: fit_page_start + page_origins.step_at(fit_page),
            next_page_step: page_origins.step_at(fit_page.saturating_add(1)),
        };
        let mut overflow = avoided_breaks.measure(
            document,
            cascade,
            candidate,
            &fit,
            &self.table_headers,
            &mut self.trailing_flex_children,
        );
        overflow.page |= closing_edges_move(candidate, &fit)
            || leading_edges_move(candidate, &fit)
            || run_tail_move(candidate, &fit);
        // A named page requested by a class-A box starts a new page when it
        // differs from the current named page. `auto` leaves the current page
        // type in place; it does not manufacture a second break boundary.
        // Any non-empty class-A box with a different page name starts a new
        // page.  `None` is the anonymous/unnamed page type, so an anonymous
        // box after a named one is a transition too.  Named boxes that start
        // at the coordinate of a preceding zero-height named box join its
        // transition: no block-size has been consumed on that page yet, so a
        // break there would only leave the zero-height boxes alone on a page.
        // This matters for chains of empty named boxes with overflowing text,
        // including the first non-empty box that follows the chain.
        let named_page_change = box_changes_page_name(
            name_context_seen,
            &candidate.page_name,
            comparison_page_name,
            self.last_named.joins_transition(candidate),
        );
        // A forced break at the start of a page already opened by natural
        // overflow describes the same boundary and must not create a blank
        // page. The forced break sits before the box's margin box, so the
        // test uses its block-start margin edge.
        let forced_break_at_page_start = forced_before
            && margin_box_starts_page(
                page_origins,
                effective_y - margin_top.max(0.0),
                self.current_page,
                page_start,
            );

        let collapses_with_previous_block =
            collapses_with_previous_block(document, candidate, participates_in_margin_adjacency);
        let pending_break_applies = self.pending_break.applies_to(document, node_id);
        // CSS Fragmentation 3 §5.2: margins adjoining an unforced break are
        // truncated, while a forced break keeps the block-start margin of the
        // box after it.
        let forced_transition = pending_break_applies
            || (forced_before && !forced_break_at_page_start)
            || named_page_change;
        let page_break = BoxBreak {
            margin_top,
            relative_offset,
            overflow,
            fit_page,
            forced_before,
            forced_transition,
            margins_collapse_through,
            collapses_with_previous_block,
            pending_break_applies,
        };
        let position = BoxPosition {
            effective_y,
            starts_after_forced_break: false,
        };
        (page_break, position)
    }

    /// Opens the page a box starts on when it meets a page transition: a
    /// forced break, a change of the page type it names, or a break inside it
    /// that it avoids. The box moves to the top of that page, and the flow
    /// after it, its floats and the siblings of a wrapped flex line move with
    /// it.
    pub(super) fn open_page_for_box(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        page_break: &BoxBreak,
        position: &mut BoxPosition,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            ..
        } = *env;
        let BoxBreak {
            margin_top,
            relative_offset,
            ref overflow,
            fit_page,
            forced_before,
            forced_transition,
            margins_collapse_through,
            pending_break_applies,
            ..
        } = *page_break;
        let node_id = candidate.node_id;
        // Content kept on its page as monolithic (see `monolithic_paper_fit`)
        // has not opened a later page.
        let natural_page = page_origins.transition_natural_page(
            position.effective_y,
            margin_top,
            self.current_page,
            self.reach.flow_max,
            forced_transition,
        );
        let mut target_page = forced_target_page(self.current_page, natural_page);
        if !forced_transition {
            // A box that moves because it would cross a page edge goes to the
            // page after the one it lies on.
            target_page = target_page.max(fit_page.saturating_add(1));
        }
        reach_page(control, target_page)?;
        // A negative block-start margin can pull a forced-break box
        // back into the preceding page.  Keep the break boundary for
        // following siblings (and for page count), but materialize
        // this box at the underflowed coordinate.  This is the
        // `underflow-from-next-page` case from CSS Break.
        let landing = TransitionLanding::new(
            page_origins.origin(target_page),
            position.effective_y,
            margin_top,
            relative_offset,
            forced_transition,
            overflow,
        );
        let row_flex_break_before =
            forced_before && in_row_flex_container(document, cascade, node_id);
        if landing.is_finite() {
            let moved_in_bounded_ancestor = move_box_for_transition(
                document,
                cascade,
                body_id,
                candidate,
                &TransitionMove {
                    node_delta: landing.node_delta,
                    shift_delta: landing.shift_delta,
                    page_overflow: overflow.page,
                    forced_transition,
                },
                &mut self.subtree_shifts,
                entry.page_before_candidate,
            );
            // A forced break on one wrapped row-flex item belongs to
            // its whole flex line. Move same-line siblings together;
            // other lines keep their existing flow coordinates.
            if row_flex_break_before && let Some(parent_id) = document.parent_of(node_id) {
                move_flex_line_siblings(
                    document,
                    parent_id,
                    node_id,
                    candidate.raw_y,
                    landing.shift_delta,
                );
            }
            // A kept margin moves the box's descendants and its
            // later siblings along with the box.
            self.flow_shift += landing.node_delta;
            if let Some(correction) = landing.underflow_correction() {
                self.deferred_shifts.underflow = Some((node_id, correction));
            }
            self.float_shifts.settle_transition(
                document,
                cascade,
                body_id,
                candidate,
                &FloatTransition {
                    page_overflow: overflow.page,
                    forced_transition,
                    delta: landing.node_delta,
                    target_y: landing.target_y,
                    relative_offset,
                    flow_page: entry.page_before_candidate,
                    height: candidate.height,
                    margin_top,
                    moved_in_bounded_ancestor,
                },
                &mut self.subtree_shifts,
            );
            position.effective_y += landing.node_delta;
        }
        self.current_page = target_page;
        if forced_transition {
            self.sibling_margins.forced_break_box = margins_collapse_through.then_some(node_id);
            position.starts_after_forced_break = true;
        }
        if pending_break_applies {
            // A float that takes the break moves to the new page in the place
            // of the in-flow box that follows it.
            if is_floating_box_for_pagination(document, cascade, node_id) {
                self.sibling_margins.break_taken_by_float = self.pending_break.source;
            }
            self.pending_break.clear();
        }
        Ok(())
    }

    /// Starts a box that meets no page transition on the page its position
    /// falls on. A forced break that coincides with a page edge also starts
    /// that page, and an unforced break that falls inside the collapsed
    /// margin between the box and its preceding sibling truncates that
    /// margin.
    pub(super) fn start_box_on_natural_page(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        page_break: &BoxBreak,
        (position, relative_offset): (&mut BoxPosition, f32),
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            ..
        } = *env;
        let BoxBreak {
            margin_top,
            forced_before,
            margins_collapse_through,
            collapses_with_previous_block,
            ..
        } = *page_break;
        let node_id = candidate.node_id;
        // A relative offset only paints the box elsewhere: the page it starts
        // on, and the page edge that cuts the margin above it, are found from
        // its position in the flow.
        let flow_y = position.effective_y - relative_offset;
        let mut natural_page = page_origins.start_page(flow_y, candidate.height);
        // A forced break that coincides with a page edge also starts
        // that page.
        if forced_before {
            self.sibling_margins.forced_break_box = margins_collapse_through.then_some(node_id);
            position.starts_after_forced_break = true;
        }
        // An unforced break that falls inside the collapsed margin
        // between this box and its preceding sibling truncates that
        // margin (CSS Fragmentation 3 §5.2): the box starts at the
        // top of the page instead of keeping the part of the margin
        // that crossed the page edge.
        let truncation = self.sibling_margins.truncation_at_break(
            document,
            cascade,
            page_origins,
            &MarginBreak {
                node_id,
                raw_y: candidate.raw_y,
                effective_y: flow_y,
                natural_page,
                forced_before,
                margin_top,
                collapses_with_previous_block,
            },
        );
        if let Some(MarginTruncation { delta, underflow }) = truncation {
            self.sibling_margins.moved_boxes |= delta != 0.0;
            move_box_by_truncated_margin(document, cascade, node_id, delta, body_id);
            self.flow_shift += delta;
            position.effective_y += delta;
            if let Some(underflow) = underflow {
                // Descendants follow the negative margin; later
                // siblings still follow the forced break boundary.
                self.deferred_shifts.underflow = Some((node_id, underflow));
            } else {
                natural_page = page_origins.page_index_for_y(flow_y + delta);
            }
        }
        reach_page(control, natural_page)?;
        self.current_page = self.current_page.max(natural_page);
        Ok(())
    }

    /// Pushes the fixed-height block whose first line would straddle the end
    /// of the page `text` is on to the next page as a unit. This matches
    /// normal block fragmentation for monolithic content while leaving
    /// already-forced page transitions to the existing fragment logic. A
    /// block that fits a page is judged by its text as a whole, a taller one
    /// by its first line alone (see [`first_line_judged_height`]). A block
    /// nested in plain block flow is moved the same way; its ancestors then
    /// grow to hold it.
    pub(super) fn push_first_line_block(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        blocks: &LineBlocks,
        text: &mut TextPosition,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            text_lines,
            root_flow_offset,
            ..
        } = *env;
        if let Some((block_id, nested)) = blocks.block
            && self.checked_paragraphs.block_text.insert(block_id)
            && has_fixed_height(document, block_id)
        {
            let block_height = document.nodes[block_id].layout.size.height;
            let block_effective_y = block_effective_y(
                document,
                block_id,
                candidate.raw_y,
                text.effective_y,
                root_flow_offset,
                self.flow_shift,
            );
            let judged_height = first_line_judged_height(
                block_height,
                page_origins.step_at(self.current_page),
                block_effective_y,
                text.effective_y,
                text.height,
                || first_owned_line_height(document, candidate.node_id, text_lines),
            );
            if let Some(target_page) = judged_height.and_then(|judged_height| {
                page_origins.first_line_push_page(
                    text.effective_y,
                    judged_height,
                    block_effective_y,
                    self.current_page,
                )
            }) {
                reach_page(control, target_page)?;
                let target_y = page_origins.origin(target_page);
                let delta = move_line_block(
                    document,
                    cascade,
                    body_id,
                    block_id,
                    nested,
                    block_effective_y,
                    target_y,
                    page_origins.origin(self.current_page),
                    &mut self.subtree_shifts,
                    entry.page_before_candidate,
                );
                self.flow_shift += delta;
                text.effective_y += delta;
            }
        }
        Ok(())
    }

    /// Keeps the lines of the text from breaking badly at the end of the
    /// page. `orphans` and `widows` constrain breaks between line boxes, not
    /// breaks between block-level children. A line that crosses the end of a
    /// page starts the next one, and a break that leaves too few widows moves
    /// up between earlier lines as far as the orphans allow; a fitting block
    /// is moved as a unit instead when its natural split would leave too few
    /// orphans. A block taller than a
    /// page is moved as a unit only when its break would leave too few
    /// orphans, content comes before it on a page it does not start, and its
    /// first line fits a page; otherwise it keeps its natural break. A
    /// paragraph that is a flex item alone on its line cannot move as a unit,
    /// but its later lines can move down inside it. The padding and border
    /// that close the boxes around the paragraph stay with its last lines:
    /// when its lines fit a page but the edges do not, the lines that the edge
    /// break asks for move down, on the page the paragraph starts on and on
    /// the one it moved to as a unit.
    pub(super) fn keep_lines_together(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        blocks: &LineBlocks,
        text: &mut TextPosition,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            text_lines,
            root_flow_offset,
            ..
        } = *env;
        let node_id = candidate.node_id;
        if let Some((block_id, nested)) = blocks
            .block
            .or(blocks.flex_line.map(|(root, _)| (root, true)))
            && self.checked_paragraphs.orphans_widows.insert(block_id)
            && let Some(line_metrics) = paragraph_line_bounds(document, node_id, text_lines)
        {
            // Lines move down inside the paragraph only when it is its own
            // inline layout root, has no earlier split pending, and can shift
            // its lines.
            let lines_can_move = blocks.paragraph_root == Some(block_id)
                && self.deferred_shifts.line_splits.is_empty()
                && lines_can_shift_inside(document, block_id);
            // The break that keeps the last lines with the padding and border
            // that close the boxes around the paragraph, on `page`.
            let closing_edges_break = |page: u32, effective_y: f32| {
                closing_edges_line_move(
                    &line_metrics,
                    candidate.context.lines_closing_edges(),
                    effective_y,
                    page,
                    cascade.computed[block_id].orphans.max(1) as usize,
                    cascade.computed[block_id].widows.max(1) as usize,
                    page_origins,
                )
            };
            // Forced breaks between the lines come first. The lines they
            // leave on either side are not repaired for orphans and widows.
            if lines_can_move {
                let moves = forced_line_moves(
                    &forced_line_breaks(document, block_id),
                    &line_metrics,
                    text.effective_y,
                    page_origins,
                );
                if !moves.is_empty() {
                    return self.move_lines(
                        document, env, candidate, entry, blocks, text, block_id, moves,
                    );
                }
                // The padding and border that close the blocks around the
                // paragraph stay with its last lines.
                if let Some(moved) = closing_edges_break(self.current_page, text.effective_y) {
                    return self.move_lines(
                        document,
                        env,
                        candidate,
                        entry,
                        blocks,
                        text,
                        block_id,
                        [moved],
                    );
                }
            }
            let block_height = document.nodes[block_id].layout.size.height;
            // The padding and border of the paragraph below its last line.
            // They follow the line only when the lines are all of the content
            // of the box: a block laid out after the last line, such as a
            // block-level `::after`, or a height taller than the lines, is
            // what they follow instead. Half a pixel absorbs the rounding of
            // the line boxes.
            let own_closing_edges = if blocks.paragraph_root == Some(block_id) {
                let layout = &document.nodes[block_id].layout;
                let edges = layout.padding.bottom + layout.border.bottom;
                let content = block_height - edges - layout.padding.top - layout.border.top;
                let lines = line_metrics[line_metrics.len() - 1].1 - line_metrics[0].0;
                if (content - lines).abs() <= 0.5 {
                    edges
                } else {
                    0.0
                }
            } else {
                0.0
            };
            let mut page = self.current_page;
            // A block moved whole to the next page is judged once more from
            // there, for the lines that break again on the pages after it.
            let mut moved_whole = false;
            loop {
                let page_start = page_origins.origin(page);
                let page_step = page_origins.step_at(page);
                let mut line_fit = ParagraphLineFit::measure(
                    &line_metrics,
                    text.effective_y,
                    page_start,
                    page_start + page_step,
                    cascade.computed[block_id].orphans,
                    cascade.computed[block_id].widows,
                    own_closing_edges,
                );
                // The block is measured from where it is now.
                let block_effective_y = block_effective_y(
                    document,
                    block_id,
                    candidate.raw_y,
                    text.effective_y,
                    root_flow_offset,
                    self.flow_shift,
                );
                // Lines that start on a later page than the boxes that hold
                // them leave those boxes' padding and border at the foot of
                // this page, unless the page breaks before the boxes. Only
                // the edges of the block may lie above the first line: a box
                // before it in the block stays on this page.
                let first_line_top = text.effective_y + line_metrics[0].0;
                let block_edges = document.nodes[block_id].layout.border.top
                    + document.nodes[block_id].layout.padding.top;
                if !line_fit.starts_on_page
                    && first_line_top >= page_start + page_step - 0.001
                    && first_line_top - (block_effective_y + block_edges) <= 0.5
                {
                    let holder_top = block_effective_y
                        - if nested {
                            let holder = first_box_ancestor_at_start(
                                document, cascade, block_id, body_id, page_start,
                            );
                            lead_below_ancestor(document, cascade, body_id, block_id, holder)
                        } else {
                            0.0
                        };
                    line_fit.top_edges_end_page = holder_top > page_start + 0.001
                        && holder_top < page_start + page_step - 0.001;
                }
                // A block taller than a page moves whole only to keep its
                // orphans, as it fragments on the next page anyway. That
                // takes content before it on a page it does not start, and a
                // first line that fits a page. Edges above the lines that end
                // this page move it whatever its height: the page breaks
                // before the block, not between its edges and its lines, even
                // when they run over a page.
                let can_move_block = !moved_whole
                    && blocks.block.is_some()
                    && (block_fits_page_step(block_height, page_step)
                        || line_fit.top_edges_end_page
                        || (block_effective_y > page_start + 0.001
                            && follows_flow_content(document, cascade, body_id, block_id)
                            && line_fit.gains_from_moving_tall_block(
                                text.effective_y - block_effective_y + line_metrics[0].1,
                                page_step,
                            )));
                let fix = line_fit.fix(
                    &line_metrics,
                    text.effective_y,
                    page,
                    page_origins,
                    can_move_block,
                    || lines_can_move,
                );
                match fix {
                    LineBreakFix::MoveLines(moves) => {
                        self.move_lines(
                            document, env, candidate, entry, blocks, text, block_id, moves,
                        )?;
                        break;
                    }
                    LineBreakFix::MoveBlock => {
                        page = page.saturating_add(1);
                        reach_page(control, page)?;
                        let delta = move_line_block(
                            document,
                            cascade,
                            body_id,
                            block_id,
                            nested,
                            block_effective_y,
                            page_origins.origin(page),
                            page_start,
                            &mut self.subtree_shifts,
                            entry.page_before_candidate,
                        );
                        self.flow_shift += delta;
                        text.effective_y += delta;
                        moved_whole = true;
                    }
                    LineBreakFix::Keep => {
                        // A paragraph moved whole to a page it fits there may
                        // leave the padding and border that close the boxes
                        // around it behind its lines, as on the page it left.
                        if moved_whole
                            && lines_can_move
                            && let Some(moved) = closing_edges_break(page, text.effective_y)
                        {
                            return self.move_lines(
                                document,
                                env,
                                candidate,
                                entry,
                                blocks,
                                text,
                                block_id,
                                [moved],
                            );
                        }
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Keeps the lines that come after a block child of a paragraph from
    /// straddling the end of the page. The paragraph's own checks judge all
    /// of its lines as one run, which cannot move when blocks lie among the
    /// lines; the run of lines between two blocks breaks like a block of its
    /// own instead. It moves whole to the next page when it fits a page and
    /// the break would leave too few orphans or widows; otherwise the line
    /// that crosses a page end starts the next page, and the lines that keep
    /// the widows move down with it. The rest of the
    /// paragraph follows the moved lines. Returns how far the moves took the
    /// run's first line.
    pub(super) fn keep_lines_after_block(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        blocks: &LineBlocks,
        text: &mut TextPosition,
    ) -> Result<f32, Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            text_lines,
            ..
        } = *env;
        let node_id = candidate.node_id;
        // Text placed by positioned insets, or inside a float or an
        // out-of-flow box, is not in the flow that the moved lines push down.
        let in_flow = !entry.uses_positioned_coordinates
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant();
        let Some(root) = blocks.paragraph_root.filter(|_| in_flow) else {
            return Ok(0.0);
        };
        if !self.deferred_shifts.line_splits.is_empty()
            || !lines_can_shift_between_blocks(document, env.cascade, root)
        {
            return Ok(0.0);
        }
        let Some(run) = lines_between_blocks(document, text_lines, node_id, root) else {
            return Ok(0.0);
        };
        let run_metrics = |document: &LayoutSession<'_>| {
            paragraph_line_bounds(document, node_id, text_lines)
                .and_then(|all| all.get(run.clone()).map(<[_]>::to_vec))
                .filter(|metrics| !metrics.is_empty())
        };
        // The first candidate on the run judges it for all of them.
        let first_visit = self
            .checked_paragraphs
            .lines_after_blocks
            .insert((root, run.start));
        let Some(mut metrics) = run_metrics(document).filter(|_| first_visit) else {
            return Ok(0.0);
        };
        let first_owned_line = text_lines
            .get(node_id)
            .and_then(|owned| owned.lines.first())
            .map_or(run.start, |line| line.line);
        let mut page = self
            .current_page
            .max(page_origins.page_index_for_y(text.effective_y + metrics[0].0 + 0.001));
        let mut moved = false;
        let mut first_line_shift = 0.0;
        loop {
            let page_start = page_origins.origin(page);
            let page_step = page_origins.step_at(page);
            let fit = ParagraphLineFit::measure(
                &metrics,
                text.effective_y,
                page_start,
                page_start + page_step,
                cascade.computed[root].orphans,
                cascade.computed[root].widows,
                0.0,
            );
            let run_top = text.effective_y + metrics[0].0;
            let run_height = metrics[metrics.len() - 1].1 - metrics[0].0;
            // A run taller than a page moves whole only to keep its orphans,
            // when content comes before it on this page.
            let can_move_block = !moved
                && (block_fits_page_step(run_height, page_step)
                    || (run_top > page_start + 0.001
                        && fit
                            .gains_from_moving_tall_block(metrics[0].1 - metrics[0].0, page_step)));
            let (moves, repeat): (Vec<LineBreakMove>, bool) = match fit.fix(
                &metrics,
                text.effective_y,
                page,
                page_origins,
                can_move_block,
                || true,
            ) {
                LineBreakFix::MoveLines(moves) => (moves, false),
                LineBreakFix::MoveBlock => {
                    page = page.saturating_add(1);
                    (vec![(0, page_origins.origin(page) - run_top, page)], true)
                }
                LineBreakFix::Keep => {
                    // The natural break honors the orphans and widows, but a
                    // line is not split by it: the one that straddles the
                    // page end starts the next page.
                    let Some(line) = straddling_line(
                        &metrics,
                        text.effective_y,
                        page_start,
                        page_start + page_step,
                    ) else {
                        break;
                    };
                    page = page.saturating_add(1);
                    let top = text.effective_y + metrics[line].0;
                    (vec![(line, page_origins.origin(page) - top, page)], true)
                }
            };
            for (line, shift, move_page) in moves {
                reach_page(control, move_page)?;
                let line = run.start + line;
                move_paragraph_lines(
                    document,
                    cascade,
                    body_id,
                    &LineMove {
                        block_id: root,
                        flex_line: None,
                        line,
                        shift,
                    },
                    &mut self.subtree_shifts,
                    entry.page_before_candidate,
                );
                if line <= first_owned_line {
                    text.effective_y += shift;
                    first_line_shift += shift;
                } else if reaches_line(text_lines.get(node_id), line) {
                    text.height += shift;
                }
                self.deferred_shifts
                    .line_splits
                    .push((root, line, shift, move_page));
            }
            if !repeat {
                break;
            }
            // Judged once more from the page the lines moved to, for the
            // lines that break again on the pages after it.
            moved = true;
            if let Some(next) = run_metrics(document) {
                metrics = next;
            }
        }
        Ok(first_line_shift)
    }

    /// Moves lines of the paragraph `block_id`, which holds the text
    /// `candidate`, down to the pages `moves` name: see [`LineBreakMove`].
    /// Each shift is also taken by the flow after the paragraph, and by the
    /// text of the paragraph that starts on a moved line.
    #[allow(clippy::too_many_arguments)]
    fn move_lines(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        blocks: &LineBlocks,
        text: &mut TextPosition,
        block_id: usize,
        moves: impl IntoIterator<Item = LineBreakMove>,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            text_lines,
            ..
        } = *env;
        for (line, shift, page) in moves {
            reach_page(control, page)?;
            move_paragraph_lines(
                document,
                cascade,
                body_id,
                &LineMove {
                    block_id,
                    flex_line: blocks.flex_line,
                    line,
                    shift,
                },
                &mut self.subtree_shifts,
                entry.page_before_candidate,
            );
            if reaches_line(text_lines.get(candidate.node_id), line) {
                text.height += shift;
            }
            self.deferred_shifts
                .line_splits
                .push((block_id, line, shift, page));
        }
        Ok(())
    }

    /// Settles the page the in-flow text `candidate` starts on, once its
    /// lines are kept together and it is placed at `text.effective_y`. The
    /// first candidate opens the first page. A
    /// pending break or a change of the page type the text names opens a
    /// page, to the top of which the text moves; any other text goes on from
    /// the page it falls on.
    pub(super) fn step_text_page(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        text: &mut TextPosition,
    ) -> Result<TextPageStep, Halt> {
        let FlowEnv {
            control,
            page_origins,
            text_lines,
            ..
        } = *env;
        let node_id = candidate.node_id;
        let name_participates_in_flow = candidate.context.page_name_in_flow();
        let text_ifc_root = candidate.ifc_root.map(|(root, _)| root);
        // A relative offset only paints the text elsewhere: the page it starts
        // on is the one its place in the flow falls on.
        let relative_offset =
            flow_relative_offset_y(document, env, candidate, entry).unwrap_or(0.0);
        // Text is not itself a class-A box, but non-empty text can be the
        // content that follows a nested named box.  Carry the containing
        // box's page type so that content after `page:b` inside a
        // `page:a` box resumes on an `a` page rather than becoming an
        // anonymous page.  Only direct body text consumes a pending
        // break-after; descendants must wait until their containing box
        // has finished.
        let named_page_change = text_changes_page_name(
            entry.name_context_seen,
            name_participates_in_flow,
            text.height,
            &candidate.page_name,
            &entry.comparison_page_name,
        );
        let pending_break_applies = self.pending_break.applies_to(document, node_id);
        let consumes_pending_break = pending_break_applies && name_participates_in_flow;
        if self.saw_child {
            if consumes_pending_break || named_page_change {
                let natural_page = page_origins
                    .page_index_in_flow(text.effective_y - relative_offset)
                    .unwrap_or(self.current_page);
                let target_page = forced_target_page(self.current_page, natural_page);
                reach_page(control, target_page)?;
                let target_y = page_origins.origin(target_page);
                // `place_text` already materialized the candidate at
                // `effective_y`, so this is the additional movement for this
                // boundary.
                // The same amount advances `flow_shift`, which also moves
                // later candidates whose ancestor is not moved here.
                let delta = target_y - text.effective_y;
                if delta.is_finite() {
                    shift_text_candidate(
                        document,
                        text_lines,
                        node_id,
                        entry.ifc_root.map(|(root, _)| root),
                        text_ifc_root,
                        delta,
                    );
                    self.flow_shift += delta;
                    text.effective_y += delta;
                }
                self.current_page = target_page;
                self.page_names.current = candidate.page_name.clone();
            } else if let Some(natural_page) =
                page_origins.page_index_in_flow(text.effective_y - relative_offset)
            {
                reach_page(control, natural_page)?;
                self.current_page = self.current_page.max(natural_page);
            }
        } else {
            self.saw_child = true;
            self.current_page = 0;
            if name_participates_in_flow {
                self.page_names.current = candidate.page_name.clone();
            }
        }
        Ok(TextPageStep {
            named_page_change,
            consumes_pending_break,
        })
    }

    /// Gives the boxes outside the paginated flow the pages to paint on. The
    /// candidates stand for the paginated flow only; boxes outside it, such
    /// as a nested absolutely positioned box or the overflow of a monolithic
    /// box, still need pages, and so do table edges that were grown over
    /// repeated headers. The pages past the last one the normal flow reached
    /// keep the page type the flow ended on.
    pub(super) fn reach_overflow_pages(
        &mut self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        content_height: f32,
    ) -> Result<(), LayoutError> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            root_flow_offset,
            ..
        } = *env;
        if !control.page_limit_reached.get() {
            // Pagination materialized the moved boxes in page coordinates,
            // which already include `root_flow_offset`; boxes it left alone
            // paint where their own coordinates put them.
            let end =
                content_overflow_end(document, cascade, body_id, root_flow_offset, content_height);
            if end.is_finite() && end > 0.0 {
                self.reach
                    .include_checked(control, page_origins.page_index_for_end(end), false)?;
            }
        }
        // Pages past the last one the normal flow reached hold only boxes
        // outside it, whatever page type was active when one of those boxes
        // first reached them.
        let names_end = control.capped_page_index(self.reach.max);
        self.page_names
            .continue_onto(self.reach.flow_max, names_end, true);
        Ok(())
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
        // What follows a line that moved to a later page in the flow is on
        // that page or after it, though the candidates placed so far are on
        // the one before. A float or a box out of the flow is placed by its
        // own position.
        let in_flow = !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant();
        if let Some(page) = self.deferred_shifts.apply_reached(
            document,
            text_lines,
            node_id,
            candidate.ifc_root,
            &mut self.flow_shift,
        ) && in_flow
        {
            self.current_page = self.current_page.max(page);
        }
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
            } else if !candidate.context.is_direct_body_element() {
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
