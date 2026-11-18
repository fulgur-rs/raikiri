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
    TransitionMove, accumulated_relative_offset_y, avoided_box_moves, block_after_last_line,
    block_effective_y, block_fits_page_step, border_box_height_bounds,
    box_avoids_page_break_inside, box_changes_page_name, box_occupies_column, box_sets_page_name,
    break_margins, closing_edges_line_move, closing_edges_move, collapsed_start_margin_gain,
    collapses_through, collapses_with_previous_block, content_end_above, content_overflow,
    current_abs_y, first_box_ancestor_at_start, first_line_judged_height, first_owned_line_height,
    fits_decorated_paper, flex_line_paragraph, floats_put_first_line_down, flow_raw_y,
    follows_box_on_page, follows_flow_content, forced_line_breaks, forced_line_moves,
    forced_target_page, generated_content_runs_past, grow_ancestors_below,
    grow_ancestors_for_shift, grow_container_beside_item, has_explicit_positioned_block_ancestor,
    has_fixed_height, has_footnote_ancestor, height_follows_content,
    is_floating_box_for_pagination, is_unfixed_absolute_box, item_bottom_in, item_of,
    last_flow_box, lead_below_ancestor, lead_below_edges, leading_edges_move, leading_holder,
    line_block_parent, line_run_among_blocks, lines_between_blocks, lines_can_shift_above_blocks,
    lines_can_shift_between_blocks, lines_can_shift_inside, lines_move_grows_box,
    make_room_for_caption_header, make_room_for_table_header, margin_box_starts_page,
    margin_kept_by_floats, margins_adjoin_in_flow, materialize_y, move_box_by_truncated_margin,
    move_box_for_transition, move_line_block, move_paragraph_lines, page_break_is_forced,
    page_type_leaves_local_flow, paragraph_line_bounds, paragraph_line_moves,
    participates_in_margin_adjacency, place_first_table_header, place_table_footer,
    place_text_candidate, positioned_line_moves, positioned_text_start_y, push_in_flow_boxes_rev,
    reaches_line, record_candidate_pages, reset_inline_x_for_page_transition, row_flex_container,
    run_ends_paragraph, run_tail_move, shift_paragraph_lines, shift_text_candidate,
    shrink_ancestors_after_content_change, starts_on_page_and_runs_past_it, straddling_line,
    text_changes_page_name, text_sets_page_name, truncation_after_generated_content,
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
    /// The edge of the nearest clip around the lines of each inline layout
    /// root that has one, in raw flow coordinates: the lines past it never
    /// move. It is the end of the clipping box when a clip margin reaches
    /// past that box; the lines in the margin stay visible where they are.
    pub(super) clip_edges: &'a HashMap<usize, super::candidates::LineClip>,
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
    /// The flex line of the box, as an item of a row flex container, when a
    /// forced break or a change of the page type may move it.
    pub(super) flex_line: Option<super::RowFlexLine>,
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
    /// How much of the margin above the edges of each block a page edge
    /// truncated (see [`Self::truncate_margin_above_edges`]): the block ends
    /// that much above the margin box of its last child.
    pub(super) margin_cuts: HashMap<usize, f32>,
    /// The blocks that end at a margin kept below their last box (see
    /// [`Self::end_block_at_kept_margin`]), which their content does not
    /// extend past that end once pagination is over.
    pub(super) kept_margin_ends: Vec<usize>,
    /// For each block, the block size of the margins inside it that a page
    /// edge may truncate (see [`super::truncatable_margin_rooms`]), which the pages
    /// its run-on reaches leave out.
    pub(super) margin_rooms: HashMap<usize, f32>,
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
            margin_cuts: HashMap::new(),
            kept_margin_ends: Vec::new(),
            margin_rooms: HashMap::new(),
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
            // and what follows the table follows the footer: the shifts that
            // end before it, as inside a cell of those rows, are settled
            // first.
            if self
                .table_headers
                .headers
                .get(&group)
                .is_some_and(|footer| footer.footer && footer.visit_shift.is_none())
                && let Some(page) =
                    self.subtree_shifts
                        .leave(document, node_id, &mut self.flow_shift)
            {
                self.current_page = page;
            }
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
        // Content that places no box of the flow, as generated content, an
        // empty box or a phantom line, comes before the first box too, so a
        // page type the box names opens a page after it.
        let content_before = self.saw_child
            || (name_participates_in_flow
                && follows_box_on_page(document, env.cascade, env.body_id, node_id, 0.0));
        let (name_context_seen, comparison_page_name) = self.page_names.comparison(
            candidate,
            content_before,
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
        // A relative offset of the paragraph or of a block around it only
        // paints the lines elsewhere: the page breaks between them where the
        // flow puts them (CSS Fragmentation 3 §5.5). An offset of an inline
        // box inside the paragraph moves its text within the lines, which
        // the lines' own checks do not measure.
        let relative_offset =
            flow_relative_offset_y(document, env, candidate, entry).map_or(0.0, |text_offset| {
                text_ifc_root.map_or(text_offset, |root| {
                    accumulated_relative_offset_y(document, cascade, env.body_id, root)
                })
            });
        text.effective_y -= relative_offset;
        self.reach_paragraph_page(document, env, candidate, entry, &blocks, &text)?;
        self.push_first_line_block(document, env, candidate, entry, &blocks, &mut text)?;
        self.keep_lines_together(document, env, candidate, entry, &blocks, &mut text)?;
        self.keep_lines_after_block(document, env, candidate, entry, &blocks, &mut text)?;
        text.effective_y += relative_offset;
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
        // The text that pagination moved down past the end of a clip around
        // its lines, as a forced break inside the clipping box does, paints
        // nowhere and reaches no page.
        let reach_height = text
            .height
            .min((text_clip_end(document, env, candidate) - text.effective_y).max(0.0));
        if name_participates_in_flow && reach_height > 0.0 {
            self.column_contexts
                .occupy_enclosing(node_id, self.current_page, document, cascade);
        }
        let end_page = page_origins.end_page(text.effective_y, reach_height);
        let in_flow = !candidate.context.is_out_of_flow_descendant();
        self.record_pages(control, end_page, in_flow)?;
        if step.consumes_pending_break {
            self.pending_break.clear();
        }
        self.pending_break.note_after(
            document,
            cascade,
            env.body_id,
            node_id,
            candidate.context.deferred_named_break_after(),
            // A text node forces no break after it, so no clip holds one.
            || false,
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
        // The holder's place in the flow, without the offsets that only paint
        // it elsewhere.
        let top = current_abs_y(document, holder)
            - accumulated_relative_offset_y(document, env.cascade, env.body_id, holder);
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

    /// The block-start margin edge of the box `node_id` whose border box
    /// starts at `border_top` in the flow below its block-start margin
    /// `margin_top`, and the edge up to which content of no height before it
    /// counts as content above it (see [`content_end_above`]): the border edge
    /// when its margin can adjoin that content (see
    /// [`margins_adjoin_in_flow`]), the margin edge otherwise.
    fn content_edges(
        &self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        node_id: usize,
        border_top: f32,
        margin_top: f32,
    ) -> (f32, f32) {
        let margin_edge = border_top - margin_top.max(0.0);
        if margins_adjoin_in_flow(document, env.body_id, node_id) {
            (margin_edge, border_top)
        } else {
            (margin_edge, margin_edge)
        }
    }

    /// Whether in-flow content ends above the box `candidate` whose
    /// block-start margin edge, at `margin_edge` in the flow, lies on the
    /// page the flow starts on, not at its end, and whose border box starts
    /// at `border_top` (see [`content_end_above`]).
    /// The box is in the normal flow: one out of it or floated has no forced
    /// break of the flow.
    fn follows_content_at_flow_start(
        &self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        (margin_edge, border_top): (f32, f32),
    ) -> bool {
        let first_page = env.page_origins.page_index_in_flow(0.0).unwrap_or(0);
        margin_edge < env.page_origins.origin(first_page.saturating_add(1)) - 0.001
            && stacks_in_flow(document, env.cascade, candidate)
            && content_end_above(
                document,
                env.cascade,
                env.body_id,
                candidate.node_id,
                margin_edge,
                border_top,
            )
            .is_some()
    }

    /// The last page the flow has reached before the box `node_id` at
    /// `flow_y`: the last page a box of the flow reached, or the page of
    /// `flow_y` when generated content before the box runs onto it (see
    /// [`generated_content_runs_past`]), as it places no box there.
    fn flow_reach(
        &self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        node_id: usize,
        flow_y: f32,
    ) -> u32 {
        let flow_max = self.reach.flow_max;
        match env.page_origins.page_index_in_flow(flow_y) {
            Some(page)
                if page > flow_max
                    && generated_content_runs_past(
                        document,
                        env.cascade,
                        env.body_id,
                        node_id,
                        env.page_origins.origin(page),
                    ) =>
            {
                page
            }
            _ => flow_max,
        }
    }

    /// The page that the box `node_id` at `flow_y` in the flow fits on: the
    /// page of that position once the flow has reached it (see
    /// [`Self::flow_reach`]), and the current page before that.
    fn flow_page(
        &self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        node_id: usize,
        flow_y: f32,
    ) -> u32 {
        let reach = self.flow_reach(document, env, node_id, flow_y);
        env.page_origins
            .page_index_in_flow(flow_y)
            .filter(|&page| page > self.current_page && reach >= page)
            .unwrap_or(self.current_page)
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
        let page = self.flow_page(document, env, candidate.node_id, flow_y);
        let start = env.page_origins.origin(page);
        let delta = row.content_shift(flow_y, start, start + env.page_origins.step_at(page))?;
        if delta <= 0.001 {
            return None;
        }
        // Every recorded in-flow item follows the deferred first row, including
        // later rows. The container's leading edges keep their original position.
        let items: Vec<usize> = if document.nodes[candidate.node_id].display()
            == raikiri_style::property::DisplayValue::Flex
        {
            super::grid_rows::flex_items(document, candidate.node_id)?
        } else {
            document.nodes[candidate.node_id]
                .grid_item_rows
                .iter()
                .map(|row| row.item)
                .collect()
        };
        for item in items {
            let top = current_abs_y(document, item);
            materialize_y(document, item, top + delta);
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
            // A line taller than the next page moves too, as Chromium moves
            // it, but only past content before it: a line that starts the
            // flow, below nothing but margins, has no break before it, and
            // runs over the pages from where it is (css/printing
            // fragmented-inline-block-002).
            && (candidate.context.leading_block_piece_end().is_some()
                || block_fits_page_step(
                    height,
                    env.page_origins.step_at(natural.saturating_add(1)),
                )
                || follows_flow_content(document, env.cascade, env.body_id, candidate.node_id))
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
            self.move_delegated_line(document, env, entry, owner, shift, target);
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

        let moved = self.break_before_unplaced_line(document, env, candidate, entry)?;
        // Sibling adjacency uses the original unpaginated bottom. Only page
        // reach and trailing-edge placement need the grown grid's or
        // paragraph's height.
        let candidate = grown.as_ref().or(moved.as_ref()).unwrap_or(candidate);
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

    /// Breaks the page between the padding and border above the line of a
    /// paragraph of one line that no text places (see `unplaced_line`), or
    /// the floats that put the line down, and that line, where it starts
    /// below the top of its page and runs past its end. Chromium moves the
    /// line to the top of the next page and leaves the edges above it, as for
    /// a line of text, even when the line is taller than that page. The
    /// paragraph must run past the page end itself: a box that ends on its
    /// page has no break inside it, and a line that overflows it stays. A box
    /// that no page break goes through keeps its line too. Returns the
    /// paragraph grown by the move, when it moved.
    fn break_before_unplaced_line(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
    ) -> Result<Option<PageCandidate>, Halt> {
        let node_id = candidate.node_id;
        if candidate.context.unplaced_line().is_none() || candidate.context.is_unbreakable() {
            return Ok(None);
        }
        let owner = super::leading_inline::LeadingInlineLine {
            root: node_id,
            line: 0,
        };
        let (top, height) = super::leading_inline::bounds(document, owner);
        let relative = accumulated_relative_offset_y(document, env.cascade, env.body_id, node_id);
        let flow_top = top - relative;
        let box_top = current_abs_y(document, node_id) - relative;
        let page_origins = env.page_origins;
        let page = page_origins.page_index_for_y(flow_top + 0.001);
        let next = page.saturating_add(1);
        if flow_top - box_top <= 0.5
            || !runs_past_page_end(
                document,
                env.cascade,
                env.body_id,
                node_id,
                page_origins.origin(next),
            )
            || flow_top <= page_origins.origin(page) + 0.001
            || flow_top + height <= page_origins.origin(next) + 0.001
        {
            return Ok(None);
        }
        reach_page(env.control, next)?;
        self.move_delegated_line(
            document,
            env,
            entry,
            owner,
            page_origins.origin(next) - flow_top,
            next,
        );
        let mut grown = candidate.clone();
        grown.height = document.nodes[node_id].layout.size.height;
        Ok(Some(grown))
    }

    /// Moves the line `owner` and the lines after it down by `shift` to
    /// `page`, and keeps the split for the shifts that settle later.
    fn move_delegated_line(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        entry: &CandidateEntry,
        owner: super::leading_inline::LeadingInlineLine,
        shift: f32,
        page: u32,
    ) {
        let growth = super::leading_inline::move_line(
            document,
            env.cascade,
            env.body_id,
            owner,
            shift,
            &mut self.subtree_shifts,
            (entry.page_before_candidate, page),
        );
        self.deferred_shifts.line_splits.push(LineSplit {
            root: owner.root,
            first_moved: owner.line,
            shift,
            growth,
            page,
        });
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
            root_flow_offset,
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
        // A truncated margin inside the box reaches no page of its own: the
        // content and the edges around it reach the pages the box runs on to.
        let reach_height = height - self.margin_rooms.get(&node_id).copied().unwrap_or(0.0);
        // A clip around the box that keeps its place hides what the box
        // paints past it, also when pagination moved the box down inside it:
        // that part reaches no page.
        let reach_height = candidate.context.clip_box().map_or(reach_height, |owner| {
            let clip_end =
                super::moved_clip_end(document, cascade, env.body_id, owner, root_flow_offset);
            reach_height.min(clip_end - effective_y)
        });
        let end_page = page_origins
            .end_page(effective_y, reach_height)
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
            env.body_id,
            node_id,
            candidate.context.deferred_named_break_after(),
            || super::clip_holds_breaks(document, env, node_id, false, effective_y + height),
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
    /// by the room left on the page, and the flow after it follows. A page
    /// edge inside the margin above the edges truncates that margin first
    /// (see [`Self::truncate_margin_above_edges`]).
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
        self.subtree_shifts.take_left_later_item();
        // The flow inside a block that its set height or `min-height` holds
        // moved further than the end of its content box, which keeps its
        // place: the part the block holds is left out. Leaving a paragraph
        // from outside it gave that part back already.
        let held = if outside == node_id {
            self.subtree_shifts.held_by(node_id)
        } else {
            0.0
        };
        let end = self.truncate_margin_above_edges(
            document,
            env,
            edges_break,
            edges_break.raw_y + self.flow_shift - held,
        )?;
        let page = page_origins.page_index_for_end(end);
        let page_end = page_origins.origin(page) + page_origins.step_at(page);
        // The content must end below the top of its page, and the edges must
        // reach past the bottom of it from a content end short of that. An end
        // that is not a number fails every comparison and leaves the edges be.
        let gap = page_end - end;
        // An empty box that ends the flow below the end of the page, where a
        // negative margin below it pulls the content end back up, takes the
        // edges after it to the next page even where they fit, as in
        // Chromium.
        if !(end > page_origins.origin(page) + 0.001
            && gap > 0.001
            && (edges_break.edges > gap + 0.001 || edges_break.margin_reach > gap + 0.001))
        {
            // The pages of a block leave out the margins in it that a page
            // edge may truncate, so edges that stay where they are reach
            // their pages here, such as a page that they begin.
            if self.margin_rooms.contains_key(&node_id) {
                let visible = self.visible_edges(edges_break, 0.0);
                if visible > 0.001 {
                    self.record_pages(control, page_origins.end_page(end, visible), true)?;
                }
            }
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
        // The item of a flex or grid container that holds the block, and
        // where its margin box ends in the container.
        let beside = edges_break.beside.map(|container| {
            let item = item_of(document, container, node_id).unwrap_or(node_id);
            (container, item, item_bottom_in(document, container, item))
        });
        document.nodes[node_id].layout.size.height += gap;
        document.mark_pagination_extent_dirty(node_id);
        if let Some((container, item, before)) = beside {
            // The boxes in the item grow and follow the edges, but the items
            // beside it stay where they are: the shift that reaches the item
            // ends with it.
            let (_, passed) = grow_ancestors_below(
                document,
                cascade,
                node_id,
                gap,
                body_id,
                &mut self.subtree_shifts,
                page,
                &IfcBoxShifts::default(),
                Some(container),
                false,
            );
            self.flow_shift += gap;
            if passed > 0.001 {
                self.subtree_shifts.bounded.push((item, passed, page));
            }
            let after = item_bottom_in(document, container, item);
            grow_container_beside_item(
                document,
                cascade,
                body_id,
                container,
                item,
                (before, after),
                &mut self.subtree_shifts,
                (page, next_page),
            );
        } else {
            grow_ancestors_for_shift(
                document,
                cascade,
                node_id,
                gap,
                body_id,
                &mut self.subtree_shifts,
                page,
                &IfcBoxShifts::default(),
            );
            self.flow_shift += gap;
        }
        // Bounds and float extents can absorb part of the gap. Record each
        // box's actual growth rather than subtracting the raw gap everywhere.
        for (owner, height) in heights {
            let growth = document.nodes[owner].layout.size.height - height;
            if growth > 0.001 {
                *self.edge_growth.entry(owner).or_default() += growth;
            }
        }
        self.current_page = self.current_page.max(next_page);
        if visible > 0.001 {
            self.record_pages(control, page_origins.end_page(page_end, visible), true)?;
        }
        Ok(())
    }

    /// Moves the flow after `candidate`, an empty box after a forced break,
    /// by `kept`, the part of its margin that the layout did not leave it,
    /// up to an ancestor whose height does not grow with it.
    fn follow_kept_margin(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        kept: f32,
        flow_page: u32,
    ) {
        if kept.abs() <= 0.001 {
            return;
        }
        self.flow_shift += kept;
        if !candidate.context.is_direct_body_element() {
            super::shift_block_flow_after(
                document,
                env.cascade,
                candidate.node_id,
                env.body_id,
                kept,
                &mut self.subtree_shifts,
                flow_page,
            );
        }
    }

    /// Ends the content box of the block `node_id` at `end`, where the flow
    /// inside it ends, when its last box kept a margin below it that the
    /// flow follows: an empty box after a forced break (see
    /// `forced_empty_box_shift`) or a box that splits the margin (see
    /// `split_box_kept_margin`). The flow after the block follows that
    /// margin, and so does the block's own height, within its min-height
    /// and max-height.
    fn end_block_at_kept_margin(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        node_id: usize,
        end: f32,
    ) {
        let cascade = env.cascade;
        let Some(last) = last_flow_box(document, cascade, node_id) else {
            return;
        };
        if !document.block_axis(node_id).height().is_auto()
            || (self.sibling_margins.forced_break_box != Some(last)
                && !super::splits_margin(document, cascade, last))
        {
            return;
        }
        let layout = &document.nodes[node_id].layout;
        let content_end = current_abs_y(document, node_id) + layout.size.height
            - layout.padding.bottom
            - layout.border.bottom;
        let (min_height, max_height) = border_box_height_bounds(document, node_id);
        let height = (layout.size.height + end - content_end)
            .min(max_height)
            .max(min_height);
        if (height - layout.size.height).abs() > 0.001 {
            document.nodes[node_id].layout.size.height = height;
            document.mark_pagination_extent_dirty(node_id);
        }
        self.kept_margin_ends.push(node_id);
    }

    /// Truncates the margin between the last box of the block of
    /// `edges_break` and its edges, which end the content at `end` in the
    /// flow, when a page edge falls inside it (CSS Fragmentation 3 §5.2):
    /// the edges begin the page at that edge, as a box after the margin
    /// would. The block shrinks by the part of the margin below the edge, as
    /// far as its min-height lets it, and the flow after it moves up, and
    /// the flow is on the page the edges begin. Returns where the content
    /// now ends.
    fn truncate_margin_above_edges(
        &mut self,
        document: &mut LayoutSession<'_>,
        env: &FlowEnv<'_>,
        edges_break: &EdgesBreak,
        end: f32,
    ) -> Result<f32, Halt> {
        self.end_block_at_kept_margin(document, env, edges_break.node_id, end);
        if edges_break.margin_above <= 0.001 {
            return Ok(end);
        }
        let page_origins = env.page_origins;
        let margin_start = end - edges_break.margin_above;
        let mut edge_page = page_origins.first_edge_in_margin(margin_start, false, u32::MAX);
        let node_id = edges_break.node_id;
        // An empty box that a forced break took to the top of a page keeps
        // its margins whole below the page top (see
        // `forced_empty_box_shift`), up to the next page edge.
        if let Some(forced) = self.sibling_margins.forced_break_box
            && let Some(page_top) = self.sibling_margins.forced_page_top
            && last_flow_box(document, env.cascade, node_id) == Some(forced)
            && page_origins.origin(edge_page) <= page_top + 0.001
        {
            edge_page = page_origins.page_index_for_y(page_top) + 1;
        }
        let edge = page_origins.origin(edge_page);
        if end <= edge + 0.001 {
            return Ok(end);
        }
        // Cuts at the edges of blocks inside this one shrank it already, so
        // the cut stops where the block reaches its min-height now.
        let (min_height, _) = border_box_height_bounds(document, node_id);
        let room = (document.nodes[node_id].layout.size.height - min_height).max(0.0);
        // The floats it holds keep the margin down to where they lie now,
        // after the breaks before moved them.
        let kept = last_flow_box(document, env.cascade, node_id).map_or(0.0, |last| {
            margin_kept_by_floats(
                document,
                env.cascade,
                env.body_id,
                node_id,
                last,
                edges_break.margin_above,
            )
        });
        let delta = (edge - end).max(-room).max(kept - edges_break.margin_above);
        if delta >= -0.001 {
            return Ok(end);
        }
        let new_end = end + delta;
        let start_page = if new_end <= edge + 0.001 {
            edge_page
        } else {
            page_origins.page_index_for_y(new_end)
        };
        reach_page(env.control, start_page)?;
        document.nodes[node_id].layout.size.height += delta;
        document.mark_pagination_extent_dirty(node_id);
        let held = shrink_ancestors_after_content_change(
            document,
            env.cascade,
            node_id,
            delta,
            delta,
            env.body_id,
            &IfcBoxShifts::default(),
            None,
        );
        self.flow_shift += delta;
        *self.margin_cuts.entry(node_id).or_default() -= delta;
        // An ancestor whose height does not shrink with its content keeps
        // the flow after it where it was.
        self.subtree_shifts.bounded.extend(
            held.into_iter()
                .map(|(ancestor, held)| (ancestor, held, start_page)),
        );
        self.current_page = self.current_page.max(start_page);
        let edges_end = page_origins.end_page(new_end, edges_break.edges);
        self.record_pages(env.control, edges_end, true)?;
        Ok(new_end)
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
        document: &mut LayoutSession<'_>,
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
                let kept = self.sibling_margins.pass_on_forced_break(
                    document,
                    env.cascade,
                    parent_id,
                    node_id,
                    page_break.collapses_with_previous_block && page_break.margins_collapse_through,
                );
                self.follow_kept_margin(document, env, candidate, kept, self.current_page);
            }
            self.sibling_margins.record_bottom(
                parent_id,
                node_id,
                page_break.collapses_with_previous_block,
                flow_raw_y(document, env.cascade, env.body_id, candidate) + candidate.height,
            );
        }
    }

    /// Starts the box `candidate` on a page: the first box of the flow, with
    /// no content above it, starts on the first page, and a later one, or one
    /// below content that places no box of the flow, opens the page its
    /// transition asks for, or starts on the page its position falls on.
    /// Returns whether the box met a page transition.
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
        let (margin_edge, border_top) = self.content_edges(
            document,
            env,
            node_id,
            position.effective_y - page_break.relative_offset,
            page_break.margin_top,
        );
        if !self.saw_child
            && let Some(end) = stacks_in_flow(document, env.cascade, candidate)
                .then(|| {
                    content_end_above(
                        document,
                        env.cascade,
                        env.body_id,
                        node_id,
                        margin_edge,
                        border_top,
                    )
                })
                .flatten()
        {
            // Content that places no box of the flow on a page, as generated
            // content, an empty box or the phantom line of an empty inline
            // element, still comes before the box, so the box is not the
            // first of the flow: a forced break before it opens the page
            // after the one that content ends on, and an unforced one cuts
            // the margin between them, as Chromium breaks there.
            self.saw_child = true;
            self.current_page = env.page_origins.page_index_for_end(end);
        }
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
            // A forced break before the first class-A box with nothing above
            // it does not manufacture a leading blank page. The box starts on the page its top falls
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
            // `floats_on_moved_lines`): it stays with them, and so does what
            // is inside it, up to its end.
            let moved_y = current_abs_y(document, node_id);
            let ahead = moved_y - (raw_y + self.flow_shift);
            if ahead > 0.001 {
                self.flow_shift += ahead;
                self.subtree_shifts
                    .bounded
                    .push((node_id, ahead, self.current_page));
            }
            moved_y
        } else {
            raw_y + self.flow_shift
        };
        // The other items of a flex line sit where the flow has put their
        // container, as the box does before it moves to `effective_y`.
        let line_y = current_abs_y(document, node_id);
        materialize_y(document, node_id, effective_y);
        let forced_before = page_break_is_forced(document, node_id, computed.break_before)
            && !super::clip_holds_breaks(document, env, node_id, true, effective_y);
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
        let flex_line = (forced_before || pending_break_applies || named_page_change)
            .then(|| super::row_flex_line(document, cascade, body_id, node_id, line_y))
            .flatten();
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
        let fit_page = self.flow_page(document, env, node_id, effective_y - relative_offset);
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
        // A row flex container or grid nested at the start of plain blocks
        // takes them along with its leading piece, as a column does with its
        // first replaced piece.
        let replaced_piece = candidate.context.leading_column_replaced_piece_end();
        let container_piece = (replaced_piece.is_none()
            && !candidate.context.is_direct_body_element())
        .then(|| {
            candidate.context.leading_piece_end().or_else(|| {
                candidate
                    .context
                    .first_grid_row()
                    .and_then(super::grid_rows::FirstRow::whole_end)
            })
        })
        .flatten();
        let column_piece = replaced_piece.or(container_piece);
        let column_crosses = column_piece.is_some_and(|end| {
            effective_y - relative_offset + end > fit_page_start + page_origins.step_at(fit_page)
        });
        let unbreakable_crosses = effective_y - relative_offset
            + candidate.context.unbreakable_extent(height)
            + trailing_margin
            > fit_page_start + page_origins.step_at(fit_page);
        // A paragraph of one unplaced line is kept whole by that line and the
        // edges that follow it (see `unplaced_line_extent`): it goes with the
        // blocks it begins, as a box that no break goes through does.
        let unplaced_crosses = candidate.context.unplaced_line().is_some_and(|line| {
            effective_y - relative_offset
                + super::unplaced_line_extent(document, node_id, line, height)
                > fit_page_start + page_origins.step_at(fit_page)
        });
        let holder = if (candidate.context.is_unbreakable() && unbreakable_crosses
            || unplaced_crosses
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
                        && (replaced_piece.is_some()
                            || lead
                                <= super::leading_space_between(document, cascade, node_id, holder)
                                    + 0.01)
                        && (lead <= 0.0
                            // A container goes on with its leading piece even
                            // when the piece is taller than the next page (see
                            // `leading_edges_move`), and the blocks it begins
                            // go along.
                            || container_piece.is_some()
                            // A paragraph of one unplaced line needs room for
                            // the line alone: the edges after it go on to the
                            // pages after it.
                            || lead
                                + column_piece
                                    .or(candidate
                                        .context
                                        .unplaced_line()
                                        .filter(|&line| {
                                            super::unplaced_line_ends_content(
                                                document, node_id, line, height,
                                            )
                                        })
                                        .map(|line| line.end_within(height)))
                                    .unwrap_or(height)
                                <= next_page_step + 0.001
                            || super::contained_cloned_leading_holder(
                                document, cascade, node_id, holder, lead,
                            ))
                },
            )
        } else {
            None
        };
        // A box that avoids a break inside it, at the start of the plain
        // blocks around it or below the padding or border they keep above
        // it, goes with them: the page does not break between their start and
        // the box (CSS Break 3 §4.1), so it is their margin box that tells
        // whether the box starts its page. That margin box reaches up over
        // every margin that collapses into the top of the outermost block,
        // also one that collapses up through it from a block inside. Where
        // they start the page, the box stays below that edge and breaks
        // inside, as browsers do, rather than leave it alone at the top of
        // the page. Only a box that runs past the page end moves, so the walk
        // over its ancestors is left to those.
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
            super::holder_below_edges(document, cascade, body_id, node_id)
                .filter(|&(_, lead)| effective_y - relative_offset - lead >= fit_page_start - 0.001)
                .map(|(holder, lead)| {
                    (
                        lead,
                        super::collapsed_margin_above(document, cascade, holder),
                    )
                })
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
            body_id,
            candidate,
            &fit,
            &self.table_headers,
            &mut self.trailing_flex_children,
        );
        // The only item of a grid that stacks it as block flow breaks between
        // the lines of its paragraphs as a block does, and does not move whole.
        if overflow.grid_item && !super::unplaced_item_moves(document, cascade, candidate, &fit) {
            overflow.grid_item = !holds_block_flow_paragraph(document, cascade, body_id, node_id);
        }
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
        // A container whose holder cannot go along moves alone, by the rules
        // for its own edges (see `leading_edges_move`).
        overflow.leading_piece_with_holder = replaced_piece
            .or(container_piece.filter(|_| holder.is_some()))
            .is_some_and(|end| {
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
        overflow.page |= overflow.leading_piece_with_holder;
        let grid_row = candidate.context.first_grid_row();
        // An empty box that its margins collapse through may stay with the
        // box before it.
        let stays_empty_once = std::cell::OnceCell::new();
        let stays_empty = || {
            *stays_empty_once.get_or_init(|| super::collapsed_empty_box(document, cascade, node_id))
        };
        let grid_content_break =
            grid_row.and_then(|row| row.content_shift(fit.flow_y, fit.page_start, fit.page_end));
        overflow.page |= grid_row.is_some_and(|row| {
            row.whole_moves(fit.flow_y, fit.margin_top, fit.page_start, fit.page_end)
        }) || (closing_edges_move(candidate, &fit)
            && !stays_empty()
            && super::capped_edges_follow(
                document,
                cascade,
                body_id,
                env.root_flow_offset,
                candidate,
                &fit,
            ))
            // A flex container goes on with its edges first; its line may then
            // break after them on the page it moves to.
            || ((grid_content_break.is_none()
                || document.nodes[node_id].display() == raikiri_style::property::DisplayValue::Flex)
                && leading_edges_move(candidate, &fit))
            || (run_tail_move(candidate, &fit)
                || super::min_height_edges_move(
                document,
                cascade,
                body_id,
                env.root_flow_offset,
                    candidate,
                    &fit,
                ))
                && !stays_empty();
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
                + collapsed_start_margin_gain(
                    document,
                    cascade,
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
        // The start of the flow is no page boundary: content above a box
        // there, even content of no height, as an empty box, generated
        // content or the phantom line of an empty inline element, stays on
        // the first page, so a forced break after it opens the next page, as
        // Chromium breaks there.
        let forced_break_at_page_start = forced_before
            && !self.follows_content_at_flow_start(
                document,
                env,
                candidate,
                self.content_edges(
                    document,
                    env,
                    node_id,
                    effective_y - relative_offset,
                    margin_top,
                ),
            )
            && if let Some(owner) = potential_forced_holder {
                let owner_margin_box_y = current_abs_y(document, owner.node)
                    - accumulated_relative_offset_y(document, cascade, body_id, owner.node)
                    - owner.margin_top;
                owner_margin_box_y <= page_start + 0.001
                    && self.child_positions.begins_page_below_leading_edges(
                        document, cascade, body_id, node_id, page_start,
                    )
            } else {
                // The break sits at the top of the margin box. A negative
                // margin pulls the border box above that point, over content
                // before it on this page, which the break still leaves
                // behind, or back over a page edge the break would share,
                // which the break then moves the box over.
                // The break before a flex item sits at the top of its line.
                let margin_box_y = effective_y
                    - relative_offset
                    - margin_top
                    - flex_line.as_ref().map_or(0.0, |line| line.offset);
                let starts_page = if margin_top < 0.0 {
                    (margin_box_y - page_start).abs() <= 0.001
                } else {
                    margin_box_starts_page(
                        page_origins,
                        margin_box_y,
                        self.current_page,
                        page_start,
                    )
                };
                starts_page
                    || self.child_positions.begins_page_below_leading_edges(
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
            flex_line,
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
        // the blocks that start where the box does move with it, and so do
        // those that begin a page below the margin they keep inside them
        // above a box that avoids a break (see `avoided_box_holder`).
        let (holder, holder_lead) = if let Some(owner) = forced_holder {
            (owner.node, owner.lead)
        } else if (overflow.unbreakable
            || overflow.moved_for_avoided_break
            || overflow.leading_piece_with_holder)
            && !forced_transition
            && is_block_flow_box(document, cascade, candidate)
            && !candidate.context.is_float_descendant()
            && !candidate.context.is_out_of_flow_descendant()
        {
            page_break
                .holder
                .filter(|_| overflow.unbreakable || overflow.leading_piece_with_holder)
                .unwrap_or_else(|| {
                    if !overflow.moved_for_avoided_break || overflow.unbreakable {
                        return leading_holder(document, cascade, body_id, node_id, false);
                    }
                    // The kept margin goes along even where the box then
                    // breaks inside on the next page, as in Chromium.
                    super::avoided_box_holder(
                        document,
                        cascade,
                        body_id,
                        node_id,
                        page_origins.origin(fit_page),
                    )
                })
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
        // The page the box falls on is the one its place in the flow is on.
        let natural_y = if forced_holder.is_some() {
            landing_y
        } else {
            position.effective_y - relative_offset
        };
        let natural_page = page_origins.transition_natural_page(
            natural_y,
            margin_top,
            self.current_page,
            self.flow_reach(document, env, node_id, natural_y),
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
        // A negative block-start margin can pull a forced-break box back
        // into the preceding page (the `underflow-from-next-page` case from
        // CSS Break). The break still opens its page, and the box lands at
        // the pulled-up coordinate with the flow after it.
        // A carried holder lands from its flow position, and a float that a
        // forced break moves resumes below the boundary its offset keeps (see
        // `float_resumption`): neither lands with the offset.
        let landing_relative = if forced_holder.is_some()
            || (lands_after_forced_break
                && is_floating_box_for_pagination(document, cascade, node_id))
        {
            0.0
        } else {
            relative_offset
        };
        let row_tail_move = overflow.page && candidate.context.item_tail_edges() > 0.0;
        // A forced break moves the top of the item's flex line to the top of
        // the page, and the item keeps its place in the line below that top,
        // as alignment and the margins of the items set it.
        let line_offset = page_break
            .flex_line
            .as_ref()
            .filter(|_| lands_after_forced_break)
            .map_or(0.0, |line| line.offset);
        let landing = TransitionLanding::new(
            page_origins.origin(target_page) + line_offset,
            landing_y,
            margin_top,
            landing_relative,
            lands_after_forced_break,
            overflow,
        );
        if forced_transition {
            self.sibling_margins.forced_margins = None;
        }
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
            let tail_line = row_tail_move
                .then(|| row_flex_container(document, cascade, node_id))
                .flatten()
                .map(|parent| {
                    let before_y = current_abs_y(document, node_id);
                    (
                        before_y,
                        super::flex_line_peers(document, cascade, body_id, parent, node_id),
                    )
                });
            let moved_in_bounded_ancestor = move_box_for_transition(
                document,
                cascade,
                body_id,
                candidate,
                &TransitionMove {
                    node_delta: landing.node_delta,
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
            // After a forced break the line moves as far as the box's kept
            // margin box; else as far as its border box, whose margin the
            // break truncates.
            if let Some((before_y, peers)) = tail_line {
                let delta = current_abs_y(document, node_id) - before_y;
                super::place_flex_line_peers(document, &peers, delta);
            } else if let Some(line) = &page_break.flex_line {
                let kept = lands_after_forced_break;
                let kept_margin = if kept { margin_top } else { 0.0 };
                super::place_flex_line_peers(
                    document,
                    &line.peers,
                    landing.shift_delta + kept_margin,
                );
            }
            // A kept margin moves the box's descendants and its later
            // siblings along with the box, a negative one too: the flow
            // after a box it pulls back over the break follows the box, as
            // in Chromium.
            self.flow_shift += landing.node_delta;
            if lands_after_forced_break && margins_collapse_through {
                let kept = self.sibling_margins.forced_empty_box_shift(
                    document,
                    cascade,
                    node_id,
                    flow_raw_y(document, cascade, body_id, candidate),
                    landing.target_y - landing_relative,
                    (
                        page_origins.origin(target_page),
                        page_origins.origin(target_page.saturating_add(1)),
                    ),
                );
                self.follow_kept_margin(
                    document,
                    env,
                    candidate,
                    kept,
                    entry.page_before_candidate,
                );
            }
            if !lands_after_forced_break && (overflow.page || margin_pushes_box_on) {
                self.flow_shift += super::split_box_kept_margin(document, cascade, node_id);
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
                    relative_offset: if forced_holder.is_some() {
                        relative_offset
                    } else {
                        landing_relative
                    },
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
            self.sibling_margins.forced_page_top = Some(page_origins.origin(target_page));

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
            self.sibling_margins.forced_margins = None;
            self.sibling_margins.forced_page_top = Some(page_origins.origin(natural_page));
            position.starts_after_forced_break = true;
        }
        // An unforced break that falls inside the collapsed margin
        // between this box and its preceding sibling truncates that
        // margin (CSS Fragmentation 3 §5.2): the box starts at the
        // top of the page instead of keeping the part of the margin
        // that crossed the page edge.
        let at = MarginBreak {
            node_id,
            body_id,
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
        let truncation = self
            .sibling_margins
            .truncation_at_break(document, cascade, page_origins, &at)
            .or_else(|| {
                // A float that took a forced break holds the margin after it
                // on its page (see `truncation_at_break`).
                self.sibling_margins
                    .break_taken_by_float
                    .is_none()
                    .then(|| {
                        truncation_after_generated_content(
                            document,
                            cascade,
                            body_id,
                            page_origins,
                            &at,
                        )
                    })
                    .flatten()
            });
        if let Some(MarginTruncation { delta, underflows }) = truncation {
            self.sibling_margins.moved_boxes |= delta != 0.0;
            let held = move_box_by_truncated_margin(document, cascade, node_id, delta, body_id);
            self.flow_shift += delta;
            position.effective_y += delta;
            // A negative margin pulls the box back over a forced break, which
            // still opens its page. Otherwise a pulled empty box stands where
            // the box after it lands, which is the page the flow reaches.
            if !underflows {
                natural_page = page_origins.page_index_for_y(flow_y + delta - pulled);
            }
            // An ancestor whose height does not shrink with its content keeps
            // the flow after it where it was: the part of the move it held
            // ends with its subtree.
            self.subtree_shifts.bounded.extend(
                held.into_iter()
                    .map(|(ancestor, held)| (ancestor, held, natural_page)),
            );
        }
        // A box that the margins inside it took to the top of a page keeps
        // its bottom margin whole below it there (see
        // `split_box_kept_margin`).
        if !forced_before
            && self.sibling_margins.forced_break_box.is_none()
            && natural_page > 0
            && (position.effective_y - relative_offset - page_origins.origin(natural_page)).abs()
                <= 0.001
            && super::margin_above_page_edge(document, cascade, node_id)
        {
            self.flow_shift += super::split_box_kept_margin(document, cascade, node_id);
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

    /// Takes the flow to the page the boxes of the paragraph that holds
    /// `candidate` start on. A paragraph before it in the same block whose
    /// last line started the next page exactly leaves the flow on the page
    /// before, where nothing of this paragraph is: neither its lines nor the
    /// top edges of the boxes it begins. Its lines are judged from the page
    /// after, as a box that starts there is.
    fn reach_paragraph_page(
        &mut self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        blocks: &LineBlocks,
        text: &TextPosition,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            body_id,
            control,
            page_origins,
            root_flow_offset,
            ..
        } = *env;
        let page_end =
            page_origins.origin(self.current_page) + page_origins.step_at(self.current_page);
        let Some(block_id) = blocks
            .block
            .map(|(block, _)| block)
            .or(blocks.flex_line.map(|(root, _)| root))
            .or(blocks.column_paragraph)
            .filter(|_| {
                text.effective_y >= page_end - 0.001
                    && !entry.uses_positioned_coordinates
                    && !candidate.context.is_float_descendant()
                    && !candidate.context.is_out_of_flow_descendant()
            })
        else {
            return Ok(());
        };
        let block_top = block_effective_y(
            document,
            block_id,
            candidate.raw_y,
            text.effective_y,
            root_flow_offset,
            self.flow_shift,
        );
        // The boxes that start with the paragraph and their top edges come
        // before its lines; a paragraph in the body starts nothing but itself.
        let holder = first_box_ancestor_at_start(
            document,
            cascade,
            block_id,
            body_id,
            page_origins.origin(self.current_page),
        );
        let lead = lead_below_ancestor(document, cascade, body_id, block_id, holder);
        let page = page_origins.page_index_for_y(block_top - lead + 0.001);
        if page > self.current_page {
            reach_page(control, page)?;
            self.current_page = page;
        }
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
                self.float_shifts
                    .record_grown_floats(document, cascade, body_id, candidate, block_id, delta);
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
    /// orphans and content comes before it on a page it does not start, even
    /// when its first line is taller than a page, or when it has one line and
    /// its own padding and border below that line cross the page end;
    /// otherwise it keeps its natural break. A
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
            clip_edges,
            ..
        } = *env;
        let node_id = candidate.node_id;
        if let Some((block_id, nested)) = blocks
            .block
            .or(blocks.flex_line.map(|(root, _)| (root, true)))
            .or(blocks.column_paragraph.map(|root| (root, true)))
            && self.checked_paragraphs.orphans_widows.insert(block_id)
            && let Some(mut all_line_metrics) = paragraph_line_bounds(document, node_id, text_lines)
        {
            // The lines on either side of a block child of the paragraph
            // break apart from those on the other side. A block after them,
            // not the lines, is what the edges below follow.
            let run = blocks
                .paragraph_root
                .filter(|&root| root == block_id)
                .and_then(|root| line_run_among_blocks(document, text_lines, node_id, root));
            let block_follows_run = run.as_ref().is_some_and(|run| !run.ends_paragraph);
            let block_precedes_run = run.as_ref().is_some_and(|run| run.after_block);
            if let Some(run) = run {
                all_line_metrics = all_line_metrics.get(run.lines).unwrap_or_default().to_vec();
            }
            let clip = clip_edges.get(&block_id);
            let clipped = |document: &LayoutSession<'_>, effective_y: f32| {
                clipped_line_metrics(
                    document,
                    page_origins,
                    root_flow_offset,
                    clip,
                    effective_y,
                    &all_line_metrics,
                    cascade.computed[block_id].widows,
                )
            };
            let (mut line_metrics, mut widows) = clipped(document, text.effective_y);
            if line_metrics.is_empty() {
                return Ok(());
            }
            // The lines of a run before the first block child of the
            // paragraph move down inside it as a run after a block child does,
            // the blocks after them following. A run after a block child is
            // moved by `keep_lines_after_block`, which counts its lines from
            // the paragraph's first; the moves here count them from the run's.
            let lines_can_move = self.lines_can_move(document, cascade, blocks, block_id)
                || (block_follows_run
                    && !block_precedes_run
                    && self.deferred_shifts.line_splits.is_empty()
                    && lines_can_shift_between_blocks(document, cascade, block_id));
            // The break that keeps the last lines with the padding and border
            // that close the boxes around the paragraph, on `page`.
            let closing_edges = if block_follows_run {
                0.0
            } else {
                candidate.context.lines_closing_edges()
            };
            // The room that floats leave at the top of the paragraph, above
            // its first line: the content box holds it, but no line does.
            let float_lead = {
                let layout = &document.nodes[block_id].layout;
                let content_top = block_effective_y(
                    document,
                    block_id,
                    candidate.raw_y,
                    text.effective_y - text.first_line_shift,
                    root_flow_offset,
                    self.flow_shift,
                ) + layout.border.top
                    + layout.padding.top;
                let gap = text.effective_y + line_metrics[0].0 - content_top;
                if floats_put_first_line_down(document, block_id, text_lines.get(node_id), gap) {
                    gap
                } else {
                    0.0
                }
            };
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
                    (content - float_lead - lines).abs() > 0.5
                        || lead + layout.border.top + layout.padding.top + lines + closing_edges
                            <= page_origins.step_at(self.current_page) + 0.001
                };
            let closing_edges_break =
                |line_metrics: &[(f32, f32)], widows: i32, page: u32, effective_y: f32| {
                    if avoided_whole {
                        return None;
                    }
                    closing_edges_line_move(
                        line_metrics,
                        closing_edges,
                        effective_y,
                        page,
                        cascade.computed[block_id].orphans.max(1) as usize,
                        widows.max(1) as usize,
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
                if let Some(moved) =
                    closing_edges_break(&line_metrics, widows, self.current_page, text.effective_y)
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
            }
            let block_height = document.nodes[block_id].layout.size.height;
            // The padding and border of the paragraph below its last line.
            // They follow the line only when the lines, below any room that
            // floats leave above the first of them, are all of the content of
            // the box: a block laid out after the last line, such as a
            // block-level `::after`, or a height taller than the lines, is
            // what they follow instead, also when a set height ends the box
            // at its lines and the block overflows it. Half a pixel absorbs
            // the rounding of the line boxes.
            let own_closing_edges = if blocks.paragraph_root == Some(block_id) {
                let layout = &document.nodes[block_id].layout;
                let edges = layout.padding.bottom + layout.border.bottom;
                let content = block_height - edges - layout.padding.top - layout.border.top;
                let lines = line_metrics[line_metrics.len() - 1].1 - line_metrics[0].0;
                if (content - float_lead - lines).abs() <= 0.5
                    && !block_after_last_line(document, block_id)
                {
                    edges
                } else {
                    0.0
                }
            } else {
                0.0
            };
            // A paragraph whose `min-height` leaves room below its last line:
            // where its content box ends below its top, and the padding and
            // border below it. Once the lines move down past that end, the
            // paragraph grows with them, and the edges follow the last line,
            // as in Chromium, while a `max-height` lets the paragraph grow
            // that far; past it, the edges stay at the paragraph's end.
            let min_height_room = (blocks.paragraph_root == Some(block_id)
                && !block_follows_run
                && own_closing_edges <= 0.0
                && lines_can_move
                && lines_move_grows_box(document, block_id)
                && !height_follows_content(document, block_id)
                && !block_after_last_line(document, block_id))
            .then(|| {
                let layout = &document.nodes[block_id].layout;
                let edges = layout.padding.bottom + layout.border.bottom;
                let max_end = super::border_box_height_bounds(document, block_id).1 - edges;
                (block_height - edges, max_end, edges)
            })
            .filter(|&(_, _, edges)| edges > 0.0);
            // A box that keeps its height ends where it ends, wherever its
            // lines go: its edges follow the lines only while they all fit
            // the page, before any of them moves.
            let edges_follow_moved_lines = !lines_can_move
                || own_closing_edges <= 0.0
                || lines_move_grows_box(document, block_id);
            // The page breaks between a block child of the paragraph and the
            // lines after it, rather than before the paragraph, when those
            // lines can move down inside it: they break as a block of their own
            // (see `keep_lines_after_block`).
            let breaks_after_block = block_precedes_run
                && self.breaks_run_after_block(
                    document,
                    env,
                    candidate,
                    entry,
                    block_id,
                    &all_line_metrics,
                );
            let mut page = self.current_page;
            // A block moved whole to the next page is judged once more from
            // there, for the lines that break again on the pages after it.
            let mut moved_whole = false;
            // Lines a clip hides after a move are left where they are.
            while !line_metrics.is_empty() {
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
                    widows,
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
                // takes content before it on a page it does not start, even
                // when its first line is taller than a page. Edges above the
                // lines that end this page move it whatever its height: the
                // page breaks before the block, not between its edges and its
                // lines, even when they run over a page.
                // A paragraph of one line moves whole, whatever its height,
                // when its own edges cross the page end (see
                // `lone_line_moves_with_edges` below).
                // The page already begins with the holder's leading edges,
                // after nothing but empty boxes. Keep a fitting first line
                // instead of breaking before the first child again to satisfy
                // orphans.
                let leading_page_lines = lines_can_move
                    && line_fit.lines_before_break > 0
                    && self.child_positions.gains_nothing_from_next_page(
                        document, cascade, body_id, block_id, page_start,
                    );
                let retained_margin_opens_page = self
                    .retained_margin_starts
                    .get(&block_id)
                    .is_some_and(|&(opening_page, margin)| {
                        opening_page == page
                            && (block_effective_y - margin - page_start).abs() <= 0.001
                    });
                // A kept margin already consumes the opening break. Preserve
                // a line that fits instead of breaking before that margin again.
                let keep_first_fragment =
                    retained_margin_opens_page && lines_can_move && line_fit.lines_before_break > 0;
                // A paragraph of one line has no break between its line and
                // the padding and border below it. When they cross the page
                // end, Chromium breaks before the paragraph, as before a box
                // that cannot break inside, even when the paragraph is taller
                // than a page: its edges then go on to the pages after it.
                let follows_content = || {
                    block_effective_y > page_start + 0.001
                        && follows_flow_content(document, cascade, body_id, block_id)
                };
                line_fit.lone_line_moves_with_edges = line_fit.total_lines == 1
                    && own_closing_edges > 0.0
                    && line_fit.edges_overflow
                    && follows_content();
                // A paragraph with padding or border above its first line,
                // and nothing but margins before it on the page, has no break
                // before it, unless those edges cross the page end
                // themselves: the page breaks between the edges and the line
                // instead (see `follows_box_on_page`).
                let can_move_block = !(moved_whole
                    || leading_page_lines
                    || keep_first_fragment
                    || breaks_after_block)
                    && blocks.block.is_some()
                    && (block_fits_page_step(block_height, page_step)
                        || line_fit.top_edges_end_page
                        || line_fit.lone_line_moves_with_edges
                        || (line_fit.leaves_too_few_orphans() && follows_content()))
                    && (first_line_top - block_effective_y <= 0.5
                        || first_line_top > page_start + page_step + 0.001
                        || follows_box_on_page(document, cascade, body_id, block_id, page_start));
                // A block that stays on its page, below the padding and
                // border of its own box or of the boxes it begins, or below
                // the floats that put its first line down, where they leave
                // the line no room before the page end, breaks between them
                // and the line, as Chromium does: the line starts the page
                // after. Below its own edges or the floats it does so even
                // when it is taller than that page; below the edges of the
                // boxes it begins only when it fits there. The lines after it
                // are judged again from there, as a later fragment, as they
                // may break on the pages after it in turn. A block that ends
                // on its page has no break inside it: a line that overflows
                // it stays.
                if !can_move_block
                    && lines_can_move
                    && line_fit.lines_before_break == 0
                    && first_line_top > page_start + 0.001
                    && first_line_top < page_start + page_step - 0.001
                    && runs_past_page_end(
                        document,
                        cascade,
                        body_id,
                        block_id,
                        page_start + page_step,
                    )
                    && (first_line_top - block_effective_y > 0.5
                        || block_fits_page_step(
                            line_metrics[0].1 - line_metrics[0].0,
                            page_origins.step_at(page.saturating_add(1)),
                        ))
                {
                    page = page.saturating_add(1);
                    let shift = page_origins.origin(page) - first_line_top;
                    let moved = self.move_lines(
                        document,
                        env,
                        candidate,
                        entry,
                        blocks,
                        text,
                        block_id,
                        [(0, shift, page)],
                    );
                    moved?;
                    // A clip that stays where it was hides more of the
                    // moved lines.
                    if clip.is_some() {
                        (line_metrics, widows) = clipped(document, text.effective_y);
                    }
                    moved_whole = true;
                    continue;
                }
                // A first line that crosses the end of the page has no break
                // before it inside the box it starts, nor before the boxes
                // that start with that box: when one of them ends on the
                // page, the page cannot break inside it, so the line
                // overflows the box and stays, as in Chromium. The lines
                // after it go on below it and break on the pages after it as
                // a later fragment does. Only a second line left alone under
                // `orphans` moves the paragraph, as the break before it
                // would.
                if !moved_whole
                    && line_fit.starts_on_page
                    && line_fit.lines_before_break == 0
                    && (line_fit.total_lines == 1 || line_fit.orphans <= 1)
                    && starting_box_ends_on_page(
                        document,
                        cascade,
                        body_id,
                        block_id,
                        block_effective_y,
                        page_start,
                        page_start + page_step,
                    )
                {
                    // The pieces of the lines start at the first, which no
                    // move takes: every later line that crosses a page end
                    // starts the page after it.
                    line_fit.first_fragment = false;
                    line_fit.tail_edges = 0.0;
                    let moves = if lines_can_move {
                        paragraph_line_moves(
                            &line_metrics,
                            text.effective_y,
                            page,
                            &line_fit,
                            page_origins,
                        )
                    } else {
                        Vec::new()
                    };
                    let moved = self.move_lines(
                        document, env, candidate, entry, blocks, text, block_id, moves,
                    );
                    moved?;
                    break;
                }
                let mut fix = line_fit.fix(
                    &line_metrics,
                    text.effective_y,
                    page,
                    page_origins,
                    can_move_block,
                    || lines_can_move,
                );
                // Moves that take the last line down past the room that
                // `min-height` left below it grow the paragraph: its edges then
                // follow that line, which the moves must keep with them. The
                // room is measured from where the block and its lines are now.
                // A break between the lines leaves no edge overflow to
                // remeasure: only the moves read the edges.
                if let (Some((content_end, max_end, edges)), LineBreakFix::MoveLines(moves)) =
                    (min_height_room, &fix)
                    && line_fit.total_lines > 1
                    && let moved_end = text.effective_y
                        + line_metrics[line_metrics.len() - 1].1
                        + moves.iter().map(|&(_, delta, _)| delta).sum::<f32>()
                    && moved_end >= block_effective_y + content_end - 0.5
                    && moved_end <= block_effective_y + max_end + 0.5
                {
                    line_fit.tail_edges = edges;
                    fix = line_fit.fix(
                        &line_metrics,
                        text.effective_y,
                        page,
                        page_origins,
                        can_move_block,
                        || lines_can_move,
                    );
                }
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
                        // The line is measured where the flow puts it, so the
                        // shift leaves its relative offset as it is.
                        // A block that moves to keep its orphans, with a line
                        // that fits beside or below the floats, takes them
                        // along.
                        let below_floats = (lines_can_move
                            && line_fit.lines_before_break == 0
                            && floats_put_first_line_down(
                                document,
                                block_id,
                                text_lines.get(node_id),
                                first_line_top - (block_effective_y + block_edges),
                            ))
                        .then(|| page_origins.origin(page) - first_line_top)
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
                            self.float_shifts.record_grown_floats(
                                document, cascade, body_id, candidate, block_id, delta,
                            );
                        }
                        moved_whole = true;
                        // A clip that stays where it was hides more of the
                        // moved lines.
                        if clip.is_some() {
                            (line_metrics, widows) = clipped(document, text.effective_y);
                        }
                    }
                    LineBreakFix::Keep => {
                        // A paragraph moved whole to a page it fits there may
                        // leave the padding and border that close the boxes
                        // around it behind its lines, as on the page it left.
                        if moved_whole
                            && lines_can_move
                            && let Some(moved) =
                                closing_edges_break(&line_metrics, widows, page, text.effective_y)
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

    /// Whether the run of lines `metrics` (block-start and block-end edges of
    /// each line) after a block child of the paragraph `root` breaks as a
    /// block of its own (see [`Self::keep_lines_after_block`]): the lines can
    /// move down inside the paragraph, which is in the flow that the moved
    /// lines push down, and no gap lies between two of them.
    fn breaks_run_after_block(
        &self,
        document: &LayoutSession<'_>,
        env: &FlowEnv<'_>,
        candidate: &PageCandidate,
        entry: &CandidateEntry,
        root: usize,
        metrics: &[(f32, f32)],
    ) -> bool {
        // Text placed by positioned insets, or inside an out-of-flow box, is
        // not in the flow that the moved lines push down. A paragraph inside
        // a float grows the float, as one in the flow grows its block; inside
        // a float, only floats may stand between the paragraph and plain
        // block flow.
        let in_flow = !entry.uses_positioned_coordinates
            && !candidate.context.is_out_of_flow_descendant()
            && (!candidate.context.is_float_descendant()
                || super::is_float_flow_paragraph(document, env.cascade, root, env.body_id));
        // A float that a line cannot sit beside, or a clearance below one,
        // leaves a gap of at least a fraction of a pixel between two lines of
        // the run: they do not break as one block of lines, and the gap is not
        // theirs to move.
        in_flow
            && self.deferred_shifts.line_splits.is_empty()
            && lines_can_shift_between_blocks(document, env.cascade, root)
            && !metrics.windows(2).any(|pair| pair[1].0 - pair[0].1 > 0.5)
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
        let Some(root) = blocks.paragraph_root else {
            return Ok(());
        };
        let Some(run) = lines_between_blocks(document, text_lines, node_id, root) else {
            return Ok(());
        };
        let run_metrics = |document: &LayoutSession<'_>| {
            paragraph_line_bounds(document, node_id, text_lines)
                .and_then(|all| all.get(run.clone()).map(<[_]>::to_vec))
                .filter(|metrics| !metrics.is_empty())
        };
        // The first candidate on the run judges it for all of them.
        if !self
            .checked_paragraphs
            .lines_after_blocks
            .insert((root, run.start))
        {
            return Ok(());
        }
        let Some(mut metrics) = run_metrics(document).filter(|metrics| {
            self.breaks_run_after_block(document, env, candidate, entry, root, metrics)
        }) else {
            return Ok(());
        };
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
                    || (run_top > page_start + 0.001 && fit.leaves_too_few_orphans()));
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
                        page: move_page,
                    },
                    &mut self.subtree_shifts,
                    entry.page_before_candidate,
                );
                self.float_shifts
                    .record_grown_floats(document, cascade, body_id, candidate, root, growth);
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
        owner_heights: &HashMap<usize, f32>,
    ) -> Result<(), Halt> {
        let FlowEnv {
            cascade,
            control,
            page_origins,
            text_lines,
            clip_edges,
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
            // The floats among the lines break between their own lines in
            // the walk of the box's flow.
            let holds_floats = document
                .ifc_layout_node(root)
                .and_then(|node| node.ifc.as_ref())
                .is_some_and(|ifc| ifc.boxes.iter().any(|b| b.kind == super::IfcBoxKind::Float));
            if holds_floats || !lines_can_shift_inside(document, cascade, root) {
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
            let fresh_edge = reflowed
                .then(|| super::positioned_flow::clip_bounds(document, env, root).lines_end);
            if let Some(edge) = fresh_edge.or_else(|| clip_edges.get(&root).map(|clip| clip.edge)) {
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
                            page,
                        },
                        None,
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
                    page,
                },
                &mut self.subtree_shifts,
                entry.page_before_candidate,
            );
            self.float_shifts
                .record_grown_floats(document, cascade, body_id, candidate, block_id, growth);
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
                // The break before the first text of a flex line moves the
                // top of the line to the top of the page, and the text keeps
                // its place in the line below that top.
                // An offset of an inline box around the text only paints it
                // lower in its line.
                let line_offset = self
                    .pending_break
                    .source
                    .filter(|_| consumes_pending_break)
                    .zip(candidate.ifc_root)
                    .and_then(|(source, (root, offset))| {
                        let inline_offset = relative_offset
                            - accumulated_relative_offset_y(
                                document,
                                env.cascade,
                                env.body_id,
                                root,
                            );
                        super::text_offset_in_flex_line(
                            document,
                            env.cascade,
                            env.body_id,
                            source,
                            root,
                            offset - inline_offset,
                        )
                    })
                    .unwrap_or(0.0);
                let flow_top = text.effective_y - relative_offset - line_offset;
                let natural_page = page_origins
                    .page_index_in_flow(flow_top)
                    .unwrap_or(self.current_page);
                let target_page = forced_target_page(self.current_page, natural_page);
                reach_page(control, target_page)?;
                let target_y = page_origins.origin(target_page);
                // `place_text` already materialized the candidate at
                // `effective_y`, so this is the additional movement for this
                // boundary.
                // The same amount advances `flow_shift`, which also moves
                // later candidates whose ancestor is not moved here. The text
                // starts the page where the flow puts it, and its relative
                // offset paints it that far from there.
                let delta = target_y - flow_top;
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
                // Text that a clip hides whole starts no page.
                && text.effective_y < text_clip_end(document, env, candidate) - 0.001
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
        // Text in a float that follows lines moved inside it is on their page
        // or after it too.
        if let Some(page) = reached_page
            && !candidate.context.is_out_of_flow_descendant()
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
        // The flow goes back to the page of a box that the items moved down
        // before it left in place, beside them.
        if self.subtree_shifts.take_left_later_item() && in_flow {
            let page = page_origins.page_index_for_y(candidate.raw_y + self.flow_shift);
            self.current_page = self.current_page.min(page);
        }
        // A box that moved below a float grown inside another float starts
        // on the page it moved to, after the page the float began on.
        if self.subtree_shifts.take_reached_moved_box() {
            let page = page_origins.page_index_for_y(candidate.raw_y + self.flow_shift);
            self.current_page = self.current_page.max(page);
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

/// Whether `node_id` is or holds a paragraph whose lines the paragraph line
/// pass breaks between as it does in plain block flow (see
/// [`super::is_block_flow_paragraph`]). A paragraph inside a float counts too
/// (see [`super::is_float_flow_paragraph`]), unless the box also holds lines
/// that pass does not break, which then move the box whole.
fn holds_block_flow_paragraph(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    let mut float_paragraph = false;
    let mut other_lines = false;
    let mut stack = vec![node_id];
    while let Some(node) = stack.pop() {
        if document.nodes[node].is_ifc_root() {
            if super::is_block_flow_paragraph(document, cascade, node, body_id) {
                return true;
            }
            if super::is_float_flow_paragraph(document, cascade, node, body_id) {
                float_paragraph = true;
            } else {
                other_lines = true;
            }
        }
        stack.extend_from_slice(&document.nodes[node].children);
    }
    float_paragraph && !other_lines
}

/// Whether `candidate` lies below the in-flow content before it: a box in
/// the normal flow (see [`is_block_flow_box`]), or an item of a flex container
/// whose items stack in the block axis in their order, a single column that
/// is not reversed, as the items before it lie above it; and not inside a
/// float or an out-of-flow box.
fn stacks_in_flow(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    candidate: &PageCandidate,
) -> bool {
    let stacked_flex_item = || {
        candidate.context.is_flex_item()
            && crate::layout::box_parent(
                document,
                cascade,
                candidate.node_id,
                crate::layout::BoxParentScope::Document,
            )
            .is_some_and(|parent| {
                let container = document.physical_style(parent);
                !container.flex_direction_is_row()
                    && !container.flex_direction_is_reversed()
                    && !container.flex_wraps()
            })
    };
    (is_block_flow_box(document, cascade, candidate) || stacked_flex_item())
        && !candidate.context.is_float_descendant()
        && !candidate.context.is_out_of_flow_descendant()
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

/// The end of the nearest clip in the flow around the lines of the text of
/// `candidate`, where its box is now, in the coordinates of the candidates
/// with the relative offsets that paint the text: the clip stays with its box
/// while pagination moves the lines inside it, and hides what they paint past
/// it. Infinite when no such clip holds the lines.
fn text_clip_end(
    document: &LayoutSession<'_>,
    env: &FlowEnv<'_>,
    candidate: &PageCandidate,
) -> f32 {
    candidate
        .ifc_root
        .and_then(|(root, _)| env.clip_edges.get(&root))
        .and_then(|clip| moved_line_clip_end(document, clip, env.root_flow_offset))
        .unwrap_or(f32::INFINITY)
}

/// The nearest end of the boxes in the flow whose clip holds the lines of
/// `clip`, where they are now, in the coordinates of the candidates. Each
/// one's end moves with it; the lines inside it do not move it.
fn moved_line_clip_end(
    document: &LayoutSession<'_>,
    clip: &super::candidates::LineClip,
    root_flow_offset: f32,
) -> Option<f32> {
    clip.owners
        .iter()
        .map(|&owner| current_abs_y(document, owner) + document.nodes[owner].layout.size.height)
        .reduce(f32::min)
        .map(|end| end + root_flow_offset)
}

/// The lines of `all_line_metrics`, relative to the text at `effective_y`,
/// that pagination may move, with the widows they keep. A clip hides the lines past its edge: they never move, and a page
/// that the box ends on, at or above the edge, has no break inside the box for
/// a line running over it. Chromium keeps no widows once lines run past the
/// clip. The edge moves with the box that clips, not with lines moved down
/// inside that box.
#[allow(clippy::too_many_arguments)]
fn clipped_line_metrics(
    document: &LayoutSession<'_>,
    page_origins: &PageOrigins,
    root_flow_offset: f32,
    clip: Option<&super::candidates::LineClip>,
    effective_y: f32,
    all_line_metrics: &[(f32, f32)],
    widows: i32,
) -> (Vec<(f32, f32)>, i32) {
    let mut line_metrics = all_line_metrics.to_vec();
    // Only boxes in the flow hold the lines that pagination moves here.
    let Some(end) = clip.and_then(|clip| moved_line_clip_end(document, clip, root_flow_offset))
    else {
        return (line_metrics, widows);
    };
    let visible = end - effective_y;
    let widows = if line_metrics
        .last()
        .is_some_and(|&(_, bottom)| bottom > visible + 0.001)
    {
        1
    } else {
        widows
    };
    line_metrics.truncate(line_metrics.partition_point(|&(top, _)| top < visible - 0.001));
    if let Some(&(top, bottom)) = line_metrics.last()
        && bottom > visible + 0.001
    {
        let page = page_origins.page_index_for_y(effective_y + top);
        let page_end = page_origins.origin(page) + page_origins.step_at(page);
        if effective_y + visible <= page_end + 0.001 {
            line_metrics.pop();
        }
    }
    (line_metrics, widows)
}

/// Whether the block `block_id`, whose border box the flow puts at
/// `block_top` now, or one of the boxes that start
/// with it and so have no break between their top and it, ends by
/// `page_end`: the page cannot break inside such a box. A box whose height
/// follows its content holds the block and ends below it; the others keep the
/// height they have and move with the block, their first box. Positions are
/// those of the flow, without relative offsets, which only paint the boxes
/// elsewhere.
fn starting_box_ends_on_page(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    block_id: usize,
    block_top: f32,
    page_start: f32,
    page_end: f32,
) -> bool {
    let flow_top = |id: usize| {
        current_abs_y(document, id) - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    // How far pagination has moved the block, and the boxes that start with
    // it, from where their layout put them.
    let shift = block_top - flow_top(block_id);
    let holder = first_box_ancestor_at_start(document, cascade, block_id, body_id, page_start);
    let chain = std::iter::once(block_id)
        .chain(
            document
                .layout_ancestors(block_id)
                .take_while(|&id| id != body_id),
        )
        // Up to the holder, the outermost box that starts with the block.
        .take_while({
            let mut reached = false;
            move |&id| !std::mem::replace(&mut reached, id == holder)
        })
        .collect::<Vec<_>>();
    !chain.iter().any(|&id| {
        empty_box_before_lies_past(document, cascade, id, |empty| {
            flow_top(empty) + shift >= page_end - 0.001
        })
    }) && chain.into_iter().any(|id| {
        (id == block_id
            || (!document.nodes[id].display().is_inline_level()
                && document.nodes[id].display() != raikiri_style::property::DisplayValue::Contents
                && !height_follows_content(document, id)))
            && flow_top(id) + shift + document.nodes[id].layout.size.height <= page_end + 0.001
    })
}

/// Whether one of the empty boxes right before `node_id` among its in-flow
/// siblings, whose margins collapse through them, lies where `past_end` says
/// the page has ended: the page breaks before that box, so the box after it
/// starts the next page.
fn empty_box_before_lies_past(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
    past_end: impl Fn(usize) -> bool,
) -> bool {
    document.parent_of(node_id).is_some_and(|parent| {
        let children = &document.nodes[parent].children;
        let before = children
            .iter()
            .position(|&child| child == node_id)
            .unwrap_or(children.len());
        let mut boxes = Vec::new();
        for &child in children[..before].iter().rev() {
            boxes.clear();
            push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
            for &empty in &boxes {
                if !collapses_through(
                    document,
                    cascade,
                    empty,
                    document.nodes[empty].layout.size.height,
                ) {
                    return false;
                }
                if past_end(empty) {
                    return true;
                }
            }
        }
        false
    })
}

/// Whether the block `node_id` and every block around it inside the body run
/// past `page_end`: a box that ends on the page has no break inside it, and
/// the content that overflows it, such as a line below its end, stays on the
/// page with it, as in Chromium.
fn runs_past_page_end(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
    page_end: f32,
) -> bool {
    std::iter::once(node_id)
        .chain(
            document
                .layout_ancestors(node_id)
                .take_while(|&id| id != body_id),
        )
        .filter(|&id| !document.nodes[id].display().is_inline_level())
        .all(|id| {
            current_abs_y(document, id)
                - accumulated_relative_offset_y(document, cascade, body_id, id)
                + document.nodes[id].layout.size.height
                > page_end + 0.001
        })
}
