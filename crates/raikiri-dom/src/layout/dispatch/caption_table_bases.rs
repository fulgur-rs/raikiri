//! The flex base size of a table item with a caption in a column flex
//! container.
//!
//! A table states the height of its grid box: its captions come on top of it,
//! in the table wrapper box (CSS Tables 3 §3.1). A definite `flex-basis`, or
//! the `height` that stands in for an `auto` one, is a height the table states
//! in the same way, so the flex base size of the item, which is the size of
//! the whole wrapper box, is that height with the height of the captions added
//! to it. Chromium flexes the item from there: a container that is no taller
//! than the stated height and the captions shrinks the item, and a container
//! whose height follows its items grows by the captions. The layout library
//! knows nothing of the captions, so the flex basis of such an item is set to
//! the wrapper's height while the container is laid out, and the height the
//! container gives the item is then the wrapper's (see
//! [`LayoutContext::caption_inclusive_bases`]).
//!
//! spec: <https://www.w3.org/TR/css-flexbox-1/#algo-main-item>
//!
//! [`LayoutContext::caption_inclusive_bases`]: crate::layout::context::LayoutContext::caption_inclusive_bases

use super::column_main_limits::scoped;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::node::FormattingContext;
use raikiri_layout_geometry::{Line, Size};
use raikiri_layout_style::values::{
    BoxGenerationMode, BoxSizing, ExpandedDimension, Position, ResolveOrZero,
};
use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

/// Run `layout`, a layout of the flex container `node_id` for `inputs`, with
/// the flex basis of each table item that has a caption in flow made the
/// height of its whole wrapper box, when the container is a column of a
/// definite inner width.
pub(super) fn with_caption_inclusive_bases<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let idx = usize::from(node_id);
    let container = tree.physical_style(idx);
    if container.flex_direction_is_row()
        || tree.has_vertical_writing_mode(idx)
        || super::block_right_to_left(tree, idx).is_some()
    {
        return layout(tree);
    }
    let (size, inset, _) = tree.block_definite_size(idx, &inputs);
    let Some(inner_width) = size.width.map(|width| (width - inset.width).max(0.0)) else {
        return layout(tree);
    };
    // A height the container is laid out at that its content decides is no
    // basis for a percentage of the items.
    let inner_height = size
        .height
        .filter(|_| {
            inputs.known_dimensions.height.is_none() || inputs.known_dimensions_are_definite.height
        })
        .map(|height| (height - inset.height).max(0.0));
    // The items of a column that wraps them are measured for their flex base
    // sizes before the lines are known, so a table there is measured at the
    // width it takes for itself, as Chromium does, though the line stretches
    // it afterwards.
    let single_line = !container.flex_wraps();
    let children: Vec<usize> = tree.child_ids(node_id).map(usize::from).collect();
    let mut candidates = Vec::new();
    for child in children {
        if tree.nodes[child].formatting_context != FormattingContext::Table {
            continue;
        }
        let style = tree.physical_style(child);
        if style.box_generation_mode() == BoxGenerationMode::None
            || style.position() == Position::Absolute
        {
            continue;
        }
        let stated = match style.flex_basis().expand() {
            ExpandedDimension::Auto => {
                crate::layout::resolve_dimension(style.size().height, inner_height)
            }
            ExpandedDimension::Length(_)
            | ExpandedDimension::Percent(_)
            | ExpandedDimension::Calc(_) => {
                crate::layout::resolve_dimension(style.flex_basis(), inner_height)
            }
            _ => None,
        };
        let Some(stated) = stated.filter(|stated| stated.is_finite()) else {
            continue;
        };
        // The table is as wide as it lays itself out in the room its margins
        // leave, or at that room when the container stretches it; a table
        // is never narrower than its columns and captions need.
        let margin = style.margin();
        let side = |length| {
            crate::layout::resolve_length_percentage_auto(length, Some(inner_width)).unwrap_or(0.0)
        };
        let room = (inner_width - side(margin.left) - side(margin.right)).max(0.0);
        let stretched = single_line
            && style.size().width.is_auto()
            && super::content_limits::alignment_stretches(&container, &style, false);
        // A stretched item is as wide as the room within its `min-width` and
        // `max-width`, which hold its border box or its content box.
        let stretched_width = stretched.then(|| {
            let edges = if style.box_sizing() == BoxSizing::ContentBox {
                let padding = style.padding().resolve_or_zero(Some(inner_width));
                let border = style.border().resolve_or_zero(Some(inner_width));
                padding.left + padding.right + border.left + border.right
            } else {
                0.0
            };
            let bound = |length| {
                crate::layout::resolve_length_percentage_auto(length, Some(inner_width))
                    .map(|bound| bound + edges)
            };
            let width = bound(style.max_size().width).map_or(room, |max| room.min(max));
            bound(style.min_size().width).map_or(width, |min| width.max(min))
        });
        candidates.push((
            child,
            stated,
            width_probe(inner_width, room, stretched_width),
        ));
    }
    let mut entries = Vec::new();
    for (child, stated, probe) in candidates {
        let width = tree
            .compute_child_layout(NodeId::from(child), probe)
            .size
            .width;
        let Some(captions) =
            crate::layout::table::in_flow_caption_height(tree, child, width, inner_height)
        else {
            continue;
        };
        entries.push((
            child,
            O::length(P::FlexBasis, L::Px(stated + captions), true)
                .expect("caption-inclusive flex basis"),
        ));
    }
    if entries.is_empty() {
        return layout(tree);
    }
    let depth = tree.context.caption_inclusive_bases.len();
    tree.context
        .caption_inclusive_bases
        .extend(entries.iter().map(|(child, _)| *child));
    let result = scoped(tree, idx, entries, inputs.run_mode, layout);
    tree.context.caption_inclusive_bases.truncate(depth);
    result
}

/// The inputs that ask a table item for its width in a container whose inner
/// width is `inner_width`, where its margins leave it `room`: at
/// `stretched_width` when the container stretches it, and within the room
/// otherwise.
fn width_probe(inner_width: f32, room: f32, stretched_width: Option<f32>) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: stretched_width,
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: stretched_width.is_some(),
            height: false,
        },
        parent_size: Size {
            width: Some(inner_width),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(room),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}
