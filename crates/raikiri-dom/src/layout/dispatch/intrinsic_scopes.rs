use super::*;
use crate::layout::cache_context::SavedLayoutCache;
use raikiri_layout_style::values::{BoxSizing, Dimension, ResolveOrZero};
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
/// ([`with_flex_item_basis_bounds`]).
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
    /// The `height` the item states, if it is a length.
    height: Option<f32>,
    /// Whether the sizes of the item are those of its content box.
    content_box: bool,
    /// The padding and border of the item, left and right, top and bottom.
    edges: Size<f32>,
}

/// The automatic minimum size of the item `child` of a row flex container, as
/// the value of a `min-width` of the item's own: the smaller of its min-content
/// size and the width it states, within its `max-width` (CSS Flexbox 1 section
/// 4.5). The item is measured with the height it states, which gives a replaced
/// item with an aspect ratio the width that follows from that height.
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
    let (across, along) = if inputs.content_box {
        (inputs.edges.width, inputs.edges.height)
    } else {
        (0.0, 0.0)
    };
    let measured = tree
        .compute_child_layout(
            child,
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::ContentSize,
                axis: RequestedAxis::Horizontal,
                known_dimensions: Size {
                    width: None,
                    height: inputs.height.map(|height| height + along),
                },
                known_dimensions_are_definite: Size {
                    width: false,
                    height: inputs.height.is_some(),
                },
                parent_size: Size::NONE,
                available_space: Size {
                    width: AvailableSpace::MinContent,
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            },
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
/// whose flex base size is a length bounded by it.
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
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    tree.sync_layout_cache_mutations();
    let is_row = tree
        .physical_style(usize::from(node_id))
        .flex_direction_is_row();
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
            let margins: f32 = [margin.left, margin.right]
                .into_iter()
                .map(|margin| {
                    crate::layout::resolve_length_percentage_auto(margin, None).unwrap_or(0.0)
                })
                .sum();
            let (cannot_shrink, cannot_grow) =
                (style.flex_shrink() == 0.0, style.flex_grow() == 0.0);
            let preferred = crate::layout::resolve_dimension(width, None);
            let automatic_minimum = min.is_auto().then(|| AutomaticMinimumInputs {
                scrolls: is_scroll_container(style.overflow()),
                specified: preferred,
                maximum: max_length,
                height: crate::layout::resolve_dimension(style.size().height, None),
                content_box: style.box_sizing() == BoxSizing::ContentBox,
                edges: {
                    let padding = style.padding().resolve_or_zero(None);
                    let border = style.border().resolve_or_zero(None);
                    Size {
                        width: padding.left + padding.right + border.left + border.right,
                        height: padding.top + padding.bottom + border.top + border.bottom,
                    }
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
