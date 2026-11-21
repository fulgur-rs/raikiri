//! Collection of the page candidates of a laid-out flow: the boxes and
//! texts pagination may move to a later page, in source order.

use super::super::box_tree::BoxTree;
use super::super::context::LayoutSession;
use super::{
    AvoidChain, BoxParentScope, BreakBetween, CascadeResult, DisplayValue, Document,
    FlexDirectionValue, FloatValue, FormattingContext, LayoutError, NodeFlags, NodeKind,
    PageLayoutControl, PositionValue, TextLineIndex, UnplacedBlockLine,
    accumulated_relative_offset_y, avoid_chain_before_last_box, avoided_run_head,
    avoids_page_break_inside, block_after_last_line, box_is_unbreakable, boxes_are_tied,
    breaks_only_around, candidate_ancestor_constraints, clears_float_side, clears_in_cell_flow,
    closing_edges_of, container_item_tail, content_bottom_edges, edges_break_container,
    end_margin_through, first_named_item_start_page, first_owned_text_line_top, forces_page_break,
    has_block_leading_content, has_mandatory_break_inside, height_follows_content, holds_floats,
    in_inset_out_of_flow_subtree, is_block_level_flow_box, is_floating_box_for_pagination,
    is_flow_table_caption, is_monolithic_for_pagination_with_transposed_axes, is_plain_block_box,
    is_plain_container_box, is_plain_table_box, leading_holder, leading_piece_end,
    lineless_leading_piece_end, lone_box_with_floats, margin_above_edges, moves_later_siblings,
    moves_reach_cell_row, nested_run_units, normal_flow_bottom, normal_flow_y, page_applies_to_box,
    page_edge_constraint, page_name_to_owned, pagination_child_order, pagination_child_parent_y,
    paragraph_has_lines_to_split, paragraph_lines_keep_together, passes_closing_edges,
    positioned_owned_text_line_bounds, propagated_start_page_name_with_transposed_axes,
    push_in_flow_boxes_rev, run_paragraph, selected_page_name, unplaced_line,
    unplaced_line_after_block,
};
use std::collections::HashMap;

/// How much of the content of an absolutely positioned box that starts past
/// the edge of the clip that contains it is kept, in px: the box is hidden
/// whole, but the page it starts on stays.
const HIDDEN_BOX_SLIVER: f32 = 1.0;

/// The edge past which the lines of a paragraph stay where they are, in the
/// coordinates of the candidates, and, when a box in the flow gives it, every
/// clipping box in the flow around the paragraph whose height is not that of
/// its content, nearest first: each one's end moves with that box, and the
/// lines stop at the nearest end once pagination has moved them.
#[derive(Clone, Debug)]
pub(super) struct LineClip {
    pub(super) edge: f32,
    pub(super) owners: Vec<usize>,
}

/// Overflow edges inherited by normal and absolutely positioned descendants.
#[derive(Clone, Copy)]
pub(super) struct ClipBounds {
    pub(super) end: f32,
    pub(super) fixed_end: f32,
    pub(super) absolute_end: f32,
    /// `end`, but ending at the box of a clip whose margin reaches past it.
    pub(super) lines_end: f32,
    /// The box whose clip gives `lines_end`, with its top where it was
    /// collected, when a box in the flow does.
    pub(super) lines_owner: Option<(usize, f32)>,
    /// The nearest box in the flow whose clip ends the lines inside it at its
    /// own end, wherever pagination moves it.
    pub(super) lines_clip: Option<usize>,
}

impl ClipBounds {
    pub(super) fn unclipped() -> Self {
        Self {
            end: f32::INFINITY,
            fixed_end: f32::INFINITY,
            absolute_end: f32::INFINITY,
            lines_end: f32::INFINITY,
            lines_owner: None,
            lines_clip: None,
        }
    }

    pub(super) fn descend(
        self,
        document: &LayoutSession<'_>,
        cascade: &CascadeResult,
        node_id: usize,
        raw_y: f32,
        is_body: bool,
    ) -> Self {
        // Only a fixed clip that comes down the flow unchanged is known: an
        // absolutely positioned box has its own.
        let (inherited, inherited_fixed) = match document.nodes[node_id].position() {
            PositionValue::Absolute if raw_y < self.absolute_end => {
                (self.absolute_end, f32::INFINITY)
            }
            PositionValue::Absolute if self.absolute_end.is_finite() => {
                let layout = &document.nodes[node_id].layout;
                (
                    raw_y + layout.border.top + layout.padding.top + HIDDEN_BOX_SLIVER,
                    f32::INFINITY,
                )
            }
            PositionValue::Absolute | PositionValue::Fixed => (f32::INFINITY, f32::INFINITY),
            _ => (self.end, self.fixed_end),
        };
        let (inherited_lines, mut lines_clip) = match document.nodes[node_id].position() {
            PositionValue::Absolute | PositionValue::Fixed => ((inherited, None), None),
            _ => ((self.lines_end, self.lines_owner), self.lines_clip),
        };
        let (clip_end, fixed_clip_end, (lines_end, lines_owner)) =
            if !is_body && super::element_clips_overflow(document, cascade, node_id) {
                let own = super::clip_block_end(document, cascade, node_id, raw_y);
                // A clip on a box whose height follows its content grows
                // with what is pushed down inside it.
                let follows_content = height_follows_content(document, node_id);
                let own_fixed = if follows_content { f32::INFINITY } else { own };
                // Lines past the end of the box itself stay where they are,
                // even when a clip margin keeps them visible. A box whose
                // height follows its content grows with the lines that move
                // inside it, so its end hides none of them.
                let own_lines = if follows_content {
                    f32::INFINITY
                } else {
                    lines_clip = Some(node_id);
                    raw_y + document.nodes[node_id].layout.size.height
                };
                (
                    inherited.min(own),
                    inherited_fixed.min(own_fixed),
                    if own_lines < inherited_lines.0 {
                        (own_lines, Some((node_id, raw_y)))
                    } else {
                        inherited_lines
                    },
                )
            } else {
                (inherited, inherited_fixed, inherited_lines)
            };
        let absolute_clip_end = if clip_end != self.absolute_end
            && crate::layout::fixed::establishes_absolute_containing_block(
                document,
                node_id,
                &cascade.computed[node_id],
            ) {
            clip_end
        } else {
            self.absolute_end
        };
        Self {
            end: clip_end,
            fixed_end: fixed_clip_end,
            absolute_end: absolute_clip_end,
            lines_end,
            lines_owner,
            lines_clip,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct CandidateFlowContext {
    pub(super) is_float_descendant: bool,
    pub(super) is_out_of_flow_descendant: bool,
    /// Whether the candidate takes part in page-name transitions. Floats
    /// and out-of-flow subtrees are parallel flows whose names are
    /// ignored, except for a named box inside an absolutely positioned
    /// box left at its static position, which keeps its name in the page
    /// flow.
    pub(super) page_name_in_flow: bool,
    /// For the text of a paragraph that ends the blocks around it: the block
    /// size of the bottom padding and border that follow its last line, which
    /// would break away from it if the lines stayed on a page they do not fit
    /// (see [`super::closing_edges_line_move`]).
    pub(super) lines_closing_edges: f32,
}

/// Whether this source child takes up a place in its parent's in-flow box
/// sequence. This separate probe does not change the older adjacency state
/// when that state was not needed for a contents box.
fn column_child_has_flow_box(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    use super::sibling_adjacency::{FlowBox, flow_box};
    match flow_box(document, cascade, id) {
        FlowBox::Skip | FlowBox::Phantom => false,
        FlowBox::Box => true,
        FlowBox::Contents => document.nodes[id]
            .children
            .iter()
            .any(|&child| column_child_has_flow_box(document, cascade, child)),
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct ElementContext {
    /// The atomic-only line this candidate places instead of its own box.
    pub(super) leading_inline_line: Option<super::leading_inline::LeadingInlineLine>,
    pub(super) is_rendered_leaf: bool,
    pub(super) is_direct_body_element: bool,
    pub(super) is_table_row: bool,
    /// A caption of a block table whose lines no page break goes between: it
    /// moves to the next page whole when it would cross the page end.
    pub(super) is_table_caption: bool,
    pub(super) is_flex_item: bool,
    pub(super) is_grid_item: bool,
    pub(super) is_named: bool,
    /// The nearest box around the candidate whose height does not follow its
    /// content and that clips its overflow (see [`ClipBounds::lines_clip`]).
    /// Its clip hides what the candidate paints past it, wherever pagination
    /// moves the candidate inside it.
    pub(super) clip_box: Option<usize>,
    pub(super) deferred_named_break_after: bool,
    /// The line of a paragraph of one line of generated text or of atomic
    /// inlines that no text places: the box is kept whole by that line,
    /// unless a block child of the paragraph comes above it.
    pub(super) unplaced_line: Option<UnplacedBlockLine>,
    /// The box is replaced content, full size containment, or an atomic
    /// inline-level box that no page break goes through (see
    /// [`super::box_is_unbreakable`]).
    pub(super) unbreakable: bool,
    /// The block size of the part of an unbreakable box that lies inside the
    /// blocks it begins, when the rest of it is visible overflow of one of
    /// them (see [`unbreakable_room`]). Only that part decides whether the box
    /// crosses the page end: Chromium does not move overflow of a block that
    /// ends on the page, it cuts it.
    pub(super) unbreakable_room: Option<f32>,
    /// Content of its parent's flow comes before the box, and the page break
    /// between the two is avoided, by the box or by the box before it. A
    /// box that avoids a break inside it does not move past that break on
    /// its own: Chromium rates both breaks alike and takes the later one,
    /// inside the box (see [`BoxContext::avoids_break_before`]).
    pub(super) avoided_break_before: bool,
    /// The block size of the bottom padding and border that close the blocks
    /// around the box, which it ends: they would break away from it if it
    /// stayed on a page they do not fit (see [`super::closing_edges_move`]).
    pub(super) closing_edges: f32,
    /// For a block that holds one box: the block size at its end that follows
    /// that box, inside the block's own border box, so that the block moves to
    /// the next page whole when the box fits a page but this tail does not.
    pub(super) closing_edges_inside: f32,
    /// For the first box of a run of boxes tied by avoided breaks that ends
    /// its block: the block size from the bottom of this box to the bottom of
    /// the last box of the run, which the edges in `closing_edges` follow. The
    /// run moves to the next page as one when they do not fit after it.
    pub(super) closing_edges_span: f32,
    /// For the first box of such a run: the last box of the run, which the
    /// edges in `closing_edges` follow.
    /// Kept as `u32` to keep the context small.
    pub(super) closing_edges_last_box: Option<u32>,
    /// How far below the border-box top the run of boxes tied by avoided
    /// breaks ends that the box starts among the boxes of its parent, or that
    /// it holds at the start of its own boxes. The box moves to the next page
    /// with the run when the run would cross a page edge (see
    /// [`super::avoided_run_ends`]).
    pub(super) avoided_unit: Option<super::RunUnit>,
    /// For a flex container or grid with padding or border above its items:
    /// how far below the border-box top the first piece of its content ends.
    /// The box moves to the next page when that crosses a page edge, so that
    /// the edges do not stay behind alone (see [`super::leading_edges_move`]).
    pub(super) leading_piece_end: Option<f32>,
    pub(super) first_grid_row: Option<super::grid_rows::FirstRow>,
    /// For a block-like wrapper with edges above a box that has no line:
    /// how far below the border-box top the first piece of its content ends.
    /// The box moves to the next page when that crosses a page edge, whether
    /// or not the next page can hold the piece (see
    /// [`super::leading_edges_move`]).
    pub(super) leading_block_piece_end: Option<f32>,
    /// A replaced or size-contained piece at the leading edge of a nowrap column flex
    /// container. It can move the column without making the whole column
    /// monolithic or opening a page before the first box of the flow.
    pub(super) leading_column_replaced_piece_end: Option<f32>,
    /// For the last box of a run of boxes tied by avoided breaks that ends its
    /// block: the block size of the bottom padding and border that follow it.
    /// The box moves to the next page with them when the run is as far on as
    /// it goes and they do not fit after the box, the avoided break giving way
    /// (see [`super::run_tail_move`]).
    pub(super) run_tail_edges: f32,
    /// For the last item of a flex container that stacks its items, or the
    /// last row of a table: the block size of the bottom padding, border and
    /// margin that close the container and the blocks it ends, which follow
    /// the item. The box moves to the next page with them when they do not fit
    /// after it (see [`super::item_tail_move`]).
    pub(super) item_tail_edges: f32,
    /// For a block that holds one box (see `closing_edges_inside`) and is tied
    /// by an avoided break to the box before it: how far below the top of the
    /// margin box of the first box of the tied run the block's border box
    /// starts. The avoided break gives way, and the block moves to the next
    /// page, only when that first box starts its page (see
    /// [`super::closing_edges_move`]).
    pub(super) lone_run_head_offset: Option<f32>,
    /// How far below the end of the box that the closing edges follow the
    /// floats of the blocks they close reach: a float that crosses a page edge
    /// has a break to take of its own, and the box stays; one that does not
    /// moves along with the box (see [`super::closing_edges_move`]).
    pub(super) closing_edges_float_reach: f32,
    /// The block that the box ends, and the bottom padding and border that
    /// close it, when the block's `min-height` leaves room below the box: the
    /// edges follow the box only once pagination moved content in the block
    /// down far enough to grow it past that room (see
    /// [`super::min_height_edges_move`]).
    pub(super) min_height_edges: Option<(usize, f32)>,
    /// The block whose bottom padding and border `closing_edges` holds, when a
    /// `height` or `max-height` ends that block where it ends: the edges
    /// follow the box only while it ends inside the block (see
    /// [`super::capped_edges_follow`]).
    pub(super) capped_edges_block: Option<usize>,
}

#[derive(Clone, Copy)]
pub(super) enum BoxContext {
    Text(CandidateFlowContext),
    Element {
        flow: CandidateFlowContext,
        attributes: ElementContext,
    },
}

impl Default for BoxContext {
    fn default() -> Self {
        Self::Element {
            flow: CandidateFlowContext::default(),
            attributes: ElementContext::default(),
        }
    }
}

impl BoxContext {
    pub(super) fn text(flow: CandidateFlowContext) -> Self {
        Self::Text(flow)
    }

    pub(super) fn element(flow: CandidateFlowContext, attributes: ElementContext) -> Self {
        Self::Element { flow, attributes }
    }

    fn flow(self) -> CandidateFlowContext {
        match self {
            Self::Text(flow) | Self::Element { flow, .. } => flow,
        }
    }

    fn element_attributes(self) -> Option<ElementContext> {
        match self {
            Self::Text(_) => None,
            Self::Element { attributes, .. } => Some(attributes),
        }
    }

    pub(super) fn leading_inline_line(self) -> Option<super::leading_inline::LeadingInlineLine> {
        self.element_attributes()
            .and_then(|attributes| attributes.leading_inline_line)
    }

    pub(super) fn is_text(self) -> bool {
        matches!(self, Self::Text(_))
    }

    pub(super) fn is_rendered_leaf(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_rendered_leaf)
    }

    pub(super) fn clip_box(self) -> Option<usize> {
        self.element_attributes()
            .and_then(|attributes| attributes.clip_box)
    }

    pub(super) fn is_direct_body_element(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_direct_body_element)
    }

    pub(super) fn is_table_row(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_table_row)
    }

    pub(super) fn is_table_caption(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_table_caption)
    }

    pub(super) fn is_flex_item(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_flex_item)
    }

    pub(super) fn first_grid_row(self) -> Option<super::grid_rows::FirstRow> {
        self.element_attributes()
            .and_then(|attributes| attributes.first_grid_row)
    }

    pub(super) fn is_grid_item(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_grid_item)
    }

    /// The single unplaced line that keeps the box whole (see
    /// [`super::unplaced_line`]).
    pub(super) fn unplaced_line(self) -> Option<UnplacedBlockLine> {
        self.element_attributes()
            .and_then(|attributes| attributes.unplaced_line)
            .filter(|line| !line.after_block)
    }

    /// The single unplaced line of the paragraph laid out below a block child
    /// of it (see [`super::unplaced_line_after_block`]).
    pub(super) fn unplaced_line_after_block(self) -> Option<UnplacedBlockLine> {
        self.element_attributes()
            .and_then(|attributes| attributes.unplaced_line)
            .filter(|line| line.after_block)
    }

    /// Whether a page break is avoided between the box and content of its
    /// parent's flow before it (see [`ElementContext::avoided_break_before`]).
    pub(super) fn avoids_break_before(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.avoided_break_before)
    }

    pub(super) fn is_unbreakable(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.unbreakable)
    }

    /// The block size of an unbreakable box of `height` that decides whether
    /// it crosses the page end (see [`ElementContext::unbreakable_room`]).
    pub(super) fn unbreakable_extent(self, height: f32) -> f32 {
        self.element_attributes()
            .and_then(|attributes| attributes.unbreakable_room)
            .map_or(height, |room| room.min(height))
    }

    pub(super) fn closing_edges(self) -> f32 {
        self.element_attributes()
            .map_or(0.0, |attributes| attributes.closing_edges)
    }

    pub(super) fn closing_edges_inside(self) -> f32 {
        self.element_attributes()
            .map_or(0.0, |attributes| attributes.closing_edges_inside)
    }

    pub(super) fn closing_edges_span(self) -> f32 {
        self.element_attributes()
            .map_or(0.0, |attributes| attributes.closing_edges_span)
    }

    pub(super) fn closing_edges_last_box(self) -> Option<usize> {
        self.element_attributes()
            .and_then(|attributes| attributes.closing_edges_last_box)
            .map(|last| last as usize)
    }

    pub(super) fn avoided_unit(self) -> Option<super::RunUnit> {
        self.element_attributes()
            .and_then(|attributes| attributes.avoided_unit)
    }

    pub(super) fn leading_piece_end(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.leading_piece_end)
    }

    pub(super) fn leading_block_piece_end(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.leading_block_piece_end)
    }

    pub(super) fn leading_column_replaced_piece_end(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.leading_column_replaced_piece_end)
    }

    pub(super) fn run_tail_edges(self) -> f32 {
        self.element_attributes()
            .map_or(0.0, |attributes| attributes.run_tail_edges)
    }

    pub(super) fn item_tail_edges(self) -> f32 {
        self.element_attributes()
            .map_or(0.0, |attributes| attributes.item_tail_edges)
    }

    pub(super) fn lone_run_head_offset(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.lone_run_head_offset)
    }

    pub(super) fn closing_edges_float_reach(self) -> f32 {
        self.element_attributes()
            .map_or(0.0, |attributes| attributes.closing_edges_float_reach)
    }

    pub(super) fn capped_edges_block(self) -> Option<usize> {
        self.element_attributes()
            .and_then(|attributes| attributes.capped_edges_block)
    }

    pub(super) fn min_height_edges(self) -> Option<(usize, f32)> {
        self.element_attributes()
            .and_then(|attributes| attributes.min_height_edges)
    }

    pub(super) fn lines_closing_edges(self) -> f32 {
        self.flow().lines_closing_edges
    }

    pub(super) fn is_named(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_named)
    }

    pub(super) fn is_float_descendant(self) -> bool {
        self.flow().is_float_descendant
    }

    pub(super) fn is_out_of_flow_descendant(self) -> bool {
        self.flow().is_out_of_flow_descendant
    }

    pub(super) fn page_name_in_flow(self) -> bool {
        self.flow().page_name_in_flow
    }

    pub(super) fn deferred_named_break_after(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.deferred_named_break_after)
    }
}

#[derive(Clone)]
pub(super) struct PageCandidate {
    pub(super) node_id: usize,
    pub(super) raw_y: f32,
    pub(super) height: f32,
    pub(super) context: BoxContext,
    /// Page type inherited from the nearest containing class-A box.
    /// `None` is the anonymous page type, not an unknown value.
    pub(super) page_name: Option<String>,
    /// Column-flex descendants compare names within their containing flow.
    pub(super) named_flex_context: Option<usize>,
    /// For a text node of an ifc paragraph: the paragraph root and the
    /// text's offset below the root's border-box top. The root's lines
    /// are painted from the root, so the text moves by moving the root.
    pub(super) ifc_root: Option<(usize, f32)>,
}

/// Whether `node_id` or an ancestor is a fixed box, or an absolutely
/// positioned box with a non-auto block-start or block-end inset, which
/// moves it away from its static position in the page flow. A fixed box
/// repeats on every page, so it never joins the page flow.
fn can_descend_for_named_page(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    document.get_node(node_id).is_some_and(|node| {
        node.is_in_document()
            && !node.is_non_rendered_html_element()
            && document.nodes[node_id].display() != DisplayValue::None
            && !matches!(
                document.nodes[node_id].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            && !is_floating_box_for_pagination(document, cascade, node_id)
    })
}

fn named_page_subtree_has_name(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pagination_on_transposed_axes: bool,
    cache: &mut HashMap<usize, bool>,
) -> bool {
    #[cfg(test)]
    super::super::page::record_page_name_work(4);
    let mut pending = vec![(node_id, false)];
    while let Some((current_id, visited)) = pending.pop() {
        if cache.contains_key(&current_id) {
            continue;
        }
        if !can_descend_for_named_page(document, cascade, current_id) {
            cache.insert(current_id, false);
            continue;
        }
        if page_applies_to_box(document, current_id)
            && selected_page_name(cascade, current_id).is_some()
        {
            cache.insert(current_id, true);
            continue;
        }
        if is_monolithic_for_pagination_with_transposed_axes(
            document,
            cascade,
            current_id,
            pagination_on_transposed_axes,
        ) {
            cache.insert(current_id, false);
            continue;
        }
        let Some(node) = document.get_node(current_id) else {
            cache.insert(current_id, false);
            continue;
        };
        if visited {
            let found = node
                .children
                .iter()
                .any(|child_id| cache.get(child_id).copied().unwrap_or(false));
            cache.insert(current_id, found);
        } else {
            pending.push((current_id, true));
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .copied()
                    .map(|child_id| (child_id, false)),
            );
        }
    }
    cache.get(&node_id).copied().unwrap_or(false)
}

fn has_nested_named_page_descendant(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    depth: u32,
    pagination_on_transposed_axes: bool,
    cache: &mut HashMap<usize, bool>,
) -> bool {
    let Some(node) = document.get_node(node_id) else {
        return false;
    };
    let child_depth = depth.saturating_add(1);
    for &child_id in &node.children {
        if !can_descend_for_named_page(document, cascade, child_id) {
            continue;
        }
        if child_depth >= 2 {
            if named_page_subtree_has_name(
                document,
                cascade,
                child_id,
                pagination_on_transposed_axes,
                cache,
            ) {
                return true;
            }
            continue;
        }
        // The immediate child is below the nesting threshold, but its
        // descendants may still contain a page-name transition.
        if is_monolithic_for_pagination_with_transposed_axes(
            document,
            cascade,
            child_id,
            pagination_on_transposed_axes,
        ) {
            continue;
        }
        let Some(child) = document.get_node(child_id) else {
            continue;
        };
        if child.children.iter().any(|&grandchild_id| {
            named_page_subtree_has_name(
                document,
                cascade,
                grandchild_id,
                pagination_on_transposed_axes,
                cache,
            )
        }) {
            return true;
        }
    }
    false
}

/// A named box containing a hidden subtree still establishes an explicit
/// page boundary. Keep that boundary distinct from the zero-height named
/// runs that are otherwise coalesced at one source coordinate.
pub(super) fn has_display_none_descendant(document: &Document, node_id: usize) -> bool {
    let mut pending: Vec<_> = document
        .get_node(node_id)
        .into_iter()
        .flat_map(|node| node.children.iter().rev().copied())
        .collect();
    while let Some(current_id) = pending.pop() {
        let Some(node) = document.get_node(current_id) else {
            continue; // cov:ignore: document-owned child links are valid by construction.
        };
        if node.is_display_none() {
            return true;
        }
        pending.extend(node.children.iter().rev().copied());
    }
    false
}

/// The state a node inherits from its ancestors while its page
/// candidates are collected.
#[derive(Clone)]
struct CandidateFrame<'name> {
    /// The source-coordinate y the node's layout location is relative to:
    /// its layout parent's border-box y, or the paragraph root's for a node
    /// inside a paragraph laid out by the inline engine.
    parent_abs_y: f32,
    direct_body_child: bool,
    parent_height: f32,
    inherited_page_name: Option<&'name str>,
    inside_table: bool,
    flex_column_parent: bool,
    grid_single_column_parent: bool,
    inside_flex: bool,
    named_flex_context: Option<usize>,
    inside_float: bool,
    inside_out_of_flow: bool,
    /// The source-coordinate y at which the nearest ancestor that clips its
    /// block-axis overflow hides what its content paints, or infinity when no
    /// ancestor does. A text or box starting at or past it is never visible,
    /// so it takes no part in pagination.
    clip_end: f32,
    /// The same for the clips on boxes whose height is not that of their
    /// content, which stay where they are when what they hold grows, and
    /// infinity when no ancestor has one.
    fixed_clip_end: f32,
    /// What `clip_end` is for an absolutely positioned child: the edge past
    /// which the clips of its containing block and of the ancestors above
    /// that hide it, or infinity when none does. The clips of the boxes
    /// between the child and its containing block do not reach it.
    absolute_clip_end: f32,
    /// The edge past which the lines under the nearest clips stay where they
    /// are; see [`ClipBounds::lines_end`].
    lines_clip_end: f32,
    /// See [`ClipBounds::lines_owner`].
    lines_clip_owner: Option<(usize, f32)>,
    /// See [`ClipBounds::lines_clip`].
    lines_clip_box: Option<usize>,
    /// The block size of the bottom padding and border of the ancestors that
    /// end with the node: it is the last box in the flow of each of them, and
    /// their heights follow their content.
    closing_edges: f32,
    /// The parent and the bottom padding and border that close it, when the
    /// node is the last box in its flow and the parent's `min-height` leaves
    /// room below the node (see [`ElementContext::min_height_edges`]).
    min_height_edges: Option<(usize, f32)>,
    /// The block whose edges `closing_edges` holds, when its height does not
    /// follow its content (see [`ElementContext::capped_edges_block`]).
    capped_edges_block: Option<usize>,
    /// The box before the node in the flow of its parent, which a break can
    /// fall after, when the node ends the blocks that `closing_edges` counts.
    previous_box: Option<usize>,
    /// Whether any in-flow content of the parent comes before the node, text
    /// included, which `previous_box` leaves out.
    box_before: bool,
    /// A free boundary before the leading plain blocks of this node. Only
    /// the column's leading-replaced-piece rule, and the leading piece and
    /// first row of a row flex container or grid, inherit this boundary.
    column_break_before: bool,
    /// Whether no box comes before the node in its parent or in any block the
    /// node starts, up to the outermost block whose bottom padding or border
    /// follows the node: no page break before the node falls inside that
    /// block. Boxes that `tied_to_start` ties to the node do not count. It is
    /// read only where `closing_edges` follows the node, and the boxes before
    /// it are known only for the blocks on that path.
    lone_in_edges: bool,
    /// Whether avoided breaks tie the node to the box before it in its parent,
    /// and that box to the one before it in turn, all the way to the first box
    /// of the parent: no break between boxes is left to take before the node
    /// inside its parent. It is read only where `closing_edges` follows the
    /// node.
    tied_to_start: bool,
    /// The block size of the bottom padding and border that follow the last
    /// line of the paragraph the node is in or is the text of.
    lines_closing_edges: f32,
    /// Whether the node is a paragraph whose `closing_edges` are the boxes
    /// that avoided breaks tie to its last lines: it breaks between its lines
    /// for them, whatever ties it to the boxes before it.
    lines_tail: bool,
    /// How the node is tied to the boxes around it by avoided breaks, when it
    /// ends its block or starts the run of boxes that does.
    tie: AvoidTie,
    /// How far below its border-box top the run of boxes tied by avoided
    /// breaks ends that the node starts among the boxes of its parent.
    run_unit: Option<super::RunUnit>,
    /// Whether the node is the last box of a run that avoided breaks tie it
    /// to, from a free break on, and ends its block: `closing_edges` follows
    /// it.
    run_tail: bool,
    /// The last item of the flex container, or the last row of the table, that
    /// the node is in, when the edges that close the container follow it.
    item_tail: Option<ItemTail>,
    /// The box that starts the run of boxes tied by avoided breaks among the
    /// boxes of the node's parent, when an avoided break ties the node to the
    /// box before it: the run runs from this box on, with a break that is not
    /// avoided before it. It is tracked only where the document avoids breaks.
    tie_head: Option<usize>,
    /// The lowest margin-box bottom, in source coordinates, of the floats of
    /// the blocks that `closing_edges` counts, or negative infinity when they
    /// have none. They are read only where `closing_edges` follows the node.
    float_bottom: f32,
}

/// The last item of a flex container that stacks its items, or the last row of
/// a table, with the edges that follow it. An avoided final row group is
/// represented by its first row, including the rest of the group in the tail.
#[derive(Clone, Copy)]
struct ItemTail {
    /// The item, final row, or first row of an avoided final group.
    item: usize,
    /// The final row can split from a group whose closing tail still does not fit.
    fallback: Option<usize>,
    /// Where the border box of the container ends, in source coordinates.
    container_bottom: f32,
    /// The block size of the bottom padding and border of the blocks that the
    /// container ends, and of the margin below it.
    inherited: f32,
}

/// How a box is tied by avoided page breaks to the others of the run that ends
/// its block, with the edges of the block in `CandidateFrame::closing_edges`.
#[derive(Clone, Copy, Default, PartialEq)]
enum AvoidTie {
    #[default]
    None,
    /// The box starts the run, which moves to the next page with the edges
    /// after it when they do not fit: `span` is the block size from the bottom
    /// of the box to the bottom of the run, and `last` the run's last box.
    RunStart { span: f32, last: usize },
    /// The box ends a block whose boxes are all tied to the first of them, so
    /// that breaking before it is the only break left, the avoided one giving
    /// way.
    Waived,
}

impl CandidateFrame<'_> {
    /// The frame of the body: the root of candidate collection.
    fn root(parent_abs_y: f32) -> Self {
        Self {
            parent_abs_y,
            direct_body_child: false,
            parent_height: 0.0,
            inherited_page_name: None,
            inside_table: false,
            flex_column_parent: false,
            grid_single_column_parent: false,
            inside_flex: false,
            named_flex_context: None,
            inside_float: false,
            inside_out_of_flow: false,
            clip_end: f32::INFINITY,
            fixed_clip_end: f32::INFINITY,
            absolute_clip_end: f32::INFINITY,
            lines_clip_end: f32::INFINITY,
            lines_clip_owner: None,
            lines_clip_box: None,
            closing_edges: 0.0,
            min_height_edges: None,
            capped_edges_block: None,
            previous_box: None,
            box_before: false,
            column_break_before: false,
            lone_in_edges: true,
            tied_to_start: false,
            lines_closing_edges: 0.0,
            lines_tail: false,
            tie: AvoidTie::None,
            run_unit: None,
            run_tail: false,
            item_tail: None,
            tie_head: None,
            float_bottom: f32::NEG_INFINITY,
        }
    }
}

/// The source element and its main content sequence used by pagination.
/// The element supplies actual coordinates; the contents retain the existing
/// break-opportunity policies for the document's main flow.
#[derive(Clone, Copy)]
pub(super) struct FlowRoots {
    pub(super) element: usize,
    pub(super) contents: usize,
    pub(super) contents_box: usize,
}

impl FlowRoots {
    pub(super) fn new(document: &Document, element: usize) -> Self {
        let ordinary_root = matches!(
            document.nodes[element].formatting_context,
            FormattingContext::Block | FormattingContext::Inline
        );
        let contents = if ordinary_root {
            crate::layout::find_body(document).unwrap_or(element)
        } else {
            element
        };
        let contents_box = if document.nodes[contents].in_ifc_subtree()
            || document.nodes[contents].display() == DisplayValue::Contents
        {
            candidate_parent(document, contents).unwrap_or(element)
        } else {
            contents
        };
        Self {
            element,
            contents,
            contents_box,
        }
    }
}

/// The box from whose coordinates a candidate is positioned. Inline
/// ancestors of paragraph boxes and contents elements add no layout frame.
pub(super) fn candidate_parent(document: &Document, child: usize) -> Option<usize> {
    document
        .layout_ancestors(child)
        .find(|&parent| document.nodes[parent].display() != DisplayValue::Contents)
}

/// Whether the page may break before the block `node_id` after what comes
/// before it in a paragraph: the box around it mixes text and blocks into one
/// inline formatting context, so the walk keeps no box before it, but
/// something before it renders in that flow and does not tie the two.
fn breaks_after_paragraph_content(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    paragraph_content_before(document, cascade, node_id)
        .is_some_and(|before| !boxes_are_tied(document, cascade, before, node_id))
}

/// What renders last before `node_id` among the lines of the paragraph that
/// holds it: a sibling, or a sibling of the inline or `display: contents`
/// elements around it inside the paragraph.
fn paragraph_content_before(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<usize> {
    let paragraph = candidate_parent(document, node_id)
        .filter(|&parent| document.nodes[parent].is_ifc_root())?;
    let mut child = node_id;
    while child != paragraph {
        let parent = document.nodes[child].parent?;
        let children = &document.nodes[parent].children;
        let position = children.iter().position(|&sibling| sibling == child)?;
        if let Some(before) = children[..position]
            .iter()
            .rev()
            .copied()
            .find(|&sibling| renders_paragraph_content(document, cascade, sibling))
        {
            return Some(before);
        }
        child = parent;
    }
    None
}

/// Whether `node_id` puts something in the flow of a paragraph: text that
/// is not collapsed away, or an element with a box of its own. An inline or
/// `display: contents` element counts only through what it holds, and a
/// float or an out-of-flow box not at all.
fn renders_paragraph_content(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    match super::sibling_adjacency::flow_box(document, cascade, node_id) {
        super::sibling_adjacency::FlowBox::Skip | super::sibling_adjacency::FlowBox::Phantom => {
            false
        }
        super::sibling_adjacency::FlowBox::Box
            if node.kind() != NodeKind::Element || node.display() != DisplayValue::Inline =>
        {
            true
        }
        _ => node
            .children
            .iter()
            .any(|&child| renders_paragraph_content(document, cascade, child)),
    }
}

/// For each of `children`, its last box in the flow and whether it floats.
fn child_flow_flags(
    document: &Document,
    cascade: &CascadeResult,
    children: &[usize],
) -> Vec<(Option<usize>, bool)> {
    let mut boxes = Vec::new();
    children
        .iter()
        .map(|&child_id| {
            boxes.clear();
            push_in_flow_boxes_rev(document, cascade, child_id, &mut boxes);
            (
                boxes.first().copied(),
                is_floating_box_for_pagination(document, cascade, child_id),
            )
        })
        .collect()
}

/// The place in the flow where the bottom padding and border of a block begin,
/// below its content: the page may break there, between the content and the
/// edges, when they do not fit after it.
#[derive(Clone, Copy)]
pub(super) struct EdgesBreak {
    /// The block whose edges follow.
    pub(super) node_id: usize,
    /// How many candidates come before the break in the order they are placed.
    pub(super) before: usize,
    /// Where the content of the block ends, in the coordinates of the
    /// candidates.
    pub(super) raw_y: f32,
    /// The block size of the bottom padding and border.
    pub(super) edges: f32,
    /// The block size of the margin between the last box in the flow of the
    /// block and the edges (see [`margin_above_edges`]).
    pub(super) margin_above: f32,
    /// The block size of the top of that margin that a float the block
    /// holds keeps from a cut.
    pub(super) margin_kept: f32,
    /// How far below the end of the content the last of the empty boxes that
    /// end the flow lies, when a negative margin ends the content above it
    /// (see [`margin_above_edges`]).
    pub(super) margin_reach: f32,
    /// The end of the clip around the block, in the coordinates of the
    /// candidates, when that clip keeps its place as the edges are pushed
    /// down, and infinity when none does. It names the clip.
    pub(super) clip_end: f32,
    /// How much room the clip leaves below the edges where they are, in the
    /// coordinates of the candidates; infinity without a clip that holds.
    pub(super) clip_room: f32,
    /// The flex or grid container whose items sit beside the one that holds
    /// the block, if any: the edges then move that item and the container,
    /// not the items beside it.
    pub(super) beside: Option<usize>,
}

/// Collects the page candidates of a laid-out subtree. Page membership is
/// decided from source coordinates before local page offsets, so the
/// recursion carries each node's inherited state in a [`CandidateFrame`].
pub(super) struct CandidateCollector<'a, 'doc, 'c> {
    document: &'a LayoutSession<'doc>,
    cascade: &'a CascadeResult,
    roots: FlowRoots,
    page_step: f32,
    out: &'a mut Vec<PageCandidate>,
    text_lines: &'a mut TextLineIndex,
    /// For the paragraphs whose text a clip cuts, by paragraph root: the edge
    /// of the clip, or the end of the clipping box when the clip's margin
    /// reaches past it, in the coordinates of the candidates.
    clip_edges: HashMap<usize, LineClip>,
    /// For each box of [`ClipBounds::lines_clip`], the next one around it.
    enclosing_line_clips: HashMap<usize, Option<usize>>,
    control: &'a PageLayoutControl<'c>,
    named_page_subtree_cache: HashMap<usize, bool>,
    /// Whether each flex container looked at holds a sole table that
    /// fragments as block flow does, so that its items are not scanned again
    /// for each of them.
    sole_table_holders: HashMap<usize, bool>,
    /// The selected inner tail of each plain block on its column-item path.
    inner_tail_paths: HashMap<usize, usize>,
    /// The runs of boxes tied by avoided breaks that start inside their first
    /// box, keyed by the box they start at (see `nested_run_units`).
    run_breaks: HashMap<usize, super::RunUnit>,
    run_break_starts: super::RunBreakStarts,
    has_named_pages: bool,
    /// Whether runs of boxes tied by avoided breaks may be looked for inside
    /// blocks: some box avoids a break before or after it, and no box names a
    /// page, which would break the flow between boxes of its own.
    finds_nested_runs: bool,
    /// Whether some box forces a break after itself, so that a box after one
    /// may take it.
    forces_break_after: bool,
    /// Where the edges of the blocks collected begin below their content.
    edges_breaks: Vec<EdgesBreak>,
    /// For each parent, the last in-flow block among the children seen so
    /// far that is not a candidate, with its unpaginated border-box bottom
    /// without its own relative offset, until a box that takes space follows.
    pending_bottoms: HashMap<usize, (usize, f32)>,
    /// For each candidate right after such a block, past empty boxes, that
    /// block and its bottom: the margin above the candidate starts there.
    preceding_bottoms: PrecedingBottoms,
}

/// For each candidate right after an in-flow block that is not a candidate,
/// that block and its unpaginated border-box bottom.
pub(super) type PrecedingBottoms = HashMap<usize, (usize, f32)>;

impl<'a, 'doc, 'c> CandidateCollector<'a, 'doc, 'c> {
    pub(super) fn new(
        document: &'a LayoutSession<'doc>,
        cascade: &'a CascadeResult,
        roots: FlowRoots,
        page_step: f32,
        out: &'a mut Vec<PageCandidate>,
        text_lines: &'a mut TextLineIndex,
        control: &'a PageLayoutControl<'c>,
    ) -> Self {
        let has_named_pages = super::super::page::has_named_pages(cascade);
        Self {
            document,
            cascade,
            roots,
            page_step,
            out,
            text_lines,
            clip_edges: HashMap::new(),
            enclosing_line_clips: HashMap::new(),
            control,
            named_page_subtree_cache: HashMap::new(),
            sole_table_holders: HashMap::new(),
            inner_tail_paths: HashMap::new(),
            run_breaks: HashMap::new(),
            run_break_starts: super::RunBreakStarts::default(),
            has_named_pages,
            finds_nested_runs: cascade.computed.iter().any(|computed| {
                [computed.break_before, computed.break_after]
                    .into_iter()
                    .any(|edge| {
                        matches!(
                            edge,
                            raikiri_style::property::BreakBetween::Avoid
                                | raikiri_style::property::BreakBetween::AvoidPage
                        )
                    })
            }),
            forces_break_after: cascade
                .computed
                .iter()
                .any(|computed| computed.break_after.may_force_page_break()),
            edges_breaks: Vec::new(),
            pending_bottoms: HashMap::new(),
            preceding_bottoms: HashMap::new(),
        }
    }

    /// Collect the page candidates of the flow under the body into `out`,
    /// in source order. `root_flow_offset` is where the body's border box
    /// starts in the page flow.
    pub(super) fn collect_flow(&mut self, root_flow_offset: f32) -> Result<(), LayoutError> {
        self.collect(self.roots.element, CandidateFrame::root(root_flow_offset))?;
        super::leading_inline::assign(
            self.document,
            self.out,
            &mut self.edges_breaks,
            self.control,
        )
    }

    /// The edges of the clips that cut the text of the paragraphs collected,
    /// by paragraph root, in the coordinates of the candidates, where the
    /// bottom edges of the blocks begin below their content, and the bottoms
    /// of the in-flow blocks that are not candidates before the candidates
    /// after them (see `preceding_bottoms`).
    pub(super) fn into_outputs(
        self,
    ) -> (HashMap<usize, LineClip>, Vec<EdgesBreak>, PrecedingBottoms) {
        (self.clip_edges, self.edges_breaks, self.preceding_bottoms)
    }

    /// Notes the in-flow box `node_id` among the children of its parent: a
    /// candidate takes the block that is not a candidate before it, any box
    /// that takes space ends the margin after that block, and a block that
    /// is not a candidate starts one (`bottom`). A candidate ends it too,
    /// also an empty one: pagination records each candidate as the block
    /// before the next, and an empty one may carry a forced break on.
    fn follow_sibling(&mut self, node_id: usize, candidate: bool, bottom: Option<f32>) {
        // Every box this is asked about is below the root, so it has a parent.
        let parent = candidate_parent(self.document, node_id);
        let takes_space = !super::takes_no_block_space(self.document, node_id);
        let pending = parent.and_then(|parent| {
            if takes_space || candidate {
                self.pending_bottoms.remove(&parent)
            } else {
                self.pending_bottoms.get(&parent).copied()
            }
        });
        if candidate && let Some(pending) = pending {
            self.preceding_bottoms.insert(node_id, pending);
        }
        if let (Some(parent), Some(bottom)) = (parent, bottom.filter(|_| takes_space)) {
            self.pending_bottoms.insert(parent, (node_id, bottom));
        }
    }

    /// Whether the flex container `id` holds a sole table that fragments as
    /// block flow does, computed once per container.
    fn holds_sole_table(&mut self, id: usize) -> bool {
        let (document, cascade) = (self.document, self.cascade);
        *self.sole_table_holders.entry(id).or_insert_with(|| {
            crate::layout::table::headers::sole_flex_table(document, cascade, id).is_some()
        })
    }

    #[inline(never)]
    fn collect(&mut self, node_id: usize, frame: CandidateFrame<'a>) -> Result<(), LayoutError> {
        // The walk goes down the children of the tree, whose depth
        // `validate_layout_depth` has bounded, so it keeps no count of its own.
        stacker::maybe_grow(128 * 1024, 1024 * 1024, || {
            self.collect_inner(node_id, frame)
        })
    }

    #[inline(never)]
    fn collect_inner(
        &mut self,
        node_id: usize,
        frame: CandidateFrame<'a>,
    ) -> Result<(), LayoutError> {
        let Self {
            document,
            cascade,
            roots,
            page_step,
            control,
            ..
        } = *self;
        let CandidateFrame {
            parent_abs_y,
            direct_body_child,
            parent_height,
            inside_table,
            flex_column_parent,
            grid_single_column_parent,
            inside_flex,
            named_flex_context,
            inside_float,
            inside_out_of_flow,
            ..
        } = frame;
        let body_id = roots.contents;
        control.check_aborted()?;
        let Some(node) = document.get_node(node_id) else {
            return Ok(()); // cov:ignore: traversal IDs come from this document's valid child graph.
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            return Ok(());
        }
        match node.kind() {
            NodeKind::Text => {
                // A text node after the first of the run that makes one
                // anonymous flex or grid item is laid out with it: the first
                // node is the candidate for the whole item.
                let later_text_of_item = node.in_ifc_subtree()
                    && document
                        .ifc_root_of(node_id)
                        .and_then(|root| document.get_node(root))
                        .is_some_and(|root| root.kind() == NodeKind::Text);
                if (direct_body_child || parent_height > 0.0)
                    && !later_text_of_item
                    && matches!(
                        &node.data,
                        crate::node::NodeData::Text(text)
                            if crate::layout::ifc::text_lines::places_lines(cascade, node_id, &text.text_content)
                    )
                {
                    // A text node of an ifc paragraph has no layout of its own:
                    // it stands for the lines it owns, measured from the root's
                    // content box. The inline elements between the root and
                    // the text pass the root's border-box y down unchanged, so
                    // `parent_abs_y` is the root's. A text node that is a root
                    // itself (an anonymous flex or grid item) has a box of its
                    // own, located like any other.
                    if text_moves_with_orthogonal_flow(document, cascade, body_id, node_id) {
                        return Ok(());
                    }
                    self.text_lines
                        .prepare_for(document, node_id, || control.check_aborted())?;
                    // White space alone, preserved or a no-break space. When
                    // it shows up on no line, such as a lone line feed that
                    // only ends the line before it, it has nothing to place.
                    let blank = matches!(&node.data, crate::node::NodeData::Text(text)
                        if text.text_content.trim().is_empty());
                    if blank && node.in_ifc_subtree() && self.text_lines.get(node_id).is_none() {
                        return Ok(());
                    }
                    let (raw_y, height, ifc_root) = match self.text_lines.get(node_id) {
                        Some(owned) if owned.root != node_id => {
                            let root_layout = document
                                .ifc_layout_node(owned.root)
                                .expect("owned text lines require an IFC layout view")
                                .layout;
                            let root_y = if document.is_anonymous_cell(owned.root) {
                                root_layout.location.y
                            } else {
                                0.0
                            };
                            let positioned = positioned_owned_text_line_bounds(document, owned)
                                .expect(
                                    "ifc_text_lines returns Some only when root line records exist",
                                );
                            let first_top = first_owned_text_line_top(owned, Some(&positioned));
                            let last_bottom = positioned
                                .iter()
                                .map(|line| line.1)
                                .fold(f32::NEG_INFINITY, f32::max);
                            let offset =
                                root_layout.border.top + root_layout.padding.top + first_top;
                            (
                                parent_abs_y + root_y + offset,
                                (last_bottom - first_top).max(0.0),
                                Some((owned.root, offset)),
                            )
                        }
                        _ => (
                            parent_abs_y + node.layout.location.y,
                            node.layout.size.height.max(0.0),
                            None,
                        ),
                    };
                    // Inline boxes do not clip, so every text of one inline
                    // formatting context sees the same clip.
                    if let Some((root, _)) = ifc_root
                        && frame.lines_clip_end.is_finite()
                        && !self.clip_edges.contains_key(&root)
                    {
                        let mut owners = Vec::new();
                        if frame.lines_clip_owner.is_some() {
                            let mut next = frame.lines_clip_box;
                            while let Some(owner) = next {
                                owners.push(owner);
                                next = self.enclosing_line_clips.get(&owner).copied().flatten();
                            }
                        }
                        self.clip_edges.insert(
                            root,
                            LineClip {
                                edge: frame.lines_clip_end,
                                owners,
                            },
                        );
                    }
                    // Past a clipping ancestor's edge the text paints nowhere
                    // and gives the flow no page; a text running over the edge
                    // occupies only the part above it.
                    if raw_y < frame.clip_end {
                        // Text ends the margin after the block before it.
                        if let Some(parent) = candidate_parent(document, node_id) {
                            self.pending_bottoms.remove(&parent);
                        }
                        self.out.push(PageCandidate {
                            node_id,
                            raw_y,
                            height: height.min(frame.clip_end - raw_y),
                            context: BoxContext::text(CandidateFlowContext {
                                is_float_descendant: inside_float,
                                is_out_of_flow_descendant: inside_out_of_flow,
                                // The lines of preserved white space are
                                // content of the flow: they take forced
                                // breaks and page names (Chromium). White
                                // space that lays out no line, such as
                                // between flex items, is none.
                                page_name_in_flow: !inside_float
                                    && !inside_out_of_flow
                                    && (!blank || ifc_root.is_some()),
                                lines_closing_edges: frame.lines_closing_edges,
                            }),
                            page_name: page_name_to_owned(frame.inherited_page_name),
                            named_flex_context,
                            ifc_root,
                        });
                    }
                }
            }
            NodeKind::Element => {
                if node.is_display_none() {
                    return Ok(());
                }
                // An element inside a paragraph laid out by the inline engine
                // moves with the paragraph's lines; breaking at it on its own
                // would leave the lines where they are. Its text stands for
                // lines of the root and the boxes inside it are located from
                // the root, like the root's own children, so both are
                // collected as if they were. The flag is set only when the
                // inline engine is switched on.
                if node.flags.contains(NodeFlags::IN_IFC_SUBTREE) {
                    let rendered_children = node.children.iter().copied().filter(|&child_id| {
                        let kind = document.nodes[child_id].kind();
                        matches!(kind, NodeKind::Element | NodeKind::Text) // cov:ignore: children of an inline-engine element are elements or text.
                    });
                    for child_id in rendered_children {
                        let mut child_frame = frame.clone();
                        child_frame.direct_body_child =
                            candidate_parent(document, child_id) == Some(roots.contents_box);
                        child_frame.closing_edges = 0.0;
                        child_frame.previous_box = None;
                        child_frame.column_break_before = false;
                        child_frame.tie = AvoidTie::None;
                        child_frame.run_unit = None;
                        child_frame.run_tail = false;
                        child_frame.item_tail = None;
                        child_frame.tie_head = None;
                        child_frame.float_bottom = f32::NEG_INFINITY;
                        self.collect(child_id, child_frame)?;
                    }
                    return Ok(());
                }
                let raw_y = if document.oof_coordinates.contains_key(&node_id) {
                    super::current_abs_y(document, node_id)
                } else {
                    parent_abs_y + node.layout.location.y
                };
                let computed = &cascade.computed[node_id];
                // A floated box does not establish a page transition merely
                // because it carries an inherited/explicit `page` value.  In
                // particular, CSS Page 3 keeps a float in the preceding
                // fragmentainer in the page-name-float cases.
                let is_float = is_floating_box_for_pagination(document, cascade, node_id);
                let float_subtree = inside_float || is_float;
                let out_of_flow_subtree = inside_out_of_flow
                    || matches!(
                        document.nodes[node_id].position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    );
                // An absolutely positioned box left at its static position
                // still lays its in-flow descendants out where the page flow
                // puts them, so their page names take part in pagination.
                let inset_out_of_flow_subtree =
                    out_of_flow_subtree && in_inset_out_of_flow_subtree(document, cascade, node_id);
                let explicit_page_name = selected_page_name(cascade, node_id);
                // CSS Page 3 §4.1: `page` applies to boxes that create class A
                // break points, which are block-level. An inline-level box,
                // such as a replaced element in a line or an inline-block,
                // takes the page type of its containing block; it is a
                // candidate carrying that inherited name instead.
                let inline_level = !page_applies_to_box(document, node_id);
                // The `page` property on an out-of-flow box does not open a
                // normal-flow page boundary. Keep its inherited page context,
                // but do not use its explicit name to split pagination.
                let parent_box =
                    BoxTree::for_layout(document).parent(node_id, BoxParentScope::Document);
                let is_direct_flex_item = parent_box.is_some_and(|parent| {
                    document.nodes[parent].formatting_context == FormattingContext::Flex
                        && !self.holds_sole_table(parent)
                });
                let own_page_name = if !inline_level
                    && !float_subtree
                    && (!inside_flex || named_flex_context.is_some())
                    && !is_direct_flex_item
                    && !inset_out_of_flow_subtree
                    && !matches!(
                        document.nodes[node_id].position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    ) {
                    explicit_page_name
                } else {
                    None
                };
                let (has_propagated_page_name, propagated_page_name) = if self.has_named_pages {
                    propagated_start_page_name_with_transposed_axes(
                        document,
                        cascade,
                        node_id,
                        frame.inherited_page_name,
                        document.context.pagination_on_transposed_axes,
                    )
                } else {
                    // Without Named values, both own and inherited names remain None.
                    (false, None)
                };
                let is_flex = document.nodes[node_id].formatting_context == FormattingContext::Flex;
                // A single-line row flex container holding only a table
                // fragments it as block flow does, so that its header and
                // footer repeat on each page.
                let holds_sole_table = is_flex && self.holds_sole_table(node_id);
                // A table in a single-line column flex container stacks as
                // a block does, so it fragments as block flow does too.
                let stacks_tables =
                    crate::layout::table::headers::stacking_column_flex(document, cascade, node_id);
                let column_flex = is_flex
                    && matches!(
                        computed.flex_direction,
                        FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                    );
                // An inline-level flex container takes the page of its
                // containing block, and one inside an item of a row flex
                // container sits on the page that row's line is printed on,
                // so the names inside either neither name a page nor force
                // one after it.
                let deferred_named_break_after = self.has_named_pages
                    && !float_subtree
                    && !inline_level
                    && (!inside_flex || named_flex_context.is_some())
                    && document.nodes[node_id].display().is_flex_container()
                    && !column_flex
                    && has_nested_named_page_descendant(
                        document,
                        cascade,
                        node_id,
                        0,
                        document.context.pagination_on_transposed_axes,
                        &mut self.named_page_subtree_cache,
                    );
                // The items of a row flex container sit side by side rather
                // than as class A siblings, so their names do not become the
                // container's start page value.
                let row_flex = is_flex && !column_flex && !holds_sole_table;
                let flex_names_stay_on_items = column_flex
                    || (row_flex && !deferred_named_break_after)
                    || (named_flex_context.is_some() && !deferred_named_break_after);
                // Such a container takes its start page value from the first of
                // its items that has one, as Chromium prints the line on the
                // page that item names.
                let propagated_page_name = if deferred_named_break_after && row_flex {
                    propagated_page_name.or_else(|| {
                        first_named_item_start_page(
                            document,
                            cascade,
                            node_id,
                            frame.inherited_page_name,
                            document.context.pagination_on_transposed_axes,
                        )
                    })
                } else {
                    propagated_page_name
                };
                let page_name = if has_propagated_page_name && !flex_names_stay_on_items {
                    propagated_page_name
                } else {
                    own_page_name.or(frame.inherited_page_name)
                };
                // A row flex container that takes the start page value of a
                // named box nested in its items puts all its items on that
                // page, so the boxes inside them inherit it.
                let child_page_name = if deferred_named_break_after && row_flex {
                    page_name
                } else {
                    own_page_name.or(frame.inherited_page_name)
                };
                let is_body = node_id == body_id || node_id == roots.element;
                let participates_in_flow = matches!(
                    document.nodes[node_id].position(),
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                );
                let is_tall_direct_absolute = direct_body_child
                    && matches!(document.nodes[node_id].position(), PositionValue::Absolute)
                    && node.layout.size.height > page_step;
                let table_row_candidate = inside_table
                    && matches!(document.nodes[node_id].display(), DisplayValue::TableRow)
                    && !inside_float
                    && !inside_flex;
                // A caption of a block table is a box of its own for the page
                // break: with no good place to break between its lines, it
                // moves to the next page whole (see `BoxOverflow`). Left to its
                // text, the line that crosses the page end stays and the page
                // cuts through it. A caption that holds blocks is one only when
                // it avoids a break inside it; its blocks break as in block
                // flow. A caption inside a table cell goes with its row.
                let table_caption_candidate = inside_table
                    && participates_in_flow
                    && matches!(
                        document.nodes[node_id].display(),
                        DisplayValue::TableCaption
                    )
                    && !inside_float
                    && (!inside_flex
                        || is_flow_table_caption(document, cascade, node_id, roots.element))
                    && !out_of_flow_subtree
                    && !node.used_writing_mode.is_vertical()
                    && (node.is_ifc_root() || avoids_page_break_inside(computed.break_inside))
                    && document
                        .layout_parent_of(node_id)
                        .is_some_and(|table| is_plain_table_box(document, table))
                    && !document.ancestors(node_id).any(|ancestor| {
                        matches!(document.nodes[ancestor].display(), DisplayValue::TableCell)
                    });
                let flex_item_candidate =
                    flex_column_parent && !inside_float && !out_of_flow_subtree;
                let grid_item_candidate =
                    grid_single_column_parent && !inside_float && !out_of_flow_subtree;
                // The table engine stores row geometry on its cells rather
                // than on the anonymous row box. Derive the row border box
                // from those direct cells so pagination can keep the row
                // together without changing table sizing.
                let (candidate_raw_y, candidate_height) = if table_row_candidate {
                    let mut row_top = f32::INFINITY;
                    let mut row_bottom = f32::NEG_INFINITY;
                    for &child_id in &node.children {
                        let Some(child) = document.get_node(child_id) else {
                            continue; // cov:ignore: document-owned child links are valid by construction.
                        };
                        let child_top = child.layout.location.y;
                        row_top = row_top.min(child_top);
                        row_bottom = row_bottom.max(child_top + child.layout.size.height);
                    }
                    if row_top.is_finite() && row_bottom.is_finite() && row_bottom > row_top {
                        (raw_y + row_top, row_bottom - row_top)
                    } else {
                        (raw_y, node.layout.size.height.max(0.0))
                    }
                } else {
                    (raw_y, node.layout.size.height.max(0.0))
                };
                // Non-text content can establish the anonymous page before
                // a later named descendant, even within the first flex item.
                // Container geometry alone does not establish that context.
                let is_rendered_leaf = named_flex_context.is_some()
                    && document.nodes[node_id].display() != DisplayValue::Contents
                    && !float_subtree
                    && !out_of_flow_subtree
                    && candidate_height > 0.0
                    && !node.children.iter().any(|&child| {
                        let child_node = &document.nodes[child];
                        if child_node.is_display_none()
                            || child_node.is_non_rendered_html_element()
                            || matches!(
                                document.nodes[child].position(),
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                            || is_floating_box_for_pagination(document, cascade, child)
                        {
                            return false;
                        }
                        match &child_node.data {
                            crate::node::NodeData::Text(text) => {
                                !text.text_content.trim().is_empty()
                            }
                            crate::node::NodeData::Element(_) => {
                                child_node.layout.size.height > 0.0
                            }
                            _ => false,
                        }
                    });
                // `break-inside: avoid` applies to block-level boxes at any
                // depth (CSS Fragmentation 3 §3.2), not only to the body's
                // children. A nested avoiding block is a candidate so the
                // pass can move it to the next page intact; its descendants
                // follow it. Tables and flex containers keep their own
                // item-level candidates.
                let is_block_flow_box = is_block_level_flow_box(document.nodes[node_id].display());
                // The one generated, glyphless or atomic-inline line of a
                // block is placed by no text candidate, so the block moves
                // whole instead. Content placed by the rules of a float, an
                // out-of-flow box, or a flex, grid or table keeps them, and
                // so does a document paginated on transposed axes, whose
                // lines do not run along y.
                let unplaced_base = !inside_float
                    && !inside_out_of_flow
                    && !inside_table
                    && !is_rendered_leaf
                    && !document.context.pagination_on_transposed_axes;
                let unplaced_line =
                    (unplaced_base && !inside_flex && !flex_item_candidate && !grid_item_candidate)
                        .then(|| unplaced_line(document, cascade, node_id))
                        .flatten();
                // A line after a block child keeps nothing whole: the page
                // breaks between the block and the line, inside a flex or
                // grid item too, as between the lines of text there.
                let line_after_block = unplaced_base
                    .then(|| unplaced_line_after_block(document, cascade, node_id))
                    .flatten();
                let unbreakable_box = box_is_unbreakable(
                    document,
                    cascade,
                    node_id,
                    document.context.pagination_on_transposed_axes,
                );
                // The part of such a box that lies inside the blocks it begins,
                // when it runs out past the bottom of one whose height is held
                // below its content and nothing clips it: the rest is visible
                // overflow of that block, which Chromium cuts at the page end
                // rather than moving the block on. A box with something before
                // it in its parent has a break before it to take instead, and
                // moves whole as usual. A float that cannot be split and whose
                // margin box fits on a page is not overflow of the blocks
                // around it: it moves on whole. One that establishes an
                // orthogonal flow is cut as before.
                let margin = node.layout.margin;
                let moves_on_whole = is_float
                    && unbreakable_box
                    && margin.top.max(0.0) + node.layout.size.height + margin.bottom.max(0.0)
                        <= page_step
                    && !super::super::page::establishes_orthogonal_flow(document, cascade, node_id);
                let unbreakable_room = (unbreakable_box
                    && !moves_on_whole
                    && !direct_body_child
                    && frame.clip_end.is_infinite())
                .then(|| unbreakable_room(document, cascade, body_id, node_id))
                .flatten();
                let avoids_by_property = matches!(
                    computed.break_inside,
                    raikiri_style::property::BreakInside::Avoid
                        | raikiri_style::property::BreakInside::AvoidPage
                );
                let avoids_break = avoids_by_property || unplaced_line.is_some() || unbreakable_box;
                let clears_float = !inline_level
                    && is_block_flow_box
                    && [FloatValue::Left, FloatValue::Right]
                        .into_iter()
                        .any(|side| clears_float_side(computed.clear, side));
                let nested_flow_base = participates_in_flow
                    && !direct_body_child
                    && !inside_flex
                    && !out_of_flow_subtree;
                let nested_candidate_base = nested_flow_base && !inside_table;
                // A block in the flow of a table cell avoids a break inside
                // it as in any block flow: Chromium moves it to the next
                // page, and its row grows to hold it.
                let cell_avoid_base = nested_flow_base
                    && inside_table
                    && avoids_by_property
                    && moves_reach_cell_row(document, node_id);
                // A block in the flow of a table cell that clears floats goes
                // below a float that pagination moves to a later page, as in
                // any block flow, and its row grows to hold it.
                let cell_clear_base = nested_flow_base
                    && inside_table
                    && clears_float
                    && !is_float
                    && clears_in_cell_flow(document, cascade, node_id, body_id);
                // A box in the flow of its container, among blocks or among the
                // items of a flex container.
                let in_any_flow = participates_in_flow
                    && !out_of_flow_subtree
                    && !inside_float
                    && !inline_level
                    && !is_float;
                let in_container_flow = in_any_flow && !inside_table;
                let in_flow_block_level = in_container_flow && !inside_flex;
                // A block box in the flow that is a candidate to move to the
                // next page, along with the box it holds or the edges it ends.
                let in_flow_block = in_flow_block_level && is_plain_block_box(document, node_id);
                // A box after one that forces a break after itself takes the
                // break, though it holds no line of its own to take it: the
                // pass otherwise leaves it on the page the break closes, and
                // the break falls before the next box that holds a line. The
                // box before it is the last box in the flow ahead of it, a
                // float or an out-of-flow box between the two not counted,
                // and only a box that lays its children out in block flow
                // has one (see `lays_out_children_in_block_flow`). Chromium
                // takes the break there whatever holds that box: a flex item,
                // a grid item or a table cell, anonymous or not.
                let after_forced_break = self.forces_break_after
                    && in_any_flow
                    && is_plain_block_box(document, node_id)
                    && !direct_body_child
                    && frame
                        .previous_box
                        .or_else(|| super::spanner_before(document, cascade, node_id))
                        .is_some_and(|previous| {
                            page_edge_constraint(document, cascade, previous, false).0
                        });
                // A flex container, grid or table is a box that can end a block
                // too, and move to the next page for the edges it ends.
                let in_flow_container =
                    in_flow_block_level && is_plain_container_box(document, node_id);
                // A box before it must be there to break from, with no break
                // avoided between the two, unless every break before it is
                // avoided: one that gives way is the only one left.
                let break_before_available = || {
                    let waived = frame.tie == AvoidTie::Waived;
                    frame.previous_box.is_some_and(|previous| {
                        waived || !page_edge_constraint(document, cascade, previous, false).1
                    }) && (waived || !page_edge_constraint(document, cascade, node_id, true).1)
                };
                // The last box of a block whose bottom padding or border
                // would break away from it stays with them: it is a nested
                // candidate that moves to the next page when they do not fit
                // after it.
                let closing_edges_base_shape =
                    (in_flow_block || in_flow_container) && frame.closing_edges > 0.0;
                // A paragraph that avoided breaks tie to the boxes before it,
                // all the way to the first, has no break between boxes to
                // take, not even the one that gives way for the last box of a
                // block: it is split between its lines like a first box.
                let closing_edges_shape = closing_edges_base_shape
                    && break_before_available()
                    && !(frame.tied_to_start && node.is_ifc_root());
                // With no box before it in the block, or only boxes tied to
                // it, the edges follow its lines when it is a paragraph that
                // breaks between them.
                let lines_shape = closing_edges_shape
                    || (closing_edges_base_shape
                        && (frame.previous_box.is_none()
                            || frame.tied_to_start
                            || frame.lines_tail)
                        && node.is_ifc_root());
                // Nothing may break inside it at a better place than before
                // it. A paragraph with lines enough to break between moves
                // some of them instead (see `lines_closing_edges`).
                // The first box of a tied run was found to move whole when the
                // run was. So does a paragraph after a box to break from whose
                // lines are too few for its orphans and widows: the break
                // before it is the one between boxes, which beats one between
                // lines that leaves too few at either end.
                let unbreakable = lines_shape
                    && !frame.lines_tail
                    && (matches!(frame.tie, AvoidTie::RunStart { .. })
                        || breaks_only_around(document, cascade, node_id, self.has_named_pages)
                        || (closing_edges_shape
                            && node.is_ifc_root()
                            && paragraph_lines_keep_together(document, cascade, node_id)));
                // A box that asks to avoid a break inside it has nothing to
                // break at either: it moves to the next page with the edges
                // that would break away from it, so that the page breaks
                // between boxes rather than inside it or before the edges. A
                // box that forces a break inside it splits there instead.
                let closing_edges_base = closing_edges_shape
                    && (unbreakable
                        || (avoids_by_property
                            && !has_mandatory_break_inside(document, cascade, node_id)));
                // How far below the first box of the run that avoided breaks
                // tie the node to its top sits, so that the avoided break
                // before it can give way when that box starts its page.
                let tie_offset = frame.tie_head.map(|head| {
                    normal_flow_y(document, cascade, body_id, node_id)
                        - normal_flow_y(document, cascade, body_id, head)
                        + document.nodes[head].layout.margin.top.max(0.0)
                });
                // A block that holds one box moves whole with the edges that
                // would break away from it, its own and those of the blocks
                // it ends: no box before the one in it is there to break from.
                // A forced break at either end of the block, or a clipped
                // part of it, is left to the rules for those. Tied to the box
                // before it, it is still a candidate: the run those boxes
                // make is kept together only while its first box leaves the
                // page room for it.
                // The last box of a tied run whose lines break for the edges
                // breaks there instead (see `lines_tail`).
                let lone_tail = (in_flow_block
                    && frame.previous_box.is_some()
                    && !frame.lines_tail
                    && !matches!(frame.tie, AvoidTie::RunStart { .. }))
                .then(|| {
                    lone_box_with_floats(
                        document,
                        cascade,
                        body_id,
                        node_id,
                        frame.closing_edges > 0.0,
                        true,
                    )
                })
                .flatten()
                .filter(|lone| {
                    lone.tail + frame.closing_edges > 0.001
                        && candidate_raw_y + node.layout.size.height <= frame.clip_end
                        && (tie_offset.is_some() || break_before_available())
                        && !page_edge_constraint(document, cascade, node_id, true).0
                        && !page_edge_constraint(document, cascade, node_id, false).0
                });
                // A box that starts a run of boxes tied by avoided breaks among
                // the boxes of its parent (`frame.run_unit`), or holds one at
                // the start of its own boxes, moves to the next page with the
                // run when that crosses a page edge. A box with no box before
                // it has no break to take before it: the block around it holds
                // the run and moves for it.
                let free_to_break_before =
                    frame.previous_box.map_or(direct_body_child, |previous| {
                        !boxes_are_tied(document, cascade, previous, node_id)
                    });
                // Generated content right before a box with no box before it
                // in its parent's flow (see `generated_content_end_above`):
                // where it ends in the flow, and where the box starts there.
                let generated_before = std::cell::OnceCell::new();
                let generated_before = || {
                    *generated_before.get_or_init(|| {
                        (frame.previous_box.is_none()
                            && !direct_body_child
                            && !cascade.pseudo.is_empty())
                        .then(|| {
                            let flow_top = candidate_raw_y
                                - accumulated_relative_offset_y(
                                    document, cascade, body_id, node_id,
                                );
                            super::generated_content_end_above(
                                document, cascade, body_id, node_id, flow_top,
                            )
                            .map(|(_, end)| (end, flow_top))
                        })
                        .flatten()
                    })
                };
                // Such content gives the flow a break before the box, as a
                // box there would: edges that would be left alone at the page
                // end after it may move on with what they hold.
                // So do the lines of a paragraph before a block among them
                // (an anonymous block holds them): Chromium breaks between
                // the last of them and the block.
                let after_paragraph_content = frame.previous_box.is_none()
                    && breaks_after_paragraph_content(document, cascade, node_id);
                let breaks_before_box =
                    free_to_break_before || generated_before().is_some() || after_paragraph_content;
                let column_break_before = (node.display() == DisplayValue::Contents
                    || !matches!(
                        computed.break_before,
                        BreakBetween::Avoid | BreakBetween::AvoidPage
                    ))
                    && (breaks_before_box
                        || frame.previous_box.is_none() && frame.column_break_before);
                let leading_unit =
                    // A block among the lines of a paragraph may also break
                    // after the text or boxes before it there.
                    (self.finds_nested_runs
                        && in_flow_block
                        && !is_body
                        && (free_to_break_before || after_paragraph_content))
                        .then(|| avoided_run_head(document, cascade, body_id, node_id, true))
                        .filter(|held| held.tied)
                        .and_then(|held| {
                            held.end.map(|end| super::RunUnit {
                                end,
                                firm: held.firm,
                                breaks_inside: false,
                            })
                        });
                // A run that starts at the box inside the first box of the run
                // (see `nested_run_units`).
                // The box may also start a run among the boxes of its parent:
                // the page holds the longer of the two.
                let run_unit = [frame.run_unit, self.run_breaks.remove(&node_id)]
                    .into_iter()
                    .flatten()
                    .reduce(|own, inner| if inner.end > own.end { inner } else { own });
                let avoided_unit = match (run_unit, leading_unit) {
                    (Some(run), Some(leading)) if leading.end > run.end => Some(leading),
                    (run, leading) => run.or(leading),
                };
                // A grid breaks between its padding or border and a first
                // row that does not fit after them; the line of a flex
                // container may do so on the page it moves to (see
                // `grid_rows`).
                // The first box of plain blocks that start after a free
                // break takes that break too (see `column_break_before`).
                // Content of the parent's flow comes before the box, and the
                // page break between the two is avoided, by the box or by
                // the box before it (see `ElementContext::avoided_break_before`).
                let avoided_break_before =
                    (frame.box_before || super::lines_before_block(document, node_id)) && {
                        let (forced, avoided) =
                            page_edge_constraint(document, cascade, node_id, true);
                        !forced
                            && (avoided
                                || frame.previous_box.is_some_and(|previous| {
                                    let (forced, avoided) =
                                        page_edge_constraint(document, cascade, previous, false);
                                    !forced && avoided
                                }))
                    };
                // The part of that which the page passes on to the content
                // before the box: lines of its paragraph, which give way
                // for it (see `PaginationState::keep_lines_with_next_block`),
                // or a plain block of the same parent tied to it, which
                // moves on with it as a run (see `nested_run_units` and
                // `avoided_run_ends`). Only then does a box that avoids
                // breaks inside it split where it stands rather than move
                // on alone.
                let avoided_break_passed_on = avoided_break_before
                    && (super::lines_before_block(document, node_id)
                        || frame.previous_box.is_some_and(|previous| {
                            let parent = document.nodes[node_id].parent;
                            is_plain_block_box(document, previous)
                                && document.nodes[previous].parent == parent
                                && parent.is_some_and(|parent| {
                                    document.nodes[parent].display() != DisplayValue::Contents
                                })
                        }));
                // A break avoided before the box is still a better one than a
                // break through the first piece of its content, which cuts a
                // line or a monolithic box: Chromium takes it when that piece
                // does not fit.
                let container_breaks_before =
                    breaks_before_box || column_break_before || avoided_break_before;
                let first_grid_row = (in_flow_container && !is_body && container_breaks_before)
                    .then(|| super::grid_rows::first_row(document, cascade, body_id, node_id))
                    .flatten();
                // A row flex container whose padding or border above its
                // items would be left alone at the foot of a page, because
                // the first piece of its content does not fit after them,
                // moves to the next page whole. So does a grid of several
                // columns whose rows cannot be read, when its first item
                // cannot be split from them either. A column flex container,
                // or a grid of one column, which stacks its items as block
                // flow, breaks between those edges and its first item, as
                // Chromium does, even when the item is monolithic.
                let display = document.nodes[node_id].display();
                let leading_piece = (in_flow_container
                    && !is_body
                    && container_breaks_before
                    && (display == DisplayValue::Flex && !column_flex
                        || display == DisplayValue::Grid
                            && first_grid_row.is_none()
                            && !super::grid_stacks_items(document, node_id)
                            && super::grid_rows::first_item_is_unbreakable(
                                document, cascade, node_id,
                            ))
                    && !document.context.pagination_on_transposed_axes)
                    .then(|| leading_piece_end(document, cascade, body_id, node_id))
                    .flatten();
                // Keep leading edges with a first unbreakable block piece.
                // Atomic inline placement is assigned to its owning line.
                let leading_block_piece = (leading_piece.is_none()
                    && participates_in_flow
                    && !inside_table
                    && !inside_flex
                    && !out_of_flow_subtree
                    && !inside_float
                    && has_block_leading_content(document, node_id)
                    && !is_body
                    && (breaks_before_box
                        || is_float
                        || node.display() == DisplayValue::InlineBlock)
                    && !document.context.pagination_on_transposed_axes)
                    .then(|| {
                        // A float's parallel flow keeps its first indivisible
                        // piece even without leading edges. Its owner moves;
                        // the children remain outside the outer candidate walk.
                        if is_float {
                            super::lineless_piece_end(document, cascade, body_id, node_id, 0)
                                .map(|(end, _)| end)
                        } else {
                            lineless_leading_piece_end(document, cascade, body_id, node_id)
                        }
                    })
                    .flatten();
                let column_replaced_piece = (in_flow_container
                    && !is_body
                    && (column_break_before || avoided_break_before)
                    && !document.context.pagination_on_transposed_axes)
                    .then(|| {
                        super::column_replaced::first_piece(document, cascade, body_id, node_id)
                    })
                    .flatten();
                // The last box of such a run goes to the next page with the
                // edges that follow it when the run has moved as far on as it
                // goes (the first box starts its page) and they still do not
                // fit after the box.
                let run_tail_base = frame.run_tail
                    && (in_flow_block || in_flow_container)
                    && frame.closing_edges > 0.0;
                // A nested block after a block of its parent's flow, or
                // first in it after generated content that has no candidate
                // of its own (see `generated_content_end_above`), with a
                // margin between the two: a page edge in that margin cuts it.
                // Only margins lie between such content and the block, so
                // any gap there is one.
                let margin_shape = in_flow_block_level
                    && !direct_body_child
                    && !node.used_writing_mode.is_vertical()
                    && match frame.previous_box {
                        Some(previous) => {
                            node.layout.margin.top > 0.0
                                || document.nodes[previous].layout.margin.bottom > 0.0
                                // A margin of a box inside the one before it
                                // that collapses through that box's bottom.
                                // An anonymous box has no style of its own.
                                || (cascade.computed.get(previous).is_some()
                                    && super::adjoining_margin_extremes(
                                        document, cascade, previous, false, None,
                                    )
                                    .0 > 0.0)
                                // Margins that collapse through an empty box
                                // put it below them. With no clearance, only
                                // margins lie between two siblings.
                                || (document.nodes[previous].parent == node.parent
                                    && matches!(
                                        computed.clear,
                                        raikiri_style::property::ClearValue::None
                                    )
                                    && normal_flow_y(document, cascade, body_id, node_id)
                                        - normal_flow_bottom(document, cascade, body_id, previous)
                                        > 0.001)
                        }
                        None => {
                            generated_before().is_some_and(|(end, flow_top)| end < flow_top - 0.001)
                        }
                    };
                // A paragraph whose single unplaced line comes after a block
                // child of it breaks between the two: it is placed as a box
                // to move that line on (see `break_before_line_after_block`).
                // Not in columns, whose balance the move would change, nor
                // beside boxes at more than one level.
                let line_after_block_base = participates_in_flow
                    && !out_of_flow_subtree
                    && edges_break_container(document, node_id, body_id).is_some()
                    && !inline_level
                    && is_block_flow_box
                    && line_after_block.is_some();
                // A flex container nested in block flow moves whole when what
                // follows its items runs past the page end (see
                // `flex_margins_move`), so it needs a place of its own.
                // Inside the item of a flex row, it moves the row it begins
                // with it (see `starts_single_row`); the walk over its
                // ancestors drops it where some other one lays boxes beside it.
                let flex_margins_base = (nested_candidate_base
                    || (inside_flex
                        && participates_in_flow
                        && !direct_body_child
                        && !out_of_flow_subtree
                        && !inside_table))
                    && !inline_level
                    && super::has_flex_trailing_space(document, node_id);
                let run_break_base = self.finds_nested_runs
                    && in_flow_block
                    && !direct_body_child
                    && self
                        .run_break_starts
                        .starts_break(document, cascade, body_id, node_id);
                let needs_ancestor_constraints = margin_shape
                    || run_break_base
                    || line_after_block_base
                    || (nested_candidate_base
                        && ((!inline_level && is_block_flow_box && avoids_break)
                            || is_float
                            || clears_float))
                    || (cell_avoid_base && !inline_level && is_block_flow_box)
                    || cell_clear_base
                    || lines_shape
                    || lone_tail.is_some()
                    || run_tail_base
                    || (avoided_unit.is_some() && !direct_body_child)
                    || (leading_piece.is_some() && !direct_body_child)
                    || (leading_block_piece.is_some() && !direct_body_child)
                    || (column_replaced_piece.is_some() && !direct_body_child)
                    || (first_grid_row.is_some() && !direct_body_child)
                    || flex_margins_base;
                // A forced break and a page name change the pages wherever the
                // box sits, even past the edge of a clipping ancestor. Whether
                // a clip box around it holds the break depends on where
                // pagination puts that box, so it is judged there (see
                // `clip_holds_breaks`).
                let sets_page_flow = deferred_named_break_after
                    || own_page_name.is_some()
                    || forces_page_break(document, cascade, node_id, true)
                    || forces_page_break(document, cascade, node_id, false);
                let (side_by_side_ancestor, bounded_clipping_ancestor) =
                    if needs_ancestor_constraints {
                        // A replaced box that no break goes through and that
                        // begins the plain blocks around it takes them to the
                        // next page with it, so a clipping ancestor among them
                        // with a height of its own does not keep it from
                        // moving. The conditions are those under which
                        // `open_page_for_box` moves the holder.
                        let holder = ((unbreakable_box || flex_margins_base || avoids_by_property)
                            && !sets_page_flow
                            && !after_forced_break
                            && !inside_float
                            && !out_of_flow_subtree
                            && !is_float
                            && !flex_item_candidate
                            && !grid_item_candidate)
                            .then(|| leading_holder(document, cascade, body_id, node_id, true).0)
                            .filter(|&holder| holder != node_id)
                            // A box that only avoids a break inside it splits
                            // in columns rather than move them on (see
                            // `avoided_box_holder`).
                            .filter(|&holder| {
                                unbreakable_box
                                    || flex_margins_base
                                    || !document
                                        .ancestors(node_id)
                                        .take_while(|&id| id != holder)
                                        .chain(std::iter::once(holder))
                                        .any(|id| document.nodes[id].is_multicol_container())
                            });
                        candidate_ancestor_constraints(document, cascade, node_id, body_id, holder)
                    } else {
                        (false, false)
                    };
                let can_move_nested = !side_by_side_ancestor && !bounded_clipping_ancestor;
                let nested_avoid_candidate = (nested_candidate_base || cell_avoid_base)
                    && !inline_level
                    && is_block_flow_box
                    && avoids_break
                    && !side_by_side_ancestor
                    && (!bounded_clipping_ancestor || unbreakable_box);
                let nested_float_candidate = (nested_candidate_base && (is_float || clears_float)
                    || cell_clear_base)
                    && can_move_nested;
                let closing_edges_candidate = closing_edges_base && can_move_nested;
                // Not beside other boxes, but past a clip that the page end
                // cuts: the clip box moves along with the container.
                let flex_margins_candidate = flex_margins_base && !side_by_side_ancestor;
                let run_tail_candidate = run_tail_base && can_move_nested;
                // A block that a run of boxes tied by avoided breaks starts at,
                // inside the first box of the run.
                let run_break_candidate = run_break_base && can_move_nested;
                let line_after_block_candidate =
                    line_after_block_base && !bounded_clipping_ancestor;
                let line_after_block = line_after_block.filter(|_| line_after_block_candidate);
                let avoided_unit = avoided_unit.filter(|_| direct_body_child || can_move_nested);
                let first_grid_row =
                    first_grid_row.filter(|_| direct_body_child || can_move_nested);
                let leading_piece = leading_piece.filter(|_| direct_body_child || can_move_nested);
                let leading_block_piece =
                    leading_block_piece.filter(|_| direct_body_child || can_move_nested);
                let column_replaced_piece =
                    column_replaced_piece.filter(|_| direct_body_child || can_move_nested);
                // A box that sticks out below its parent has no tail to keep.
                let lone_box_candidate = lone_tail
                    .filter(|_| can_move_nested)
                    .map(|lone| lone.tail.max(0.0));
                // How far the floats beside the box of such a block, and those
                // of the blocks it ends, reach below the box. Both ends are
                // measured at their normal-flow positions, as the page fit is.
                let closing_edges_float_reach =
                    if let Some(lone) = lone_tail.filter(|_| can_move_nested) {
                        let box_end = candidate_raw_y + node.layout.size.height
                            - lone.tail.max(0.0)
                            - accumulated_relative_offset_y(document, cascade, body_id, node_id);
                        lone.float_reach
                            .max((frame.float_bottom - box_end).max(0.0))
                    } else if closing_edges_candidate || run_tail_candidate {
                        let span = match frame.tie {
                            AvoidTie::RunStart { span, .. } => span,
                            _ => 0.0,
                        };
                        let box_end = candidate_raw_y + candidate_height + span
                            - accumulated_relative_offset_y(document, cascade, body_id, node_id);
                        (frame.float_bottom - box_end).max(0.0)
                    } else {
                        0.0
                    };
                // The edges also follow the lines of a paragraph that breaks
                // between them. With no box before it in the block that the
                // edges close, the page cannot break before the paragraph at
                // a better place than between its lines (class C against
                // class B), so the orphans it cannot keep do not stop it.
                // A paragraph with a box before it that moves whole to the
                // next page for them has no such break there: its box ends up
                // at the top of that page, and its lines split when the edges
                // still run past the page end.
                //
                // A paragraph that asks to avoid a break inside it takes part
                // too: it moves whole with the edges when they fit a page
                // together, and its lines split only when they cannot (see
                // `keep_lines_together`). A line that no text places, or a
                // box that no break goes through, never splits. A block laid
                // out after the last line is what the edges follow instead.
                let lines_closing_edges = if lines_shape
                    && can_move_nested
                    && (!unbreakable || frame.lone_in_edges || closing_edges_shape)
                    && unplaced_line.is_none()
                    && !unbreakable_box
                    && node.is_ifc_root()
                    && !block_after_last_line(document, node_id)
                    && !matches!(frame.tie, AvoidTie::RunStart { .. })
                {
                    frame.closing_edges + node.layout.padding.bottom + node.layout.border.bottom
                } else {
                    0.0
                };
                let selected_tail = frame.item_tail.is_some_and(|tail| tail.item == node_id);
                let extra_tail_candidate = selected_tail
                    && (node.display() == DisplayValue::TableCaption
                        || (inside_flex
                            && !flex_item_candidate
                            && is_plain_block_box(document, node_id)));
                let margin_candidate = margin_shape && can_move_nested;
                // The last box of a block whose `min-height` leaves room below
                // it, which moved content may still fill. A paragraph too
                // short to break between its lines moves whole with the
                // edges, as a box does.
                let min_height_edges_candidate = frame.min_height_edges.filter(|_| {
                    in_flow_block
                        && can_move_nested
                        && break_before_available()
                        && breaks_only_around(document, cascade, node_id, self.has_named_pages)
                });
                let is_break_candidate = !is_body
                    && document.nodes[node_id].display() != DisplayValue::Contents
                    && !inside_float
                    && (margin_candidate
                        || (participates_in_flow
                            && direct_body_child
                            && (explicit_page_name.is_none() || inline_level))
                        || nested_avoid_candidate
                        || nested_float_candidate
                        || closing_edges_candidate
                        || flex_margins_candidate
                        || min_height_edges_candidate.is_some()
                        || lone_box_candidate.is_some()
                        || run_tail_candidate
                        || run_break_candidate
                        || line_after_block_candidate
                        || avoided_unit.is_some()
                        || leading_piece.is_some()
                        || leading_block_piece.is_some()
                        || column_replaced_piece.is_some()
                        || first_grid_row.is_some()
                        || is_tall_direct_absolute
                        || table_row_candidate
                        || table_caption_candidate
                        || extra_tail_candidate
                        || flex_item_candidate
                        || is_rendered_leaf
                        || grid_item_candidate
                        || after_forced_break
                        || sets_page_flow);
                // A box is measured against a clipping ancestor's edge like
                // text: one past it is not a candidate, and one running over
                // it occupies only the part above.
                let pushed =
                    is_break_candidate && (sets_page_flow || candidate_raw_y < frame.clip_end);
                // A contents box has no box of its own: its children stand
                // among its parent's.
                if !is_body
                    && !is_float
                    && participates_in_flow
                    && !inside_float
                    && node.display() != DisplayValue::Contents
                {
                    let bottom = (!pushed && in_flow_block_level).then(|| {
                        candidate_raw_y + node.layout.size.height
                            - super::layout_relative_offset_y(document, cascade, body_id, node_id)
                    });
                    self.follow_sibling(node_id, pushed, bottom);
                }
                if pushed {
                    self.out.push(PageCandidate {
                        node_id,
                        raw_y: candidate_raw_y,
                        height: if sets_page_flow {
                            candidate_height
                        } else {
                            candidate_height.min(frame.clip_end - candidate_raw_y)
                        },
                        context: BoxContext::element(
                            CandidateFlowContext {
                                is_float_descendant: inside_float,
                                is_out_of_flow_descendant: out_of_flow_subtree,
                                page_name_in_flow: !float_subtree
                                    && (!out_of_flow_subtree || own_page_name.is_some()),
                                // The edges that close the boxes around a
                                // paragraph go with its line after a block.
                                lines_closing_edges: if line_after_block.is_some() {
                                    frame.closing_edges
                                        + node.layout.padding.bottom
                                        + node.layout.border.bottom
                                } else {
                                    0.0
                                },
                            },
                            ElementContext {
                                leading_inline_line: None,
                                is_rendered_leaf,
                                is_direct_body_element: direct_body_child,
                                is_table_row: table_row_candidate,
                                is_table_caption: table_caption_candidate,
                                is_flex_item: flex_item_candidate,
                                is_grid_item: grid_item_candidate,
                                is_named: own_page_name.is_some(),
                                // The clips between an absolutely positioned
                                // or fixed box and its containing block do
                                // not cut it.
                                clip_box: frame.lines_clip_box.filter(|_| {
                                    !matches!(
                                        node.position(),
                                        PositionValue::Absolute | PositionValue::Fixed
                                    )
                                }),
                                deferred_named_break_after,
                                unplaced_line: unplaced_line.or(line_after_block),
                                unbreakable: unbreakable_box,
                                unbreakable_room,
                                avoided_break_before: avoided_break_passed_on,
                                closing_edges: if closing_edges_candidate
                                    || lone_box_candidate.is_some()
                                {
                                    frame.closing_edges
                                } else {
                                    0.0
                                },
                                capped_edges_block: frame.capped_edges_block.filter(|_| {
                                    closing_edges_candidate || lone_box_candidate.is_some()
                                }),
                                closing_edges_inside: lone_box_candidate.unwrap_or(0.0),
                                closing_edges_span: match frame.tie {
                                    AvoidTie::RunStart { span, .. } if closing_edges_candidate => {
                                        span
                                    }
                                    _ => 0.0,
                                },
                                closing_edges_last_box: match frame.tie {
                                    AvoidTie::RunStart { last, .. } if closing_edges_candidate => {
                                        u32::try_from(last).ok()
                                    }
                                    _ => None,
                                },
                                avoided_unit,
                                leading_piece_end: leading_piece,
                                first_grid_row,
                                leading_block_piece_end: leading_block_piece,
                                leading_column_replaced_piece_end: column_replaced_piece,
                                run_tail_edges: if run_tail_candidate {
                                    frame.closing_edges
                                } else {
                                    0.0
                                },
                                item_tail_edges: frame
                                    .item_tail
                                    .filter(|tail| {
                                        tail.item == node_id || tail.fallback == Some(node_id)
                                    })
                                    .map_or(0.0, |tail| {
                                        (tail.container_bottom
                                            - (candidate_raw_y + candidate_height)
                                            + tail.inherited)
                                            .max(0.0)
                                    }),
                                lone_run_head_offset: tie_offset
                                    .filter(|_| !break_before_available()),
                                closing_edges_float_reach,
                                min_height_edges: min_height_edges_candidate,
                            },
                        ),
                        page_name: page_name_to_owned(page_name),
                        named_flex_context,
                        ifc_root: None,
                    });
                }
                // An atomic inline-level box or an orthogonal flow is
                // monolithic: like a float, its content offers no class A
                // break points and its page names do not open pages. A
                // blockified flex or grid item is no longer atomic.
                let monolithic = !is_body
                    && is_monolithic_for_pagination_with_transposed_axes(
                        document,
                        cascade,
                        node_id,
                        document.context.pagination_on_transposed_axes,
                    );
                // An absolutely positioned box is clipped by its containing
                // block and the ancestors above it, not by the boxes between
                // them. One that starts past the edge of such a clip is hidden
                // whole, but still starts the page it is on: it keeps the first
                // sliver of its content. A fixed box is not clipped here. A
                // document paginated on transposed axes is not measured at
                // all: the overflow that clips its flow is not the one on y.
                let clip = ClipBounds {
                    end: frame.clip_end,
                    fixed_end: frame.fixed_clip_end,
                    absolute_end: frame.absolute_clip_end,
                    lines_end: frame.lines_clip_end,
                    lines_owner: frame.lines_clip_owner,
                    lines_clip: frame.lines_clip_box,
                }
                .descend(document, cascade, node_id, raw_y, is_body);
                if clip.lines_clip == Some(node_id) {
                    self.enclosing_line_clips
                        .insert(node_id, frame.lines_clip_box);
                }
                let (clip_end, fixed_clip_end, absolute_clip_end) =
                    (clip.end, clip.fixed_end, clip.absolute_end);
                let (lines_clip_end, lines_clip_owner, lines_clip_box) =
                    (clip.lines_end, clip.lines_owner, clip.lines_clip);
                let child_order = pagination_child_order(document, cascade, node_id);
                // The closing edges of this block, and of the blocks that it
                // ends in turn, follow its last box in the flow: a float after
                // a box keeps it from being the last. A `display: contents`
                // element has no edges of its own, but lends its boxes to the
                // flow of its parent, so its last box carries the edges of the
                // blocks that it ends.
                let contents = node.display() == DisplayValue::Contents;
                let own_closing_edges = if contents {
                    Some(0.0)
                } else {
                    closing_edges_of(document, cascade, body_id, node_id)
                };
                // The edges of the blocks this one ends are not below a box that
                // only starts a run of boxes tied by avoided breaks: the rest
                // of the run lies between. Nor are they below the last box of a
                // block whose own height does not follow its content: they
                // follow that block's border box.
                let inherited_edges = if matches!(frame.tie, AvoidTie::RunStart { .. })
                    || (frame.closing_edges > 0.0
                        && !contents
                        && !passes_closing_edges(document, cascade, body_id, node_id))
                {
                    0.0
                } else {
                    frame.closing_edges
                };
                let closing_chain = own_closing_edges
                    .map(|own| own + inherited_edges)
                    .filter(|&edges| edges > 0.0);
                // The last item of a flex container that stacks its items, or
                // the last row of a table, is the box that the edges closing
                // the container follow: its own bottom padding and border, and
                // those of the blocks it ends. The page may break between the
                // items or rows instead of between the last of them and the
                // edges.
                let item_tail = (in_flow_container
                    && !is_body
                    && !document.context.pagination_on_transposed_axes
                    && height_follows_content(document, node_id)
                    && node.layout.padding.bottom + node.layout.border.bottom + inherited_edges
                        > 0.0)
                    .then(|| container_item_tail(document, cascade, node_id, self.has_named_pages))
                    .flatten()
                    .filter(|_| {
                        let (side_by_side, bounded) = candidate_ancestor_constraints(
                            document, cascade, node_id, body_id, None,
                        );
                        !side_by_side && !bounded
                    })
                    .map(|(item, fallback)| {
                        if node.display() == DisplayValue::Flex {
                            for ancestor in document.ancestors(item).take_while(|&id| id != node_id)
                            {
                                self.inner_tail_paths.insert(ancestor, item);
                            }
                        }
                        ItemTail {
                            item,
                            fallback,
                            container_bottom: raw_y + node.layout.size.height,
                            inherited: inherited_edges,
                        }
                    });
                // Row groups pass on their table's selected row. Plain blocks
                // pass on an inner column tail only along its prepared path.
                let child_item_tail = if matches!(
                    document.nodes[node_id].display(),
                    DisplayValue::TableRowGroup
                        | DisplayValue::TableHeaderGroup
                        | DisplayValue::TableFooterGroup
                ) || (inside_flex
                    && (is_plain_block_box(document, node_id)
                        || node.display() == DisplayValue::Contents)
                    && frame.item_tail.is_some_and(|tail| {
                        self.inner_tail_paths.get(&node_id) == Some(&tail.item)
                    })) {
                    frame.item_tail
                } else {
                    item_tail
                };
                // For each child of a block container, its last in-flow box and
                // whether it floats: they tell the box before each child, and
                // the last box that the closing edges follow. The children of
                // other boxes that lay them out in block flow need the box
                // before them only to take a forced break, so they get it only
                // when the document forces one. The children of a `display:
                // contents` element stand among its parent's (see
                // `previous_box`), so they get it as those do.
                // The edges of a block whose `min-height` leaves room below
                // its last box, which they follow once moved content fills it.
                let min_height_room = own_closing_edges
                    .is_none()
                    .then(|| super::min_height_room_edges(document, cascade, body_id, node_id))
                    .flatten();
                let flow_flags: Vec<(Option<usize>, bool)> = if closing_chain.is_some()
                    || min_height_room.is_some()
                    || (contents && (frame.previous_box.is_some() || child_order.len() > 1))
                    || (child_order.len() > 1
                        && (moves_later_siblings(document, node_id)
                            || (self.forces_break_after
                                && lays_out_children_in_block_flow(document, node_id))))
                {
                    child_flow_flags(document, cascade, &child_order)
                } else {
                    Vec::new()
                };
                let last_in_flow = flow_flags
                    .iter()
                    .rposition(|&(in_flow, _)| in_flow.is_some());
                // The lowest normal-flow margin-box bottom of this block's
                // floats. A float crossing a page edge keeps the last box
                // where it is; inherited bounds constrain that fit too.
                let owned_floats_bottom = flow_flags
                    .iter()
                    .zip(child_order.iter().copied())
                    .filter(|&(&(_, floated), _)| floated)
                    .map(|(_, child_id)| {
                        let layout = &document.nodes[child_id].layout;
                        pagination_child_parent_y(document, node_id, child_id, raw_y)
                            + layout.location.y
                            + layout.size.height
                            + layout.margin.bottom
                            - accumulated_relative_offset_y(document, cascade, body_id, child_id)
                    })
                    .fold(f32::NEG_INFINITY, f32::max);
                let floats_bottom = if inherited_edges > 0.0 {
                    owned_floats_bottom.max(frame.float_bottom)
                } else {
                    owned_floats_bottom
                };
                let enclosed_float_end = (owned_floats_bottom.is_finite()
                    && holds_floats(document, cascade, node_id))
                .then(|| {
                    owned_floats_bottom - raw_y
                        + accumulated_relative_offset_y(document, cascade, body_id, node_id)
                });
                // The margin below the last box sits between it and the padding
                // and border of this block, unless it collapses through that
                // block (when it has neither) or through the box itself (when
                // that box is empty and stays with the box before it).
                let edges_after_box = |index: usize| {
                    closing_chain.map_or(0.0, |edges| {
                        let margin = flow_flags[index]
                            .0
                            .filter(|&last_box| {
                                own_closing_edges.is_some_and(|own| own > 0.0)
                                    && !super::collapsed_empty_box(document, cascade, last_box)
                            })
                            .map_or(0.0, |last_box| {
                                end_margin_through(document, cascade, last_box)
                            });
                        // Something else, such as a float that the block
                        // encloses, reaches below the last box: the edges
                        // follow that, not the box.
                        let content_end = node.layout.size.height
                            - node.layout.padding.bottom
                            - node.layout.border.bottom;
                        // The box sits at its normal-flow position, less the
                        // relative offset that layout applied.
                        let box_end = flow_flags[index].0.map_or(f32::INFINITY, |last_box| {
                            normal_flow_bottom(document, cascade, body_id, last_box) + margin
                        });
                        let enclosed_gap = if content_end > box_end + 0.001 {
                            // Only a float held by this block can move its
                            // closing edges below the last flow box. The
                            // inherited float bound also includes escaping
                            // floats and is only a page-fit constraint.
                            if enclosed_float_end
                                .is_some_and(|end| (end - content_end).abs() <= 0.001)
                            {
                                content_end - box_end
                            } else {
                                return 0.0;
                            }
                        } else {
                            0.0
                        };
                        // Keep negative margins until the complete tail has
                        // been measured, including any enclosed float gap.
                        (edges + margin + enclosed_gap).max(0.0)
                    })
                };
                let last_box_edges = last_in_flow.map_or(0.0, edges_after_box);
                // A block whose `min-height` leaves room below its last box
                // grows past it once pagination moves the content above that
                // box down; the edges then follow the box (see
                // `min_height_edges_move`).
                let min_height_edges = last_in_flow
                    .and_then(|index| flow_flags[index].0)
                    .and(min_height_room)
                    .map(|edges| (node_id, edges));
                // A last box that an avoided break ties to the box before it
                // goes to the next page with the boxes it is tied to, when
                // they all fit after the break before the first of them but
                // the edges do not.
                let avoid_chain =
                    last_in_flow
                        .filter(|_| closing_chain.is_some())
                        .and_then(|last| {
                            avoid_chain_before_last_box(
                                document,
                                cascade,
                                body_id,
                                &child_order,
                                &flow_flags,
                                last,
                                self.has_named_pages.then_some(|id| {
                                    named_page_subtree_has_name(
                                        document,
                                        cascade,
                                        id,
                                        document.context.pagination_on_transposed_axes,
                                        &mut self.named_page_subtree_cache,
                                    )
                                }),
                            )
                        });
                // Where the run starts, the block size from the bottom of its
                // first box to the bottom of its last, and the edges after it.
                // A run that starts at a paragraph breaking between its lines
                // puts the block size after the paragraph to the next page
                // with its last lines, as edges that close it.
                let chain_lines = match (avoid_chain, last_in_flow) {
                    (Some(AvoidChain::Lines { first }), Some(last)) if last_box_edges > 0.0 => {
                        let bottom = |index: usize| {
                            normal_flow_bottom(document, cascade, body_id, child_order[index])
                        };
                        let span = bottom(last) - bottom(first);
                        (span >= 0.0).then_some((first, span + last_box_edges))
                    }
                    _ => None,
                };
                let chain_run = match (avoid_chain, last_in_flow) {
                    (Some(AvoidChain::Run { first }), Some(last)) => {
                        let bottom = |index: usize| {
                            normal_flow_bottom(document, cascade, body_id, child_order[index])
                        };
                        let span = bottom(last) - bottom(first);
                        (span > 0.0 && last_box_edges > 0.0).then_some((
                            first,
                            span,
                            last_box_edges,
                            flow_flags[last].0.unwrap_or(child_order[last]),
                        ))
                    }
                    _ => None,
                };
                // Where the runs of boxes tied by avoided breaks that start
                // among the boxes of this block end.
                let (run_units, run_line_tails) =
                    if is_body && self.finds_nested_runs && !flow_flags.is_empty() {
                        // The runs of the body's own boxes are found from its
                        // candidates (see `avoided_run_ends`); only the lines a
                        // paragraph among them keeps with the boxes after it
                        // are found here.
                        let (_, tails) = nested_run_units(
                            document,
                            cascade,
                            body_id,
                            &child_order,
                            &flow_flags,
                            &mut HashMap::new(),
                        );
                        (Vec::new(), tails)
                    } else if !self.finds_nested_runs || !in_flow_block || is_body {
                        (Vec::new(), Vec::new())
                    } else if !flow_flags.is_empty() {
                        nested_run_units(
                            document,
                            cascade,
                            body_id,
                            &child_order,
                            &flow_flags,
                            &mut self.run_breaks,
                        )
                    } else if node.is_ifc_root()
                        && child_order
                            .iter()
                            .any(|&child| is_plain_block_box(document, child))
                    {
                        // A paragraph that mixes text and blocks lays them out as
                        // its lines, and its blocks follow a moved block through
                        // its line shifts. A run of blocks tied by avoided breaks
                        // among them still moves together; text between them is
                        // a box of the flow that ends the run.
                        let paragraph_flags = child_flow_flags(document, cascade, &child_order);
                        nested_run_units(
                            document,
                            cascade,
                            body_id,
                            &child_order,
                            &paragraph_flags,
                            &mut self.run_breaks,
                        )
                    } else {
                        (Vec::new(), Vec::new())
                    };
                // A `display: contents` element lends its boxes to the flow
                // of its parent: the box before its first one is the box
                // before the element. A free break or one that gives way may
                // follow it, and a page edge cuts the margin between them, as
                // it would between siblings. The boxes are walked in turn
                // whenever there is such a box.
                let mut previous_box = frame.previous_box.filter(|_| contents);
                let can_inherit_column_break = column_break_before
                    && !node.has_before_or_after_content
                    && (node.display() == DisplayValue::Contents
                        || is_plain_block_box(document, node_id));
                let mut column_box_before = false;
                // Whether any in-flow box comes before the child, text
                // included: `previous_box` forgets a text box, which is no box
                // to break from for some rules but is one for this.
                let mut box_before = previous_box.is_some();
                // Whether avoided breaks tie each box so far to the one before
                // it, from the first box on. Boxes of the parent's flow before
                // a `display: contents` element are not counted, so its boxes
                // are not taken as tied to the start.
                let mut tied_from_start = previous_box.is_none();
                // The child that starts the run of boxes tied by avoided
                // breaks which the box before the next child ends. The run of
                // the box before a `display: contents` element goes on into
                // its boxes.
                let mut run_head = if contents {
                    frame.tie_head.or(frame.previous_box)
                } else {
                    None
                };
                let is_block = |id: usize| {
                    is_plain_block_box(document, id) || is_plain_container_box(document, id)
                };
                // A block whose edges nothing around it carries is where the
                // boxes below it stop looking for a box before them.
                let child_scope_is_lone = (own_closing_edges.is_some_and(|own| own > 0.0)
                    && inherited_edges == 0.0)
                    || frame.lone_in_edges;
                let flow_caption = inside_table
                    && is_flow_table_caption(document, cascade, node_id, roots.element);
                for (index, child_id) in child_order.iter().copied().enumerate() {
                    let at_chain_start = chain_run.filter(|&(first, ..)| index == first);
                    let lines_tail = chain_lines
                        .filter(|&(first, _)| index == first)
                        .map(|(_, tail)| tail);
                    let block_edges = match (at_chain_start, lines_tail) {
                        (Some((_, _, edges, _)), _) => edges,
                        (None, Some(tail)) => tail,
                        (None, None) if last_in_flow == Some(index) => last_box_edges,
                        (None, None) => 0.0,
                    };
                    // A paragraph whose last lines a run of tied boxes after
                    // it takes along carries that run like closing edges.
                    let run_line_tail = run_line_tails.get(index).copied().unwrap_or(0.0);
                    let child_closing_edges = block_edges.max(run_line_tail);
                    // A block with edges of its own whose boxes are all tied to
                    // the first one has no break between boxes left to take: its
                    // last box moves alone with them, the avoided break giving
                    // way. When the edges of a block around it follow it too, a
                    // break before this block is a better one, which is left to
                    // the rules for that.
                    let tie = if let Some((_, span, _, last)) = at_chain_start {
                        AvoidTie::RunStart { span, last }
                    } else if matches!(avoid_chain, Some(AvoidChain::Start))
                        && own_closing_edges.is_some_and(|own| own > 0.0)
                        && inherited_edges == 0.0
                        && last_in_flow == Some(index)
                    {
                        AvoidTie::Waived
                    } else {
                        AvoidTie::None
                    };
                    // The last box of a run that avoided breaks tie to the boxes
                    // before it, from a free break on.
                    // A paragraph with lines to break between breaks there
                    // instead, when a box of the run before it takes block
                    // space: a break that leaves fewer lines than `widows`
                    // beats one that an avoided break forbids (see
                    // `edge_break_lines`).
                    let tail_splits = match avoid_chain {
                        Some(AvoidChain::Run { first }) => {
                            last_in_flow == Some(index)
                                && run_paragraph(document, cascade, child_id).is_some_and(
                                    |paragraph| paragraph_has_lines_to_split(document, paragraph),
                                )
                                && flow_flags[first..index].iter().any(|&(in_flow, floated)| {
                                    in_flow.is_some_and(|id| {
                                        !floated && !super::takes_no_block_space(document, id)
                                    })
                                })
                        }
                        _ => false,
                    };
                    let run_tail = matches!(avoid_chain, Some(AvoidChain::Run { .. }))
                        && last_in_flow == Some(index)
                        && !tail_splits;
                    let child_previous_box = previous_box;
                    let child_box_before = box_before;
                    let child_column_break = can_inherit_column_break && !column_box_before;
                    if can_inherit_column_break && !column_box_before {
                        column_box_before = flow_flags.get(index).map_or_else(
                            || column_child_has_flow_box(document, cascade, child_id),
                            |(in_flow, _)| in_flow.is_some(),
                        );
                    }
                    let child_tied = self.finds_nested_runs
                        && child_previous_box.is_some_and(|previous| {
                            boxes_are_tied(document, cascade, previous, child_id)
                        });
                    // A float between the boxes is no box in their flow, and
                    // text before the child is left to the rules for it.
                    let child_tied_to_start = tied_from_start
                        && flow_flags.get(index).and_then(|&(in_flow, _)| in_flow)
                            == Some(child_id)
                        && is_block(child_id)
                        && child_previous_box.is_some_and(|previous| {
                            is_block(previous)
                                && boxes_are_tied(document, cascade, previous, child_id)
                        });
                    box_before |= matches!(flow_flags.get(index), Some((Some(_), _)));
                    if let Some(&(Some(last_box), _)) = flow_flags.get(index) {
                        previous_box = Some(last_box);
                        tied_from_start = !child_box_before || child_tied_to_start;
                        if !child_tied {
                            run_head = (last_box == child_id).then_some(child_id);
                        }
                    }
                    let child_parent_y =
                        pagination_child_parent_y(document, node_id, child_id, raw_y);
                    self.collect(
                        child_id,
                        CandidateFrame {
                            parent_abs_y: child_parent_y,
                            direct_body_child: candidate_parent(document, child_id)
                                == Some(roots.contents_box),
                            parent_height: node.layout.size.height.max(0.0),
                            inherited_page_name: child_page_name,
                            // The children of a caption that stacks in plain block
                            // flow are placed in that flow, as those of a block.
                            inside_table: (inside_table && !flow_caption)
                                || document.anonymous_table_of(child_id).is_some()
                                || matches!(
                                    document.nodes[node_id].display(),
                                    DisplayValue::Table
                                        | DisplayValue::InlineTable
                                        | DisplayValue::TableRowGroup
                                        | DisplayValue::TableHeaderGroup
                                        | DisplayValue::TableFooterGroup
                                        | DisplayValue::TableRow
                                ),
                            flex_column_parent: column_flex
                                || (document.nodes[node_id].display() == DisplayValue::Contents
                                    && flex_column_parent),
                            grid_single_column_parent: document.nodes[node_id].formatting_context
                                == FormattingContext::Grid
                                && document.nodes[node_id].grid_column_count == 1,
                            inside_flex: inside_flex
                                || (document.nodes[node_id].display().is_flex_container()
                                    && !holds_sole_table
                                    && !(document.nodes[child_id].display()
                                        == DisplayValue::Table
                                        && stacks_tables)),
                            named_flex_context: if column_flex {
                                Some(node_id)
                            } else if is_flex {
                                None
                            } else {
                                named_flex_context
                            },
                            inside_float: float_subtree || monolithic,
                            inside_out_of_flow: out_of_flow_subtree,
                            clip_end,
                            fixed_clip_end,
                            absolute_clip_end,
                            lines_clip_end,
                            lines_clip_owner,
                            lines_clip_box,
                            closing_edges: child_closing_edges,
                            min_height_edges: min_height_edges
                                .filter(|_| last_in_flow == Some(index)),
                            capped_edges_block: (child_closing_edges > 0.0
                                && at_chain_start.is_none()
                                && lines_tail.is_none()
                                && run_line_tail <= 0.0)
                                .then(|| {
                                    if contents || height_follows_content(document, node_id) {
                                        frame.capped_edges_block.filter(|_| inherited_edges > 0.0)
                                    } else {
                                        Some(node_id)
                                    }
                                })
                                .flatten(),
                            previous_box: child_previous_box,
                            box_before: child_box_before,
                            column_break_before: child_column_break,
                            lone_in_edges: (!child_box_before || child_tied_to_start)
                                && child_scope_is_lone,
                            tied_to_start: child_tied_to_start,
                            lines_closing_edges,
                            // The last box of a tied run that moved to the
                            // next page breaks between its lines there
                            // rather than before itself, away from the run.
                            lines_tail: lines_tail.is_some()
                                || run_line_tail > 0.0
                                || (chain_run.is_some() && tail_splits),
                            tie,
                            run_unit: run_units.get(index).copied().flatten(),
                            run_tail,
                            item_tail: child_item_tail,
                            tie_head: child_tied.then_some(run_head).flatten(),
                            float_bottom: if block_edges > 0.0 {
                                floats_bottom
                            } else {
                                f32::NEG_INFINITY
                            },
                        },
                    )?;
                }
                // The padding and border at the bottom of a block whose height
                // is that of its flow content begin where that content ends
                // (at the top of the block when it holds none), and the page
                // breaks before them when they do not fit below it on its
                // page. Those of a block whose set height leaves room below
                // its content begin at the end of its content box instead. A monolithic box, such as an image, keeps its edges.
                // Like the page fit of the boxes, the end is that of the normal
                // flow: a relative offset only moves the paint. In an item of a
                // flex container or of a grid of columns, the edges move that
                // item and the container but not the items beside it. They stay
                // in columns, whose balance the move would change.
                if in_container_flow
                    && !monolithic
                    && let Some(edges) = content_bottom_edges(document, cascade, body_id, node_id)
                    && candidate_raw_y + node.layout.size.height <= frame.clip_end
                    && let Some(beside) = edges_break_container(document, node_id, body_id)
                {
                    let offset = accumulated_relative_offset_y(document, cascade, body_id, node_id);
                    let bottom = candidate_raw_y + node.layout.size.height;
                    // The box around a fixed clip does not grow as the edges are
                    // pushed down inside it, so it hides more of them.
                    let (clip_end, clip_room) = if frame.fixed_clip_end.is_finite() {
                        (frame.fixed_clip_end, frame.fixed_clip_end - bottom)
                    } else {
                        (f32::INFINITY, f32::INFINITY)
                    };
                    let (margin_above, margin_kept, margin_reach) =
                        margin_above_edges(document, cascade, body_id, node_id);
                    self.edges_breaks.push(EdgesBreak {
                        node_id,
                        before: self.out.len(),
                        raw_y: bottom - edges - offset,
                        edges,
                        margin_above,
                        margin_kept,
                        margin_reach,
                        clip_end,
                        clip_room,
                        beside,
                    });
                }
            }
            _ => {} // cov:ignore: non-rendered node kinds are filtered by the document invariant.
        }
        Ok(())
    }
}

/// Whether the text node `node_id` is a line of a vertical paragraph inside an
/// orthogonal flow of a document paginated along y. Its lines are columns
/// across the flow, with their block offsets along x, and no page break goes
/// through the orthogonal flow (see [`box_is_unbreakable`]), so the text moves
/// with that box and stands for no break of its own.
fn text_moves_with_orthogonal_flow(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> bool {
    if document.context.pagination_on_transposed_axes
        || !document.nodes[node_id].under_vertical_writing
    {
        return false;
    }
    let Some(root) = document
        .ifc_root_of(node_id)
        .filter(|&root| root != node_id)
    else {
        return false;
    };
    if !document
        .ifc_layout_node(root)
        .and_then(|root| root.ifc_writing_mode())
        .is_some_and(|mode| mode.is_vertical())
    {
        return false;
    }
    std::iter::successors(Some(root), |&id| document.parent_of(id))
        .take_while(|&id| id != body_id)
        .any(|id| super::super::page::establishes_orthogonal_flow(document, cascade, id))
}

/// The block size of the part of the unbreakable box `node_id` that lies
/// inside the blocks it begins, when it runs out past the bottom of one of
/// them: how far below its normal-flow top the first of their bottoms comes.
/// The walk goes up as [`super::leading_holder`] does, and takes in the box
/// the walk stops below as well, since the box begins that one too. `None`
/// when the box begins no block, when it ends inside all of those it begins,
/// or when something comes before it: then there is a break before the box,
/// which moves whole.
fn unbreakable_room(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    node_id: usize,
) -> Option<f32> {
    // Positions in the flow, before any relative offset: the page breaks
    // before the offsets move the boxes.
    let flow_y = |id: usize| {
        super::current_abs_y(document, id)
            - accumulated_relative_offset_y(document, cascade, body_id, id)
    };
    let node = &document.nodes[node_id];
    let top = flow_y(node_id);
    let height = node.layout.size.height;
    // A negative bottom margin lets the blocks around the box end above its
    // bottom without holding their height below their content.
    let overflows = |bottom: f32| bottom - top < height + node.layout.margin.bottom.min(0.0) - 0.01;
    let mut room = f32::INFINITY;
    let mut child = node_id;
    for ancestor in document
        .layout_ancestors(node_id)
        .take_while(|&id| id != body_id)
    {
        if renders_in_flow_before(document, ancestor, child) {
            break;
        }
        child = ancestor;
        if document.nodes[ancestor].display() == DisplayValue::Contents {
            continue;
        }
        // Positions inside an inline formatting context or out of the flow
        // are not those of the flow: the walk stops below such a box.
        if super::has_unusual_coordinates(document, ancestor) {
            break;
        }
        room = room.min(flow_y(ancestor) + document.nodes[ancestor].layout.size.height - top);
        if !is_plain_block_box(document, ancestor)
            || document.nodes[ancestor].is_multicol_container()
            || is_floating_box_for_pagination(document, cascade, ancestor)
        {
            break;
        }
    }
    if !overflows(top + room) {
        return None;
    }
    // The walk cannot see what an anonymous block holds, so a box further
    // below the outermost block it begins than the edges and margins between
    // them put it follows something there.
    (top - flow_y(child) <= super::leading_space_between(document, cascade, node_id, child) + 0.01)
        .then_some(room.max(0.0))
}

/// Whether something in the flow of `parent` comes before `child`: a box
/// placed out of the flow leaves no break before it.
fn renders_in_flow_before(document: &Document, parent: usize, child: usize) -> bool {
    document.nodes[parent]
        .layout_children()
        .iter()
        .take_while(|&&sibling| sibling != child)
        .any(|&sibling| {
            super::sibling_renders(document, sibling)
                && !(document.nodes[sibling].kind() == NodeKind::Element
                    && matches!(
                        document.nodes[sibling].position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    ))
        })
}

/// Whether `parent` lays its children out one under the other in block flow,
/// so that a forced break after one child falls before the next. Besides the
/// blocks that move their later siblings, that is a block that also holds
/// inline content, a table cell, and a table or table row whose block
/// children an anonymous cell holds (CSS Tables 3, section 3.9). An inline
/// block and a multi-column container lay their children out otherwise.
fn lays_out_children_in_block_flow(document: &Document, parent: usize) -> bool {
    let node = &document.nodes[parent];
    matches!(
        node.display(),
        DisplayValue::Block
            | DisplayValue::FlowRoot
            | DisplayValue::ListItem
            | DisplayValue::TableCaption
            | DisplayValue::TableCell
            | DisplayValue::Table
            | DisplayValue::TableRow
    ) && node.multicol.is_none()
}
