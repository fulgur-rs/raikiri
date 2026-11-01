//! The layout dispatch of [`LayoutSession`].
//!
//! The session lays a box out by dispatching on its display: block, flex, grid,
//! table, multicolumn and leaf layouts, with a per-node result cache. The layout
//! library's algorithms call back into the session through the trait
//! implementations in [`crate::layout::taffy_adapter`], which convert their
//! arguments to the types the methods here take:
//!
//! - `child_ids` / `child_count` / `get_child_id`: the children of a node
//! - `cache_get` / `cache_store` / `cache_clear`: the per-node result cache
//! - `compute_child_layout`: the dispatch on the display of the node
//! - `set_unrounded_layout` / `set_detailed_grid_info`: where results are stored

use crate::layout::protocol::{
    AvailableSpace, Baselines, CollapsibleMarginSet, LayoutInput, LayoutOutput, NodeId,
    OofCandidates, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::taffy_adapter::algorithms::{
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_leaf_layout,
};
use crate::layout::taffy_adapter::{GridInfo, RowSizing};
use raikiri_layout_geometry::{BoxLayout, Edges, Line, MaybeMath, Point, Size};
use raikiri_layout_style::BoxStyle as Style;
use raikiri_layout_style::values::{
    AlignContentKeyword, AlignItems, AlignSelf, BoxGenerationMode, BoxSizing, CalcRef, Contain,
    Dimension, Direction, Display, ExpandedDimension, ExpandedLengthPercentageAuto, Float,
    LengthPercentage, LengthPercentageAuto, MaybeResolve, Overflow, Position, ResolveOrZero,
};

use self::block_flow::{compute_block_flow, context::BlockContext};
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::ifc::geometry::IfcAxes;
use crate::node::{FormattingContext, NodeData, NodeFlags};
use raikiri_layout_style::{
    LayoutStyleView, LeafStyle, TaffyStyle, projected_calc_is_live, resolve_projected_calc,
};
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::property::DisplayValue;

/// The intrinsic size of a leaf: an `<img>`'s resolved size, `None` for any
/// other leaf (a text node outside every paragraph has no size).
fn leaf_intrinsic_size(node: &crate::node::Node) -> Option<Size<f32>> {
    node.image_intrinsic_size()
        .map(|(width, height)| Size { width, height })
}

/// The natural size that `contain-intrinsic-width` and `-height` give a
/// size-contained box measured as if it had no content (CSS Box Sizing 4 §5):
/// the box is sized as a leaf of that size. An axis without a length has no
/// size, as for `none`, which is the size of an empty box in that axis for
/// everything but a grid container: its explicit tracks keep their size where
/// no length is stated, so a grid that states one length only is not a leaf
/// (see [`grid_with_one_intrinsic_length`]). `None` when the box is not sized
/// as a leaf.
fn contain_intrinsic_natural(node: &crate::node::Node) -> Option<Size<f32>> {
    let lengths = node.contain_intrinsic;
    let sized_as_leaf = match (lengths.width, lengths.height) {
        (None, None) => false,
        (Some(_), Some(_)) => true,
        _ => !is_grid_container(node),
    };
    sized_as_leaf.then(|| Size {
        width: lengths.width.unwrap_or(0.0),
        height: lengths.height.unwrap_or(0.0),
    })
}

fn is_grid_container(node: &crate::node::Node) -> bool {
    node.formatting_context == FormattingContext::Grid
}

/// The natural size of a size-contained grid container that states a
/// `contain-intrinsic-*` length in one axis only. The axis without a length
/// keeps the size its explicit tracks give an empty grid, so the grid is laid
/// out as without the length and only the axis that states one is replaced.
fn grid_with_one_intrinsic_length(node: &crate::node::Node) -> Option<Size<f32>> {
    let lengths = node.contain_intrinsic;
    (is_grid_container(node) && lengths.width.is_some() != lengths.height.is_some()).then(|| Size {
        width: lengths.width.unwrap_or(0.0),
        height: lengths.height.unwrap_or(0.0),
    })
}

/// Apply sizing entries to a local clone without publishing session state.
/// Callers keep calc owners alive for every read of the clone.
fn apply_local_sizing(style: &mut Style, entries: &[OwnedPropertyOverride]) {
    for entry in entries {
        entry
            .apply_physical_sizing(style)
            .expect("local physical sizing override");
    }
}

/// Give the leaf style `style` of box `idx` the border-box `height`, as a
/// `height` of its own that follows its `box-sizing`.
fn apply_border_box_height(
    tree: &LayoutSession<'_>,
    idx: usize,
    style: &mut Style,
    height: f32,
    inline_basis: f32,
) {
    let adjustment =
        crate::layout::ifc::boxes::box_sizing_height_adjustment(tree, idx, inline_basis);
    apply_local_sizing(
        style,
        &[OwnedPropertyOverride::length(
            LengthProperty::Height,
            raikiri_style::ComputedLengthPercentageOrAuto::Px((height - adjustment).max(0.0)),
            true,
        )
        .expect("resolved pixel height")],
    );
}

/// A replaced box that a paragraph measures for its min-content contribution
/// is given a zero basis for the percentages of its sizes, as the width they
/// refer to is what the contributions decide. A `width` or `max-width` that is
/// a `calc()` with a percentage would then still add its length to the
/// contribution, but a percentage anywhere in such a size makes the box
/// compressible, and a compressible box contributes nothing, whatever length
/// the percentage is added to (CSS Sizing 3 §5.2.2). The sizes are measured as
/// zero, along each axis that is measured for its min-content size.
fn zero_cyclic_calc_sizes(style: &mut Style, inputs: &LayoutInput, replaced: bool) {
    if !replaced {
        return;
    }
    let measured = |available: AvailableSpace, basis: Option<f32>| {
        available == AvailableSpace::MinContent && basis.is_none_or(|basis| basis == 0.0)
    };
    let width = measured(inputs.available_space.width, inputs.parent_size.width);
    let height = measured(inputs.available_space.height, inputs.parent_size.height);
    let size_has_percentage = |size: Dimension| matches!(size.expand(), ExpandedDimension::Calc(calc) if calc.has_percentage());
    let max_has_percentage = |size: LengthPercentageAuto| matches!(size.expand(), ExpandedLengthPercentageAuto::Calc(calc) if calc.has_percentage());
    let zeroed: Vec<_> = [
        (
            LengthProperty::Width,
            width && size_has_percentage(style.size.width),
        ),
        (
            LengthProperty::Height,
            height && size_has_percentage(style.size.height),
        ),
        (
            LengthProperty::MaxWidth,
            width && max_has_percentage(style.max_size.width),
        ),
        (
            LengthProperty::MaxHeight,
            height && max_has_percentage(style.max_size.height),
        ),
    ]
    .into_iter()
    .filter(|&(_, percentage)| percentage)
    .map(|(property, _)| {
        OwnedPropertyOverride::length(
            property,
            raikiri_style::ComputedLengthPercentageOrAuto::Px(0.0),
            false,
        )
        .expect("local zero size")
    })
    .collect();
    apply_local_sizing(style, &zeroed);
}

/// The preferred ratio a replaced leaf can use for size transfer.
pub(crate) fn leaf_intrinsic_ratio(node: &crate::node::Node) -> Option<f32> {
    let intrinsic = node.image_intrinsic_box()?;
    intrinsic
        .aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .or_else(|| {
            if node.is_inline_svg_root()
                || !intrinsic.width.is_finite()
                || !intrinsic.height.is_finite()
                || intrinsic.width <= 0.0
                || intrinsic.height <= 0.0
            {
                None
            } else {
                let ratio = intrinsic.width / intrinsic.height;
                (ratio.is_finite() && ratio > 0.0).then_some(ratio)
            }
        })
}

/// Keep `value` in the document arena and return a reference to it.
pub(crate) fn push_calc(
    arena: &mut Vec<std::sync::Arc<raikiri_layout_style::AlignedCalc>>,
    value: impl Into<raikiri_layout_style::CalcExpression>,
    non_negative: bool,
) -> CalcRef {
    let payload = std::sync::Arc::new(raikiri_layout_style::AlignedCalc::new(value, non_negative));
    let calc = CalcRef::retained(&payload);
    arena.push(payload);
    calc
}

/// Resolve a `calc()` payload handle against `basis`.
///
/// The handle does not depend on the document, so leaf measure callbacks that
/// cannot borrow the tree call this directly.
pub(crate) fn resolve_calc(val: *const (), basis: f32) -> f32 {
    // The document retains every payload before it publishes a handle to it, so
    // a handle that names no live payload means an owner was dropped too early.
    debug_assert!(projected_calc_is_live(val), "stale calc handle");
    resolve_projected_calc(val, basis)
}

/// The limit of a `fit-content(<length-percentage>)` size, resolved against
/// the percentage `basis` of the containing block.
///
/// `None` when `size` is not that function, or when its limit needs a basis
/// that is indefinite: a percentage that cannot resolve leaves the box with no
/// limit, which CSS Sizing 3 §5.2.1 treats as the initial value, so the room
/// its parent offers sizes it.
fn fit_content_limit(size: ExpandedDimension, basis: Option<f32>) -> Option<f32> {
    let limit = match size {
        ExpandedDimension::FitContentPx(limit) => limit,
        ExpandedDimension::FitContentPercent(percent) => basis? * percent,
        ExpandedDimension::FitContentCalc(handle) if handle.has_percentage() => {
            handle.resolve(basis?)
        }
        ExpandedDimension::FitContentCalc(handle) => handle.resolve(0.0),
        _ => return None,
    };
    limit.is_finite().then_some(limit.max(0.0))
}

/// The border-box size that `fit-content(<length-percentage>)` offers the
/// box `idx` to shrink to fit in, along the width when `horizontal` and the
/// height otherwise; `None` when `size` is not that function or its limit
/// cannot resolve (see [`fit_content_limit`]).
///
/// The limit is a size of the box-sizing box, so a `content-box` needs its
/// padding and border added. Their percentages refer to the same `basis`, an
/// indefinite one counting as zero as it does for the box's own edges.
pub(crate) fn fit_content_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    size: ExpandedDimension,
    basis: Option<f32>,
    horizontal: bool,
) -> Option<f32> {
    let limit = fit_content_limit(size, basis)?;
    let edge_basis = basis.unwrap_or(0.0);
    let adjustment = if horizontal {
        crate::layout::ifc::boxes::box_sizing_adjustment(tree, idx, edge_basis)
    } else {
        crate::layout::ifc::boxes::box_sizing_height_adjustment(tree, idx, edge_basis)
    };
    Some(limit + adjustment)
}

/// Whether the `height`, `min-height` or `max-height` of `node` is a
/// percentage, or a `calc()` with one, of the height of its containing block.
/// A box with an aspect ratio carries each of them over to its width.
pub(crate) fn height_follows_containing_block(tree: &LayoutSession<'_>, node: usize) -> bool {
    let style = tree.physical_style(node);
    [
        style.size().height,
        style.min_size().height.into(),
        style.max_size().height.into(),
    ]
    .into_iter()
    .any(|height| match height.expand() {
        ExpandedDimension::Percent(_) => true,
        ExpandedDimension::Calc(calc) => calc.has_percentage(),
        _ => false,
    })
}

/// Runs `measure` for `node` against a containing block of `parent_height`.
///
/// Taffy's cache does not key on the parent size, so a size measured against
/// one containing height would answer a measurement against another. A box
/// whose height follows the containing height is measured on an empty cache,
/// which then gives way to the entries it had before.
pub(crate) fn measure_against_parent_height<T>(
    tree: &mut LayoutSession<'_>,
    node: usize,
    parent_height: Option<f32>,
    measure: impl FnOnce(&mut LayoutSession<'_>) -> T,
) -> T {
    let afresh = parent_height.is_some() && height_follows_containing_block(tree, node);
    let saved = afresh.then(|| tree.take_layout_cache(node));
    let measured = measure(tree);
    if let Some(saved) = saved {
        tree.restore_layout_cache(node, saved);
    }
    measured
}

/// Resolve a padding or border length of an ifc root against the containing
/// block's inline size.
fn ifc_edge(value: LengthPercentage, parent_inline_size: Option<f32>) -> f32 {
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
            tree.nodes[node].layout_position() != Position::Absolute
                && tree.taffy_parent_of(node) == Some(container)
        })
        .map(|&(_, basis)| basis)
}

/// The height that a replaced item of a column flex container fills when it
/// states `height: stretch`, for the probe that asks for its content size along
/// the main axis: the height of the container less the margins of the item,
/// resolved against the container's width as Taffy resolves them.
///
/// The automatic minimum size of a flex item is the smaller of its content size
/// and its specified size (CSS Flexbox 1 §4.5), and a `stretch` height is a
/// specified size once the container's height is definite. Taffy only sees the
/// specified size of a length, so it takes the content size alone, which is the
/// height the item's width gives it along its ratio: the item would grow past
/// the stretch height. The cap is only a ceiling for that probe: a `min-height`
/// or `max-height` of the item still clamps the size it ends up with.
fn column_flex_stretch_cap(
    tree: &LayoutSession<'_>,
    idx: usize,
    style: &Style,
    inputs: &LayoutInput,
) -> Option<f32> {
    let probes_content_height = inputs.run_mode == RunMode::ComputeSize
        && inputs.axis == RequestedAxis::Vertical
        && inputs.sizing_mode == SizingMode::ContentSize
        && inputs.known_dimensions.height.is_none()
        && matches!(inputs.available_space.height, AvailableSpace::MinContent)
        && matches!(style.size.height.expand(), ExpandedDimension::Stretch)
        && tree.in_column_flex_container(idx);
    if !probes_content_height {
        return None;
    }
    let room = flex_item_percent_basis(tree, idx)?.height?;
    let margin = style
        .margin
        .map(|edge| edge.resolve(inputs.parent_size.width).unwrap_or(0.0));
    // The border box is never smaller than its padding and border.
    let edges = replaced_size::padding_and_border(style, inputs.parent_size.width).height;
    Some((room - margin.top - margin.bottom).max(edges))
}

/// Whether the size that replaced leaf `idx` reports follows the height of its
/// column flex container (see [`column_flex_stretch_cap`]), which the cache of
/// layouts does not key on.
fn reads_column_flex_stretch_room(tree: &LayoutSession<'_>, idx: usize) -> bool {
    let node = &tree.nodes[idx];
    let replaced_leaf = node.is_inline_svg_root()
        || (node.children.is_empty() && leaf_intrinsic_size(node).is_some());
    replaced_leaf
        && matches!(
            tree.physical_style(idx).size().height.expand(),
            ExpandedDimension::Stretch
        )
        && tree.in_column_flex_container(idx)
}

/// Whether a margin, padding or border of `style` depends on its percentage
/// basis.
fn has_percent_edge(style: &LayoutStyleView<'_>) -> bool {
    let margin_depends_on_basis = |edge: LengthPercentageAuto| {
        edge.is_calc() || edge.resolve(Some(0.0)) != edge.resolve(Some(1.0))
    };
    let margin = style.margin();
    has_percent_padding_or_border(style)
        || [margin.left, margin.right, margin.top, margin.bottom]
            .into_iter()
            .any(margin_depends_on_basis)
}

/// Whether a padding or border of `style` depends on its percentage basis.
fn has_percent_padding_or_border(style: &LayoutStyleView<'_>) -> bool {
    [style.padding(), style.border()].into_iter().any(|edges| {
        [edges.left, edges.right, edges.top, edges.bottom]
            .into_iter()
            .any(edge_depends_on_basis)
    })
}

/// Whether a padding or border length depends on its percentage basis.
fn edge_depends_on_basis(edge: LengthPercentage) -> bool {
    // A `calc()` edge always carries a percentage term (one without folds to
    // a length) and may clamp to zero at small probe bases.
    edge.is_calc() || edge.resolve_or_zero(Some(0.0)) != edge.resolve_or_zero(Some(1.0))
}

/// Give Taffy pixel edge values resolved against the containing block's
/// inline size while keeping its physical width and height properties intact.
fn ifc_style_with_inline_edge_basis(style: &Style, inline_size: Option<f32>) -> Style {
    let basis = inline_size.unwrap_or(0.0);
    let mut style = style.clone();
    style.margin = style.margin.map(|margin| {
        margin
            .resolve(Some(basis))
            .map_or_else(LengthPercentageAuto::auto, LengthPercentageAuto::length)
    });
    style.padding = style
        .padding
        .map(|edge| LengthPercentage::length(ifc_edge(edge, Some(basis))));
    style.border = style
        .border
        .map(|edge| LengthPercentage::length(ifc_edge(edge, Some(basis))));
    style
}

/// Padding-top plus border-top of an ifc root, resolved from its style.
///
/// The parent has not stored this node's layout yet when the root is
/// measured, so the insets come from the style and the parent's inline size.
fn ifc_top_inset(style: &Style, parent_inline_size: Option<f32>) -> f32 {
    ifc_edge(style.padding.top, parent_inline_size) + ifc_edge(style.border.top, parent_inline_size)
}

fn ifc_block_edge_inset(
    style: &Style,
    parent_inline_size: Option<f32>,
    writing_mode: shodo::geometry::WritingMode,
    start: bool,
) -> f32 {
    use shodo::geometry::WritingMode;
    let edge = match writing_mode {
        WritingMode::HorizontalTb => {
            if start {
                (style.padding.top, style.border.top)
            } else {
                (style.padding.bottom, style.border.bottom)
            }
        }
        WritingMode::VerticalRl | WritingMode::SidewaysRl => {
            if start {
                (style.padding.right, style.border.right)
            } else {
                (style.padding.left, style.border.left)
            }
        }
        WritingMode::VerticalLr | WritingMode::SidewaysLr => {
            if start {
                (style.padding.left, style.border.left)
            } else {
                (style.padding.right, style.border.right)
            }
        }
    };
    ifc_edge(edge.0, parent_inline_size) + ifc_edge(edge.1, parent_inline_size)
}

fn ifc_block_size_is_auto(style: &Style, writing_mode: shodo::geometry::WritingMode) -> bool {
    if writing_mode == shodo::geometry::WritingMode::HorizontalTb {
        style.size.height.is_auto()
    } else {
        style.size.width.is_auto()
    }
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

fn ifc_all_insets(style: &Style, parent_inline_size: Option<f32>) -> Edges<f32> {
    Edges {
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
pub(crate) struct TaffyChildIter<'a> {
    doc: &'a Document,
    flatten: bool,
    /// Whether the children are those of a source box, among which an
    /// anonymous table stands in for the rows it wraps.
    wrap_tables: bool,
    /// Whether the children are those of a flex or grid container, whose
    /// anonymous items of text each stand for a run of text nodes.
    wraps_text_runs: bool,
    current: core::slice::Iter<'a, usize>,
    stack: Vec<core::slice::Iter<'a, usize>>,
}

impl<'a> TaffyChildIter<'a> {
    pub(crate) fn new_for_session(session: &'a LayoutSession<'_>, parent: usize) -> Self {
        let doc = &*session.document;
        // A size-contained box is measured as if it had no content.
        let hidden = session.context.size_contained_measure.last() == Some(&parent);
        Self::new(doc, parent, hidden)
    }

    pub(crate) fn new(doc: &'a Document, parent: usize, hidden: bool) -> Self {
        let node = &doc.nodes[parent];
        Self {
            doc,
            flatten: !node.is_display_none() && node.display() != DisplayValue::Contents,
            wrap_tables: !doc.is_anonymous_table(parent),
            wraps_text_runs: node.display().is_flex_or_grid_container(),
            current: if hidden {
                [].iter()
            } else {
                node.layout_children().iter()
            },
            stack: Vec::new(),
        }
    }

    /// Whether `child` is a `display: contents` element whose children take
    /// its place among a container's children.
    fn is_flattened(doc: &Document, child: usize) -> bool {
        let node = &doc.nodes[child];
        node.kind() == raikiri_traits::NodeKind::Element
            && node.display() == DisplayValue::Contents
            && !node.is_ifc_root()
            && !node.flags.contains(NodeFlags::IS_ROOT_ELEMENT)
    }

    fn includes(doc: &Document, child: usize, wraps_text_runs: bool) -> bool {
        if !doc.nodes[child].is_in_document() {
            return false;
        }
        // A text node after the first of a run is laid out with it, by the
        // paragraph of that first node: the run makes one anonymous item.
        if wraps_text_runs
            && doc.nodes[child].kind() == raikiri_traits::NodeKind::Text
            && doc.nodes[child].in_ifc_subtree()
        {
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
        let container = &self.nodes[parent];
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
        let top = self
            .inclusive_ancestors(parent)
            .find(|&id| !TaffyChildIter::is_flattened(self, id))?;
        let container = &self.nodes[top];
        let lifted = top != parent
            && !container.is_display_none()
            && container.display() != DisplayValue::Contents;
        Some(if lifted { top } else { parent })
    }
}

impl LayoutSession<'_> {
    /// Whether box `node` is an item of a flex container with a vertical main
    /// axis.
    pub(crate) fn in_column_flex_container(&self, node: usize) -> bool {
        self.taffy_parent_of(node)
            .filter(|&parent| self.nodes[parent].layout_display() == Display::Flex)
            .is_some_and(|parent| !self.physical_style(parent).flex_direction_is_row())
    }

    /// Whether box `node` is an item of a flex container with a horizontal main
    /// axis.
    pub(crate) fn in_row_flex_container(&self, node: usize) -> bool {
        self.taffy_parent_of(node)
            .filter(|&parent| self.nodes[parent].layout_display() == Display::Flex)
            .is_some_and(|parent| self.physical_style(parent).flex_direction_is_row())
    }

    /// Whether `known` is only the cross size that a flex container stretches
    /// its item `node`, with the box style `style`, to while the item's main
    /// size is measured.
    ///
    /// A container that wraps its items onto more than one line does not take
    /// the cross size it stretches an item with an `auto` cross size to as
    /// definite then, so that size gives the item no main size through a ratio.
    pub(crate) fn is_wrapping_stretch_hint(
        &self,
        node: usize,
        style: &Style,
        known: Size<Option<f32>>,
    ) -> bool {
        let Some(container) = self
            .taffy_parent_of(node)
            .filter(|&parent| self.nodes[parent].layout_display() == Display::Flex)
            .map(|parent| self.physical_style(parent))
            .filter(|container| container.flex_wraps())
        else {
            return false;
        };
        let (known_main, cross_size) = if container.flex_direction_is_row() {
            (known.width, style.size.height)
        } else {
            (known.height, style.size.width)
        };
        known_main.is_none() && cross_size.is_auto()
    }
}

impl Document {
    /// The children of `node` that take part in layout, in layout order, as
    /// arena indices.
    ///
    /// These are the flat-tree children that generate boxes: nodes outside the
    /// document (comments, processing instructions, a `<template>` contents
    /// fragment) and whitespace-only text that makes no box are left out, and
    /// the children of a `display: contents` child stand in for it.
    #[doc(hidden)]
    pub fn layout_child_ids(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        TaffyChildIter::new(self, node, false)
    }
}

impl Iterator for TaffyChildIter<'_> {
    /// The arena index of the child.
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        let doc = self.doc;
        loop {
            let Some(&child) = self.current.next() else {
                self.current = self.stack.pop()?;
                continue;
            };
            if !Self::includes(doc, child, self.wraps_text_runs) {
                continue;
            }
            if self.flatten && Self::is_flattened(doc, child) {
                self.stack.push(std::mem::replace(
                    &mut self.current,
                    doc.nodes[child].layout_children().iter(),
                ));
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
                return Some(table);
            }
            return Some(child);
        }
    }
}

/// The number of children from which taffy's walks over a container's
/// children by index read them from a list; walking fewer children from
/// the start each time costs less than building the list.
const WALKED_CHILDREN_MIN: usize = 32;

impl LayoutSession<'_> {
    /// The cached result of the node for `inputs`, if it holds one that is
    /// valid in the current cache context.
    pub(crate) fn cache_get(
        &mut self,
        node_id: NodeId,
        inputs: &LayoutInput,
    ) -> Option<LayoutOutput> {
        self.sync_layout_cache_mutations();
        let idx = usize::from(node_id);
        let key = self.nodes.box_key(idx)?;
        self.layout_cache_context
            .can_read(key)
            .then(|| self.nodes[idx].cache.get(inputs))
            .flatten()
    }

    /// Store the result of the node for `inputs` in its cache.
    pub(crate) fn cache_store(
        &mut self,
        node_id: NodeId,
        inputs: &LayoutInput,
        layout_output: LayoutOutput,
    ) {
        self.sync_layout_cache_mutations();
        let idx = usize::from(node_id);
        let key = self.nodes.box_key(idx).expect("cached node has a box key");
        // Reject a late outer store before touching a valid inner computation's cache.
        if !self.layout_cache_context.can_store(key) {
            return;
        }
        if !self.layout_cache_context.can_read(key) {
            self.nodes[idx].cache.clear();
        }
        self.nodes[idx].cache.store(inputs, layout_output);
        self.layout_cache_context.mark_stored(key);
    }

    /// Whether a result stored now lands in the cache of the node: its own
    /// cache frame is the innermost one and no override has changed since it
    /// opened.
    pub(crate) fn caches_node(&self, node_id: NodeId) -> bool {
        self.nodes
            .box_key(usize::from(node_id))
            .is_some_and(|key| self.layout_cache_context.can_store(key))
    }

    /// The intrinsic width kept for the node under `inputs`, if the cache
    /// still holds one that is valid in the current cache context.
    pub(crate) fn cached_width(&mut self, node_id: NodeId, inputs: &LayoutInput) -> Option<f32> {
        self.sync_layout_cache_mutations();
        let idx = usize::from(node_id);
        let key = self.nodes.box_key(idx)?;
        self.layout_cache_context
            .can_read(key)
            .then(|| self.nodes[idx].cache.width(inputs))
            .flatten()
    }

    /// Keep `width` as the intrinsic width of the node under `inputs`, with
    /// the same rules as [`LayoutSession::cache_store`].
    pub(crate) fn cache_width(&mut self, node_id: NodeId, inputs: &LayoutInput, width: f32) {
        self.sync_layout_cache_mutations();
        let idx = usize::from(node_id);
        let key = self.nodes.box_key(idx).expect("cached node has a box key");
        if !self.layout_cache_context.can_store(key) {
            return;
        }
        if !self.layout_cache_context.can_read(key) {
            self.nodes[idx].cache.clear();
        }
        self.nodes[idx].cache.store_width(inputs, width);
        self.layout_cache_context.mark_stored(key);
    }

    /// Drop everything the cache of the node holds.
    pub(crate) fn cache_clear(&mut self, node_id: NodeId) {
        self.nodes[usize::from(node_id)].cache.clear();
    }

    /// Drop the cached results of the node and of every box inside it. Such a
    /// result stands for the lines and fragments the boxes held when it was
    /// kept, and a layout of the subtree that did not store its results has
    /// replaced them since.
    pub(crate) fn cache_clear_subtree(&mut self, node_id: NodeId) {
        let mut pending = vec![usize::from(node_id)];
        while let Some(idx) = pending.pop() {
            self.nodes[idx].cache.clear();
            self.context.variant_caches.forget(idx);
            pending.extend(self.nodes[idx].children.iter().copied());
        }
    }
}

impl LayoutSession<'_> {
    /// The **only** path by which the layout algorithms write layout into the
    /// arena.
    ///
    /// Storing through [`LayoutSession::store_layout`] here structurally
    /// guarantees that `Node.layout` contains no non-finite f32 values.
    /// The input guard in the bridge (`layout::sanitize_taffy`) alone cannot
    /// close the gap: nested percentages compound at the used-value layer
    /// and can become non-finite again. See the documentation for
    /// `layout::sanitize_taffy_layout` for the reasoning and measurements.
    ///
    /// Record fields actually clamped in `self.context.layout_warnings`.
    /// The signature the algorithms call this through is fixed, so diagnostic
    /// arguments cannot be added. Instead, write to the owned buffer through
    /// `self` without changing the signature. At pass end, `layout_single_page`
    /// drains the buffer to an observer or eprintln (see the documentation for
    /// the diagnostics buffer below).
    /// The session context drops this buffer when the layout operation ends.
    pub(crate) fn set_unrounded_layout(&mut self, node_id: NodeId, layout: &BoxLayout) {
        self.note_transposed_layout(usize::from(node_id));
        self.store_layout(usize::from(node_id), layout);
    }

    /// Lay out the node under `inputs`, from its cache when it holds a result
    /// for them.
    pub(crate) fn compute_child_layout(
        &mut self,
        node_id: NodeId,
        inputs: LayoutInput,
    ) -> LayoutOutput {
        self.assert_style_queries_allowed();
        let idx = usize::from(node_id);
        if inputs.run_mode == RunMode::PerformLayout
            && let Some(capture) = self.context.oof_content_limit_inputs.last_mut()
        {
            capture.insert(idx, inputs);
        }
        self.compute_child_layout_with_block_ctx(node_id, inputs, None)
    }
}

impl LayoutSession<'_> {
    /// Unified child dispatch for the native layout traits and project flow.
    ///
    /// Public Taffy flex/grid algorithms enter through
    /// `LayoutPartialTree::compute_child_layout` without a shared context.
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
        mut inputs: LayoutInput,
        mut block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.assert_style_queries_allowed();
        let idx = usize::from(node_id);
        if !self.context.recursion.enter(idx) {
            return LayoutOutput::HIDDEN;
        }
        if let Some(parent) = self.nodes[idx].parent
            && let Some((_, height)) = self
                .context
                .content_limit_percentage_height_bases
                .iter()
                .rev()
                .find(|(ancestor, _)| *ancestor == parent)
        {
            inputs.parent_size.height = Some(*height);
        }
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
        let mut output = if let Some(output) =
            self.compute_transposed_child(node_id, inputs, block_ctx.as_deref_mut())
        {
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
        self.context.recursion.leave();
        // Whichever path laid the box out, layout containment leaves it with
        // no baselines: whatever aligns it, or collects the baseline of its
        // content, finds none in it and synthesizes one from its border box
        // instead, so the first line inside it does not decide where it sits
        // (CSS Contain 2 §3.2).
        // spec: https://www.w3.org/TR/css-contain-2/#containment-layout
        if exports_no_baseline(self, idx) {
            output.baselines = Baselines::NONE;
        }
        // A grid container reads this when its algorithm returns, and the
        // last layout the algorithm performs of an item is the one that
        // places it, so it is the baseline the container saw of the item.
        if inputs.run_mode == RunMode::PerformLayout {
            self.nodes[idx].exported_first_baseline = output.baselines.first;
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
        self.sync_layout_cache_mutations();
        let inputs = vertical_oof::absolute_child_inputs(self, usize::from(node_id), inputs);
        let inputs = with_content_height_not_definite(self, usize::from(node_id), inputs);
        let inputs = with_grid_ratio_width(self, usize::from(node_id), inputs);
        if inputs.run_mode == RunMode::PerformLayout {
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
            || (paragraph && ifc_basis.is_some() && has_percent_edge(&self.physical_style(idx)))
            || self.context.size_contained_measure.last() == Some(&idx)
            // Stretch-dependent sizing reads the margins the parent holds
            // in, which are not part of the cache's layout inputs.
            || content_limits::needs_stretch_height_room(self, idx)
            // The height of a column flex container is not either, and the
            // same container is laid out at its content height first and at
            // the height it is stretched to after.
            || reads_column_flex_stretch_room(self, idx);
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
            .filter(|_| has_percent_padding_or_border(&self.physical_style(idx)));
        let dispatch = |tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput| {
            let idx = usize::from(node_id);
            let inputs = with_fit_content_room(tree, idx, inputs);
            let inputs = with_column_flex_keyword_room(tree, idx, inputs);
            let inputs = with_stretch_cross_size(tree, idx, inputs);
            let empty = tree.context.size_contained_measure.last() == Some(&idx);
            // Size containment may give the box a width the request did not
            // have; whether the request stretched it is read before that.
            let stretched_width = inputs.known_dimensions.width.is_some();
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
                    let mut output =
                        crate::layout::table::compute_table_layout(tree, node_id, inputs);
                    if inputs.run_mode == RunMode::PerformLayout {
                        oof::layout_local_oof(tree, node_id, &mut output);
                        center_absolute_children_in_block_axis(
                            tree,
                            idx,
                            &inputs,
                            output.size,
                            None,
                        );
                    }
                    return output;
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
                        && tree.nodes[idx].display() != DisplayValue::InlineTable)
                    || is_floated_flex_or_grid(tree, idx)
                    || is_button_in_multicol(tree, idx);
                // Size containment already gave the box its width.
                let contained_width = tree.nodes[idx].size_containment.width && !empty;
                if inline_shrink_wrap
                    && !tree.nodes[idx].flags.contains(NodeFlags::IS_ROOT_ELEMENT)
                    && !contained_width
                    && tree.physical_style(idx).size().width.is_auto()
                    && (inputs.known_dimensions.width.is_none()
                        || inputs.sizing_mode != SizingMode::ContentSize)
                {
                    return compute_inline_block_shrink_wrap(tree, node_id, inputs, block_ctx);
                }
            }
            // A vertical box under a horizontal block container is sized by
            // its content on the axis its container stretched it along. Its
            // own measure of that content asks for no width, and a size
            // contained box answers that with the width containment gives it;
            // that answer is not a stretched width to measure again.
            if !empty
                && stretched_width
                && inputs.sizing_mode == SizingMode::InherentSize
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
            // Empty flex/grid containers still run their layout algorithms;
            // explicit grid tracks contribute even when there are no items.
            let item_is_replaced = tree.physical_style(idx).is_replaced();
            // An empty box that is not replaced content, yet has a preferred
            // aspect ratio, is a block container like any other: its sizes
            // follow block flow, including the automatic minimum size and the
            // way a minimum or maximum carries across the ratio.
            let ratio_block = !empty
                && formatting_context == FormattingContext::Block
                && !item_is_replaced
                && leaf_intrinsic_size(&tree.nodes[idx]).is_none()
                && !tree.nodes[idx].is_inline_svg_root()
                && tree.physical_style(idx).aspect_ratio().is_some();
            // The size of its contents is what a size-contained box gives up,
            // and `contain-intrinsic-*` names the size it takes instead.
            let contain_natural = contain_intrinsic_natural(&tree.nodes[idx]).filter(|_| empty);
            let is_leaf = contain_natural.is_some()
                || (tree.nodes[idx].children.is_empty()
                    && !ratio_block
                    && (!matches!(
                        formatting_context,
                        FormattingContext::Flex | FormattingContext::Grid
                    ) || item_is_replaced))
                || tree.nodes[idx].is_inline_svg_root();
            if empty && is_leaf {
                // Replaced content with size containment has no natural size
                // or aspect ratio; the style bridge left only the ratio the
                // box states itself.
                let style = tree.physical_style(idx);
                let natural = contain_natural.unwrap_or(Size {
                    width: 0.0,
                    height: 0.0,
                });
                return compute_leaf_layout(
                    inputs,
                    &TaffyStyle::new(style),
                    resolve_calc,
                    |known, _| Size {
                        width: known.width.unwrap_or(natural.width),
                        height: known.height.unwrap_or(natural.height),
                    },
                );
            }
            if tree.nodes[idx].is_inline_svg_root()
                && tree.nodes[idx].attribute("width").is_none()
                && tree.nodes[idx].attribute("height").is_none()
                && let Some(intrinsic) = tree.nodes[idx].image_intrinsic_box()
                && let Some(ratio) = intrinsic.aspect_ratio
            {
                // CSS Sizing 3, intrinsic sizes: ratio-only replaced content
                // uses the definite available inline size, not a 300px default.
                // An authored `aspect-ratio` replaces the ratio of the viewBox;
                // `auto` keeps it.
                let ratio = tree.nodes[idx].preferred_aspect_ratio.unwrap_or(ratio);
                let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
                let mut resolver_style = block_flow::style_input::physical(tree, idx);
                zero_cyclic_calc_sizes(&mut resolver_style, &inputs, true);
                apply_local_sizing(
                    &mut resolver_style,
                    &[OwnedPropertyOverride::aspect_ratio(Some(ratio))],
                );
                if tree.nodes[idx].svg_width_follows_available
                    && matches!(inputs.available_space.width, AvailableSpace::Definite(_))
                {
                    // The style holds `width: 100%` so that the box fills the
                    // room its parent offers. Measured as an authored width it
                    // would keep that width whatever the `min-*` and `max-*`
                    // sizes of the height ask for; as `auto` the constraint
                    // table of CSS 2.1 §10.4 moves it along the ratio. An
                    // intrinsic size measurement keeps the percentage.
                    apply_local_sizing(
                        &mut resolver_style,
                        &[OwnedPropertyOverride::length(
                            LengthProperty::Width,
                            raikiri_style::ComputedLengthPercentageOrAuto::Auto,
                            false,
                        )
                        .expect("local auto width")],
                    );
                }
                let mut style = resolver_style.clone();
                // Transfer the ratio in measurement only. Taffy's leaf ratio
                // floor would otherwise override explicit/max heights.
                apply_local_sizing(&mut style, &[OwnedPropertyOverride::aspect_ratio(None)]);
                let limits = tree.nodes[idx].content_limits;
                // An authored height gives an auto width through the ratio
                // (CSS 2.1 10.3.2), also when no parent passes the height in
                // as a known dimension; taffy's leaf then hands it to the
                // measure function as the definite available height.
                let height_is_authored = style.size.width.is_auto()
                    && MaybeResolve::maybe_resolve(style.size.height, inputs.parent_size.height)
                        .is_some();
                let insets = replaced_size::ratio_box_insets(
                    &resolver_style,
                    replaced_size::padding_and_border(&resolver_style, inputs.parent_size.width),
                    content_box_ratio,
                );
                let stretch_cap = column_flex_stretch_cap(tree, idx, &resolver_style, &inputs);
                let mut output = compute_leaf_layout(
                    inputs,
                    &LeafStyle::new(&style),
                    resolve_calc,
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
                        let width_for =
                            |height| replaced_size::width_for_height(height, ratio, insets);
                        let width = known
                            .width
                            .or(known.height.map(width_for))
                            .or(authored_height.map(width_for))
                            .unwrap_or(match available.width {
                                AvailableSpace::Definite(width) => width,
                                AvailableSpace::MinContent => 0.0,
                                AvailableSpace::MaxContent => intrinsic.width,
                            });
                        let natural = Size {
                            width,
                            height: known.height.unwrap_or_else(|| {
                                replaced_size::height_for_width(width, ratio, insets)
                            }),
                        };
                        let resolver_inputs = LayoutInput {
                            known_dimensions: known,
                            available_space: available,
                            ..inputs
                        };
                        let resolved = replaced_size::auto_sides_size(
                            &resolver_style,
                            &resolver_inputs,
                            natural,
                            limits,
                            content_box_ratio,
                        )
                        .unwrap_or(natural);
                        Size {
                            width: resolved.width,
                            height: resolved.height,
                        }
                    },
                );
                if let Some(cap) = stretch_cap {
                    output.size.height = output.size.height.min(cap);
                }
                return output;
            }
            // A paragraph measured as if empty is a block container with no
            // lines.
            if formatting_context == FormattingContext::Inline && !empty {
                let inputs = orthogonal_flow::with_page_room(tree, idx, inputs);
                return compute_ifc_root(tree, node_id, inputs, block_ctx);
            }
            if is_leaf {
                let mut style = block_flow::style_input::physical(tree, idx);
                zero_cyclic_calc_sizes(&mut style, &inputs, item_is_replaced);
                let leaf_intrinsic = leaf_intrinsic_size(&tree.nodes[idx]);
                let limits = tree.nodes[idx].content_limits;
                // Natural ratios resolve intrinsic limits; without them,
                // preserve Taffy's ordinary replaced sizing path.
                if limits != content_limits::ContentLimits::NONE
                    && style.aspect_ratio.is_none()
                    && !tree.nodes[idx].size_containment.width
                    && !tree.nodes[idx].size_containment.height
                {
                    apply_local_sizing(
                        &mut style,
                        &[OwnedPropertyOverride::aspect_ratio(
                            tree.nodes[idx].keyword_aspect_ratio,
                        )],
                    );
                }
                // A replaced leaf with an `auto` side takes its size from the
                // ratio and the min/max sizes; Taffy's ratio floor on the
                // leaf is switched off for it.
                let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
                let constrained = leaf_intrinsic.and_then(|natural| {
                    replaced_size::auto_sides_size(
                        &style,
                        &inputs,
                        natural,
                        limits,
                        content_box_ratio,
                    )
                    .or_else(|| {
                        let stretch_hint =
                            tree.is_wrapping_stretch_hint(idx, &style, inputs.known_dimensions);
                        if stretch_hint {
                            None
                        } else {
                            replaced_size::fixed_side_size(&style, &inputs, content_box_ratio)
                        }
                    })
                });
                if constrained.is_some() {
                    apply_local_sizing(&mut style, &[OwnedPropertyOverride::aspect_ratio(None)]);
                }
                let stretch_cap = leaf_intrinsic
                    .and_then(|_| column_flex_stretch_cap(tree, idx, &style, &inputs));
                let mut output = compute_leaf_layout(
                    inputs,
                    &LeafStyle::new(&style),
                    resolve_calc,
                    |known, _available| {
                        // An explicit size from the style wins; otherwise an
                        // image's intrinsic size; a text node outside every
                        // paragraph and any other leaf measure zero.
                        let natural = constrained.or(leaf_intrinsic);
                        Size {
                            width: known.width.or(natural.map(|s| s.width)).unwrap_or(0.0),
                            height: known.height.or(natural.map(|s| s.height)).unwrap_or(0.0),
                        }
                    },
                );
                if let Some(cap) = stretch_cap {
                    output.size.height = output.size.height.min(cap);
                }
                output
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
        // A `min-*` or `max-*` size that is an intrinsic sizing keyword is a
        // length by the time the box is dispatched.
        let compute = |tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput| {
            content_limits::with_content_limits(tree, node_id, inputs, dispatch)
        };
        let view_context = raikiri_layout_style::ViewContext {
            layout_wm: self.nodes[idx].used_writing_mode,
            percent_basis: edge_basis.map_or(
                raikiri_layout_style::PercentBasis::Taffy,
                raikiri_layout_style::PercentBasis::Definite,
            ),
            ..raikiri_layout_style::ViewContext::default()
        };
        let mut output = if let Some(basis) = edge_basis {
            self.with_resolved_edges(idx, basis, true, |tree| {
                tree.with_cache_frame(idx, view_context, |tree| {
                    compute_cached_layout(tree, node_id, inputs, compute)
                })
            })
        } else if uncached {
            compute(self, node_id, inputs)
        } else {
            self.with_cache_frame(idx, view_context, |tree| {
                compute_cached_layout(tree, node_id, inputs, compute)
            })
        };
        // The baselines of a box laid out on vertical axes are offsets along
        // its block axis from its block-start border edge, which is not what
        // the lookups below read.
        let vertical = on_vertical_axes(self, idx);
        // An ifc root measured its baseline from the lines of this very call;
        // the stored lines may be those of another width.
        if !vertical
            && !self.nodes[usize::from(node_id)]
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
        let scrolls = is_scroll_container(self.physical_style(idx).overflow());
        if scrolls && inputs.run_mode == RunMode::PerformLayout {
            let block_size = if vertical {
                output.size.width
            } else {
                output.size.height
            };
            clamp_baselines(&mut output.baselines, block_size);
        }
        // A flex container keeps the first baseline of its performed layout,
        // which a list item's outside marker aligns with. A grid container's
        // baseline is found from the first-row items its detailed grid
        // information records instead, because Taffy's grid baseline does
        // not measure an item's own first line. A marker sits on the line of
        // a horizontal container only.
        if inputs.run_mode == RunMode::PerformLayout
            && !vertical
            && self.nodes[usize::from(node_id)].formatting_context == FormattingContext::Flex
        {
            self.nodes[usize::from(node_id)].flex_first_baseline = output.baselines.first;
        }
        // The baselines of a box in a writing mode orthogonal to its parent's
        // run across the parent's line, so they say nothing about where the
        // box sits along it: it exposes none and is synthesized from its
        // border box instead. A container that aligns the box by a baseline
        // along its own inline axis finds the one of its first line in the
        // node, where the line runs along that axis.
        if self.is_orthogonal_to_parent(usize::from(node_id)) {
            if inputs.run_mode == RunMode::PerformLayout {
                self.nodes[usize::from(node_id)].own_first_baseline = output.baselines.first;
            }
            output.baselines = Baselines::NONE;
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
mod float_container_tests;
#[cfg(test)]
mod grid_baseline_tests;
#[cfg(test)]
mod item_width_tests;
#[cfg(test)]
mod self_alignment_tests;
#[cfg(test)]
mod shrink_to_fit_keyword_tests;
#[cfg(test)]
mod tests;

mod axis_adapter;
pub(crate) mod block_flow;
pub(crate) mod content_limits;
pub(crate) mod grid_baseline;
mod grid_oof;
mod inline_baseline;
mod intrinsic_scopes;
use intrinsic_scopes::{
    with_flex_intrinsic_probe, with_flex_item_basis_bounds, wrapping_min_content_width,
};
mod oof;
mod orthogonal_flow;
mod replaced_size;
mod transposed_axes;
mod vertical_block;
mod vertical_oof;
pub(crate) use oof::{compute_root_layout_with_content_limits, layout_container_candidates};
pub(crate) use transposed_axes::reverse_static_position;
pub(crate) use vertical_block::{
    TransposedBlock, TransposedSizingUndo, TransposedStyleIndex, block_right_to_left, is_table_box,
    with_final_vertical_table_cell, with_orthogonal_fit_content,
};
pub(crate) use vertical_oof::used_writing_mode;

/// Lay out an ifc root: its paragraph is measured and broken into lines by
/// the inline engine, and taffy sees it as a leaf.
fn compute_ifc_root(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    mut block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    if tree.nodes[idx]
        .ifc
        .as_ref()
        .is_some_and(|root| root.writing_mode != shodo::geometry::WritingMode::HorizontalTb)
        && !crate::layout::dispatch::vertical_block::has_same_vertical_block_parent(tree, idx)
    {
        block_ctx = None;
    }
    let style = block_flow::style_input::physical(tree, idx);
    let parent_inline_size = ifc_parent_inline_size(tree, idx, inputs.parent_size);
    let mut layout_style = ifc_style_with_inline_edge_basis(&style, parent_inline_size);
    // A horizontal paragraph box with an `aspect-ratio` is sized as block
    // flow sizes a block: a size the ratio gives is raised by the content up
    // to the maximum the box states. Taffy's leaf takes it as final, so the
    // leaf is given the width the ratio settles and no ratio, and the height
    // follows once the content is measured.
    let ratio = layout_style.aspect_ratio.filter(|_| {
        inputs.sizing_mode == SizingMode::InherentSize
            && tree.nodes[idx].multicol.is_none()
            && tree.nodes[idx]
                .ifc
                .as_ref()
                .is_some_and(|root| root.writing_mode == shodo::geometry::WritingMode::HorizontalTb)
    });
    let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
    let stated = MaybeResolve::maybe_resolve(layout_style.size, inputs.parent_size);
    let ratio_height_follows =
        ratio.is_some() && inputs.known_dimensions.height.is_none() && stated.height.is_none();
    let mut inputs = inputs;
    if ratio.is_some() {
        let metrics = block_flow::Metrics::resolve(&layout_style, false, content_box_ratio, inputs);
        if inputs.known_dimensions.width.is_none()
            && stated.width.is_none()
            && let Some(width) = metrics.preferred.width
        {
            // The width the ratio takes from the height, raised to the
            // min-content width of the lines unless the box scrolls or
            // states a `min-width`.
            let width = if metrics.ratio_width {
                let minimum = ifc_min_content_width(tree, node_id, inputs.parent_size);
                width.max(minimum.maybe_clamp(None, metrics.own_max.width))
            } else {
                width
            };
            inputs.known_dimensions.width = Some(width);
            inputs.known_dimensions_are_definite.width = true;
        }
        apply_local_sizing(
            &mut layout_style,
            &[OwnedPropertyOverride::aspect_ratio(None)],
        );
    }
    // Border-box top inset, resolved here: the parent has not
    // stored this node's layout yet.
    let top_inset = ifc_top_inset(&style, parent_inline_size);
    // A known width on the input means the parent stretched or
    // fixed the box, and a definite `width` of its own fixes it
    // too (taffy passes it as the available width); otherwise the
    // box shrinks to fit.
    let stretched_width = inputs.known_dimensions.width.is_some()
        || (inputs.sizing_mode == SizingMode::InherentSize
            && ifc_has_definite_width(&style, inputs.parent_size.width));
    let stretched_height = inputs.known_dimensions.height.is_some()
        || (inputs.sizing_mode == SizingMode::InherentSize
            && ifc_has_definite_height(&style, inputs.parent_size.height));
    let writing_mode = tree.nodes[idx]
        .ifc
        .as_ref()
        .map_or(shodo::geometry::WritingMode::HorizontalTb, |root| {
            root.writing_mode
        });
    let block_start_inset = ifc_block_edge_inset(&style, parent_inline_size, writing_mode, true);
    let block_end_inset = ifc_block_edge_inset(&style, parent_inline_size, writing_mode, false);
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
            && tree.nodes[idx].layout_display() == Display::Block
            && tree.nodes[idx].layout_position() != Position::Absolute
            && style.float == Float::None
            && !style.overflow.x.is_scroll_container()
            && !style.overflow.y.is_scroll_container()
            && !style.contain.establishes_independent_formatting_context()
            && style.align_content.keyword == AlignContentKeyword::Normal
            && tree.nodes[idx].multicol.is_none()
            && block_start_inset == 0.0,
        // CSS 2.1 8.3.1: the bottom margin of a last in-flow child and that
        // of its parent are adjoining when the parent has no bottom padding
        // or border and an `auto` height, and the parent is in the same
        // formatting context as its own parent (a block context is passed).
        bottom_margin_escapes: block_ctx.is_some()
            && inputs.vertical_margins_are_collapsible.end
            && ifc_block_size_is_auto(&style, writing_mode)
            && block_end_inset == 0.0,
        column_height: None,
        definite_height: None,
    };
    let mut content_baseline = None;
    let mut leading_margin = CollapsibleMarginSet::ZERO;
    let mut escaping_margin = CollapsibleMarginSet::ZERO;
    let mut nested_oof_candidates = OofCandidates::new();
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
            let insets = if style.box_sizing == BoxSizing::BorderBox {
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
    let border_box_insets = top_inset
        + ifc_edge(style.padding.bottom, parent_inline_size)
        + ifc_edge(style.border.bottom, parent_inline_size);
    let border_box = style.box_sizing == BoxSizing::BorderBox;
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
            .map(|height| (height - if border_box { border_box_insets } else { 0.0 }).max(0.0))
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
    let mut output = compute_leaf_layout(
        inputs,
        &LeafStyle::new(&layout_style),
        resolve_calc,
        |known, available| {
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
            nested_oof_candidates = measured.oof_candidates;
            measured.size
        },
    );
    if ratio_height_follows {
        apply_local_sizing(
            &mut layout_style,
            &[OwnedPropertyOverride::aspect_ratio(ratio)],
        );
        let mut settled = inputs;
        settled.known_dimensions.width = Some(output.size.width);
        let metrics =
            block_flow::Metrics::resolve(&layout_style, false, content_box_ratio, settled);
        // `output` holds the height of the lines. They raise the height the
        // ratio gives unless the box scrolls or states a `min-height`.
        let height = metrics.preferred.height.unwrap_or(output.size.height);
        output.size.height = if metrics.ratio_height {
            height.max(output.size.height.maybe_clamp(None, metrics.own_max.height))
        } else {
            height
        };
    }
    if auto_vertical_block_size && let Some(measured_size) = measured_size {
        // `compute_leaf_layout` retains Taffy's intrinsic auto-width probe as
        // a known width. In vertical flow, the performed IFC layout measures
        // the cumulative block extent and must replace that probe result.
        output.size.width = measured_size.width + measure.insets.left + measure.insets.right;
    }
    output.top_margin = leading_margin;
    // The baseline of a horizontal root lies below the top edge of its border
    // box; that of a vertical root, along the block axis from the edge its
    // block starts at.
    let vertical_axes = on_vertical_axes(tree, idx);
    let baseline_inset = if vertical_axes {
        block_start_inset
    } else {
        top_inset
    };
    output.baselines.first = content_baseline.map(|baseline| baseline + baseline_inset);
    // A vertical root with block children takes the baseline of whichever of
    // its first line and its first block child with one comes first in the
    // content, whatever margins put where. An inline-block exposes its last
    // line instead, and falls back on the first block child only when it has
    // no line.
    if vertical_axes && inputs.run_mode == RunMode::PerformLayout {
        let block = first_block_baseline(tree, idx);
        let block_first = |child| {
            tree.nodes[idx]
                .ifc
                .as_ref()
                .and_then(|root| root.lines.as_ref())
                .is_none_or(|lines| {
                    lines
                        .block_line_starts
                        .iter()
                        .any(|&(node, first)| node == child && first == 0)
                })
        };
        output.baselines.first = match (output.baselines.first, block) {
            (Some(line), Some((child, baseline)))
                if tree.nodes[idx].display()
                    != raikiri_style::property::DisplayValue::InlineBlock =>
            {
                Some(if block_first(child) {
                    baseline + baseline_inset
                } else {
                    line
                })
            }
            (line, block) => line.or(block.map(|(_, baseline)| baseline + baseline_inset)),
        };
    }
    output.oof_candidates.append(&mut nested_oof_candidates);
    if inputs.run_mode == RunMode::PerformLayout {
        crate::layout::ifc::boxes::place_out_of_flow(tree, idx, output.size, &mut output);
    }
    // When `min-height` gives the used height, the last child's bottom margin
    // no longer adjoins the box's bottom edge and does not collapse through
    // it (CSS 2.1 8.3.1), as taffy's block algorithm decides.
    let min_height = MaybeResolve::maybe_resolve(style.min_size.height, inputs.parent_size.height);
    let min_block_size = if writing_mode == shodo::geometry::WritingMode::HorizontalTb {
        min_height
    } else {
        MaybeResolve::maybe_resolve(style.min_size.width, inputs.parent_size.width)
    };
    let block_size = if writing_mode == shodo::geometry::WritingMode::HorizontalTb {
        output.size.height
    } else {
        output.size.width
    };
    let block_axis_insets = if writing_mode == shodo::geometry::WritingMode::HorizontalTb {
        measure.insets.top + measure.insets.bottom
    } else {
        measure.insets.left + measure.insets.right
    };
    // `output.size` is a border-box size; content-box minimums need the
    // block-axis padding and border added before comparing them.
    let constrained_by_min_block_size = min_block_size.is_some_and(|min| {
        let min_border_box = min
            + if style.box_sizing == BoxSizing::ContentBox {
                block_axis_insets
            } else {
                0.0
            };
        min > 0.0 && min_border_box >= block_size
    });
    output.bottom_margin = if constrained_by_min_block_size {
        CollapsibleMarginSet::ZERO
    } else {
        escaping_margin
    };
    if inputs.run_mode == RunMode::PerformLayout {
        let in_flow_overflow = output.scrollable_overflow_rect;
        oof::layout_local_oof(tree, node_id, &mut output);
        center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, None);
        if let Some(area) = output.oof_positioning_area
            && let Some(hoisted_overflow) = vertical_oof::align_hoisted_children(tree, idx, area)
        {
            output.scrollable_overflow_rect = in_flow_overflow.union(hoisted_overflow);
        }
    }
    output
}

/// The min-content width of the border box of the paragraph box `node_id`,
/// without any width the box states itself.
fn ifc_min_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    parent_size: Size<Option<f32>>,
) -> f32 {
    let probe = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::ContentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size,
        available_space: Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    compute_ifc_root(tree, node_id, probe, None).size.width
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
fn ifc_length_only(value: LengthPercentageAuto) -> LengthPercentageAuto {
    let length = match value.expand() {
        ExpandedLengthPercentageAuto::Calc(handle) => !handle.has_percentage(),
        ExpandedLengthPercentageAuto::Length(_) => true,
        ExpandedLengthPercentageAuto::Percent(_) | ExpandedLengthPercentageAuto::Auto => false,
    };
    if length {
        value
    } else {
        LengthPercentageAuto::auto()
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
    min_size: LengthPercentageAuto,
    max_size: LengthPercentageAuto,
    box_sizing: BoxSizing,
    padding: [LengthPercentage; 2],
    border: [LengthPercentage; 2],
    dimension_basis: Option<f32>,
    edge_basis: Option<f32>,
) -> (f32, f32) {
    let edge_basis = edge_basis.unwrap_or(0.0);
    let resolve_lp = |value: LengthPercentage| {
        crate::layout::used_style_length_percentage(value, edge_basis).unwrap_or(0.0)
    };
    let insets = if box_sizing == BoxSizing::BorderBox {
        resolve_lp(padding[0])
            + resolve_lp(padding[1])
            + resolve_lp(border[0])
            + resolve_lp(border[1])
    } else {
        0.0
    };
    let resolve_dim = |value: LengthPercentageAuto, auto: f32| {
        // A percentage of an indefinite basis behaves as the initial value,
        // as taffy's leaf sizing resolves it.
        let basis = dimension_basis.or_else(|| {
            matches!(
                value.expand(),
                ExpandedLengthPercentageAuto::Calc(handle) if !handle.has_percentage()
            )
            .then_some(0.0)
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
    insets: Edges<f32>,
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
    oof_candidates: OofCandidates,
}

impl IfcMeasurement {
    const EMPTY: Self = Self {
        size: Size::ZERO,
        first_baseline: None,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        oof_candidates: OofCandidates::new(),
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
    // max-content measure is not a room to fit, and is kept; so is the room
    // that the limit of `fit-content(<length-percentage>)` makes.
    let page_width = tree.ifc.as_ref().and_then(|state| state.page_width);
    let limited = matches!(
        tree.physical_style(idx).size().width.expand(),
        ExpandedDimension::FitContentPx(_)
            | ExpandedDimension::FitContentPercent(_)
            | ExpandedDimension::FitContentCalc(_)
    );
    let available = match page_width {
        Some(page_width)
            if fixed
                && !limited
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
            AvailableSpace::Definite(size) => Some(size),
            _ => None,
        };
        let percent_height = measure.definite_height.filter(|_| !vertical);
        // A vertical paragraph's boxes are measured along their heights, and
        // their percentage widths refer to the physical width the root has
        // when it is known or stretched.
        let physical_width = known_width
            .or(match available.width {
                AvailableSpace::Definite(size) if measure.stretched_width => Some(size),
                _ => None,
            })
            .filter(|_| vertical);
        crate::layout::ifc::boxes::intrinsics_of_boxes(
            tree,
            idx,
            basis,
            percent_height,
            physical_width,
        )
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
    // The lines of a multicol container are laid out in its columns, so the
    // widths of the container repeat the widths of its lines.
    let (min_content, max_content) = tree.nodes[idx]
        .multicol
        .filter(|style| style.horizontal && !vertical)
        .map_or((min_content, max_content), |style| {
            style.intrinsic_inline_sizes((min_content, max_content), (0.0, 0.0))
        });
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
    // paragraph that holds only them is split line by line, in a nested
    // container as well. A paragraph with floats, blocks, out-of-flow boxes or
    // generated blocks of its own is laid out in one column as wide as the
    // container.
    let nested_fragmentation = (!vertical)
        .then(|| fragmentation_context_for_ifc(tree, idx, &probe))
        .flatten();
    let atomics_only = probe.generated_blocks.is_empty()
        && probe
            .boxes
            .iter()
            .all(|box_| box_.kind == crate::layout::ifc::boxes::IfcBoxKind::Atomic);
    // Column spanners among the boxes split the paragraph in groups of lines,
    // each of which fills the columns on its own.
    let spanner_groups = !vertical
        && nested_fragmentation.is_none()
        && crate::layout::multicol::paragraph_spanners::supports(tree, idx, &probe);
    let fragmentation = if vertical {
        None
    } else if nested_fragmentation.is_some() {
        nested_fragmentation
    } else if !atomics_only && !spanner_groups {
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
        // The spanners are laid out across the columns once the lines are
        // placed in them.
        geometry.blocks_apart = spanner_groups && fragmentation.is_some();
        geometry
    };
    // The block extent a vertical root with an auto block size settled on
    // last time at the same inline size starts the search for it, so that
    // the first pass is the last one. From its intrinsic inline size, which is
    // not an extent along the block axis, the search lays the children out a
    // second time to confirm the extent, and that doubles the work for every
    // level of vertical roots nested in each other.
    let seed_key = (vertical && !measure.stretched_width && known_width.is_none())
        .then(|| (idx, content_size.height.to_bits()));
    if let Some(seed) = seed_key.and_then(|key| tree.context.vertical_block_size_seeds.get(&key)) {
        content_size.width = *seed;
    }
    let mut geometry = geometry_of(content_size);
    // Lines, and the paragraph's own floats, are laid out by one loop; it
    // takes the engine state only around each call into the engine.
    let perform = measure.run_mode == RunMode::PerformLayout;
    let auto_vertical_block_size = vertical && !measure.stretched_width;
    let (mut lines, mut measured_block_size) = if auto_vertical_block_size {
        // Auto-size probes place floats even when they only measure; keep one probe in the
        // shared context, and restore it before the performed layout.
        let probe_context = block_ctx.as_deref().map(BlockContext::snapshot);
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
            if let (Some(ctx), Some(snapshot)) = (block_ctx.as_deref_mut(), &probe_context) {
                ctx.restore(snapshot);
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
        let needs_final_probe = (block_size - geometry.content_size.width).abs() > 0.01;
        if (perform || needs_final_probe)
            && let (Some(ctx), Some(snapshot)) = (block_ctx.as_deref_mut(), &probe_context)
        {
            ctx.restore(snapshot);
        }
        if needs_final_probe {
            content_size.width = block_size;
            geometry = geometry_of(content_size);
            if !perform {
                // The bounded probe loop may end between widths. Keep the
                // returned extent and shared float context on the same width.
                lines = crate::layout::ifc::boxes::layout_with_boxes_in(
                    tree,
                    idx,
                    geometry,
                    block_ctx.as_deref_mut(),
                    false,
                    measure.top_margin_escapes,
                    measure.bottom_margin_escapes,
                );
            }
        }
        // Only an extent the search settled on is worth starting from.
        if let Some(key) = seed_key.filter(|_| !needs_final_probe) {
            tree.context
                .vertical_block_size_seeds
                .insert(key, block_size);
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
    // The baselines of the groups of lines around the column spanners.
    let mut group_baselines = None;
    let fragments = lines
        .fragmentainer_line_ranges
        .clone()
        .map(|fragments| (fragments, lines.height))
        .or_else(|| {
            fragmentation.map(|context| {
                if !spanner_groups {
                    return crate::layout::root_column_fragments(&lines, context);
                }
                let groups = crate::layout::multicol::paragraph_spanners::arrange(
                    tree, idx, &lines, context, geometry, width, perform,
                );
                group_baselines = Some((groups.first_baseline, groups.last_baseline));
                (groups.fragments, groups.height)
            })
        });
    // The groups of an earlier layout of this root no longer apply when it
    // is not laid out in groups now, for instance inside the columns of
    // another container.
    if perform && group_baselines.is_none() {
        tree.nodes[idx].multicol_groups.clear();
    }
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
    // 10.8.1); every other box exposes its first. The lines of a vertical
    // root sit on the central baseline unless its text is set sideways.
    let kind = dominant_baseline(tree, idx);
    let inline_block =
        tree.nodes[idx].display() == raikiri_style::property::DisplayValue::InlineBlock;
    let baseline = match group_baselines {
        // The lines stand apart from the spanners between them.
        Some((first, last)) if inline_block => last.or(first),
        Some((first, _)) => first,
        None if inline_block => flow::last_baseline_of(&lines, kind),
        None => flow::first_baseline_of(&lines, kind),
    };
    let leading_margin = if measure.top_margin_escapes {
        lines
            .leading_block_margin
            .unwrap_or(CollapsibleMarginSet::ZERO)
    } else {
        CollapsibleMarginSet::ZERO
    };
    let block_size_is_auto = if vertical {
        known_width.is_none()
    } else {
        known_height.is_none()
    };
    let escaping_margin = if block_size_is_auto {
        lines.escaping_margin
    } else {
        CollapsibleMarginSet::ZERO
    };
    let oof_candidates = if measure.run_mode == RunMode::PerformLayout {
        lines.oof_candidates.clone()
    } else {
        OofCandidates::new()
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
        oof_candidates,
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
        let node = &tree.nodes[node_id];
        if let Some(multicol) = node.multicol {
            return (has_row_flex && multicol.horizontal).then_some(context);
        }
        has_row_flex |= node.formatting_context == FormattingContext::Flex
            && tree.physical_style(node_id).flex_direction_is_row();
    }
    None // cov:ignore: the unsupported ancestor path remains outside the ignored nested flex-float WPT scope.
}

/// Whether `overflow` makes a box a scroll container on either axis.
fn is_scroll_container(overflow: Point<Overflow>) -> bool {
    overflow.x.is_scroll_container() || overflow.y.is_scroll_container()
}

/// Keep each baseline of a scroll container within its border box, whose
/// block size is `height`.
fn clamp_baselines(baselines: &mut Baselines, height: f32) {
    for baseline in [&mut baselines.first, &mut baselines.last] {
        *baseline = baseline.map(|offset| offset.max(0.0).min(height));
    }
}

/// Whether the box `idx` is laid out on vertical axes: it has a vertical
/// writing mode, and is neither a table nor inside a multicol container, which
/// run vertical writing on horizontal axes.
fn on_vertical_axes(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.nodes[idx].used_writing_mode.is_vertical() && block_right_to_left(tree, idx).is_some()
}

/// The baseline the lines of the box `idx` are measured against: the central
/// baseline in a vertical writing mode whose text is not set sideways, and the
/// alphabetic one otherwise (CSS Writing Modes 4 §4.2). A box laid out on
/// horizontal axes has the alphabetic baseline whatever its writing mode.
fn dominant_baseline(tree: &LayoutSession<'_>, idx: usize) -> shodo::geometry::BaselineKind {
    if tree.nodes[idx].central_baseline && on_vertical_axes(tree, idx) {
        shodo::geometry::BaselineKind::Central
    } else {
        shodo::geometry::BaselineKind::Alphabetic
    }
}

/// The first block child of the vertical ifc root `idx` that has a baseline,
/// with that baseline: an offset along the block axis from the block-start
/// edge of the root's content box. The children of a `display: contents`
/// element count as the root's own.
///
/// The children's layouts recorded it when the root placed them, from the
/// baseline each exported: the one its own formatting context gave it,
/// clamped when it scrolls, and measured from its place in normal flow
/// whatever its relative offset.
fn first_block_baseline(doc: &LayoutSession<'_>, idx: usize) -> Option<(usize, f32)> {
    doc.nodes[idx].children.iter().find_map(|&child| {
        let node = &doc.nodes[child];
        if !node.is_in_document() || node.is_display_none() {
            None
        } else if node.display() == raikiri_style::property::DisplayValue::Contents {
            first_block_baseline(doc, child)
        } else {
            node.vertical_first_baseline
                .map(|baseline| (child, baseline))
        }
    })
}

/// Whether the box `idx` exports no baseline because layout containment
/// applies to it. The containment has no effect on `display: contents` and on
/// internal table boxes other than cells (CSS Containment 2 §3.2); a
/// non-atomic inline box is not laid out as a box of its own, so it never
/// asks.
/// spec: <https://www.w3.org/TR/css-contain-2/#containment-layout>
fn exports_no_baseline(doc: &LayoutSession<'_>, idx: usize) -> bool {
    doc.physical_style(idx).contain().contains(Contain::LAYOUT)
        && layout_containment_applies(doc, idx)
}

/// Whether layout containment can apply to the box `idx`, by its `display`.
fn layout_containment_applies(doc: &LayoutSession<'_>, idx: usize) -> bool {
    !matches!(
        doc.nodes[idx].display(),
        DisplayValue::Contents
            | DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup
    )
}

/// Return the first baseline of an inline-block from the descendant layout
/// tree: inline-blocks use the baseline of their last in-flow line box, so recover
/// the last in-flow text line from the descendant layout tree. See CSS 2.1
/// §10.8.1: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn first_inline_baseline(doc: &LayoutSession<'_>, root: usize) -> Option<f32> {
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
            && doc.nodes[child].layout_position() != Position::Absolute
        {
            stack.push((
                child,
                doc.nodes[child].layout.location.y,
                (f32::NEG_INFINITY, f32::INFINITY),
            ));
        }
    }
    let mut last_baseline = None;
    while let Some((idx, offset_y, outer_clip)) = stack.pop() {
        let node = &doc.nodes[idx];
        // Clamping a value to this box and then to the boxes around it is
        // clamping it to this box's edges, each clamped to the outer range.
        let is_text = node.kind() == raikiri_traits::NodeKind::Text;
        let style = (!is_text).then(|| doc.physical_style(idx));
        let clip = if style
            .as_ref()
            .is_some_and(|style| is_scroll_container(style.overflow()))
        {
            let bottom = offset_y + node.layout.size.height;
            (
                offset_y.max(outer_clip.0).min(outer_clip.1),
                bottom.max(outer_clip.0).min(outer_clip.1),
            )
        } else {
            outer_clip
        };
        // A box with layout containment exports no baseline, so the lines
        // inside it say nothing about where this one sits (CSS Contain 2
        // §3.2).
        // spec: https://www.w3.org/TR/css-contain-2/#containment-layout
        if style
            .as_ref()
            .is_some_and(|style| style.contain().contains(Contain::LAYOUT))
            && layout_containment_applies(doc, idx)
        {
            continue;
        }
        if node.flags.contains(NodeFlags::IS_IFC_ROOT) {
            if let Some(line_baseline) = node.ifc.as_ref().and_then(|root| root.last_baseline()) {
                let layout = &node.layout;
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
                && doc.nodes[child].layout_position() != Position::Absolute
            {
                stack.push((child, offset_y + doc.nodes[child].layout.location.y, clip));
            }
        }
    }
    last_baseline
}

/// Whether `idx` is a block-level `<button>` that is a direct child of a
/// multicol container. A column does not stretch it to its width, as a block
/// does not (HTML Standard 15.5.3): its `auto` width is the fit-content one,
/// which the column's width bounds.
fn is_button_in_multicol(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.nodes[idx].is_html_button()
        && tree
            .taffy_parent_of(idx)
            .is_some_and(|parent| tree.nodes[parent].is_multicol_container())
}

/// Whether element `idx` is a flex or grid container that floats.
///
/// A float is shrink-to-fit like an inline-level box (CSS 2.1 section 10.3.5);
/// Taffy sizes such a flex or grid container from the flex base sizes of its
/// items instead, which differ from the fit-content size.
fn is_floated_flex_or_grid(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.nodes[idx].display().is_block_flex_or_grid_container()
        && tree.physical_style(idx).float() != Float::None
}

/// The border-box `min-width` and `max-width` of element `idx` that resolve
/// against `parent_width`, the maximum no smaller than the minimum and neither
/// smaller than the padding and border.
fn border_box_width_bounds(
    tree: &LayoutSession<'_>,
    idx: usize,
    parent_width: Option<f32>,
) -> (f32, f32) {
    let style = tree.physical_style(idx);
    let basis = parent_width.unwrap_or(0.0);
    let adjustment = crate::layout::ifc::boxes::box_sizing_adjustment(tree, idx, basis);
    let edges = crate::layout::ifc::boxes::horizontal_padding_border(tree, idx, basis);
    let resolve = |value, auto: f32| {
        crate::layout::resolve_length_percentage_auto(value, parent_width)
            .filter(|resolved| resolved.is_finite())
            .map_or(auto, |resolved| resolved + adjustment)
    };
    let min = resolve(style.min_size().width, 0.0).max(edges);
    let max = resolve(style.max_size().width, f32::INFINITY);
    (min, max.max(min))
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
    // An empty flex or grid container still runs its layout algorithm:
    // explicit grid tracks contribute even when there are no items.
    let contain_natural = contain_intrinsic_natural(&tree.nodes[idx]).filter(|_| empty);
    let is_leaf = contain_natural.is_some()
        || (tree.nodes[idx].children.is_empty()
            && !matches!(
                display,
                DisplayValue::Flex
                    | DisplayValue::InlineFlex
                    | DisplayValue::Grid
                    | DisplayValue::InlineGrid
            ));
    let limits = tree.nodes[idx].content_limits;
    // Clone what the leaf path needs before any exclusive tree use below.
    let mut leaf_style = block_flow::style_input::physical(tree, idx);
    zero_cyclic_calc_sizes(
        &mut leaf_style,
        &inputs,
        tree.physical_style(idx).is_replaced(),
    );
    if limits != content_limits::ContentLimits::NONE
        && !empty
        && !tree.nodes[idx].size_containment.width
        && !tree.nodes[idx].size_containment.height
        && leaf_style.aspect_ratio.is_none()
    {
        apply_local_sizing(
            &mut leaf_style,
            &[OwnedPropertyOverride::aspect_ratio(
                tree.nodes[idx].keyword_aspect_ratio,
            )],
        );
    }
    if empty {
        apply_local_sizing(
            &mut leaf_style,
            &[OwnedPropertyOverride::aspect_ratio(None)],
        );
    }
    let leaf_intrinsic = leaf_intrinsic_size(&tree.nodes[idx])
        .filter(|_| !empty)
        .or(contain_natural);
    // A replaced leaf with an `auto` side is sized by its ratio and its
    // min/max sizes, not by Taffy's ratio floor, before any width is fixed.
    let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
    // An item of a column flex container takes its width from a height along its
    // ratio. While the container's own width is measured, that is the height the
    // item states, though the item is measured for its content there: its main
    // size is not content-sized. Once the container's width is fixed, which Taffy
    // says by offering the item a definite width, it is the height the
    // container flexed the item to. An item that states `height: stretch` states
    // the height of its container, which is the height it is offered at any
    // width.
    let in_column_flex = leaf_intrinsic.is_some() && tree.in_column_flex_container(idx);
    let stretches_height = matches!(leaf_style.size.height.expand(), ExpandedDimension::Stretch);
    // While the container's width is measured, which Taffy says by offering the
    // item a min-content or max-content width, a stretched item contributes the
    // width of the height it stretches to, the definite height of the
    // container less its margins, whatever height the container flexes it to:
    // the width of the container does not shrink with its items.
    let stretch_room = inputs.parent_size.height.filter(|_| {
        in_column_flex
            && stretches_height
            && inputs.sizing_mode == SizingMode::ContentSize
            && inputs.axis == RequestedAxis::Horizontal
            && inputs.run_mode == RunMode::ComputeSize
            && inputs.known_dimensions.height.is_some()
            && inputs.known_dimensions_are_definite.height
            && !matches!(inputs.available_space.width, AvailableSpace::Definite(_))
    });
    let flexed_height = if let Some(room) = stretch_room {
        // A percentage margin counts as zero in the width the item contributes.
        let margin = leaf_style
            .margin
            .map(|edge| edge.resolve(Some(0.0)).unwrap_or(0.0));
        Some((room - margin.top - margin.bottom).max(0.0))
    } else {
        inputs.known_dimensions.height.filter(|_| {
            in_column_flex
                && inputs.sizing_mode == SizingMode::ContentSize
                && (stretches_height
                    || matches!(inputs.available_space.width, AvailableSpace::Definite(_)))
        })
    };
    // An item of a row flex container whose height is fixed before its width is
    // measured, because it states the height or the container stretches it to a
    // definite one, takes its width from that height along its ratio, say for its
    // automatic minimum size. A container that wraps its items does not take the
    // height it stretches an item to as definite then.
    //
    // Nor does a container whose own height is not definite stretch an item
    // with an `auto` height to anything: the height that Taffy still offers
    // such an item is only its largest height, which its `max-*` sizes make,
    // and it gives the item no width through the ratio.
    let horizontal_row_item = leaf_intrinsic.is_some()
        && !tree.has_vertical_writing_mode(idx)
        && tree.in_row_flex_container(idx);
    let row_height = inputs.known_dimensions.height.filter(|_| {
        leaf_intrinsic.is_some()
            && !in_column_flex
            && inputs.sizing_mode == SizingMode::ContentSize
            && inputs.known_dimensions_are_definite.height
            && tree.in_row_flex_container(idx)
            && !(horizontal_row_item
                && leaf_style.size.height.is_auto()
                && flex_item_percent_basis(tree, idx).is_some_and(|basis| basis.height.is_none()))
            && !tree.is_wrapping_stretch_hint(idx, &leaf_style, inputs.known_dimensions)
    });
    // The height that `height: stretch` gave a box with an aspect ratio is as
    // good as one it states, so the width follows from it through the ratio.
    let stretched_height = inputs
        .known_dimensions
        .height
        .filter(|_| leaf_style.aspect_ratio.is_some() && stretches_height);
    // A replaced box given the height it fills, say an image in a line of text,
    // is sized by it before any width is fixed.
    let stretched_by_parent = stretched_height
        .filter(|_| leaf_intrinsic.is_some() && inputs.sizing_mode == SizingMode::InherentSize);
    let sides_style = flexed_height
        .or(row_height)
        .or(stretched_by_parent)
        .map(|height| {
            let mut style = leaf_style.clone();
            apply_border_box_height(
                tree,
                idx,
                &mut style,
                height,
                inputs.parent_size.width.unwrap_or(0.0),
            );
            style
        });
    // An item of a row flex container that states no size, and has no height
    // fixed for it, is measured for its content as it is sized outside a flex
    // container: by its natural size within the `min-*` and `max-*` sizes it
    // states, where the sizes of one axis carry over to the other along its
    // ratio (CSS 2.1 §10.4).
    let sized_by_its_bounds = horizontal_row_item
        && row_height.is_none()
        && inputs.sizing_mode == SizingMode::ContentSize
        && leaf_style.size.width.is_auto()
        && leaf_style.size.height.is_auto();
    let leaf_auto_sides = leaf_intrinsic.and_then(|natural| {
        replaced_size::auto_sides_size(
            sides_style.as_ref().unwrap_or(&leaf_style),
            &LayoutInput {
                known_dimensions: Size::NONE,
                sizing_mode: if in_column_flex || row_height.is_some() || sized_by_its_bounds {
                    SizingMode::InherentSize
                } else {
                    inputs.sizing_mode
                },
                // A percentage height has a basis only while the height of the
                // container does not depend on the content of its items.
                parent_size: Size {
                    width: inputs.parent_size.width,
                    height: inputs
                        .parent_size
                        .height
                        .filter(|_| !in_column_flex || inputs.known_dimensions_are_definite.height),
                },
                ..inputs
            },
            natural,
            limits,
            content_box_ratio,
        )
    });
    let leaf_intrinsic_before_width = leaf_auto_sides.or(leaf_intrinsic);
    // Scalar copies so the measure closure below captures no large state.
    let sizing_mode = inputs.sizing_mode;
    let known_height = inputs.known_dimensions.height;
    let avail_height = inputs.available_space.height;
    // It is put in the style the box is sized with.
    if let Some(height) = stretched_height {
        apply_border_box_height(
            tree,
            idx,
            &mut leaf_style,
            height,
            inputs.parent_size.width.unwrap_or(0.0),
        );
    }

    // Taffy's ratio floor is off for a leaf the helper above sizes.
    let mut ratio_free_style = leaf_style.clone();
    if leaf_auto_sides.is_some() {
        apply_local_sizing(
            &mut ratio_free_style,
            &[OwnedPropertyOverride::aspect_ratio(None)],
        );
    }

    // The probes of a box that holds content run that content once each, and
    // they are asked again whenever the box is laid out at another width. A
    // parent that tries three widths on a box in a chain of nested ones would
    // make every level lay out its box three times as often as the one above,
    // so the answers are kept in the node's cache. Only while the node's own
    // cache frame is open: a box laid out apart from its cache (in a block
    // context shared with its parent, say) also measures apart from it. A flex
    // container that has a `max-width`, or items with a length as their flex
    // basis, measures under style overrides, which move the cache generation,
    // so its answers are neither found nor kept.
    let cache_probes = !is_leaf && tree.caches_node(node_id);

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
                &LeafStyle::new(&ratio_free_style),
                resolve_calc,
                |known, _avail| Size {
                    width: known
                        .width
                        .or(leaf_intrinsic_before_width.map(|s| s.width))
                        .unwrap_or(0.0),
                    height: known
                        .height
                        .or(leaf_intrinsic_before_width.map(|s| s.height))
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
            if cache_probes && let Some(cached) = tree.cached_width(node_id, &intrinsic_inputs) {
                return cached;
            }
            let output = match display {
                DisplayValue::Flex | DisplayValue::InlineFlex => {
                    flex_layout(tree, node_id, intrinsic_inputs)
                }
                DisplayValue::Grid | DisplayValue::InlineGrid => {
                    grid_layout(tree, node_id, intrinsic_inputs)
                }
                _ if ifc_root => compute_ifc_root(tree, node_id, intrinsic_inputs, None),
                _ => block_layout(tree, node_id, intrinsic_inputs, None),
            };
            let width = output.size.width;
            if cache_probes {
                tree.cache_width(node_id, &intrinsic_inputs, width);
            }
            width
        }
    };

    let measure = |tree: &mut LayoutSession<'_>| {
        (
            intrinsic_width(tree, AvailableSpace::MinContent),
            intrinsic_width(tree, AvailableSpace::MaxContent),
        )
    };
    let (min_w, max_w) = if matches!(display, DisplayValue::Flex | DisplayValue::InlineFlex) {
        let (_, max_width) = border_box_width_bounds(tree, idx, inputs.parent_size.width);
        let (min_w, max_w) = with_flex_intrinsic_probe(tree, node_id, measure);
        (min_w.min(max_width), max_w.min(max_width))
    } else {
        measure(tree)
    };
    // `clamp` panics on inverted bounds, so order them whatever the two probes
    // returned.
    let (lo, hi) = (min_w.min(max_w), min_w.max(max_w));
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
        // The width is fixed now; the height follows it through the ratio.
        let fixed_width = leaf_intrinsic.and_then(|natural| {
            replaced_size::auto_sides_size(
                &leaf_style,
                &final_inputs,
                natural,
                limits,
                content_box_ratio,
            )
        });
        // The ratio has done its work for a box whose height the parent fixed
        // and whose width it gave, and Taffy's ratio floor would raise that
        // height again to the border-box width divided by the ratio.
        if fixed_width.is_some() || stretched_by_parent.is_some() {
            apply_local_sizing(
                &mut leaf_style,
                &[OwnedPropertyOverride::aspect_ratio(None)],
            );
        }
        let natural = fixed_width.or(leaf_intrinsic);
        compute_leaf_layout(
            final_inputs,
            &LeafStyle::new(&leaf_style),
            resolve_calc,
            |known, _avail| Size {
                width: known.width.or(natural.map(|s| s.width)).unwrap_or(0.0),
                height: known.height.or(natural.map(|s| s.height)).unwrap_or(0.0),
            },
        )
    } else {
        match display {
            DisplayValue::Flex | DisplayValue::InlineFlex => {
                flex_layout(tree, node_id, final_inputs)
            }
            DisplayValue::Grid | DisplayValue::InlineGrid => {
                grid_layout(tree, node_id, final_inputs)
            }
            _ if ifc_root => compute_ifc_root(tree, node_id, final_inputs, block_ctx),
            _ => block_layout(tree, node_id, final_inputs, block_ctx),
        }
    }
}

/// Position a vertical block's out-of-flow boxes after its flow is aligned.
/// Table cells defer this step until their final block-axis alignment.
pub(crate) fn finish_vertical_block_oof(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: &LayoutInput,
    mut output: LayoutOutput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let in_flow_overflow = output.scrollable_overflow_rect;
    if inputs.run_mode == RunMode::PerformLayout {
        oof::layout_local_oof_now(tree, node_id, &mut output);
    }
    center_absolute_children_in_block_axis(tree, idx, inputs, output.size, None);
    if let Some(area) = output.oof_positioning_area
        && let Some(hoisted_overflow) = vertical_oof::align_hoisted_children(tree, idx, area)
        && inputs.run_mode == RunMode::PerformLayout
    {
        output.scrollable_overflow_rect = in_flow_overflow.union(hoisted_overflow);
    }
    output
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
        let block_ctx =
            block_ctx.filter(|_| vertical_block::has_same_vertical_block_parent(tree, idx));
        let output =
            vertical_block::compute_vertical_block(tree, node_id, inputs, right_to_left, block_ctx);
        if tree.nodes[idx]
            .flags
            .contains(NodeFlags::FINAL_VERTICAL_TABLE_CELL)
        {
            return output;
        }
        return finish_vertical_block_oof(tree, node_id, &inputs, output);
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
    if inputs.run_mode == RunMode::PerformLayout {
        oof::layout_local_oof(tree, node_id, &mut output);
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
    let mut empty = tree
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
    if let Some(natural) = grid_with_one_intrinsic_length(&tree.nodes[idx]) {
        // A leaf of the stated length gives the size of that axis with the
        // padding, the border and the limits of the box around it.
        let stated = tree.nodes[idx].contain_intrinsic;
        let leaf = compute_leaf_layout(
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                axis: RequestedAxis::Both,
                ..inputs
            },
            &TaffyStyle::new(tree.physical_style(idx)),
            resolve_calc,
            |known, _| Size {
                width: known.width.unwrap_or(natural.width),
                height: known.height.unwrap_or(natural.height),
            },
        );
        if stated.width.is_some() {
            empty.width = leaf.size.width;
        }
        if stated.height.is_some() {
            empty.height = leaf.size.height;
        }
    }
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

/// The absolutely positioned boxes that `idx` is the containing block of and
/// that generate a box, as the last positioning pass hoisted them.
fn hoisted_children(tree: &LayoutSession<'_>, idx: usize) -> Vec<usize> {
    tree.context
        .taffy_hoisted_children
        .get(&idx)
        .into_iter()
        .flatten()
        .map(|child| usize::from(*child))
        .filter(|&child| !tree.nodes[child].is_display_none())
        .collect()
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
    grid: Option<&grid_oof::GridAreas<'_>>,
) {
    if inputs.run_mode != RunMode::PerformLayout {
        return;
    }
    let block_is_height = !used_writing_mode(tree, idx).is_vertical();
    let (border, direction) = {
        let style = tree.physical_style(idx);
        (
            style.border().resolve_or_zero(inputs.parent_size.width),
            style.direction(),
        )
    };
    let padding_box = Edges {
        left: border.left,
        right: size.width - border.right,
        top: border.top,
        bottom: size.height - border.bottom,
    };
    let children = hoisted_children(tree, idx);
    for child in children.iter().copied() {
        let style = tree.physical_style(child);
        let (margin, inset) = (style.margin(), style.inset());
        let (start, end) = if block_is_height {
            (margin.top, margin.bottom)
        } else {
            (margin.left, margin.right)
        };
        if !start.is_auto() || !end.is_auto() {
            continue;
        }
        let area = match grid {
            Some(grid) => grid.area(&style, direction, padding_box),
            None => padding_box,
        };
        let (area_start, area_end, start, end) = if block_is_height {
            (area.top, area.bottom, inset.top, inset.bottom)
        } else {
            (area.left, area.right, inset.left, inset.right)
        };
        let area_size = area_end - area_start;
        let start = start.maybe_resolve(area_size);
        let end = end.maybe_resolve(area_size);
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        let mut layout = tree.nodes[child].layout;
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
    if !block_is_height {
        for child in children {
            let style = tree.physical_style(child);
            let (margin, inset) = (style.margin(), style.inset());
            if !margin.top.is_auto() || !margin.bottom.is_auto() {
                continue;
            }
            let area = match grid {
                Some(grid) => grid.area(&style, direction, padding_box),
                None => padding_box,
            };
            let start = inset.top.maybe_resolve(area.bottom - area.top);
            let end = inset.bottom.maybe_resolve(area.bottom - area.top);
            let (Some(start), Some(end)) = (start, end) else {
                continue;
            };
            let mut layout = tree.nodes[child].layout;
            let free_space = area.bottom - area.top - start - end - layout.size.height;
            if free_space >= 0.0 {
                continue;
            }
            layout.margin.top = 0.0;
            layout.margin.bottom = 0.0;
            layout.location.y = if direction == Direction::Rtl {
                area.bottom - end - layout.size.height
            } else {
                area.top + start
            };
            tree.set_unrounded_layout(NodeId::from(child), &layout);
        }
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
    let inputs = with_clamped_keyword_width(tree, node_id, inputs, flex_layout_sized);
    if block_right_to_left(tree, usize::from(node_id)).is_none() {
        return laid_out_with_ratio_height(tree, node_id, inputs, flex_layout_sized);
    }
    flex_layout_sized(tree, node_id, inputs)
}

/// The border-box width CSS Flexbox 1 section 9.9.1 gives a row flex container
/// that wraps its items at its min-content width: that of its widest item, as
/// each item breaks onto a line of its own. Taffy sums the flex base sizes of
/// the items of a line instead, which is neither the width an item contributes
/// nor bounded by its `max-width`. `None` for any other container, and for one
/// that states its width or runs on the axes of a vertical writing mode.
fn wrapping_row_min_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: &LayoutInput,
) -> Option<f32> {
    let idx = usize::from(node_id);
    if inputs.known_dimensions.width.is_some()
        || block_right_to_left(tree, idx).is_some()
        || !{
            let style = tree.physical_style(idx);
            style.flex_direction_is_row() && style.flex_wraps()
        }
    {
        return None;
    }
    let (size, inset, _) = tree.block_definite_size(idx, inputs);
    if size.width.is_some() {
        return None;
    }
    let room = item_cross_room(
        size.height.map(|height| (height - inset.height).max(0.0)),
        inputs,
    );
    let (min, max) = border_box_width_bounds(tree, idx, inputs.parent_size.width);
    let width = wrapping_min_content_width(tree, node_id, room) + inset.width;
    Some(width.clamp(min, max))
}

/// Give a row flex container that wraps its items, and is laid out at its
/// min-content width, the width [`wrapping_row_min_content_width`] gives it.
fn with_wrapping_min_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if inputs.available_space.width == AvailableSpace::MinContent
        && let Some(width) = wrapping_row_min_content_width(tree, node_id, &inputs)
    {
        inputs.known_dimensions.width = Some(width);
        inputs.known_dimensions_are_definite.width = true;
    }
    inputs
}

/// The inner height of a flex container that its items take their height from
/// while they are measured for their widths, given the `inner_height` the
/// container has if it is definite. The height of a container that follows its
/// items is no room for them.
fn item_cross_room(inner_height: Option<f32>, inputs: &LayoutInput) -> Option<f32> {
    inner_height.filter(|_| {
        inputs.known_dimensions.height.is_none() || inputs.known_dimensions_are_definite.height
    })
}

/// Whether `inputs` ask for the size of a container at its min-content or
/// max-content width, wherever the query comes from: the shrink-to-fit width of
/// the container, a `min-content` or `max-content` keyword width, or a parent
/// that measures it as an item. The flex base sizes of its items bound them
/// then (CSS Flexbox 1 section 9.9.1).
fn measures_intrinsic_width(inputs: &LayoutInput) -> bool {
    inputs.run_mode == RunMode::ComputeSize
        && inputs.known_dimensions.width.is_none()
        && matches!(
            inputs.available_space.width,
            AvailableSpace::MinContent | AvailableSpace::MaxContent
        )
}

/// [`flex_layout`] once the width of a `fit-content` container is settled.
fn flex_layout_sized(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let inputs = with_wrapping_min_content_width(tree, node_id, inputs);
    // The max-content width of a container that wraps its items is no less
    // than its min-content width: the widest item, whatever its flex base size.
    let max_content_floor = if inputs.run_mode == RunMode::ComputeSize
        && inputs.available_space.width == AvailableSpace::MaxContent
    {
        wrapping_row_min_content_width(tree, node_id, &inputs)
    } else {
        None
    };
    let (size, inset, _) = tree.block_definite_size(idx, &inputs);
    let basis = Size {
        width: size.width.map(|width| (width - inset.width).max(0.0)),
        height: size.height.map(|height| (height - inset.height).max(0.0)),
    };
    tree.context.flex_item_percent_bases.push((idx, basis));
    let mut output = content_limits::with_item_content_limits(
        tree,
        node_id,
        inputs,
        basis,
        |tree, node_id, inputs| {
            if block_right_to_left(tree, idx).is_some() {
                // The container runs its algorithm on the axes of its own
                // writing mode.
                tree.compute_on_own_axes(node_id, inputs, |tree, node_id, inputs| {
                    let inputs = vertical_block::with_atomic_fit_content(
                        tree,
                        node_id,
                        inputs,
                        compute_flexbox_layout,
                    );
                    compute_flexbox_layout(tree, node_id, inputs)
                })
            } else if tree.context.flex_intrinsic_probe == Some(idx)
                || measures_intrinsic_width(&inputs)
            {
                let cross_room = item_cross_room(basis.height, &inputs);
                with_flex_item_basis_bounds(tree, node_id, cross_room, |tree| {
                    compute_flexbox_layout(tree, node_id, inputs)
                })
            } else {
                compute_flexbox_layout(tree, node_id, inputs)
            }
        },
    );
    if let Some(width) = content_limits::wrapping_column_min_content_width(tree, node_id, &inputs) {
        let (min, max) = border_box_width_bounds(tree, idx, inputs.parent_size.width);
        output.size.width = width.clamp(min, max);
    }
    if let Some(width) = max_content_floor {
        output.size.width = output.size.width.max(width);
    }
    let in_flow_overflow = output.scrollable_overflow_rect;
    if inputs.run_mode == RunMode::PerformLayout {
        oof::layout_local_oof(tree, node_id, &mut output);
    }
    tree.context.flex_item_percent_bases.pop();
    center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, None);
    // Taffy aligns the items of a column container by their baselines as if
    // they were at `start`; its cross axis is the inline axis of the container.
    // It also keeps the items of a vertical row that run against the block
    // direction of the container at `start`, for the container to align.
    let vertical = block_right_to_left(tree, idx).is_some();
    let in_flow_overflow = if inputs.run_mode == RunMode::PerformLayout
        && let Some(reach) = inline_baseline::align_flex(
            tree,
            idx,
            output.size,
            vertical,
            if vertical {
                vertical_block::containing_inline_size(tree, idx, &inputs)
            } else {
                inputs.parent_size.width
            },
            &mut output.oof_candidates,
        ) {
        output.scrollable_overflow_rect = output.scrollable_overflow_rect.union(reach);
        in_flow_overflow.union(reach)
    } else {
        in_flow_overflow
    };
    // Taffy aligns the boxes by the physical axes of `horizontal-tb`; a flex
    // container on the axes of a vertical writing mode aligns them by its own.
    if inputs.run_mode == RunMode::PerformLayout
        && let Some(area) = output.oof_positioning_area
        && let Some(hoisted_overflow) = vertical_oof::align_hoisted_children(tree, idx, area)
    {
        output.scrollable_overflow_rect = in_flow_overflow.union(hoisted_overflow);
    }
    output
}

/// Fill a grid item's normal inline alignment at the offered area width,
/// preserving the width floor transferred from its minimum height and edges
/// and the ceiling transferred from its maximum height. Content may raise
/// its height without transferring that height back to the width.
/// A width-only minimum-contribution probe states a width and is left alone.
fn with_grid_ratio_width(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if inputs.sizing_mode == SizingMode::ContentSize
        || !tree
            .taffy_parent_of(idx)
            .is_some_and(|parent| tree.nodes[parent].layout_display() == Display::Grid)
        || !matches!(
            tree.query_style(idx, crate::layout::style_views::ReadRole::GridItem)
                .context()
                .grid_item_ratio,
            raikiri_layout_style::GridItemRatio::MeasuredByItem { .. }
        )
    {
        return inputs;
    }
    let style = block_flow::style_input::physical(tree, idx);
    let item = tree.physical_style(idx);
    let justify = item.bridge_justify_self().unwrap_or_else(|| {
        tree.taffy_parent_of(idx)
            .map_or(raikiri_layout_style::values::AlignSelf::NORMAL, |parent| {
                tree.physical_style(parent).justify_items()
            })
    });
    if justify.keyword == raikiri_layout_style::values::AlignItemsKeyword::Stretch
        && inputs.known_dimensions.width.is_some()
    {
        return inputs;
    }
    let content_ratio = tree.nodes[idx].ratio_in_content_box;
    let edges = replaced_size::padding_and_border(&style, inputs.parent_size.width);
    let metrics = block_flow::Metrics::resolve(&style, false, content_ratio, inputs);
    let floor = metrics.min.height.unwrap_or(0.0).max(edges.height);
    let insets = replaced_size::ratio_box_insets(&style, edges, content_ratio);
    let floor = replaced_size::width_for_height(
        (floor - edges.height).max(0.0),
        style.aspect_ratio.expect("checked ratio"),
        insets,
    ) + edges.width;
    let floor = floor.maybe_clamp(metrics.min.width, metrics.own_max.width);
    // A minimum height that exceeds the maximum wins before either is
    // transferred; an authored maximum width has already capped the floor.
    let ceiling = metrics.max.width.map(|maximum| maximum.max(floor));
    let normal_width = justify.keyword == raikiri_layout_style::values::AlignItemsKeyword::Normal;
    let mut width = inputs.known_dimensions.width.or_else(|| {
        normal_width
            .then(|| inputs.available_space.width.into_option())
            .flatten()
    });
    if width.is_none() && (floor > 0.0 || ceiling.is_some()) {
        // Keep a non-stretched item's natural shrink-to-fit width within
        // the transferred limits, including an unbounded max-content probe.
        let probe = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Horizontal,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size::splat(false),
            ..inputs
        };
        width = Some(
            tree.compute_child_layout(NodeId::from(idx), probe)
                .size
                .width,
        );
    }
    if let Some(width) = width {
        inputs.known_dimensions.width =
            Some(width.max(floor).maybe_clamp(metrics.min.width, ceiling));
        inputs.known_dimensions_are_definite.width = true;
    }
    inputs
}

/// Keep the height Taffy settles an absolutely positioned box at from being a
/// definite height for its children.
///
/// Taffy lays such a box out a second time at the size it measured, and calls
/// both sizes definite. A box with an `auto` height and a `top` or `bottom`
/// of `auto` has a height that follows from its content, and so a percentage
/// height of a child is `auto` (CSS 2.1 10.5). A `height` of the box's own, a
/// `top` and `bottom` that fix it, an aspect ratio that gives it one, or a
/// `min-height` that its `max-height` does not exceed, make it definite.
fn with_content_height_not_definite(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if inputs.known_dimensions.height.is_none()
        || !inputs.known_dimensions_are_definite.height
        || tree.nodes[idx].layout_position() != Position::Absolute
    {
        return inputs;
    }
    let follows_content = {
        let style = tree.physical_style(idx);
        let min = style.min_size().maybe_resolve(inputs.parent_size).height;
        let max = style.max_size().maybe_resolve(inputs.parent_size).height;
        let pinned = matches!((min, max), (Some(min), Some(max)) if max <= min);
        let inset = style.inset();
        style.size().height.is_auto()
            && style.aspect_ratio().is_none()
            && !pinned
            && (inset.top.is_auto() || inset.bottom.is_auto())
    };
    if follows_content {
        inputs.known_dimensions_are_definite.height = false;
    }
    inputs
}

/// Offer a box sized by `fit-content(<length-percentage>)` its limit as the
/// room it shrinks to fit in, whatever room its parent offers.
///
/// A limit that cannot resolve (see [`fit_content_room`]) leaves the room
/// alone, so the box is sized by the room it is offered.
fn with_fit_content_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if inputs.known_dimensions.width.is_some() {
        return inputs;
    }
    let room = {
        let size = tree.physical_style(idx).size().width.expand();
        fit_content_room(tree, idx, size, inputs.parent_size.width, true)
    };
    if let Some(room) = room {
        inputs.available_space.width = AvailableSpace::Definite(room);
    }
    inputs
}

/// Offer an item of a column flex container with a `min-content` or
/// `max-content` width that room, instead of the room of its container.
///
/// Taffy measures such an item along the main axis, its height, in the room of
/// its container, which the keyword width ignores; the lines of the item would
/// then break at another width than the item gets. A parent that asks for the
/// min-content or max-content size of the content (the automatic minimum size
/// of a flex item, say) gets what it asks for.
fn with_column_flex_keyword_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if inputs.known_dimensions.width.is_some()
        || !matches!(inputs.available_space.width, AvailableSpace::Definite(_))
    {
        return inputs;
    }
    let room = match tree.physical_style(idx).size().width.expand() {
        ExpandedDimension::MinContent => Some(AvailableSpace::MinContent),
        ExpandedDimension::MaxContent => Some(AvailableSpace::MaxContent),
        _ => None,
    }
    .filter(|_| tree.in_column_flex_container(idx));
    if let Some(room) = room {
        inputs.available_space.width = room;
    }
    inputs
}

/// Give an item of a flex container with a `stretch` cross size the size the
/// container gives it while Taffy measures the item, where the size does not
/// depend on the line the item lands in.
///
/// An item of a wrapping column container: a flex line is as wide as its
/// widest item, and the width of an item that stretches counts as the width of
/// the container there (CSS Sizing 4 §5.1). Taffy leaves the keyword open until
/// the item stretches to its line, so a line of stretching items alone would be
/// as wide as their contents. The line still gives the item its final width.
///
/// An item with an aspect ratio, of a container that keeps its items on one
/// line: the cross size the keyword stands for is definite when the flex base
/// size and the automatic minimum size are found, and they follow from it
/// through the ratio. Taffy takes the cross size from the item's own `width`
/// or `height` or from `align-self: stretch`, not from the keyword.
fn with_stretch_cross_size(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    let Some(container) = tree
        .taffy_parent_of(idx)
        .filter(|&parent| tree.nodes[parent].layout_display() == Display::Flex)
        .map(|parent| tree.physical_style(parent))
    else {
        return inputs;
    };
    let item = tree.physical_style(idx);
    let ratio = item.stated_aspect_ratio().is_some();
    if container.flex_direction_is_row() {
        if !ratio
            || container.flex_wraps()
            || inputs.known_dimensions.height.is_some()
            || inputs.parent_size.height.is_none()
            || item.size().height.expand() != ExpandedDimension::Stretch
        {
            return inputs;
        }
        inputs.known_dimensions.height = content_limits::stretch_fit_height(tree, idx, &inputs);
        inputs.known_dimensions_are_definite.height = true;
        return inputs;
    }
    let AvailableSpace::Definite(room) = inputs.available_space.width else {
        return inputs;
    };
    if inputs.known_dimensions.width.is_some()
        || inputs.parent_size.width.is_none()
        || item.size().width.expand() != ExpandedDimension::Stretch
        || !(container.flex_wraps() || ratio)
    {
        return inputs;
    }
    inputs.known_dimensions.width =
        Some(content_limits::stretch_fit_width(tree, idx, &inputs, room));
    inputs.known_dimensions_are_definite.width = true;
    inputs
}

/// Whether the width of the flex container `idx` is the `fit-content` one: the
/// keyword, `fit-content(<length-percentage>)`, or the `auto` width of an
/// absolutely positioned box, which shrinks to fit (CSS Positioned Layout 3
/// section 5.1.1) unless its insets state the width.
fn has_fit_content_width(tree: &LayoutSession<'_>, idx: usize) -> bool {
    let width = tree.physical_style(idx).size().width;
    matches!(
        width.expand(),
        ExpandedDimension::FitContent
            | ExpandedDimension::FitContentPx(_)
            | ExpandedDimension::FitContentPercent(_)
            | ExpandedDimension::FitContentCalc(_)
    ) || (width.is_auto() && tree.nodes[idx].layout_position() == Position::Absolute)
}

/// Give a flex container with a `fit-content` width (see
/// [`has_fit_content_width`]), or a `fit-content(<length-percentage>)` one whose
/// limit [`with_fit_content_room`] made the room, the width CSS Sizing 3 §3.1.1
/// defines for it: the room offered, but at least its min-content and at most
/// its max-content width.
///
/// Taffy's flexbox algorithm reads the keyword as `max-content`. It sizes the
/// container that shrinks to fit by the flex base sizes of its items, which a
/// `max-width` of an item, a flex basis of `0` or the room does not bound.
fn with_fit_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
) -> LayoutInput {
    let AvailableSpace::Definite(room) = inputs.available_space.width else {
        return inputs;
    };
    if inputs.known_dimensions.width.is_some() || !has_fit_content_width(tree, usize::from(node_id))
    {
        return inputs;
    }
    let mut measure = |space: AvailableSpace| {
        let mut probe = inputs;
        probe.run_mode = RunMode::ComputeSize;
        probe.axis = RequestedAxis::Horizontal;
        probe.available_space.width = space;
        flex_layout_sized(tree, node_id, probe).size.width
    };
    // A container whose items fit the room is as wide as they are, whatever
    // they need at their least.
    let max_content = measure(AvailableSpace::MaxContent);
    let width = if max_content <= room {
        max_content
    } else {
        max_content.min(measure(AvailableSpace::MinContent).max(room))
    };
    inputs.known_dimensions.width = Some(width);
    inputs.known_dimensions_are_definite.width = true;
    inputs
}

/// Settle the width of a flex or grid container whose `width` is `min-content`
/// or `max-content` and that a `min-width` or `max-width` clamps, before its
/// lines or tracks are sized.
///
/// Taffy sizes the content of such a container at the keyword size and clamps
/// the box afterwards, so the items of the container break their lines at a
/// width the box does not keep, and its height follows that width. A
/// container that no limit clamps is sized by Taffy alone. `measure` is the
/// layout of the container, which gives the clamped width when asked for its
/// size.
fn with_clamped_keyword_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
    measure: fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutInput {
    let idx = usize::from(node_id);
    // A width measured as that of the content (`flex-basis: content`) is not
    // the width the box keeps; a height measured at its width is.
    if inputs.known_dimensions.width.is_some()
        || (inputs.sizing_mode == SizingMode::ContentSize
            && inputs.axis == RequestedAxis::Horizontal)
    {
        return inputs;
    }
    let space = match tree.physical_style(idx).size().width.expand() {
        ExpandedDimension::MinContent => AvailableSpace::MinContent,
        ExpandedDimension::MaxContent => AvailableSpace::MaxContent,
        _ => return inputs,
    };
    let clamped = {
        let style = tree.physical_style(idx);
        let zero = |size: ExpandedLengthPercentageAuto| {
            matches!(
                size,
                ExpandedLengthPercentageAuto::Auto | ExpandedLengthPercentageAuto::Length(0.0)
            )
        };
        !zero(style.min_size().width.expand())
            || !matches!(
                style.max_size().width.expand(),
                ExpandedLengthPercentageAuto::Auto
            )
    } || tree.nodes[idx].content_limits != content_limits::ContentLimits::NONE
        // Taffy reads the `min-content` width of a container that wraps its
        // items onto vertical lines as the sum of the lines.
        || (space == AvailableSpace::MinContent && content_limits::is_wrapping_column_flex(tree, idx));
    if clamped {
        let mut probe = inputs;
        probe.run_mode = RunMode::ComputeSize;
        probe.sizing_mode = SizingMode::InherentSize;
        probe.axis = RequestedAxis::Horizontal;
        probe.available_space.width = space;
        inputs.known_dimensions.width = Some(measure(tree, node_id, probe).size.width);
        inputs.known_dimensions_are_definite.width = true;
    }
    inputs
}

/// Lay out the grid container `node_id` with taffy's grid algorithm.
///
/// A container with a preferred aspect ratio and an `auto` height is given the
/// height [`with_ratio_height`] settles for it.
fn grid_layout(tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput) -> LayoutOutput {
    let idx = usize::from(node_id);
    let inputs = with_clamped_keyword_width(tree, node_id, inputs, grid_layout_sized);
    if grid_oof::right_to_left(tree, idx).is_none() {
        return laid_out_with_ratio_height(tree, node_id, inputs, grid_layout_sized);
    }
    grid_layout_sized(tree, node_id, inputs)
}

/// Give a flex or grid container with a preferred aspect ratio and an `auto`
/// height the height the ratio gives for its width, whether the width is the
/// one the container states or the one its parent imposes; taffy applies the
/// ratio to the former only. Unless the container scrolls or states a
/// `min-height`, its content raises that height up to the `max-height` it
/// states: the automatic minimum size of CSS Sizing 4 §5.1.
///
/// `sized` is the layout of the container once its height is settled; it
/// measures the content, and gives the width of a container whose width comes
/// from its content.
fn with_ratio_height(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
    sized: fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutInput {
    let idx = usize::from(node_id);
    let height_follows_width = inputs.known_dimensions.height.is_none()
        && !(inputs.run_mode == RunMode::ComputeSize && inputs.axis == RequestedAxis::Horizontal);
    if !height_follows_width {
        return inputs;
    }
    let style = block_flow::style_input::physical(tree, idx);
    // A measure that ignores the sizes of the box (`ContentSize`) sees an
    // `auto` height whatever the style says.
    let auto_height = inputs.sizing_mode == SizingMode::ContentSize || style.size.height.is_auto();
    if style.aspect_ratio.is_none() || !auto_height {
        return inputs;
    }
    let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
    let mut metrics = block_flow::Metrics::resolve(&style, false, content_box_ratio, inputs);
    if metrics.preferred.width.is_none() {
        // A width that comes from the content, as for an item of a flex row,
        // is settled first and the ratio then gives the height for it.
        let width = sized(
            tree,
            node_id,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                axis: RequestedAxis::Horizontal,
                ..inputs
            },
        )
        .size
        .width;
        inputs.known_dimensions.width = Some(width);
        inputs.known_dimensions_are_definite.width = true;
        metrics = block_flow::Metrics::resolve(&style, false, content_box_ratio, inputs);
    }
    let height = ratio_height(tree, node_id, inputs, &metrics, sized);
    inputs.known_dimensions.height = height;
    inputs.known_dimensions_are_definite.height |= height.is_some();
    inputs
}

/// Lay out a flex or grid container with `sized` once [`with_ratio_height`] has
/// settled its height.
///
/// When the width and height are then both settled, or the ratio gave the
/// height, it has nothing left to give the sizes. Taffy would still carry a
/// `max-width` through the ratio into a `max-height` and clamp the height back
/// to it, which undoes a height the content raised (CSS Sizing 4 §5.1):
/// `max-width: 50px` at 2/1 would cap a 100px-tall content at 25px. So the
/// container is laid out without the ratio.
fn laid_out_with_ratio_height(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    original: LayoutInput,
    sized: fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let inputs = with_ratio_height(tree, node_id, original, sized);
    let height_from_ratio =
        original.known_dimensions.height.is_none() && inputs.known_dimensions.height.is_some();
    let both_settled =
        inputs.known_dimensions.width.is_some() && inputs.known_dimensions.height.is_some();
    let drop_ratio = (height_from_ratio || both_settled)
        && block_flow::style_input::physical(tree, usize::from(node_id))
            .aspect_ratio
            .is_some();
    if !drop_ratio {
        return sized(tree, node_id, inputs);
    }
    without_container_ratio(tree, node_id, |tree| sized(tree, node_id, inputs))
}

/// Disable only the container's ratio, keeping its cache apart and preserving
/// descendant caches. Advancing the global override generation here would
/// repeat each nested ratio container's intrinsic work at every ancestor.
fn without_container_ratio<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let idx = usize::from(node_id);
    intrinsic_scopes::with_saved_probe_caches(tree, 1, |tree, saved| {
        let key = tree.nodes.box_key(idx).expect("live container box");
        saved.push((idx, tree.take_layout_cache(idx)));
        tree.push_local_overrides(vec![(key, OwnedPropertyOverride::aspect_ratio(None))]);
        layout(tree)
    })
}

/// The border-box height a flex or grid container takes from its aspect ratio
/// at the width in `metrics`, or `None` while the ratio has no width to relate
/// to.
///
/// When the automatic minimum size applies, the height of the content raises
/// it: that comes from laying the container out without the ratio at the
/// width the ratio relates to.
fn ratio_height(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    metrics: &block_flow::Metrics,
    sized: fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> Option<f32> {
    let width = metrics.preferred.width?;
    let ratio_height = metrics.preferred.height?;
    if !metrics.ratio_height {
        return Some(ratio_height);
    }
    let probe = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Vertical,
        known_dimensions: Size {
            width: Some(width),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: inputs.available_space.height,
        },
        ..inputs
    };
    let content = without_container_ratio(tree, node_id, |tree| sized(tree, node_id, probe))
        .size
        .height;
    Some(ratio_height.max(content.maybe_clamp(None, metrics.own_max.height)))
}

/// [`grid_layout`] once the height of a container with an aspect ratio is
/// settled.
fn grid_layout_sized(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let (size, inset, _) = tree.block_definite_size(idx, &inputs);
    let parent_size = Size {
        width: size.width.map(|width| (width - inset.width).max(0.0)),
        height: size.height.map(|height| (height - inset.height).max(0.0)),
    };
    let right_to_left = grid_oof::right_to_left(tree, idx);
    let mut output = content_limits::with_grid_item_content_limits(
        tree,
        node_id,
        inputs,
        parent_size,
        |tree, node_id, inputs| {
            // The items are still where the algorithm placed them, on the
            // axes it ran on, when the baseline is taken from one of them.
            let algorithm = |tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput| {
                let output = compute_grid_layout(tree, node_id, inputs);
                grid_baseline::with_first_baseline(
                    tree,
                    usize::from(node_id),
                    right_to_left.is_some(),
                    inputs.run_mode,
                    output,
                )
            };
            if right_to_left.is_some() {
                // The container runs its algorithm on the axes of its own
                // writing mode.
                tree.compute_on_own_axes(node_id, inputs, |tree, node_id, inputs| {
                    let inputs = vertical_block::with_atomic_fit_content(
                        tree,
                        node_id,
                        inputs,
                        compute_grid_layout,
                    );
                    algorithm(tree, node_id, inputs)
                })
            } else {
                algorithm(tree, node_id, inputs)
            }
        },
    );
    let in_flow_overflow = output.scrollable_overflow_rect;
    if inputs.run_mode == RunMode::PerformLayout {
        oof::layout_local_oof(tree, node_id, &mut output);
    }
    // Taffy reports the tracks only from a layout it performs.
    let grid = tree.context.grid_layout_info.remove(&idx);
    let axes = grid_oof::GridAxes::new(right_to_left, output.size.width);
    // Taffy aligns a grid item by its baseline along the columns as if it were
    // at `start`, and keeps the items that run against the block direction of
    // a vertical container at `start` along the rows, for the container to
    // align.
    let in_flow_overflow = if inputs.run_mode == RunMode::PerformLayout
        && let Some(grid) = &grid
        && let Some(reach) =
            inline_baseline::align_grid(tree, idx, grid, axes, &mut output.oof_candidates)
    {
        output.scrollable_overflow_rect = output.scrollable_overflow_rect.union(reach);
        in_flow_overflow.union(reach)
    } else {
        in_flow_overflow
    };
    let areas = grid_oof::GridAreas::of(grid.as_ref(), axes);
    if let Some(areas) = &areas {
        center_absolute_children_in_block_axis(tree, idx, &inputs, output.size, Some(areas));
    }
    // The grid's own padding percentages refer to the inline size of its
    // containing block.
    let percent_basis = match axes {
        Some(_) => vertical_block::containing_inline_size(tree, idx, &inputs),
        None => inputs.parent_size.width,
    };
    // Taffy aligns the boxes by the physical axes of `horizontal-tb`; a grid on
    // the axes of a vertical writing mode aligns them by its own.
    let vertical_area = output
        .oof_positioning_area
        .filter(|_| axes.is_some() && inputs.run_mode == RunMode::PerformLayout);
    if let Some(area) = vertical_area {
        vertical_oof::align_hoisted_children_in_grid(tree, idx, area, areas.as_ref());
    }
    align_auto_grid_abspos_children(tree, idx, inputs.run_mode, percent_basis, output.size);
    if let Some(area) = vertical_area {
        output.scrollable_overflow_rect =
            in_flow_overflow.union(vertical_oof::hoisted_overflow_rect(tree, idx, area));
    }
    output
}

/// The shares of the free space of its grid area that Grid static alignment
/// leaves before an absolutely positioned child.
///
/// `overflows` tells, per axis, whether the child's margin box is larger than
/// the content box it is aligned in: a safe alignment of an overflowing box
/// aligns as `start` (CSS Box Alignment 3 §4.4).
///
/// The shares are those of the physical x and y axes of a grid laid out on
/// the axes of `horizontal-tb`; see [`auto_grid_abspos_vertical_alignment`]
/// for one laid out on vertical axes.
// spec: https://www.w3.org/TR/css-align-3/#overflow-values
fn auto_grid_abspos_static_alignment(
    child: &LayoutStyleView<'_>,
    parent: &LayoutStyleView<'_>,
    overflows: Size<bool>,
) -> Option<Size<f32>> {
    let alignment = auto_grid_abspos_axis_alignment(child, parent, overflows)?;
    Some(Size {
        width: if alignment.inline_reversed {
            1.0 - alignment.inline
        } else {
            alignment.inline
        },
        height: alignment.block,
    })
}

/// [`auto_grid_abspos_static_alignment`] for a grid laid out on the axes of a
/// vertical writing mode, whose block axis runs from right to left when
/// `right_to_left`: the inline axis is the physical y axis, and the block axis
/// the physical x axis. `overflows` is on physical axes.
fn auto_grid_abspos_vertical_alignment(
    child: &LayoutStyleView<'_>,
    parent: &LayoutStyleView<'_>,
    overflows: Size<bool>,
    right_to_left: bool,
) -> Option<Size<f32>> {
    let alignment = auto_grid_abspos_axis_alignment(
        child,
        parent,
        Size {
            width: overflows.height,
            height: overflows.width,
        },
    )?;
    Some(Size {
        width: if right_to_left {
            1.0 - alignment.block
        } else {
            alignment.block
        },
        height: if alignment.inline_reversed {
            1.0 - alignment.inline
        } else {
            alignment.inline
        },
    })
}

/// How a grid aligns an out-of-flow child along its own axes.
struct AxisAlignment {
    /// The share of the free space before the box along the inline axis, from
    /// the inline-start edge.
    inline: f32,
    /// The share along the block axis, from the block-start edge.
    block: f32,
    /// Whether the inline-start edge is the physical end edge.
    inline_reversed: bool,
}

/// The static alignment of `child` in its grid `parent` on the axes of the
/// grid's writing mode, when all of its insets and grid-placement properties
/// are `auto`. `overflows` is (inline, block).
fn auto_grid_abspos_axis_alignment(
    child: &LayoutStyleView<'_>,
    parent: &LayoutStyleView<'_>,
    overflows: Size<bool>,
) -> Option<AxisAlignment> {
    use raikiri_layout_style::values::{AlignItemsKeyword, AlignmentSafety};

    let (child_direction, parent_direction) = (child.direction(), parent.direction());
    let inset = child.inset();
    if child.position() != Position::Absolute
        || !inset.top.is_auto()
        || !inset.right.is_auto()
        || !inset.bottom.is_auto()
        || !inset.left.is_auto()
        || child.has_grid_placement()
    {
        return None;
    }

    // `self-start` and `self-end` follow the box's own direction where the
    // axis is its inline axis (CSS Box Alignment 3 §5.2).
    let alignment_fraction = |self_alignment: Option<AlignSelf>,
                              parent_alignment: AlignItems,
                              inline: bool,
                              overflows: bool| {
        let alignment = self_alignment
            .unwrap_or(parent_alignment)
            .resolve_self_relative(child_direction, parent_direction, inline);
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
    Some(AxisAlignment {
        inline: alignment_fraction(
            child.bridge_justify_self(),
            parent.justify_items(),
            true,
            overflows.width,
        ),
        block: alignment_fraction(
            child.bridge_align_self(),
            parent.align_items(),
            false,
            overflows.height,
        ),
        inline_reversed: parent_direction == Direction::Rtl,
    })
}

/// Align auto-placed out-of-flow children to this grid's padding box.
///
/// Taffy's static-position fallback uses the grid's border edge. CSS Grid 2
/// §10.2 instead aligns the margin box within the area of the grid using the
/// child's `justify-self` and `align-self`, which fall back to the grid's
/// `justify-items` and `align-items`. Every line of the child is `auto` and
/// the grid is its containing block, so the area runs to the padding edges
/// of the grid, as in Chromium; a grid that is not the containing block
/// aligns the child within its content box (see
/// [`align_hoisted_grid_abspos_children`]).
fn align_auto_grid_abspos_children(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    run_mode: RunMode,
    percent_basis: Option<f32>,
    size: Size<f32>,
) {
    if run_mode != RunMode::PerformLayout {
        return;
    }

    let parent_style = tree.physical_style(idx);
    let border = parent_style.border().resolve_or_zero(percent_basis);

    let placed: Vec<(usize, BoxLayout)> = tree
        .context
        .taffy_hoisted_children
        .get(&idx)
        .into_iter()
        .flatten()
        .filter_map(|child| {
            let child = usize::from(*child);
            // This grid is the containing block of the children it hoists, so
            // they are aligned in its padding box.
            auto_grid_abspos_aligned_layout(
                tree,
                idx,
                child,
                size,
                border,
                Edges::ZERO,
                Point::ZERO,
            )
            .map(|layout| (child, layout))
        })
        .collect();

    for (child, layout) in placed {
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
}

/// Apply grid static alignment after an outer containing block has laid out
/// an out-of-flow grid child.
pub(super) fn align_hoisted_grid_abspos_children(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    children: impl Iterator<Item = NodeId>,
) {
    let owner = usize::from(owner);
    let owner_origin = tree.document.layout_origin_of(owner).unwrap_or(Point::ZERO);
    let placed: Vec<_> = children
        .filter_map(|candidate| {
            let child = usize::from(candidate);
            let grid = tree.taffy_parent_of(child)?;
            if grid == owner
                || !matches!(
                    tree.nodes[grid].display(),
                    DisplayValue::Grid | DisplayValue::InlineGrid
                )
            {
                return None;
            }
            let grid_layout = tree.nodes[grid].layout;
            let grid_origin = tree.document.layout_origin_of(grid).unwrap_or(Point::ZERO);
            let grid_to_owner = Point {
                x: grid_origin.x - owner_origin.x,
                y: grid_origin.y - owner_origin.y,
            };
            // An outer box is the containing block, so the grid aligns the child
            // in its content box.
            auto_grid_abspos_aligned_layout(
                tree,
                grid,
                child,
                grid_layout.size,
                grid_layout.border,
                grid_layout.padding,
                grid_to_owner,
            )
            .map(|layout| (child, layout))
        })
        .collect();

    for (child, layout) in placed {
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
}

/// The layout of the out-of-flow `child` of `grid`, aligned in the area that
/// lies `area_inset` inside the padding box of the grid, whose border box is
/// `size` large with `border` edges: the padding of the grid to align `child`
/// in its content box, as when `grid` is not the containing block of `child`,
/// or no inset to align it in its padding box, as when it is.
fn auto_grid_abspos_aligned_layout(
    tree: &LayoutSession<'_>,
    grid: usize,
    child: usize,
    size: Size<f32>,
    border: Edges<f32>,
    area_inset: Edges<f32>,
    grid_to_owner: Point<f32>,
) -> Option<BoxLayout> {
    if tree.nodes[child].is_display_none() || tree.taffy_parent_of(child) != Some(grid) {
        return None;
    }
    let parent_style = tree.physical_style(grid);
    let style = tree.physical_style(child);
    let mut layout = tree.nodes[child].layout;
    // With every inset `auto`, auto margins are zero (CSS 2.1 §10.3.7,
    // §10.6.4).
    let style_margin = style.margin();
    let margin = Edges {
        left: if style_margin.left.is_auto() {
            0.0
        } else {
            layout.margin.left
        },
        right: if style_margin.right.is_auto() {
            0.0
        } else {
            layout.margin.right
        },
        top: if style_margin.top.is_auto() {
            0.0
        } else {
            layout.margin.top
        },
        bottom: if style_margin.bottom.is_auto() {
            0.0
        } else {
            layout.margin.bottom
        },
    };
    let area_width =
        (size.width - border.left - border.right - area_inset.left - area_inset.right).max(0.0);
    let area_height =
        (size.height - border.top - border.bottom - area_inset.top - area_inset.bottom).max(0.0);
    let free_width = area_width - layout.size.width - margin.left - margin.right;
    let free_height = area_height - layout.size.height - margin.top - margin.bottom;
    let overflows = Size {
        width: free_width < 0.0,
        height: free_height < 0.0,
    };
    let alignment = match grid_oof::right_to_left(tree, grid) {
        Some(right_to_left) => {
            auto_grid_abspos_vertical_alignment(&style, &parent_style, overflows, right_to_left)?
        }
        None => auto_grid_abspos_static_alignment(&style, &parent_style, overflows)?,
    };
    layout.location.x = grid_to_owner.x
        + border.left
        + area_inset.left
        + margin.left
        + alignment.width * free_width;
    layout.location.y =
        grid_to_owner.y + border.top + area_inset.top + margin.top + alignment.height * free_height;
    Some(layout)
}

impl LayoutSession<'_> {
    /// Whether the bridged writing mode of `idx` has a vertical inline axis.
    fn has_vertical_writing_mode(&self, idx: usize) -> bool {
        self.nodes[idx].used_writing_mode.is_vertical()
    }

    /// The border-box width of block container `idx` that taffy's block
    /// algorithm derives from `inputs` before it lists the container's
    /// children (the definite size the container is given or styled with,
    /// `None` when there is none yet), and the padding and border that
    /// separate it from the content box.
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
    /// children, the padding and border that separate it from the content
    /// box along each axis, and the border-box height its own `height` and
    /// `min-height` give it.
    ///
    /// Mirrors [`taffy::compute_block_layout`] and its `compute_inner`.
    fn block_definite_size(
        &self,
        idx: usize,
        inputs: &LayoutInput,
    ) -> (Size<Option<f32>>, Size<f32>, Option<f32>) {
        use crate::layout::protocol::SizingMode;

        let style = self.physical_style(idx);
        let parent_size = inputs.parent_size;
        let padding = style.padding().resolve_or_zero(parent_size.width);
        let border = style.border().resolve_or_zero(parent_size.width);
        let padding_border_size = (padding + border).sum_axes();
        let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            padding_border_size
        } else {
            Size::ZERO
        };
        let aspect_ratio = style.aspect_ratio();
        let min_size = style
            .min_size()
            .maybe_resolve(parent_size)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let max_size = style
            .max_size()
            .maybe_resolve(parent_size)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let style_size = style
            .size()
            .maybe_resolve(parent_size)
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
        (
            Size {
                width: known.width.or(derived.width),
                height: known.height.or(derived.height),
            },
            Size {
                width: padding.horizontal_axis_sum() + border.horizontal_axis_sum(),
                height: padding.vertical_axis_sum() + border.vertical_axis_sum(),
            },
            style_size.height.maybe_max(min_size.height),
        )
    }

    /// Whether an in-flow child of block container `idx` has a top or bottom
    /// padding or border that depends on its percentage basis.
    fn has_percent_vertical_edge_child(&self, idx: usize) -> bool {
        TaffyChildIter::new_for_session(self, idx).any(|child| {
            if self.nodes[child].layout_position() == Position::Absolute {
                return false;
            }
            let style = self.physical_style(child);
            let (padding, border) = (style.padding(), style.border());
            [padding.top, padding.bottom, border.top, border.bottom]
                .into_iter()
                .any(edge_depends_on_basis)
        })
    }
}

impl LayoutSession<'_> {
    /// Record what the grid algorithm reports about the grid container, for
    /// pagination and the out-of-flow positioning pass.
    pub(crate) fn set_detailed_grid_info(&mut self, node_id: NodeId, info: GridInfo) {
        let parent = usize::from(node_id);
        let children = TaffyChildIter::new_for_session(self, parent)
            .filter(|&child| {
                self.nodes[child].layout_display() != Display::None
                    && self.nodes[child].layout_position() != Position::Absolute
            })
            .collect::<Vec<_>>();

        // The grid algorithm lists the items in flow in source order. Pair
        // those resolved rows with the same child projection that it received,
        // so pagination can order by actual placement rather than trying to
        // reconstruct auto-placement or relative offsets.
        debug_assert_eq!(children.len(), info.items().len());
        self.nodes[parent].grid_item_row_starts = children
            .iter()
            .zip(info.items())
            .map(|(&child, item)| (child, item.row_start))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let rows = RowSizing::of(&self.physical_style(parent), info.row_counts());
        // The rows run along the block axis: the physical height, or the
        // physical width in a vertical writing mode.
        let vertical = grid_oof::right_to_left(self, parent).is_some();
        self.nodes[parent].grid_items_without_baseline = children
            .iter()
            .zip(info.items())
            .filter(|&(&child, item)| {
                let spanned = usize::from(item.row_start.saturating_sub(1))
                    ..usize::from(item.row_end.saturating_sub(1));
                let size = self.physical_style(child).size();
                (if vertical { size.width } else { size.height }).uses_percentage()
                    && rows.any_sized_by_content_or_free_space(spanned)
            })
            .map(|(&child, _)| child)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // The areas are on the layout axes of the grid, which are the physical
        // ones swapped in a vertical writing mode.
        self.nodes[parent].grid_item_areas = children
            .iter()
            .enumerate()
            .filter_map(|(index, &child)| {
                let area = info.item_area_size(index)?;
                Some((child, if vertical { area.transpose() } else { area }))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.nodes[parent].grid_item_column_starts = children
            .into_iter()
            .zip(info.items())
            .map(|(child, item)| (child, item.column_start))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.nodes[parent].grid_column_count = info.column_count();
        self.context.grid_layout_info.insert(parent, info);
    }
}

impl LayoutSession<'_> {
    /// The children of the node that take part in layout.
    pub(crate) fn child_ids(&self, node_id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        TaffyChildIter::new_for_session(self, usize::from(node_id)).map(NodeId::from)
    }

    /// The number of children of the node that take part in layout.
    ///
    /// A container with many children also records the list, which the
    /// following calls of [`Self::get_child_id`] for it read from.
    pub(crate) fn child_count(&self, node_id: NodeId) -> usize {
        let parent = usize::from(node_id);
        let count = TaffyChildIter::new_for_session(self, parent).count();
        let children = (count >= WALKED_CHILDREN_MIN).then(|| {
            TaffyChildIter::new_for_session(self, parent)
                .map(NodeId::from)
                .collect()
        });
        if let Some(mut state) = self.node_state(parent) {
            state.taffy_child_walks = children;
        }
        count
    }

    /// The `index`th child of the node that takes part in layout.
    pub(crate) fn get_child_id(&self, node_id: NodeId, index: usize) -> NodeId {
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
        NodeId::from(
            TaffyChildIter::new_for_session(self, parent)
                .nth(index)
                .expect("get_child_id: index out of range"),
        )
    }
}
