//! Taffy layout trait implementations on LayoutSession.
//!
//! Production adaptation of the SpikeTree pattern from the original
//! `taffy-layout-modes` spike, equivalent in scope to that prototype:
//! - `TraversePartialTree`: children iterator
//! - `CacheTree`: per-node cache getter / setter
//! - `LayoutPartialTree`: dispatch block / flexbox / grid based on display
//! - `LayoutBlockContainer / LayoutFlexboxContainer / LayoutGridContainer`:
//!   style getter marker impls

use taffy::tree::{RequestedAxis, RunMode};
use taffy::{
    AvailableSpace, BoxGenerationMode, CacheTree, CollapsibleMarginSet, CoreStyle,
    DetailedGridInfo, ExpandedDimension, Layout, LayoutBlockContainer, LayoutFlexboxContainer,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, NodeId,
    Position as TaffyPosition, Size, Style, TraversePartialTree, TraverseTree,
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_leaf_layout,
};

use self::block_flow::{compute_block_flow, context::BlockContext};
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::ifc::geometry::IfcAxes;
use crate::node::{FormattingContext, NodeData, NodeFlags};
use raikiri_style::property::DisplayValue;

/// The intrinsic size of a leaf: an `<img>`'s resolved size, `None` for any
/// other leaf (a text node outside every paragraph has no size).
fn leaf_intrinsic_size(node: &crate::node::Node) -> Option<Size<f32>> {
    node.image_intrinsic_size()
        .map(|(width, height)| Size { width, height })
}

/// A `calc()` payload retained with the document's layout styles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ArenaCalc {
    pub(crate) value: raikiri_style::property::CalcLengthPercentage,
    /// The property's range is `[0,∞]`: a sum that resolves below zero once
    /// its percentage basis is known clamps to zero (CSS Values 4 "Range
    /// Checking", <https://www.w3.org/TR/css-values-4/#calc-range>).
    pub(crate) non_negative: bool,
}

/// Keep `value` in the document arena and return its Taffy handle.
pub(crate) fn push_calc(
    arena: &mut Vec<std::sync::Arc<ArenaCalc>>,
    value: raikiri_style::property::CalcLengthPercentage,
    non_negative: bool,
) -> *const () {
    let payload = std::sync::Arc::new(ArenaCalc {
        value,
        non_negative,
    });
    let pointer = std::sync::Arc::as_ptr(&payload).cast();
    arena.push(payload);
    pointer
}

/// Resolve a `calc()` payload handle against `basis`.
///
/// The handle does not depend on the document, so leaf measure callbacks that
/// cannot borrow the tree call this directly.
#[allow(unsafe_code)]
pub(crate) fn resolve_calc(val: *const (), basis: f32) -> f32 {
    // `Document::retained_layout_styles` owns the payload for as long as a
    // retained Taffy style can reference this handle.
    let calc = unsafe { &*(val as *const ArenaCalc) };
    let resolved = basis * calc.value.percent / 100.0 + calc.value.px;
    if !resolved.is_finite() {
        0.0
    } else if calc.non_negative {
        resolved.max(0.0)
    } else {
        resolved
    }
}

/// Whether a `calc()` payload handle has a percentage term, which needs a
/// basis to resolve.
#[allow(unsafe_code)]
pub(crate) fn calc_has_percentage(val: *const ()) -> bool {
    // The retained style owner keeps this handle valid, as in `resolve_calc`.
    let calc = unsafe { &*(val as *const ArenaCalc) };
    calc.value.percent != 0.0
}

/// Resolve a padding or border length of an ifc root against the containing
/// block's inline size.
fn ifc_edge(value: taffy::LengthPercentage, parent_inline_size: Option<f32>) -> f32 {
    let basis = parent_inline_size.unwrap_or(0.0);
    crate::layout::used_style_length_percentage(value, basis).unwrap_or(0.0)
}

/// The physical dimension used by percentage margins, padding, and borders
/// depends on the containing block's used writing mode, including sideways
/// modes. This CSS percentage basis is independent of shaping fallbacks.
///
/// Taffy's flexbox measures an item's flex base size and automatic minimum
/// size without a main-axis parent size, so that a percentage *size* does not
/// contribute to them (CSS Sizing 3 §5.2.1). Percentage edges still refer to
/// the containing block's width, which a definite flex container knows; it is
/// read from the active [`LayoutSession`] then.
fn ifc_parent_inline_size(
    tree: &LayoutSession<'_>,
    node: usize,
    parent_size: Size<Option<f32>>,
) -> Option<f32> {
    let basis = flex_item_percent_basis(tree, node);
    let width = || {
        parent_size
            .width
            .or(basis.and_then(|basis| basis.width))
            .unwrap_or(0.0)
    };
    let vertical = tree
        .taffy_parent_of(node)
        .is_some_and(|parent| tree.nodes[parent].used_writing_mode.is_vertical());
    Some(if vertical {
        parent_size
            .height
            .or(basis.and_then(|basis| basis.height))
            .unwrap_or(0.0)
    } else {
        width()
    })
}

/// The content-box size of the flex container being laid out, when `node`
/// is one of its items; each axis is `None` where that size is not definite.
fn flex_item_percent_basis(tree: &LayoutSession<'_>, node: usize) -> Option<Size<Option<f32>>> {
    tree.context
        .flex_item_percent_bases
        .last()
        .filter(|&&(container, _)| {
            tree.nodes[node].taffy_position() != TaffyPosition::Absolute
                && tree.taffy_parent_of(node) == Some(container)
        })
        .map(|&(_, basis)| basis)
}

/// Whether a margin, padding or border of `style` depends on its percentage
/// basis.
fn has_percent_edge(style: &Style) -> bool {
    let margin_depends_on_basis = |edge: taffy::LengthPercentageAuto| {
        edge.into_raw().is_plain_calc()
            || edge.resolve_to_option(0.0, resolve_calc)
                != edge.resolve_to_option(1.0, resolve_calc)
    };
    has_percent_padding_or_border(style)
        || [
            style.margin.left,
            style.margin.right,
            style.margin.top,
            style.margin.bottom,
        ]
        .into_iter()
        .any(margin_depends_on_basis)
}

/// Whether a padding or border of `style` depends on its percentage basis.
fn has_percent_padding_or_border(style: &Style) -> bool {
    [style.padding, style.border].into_iter().any(|edges| {
        [edges.left, edges.right, edges.top, edges.bottom]
            .into_iter()
            .any(edge_depends_on_basis)
    })
}

/// Whether a padding or border length depends on its percentage basis.
fn edge_depends_on_basis(edge: taffy::LengthPercentage) -> bool {
    use taffy::ResolveOrZero;
    // A `calc()` edge always carries a percentage term (one without folds to
    // a length) and may clamp to zero at small probe bases.
    edge.into_raw().is_plain_calc()
        || edge.resolve_or_zero(Some(0.0), resolve_calc)
            != edge.resolve_or_zero(Some(1.0), resolve_calc)
}

/// Give Taffy pixel edge values resolved against the containing block's
/// inline size while keeping its physical width and height properties intact.
fn ifc_style_with_inline_edge_basis(style: &Style, inline_size: Option<f32>) -> Style {
    let basis = inline_size.unwrap_or(0.0);
    let mut style = style.clone();
    style.margin = style.margin.map(|margin| {
        margin.resolve_to_option(basis, resolve_calc).map_or_else(
            taffy::LengthPercentageAuto::auto,
            taffy::LengthPercentageAuto::length,
        )
    });
    style.padding = style
        .padding
        .map(|edge| taffy::LengthPercentage::length(ifc_edge(edge, Some(basis))));
    style.border = style
        .border
        .map(|edge| taffy::LengthPercentage::length(ifc_edge(edge, Some(basis))));
    style
}

/// Padding-top plus border-top of an ifc root, resolved from its style.
///
/// The parent has not stored this node's layout yet when the root is
/// measured, so the insets come from the style and the parent's inline size.
fn ifc_top_inset(style: &Style, parent_inline_size: Option<f32>) -> f32 {
    ifc_edge(style.padding.top, parent_inline_size) + ifc_edge(style.border.top, parent_inline_size)
}

/// Border plus padding on the left and right of an ifc root, resolved from
/// its style like [`ifc_top_inset`].
fn ifc_horizontal_edges(style: &Style, parent_inline_size: Option<f32>) -> (f32, f32) {
    (
        ifc_edge(style.padding.left, parent_inline_size)
            + ifc_edge(style.border.left, parent_inline_size),
        ifc_edge(style.padding.right, parent_inline_size)
            + ifc_edge(style.border.right, parent_inline_size),
    )
}

fn ifc_all_insets(style: &Style, parent_inline_size: Option<f32>) -> taffy::Rect<f32> {
    taffy::Rect {
        top: ifc_edge(style.padding.top, parent_inline_size)
            + ifc_edge(style.border.top, parent_inline_size),
        right: ifc_edge(style.padding.right, parent_inline_size)
            + ifc_edge(style.border.right, parent_inline_size),
        bottom: ifc_edge(style.padding.bottom, parent_inline_size)
            + ifc_edge(style.border.bottom, parent_inline_size),
        left: ifc_edge(style.padding.left, parent_inline_size)
            + ifc_edge(style.border.left, parent_inline_size),
    }
}

/// Taffy child iterator: filter nodes with `is_in_document() == false`
/// (such as a detached `<template>` contents fragment) and whitespace-only
/// text nodes, which generate no box, from the raw arena children.
///
/// The taffy layout tree is the web-spec “flat tree,” so layout must treat
/// template contents as nonexistent. Skipping them only at paint time would
/// still let their layout sizes / positions shift sibling positions.
/// `raikiri_traits::Dom::child_ids` returns raw children by contract;
/// filter only on the taffy path rather than changing that contract.
///
/// The children of a block, flex or grid container also replace each of its
/// `display: contents` children that is not a paragraph root, recursively:
/// such an element generates no box (CSS Display 3 §2.5), so its children
/// are the container's children, sized and positioned against the
/// container itself. Its own layout stays zero-sized at the
/// container's origin, so positions summed along the DOM ancestors are
/// unchanged. A `display: contents` element holding only inline content
/// remains one node, standing in for the anonymous item of that text.
pub struct TaffyChildIter<'a> {
    doc: &'a Document,
    flatten: bool,
    /// Whether the children are those of a source box, among which an
    /// anonymous table stands in for the rows it wraps.
    wrap_tables: bool,
    stack: Vec<core::slice::Iter<'a, usize>>,
}

impl<'a> TaffyChildIter<'a> {
    fn new_for_session(session: &'a LayoutSession<'_>, parent: NodeId) -> Self {
        let doc = &*session.document;
        let parent_index = usize::from(parent);
        // A size-contained box is measured as if it had no content.
        let hidden = session.context.size_contained_measure.last() == Some(&parent_index);
        Self::new(doc, parent, hidden)
    }

    fn new(doc: &'a Document, parent: NodeId, hidden: bool) -> Self {
        let parent = usize::from(parent);
        let node = doc.layout_node_with_style(parent).unwrap();
        let layout_node = &doc.nodes[parent];
        Self {
            doc,
            flatten: !node.is_display_none() && node.display() != DisplayValue::Contents,
            wrap_tables: !doc.is_anonymous_table(parent),
            stack: if hidden {
                Vec::new()
            } else {
                vec![layout_node.layout_children().iter()]
            },
        }
    }

    /// Whether `child` is a `display: contents` element whose children take
    /// its place among a container's children.
    fn is_flattened(doc: &Document, child: usize) -> bool {
        let node = &doc.layout_node_with_style(child).unwrap();
        node.kind() == raikiri_traits::NodeKind::Element
            && node.display() == DisplayValue::Contents
            && !node.is_ifc_root()
            && !node.flags.contains(NodeFlags::IS_ROOT_ELEMENT)
    }

    fn includes(doc: &Document, child: usize) -> bool {
        if !doc.nodes[child].is_in_document() {
            return false;
        }
        // Whitespace-only text generates no box of its own:
        // - CSS Flexbox §4 and CSS Grid §6: no anonymous item is generated
        //   for a whitespace-only text run, whatever its `white-space`.
        // - CSS 2.1 §9.2.1.1 and §16.6.1: in a block container, a
        //   whitespace-only run between block-level boxes collapses away and
        //   generates no anonymous block box, so it must not separate the
        //   adjoining margins of its siblings (CSS 2.1 §8.3.1). White space
        //   that `white-space` preserves forms a line instead; such text makes
        //   its container a paragraph root (see `needs_ifc_layout` in
        //   `layout::ifc::assign`), whose children taffy never visits, so
        //   every whitespace-only text child that reaches this point has
        //   collapsed away.
        !matches!(
            &doc.nodes[child].data,
            NodeData::Text(text)
                if text
                    .text_content
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        )
    }
}

impl Document {
    /// The raw children of `parent`, with each `display: contents` child that
    /// generates no box replaced by its own children, recursively. This is the
    /// list of boxes taffy lays out inside a block container, but unfiltered
    /// otherwise, so a container without such a child yields its raw children.
    /// Loose table rows need no anonymous table here: none is built for a
    /// multicolumn container or for a `display: contents` element.
    pub(crate) fn children_through_contents(&self, parent: usize) -> Vec<usize> {
        let container = self.layout_node_with_style(parent).unwrap();
        let flatten = !container.is_display_none() && container.display() != DisplayValue::Contents;
        let mut children = Vec::new();
        let mut stack = vec![self.nodes[parent].children.iter()];
        while let Some(siblings) = stack.last_mut() {
            let Some(&child) = siblings.next() else {
                stack.pop();
                continue;
            };
            if flatten && TaffyChildIter::is_flattened(self, child) {
                stack.push(self.nodes[child].children.iter());
            } else {
                children.push(child);
            }
        }
        children
    }
}

impl Document {
    /// The node whose taffy children include `node`: its DOM parent, or the
    /// container its `display: contents` ancestors lift it into.
    /// An anonymous table cell wrapping `node` is its container, though the
    /// cell exists only in the layout view.
    pub(crate) fn taffy_parent_of(&self, node: usize) -> Option<usize> {
        if let Some(icb) = self.nodes.initial_containing_block_key()
            && self.nodes[icb].children.contains(&node)
        {
            return Some(icb);
        }
        if let Some(cell) = self.anonymous_cell_of(node) {
            return Some(cell);
        }
        if let Some(table) = self.anonymous_table_of(node) {
            return Some(table);
        }
        let parent = self.parent_of(node)?;
        let mut top = parent;
        while TaffyChildIter::is_flattened(self, top) {
            top = self.parent_of(top)?;
        }
        let container = &self.layout_node_with_style(top).unwrap();
        let lifted = top != parent
            && !container.is_display_none()
            && container.display() != DisplayValue::Contents;
        Some(if lifted { top } else { parent })
    }
}

impl Iterator for TaffyChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        let doc = self.doc;
        while let Some(children) = self.stack.last_mut() {
            let Some(&child) = children.next() else {
                self.stack.pop();
                continue;
            };
            if !Self::includes(doc, child) {
                continue;
            }
            if self.flatten && Self::is_flattened(doc, child) {
                self.stack.push(doc.nodes[child].layout_children().iter());
                continue;
            }
            // An anonymous table takes the place of the run of rows it wraps
            // among its parent's children; the rows are its own children.
            if self.wrap_tables
                && let Some(table) = doc.anonymous_table_of(child)
            {
                if doc.nodes[table].children.first() != Some(&child) {
                    continue;
                }
                return Some(NodeId::from(table));
            }
            return Some(NodeId::from(child));
        }
        None
    }
}

/// The number of children from which taffy's walks over a container's
/// children by index read them from a list; walking fewer children from
/// the start each time costs less than building the list.
const WALKED_CHILDREN_MIN: usize = 32;

impl CacheTree for LayoutSession<'_> {
    fn cache_get(&mut self, node_id: NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.nodes[usize::from(node_id)].cache.get(inputs)
    }

    fn cache_store(&mut self, node_id: NodeId, inputs: &LayoutInput, layout_output: LayoutOutput) {
        self.nodes[usize::from(node_id)]
            .cache
            .store(inputs, layout_output)
    }

    fn cache_clear(&mut self, node_id: NodeId) {
        self.nodes[usize::from(node_id)].cache.clear();
    }
}

impl LayoutPartialTree for LayoutSession<'_> {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_core_container_style(&self, node_id: NodeId) -> Self::CoreContainerStyle<'_> {
        self.layout_node_with_style(usize::from(node_id))
            .unwrap()
            .style
    }

    /// The **only** path by which taffy writes layout into the arena.
    ///
    /// Storing through [`LayoutSession::store_unrounded_layout`] here structurally
    /// guarantees that `Node.unrounded_layout` contains no non-finite f32 values.
    /// The input guard in the bridge (`layout::sanitize_taffy`) alone cannot
    /// close the gap: nested percentages compound at the used-value layer
    /// and can become non-finite again. See the documentation for
    /// `layout::sanitize_taffy_layout` for the reasoning and measurements.
    ///
    /// Record fields actually clamped in `self.context.layout_warnings`.
    /// The `taffy` crate fixes this trait-method signature, so diagnostic
    /// arguments cannot be added. Instead, write to the owned buffer through
    /// `self` without changing the signature. At pass end, `layout_single_page`
    /// drains the buffer to an observer or eprintln (see the documentation for
    /// the diagnostics buffer below).
    /// The session context drops this buffer when the layout operation ends.
    fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &Layout) {
        self.note_transposed_layout(usize::from(node_id));
        self.store_unrounded_layout(usize::from(node_id), layout);
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        resolve_calc(val, basis)
    }

    fn compute_child_layout(&mut self, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        self.compute_child_layout_with_block_ctx(node_id, inputs, None)
    }
}

impl LayoutSession<'_> {
    /// Unified child dispatch for the native layout traits and project flow.
    ///
    /// Public Taffy flex/grid algorithms enter through
    /// [`LayoutPartialTree::compute_child_layout`] without a shared context.
    /// Project block and inline flow pass their own [`BlockContext`] here,
    /// keeping ordinary descendants in the same float formatting context.
    /// Such layouts bypass size-only caching because a cached result cannot
    /// replay float placement or unresolved margin-strut side effects.
    ///
    /// Overflow is bridged to `taffy::Style::overflow`; the remaining
    /// overflow-area details are handled by the paint and block-layout paths.
    #[inline(never)]
    pub(crate) fn compute_child_layout_with_block_ctx(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        // A box that does not share its parent's block context is not part of
        // the parent's `text-group-align` group.
        let keeps_group = std::mem::take(&mut self.context.keep_text_group);
        self.context.group_child_uncached = keeps_group;
        let outer_group = (block_ctx.is_none() && !keeps_group).then(|| {
            (
                std::mem::take(&mut self.context.text_group),
                self.context.text_group_shortest.take(),
            )
        });
        let output = if let Some(output) = self.compute_transposed_child(node_id, inputs) {
            output
        } else {
            stacker::maybe_grow(128 * 1024, 1024 * 1024, || {
                self.compute_child_layout_with_block_ctx_inner(node_id, inputs, block_ctx)
            })
        };
        self.context.group_child_uncached = false;
        if let Some((group, shortest)) = outer_group {
            self.context.text_group = group;
            self.context.text_group_shortest = shortest;
        }
        output
    }

    #[inline(never)]
    fn compute_child_layout_with_block_ctx_inner(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        // Lazy layout cache invalidation — mutations only set a flag; we
        // clear all node caches on the first compute after the flag flips
        // (amortized O(1) per mutation over N-node batches).
        if self.layout_dirty {
            for node in self.nodes.layout_nodes_mut() {
                node.cache.clear();
            }
            self.context.grid_layout_info.clear();
            self.context.taffy_hoisted_children.clear();
            self.layout_dirty = false;
        }
        let inputs = vertical_oof::absolute_child_inputs(self, usize::from(node_id), inputs);
        if inputs.run_mode == taffy::RunMode::PerformLayout {
            let idx = usize::from(node_id);
            self.nodes[idx].percent_basis_height = self.block_percent_basis_height(idx, &inputs);
        }
        // Taffy's cache does not key on the parent size, so a flex item whose
        // edges take their basis from the flex container instead is measured
        // afresh: the same container may be laid out at another size.
        let idx = usize::from(node_id);
        let flex_basis = flex_item_percent_basis(self, idx);
        // The basis an ifc root item finds there in the axis taffy left out:
        // the width, or the height in a vertical writing mode.
        let ifc_basis = flex_basis.and_then(|basis| {
            if self.has_vertical_writing_mode(idx) {
                basis.height.filter(|_| inputs.parent_size.height.is_none())
            } else {
                basis.width.filter(|_| inputs.parent_size.width.is_none())
            }
        });
        let paragraph = self.nodes[idx].formatting_context == FormattingContext::Inline;
        // A shared block context owns float placements and unresolved margin struts.
        // A size-only cache cannot replay them after a speculative layout is restored.
        let uncached = block_ctx.is_some()
            || std::mem::take(&mut self.context.group_child_uncached)
            || (paragraph
                && ifc_basis.is_some()
                && has_percent_edge(self.layout_node_with_style(idx).unwrap().style))
            || self.context.size_contained_measure.last() == Some(&idx);
        // Any other flex item container resolves its own padding and borders
        // against the parent size taffy hands it, which is missing in the
        // main axis while taffy measures the item; they are resolved against
        // the flex container's width up front instead. A vertical item is
        // left to taffy, as in `block_layout`.
        let edge_basis = flex_basis
            .filter(|_| {
                inputs.parent_size.width.is_none()
                    && !paragraph
                    && !self.has_vertical_writing_mode(idx)
            })
            .and_then(|basis| basis.width)
            .filter(|_| {
                has_percent_padding_or_border(self.layout_node_with_style(idx).unwrap().style)
            });
        let compute = |tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput| {
            let idx = usize::from(node_id);
            let empty = tree.context.size_contained_measure.last() == Some(&idx);
            let inputs = if empty {
                inputs
            } else {
                size_contained_inputs(tree, node_id, inputs)
            };
            let formatting_context = tree.nodes[idx].formatting_context;
            match formatting_context {
                FormattingContext::None => return LayoutOutput::HIDDEN,
                // Taffy has no table layout; the table engine lays the
                // wrapper box out.
                FormattingContext::Table => {
                    return crate::layout::table::compute_table_layout(tree, node_id, inputs);
                }
                _ => {}
            }
            // Inline-block shrink-wrap: CSS InlineBlock lowers to
            // taffy Block, so without this a width:auto inline-block would
            // stretch-fill like a plain block. Explicit widths keep the normal
            // block path below; only width:auto sizes to fit-content here.
            {
                use raikiri_style::property::DisplayValue;
                // A table box laid out as an ifc root is an anonymous cell's
                // content inside the table wrapper, whose auto width shrinks to
                // fit as well (CSS 2.1 17.5.2.2).
                let ifc_table = formatting_context == FormattingContext::Inline
                    && matches!(
                        tree.nodes[idx].display(),
                        DisplayValue::Table | DisplayValue::InlineTable
                    );
                let inline_shrink_wrap = ifc_table
                    || (tree.nodes[idx].display().is_atomic_inline_level()
                        && tree.nodes[idx].display() != DisplayValue::InlineTable);
                // Size containment already gave the box its width.
                let contained_width = tree.nodes[idx].size_containment.width && !empty;
                if inline_shrink_wrap
                    && !tree.nodes[idx].flags.contains(NodeFlags::IS_ROOT_ELEMENT)
                    && !contained_width
                    && tree
                        .layout_node_with_style(idx)
                        .unwrap()
                        .style
                        .size
                        .width
                        .is_auto()
                    && (inputs.known_dimensions.width.is_none()
                        || inputs.sizing_mode != taffy::SizingMode::ContentSize)
                {
                    return compute_inline_block_shrink_wrap(tree, node_id, inputs, block_ctx);
                }
            }
            // A vertical box under a horizontal block container is sized by
            // its content on the axis its container stretched it along.
            if !empty
                && inputs.known_dimensions.width.is_some()
                && inputs.sizing_mode == taffy::SizingMode::InherentSize
                && tree.context.orthogonal_block_in_layout != Some(idx)
                && orthogonal_flow::is_orthogonal_auto_block_size(tree, idx)
            {
                return orthogonal_flow::compute_orthogonal_block(tree, node_id, inputs);
            }
            // CSS multicol is not a native Taffy display mode. Dispatch at
            // this seam so nested containers receive a local fragmentation
            // context instead of being repaired by a global post-pass.
            // Keep malformed/deeply recursive DOM from exhausting the layout
            // stack; the ordinary Taffy path remains the bounded fallback.
            const MAX_NESTED_MULTICOL_DEPTH: usize = 64;
            // A multicol container whose own content is a paragraph is
            // classified as one and laid out by the ifc branch below, which
            // splits its lines in columns.
            if formatting_context == FormattingContext::Multicol
                && !empty
                && tree.context.fragmentation_stack.len() < MAX_NESTED_MULTICOL_DEPTH
            {
                return crate::layout::compute_multicol_layout(tree, node_id, inputs, block_ctx); // cov:ignore: exercised by ignored nested multicol WPT reftests
            }
            // An inline SVG root is replaced content: its children are drawn
            // by the SVG renderer, not laid out, so it is measured as a leaf.
            let is_leaf =
                tree.nodes[idx].children.is_empty() || tree.nodes[idx].is_inline_svg_root();
            if empty && is_leaf {
                // Replaced content with size containment has no natural size
                // or aspect ratio.
                let mut style = tree.layout_node_with_style(idx).unwrap().style.clone();
                style.aspect_ratio = None;
                return compute_leaf_layout(inputs, &style, resolve_calc, |known, _| Size {
                    width: known.width.unwrap_or(0.0),
                    height: known.height.unwrap_or(0.0),
                });
            }
            if tree.nodes[idx].is_inline_svg_root()
                && tree.nodes[idx].attribute("width").is_none()
                && tree.nodes[idx].attribute("height").is_none()
                && let Some(intrinsic) = tree.nodes[idx].image_intrinsic_box()
                && let Some(ratio) = intrinsic.aspect_ratio
            {
                // CSS Sizing 3, intrinsic sizes: ratio-only replaced content
                // uses the definite available inline size, not a 300px default.
                let mut style = tree.layout_node_with_style(idx).unwrap().style.clone();
                // Transfer the ratio in measurement only. Taffy's leaf ratio
                // floor would otherwise override explicit/max heights.
                style.aspect_ratio = None;
                // An authored height gives an auto width through the ratio
                // (CSS 2.1 10.3.2), also when no parent passes the height in
                // as a known dimension; taffy's leaf then hands it to the
                // measure function as the definite available height.
                let height_is_authored = style.size.width.is_auto()
                    && taffy::util::MaybeResolve::maybe_resolve(
                        style.size.height,
                        inputs.parent_size.height,
                        |val, basis| tree.resolve_calc_value(val, basis),
                    )
                    .is_some();
                return compute_leaf_layout(
                    inputs,
                    &style,
                    |val, basis| tree.resolve_calc_value(val, basis),
                    |known, available| {
                        // Taffy's known dimensions include padding and border;
                        // its available space has already removed those insets.
                        let known = Size {
                            width: known.width.and(available.width.into_option()),
                            height: known.height.and(available.height.into_option()),
                        };
                        let authored_height = available
                            .height
                            .into_option()
                            .filter(|_| height_is_authored);
                        let width = known
                            .width
                            .or(known.height.map(|height| height * ratio))
                            .or(authored_height.map(|height| height * ratio))
                            .unwrap_or(match available.width {
                                AvailableSpace::Definite(width) => width,
                                AvailableSpace::MinContent => 0.0,
                                AvailableSpace::MaxContent => intrinsic.width,
                            });
                        Size {
                            width,
                            height: known.height.unwrap_or(width / ratio),
                        }
                    },
                );
            }
            // A paragraph measured as if empty is a block container with no
            // lines.
            if formatting_context == FormattingContext::Inline && !empty {
                return compute_ifc_root(tree, node_id, inputs, block_ctx);
            }
            if is_leaf {
                let style = tree.layout_node_with_style(idx).unwrap().style.clone();
                let leaf_intrinsic = leaf_intrinsic_size(&tree.nodes[idx]);
                compute_leaf_layout(inputs, &style, resolve_calc, |known, _available| {
                    // An explicit size from the style wins; otherwise an
                    // image's intrinsic size; a text node outside every
                    // paragraph and any other leaf measure zero.
                    Size {
                        width: known
                            .width
                            .or(leaf_intrinsic.map(|s| s.width))
                            .unwrap_or(0.0),
                        height: known
                            .height
                            .or(leaf_intrinsic.map(|s| s.height))
                            .unwrap_or(0.0),
                    }
                })
            } else {
                match formatting_context {
                    FormattingContext::Flex => flex_layout(tree, node_id, inputs),
                    FormattingContext::Grid => grid_layout(tree, node_id, inputs),
                    // A block container, including a paragraph or multicol
                    // container measured as if empty or nested too deeply.
                    _ => {
                        let vertical_cb =
                            vertical_oof::containing_block_inline_size(tree, idx, &inputs);
                        if let Some(inline_size) = vertical_cb {
                            tree.context
                                .vertical_oof_containing_blocks
                                .push((idx, inline_size));
                        }
                        let output = block_layout(tree, node_id, inputs, block_ctx);
                        if vertical_cb.is_some() {
                            tree.context.vertical_oof_containing_blocks.pop();
                        }
                        if inputs.run_mode == RunMode::PerformLayout {
                            vertical_oof::place_static_block_start(tree, idx, output.size);
                        }
                        output
                    }
                }
            }
        };
        let mut output = if let Some(basis) = edge_basis {
            // The edges come out as they would against a parent size of the
            // same width, so cached layouts stay valid while the basis does.
            if self.nodes[idx].flex_edge_basis != Some(basis) {
                self.nodes[idx].cache.clear();
                self.nodes[idx].flex_edge_basis = Some(basis);
            }
            let style = self.layout_style_for_key_mut(idx).unwrap();
            let edges = (style.padding, style.border);
            style.padding = style
                .padding
                .map(|edge| taffy::LengthPercentage::length(ifc_edge(edge, Some(basis))));
            style.border = style
                .border
                .map(|edge| taffy::LengthPercentage::length(ifc_edge(edge, Some(basis))));
            let output = compute_cached_layout(self, node_id, inputs, compute);
            let style = self.layout_style_for_key_mut(idx).unwrap();
            (style.padding, style.border) = edges;
            output
        } else if uncached {
            compute(self, node_id, inputs)
        } else {
            compute_cached_layout(self, node_id, inputs, compute)
        };
        // An ifc root measured its baseline from the lines of this very call;
        // the stored lines may be those of another width.
        if !self.nodes[usize::from(node_id)]
            .flags
            .contains(NodeFlags::IS_IFC_ROOT)
            && let Some(baseline) = first_inline_baseline(self, usize::from(node_id))
        {
            output.baselines.first = Some(baseline);
        }
        // A scroll container exports baselines clamped to its border box
        // (CSS Box Alignment 3 §9.1), so content scrolled out of view does
        // not move the baseline its parent aligns to.
        // spec: https://www.w3.org/TR/css-align-3/#baseline-export
        let scrolls = self
            .layout_node_with_style(idx)
            .is_some_and(|node| is_scroll_container(node.style.overflow));
        if scrolls && inputs.run_mode == RunMode::PerformLayout {
            clamp_baselines(&mut output.baselines, output.size.height);
        }
        // A flex container keeps the first baseline of its performed layout,
        // which a list item's outside marker aligns with. A grid container's
        // baseline is found from the first-row items its detailed grid
        // information records instead, because Taffy's grid baseline does
        // not measure an item's own first line.
        if inputs.run_mode == RunMode::PerformLayout
            && self.nodes[usize::from(node_id)].formatting_context == FormattingContext::Flex
        {
            self.nodes[usize::from(node_id)].flex_first_baseline = output.baselines.first;
        }
        // The baselines of a box in a writing mode orthogonal to its parent's
        // run across the parent's line, so they say nothing about where the
        // box sits along it: it exposes none and is synthesized from its
        // border box instead.
        if self.is_orthogonal_to_parent(usize::from(node_id)) {
            output.baselines = taffy::Baselines::NONE;
        }
        output
    }

    /// Whether the box `idx` has a vertical inline axis while the container
    /// that lays it out has a horizontal one, or the other way round. A
    /// `display: contents` ancestor generates no box, so its writing mode
    /// does not count.
    fn is_orthogonal_to_parent(&self, idx: usize) -> bool {
        self.taffy_parent_of(idx).is_some_and(|parent| {
            self.has_vertical_writing_mode(parent) != self.has_vertical_writing_mode(idx)
        })
    }
}

#[cfg(test)]
mod tests;

pub(crate) mod block_flow;
mod oof;
mod orthogonal_flow;
mod vertical_block;
mod vertical_oof;
pub(crate) use vertical_block::{TransposedBlock, block_right_to_left, is_table_box};
pub(crate) use vertical_oof::used_writing_mode;

/// Lay out an ifc root: its paragraph is measured and broken into lines by
/// the inline engine, and taffy sees it as a leaf.
fn compute_ifc_root(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let style = tree.layout_node_with_style(idx).unwrap().style.clone();
    let parent_inline_size = ifc_parent_inline_size(tree, idx, inputs.parent_size);
    let layout_style = ifc_style_with_inline_edge_basis(&style, parent_inline_size);
    // Border-box top inset, resolved here: the parent has not
    // stored this node's layout yet.
    let top_inset = ifc_top_inset(&style, parent_inline_size);
    // A known width on the input means the parent stretched or
    // fixed the box, and a definite `width` of its own fixes it
    // too (taffy passes it as the available width); otherwise the
    // box shrinks to fit.
    let stretched_width = inputs.known_dimensions.width.is_some()
        || (inputs.sizing_mode == taffy::SizingMode::InherentSize
            && ifc_has_definite_width(&style, inputs.parent_size.width));
    let stretched_height = inputs.known_dimensions.height.is_some()
        || (inputs.sizing_mode == taffy::SizingMode::InherentSize
            && ifc_has_definite_height(&style, inputs.parent_size.height));
    let measure = IfcMeasure {
        run_mode: inputs.run_mode,
        stretched_width,
        stretched_height,
        width_bounds: ifc_content_width_bounds(
            &style,
            inputs.parent_size.width,
            parent_inline_size,
        ),
        height_bounds: ifc_content_height_bounds(
            &style,
            inputs.parent_size.height,
            parent_inline_size,
        ),
        edges: ifc_horizontal_edges(&style, parent_inline_size),
        insets: ifc_all_insets(&style, parent_inline_size),
        top_margin_escapes: block_ctx.is_some()
            && inputs.vertical_margins_are_collapsible.start
            && tree.nodes[idx].taffy_display() == taffy::Display::Block
            && tree.nodes[idx].taffy_position() != TaffyPosition::Absolute
            && style.float == taffy::Float::None
            && !style.overflow.x.is_scroll_container()
            && !style.overflow.y.is_scroll_container()
            && !style.contain.establishes_independent_formatting_context()
            && style.align_content.keyword == taffy::AlignContentKeyword::Normal
            && tree.nodes[idx].multicol.is_none()
            && tree.nodes[idx].ifc.as_ref().is_some_and(|root| {
                root.writing_mode == shodo::geometry::WritingMode::HorizontalTb
            })
            && top_inset == 0.0,
        // CSS 2.1 8.3.1: the bottom margin of a last in-flow child and that
        // of its parent are adjoining when the parent has no bottom padding
        // or border and an `auto` height, and the parent is in the same
        // formatting context as its own parent (a block context is passed).
        bottom_margin_escapes: block_ctx.is_some()
            && inputs.vertical_margins_are_collapsible.end
            && style.size.height.is_auto()
            && ifc_edge(style.padding.bottom, parent_inline_size)
                + ifc_edge(style.border.bottom, parent_inline_size)
                == 0.0,
        column_height: None,
        definite_height: None,
    };
    let mut content_baseline = None;
    let mut leading_margin = CollapsibleMarginSet::ZERO;
    let mut escaping_margin = CollapsibleMarginSet::ZERO;
    let mut measure = measure;
    let auto_vertical_block_size = tree.nodes[idx]
        .ifc
        .as_ref()
        .is_some_and(|root| root.writing_mode != shodo::geometry::WritingMode::HorizontalTb)
        && !measure.stretched_width;
    let mut measured_size = None;
    measure.column_height = tree.nodes[idx]
        .multicol
        .filter(|multicol| multicol.height_definite)
        .and_then(|_| {
            crate::layout::multicol_definite_dimension(
                tree,
                style.size.height,
                inputs.parent_size.height,
            )
        })
        .map(|height| {
            let insets = if style.box_sizing == taffy::BoxSizing::BorderBox {
                top_inset
                    + ifc_edge(style.padding.bottom, parent_inline_size)
                    + ifc_edge(style.border.bottom, parent_inline_size)
            } else {
                0.0
            };
            (height - insets).max(0.0)
        });
    // CSS 2.1 §10.5: a percentage height resolves against the containing
    // block's height only when that height does not depend on content. A
    // height fixed by the parent is border-box; one from the box's own
    // `height` follows `box-sizing`.
    // A horizontal scrollbar reserves its gutter from the block axis.
    let horizontal_gutter = if style.overflow.x == taffy::Overflow::Scroll {
        style.scrollbar_width
    } else {
        0.0
    };
    let border_box_insets = top_inset
        + ifc_edge(style.padding.bottom, parent_inline_size)
        + ifc_edge(style.border.bottom, parent_inline_size)
        + horizontal_gutter;
    let border_box = style.box_sizing == taffy::BoxSizing::BorderBox;
    measure.definite_height = inputs
        .known_dimensions
        .height
        .filter(|_| inputs.known_dimensions_are_definite.height)
        .map(|height| (height - border_box_insets).max(0.0))
        .or_else(|| {
            if style.size.height.is_auto()
                || (inputs.parent_size.height.is_none()
                    && !matches!(style.size.height.expand(), ExpandedDimension::Length(_)))
            {
                return None;
            }
            crate::layout::multicol_definite_dimension(
                tree,
                style.size.height,
                inputs.parent_size.height,
            )
            // A `height` of its own follows `box-sizing`.
            .map(|height| {
                (height
                    - if border_box {
                        border_box_insets
                    } else {
                        horizontal_gutter
                    })
                .max(0.0)
            })
        })
        // `min-height` and `max-height` clamp the used height; a percentage
        // bound against an indefinite containing block does not apply.
        .map(|height| {
            let (min, max) = if inputs.parent_size.height.is_some() {
                measure.height_bounds
            } else {
                ifc_content_axis_bounds(
                    ifc_length_only(style.min_size.height),
                    ifc_length_only(style.max_size.height),
                    style.box_sizing,
                    [style.padding.top, style.padding.bottom],
                    [style.border.top, style.border.bottom],
                    None,
                    parent_inline_size,
                )
            };
            height.min(max).max(min)
        });
    let mut output =
        compute_leaf_layout(inputs, &layout_style, resolve_calc, |known, available| {
            let measured = measure_ifc_root(
                tree,
                idx,
                known.width,
                known.height,
                available,
                measure,
                block_ctx,
            );
            measured_size = Some(measured.size);
            content_baseline = measured.first_baseline;
            leading_margin = measured.top_margin;
            escaping_margin = measured.bottom_margin;
            measured.size
        });
    if auto_vertical_block_size && let Some(measured_size) = measured_size {
        // `compute_leaf_layout` retains Taffy's intrinsic auto-width probe as
        // a known width. In vertical flow, the performed IFC layout measures
        // the cumulative block extent and must replace that probe result.
        output.size.width = measured_size.width + measure.insets.left + measure.insets.right;
    }
    output.top_margin = leading_margin;
    output.baselines.first = content_baseline.map(|baseline| baseline + top_inset);
    if inputs.run_mode == RunMode::PerformLayout {
        crate::layout::ifc::boxes::place_out_of_flow(tree, idx, output.size);
    }
    // When `min-height` gives the used height, the last child's bottom margin
    // no longer adjoins the box's bottom edge and does not collapse through
    // it (CSS 2.1 8.3.1), as taffy's block algorithm decides.
    let min_height = taffy::util::MaybeResolve::maybe_resolve(
        style.min_size.height,
        inputs.parent_size.height,
        resolve_calc,
    );
    let constrained_by_min_height =
        min_height.is_some_and(|min| min > 0.0 && min >= output.size.height);
    output.bottom_margin = if constrained_by_min_height {
        CollapsibleMarginSet::ZERO
    } else {
        escaping_margin
    };
    output
}

/// The node's own `min-width` and `max-width`, as content-box widths.
///
/// A shrink-to-fit box is clamped by these after it is measured, so the lines
/// have to be broken at the clamped width to match the box taffy returns.
/// `auto` gives `0.0` and `f32::INFINITY`.
fn ifc_content_width_bounds(
    style: &Style,
    parent_width: Option<f32>,
    parent_inline_size: Option<f32>,
) -> (f32, f32) {
    ifc_content_axis_bounds(
        style.min_size.width,
        style.max_size.width,
        style.box_sizing,
        [style.padding.left, style.padding.right],
        [style.border.left, style.border.right],
        parent_width,
        parent_inline_size,
    )
}

/// Drops a percentage bound, or a `calc()` one with a percentage term, which
/// cannot resolve against an indefinite containing block (CSS 2.1 §10.7).
fn ifc_length_only(value: taffy::LengthPercentageAuto) -> taffy::LengthPercentageAuto {
    let raw = value.into_raw();
    // Calc pointers carry address bits in `tag()`, so test them first.
    let length = if raw.is_plain_calc() {
        !calc_has_percentage(raw.calc_value())
    } else {
        raw.tag() == taffy::CompactLength::LENGTH_TAG
    };
    if length {
        value
    } else {
        taffy::LengthPercentageAuto::auto()
    }
}

fn ifc_content_height_bounds(
    style: &Style,
    parent_height: Option<f32>,
    parent_inline_size: Option<f32>,
) -> (f32, f32) {
    ifc_content_axis_bounds(
        style.min_size.height,
        style.max_size.height,
        style.box_sizing,
        [style.padding.top, style.padding.bottom],
        [style.border.top, style.border.bottom],
        parent_height,
        parent_inline_size,
    )
}

fn ifc_content_axis_bounds(
    min_size: taffy::LengthPercentageAuto,
    max_size: taffy::LengthPercentageAuto,
    box_sizing: taffy::BoxSizing,
    padding: [taffy::LengthPercentage; 2],
    border: [taffy::LengthPercentage; 2],
    dimension_basis: Option<f32>,
    edge_basis: Option<f32>,
) -> (f32, f32) {
    let edge_basis = edge_basis.unwrap_or(0.0);
    let resolve_lp = |value: taffy::LengthPercentage| {
        crate::layout::used_style_length_percentage(value, edge_basis).unwrap_or(0.0)
    };
    let insets = if box_sizing == taffy::BoxSizing::BorderBox {
        resolve_lp(padding[0])
            + resolve_lp(padding[1])
            + resolve_lp(border[0])
            + resolve_lp(border[1])
    } else {
        0.0
    };
    let resolve_dim = |value: taffy::LengthPercentageAuto, auto: f32| {
        let raw = value.into_raw();
        // A percentage of an indefinite basis behaves as the initial value,
        // as taffy's leaf sizing resolves it.
        let basis = dimension_basis.or_else(|| {
            (raw.is_plain_calc() && !calc_has_percentage(raw.calc_value())).then_some(0.0)
        });
        let Some(resolved) = crate::layout::resolve_length_percentage_auto(value, basis) else {
            return auto;
        };
        if resolved.is_finite() {
            (resolved - insets).max(0.0)
        } else {
            auto
        }
    };
    (
        resolve_dim(min_size, 0.0),
        resolve_dim(max_size, f32::INFINITY),
    )
}

/// Whether the box's own `width` resolves to a length, as taffy's leaf layout
/// resolves it before calling the measure callback.
fn ifc_has_definite_width(style: &Style, parent_width: Option<f32>) -> bool {
    crate::layout::resolve_dimension(style.size.width, parent_width).is_some()
}

fn ifc_has_definite_height(style: &Style, parent_height: Option<f32>) -> bool {
    crate::layout::resolve_dimension(style.size.height, parent_height).is_some()
}

/// What the measure callback of an ifc root needs besides the available
/// space.
#[derive(Clone, Copy)]
struct IfcMeasure {
    run_mode: RunMode,
    /// The parent stretched or fixed the box's physical width.
    stretched_width: bool,
    /// The parent stretched or fixed the box's physical height.
    stretched_height: bool,
    /// The box's own content-box `min-width` and `max-width`.
    width_bounds: (f32, f32),
    /// The box's own content-box `min-height` and `max-height`.
    height_bounds: (f32, f32),
    /// Border plus padding on the left and right.
    edges: (f32, f32),
    /// Border plus padding on every physical side.
    insets: taffy::Rect<f32>,
    /// Leading block margins leave the content box through the root's top edge.
    top_margin_escapes: bool,
    /// The root's bottom margin may collapse with its last block child's.
    bottom_margin_escapes: bool,
    /// The content-box height of a multicol root with a definite height:
    /// the height of its columns.
    column_height: Option<f32>,
    /// The root's definite content-box height, when it has one: the basis
    /// for the percentage heights of its floats and atomic inlines and for
    /// the vertical percentage offsets of its relative inline boxes.
    definite_height: Option<f32>,
}

/// A paragraph measurement with its independent leading and trailing margin sets.
struct IfcMeasurement {
    size: Size<f32>,
    first_baseline: Option<f32>,
    top_margin: CollapsibleMarginSet,
    bottom_margin: CollapsibleMarginSet,
}

impl IfcMeasurement {
    const EMPTY: Self = Self {
        size: Size::ZERO,
        first_baseline: None,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
    };
}

/// Measure an ifc root's paragraph for taffy's leaf measure callback.
///
/// The width comes from `available.width`, which taffy has already reduced to
/// the content box. `known` is not used for the width: it is `NONE` when
/// performing layout and border-box when only computing a size. A box the
/// parent stretched or fixed breaks at the available width; a shrink-to-fit
/// box is kept between its min- and max-content widths. The lines wrap around
/// the floats of `block_ctx`, the parent's float context, when there is one.
/// Every performed layout stores its lines, so the last one wins after a
/// parent's probes.
fn measure_ifc_root(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    known_width: Option<f32>,
    known_height: Option<f32>,
    available: Size<AvailableSpace>,
    measure: IfcMeasure,
    mut block_ctx: Option<&mut BlockContext<'_>>,
) -> IfcMeasurement {
    use crate::layout::ifc::{flow, root::with_state};
    let width_only = std::mem::take(&mut tree.context.ifc_width_probe);
    let Some((probe, writing_mode, fixed, direction)) = tree.nodes[idx].ifc.as_ref().map(|root| {
        (
            root.without_lines(),
            root.writing_mode,
            root.fixed,
            if root.rtl {
                shodo::geometry::Direction::Rtl
            } else {
                shodo::geometry::Direction::Ltr
            },
        )
    }) else {
        return IfcMeasurement::EMPTY; // cov:ignore: pre-existing defensive guard; valid IFC classification always carries a paragraph.
    };
    // A fixed box shrinks to fit against its containing block, the page area
    // (CSS 2.1 10.1), while taffy hands it its parent's room. A min-content or
    // max-content measure is not a room to fit, and is kept.
    let page_width = tree.ifc.as_ref().and_then(|state| state.page_width);
    let available = match page_width {
        Some(page_width)
            if fixed
                && !measure.stretched_width
                && matches!(available.width, AvailableSpace::Definite(_)) =>
        {
            Size {
                width: AvailableSpace::Definite(
                    (page_width - measure.edges.0 - measure.edges.1).max(0.0),
                ),
                ..available
            }
        }
        _ => available,
    };
    // Take the engine state only around the calls into the engine: laying
    // out another node in between may need the state for that node.
    let has_boxes = !probe.boxes.is_empty() || !probe.generated_blocks.is_empty();
    // Marker and generated images and empty atomic pseudo-elements are sized
    // without laying out a node, so they take part in the intrinsic widths
    // but keep the paragraph's columns.
    let has_box_intrinsics = has_boxes
        || probe.marker_atomic.is_some()
        || !probe.generated_atomics.is_empty()
        || !probe.generated_boxes.is_empty();
    let vertical = writing_mode != shodo::geometry::WritingMode::HorizontalTb;
    let inline_available = if vertical {
        available.height
    } else {
        available.width
    };
    let inline_stretched = if vertical {
        measure.stretched_height
    } else {
        measure.stretched_width
    };
    let inline_bounds = if vertical {
        measure.height_bounds
    } else {
        measure.width_bounds
    };
    // The boxes are measured as nodes of their own, outside the state scope,
    // and only when the intrinsic widths decide the width.
    let needs_intrinsics =
        !(inline_stretched && matches!(inline_available, AvailableSpace::Definite(_)));
    let box_intrinsics = if has_box_intrinsics && needs_intrinsics {
        let basis = match inline_available {
            AvailableSpace::Definite(size) => size,
            _ => 0.0,
        };
        crate::layout::ifc::boxes::intrinsics_of_boxes(tree, idx, basis)
    } else {
        crate::layout::ifc::boxes::BoxIntrinsics::EMPTY
    };
    // A horizontal paragraph that is stretched to a definite width breaks at
    // that width, so its min- and max-content widths decide nothing and the
    // paragraph is not scanned for them. A vertical one still needs them for
    // its block extent below.
    let intrinsics = if !vertical && !needs_intrinsics {
        tree.ifc.is_some().then_some((0.0, 0.0))
    } else {
        with_state(tree, |state| {
            #[cfg(test)]
            {
                state.intrinsic_measurements += 1;
            }
            flow::intrinsic_widths_with(&probe, &mut state.layout_cx, &box_intrinsics.engine)
        })
    };
    let Some((min_content, max_content)) = intrinsics else {
        return IfcMeasurement::EMPTY; // cov:ignore: pre-existing defensive guard; paragraph assignment requires the document's inline engine.
    };
    let (min_content, max_content) = (
        min_content.max(box_intrinsics.blocks.0),
        max_content.max(box_intrinsics.blocks.1),
    );
    let width = match inline_available {
        AvailableSpace::Definite(width) if inline_stretched => width,
        AvailableSpace::Definite(width) => width.max(min_content).min(max_content),
        AvailableSpace::MinContent => min_content,
        AvailableSpace::MaxContent => max_content,
    };
    // A stretched or fixed box already carries its clamped width; a
    // shrink-to-fit box is clamped by its own min/max after measurement.
    let width = if inline_stretched {
        width
    } else {
        width.min(inline_bounds.1).max(inline_bounds.0)
    };
    // A width-only probe of a horizontal paragraph is answered here; its
    // height is never read (the caller restores the node's layout cache).
    if width_only && !vertical && measure.run_mode == RunMode::ComputeSize {
        return IfcMeasurement {
            size: Size { width, height: 0.0 },
            ..IfcMeasurement::EMPTY
        };
    }
    // The content of a multicol container is broken at the column width and
    // split in its columns. Atomic inlines sit on the lines like text, so a
    // paragraph that holds only them is split line by line in an outermost
    // container. A paragraph with floats, blocks, out-of-flow boxes or
    // generated blocks of its own, and one with boxes in a container nested in
    // another, is laid out in one column as wide as the container.
    let nested_fragmentation = (!vertical)
        .then(|| fragmentation_context_for_ifc(tree, idx, &probe))
        .flatten();
    let atomics_only = probe.generated_blocks.is_empty()
        && probe
            .boxes
            .iter()
            .all(|box_| box_.kind == crate::layout::ifc::boxes::IfcBoxKind::Atomic)
        && (probe.boxes.is_empty() || tree.context.fragmentation_stack.is_empty());
    let fragmentation = if vertical {
        None
    } else if nested_fragmentation.is_some() {
        nested_fragmentation
    } else if !atomics_only {
        None
    } else {
        tree.nodes[idx]
            .multicol
            .filter(|style| style.horizontal)
            .and_then(|style| {
                crate::fragment::FragmentationContext::resolve(width, measure.column_height, style)
            })
    };
    let line_inline_size = if nested_fragmentation.is_some() {
        width
    } else {
        fragmentation.map_or(width, |context| context.column_width)
    };
    let physical_block_extent = if vertical {
        known_width.unwrap_or_else(|| match available.width {
            AvailableSpace::Definite(size) if measure.stretched_width => size,
            AvailableSpace::Definite(size) => size.max(min_content).min(max_content),
            AvailableSpace::MinContent => min_content,
            AvailableSpace::MaxContent => max_content,
        })
    } else {
        line_inline_size
    };
    let mut content_size = if vertical {
        shodo::geometry::PhysicalSize {
            width: physical_block_extent,
            height: line_inline_size,
        }
    } else {
        shodo::geometry::PhysicalSize {
            width: line_inline_size,
            height: known_height.unwrap_or(0.0),
        }
    };
    // A percentage `top` or `bottom` of an inline element refers to the
    // root's content height when it is definite: the known height or the
    // root's own `height`. A vertical root's content-sized height is not.
    let definite_height = known_height.or(measure.definite_height);
    // A percentage `left` or `right` refers to the whole content box, not to
    // the column the lines are broken in. A vertical root's physical width
    // is settled by the layout below, which resolves against it.
    let containing_width = (!vertical).then_some(width);
    // A vertical root's width is its block extent, definite when it is known
    // or stretched to the available space rather than sized by its lines.
    let vertical_width = (vertical && (known_width.is_some() || measure.stretched_width))
        .then_some(physical_block_extent);
    // The root's paragraph was taken above, so it is present.
    tree.nodes[idx].ifc.iter_mut().for_each(|root| {
        root.definite_height = definite_height;
        root.containing_width = containing_width;
        root.vertical_width = vertical_width;
    });
    let axes = IfcAxes::new(writing_mode, direction);
    // Every geometry of the root keeps its definite height, also when the
    // auto-width loop below lays it out again at a new width.
    let geometry_of = |content_size| {
        let mut geometry = flow::FlowGeometry::new(axes, content_size, measure.insets);
        geometry.definite_height = measure.definite_height;
        geometry.vertical_width = vertical_width;
        geometry
    };
    let mut geometry = geometry_of(content_size);
    // Lines, and the paragraph's own floats, are laid out by one loop; it
    // takes the engine state only around each call into the engine.
    let perform = measure.run_mode == RunMode::PerformLayout;
    let auto_vertical_block_size = vertical && !measure.stretched_width;
    let (mut lines, mut measured_block_size) = if auto_vertical_block_size {
        let mut lines = crate::layout::ifc::boxes::layout_with_boxes_in(
            tree,
            idx,
            geometry,
            block_ctx.as_deref_mut(),
            false,
            measure.top_margin_escapes,
            measure.bottom_margin_escapes,
        );
        let mut block_size = lines
            .height
            .min(measure.width_bounds.1)
            .max(measure.width_bounds.0);
        // Taffy's auto-width probe can only see the largest block child's
        // intrinsic width. A vertical IFC root instead needs the accumulated
        // block-axis extent of all its block children.
        for _ in 0..3 {
            if (block_size - geometry.content_size.width).abs() <= 0.01 {
                break;
            }
            content_size.width = block_size;
            geometry = geometry_of(content_size);
            lines = crate::layout::ifc::boxes::layout_with_boxes_in(
                tree,
                idx,
                geometry,
                block_ctx.as_deref_mut(),
                false,
                measure.top_margin_escapes,
                measure.bottom_margin_escapes,
            );
            block_size = lines
                .height
                .min(measure.width_bounds.1)
                .max(measure.width_bounds.0);
        }
        if (block_size - geometry.content_size.width).abs() > 0.01 {
            content_size.width = block_size;
            geometry = geometry_of(content_size);
        }
        (lines, block_size)
    } else {
        let lines = if let Some(context) = nested_fragmentation {
            crate::layout::ifc::boxes::layout_with_boxes_in_fragmentainers(
                tree, idx, geometry, context, perform,
            )
        } else {
            crate::layout::ifc::boxes::layout_with_boxes_in(
                tree,
                idx,
                geometry,
                block_ctx.as_deref_mut(),
                perform,
                measure.top_margin_escapes,
                measure.bottom_margin_escapes,
            )
        };
        (lines, physical_block_extent)
    };
    if auto_vertical_block_size && perform {
        lines = crate::layout::ifc::boxes::layout_with_boxes_in(
            tree,
            idx,
            geometry,
            block_ctx,
            true,
            measure.top_margin_escapes,
            measure.bottom_margin_escapes,
        );
        measured_block_size = lines
            .height
            .min(measure.width_bounds.1)
            .max(measure.width_bounds.0);
    }
    let fragments = lines
        .fragmentainer_line_ranges
        .clone()
        .map(|fragments| (fragments, lines.height))
        .or_else(|| {
            fragmentation.map(|context| crate::layout::root_column_fragments(&lines, context))
        });
    // The fragments of a paragraph laid out across the columns by the engine
    // come with its lines; only the ones split here still hold the boxes where
    // the lines stand in one tall column.
    if perform
        && !probe.boxes.is_empty()
        && lines.fragmentainer_line_ranges.is_none()
        && let Some((fragments, _)) = &fragments
    {
        crate::layout::ifc::boxes::move_atomics_to_columns(
            tree,
            &probe.boxes,
            &lines.lines,
            fragments,
        );
    }
    let content_height = fragments
        .as_ref()
        .map_or(lines.height, |(_, height)| *height);
    let size = if vertical {
        Size {
            width: if auto_vertical_block_size {
                measured_block_size
            } else {
                known_width.unwrap_or(content_height)
            },
            height: known_height.unwrap_or(width),
        }
    } else {
        Size {
            width,
            height: known_height.unwrap_or(content_height),
        }
    };
    // An inline-block's baseline is that of its last line box (CSS 2.1
    // 10.8.1); every other box exposes its first.
    let baseline =
        if tree.nodes[idx].display() == raikiri_style::property::DisplayValue::InlineBlock {
            flow::last_baseline(&lines)
        } else {
            flow::first_baseline(&lines)
        };
    let leading_margin = if measure.top_margin_escapes {
        lines
            .leading_block_margin
            .unwrap_or(CollapsibleMarginSet::ZERO)
    } else {
        CollapsibleMarginSet::ZERO
    };
    let escaping_margin = if known_height.is_none() {
        lines.escaping_margin
    } else {
        CollapsibleMarginSet::ZERO
    };
    if measure.run_mode == RunMode::PerformLayout
        && let Some(mut root) = tree.nodes[idx].ifc.take()
    {
        with_state(tree, |state| {
            flow::apply_text_overflow(&root, &mut lines, &mut state.layout_cx, geometry.width);
        });
        root.lines = Some(lines);
        root.multicol_fragments = fragments.map(|(fragments, _)| fragments);
        tree.nodes[idx].ifc = Some(root);
    }
    IfcMeasurement {
        size,
        first_baseline: baseline,
        top_margin: leading_margin,
        bottom_margin: escaping_margin,
    }
}

fn fragmentation_context_for_ifc(
    tree: &LayoutSession<'_>,
    idx: usize,
    root: &crate::layout::ifc::root::IfcRoot,
) -> Option<crate::fragment::FragmentationContext> {
    use crate::layout::ifc::boxes::IfcBoxKind;
    let context = *tree.context.fragmentation_stack.last()?;
    context.available_height?;
    if !root.boxes.iter().any(|box_| box_.kind == IfcBoxKind::Float) {
        return None;
    }
    if tree.nodes[idx].under_vertical_writing {
        return None;
    }

    let mut has_row_flex = false;
    for node_id in tree.ancestors(idx) {
        let node = &tree.layout_node_with_style(node_id).unwrap();
        if let Some(multicol) = node.multicol {
            return (has_row_flex && multicol.horizontal).then_some(context);
        }
        has_row_flex |= node.formatting_context == FormattingContext::Flex
            && matches!(
                node.style.flex_direction,
                taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
            );
    }
    None // cov:ignore: the unsupported ancestor path remains outside the ignored nested flex-float WPT scope.
}

/// Whether `overflow` makes a box a scroll container on either axis.
fn is_scroll_container(overflow: taffy::Point<taffy::Overflow>) -> bool {
    overflow.x.is_scroll_container() || overflow.y.is_scroll_container()
}

/// Keep each baseline of a scroll container within its border box, whose
/// block size is `height`.
fn clamp_baselines(baselines: &mut taffy::Baselines, height: f32) {
    for baseline in [&mut baselines.first, &mut baselines.last] {
        *baseline = baseline.map(|offset| offset.max(0.0).min(height));
    }
}

/// Return the first baseline of an inline-block from the descendant layout
/// tree: inline-blocks use the baseline of their last in-flow line box, so recover
/// the last in-flow text line from the descendant layout tree. See CSS 2.1
/// §10.8.1: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn first_inline_baseline(doc: &Document, root: usize) -> Option<f32> {
    use raikiri_style::property::DisplayValue;
    let root_node = doc.nodes.get(root)?;
    if root_node.is_display_none() {
        return None;
    }
    // A text node outside every paragraph has no line, so no baseline.
    if root_node.kind() == raikiri_traits::NodeKind::Text {
        return None;
    }
    // An ifc root has measured its own baseline from its lines; its children
    // carry no lines of their own.
    if root_node.display() != DisplayValue::InlineBlock
        || root_node.flags.contains(NodeFlags::IS_IFC_ROOT)
    {
        return None;
    }

    // A line found under a scroll container reports a baseline clamped to the
    // container's border box, as the container itself would export it (CSS
    // Box Alignment 3 §9.1). Each stack entry carries the top of its box in
    // the coordinates of `root` and the vertical range that clamps the
    // lines beneath it.
    // spec: https://www.w3.org/TR/css-align-3/#baseline-export
    let mut stack = Vec::new();
    for &child in root_node.children.iter().rev() {
        if doc.nodes[child].is_in_document()
            && !doc.nodes[child].is_display_none()
            && doc.nodes[child].taffy_position() != TaffyPosition::Absolute
        {
            stack.push((
                child,
                doc.nodes[child].unrounded_layout.location.y,
                (f32::NEG_INFINITY, f32::INFINITY),
            ));
        }
    }
    let mut last_baseline = None;
    while let Some((idx, offset_y, outer_clip)) = stack.pop() {
        let node = &doc.layout_node_with_style(idx).unwrap();
        // Clamping a value to this box and then to the boxes around it is
        // clamping it to this box's edges, each clamped to the outer range.
        let clip = if is_scroll_container(node.style.overflow) {
            let bottom = offset_y + node.unrounded_layout.size.height;
            (
                offset_y.max(outer_clip.0).min(outer_clip.1),
                bottom.max(outer_clip.0).min(outer_clip.1),
            )
        } else {
            outer_clip
        };
        if node.flags.contains(NodeFlags::IS_IFC_ROOT) {
            if let Some(line_baseline) = node.ifc.as_ref().and_then(|root| root.last_baseline()) {
                let layout = &node.unrounded_layout;
                let baseline = offset_y + layout.padding.top + layout.border.top + line_baseline;
                if baseline.is_finite() {
                    last_baseline = Some(baseline.max(clip.0).min(clip.1));
                }
            }
            continue;
        }
        if node.kind() == raikiri_traits::NodeKind::Text {
            continue;
        }
        for &child in node.children.iter().rev() {
            if doc.nodes[child].is_in_document()
                && !doc.nodes[child].is_display_none()
                && doc.nodes[child].taffy_position() != TaffyPosition::Absolute
            {
                stack.push((
                    child,
                    offset_y + doc.nodes[child].unrounded_layout.location.y,
                    clip,
                ));
            }
        }
    }
    last_baseline
}

/// Shrink-to-fit layout for a width:auto inline-block.
///
/// A width:auto inline-block sizes to its fit-content size: the preferred
/// max-content size clamped by the available width with the min-content size
/// as the floor (CSS 2.1 section 10.3.7 shrink-to-fit, CSS Sizing 3
/// fit-content). This compensates for the bridge mapping inline-block to the
/// taffy Block display, which would otherwise stretch-fill the containing
/// block like a plain block-level box.
///
/// The intrinsic min/max-content widths are measured by running the block (or
/// leaf, for childless boxes) algorithm directly in size-computation mode, not
/// through the cached entry point, so this same width:auto branch is not
/// re-entered for the node being measured. Measurement uses a fresh formatting
/// context, which is the correct shape here because an inline-block
/// establishes an independent context for its contents. The final pass fixes
/// the fitted width as a known dimension and runs the normal algorithm, so
/// style min/max clamps and child positioning behave exactly as they do for
/// an explicitly sized block of the same width.
///
/// Nodes with an explicit width never reach this function (the caller only
/// routes width:auto boxes here), and intrinsic-constraint callers
/// (min/max-content available space) short-circuit to their own bound, which
/// is what the fit-content formula reduces to under such a constraint.
fn compute_inline_block_shrink_wrap(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    // A size-contained box measured as if it had no content has no lines and
    // no natural size.
    let empty = tree.context.size_contained_measure.last() == Some(&idx);
    // An ifc root's children are hidden from the block algorithm: the inline
    // engine measures and lays out its content in the same steps below.
    let ifc_root = tree.nodes[idx].flags.contains(NodeFlags::IS_IFC_ROOT) && !empty;
    let display = tree.nodes[idx].display();
    let is_leaf = tree.nodes[idx].children.is_empty();
    // Clone what the leaf path needs before any exclusive tree use below.
    let mut leaf_style = tree.layout_node_with_style(idx).unwrap().style.clone();
    if empty {
        leaf_style.aspect_ratio = None;
    }
    let leaf_intrinsic = leaf_intrinsic_size(&tree.nodes[idx]).filter(|_| !empty);
    // Scalar copies so the measure closure below captures no large state.
    let sizing_mode = inputs.sizing_mode;
    let known_height = inputs.known_dimensions.height;
    let avail_height = inputs.available_space.height;

    // Intrinsic outer width under the given width constraint.
    let intrinsic_width = |tree: &mut LayoutSession<'_>, width: AvailableSpace| -> f32 {
        if is_leaf {
            compute_leaf_layout(
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode,
                    axis: RequestedAxis::Horizontal,
                    known_dimensions: Size::NONE,
                    available_space: Size {
                        width,
                        height: avail_height,
                    },
                    ..inputs
                },
                &leaf_style,
                |_val, _basis| 0.0,
                |known, _avail| Size {
                    width: known
                        .width
                        .or(leaf_intrinsic.map(|s| s.width))
                        .unwrap_or(0.0),
                    height: known
                        .height
                        .or(leaf_intrinsic.map(|s| s.height))
                        .unwrap_or(0.0),
                },
            )
            .size
            .width
        } else {
            let intrinsic_inputs = LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode,
                axis: RequestedAxis::Horizontal,
                known_dimensions: Size {
                    width: None,
                    height: known_height,
                },
                available_space: Size {
                    width,
                    height: avail_height,
                },
                ..inputs
            };
            match display {
                DisplayValue::InlineFlex => flex_layout(tree, node_id, intrinsic_inputs).size.width,
                DisplayValue::InlineGrid => {
                    compute_grid_layout(tree, node_id, intrinsic_inputs)
                        .size
                        .width
                }
                _ if ifc_root => {
                    compute_ifc_root(tree, node_id, intrinsic_inputs, None)
                        .size
                        .width
                }
                _ => {
                    block_layout(tree, node_id, intrinsic_inputs, None)
                        .size
                        .width
                }
            }
        }
    };

    let min_w = intrinsic_width(tree, AvailableSpace::MinContent);
    let max_w = intrinsic_width(tree, AvailableSpace::MaxContent);
    let (lo, hi) = if min_w <= max_w {
        (min_w, max_w)
    } else {
        (max_w, min_w)
    };
    let fit = match inputs.available_space.width {
        AvailableSpace::Definite(avail) => avail.clamp(lo, hi),
        AvailableSpace::MinContent => lo,
        AvailableSpace::MaxContent => hi,
    }
    .max(0.0);

    let final_inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(fit),
            height: inputs.known_dimensions.height,
        },
        available_space: Size {
            width: AvailableSpace::Definite(fit),
            height: inputs.available_space.height,
        },
        ..inputs
    };
    if is_leaf {
        compute_leaf_layout(
            final_inputs,
            &leaf_style,
            |_val, _basis| 0.0,
            |known, _avail| Size {
                width: known
                    .width
                    .or(leaf_intrinsic.map(|s| s.width))
                    .unwrap_or(0.0),
                height: known
                    .height
                    .or(leaf_intrinsic.map(|s| s.height))
                    .unwrap_or(0.0),
            },
        )
    } else {
        match display {
            DisplayValue::InlineFlex => flex_layout(tree, node_id, final_inputs),
            DisplayValue::InlineGrid => grid_layout(tree, node_id, final_inputs),
            _ if ifc_root => compute_ifc_root(tree, node_id, final_inputs, block_ctx),
            _ => block_layout(tree, node_id, final_inputs, block_ctx),
        }
    }
}

/// Lay out the block container `node_id` with taffy's block algorithm.
///
/// Every block container goes through here so that
/// [`taffy::LayoutBlockContainer::get_block_child_style`] knows the content-box width its
/// children's percentage padding and borders resolve against. A container
/// in a vertical writing mode is laid out on transposed axes instead (see
/// `vertical_block`), where taffy resolves those percentages against its
/// inline size, the physical height.
///
/// A container whose width is not definite yet (an auto-width float, say)
/// only learns it from its contents, after taffy has listed the children.
/// When such a container starts a formatting context of its own and a child
/// has percentage vertical edges, it is laid out a second time with the
/// width the first pass derived.
pub(crate) fn block_layout(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    mut block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    if let Some(right_to_left) = vertical_block::block_right_to_left(tree, idx) {
        let output = vertical_block::compute_vertical_block(tree, node_id, inputs, right_to_left);
        center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, None);
        return output;
    }
    let (width, inline_inset) = tree.block_definite_width(idx, &inputs);
    let basis = width.map(|width| width - inline_inset);
    tree.context.block_item_percent_bases.push((idx, basis));
    let mut output = compute_block_flow(tree, node_id, inputs, block_ctx.as_deref_mut());
    tree.context.block_item_percent_bases.pop();
    if basis.is_none() && block_ctx.is_none() && tree.has_percent_vertical_edge_child(idx) {
        let basis = Some(output.size.width - inline_inset);
        tree.context.block_item_percent_bases.push((idx, basis));
        output = compute_block_flow(tree, node_id, inputs, None);
        tree.context.block_item_percent_bases.pop();
    }
    center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, None);
    output
}

/// `inputs` with the axes in which `node_id` has size containment sized as
/// if the box had no content (CSS Containment 2 §3.1, §3.2).
///
/// The box is measured once with the session's `size_contained_measure`
/// hiding its children, through the same layout algorithm and with the same
/// constraints, so that its padding, borders, sizing properties and, for a
/// grid, its explicit tracks still count. The size found becomes a known
/// dimension, in which the content is then laid out and may overflow.
/// A height found this way is not definite for the percentages inside: the
/// box is no more definite than without containment. A width always is, as
/// the used width of any box.
fn size_contained_inputs(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> LayoutInput {
    let idx = usize::from(node_id);
    let contained = tree.nodes[idx].size_containment;
    let width = contained.width && inputs.known_dimensions.width.is_none();
    let height = contained.height && inputs.known_dimensions.height.is_none();
    if !width && !height {
        return inputs;
    }
    tree.context.size_contained_measure.push(idx);
    let empty = tree
        .compute_child_layout(
            node_id,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                axis: RequestedAxis::Both,
                ..inputs
            },
        )
        .size;
    tree.context.size_contained_measure.pop();
    let mut inputs = inputs;
    if width {
        inputs.known_dimensions.width = Some(empty.width);
    }
    if height {
        inputs.known_dimensions.height = Some(empty.height);
        inputs.known_dimensions_are_definite.height = false;
    }
    inputs
}

/// Split negative free space between the two auto block-axis margins of
/// the absolutely positioned children of block, flex or grid container
/// `idx`.
///
/// When the block-axis insets and size are all non-auto and both
/// block-axis margins are auto, CSS 2.1 §10.6.4 solves for the margins
/// under the constraint that they are equal, with no exception for a
/// negative result, unlike the inline-axis rule of §10.3.7. CSS Writing
/// Modes 4 §7.1 applies each rule to the axis of the containing block's
/// writing mode. Taffy 0.14 applies the inline-axis rule to both physical
/// axes and zeroes or drops a negative margin instead, which leaves an
/// overconstrained box at its start inset rather than centered on its
/// insets. The block axis is the physical height in `horizontal-tb` and the
/// physical width in every other mode.
///
/// The children's containing block is the container's padding box, or for
/// the children of a grid container, the grid area their grid-placement
/// properties resolve to in `grid` (CSS Grid 1 §9.1).
///
/// `size` is the container's border box from the layout taffy just
/// performed; the children's positions are recomputed from their insets, so
/// running this again on a cached layout changes nothing.
fn center_absolute_children_in_block_axis(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
    size: Size<f32>,
    grid: Option<&DetailedGridInfo<String>>,
) {
    use taffy::{MaybeResolve, Overflow, ResolveOrZero};

    if inputs.run_mode != RunMode::PerformLayout {
        return;
    }
    let block_is_height = !used_writing_mode(tree, idx).is_vertical();
    let style = &tree.layout_node_with_style(idx).unwrap().style;
    let border = style
        .border
        .resolve_or_zero(inputs.parent_size.width, resolve_calc);
    let direction = style.direction;
    // A scrollbar takes its gutter from the absolute positioning area: a
    // horizontal one from the bottom, a vertical one from the inline end.
    let gutter = |overflow: Overflow| {
        if overflow == Overflow::Scroll {
            style.scrollbar_width
        } else {
            0.0
        }
    };
    let (gutter_x, gutter_y) = (gutter(style.overflow.y), gutter(style.overflow.x));
    let padding_box = if direction == taffy::Direction::Rtl {
        taffy::Rect {
            left: border.left + gutter_x,
            right: size.width - border.right,
            top: border.top,
            bottom: size.height - border.bottom - gutter_y,
        }
    } else {
        taffy::Rect {
            left: border.left,
            right: size.width - border.right - gutter_x,
            top: border.top,
            bottom: size.height - border.bottom - gutter_y,
        }
    };
    let children: Vec<usize> = TaffyChildIter::new_for_session(tree, NodeId::from(idx))
        .map(usize::from)
        .filter(|&child| {
            let style = &tree.layout_node_with_style(child).unwrap().style;
            let (start, end) = if block_is_height {
                (style.margin.top, style.margin.bottom)
            } else {
                (style.margin.left, style.margin.right)
            };
            tree.nodes[child].taffy_position() == TaffyPosition::Absolute
                && !tree.nodes[child].is_display_none()
                && start.is_auto()
                && end.is_auto()
        })
        .collect();
    for child in children {
        let node = &tree.layout_node_with_style(child).unwrap();
        let style = &node.style;
        let area = match grid {
            Some(grid) => grid.resolve_absolute_grid_area(
                style.grid_row.clone(),
                style.grid_column.clone(),
                direction,
                padding_box,
            ),
            None => padding_box,
        };
        let (area_start, area_end, start, end) = if block_is_height {
            (area.top, area.bottom, style.inset.top, style.inset.bottom)
        } else {
            (area.left, area.right, style.inset.left, style.inset.right)
        };
        let area_size = area_end - area_start;
        let start = start.maybe_resolve(area_size, resolve_calc);
        let end = end.maybe_resolve(area_size, resolve_calc);
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        let mut layout = node.unrounded_layout;
        let box_size = if block_is_height {
            layout.size.height
        } else {
            layout.size.width
        };
        let free_space = area_size - start - end - box_size;
        if free_space >= 0.0 {
            continue;
        }
        let margin = free_space / 2.0;
        let position = area_start + start + margin;
        if block_is_height {
            layout.margin.top = margin;
            layout.margin.bottom = margin;
            layout.location.y = position;
        } else {
            layout.margin.left = margin;
            layout.margin.right = margin;
            layout.location.x = position;
        }
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
}

/// Lay out the flex container `node_id` with taffy's flexbox algorithm,
/// recording the content-box width its items' percentage edges resolve
/// against while taffy measures them without one; see
/// [`ifc_parent_inline_size`].
pub(crate) fn flex_layout(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let inputs = with_fit_content_width(tree, node_id, inputs);
    let idx = usize::from(node_id);
    let (size, inset, _) = tree.block_definite_size(idx, &inputs);
    let basis = Size {
        width: size.width.map(|width| (width - inset.width).max(0.0)),
        height: size.height.map(|height| (height - inset.height).max(0.0)),
    };
    tree.context.flex_item_percent_bases.push((idx, basis));
    let mut output = compute_flexbox_layout(tree, node_id, inputs);
    oof::layout_local_oof(tree, node_id, &mut output);
    tree.context.flex_item_percent_bases.pop();
    center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, None);
    output
}

/// Give a flex container with a `fit-content` width the width CSS Sizing 3
/// §3.1.1 defines for it: the room offered, but at least its min-content and at
/// most its max-content width. Taffy's flexbox algorithm reads the keyword as
/// `max-content`.
fn with_fit_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
) -> LayoutInput {
    let AvailableSpace::Definite(room) = inputs.available_space.width else {
        return inputs;
    };
    if inputs.known_dimensions.width.is_some()
        || !tree
            .layout_node_with_style(usize::from(node_id))
            .is_some_and(|node| {
                matches!(
                    node.style.size.width.expand(),
                    taffy::ExpandedDimension::FitContent
                )
            })
    {
        return inputs;
    }
    let mut measure = |space: AvailableSpace| {
        let mut probe = inputs;
        probe.run_mode = RunMode::ComputeSize;
        probe.axis = RequestedAxis::Horizontal;
        probe.available_space.width = space;
        flex_layout(tree, node_id, probe).size.width
    };
    let min_content = measure(AvailableSpace::MinContent);
    let max_content = measure(AvailableSpace::MaxContent);
    inputs.known_dimensions.width = Some(max_content.min(min_content.max(room)));
    inputs.known_dimensions_are_definite.width = true;
    inputs
}

/// Lay out the grid container `node_id` with taffy's grid algorithm.
fn grid_layout(tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
    let idx = usize::from(node_id);
    let mut output = compute_grid_layout(tree, node_id, inputs);
    oof::layout_local_oof(tree, node_id, &mut output);
    // Taffy reports the tracks only from a layout it performs.
    let grid = tree.context.grid_layout_info.remove(&idx);
    if let Some(taffy::DetailedLayoutInfo::Grid(grid)) = grid {
        center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, Some(&grid));
    }
    align_auto_grid_abspos_children(
        tree,
        idx,
        inputs.run_mode,
        inputs.parent_size.width,
        output.size,
    );
    output
}

/// Share Grid static alignment with fixed-box relayout, where page-area
/// sizing can change the child's margin-box size after this dispatch.
///
/// `overflows` tells, per axis, whether the child's margin box is larger than
/// the content box it is aligned in: a safe alignment of an overflowing box
/// aligns as `start` (CSS Box Alignment 3 §4.4).
// spec: https://www.w3.org/TR/css-align-3/#overflow-values
pub(crate) fn auto_grid_abspos_static_alignment(
    child: &Style,
    parent: &Style,
    overflows: Size<bool>,
) -> Option<Size<f32>> {
    use taffy::{AlignItemsKeyword, AlignmentSafety, GridPlacement};

    if child.position != TaffyPosition::Absolute
        || !child.inset.top.is_auto()
        || !child.inset.right.is_auto()
        || !child.inset.bottom.is_auto()
        || !child.inset.left.is_auto()
        || !matches!(&child.grid_row.start, GridPlacement::Auto)
        || !matches!(&child.grid_row.end, GridPlacement::Auto)
        || !matches!(&child.grid_column.start, GridPlacement::Auto)
        || !matches!(&child.grid_column.end, GridPlacement::Auto)
    {
        return None;
    }

    // `self-start` and `self-end` follow the box's own direction where the
    // axis is its inline axis (CSS Box Alignment 3 §5.2).
    let alignment_fraction = |self_alignment: Option<taffy::AlignSelf>,
                              parent_alignment: taffy::AlignItems,
                              inline: bool,
                              overflows: bool| {
        let alignment = self_alignment
            .unwrap_or(parent_alignment)
            .resolve_self_relative(child.direction, parent.direction, inline);
        let keyword = if alignment.safety == AlignmentSafety::Safe && overflows {
            AlignItemsKeyword::Start
        } else {
            alignment.keyword
        };
        match keyword {
            AlignItemsKeyword::Center => 0.5,
            AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd => 1.0,
            _ => 0.0,
        }
    };
    let inline = alignment_fraction(
        child.justify_self,
        parent.justify_items,
        true,
        overflows.width,
    );
    Some(Size {
        width: if parent.direction == taffy::Direction::Rtl {
            1.0 - inline
        } else {
            inline
        },
        height: alignment_fraction(
            child.align_self,
            parent.align_items,
            false,
            overflows.height,
        ),
    })
}

/// Align auto-placed out-of-flow children to this grid's content box.
///
/// Taffy's static-position fallback uses the grid's border edge. CSS Grid 2
/// §10.2 instead aligns the margin box within the content box using the
/// child's `justify-self` and `align-self`, which fall back to the grid's
/// `justify-items` and `align-items`.
fn align_auto_grid_abspos_children(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    run_mode: RunMode,
    percent_basis: Option<f32>,
    size: Size<f32>,
) {
    use taffy::ResolveOrZero;

    if run_mode != RunMode::PerformLayout {
        return;
    }

    let parent_style = &tree.layout_node_with_style(idx).unwrap().style;
    // Percentage padding resolves against the containing block's width, not
    // this grid's used width.
    let border = parent_style
        .border
        .resolve_or_zero(percent_basis, resolve_calc);
    let padding = parent_style
        .padding
        .resolve_or_zero(percent_basis, resolve_calc);
    let content_width =
        (size.width - border.left - border.right - padding.left - padding.right).max(0.0);
    let content_height =
        (size.height - border.top - border.bottom - padding.top - padding.bottom).max(0.0);

    let placed: Vec<(usize, taffy::Layout)> =
        TaffyChildIter::new_for_session(tree, NodeId::from(idx))
            .filter_map(|child| {
                let child = usize::from(child);
                if tree.nodes[child].taffy_position() != TaffyPosition::Absolute
                    || tree.nodes[child].is_display_none()
                {
                    return None;
                }
                let node = tree.layout_node_with_style(child).unwrap();
                let style = &node.style;
                let mut layout = node.unrounded_layout;
                // With every inset `auto`, auto margins are zero (CSS 2.1 §10.3.7,
                // §10.6.4).
                let margin = taffy::Rect {
                    left: if style.margin.left.is_auto() {
                        0.0
                    } else {
                        layout.margin.left
                    },
                    right: if style.margin.right.is_auto() {
                        0.0
                    } else {
                        layout.margin.right
                    },
                    top: if style.margin.top.is_auto() {
                        0.0
                    } else {
                        layout.margin.top
                    },
                    bottom: if style.margin.bottom.is_auto() {
                        0.0
                    } else {
                        layout.margin.bottom
                    },
                };
                let free_width = content_width - layout.size.width - margin.left - margin.right;
                let free_height = content_height - layout.size.height - margin.top - margin.bottom;
                let overflows = Size {
                    width: free_width < 0.0,
                    height: free_height < 0.0,
                };
                let alignment = auto_grid_abspos_static_alignment(style, parent_style, overflows)?;
                layout.location.x =
                    border.left + padding.left + margin.left + alignment.width * free_width;
                layout.location.y =
                    border.top + padding.top + margin.top + alignment.height * free_height;
                Some((child, layout))
            })
            .collect();

    for (child, layout) in placed {
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
}

impl LayoutSession<'_> {
    /// Whether the bridged writing mode of `idx` has a vertical inline axis.
    fn has_vertical_writing_mode(&self, idx: usize) -> bool {
        self.nodes[idx].used_writing_mode.is_vertical()
    }

    /// The border-box width of block container `idx` that taffy's block
    /// algorithm derives from `inputs` before it lists the container's
    /// children (the definite size the container is given or styled with,
    /// `None` when there is none yet), and the padding, border and vertical
    /// scrollbar gutter that separate it from the content box.
    fn block_definite_width(&self, idx: usize, inputs: &LayoutInput) -> (Option<f32>, f32) {
        let (size, inset, _) = self.block_definite_size(idx, inputs);
        (size.width, inset.width)
    }

    /// The content-box height that percentages inside block container `idx`
    /// resolve against when taffy's block algorithm lays it out with
    /// `inputs`: the definite height it is given, else its own `height`
    /// (at least its `min-height`), else `None`.
    fn block_percent_basis_height(&self, idx: usize, inputs: &LayoutInput) -> Option<f32> {
        let (size, inset, style_height) = self.block_definite_size(idx, inputs);
        size.height
            .filter(|_| inputs.known_dimensions_are_definite.height)
            .or(style_height)
            .map(|height| height - inset.height)
    }

    /// The border-box size of block container `idx` that taffy's block
    /// algorithm derives from `inputs` before it lists the container's
    /// children, the padding, border and scrollbar gutters that separate it
    /// from the content box along each axis, and the border-box height its
    /// own `height` and `min-height` give it.
    ///
    /// Mirrors [`taffy::compute_block_layout`] and its `compute_inner`.
    fn block_definite_size(
        &self,
        idx: usize,
        inputs: &LayoutInput,
    ) -> (Size<Option<f32>>, Size<f32>, Option<f32>) {
        use taffy::{BoxSizing, MaybeMath, MaybeResolve, Overflow, ResolveOrZero, SizingMode};

        let style = &self.layout_node_with_style(idx).unwrap().style;
        let parent_size = inputs.parent_size;
        let padding = style
            .padding
            .resolve_or_zero(parent_size.width, resolve_calc);
        let border = style
            .border
            .resolve_or_zero(parent_size.width, resolve_calc);
        let padding_border_size = (padding + border).sum_axes();
        let box_sizing_adjustment = if style.box_sizing == BoxSizing::ContentBox {
            padding_border_size
        } else {
            Size::ZERO
        };
        let aspect_ratio = style.aspect_ratio;
        let min_size = style
            .min_size
            .maybe_resolve(parent_size, resolve_calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let max_size = style
            .max_size
            .maybe_resolve(parent_size, resolve_calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let style_size = style
            .size
            .maybe_resolve(parent_size, resolve_calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let clamped_style_size = if inputs.sizing_mode == SizingMode::InherentSize {
            style_size.maybe_clamp(min_size, max_size)
        } else {
            Size::NONE
        };
        let min_max_definite_size = min_size.zip_map(max_size, |min, max| match (min, max) {
            (Some(min), Some(max)) if max <= min => Some(min),
            _ => None,
        });
        let known = inputs
            .known_dimensions
            .or(min_max_definite_size)
            .or(clamped_style_size)
            .maybe_max(padding_border_size);
        let derived = known
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_clamp(min_size, max_size);
        // A vertical scrollbar takes its gutter from the inline axis, and a
        // horizontal one from the block axis.
        let gutter = |overflow| {
            if overflow == Overflow::Scroll {
                style.scrollbar_width
            } else {
                0.0
            }
        };
        (
            Size {
                width: known.width.or(derived.width),
                height: known.height.or(derived.height),
            },
            Size {
                width: padding.horizontal_axis_sum()
                    + border.horizontal_axis_sum()
                    + gutter(style.overflow.y),
                height: padding.vertical_axis_sum()
                    + border.vertical_axis_sum()
                    + gutter(style.overflow.x),
            },
            style_size.height.maybe_max(min_size.height),
        )
    }

    /// Whether an in-flow child of block container `idx` has a top or bottom
    /// padding or border that depends on its percentage basis.
    fn has_percent_vertical_edge_child(&self, idx: usize) -> bool {
        TaffyChildIter::new_for_session(self, NodeId::from(idx)).any(|child| {
            let Some(node) = self.layout_node_with_style(usize::from(child)) else {
                return false;
            };
            let style = node.style;
            node.taffy_position() != TaffyPosition::Absolute
                && [
                    style.padding.top,
                    style.padding.bottom,
                    style.border.top,
                    style.border.bottom,
                ]
                .into_iter()
                .any(edge_depends_on_basis)
        })
    }
}

/// A borrowed style query with optional resolved block-child edges.
///
/// Project block flow supplies its percentage basis here. The public
/// out-of-flow adapter uses unresolved edges and captures CSS table identity
/// for [`taffy::LayoutContainingBlock::get_oof_item_style`].
#[derive(Clone, Copy)]
pub struct BlockChildStyle<'a> {
    style: &'a Style,
    padding: taffy::Rect<taffy::LengthPercentage>,
    border: taffy::Rect<taffy::LengthPercentage>,
    oof_table: bool,
}

impl<'a> BlockChildStyle<'a> {
    /// `percent_basis` is the container's content-box width, or `None` to
    /// keep taffy's own resolution.
    fn new(style: &'a Style, percent_basis: Option<f32>) -> Self {
        let resolve_vertical = |edges: taffy::Rect<taffy::LengthPercentage>, basis: f32| {
            let resolve = |edge: taffy::LengthPercentage| {
                use taffy::ResolveOrZero;
                taffy::LengthPercentage::length(edge.resolve_or_zero(Some(basis), resolve_calc))
            };
            taffy::Rect {
                top: resolve(edges.top),
                bottom: resolve(edges.bottom),
                ..edges
            }
        };
        match percent_basis {
            Some(basis) => Self {
                style,
                padding: resolve_vertical(style.padding, basis),
                border: resolve_vertical(style.border, basis),
                oof_table: taffy::OofItemStyle::is_table(style),
            },
            None => Self {
                style,
                padding: style.padding,
                border: style.border,
                oof_table: taffy::OofItemStyle::is_table(style),
            },
        }
    }
}

impl CoreStyle for BlockChildStyle<'_> {
    type CustomIdent = String;

    fn box_generation_mode(&self) -> BoxGenerationMode {
        self.style.box_generation_mode()
    }
    fn is_block(&self) -> bool {
        self.style.is_block()
    }
    fn is_compressible_replaced(&self) -> bool {
        self.style.is_compressible_replaced()
    }
    fn is_replaced(&self) -> bool {
        self.style.is_replaced()
    }
    fn box_sizing(&self) -> taffy::BoxSizing {
        self.style.box_sizing()
    }
    fn direction(&self) -> taffy::Direction {
        self.style.direction()
    }
    fn overflow(&self) -> taffy::Point<taffy::Overflow> {
        self.style.overflow()
    }
    fn scrollbar_width(&self) -> f32 {
        self.style.scrollbar_width()
    }
    fn position(&self) -> TaffyPosition {
        self.style.position()
    }
    fn inset(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.style.inset()
    }
    fn size(&self) -> Size<taffy::Dimension> {
        self.style.size()
    }
    fn min_size(&self) -> Size<taffy::LengthPercentageAuto> {
        self.style.min_size()
    }
    fn max_size(&self) -> Size<taffy::LengthPercentageAuto> {
        self.style.max_size()
    }
    fn aspect_ratio(&self) -> Option<f32> {
        self.style.aspect_ratio()
    }
    fn margin(&self) -> taffy::Rect<taffy::LengthPercentageAuto> {
        self.style.margin()
    }
    fn padding(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.padding
    }
    fn border(&self) -> taffy::Rect<taffy::LengthPercentage> {
        self.border
    }
    fn contain(&self) -> taffy::Contain {
        self.style.contain()
    }
}

impl taffy::BlockItemStyle for BlockChildStyle<'_> {
    fn is_table(&self) -> bool {
        taffy::BlockItemStyle::is_table(self.style)
    }
    fn float(&self) -> taffy::Float {
        taffy::BlockItemStyle::float(self.style)
    }
    fn clear(&self) -> taffy::Clear {
        taffy::BlockItemStyle::clear(self.style)
    }
}

impl taffy::OofItemStyle for BlockChildStyle<'_> {
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.style.align_self
    }
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.style.justify_self
    }
    fn is_table(&self) -> bool {
        self.oof_table
    }
    fn grid_row(&self) -> taffy::Line<taffy::GridPlacement<String>> {
        self.style.grid_row.clone()
    }
    fn grid_column(&self) -> taffy::Line<taffy::GridPlacement<String>> {
        self.style.grid_column.clone()
    }
}

impl LayoutBlockContainer for LayoutSession<'_> {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = BlockChildStyle<'a>
    where
        Self: 'a;

    fn get_block_container_style(&self, node_id: NodeId) -> Self::BlockContainerStyle<'_> {
        let idx = usize::from(node_id);
        self.transposed_container_style(idx)
            .unwrap_or(self.layout_node_with_style(idx).unwrap().style)
    }

    /// Taffy also reads a child's style when it places the child as an
    /// absolutely positioned box, against a different basis (the padding
    /// box of the containing block), so only in-flow children of the
    /// container on top of the session's block item percent basis get their
    /// vertical edges resolved here. Neither do the children of a container
    /// whose width is not known yet when taffy lists them (an auto-width
    /// float, say): taffy sizes such a container from its contents only
    /// afterwards, and a zero inset fixed now could not follow that width.
    fn get_block_child_style(&self, child_node_id: NodeId) -> Self::BlockItemStyle<'_> {
        let child = usize::from(child_node_id);
        if let Some(style) = self.transposed_child_style(child) {
            return BlockChildStyle::new(style, None);
        }
        let style = self.layout_node_with_style(child).unwrap().style;
        let basis = self
            .context
            .block_item_percent_bases
            .last()
            .filter(|&&(container, _)| {
                self.nodes[child].taffy_position() != TaffyPosition::Absolute
                    && self.taffy_parent_of(child) == Some(container)
            })
            .and_then(|&(_, basis)| basis);
        BlockChildStyle::new(style, basis)
    }
}

impl LayoutFlexboxContainer for LayoutSession<'_> {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node_id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.layout_node_with_style(usize::from(node_id))
            .unwrap()
            .style
    }

    fn get_flexbox_child_style(&self, child_node_id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.layout_node_with_style(usize::from(child_node_id))
            .unwrap()
            .style
    }
}

impl LayoutGridContainer for LayoutSession<'_> {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node_id: NodeId) -> Self::GridContainerStyle<'_> {
        self.layout_node_with_style(usize::from(node_id))
            .unwrap()
            .style
    }

    fn get_grid_child_style(&self, child_node_id: NodeId) -> Self::GridItemStyle<'_> {
        self.layout_node_with_style(usize::from(child_node_id))
            .unwrap()
            .style
    }

    fn set_detailed_grid_info(
        &mut self,
        node_id: NodeId,
        detailed_grid_info: DetailedGridInfo<Self::CustomIdent>,
    ) {
        let parent = usize::from(node_id);
        let children = TaffyChildIter::new_for_session(self, node_id)
            .map(usize::from)
            .filter(|&child| {
                self.nodes[child].taffy_display() != taffy::Display::None
                    && self.nodes[child].taffy_position() != TaffyPosition::Absolute
            })
            .collect::<Vec<_>>();

        // Taffy builds `DetailedGridInfo.items` from its in-flow children in
        // source order. Pair those resolved rows with the same child projection
        // that Taffy received, so pagination can order by actual placement rather
        // than trying to reconstruct auto-placement or relative offsets.
        let grid_column_count = detailed_grid_info.columns.positions.len();
        debug_assert_eq!(children.len(), detailed_grid_info.items.len());
        self.nodes[parent].grid_item_row_starts = children
            .iter()
            .zip(detailed_grid_info.items.iter())
            .map(|(&child, item)| (child, item.row_start))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let rows = row_sizing_functions(
            self.layout_node_with_style(parent).unwrap().style,
            &detailed_grid_info.rows,
        );
        self.nodes[parent].grid_items_without_baseline = children
            .iter()
            .zip(detailed_grid_info.items.iter())
            .filter(|&(&child, item)| {
                let spanned = usize::from(item.row_start.saturating_sub(1))
                    ..usize::from(item.row_end.saturating_sub(1));
                self.layout_style_for_key(child)
                    .unwrap()
                    .size
                    .height
                    .into_raw()
                    .uses_percentage()
                    && rows.get(spanned).is_some_and(|rows| {
                        rows.iter().any(|row| {
                            row.min.is_intrinsic() || row.max.is_intrinsic() || row.max.is_fr()
                        })
                    })
            })
            .map(|(&child, _)| child)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.nodes[parent].grid_item_column_starts = children
            .into_iter()
            .zip(detailed_grid_info.items.iter())
            .map(|(child, item)| (child, item.column_start))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.nodes[parent].grid_column_count = grid_column_count;
        self.context.grid_layout_info.insert(
            parent,
            taffy::DetailedLayoutInfo::Grid(Box::new(detailed_grid_info)),
        );
    }
}

impl TraversePartialTree for LayoutSession<'_> {
    type ChildIter<'a>
        = TaffyChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        TaffyChildIter::new_for_session(self, node_id)
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        let parent = usize::from(node_id);
        let count = TaffyChildIter::new_for_session(self, node_id).count();
        let children = (count >= WALKED_CHILDREN_MIN)
            .then(|| TaffyChildIter::new_for_session(self, node_id).collect());
        if let Some(mut state) = self.node_state(parent) {
            state.taffy_child_walks = children;
        }
        count
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        let parent = usize::from(node_id);
        if let Some(mut state) = self.node_state(parent)
            && let Some(children) = state.taffy_child_walks.as_ref()
            && let Some(&child) = children.get(index)
        {
            if children.len() == index + 1 {
                state.taffy_child_walks = None;
            }
            return child;
        }
        TaffyChildIter::new_for_session(self, node_id)
            .nth(index)
            .expect("get_child_id: index out of range")
    }
}

impl TraverseTree for LayoutSession<'_> {}

impl TraversePartialTree for Document {
    type ChildIter<'a>
        = TaffyChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node_id: NodeId) -> Self::ChildIter<'_> {
        TaffyChildIter::new(self, node_id, false)
    }

    fn child_count(&self, node_id: NodeId) -> usize {
        TaffyChildIter::new(self, node_id, false).count()
    }

    fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
        TaffyChildIter::new(self, node_id, false)
            .nth(index)
            .expect("get_child_id: index out of range")
    }
}

impl TraverseTree for Document {}

/// The sizing function of every row of the grid laid out with `style`, from
/// the first implicit row before the explicit grid to the last after it,
/// as taffy's grid algorithm expands them for the track counts in `rows`
/// (CSS Grid 1 §7.2 explicit tracks, §7.6 implicit tracks).
fn row_sizing_functions(
    style: &Style,
    rows: &taffy::DetailedGridTracksInfo<String>,
) -> Vec<taffy::TrackSizingFunction> {
    use taffy::prelude::TaffyAuto;
    use taffy::{GridTemplateComponent, RepetitionCount, TrackSizingFunction};

    let negative = usize::from(rows.negative_implicit_tracks);
    let explicit = usize::from(rows.explicit_tracks);
    let positive = usize::from(rows.positive_implicit_tracks);
    let auto_rows = &style.grid_auto_rows;
    // Without `grid-auto-rows`, implicit rows are `auto`.
    let implicit = |index: usize| {
        let count = auto_rows.len().max(1);
        auto_rows
            .get(index % count)
            .copied()
            .unwrap_or(TrackSizingFunction::AUTO)
    };
    let mut functions = Vec::with_capacity(negative + explicit + positive);
    // The implicit rows before the explicit grid end with the last of
    // `grid-auto-rows`.
    let offset = if auto_rows.is_empty() {
        0
    } else {
        auto_rows.len() - negative % auto_rows.len()
    };
    functions.extend((0..negative).map(|index| implicit(offset + index)));
    // An automatic repetition takes the explicit rows that the other
    // components leave.
    let fixed_count = style
        .grid_template_rows
        .iter()
        .map(|component| match component {
            GridTemplateComponent::Single(_) => 1,
            GridTemplateComponent::Repeat(repeat) => match repeat.count {
                RepetitionCount::Count(count) => repeat.tracks.len() * usize::from(count),
                RepetitionCount::AutoFill | RepetitionCount::AutoFit => 0,
            },
        })
        .sum::<usize>();
    let mut template = Vec::with_capacity(explicit);
    for component in &style.grid_template_rows {
        match component {
            GridTemplateComponent::Single(function) => template.push(*function),
            GridTemplateComponent::Repeat(repeat) => {
                let count = match repeat.count {
                    RepetitionCount::Count(count) => repeat.tracks.len() * usize::from(count),
                    RepetitionCount::AutoFill | RepetitionCount::AutoFit => {
                        explicit.saturating_sub(fixed_count)
                    }
                };
                template.extend(repeat.tracks.iter().copied().cycle().take(count));
            }
        }
    }
    template.truncate(explicit);
    let template_len = template.len();
    functions.extend(template);
    // Explicit rows the template does not reach, if any, are sized as the
    // implicit rows after them.
    functions.extend((0..explicit - template_len + positive).map(implicit));
    functions
}

// SAFETY: with taffy calc support enabled, taffy length types hold
// a raw pointer to a caller-owned calc payload through a compact tagged
// representation. Raw pointers are not Send, making `Style` not Send and
// thus `Document` not Send without this impl.
//
// Invariant, self-contained arena approach:
// Every calc pointer in a retained style points to an immutable payload held
// by the same Document's `RetainedLayoutStyles`. Its Arc owners keep pointees
// stable across vector growth and `Document::clone()`, so moving the Document
// to another thread preserves pointer validity.
// The bridge retains new payloads before publishing their pointers in the
// style map. It keeps old payloads through errors and unwinds, then drains
// the previous prefix only after a complete successful rebuild.
//
// Enforcement at the boundary:
// The public element constructor sanitizes any externally supplied style
// before storage, replacing foreign calc handles with safe keyword
// fallbacks. Cross-document copies go through the same constructor, so the
// target never adopts the source arena's pointers. The bridge rebuilds
// owned handles from computed values before each layout pass, so valid
// styles without calc pass through untouched.
//
// Do not implement Sync: the planned parallel layout path uses an owned
// deep copy or a read-only borrow of style data. No path reads a shared
// Document reference concurrently across threads, so Sync is unnecessary.
// blitz-dom added an unsafe Sync impl for its node type to serve a parallel
// style traversal that raikiri does not have.
//
// Precedent: blitz-dom Node has a similar unsafe Send impl without
// annotation, an established taffy plus calc pattern. blitz uses an external
// arena model where a separate computed-values chain owns calc data;
// raikiri uses a self-contained arena model with different invariants.
#[allow(unsafe_code)]
unsafe impl Send for Document {}
