//! Collection of the page candidates of a laid-out flow: the boxes and
//! texts pagination may move to a later page, in source order.

use super::super::box_tree::BoxTree;
use super::super::context::LayoutSession;
use super::{
    BoxParentScope, CascadeResult, DisplayValue, Document, FlexDirectionValue, FloatValue,
    FormattingContext, LayoutError, NodeFlags, NodeKind, PageLayoutControl, PositionValue,
    TextLineIndex, candidate_ancestor_constraints, clears_float_side, first_owned_text_line_top,
    in_inset_out_of_flow_subtree, is_block_level_flow_box, is_floating_box_for_pagination,
    is_monolithic_for_pagination_with_transposed_axes, page_applies_to_box, page_break_is_forced,
    page_name_to_owned, pagination_child_order, pagination_child_parent_y,
    positioned_owned_text_line_bounds, propagated_start_page_name_with_transposed_axes,
    selected_page_name, unplaced_line_end,
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
        Self {
            document,
            cascade,
            roots,
            page_step,
            out,
            text_lines,
            control,
            named_page_subtree_cache: HashMap::new(),
            has_named_pages: super::super::page::has_named_pages(cascade),
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
                if (direct_body_child || parent_height > 0.0)
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
                let needs_ancestor_constraints = nested_candidate_base
                    && ((!inline_level && is_block_flow_box && avoids_break)
                        || is_float
                        || clears_float);
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
                // A forced break and a page name change the pages wherever the
                // box sits, even past the edge of a clipping ancestor.
                let sets_page_flow = deferred_named_break_after
                    || own_page_name.is_some()
                    || page_break_is_forced(document, node_id, computed.break_before)
                    || page_break_is_forced(document, node_id, computed.break_after);
                let is_break_candidate = !is_body
                    && document.nodes[node_id].display() != DisplayValue::Contents
                    && !inside_float
                    && ((participates_in_flow
                        && direct_body_child
                        && (explicit_page_name.is_none() || inline_level))
                        || nested_avoid_candidate
                        || nested_float_candidate
                        || is_tall_direct_absolute
                        || table_row_candidate
                        || flex_item_candidate
                        || is_rendered_leaf
                        || grid_item_candidate
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
                                page_name_in_flow: !inside_float
                                    && (!out_of_flow_subtree || own_page_name.is_some()),
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
                for child_id in child_order {
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
                        },
                    )?;
                }
            }
            _ => {} // cov:ignore: non-rendered node kinds are filtered by the document invariant.
        }
        Ok(())
    }
}
