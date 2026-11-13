mod authored;
mod break_flow;
mod constrained_chain;
mod lineless;
mod paged_spanning;
mod paragraph_group;
pub(super) mod paragraph_spanners;
pub(super) mod spanning;

use authored::AuthoredBox;
use raikiri_layout_geometry::Edges;

use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::style_views::ReadRole;
use crate::node::FormattingContext;
use raikiri_layout_geometry::{BoxLayout, Line, Point, Size};
use raikiri_layout_style::LayoutStyleView;
use raikiri_layout_style::values::{
    AlignItems, BoxSizing, Clear, Contain, Dimension, Direction, Display, ExpandedDimension,
    ExpandedLengthPercentage, Float, LengthPercentage, LengthPercentageAuto, MaybeResolve,
    Overflow, Position, ResolveOrZero,
};

pub(super) use paged_spanning::{
    FragmentEdits, paginate_spanning_columns, release_unspanned_groups,
};

/// The style `idx` lays out with: its computed values with the overrides the
/// layout passes have applied.
fn style_of<'a>(tree: &'a LayoutSession<'_>, idx: usize) -> LayoutStyleView<'a> {
    tree.query_style(idx, ReadRole::CoreContainer)
}

/// The node or generated box that `idx` names.
fn layout_node(tree: &Document, idx: usize) -> &crate::node::Node {
    tree.nodes
        .layout_node(idx)
        .expect("a laid-out box has a node")
}

/// Whether `idx` is a float.
fn is_floated(tree: &LayoutSession<'_>, idx: usize) -> bool {
    style_of(tree, idx).float().is_floated()
}

/// How far block flow moved a box right for its `position: relative` insets
/// when the container's content box is `width` wide. A sticky box is moved
/// alike, but its offset is not one that a painter applies again, so it has
/// none here. This is the left-to-right offset that block flow gives a child.
fn relative_inline_offset(style: &LayoutStyleView<'_>, width: f32) -> f32 {
    if style.position() != Position::Relative {
        return 0.0;
    }
    let inset = style.inset();
    let resolve = |value: LengthPercentageAuto| value.maybe_resolve(Some(width));
    resolve(inset.left)
        .or(resolve(inset.right).map(|right| -right))
        .unwrap_or(0.0)
}

/// Give a multicol container whose `width` is `min-content`, `max-content` or
/// `fit-content` the border-box width the keyword makes, which its columns
/// are sized from; without it they would be sized from the room the container
/// is offered. The block algorithm measures the container the same way when
/// it gets no width.
fn with_keyword_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
) -> LayoutInput {
    use raikiri_layout_style::values::ExpandedDimension as Keyword;
    if inputs.known_dimensions.width.is_some() {
        return inputs;
    }
    let space = match style_of(tree, usize::from(node_id)).size().width.expand() {
        Keyword::MinContent => AvailableSpace::MinContent,
        Keyword::MaxContent => AvailableSpace::MaxContent,
        Keyword::FitContent
        | Keyword::FitContentPx(_)
        | Keyword::FitContentPercent(_)
        | Keyword::FitContentCalc(_) => inputs.available_space.width,
        _ => return inputs,
    };
    let mut measure = |space: AvailableSpace| {
        let mut probe = inputs;
        probe.run_mode = RunMode::ComputeSize;
        probe.axis = RequestedAxis::Horizontal;
        probe.available_space.width = space;
        crate::layout::dispatch::block_layout(tree, node_id, probe, None)
            .size
            .width
    };
    let width = match space {
        AvailableSpace::Definite(room) => {
            let max_content = measure(AvailableSpace::MaxContent);
            max_content
                .min(room)
                .max(measure(AvailableSpace::MinContent))
        }
        keyword => measure(keyword),
    };
    inputs.known_dimensions.width = Some(width);
    inputs.known_dimensions_are_definite.width = true;
    inputs
}

/// Dispatch a multicolumn container through the nested fragmentation seam.
///
/// Taffy still resolves the ordinary box and intrinsic sizes, while this
/// seam owns child placement, fragmentainer breaks, and fragment-tree
/// records. Cases that no column strategy supports keep the ordinary block
/// layout.
// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
pub(crate) fn compute_multicol_layout(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let index = usize::from(node_id);
    if tree.fragment_tree.limit_exceeded {
        return crate::layout::dispatch::block_layout(tree, node_id, inputs, block_ctx);
    }
    let Some(style) = tree.nodes[index].multicol else {
        return crate::layout::dispatch::block_layout(tree, node_id, inputs, block_ctx);
    };
    #[cfg(test)]
    crate::layout::test_support::MULTICOL_LAYOUTS.with(|count| count.set(count.get() + 1));
    let inputs = with_keyword_width(tree, node_id, inputs);

    // Taffy supplies the used border-box width to a block child. Do not fall
    // back to the page width when a nested box is measured: that would make a
    // nested percentage gap resolve against the wrong containing block.
    let parent_width = inputs.parent_size.width;
    let available_width = inputs
        .known_dimensions
        .width
        .or(match inputs.available_space.width {
            AvailableSpace::Definite(value) => Some(value),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        })
        .or_else(|| {
            multicol_definite_dimension(tree, style_of(tree, index).size().width, parent_width)
        })
        .unwrap_or(0.0);
    // A row flex container or a grid container stretches an auto-height
    // item to its line or area, a column flex container grows or bases
    // it on its flex properties, and an absolutely positioned box with
    // both block insets fills its containing block; the columns then
    // take that height. Other known heights come from the item's own
    // content and are rebalanced here.
    let css = style_of(tree, index);
    let positioned = tree.nodes[index].layout_position() == Position::Absolute;
    let inset = css.inset();
    let inset_height = positioned && !inset.top.is_auto() && !inset.bottom.is_auto();
    // The layout mode of the container that places this box in flow.
    let placement = tree.parent_of(index).filter(|_| !positioned).map(|parent| {
        let container = style_of(tree, parent);
        let margins = css.margin();
        let stretched = !margins.top.is_auto()
            && !margins.bottom.is_auto()
            && matches!(
                css.align_self().or(Some(container.align_items())),
                None | Some(AlignItems::STRETCH | AlignItems::NORMAL)
            );
        match tree.nodes[parent].formatting_context {
            FormattingContext::Flex if container.flex_direction_is_row() => {
                FlowPlacement::Stretched(stretched)
            }
            FormattingContext::Flex => FlowPlacement::ColumnFlex {
                flexed: css.flex_grow() > 0.0 || css.flex_basis() != Dimension::auto(),
            },
            FormattingContext::Grid => FlowPlacement::Stretched(stretched),
            _ => FlowPlacement::Stretched(false),
        }
    });
    let stretched_height = inputs.known_dimensions.height.is_some()
        && (inset_height
            || matches!(
                placement,
                Some(FlowPlacement::Stretched(true) | FlowPlacement::ColumnFlex { flexed: true })
            ));
    let available_height = if style.height_definite {
        multicol_definite_dimension(
            tree,
            style_of(tree, index).size().height,
            inputs.parent_size.height,
        )
        .or(inputs.known_dimensions.height)
        .map(|height| {
            multicol_authored_content_height(tree, index, height, parent_width, available_width)
        })
    } else if stretched_height
        && (!matches!(placement, Some(FlowPlacement::Stretched(_)))
            || (inputs.parent_size.height.is_some() && inputs.known_dimensions_are_definite.height))
    {
        // An auto-height flex line supplies a content-derived target size,
        // which must not prevent its columns from balancing. A definite
        // parent or grid area can constrain the columns instead.
        // A known size from flex/grid placement is already a border-box
        // size, independent of the authored box-sizing value.
        let css = style_of(tree, index);
        let basis = parent_width.unwrap_or(available_width).max(0.0);
        let (padding, border) = (css.padding(), css.border());
        let insets = multicol_resolve_inset(tree, padding.top, basis)
            + multicol_resolve_inset(tree, padding.bottom, basis)
            + multicol_resolve_inset(tree, border.top, basis)
            + multicol_resolve_inset(tree, border.bottom, basis);
        inputs
            .known_dimensions
            .height
            .map(|height| (height - insets).max(0.0))
    } else {
        None
    };
    let Some(context) = FragmentationContext::resolve(available_width, available_height, style)
    else {
        return crate::layout::dispatch::block_layout(tree, node_id, inputs, block_ctx);
    };

    // Keep the existing foundational post-pass authoritative for standalone
    // multicol boxes and for deferred out-of-flow cases. The custom path is
    // entered only at a real nested block-flow boundary.
    // A logical min-block-size + break-inside:avoid child with no direct
    // line-break flow stays off the custom path: children that hold no lines
    // are sliced at the column edges, and any other such child keeps the
    // foundational block projection, which preserves the item's unfragmented
    // minimum before the column offsets below. Text-bearing children retain
    // the existing custom line-range projection.
    let span_scope = tree
        .children_through_contents(index)
        .into_iter()
        .any(|child| spanning::is_spanner(tree, child));
    let custom_scope = (span_scope
        || tree.context.fragmentation_stack.is_empty()
            && (multicol_has_nested_descendant(tree, index)
                || (multicol_has_direct_block_child(tree, index)
                    && style_of(tree, index).size().height == Dimension::auto())
                || multicol_has_direct_break_flow(tree, index)
                || multicol_ends_with_its_only_float(tree, index))
        || !tree.context.fragmentation_stack.is_empty())
        && (!multicol_has_min_constrained_child(tree, index)
            || multicol_has_direct_break_flow(tree, index))
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index);
    // A definite-height container of paragraphs breaks them between lines
    // too.
    let definite_paragraphs = !custom_scope
        && !span_scope
        && tree.context.fragmentation_stack.is_empty()
        && style.horizontal
        && available_height.is_some_and(|height| height > 0.0)
        && multicol_has_direct_block_child(tree, index)
        && multicol_has_only_paragraph_children(
            tree,
            index,
            context.column_fill != ColumnFillValue::Auto,
        )
        && !multicol_has_out_of_flow_descendant(tree, index);
    // Children that hold no lines are sliced at the column edges, in a
    // nested container as well as an outermost one. Only an outermost
    // container also slices the blocks that wrappers hold.
    let slice_wrappers = tree.context.fragmentation_stack.is_empty();
    let lineless_children =
        (inputs.run_mode == RunMode::PerformLayout && !span_scope && style.horizontal)
            .then(|| {
                if slice_wrappers {
                    lineless::supported_with_wrappers(tree, index)
                } else {
                    lineless::supported(tree, index, true)
                }
            })
            .flatten();
    // A nested container whose children are sliced only because they hold
    // absolutely positioned boxes.
    let nested_out_of_flow = lineless_children.is_some()
        && !tree.context.fragmentation_stack.is_empty()
        && lineless::supported(tree, index, false).is_none();
    // A nested container that turns out taller than its outer column falls
    // back to the custom path, which also breaks it at the outer column end.
    let nested_fallback = custom_scope && !tree.context.fragmentation_stack.is_empty();
    let mut custom_scope = custom_scope && lineless_children.is_none();
    let stack_depth = tree.context.fragmentation_stack.len();
    let outer_height = stack_depth
        .checked_sub(1)
        .and_then(|outer| tree.context.fragmentation_stack[outer].available_height);
    // A container with a column container inside it lays its children out
    // again in its columns after the pass across its whole width below.
    let relays_out_children = custom_scope
        && inputs.run_mode == RunMode::PerformLayout
        && multicol_has_nested_descendant(tree, index);
    let deferred = tree.context.columns_relayout_pending;
    let deferred_before = tree.context.columns_deferred_layouts;
    tree.context.fragmentation_stack.push(context);
    tree.context.columns_relayout_pending = deferred || relays_out_children;
    let mut output = crate::layout::dispatch::block_layout(tree, node_id, inputs, block_ctx);
    tree.context.columns_relayout_pending = deferred;
    // Block layout has placed the children again, with the offsets of their
    // insets, which a column strategy below leaves out when it places them.
    if inputs.run_mode == RunMode::PerformLayout {
        for child in tree.children_through_contents(index) {
            tree.nodes[child].needs_relative_block_paint_offset = false;
        }
    }
    if tree.fragment_tree.limit_exceeded {
        tree.context.fragmentation_stack.truncate(stack_depth);
        return output;
    }
    // The size of a box with a definite height does not depend on its
    // columns, and the container that lays it out again settles them then.
    // What the caches of the boxes inside hold was laid out before the pass
    // above, which has replaced their lines and fragments. A layout that does
    // not go through the cache of this box leaves it to this call to drop them.
    if deferred && inputs.run_mode == RunMode::PerformLayout && available_height.is_some() {
        tree.context.fragmentation_stack.truncate(stack_depth);
        tree.context.columns_deferred_layouts += 1;
        tree.cache_clear_subtree(NodeId::from(index));
        return output;
    }
    // The columns below are settled by the end of this layout if it lays its
    // children out again.
    if relays_out_children {
        tree.context.columns_deferred_layouts = deferred_before;
    }
    // Columns sliced here are not cut again at the end of an outer column,
    // so such a container keeps the nested strategies from the start unless
    // it starts the outer columns and fits in the first one.
    let lineless_children = lineless_children.filter(|_| {
        !nested_out_of_flow
            || lineless::starts_the_outer_columns(tree, index)
                && outer_height.is_some_and(|outer| output.size.height <= outer + lineless::EPSILON)
    });
    custom_scope |= definite_paragraphs && lineless_children.is_none();
    let fragmentainer_height = available_height.or_else(|| {
        multicol_max_fragmentainer_height(
            tree,
            index,
            output.size.height,
            inputs.parent_size.height,
            parent_width,
        )
    });
    // Children that hold no lines are sliced below instead, minimum and all,
    // unless the slicing fails.
    if lineless_children.is_none() {
        space_min_constrained_children(
            tree,
            index,
            available_height,
            style.height_definite,
            &mut output.size.height,
        );
    }
    let break_flow_scope =
        !span_scope && lineless_children.is_none() && break_flow::supports(tree, index, context);
    let constrained_chain = (inputs.run_mode == RunMode::PerformLayout
        && !custom_scope
        && !break_flow_scope
        && lineless_children.is_none()
        && available_height.is_some_and(|height| height > 0.0)
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index))
    .then(|| constrained_chain::supported(tree, index))
    .flatten();
    // Inspect fresh child layouts before opting a definite-height plain block
    // chain into fragmentation. A chain may end in a paragraph with atomic
    // inlines, which go to the columns of their lines. Constrained children
    // and other boxes in a paragraph keep their foundational placement path.
    let plain_block_scope = inputs.run_mode == RunMode::PerformLayout
        && !custom_scope
        && !break_flow_scope
        && lineless_children.is_none()
        && available_height.is_some_and(|height| height > 0.0)
        && style.horizontal
        && !multicol_has_out_of_flow_descendant(tree, index)
        && multicol_has_plain_paragraph_chain(tree, index);
    if (custom_scope
        || break_flow_scope
        || plain_block_scope
        || constrained_chain.is_some()
        || lineless_children.is_some())
        && inputs.run_mode == RunMode::PerformLayout
    {
        // Taffy's input width is normally already the content width for this
        // bridge. Correct it for authored padding/border before deriving the
        // child column width, so percentage gaps use the used content box.
        let content_width = multicol_content_width(tree, index, output.size.width, parent_width);
        let resolved = FragmentationContext::resolve(content_width, fragmentainer_height, style)
            .unwrap_or(context);
        if let Some(active) = tree.context.fragmentation_stack.last_mut() {
            *active = resolved;
        }
        // The parent writes the container's layout after this callback.
        // Resolve insets from the used percentage basis before placing children.
        let css = style_of(tree, index);
        let (padding, border) = (css.padding(), css.border());
        let basis = parent_width.unwrap_or(output.size.width).max(0.0);
        let content_origin = Point {
            x: multicol_resolve_inset(tree, padding.left, basis)
                + multicol_resolve_inset(tree, border.left, basis),
            y: multicol_resolve_inset(tree, padding.top, basis)
                + multicol_resolve_inset(tree, border.top, basis),
        };
        let block_end_inset = multicol_resolve_inset(tree, padding.bottom, basis)
            + multicol_resolve_inset(tree, border.bottom, basis);
        // The authored minimum border-box height of the container.
        let minimum = multicol_definite_dimension(
            tree,
            css.min_size().height.into(),
            inputs.parent_size.height,
        )
        .map_or(0.0, |height| {
            if css.box_sizing() == TaffyBoxSizing::BorderBox {
                height
            } else {
                height
                    + multicol_resolve_inset(tree, padding.top, basis)
                    + multicol_resolve_inset(tree, padding.bottom, basis)
                    + multicol_resolve_inset(tree, border.top, basis)
                    + multicol_resolve_inset(tree, border.bottom, basis)
            }
        });
        let lineless_definite = available_height.is_some() || stretched_height;
        // An auto-basis item of a column flex container that does not grow
        // keeps its content height unless flexing shrinks it, so its known
        // height bounds the balanced columns like a maximum height.
        let flexed_maximum = inputs
            .known_dimensions
            .height
            .filter(|_| matches!(placement, Some(FlowPlacement::ColumnFlex { flexed: false })))
            .map(|height| (height - content_origin.y - block_end_inset).max(0.0));
        let lineless_maximum = match (fragmentainer_height, flexed_maximum) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let used_height = if let Some(children) = &lineless_children {
            lineless::layout(
                tree,
                index,
                children,
                resolved,
                output.size,
                content_origin,
                block_end_inset,
                minimum,
                lineless_definite,
                lineless_maximum,
                outer_height,
                slice_wrappers,
            )
            .unwrap_or_else(|| {
                if nested_fallback {
                    return relayout_nested_multicol_children(
                        tree,
                        node_id,
                        resolved,
                        output.size,
                        fragmentainer_height.is_some(),
                        content_origin,
                        block_end_inset,
                    );
                }
                space_min_constrained_children(
                    tree,
                    index,
                    available_height,
                    style.height_definite,
                    &mut output.size.height,
                );
                output.size.height
            })
        } else if let Some(chain) = constrained_chain {
            constrained_chain::layout(tree, index, &chain, resolved, output.size, content_origin)
                .unwrap_or(output.size.height)
        } else if break_flow_scope {
            break_flow::layout(tree, index, resolved, output.size, content_origin)
        } else {
            relayout_nested_multicol_children(
                tree,
                node_id,
                resolved,
                output.size,
                fragmentainer_height.is_some(),
                content_origin,
                block_end_inset,
            )
        };
        // Balanced columns size an auto-height container even under a
        // maximum height, which only bounds the balanced columns.
        let balances_height =
            fragmentainer_height.is_none() || (lineless_children.is_some() && !lineless_definite);
        if balances_height {
            output.size.height = used_height.max(minimum);
        }
        // The principal fragment paints the same border box that the parent
        // receives, even when the balanced column content is shorter.
        if balances_height || stretched_height {
            for fragment in &mut tree.fragment_tree.fragments {
                if fragment.node_id == index && fragment.parent.is_none() {
                    fragment.rect.height = output.size.height;
                }
            }
        }
    }
    // Truncating to the depth captured on entry keeps the stack balanced if a
    // future child strategy starts pushing a nested context of its own.
    tree.context.fragmentation_stack.truncate(stack_depth);
    output
}

/// Space children with a logical minimum block size that avoid breaks
/// inside across the definite `available_height` of the container `index`,
/// and keep an auto `height` at the tallest such child. The foundational
/// block path cannot break between them, so this approximates the columns.
fn space_min_constrained_children(
    tree: &mut LayoutSession<'_>,
    index: usize,
    available_height: Option<f32>,
    height_definite: bool,
    height: &mut f32,
) {
    if let Some(parent_height) = available_height.filter(|_| {
        multicol_has_min_constrained_child(tree, index)
            && !multicol_has_direct_break_flow(tree, index)
    }) {
        let children: Vec<usize> = tree
            .children_through_contents(index)
            .into_iter()
            .filter(|&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && !tree.nodes[child].is_display_none()
            })
            .collect();
        // The minimum-sized child makes the list non-empty.
        let child_height = children
            .first()
            .map_or(0.0, |&first| tree.nodes[first].layout.size.height);
        // `break-inside: avoid` consumes the unfragmented overflow on both
        // sides of a too-short/too-tall fragmentainer. Taffy does not
        // expose that fragmentation marker on its foundational block
        // path, so express the equivalent separation as a direct-child
        // block offset.
        let step = child_height + 2.0 * (parent_height - child_height).abs();
        for (order, child) in children.into_iter().enumerate() {
            tree.nodes[child].layout.location.y = order as f32 * step;
            tree.nodes[child].needs_relative_block_paint_offset = true;
        }
    }
    if !height_definite && multicol_has_min_constrained_child(tree, index) {
        // The minimum-sized children overflow the auto multicol box, but the
        // auto box's used block-size remains the minimum child size rather
        // than the sum of those overflowing children.
        let min_child_height = tree
            .children_through_contents(index)
            .into_iter()
            .filter(|&child| {
                tree.nodes[child].is_in_document()
                    && tree.nodes[child].kind() == NodeKind::Element
                    && !tree.nodes[child].is_display_none()
            })
            .map(|child| tree.nodes[child].layout.size.height)
            .fold(0.0_f32, f32::max);
        *height = height.min(min_child_height);
    }
}

/// How the parent of an in-flow multicol container sizes its block axis.
#[derive(Clone, Copy)]
enum FlowPlacement {
    /// A row flex line or a grid area; `true` when the item stretches to it.
    Stretched(bool),
    /// A column flex container; `flexed` when the item grows or has a
    /// non-auto flex basis.
    ColumnFlex { flexed: bool },
}

fn multicol_max_fragmentainer_height(
    tree: &LayoutSession<'_>,
    node_id: usize,
    border_box_height: f32,
    parent_height: Option<f32>,
    parent_width: Option<f32>,
) -> Option<f32> {
    let style = style_of(tree, node_id);
    let max_height =
        multicol_definite_dimension(tree, style.max_size().height.into(), parent_height)?;
    let basis = parent_width.unwrap_or(border_box_height).max(0.0);
    let (padding, border) = (style.padding(), style.border());
    let insets = multicol_resolve_inset(tree, padding.top, basis)
        + multicol_resolve_inset(tree, padding.bottom, basis)
        + multicol_resolve_inset(tree, border.top, basis)
        + multicol_resolve_inset(tree, border.bottom, basis);
    let max_content_height = if style.box_sizing() == BoxSizing::BorderBox {
        (max_height - insets).max(0.0)
    } else {
        max_height
    };
    let used_content_height = (border_box_height - insets).max(0.0);
    Some(used_content_height.min(max_content_height))
}

/// The input that lays a child out in a column of `column_width`, at
/// `known_width` when the column fixes its width; its height comes from its
/// content within `available_height`.
fn column_child_input(
    known_width: Option<f32>,
    column_width: f32,
    parent_height: Option<f32>,
    available_height: AvailableSpace,
) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: known_width,
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(column_width),
            height: parent_height,
        },
        available_space: Size {
            width: AvailableSpace::Definite(column_width),
            height: available_height,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

/// The left and right margins of a box that are not auto, with percentages
/// and `calc()` resolved against the column width.
fn column_definite_margins(
    style: &LayoutStyleView<'_>,
    column_width: f32,
) -> (Option<f32>, Option<f32>) {
    let margin = style.margin();
    let resolve = |value: LengthPercentageAuto| value.maybe_resolve(Some(column_width));
    (resolve(margin.left), resolve(margin.right))
}

/// The layout block flow stored for `child`, with the padding and the margins
/// above and below it that depend on a percentage basis resolved against the
/// column of `column_width` that it is measured in. Block flow resolved them
/// against the whole container, but the column is the containing block of a
/// box in it.
fn column_child_layout(tree: &LayoutSession<'_>, child: usize, column_width: f32) -> BoxLayout {
    let mut layout = tree.nodes[child].layout;
    if layout_node(tree, child).kind() != NodeKind::Element {
        return layout;
    }
    let style = style_of(tree, child);
    let padding = style.padding();
    let depends_on_basis = |value: LengthPercentage| {
        value.is_calc() || value.resolve_or_zero(Some(0.0)) != value.resolve_or_zero(Some(1.0))
    };
    if [padding.top, padding.right, padding.bottom, padding.left]
        .into_iter()
        .any(depends_on_basis)
    {
        layout.padding = padding.resolve_or_zero(Some(column_width));
    }
    let margin = style.margin();
    let resolved = |value: LengthPercentageAuto, used: f32| {
        if value.is_calc() || value.resolve(Some(0.0)) != value.resolve(Some(1.0)) {
            value.maybe_resolve(Some(column_width)).unwrap_or(0.0)
        } else {
            used
        }
    };
    layout.margin.top = resolved(margin.top, layout.margin.top);
    layout.margin.bottom = resolved(margin.bottom, layout.margin.bottom);
    layout
}

/// The room between the horizontal margins of a box in a column of
/// `column_width`, where an auto margin counts as zero.
fn column_room(style: &LayoutStyleView<'_>, column_width: f32) -> f32 {
    let (left, right) = column_definite_margins(style, column_width);
    (column_width - left.unwrap_or(0.0) - right.unwrap_or(0.0)).max(0.0)
}

/// The left and right margins of a box when its border box is `child_width`
/// wide in a column of `column_width`. Auto margins share the room the width
/// and the other margin leave (CSS 2, 10.3.3), except on a float, where they
/// are zero.
fn column_horizontal_margins(
    style: &LayoutStyleView<'_>,
    column_width: f32,
    child_width: f32,
) -> (f32, f32) {
    let (left, right) = column_definite_margins(style, column_width);
    let free = if style.float().is_floated() {
        0.0
    } else {
        (column_width - child_width - left.unwrap_or(0.0) - right.unwrap_or(0.0)).max(0.0)
    };
    match (left, right) {
        (Some(left), Some(right)) => (left, right),
        (Some(left), None) => (left, free),
        (None, Some(right)) => (free, right),
        (None, None) => (free / 2.0, free / 2.0),
    }
}

/// The input that lays `child` out in a column of `column_width`: at the
/// width the column imposes on it, if any, and for a float or a button with an
/// auto width within the room its margins leave. A block that `justify-self`
/// places at a position takes its fit-content width in that room instead
/// (CSS Box Alignment 3 §6.1).
fn column_child_layout_input(
    tree: &mut LayoutSession<'_>,
    child: usize,
    column_width: f32,
    parent_height: Option<f32>,
    available_height: AvailableSpace,
) -> LayoutInput {
    if layout_node(tree, child).kind() != NodeKind::Element {
        return column_child_input(
            Some(column_width),
            column_width,
            parent_height,
            available_height,
        );
    }
    // An authored width resolves against the column width instead, and a
    // float or a button with an auto width shrinks to fit its content within
    // the column (HTML Standard 15.5.3 for the button).
    let (room, shrinks, auto_width) = {
        let style = style_of(tree, child);
        (
            column_room(&style, column_width),
            style.float().is_floated() || layout_node(tree, child).is_html_button(),
            style.size().width == Dimension::auto(),
        )
    };
    let imposed_width = (auto_width && !shrinks).then(|| {
        if crate::layout::dispatch::block_flow::justify_fits_content(tree, child) {
            column_fit_content_width(tree, child, column_width, parent_height, room)
        } else {
            room
        }
    });
    let mut input =
        column_child_input(imposed_width, column_width, parent_height, available_height);
    if input.known_dimensions.width.is_none() && shrinks {
        input.available_space.width = AvailableSpace::Definite(room);
    }
    input
}

/// The fit-content width of `child` in the `room` of a column of
/// `column_width`: its max-content width, but no wider than the room unless
/// its min-content width is.
pub(super) fn column_fit_content_width(
    tree: &mut LayoutSession<'_>,
    child: usize,
    column_width: f32,
    parent_height: Option<f32>,
    room: f32,
) -> f32 {
    let mut measure = |space| {
        let mut input = column_child_input(
            None,
            column_width,
            parent_height,
            AvailableSpace::MaxContent,
        );
        input.run_mode = RunMode::ComputeSize;
        input.known_dimensions_are_definite.width = false;
        input.available_space.width = space;
        tree.compute_child_layout(NodeId::from(child), input)
            .size
            .width
    };
    let max_content = measure(AvailableSpace::MaxContent);
    let min_content = measure(AvailableSpace::MinContent);
    max_content.min(room).max(min_content)
}

pub(crate) fn multicol_definite_dimension(
    _tree: &Document,
    value: Dimension,
    basis: Option<f32>,
) -> Option<f32> {
    let basis = basis.unwrap_or(0.0);
    let resolved = match value.expand() {
        ExpandedDimension::Length(px) => px,
        ExpandedDimension::Percent(percent) => basis * percent,
        ExpandedDimension::Calc(pointer) => pointer.resolve(basis),
        ExpandedDimension::Auto
        | ExpandedDimension::MinContent
        | ExpandedDimension::MaxContent
        | ExpandedDimension::FitContentPx(_)
        | ExpandedDimension::FitContentPercent(_)
        | ExpandedDimension::FitContentCalc(_)
        | ExpandedDimension::FitContent
        | ExpandedDimension::Stretch
        | ExpandedDimension::Content => return None,
    };
    resolved.is_finite().then_some(resolved.max(0.0))
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_resolve_inset(_tree: &Document, value: LengthPercentage, basis: f32) -> f32 {
    let resolved = match value.expand() {
        ExpandedLengthPercentage::Length(px) => px,
        ExpandedLengthPercentage::Percent(percent) => basis * percent,
        ExpandedLengthPercentage::Calc(pointer) => pointer.resolve(basis),
    };
    if resolved.is_finite() {
        resolved.max(0.0)
    } else {
        0.0
    }
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_content_width(
    tree: &LayoutSession<'_>,
    node_id: usize,
    border_box_width: f32,
    parent_width: Option<f32>,
) -> f32 {
    let style = style_of(tree, node_id);
    let (padding, border) = (style.padding(), style.border());
    let basis = parent_width.unwrap_or(border_box_width).max(0.0);
    let horizontal = multicol_resolve_inset(tree, padding.left, basis)
        + multicol_resolve_inset(tree, padding.right, basis)
        + multicol_resolve_inset(tree, border.left, basis)
        + multicol_resolve_inset(tree, border.right, basis);
    (border_box_width - horizontal).max(0.0)
}

fn multicol_authored_content_height(
    tree: &LayoutSession<'_>,
    node_id: usize,
    height: f32,
    parent_width: Option<f32>,
    fallback_width: f32,
) -> f32 {
    let style = style_of(tree, node_id);
    if style.box_sizing() != TaffyBoxSizing::BorderBox {
        return height;
    }
    // A fragmentainer's height measures content, while a border-box authored
    // height includes block-axis padding and borders.
    let basis = parent_width.unwrap_or(fallback_width).max(0.0);
    let (padding, border) = (style.padding(), style.border());
    let insets = multicol_resolve_inset(tree, padding.top, basis)
        + multicol_resolve_inset(tree, padding.bottom, basis)
        + multicol_resolve_inset(tree, border.top, basis)
        + multicol_resolve_inset(tree, border.bottom, basis);
    (height - insets).max(0.0)
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_has_nested_descendant(tree: &Document, node_id: usize) -> bool {
    let mut pending = tree.nodes[node_id].children.clone();
    while let Some(child) = pending.pop() {
        if tree.nodes[child].multicol.is_some() {
            return true;
        }
        pending.extend(tree.nodes[child].children.iter().copied());
    }
    false
}

// cov:ignore: nested recursive layout is exercised by the ignored nested WPT reftests.
fn multicol_has_direct_break_flow(tree: &Document, node_id: usize) -> bool {
    tree.children_through_contents(node_id)
        .into_iter()
        .any(|child| {
            tree.nodes[child].kind() == NodeKind::Element
                && tree.nodes[child].is_in_document()
                && tree.nodes[child]
                    .children
                    .iter()
                    .any(|&grandchild| tree.nodes[grandchild].tag_name() == Some("br"))
        })
}

/// Whether the last rendered child of `node_id` is a float, the earlier
/// children hold no float and no block-direction margin, and the container
/// holds no inline content of its own. Nothing then wraps beside the float, no
/// other float shares its line, and no adjoining margins collapse, so the
/// column strategies can place it like a block of its own: shrunk to its
/// content within the column, at the edge of its own side.
fn multicol_ends_with_its_only_float(tree: &LayoutSession<'_>, node_id: usize) -> bool {
    if tree.nodes[node_id].ifc.is_some() {
        return false;
    }
    let is_float = |child: usize| is_floated(tree, child);
    let rendered: Vec<usize> = tree
        .children_through_contents(node_id)
        .into_iter()
        .filter(|&child| {
            tree.nodes[child].is_in_document()
                && tree.nodes[child].kind() == NodeKind::Element
                && !tree.nodes[child].is_display_none()
        })
        .collect();
    rendered.split_last().is_some_and(|(&last, before)| {
        is_float(last)
            && !before
                .iter()
                .any(|&child| multicol_subtree_has_float_or_block_margin(tree, child))
    })
}

fn multicol_has_out_of_flow_descendant(tree: &Document, node_id: usize) -> bool {
    let mut pending = tree.nodes[node_id].children.clone();
    while let Some(child) = pending.pop() {
        if tree.nodes[child].layout_position() == Position::Absolute {
            return true;
        }
        pending.extend(tree.nodes[child].children.iter().copied());
    }
    false
}

// cov:ignore: direct block fragmentation is exercised by the ignored multicol WPT reftests.
fn multicol_has_direct_block_child(tree: &Document, node_id: usize) -> bool {
    tree.children_through_contents(node_id)
        .into_iter()
        .any(|child| {
            tree.nodes[child].is_in_document()
                && tree.nodes[child].kind() == NodeKind::Element
                && !tree.nodes[child].is_display_none()
        })
}

// Whether `node_id` has several child boxes, or one with block-edge padding
// or border, and every one is an in-flow block of lines whose height comes from
// its content and whose block edges, if any, its pieces slice, which the custom
// path breaks between lines in a definite-height container as well. A box with
// nothing inside it may stand among them, if one of the others holds lines: one
// with a block size of its own, which the planner places whole in a column, or
// one with no room of its own, whose margins collapse through it. So may a box
// of lines with a height of its own (see `is_sized_paragraph`).
//
// One undecorated paragraph also counts when `balances` is set: the columns of
// a definite-height container that balance take their height from the planner,
// which the lines of a single paragraph need as well.
fn multicol_has_only_paragraph_children(
    tree: &LayoutSession<'_>,
    node_id: usize,
    balances: bool,
) -> bool {
    let mut children = tree
        .children_through_contents(node_id)
        .into_iter()
        .filter(|&child| {
            tree.nodes[child].is_in_document()
                && tree.nodes[child].kind() == NodeKind::Element
                && !tree.nodes[child].is_display_none()
        });
    // One undecorated paragraph already breaks between its lines on the
    // existing path.
    let several = children.clone().nth(1).is_some();
    let mut decorated = false;
    let mut paragraphs = 0usize;
    let plain = children.all(|child| {
        if lineless::is_sized_leaf(tree, child) || lineless::is_margin_only_block(tree, child) {
            return insets_are_a_relative_offset(tree, child);
        }
        // A wrapper whose paragraphs join the flow of the columns counts as
        // the paragraphs it holds. A chain of wrappers around one paragraph
        // keeps the strategies of such chains.
        if let Some(count) =
            paragraph_group::wrapped_paragraphs(tree, child).filter(|&count| count > 1)
        {
            paragraphs += count;
            return true;
        }
        // A box of lines with a height of its own goes whole to a column, so
        // it needs the planner as a box of lines does.
        if is_sized_paragraph(tree, child) {
            paragraphs += 1;
            return insets_are_a_relative_offset(tree, child);
        }
        let node = &tree.nodes[child];
        let style = style_of(tree, child);
        let margin = style.margin();
        let block_edges = lineless::has_block_edge_decorations(&style);
        // A paragraph that stands alone with padding or border at its sides
        // also needs the planner: the path it would take otherwise lays it
        // out at the width of the container.
        decorated |= block_edges
            || (lineless::has_inline_edge_decorations(&style) && slices_block_edges(node));
        paragraphs += 1;
        // Block-edge decoration needs slice geometry for each piece, which the
        // stream gives the lines it places; the planner collapses the adjoining
        // margins as numbers, which `auto` would leave to the layout of the box
        // (a percentage is a share of the column width, which
        // `column_child_layout` resolves before the planner reads it); and the
        // line offsets are read as horizontal block offsets.
        node.formatting_context == FormattingContext::Inline
            && !node
                .authored_writing_mode
                .is_some_and(WritingMode::is_vertical)
            && matches!(node.display(), DisplayValue::Block)
            && (!block_edges || slices_block_edges(node))
            && !margin.top.is_auto()
            && !margin.bottom.is_auto()
            && node.layout_position() != Position::Absolute
            && insets_are_a_relative_offset(tree, child)
            && !style.float().is_floated()
            && style.size().height.is_auto()
            && style.min_size().height.is_auto()
            && style.max_size().height.is_auto()
    });
    // The paragraphs of a wrapper count as the container's own.
    plain && paragraphs > 0 && (several || paragraphs > 1 || decorated || balances)
}

/// Whether the insets of `child` can be left to the painter once the columns
/// place the box: it has none, or it is `position: relative` with only inline
/// insets, whose offset the painter adds to the location the column strategy
/// gives it.
///
/// This is the rule for a box that a strategy derives from the location block
/// flow gave it, as the boxes of a chain of single children are: a block-axis
/// inset is part of that location and moves the box across pages, which
/// pagination measures from it. A box that a strategy places itself takes
/// [`insets_are_a_relative_offset`].
fn insets_are_a_relative_inline_offset(tree: &LayoutSession<'_>, child: usize) -> bool {
    let inset = style_of(tree, child).inset();
    inset.top.is_auto()
        && inset.bottom.is_auto()
        && (inset.left.is_auto() && inset.right.is_auto()
            || tree.nodes[child].layout_position() == Position::Relative)
}

/// Whether the insets of `child`, a box that the columns place themselves, can
/// be left to the painter: it has none, or it is `position: relative`. The
/// painter adds the whole offset to the location the column strategy gives it,
/// and [`leave_block_offset_to_painter`] tells pagination so.
fn insets_are_a_relative_offset(tree: &LayoutSession<'_>, child: usize) -> bool {
    tree.nodes[child].layout_position() == Position::Relative
        || insets_are_a_relative_inline_offset(tree, child)
}

/// Record that the columns placed `child` without the block offset of its
/// `position: relative` insets, which the painter adds. Pagination measures
/// that offset from the location layout gave the box unless it is told so.
/// A box on a line is placed with its offset, so it is left as it is.
fn leave_block_offset_to_painter(tree: &mut LayoutSession<'_>, child: usize) {
    let inset = style_of(tree, child).inset();
    let node = &tree.nodes[child];
    if node.layout_position() == Position::Relative
        && !node.display().is_inline_level()
        && !(inset.top.is_auto() && inset.bottom.is_auto())
    {
        tree.nodes[child].needs_relative_block_paint_offset = true;
    }
}

// A logical non-auto child minimum paired with break avoidance participates
// in block sizing before column projection. Keep that case on the foundational
// path until the custom projection can account for the minimum's unfragmented
// overflow.
fn multicol_has_min_constrained_child(tree: &LayoutSession<'_>, node_id: usize) -> bool {
    tree.children_through_contents(node_id)
        .into_iter()
        .any(|child| {
            tree.nodes[child].is_in_document()
                && tree.nodes[child].kind() == NodeKind::Element
                && !tree.nodes[child].is_display_none()
                && tree.nodes[child].has_logical_min_block_size
                && !style_of(tree, child).min_size().height.is_auto()
        })
}

fn can_balance_single_paragraph(tree: &LayoutSession<'_>, child: usize) -> bool {
    can_balance_atomic_paragraph(tree, child, BlockEdges::Refused)
        && tree.nodes[child].ifc_boxes().is_empty()
}

/// Whether `child` is a paragraph whose lines break between columns and whose
/// boxes, if any, are atomic inlines. Those sit on the lines like text, so
/// each goes to the column of its line. With `edges`, the paragraph may have
/// block-edge padding and border (see [`can_balance_paragraph_box`]).
fn can_balance_atomic_paragraph(tree: &LayoutSession<'_>, child: usize, edges: BlockEdges) -> bool {
    can_balance_paragraph_box(tree, child, edges)
        && tree.nodes[child].ifc_writing_mode() == Some(shodo::geometry::WritingMode::HorizontalTb)
        && paragraph_boxes_are_atomic(tree, child)
}

/// Whether the paragraph `node_id` has no box but atomic inlines.
fn paragraph_boxes_are_atomic(tree: &Document, node_id: usize) -> bool {
    tree.nodes[node_id].ifc.as_ref().is_some_and(|root| {
        root.boxes
            .iter()
            .all(|box_| box_.kind == crate::layout::ifc::boxes::IfcBoxKind::Atomic)
    })
}

/// Whether `child` is a block that breaks between its lines at the column
/// edges and whose height follows its content. Block-edge padding and border
/// need slice or clone geometry for each piece, which only a stream that
/// places the pieces with their edges has: with `edges`, the padding and border
/// are accepted when the pieces slice them and paint them as one box.
fn can_balance_paragraph_box(tree: &LayoutSession<'_>, child: usize, edges: BlockEdges) -> bool {
    let node = layout_node(tree, child);
    let layout = node.layout;
    let undecorated = layout.padding.top == 0.0
        && layout.padding.bottom == 0.0
        && layout.border.top == 0.0
        && layout.border.bottom == 0.0;
    (undecorated || edges == BlockEdges::Sliced && slices_block_edges(node))
        && matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
        && node.break_inside == raikiri_style::property::BreakInside::Auto
        && {
            let style = style_of(tree, child);
            style.size().height.is_auto()
                && style.min_size().height.is_auto()
                && style.max_size().height.is_auto()
        }
}

/// Whether `child` is a plain block of lines whose height is authored: a box
/// the planner places whole in a column, whatever its lines would give it, as
/// it places a box with no lines. The structure decides this before the box
/// is laid out; [`paragraph_group`] also checks that the lines fit the height
/// once they are shaped.
fn is_sized_paragraph(tree: &LayoutSession<'_>, child: usize) -> bool {
    let node = layout_node(tree, child);
    let style = style_of(tree, child);
    node.formatting_context == FormattingContext::Inline
        && lineless::is_plain_block_holding_lines(tree, child)
        && !node
            .authored_writing_mode
            .is_some_and(WritingMode::is_vertical)
        && node.break_inside == raikiri_style::property::BreakInside::Auto
        && !node.has_before_or_after_content
        && !multicol_subtree_has_float(tree, child)
        && style_dimension_length(style.size().height).is_some_and(|height| height > 0.0)
        && style.min_size().height.is_auto()
        && style.max_size().height.is_auto()
        && !style.margin().top.is_auto()
        && !style.margin().bottom.is_auto()
}

/// Whether the padding and border above the first line and below the last of
/// a paragraph are accepted, which only a stream that places them with the
/// lines of each piece can do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockEdges {
    /// The paragraph has none.
    Refused,
    /// The pieces of the paragraph slice them.
    Sliced,
}

/// The block size of the padding and border above the first line and below
/// the last of a box.
fn block_edges(layout: &BoxLayout) -> (f32, f32) {
    (
        layout.padding.top + layout.border.top,
        layout.padding.bottom + layout.border.bottom,
    )
}

/// Whether the pieces of the box `node` cut at a column end slice its block
/// edges (CSS Fragmentation 3, 5.4): each paints the whole box, clipped to
/// its column, so the padding and border of an edge show where the box begins
/// or ends only. A box that repeats them at every break, or paints effects
/// sized from its whole border box, is not sliced.
fn slices_block_edges(node: &crate::node::Node) -> bool {
    !node.clones_box_decorations && !node.paints_whole_box_effects
}

/// Whether a right-to-left parent aligns `child` to its right edge, which
/// happens to a block narrower than the parent's content box. The streams of
/// pieces that cut a block at the column edges place it from the left, so
/// they leave such a block to the single chain or to the placement of each
/// child.
fn is_right_aligned_in_parent(tree: &LayoutSession<'_>, child: usize) -> bool {
    let rtl_parent = tree
        .taffy_parent_of(child)
        .is_some_and(|parent| style_of(tree, parent).direction() == TaffyDirection::Rtl);
    rtl_parent
        && !(layout_node(tree, child).multicol_auto_width && {
            let style = style_of(tree, child);
            style.min_size().width.is_auto() && style.max_size().width.is_auto()
        })
}

fn multicol_has_plain_paragraph_chain(tree: &LayoutSession<'_>, parent: usize) -> bool {
    if tree.nodes[parent].ifc.is_some() {
        return false;
    }
    only_element_child(tree, parent)
        .and_then(|child| plain_paragraph_chain_leaf(tree, child))
        .is_some()
}

/// The only rendered element child of `parent`, if it has exactly one.
fn only_element_child(tree: &Document, parent: usize) -> Option<usize> {
    let mut children = tree
        .children_through_contents(parent)
        .into_iter()
        .filter(|&child| {
            tree.nodes[child].is_in_document()
                && tree.nodes[child].kind() == NodeKind::Element
                && !tree.nodes[child].is_display_none()
        });
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

/// The paragraph at the end of the chain that starts at `start`, when every
/// box of the chain, the paragraph included, is a plain block in the flow:
/// no margin, padding, border, float, authored size, size containment or
/// column span, so its height follows its content, and each box but the last
/// holds nothing but the next one, clips nothing and paints no effect sized
/// from its whole box, as the pieces of the paragraph in the later columns
/// lie outside its box. The paragraph breaks between its lines and holds no
/// box but atomic inlines.
fn plain_paragraph_chain_leaf(tree: &LayoutSession<'_>, start: usize) -> Option<usize> {
    let mut current = start;
    loop {
        let node = layout_node(tree, current);
        let style = style_of(tree, current);
        let zero_margin = LengthPercentageAuto::length(0.0);
        let overflow = style.overflow();
        let margin = style.margin();
        let clips = node.ifc.is_none()
            && (overflow.x != Overflow::Visible || overflow.y != Overflow::Visible);
        // A slice of a wrapper cannot reproduce an effect sized from its
        // whole box.
        let painted = node.ifc.is_none() && node.paints_whole_box_effects;
        if clips
            || painted
            // The paragraph at the end may be a list item, whose marker sits on
            // its first line.
            || !(node.display() == DisplayValue::Block
                || node.ifc.is_some() && node.display() == DisplayValue::ListItem)
            || style.float().is_floated()
            || margin.top != zero_margin
            || margin.right != zero_margin
            || margin.bottom != zero_margin
            || margin.left != zero_margin
            || style.padding() != Edges::splat(LengthPercentage::ZERO)
            || style.border() != Edges::splat(LengthPercentage::ZERO)
            || !node.multicol_auto_width
            || !style.min_size().width.is_auto()
            || !style.max_size().width.is_auto()
            || !style.size().height.is_auto()
            || !style.min_size().height.is_auto()
            || !style.max_size().height.is_auto()
            || node.size_containment.height
            || node.column_span_all
            || node.break_inside != raikiri_style::property::BreakInside::Auto
            || node.break_before != BreakBetween::Auto
            || node.break_after != BreakBetween::Auto
        {
            return None;
        }
        if node.ifc.is_some() {
            return (node.ifc_writing_mode() == Some(shodo::geometry::WritingMode::HorizontalTb)
                && paragraph_boxes_are_atomic(tree, current)
                && !multicol_subtree_has_float(tree, current))
            .then_some(current);
        }
        current = only_element_child(tree, current)?;
    }
}

/// The paragraph at the end of the chain that starts at the wrapper `start`,
/// as [`plain_paragraph_chain_leaf`] finds it, when the wrapper fills its box
/// with a color and the paragraph is all it holds. Generated content of a box
/// of the chain is no arena child, so it would lie beside the paragraph in a
/// box that is cut at the columns of the paragraph.
fn painted_wrapper_leaf(tree: &LayoutSession<'_>, start: usize) -> Option<usize> {
    let leaf = plain_paragraph_chain_leaf(tree, start)?;
    let mut current = start;
    while current != leaf {
        if tree.nodes[current].has_before_or_after_content {
            return None;
        }
        current = only_element_child(tree, current)?;
    }
    Some(leaf)
}

fn relayout_nested_multicol_children(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    context: FragmentationContext,
    border_box_size: Size<f32>,
    has_fragmentainer_height_constraint: bool,
    content_origin: Point<f32>,
    block_end_inset: f32,
) -> f32 {
    let index = usize::from(node_id);
    let fallback_height = border_box_size.height;
    let children: Vec<usize> = tree
        .children_through_contents(index)
        .into_iter()
        .filter(|&child| tree.nodes[child].is_in_document() && !tree.nodes[child].is_display_none())
        .collect();
    let has_spanners = children
        .iter()
        .any(|&child| spanning::is_spanner(tree, child));
    if has_spanners {
        spanning::retire_preliminary_fragments(tree, &children);
    }
    tree.fragment_tree.retire_previous_roots(index);
    let Some(container_fragment) = tree
        .fragment_tree
        .try_push(crate::fragment::LayoutFragment {
            node_id: index,
            parent: None,
            fragmentainer: context.column_index,
            rect: crate::fragment::FragmentRect {
                x: context.origin_x,
                y: context.origin_y,
                width: border_box_size.width,
                height: fallback_height,
            },
            fragmentainer_clip: None,
            fragment_index: 0,
            fragment_count: 1,
            line_start: None,
            line_end: None,
        })
    else {
        return fallback_height;
    };
    tree.nodes[index].multicol_groups.clear();
    tree.nodes[index].multicol_rows.clear();
    let used = if has_spanners {
        spanning::layout(
            tree,
            index,
            &children,
            container_fragment,
            context,
            border_box_size,
            content_origin,
            block_end_inset,
        )
    } else {
        layout_column_group(
            tree,
            index,
            &children,
            0,
            container_fragment,
            context,
            border_box_size,
            content_origin,
            block_end_inset,
            tree.context.fragmentation_stack.len() <= 1
                && context.available_height.is_none()
                && !tree.context.unpaged_plain_columns,
        )
    };
    let fragment_height = if tree.nodes[index]
        .multicol
        .is_some_and(|style| style.height_definite)
    {
        fallback_height
    } else if has_fragmentainer_height_constraint {
        used.min(fallback_height)
    } else {
        used
    };
    if let Some(fragment) = tree.fragment_tree.fragments.get_mut(container_fragment) {
        fragment.rect.height = fragment_height;
    } // cov:ignore: the successful try_push index remains stable throughout recursive group layout.
    used
}

/// Place the boxes inside the wrapper that the `entry`th child of `group`
/// is, where the columns of `context` put them: each relative to the wrapper
/// that holds it, whose box starts with its first box. Returns the columns
/// their lines reach.
fn place_wrapped_boxes(
    tree: &mut LayoutSession<'_>,
    group: &paragraph_group::Group,
    entry: usize,
    context: FragmentationContext,
) -> Vec<usize> {
    let mut columns = Vec::new();
    for wrapped in group.wrapped(entry) {
        let (placement, parent) = (&wrapped.placement, &wrapped.parent);
        let parent_layout = tree.nodes[wrapped.parent_node].layout;
        let mut layout = tree.nodes[wrapped.node].layout;
        // The box stands at the start of the wrapper's content box, which
        // starts at the left of the wrapper's first column, in its own.
        layout.location = Point {
            x: parent_layout.border.left
                + parent_layout.padding.left
                + layout.margin.left
                + context.column_offset_x(placement.first_column)
                - context.column_offset_x(parent.first_column),
            y: placement.first_y - parent.first_y,
        };
        layout.size.height = placement.first_height;
        tree.set_unrounded_layout(NodeId::from(wrapped.node), &layout);
        leave_block_offset_to_painter(tree, wrapped.node);
        if let Some(root) = tree.nodes[wrapped.node].ifc.as_mut() {
            root.multicol_fragments = Some(placement.fragments.clone());
            root.multicol_fragment_origins_recorded = true;
        }
        columns.extend(
            placement
                .fragments
                .iter()
                .filter(|range| range.line_start < range.line_end)
                .map(|range| range.fragmentainer),
        );
        let column_context = context.in_column(
            placement.first_column,
            context.origin_x + context.column_offset_x(placement.first_column),
            context.origin_y + placement.first_y - layout.margin.top,
        );
        refresh_paragraph_boxes(
            tree,
            wrapped.node,
            column_context,
            FlexFloatChain::of_ancestors(tree, wrapped.node),
        );
    }
    columns
}

/// Record the pieces of the boxes inside the wrapper that the `entry`th child
/// of `group` is, whose fragment is `wrapper_fragment`: a fragment for each
/// wrapper inside it, and one for each piece of a paragraph, under the
/// fragment of the wrapper that holds it. `None` when the fragments run out.
fn record_wrapped_fragments(
    tree: &mut LayoutSession<'_>,
    group: &paragraph_group::Group,
    entry: usize,
    wrapper_fragment: usize,
    context: FragmentationContext,
) -> Option<()> {
    let mut fragments = HashMap::new();
    fragments.insert(
        tree.fragment_tree.fragments[wrapper_fragment].node_id,
        wrapper_fragment,
    );
    for wrapped in group.wrapped(entry) {
        let parent_fragment = *fragments.get(&wrapped.parent_node)?;
        let layout = tree.nodes[wrapped.node].layout;
        let Some(lines) = tree.nodes[wrapped.node]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .filter(|_| !wrapped.placement.fragments.is_empty())
        else {
            let fragment = tree
                .fragment_tree
                .try_push(crate::fragment::LayoutFragment {
                    node_id: wrapped.node,
                    parent: Some(parent_fragment),
                    fragmentainer: wrapped.placement.first_column,
                    rect: crate::fragment::FragmentRect {
                        x: layout.location.x,
                        y: layout.location.y,
                        width: layout.size.width,
                        height: layout.size.height,
                    },
                    fragmentainer_clip: None,
                    fragment_index: 0,
                    fragment_count: 1,
                    line_start: None,
                    line_end: None,
                })?;
            fragments.insert(wrapped.node, fragment);
            continue;
        };
        let extents: Vec<(f32, f32)> = lines
            .lines
            .iter()
            .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
            .collect();
        let count = wrapped.placement.fragments.len();
        // The top of the columns in the space of the wrapper's fragment.
        let columns_top = -wrapped.parent.first_y;
        for (index, range) in wrapped.placement.fragments.iter().enumerate() {
            let y = layout.location.y + range.y;
            let last = range.line_end >= extents.len();
            let height = if last {
                let first = extents.get(range.line_start).map_or(0.0, |extent| extent.0);
                extents
                    .get(range.line_end.saturating_sub(1))
                    .map_or(0.0, |extent| extent.1)
                    - first
            } else {
                (columns_top + group.height - y).max(0.0)
            };
            let rect = crate::fragment::FragmentRect {
                x: layout.location.x + range.x,
                y,
                width: layout.size.width,
                height,
            };
            // The clip cuts a piece at the block edges of its column, so the
            // lines of its neighbours stay out, and leaves the inline axis
            // open. The edge where the paragraph begins or ends has no
            // neighbour behind it.
            let clip = (count > 1).then(|| {
                let reach = paragraph_group::BLOCK_INK_REACH;
                let top = if range.line_start == 0 {
                    columns_top - reach
                } else {
                    columns_top
                };
                let bottom = columns_top + group.height + if last { reach } else { 0.0 };
                paragraph_group::inline_open_clip(crate::fragment::FragmentRect {
                    x: rect.x,
                    y: top,
                    width: context.column_width,
                    height: bottom - top,
                })
            });
            tree.fragment_tree
                .try_push(crate::fragment::LayoutFragment {
                    node_id: wrapped.node,
                    parent: Some(parent_fragment),
                    fragmentainer: range.fragmentainer,
                    rect,
                    fragmentainer_clip: clip,
                    fragment_index: index,
                    fragment_count: count,
                    line_start: Some(range.line_start),
                    line_end: Some(range.line_end),
                })?;
        }
    }
    Some(())
}

/// Record the pieces of `child`, a box with no lines that `placement` slices
/// between columns of `column_height`: its first piece `first_fragment`,
/// pushed under the container, and one more for each column after it. Each
/// piece but the last fills its column to the end; the last holds what is
/// left of `total`, the box's block size, which the node keeps. A box that
/// slices its decorations paints whole in each piece, moved up by what the
/// earlier pieces took, and the column clips it. `None` when the fragments
/// run out.
fn push_sliced_box_pieces(
    tree: &mut LayoutSession<'_>,
    child: usize,
    first_fragment: usize,
    placement: &paragraph_group::Placement,
    total: f32,
    column_height: f32,
) -> Option<()> {
    let first = tree.fragment_tree.fragments[first_fragment];
    let mut layout = tree.nodes[child].layout;
    layout.size.height = total;
    tree.set_unrounded_layout(NodeId::from(child), &layout);
    let sliced = lineless::slices_decorations(tree, child);
    let column_top = first.rect.y - placement.first_y;
    let count = placement.fragments.len();
    let mut taken = 0.0f32;
    for (index, piece) in placement.fragments.iter().enumerate() {
        let height = if index == 0 {
            placement.first_height
        } else if index + 1 == count {
            total - taken
        } else {
            column_height
        }
        .max(0.0);
        let rect = crate::fragment::FragmentRect {
            x: first.rect.x + piece.x,
            y: first.rect.y + piece.y,
            height,
            ..first.rect
        };
        let fragment = crate::fragment::LayoutFragment {
            fragmentainer: first.fragmentainer
                + piece.fragmentainer.checked_sub(placement.first_column)?,
            rect: lineless::sliced_rect(rect, sliced.then_some((taken, total))),
            fragmentainer_clip: Some(paragraph_group::inline_open_clip(
                crate::fragment::FragmentRect {
                    y: column_top,
                    height: column_height,
                    ..rect
                },
            )),
            fragment_index: index,
            fragment_count: count,
            ..first
        };
        if index == 0 {
            tree.fragment_tree.fragments[first_fragment] = fragment;
        } else {
            tree.fragment_tree.try_push(fragment)?;
        }
        taken += height;
    }
    Some(())
}

/// The only paragraph of a balanced column container, with the line ranges
/// that balance it over the columns.
struct ParagraphBalance {
    /// The child of the container that holds the paragraph.
    child: usize,
    /// The paragraph: `child` itself, or the end of the chain of plain
    /// wrappers that starts at it.
    leaf: usize,
    fragments: Vec<MulticolTextFragment>,
    /// The height of the tallest column.
    height: f32,
    /// The height of the paragraph's lines in one column.
    stacked: f32,
}

/// The slack that lets floats whose widths add up to the column width share a
/// line, whatever rounding the widths carry.
const LINE_EPSILON: f32 = 0.01;

/// A float a column holds, by the margin box it takes.
#[derive(Clone, Copy)]
struct PlacedFloat {
    right: bool,
    /// The edge of the margin box that faces the middle of the column, from the
    /// left edge of the column.
    inner_edge: f32,
    /// The block offsets of the top and the bottom of the margin box from the
    /// top of the group.
    top: f32,
    bottom: f32,
}

/// Where a float goes among the floats before it: the block offset of the top
/// of its margin box, and the width the floats beside it take on each side.
#[derive(Clone, Copy)]
struct FloatSpot {
    top: f32,
    left: f32,
    right: f32,
}

/// The floats in a column that the floats after them may still sit beside,
/// placed by the rules of CSS 2 section 9.5.1.
///
/// A float is as high as it can be, but never above the top of a float before
/// it (rule 5), nor above the bottom of the floats it clears (rule 6), and it
/// moves down to the bottom of a float in its way until it fits (rules 2, 3 and
/// 7). The line keeps the floats whose margin boxes reach below the top of the
/// last float: the ones above can neither be beside a later float nor be
/// cleared below its top.
#[derive(Clone)]
struct FloatLine {
    /// The column of the line.
    column: usize,
    /// The block offset of the top of the last float from the top of the group,
    /// which is the least a float after it can have.
    top: f32,
    floats: Vec<PlacedFloat>,
    /// Where the first float of the line starts.
    start: f32,
    /// Where the in-flow block after the floats starts, as floats take no
    /// room in the flow: where the line starts, or where the blocks that
    /// flowed beside earlier floats of it end.
    next: f32,
    /// Every float of the line, in order, for the lines of the block after
    /// them to flow beside.
    placed: Vec<WrappedFloat>,
}

/// A float that the lines of the blocks after it flow beside: the size of its
/// margin box, its side, and the block offset of its top in the column.
#[derive(Clone, Copy)]
struct WrappedFloat {
    size: Size<f32>,
    right: bool,
    top: f32,
}

/// The floats of a column that the in-flow blocks after them flow beside
/// (CSS 2, 9.5): the column, where the floats start, where the next block
/// starts, and how low the floats reach.
struct ColumnFloatRun {
    column: usize,
    start: f32,
    next: f32,
    bottom: f32,
    floats: Vec<WrappedFloat>,
    /// The floats a float after the blocks may still sit beside.
    line: Vec<PlacedFloat>,
    /// The top of the last float, which a float after it is not above.
    top: f32,
}

impl ColumnFloatRun {
    fn of(line: &FloatLine) -> Self {
        Self {
            column: line.column,
            start: line.start,
            next: line.next,
            bottom: line
                .placed
                .iter()
                .map(|float| float.top + float.size.height)
                .fold(line.start, f32::max),
            floats: line.placed.clone(),
            line: line.floats.clone(),
            top: line.top,
        }
    }

    /// Whether a block with `clear` clears any of the floats.
    fn clears(&self, clear: Clear) -> bool {
        self.floats.iter().any(|float| match clear {
            Clear::None => false,
            Clear::Left => !float.right,
            Clear::Right => float.right,
            Clear::Both => true,
        })
    }

    /// The line of floats a float after the blocks joins, at the end of the
    /// last of them (CSS 2, 9.5.1 rule 6 keeps it no higher than that line).
    fn into_line(self) -> FloatLine {
        FloatLine {
            column: self.column,
            top: self.top.max(self.next),
            floats: self.line,
            start: self.start,
            next: self.next,
            placed: self.floats,
        }
    }
}

impl FloatLine {
    /// A line in `column` with no float on it, where a float goes from `top`.
    fn at(column: usize, top: f32) -> Self {
        Self {
            column,
            top,
            floats: Vec::new(),
            start: top,
            next: top,
            placed: Vec::new(),
        }
    }

    /// Where `float`, `needed` high with its margins, goes in a column of
    /// `column_width`.
    fn place(&self, float: ColumnFloat, needed: f32, column_width: f32) -> FloatSpot {
        let mut top = self.top;
        for placed in &self.floats {
            let cleared = match float.clear {
                Clear::Left => !placed.right,
                Clear::Right => placed.right,
                Clear::Both => true,
                Clear::None => false,
            };
            if cleared {
                top = top.max(placed.bottom);
            }
        }
        loop {
            // The edges the floats that reach into the float leave, and the
            // first bottom among them below its top.
            let (mut left, mut right) = (0.0f32, column_width);
            let mut next = f32::INFINITY;
            // A float with no height still has its place beside the floats that
            // reach its top.
            let reach = top + needed.max(LINE_EPSILON);
            for placed in self
                .floats
                .iter()
                .filter(|placed| placed.top < reach && placed.bottom > top)
            {
                if placed.right {
                    right = right.min(placed.inner_edge);
                } else {
                    left = left.max(placed.inner_edge);
                }
                next = next.min(placed.bottom);
            }
            // Moving down to the bottom of a float ends in a spot with no float
            // beside it, where the float stands whatever its width.
            if next.is_infinite() || left + float.margin_box_width <= right + LINE_EPSILON {
                return FloatSpot {
                    top,
                    left,
                    right: column_width - right,
                };
            }
            top = next;
        }
    }

    /// Put `float`, `needed` high, on the line at `spot`.
    fn push(&mut self, float: ColumnFloat, needed: f32, spot: FloatSpot, column_width: f32) {
        self.top = spot.top;
        self.floats.retain(|placed| placed.bottom > spot.top);
        self.floats.push(PlacedFloat {
            right: float.right,
            inner_edge: if float.right {
                column_width - spot.right - float.margin_box_width
            } else {
                spot.left + float.margin_box_width
            },
            top: spot.top,
            bottom: spot.top + needed,
        });
        self.placed.push(WrappedFloat {
            size: Size {
                width: float.margin_box_width,
                height: needed,
            },
            right: float.right,
            top: spot.top,
        });
    }
}

/// A float child of a column container: the side it floats to, the floats it
/// clears, and the width of its margin box in a column.
#[derive(Clone, Copy)]
struct ColumnFloat {
    right: bool,
    clear: Clear,
    margin_box_width: f32,
}

impl ColumnFloat {
    /// The float `child` is, when it is one, `width` wide in a column of
    /// `column_width`.
    fn of(tree: &LayoutSession<'_>, child: usize, column_width: f32, width: f32) -> Option<Self> {
        let style = style_of(tree, child);
        style.float().is_floated().then(|| {
            let (left, right) = column_horizontal_margins(&style, column_width, width);
            Self {
                right: matches!(style.float(), Float::Right),
                clear: style.clear(),
                margin_box_width: left + width + right,
            }
        })
    }
}

/// Whether `child` of the column container `parent` is a block whose lines
/// may flow beside the floats before it in its column: an in-flow block that
/// shares the container's float context. One that clears them does not (see
/// [`ColumnFloatRun::clears`]).
fn flows_beside_column_floats(tree: &LayoutSession<'_>, parent: usize, child: usize) -> bool {
    let node = layout_node(tree, child);
    node.kind() == NodeKind::Element
        && matches!(node.display(), DisplayValue::Block | DisplayValue::ListItem)
        && crate::layout::dispatch::block_flow::child_shares_parent_context(
            tree,
            NodeId::from(parent),
            NodeId::from(child),
        )
}

/// Set `root` up as a column of `column_width` holding the floats of `run`,
/// with the top of the run at its top.
fn replay_column_floats(
    root: &mut crate::layout::dispatch::block_flow::context::BlockContext<'_>,
    run: &ColumnFloatRun,
    column_width: f32,
) {
    use crate::layout::protocol::FloatDirection;

    root.set_width(column_width);
    root.set_origin_resolved(true);
    for float in &run.floats {
        root.place_floated_box(
            float.size,
            float.top - run.start,
            if float.right {
                FloatDirection::Right
            } else {
                FloatDirection::Left
            },
            Clear::None,
            false,
        );
    }
}

/// Lay out `child`, a block `width` wide with the top margin `margin_top`,
/// at `run.next` in a column of `column_width`, with the floats of `run`
/// beside it, so that its lines flow beside them. Its side margins resolve
/// against the column, and a right-to-left column puts a narrower block at
/// its right edge, as the columns place it.
#[allow(clippy::too_many_arguments)]
fn layout_beside_column_floats(
    tree: &mut LayoutSession<'_>,
    child: usize,
    inputs: LayoutInput,
    run: &ColumnFloatRun,
    column_width: f32,
    rtl: bool,
    margin_top: f32,
    width: f32,
) -> LayoutOutput {
    use crate::layout::dispatch::block_flow::context::BlockFormattingContext;

    let mut owner = BlockFormattingContext::new();
    let mut root = owner.root_block_context();
    replay_column_floats(&mut root, run, column_width);
    let (margin_left, margin_right) =
        column_horizontal_margins(&style_of(tree, child), column_width, width);
    let left = if rtl {
        column_width - margin_right - width
    } else {
        margin_left
    };
    let mut block = root.sub_context(
        run.next - run.start + margin_top,
        [left, (column_width - left - width).max(0.0)],
    );
    block.set_origin_resolved(true);
    tree.cache_clear_subtree(NodeId::from(child));
    tree.compute_child_layout_with_block_ctx(NodeId::from(child), inputs, Some(&mut block))
}

/// How an in-flow child of a column container goes after the floats before it
/// in its column.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BesideFloats {
    /// A block that shares their context: its lines flow beside them
    /// ([`flows_beside_column_floats`]).
    Flows,
    /// A box with a context of its own: its border box sits beside them
    /// where it fits ([`sits_beside_column_floats`]).
    Sits,
}

impl BesideFloats {
    /// How `child` of the column container `parent` goes after floats, if
    /// it goes beside them at all.
    fn of(tree: &LayoutSession<'_>, parent: usize, child: usize) -> Option<Self> {
        if flows_beside_column_floats(tree, parent, child) {
            Some(Self::Flows)
        } else if sits_beside_column_floats(tree, parent, child) {
            Some(Self::Sits)
        } else {
            None
        }
    }
}

/// Whether `child` of the column container `parent` is an in-flow block-level
/// box that establishes a formatting context of its own, such as one with
/// `overflow: hidden` or `display: flow-root`. Its border box goes beside the
/// floats before it in its column where it fits, and below them where it
/// does not (CSS 2, 9.5).
fn sits_beside_column_floats(tree: &LayoutSession<'_>, parent: usize, child: usize) -> bool {
    let node = layout_node(tree, child);
    node.kind() == NodeKind::Element
        && matches!(
            node.display(),
            DisplayValue::Block
                | DisplayValue::ListItem
                | DisplayValue::FlowRoot
                | DisplayValue::Flex
                | DisplayValue::Grid
        )
        && !style_of(tree, child).float().is_floated()
        // A nested column container keeps its own placement in the columns.
        && tree.nodes[child].multicol.is_none()
        && !crate::layout::dispatch::block_flow::child_shares_parent_context(
            tree,
            NodeId::from(parent),
            NodeId::from(child),
        )
}

/// A child laid out after the floats of its column: its layout, the block
/// offset in the column where its top margin starts, and, for a box that sits
/// beside the floats, the offset of its border box from the left edge of the
/// column.
struct AfterColumnFloats {
    output: LayoutOutput,
    top: f32,
    x: Option<f32>,
}

/// Lay out `child`, which goes `kind` of beside floats, after the floats of
/// `run`: a block that shares their context at `run.next` with its lines
/// beside them, and a box with a context of its own in the first room beside
/// or below them that its border box fits, as block flow places it. `width`
/// is the border-box width the child has in the whole column.
#[allow(clippy::too_many_arguments)]
fn layout_after_column_floats(
    tree: &mut LayoutSession<'_>,
    kind: BesideFloats,
    child: usize,
    inputs: LayoutInput,
    run: &ColumnFloatRun,
    column_width: f32,
    rtl: bool,
    margin_top: f32,
    width: f32,
) -> AfterColumnFloats {
    use crate::layout::dispatch::block_flow::context::{BlockFormattingContext, BoxProbe};

    if kind == BesideFloats::Flows {
        return AfterColumnFloats {
            output: layout_beside_column_floats(
                tree,
                child,
                inputs,
                run,
                column_width,
                rtl,
                margin_top,
                width,
            ),
            top: run.next,
            x: None,
        };
    }
    let mut owner = BlockFormattingContext::new();
    let mut root = owner.root_block_context();
    replay_column_floats(&mut root, run, column_width);
    let style = style_of(tree, child);
    let (margin_left, margin_right) = column_definite_margins(&style, column_width);
    let margins = [margin_left.unwrap_or(0.0), margin_right.unwrap_or(0.0)];
    let direction = if rtl { Direction::Rtl } else { Direction::Ltr };
    let clear = style.clear();
    // An `auto` width fills the room the floats leave, held to the limits
    // block flow holds it to, as it fills the column; any other width is the
    // one the box has in the column.
    let stretches = inputs.known_dimensions.width.is_some();
    let at_width = |width: f32| {
        let mut input = inputs;
        input.known_dimensions.width = Some(width);
        input.available_space.width = AvailableSpace::Definite(width);
        input
    };
    let placed = root.find_box_slot(
        run.next - run.start + margin_top,
        margins,
        direction,
        clear,
        |probe| match probe {
            BoxProbe::Width { stretch } => {
                if stretches {
                    crate::layout::dispatch::block_flow::clamp_child_width(
                        tree,
                        NodeId::from(child),
                        LayoutInput {
                            parent_size: Size {
                                width: Some(column_width),
                                ..inputs.parent_size
                            },
                            ..inputs
                        },
                        stretch.max(0.0),
                    )
                } else {
                    width
                }
            }
            BoxProbe::Height { width, .. } => {
                let mut input = at_width(width);
                input.run_mode = RunMode::ComputeSize;
                tree.compute_child_layout(NodeId::from(child), input)
                    .size
                    .height
            }
        },
    );
    let slot = placed.slot;
    // Auto margins share the room the box leaves, and otherwise a
    // right-to-left column puts a narrower box at the right edge of the room.
    // The room keeps the margin on the trailing side, as block flow keeps it.
    let (area_start, area_width) = placed.stretch_room(direction);
    let free = (area_width - placed.width).max(0.0);
    let x = area_start
        + match (margin_left, margin_right) {
            (None, None) => free / 2.0,
            (None, Some(_)) => free,
            (Some(_), None) => 0.0,
            (Some(_), Some(_)) if rtl => free,
            (Some(_), Some(_)) => 0.0,
        };
    let top = run.start + slot.y - margin_top;
    // Below some of the floats, less of the column is left.
    let mut input = at_width(placed.width);
    if let AvailableSpace::Definite(height) = input.available_space.height {
        input.available_space.height =
            AvailableSpace::Definite((height - (top - run.next)).max(0.0));
    }
    let output = tree.compute_child_layout(NodeId::from(child), input);
    AfterColumnFloats {
        output,
        top,
        x: Some(x),
    }
}

/// Whether `child`, `needed` high in its column, leaves the line of floats
/// before it open: the white space between two floats, or an empty block, has
/// no block size and clears nothing, so a float after it is no lower than the
/// floats before it (CSS 2, 9.5.1).
fn keeps_float_line(tree: &LayoutSession<'_>, child: usize, needed: f32) -> bool {
    needed == 0.0 && style_of(tree, child).clear() == Clear::None
}

/// The block size the measured children of a column container take one below
/// the other in a column, added one at a time: floats that share a line count
/// once, and the in-flow blocks after floats start beside them (CSS 2, 9.5).
#[derive(Default)]
struct ColumnStack {
    total: f32,
    /// The floats of the last children, when they are floats.
    line: Option<FloatLine>,
    /// The floats the last in-flow blocks flowed beside.
    run: Option<ColumnFloatRun>,
}

impl ColumnStack {
    /// Add `float`, `needed` high, in a column of `column_width`. Returns
    /// where its top margin starts.
    fn float(&mut self, float: ColumnFloat, needed: f32, column_width: f32) -> f32 {
        let mut floats = match (self.line.take(), self.run.take()) {
            (Some(line), _) => line,
            (None, Some(run)) => run.into_line(),
            (None, None) => FloatLine::at(0, self.total),
        };
        let spot = floats.place(float, needed, column_width);
        self.total = self.total.max(spot.top + needed);
        floats.push(float, needed, spot, column_width);
        self.line = Some(floats);
        spot.top
    }

    /// The floats an in-flow block added next starts beside, if it starts
    /// above the lowest of them.
    fn beside(&mut self) -> Option<&ColumnFloatRun> {
        if self.run.is_none() {
            self.run = self.line.as_ref().map(ColumnFloatRun::of);
        }
        self.run
            .as_ref()
            .filter(|run| run.next + LINE_EPSILON < run.bottom)
    }

    /// Add an in-flow block `needed` high, which starts beside the floats of
    /// [`Self::beside`] when `beside` is set and below them otherwise.
    /// `keeps_line` says whether a float after it may still join the line of
    /// the floats before it. Returns where it starts.
    fn block(&mut self, needed: f32, beside: bool, keeps_line: bool) -> f32 {
        let top = match self.run.take() {
            Some(mut run) if beside => {
                let top = run.next;
                run.next += needed;
                self.total = self.total.max(run.next);
                self.run = Some(run);
                top
            }
            run => {
                let top = run.map_or(self.total, |run| self.total.max(run.bottom));
                self.total = top + needed;
                top
            }
        };
        if !keeps_line {
            self.line = None;
        }
        top
    }

    /// The block size of all the children added.
    fn height(&self) -> f32 {
        self.run
            .as_ref()
            .map_or(self.total, |run| self.total.max(run.bottom))
    }
}

/// A measured child of a column container where it stands in one column:
/// its top and bottom, and the extents of its lines, between which a column
/// may end, when it is a paragraph.
struct StackedChild {
    top: f32,
    bottom: f32,
    lines: Vec<(f32, f32)>,
}

impl StackedChild {
    /// A child that goes whole into one column.
    fn atomic(top: f32, needed: f32) -> Self {
        Self {
            top,
            bottom: top + needed,
            lines: Vec::new(),
        }
    }

    /// `child`, `needed` high with its top margin `margin_top`, at `top`.
    fn of(tree: &LayoutSession<'_>, child: usize, top: f32, margin_top: f32, needed: f32) -> Self {
        let layout = tree.nodes[child].layout;
        let content = top + margin_top + layout.border.top + layout.padding.top;
        let lines = tree.nodes[child]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .map(|lines| {
                lines
                    .lines
                    .iter()
                    .map(|line| {
                        let start = content + line.block_offset();
                        (start, start + line.block_size())
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            top,
            bottom: top + needed,
            lines,
        }
    }
}

/// How `children` fill `count` columns `height` tall: whether they fit, and
/// how tall the tallest column is. A column ends before a line or a child
/// that would reach past its end, unless the line or the child starts it.
/// A break between the lines of a paragraph leaves at least `orphans` of
/// them before it and `widows` after it where the paragraph has that many,
/// and goes before the whole paragraph when it cannot.
fn fill_stacked_columns(
    children: &[StackedChild],
    count: usize,
    height: f32,
    (orphans, widows): (usize, usize),
) -> (bool, f32) {
    let mut column = 0;
    let mut offset = 0.0f32;
    let mut used = 0.0f32;
    let overflows = |start: f32, end: f32, offset: f32| {
        end - offset > height + LINE_EPSILON && start - offset > LINE_EPSILON
    };
    for child in children {
        let lines = &child.lines;
        if lines.is_empty() {
            if overflows(child.top, child.bottom, offset) {
                column += 1;
                offset = child.top;
            }
            used = used.max(child.bottom - offset);
            continue;
        }
        // The first line of the piece of the paragraph in this column.
        let mut piece = 0;
        let mut line = 0;
        while line < lines.len() {
            let (start, end) = lines[line];
            if !overflows(start, end, offset) {
                used = used.max(end - offset);
                line += 1;
                continue;
            }
            // Move the break up to where it keeps the orphans and widows.
            let mut at = line;
            while at > piece && (at - piece < orphans || lines.len() - at < widows) {
                at -= 1;
            }
            // A break that keeps no line of the piece here goes before it,
            // unless the piece starts the column.
            if at == piece && lines[piece].0 - offset <= LINE_EPSILON {
                at = line;
            }
            column += 1;
            offset = lines[at].0;
            piece = at;
            line = at;
        }
        // The edges and margin below the last line.
        used = used.max(child.bottom - offset);
    }
    (column < count, used)
}

/// The height of `count` balanced columns holding `children`, which take
/// `total` in one column: the lowest that fits them all, found to within
/// a thousandth of a pixel or the precision of the heights.
fn balanced_stack_height(
    children: &[StackedChild],
    count: usize,
    total: f32,
    breaks: (usize, usize),
) -> f32 {
    let (mut low, mut high) = (total / count.max(1) as f32, total);
    let (fits, used) = fill_stacked_columns(children, count, low, breaks);
    if fits {
        return used;
    }
    loop {
        let middle = (low + high) / 2.0;
        // Tall columns can have no float between the bounds to try.
        if high - low <= 0.001 || middle <= low || middle >= high {
            break;
        }
        if fill_stacked_columns(children, count, middle, breaks).0 {
            high = middle;
        } else {
            low = middle;
        }
    }
    fill_stacked_columns(children, count, high, breaks).1
}

#[allow(clippy::too_many_arguments)]
fn layout_column_group(
    tree: &mut LayoutSession<'_>,
    index: usize,
    children: &[usize],
    source_order: usize,
    container_fragment: usize,
    context: FragmentationContext,
    border_box_size: Size<f32>,
    content_origin: Point<f32>,
    block_end_inset: f32,
    retain_group: bool,
) -> f32 {
    let fallback_height = border_box_size.height;
    // Definite-height columns place block children sequentially. For
    // auto-height columns, the selected fill mode controls balancing below.
    let fragment_height = context.available_height;
    let mut column = 0usize;
    let mut cursor = 0.0f32;
    let mut maximum = 0.0f32;
    // For balance-filled auto-height multicol containers, measure block
    // children once before placing them. This gives the simple balancing pass
    // a target height; measuring against the current column would otherwise
    // keep every child in the first column and leave the container taller
    // than necessary.
    // Paragraphs in definite-height columns are placed by the same planner,
    // so that a paragraph broken between columns hands its end to the next.
    let definite_paragraphs = fragment_height.is_some()
        && tree.context.fragmentation_stack.len() <= 1
        && multicol_has_only_paragraph_children(
            tree,
            index,
            context.column_fill != ColumnFillValue::Auto,
        )
        && !multicol_has_nested_descendant(tree, index);
    // The paragraph of a chain of plain wrappers balances in a container with
    // a definite height too: its lines need only as much of the height as
    // they take, so the columns do not run to the container's height.
    let definite_chain = !definite_paragraphs
        && fragment_height.is_some()
        && context.column_fill != ColumnFillValue::Auto
        && tree.context.fragmentation_stack.len() <= 1
        && multicol_has_plain_paragraph_chain(tree, index)
        && !multicol_has_nested_descendant(tree, index);
    // The measured children laid out with their lines beside the floats
    // before them.
    let mut measured_beside: Vec<bool> = Vec::new();
    let auto_measurements = if (context.column_fill != ColumnFillValue::Auto
        && fragment_height.is_none()
        && !multicol_has_nested_descendant(tree, index))
        || definite_paragraphs
        || definite_chain
    {
        let mut measurements = Vec::with_capacity(children.len());
        let mut stack = ColumnStack::default();
        // Where each child stands in one column, for the columns to divide.
        let mut stacked = Vec::new();
        measured_beside = vec![false; children.len()];
        for (local_order, &child) in children.iter().enumerate() {
            let child_inputs = column_child_layout_input(
                tree,
                child,
                context.column_width,
                context.available_height,
                AvailableSpace::MaxContent,
            );
            // A direct text child is measured again against the column
            // width instead of reusing a size cached for another width.
            if matches!(tree.nodes[child].data, NodeData::Text(_)) {
                tree.nodes[child].cache.clear();
            }
            let mut output = tree.compute_child_layout(NodeId::from(child), child_inputs);
            let layout = column_child_layout(tree, child, context.column_width);
            let margin_top = layout.margin.top;
            let margin_bottom = layout.margin.bottom;
            let mut needed = margin_top + output.size.height + margin_bottom;
            if let Some(float) =
                ColumnFloat::of(tree, child, context.column_width, output.size.width)
            {
                let top = stack.float(float, needed, context.column_width);
                stacked.push(StackedChild::atomic(top, needed));
            } else if !(matches!(tree.nodes[child].data, NodeData::Text(_)) && needed == 0.0
                || tree.nodes[child].layout_position() == Position::Absolute)
            {
                // A block after floats is measured with its lines beside them.
                let clear = style_of(tree, child).clear();
                // The room above a box that moved below some of the floats.
                let mut skipped = 0.0;
                let kind = BesideFloats::of(tree, index, child);
                let beside = match (kind, stack.beside().filter(|run| !run.clears(clear))) {
                    (Some(kind), Some(run)) => {
                        let after = layout_after_column_floats(
                            tree,
                            kind,
                            child,
                            child_inputs,
                            run,
                            context.column_width,
                            context.rtl,
                            margin_top,
                            output.size.width,
                        );
                        skipped = after.top - run.next;
                        output = after.output;
                        needed = margin_top + output.size.height + margin_bottom;
                        measured_beside[local_order] = true;
                        true
                    }
                    _ => false,
                };
                let top = stack.block(
                    skipped + needed,
                    beside,
                    keeps_float_line(tree, child, skipped + needed),
                ) + skipped;
                // Only the lines that flow beside floats break between
                // columns where they are; any other block goes whole.
                stacked.push(if beside && splits_beside_floats(tree, child, kind) {
                    StackedChild::of(tree, child, top, margin_top, needed)
                } else {
                    StackedChild::atomic(top, needed)
                });
            }
            measurements.push((child, output, layout, margin_top, margin_bottom, needed));
        }
        let total = stack.height();
        let largest = measurements.iter().map(|entry| entry.5).fold(0.0, f32::max);
        let target = if measured_beside.contains(&true) && fragment_height.is_none() {
            // Lines beside floats break between columns where they are, so
            // the columns are as tall as the lowest height that fits them all.
            balanced_stack_height(
                &stacked,
                context.column_count,
                total,
                (context.orphans.max(1), context.widows.max(1)),
            )
        } else {
            (total / context.column_count as f32).max(largest)
        };
        Some((measurements, target))
    } else {
        None
    };
    // One ordinary paragraph can break inside its lines. Measuring its whole
    // block as an atomic item would keep the container at the unbroken height
    // while the inline post-pass already assigns lines to several columns.
    // The planners place lines and boxes in whole columns; none of them
    // narrows lines beside a float.
    let floats_narrow_lines = measured_beside.contains(&true);
    let group_balance = auto_measurements
        .as_ref()
        .filter(|_| !floats_narrow_lines)
        .and_then(|(entries, _)| match fragment_height {
            Some(height) => paragraph_group::fill_definite(tree, entries, context, height),
            None => paragraph_group::balance(tree, entries, context),
        });
    let paragraph_balance = auto_measurements
        .as_ref()
        .filter(|_| !floats_narrow_lines)
        .and_then(|(entries, _)| {
            if fragment_height.is_some() && !definite_chain {
                return None;
            }
            let mut boxes = entries.iter().filter(|(child, output, _, _, _, _)| {
                tree.nodes[*child].kind() != NodeKind::Text
                    || output.size.height != 0.0
                    || tree.nodes[*child].ifc.is_some()
            });
            let (child, _, _, _, _, _) = boxes.next()?;
            if boxes.next().is_some() {
                return None;
            }
            // A paragraph below plain wrappers is balanced like one of the
            // container's own.
            let leaf = if tree.nodes[*child].ifc.is_some() {
                *child
            } else if tree.nodes[*child].paints_background_color {
                painted_wrapper_leaf(tree, *child)?
            } else {
                plain_paragraph_chain_leaf(tree, *child)?
            };
            if !can_balance_atomic_paragraph(tree, leaf, BlockEdges::Refused) {
                return None;
            }
            let lines = tree.nodes[leaf].ifc.as_ref()?.lines.as_ref()?;
            let (fragments, height) = root_column_fragments(lines, context);
            // Lines that need all of a definite height go in turn, as they did.
            if let Some(definite) = fragment_height {
                balanced_below(height, definite)?;
            }
            let (first, last) = (lines.lines.first()?, lines.lines.last()?);
            let stacked = last.block_offset() + last.block_size() - first.block_offset();
            (fragments.len() > 1).then_some(ParagraphBalance {
                child: *child,
                leaf,
                fragments,
                height,
                stacked,
            })
        });
    let mut avoid_column_break_after_previous = false;
    let mut occupied = std::collections::BTreeSet::new();
    // The line of floats the last child belongs to, if it is a float.
    let mut float_line: Option<FloatLine> = None;
    // The floats the in-flow blocks after them flow beside.
    let mut float_run: Option<ColumnFloatRun> = None;
    for (local_order, &child) in children.iter().enumerate() {
        let order = source_order + local_order;
        let measured = auto_measurements
            .as_ref()
            .and_then(|(entries, _)| entries.get(local_order));
        let (mut child_output, mut child_layout, margin_top, margin_bottom, needed) =
            if let Some((_, output, layout, margin_top, margin_bottom, needed)) = measured {
                (
                    output.clone(),
                    *layout,
                    *margin_top,
                    *margin_bottom,
                    *needed,
                )
            } else {
                let child_height = match fragment_height {
                    Some(height) => AvailableSpace::Definite((height - cursor).max(0.0)),
                    None => AvailableSpace::MaxContent,
                };
                let child_inputs = column_child_layout_input(
                    tree,
                    child,
                    context.column_width,
                    context.available_height,
                    child_height,
                );

                // A direct text child is measured again against the column
                // width instead of reusing a size cached for another width.
                if matches!(tree.nodes[child].data, NodeData::Text(_)) {
                    tree.nodes[child].cache.clear();
                }
                let output = tree.compute_child_layout(NodeId::from(child), child_inputs);
                if tree.fragment_tree.limit_exceeded {
                    return fallback_height;
                }
                refresh_nested_text_fragments(tree, child, context);
                let layout = column_child_layout(tree, child, context.column_width);
                let margin_top = layout.margin.top;
                let margin_bottom = layout.margin.bottom;
                let needed = margin_top + output.size.height + margin_bottom;
                (output, layout, margin_top, margin_bottom, needed)
            };
        let planned = group_balance
            .as_ref()
            .and_then(|group| group.placement_of(local_order));
        let planned = planned.as_ref();
        if let Some(placement) = planned {
            column = placement.first_column;
            cursor = placement.first_y - margin_top;
            child_output.size.height = placement.first_height;
        } else if let Some(balance) = &paragraph_balance
            && balance.child == child
        {
            child_output.size.height = balance.height;
        }
        let break_height =
            fragment_height.or_else(|| auto_measurements.as_ref().map(|(_, height)| *height));
        let avoid_column_break = avoid_column_break_after_previous
            || matches!(tree.nodes[child].break_before, BreakBetween::Avoid);
        // A nested column container taller than the rest of this column
        // continues in the following ones. When nothing of it fits here, it
        // starts at the top of the next column instead.
        let mut continuation = None;
        if let Some(height) = fragment_height
            && measured.is_none()
            && planned.is_none()
        {
            continuation = plan_nested_continuation(
                tree,
                child,
                context,
                cursor,
                height,
                margin_top,
                child_output.size,
                !avoid_column_break,
            );
            // Running out of fragments while planning from here ends the
            // layout only if starting in the next column does not help, so
            // it is held back until that has been tried. A container that
            // fits a column whole needs no rows there, and moves below.
            let mut exhausted = std::mem::take(&mut tree.fragment_tree.limit_exceeded);
            let mut fits_next_column = false;
            if continuation.is_none()
                && cursor > 0.0
                && cursor + needed > height
                && !avoid_column_break
            {
                fits_next_column = needed <= height;
                let moved = plan_nested_continuation(
                    tree,
                    child,
                    context,
                    0.0,
                    height,
                    margin_top,
                    child_output.size,
                    false,
                );
                exhausted |= std::mem::take(&mut tree.fragment_tree.limit_exceeded);
                if let Some(plan) = moved {
                    maximum = maximum.max(cursor);
                    tree.fragment_tree
                        .record_break(crate::fragment::BreakToken {
                            node_id: index,
                            child_index: order,
                            line_index: 0,
                        });
                    column += 1;
                    cursor = 0.0;
                    continuation = Some(plan);
                }
            }
            if exhausted && continuation.is_none() && !fits_next_column {
                tree.fragment_tree.limit_exceeded = true;
                return fallback_height;
            }
        }
        if let Some(plan) = &continuation {
            child_output.size.height = plan.first_height;
        }
        // An in-flow block after floats starts where they do, and its lines
        // flow beside them (CSS 2, 9.5). One that cannot, or that would not
        // fit the column so, starts below them.
        let mut needed = needed;
        let mut beside_floats = None;
        // Where the border box of a box with a formatting context of its own
        // starts in the room beside the floats.
        let mut slot_x = None;
        // The room above it that a box placed below some of the floats skips.
        let mut skipped = 0.0;
        // Where the content of a paragraph whose lines go on in the next
        // columns starts in its column.
        let mut split_lines_at = None;
        // White space and boxes out of the flow take no room in it.
        let gap = matches!(tree.nodes[child].data, NodeData::Text(_)) && needed == 0.0
            || tree.nodes[child].layout_position() == Position::Absolute;
        let mut laid_beside = measured_beside.get(local_order).copied().unwrap_or(false);
        if !gap {
            let flows = (planned.is_none()
                && continuation.is_none()
                && group_balance.is_none()
                && paragraph_balance.is_none())
            .then(|| BesideFloats::of(tree, index, child))
            .flatten();
            if flows.is_some()
                && float_run.is_none()
                && let Some(line) = float_line.as_ref().filter(|line| line.column == column)
            {
                float_run = Some(ColumnFloatRun::of(line));
            }
            // A float after the blocks goes among the floats they flowed
            // beside, below the line the last of them ends on.
            if float_line.is_none()
                && is_floated(tree, child)
                && let Some(run) = float_run.take_if(|run| run.column == column)
            {
                cursor = cursor.max(run.bottom);
                float_line = Some(run.into_line());
            }
            if let Some(run) = float_run.take() {
                let mut kept = None;
                if let Some(kind) = flows
                    && run.column == column
                    && run.next + LINE_EPSILON < run.bottom
                    && !run.clears(style_of(tree, child).clear())
                {
                    let child_inputs = column_child_layout_input(
                        tree,
                        child,
                        context.column_width,
                        context.available_height,
                        match fragment_height.filter(|_| measured.is_none()) {
                            Some(height) => AvailableSpace::Definite((height - run.next).max(0.0)),
                            None => AvailableSpace::MaxContent,
                        },
                    );
                    let after = layout_after_column_floats(
                        tree,
                        kind,
                        child,
                        child_inputs,
                        &run,
                        context.column_width,
                        context.rtl,
                        margin_top,
                        child_output.size.width,
                    );
                    laid_beside = true;
                    let wrapped = margin_top + after.output.size.height + margin_bottom;
                    let content_top = splits_beside_floats(tree, child, Some(kind))
                        .then_some(after.top + margin_top)
                        .filter(|&content_top| {
                            break_height.is_some_and(|height| {
                                first_piece_fits(tree, child, content_top, height, context.orphans)
                            })
                        });
                    // A block at the top of a column stays there, and its
                    // lines past the column's end go to the next ones, as
                    // they would below no float. A paragraph whose first
                    // line fits the rest of its column breaks there too.
                    // Elsewhere a block fits as the column break below
                    // measures it.
                    if after.top <= 0.0
                        || content_top.is_some()
                        || break_height.is_none_or(|height| after.top + wrapped <= height)
                    {
                        split_lines_at = content_top;
                        refresh_nested_text_fragments(tree, child, context);
                        child_output = after.output;
                        needed = wrapped;
                        beside_floats = Some(after.top);
                        skipped = after.top - run.next;
                        slot_x = after.x;
                        kept = Some(ColumnFloatRun {
                            next: after.top + wrapped,
                            ..run
                        });
                    }
                }
                if kept.is_none() && run.column == column {
                    cursor = cursor.max(run.bottom);
                }
                float_run = kept;
            }
        }
        // A block laid out beside floats that it does not end up beside is
        // laid out again without them.
        if beside_floats.is_none() && laid_beside {
            tree.cache_clear_subtree(NodeId::from(child));
            let child_inputs = column_child_layout_input(
                tree,
                child,
                context.column_width,
                context.available_height,
                fragment_height
                    .filter(|_| measured.is_none())
                    .map_or(AvailableSpace::MaxContent, |height| {
                        AvailableSpace::Definite((height - cursor).max(0.0))
                    }),
            );
            child_output = tree.compute_child_layout(NodeId::from(child), child_inputs);
            refresh_nested_text_fragments(tree, child, context);
            needed = margin_top + child_output.size.height + margin_bottom;
        }
        // A float goes beside the floats before it where it fits, instead of
        // below them like a block (CSS 2, 9.5.1).
        let column_float =
            ColumnFloat::of(tree, child, context.column_width, child_output.size.width);
        let mut beside = column_float.and_then(|float| {
            float_line
                .as_ref()
                .filter(|line| line.column == column && planned.is_none() && continuation.is_none())
                .map(|line| line.place(float, needed, context.column_width))
        });
        // Content past the last column of a container with a definite
        // height continues in overflow columns (CSS Multi-column 1, 8.2).
        let from = beside_floats.unwrap_or_else(|| beside.map_or(cursor, |spot| spot.top));
        if let Some(height) = break_height
            && continuation.is_none()
            && group_balance.is_none()
            && (column + 1 < context.column_count || fragment_height.is_some())
            && from > 0.0
            && from + needed > height
            && !avoid_column_break
            && split_lines_at.is_none()
        {
            maximum = maximum.max(cursor);
            tree.fragment_tree
                .record_break(crate::fragment::BreakToken {
                    node_id: index,
                    child_index: order,
                    line_index: 0,
                });
            column += 1;
            cursor = 0.0;
            beside = None;
        }
        let top = beside_floats.unwrap_or_else(|| beside.map_or(cursor, |spot| spot.top));

        let column_context = context.in_column(
            column,
            context.origin_x + context.column_offset_x(column),
            context.origin_y + top,
        );
        // The room the floats before the child leave it, which is all of the
        // column when it goes beside none.
        let spot = beside.unwrap_or(FloatSpot {
            top,
            left: 0.0,
            right: 0.0,
        });
        // Block flow resolved the margins against the whole container; they
        // resolve against the column the child ends up in.
        let (margin_left, margin_right, x) = {
            let style = style_of(tree, child);
            let float = style.float();
            let (margin_left, margin_right) =
                column_horizontal_margins(&style, context.column_width, child_output.size.width);
            // A right-to-left container aligns its blocks to the right edge
            // of the column, leaving the room a narrower block does not use
            // to its left. A float sits at the edge of its own side in either
            // direction.
            let flush_right = match float {
                Float::Left => false,
                Float::Right => true,
                Float::None => context.rtl,
            };
            // `justify-self` places a block in the room of the column the
            // floats leave it.
            let justified = (!float.is_floated() && slot_x.is_none())
                .then(|| {
                    let free = context.column_width
                        - spot.left
                        - spot.right
                        - margin_left
                        - margin_right
                        - child_output.size.width;
                    crate::layout::dispatch::block_flow::justified_offset(
                        tree,
                        child,
                        context.rtl,
                        free,
                    )
                })
                .flatten();
            let x = if let Some(slot_x) = slot_x {
                content_origin.x + context.column_offset_x(column) + slot_x
            } else if let Some(offset) = justified {
                content_origin.x
                    + context.column_offset_x(column)
                    + spot.left
                    + margin_left
                    + offset
            } else if flush_right {
                content_origin.x + context.right_flush_offset_x(column, child_output.size.width)
                    - margin_right
                    - spot.right
            } else if float.is_floated() && float_line.is_some() {
                // Block flow placed a float after another against the floats
                // it saw at the width of the whole container, so it is placed
                // from the edge of the column here.
                content_origin.x + context.column_offset_x(column) + spot.left + margin_left
            } else {
                // Block flow placed the child with its `position: relative`
                // inset. The painter adds that inset to a box placed in a
                // column and to everything inside it, so it is taken out of
                // the location here. A float is placed without the inset, so
                // its location has none to take out.
                let inset = if float.is_floated() {
                    0.0
                } else {
                    relative_inline_offset(&style, context.available_width)
                };
                child_layout.location.x - inset
                    + (margin_left - child_layout.margin.left)
                    + context.column_offset_x(column)
            };
            (margin_left, margin_right, x)
        };
        let y = content_origin.y + top + margin_top;
        // An empty block does not end the line, but a float with no height
        // still holds the top of the floats after it (CSS 2, 9.5.1 rule 5).
        if column_float.is_some() || !keeps_float_line(tree, child, skipped + needed) {
            float_line = column_float.map(|float| {
                let mut line = match (beside, float_line.take()) {
                    (Some(_), Some(line)) => line,
                    _ => FloatLine::at(column, top),
                };
                line.push(float, needed, spot, context.column_width);
                line
            });
        }
        child_layout.order = order as u32;
        child_layout.margin.left = margin_left;
        child_layout.margin.right = margin_right;
        child_layout.size = child_output.size;
        child_layout.location = Point { x, y };
        tree.set_unrounded_layout(NodeId::from(child), &child_layout);
        leave_block_offset_to_painter(tree, child);
        if let Some(placement) = planned {
            if let Some(root) = tree.nodes[child].ifc.as_mut() {
                root.multicol_fragments = Some(placement.fragments.clone());
                root.multicol_fragment_origins_recorded = true;
            }
            refresh_paragraph_boxes(
                tree,
                child,
                column_context,
                FlexFloatChain::of_ancestors(tree, child),
            );
            if let Some(group) = &group_balance {
                occupied.extend(place_wrapped_boxes(tree, group, local_order, context));
            }
        } else {
            // A block measured as one item of the balance stays whole in its
            // column, so no line of it may go to a later column.
            let whole_in_column = fragment_height.is_none() && measured.is_some();
            // A paragraph that starts down its column beside floats goes
            // on in the next columns from where its lines end in this one.
            let split = split_lines_at.zip(break_height);
            refresh_nested_text_fragments(
                tree,
                child,
                if let Some((_, height)) = split {
                    FragmentationContext {
                        available_height: Some(height),
                        ..column_context
                    }
                } else if whole_in_column {
                    FragmentationContext {
                        column_count: column_context.column_index + 1,
                        ..column_context
                    }
                } else {
                    column_context
                },
            );
            if let Some((content_top, height)) = split
                && let Some(root) = tree.nodes[child].ifc.as_mut()
                && let Some(fragments) = lines_below_column_top(
                    root,
                    content_top,
                    height,
                    column_context,
                    fragment_height.is_none(),
                )
            {
                root.multicol_fragments = Some(fragments);
                refresh_paragraph_boxes(
                    tree,
                    child,
                    column_context,
                    FlexFloatChain::of_ancestors(tree, child),
                );
            }
        }
        if let Some(balance) = &paragraph_balance
            && balance.child == child
            && let Some(root) = tree.nodes[balance.leaf].ifc.as_mut()
        {
            root.multicol_fragments = Some(balance.fragments.clone());
            refresh_paragraph_boxes(
                tree,
                balance.leaf,
                column_context,
                FlexFloatChain::of_ancestors(tree, balance.leaf),
            );
        }
        let Some(child_fragment) = tree
            .fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: child,
                parent: Some(container_fragment),
                fragmentainer: column_context.column_index,
                rect: crate::fragment::FragmentRect {
                    x: child_layout.location.x,
                    y,
                    width: child_output.size.width,
                    height: child_output.size.height,
                },
                fragmentainer_clip: None,
                fragment_index: 0,
                fragment_count: 1,
                line_start: None,
                line_end: None,
            })
        else {
            return fallback_height;
        };
        tree.fragment_tree.reparent_roots(child, child_fragment);
        // A box with no lines that the columns sliced has a piece in each
        // column it reaches after the first.
        if let (Some(placement), Some((_, output, ..)), Some(group)) =
            (planned, measured, group_balance.as_ref())
            && placement.sliced
            && placement.fragments.len() > 1
            && push_sliced_box_pieces(
                tree,
                child,
                child_fragment,
                placement,
                output.size.height,
                group.height,
            )
            .is_none()
        {
            return fallback_height;
        }
        // The boxes inside a wrapper whose boxes joined the flow have their
        // pieces in the columns the flow gave them.
        let wrapped = match group_balance.as_ref() {
            Some(group)
                if group.flow.is_some()
                    && planned.is_some()
                    && paragraph_group::is_flow_wrapper(tree, child) =>
            {
                if record_wrapped_fragments(tree, group, local_order, child_fragment, context)
                    .is_none()
                {
                    return fallback_height;
                }
                true
            }
            _ => false,
        };
        if let Some(plan) = &continuation {
            if apply_nested_continuation(tree, plan).is_none() {
                return fallback_height;
            }
            occupied.extend(column..=column + plan.columns);
            column += plan.columns;
            cursor = plan.end + margin_bottom;
            maximum = maximum.max(fragment_height.unwrap_or(0.0));
            if tree.nodes[child].kind() == NodeKind::Element {
                avoid_column_break_after_previous =
                    matches!(tree.nodes[child].break_after, BreakBetween::Avoid);
            }
            continue;
        }
        let record_context = group_balance.as_ref().map_or_else(
            || {
                paragraph_balance
                    .as_ref()
                    .filter(|balance| balance.child == child)
                    .map(|balance| balance.height)
                    .or(split_lines_at.and(break_height))
                    .map_or(context, |height| FragmentationContext {
                        available_height: Some(height),
                        ..context
                    })
            },
            |group| FragmentationContext {
                available_height: Some(group.height),
                column_index: column,
                ..context
            },
        );
        let record_context = FragmentationContext {
            origin_y: content_origin.y,
            ..record_context
        };
        if !wrapped
            && record_nested_ifc_box_fragments(
                tree,
                child,
                child_fragment,
                record_context,
                planned.is_some(),
            )
            .is_none()
        {
            return fallback_height;
        }
        // The lines of a balanced paragraph below wrappers occupy the
        // columns of the wrappers.
        let paragraph = paragraph_balance
            .as_ref()
            .filter(|balance| balance.child == child)
            .map_or(child, |balance| balance.leaf);
        if let Some(root) = tree.nodes[paragraph].ifc.as_ref()
            && let Some(ranges) = &root.multicol_fragments
        {
            occupied.extend(
                ranges
                    .iter()
                    .filter(|range| range.line_start < range.line_end)
                    .map(|range| range.fragmentainer),
            );
        } else if child_output.size.width > 0.0 && child_output.size.height > 0.0 {
            occupied.insert(column);
        }
        if let Some(placement) = planned {
            column = placement.last_column;
            cursor = placement.last_y;
            let group_height = group_balance.as_ref().map_or(0.0, |group| group.height);
            maximum = maximum.max(group_height);
            // The margin below the last box of balanced columns stays inside
            // them: it counted in the height divided among the columns, and
            // does not add to it.
            if fragment_height.is_none() {
                cursor = cursor.min(group_height);
            }
        } else if let Some((last_column, end)) = (definite_paragraphs || split_lines_at.is_some())
            .then(|| paragraph_end_in_later_column(tree, child, column))
            .flatten()
        {
            // A paragraph broken between definite-height columns hands the
            // end of its last piece to the next box.
            maximum = maximum.max(fragment_height.or(break_height).unwrap_or(0.0));
            column = last_column;
            cursor = end + margin_bottom;
        } else {
            let bottom = y - content_origin.y + child_output.size.height + margin_bottom;
            // The floats of a line end where the lowest of them does.
            cursor = if beside.is_some() {
                cursor.max(bottom)
            } else {
                bottom
            };
            if let Some(balance) = &paragraph_balance
                && balance.child == child
            {
                // The columns are as tall as the paragraph and its margins in
                // one column divided among them, or as the tallest piece when
                // that is more; the margin below the paragraph does not add to
                // them.
                let columns = context.column_count.saturating_sub(context.column_index);
                let divided = (margin_top + balance.stacked + margin_bottom) / columns as f32;
                cursor = (cursor - margin_bottom).max(divided);
            }
        }
        if tree.nodes[child].kind() == NodeKind::Element {
            avoid_column_break_after_previous =
                matches!(tree.nodes[child].break_after, BreakBetween::Avoid);
        }
    }
    // The floats the last blocks flowed beside reach below them.
    if let Some(run) = float_run
        && run.column == column
    {
        cursor = cursor.max(run.bottom);
    }
    if retain_group {
        tree.nodes[index]
            .multicol_groups
            .push(crate::fragment::MulticolGroup {
                context: FragmentationContext {
                    origin_x: content_origin.x,
                    origin_y: content_origin.y,
                    ..context
                },
                height: maximum.max(cursor),
                occupied,
                slice: None,
            });
    }
    let minimum_height = if auto_measurements.is_some()
        && fragment_height.is_none()
        && !multicol_has_nested_descendant(tree, index)
    {
        0.0
    } else {
        fragment_height
            .map(|height| fallback_height.min(height))
            .unwrap_or(fallback_height)
    };
    (maximum.max(cursor) + content_origin.y + block_end_inset).max(minimum_height)
}

/// The last column of `child`'s line pieces and the block end of the piece
/// there, from the column's top, when they reach past `column`.
fn paragraph_end_in_later_column(
    tree: &Document,
    child: usize,
    column: usize,
) -> Option<(usize, f32)> {
    let root = tree.nodes[child].ifc.as_ref()?;
    let last = root
        .multicol_fragments
        .as_ref()?
        .iter()
        .rfind(|range| range.line_start < range.line_end)?;
    if last.fragmentainer <= column {
        return None;
    }
    let lines = &root.lines.as_ref()?.lines;
    let first = lines.get(last.line_start)?;
    let end = lines.get(last.line_end - 1)?;
    Some((
        last.fragmentainer,
        end.block_offset() + end.block_size() - first.block_offset(),
    ))
}

/// A nested column container that continues from one column of its outer
/// container into the following ones: its paragraphs fill a row of its own
/// columns in each outer column it reaches. A fragmentation context nested in
/// another is fragmented by the outer one too (CSS Fragmentation 3, 2).
struct NestedContinuation {
    /// The nested container's retained root record.
    root: usize,
    /// The nested container's children, by entry.
    children: Vec<usize>,
    rows: paragraph_group::Rows,
    /// The pieces of children that hold no lines, in place of the paragraph
    /// fragments of `rows`, whose heights they share.
    lineless: Vec<lineless::RowPiece>,
    /// The nested container's own columns.
    context: FragmentationContext,
    /// Its content-box origin in its border box.
    content_origin: Point<f32>,
    /// Whether `box-decoration-break: clone` gives every row's box its own
    /// padding and border (CSS Fragmentation 3, 5.4).
    cloned: bool,
    /// Block size of its block-end padding and border.
    block_end_inset: f32,
    /// Block offset of the outer column's top from the nested border box.
    row_top: f32,
    /// Inline distance between two outer columns.
    row_step: f32,
    /// Border-box block size of its first fragment, which reaches the end of
    /// the outer column.
    first_height: f32,
    /// Outer columns it reaches after the first.
    columns: usize,
    /// Its border-box block end in the last outer column it reaches, from
    /// that column's top.
    end: f32,
}

impl NestedContinuation {
    /// Block offset of the content box in the box of a row after the first:
    /// the block-start padding and border of a box that clones them, else
    /// zero because a continued box that slices has none.
    fn continued_origin_y(&self) -> f32 {
        if self.cloned {
            self.content_origin.y
        } else {
            0.0
        }
    }

    /// Block size of the block-end padding and border that a box that clones
    /// them keeps below the content of every row but the last, else zero.
    fn cloned_end_inset(&self) -> f32 {
        if self.cloned {
            self.block_end_inset
        } else {
            0.0
        }
    }

    /// Border-box block size of the box of a row after the first and before
    /// the last, whose columns are `content` high.
    fn row_box_height(&self, content: f32) -> f32 {
        self.continued_origin_y() + content + self.cloned_end_inset()
    }
}

/// Plan how the nested column container `child`, laid out at `cursor` in a
/// column of `outer`, continues in the following outer columns.
/// Rows continue in outer overflow columns past the last one, as the outer
/// container has a definite height (CSS Multi-column 1, 8.2). `break_before`
/// says a break before the container is allowed, which is preferred to
/// cutting a child that holds no lines at the end of this column. `None`
/// when it fits in the rest of the column, or when its content is neither
/// plain paragraphs nor children without lines, or when the fragment budget
/// cannot hold the rows, which also sets `limit_exceeded` for the caller to
/// weigh against another placement.
#[allow(clippy::too_many_arguments)]
fn plan_nested_continuation(
    tree: &mut LayoutSession<'_>,
    child: usize,
    outer: FragmentationContext,
    cursor: f32,
    fragmentainer_height: f32,
    margin_top: f32,
    border_box_size: Size<f32>,
    break_before: bool,
) -> Option<NestedContinuation> {
    let node = layout_node(tree, child);
    let style = node.multicol?;
    let css = style_of(tree, child);
    // Children that hold no lines are cut at any block offset, so their
    // container may also have a definite height and continue in overflow
    // columns of the outer container (CSS Multi-column 1, 8.2).
    let lineless_children = style
        .horizontal
        .then(|| lineless::supported(tree, child, false))
        .flatten();
    // A box that avoids breaks inside, has a minimum height or generated
    // content keeps its previous layout: the rows below hold its paragraph
    // lines only.
    if (lineless_children.is_none() && (style.height_definite || !css.size().height.is_auto()))
        || !style.horizontal
        || !css.min_size().height.is_auto()
        || !css.max_size().height.is_auto()
        || node.has_logical_min_block_size
        || node.has_before_or_after_content
        || node.break_inside != raikiri_style::property::BreakInside::Auto
    {
        return None;
    }
    let (padding, border) = (css.padding(), css.border());
    let basis = outer.column_width;
    let inset = |value| multicol_resolve_inset(tree, value, basis);
    let content_origin = Point {
        x: inset(padding.left) + inset(border.left),
        y: inset(padding.top) + inset(border.top),
    };
    let block_end_inset = inset(padding.bottom) + inset(border.bottom);
    let content_width =
        border_box_size.width - content_origin.x - inset(padding.right) - inset(border.right);
    // A box that clones its decorations has the block-end padding and border
    // in every fragment, and the block-start ones in every fragment but the
    // first (CSS Fragmentation 3, 5.4). A continued box that slices has
    // neither, so its rows hold content up to the end of the outer column.
    // Padding and border that leave a row no room for content keep the box
    // from continuing in rows.
    let cloned = node.clones_box_decorations;
    let (continued_origin_y, cloned_end_inset) = if cloned {
        (content_origin.y, block_end_inset)
    } else {
        (0.0, 0.0)
    };
    let first = fragmentainer_height - cursor - margin_top - content_origin.y - cloned_end_inset;
    let later = fragmentainer_height - continued_origin_y - cloned_end_inset;
    let first_height = content_origin.y + first + cloned_end_inset;
    let content_height = border_box_size.height - content_origin.y - block_end_inset;
    if content_height <= first + 0.001 {
        return None;
    }
    let context = FragmentationContext::resolve(content_width, None, style)?;
    let row_top = -(cursor + margin_top);
    let row_step = outer.column_offset_x(1) - outer.column_offset_x(0);
    let max_pieces = tree
        .fragment_tree
        .limit
        .saturating_sub(tree.fragment_tree.fragments.len());
    if let Some(children) = lineless_children {
        let rows = lineless::fill_rows(
            tree,
            &children,
            context.column_count,
            context.column_fill,
            style.height_definite.then_some(content_height),
            first,
            later,
            max_pieces,
        )?;
        // Cutting a box at the end of the outer column is a last resort
        // when content before the container offers a break between boxes
        // (CSS Fragmentation 3, 4.4): the container then starts in the next
        // outer column instead.
        if break_before && cursor > 0.0 && cuts_a_box_at_the_first_row_end(&rows.pieces) {
            return None;
        }
        let root = latest_root(tree, child)?;
        let columns = rows.heights.len() - 1;
        let end = continued_origin_y + rows.heights[columns] + block_end_inset;
        if end > fragmentainer_height + 0.001 {
            return None;
        }
        return Some(NestedContinuation {
            root,
            children,
            rows: paragraph_group::Rows {
                fragments: Vec::new(),
                heights: rows.heights,
            },
            lineless: rows.pieces,
            context,
            content_origin,
            cloned,
            block_end_inset,
            row_top,
            row_step,
            first_height,
            columns,
            end,
        });
    }
    let root = latest_root(tree, child)?;
    let children: Vec<usize> = tree
        .children_through_contents(child)
        .into_iter()
        .filter(|&id| tree.nodes[id].is_in_document() && !tree.nodes[id].is_display_none())
        .collect();
    if children.iter().any(|&id| spanning::is_spanner(tree, id)) {
        return None;
    }
    let entries: Vec<_> = children
        .iter()
        .map(|&id| {
            let layout = tree.nodes[id].layout;
            (
                id,
                LayoutOutput::from_outer_size(layout.size),
                layout,
                layout.margin.top,
                layout.margin.bottom,
                layout.margin.top + layout.size.height + layout.margin.bottom,
            )
        })
        .collect();
    // The outer container has a definite height here, so the rows may also
    // fill overflow columns past its last one. Rows whose records outgrow the
    // fragment budget stop the planning, which reports the exhaustion. The
    // container's own record in the outer column comes before the rows'.
    let rows = match paragraph_group::fill_rows(
        tree,
        &entries,
        context,
        first,
        later,
        max_pieces.saturating_sub(1),
    ) {
        Ok(rows) => rows,
        Err(paragraph_group::RowsStop::Unsupported) => return None,
        Err(paragraph_group::RowsStop::Exhausted) => {
            tree.fragment_tree.limit_exceeded = true;
            return None;
        }
    };
    let columns = rows.heights.len().checked_sub(1).filter(|&n| n > 0)?;
    // The block-end padding and border must fit in the last outer column too.
    let end = continued_origin_y + rows.heights[columns] + block_end_inset;
    if end > fragmentainer_height + 0.001 {
        return None;
    }
    Some(NestedContinuation {
        root,
        children,
        rows,
        lineless: Vec::new(),
        context,
        content_origin,
        cloned,
        block_end_inset,
        row_top,
        row_step,
        first_height,
        columns,
        end,
    })
}

/// Whether a box with a piece in the first row continues in a later one.
fn cuts_a_box_at_the_first_row_end(pieces: &[lineless::RowPiece]) -> bool {
    let first_row: std::collections::HashSet<usize> = pieces
        .iter()
        .filter(|piece| piece.row == 0)
        .map(|piece| piece.entry)
        .collect();
    pieces
        .iter()
        .any(|piece| piece.row > 0 && first_row.contains(&piece.entry))
}

/// The live root record of `node` laid out last.
fn latest_root(tree: &Document, node: usize) -> Option<usize> {
    (0..tree.fragment_tree.fragments.len())
        .rev()
        .find(|&index| {
            let fragment = &tree.fragment_tree.fragments[index];
            fragment.node_id == node
                && fragment.parent.is_none()
                && !tree.fragment_tree.is_retired(index)
        })
}

/// Record the planned rows of a nested column container, in place of the
/// records of its own balanced layout. The container's root is its box in the
/// first row; each later row gets a box of its own in its outer column, so
/// its background, border and overflow clip follow the row. Fragmentainers
/// number the nested columns row by row.
fn apply_nested_continuation(
    tree: &mut LayoutSession<'_>,
    plan: &NestedContinuation,
) -> Option<()> {
    let root = tree.fragment_tree.fragments[plan.root];
    tree.fragment_tree.fragments[plan.root].rect.height = plan.first_height;
    let replaced: std::collections::HashSet<usize> = plan.children.iter().copied().collect();
    let stale: Vec<usize> = tree
        .fragment_tree
        .fragments
        .iter()
        .enumerate()
        .filter(|(_, fragment)| {
            fragment.parent == Some(plan.root) && replaced.contains(&fragment.node_id)
        })
        .map(|(index, _)| index)
        .collect();
    tree.fragment_tree.retire_subtrees(stale);
    // Each row's box record and its content-box origin within that box.
    let last = plan.rows.heights.len() - 1;
    let mut row_boxes = vec![(plan.root, plan.content_origin)];
    for (row, &height) in plan.rows.heights.iter().enumerate().skip(1) {
        let index = tree
            .fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: root.node_id,
                parent: root.parent,
                fragmentainer: root.fragmentainer + row,
                rect: crate::fragment::FragmentRect {
                    x: root.rect.x + row as f32 * plan.row_step,
                    y: root.rect.y + plan.row_top,
                    width: root.rect.width,
                    height: if row == last {
                        plan.end
                    } else {
                        plan.row_box_height(height)
                    },
                },
                fragmentainer_clip: None,
                fragment_index: 0,
                fragment_count: 1,
                line_start: None,
                line_end: None,
            })?;
        // A continued box has no block-start padding or border unless it
        // clones them.
        row_boxes.push((
            index,
            Point {
                x: plan.content_origin.x,
                y: plan.continued_origin_y(),
            },
        ));
    }
    // The origin of each row's box in the first box's coordinates.
    let row_origin = |row: usize| {
        if row == 0 {
            Point::ZERO
        } else {
            Point {
                x: row as f32 * plan.row_step,
                y: plan.row_top,
            }
        }
    };
    let mut first_rects = std::collections::HashMap::<usize, crate::fragment::FragmentRect>::new();
    let mut text = std::collections::HashMap::<usize, Vec<MulticolTextFragment>>::new();
    for fragment in &plan.rows.fragments {
        let child = plan.children[fragment.entry];
        let layout = tree.nodes[child].layout;
        let (row_box, content) = row_boxes[fragment.row];
        let column_x = content.x + plan.context.column_offset_x(fragment.column);
        let rect = crate::fragment::FragmentRect {
            x: column_x + layout.margin.left,
            y: content.y + fragment.y,
            width: layout.size.width,
            height: fragment.height,
        };
        let fragmentainer = fragment.row * plan.context.column_count + fragment.column;
        tree.fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: child,
                parent: Some(row_box),
                fragmentainer,
                rect,
                fragmentainer_clip: Some(
                    crate::fragment::FragmentRect {
                        x: column_x,
                        y: content.y,
                        width: plan.context.column_width,
                        height: plan.rows.heights[fragment.row],
                    }
                    .into(),
                ),
                fragment_index: 0,
                fragment_count: 1,
                line_start: Some(fragment.line_start),
                line_end: Some(fragment.line_end),
            })?;
        let origin = row_origin(fragment.row);
        let rect = crate::fragment::FragmentRect {
            x: origin.x + rect.x,
            y: origin.y + rect.y,
            ..rect
        };
        let first = *first_rects.entry(child).or_insert(rect);
        text.entry(child).or_default().push(MulticolTextFragment {
            line_start: fragment.line_start,
            line_end: fragment.line_end,
            fragmentainer,
            x: rect.x - first.x,
            y: rect.y - first.y,
        });
    }
    for piece in &plan.lineless {
        let child = plan.children[piece.entry];
        let layout = tree.nodes[child].layout;
        let (row_box, content) = row_boxes[piece.row];
        let column_x = content.x + plan.context.column_offset_x(piece.column);
        let rect = crate::fragment::FragmentRect {
            x: column_x,
            y: content.y + piece.y,
            width: layout.size.width,
            height: piece.height,
        };
        tree.fragment_tree
            .try_push(crate::fragment::LayoutFragment {
                node_id: child,
                parent: Some(row_box),
                fragmentainer: piece.row * plan.context.column_count + piece.column,
                rect,
                // A box that stays in one column is not cut.
                fragmentainer_clip: (piece.count > 1).then_some(
                    crate::fragment::FragmentRect {
                        x: column_x,
                        y: content.y,
                        width: layout.size.width.max(plan.context.column_width),
                        height: plan.rows.heights[piece.row],
                    }
                    .into(),
                ),
                fragment_index: piece.index,
                fragment_count: piece.count,
                line_start: None,
                line_end: None,
            })?;
        if piece.index == 0 {
            let origin = row_origin(piece.row);
            tree.nodes[child].layout.location = Point {
                x: origin.x + rect.x,
                y: origin.y + rect.y,
            };
            leave_block_offset_to_painter(tree, child);
        }
    }
    // Column rules are drawn in each row's box, which the row's group names
    // by its fragmentainer. The row's box shows the part of the whole box
    // that its block range covers, unless the box clones its decorations and
    // so is no slice of a whole box.
    let mut box_start = 0.0;
    let mut rows: Vec<crate::fragment::MulticolGroup> = plan
        .rows
        .heights
        .iter()
        .enumerate()
        .map(|(row, &height)| {
            let box_height = match row {
                0 => plan.first_height,
                row if row == last => plan.end,
                _ => plan.row_box_height(height),
            };
            let slice = (box_start, box_start + box_height);
            box_start = slice.1;
            crate::fragment::MulticolGroup {
                context: FragmentationContext {
                    origin_x: row_boxes[row].1.x,
                    origin_y: row_boxes[row].1.y,
                    available_height: Some(height),
                    column_index: root.fragmentainer + row,
                    ..plan.context
                },
                height,
                occupied: std::collections::BTreeSet::new(),
                slice: (!plan.cloned).then_some(slice),
            }
        })
        .collect();
    for fragment in &plan.rows.fragments {
        if fragment.line_start < fragment.line_end {
            rows[fragment.row].occupied.insert(fragment.column);
        }
    }
    for piece in &plan.lineless {
        if piece.height > 0.0 {
            rows[piece.row].occupied.insert(piece.column);
        }
    }
    tree.nodes[root.node_id].multicol_rows = rows;
    for (child, fragments) in text {
        let first = first_rects[&child];
        let node = &mut tree.nodes[child];
        node.layout.location = Point {
            x: first.x,
            y: first.y,
        };
        node.layout.size.height = first.height;
        if let Some(root) = node.ifc.as_mut() {
            root.multicol_fragments = Some(fragments);
            root.multicol_fragment_origins_recorded = true;
        }
        leave_block_offset_to_painter(tree, child);
    }
    Some(())
}

// cov:ignore: nested box and float fragment records are exercised by the ignored flex-float WPT.
fn record_nested_ifc_box_fragments(
    tree: &mut LayoutSession<'_>,
    subtree_root: usize,
    parent_fragment: usize,
    context: FragmentationContext,
    edges_sliced: bool,
) -> Option<()> {
    let Some(height) = context.available_height.filter(|height| *height > 0.0) else {
        return Some(());
    };
    // A wrapper that fills its box with a color and holds a plain paragraph
    // has a box of its own in each column the paragraph reaches.
    let wrapper_fragments = tree.nodes[subtree_root].ifc.is_none()
        && tree.nodes[subtree_root].paints_background_color
        && painted_wrapper_leaf(tree, subtree_root).is_some();
    let subtree_has_float = multicol_subtree_has_float(tree, subtree_root);
    // The column the subtree root was placed in by its multicol container.
    let root_column = tree.fragment_tree.fragments[parent_fragment].fragmentainer;
    let mut existing_fragments = std::collections::HashMap::<(usize, usize), usize>::new();
    // Both scopes are properties of a node's ancestor chain. Carry them down
    // the traversal instead of walking to the root again for every node.
    let mut pending = vec![(
        subtree_root,
        FlexFloatChain::of_ancestors(tree, subtree_root),
        tree.nodes[subtree_root].has_logical_min_block_size,
    )];
    while let Some((node_id, flex_float_chain, nested_logical_minimum_scope)) = pending.pop() {
        let nested_row_flex_scope = flex_float_chain.is_scope();
        let lines = tree.nodes[node_id]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .cloned();
        let mut placements = lines
            .as_ref()
            .map(|lines| lines.fragment_box_placements.clone())
            .unwrap_or_default();
        // Non-float nested text needs explicit ranges only when lines cross
        // columns. Break-avoided logical minimum children keep block placement.
        let line_ranges = (nested_row_flex_scope
            || (!subtree_has_float && !nested_logical_minimum_scope))
            .then(|| {
                lines
                    .as_ref()
                    .and_then(|lines| lines.fragmentainer_line_ranges.clone())
                    .or_else(|| {
                        tree.nodes[node_id]
                            .ifc
                            .as_ref()
                            .and_then(|root| root.multicol_fragments.clone())
                    })
            })
            .flatten()
            .filter(|ranges| nested_row_flex_scope || ranges.len() > 1);
        // A subpixel column height can make one tall float span millions of
        // fragmentainers. Cap expanded placements before building paint paths.
        const MAX_NESTED_FLOAT_FRAGMENTS: usize = 1_024;
        let mut placement_keys = placements
            .iter()
            .map(|placement| (placement.node_id, placement.fragmentainer))
            .collect::<std::collections::HashSet<_>>();
        let mut last_placement_columns = std::collections::HashMap::<usize, usize>::new();
        for placement in &placements {
            last_placement_columns
                .entry(placement.node_id)
                .and_modify(|last| *last = (*last).max(placement.fragmentainer))
                .or_insert(placement.fragmentainer);
        }
        let mut overflow_clip_heights = std::collections::HashMap::<usize, f32>::new();
        for source in placements.clone() {
            if !is_floated(tree, source.node_id) {
                continue;
            }
            let logical_top =
                source.fragmentainer.saturating_sub(root_column) as f32 * height + source.rect.y;
            let last_column = root_column.saturating_add(
                (((logical_top + source.rect.height).max(0.0) / height).ceil() as usize)
                    .saturating_sub(1),
            );
            for column in source.fragmentainer.saturating_add(1)..=last_column {
                if placements.len() >= MAX_NESTED_FLOAT_FRAGMENTS {
                    break;
                }
                if !placement_keys.insert((source.node_id, column)) {
                    continue;
                }
                placements.push(crate::layout::ifc::root::IfcBoxFragment {
                    node_id: source.node_id,
                    fragmentainer: column,
                    rect: crate::fragment::FragmentRect {
                        y: source.rect.y - (column - source.fragmentainer) as f32 * height,
                        ..source.rect
                    },
                });
                last_placement_columns
                    .entry(source.node_id)
                    .and_modify(|last| *last = (*last).max(column))
                    .or_insert(column);
            }
            let last_emitted = last_placement_columns[&source.node_id];
            if last_emitted < last_column {
                // Keep the unexpanded tail visible in the final emitted
                // column when the defensive fragment budget is exhausted.
                let remaining_bottom = source.rect.y + source.rect.height
                    - last_emitted.saturating_sub(source.fragmentainer) as f32 * height;
                overflow_clip_heights
                    .entry(last_emitted)
                    .and_modify(|clip| *clip = clip.max(remaining_bottom))
                    .or_insert(remaining_bottom);
            }
        }
        if !placements.is_empty() || line_ranges.is_some() {
            // Index ranges and placements by column once; scanning every
            // range and placement again for each column is quadratic in the
            // column count. A column takes its first range, as in source
            // order, together with that range's ordinal.
            let mut columns = std::collections::BTreeMap::<usize, ColumnEntries>::new();
            for (index, placement) in placements.iter().enumerate() {
                columns
                    .entry(placement.fragmentainer)
                    .or_default()
                    .placements
                    .push(index);
            }
            if let Some(ranges) = &line_ranges {
                for (index, range) in ranges.iter().enumerate() {
                    columns
                        .entry(range.fragmentainer)
                        .or_default()
                        .range
                        .get_or_insert((index, *range));
                }
            }

            // A paragraph that is the subtree root owns its fragments: the
            // one it was placed with, and one at the top of each later
            // column it continues in. A column container that holds its own
            // paragraph is no such root: its ranges name its own columns, and
            // the container stays one box in the column it was placed in.
            let root_owns_fragments = node_id == subtree_root
                && tree.nodes[node_id].multicol.is_none()
                && tree.fragment_tree.fragments[parent_fragment].node_id == node_id;
            // Its ranges give each line its block offset in the paragraph,
            // so lines in a later column would be drawn that far below the
            // column top. Draw each range from its own fragment instead.
            // That leaves the first column as it was only when its range
            // starts at the paragraph's top.
            if root_owns_fragments
                && let (Some(ranges), Some(lines)) = (&line_ranges, &lines)
                && ranges
                    .iter()
                    .any(|range| range.fragmentainer != root_column)
                && ranges.iter().all(|range| {
                    lines.lines.get(range.line_start).is_some_and(|first| {
                        range.y == first.block_offset()
                            && (range.fragmentainer != root_column || first.block_offset() == 0.0)
                    })
                })
                && let Some(root) = tree.nodes[node_id].ifc.as_mut()
            {
                root.multicol_fragment_origins_recorded = true;
            }

            let mut path = Vec::new();
            let mut current = node_id;
            while current != subtree_root {
                path.push(current);
                current = tree.parent_of(current).unwrap_or(subtree_root);
            }
            path.reverse();
            let plain_block_path = !subtree_has_float
                && !nested_logical_minimum_scope
                && tree.nodes[subtree_root].layout_display() == Display::Block
                && tree.nodes[node_id].ifc_writing_mode()
                    == Some(shodo::geometry::WritingMode::HorizontalTb)
                && path
                    .iter()
                    .all(|&ancestor| tree.nodes[ancestor].layout_display() == Display::Block);
            let node_offset_y = path
                .iter()
                .map(|&ancestor| tree.nodes[ancestor].layout.location.y)
                .sum::<f32>();

            // An atomic inline sits on a line, so it belongs to the column of
            // that line: it gets a fragment in the paragraph's fragment of
            // that column, as the paragraph's fragments would each paint it
            // otherwise.
            if (root_owns_fragments || plain_block_path)
                && !nested_row_flex_scope
                && tree.nodes[node_id].multicol.is_none()
                && let (Some(ranges), Some(lines)) = (&line_ranges, &lines)
            {
                let (recorded, atomics) =
                    tree.nodes[node_id]
                        .ifc
                        .as_ref()
                        .map_or((false, Vec::new()), |root| {
                            (
                                root.multicol_fragment_origins_recorded,
                                crate::layout::ifc::boxes::atomic_fragments(
                                    &root.boxes,
                                    &lines.lines,
                                    ranges,
                                ),
                            )
                        });
                for (atomic, range) in atomics {
                    let layout = tree.nodes[atomic].layout;
                    // The first line of a range is drawn at the top of its
                    // fragment, or at the range's own offset.
                    let range_top = if recorded { 0.0 } else { range.y };
                    let first_line = lines.lines[range.line_start].block_offset();
                    columns
                        .entry(range.fragmentainer)
                        .or_default()
                        .placements
                        .push(placements.len());
                    placements.push(crate::layout::ifc::root::IfcBoxFragment {
                        node_id: atomic,
                        fragmentainer: range.fragmentainer,
                        rect: crate::fragment::FragmentRect {
                            x: layout.location.x,
                            y: range_top + layout.location.y - first_line,
                            width: layout.size.width,
                            height: layout.size.height,
                        },
                    });
                }
            }

            for (column, entries) in columns {
                let column_x =
                    context.column_offset_x(column) - context.column_offset_x(root_column);
                let line_range = entries.range.map(|(_, range)| range);
                let line_bottom = line_range
                    .and_then(|range| {
                        lines
                            .as_ref()?
                            .lines
                            .get(range.line_end.checked_sub(1)?)
                            .map(|line| line.block_offset() + line.block_size())
                    })
                    .unwrap_or(0.0);
                // A slice of this paragraph in a later column than the
                // subtree root starts at the top of that column: move the
                // path up by the block offset of its first line below the
                // root's column-top position. Earlier content can leave the
                // previous column short of its end (widows and orphans), so
                // whole column heights would drift.
                let line_shift = line_range
                    .filter(|_| !nested_row_flex_scope && column != root_column)
                    .map(|range| {
                        tree.nodes[subtree_root].layout.location.y - context.origin_y
                            + node_offset_y
                            + range.y
                    });
                let column_delta = match line_range {
                    Some(_) if !nested_row_flex_scope => line_shift.unwrap_or(0.0),
                    _ => column.saturating_sub(root_column) as f32 * height,
                };
                // The bottom of the last line is measured from the column
                // top where this slice is painted, so a shifted slice drops
                // the offset of its first line.
                let tail_shift = line_range
                    .filter(|_| line_shift.is_some())
                    .map_or(0.0, |range| node_offset_y + range.y);
                let tail_line_bottom = lines
                    .as_ref()
                    .filter(|lines| lines.unfragmented_tail_column == Some(column))
                    .map(|_| node_offset_y + line_bottom)
                    .map_or(0.0, |bottom| bottom - tail_shift);
                let overflow_float_bottom = overflow_clip_heights
                    .get(&column)
                    .map(|bottom| node_offset_y + bottom)
                    .unwrap_or(0.0);
                let column_clip_height = height.max(tail_line_bottom).max(overflow_float_bottom);
                let mut parent = parent_fragment;
                let mut parent_offset = Point::ZERO;
                let fragment_index = entries.range.map_or(0, |(index, _)| index);
                let fragment_count = line_ranges.as_ref().map_or(1, Vec::len);
                // Height of the box fragment that draws `range`: its lines,
                // or, for a slice the paragraph goes on from, down to the
                // fragmentainer edge, including unused space left by
                // paragraph break minima.
                let slice_height =
                    |range: MulticolTextFragment,
                     extend: bool,
                     base: crate::fragment::LayoutFragment| {
                        if extend && fragment_index + 1 < fragment_count {
                            let top = if fragment_index == 0 {
                                base.rect.y - context.origin_y
                            } else {
                                0.0
                            };
                            return (height - top).max(0.0);
                        }
                        lines
                            .as_ref()
                            .and_then(|lines| {
                                let first = lines.lines.get(range.line_start)?;
                                let last = lines.lines.get(range.line_end.checked_sub(1)?)?;
                                Some(last.block_offset() + last.block_size() - first.block_offset())
                            })
                            .unwrap_or(base.rect.height)
                            .max(0.0)
                    };
                // The wrapper that is the subtree root has a box in each
                // column its paragraph reaches. The first column keeps the
                // fragment it was placed with; a later one is a sibling of it
                // at the column top, and the path continues from there.
                let mut wrapper_fragment = None;
                if wrapper_fragments
                    && plain_block_path
                    && !nested_row_flex_scope
                    && let Some(range) = line_range
                {
                    let base = tree.fragment_tree.fragments[parent_fragment];
                    let fragment_height = slice_height(range, true, base);
                    if base.fragmentainer == column {
                        let fragment = &mut tree.fragment_tree.fragments[parent_fragment];
                        fragment.rect.height = fragment_height;
                        fragment.fragment_index = fragment_index;
                        fragment.fragment_count = fragment_count;
                    } else {
                        let fragment_id =
                            tree.fragment_tree
                                .try_push(crate::fragment::LayoutFragment {
                                    node_id: subtree_root,
                                    parent: base.parent,
                                    fragmentainer: column,
                                    rect: crate::fragment::FragmentRect {
                                        x: base.rect.x + context.column_offset_x(column)
                                            - context.column_offset_x(base.fragmentainer),
                                        y: context.origin_y,
                                        width: base.rect.width,
                                        height: fragment_height,
                                    },
                                    fragmentainer_clip: None,
                                    fragment_index,
                                    fragment_count,
                                    line_start: None,
                                    line_end: None,
                                })?;
                        parent = fragment_id;
                        wrapper_fragment = Some((fragment_id, node_offset_y + range.y));
                    }
                }
                // `edges_sliced` tells that the planner placed the lines of the
                // paragraph with the padding and border above its first line
                // and below its last, which its pieces slice: each piece is
                // the whole box, clipped to its column, with its lines drawn
                // from the content box. A piece after the first begins at the
                // column top with the lines, so its box begins above the
                // column by the edge above the first line, and a piece that
                // goes on ends below the column by the edge below the last
                // line, so neither edge shows where the paragraph breaks.
                let origins_recorded = tree.nodes[node_id]
                    .ifc
                    .as_ref()
                    .is_some_and(|root| root.multicol_fragment_origins_recorded);
                let (edge_start, edge_end) = if edges_sliced
                    && origins_recorded
                    && root_owns_fragments
                    && fragment_count > 1
                    && slices_block_edges(layout_node(tree, node_id))
                {
                    block_edges(&tree.nodes[node_id].layout)
                } else {
                    (0.0, 0.0)
                };
                let sliced = edge_start > 0.0 || edge_end > 0.0;
                // How far the box of the piece begins above its first line.
                let piece_lead = if fragment_index > 0 { edge_start } else { 0.0 };
                if root_owns_fragments && let Some(range) = line_range {
                    let base = tree.fragment_tree.fragments[parent_fragment];
                    let fragment_height = slice_height(range, origins_recorded, base)
                        + if sliced { piece_lead + edge_end } else { 0.0 };
                    let fragment_id = if base.fragmentainer == column {
                        let fragment = &mut tree.fragment_tree.fragments[parent_fragment];
                        fragment.rect.height = fragment_height;
                        fragment.line_start = Some(range.line_start);
                        fragment.line_end = Some(range.line_end);
                        fragment.fragment_index = fragment_index;
                        fragment.fragment_count = fragment_count;
                        if sliced {
                            fragment.fragmentainer_clip = Some(paragraph_group::column_window(
                                fragment.rect,
                                context,
                                height,
                            ));
                        }
                        parent_fragment
                    } else {
                        let rect = crate::fragment::FragmentRect {
                            x: base.rect.x + context.column_offset_x(column)
                                - context.column_offset_x(base.fragmentainer),
                            y: context.origin_y - piece_lead,
                            width: base.rect.width,
                            height: fragment_height,
                        };
                        tree.fragment_tree
                            .try_push(crate::fragment::LayoutFragment {
                                node_id,
                                parent: base.parent,
                                fragmentainer: column,
                                rect,
                                fragmentainer_clip: sliced
                                    .then(|| paragraph_group::column_window(rect, context, height)),
                                fragment_index,
                                fragment_count,
                                line_start: Some(range.line_start),
                                line_end: Some(range.line_end),
                            })?
                    };
                    existing_fragments.insert((node_id, column), fragment_id);
                    parent = fragment_id;
                }
                for &ancestor in &path {
                    let layout = tree.nodes[ancestor].layout;
                    let is_flex_item = tree.parent_of(ancestor).is_some_and(|parent| {
                        tree.nodes[parent].formatting_context == FormattingContext::Flex
                    });
                    // Move a plain block path into its column once; descendants
                    // retain their local coordinates. The path below a wrapper
                    // fragment at the column top only drops the offset of its
                    // first line.
                    let y = if ancestor == subtree_root || is_flex_item {
                        layout.location.y
                    } else if let Some((_, top)) = wrapper_fragment.filter(|(id, _)| *id == parent)
                    {
                        layout.location.y - top
                    } else if plain_block_path && parent != parent_fragment {
                        layout.location.y
                    } else {
                        layout.location.y - column_delta
                    };
                    let fragment_id =
                        if let Some(&fragment_id) = existing_fragments.get(&(ancestor, column)) {
                            let fragment = &mut tree.fragment_tree.fragments[fragment_id];
                            extend_fragment_clip(fragment, column_clip_height);
                            if ancestor == node_id
                                && let Some(range) = line_range
                            {
                                fragment.line_start = Some(range.line_start);
                                fragment.line_end = Some(range.line_end);
                            }
                            fragment_id
                        } else {
                            let fragment_id =
                                tree.fragment_tree
                                    .try_push(crate::fragment::LayoutFragment {
                                        node_id: ancestor,
                                        parent: Some(parent),
                                        fragmentainer: column,
                                        rect: crate::fragment::FragmentRect {
                                            x: layout.location.x
                                                + if parent == parent_fragment {
                                                    column_x
                                                } else {
                                                    0.0
                                                },
                                            y,
                                            width: layout.size.width,
                                            height: layout.size.height,
                                        },
                                        fragmentainer_clip: Some(
                                            crate::fragment::FragmentRect {
                                                x: if parent == parent_fragment {
                                                    column_x
                                                } else {
                                                    -parent_offset.x
                                                },
                                                y: -parent_offset.y,
                                                width: context.column_width,
                                                height: column_clip_height,
                                            }
                                            .into(),
                                        ),
                                        fragment_index: 0,
                                        fragment_count: 1,
                                        line_start: (ancestor == node_id)
                                            .then_some(line_range)
                                            .flatten()
                                            .map(|range| range.line_start),
                                        line_end: (ancestor == node_id)
                                            .then_some(line_range)
                                            .flatten()
                                            .map(|range| range.line_end),
                                    })?;
                            existing_fragments.insert((ancestor, column), fragment_id);
                            fragment_id
                        };
                    parent = fragment_id;
                    parent_offset.x += layout.location.x;
                    parent_offset.y += if plain_block_path {
                        tree.fragment_tree.fragments[fragment_id].rect.y
                    } else {
                        layout.location.y
                    };
                }

                for placement in entries.placements.iter().map(|&index| &placements[index]) {
                    let fragment_id =
                        tree.fragment_tree
                            .try_push(crate::fragment::LayoutFragment {
                                node_id: placement.node_id,
                                parent: Some(parent),
                                fragmentainer: column,
                                rect: placement.rect,
                                fragmentainer_clip: Some(
                                    crate::fragment::FragmentRect {
                                        x: if parent == parent_fragment {
                                            column_x
                                        } else {
                                            -parent_offset.x
                                        },
                                        // A box that begins above the column top
                                        // by `piece_lead` has its column below
                                        // that much of it.
                                        y: -parent_offset.y
                                            + if path.is_empty() && sliced {
                                                piece_lead
                                            } else {
                                                0.0
                                            },
                                        width: context.column_width,
                                        height: column_clip_height,
                                    }
                                    .into(),
                                ),
                                fragment_index: 0,
                                fragment_count: 1,
                                line_start: None,
                                line_end: None,
                            })?;
                    existing_fragments.insert((placement.node_id, column), fragment_id);
                }
            }
        }
        pending.extend(
            tree.nodes[node_id]
                .children
                .iter()
                .copied()
                .filter(|&child| {
                    tree.nodes[child].is_in_document() && !tree.nodes[child].is_display_none()
                })
                .map(|child| {
                    (
                        child,
                        flex_float_chain.below(tree, child),
                        nested_logical_minimum_scope
                            || tree.nodes[child].has_logical_min_block_size,
                    )
                }),
        );
    }
    Some(())
}

/// The ranges and placements of one nested node that land in one column.
#[derive(Default)]
struct ColumnEntries {
    /// The first line range in the column and its ordinal among all ranges.
    range: Option<(usize, MulticolTextFragment)>,
    /// Indices of the column's box placements, in source order.
    placements: Vec<usize>,
}

/// What a node's ancestor chain, the node included, contributes to the
/// row-flex float scope: walking up, the chain ends at the nearest vertical
/// writing mode (no scope) or multicol container. Floats count up to and
/// including that container; row flex boxes count only below it.
#[derive(Clone, Copy)]
enum FlexFloatChain {
    /// Neither a multicol container nor a vertical writing mode above.
    Open,
    /// A vertical writing mode comes before any multicol container.
    Vertical,
    Multicol {
        horizontal: bool,
        row_flex: bool,
        float: bool,
    },
}

impl FlexFloatChain {
    fn of_ancestors(tree: &LayoutSession<'_>, node_id: usize) -> Self {
        let chain: Vec<_> = tree.inclusive_ancestors(node_id).collect();
        chain
            .into_iter()
            .rev()
            .fold(Self::Open, |above, id| above.below(tree, id))
    }

    /// The chain of `node_id`, whose parent's chain is `self`.
    fn below(self, tree: &LayoutSession<'_>, node_id: usize) -> Self {
        let node = layout_node(tree, node_id);
        if node
            .authored_writing_mode
            .is_some_and(WritingMode::is_vertical)
        {
            return Self::Vertical;
        }
        // Whether the box is a float, or holds one on its lines.
        let float = || {
            is_floated(tree, node_id)
                || node
                    .ifc
                    .as_ref()
                    .and_then(|root| root.lines.as_ref())
                    .is_some_and(|lines| {
                        lines
                            .fragment_box_placements
                            .iter()
                            .any(|placement| is_floated(tree, placement.node_id))
                    })
        };
        if let Some(multicol) = node.multicol {
            return Self::Multicol {
                horizontal: multicol.horizontal,
                row_flex: false,
                float: float(),
            };
        }
        let Self::Multicol {
            horizontal,
            row_flex: above_row_flex,
            float: above_float,
        } = self
        else {
            return self;
        };
        let row_flex = node.formatting_context == FormattingContext::Flex
            && style_of(tree, node_id).flex_direction_is_row();
        Self::Multicol {
            horizontal,
            row_flex: above_row_flex || row_flex,
            float: above_float || float(),
        }
    }

    /// Whether a row flex box and a float share the nearest horizontal
    /// multicol container.
    fn is_scope(self) -> bool {
        matches!(
            self,
            Self::Multicol {
                horizontal: true,
                row_flex: true,
                float: true,
            }
        )
    }
}

fn extend_fragment_clip(fragment: &mut crate::fragment::LayoutFragment, height: f32) {
    if let Some(clip) = fragment.fragmentainer_clip.as_mut() {
        clip.rect.height = clip.rect.height.max(height);
    }
}

fn multicol_subtree_has_float(tree: &LayoutSession<'_>, subtree_root: usize) -> bool {
    let mut pending = vec![subtree_root];
    while let Some(node_id) = pending.pop() {
        let node = layout_node(tree, node_id);
        if !node.is_in_document() || node.is_display_none() {
            continue;
        }
        if is_floated(tree, node_id) {
            return true;
        }
        pending.extend(node.children.iter().copied());
    }
    false
}

/// Whether `subtree_root` or a rendered descendant is a float or has a
/// block-direction margin, which adjoins and collapses with the margin of a
/// sibling on the block layout but not on the column strategies.
fn multicol_subtree_has_float_or_block_margin(
    tree: &LayoutSession<'_>,
    subtree_root: usize,
) -> bool {
    let zero = LengthPercentageAuto::length(0.0);
    let mut pending = vec![subtree_root];
    while let Some(node_id) = pending.pop() {
        let node = layout_node(tree, node_id);
        if !node.is_in_document() || node.is_display_none() {
            continue;
        }
        let style = style_of(tree, node_id);
        let margin = style.margin();
        if style.float().is_floated() || margin.top != zero || margin.bottom != zero {
            return true;
        }
        pending.extend(node.children.iter().copied());
    }
    false
}

/// Split lines, given by their block-start and block-end offsets, over the
/// columns of `context` from its current column on: by the column height
/// when it is definite, else balanced by count; then `widows` and `orphans`
/// move lines across each break. Returns `(first, end, column)` ranges.
fn line_ranges_in_columns(
    extents: &[(f32, f32)],
    context: FragmentationContext,
    preserve_block_offsets: bool,
) -> Vec<(usize, usize, usize)> {
    let line_count = extents.len();
    if line_count == 0 {
        return Vec::new();
    }
    let available_columns = context.column_count.saturating_sub(context.column_index);
    let mut ranges = Vec::with_capacity(available_columns.max(1).min(line_count));
    let mut start = 0usize;
    let definite_height = context.available_height.filter(|height| *height > 0.0);
    if let Some(height) = definite_height
        && !preserve_block_offsets
    {
        // Lines that do not fit in the columns of a definite-height container
        // continue in as many overflow columns as they need (CSS Multi-column
        // 1, 8.2).
        let mut column = context.column_index;
        while start < line_count {
            let end = column_end(extents, start, height);
            ranges.push((start, end, column));
            column += 1;
            start = end;
        }
    } else if preserve_block_offsets
        && let Some(height) = context.available_height.filter(|height| *height > 0.0)
    {
        while start < line_count {
            let column_offset = (extents[start].0.max(0.0) / height).floor() as usize;
            let column = context.column_index.saturating_add(column_offset);
            let mut end = start + 1;
            while end < line_count
                && (extents[end].0.max(0.0) / height).floor() as usize == column_offset
            {
                end += 1;
            }
            ranges.push((start, end, column));
            start = end;
        }
    } else {
        if available_columns == 0 {
            return Vec::new();
        }
        for column in 0..available_columns {
            if start >= line_count {
                break;
            }
            let origin = extents.get(start).map_or(0.0, |extent| extent.0);
            let mut end = start;
            while end < line_count {
                let fits = context
                    .available_height
                    .filter(|height| *height > 0.0)
                    .map(|height| {
                        extents
                            .get(end)
                            .is_some_and(|extent| extent.1 - origin <= height + f32::EPSILON)
                    })
                    .unwrap_or_else(|| {
                        let remaining = line_count - start;
                        let target = remaining.div_ceil(available_columns - column);
                        end - start < target
                    });
                if !fits && end > start {
                    break;
                }
                end += 1;
            }
            ranges.push((
                start,
                end.max(start + 1).min(line_count),
                context.column_index + column,
            ));
            start = end.max(start + 1).min(line_count);
        }
        if let Some(last) = ranges.last_mut()
            && last.1 < line_count
        {
            last.1 = line_count;
        }
    }

    respect_orphans_and_widows(&mut ranges, context);
    ranges
}

/// Move lines between the pieces `ranges` of a paragraph so that each keeps
/// the `orphans` and `widows` of `context`.
fn respect_orphans_and_widows(ranges: &mut [(usize, usize, usize)], context: FragmentationContext) {
    // CSS Fragmentation §3.3 constrains each line break. If the final
    // fragment would contain fewer than `widows` lines, move lines from the
    // preceding fragment while preserving its `orphans` minimum. This is the
    // smallest safe adjustment because the shaped layout and line metrics do
    // not change; only the fragment ranges move.
    let orphans = context.orphans.max(1);
    let widows = context.widows.max(1);
    for boundary in 0..ranges.len().saturating_sub(1) {
        let left_len = ranges[boundary].1 - ranges[boundary].0;
        let right_len = ranges[boundary + 1].1 - ranges[boundary + 1].0;
        if right_len < widows {
            let movable = left_len.saturating_sub(orphans);
            let moved = (widows - right_len).min(movable);
            ranges[boundary].1 -= moved;
            ranges[boundary + 1].0 -= moved;
        }
    }
}

/// Find the smallest column height that fits `extents` within the available
/// columns, with at least `orphans` lines in every column that a break
/// follows. The returned height is then used to fill each column in source
/// order, including a partially filled final column.
///
/// A definite `available_height` caps the columns: lines that do not fit
/// within it fill it and go on in overflow columns (CSS Multi-column 1, 8.2),
/// so there is nothing to balance and the answer is `None`.
fn balanced_column_height(extents: &[(f32, f32)], context: FragmentationContext) -> Option<f32> {
    let available_columns = context.column_count.saturating_sub(context.column_index);
    if extents.is_empty() || available_columns == 0 {
        return None;
    }
    let mut low = extents
        .iter()
        .map(|(top, bottom)| (bottom - top).max(0.0))
        .fold(0.0_f32, f32::max);
    let mut high = extents.last()?.1 - extents.first()?.0;
    if !low.is_finite() || !high.is_finite() || high < low {
        return None;
    }
    // The columns the lines take at `height`, when the column of the last one
    // exists and no column is taller than `height`.
    let columns_at = |height: f32| {
        let trial_context = FragmentationContext {
            available_height: Some(height),
            ..context
        };
        let ranges = line_ranges_in_columns(extents, trial_context, false);
        (ranges
            .last()
            .is_some_and(|&(_, _, column)| column < context.column_count)
            && ranges.iter().all(|&(start, end, _)| {
                extents[end - 1].1 - extents[start].0 <= height + f32::EPSILON
            }))
        .then_some(ranges)
    };
    if let Some(cap) = context.available_height.filter(|height| *height > 0.0)
        && cap < high
    {
        columns_at(cap)?;
        high = cap;
    }

    // The columns never need to be taller than `limit`: the cap of a definite
    // height, or all the lines in one column.
    let limit = high;
    for _ in 0..32 {
        if high - low <= 0.001 {
            break;
        }
        let height = (low + high) * 0.5;
        if columns_at(height).is_some() {
            high = height;
        } else {
            low = height;
        }
    }

    // A column that a break follows keeps at least `orphans` lines: a break
    // that leaves fewer lines before it is not an option, so the columns grow,
    // one step at a time to the next height at which a column takes another
    // line, until every such column does. `widows` is not checked: the columns
    // after a break give up lines to it when they can, and when they cannot,
    // the last column keeps the lines it has. When no height below `limit`
    // keeps the `orphans`, the answer is `limit`. After too many steps the
    // `orphans` are given up and the last height stands.
    let orphans = context.orphans.max(1);
    let keeps_orphans = |height: f32| {
        columns_at(height).is_some_and(|ranges| {
            ranges
                .iter()
                .rev()
                .skip(1)
                .all(|&(start, end, _)| end - start >= orphans)
        })
    };
    for _ in 0..MAX_ORPHAN_STEPS {
        if keeps_orphans(high) {
            return Some(high);
        }
        match next_taller_column(extents, high) {
            Some(next) if next < limit => high = next,
            _ => return Some(limit),
        }
    }
    Some(high)
}

/// The most steps `balanced_column_height` takes while it grows the columns to
/// keep the `orphans` of each, so that a huge `orphans` costs a bounded number
/// of passes over the lines.
const MAX_ORPHAN_STEPS: usize = 256;

/// Whether `child`, which goes `kind` of beside floats, is a paragraph whose
/// lines break between columns where they are: one that shares the floats'
/// context, with no block-edge padding or border, no authored height and
/// nothing that avoids a break inside it.
fn splits_beside_floats(
    tree: &LayoutSession<'_>,
    child: usize,
    kind: Option<BesideFloats>,
) -> bool {
    kind == Some(BesideFloats::Flows)
        && tree.nodes[child].ifc.is_some()
        && can_balance_paragraph_box(tree, child, BlockEdges::Refused)
}

/// Whether the paragraph `child`, whose content starts `content_top` below
/// the top of its column, ends `orphans` of its lines, or all of them when it
/// has fewer, within the column's `height`.
fn first_piece_fits(
    tree: &LayoutSession<'_>,
    child: usize,
    content_top: f32,
    height: f32,
    orphans: usize,
) -> bool {
    tree.nodes[child]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .and_then(|lines| {
            let lines = &lines.lines;
            lines.get(orphans.max(1).min(lines.len()).checked_sub(1)?)
        })
        .is_some_and(|line| {
            content_top + line.block_offset() + line.block_size() <= height + LINE_EPSILON
        })
}

/// The pieces of the paragraph `root` in columns `height` tall when its
/// content starts `content_top` below the top of the column of `context`:
/// the lines that end within that column stay in it, and each next column
/// starts with the first line that did not fit the one before. With
/// `balanced`, the last column of `context` takes all the lines left.
fn lines_below_column_top(
    root: &crate::layout::ifc::root::IfcRoot,
    content_top: f32,
    height: f32,
    context: FragmentationContext,
    balanced: bool,
) -> Option<Vec<MulticolTextFragment>> {
    let extents: Vec<(f32, f32)> = root
        .lines
        .as_ref()?
        .lines
        .iter()
        .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
        .collect();
    let last_column = balanced.then(|| context.column_count.saturating_sub(1));
    // The column of `context` holds the lines that end within the room below
    // `content_top`, and at least one.
    // A paragraph without lines has no pieces.
    extents.first()?;
    let mut start = extents
        .iter()
        .skip(1)
        .position(|extent| content_top + extent.1 > height + f32::EPSILON)
        .filter(|_| last_column.is_none_or(|last| context.column_index < last))
        .map_or(extents.len(), |fitting| fitting + 1);
    let mut ranges = vec![(0, start, context.column_index)];
    let mut column = context.column_index;
    while start < extents.len() {
        column += 1;
        let end = if last_column.is_some_and(|last| column >= last) {
            extents.len()
        } else {
            column_end(&extents, start, height)
        };
        ranges.push((start, end, column));
        start = end;
    }
    respect_orphans_and_widows(&mut ranges, context);
    Some(
        ranges
            .into_iter()
            .map(|(start, end, column)| MulticolTextFragment {
                line_start: start,
                line_end: end,
                fragmentainer: column,
                x: 0.0,
                y: extents[start].0,
            })
            .collect(),
    )
}

/// The end of the column that starts at the line `start` and takes lines for as
/// long as they fit in `height`, and at least one line.
fn column_end(extents: &[(f32, f32)], start: usize, height: f32) -> usize {
    let origin = extents[start].0;
    let mut end = start + 1;
    while end < extents.len() && extents[end].1 - origin <= height + f32::EPSILON {
        end += 1;
    }
    end
}

/// The least height above `height` at which the columns that take the lines
/// greedily at `height` would take one more line, if there is one.
fn next_taller_column(extents: &[(f32, f32)], height: f32) -> Option<f32> {
    let mut next: Option<f32> = None;
    let mut start = 0;
    while start < extents.len() {
        let end = column_end(extents, start, height);
        if let Some(line) = extents.get(end) {
            let taller = line.1 - extents[start].0;
            next = Some(next.map_or(taller, |least| least.min(taller)));
        }
        start = end;
    }
    next
}

/// The height of columns that balance in a container with the definite
/// `height`, given the `balanced` height their content needs: columns are as
/// tall as the content needs, up to that height (CSS Multi-column 1, 8.1).
/// `None` when the content needs all of `height` or more, and the columns
/// take it and go on in overflow columns.
fn balanced_below(balanced: f32, height: f32) -> Option<f32> {
    (balanced > 0.0 && balanced < height).then_some(balanced)
}

// cov:ignore: nested text fragment refresh is exercised by ignored multicol WPT reftests.
fn refresh_nested_text_fragments(
    tree: &mut LayoutSession<'_>,
    node_id: usize,
    context: FragmentationContext,
) {
    // Auto fill without a block constraint has no soft column breaks.
    let context =
        if context.available_height.is_none() && context.column_fill == ColumnFillValue::Auto {
            FragmentationContext {
                column_count: context.column_index.saturating_add(1),
                ..context
            }
        } else {
            context
        };
    let float_fragmentainer = tree.parent_of(node_id).and_then(|parent| {
        let parent_root = tree.nodes[parent].ifc.as_ref()?;
        parent_root
            .lines
            .as_ref()?
            .fragment_box_placements
            .iter()
            .find(|placement| placement.node_id == node_id)
            .map(|placement| placement.fragmentainer)
    });
    refresh_nested_text_fragments_in(
        tree,
        node_id,
        context,
        FlexFloatChain::of_ancestors(tree, node_id),
        float_fragmentainer,
    );
}

/// `chain` is the row-flex float chain of `node_id`, and
/// `float_fragmentainer` the column of its first placement in its parent's
/// paragraph. Both are carried down so that the subtree walk does not search
/// ancestors or sibling placements again for every node.
fn refresh_nested_text_fragments_in(
    tree: &mut LayoutSession<'_>,
    node_id: usize,
    context: FragmentationContext,
    chain: FlexFloatChain,
    float_fragmentainer: Option<usize>,
) {
    // A nested column container assigns the lines below it to its own
    // columns when it is laid out; the outer columns only move it as a whole.
    if tree.nodes[node_id].multicol.is_some() {
        return;
    }
    let nested_row_flex_scope = chain.is_scope();
    let floated = is_floated(tree, node_id);
    // A paragraph laid out by the inline engine keeps its lines on its root:
    // its fragments go there, and only its boxes hold text of their own.
    if let Some(root) = tree.nodes[node_id].ifc.as_mut() {
        root.multicol_fragment_origins_recorded = false;
        let committed_ranges = root
            .lines
            .as_ref()
            .and_then(|lines| lines.fragmentainer_line_ranges.clone());
        let extents: Vec<(f32, f32)> = root
            .lines
            .as_ref()
            .map(|lines| {
                lines
                    .lines
                    .iter()
                    .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
                    .collect()
            })
            .unwrap_or_default();
        root.multicol_fragments = if nested_row_flex_scope {
            float_fragmentainer
                .filter(|_| floated)
                .map(|fragmentainer| {
                    (!extents.is_empty())
                        .then_some(vec![MulticolTextFragment {
                            line_start: 0,
                            line_end: extents.len(),
                            fragmentainer,
                            x: 0.0,
                            y: extents[0].0,
                        }])
                        .unwrap_or_default()
                })
                .or(committed_ranges)
                .or_else(|| {
                    (!extents.is_empty()).then(|| {
                        line_ranges_in_columns(&extents, context, true)
                            .into_iter()
                            .map(|(start, end, column)| MulticolTextFragment {
                                line_start: start,
                                line_end: end,
                                fragmentainer: column,
                                x: 0.0,
                                y: extents[start].0,
                            })
                            .collect()
                    })
                })
        } else {
            committed_ranges.or_else(|| {
                (!extents.is_empty()).then(|| {
                    line_ranges_in_columns(&extents, context, false)
                        .into_iter()
                        .map(|(start, end, column)| MulticolTextFragment {
                            line_start: start,
                            line_end: end,
                            fragmentainer: column,
                            x: context.column_offset_x(column)
                                - context.column_offset_x(context.column_index),
                            y: extents[start].0,
                        })
                        .collect()
                })
            })
        };
        refresh_paragraph_boxes(tree, node_id, context, chain);
        return;
    }
    // This node has no paragraph, so no child has a placement in it.
    let children = tree.nodes[node_id].children.clone();
    for child in children {
        if tree.nodes[child].is_in_document() && !tree.nodes[child].is_display_none() {
            let child_chain = chain.below(tree, child);
            refresh_nested_text_fragments_in(tree, child, context, child_chain, None);
        }
    }
}

/// Refresh the text fragments of the boxes of the paragraph `node_id`, whose
/// own fragments are assigned. `chain` is the row-flex float chain of the
/// paragraph.
fn refresh_paragraph_boxes(
    tree: &mut LayoutSession<'_>,
    node_id: usize,
    context: FragmentationContext,
    chain: FlexFloatChain,
) {
    let mut first_columns = HashMap::<usize, usize>::new();
    if let Some(lines) = tree.nodes[node_id]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
    {
        for placement in &lines.fragment_box_placements {
            first_columns
                .entry(placement.node_id)
                .or_insert(placement.fragmentainer);
        }
    }
    // An atomic inline sits on a line, so its text lies in the column of that
    // line, in one piece.
    let atomic_columns: HashMap<usize, usize> = tree.nodes[node_id]
        .ifc
        .as_ref()
        .filter(|_| !chain.is_scope())
        .and_then(|root| {
            let fragments = root.multicol_fragments.as_ref()?;
            let lines = root.lines.as_ref()?;
            Some(
                crate::layout::ifc::boxes::atomic_fragments(&root.boxes, &lines.lines, fragments)
                    .into_iter()
                    .map(|(atomic, fragment)| (atomic, fragment.fragmentainer))
                    .collect(),
            )
        })
        .unwrap_or_default();
    let boxes = tree.nodes[node_id].ifc_boxes();
    for child in boxes {
        // A box may sit inside inline elements of the paragraph; only
        // those elements lie between it and this root.
        let between: Vec<_> = tree
            .ancestors(child)
            .take_while(|&id| id != node_id)
            .collect();
        let inside_root = tree.parent_of(between.last().copied().unwrap_or(child)) == Some(node_id);
        let child_chain = if inside_root {
            between
                .into_iter()
                .rev()
                .fold(chain, |above, id| above.below(tree, id))
                .below(tree, child)
        } else {
            FlexFloatChain::of_ancestors(tree, child) // cov:ignore: paragraph boxes are descendants of their root.
        };
        let float_fragmentainer = (tree.parent_of(child) == Some(node_id))
            .then(|| first_columns.get(&child).copied())
            .flatten();
        let child_context = atomic_columns
            .get(&child)
            .map_or(context, |&column| context.confined_to(column));
        refresh_nested_text_fragments_in(
            tree,
            child,
            child_context,
            child_chain,
            float_fragmentainer,
        );
    }
}

/// Visit every node of the arena once, each parent before its children.
pub(in crate::layout) fn visit_top_down(
    doc: &Document,
    mut visit: impl FnMut(Option<usize>, usize),
) {
    let mut resolved = vec![false; doc.nodes.len()];
    let mut chain = Vec::new();
    for start in 0..doc.nodes.len() {
        // Collect the unresolved part of this node's chain, then resolve it
        // from the top down.
        let mut current = Some(start);
        while let Some(id) = current.filter(|&id| !resolved[id]) {
            resolved[id] = true;
            chain.push(id);
            current = doc.parent_of(id);
        }
        while let Some(id) = chain.pop() {
            visit(doc.parent_of(id), id);
        }
    }
}

// cov:ignore: exercised by ignored WPT regression and multicol reftests; default coverage skips ignored reftests.
fn has_nonzero_horizontal_margin(cv: &ComputedValues) -> bool {
    let is_zero = |value: &ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() <= f32::EPSILON
        }
        _ => false,
    };
    !is_zero(&cv.margin.left) || !is_zero(&cv.margin.right)
}

/// Whether `id` has an `auto` width that follows from its height through a
/// preferred aspect ratio, so the box does not stretch to its container: from
/// a definite height, or from a `min-height` or `max-height` that the ratio
/// carries across to the width.
fn width_follows_height(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let cv = &cascade.computed[id];
    matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
        && [&cv.height, &cv.min_height, &cv.max_height]
            .iter()
            .any(|size| !matches!(size, ComputedLengthPercentageOrAuto::Auto))
        && AuthoredBox::new(doc, cascade, id).has_preferred_aspect_ratio()
}

/// Ancestor-chain facts for every node of the arena, derived once with each
/// parent resolved before its children. The multicol preparation pass asks
/// these questions of every element; walking to the root for each one would
/// cost the node count times the tree depth. Rebreaking uses the same table
/// with fresh page-width and measured-font inputs.
pub(crate) struct AncestorFacts {
    /// A strict ancestor is a multicol container.
    multicol_ancestor: Vec<bool>,
    /// The node or an ancestor has a vertical or sideways writing mode.
    vertical: Vec<bool>,
    /// A strict ancestor is absolutely or fixed positioned.
    out_of_flow_ancestor: Vec<bool>,
    /// A strict ancestor establishes a flex, grid, or table formatting context,
    /// or takes its width from its height through an aspect ratio.
    non_block_flow_ancestor: Vec<bool>,
    /// The content width used to resolve descendants. Projected auto block
    /// widths replace their initial inherited value before descendants run.
    content_width: Vec<f32>,
    /// The nearest strict ancestor that supplies a containing content width.
    /// Auto and intrinsic widths are skipped unless this pass projects the auto width.
    containing_width_ancestor: Vec<Option<usize>>,
    /// An auto-width block this pass projects to contain descendants.
    projected_auto_width: Vec<bool>,
    /// A projected block that makes a block formatting context of its own. It
    /// keeps its border box clear of the floats beside it (CSS 2.1 section
    /// 9.5), so its `auto` width is the room they leave, which only layout
    /// knows. Its projected width still answers where a paragraph breaks
    /// again, but layout sizes it and the blocks inside it.
    context_root: Vec<bool>,
    /// The node, or an ancestor that sizes it, is a context root.
    in_context_root: Vec<bool>,
    /// Layout, not the authored widths, decides the width of the nearest
    /// strict ancestor with a box: it has an `auto` or keyword width, and this
    /// pass does not project it (it fits its content, or leaves room for its
    /// margins and the like) or sits in such a box itself. The width of the
    /// authored ancestor further up is not the width of the blocks inside.
    laid_out_container: Vec<bool>,
    /// The node or an ancestor has a `min-content`, `max-content` or
    /// `fit-content` width.
    intrinsic_width: Vec<bool>,
    /// The node is an inline box, whose `width` applies to neither it nor the
    /// blocks it holds.
    inline_box: Vec<bool>,
}

/// Whether the block `id` makes a block formatting context of its own: a
/// scroll container, a box with layout or paint containment or with
/// `align-content` other than `normal`, or a multicol container. Overflow
/// propagated to the viewport is not the box's own.
/// Normal projection queries the source lazily; rebreaking supplies a value
/// cached before its additional walk.
fn roots_formatting_context(
    doc: &Document,
    cascade: &CascadeResult,
    id: usize,
    viewport_source: impl FnOnce() -> Option<usize>,
) -> bool {
    let cv = &cascade.computed[id];
    let scrolls = |overflow| !matches!(overflow, OverflowValue::Visible | OverflowValue::Clip);
    (scrolls(cv.overflow.x) || scrolls(cv.overflow.y)) && viewport_source() != Some(id)
        || cv
            .contain
            .contains(raikiri_style::property::Contain::LAYOUT)
        || cv.contain.contains(raikiri_style::property::Contain::PAINT)
        || cv.align_content != raikiri_style::property::ContentAlignmentValue::Normal
        || doc.nodes[id].multicol.is_some()
}

impl AncestorFacts {
    pub(crate) fn new(doc: &Document, cascade: &CascadeResult, fallback_width: f32) -> Self {
        let count = doc.nodes.len();
        let mut facts = Self {
            multicol_ancestor: vec![false; count],
            vertical: vec![false; count],
            out_of_flow_ancestor: vec![false; count],
            non_block_flow_ancestor: vec![false; count],
            content_width: vec![0.0; count],
            containing_width_ancestor: vec![None; count],
            projected_auto_width: vec![false; count],
            context_root: vec![false; count],
            in_context_root: vec![false; count],
            laid_out_container: vec![false; count],
            intrinsic_width: vec![false; count],
            inline_box: super::style_views::metadata::inline_boxes(doc, cascade),
        };
        visit_top_down(doc, |parent, id| {
            facts.resolve(doc, cascade, parent, id, fallback_width);
        });
        facts
    }

    /// Include layout-sized block contexts when breaking existing lines again.
    /// Projection keeps its narrower context-root classification; only this
    /// optional operation needs the contexts that projection cannot size.
    pub(crate) fn for_rebreak(
        doc: &Document,
        cascade: &CascadeResult,
        fallback_width: f32,
    ) -> Self {
        let mut facts = Self::new(doc, cascade, fallback_width);
        let viewport_source = super::body_overflow::viewport_overflow_source(doc, cascade);
        visit_top_down(doc, |parent, id| {
            let node = &doc.nodes[id];
            let has_box = node.kind() == NodeKind::Element
                && !facts.inline_box[id]
                && !matches!(node.display(), DisplayValue::None | DisplayValue::Contents);
            let context = has_box
                && (roots_formatting_context(doc, cascade, id, || viewport_source)
                    || matches!(
                        node.display(),
                        DisplayValue::FlowRoot
                            | DisplayValue::InlineBlock
                            | DisplayValue::TableCell
                            | DisplayValue::TableCaption
                    )
                    || node.roots_block_context()
                    || AuthoredBox::new(doc, cascade, id).is_floated()
                    || matches!(
                        node.position(),
                        PositionValue::Absolute | PositionValue::Fixed
                    ));
            facts.in_context_root[id] = context
                || parent.is_some_and(|parent| {
                    facts.in_context_root[parent]
                        && !facts.non_block_flow_ancestor[id]
                        && !facts.multicol_ancestor[id]
                });
        });
        facts
    }

    fn resolve(
        &mut self,
        doc: &Document,
        cascade: &CascadeResult,
        parent: Option<usize>,
        id: usize,
        fallback_width: f32,
    ) {
        let vertical_self = cascade
            .authored_writing_modes
            .get(id)
            .copied()
            .flatten()
            .is_some_and(WritingMode::is_vertical);
        let parent_width = parent.map_or(fallback_width, |parent| self.content_width[parent]);
        let cv = &cascade.computed[id];
        // The `width` of a non-atomic inline box does not apply to it, so it
        // supplies no width to the blocks it holds (CSS 2.1 section 10.3.1).
        let inline_box = self.inline_box[id];
        let authored_width = match cv.width {
            _ if inline_box => None,
            ComputedLengthPercentageOrAuto::Px(value) if value.is_finite() => Some(value),
            ComputedLengthPercentageOrAuto::Percent(value) if value.is_finite() => {
                Some(parent_width * value / 100.0)
            }
            ComputedLengthPercentageOrAuto::Calc(calc)
                if calc.px.is_finite() && calc.percent.is_finite() =>
            {
                Some(calc.px + parent_width * calc.percent / 100.0)
            }
            ComputedLengthPercentageOrAuto::Math(ref value) => {
                Some(raikiri_layout_style::AlignedCalc::resolve_math_expression(
                    value,
                    true,
                    parent_width,
                ))
            }
            _ => None,
        };
        self.content_width[id] = if doc.nodes[id].display() == DisplayValue::Contents {
            // Contents has no box; both preparation and rebreaking inherit its parent width.
            parent_width.max(0.0)
        } else if let Some(width) = authored_width {
            let width = width.max(0.0);
            let authored = AuthoredBox::new(doc, cascade, id);
            if authored.is_border_box() {
                (width - authored.horizontal_edges(parent_width.max(0.0))).max(0.0)
            } else {
                width
            }
        } else {
            parent_width.max(0.0)
        };
        self.intrinsic_width[id] = super::bridge::width_sized_by_keyword(doc, cascade, id)
            || parent.is_some_and(|parent| self.intrinsic_width[parent]);
        let Some(parent) = parent else {
            self.vertical[id] = vertical_self;
            return;
        };
        let values = &cascade.computed[parent];
        self.multicol_ancestor[id] =
            self.multicol_ancestor[parent] || doc.nodes[parent].multicol.is_some();
        self.vertical[id] = vertical_self || self.vertical[parent];
        self.out_of_flow_ancestor[id] = self.out_of_flow_ancestor[parent]
            || matches!(
                doc.nodes[parent].position(),
                PositionValue::Absolute | PositionValue::Fixed
            );
        self.non_block_flow_ancestor[id] = self.non_block_flow_ancestor[parent]
            || width_follows_height(doc, cascade, parent)
            || (doc.nodes[parent].display().is_flex_or_grid_container()
                || matches!(
                    doc.nodes[parent].display(),
                    DisplayValue::Table
                        | DisplayValue::InlineTable
                        | DisplayValue::TableRowGroup
                        | DisplayValue::TableHeaderGroup
                        | DisplayValue::TableFooterGroup
                        | DisplayValue::TableRow
                        | DisplayValue::TableColumnGroup
                        | DisplayValue::TableColumn
                        | DisplayValue::TableCell
                        | DisplayValue::TableCaption
                ));
        self.containing_width_ancestor[id] = if self.projected_auto_width[parent] {
            Some(parent)
        } else if doc.nodes[parent].display() == DisplayValue::Contents
            || values.width == ComputedLengthPercentageOrAuto::Auto
            || values.width.is_content_keyword()
            || self.inline_box[parent]
        {
            self.containing_width_ancestor[parent]
        } else {
            Some(parent)
        };
        self.laid_out_container[id] =
            if doc.nodes[parent].display() == DisplayValue::Contents || self.inline_box[parent] {
                // Neither has a box to size: the blocks inside belong to the
                // container around it.
                self.laid_out_container[parent]
            } else {
                match &values.width {
                    // A length is the width of the box wherever the box is.
                    ComputedLengthPercentageOrAuto::Px(_) => false,
                    // Layout decides an `auto` width, unless this pass
                    // projects it from a container whose width is known.
                    ComputedLengthPercentageOrAuto::Auto => {
                        !self.projected_auto_width[parent]
                            || self.context_root[parent]
                            || self.laid_out_container[parent]
                    }
                    width if width.is_content_keyword() => true,
                    // A percentage refers to the container, so it is as
                    // known as the container is.
                    _ => self.laid_out_container[parent],
                }
            };
        self.projected_auto_width[id] = self.can_project_auto_width(doc, cascade, id);
        // `laid_out_container` of the children reads `context_root[id]`, so it
        // is settled here, right after the projected test it depends on.
        self.context_root[id] = self.projected_auto_width[id]
            && roots_formatting_context(doc, cascade, id, || {
                super::body_overflow::viewport_overflow_source(doc, cascade)
            });
        // Only blocks in ordinary block flow below a root are sized by it.
        self.in_context_root[id] = self.context_root[id]
            || (self.in_context_root[parent]
                && !self.non_block_flow_ancestor[id]
                && !self.multicol_ancestor[id]);
        if self.projected_auto_width[id]
            && let Some(width) = self.authored_containing_width(id)
        {
            let authored = AuthoredBox::new(doc, cascade, id);
            let horizontal = authored.horizontal_edges(width);
            let content_constraint = |value: f32| {
                if authored.is_border_box() {
                    (value - horizontal).max(0.0)
                } else {
                    value
                }
            };
            let minimum = authored
                .min_width(width)
                .map(content_constraint)
                .unwrap_or(0.0);
            let maximum = authored.max_width(width).map(content_constraint);
            let content_width = ((width - horizontal).max(0.0)).max(minimum);
            self.content_width[id] = maximum
                .map(|maximum| content_width.min(maximum.max(minimum)))
                .unwrap_or(content_width);
        }
    }

    fn can_project_auto_width(&self, doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
        let node = &doc.nodes[id];
        if node.kind() != NodeKind::Element {
            return false;
        }
        let cv = &cascade.computed[id];
        if self.containing_width_ancestor[id].is_none()
            || self.multicol_ancestor[id]
            || self.vertical[id]
            || !matches!(
                node.display(),
                DisplayValue::Block | DisplayValue::InlineBlock
            )
            || cv.width != ComputedLengthPercentageOrAuto::Auto
            || !matches!(node.position(), PositionValue::Static)
            || has_nonzero_horizontal_margin(cv)
            // A float shrinks to fit, and so does a button, whose `auto` width
            // is the fit-content one; a box whose width follows its height
            // through the aspect ratio does not stretch.
            || node.is_html_button()
            // A fieldset roots its own formatting context, so its `auto`
            // width is the room the floats beside it leave.
            || node.roots_block_context()
            || AuthoredBox::new(doc, cascade, id).is_floated()
            || width_follows_height(doc, cascade, id)
            || self.out_of_flow_ancestor[id]
            || self.non_block_flow_ancestor[id]
            || AuthoredBox::new(doc, cascade, id).is_replaced()
            // A box that `justify-self` places at a position fits its content.
            || justify_self_positions(doc, cascade, id)
        {
            return false;
        }

        let scrolls = |overflow| !matches!(overflow, OverflowValue::Visible | OverflowValue::Clip);
        let engine_sized = node.display() == DisplayValue::InlineBlock
            || scrolls(cv.overflow.x)
            || scrolls(cv.overflow.y);
        // The inline engine sizes these boxes from the line width or float slot.
        let in_ifc_engine = engine_sized
            && !node.in_ifc_subtree()
            && doc.parent_of(id).is_some_and(|parent| {
                doc.nodes[parent].is_ifc_root() || doc.nodes[parent].in_ifc_subtree()
            });
        !in_ifc_engine
    }

    /// The content width supplied by the nearest authored or projected auto-width ancestor.
    pub(crate) fn authored_containing_width(&self, id: usize) -> Option<f32> {
        self.containing_width_ancestor
            .get(id)
            .copied()
            .flatten()
            .map(|ancestor| self.content_width[ancestor])
    }

    /// Layout, not the projected width, sized the horizontal box `id` or the
    /// box around it, because that box makes a formatting context of its own.
    pub(crate) fn sized_by_context_root(&self, id: usize) -> bool {
        self.in_context_root[id] && !self.vertical[id]
    }

    pub(crate) fn content_width(&self, id: usize) -> Option<f32> {
        self.content_width.get(id).copied()
    }

    /// `id` or one of its ancestors has a `min-content`, `max-content` or
    /// `fit-content` width.
    pub(crate) fn in_intrinsic_width(&self, id: usize) -> bool {
        self.intrinsic_width[id]
    }
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
pub(crate) fn line_height_px(cv: &ComputedValues) -> f32 {
    let font_size = cv.font_size.px().max(0.0);
    match cv.line_height {
        ComputedLineHeight::Length(value) => value.0.max(0.0),
        ComputedLineHeight::Number(value) => (font_size * value).max(0.0),
        ComputedLineHeight::Normal => (font_size * 1.2).max(0.0),
    }
}

/// The line ranges of a multicol container's own paragraph in its columns,
/// and the height the lines need: the tallest column. The lines fill the
/// columns of `context` by its column height when it is definite, else they
/// are balanced over them, with `widows` and `orphans` applied at each break
/// (CSS Multi-column 1, 7; CSS Fragmentation 3, 3.3). With `column-fill:auto`
/// and no definite height, soft breaks are disabled and all lines stay in the
/// current column.
pub(crate) fn root_column_fragments(
    lines: &crate::layout::ifc::root::IfcLines,
    context: FragmentationContext,
) -> (Vec<MulticolTextFragment>, f32) {
    group_column_fragments(lines, 0..lines.lines.len(), context, 0.0)
}

/// [`root_column_fragments`] for the lines `range` of the paragraph, which
/// stand with their first line at the block offset `y` of the root's content
/// box: a group of lines between two column spanners.
pub(crate) fn group_column_fragments(
    lines: &crate::layout::ifc::root::IfcLines,
    range: std::ops::Range<usize>,
    context: FragmentationContext,
    y: f32,
) -> (Vec<MulticolTextFragment>, f32) {
    let extents: Vec<(f32, f32)> = lines.lines[range.clone()]
        .iter()
        .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
        .collect();
    if extents.is_empty() {
        return (Vec::new(), 0.0);
    }
    // Columns that balance are as tall as the lines need, up to the definite
    // height of the container if it has one.
    let balanced_context = if context.column_fill != ColumnFillValue::Auto {
        balanced_column_height(&extents, context).map(|height| FragmentationContext {
            available_height: Some(height),
            ..context
        })
    } else {
        None
    };
    let layout_context = balanced_context.unwrap_or_else(|| {
        if context.available_height.is_none() && context.column_fill == ColumnFillValue::Auto {
            FragmentationContext {
                column_count: context.column_index.saturating_add(1),
                ..context
            }
        } else {
            context
        }
    });
    let ranges = line_ranges_in_columns(&extents, layout_context, false);
    let height = ranges
        .iter()
        .map(|&(start, end, _)| extents[end - 1].1 - extents[start].0)
        .fold(0.0_f32, f32::max);
    let fragments = ranges
        .into_iter()
        .map(|(start, end, column)| MulticolTextFragment {
            line_start: range.start + start,
            line_end: range.start + end,
            fragmentainer: column,
            x: context.column_offset_x(column),
            y,
        })
        .collect();
    (fragments, height)
}

/// Whether the `justify-self` of block `id`, or the `justify-items` of its
/// parent for `auto`, places it at a position rather than stretching it: such
/// a block takes its fit-content width (CSS Box Alignment 3 §6.1).
fn justify_self_positions(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    use raikiri_style::property::{AlignSelfValue, SelfAlignmentValue};
    let alignment = match cascade.computed[id].justify_self {
        AlignSelfValue::Value(value) => value,
        // The box that lays this one out holds the `justify-items`, past
        // any `display: contents` parent.
        _ => std::iter::successors(doc.parent_of(id), |&parent| doc.parent_of(parent))
            .find(|&parent| doc.nodes[parent].display() != DisplayValue::Contents)
            // `justify-items` does not apply to an inline box.
            .filter(|&parent| doc.nodes[parent].display() != DisplayValue::Inline)
            .map_or(SelfAlignmentValue::Normal, |parent| {
                cascade.computed[parent].justify_items
            }),
    };
    !matches!(
        alignment,
        SelfAlignmentValue::Normal | SelfAlignmentValue::Stretch
    )
}

/// Give `auto`-width blocks outside any multicol container the width of their
/// authored containing block before layout. Descendants of a multicol
/// container are left to the multicol dispatch in [`compute_multicol_layout`],
/// which supplies their used widths, so they are not guessed from a page
/// fallback. A replaced box keeps its natural width (CSS 2.1 §10.3.4). The
/// blocks inside an `auto`-width box that this pass does not project itself
/// (one that shrinks to fit, or leaves room for its margins) are left to layout
/// too: the width of the authored ancestor above such a box is not theirs.
pub(crate) fn prepare_authored_block_widths(
    doc: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    fallback_width: f32,
) {
    let ancestors = AncestorFacts::new(doc, cascade, fallback_width);
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        // Used widths for descendants of a multicol container are supplied by
        // the recursive Taffy seam. Do not pre-project them from cascade
        // fallback widths; that was the source of the nested-width bug.
        if ancestors.multicol_ancestor[idx] {
            continue;
        }
        if !ancestors.projected_auto_width[idx]
            || ancestors.context_root[idx]
            || ancestors.laid_out_container[idx]
        {
            continue;
        }
        if let Some(width) = ancestors.authored_containing_width(idx) {
            let used_width = if AuthoredBox::new(doc, cascade, idx).is_border_box() {
                width
            } else {
                ancestors.content_width[idx]
            };
            doc.record_pass_override(
                idx,
                raikiri_layout_style::OwnedPropertyOverride::length(
                    raikiri_layout_style::LengthProperty::Width,
                    ComputedLengthPercentageOrAuto::Px(used_width),
                    true,
                )
                .expect("resolved physical multicol width"),
            );
        }
    }
}

#[cfg(test)]
mod tests;
