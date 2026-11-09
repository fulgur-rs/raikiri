//! What pagination carries from one candidate of the flow to the next.

use super::candidates::{EdgesBreak, FlowRoots};
use super::page_start::ChildPositions;
use super::{
    AvoidedBreaks, BoxFit, BoxOverflow, BoxParentScope, BoxTree, CascadeResult, CheckedParagraphs,
    ColumnContexts, DeferredShifts, EnteredParagraphs, FloatShifts, FloatTransition, FloatValue,
    FootnoteAreas, ForcedBreakHolder, HeaderRepeats, IfcBoxShifts, LastNamedBox, LayoutError,
    LineBreakFix, LineBreakMove, LineMove, LineSplit, MarginBreak, MarginTruncation, NodeKind,
    PageBox, PageCandidate, PageContentInsets, PageLayoutControl, PageNames, PageOrigins,
    PageReach, ParagraphLineFit, PendingBreak, PendingPrefix, PositionValue, PrecedingFlexItems,
    SiblingMargins, SubtreeShifts, TextLineIndex, TrailingFlexChildren, TransitionLanding,
    TransitionMove, accumulated_relative_offset_y, avoided_box_moves, block_effective_y,
    block_fits_page_step, box_avoids_page_break_inside, box_changes_page_name, box_occupies_column,
    box_sets_page_name, break_margins, closing_edges_line_move, closing_edges_move,
    collapsed_start_margin_gain, collapsed_start_margin_gain_in_flow, collapses_through,
    collapses_with_previous_block, content_overflow, current_abs_y, first_box_ancestor_at_start,
    first_line_judged_height, first_owned_line_height, fits_decorated_paper, flex_line_paragraph,
    floats_put_first_line_down, flow_raw_y, follows_flow_content, forced_line_breaks,
    forced_line_moves, forced_target_page, grow_ancestors_for_shift,
    has_explicit_positioned_block_ancestor, has_fixed_height, has_footnote_ancestor,
    is_floating_box_for_pagination, is_unfixed_absolute_box, lead_below_ancestor, lead_below_edges,
    leading_edges_move, leading_holder, line_block_parent, lines_between_blocks,
    lines_can_shift_above_blocks, lines_can_shift_between_blocks, lines_can_shift_inside,
    lines_move_grows_box, make_room_for_caption_header, make_room_for_table_header,
    margin_box_starts_page, materialize_y, move_box_by_truncated_margin, move_box_for_transition,
    move_flex_line_siblings, move_line_block, move_paragraph_lines, page_break_is_forced,
    page_type_leaves_local_flow, paragraph_line_bounds, participates_in_margin_adjacency,
    place_first_table_header, place_table_footer, place_text_candidate, positioned_line_moves,
    positioned_text_start_y, reaches_line, record_candidate_pages,
    reset_inline_x_for_page_transition, row_flex_container, run_ends_paragraph, run_tail_move,
    shift_paragraph_lines, shift_text_candidate, starts_on_page_and_runs_past_it, straddling_line,
    text_changes_page_name, text_sets_page_name,
};
use crate::layout::context::LayoutSession;
use std::collections::HashMap;

/// How far, in px, the margin that an earlier layout kept above a box may
/// differ from the one a relayout computes for it. A page start closer to the
/// top of its page than this is at the top, with no margin kept.
const CONTINUATION_MARGIN_TOLERANCE: f32 = 0.01;

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

/// The text of an absolutely positioned box whose lines are kept whole on a
/// page: see [`PaginationState::keep_positioned_lines`].
struct PositionedParagraph {
    /// The nearest absolute box that owns this internal flow.
    owner: usize,
    /// The first text of the paragraph that was placed.
    text: usize,
    /// Where the first line of that text starts, in the coordinates of the
    /// page candidates.
    first_raw_y: f32,
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
pub(super) fn reach_page(control: &PageLayoutControl<'_>, page_index: u32) -> Result<(), Halt> {
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
    /// How far moving the lines of its paragraph took the text's first line.
    /// The paragraph's root is placed from where that line was, since the line
    /// shifts of the paragraph already hold the move.
    pub(super) first_line_shift: f32,
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
    /// Whether a relayout starts the box on a page after a break that an
    /// earlier layout made forced: the margin above the box is kept, as at a
    /// forced break.
    pub(super) keeps_continuation_margin: bool,
    /// Whether an opening break preserves this box's used start margin.
    pub(super) keeps_margin_at_break: bool,
    /// The ancestor that moves to the next page with this box when it would
    /// cross the page edge, and how far below the top of that ancestor the
    /// box starts (see [`leading_holder`]). `None` when the box moves alone.
    pub(super) holder: Option<(usize, f32)>,
    /// The plain holder carried by this box's own new forced boundary.
    pub(super) forced_holder: Option<ForcedBreakHolder>,
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
    /// A content-height paragraph that ends an eligible column flex box.
    /// Its lines can move without invoking row-flex sibling movement.
    pub(super) column_paragraph: Option<usize>,
}

impl LineBlocks {
    /// Looks the blocks of `candidate` up. Flex paragraphs are considered
    /// only when no ordinary line block was found. Trailing identities are
    /// cached, and a column paragraph is considered only before its first
    /// orphans/widows check.
    pub(super) fn find(
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        uses_positioned_coordinates: bool,
        checked_paragraphs: &mut CheckedParagraphs,
        trailing_flex_children: &mut TrailingFlexChildren,
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
        let column_paragraph = paragraph_root.filter(|&root| {
            block.is_none()
                && flex_line.is_none()
                && !uses_positioned_coordinates
                && !candidate.context.is_float_descendant()
                && !candidate.context.is_out_of_flow_descendant()
                && !checked_paragraphs.orphans_widows.contains(&root)
                && trailing_flex_children.is_trailing(document, env.cascade, root)
                && *checked_paragraphs
                    .column_eligibility
                    .entry(root)
                    .or_insert_with(|| {
                        super::column_first_piece::line_paragraph(
                            document,
                            env.cascade,
                            env.body_id,
                            root,
                        )
                    })
        });
        Self {
            paragraph_root,
            block,
            flex_line,
            column_paragraph,
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
    float_gaps: super::external_float_gap::Index,
    current_candidate_rank: usize,
    next_candidate_rank: usize,
    pub(super) float_shifts: FloatShifts,
    pub(super) reach: PageReach,
    /// How far the edges pushed down under each fixed clip have moved the
    /// flow inside it, by the end of the clip (as bits) in the coordinates of
    /// the candidates: the clip stays where it is while they move.
    edge_pushes: HashMap<u32, f32>,
    /// Box growth used to paint closing edges across a page break, excluded
    /// from the height against which outer absolute children are positioned.
    edge_growth: HashMap<usize, f32>,
    pub(super) page_names: PageNames,
    pub(super) pending_break: PendingBreak,
    pub(super) sibling_margins: SiblingMargins,
    child_positions: ChildPositions,
    pub(super) last_named: LastNamedBox,
    /// Whether any box was placed yet.
    pub(super) saw_child: bool,
    pub(super) checked_paragraphs: CheckedParagraphs,
    /// The absolutely positioned boxes that hold text, by paragraph root, with
    /// what is needed to keep the lines of the paragraph whole on a page once
    /// the flow is placed, when they are.
    positioned_paragraphs: HashMap<usize, Option<PositionedParagraph>>,
    /// Opening page and used margin for boxes whose margin starts at its boundary.
    retained_margin_starts: HashMap<usize, (u32, f32)>,
    /// Alignment room removed above the text of a cell that spans pages.
    fragmented_cell_shifts: HashMap<usize, f32>,
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
            float_gaps: super::external_float_gap::Index::default(),
            current_candidate_rank: 0,
            next_candidate_rank: 0,
            float_shifts: FloatShifts::default(),
            reach: PageReach::default(),
            edge_pushes: HashMap::new(),
            edge_growth: HashMap::new(),
            page_names,
            pending_break: PendingBreak::default(),
            sibling_margins: SiblingMargins::default(),
            child_positions: ChildPositions::default(),
            last_named: LastNamedBox::default(),
            saw_child: false,
            checked_paragraphs: CheckedParagraphs::default(),
            positioned_paragraphs: HashMap::new(),
            retained_margin_starts: HashMap::new(),
            fragmented_cell_shifts: HashMap::new(),
            footnote_areas: FootnoteAreas::default(),
            column_contexts,
            preceding_flex_items: PrecedingFlexItems::default(),
            trailing_flex_children: TrailingFlexChildren::default(),
            entered_paragraphs: EnteredParagraphs::default(),
            table_headers,
        }
    }

    pub(super) fn prepare_float_gaps(
        &mut self,
        document: &mut LayoutSession<'_>,
        cascade: &CascadeResult,
        body_id: usize,
        candidates: &[PageCandidate],
    ) {
        self.float_gaps.prepare(document, cascade, body_id);
        self.float_gaps.assign_visits(document, candidates);
    }

    pub(super) fn rebase_fragmented_cell_text(&self, candidate: &mut PageCandidate) {
        if candidate.context.is_text()
            && let Some((root, offset)) = &mut candidate.ifc_root
            && let Some(shift) = self.fragmented_cell_shifts.get(root)
        {
            // Rebase both coordinates so placing the text leaves its root at
            // the same border-box top and does not shift following content.
            candidate.raw_y -= shift;
            *offset -= shift;
        }
    }

    fn queue_prefixes(&mut self, consumed: Vec<super::external_float_gap::Removal>, page: u32) {
        for removal in consumed {
            let Some(rank) = self.float_gaps.first_visit(removal.root) else {
                // Candidate-free text overflow changes only painted lines.
                continue;
            };
            self.deferred_shifts
                .pending_prefixes
                .entry(rank)
                .or_default()
                .push(PendingPrefix {
                    split: LineSplit {
                        root: removal.root,
                        first_moved: 0,
                        shift: -removal.removed,
                        growth: removal.growth,
                        page,
                    },
                    ancestor_growth: removal.ancestor_growth,
                });
        }
        self.deferred_shifts
            .activate_prefixes(self.current_candidate_rank, &mut self.subtree_shifts);
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
        self.current_candidate_rank = self.next_candidate_rank;
        self.next_candidate_rank += 1;
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
        let discarded_gap = text_ifc_root
            .and_then(|root| document.ifc_layout_node(root))
            .and_then(|node| node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
            .map_or(0.0, |lines| lines.discarded_float_gap);
        let mut text = TextPosition {
            effective_y: if uses_positioned_coordinates {
                positioned_text_start_y(document, text_lines, node_id, text_ifc_root)
            } else {
                candidate.raw_y + self.flow_shift
            },
            height: candidate.height,
            first_line_shift: 0.0,
        };
        self.push_oversized_edge_holder(document, env, candidate, entry, &mut text)?;
        let blocks = LineBlocks::find(
            document,
            env,
            candidate,
            uses_positioned_coordinates,
            &mut self.checked_paragraphs,
            &mut self.trailing_flex_children,
        );
        self.push_first_line_block(document, env, candidate, entry, &blocks, &mut text)?;
        self.keep_lines_together(document, env, candidate, entry, &blocks, &mut text)?;
        self.keep_lines_after_block(document, env, candidate, entry, &blocks, &mut text)?;
        // A root that this text moves is placed from where the text's first
        // line was; the lines moved above it are already in the line shifts.
        let ifc_root =
            ifc_root.map(|(root, offset)| (root, offset + text.first_line_shift - discarded_gap));
        self.note_positioned_paragraph(document, candidate);
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

    /// Move a single-item holder before its paragraph's oversized own edges.
    fn push_oversized_edge_holder(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        text: &mut TextPosition,
    ) -> Result<(), Halt> {
        if entry.uses_positioned_coordinates
            || candidate.context.is_out_of_flow_descendant()
            || document.context.pagination_on_transposed_axes
        {
            return Ok(());
        }
        let Some((paragraph, _)) = candidate.ifc_root else {
            return Ok(());
        };
        let layout = &document.nodes[paragraph].layout;
        if (layout.padding.top + layout.border.top <= 0.0
            && layout.padding.bottom + layout.border.bottom <= 0.0)
            || !self.checked_paragraphs.oversized_edges.insert(paragraph)
        {
            return Ok(());
        }
        let Some(holder) =
            super::oversized_edges::holder(document, env.cascade, paragraph, env.body_id)
        else {
            return Ok(());
        };
        let top = current_abs_y(document, holder);
        let page = env.page_origins.page_index_in_flow(top).unwrap_or(0);
        let start = env.page_origins.origin(page);
        let step = env.page_origins.step_at(page);
        let layout = &document.nodes[paragraph].layout;
        let oversized = layout.padding.top + layout.border.top >= step
            || layout.padding.bottom + layout.border.bottom >= step;
        if !oversized || top <= start + 0.001 {
            return Ok(());
        }
        let next = page.saturating_add(1);
        reach_page(env.control, next)?;
        let (delta, consumed) = move_line_block(
            document,
            env.cascade,
            env.body_id,
            holder,
            false,
            top,
            env.page_origins.origin(next),
            start,
            &mut self.subtree_shifts,
            entry.page_before_candidate,
            &mut self.float_gaps,
        );
        self.note_line_block_move(text, holder, delta, consumed, next);
        Ok(())
    }

    /// Move a deferred first row without moving its grid's leading edges.
    fn move_first_grid_row(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        effective_y: f32,
    ) -> Option<PageCandidate> {
        let row = candidate.context.first_grid_row()?;
        let flow_y = effective_y
            - accumulated_relative_offset_y(document, env.cascade, env.body_id, candidate.node_id);
        let page = env
            .page_origins
            .page_index_in_flow(flow_y)
            .filter(|&page| page > self.current_page && self.reach.flow_max >= page)
            .unwrap_or(self.current_page);
        let start = env.page_origins.origin(page);
        let delta = row.content_shift(flow_y, start, start + env.page_origins.step_at(page))?;
        if delta <= 0.001 {
            return None;
        }
        // Every recorded in-flow item follows the deferred first row, including
        // later rows. The grid's leading edges keep their original position.
        let items = document.nodes[candidate.node_id].grid_item_rows.to_vec();
        for item in items {
            let top = current_abs_y(document, item.item);
            materialize_y(document, item.item, top + delta);
        }
        document.nodes[candidate.node_id].layout.size.height += delta;
        document.mark_pagination_extent_dirty(candidate.node_id);
        grow_ancestors_for_shift(
            document,
            env.cascade,
            candidate.node_id,
            delta,
            env.body_id,
            &mut self.subtree_shifts,
            entry.page_before_candidate,
            &IfcBoxShifts::default(),
        );
        self.flow_shift += delta;
        let mut grown = candidate.clone();
        grown.height = document.nodes[candidate.node_id].layout.size.height;
        Some(grown)
    }

    /// Places one delegated atomic-only line without applying box margins or
    /// moving its paragraph's earlier blocks. Later flow takes its growth.
    fn place_leading_atomic_line(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        owner: super::leading_inline::LeadingInlineLine,
    ) -> Result<(), Halt> {
        let (top, height) = super::leading_inline::bounds(document, owner);
        let relative =
            accumulated_relative_offset_y(document, env.cascade, env.body_id, owner.root);
        let flow_top = top - relative;
        let natural = env.page_origins.page_index_for_y(flow_top);
        let start = env.page_origins.origin(natural);
        let end = start + env.page_origins.step_at(natural);
        let pending = self.pending_break.applies_to(document, candidate.node_id);
        let named_page_change = box_changes_page_name(
            entry.name_context_seen,
            &candidate.page_name,
            &entry.comparison_page_name,
            self.last_named.joins_transition(candidate),
        );
        let page_transition = (pending || named_page_change) && self.saw_child;
        let target = if page_transition {
            forced_target_page(self.current_page, natural)
        } else if flow_top > start + 0.001
            && flow_top + height > end + 0.001
            && (candidate.context.leading_block_piece_end().is_some()
                || block_fits_page_step(
                    height,
                    env.page_origins.step_at(natural.saturating_add(1)),
                ))
        {
            natural.saturating_add(1)
        } else {
            natural
        };
        reach_page(env.control, target)?;
        // Forced and named-page boundaries place the atomic margin boxes at
        // the page start. Do not add their line's leading to that placement.
        let placed_top = if page_transition && candidate.context.leading_block_piece_end().is_none()
        {
            super::leading_inline::atomic_flow_top(document, env.cascade, env.body_id, owner)
        } else {
            flow_top
        };
        let shift = (env.page_origins.origin(target) - placed_top).max(0.0);
        if shift > 0.0 {
            let growth = super::leading_inline::move_line(
                document,
                env.cascade,
                env.body_id,
                owner,
                shift,
                &mut self.subtree_shifts,
                entry.page_before_candidate,
            );
            self.deferred_shifts.line_splits.push(LineSplit {
                root: owner.root,
                first_moved: owner.line,
                shift,
                growth,
                page: target,
            });
        }
        if pending {
            self.pending_break.clear();
        }
        self.saw_child = true;
        self.current_page = self.current_page.max(target);
        if box_sets_page_name(
            &candidate.context,
            page_transition,
            page_type_leaves_local_flow(
                entry.keep_local_page_type,
                self.current_page,
                entry.page_before_candidate,
            ),
        ) {
            self.page_names.current = candidate.page_name.clone();
            if candidate.context.is_direct_body_element() {
                self.last_named = LastNamedBox::new(
                    document,
                    candidate,
                    candidate.node_id,
                    candidate.raw_y,
                    candidate.height,
                );
            }
        }
        self.record_pages(
            env.control,
            Some(env.page_origins.page_index_for_end(top + shift + height)),
            true,
        )?;
        self.finish_box(
            document,
            env,
            candidate,
            current_abs_y(document, candidate.node_id),
        )
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
        if let Some(owner) = candidate.context.leading_inline_line() {
            return self.place_leading_atomic_line(document, env, candidate, entry, owner);
        }
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
        let (page_break, mut position) = self.begin_box(document, env, candidate, entry);
        let page_transition =
            self.start_box_page(document, env, candidate, entry, &page_break, &mut position)?;
        let grown = self.move_first_grid_row(document, env, candidate, entry, position.effective_y);
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

        // Sibling adjacency uses the original unpaginated bottom. Only page
        // reach and trailing-edge placement need the grown grid's height.
        let candidate = grown.as_ref().unwrap_or(candidate);
        let height = candidate.height;

        let header_delta = if page_break.overflow.tail_caption {
            make_room_for_caption_header(
                document,
                &mut self.table_headers,
                node_id,
                effective_y,
                height,
                self.current_page,
                page_origins,
            )
        } else {
            make_room_for_table_header(
                document,
                &mut self.table_headers,
                node_id,
                effective_y,
                height,
                self.current_page,
                page_origins,
            )
        };
        effective_y += header_delta;
        if header_delta > 0.001 && page_break.overflow.tail_caption {
            // A row's table is finished around its moved cells with the
            // header copies; a caption has no cell, so the boxes around it
            // follow it down here.
            grow_ancestors_for_shift(
                document,
                cascade,
                node_id,
                header_delta,
                env.body_id,
                &mut self.subtree_shifts,
                page_before_candidate,
                &IfcBoxShifts::default(),
            );
        }
        let page_start = page_origins.origin(self.current_page);
        if page_break.keeps_margin_at_break
            && page_break.margin_top > 0.0
            && (effective_y - page_break.relative_offset - page_break.margin_top - page_start).abs()
                <= 0.001
        {
            self.retained_margin_starts
                .insert(node_id, (self.current_page, page_break.margin_top));
        }
        self.flow_shift += header_delta;

        if candidate.context.is_table_row()
            && let Some(fragment) = super::table_cell_fragments::fitting(
                document,
                cascade,
                node_id,
                effective_y,
                page_start,
                page_origins.origin(self.current_page + 1),
            )
        {
            let (cell, shift) = fragment.apply(document);
            self.fragmented_cell_shifts.insert(cell, shift);
        }

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
        let height = document
            .ifc_layout_node(node_id)
            .and_then(|node| node.ifc.as_ref())
            .and_then(|ifc| ifc.lines.as_ref())
            .filter(|lines| lines.discarded_float_gap > 0.0)
            .map_or(candidate.height, |_| {
                document.nodes[node_id].layout.size.height
            });
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

    pub(super) fn track_outer_owners(&mut self, owners: &HashMap<usize, f32>) {
        self.edge_growth = owners.keys().map(|&owner| (owner, 0.0)).collect();
    }

    /// Breaks the page before the bottom padding and border of a block when
    /// they do not fit below its content on the page the content ends on, so
    /// that they begin the next page instead of running on from the content
    /// across the page edge: browsers do not split edges that way, and one
    /// that cannot fit a page still starts at the top of one. The block grows
    /// by the room left on the page, and the flow after it follows.
    pub(super) fn break_before_edges(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        edges_break: &EdgesBreak,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            text_lines,
            ..
        } = *env;
        if !self.saw_child {
            return Ok(());
        }
        // The flow has left the content of the block: what its lines and
        // boxes moved is settled before the edges are judged. A paragraph is
        // left from outside it, as the box after it leaves it.
        let node_id = edges_break.node_id;
        let outside = if document.nodes[node_id].is_ifc_root() {
            document.parent_of(node_id).unwrap_or(node_id)
        } else {
            node_id
        };
        if let Some(page) = self.deferred_shifts.apply_reached(
            document,
            text_lines,
            outside,
            None,
            self.current_page,
            &mut self.flow_shift,
        ) {
            self.current_page = self.current_page.max(page);
        }
        if let Some(page) = self
            .subtree_shifts
            .leave(document, outside, &mut self.flow_shift)
        {
            self.current_page = page;
        }
        let end = edges_break.raw_y + self.flow_shift;
        let page = page_origins.page_index_for_end(end);
        let page_end = page_origins.origin(page) + page_origins.step_at(page);
        // The content must end below the top of its page, and the edges must
        // reach past the bottom of it from a content end short of that. An end
        // that is not a number fails every comparison and leaves the edges be.
        let gap = page_end - end;
        if !(end > page_origins.origin(page) + 0.001
            && gap > 0.001
            && edges_break.edges > gap + 0.001)
        {
            return Ok(());
        }
        let next_page = page.saturating_add(1);
        let visible = self.visible_edges(edges_break, gap);
        if visible > 0.001 {
            reach_page(control, next_page)?;
        }
        let heights: Vec<_> = if self.edge_growth.is_empty() {
            Vec::new()
        } else {
            document
                .inclusive_ancestors(node_id)
                .filter(|owner| self.edge_growth.contains_key(owner))
                .map(|owner| (owner, document.nodes[owner].layout.size.height))
                .collect()
        };
        document.nodes[edges_break.node_id].layout.size.height += gap;
        document.mark_pagination_extent_dirty(edges_break.node_id);
        grow_ancestors_for_shift(
            document,
            cascade,
            edges_break.node_id,
            gap,
            body_id,
            &mut self.subtree_shifts,
            page,
            &IfcBoxShifts::default(),
        );
        // Bounds and float extents can absorb part of the gap. Record each
        // box's actual growth rather than subtracting the raw gap everywhere.
        for (owner, height) in heights {
            let growth = document.nodes[owner].layout.size.height - height;
            if growth > 0.001 {
                *self.edge_growth.entry(owner).or_default() += growth;
            }
        }
        self.flow_shift += gap;
        self.current_page = self.current_page.max(next_page);
        if visible > 0.001 {
            self.record_pages(control, page_origins.end_page(page_end, visible), true)?;
        }
        Ok(())
    }

    /// The block size of the edges that a clip leaves visible once they begin
    /// `gap` lower, and the pages they run over are those below it. A clip on
    /// a box of its own height stays where it is as the edges are pushed
    /// down, so it hides `gap` more of them, and the more for what the
    /// pushes of the blocks inside it before took.
    fn visible_edges(&mut self, edges_break: &EdgesBreak, gap: f32) -> f32 {
        if !edges_break.clip_end.is_finite() {
            return edges_break.edges;
        }
        let pushed = self
            .edge_pushes
            .entry(edges_break.clip_end.to_bits())
            .or_insert(0.0);
        let visible = edges_break
            .edges
            .min(edges_break.edges + edges_break.clip_room - *pushed - gap);
        *pushed += gap;
        visible
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
                flow_raw_y(document, env.cascade, env.body_id, candidate) + candidate.height,
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
        let mut page_transition = self.saw_child && page_break.is_page_transition();
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
            // A box that no page break goes through and that crosses the end
            // of the first page moves to the second, though nothing comes
            // before it: the margin above it at the start of the document is
            // kept, so it does not start the page.
            if page_break.overflow.unbreakable {
                page_transition = true;
                self.open_page_for_box(document, env, candidate, entry, page_break, position)?;
            }
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
        let height = if box_avoids_page_break_inside(document, node_id, computed.break_inside) {
            super::external_float_gap::fit_height(document, candidate)
        } else {
            candidate.height
        };
        place_first_table_header(
            document,
            &mut self.table_headers,
            node_id,
            self.flow_shift,
            page_origins,
        );
        let effective_y = if uses_positioned_coordinates {
            current_abs_y(document, node_id)
        } else if self.subtree_shifts.line_moved_floats.remove(&node_id) {
            // The lines of its paragraph that came before it in the source
            // took the float along to a later page (see
            // `floats_on_moved_lines`): it stays with them.
            current_abs_y(document, node_id)
        } else {
            raw_y + self.flow_shift
        };
        materialize_y(document, node_id, effective_y);
        let forced_before = page_break_is_forced(document, node_id, computed.break_before);
        // A block box with no block size and no in-flow content lets its
        // block-start and block-end margins collapse through it.
        let margins_collapse_through = collapses_through(document, cascade, node_id, height);
        let (own_margin_top, trailing_margin) =
            break_margins(document, cascade, node_id, content_width);
        let collapses_with_previous_block =
            collapses_with_previous_block(document, candidate, participates_in_margin_adjacency);
        let pending_break_applies = self.pending_break.applies_to(document, node_id);
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
        let continuation_offset = document
            .continuation_margin
            .filter(|&(node, offset)| node == node_id && offset > CONTINUATION_MARGIN_TOLERANCE)
            .map(|(_, offset)| offset);
        // A box that starts a page after a forced break keeps its block-start
        // margin, and the margins of its first children that adjoin its own
        // collapse into that one margin, which sits above its border box. So
        // does a box the break reaches through an empty box or a float, and a
        // box in a relayout, which stands in for the break of an earlier
        // layout without a forced break of its own.
        let keeps_margin_at_break = forced_before
            || pending_break_applies
            || named_page_change
            || self.sibling_margins.forced_break_reaches(
                document,
                cascade,
                node_id,
                flow_raw_y(document, cascade, body_id, candidate),
            )
            || continuation_offset.is_some();
        let margin_top = if keeps_margin_at_break
            && collapses_with_previous_block
            && !document.nodes[node_id].used_writing_mode.is_vertical()
        {
            own_margin_top
                + collapsed_start_margin_gain(
                    document,
                    cascade,
                    node_id,
                    &avoided_breaks.clearance_free,
                )
        } else {
            own_margin_top
        };
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
        let next_page_step = page_origins.step_at(fit_page.saturating_add(1));
        // A box that no page break goes through and that begins the plain
        // blocks around it takes them to the next page, rather than leaving
        // their start on this one. The padding, border and margin they keep
        // above it go with them, so it has to fit the next page below those.
        // Only a box that starts on the page and runs past its end moves, so
        // the walk over its ancestors is left to those.
        let crosses_page_end = effective_y - relative_offset + height + trailing_margin
            > fit_page_start + page_origins.step_at(fit_page);
        let column_piece = candidate.context.leading_column_replaced_piece_end();
        let column_crosses = column_piece.is_some_and(|end| {
            effective_y - relative_offset + end > fit_page_start + page_origins.step_at(fit_page)
        });
        let unbreakable_crosses = effective_y - relative_offset
            + candidate.context.unbreakable_extent(height)
            + trailing_margin
            > fit_page_start + page_origins.step_at(fit_page);
        let holder = if (candidate.context.is_unbreakable() && unbreakable_crosses
            || column_crosses)
            && !keeps_margin_at_break
            && is_block_flow_box(document, cascade, candidate)
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant()
        {
            Some(leading_holder(document, cascade, body_id, node_id, true)).filter(
                |&(holder, lead)| {
                    // The walk cannot see what an anonymous block holds: a
                    // box that no break goes through, further below the
                    // holder than the edges and margins between them, follows
                    // something there and moves alone.
                    holder != node_id
                        && (column_piece.is_some()
                            || lead
                                <= super::leading_space_between(document, cascade, node_id, holder)
                                    + 0.01)
                        && (lead <= 0.0
                            || lead + column_piece.unwrap_or(height) <= next_page_step + 0.001
                            || super::contained_cloned_leading_holder(
                                document, cascade, node_id, holder, lead,
                            ))
                },
            )
        } else {
            None
        };
        // A box that avoids a break inside it, below the padding or border
        // that the plain blocks around it keep above it, goes with them: the
        // page does not break between their start and the box (CSS Break 3
        // §4.1), so it is their margin box that tells whether the box starts
        // its page. Where they start the page, the box stays below that edge
        // and breaks inside, as browsers do, rather than leave it alone at the
        // top of the page. Only a box that runs past the page end moves, so
        // the walk over its ancestors is left to those.
        let avoided_holder = if crosses_page_end
            && !candidate.context.is_unbreakable()
            && !keeps_margin_at_break
            && !document.context.pagination_on_transposed_axes
            && box_avoids_page_break_inside(
                document,
                node_id,
                cascade.computed[node_id].break_inside,
            )
            && is_block_flow_box(document, cascade, candidate)
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant()
        {
            // The blocks begin on the page the box does; blocks that began on
            // an earlier one have left it.
            lead_below_edges(document, cascade, body_id, node_id)
                .filter(|&(_, lead)| effective_y - relative_offset - lead >= fit_page_start - 0.001)
                .map(|(ancestor, lead)| (lead, document.nodes[ancestor].layout.margin.top))
        } else {
            None
        };
        let (avoided_lead, avoided_margin_top) = avoided_holder.unwrap_or((0.0, margin_top));
        // Where the holder has no room above the box of its own, its margin
        // collapses with the margin of the box, and the larger of the two is
        // what a break cuts.
        let (holder_lead, holder_margin_top) =
            holder.map_or((0.0, margin_top), |(holder, lead)| {
                let holder_margin = break_margins(document, cascade, holder, content_width).0;
                if lead > 0.0 {
                    (lead, holder_margin)
                } else {
                    (lead, holder_margin.max(margin_top))
                }
            });
        let fit = BoxFit {
            effective_y,
            flow_y: effective_y - relative_offset,
            margin_top,
            trailing_margin,
            holder_lead,
            holder_margin_top,
            avoided_lead,
            avoided_margin_top,
            page_start: fit_page_start,
            page_end: fit_page_start + page_origins.step_at(fit_page),
            next_page_step,
        };
        let mut overflow = avoided_breaks.measure(
            document,
            cascade,
            candidate,
            &fit,
            &self.table_headers,
            &mut self.trailing_flex_children,
        );
        // A trailing item's later content may fragment, but its first piece
        // carries the box instead of leaving an empty strip on the old page.
        overflow.page |= candidate.context.is_flex_item()
            && self
                .trailing_flex_children
                .is_trailing(document, cascade, node_id)
            && super::column_first_piece::moves(document, cascade, body_id, node_id, &fit);
        // A column can travel with its first replaced or size-contained piece after earlier
        // flow content. At the start of the flow, the column stays and its
        // image moves on alone, even when body or wrapper margins remain.
        overflow.leading_column_replaced = column_piece.is_some_and(|end| {
            avoided_box_moves(
                fit.flow_y - fit.holder_lead,
                fit.holder_margin_top,
                fit.flow_y + end,
                fit.page_start,
                fit.page_end,
            ) && (document.layout_parent_of(node_id) == Some(body_id)
                || !self.child_positions.begins_page_below_leading_edges(
                    document,
                    cascade,
                    body_id,
                    node_id,
                    fit.page_start,
                ))
        });
        overflow.page |= overflow.leading_column_replaced;
        let grid_row = candidate.context.first_grid_row();
        let grid_content_break =
            grid_row.and_then(|row| row.content_shift(fit.flow_y, fit.page_start, fit.page_end));
        overflow.page |= grid_row.is_some_and(|row| {
            row.whole_moves(fit.flow_y, fit.margin_top, fit.page_start, fit.page_end)
        }) || closing_edges_move(candidate, &fit)
            || (grid_content_break.is_none() && leading_edges_move(candidate, &fit))
            || run_tail_move(candidate, &fit);
        // A child's boundary belongs to the first plain holder it begins.
        // Computing that owner does not move it or consume the break.
        let potential_forced_holder = (forced_before
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant()
            && is_block_flow_box(document, cascade, candidate))
        .then(|| {
            self.child_positions
                .forced_holder(document, cascade, body_id, node_id)
        })
        .flatten()
        .map(|(node, lead)| {
            let margin_top = break_margins(document, cascade, node, content_width).0
                + collapsed_start_margin_gain_in_flow(
                    document,
                    cascade,
                    body_id,
                    node,
                    &avoided_breaks.clearance_free,
                );
            ForcedBreakHolder {
                node,
                lead,
                margin_top,
                direct_body: super::candidates::candidate_parent(document, node)
                    == Some(env.roots.contents_box),
            }
        });
        // The child reaching a natural edge does not put its holder there.
        // The source-order check also keeps an empty preceding sibling from
        // sharing a boundary merely because its geometry has zero height.
        let forced_break_at_page_start = forced_before
            && if let Some(owner) = potential_forced_holder {
                let owner_margin_box_y = current_abs_y(document, owner.node)
                    - accumulated_relative_offset_y(document, cascade, body_id, owner.node)
                    - owner.margin_top;
                owner_margin_box_y <= page_start + 0.001
                    && self.child_positions.begins_page_below_leading_edges(
                        document, cascade, body_id, node_id, page_start,
                    )
            } else {
                margin_box_starts_page(
                    page_origins,
                    effective_y - margin_top.max(0.0),
                    self.current_page,
                    page_start,
                ) || self.child_positions.begins_page_below_leading_edges(
                    document, cascade, body_id, node_id, page_start,
                )
            };
        // CSS Fragmentation 3 §5.2: only a new forced boundary retains the
        // leading margin. Existing boundaries do not open another page.
        let forced_transition = pending_break_applies
            || (forced_before && !forced_break_at_page_start)
            || named_page_change;
        let forced_holder = potential_forced_holder.filter(|_| !forced_break_at_page_start);
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
            keeps_margin_at_break,
            keeps_continuation_margin: continuation_offset
                .is_some_and(|offset| (offset - margin_top).abs() <= CONTINUATION_MARGIN_TOLERANCE),
            holder,
            forced_holder,
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
            forced_holder,
            ..
        } = *page_break;
        let node_id = candidate.node_id;
        // A box that no page break goes through, or that avoids one inside
        // it, and that begins the plain blocks around it takes them to the
        // next page, rather than leaving their start on this one: they start
        // the page, and the box lands below the padding, border and margin
        // they keep above it where `begin_box` found room for it there. Else
        // the blocks that start where the box does move with it.
        let (holder, holder_lead) = if let Some(owner) = forced_holder {
            (owner.node, owner.lead)
        } else if (overflow.unbreakable
            || overflow.moved_for_avoided_break
            || overflow.leading_column_replaced)
            && !forced_transition
            && is_block_flow_box(document, cascade, candidate)
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant()
        {
            page_break
                .holder
                .filter(|_| overflow.unbreakable || overflow.leading_column_replaced)
                .unwrap_or_else(|| leading_holder(document, cascade, body_id, node_id, false))
        } else {
            (node_id, 0.0)
        };
        let margin_top = forced_holder.map_or(margin_top, |owner| owner.margin_top);
        let holder_relative = forced_holder.map_or(relative_offset, |owner| {
            accumulated_relative_offset_y(document, cascade, body_id, owner.node)
        });
        let landing_y = forced_holder.map_or(position.effective_y - holder_lead, |owner| {
            current_abs_y(document, owner.node) - holder_relative
        });
        // Content kept on its page as monolithic (see `monolithic_paper_fit`)
        // has not opened a later page.
        let natural_page = page_origins.transition_natural_page(
            if forced_holder.is_some() {
                landing_y
            } else {
                position.effective_y
            },
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
        // The margin a forced break keeps above a box that no page break
        // goes through puts the box below the top of its page, so it moves on
        // to the next page when it would cross the edge of this one. That
        // break is not forced, and cuts the margin.
        let landing_top = page_origins.origin(target_page)
            + margin_top
            + if forced_holder.is_some() {
                holder_lead
            } else {
                0.0
            };
        let margin_pushes_box_on = forced_transition
            && margin_top > 0.0
            && candidate.context.is_unbreakable()
            && is_block_flow_box(document, cascade, candidate)
            && starts_on_page_and_runs_past_it(
                landing_top,
                landing_top + candidate.height,
                page_origins.origin(target_page),
                page_origins.page_end(target_page),
            );
        if margin_pushes_box_on {
            target_page = target_page.saturating_add(1);
            reach_page(control, target_page)?;
        }
        let lands_after_forced_break = forced_transition && !margin_pushes_box_on;
        if pending_break_applies && is_floating_box_for_pagination(document, cascade, node_id) {
            document.context.float_resumption_boundaries.insert(
                node_id,
                super::float_resumption::Boundary {
                    page_origin: page_origins.origin(target_page),
                    relative_offset,
                },
            );
        }
        // A negative block-start margin can pull a forced-break box
        // back into the preceding page.  Keep the break boundary for
        // following siblings (and for page count), but materialize
        // this box at the underflowed coordinate.  This is the
        // `underflow-from-next-page` case from CSS Break.
        let landing = TransitionLanding::new(
            page_origins.origin(target_page),
            landing_y,
            margin_top,
            if forced_holder.is_some() {
                0.0
            } else {
                relative_offset
            },
            lands_after_forced_break,
            overflow,
        );
        let row_tail_move = overflow.page && candidate.context.item_tail_edges() > 0.0;
        let row_flex_parent = (forced_before || row_tail_move)
            .then(|| row_flex_container(document, cascade, node_id))
            .flatten();
        if landing.is_finite() {
            if !lands_after_forced_break {
                let consumed = self.float_gaps.discard_in_holder(
                    document,
                    cascade,
                    holder,
                    landing.node_delta,
                    body_id,
                );
                self.queue_prefixes(consumed, target_page);
            }
            // Capture tail membership before this transition changes positions;
            // an earlier transition may already have shifted the whole parent.
            let tail_line = row_flex_parent.filter(|_| row_tail_move).map(|parent| {
                let before_y = current_abs_y(document, node_id);
                (
                    before_y,
                    super::flex_line_peers(document, parent, node_id, before_y),
                )
            });
            let moved_in_bounded_ancestor = move_box_for_transition(
                document,
                cascade,
                body_id,
                candidate,
                &TransitionMove {
                    node_delta: landing.node_delta,
                    shift_delta: landing.shift_delta,
                    page_overflow: overflow.page || margin_pushes_box_on,
                    forced_transition: lands_after_forced_break,
                    holder,
                    forced_holder,
                },
                &mut self.subtree_shifts,
                entry.page_before_candidate,
            );
            // Forced breaks and closing-tail breaks belong to the whole
            // aligned flex line; the other lines keep their flow coordinates.
            if let Some((before_y, peers)) = tail_line {
                let delta = current_abs_y(document, node_id) - before_y;
                super::place_flex_line_peers(document, &peers, delta);
            } else if let Some(parent_id) = row_flex_parent {
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
            // A carried holder retains its margin in the following flow too.
            // The ordinary child-only underflow leaves that flow at the boundary.
            if forced_holder.is_none()
                && let Some(correction) = landing.underflow_correction()
            {
                self.deferred_shifts.underflow = Some((holder, correction));
            }
            self.float_shifts.settle_transition(
                document,
                cascade,
                body_id,
                candidate,
                &FloatTransition {
                    page_overflow: overflow.page || margin_pushes_box_on,
                    forced_transition: lands_after_forced_break,
                    delta: landing.node_delta,
                    target_y: landing.target_y
                        + holder_lead
                        + if forced_holder.is_some() {
                            relative_offset
                        } else {
                            0.0
                        },
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
                self.sibling_margins.break_taken_by_float = self
                    .pending_break
                    .source
                    .map(|source| (source, target_page));
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
            avoided_breaks,
            ..
        } = *env;
        let BoxBreak {
            margin_top,
            forced_before,
            margins_collapse_through,
            collapses_with_previous_block,
            keeps_continuation_margin,
            ..
        } = *page_break;
        let node_id = candidate.node_id;
        // A relative offset only paints the box elsewhere: the page it starts
        // on, and the page edge that cuts the margin above it, are found from
        // its position in the flow.
        let flow_y = position.effective_y - relative_offset;
        let mut natural_page = page_origins.start_page(flow_y, candidate.height);
        // A forced break that coincides with a page edge also starts
        // that page, and so does the break a relayout stands in it for.
        let forced_before = forced_before || keeps_continuation_margin;
        if forced_before {
            self.sibling_margins.forced_break_box = margins_collapse_through.then_some(node_id);
            position.starts_after_forced_break = true;
        }
        // An unforced break that falls inside the collapsed margin
        // between this box and its preceding sibling truncates that
        // margin (CSS Fragmentation 3 §5.2): the box starts at the
        // top of the page instead of keeping the part of the margin
        // that crossed the page edge.
        let at = MarginBreak {
            node_id,
            clearance_free: avoided_breaks.clearance_free.contains(node_id),
            raw_y: flow_raw_y(document, cascade, body_id, candidate),
            effective_y: flow_y,
            natural_page,
            forced_before,
            margin_top,
            collapses_with_previous_block,
            run_end: avoided_breaks.margin_run_end(node_id),
        };
        // An empty box stands where the box after it starts when a negative
        // margin there pulls that box above it, also while no margin is cut:
        // it must not take the flow on to a page that box is not on.
        let pull = self.sibling_margins.pulled_run_end(document, cascade, &at);
        let pulled = pull.map_or(0.0, |(_, pulled)| pulled);
        if let Some((end, pulled)) = pull {
            natural_page = page_origins.start_page(flow_y - pulled, end.height);
        }
        let truncation =
            self.sibling_margins
                .truncation_at_break(document, cascade, page_origins, &at);
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
                // A pulled empty box stands where the box after it lands,
                // which is the page the flow reaches.
                natural_page = page_origins.page_index_for_y(flow_y + delta - pulled);
            }
        }
        reach_page(control, natural_page)?;
        self.current_page = self.current_page.max(natural_page);
        Ok(())
    }

    /// Whether the lines of the paragraph `block_id` move down inside it, to
    /// the next page, rather than the paragraph moving as a unit: it is its own
    /// inline layout root, has no earlier split pending, and can shift its
    /// lines.
    fn lines_can_move(
        &self,
        document: &LayoutSession<'_>,
        cascade: &CascadeResult,
        blocks: &LineBlocks,
        block_id: usize,
    ) -> bool {
        blocks.paragraph_root == Some(block_id)
            && self.deferred_shifts.line_splits.is_empty()
            && (lines_can_shift_inside(document, cascade, block_id)
                || lines_can_shift_above_blocks(document, cascade, block_id))
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
            && !self.lines_can_move(document, cascade, blocks, block_id)
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
                let (delta, consumed) = move_line_block(
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
                    &mut self.float_gaps,
                );
                self.note_line_block_move(text, block_id, delta, consumed, target_page);
            }
        }
        Ok(())
    }

    /// Applies a whole-box move to the current text and defers prefix
    /// changes for later candidates of the flow.
    fn note_line_block_move(
        &mut self,
        text: &mut TextPosition,
        block_id: usize,
        delta: f32,
        consumed: Vec<super::external_float_gap::Removal>,
        page: u32,
    ) {
        self.flow_shift += delta;
        text.effective_y += delta;
        for removal in &consumed {
            if removal.root == block_id {
                text.effective_y -= removal.removed;
                text.first_line_shift -= removal.removed;
            }
        }
        self.queue_prefixes(consumed, page);
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
    /// paragraph that is a flex item alone on its row, or that ends an
    /// eligible column flex box, cannot move as a unit here, but its later
    /// lines can move down inside it. The padding and border
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
            .or(blocks.column_paragraph.map(|root| (root, true)))
            && self.checked_paragraphs.orphans_widows.insert(block_id)
            && let Some(line_metrics) = paragraph_line_bounds(document, node_id, text_lines)
        {
            let lines_can_move = self.lines_can_move(document, cascade, blocks, block_id);
            // The break that keeps the last lines with the padding and border
            // that close the boxes around the paragraph, on `page`.
            let closing_edges = candidate.context.lines_closing_edges();
            // A paragraph that avoids a break inside it moves whole with the
            // edges when they fit a page together, along with the blocks it
            // begins (a padding above it goes with it, when those blocks start
            // on this page). Only when they cannot, the page breaks between
            // its lines rather than between its last line and the edges. A box
            // taller than its lines, with a height of its own, say, has the
            // edges after the box and not after the lines: it is left whole.
            let avoided_whole = closing_edges > 0.0
                && box_avoids_page_break_inside(
                    document,
                    block_id,
                    cascade.computed[block_id].break_inside,
                )
                && {
                    let layout = &document.nodes[block_id].layout;
                    let own_edges = layout.padding.bottom + layout.border.bottom;
                    let lines = line_metrics[line_metrics.len() - 1].1 - line_metrics[0].0;
                    let content =
                        layout.size.height - own_edges - layout.padding.top - layout.border.top;
                    let block_top = block_effective_y(
                        document,
                        block_id,
                        candidate.raw_y,
                        text.effective_y,
                        root_flow_offset,
                        self.flow_shift,
                    );
                    let page_start = page_origins.origin(self.current_page);
                    let lead = lead_below_edges(document, cascade, body_id, block_id)
                        .map(|(_, lead)| lead)
                        .filter(|lead| block_top - lead >= page_start - 0.001)
                        .unwrap_or(0.0);
                    (content - lines).abs() > 0.5
                        || lead + layout.border.top + layout.padding.top + lines + closing_edges
                            <= page_origins.step_at(self.current_page) + 0.001
                };
            let closing_edges_break = |page: u32, effective_y: f32| {
                if avoided_whole {
                    return None;
                }
                closing_edges_line_move(
                    &line_metrics,
                    closing_edges,
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
            // A box that keeps its height ends where it ends, wherever its
            // lines go: its edges follow the lines only while they all fit
            // the page, before any of them moves.
            let edges_follow_moved_lines = !lines_can_move
                || own_closing_edges <= 0.0
                || lines_move_grows_box(document, block_id);
            let mut page = self.current_page;
            // A block moved whole to the next page is judged once more from
            // there, for the lines that break again on the pages after it.
            let mut moved_whole = false;
            loop {
                let page_start = page_origins.origin(page);
                let page_step = page_origins.step_at(page);
                let lines_fit = line_metrics.last().is_some_and(|(_, bottom)| {
                    text.effective_y + bottom <= page_start + page_step + 0.001
                });
                let mut line_fit = ParagraphLineFit::measure(
                    &line_metrics,
                    text.effective_y,
                    page_start,
                    page_start + page_step,
                    cascade.computed[block_id].orphans,
                    cascade.computed[block_id].widows,
                    if edges_follow_moved_lines || lines_fit {
                        own_closing_edges
                    } else {
                        0.0
                    },
                );
                line_fit.first_fragment = !moved_whole;
                // The block is measured from where it is now: lines moved
                // inside it leave it where it was.
                let block_effective_y = block_effective_y(
                    document,
                    block_id,
                    candidate.raw_y,
                    text.effective_y - text.first_line_shift,
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
                // The page already begins with the holder's leading edges.
                // Keep a fitting first line instead of breaking before the
                // first child again to satisfy orphans.
                let leading_page_lines = lines_can_move
                    && line_fit.lines_before_break > 0
                    && self.child_positions.begins_page_below_leading_edges(
                        document, cascade, body_id, block_id, page_start,
                    );
                let retained_margin_opens_page = self
                    .retained_margin_starts
                    .get(&block_id)
                    .is_some_and(|&(opening_page, margin)| {
                        opening_page == page
                            && (block_effective_y
                                - accumulated_relative_offset_y(
                                    document, cascade, body_id, block_id,
                                )
                                - margin
                                - page_start)
                                .abs()
                                <= 0.001
                    });
                // A kept margin already consumes the opening break. Preserve
                // a line that fits instead of breaking before that margin again.
                let keep_first_fragment =
                    retained_margin_opens_page && lines_can_move && line_fit.lines_before_break > 0;
                let can_move_block = !(moved_whole || leading_page_lines || keep_first_fragment)
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
                        // The floats that put the first line down end where
                        // it starts, so they stay on the page it leaves. The
                        // break falls between the empty space they left at the
                        // top of the block and the line, as in a block whose
                        // content starts below a gap (CSS Fragmentation 3
                        // §4.1): the block stays and its lines move down,
                        // rather than the block carrying that space along.
                        // A relative offset moves the line along with the
                        // block, which the flow leaves where it is.
                        let below_floats = (lines_can_move
                            && floats_put_first_line_down(
                                document,
                                block_id,
                                text_lines.get(node_id),
                                first_line_top - (block_effective_y + block_edges),
                            ))
                        .then(|| {
                            page_origins.origin(page)
                                + accumulated_relative_offset_y(
                                    document, cascade, body_id, block_id,
                                )
                                - first_line_top
                        })
                        .filter(|shift| *shift > 0.0);
                        if let Some(shift) = below_floats {
                            self.move_lines(
                                document,
                                env,
                                candidate,
                                entry,
                                blocks,
                                text,
                                block_id,
                                [(0, shift, page)],
                            )?;
                        } else {
                            reach_page(control, page)?;
                            let (delta, consumed) = move_line_block(
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
                                &mut self.float_gaps,
                            );
                            self.note_line_block_move(text, block_id, delta, consumed, page);
                        }
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
    /// paragraph follows the moved lines. The text's `first_line_shift` takes
    /// how far the moves took the run's first line. A run with a gap between
    /// two of its lines, which a float leaves, is not judged: its lines stay
    /// where the layout put them.
    pub(super) fn keep_lines_after_block(
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
            ..
        } = *env;
        let node_id = candidate.node_id;
        // Text placed by positioned insets, or inside a float or an
        // out-of-flow box, is not in the flow that the moved lines push down.
        let in_flow = !entry.uses_positioned_coordinates
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant();
        let Some(root) = blocks.paragraph_root.filter(|_| in_flow) else {
            return Ok(());
        };
        if !self.deferred_shifts.line_splits.is_empty()
            || !lines_can_shift_between_blocks(document, env.cascade, root)
        {
            return Ok(());
        }
        let Some(run) = lines_between_blocks(document, text_lines, node_id, root) else {
            return Ok(());
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
            return Ok(());
        };
        // A float that a line cannot sit beside, or a clearance below one,
        // leaves a gap of at least a fraction of a pixel between two lines of
        // the run: they do not break as one block of lines, and the gap is not
        // theirs to move.
        if metrics.windows(2).any(|pair| pair[1].0 - pair[0].1 > 0.5) {
            return Ok(());
        }
        let first_owned_line = text_lines
            .get(node_id)
            .and_then(|owned| owned.lines.first())
            .map_or(run.start, |line| line.line);
        // The padding and border of the paragraph below its last line stay
        // with the last line of the run, when no block child comes after it:
        // that block is what they follow instead. Those of the boxes around
        // the paragraph follow its edges.
        let closing_edges = if run_ends_paragraph(document, root, &run) {
            let layout = &document.nodes[root].layout;
            (layout.padding.bottom + layout.border.bottom)
                .max(candidate.context.lines_closing_edges())
        } else {
            0.0
        };
        // The run is judged from where its own first line lies. A float that
        // the paragraph holds before the lines may sit lower than they do, on
        // the next page, and take the flow's page there before the lines are
        // reached; the lines still start on the page they are on.
        let mut page = page_origins.page_index_for_y(text.effective_y + metrics[0].0 + 0.001);
        let mut moved = false;
        loop {
            let page_start = page_origins.origin(page);
            let page_step = page_origins.step_at(page);
            let mut fit = ParagraphLineFit::measure(
                &metrics,
                text.effective_y,
                page_start,
                page_start + page_step,
                cascade.computed[root].orphans,
                cascade.computed[root].widows,
                closing_edges,
            );
            fit.first_fragment = !moved;
            let run_top = text.effective_y + metrics[0].0;
            let run_height = metrics[metrics.len() - 1].1 - metrics[0].0;
            // A run taller than a page moves whole only to keep its orphans,
            // when content comes before it on this page.
            let can_move_block = !moved
                && (block_fits_page_step(run_height, page_step)
                    || (run_top > page_start + 0.001
                        && fit
                            .gains_from_moving_tall_block(metrics[0].1 - metrics[0].0, page_step)));
            // The padding and border below the last line do not fit, and the
            // page cannot give up the widows and keep the orphans: the break
            // inside the lines would leave too few of them on one side. The
            // block that holds them has a break before the lines, between
            // them and the box above, and Chromium takes that one instead.
            // Judged once more from the next page, the lines break there
            // inside as the edges ask when they still do not fit.
            let edges_leave_too_few_lines =
                fit.edges_leave_too_few_lines() && run_top > page_start + 0.001;
            let fix = if edges_leave_too_few_lines {
                LineBreakFix::MoveBlock
            } else {
                fit.fix(
                    &metrics,
                    text.effective_y,
                    page,
                    page_origins,
                    can_move_block,
                    || true,
                )
            };
            let (moves, repeat): (Vec<LineBreakMove>, bool) = match fix {
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
                let growth = move_paragraph_lines(
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
                    text.first_line_shift += shift;
                } else if reaches_line(text_lines.get(node_id), line) {
                    text.height += shift;
                }
                self.deferred_shifts.line_splits.push(LineSplit {
                    root,
                    first_moved: line,
                    shift,
                    growth,
                    page: move_page,
                });
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
        Ok(())
    }

    /// Remembers the absolute owner of the paragraph holding `candidate` for
    /// [`Self::keep_positioned_lines`], after the outer flow has settled.
    /// Fixed ancestors and vertical or transposed pagination are excluded.
    fn note_positioned_paragraph(
        &mut self,
        document: &LayoutSession<'_>,
        candidate: &PageCandidate,
    ) {
        if !candidate.context.is_out_of_flow_descendant() {
            return;
        }
        // The paragraph of the text is the box's own, even when a paragraph
        // around the box was entered before.
        let Some((root, _)) = candidate.ifc_root else {
            return;
        };
        if let std::collections::hash_map::Entry::Vacant(vacant) =
            self.positioned_paragraphs.entry(root)
        {
            let horizontal = document
                .ifc_layout_node(root)
                .and_then(|node| node.ifc.as_ref())
                .is_some_and(|ifc| ifc.writing_mode == shodo::geometry::WritingMode::HorizontalTb);
            let owner = document
                .inclusive_ancestors(root)
                .find(|&id| {
                    document.nodes[id].kind() == NodeKind::Element
                        && !document.nodes[id].generates_no_box()
                        && matches!(
                            document.nodes[id].position(),
                            PositionValue::Absolute | PositionValue::Fixed
                        )
                })
                .filter(|&id| is_unfixed_absolute_box(document, id));
            vacant.insert(
                owner
                    .filter(|_| horizontal && !document.context.pagination_on_transposed_axes)
                    .map(|owner| PositionedParagraph {
                        owner,
                        text: candidate.node_id,
                        first_raw_y: candidate.raw_y,
                    }),
            );
        }
    }

    /// Keeps each line of the absolutely positioned boxes whole on one page:
    /// the line that crosses a page end starts the next page, and the lines
    /// after it move down with it. Such a box breaks only between its lines,
    /// without regard to `orphans` and `widows`, and the padding above its
    /// first line stays where the box is. A box with nothing above its first
    /// line moves whole when that line moves. This waits for the whole flow to
    /// be placed, as the containing block of a box moves the box when its own
    /// lines move. Nothing else moves: a box outside the flow takes no room in
    /// it.
    pub(super) fn keep_positioned_lines(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        clip_edges: &HashMap<usize, f32>,
        owner_heights: &HashMap<usize, f32>,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            control,
            page_origins,
            text_lines,
            ..
        } = *env;
        let paragraphs = std::mem::take(&mut self.positioned_paragraphs);
        // A box inside another sits where the lines of the outer one leave it,
        // so the outer boxes come first.
        let owners: std::collections::HashSet<_> = paragraphs
            .values()
            .flatten()
            .map(|paragraph| paragraph.owner)
            .collect();
        let mut owners: Vec<_> = owners.into_iter().collect();
        owners.sort_by_cached_key(|&owner| (document.inclusive_ancestors(owner).count(), owner));
        let followers = std::cell::OnceCell::new();
        let changed =
            super::positioned_flow::changed_owners(document, owner_heights, &self.edge_growth);
        if !changed.is_empty() {
            let followers =
                followers.get_or_init(|| super::positioned_flow::HeightFollowers::new(document));
            for &owner in changed.keys() {
                followers.queue_outer_resize(document, owner);
            }
            followers.flush(document, control, Some(&changed))?;
        }
        for root in owners {
            if !lines_can_shift_inside(document, cascade, root) {
                let followers = followers
                    .get_or_init(|| super::positioned_flow::HeightFollowers::new(document));
                super::positioned_flow::keep(document, env, root, &mut self.reach, followers)?;
                followers.flush(document, control, None)?;
                continue;
            }
            let Some(Some(paragraph)) = paragraphs.get(&root) else {
                continue; // cov:ignore: a text-only owner reaches this legacy branch through its own indexed paragraph; descendant block flows take the other branch.
            };
            let text = paragraph.text;
            let reflowed = followers
                .get()
                .is_some_and(|followers| followers.was_reflowed(document, root));
            let mut current_lines = TextLineIndex::default();
            let text_lines = if reflowed {
                current_lines
                    .prepare_for(document, text, || control.check_aborted())
                    .map_err(Halt::Failed)?;
                &current_lines
            } else {
                text_lines
            };
            // Blocks among the lines would stay behind when the lines move.
            let Some(mut line_metrics) = lines_can_shift_inside(document, cascade, root)
                .then(|| paragraph_line_bounds(document, text, text_lines))
                .flatten()
            else {
                continue;
            };
            // A clip hides the lines past its edge, and they do not count.
            let fresh_edge =
                reflowed.then(|| super::positioned_flow::visible_end(document, env, root));
            if let Some(edge) = fresh_edge.or_else(|| clip_edges.get(&root).copied()) {
                let start = if reflowed {
                    positioned_text_start_y(document, text_lines, text, Some(root))
                } else {
                    paragraph.first_raw_y
                };
                let visible = edge - start;
                line_metrics
                    .truncate(line_metrics.partition_point(|&(top, _)| top < visible - 0.001));
            }
            let start = positioned_text_start_y(document, text_lines, text, Some(root));
            let nothing_above_first_line = document
                .ifc_layout_node(root)
                .is_some_and(|node| node.layout.border.top + node.layout.padding.top <= 0.001);
            for (line, shift, page) in positioned_line_moves(&line_metrics, start, page_origins) {
                reach_page(control, page)?;
                self.reach.include_checked(control, page, false)?;
                if line == 0 && nothing_above_first_line {
                    let y = current_abs_y(document, root) + shift;
                    materialize_y(document, root, y);
                } else {
                    // Floats here keep positioned coordinates and stay where
                    // the lines put them.
                    let (growth, _) = shift_paragraph_lines(
                        document,
                        cascade,
                        &LineMove {
                            block_id: root,
                            flex_line: None,
                            line,
                            shift,
                        },
                    );
                    if growth.abs() > 0.001 {
                        followers
                            .get_or_init(|| super::positioned_flow::HeightFollowers::new(document))
                            .queue_resize(document, root);
                    }
                }
            }
            if let Some(followers) = followers.get() {
                followers.flush(document, control, None)?;
            }
        }
        Ok(())
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
            let growth = move_paragraph_lines(
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
            if line == 0 {
                // The first line moves, and the text that starts on it with it.
                text.effective_y += shift;
                text.first_line_shift += shift;
            } else if reaches_line(text_lines.get(candidate.node_id), line) {
                text.height += shift;
            }
            self.deferred_shifts.line_splits.push(LineSplit {
                root: block_id,
                first_moved: line,
                shift,
                growth,
                page,
            });
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
                // The text of a box outside the flow falls on the page its
                // insets put it on, which says nothing about the page the
                // flow is on.
                if !candidate.context.is_out_of_flow_descendant() {
                    self.current_page = self.current_page.max(natural_page);
                }
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
            let overflow =
                content_overflow(document, cascade, body_id, root_flow_offset, content_height);
            let end_page = (overflow.end.is_finite() && overflow.end > 0.0)
                .then(|| page_origins.page_index_for_end(overflow.end));
            let start_page = overflow
                .hidden_start_y
                .map(|start| page_origins.page_index_for_y(start));
            if let Some(page) = end_page.max(start_page) {
                self.reach.include_checked(control, page, false)?;
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
        self.deferred_shifts
            .activate_prefixes(self.current_candidate_rank, &mut self.subtree_shifts);
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
        let reached_page = self.deferred_shifts.apply_reached(
            document,
            text_lines,
            node_id,
            candidate.ifc_root,
            self.current_page,
            &mut self.flow_shift,
        );
        if let Some(page) = reached_page
            && in_flow
        {
            self.current_page = self.current_page.max(page);
        }
        // A box that kept its height ends on an earlier page than its moved
        // lines, and the flow goes on from there, never from before the page
        // it was on when it reached them.
        if in_flow && let Some(floor) = self.deferred_shifts.page_floor.take() {
            let box_end_page = page_origins.page_index_for_y(candidate.raw_y + self.flow_shift);
            self.current_page = self.current_page.min(floor.max(box_end_page));
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

/// Whether `candidate` is a box of ordinary block flow: not a flex or grid
/// item, which their container places, and not a float.
fn is_block_flow_box(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    candidate: &PageCandidate,
) -> bool {
    !candidate.context.is_flex_item()
        && !candidate.context.is_grid_item()
        && !is_floating_box_for_pagination(document, cascade, candidate.node_id)
}
