//! The width a box in a vertical writing mode takes from its aspect ratio
//! while it shrinks to fit.
//!
//! The physical width of such a box is its block size and its height is its
//! inline size. A box whose width is `auto` and whose parent does not fix that
//! width (an inline-block, a float, an absolutely positioned box, or an
//! inline flex or grid container) settles its inline size first, from the
//! height it states or from its content, and the preferred aspect ratio then
//! gives the block size for it (CSS Sizing 4 §5.1). Its content raises that
//! width unless the box scrolls or states a `min-width`, as the automatic
//! minimum size of a ratio-dependent axis does.
//!
//! The block layouts below take the width they are given, so the box is
//! measured without its ratio and laid out again at the width the ratio gives.

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::node::{FormattingContext, NodeFlags};
use raikiri_layout_geometry::{MaybeMath, Size};
use raikiri_layout_style::values::{
    Display, LengthPercentage, MaybeResolve, Position, ResolveOrZero,
};

use super::block_flow::{self, Metrics, context::BlockContext};

/// Whether `idx` is a vertical box that takes its width from its aspect ratio
/// under `inputs`: it states the ratio and an `auto` width, and nothing fixed
/// the width it is laid out at.
fn takes_width_from_ratio(tree: &LayoutSession<'_>, idx: usize, inputs: &LayoutInput) -> bool {
    let node = &tree.nodes[idx];
    if inputs.known_dimensions.width.is_some()
        || !node.used_writing_mode.is_vertical()
        || node.flags.contains(NodeFlags::IS_ROOT_ELEMENT)
        || super::leaf_intrinsic_size(node).is_some()
        || matches!(
            node.formatting_context,
            FormattingContext::None | FormattingContext::Table
        )
    {
        return false;
    }
    let style = tree.physical_style(idx);
    if style.is_replaced() || style.stated_aspect_ratio().is_none() || !style.size().width.is_auto()
    {
        return false;
    }
    // The flex and grid algorithms size their items along the ratio
    // themselves; an absolutely positioned child is no item of theirs.
    node.layout_position() == Position::Absolute
        || !tree.taffy_parent_of(idx).is_some_and(|parent| {
            matches!(
                tree.nodes[parent].layout_display(),
                Display::Flex | Display::Grid
            )
        })
}

/// `inputs` with the border-box `width` and `height` fixed as definite.
fn fixed_inputs(inputs: LayoutInput, width: f32, height: f32) -> LayoutInput {
    LayoutInput {
        sizing_mode: SizingMode::ContentSize,
        known_dimensions: Size {
            width: Some(width),
            height: Some(height),
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        },
        ..inputs
    }
}

/// The keys under which the width and the height settled for `inputs` are
/// kept in the box's cache. The run mode decides nothing about them, and the
/// one no measure asks with keeps them apart from the widths measured there.
fn memo_keys(inputs: LayoutInput) -> [LayoutInput; 2] {
    let key = |axis| LayoutInput {
        run_mode: RunMode::PerformHiddenLayout,
        axis,
        ..inputs
    };
    [key(RequestedAxis::Horizontal), key(RequestedAxis::Vertical)]
}

/// Lay out `node_id` at the width its aspect ratio gives it, or return `None`
/// when that width is not the ratio's to give (see [`takes_width_from_ratio`]).
///
/// A box in a block formatting context it shares with its parent is not one
/// that shrinks to fit, and is left to that context.
pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> Option<LayoutOutput> {
    let idx = usize::from(node_id);
    if block_ctx.is_some() || !takes_width_from_ratio(tree, idx, &inputs) {
        return None;
    }
    // The probes below lay the box out again, which takes the flag that a
    // member of a `text-group-align` group carries; the layout that counts
    // gets it back.
    let group_child_uncached = tree.context.group_child_uncached;
    // Each probe lays the content out, and a parent asks for the box at the
    // same inputs again and again, so the size settled for them is kept in
    // the box's cache: a chain of such boxes nested in each other would
    // otherwise repeat the probes of every level for each probe of the one
    // above it.
    let view = raikiri_layout_style::ViewContext {
        layout_wm: tree.nodes[idx].used_writing_mode,
        ..raikiri_layout_style::ViewContext::default()
    };
    let [width_key, height_key] = memo_keys(inputs);
    let memo = tree.with_cache_frame(idx, view, |tree| {
        tree.cached_width(node_id, &width_key)
            .zip(tree.cached_width(node_id, &height_key))
    });
    let (width, height) = match memo {
        Some(size) => size,
        None => {
            let size = ratio_size(tree, node_id, inputs)?;
            tree.with_cache_frame(idx, view, |tree| {
                if tree.caches_node(node_id) {
                    tree.cache_width(node_id, &width_key, size.0);
                    tree.cache_width(node_id, &height_key, size.1);
                }
            });
            size
        }
    };
    tree.context.group_child_uncached = group_child_uncached;
    Some(tree.compute_child_layout_with_block_ctx_inner(
        node_id,
        fixed_inputs(inputs, width, height),
        None,
    ))
}

/// The border-box width and height of `node_id` under `inputs`: the height it
/// states, or the one its content takes held to the bounds its width carries
/// over through the ratio, and the width the ratio gives for that height,
/// raised to the block size of the content.
fn ratio_size(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> Option<(f32, f32)> {
    let idx = usize::from(node_id);
    let mut style = block_flow::style_input::physical(tree, idx);
    // Padding follows the inline size of the containing block, the physical
    // height of a vertical one.
    let basis = super::vertical_block::containing_inline_size(tree, idx, &inputs);
    style.padding = style
        .padding
        .resolve_or_zero(basis)
        .map(LengthPercentage::length);
    let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
    let unbounded = Metrics::resolve(&style, false, content_box_ratio, inputs);
    // A height the parent fixed or the box states settles the inline size.
    let states_height = inputs.sizing_mode == SizingMode::InherentSize
        && MaybeResolve::maybe_resolve(style.size.height, inputs.parent_size.height).is_some();
    let stated_height = inputs
        .known_dimensions
        .height
        .or_else(|| unbounded.preferred.height.filter(|_| states_height));
    // A flex or grid container takes its width from a height it states along
    // its ratio already, raised to the content its algorithm lays out.
    if stated_height.is_some()
        && matches!(
            tree.nodes[idx].formatting_context,
            FormattingContext::Flex | FormattingContext::Grid
        )
    {
        return None;
    }
    // Without a stated height, the box laid out without its ratio gives the
    // inline size its content takes, held to the bounds that the width carries
    // over to the height through the ratio, and the block size of the content.
    let content = match stated_height {
        Some(_) => None,
        None => Some(content_size(tree, node_id, inputs, false)),
    };
    let height = stated_height
        .or_else(|| {
            content.map(|size| {
                size.height
                    .maybe_clamp(unbounded.min.height, unbounded.max.height)
            })
        })
        .unwrap_or(0.0)
        .max(0.0);
    let sized = Metrics::resolve(
        &style,
        false,
        content_box_ratio,
        LayoutInput {
            known_dimensions: Size {
                width: None,
                height: Some(height),
            },
            sizing_mode: SizingMode::InherentSize,
            ..inputs
        },
    );
    let ratio_width = sized.preferred.width?;
    if !sized.ratio_width {
        return Some((ratio_width, height));
    }
    let floor = match content {
        Some(content) => content.width,
        // A size-contained box lays out no content to reach past the width:
        // the block size `contain-intrinsic-size` names stands for it, which
        // the box measured without its ratio takes. Chromium sizes
        // `contain: size; contain-intrinsic-size: 200px 200px; aspect-ratio:
        // 1; height: 100px` at 200px by 100px.
        None if tree.nodes[idx].size_containment.width => {
            content_size(tree, node_id, inputs, false).width
        }
        // With a stated height the width of the box is definite, so its
        // content is measured at that width for the percentages inside it.
        // What reaches past it there may be out-of-flow or relatively
        // positioned boxes, which no content size counts, so the block size
        // the content takes without the ratio caps it.
        None => {
            let extent = content_extent_at(tree, node_id, inputs, ratio_width, height, &sized);
            if extent > ratio_width {
                extent.min(content_size(tree, node_id, inputs, true).width)
            } else {
                extent
            }
        }
    };
    Some((
        ratio_width.max(floor).maybe_min(sized.own_max.width),
        height,
    ))
}

/// The size of `node_id` laid out under `inputs` without its ratio.
///
/// With a stated height, the percentage widths inside the box read `auto` in
/// this measure, which is not how the box is laid out after it, and the boxes
/// inside keep no answers from it: the cache does not key on the basis.
fn content_size(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    stated_height: bool,
) -> Size<f32> {
    let probe = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Both,
        ..inputs
    };
    let measure = |tree: &mut LayoutSession<'_>| {
        super::without_container_ratio(tree, node_id, |tree| {
            tree.compute_child_layout_with_block_ctx_inner(node_id, probe, None)
        })
        .size
    };
    if !stated_height {
        return measure(tree);
    }
    let mut subtree = Vec::new();
    let mut stack: Vec<NodeId> = tree.child_ids(node_id).collect();
    while let Some(child) = stack.pop() {
        subtree.push(usize::from(child));
        stack.extend(tree.child_ids(child));
    }
    super::intrinsic_scopes::with_saved_probe_caches(tree, subtree.len(), |tree, saved| {
        for &child in &subtree {
            saved.push((child, tree.take_layout_cache(child)));
        }
        measure(tree)
    })
}

/// The block size the content of `node_id` reaches when the box is laid out at
/// the border-box `width` its ratio gives for the `height` it states.
///
/// That width is definite, so the percentage widths inside the box refer to
/// it, and the content measured that way is what the automatic minimum keeps
/// the width from undercutting.
fn content_extent_at(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    width: f32,
    height: f32,
    metrics: &Metrics,
) -> f32 {
    let probe = LayoutInput {
        run_mode: RunMode::PerformLayout,
        ..fixed_inputs(inputs, width, height)
    };
    let output = super::without_container_ratio(tree, node_id, |tree| {
        tree.compute_child_layout_with_block_ctx_inner(node_id, probe, None)
    });
    // The probe placed the children at the ratio's width. A layout of the box
    // kept from before would leave them there, so the box is laid out afresh.
    drop(tree.take_layout_cache(usize::from(node_id)));
    let overflow = output.scrollable_overflow_rect;
    // The overflow is measured within the padding box, from the side the
    // block axis starts at; its extent is the same from either side.
    (overflow.right - overflow.left) + metrics.border.left + metrics.border.right
}
