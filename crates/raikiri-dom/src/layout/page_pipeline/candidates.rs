//! Collection of the page candidates of a laid-out flow: the boxes and
//! texts pagination may move to a later page, in source order.

use super::super::box_tree::BoxTree;
use super::super::context::LayoutSession;
use super::{
    AvoidChain, BoxParentScope, CascadeResult, DisplayValue, Document, FlexDirectionValue,
    FloatValue, FormattingContext, LayoutError, NodeFlags, NodeKind, PageLayoutControl,
    PositionValue, TextLineIndex, avoid_chain_before_last_box, avoided_run_head,
    box_forces_break_after, boxes_are_tied, breaks_only_around, candidate_ancestor_constraints,
    clears_float_side, closing_edges_of, first_owned_text_line_top, in_inset_out_of_flow_subtree,
    is_block_level_flow_box, is_floating_box_for_pagination,
    is_monolithic_for_pagination_with_transposed_axes, is_plain_block_box, is_plain_container_box,
    leading_piece_end, lone_box_tail, moves_later_siblings, nested_run_units, normal_flow_bottom,
    page_applies_to_box, page_break_is_forced, page_edge_constraint, page_name_to_owned,
    pagination_child_order, pagination_child_parent_y, paragraph_lines_keep_together,
    positioned_owned_text_line_bounds, propagated_start_page_name_with_transposed_axes,
    push_in_flow_boxes_rev, selected_page_name, takes_no_block_space, unplaced_line_end,
};
use std::collections::HashMap;

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

#[derive(Clone, Copy, Default)]
pub(super) struct ElementContext {
    pub(super) is_rendered_leaf: bool,
    pub(super) is_direct_body_element: bool,
    pub(super) is_table_row: bool,
    pub(super) is_flex_item: bool,
    pub(super) is_grid_item: bool,
    pub(super) is_named: bool,
    pub(super) deferred_named_break_after: bool,
    /// How far below the border-box top the line of a paragraph of one line
    /// of generated text ends: the box is kept whole by that line.
    pub(super) unplaced_line_end: Option<f32>,
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
    /// How far below the border-box top the run of boxes tied by avoided
    /// breaks ends that the box starts among the boxes of its parent, or that
    /// it holds at the start of its own boxes. The box moves to the next page
    /// with the run when the run would cross a page edge (see
    /// [`super::avoided_run_ends`]).
    pub(super) avoided_unit: Option<f32>,
    /// For a flex container or grid with padding or border above its items:
    /// how far below the border-box top the first piece of its content ends.
    /// The box moves to the next page when that crosses a page edge, so that
    /// the edges do not stay behind alone (see [`super::leading_edges_move`]).
    pub(super) leading_piece_end: Option<f32>,
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

    pub(super) fn is_text(self) -> bool {
        matches!(self, Self::Text(_))
    }

    pub(super) fn is_rendered_leaf(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_rendered_leaf)
    }

    pub(super) fn is_direct_body_element(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_direct_body_element)
    }

    pub(super) fn is_table_row(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_table_row)
    }

    pub(super) fn is_flex_item(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_flex_item)
    }

    pub(super) fn is_grid_item(self) -> bool {
        self.element_attributes()
            .is_some_and(|attributes| attributes.is_grid_item)
    }

    pub(super) fn unplaced_line_end(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.unplaced_line_end)
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

    pub(super) fn avoided_unit(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.avoided_unit)
    }

    pub(super) fn leading_piece_end(self) -> Option<f32> {
        self.element_attributes()
            .and_then(|attributes| attributes.leading_piece_end)
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
    /// The block size of the bottom padding and border of the ancestors that
    /// end with the node: it is the last box in the flow of each of them, and
    /// their heights follow their content.
    closing_edges: f32,
    /// The box before the node in the flow of its parent, which a break can
    /// fall after, when the node ends the blocks that `closing_edges` counts.
    previous_box: Option<usize>,
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
    /// How the node is tied to the boxes around it by avoided breaks, when it
    /// ends its block or starts the run of boxes that does.
    tie: AvoidTie,
    /// How far below its border-box top the run of boxes tied by avoided
    /// breaks ends that the node starts among the boxes of its parent.
    run_unit: Option<f32>,
}

/// How a box is tied by avoided page breaks to the others of the run that ends
/// its block, with the edges of the block in `CandidateFrame::closing_edges`.
#[derive(Clone, Copy, Default, PartialEq)]
enum AvoidTie {
    #[default]
    None,
    /// The box starts the run, which moves to the next page with the edges
    /// after it when they do not fit: `span` is the block size from the bottom
    /// of the box to the bottom of the run.
    RunStart { span: f32 },
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
            closing_edges: 0.0,
            previous_box: None,
            lone_in_edges: true,
            tied_to_start: false,
            lines_closing_edges: 0.0,
            tie: AvoidTie::None,
            run_unit: None,
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
    control: &'a PageLayoutControl<'c>,
    named_page_subtree_cache: HashMap<usize, bool>,
    has_named_pages: bool,
    /// Whether runs of boxes tied by avoided breaks may be looked for inside
    /// blocks: some box avoids a break before or after it, and no box names a
    /// page, which would break the flow between boxes of its own.
    finds_nested_runs: bool,
    /// Whether some box forces a break after itself, so that a box after one
    /// may take it.
    forces_break_after: bool,
}

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
            control,
            named_page_subtree_cache: HashMap::new(),
            has_named_pages,
            finds_nested_runs: !has_named_pages
                && cascade.computed.iter().any(|computed| {
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
            forces_break_after: cascade.computed.iter().any(|computed| {
                matches!(
                    computed.break_after,
                    raikiri_style::property::BreakBetween::Page
                        | raikiri_style::property::BreakBetween::Always
                )
            }),
        }
    }

    /// Collect the page candidates of the flow under the body into `out`,
    /// in source order. `root_flow_offset` is where the body's border box
    /// starts in the page flow.
    pub(super) fn collect_flow(&mut self, root_flow_offset: f32) -> Result<(), LayoutError> {
        self.collect(self.roots.element, CandidateFrame::root(root_flow_offset))
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
                        crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
                    )
                {
                    // A text node of an ifc paragraph has no layout of its own:
                    // it stands for the lines it owns, measured from the root's
                    // content box. The inline elements between the root and
                    // the text pass the root's border-box y down unchanged, so
                    // `parent_abs_y` is the root's. A text node that is a root
                    // itself (an anonymous flex or grid item) has a box of its
                    // own, located like any other.
                    self.text_lines
                        .prepare_for(document, node_id, || control.check_aborted())?;
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
                    // Past a clipping ancestor's edge the text paints nowhere
                    // and gives the flow no page; a text running over the edge
                    // occupies only the part above it.
                    if raw_y < frame.clip_end {
                        self.out.push(PageCandidate {
                            node_id,
                            raw_y,
                            height: height.min(frame.clip_end - raw_y),
                            context: BoxContext::text(CandidateFlowContext {
                                is_float_descendant: inside_float,
                                is_out_of_flow_descendant: inside_out_of_flow,
                                page_name_in_flow: !inside_float && !inside_out_of_flow,
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
                        child_frame.tie = AvoidTie::None;
                        child_frame.run_unit = None;
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
                let column_flex = is_flex
                    && matches!(
                        computed.flex_direction,
                        FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                    );
                let deferred_named_break_after = self.has_named_pages
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
                let row_flex = is_flex && !column_flex;
                let flex_names_stay_on_items = column_flex
                    || (row_flex && !deferred_named_break_after)
                    || (named_flex_context.is_some() && !deferred_named_break_after);
                let page_name = if has_propagated_page_name && !flex_names_stay_on_items {
                    propagated_page_name
                } else {
                    own_page_name.or(frame.inherited_page_name)
                };
                let child_page_name = own_page_name.or(frame.inherited_page_name);
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
                // The one line of generated text of a block is placed by no
                // text candidate, so the block moves whole instead. Content
                // placed by the rules of a float, an out-of-flow box, or a
                // flex, grid or table keeps them, and so does a document
                // paginated on transposed axes, whose lines do not run
                // along y.
                let unplaced_line_end = (!inside_float
                    && !inside_out_of_flow
                    && !inside_flex
                    && !inside_table
                    && !flex_item_candidate
                    && !grid_item_candidate
                    && !is_rendered_leaf
                    && !document.context.pagination_on_transposed_axes)
                    .then(|| unplaced_line_end(document, cascade, node_id))
                    .flatten();
                let avoids_break = matches!(
                    computed.break_inside,
                    raikiri_style::property::BreakInside::Avoid
                        | raikiri_style::property::BreakInside::AvoidPage
                ) || unplaced_line_end.is_some();
                let clears_float = !inline_level
                    && is_block_flow_box
                    && [FloatValue::Left, FloatValue::Right]
                        .into_iter()
                        .any(|side| clears_float_side(computed.clear, side));
                let nested_candidate_base = participates_in_flow
                    && !direct_body_child
                    && !inside_table
                    && !inside_flex
                    && !out_of_flow_subtree;
                let in_flow_block_level = participates_in_flow
                    && !inside_table
                    && !inside_flex
                    && !out_of_flow_subtree
                    && !inside_float
                    && !inline_level
                    && !is_float;
                // A block box in the flow that is a candidate to move to the
                // next page, along with the box it holds or the edges it ends.
                let in_flow_block = in_flow_block_level && is_plain_block_box(document, node_id);
                // A box after one that forces a break after itself takes the
                // break, though it holds no line of its own to take it: the
                // pass otherwise leaves it on the page the break closes, and
                // the break falls before the next box that holds a line. The
                // box before it is the last box in the flow ahead of it, a
                // float or an out-of-flow box between the two not counted,
                // and only a block that moves its later siblings has one.
                let after_forced_break = self.forces_break_after
                    && in_flow_block
                    && !direct_body_child
                    && frame.previous_box.is_some_and(|previous| {
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
                let closing_edges_base_shape = (in_flow_block || in_flow_container)
                    && !direct_body_child
                    && frame.closing_edges > 0.0;
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
                        && (frame.previous_box.is_none() || frame.tied_to_start)
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
                    && (matches!(frame.tie, AvoidTie::RunStart { .. })
                        || breaks_only_around(document, cascade, node_id, self.has_named_pages)
                        || (closing_edges_shape
                            && node.is_ifc_root()
                            && paragraph_lines_keep_together(document, cascade, node_id)));
                let closing_edges_base = closing_edges_shape && unbreakable;
                // A block that holds one box moves whole with the edges that
                // would break away from it, its own and those of the blocks
                // it ends: no box before the one in it is there to break from.
                // A forced break at either end of the block, or a clipped
                // part of it, is left to the rules for those.
                let lone_tail = (in_flow_block
                    && frame.previous_box.is_some()
                    && !matches!(frame.tie, AvoidTie::RunStart { .. }))
                .then(|| {
                    lone_box_tail(
                        document,
                        cascade,
                        body_id,
                        node_id,
                        frame.closing_edges > 0.0,
                    )
                })
                .flatten()
                .filter(|&tail| {
                    tail + frame.closing_edges > 0.001
                        && candidate_raw_y + node.layout.size.height <= frame.clip_end
                        && break_before_available()
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
                let leading_unit =
                    (self.finds_nested_runs && in_flow_block && !is_body && free_to_break_before)
                        .then(|| avoided_run_head(document, cascade, body_id, node_id, true))
                        .filter(|held| held.tied)
                        .and_then(|held| held.end);
                let avoided_unit = match (frame.run_unit, leading_unit) {
                    (Some(run), Some(leading)) => Some(run.max(leading)),
                    (run, leading) => run.or(leading),
                };
                // A flex container or grid whose padding or border above its
                // items would be left alone at the foot of a page, because
                // the first piece of its content does not fit after them,
                // moves to the next page whole.
                let leading_piece = (in_flow_container
                    && !is_body
                    && free_to_break_before
                    && matches!(
                        document.nodes[node_id].display(),
                        DisplayValue::Flex | DisplayValue::Grid
                    )
                    && !document.context.pagination_on_transposed_axes)
                    .then(|| leading_piece_end(document, cascade, body_id, node_id))
                    .flatten();
                let needs_ancestor_constraints = (nested_candidate_base
                    && ((!inline_level && is_block_flow_box && avoids_break)
                        || is_float
                        || clears_float))
                    || lines_shape
                    || lone_tail.is_some()
                    || (avoided_unit.is_some() && !direct_body_child)
                    || (leading_piece.is_some() && !direct_body_child);
                let (side_by_side_ancestor, bounded_clipping_ancestor) =
                    if needs_ancestor_constraints {
                        candidate_ancestor_constraints(document, cascade, node_id, body_id)
                    } else {
                        (false, false)
                    };
                let can_move_nested = !side_by_side_ancestor && !bounded_clipping_ancestor;
                let nested_avoid_candidate = nested_candidate_base
                    && !inline_level
                    && is_block_flow_box
                    && avoids_break
                    && can_move_nested;
                let nested_float_candidate =
                    nested_candidate_base && (is_float || clears_float) && can_move_nested;
                let closing_edges_candidate = closing_edges_base && can_move_nested;
                let avoided_unit = avoided_unit.filter(|_| direct_body_child || can_move_nested);
                let leading_piece = leading_piece.filter(|_| direct_body_child || can_move_nested);
                // A box that sticks out below its parent has no tail to keep.
                let lone_box_candidate = lone_tail
                    .filter(|_| can_move_nested)
                    .map(|tail| tail.max(0.0));
                // The edges also follow the lines of a paragraph that breaks
                // between them. With no box before it in the block that the
                // edges close, the page cannot break before the paragraph at
                // a better place than between its lines (class C against
                // class B), so the orphans it cannot keep do not stop it.
                // A paragraph with a box before it that moves whole to the
                // next page for them has no such break there: its box ends up
                // at the top of that page, and its lines split when the edges
                // still run past the page end.
                let lines_closing_edges = if lines_shape
                    && can_move_nested
                    && (!unbreakable || frame.lone_in_edges || closing_edges_shape)
                    && !avoids_break
                    && node.is_ifc_root()
                    && !matches!(frame.tie, AvoidTie::RunStart { .. })
                {
                    frame.closing_edges + node.layout.padding.bottom + node.layout.border.bottom
                } else {
                    0.0
                };
                // A forced break and a page name change the pages wherever the
                // box sits, even past the edge of a clipping ancestor.
                let sets_page_flow = deferred_named_break_after
                    || own_page_name.is_some()
                    || page_break_is_forced(document, node_id, computed.break_before)
                    || box_forces_break_after(document, cascade, node_id, computed.break_after);
                let is_break_candidate = !is_body
                    && document.nodes[node_id].display() != DisplayValue::Contents
                    && !inside_float
                    && ((participates_in_flow
                        && direct_body_child
                        && (explicit_page_name.is_none() || inline_level))
                        || nested_avoid_candidate
                        || nested_float_candidate
                        || closing_edges_candidate
                        || lone_box_candidate.is_some()
                        || avoided_unit.is_some()
                        || leading_piece.is_some()
                        || is_tall_direct_absolute
                        || table_row_candidate
                        || flex_item_candidate
                        || is_rendered_leaf
                        || grid_item_candidate
                        || after_forced_break
                        || sets_page_flow);
                // A box is measured against a clipping ancestor's edge like
                // text: one past it is not a candidate, and one running over
                // it occupies only the part above.
                if is_break_candidate && (sets_page_flow || candidate_raw_y < frame.clip_end) {
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
                                lines_closing_edges: 0.0,
                            },
                            ElementContext {
                                is_rendered_leaf,
                                is_direct_body_element: direct_body_child,
                                is_table_row: table_row_candidate,
                                is_flex_item: flex_item_candidate,
                                is_grid_item: grid_item_candidate,
                                is_named: own_page_name.is_some(),
                                deferred_named_break_after,
                                unplaced_line_end,
                                closing_edges: if closing_edges_candidate
                                    || lone_box_candidate.is_some()
                                {
                                    frame.closing_edges
                                } else {
                                    0.0
                                },
                                closing_edges_inside: lone_box_candidate.unwrap_or(0.0),
                                closing_edges_span: match frame.tie {
                                    AvoidTie::RunStart { span } if closing_edges_candidate => span,
                                    _ => 0.0,
                                },
                                avoided_unit,
                                leading_piece_end: leading_piece,
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
                // An absolutely positioned box is clipped by an ancestor only
                // when that ancestor is its containing block. The walk does not
                // track containing blocks, so such a box starts afresh. A
                // document paginated on transposed axes is not measured at
                // all: the overflow that clips its flow is not the one on y.
                let clip_end = if document.context.pagination_on_transposed_axes {
                    f32::INFINITY
                } else {
                    let inherited = if matches!(
                        document.nodes[node_id].position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    ) {
                        f32::INFINITY
                    } else {
                        frame.clip_end
                    };
                    if !is_body && super::element_clips_overflow(document, cascade, node_id) {
                        inherited.min(super::clip_block_end(document, cascade, node_id, raw_y))
                    } else {
                        inherited
                    }
                };
                let child_order = pagination_child_order(document, cascade, node_id);
                // The closing edges of this block, and of the blocks that it
                // ends in turn, follow its last box in the flow: a float after
                // a box keeps it from being the last.
                let own_closing_edges = closing_edges_of(document, node_id, body_id);
                // The edges of the blocks this one ends are not below a box that
                // only starts a run of boxes tied by avoided breaks: the rest
                // of the run lies between.
                let inherited_edges = if matches!(frame.tie, AvoidTie::RunStart { .. }) {
                    0.0
                } else {
                    frame.closing_edges
                };
                let closing_chain = own_closing_edges
                    .map(|own| own + inherited_edges)
                    .filter(|&edges| edges > 0.0);
                // For each child of a block container, its last in-flow box and
                // whether it floats: they tell the box before each child, and
                // the last box that the closing edges follow.
                let flow_flags: Vec<(Option<usize>, bool)> = if closing_chain.is_some()
                    || (child_order.len() > 1 && moves_later_siblings(document, node_id))
                {
                    let mut boxes = Vec::new();
                    child_order
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
                } else {
                    Vec::new()
                };
                let last_in_flow = flow_flags
                    .iter()
                    .rposition(|&(in_flow, floated)| in_flow.is_some() || floated);
                // The margin below the last box sits between it and the padding
                // and border of this block, unless it collapses through that
                // block (when it has neither) or through the box itself (when
                // it takes no block space).
                let edges_after_box = |index: usize| {
                    closing_chain.map_or(0.0, |edges| {
                        let margin = flow_flags[index]
                            .0
                            .filter(|&last_box| {
                                own_closing_edges.is_some_and(|own| own > 0.0)
                                    && !takes_no_block_space(document, last_box)
                            })
                            .map_or(0.0, |last_box| {
                                document.nodes[last_box].layout.margin.bottom
                            });
                        // A negative margin pulls the edges up under the box,
                        // and may leave none below it.
                        let edges = (edges + margin).max(0.0);
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
                        if content_end > box_end + 0.001 {
                            0.0
                        } else {
                            edges
                        }
                    })
                };
                let last_box_edges = last_in_flow.map_or(0.0, edges_after_box);
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
                        ))
                    }
                    _ => None,
                };
                // Where the runs of boxes tied by avoided breaks that start
                // among the boxes of this block end.
                let run_units = if self.finds_nested_runs
                    && in_flow_block
                    && !is_body
                    && !flow_flags.is_empty()
                {
                    nested_run_units(document, cascade, body_id, &child_order, &flow_flags)
                } else {
                    Vec::new()
                };
                let mut previous_box = None;
                // Whether any in-flow box comes before the child, text
                // included: `previous_box` forgets a text box, which is no box
                // to break from for some rules but is one for this.
                let mut box_before = false;
                // Whether avoided breaks tie each box so far to the one before
                // it, from the first box on.
                let mut tied_from_start = true;
                let is_block = |id: usize| {
                    is_plain_block_box(document, id) || is_plain_container_box(document, id)
                };
                // A block whose edges nothing around it carries is where the
                // boxes below it stop looking for a box before them.
                let child_scope_is_lone = (own_closing_edges.is_some_and(|own| own > 0.0)
                    && inherited_edges == 0.0)
                    || frame.lone_in_edges;
                for (index, child_id) in child_order.iter().copied().enumerate() {
                    let at_chain_start = chain_run.filter(|&(first, ..)| index == first);
                    let child_closing_edges = match at_chain_start {
                        Some((_, _, edges)) => edges,
                        None if last_in_flow == Some(index) => last_box_edges,
                        None => 0.0,
                    };
                    // A block with edges of its own whose boxes are all tied to
                    // the first one has no break between boxes left to take: its
                    // last box moves alone with them, the avoided break giving
                    // way. When the edges of a block around it follow it too, a
                    // break before this block is a better one, which is left to
                    // the rules for that.
                    let tie = if let Some((_, span, _)) = at_chain_start {
                        AvoidTie::RunStart { span }
                    } else if matches!(avoid_chain, Some(AvoidChain::Start))
                        && own_closing_edges.is_some_and(|own| own > 0.0)
                        && inherited_edges == 0.0
                        && last_in_flow == Some(index)
                    {
                        AvoidTie::Waived
                    } else {
                        AvoidTie::None
                    };
                    let child_previous_box = previous_box;
                    let child_box_before = box_before;
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
                            inside_table: inside_table
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
                                || document.nodes[node_id].display().is_flex_container(),
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
                            closing_edges: child_closing_edges,
                            previous_box: child_previous_box,
                            lone_in_edges: (!child_box_before || child_tied_to_start)
                                && child_scope_is_lone,
                            tied_to_start: child_tied_to_start,
                            lines_closing_edges,
                            tie,
                            run_unit: run_units.get(index).copied().flatten(),
                        },
                    )?;
                }
            }
            _ => {} // cov:ignore: non-rendered node kinds are filtered by the document invariant.
        }
        Ok(())
    }
}
