//! The width a grid item with a preferred aspect ratio takes from the height
//! of its content.
//!
//! A grid item that states a ratio and neither a width nor a height, and is
//! stretched to the height of its area, contributes to its column, and is aligned at, the wider of the width of its
//! content and the width the ratio gives for the height its content takes at
//! that width, held to the bounds of the box. Chromium sizes `aspect-ratio: 1`
//! around a 200px tall child 200px wide in a `min-content`, `max-content` or
//! `auto` column, where the same box outside a grid (an inline-block, a float,
//! an absolutely positioned box or a flex item) shrinks to the width of its
//! content.
//!
//! The layouts below take the width they are given, so the box is measured
//! at the width of its content first and laid out again at the width the
//! ratio gives when that is the wider one.

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode,
};
use crate::node::FormattingContext;
use raikiri_layout_geometry::{MaybeMath, Size};
use raikiri_layout_style::values::{
    AlignItemsKeyword, AlignSelf, Display, ExpandedDimension, MaybeResolve, Position,
};

use super::block_flow::{self, Metrics, context::BlockContext};

/// Whether `idx` is a grid item whose width the height of its content may
/// raise through its aspect ratio under `inputs`.
fn takes_width_from_content_height(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
) -> bool {
    let node = &tree.nodes[idx];
    // A content-size request asks for the box without its preferred sizes,
    // yet the width the height of its content gives through the ratio is a
    // size of that content: such a request takes it too.
    if inputs.known_dimensions.width.is_some()
        || inputs.known_dimensions.height.is_some()
        || tree.context.grid_ratio_content.contains(&idx)
        || !matches!(
            node.formatting_context,
            FormattingContext::Block
                | FormattingContext::Inline
                | FormattingContext::Flex
                | FormattingContext::Grid
        )
        || node.size_containment.width
        || node.size_containment.height
        || node.layout_position() == Position::Absolute
        || node.used_writing_mode.is_vertical()
        || tree.container_is_vertical(idx)
        || !tree
            .taffy_parent_of(idx)
            .is_some_and(|parent| tree.nodes[parent].layout_display() == Display::Grid)
    {
        return false;
    }
    let style = tree.physical_style(idx);
    let size = style.size();
    let overflow = style.overflow();
    let margin = style.margin();
    // Only an item stretched to the height of its area takes the height of
    // its content over to its width: Chromium keeps the width of the content
    // for one aligned to `start`, `center` or a baseline, or kept off the
    // stretch by an `auto` margin.
    let align = style.bridge_align_self().unwrap_or_else(|| {
        tree.taffy_parent_of(idx)
            .map_or(AlignSelf::NORMAL, |parent| {
                tree.physical_style(parent).align_items()
            })
    });
    matches!(
        align.keyword,
        AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch
    ) && !margin.top.is_auto()
        && !margin.bottom.is_auto()
        && style.stated_aspect_ratio().is_some()
        && !style.is_replaced()
        && !overflow.x.is_scroll_container()
        && !overflow.y.is_scroll_container()
        && size.height.is_auto()
        // A percentage the area resolves counts as `auto` while the column
        // is sized: Chromium sizes `width: 50%` in a `max-content` column at
        // half the width the content height gives the column.
        && !matches!(size.width.expand(), ExpandedDimension::Stretch)
        && MaybeResolve::maybe_resolve(size.width, inputs.parent_size.width).is_none()
}

/// Lay out `node_id` at the width the height of its content gives it through
/// its ratio, or return `None` when that width is not the one it takes (see
/// [`takes_width_from_content_height`]) or is no wider than its content.
pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> Option<LayoutOutput> {
    let idx = usize::from(node_id);
    if block_ctx.is_some() || !takes_width_from_content_height(tree, idx, &inputs) {
        return None;
    }
    // The probes below lay the box out again, which takes the flag that a
    // member of a `text-group-align` group carries; the layout that counts
    // gets it back.
    let group_child_uncached = tree.context.group_child_uncached;
    // A parent asks for the box at the same inputs again and again, so the
    // width settled for them is kept in the box's cache under a run mode no
    // layout asks with; a negative width marks one the ratio does not raise.
    let view = raikiri_layout_style::ViewContext {
        layout_wm: tree.nodes[idx].used_writing_mode,
        ..raikiri_layout_style::ViewContext::default()
    };
    let memo_key = LayoutInput {
        run_mode: RunMode::PerformHiddenLayout,
        axis: RequestedAxis::Horizontal,
        ..inputs
    };
    let width = match tree.with_cache_frame(idx, view, |tree| tree.cached_width(node_id, &memo_key))
    {
        Some(width) => width,
        None => {
            // The probes measure the box without what the ratio gives it, at
            // the very inputs a parent may ask with; the box keeps none of
            // their answers.
            let width = super::intrinsic_scopes::with_saved_probe_caches(tree, 1, |tree, saved| {
                saved.push((idx, tree.take_layout_cache(idx)));
                raised_width(tree, node_id, inputs)
            })
            .unwrap_or(-1.0);
            tree.with_cache_frame(idx, view, |tree| {
                if tree.caches_node(node_id) {
                    tree.cache_width(node_id, &memo_key, width);
                }
            });
            width
        }
    };
    tree.context.group_child_uncached = group_child_uncached;
    if width < 0.0 {
        return None;
    }
    Some(tree.compute_child_layout_with_block_ctx_inner(
        node_id,
        LayoutInput {
            known_dimensions: Size {
                width: Some(width),
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: true,
                ..inputs.known_dimensions_are_definite
            },
            available_space: Size {
                width: AvailableSpace::Definite(width),
                ..inputs.available_space
            },
            ..inputs
        },
        None,
    ))
}

/// The width the ratio raises `node_id` to under `inputs`, or `None` where
/// the width of its content is the wider one.
fn raised_width(tree: &mut LayoutSession<'_>, node_id: NodeId, inputs: LayoutInput) -> Option<f32> {
    match inputs.available_space.width {
        // An item aligned within its area shrinks to fit the room between
        // its two contributions.
        AvailableSpace::Definite(room) => {
            let (minimum, min_raised) =
                contribution(tree, node_id, inputs, AvailableSpace::MinContent);
            let (maximum, max_raised) =
                contribution(tree, node_id, inputs, AvailableSpace::MaxContent);
            (min_raised || max_raised).then(|| maximum.min(room).max(minimum))
        }
        space => match contribution(tree, node_id, inputs, space) {
            (width, true) => Some(width),
            (_, false) => None,
        },
    }
}

/// The border-box width `node_id` contributes under `space`: the width of its
/// content, or the width the ratio gives for the height of the content at
/// that width where that is wider, and whether the ratio raised it.
fn contribution(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    space: AvailableSpace,
) -> (f32, bool) {
    let idx = usize::from(node_id);
    let probe = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Horizontal,
        available_space: Size {
            width: space,
            ..inputs.available_space
        },
        ..inputs
    };
    tree.context.grid_ratio_content.push(idx);
    let content_width = tree
        .compute_child_layout_with_block_ctx_inner(node_id, probe, None)
        .size
        .width;
    tree.context.grid_ratio_content.pop();
    let content_height = tree
        .compute_child_layout_with_block_ctx_inner(
            node_id,
            LayoutInput {
                axis: RequestedAxis::Vertical,
                known_dimensions: Size {
                    width: Some(content_width),
                    height: None,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(content_width),
                    ..inputs.available_space
                },
                ..probe
            },
            None,
        )
        .size
        .height;
    let style = block_flow::style_input::physical(tree, idx);
    let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
    let sized = Metrics::resolve(
        &style,
        false,
        content_box_ratio,
        LayoutInput {
            known_dimensions: Size {
                width: None,
                height: Some(content_height),
            },
            ..inputs
        },
    );
    let across = sized
        .preferred
        .width
        .map(|width| width.maybe_clamp(sized.min.width, sized.max.width));
    match across {
        Some(width) if width > content_width => (width, true),
        _ => (content_width, false),
    }
}
