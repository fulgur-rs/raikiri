use super::*;
use crate::layout::cache_context::SavedLayoutCache;
use raikiri_layout_style::values::{
    AlignItems, AlignItemsKeyword, BoxSizing, Dimension, ExpandedDimension, LengthPercentageAuto,
    ResolveOrZero,
};
use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// Run `f`, which may take the layout caches of boxes it overrides and push the
/// overrides, and then leave every override undone and every cache it took put
/// back, however `f` ends.
pub(super) fn with_saved_probe_caches<R>(
    tree: &mut LayoutSession<'_>,
    capacity: usize,
    f: impl FnOnce(&mut LayoutSession<'_>, &mut Vec<(usize, SavedLayoutCache)>) -> R,
) -> R {
    let checkpoint = tree.override_checkpoint();
    let mut saved = Vec::with_capacity(capacity);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let value = f(tree, &mut saved);
        tree.restore_overrides_to(checkpoint);
        value
    }));
    tree.restore_overrides_to(checkpoint);
    for (idx, cache) in saved.into_iter().rev() {
        tree.restore_layout_cache(idx, cache);
    }
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}

/// Run `probe`, which measures the intrinsic widths of the flex container
/// `node_id`, with what Taffy leaves out of the measure put in place.
///
/// Taffy breaks the lines of a wrapping container at its own `max-width` even
/// while it measures the container's max-content width, which is the width of
/// a single line (CSS Flexbox §9.9.1). The container's `max-width` is dropped
/// for the probe; the caller clamps the result by it. The items are bounded by
/// their flex base sizes while the container lays them out
/// ([`with_flex_item_basis_bounds`]), as they are when the container is asked
/// for its min-content or max-content width without this probe.
///
/// A container with no `max-width` has nothing to drop, and the probe leaves
/// its style alone. A container with one drops it in a scope that keeps the
/// cache generation ([`LayoutSession::push_local_overrides`]), so the sizes
/// already measured inside the container, and the sizes of the boxes around it
/// that are being laid out, stay in the caches. Moving the generation instead
/// would have a chain of nested containers measure the one inside it again at
/// each of the widths its parent tries.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#intrinsic-sizes>
pub(super) fn with_flex_intrinsic_probe<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    probe: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    tree.sync_layout_cache_mutations();
    let idx = usize::from(node_id);
    let key = tree
        .nodes
        .box_key(idx)
        .expect("intrinsic container has a box key");
    let entries = if tree.physical_style(idx).max_size().width.is_auto() {
        Vec::new()
    } else {
        vec![(
            key,
            O::length(P::MaxWidth, L::Auto, false).expect("physical intrinsic maximum"),
        )]
    };
    let outer = tree.context.flex_intrinsic_probe.replace(idx);
    let result = catch_unwind(AssertUnwindSafe(|| {
        tree.with_local_overrides(entries, probe)
    }));
    tree.context.flex_intrinsic_probe = outer;
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}

/// What the automatic minimum size of a flex item with a horizontal main axis
/// depends on.
struct AutomaticMinimumInputs {
    /// Whether the item is a scroll container, which has no automatic minimum.
    scrolls: bool,
    /// The `width` the item states, if it is a length.
    specified: Option<f32>,
    /// The `max-width` of the item, if it is a length.
    maximum: Option<f32>,
    /// The border-box height the item has before its width is measured, if it
    /// has one ([`height_before_width`]).
    height: Option<f32>,
    /// The padding and border of the item across its main axis, when its sizes
    /// are those of its content box, and zero when they are those of its border
    /// box.
    across: f32,
}

/// The border-box height the item `style` of a row flex container has before
/// its width is measured, if it has one. That is the height it states, a
/// percentage of the definite inner height `room` of the container, or the
/// height the container gives it: `room` less the margins of the item, within
/// its `min-height` and `max-height`. The container gives it to an item with
/// `height: stretch` and to an item with an `auto` height that the container
/// stretches, which a container of one line does to an item aligned to stretch
/// that has no `auto` margin across (CSS Flexbox 1 section 9.4). The height is
/// no less than the padding and border of the item.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#definite-sizes>
fn height_before_width(
    style: &LayoutStyleView<'_>,
    room: Option<f32>,
    single_line: bool,
    container_align: AlignItems,
) -> Option<f32> {
    let edges = {
        let padding = style.padding().resolve_or_zero(None);
        let border = style.border().resolve_or_zero(None);
        padding.top + padding.bottom + border.top + border.bottom
    };
    let content_box = style.box_sizing() == BoxSizing::ContentBox;
    let outer = if content_box { edges } else { 0.0 };
    let height = style.size().height;
    if let Some(stated) = crate::layout::resolve_dimension(height, room) {
        return Some((stated + outer).max(edges));
    }
    let room = room?;
    let margin = style.margin();
    // The self alignment of an item whose height is a content keyword is not
    // `stretch`: the style reads the keyword as `auto` but does not stretch it.
    let stretched = matches!(height.expand(), ExpandedDimension::Stretch)
        || (height.is_auto()
            && single_line
            && !margin.top.is_auto()
            && !margin.bottom.is_auto()
            && matches!(
                style.align_self().unwrap_or(container_align).keyword,
                AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch
            ));
    stretched.then(|| {
        // A percentage margin counts as zero, as in the width of the container.
        let bound = |length| {
            crate::layout::resolve_length_percentage_auto(length, Some(room))
                .map(|bound| bound + outer)
        };
        (room - margins_sum(margin.top, margin.bottom))
            .min(bound(style.max_size().height).unwrap_or(f32::INFINITY))
            .max(bound(style.min_size().height).unwrap_or(0.0))
            .max(edges)
    })
}

/// The sum of two opposite margins of the item, where an `auto` margin and a
/// percentage of a size the probe has not got count as zero.
fn margins_sum(start: LengthPercentageAuto, end: LengthPercentageAuto) -> f32 {
    [start, end]
        .into_iter()
        .map(|margin| crate::layout::resolve_length_percentage_auto(margin, None).unwrap_or(0.0))
        .sum()
}

/// The inputs that ask a flex item for its width at its min-content size,
/// measured with the border-box `height` it has before its width is measured, if
/// it has one. A `sizing_mode` of `InherentSize` gives the item the width it
/// states within its `min-width` and `max-width`.
fn min_content_probe(sizing_mode: SizingMode, height: Option<f32>) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height,
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: height.is_some(),
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

/// The automatic minimum size of the item `child` of a row flex container, as
/// the value of a `min-width` of the item's own: the smaller of its min-content
/// size and the width it states, within its `max-width` (CSS Flexbox 1 section
/// 4.5). The item is measured with the height it has before its width is
/// measured, which gives a replaced item with an aspect ratio the width that
/// follows from that height.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#min-size-auto>
fn automatic_minimum_width(
    tree: &mut LayoutSession<'_>,
    child: NodeId,
    inputs: AutomaticMinimumInputs,
) -> f32 {
    if inputs.scrolls {
        return 0.0;
    }
    let across = inputs.across;
    let measured = tree
        .compute_child_layout(
            child,
            min_content_probe(SizingMode::ContentSize, inputs.height),
        )
        .size
        .width;
    let bounded = [inputs.specified, inputs.maximum]
        .into_iter()
        .flatten()
        .fold(measured, |minimum, bound| minimum.min(bound + across));
    (bounded - across).max(0.0)
}

/// Run `layout`, a layout of the row flex container `node_id`, with each item
/// whose flex base size is a length bounded by it. `cross_room` is the definite
/// inner height of the container, if it has one: the room that its items take
/// their height from while they are measured for their minimum width.
///
/// An item contributes at least its flex base size to the intrinsic widths of
/// the container when it cannot shrink and at most that size when it cannot
/// grow. Taffy clamps a measured contribution by the item's `min-width` and
/// `max-width` only, so an empty `flex: 1 0 100px` item would contribute
/// nothing. Each such bound is installed as the item's own bound, together
/// with the bound the item already has, which is what a keyword `min-width`
/// or `max-width` came to when the container began laying out its items.
///
/// Taffy takes the `width` of an item as its contribution without measuring
/// the item, so a `width` the flex base size bounds is set to the bounded
/// width instead of installing a bound. Where Taffy measures the item it
/// clamps the contribution with the item's margins counted in, so the margins
/// are counted in the upper bound as well and survive it.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#intrinsic-item-contributions>
pub(super) fn with_flex_item_basis_bounds<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    cross_room: Option<f32>,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    tree.sync_layout_cache_mutations();
    let container = tree.physical_style(usize::from(node_id));
    let is_row = container.flex_direction_is_row();
    let single_line = !container.flex_wraps();
    let container_align = container.align_items();
    let children: Vec<NodeId> = if is_row {
        tree.child_ids(node_id).collect()
    } else {
        Vec::new()
    };
    with_saved_probe_caches(tree, children.len(), |tree, saved| {
        for child in children {
            let child_idx = usize::from(child);
            #[cfg(test)]
            super::tests::intrinsic_scope_tests::before_child_bounds(tree, child_idx);
            let style = tree.physical_style(child_idx);
            if style.box_generation_mode() == BoxGenerationMode::None
                || style.position() == Position::Absolute
            {
                continue;
            }
            let Some(basis) = crate::layout::resolve_dimension(style.flex_basis(), None)
                .filter(|basis| basis.is_finite())
            else {
                continue;
            };
            let (width, min, max) = (
                style.size().width,
                style.min_size().width,
                style.max_size().width,
            );
            let margin = style.margin();
            // A percentage of the container's width, which the probe has not got,
            // bounds nothing, and is no margin either.
            let min_length = crate::layout::resolve_length_percentage_auto(min, None);
            let max_length = crate::layout::resolve_length_percentage_auto(max, None);
            let margins = margins_sum(margin.left, margin.right);
            let (cannot_shrink, cannot_grow) =
                (style.flex_shrink() == 0.0, style.flex_grow() == 0.0);
            let preferred = crate::layout::resolve_dimension(width, None);
            let automatic_minimum = min.is_auto().then(|| AutomaticMinimumInputs {
                scrolls: is_scroll_container(style.overflow()),
                specified: preferred,
                maximum: max_length,
                height: height_before_width(&style, cross_room, single_line, container_align),
                across: if style.box_sizing() == BoxSizing::ContentBox {
                    let padding = style.padding().resolve_or_zero(None);
                    let border = style.border().resolve_or_zero(None);
                    padding.left + padding.right + border.left + border.right
                } else {
                    0.0
                },
            });
            // The flex base size floors the contribution, and then the item's own
            // bounds clamp it, the minimum over the maximum.
            let new_min = cannot_shrink.then(|| {
                let floor = max_length.map_or(basis, |max| basis.min(max));
                min_length.map_or(floor, |min| min.max(floor))
            });
            let new_max = (cannot_grow && preferred.is_none())
                .then(|| max_length.map_or(basis, |m| m.min(basis)) + margins);
            let new_width = preferred
                .filter(|_| cannot_shrink || cannot_grow)
                .map(|preferred| {
                    let floored = if cannot_shrink {
                        preferred.max(basis)
                    } else {
                        preferred
                    };
                    Dimension::length(if cannot_grow {
                        floored.min(basis)
                    } else {
                        floored
                    })
                });
            // A bound installed as `min-width` or `max-width` is an explicit one,
            // which leaves the item no automatic minimum size. The contribution
            // of an item with one is no less than that size however far its flex
            // base size would bound it.
            let automatic = automatic_minimum
                .filter(|_| new_min.is_some() || new_max.is_some())
                .map(|inputs| automatic_minimum_width(tree, child, inputs));
            let new_min =
                new_min.map(|value| automatic.map_or(value, |minimum| value.max(minimum)));
            let new_max = new_max
                .map(|value| automatic.map_or(value, |minimum| value.max(minimum + margins)));
            if new_min.is_none() && new_max.is_none() && new_width.is_none() {
                continue;
            }
            let key = tree
                .nodes
                .box_key(child_idx)
                .expect("flex child has a box key");
            let mut entries = Vec::new();
            // The installed minimum already accounts for the maximum, so Taffy's
            // separate flex-base floor must not apply again during this probe.
            if cannot_shrink {
                entries.push((key, O::flex_shrink(1.0)));
            }
            for (property, value) in [
                (
                    P::Width,
                    new_width.and_then(|value| crate::layout::resolve_dimension(value, None)),
                ),
                (P::MinWidth, new_min),
                (P::MaxWidth, new_max),
            ] {
                if let Some(value) = value {
                    entries.push((
                        key,
                        O::length(property, L::Px(value), true).expect("physical flex base bound"),
                    ));
                }
            }
            // Record ownership before publishing any changed field. Earlier items
            // stay bounded while later items prepare their own entries.
            let cache = tree.take_layout_cache(child_idx);
            saved.push((child_idx, cache));
            tree.push_local_overrides(entries);
        }
        layout(tree)
    })
}

/// The width, less the padding and border of the container, of the row flex
/// container `node_id` that wraps its items at its min-content width: that of
/// its widest item, with the margins of the item. The items break onto lines of
/// their own, whatever flex base sizes they have, so no flex base size, and no
/// `flex-grow` or `flex-shrink`, enters it: Chromium sizes `flex: 0 0 100px;
/// width: 30px` at 30px here, though the item is 100px wide once laid out. The
/// flex base sizes bound the items at the max-content width of the container
/// ([`with_flex_item_basis_bounds`]), which is no less than this one.
///
/// What an item contributes is its min-content size, or the width it states,
/// within its `min-width` and `max-width`; the item is measured with the height
/// it has before its width is measured, as [`automatic_minimum_width`] does.
/// `cross_room` is the definite inner height of the container, if it has one.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#intrinsic-sizes>
pub(super) fn wrapping_min_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    cross_room: Option<f32>,
) -> f32 {
    let container_align = tree.physical_style(usize::from(node_id)).align_items();
    let children: Vec<NodeId> = tree.child_ids(node_id).collect();
    let mut widest: f32 = 0.0;
    for child in children {
        let style = tree.physical_style(usize::from(child));
        if style.box_generation_mode() == BoxGenerationMode::None
            || style.position() == Position::Absolute
        {
            continue;
        }
        let margin = style.margin();
        let margins = margins_sum(margin.left, margin.right);
        let height = height_before_width(&style, cross_room, false, container_align);
        let contribution = tree
            .compute_child_layout(child, min_content_probe(SizingMode::InherentSize, height))
            .size
            .width;
        widest = widest.max(contribution + margins);
    }
    widest
}
