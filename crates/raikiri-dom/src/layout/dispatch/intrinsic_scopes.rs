use super::*;
use crate::layout::cache_context::SavedLayoutCache;
use raikiri_layout_style::values::{
    AlignItems, AlignItemsKeyword, BoxSizing, Dimension, ExpandedDimension,
    ExpandedLengthPercentage, LengthPercentageAuto, ResolveOrZero,
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
/// The width of a vertical wrapping column container is its main size
/// instead, which its lines fill up to its `max-width`: the probe keeps it,
/// as it measures the width those lines take.
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
    let entries = if tree.physical_style(idx).max_size().width.is_auto()
        || super::content_limits::is_vertical_wrapping_column_flex(tree, idx)
    {
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

/// Run `layout`, which measures the min-content width of the flex container
/// `node_id`, with the width a replaced item states as a percentage taken as
/// zero.
///
/// A replaced item with a percentage `width` or `max-width` is compressible: it
/// contributes nothing to the min-content width of its container, whatever
/// length a `calc()` adds to the percentage, and the percentage then resolves
/// against the width the container ends up with (CSS Sizing 3 section 5.2.2).
/// Taffy resolves the percentage against the width of the container, which the
/// measure has not got, so it reads such a width as `auto` and the item
/// contributes its natural size. A zero width in its place is the contribution
/// of the item, in a row container as its main size and in a column container
/// as its cross size.
///
/// Only the measure sees the zero. The layout at the width the measure found
/// reads the percentage, and the content-based minimum size of the item there
/// is the natural one capped by the width it states.
///
/// spec: <https://www.w3.org/TR/css-sizing-3/#min-percentage-contribution>
pub(super) fn with_compressible_item_widths<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    tree.sync_layout_cache_mutations();
    let has_percentage = |size: Dimension| match size.expand() {
        ExpandedDimension::Percent(_) => true,
        ExpandedDimension::Calc(calc) => calc.has_percentage(),
        _ => false,
    };
    let row = tree
        .physical_style(usize::from(node_id))
        .flex_direction_is_row();
    let items = tree
        .child_ids(node_id)
        .filter_map(|child| {
            let idx = usize::from(child);
            if tree.nodes[idx].layout_position() == Position::Absolute {
                return None;
            }
            let style = tree.physical_style(idx);
            // An inline SVG root with a ratio is not flagged as replaced for the
            // layout of its box, but its percentage widths compress all the same.
            if style.box_generation_mode() == BoxGenerationMode::None
                || !(style.is_compressible_replaced() || tree.nodes[idx].is_inline_svg_root())
            {
                return None;
            }
            let width = has_percentage(style.size().width);
            let max_width = has_percentage(style.max_size().width.into());
            // A row container that wraps its items measures them by their flex
            // base sizes, which an `auto` one takes from the width of the item,
            // or from its ratio and its height, so it stands for nothing here
            // too. In a column container the base size is a height.
            let basis = row && (width || max_width) && style.flex_basis().is_auto();
            let zeroed = [
                (P::Width, width),
                (P::MaxWidth, max_width),
                (P::FlexBasis, basis),
            ]
            .into_iter()
            .filter(|&(_, zero)| zero)
            .map(|(property, _)| {
                O::length(property, L::Px(0.0), true).expect("physical compressible width")
            })
            .collect::<Vec<_>>();
            (!zeroed.is_empty()).then_some((idx, zeroed))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return layout(tree);
    }
    with_saved_probe_caches(tree, items.len(), |tree, saved| {
        for (idx, entries) in items {
            let key = tree.nodes.box_key(idx).expect("compressible item box key");
            let cache = tree.take_layout_cache(idx);
            saved.push((idx, cache));
            tree.push_local_overrides(entries.into_iter().map(|entry| (key, entry)).collect());
        }
        layout(tree)
    })
}

/// Resolve item height percentages and column flex bases against the height the container's ratio
/// supplies, before its content can raise that height. Keep those values for
/// the content probe and final layout so growth does not resolve them twice.
/// An item of a column flex container first contributes its content height,
/// so its descendants keep their ordinary percentage resolution there.
pub(super) fn with_ratio_percentage_heights<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    height: Option<f32>,
    run_mode: RunMode,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let idx = usize::from(node_id);
    let Some(height) = height.filter(|_| {
        tree.nodes[idx].layout_display() == Display::Flex
            && !tree.has_vertical_writing_mode(idx)
            && !tree.in_column_flex_container(idx)
    }) else {
        return layout(tree);
    };
    let column = !tree.physical_style(idx).flex_direction_is_row();
    let items = tree
        .child_ids(node_id)
        .filter_map(|child| {
            let idx = usize::from(child);
            if tree.nodes[idx].layout_position() == Position::Absolute {
                return None;
            }
            let style = tree.physical_style(idx);
            let entries = [
                (P::Height, style.size().height),
                (P::MinHeight, style.min_size().height.into()),
                (P::MaxHeight, style.max_size().height.into()),
            ]
            .into_iter()
            .chain(column.then(|| (P::FlexBasis, style.flex_basis())))
            .filter_map(|(property, size): (P, Dimension)| {
                let percentage = match size.expand() {
                    ExpandedDimension::Percent(_) => true,
                    ExpandedDimension::Calc(calc) => calc.has_percentage(),
                    _ => false,
                };
                percentage.then(|| {
                    O::length(
                        property,
                        L::Px(size.maybe_resolve(Some(height)).expect("percentage height")),
                        true,
                    )
                    .expect("physical item height")
                })
            })
            .collect::<Vec<_>>();
            (!entries.is_empty()).then_some((idx, entries))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return layout(tree);
    }
    with_saved_probe_caches(tree, items.len(), |tree, saved| {
        for (idx, entries) in items {
            let key = tree.nodes.box_key(idx).expect("ratio item box");
            let mut cache = tree.take_layout_cache(idx);
            if run_mode == RunMode::PerformLayout {
                // A different final layout replaces descendant geometry, so
                // the old final entry can no longer stand for that geometry.
                cache.forget_final_layout();
            }
            saved.push((idx, cache));
            tree.push_local_overrides(entries.into_iter().map(|entry| (key, entry)).collect());
        }
        layout(tree)
    })
}

/// The most width that the content box of the vertical container `container`
/// gives a horizontal box in it, when its `max-width` is a length: that length
/// raised to its `min-width`, less the padding and border of the container
/// where the limits are measured to the border box; the counterpart of
/// `max_height_cap` along the other axis.
fn max_width_cap(tree: &LayoutSession<'_>, container: usize) -> Option<f32> {
    let style = tree.physical_style(container);
    let max = style.max_size().width.maybe_resolve(None::<f32>)?;
    let min = style
        .min_size()
        .width
        .maybe_resolve(None::<f32>)
        .unwrap_or(0.0);
    let edges = if style.box_sizing() == BoxSizing::ContentBox {
        0.0
    } else {
        let padding = style.padding();
        let border = style.border().resolve_or_zero(None::<f32>);
        padding.left.maybe_resolve(None::<f32>)?
            + padding.right.maybe_resolve(None::<f32>)?
            + border.left
            + border.right
    };
    Some((max.max(min) - edges).max(0.0))
}

/// Resolve the percentage inline sizes, minimums and maximums of the items of
/// the flex container `node_id` that run in the other writing direction, and
/// in a column container their percentage `flex-basis`, against the initial
/// containing block, while the container's size along that axis is open.
///
/// That axis is the container's block axis: the main axis of a column
/// container, the cross axis of a row one. Browsers resolve a percentage of
/// the inline size of such an item, and of its limits, against the initial
/// containing block along that axis, the size an orthogonal flow is offered
/// when its container has none (CSS Writing Modes 4 section 7.3.1), rather
/// than taking the content size: the page area's height, or its width where
/// the container itself runs in a vertical writing mode. An item in the
/// writing direction of its container keeps the percentage, which is the
/// content size while the container leaves its main size open (CSS Flexbox 1
/// section 9.2.3).
///
/// `main_size_is_definite` tells whether the container's size along its block
/// axis is definite; the percentages refer to it then, and nothing is
/// overridden.
pub(super) fn with_orthogonal_percentage_main_sizes<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    main_size_is_definite: bool,
    run_mode: RunMode,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    use super::vertical_oof::used_writing_mode;
    let idx = usize::from(node_id);
    let container_vertical = used_writing_mode(tree, idx).is_vertical();
    // The room is that of the initial containing block within the limit the
    // container sets on its size along that axis.
    let room = if container_vertical {
        let page_width = tree
            .fixed_page_area
            .map(|(width, _)| width)
            .filter(|&width| width > 0.0);
        [page_width, max_width_cap(tree, idx)]
            .into_iter()
            .flatten()
            .reduce(f32::min)
            .filter(|_| page_width.is_some())
    } else {
        crate::layout::ifc::boxes::page_area_height(tree).and_then(|_| {
            crate::layout::ifc::boxes::open_height_room(
                tree,
                crate::layout::ifc::boxes::max_height_cap(tree, idx),
            )
        })
    };
    let column = !tree.physical_style(idx).flex_direction_is_row();
    let Some(room) = room.filter(|_| !main_size_is_definite) else {
        return layout(tree);
    };
    let items = tree
        .child_ids(node_id)
        .filter_map(|child| {
            let idx = usize::from(child);
            if tree.nodes[idx].layout_position() == Position::Absolute
                || used_writing_mode(tree, idx).is_vertical() == container_vertical
            {
                return None;
            }
            let style = tree.physical_style(idx);
            let main = if container_vertical {
                [
                    (P::Width, style.size().width),
                    (P::MinWidth, style.min_size().width.into()),
                    (P::MaxWidth, style.max_size().width.into()),
                ]
            } else {
                [
                    (P::Height, style.size().height),
                    (P::MinHeight, style.min_size().height.into()),
                    (P::MaxHeight, style.max_size().height.into()),
                ]
            };
            let entries = column
                .then(|| (P::FlexBasis, style.flex_basis()))
                .into_iter()
                .chain(main)
                .filter(|(_, size): &(P, Dimension)| size.uses_percentage())
                .map(|(property, size)| {
                    O::length(
                        property,
                        L::Px(size.maybe_resolve(Some(room)).expect("percentage size")),
                        true,
                    )
                    .expect("physical item main size")
                })
                .collect::<Vec<_>>();
            (!entries.is_empty()).then_some((idx, entries))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return layout(tree);
    }
    with_saved_probe_caches(tree, items.len(), |tree, saved| {
        for (idx, entries) in items {
            let key = tree.nodes.box_key(idx).expect("orthogonal item box");
            let mut cache = tree.take_layout_cache(idx);
            if run_mode == RunMode::PerformLayout {
                // A different final layout replaces descendant geometry, so
                // the old final entry can no longer stand for that geometry.
                cache.forget_final_layout();
            }
            saved.push((idx, cache));
            tree.push_local_overrides(entries.into_iter().map(|entry| (key, entry)).collect());
        }
        layout(tree)
    })
}

/// Lay out the column flex container `node_id` with the main size it gets
/// from its items' hypothetical main sizes, when its own main size is open and
/// some item has a flex base size that is a length.
///
/// The main size of a column container is its block size: the height of a
/// horizontal container and the width of one laid out `vertical`. A browser finds that
/// size as the sum of the hypothetical main sizes of the items on the longest
/// line: the flex base size, kept within the minimum and maximum main size of
/// the item (the automatic minimum where it states none). Taffy takes the
/// max-content contribution of the item instead, which is the content size of
/// an item whose automatic minimum is below it (a scroll container, an item
/// with a zero minimum, a box of the other writing direction), and which a
/// maximum below the flex base size does not bound. A flex factor then lets
/// the container grow past the sum as well.
///
/// Each such item is therefore made rigid while the container finds its main
/// size: no flex factor, and a flex base size bounded by its maximum, which
/// gives Taffy the hypothetical size as the contribution. Items whose flex
/// base size is their content are left alone; their contribution is the
/// hypothetical size already.
///
/// With no minimum or maximum main size on the container the sum is the main
/// size, and nothing is left to flex, so the rigid items are laid out as they
/// are. Otherwise, and where a negative margin could make the sum fall below
/// the zero the size cannot go under, the main size the rigid items found is
/// given to the container, which lays the items out at their own flex factors
/// within it. A wrapping container with a minimum, a maximum or a negative
/// margin among its items is laid out by [`wrapping_column_layout`], which
/// also settles its cross size, since that depends on how its lines break.
/// A wrapping container with none of these finds its main size from its rigid
/// items under a max-content room and is laid out with that size, which is
/// not definite, so its lines break where its items' hypothetical sizes put
/// them; a measure of its cross size goes to the plain layout.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#algo-main-container>
pub(super) fn with_hypothetical_column_main_size(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    main_size_is_definite: bool,
    vertical: bool,
    layout: impl Fn(&mut LayoutSession<'_>, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let container = tree.physical_style(idx);
    let (main_axis, known_main, containing_main) = if vertical {
        (
            RequestedAxis::Horizontal,
            inputs.known_dimensions.width,
            inputs.parent_size.width,
        )
    } else {
        (
            RequestedAxis::Vertical,
            inputs.known_dimensions.height,
            inputs.parent_size.height,
        )
    };
    let style_main = |size: Size<Dimension>| if vertical { size.width } else { size.height };
    let wraps = container.flex_wraps();
    // The cross size of a wrapping container depends on how its lines break,
    // which its main size limits decide.
    let measures_cross = inputs.run_mode == RunMode::ComputeSize
        && inputs.axis != main_axis
        && inputs.axis != RequestedAxis::Both;
    if main_size_is_definite
        || container.flex_direction_is_row()
        || known_main.is_some()
        || (measures_cross && !wraps)
    {
        if wraps {
            return with_line_minimum_in_content_box(tree, idx, inputs, vertical, &layout);
        }
        return layout(tree, inputs);
    }
    // The main size of a wrapping column depends on its content here, so a
    // percentage of it in the gap between the items resolves against zero in
    // every pass.
    if wraps && let Some(gap) = cyclic_main_gap(tree, idx) {
        return with_saved_probe_caches(tree, 0, |tree, _| {
            tree.push_local_overrides(vec![gap]);
            with_hypothetical_column_main_size(
                tree,
                node_id,
                inputs,
                main_size_is_definite,
                vertical,
                layout,
            )
        });
    }
    let items = tree
        .child_ids(node_id)
        .map(usize::from)
        .filter(|&item| {
            tree.nodes[item].layout_position() != Position::Absolute
                && tree.physical_style(item).box_generation_mode() != BoxGenerationMode::None
        })
        .collect::<Vec<_>>();
    // A negative margin can make the sum of the outer sizes negative. The
    // container stops at a main size of zero then, and the free space left is
    // what a growing item fills. A margin that is a percentage or a `calc()`
    // is checked for its smallest and its largest value.
    let negative_margin = items.iter().any(|&item| {
        let margin = tree.physical_style(item).margin();
        let sides = if vertical {
            [margin.left, margin.right]
        } else {
            [margin.top, margin.bottom]
        };
        sides.into_iter().any(|side| {
            [0.0, 1.0e9]
                .into_iter()
                .any(|width| side.resolve(Some(width)).is_some_and(|value| value < 0.0))
        })
    });
    // A percentage minimum or maximum main size of the container refers to
    // the size of its containing block, and has no effect where that is open.
    let limited = style_main(container.max_size().map(Into::into))
        .maybe_resolve(containing_main)
        .is_some()
        || style_main(container.min_size().map(Into::into))
            .maybe_resolve(containing_main)
            .is_some_and(|min| min > 0.0);
    let bounded = negative_margin || limited;
    let rigid = items
        .iter()
        .copied()
        .filter_map(|item| {
            let style = tree.physical_style(item);
            let basis = crate::layout::resolve_dimension(style.flex_basis(), None)
                .filter(|basis| basis.is_finite() && *basis >= 0.0)?;
            let bounded_basis = style_main(style.max_size().map(Into::into))
                .maybe_resolve(None::<f32>)
                .map_or(basis, |max| basis.min(max));
            let key = tree.nodes.box_key(item).expect("flex item box key");
            let entries = vec![
                (key, O::flex_grow(0.0)),
                (key, O::flex_shrink(0.0)),
                (
                    key,
                    O::length(P::FlexBasis, L::Px(bounded_basis), true)
                        .expect("physical flex basis"),
                ),
            ];
            Some((item, entries))
        })
        .collect::<Vec<_>>();
    if rigid.is_empty() && !(wraps && bounded) {
        return layout(tree, inputs);
    }
    let run = |tree: &mut LayoutSession<'_>, rigid: Vec<_>, inputs: LayoutInput| {
        let count = rigid.len();
        with_saved_probe_caches(tree, count, |tree, saved| {
            for (item, entries) in rigid {
                let mut cache = tree.take_layout_cache(item);
                if inputs.run_mode == RunMode::PerformLayout {
                    // A different final layout replaces descendant geometry,
                    // so the old final entry can no longer stand for it.
                    cache.forget_final_layout();
                }
                saved.push((item, cache));
                tree.push_local_overrides(entries);
            }
            layout(tree, inputs)
        })
    };
    if wraps && !limited {
        // With no limit, the items stay on one line, which is as long as the
        // sum of their outer hypothetical main sizes. Those take the automatic
        // minimum and negative margins, which a definite room skips, so the
        // line is measured without one.
        if measures_cross {
            return layout(tree, inputs);
        }
        let mut probe = LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis: main_axis,
            ..inputs
        };
        *main_mut(&mut probe.available_space, vertical) = AvailableSpace::MaxContent;
        let measured = run(tree, rigid, probe);
        if inputs.run_mode == RunMode::ComputeSize && inputs.axis == main_axis {
            return measured;
        }
        let main = if vertical {
            measured.size.width
        } else {
            measured.size.height
        };
        let mut inputs = inputs;
        *main_mut(&mut inputs.known_dimensions, vertical) = Some(main);
        *main_mut(&mut inputs.known_dimensions_are_definite, vertical) = false;
        return layout(tree, inputs);
    }
    if !bounded {
        if measures_cross {
            return layout(tree, inputs);
        }
        return run(tree, rigid, inputs);
    }
    if wraps {
        // Percentage edges refer to the inline size of the containing block,
        // which is the height of a vertical parent.
        let basis = if vertical {
            super::vertical_block::containing_inline_size(tree, idx, &inputs)
        } else {
            inputs.parent_size.width
        };
        let lines_keep_without_cross = !negative_margin
            || lines_keep_without_cross(tree, idx, &inputs, vertical, &items, &rigid);
        return wrapping_column_layout(
            tree,
            idx,
            inputs,
            vertical,
            negative_margin,
            lines_keep_without_cross,
            basis,
            containing_main,
            |tree, inputs| run(tree, rigid.clone(), inputs),
            &layout,
        );
    }
    if inputs.run_mode == RunMode::ComputeSize {
        return run(tree, rigid, inputs);
    }
    let size = run(
        tree,
        rigid,
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis: main_axis,
            ..inputs
        },
    )
    .size;
    let main = if vertical { size.width } else { size.height };
    let mut inputs = inputs;
    *main_mut(&mut inputs.known_dimensions, vertical) = Some(main);
    *main_mut(&mut inputs.known_dimensions_are_definite, vertical) = false;
    layout(tree, inputs)
}

/// The gap between the items of the column flex container `idx` along its
/// main axis, resolved against a main size of zero, when it is a percentage
/// of that size.
fn cyclic_main_gap(
    tree: &LayoutSession<'_>,
    idx: usize,
) -> Option<(crate::node::arena::BoxKey, O)> {
    let gap = tree.physical_style(idx).gap().height;
    let cyclic = match gap.expand() {
        ExpandedLengthPercentage::Percent(_) => true,
        ExpandedLengthPercentage::Calc(value) => value.has_percentage(),
        _ => false,
    };
    cyclic.then(|| {
        (
            tree.nodes.box_key(idx).expect("flex container box key"),
            O::length(P::RowGap, L::Px(gap.resolve_or_zero(Some(0.0))), true)
                .expect("resolved row gap"),
        )
    })
}

/// The width of `size` where `horizontal` is set, and its height otherwise.
fn main_of<T>(size: Size<T>, horizontal: bool) -> T {
    if horizontal { size.width } else { size.height }
}

/// The width of `size` where `horizontal` is set, and its height otherwise.
fn main_mut<T>(size: &mut Size<T>, horizontal: bool) -> &mut T {
    if horizontal {
        &mut size.width
    } else {
        &mut size.height
    }
}

/// Lay out the wrapping flex container `idx`, whose main size is known, with
/// a minimum main size no larger than its content box.
///
/// The lines of a container break at the size of its content box, and at
/// least at its minimum less its padding and border (CSS Flexbox 1 §9.3).
/// Taffy raises the room its lines break in to the border box minimum, so a
/// minimum that comes within the padding and border of the main size lets a
/// line hold more than the content box. The container then gets a minimum of
/// its content box size, which leaves its main size as it is.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#algo-line-break>
fn with_line_minimum_in_content_box(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    vertical: bool,
    layout: &impl Fn(&mut LayoutSession<'_>, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let style = tree.physical_style(idx);
    // The main axis is horizontal for a row of a horizontal container and for
    // a column of a vertical one.
    let horizontal = style.flex_direction_is_row() != vertical;
    let pick = |size: Size<Option<f32>>| if horizontal { size.width } else { size.height };
    let pick_style = |size: Size<Dimension>| if horizontal { size.width } else { size.height };
    let definite = if horizontal {
        inputs.known_dimensions_are_definite.width
    } else {
        inputs.known_dimensions_are_definite.height
    };
    let containing = pick(inputs.parent_size);
    let basis = if vertical {
        super::vertical_block::containing_inline_size(tree, idx, &inputs)
    } else {
        inputs.parent_size.width
    };
    let edges =
        (style.padding().resolve_or_zero(basis) + style.border().resolve_or_zero(basis)).sum_axes();
    let insets = if horizontal {
        edges.width
    } else {
        edges.height
    };
    let content_box = style.box_sizing() == BoxSizing::ContentBox;
    let outer =
        |value: Option<f32>| value.map(|value| if content_box { value + insets } else { value });
    let min = outer(pick_style(style.min_size().map(Into::into)).maybe_resolve(containing));
    let max = outer(pick_style(style.max_size().map(Into::into)).maybe_resolve(containing));
    let size = pick(inputs.known_dimensions)
        .or_else(|| outer(pick_style(style.size()).maybe_resolve(containing)))
        .map(|size| {
            let size = max.map_or(size, |max| size.min(max));
            min.map_or(size, |min| size.max(min))
        });
    let (Some(size), Some(min)) = (size, min) else {
        return layout(tree, inputs);
    };
    let content = (size - insets).max(0.0);
    if !definite || min <= content {
        return layout(tree, inputs);
    }
    let mut inputs = inputs;
    *main_mut(&mut inputs.known_dimensions, horizontal) = Some(size);
    let value = if content_box {
        (content - insets).max(0.0)
    } else {
        content
    };
    let property = if horizontal {
        P::MinWidth
    } else {
        P::MinHeight
    };
    let key = tree.nodes.box_key(idx).expect("flex container box key");
    let entries = vec![(
        key,
        O::length(property, L::Px(value), true).expect("container main minimum"),
    )];
    with_saved_probe_caches(tree, 0, |tree, _| {
        tree.push_local_overrides(entries);
        layout(tree, inputs)
    })
}

/// Whether the items of the wrapping column flex container `idx` keep their
/// hypothetical main sizes when the container's own cross size, minimum and
/// maximum are taken off, so that probes without them break the same lines.
///
/// That holds where nothing constrains the cross size of the container, and
/// otherwise for an item whose main size does not follow its cross size: one
/// whose inline axis is the main axis, and that is neither replaced nor given
/// an aspect ratio, or one of a length flex base size and a length minimum,
/// in either case with no percentage margin or padding along the main axis.
fn lines_keep_without_cross(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
    vertical: bool,
    items: &[usize],
    rigid: &[(usize, Vec<(crate::node::arena::BoxKey, O)>)],
) -> bool {
    let style = tree.physical_style(idx);
    let cross = |size: Size<Dimension>| if vertical { size.height } else { size.width };
    let containing_cross = if vertical {
        inputs.parent_size.height
    } else {
        inputs.parent_size.width
    };
    let known_cross = if vertical {
        inputs.known_dimensions.height
    } else {
        inputs.known_dimensions.width
    };
    let constrained = known_cross.is_some()
        || cross(style.size().map(Into::into))
            .maybe_resolve(containing_cross)
            .is_some()
        || cross(style.max_size().map(Into::into))
            .maybe_resolve(containing_cross)
            .is_some()
        || cross(style.min_size().map(Into::into))
            .maybe_resolve(containing_cross)
            .is_some_and(|min| min > 0.0);
    !constrained
        || items.iter().all(|&item| {
            let item_style = tree.physical_style(item);
            let inline_is_main = tree.has_vertical_writing_mode(item) != vertical;
            let fixed_main = rigid.iter().any(|(rigid, _)| *rigid == item) && {
                let min: Size<Dimension> = item_style.min_size().map(Into::into);
                if vertical { min.width } else { min.height }
            }
            .maybe_resolve(None::<f32>)
            .is_some();
            // A percentage edge refers to the inline size of the container,
            // which is its cross size.
            let (margin, padding) = (item_style.margin(), item_style.padding());
            let (margins, paddings) = if vertical {
                ([margin.left, margin.right], [padding.left, padding.right])
            } else {
                ([margin.top, margin.bottom], [padding.top, padding.bottom])
            };
            let fixed_edges = margins
                .into_iter()
                .all(|side| side.resolve(Some(0.0)) == side.resolve(Some(1.0e9)))
                && paddings.into_iter().all(|side| {
                    side.resolve_or_zero(Some(0.0)) == side.resolve_or_zero(Some(1.0e9))
                });
            fixed_edges
                && (fixed_main
                    || (inline_is_main
                        && item_style.aspect_ratio().is_none()
                        && !crate::paint_rules::is_replaced_element(tree.document, item)))
        })
}

/// Lay out the wrapping column flex container `idx`, whose main size is open
/// and bounded by a minimum or a maximum, at the main size of its longest
/// line of hypothetical main sizes.
///
/// The lines of the container break where the running sum of the outer sizes
/// of its items passes the size its content box may take: the room it is
/// given, at most its maximum and at least its minimum, all less its padding
/// and border (CSS Flexbox 1 §9.3). Taffy compares that sum with the border
/// box minimum and maximum, so `probe` finds the size with the container's
/// main-axis padding and border taken out and its limits on its content box.
/// `probe` runs the layout with the items rigid at their hypothetical main
/// sizes, under a max-content room: with a definite room, Taffy sums the
/// flex bases alone and skips the automatic minimum and negative margins.
///
/// Where every item fits on one line, the container keeps that line at the
/// measured size, which is known but not definite. Otherwise its lines fill
/// the room it is given, up to its maximum, and it is laid out with that size
/// as definite, as its room and as its maximum, and with a minimum that
/// leaves the room of its content box as the limit its lines break at. Every
/// line the rigid items made fits in that room and the item after each line
/// did not, so the lines break the same way.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#algo-line-break>
#[allow(clippy::too_many_arguments)]
fn wrapping_column_layout(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    vertical: bool,
    negative_margin: bool,
    lines_keep_without_cross: bool,
    basis: Option<f32>,
    containing_main: Option<f32>,
    probe: impl Fn(&mut LayoutSession<'_>, LayoutInput) -> LayoutOutput,
    layout: &impl Fn(&mut LayoutSession<'_>, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let style = tree.physical_style(idx);
    let padding = style.padding().resolve_or_zero(basis);
    let border = style.border().resolve_or_zero(basis);
    let insets = if vertical {
        (padding + border).sum_axes().width
    } else {
        (padding + border).sum_axes().height
    };
    let content_box = style.box_sizing() == BoxSizing::ContentBox;
    let style_main = |size: Size<Dimension>| if vertical { size.width } else { size.height };
    // The content box size that a resolved minimum or maximum allows.
    let to_content = |value: f32| {
        if content_box {
            value
        } else {
            (value - insets).max(0.0)
        }
    };
    let min = style_main(style.min_size().map(Into::into))
        .maybe_resolve(containing_main)
        .map(to_content);
    let max = style_main(style.max_size().map(Into::into))
        .maybe_resolve(containing_main)
        .map(to_content);
    // A length the container's own box sizing reads as the border box size
    // `outer`.
    let in_own_box = |outer: f32| {
        if content_box {
            (outer - insets).max(0.0)
        } else {
            outer
        }
    };
    let (min_property, max_property) = if vertical {
        (P::MinWidth, P::MaxWidth)
    } else {
        (P::MinHeight, P::MaxHeight)
    };
    let key = tree.nodes.box_key(idx).expect("flex container box key");
    let main_axis = if vertical {
        RequestedAxis::Horizontal
    } else {
        RequestedAxis::Vertical
    };
    let edge_properties = if vertical {
        [
            P::PaddingLeft,
            P::PaddingRight,
            P::BorderLeft,
            P::BorderRight,
        ]
    } else {
        [
            P::PaddingTop,
            P::PaddingBottom,
            P::BorderTop,
            P::BorderBottom,
        ]
    };
    let mut entries = edge_properties
        .into_iter()
        .map(|property| {
            (
                key,
                O::length(property, L::Px(0.0), true).expect("zero edge"),
            )
        })
        .collect::<Vec<_>>();
    // An `auto` minimum stays, so a minimum Taffy derives from an aspect
    // ratio is kept.
    if let Some(min) = min {
        entries.push((
            key,
            O::length(min_property, L::Px(min), true).expect("content minimum"),
        ));
    }
    if let Some(max) = max {
        entries.push((
            key,
            O::length(max_property, L::Px(max), true).expect("content maximum"),
        ));
    }
    // A negative margin can break a line whose total fits, which the probes
    // below find from the cross size of the container with its own cross
    // constraint taken off. Where that would change how long an item is, the
    // lines are left to the plain layout.
    if negative_margin && max.is_some() && !lines_keep_without_cross {
        return layout(tree, inputs);
    }
    let mut probe_inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: main_axis,
        ..inputs
    };
    {
        let room = if vertical {
            &mut probe_inputs.available_space.width
        } else {
            &mut probe_inputs.available_space.height
        };
        // A definite room would size the lines from the flex bases alone,
        // without the automatic minimum or a negative margin.
        *room = AvailableSpace::MaxContent;
    }
    let unbroken_entries = entries
        .iter()
        .map(|(key, entry)| {
            if entry.overrides_length(max_property) {
                // No maximum, so no line breaks.
                (
                    *key,
                    O::length(max_property, L::Auto, false).expect("no content maximum"),
                )
            } else {
                (*key, entry.clone())
            }
        })
        .collect::<Vec<_>>();
    let run_probe = |tree: &mut LayoutSession<'_>, entries: Vec<_>, inputs: LayoutInput| {
        with_saved_probe_caches(tree, 0, |tree, _| {
            tree.push_local_overrides(entries);
            probe(tree, inputs)
        })
    };
    // Where every item fits on one line, the probe has the size of the whole
    // line, and the container keeps that line at the measured size.
    let (mut measured, one_line) = match max {
        None => (run_probe(tree, entries, probe_inputs), true),
        Some(_) if negative_margin => {
            // A negative margin can make a running sum of the outer sizes
            // pass the limit inside a line whose total fits, so the line
            // breaks there although the whole line is no longer than the
            // limit. More lines make a container of content-sized cross
            // size larger across, by at least the gap between them, which
            // these probes set to a pixel so that lines of items with no
            // cross size still count. That gap does not move a line break.
            let (size, min_size, max_size) = if vertical {
                (P::Height, P::MinHeight, P::MaxHeight)
            } else {
                (P::Width, P::MinWidth, P::MaxWidth)
            };
            let free_cross = [size, min_size, max_size]
                .map(|property| O::length(property, L::Auto, false).expect("content cross size"))
                .into_iter()
                .chain([O::length(P::ColumnGap, L::Px(1.0), true).expect("line gap")])
                .map(|entry| (key, entry))
                .collect::<Vec<_>>();
            let mut free_inputs = LayoutInput {
                axis: RequestedAxis::Both,
                ..probe_inputs
            };
            *main_mut(&mut free_inputs.known_dimensions, !vertical) = None;
            let across = |tree: &mut LayoutSession<'_>, entries: &[_]| {
                let entries = entries.iter().chain(&free_cross).cloned().collect();
                main_of(run_probe(tree, entries, free_inputs).size, !vertical)
            };
            let one_line = across(tree, &entries) <= across(tree, &unbroken_entries) + 0.01;
            (run_probe(tree, entries, probe_inputs), one_line)
        }
        Some(max) => {
            // Without a negative margin, the line sums only grow, so the
            // items fit on one line where the whole line fits the limit.
            let unbroken = run_probe(tree, unbroken_entries, probe_inputs);
            let limit = max.max(min.unwrap_or(0.0));
            if main_of(unbroken.size, vertical) <= limit + 0.01 {
                (unbroken, true)
            } else {
                (run_probe(tree, entries, probe_inputs), false)
            }
        }
    };
    let line_content = main_of(measured.size, vertical);
    // Lines that break fill the room the container is given, up to its
    // maximum.
    let given_room = main_of(inputs.available_space, vertical);
    let content = match (one_line, given_room, max) {
        (false, AvailableSpace::Definite(space), Some(max)) => {
            line_content.max((space - insets).max(0.0).min(max))
        }
        _ => line_content,
    };
    let main = content + insets;
    if negative_margin && !one_line {
        // Lines break where the running sum of the outer sizes passes the
        // limit. A negative margin can make that sum fall inside a line, so
        // the longest line can be shorter than a part of it, and a limit of
        // that length would break the line again. Such a container keeps the
        // lines and the size of the plain layout.
        return layout(tree, inputs);
    }
    if inputs.run_mode == RunMode::ComputeSize && inputs.axis == main_axis {
        // A run that measures the main axis alone returns its size only.
        *main_mut(&mut measured.size, vertical) = main;
        return measured;
    }
    let mut inputs = inputs;
    if one_line {
        // A main size known but not definite keeps the items on one line and
        // leaves their percentages unresolved, and the items flex into it.
        *main_mut(&mut inputs.known_dimensions, vertical) = Some(main);
        *main_mut(&mut inputs.known_dimensions_are_definite, vertical) = false;
        return layout(tree, inputs);
    }
    let (known, known_is_definite, room) = if vertical {
        (
            &mut inputs.known_dimensions.width,
            &mut inputs.known_dimensions_are_definite.width,
            &mut inputs.available_space.width,
        )
    } else {
        (
            &mut inputs.known_dimensions.height,
            &mut inputs.known_dimensions_are_definite.height,
            &mut inputs.available_space.height,
        )
    };
    *room = AvailableSpace::Definite(main);
    // Taffy sizes several lines in a definite room as the content box of
    // that room, and a container whose own minimum raised it above its
    // longest line as that line again, so the size is given as definite
    // there.
    let larger = matches!(given_room, AvailableSpace::Definite(_))
        || min.is_some_and(|min| content <= min && min > 0.0);
    if larger {
        *known = Some(main);
        *known_is_definite = true;
    }
    let mut entries = vec![(
        key,
        O::length(max_property, L::Px(in_own_box(main)), true).expect("container main limit"),
    )];
    if min.is_some() {
        entries.push((
            key,
            O::length(min_property, L::Px(in_own_box(main - insets)), true)
                .expect("container main minimum"),
        ));
    }
    // The size is definite for line breaking only: the percentages of the
    // items stay unresolved as they were in the probe.
    let items = if larger {
        unresolved_item_percentages(tree, idx, vertical)
    } else {
        Vec::new()
    };
    // Nor is the size definite for the children of the items: the flexed
    // main size of an item is definite there only where its flex basis is.
    tree.context.content_sized_flex_mains.push(idx);
    let output = with_saved_probe_caches(tree, 0, |tree, _| {
        tree.push_local_overrides(entries);
        super::column_main_limits::scoped(tree, idx, items, inputs.run_mode, |tree| {
            layout(tree, inputs)
        })
    });
    tree.context.content_sized_flex_mains.pop();
    output
}

/// Read every percentage main size of the in-flow items of the container
/// `idx` as `auto`, which is what a percentage of an indefinite size is.
fn unresolved_item_percentages(
    tree: &LayoutSession<'_>,
    idx: usize,
    vertical: bool,
) -> Vec<(usize, O)> {
    item_percentages_left_unresolved(tree, idx, vertical, true)
}

/// Read every percentage height of the in-flow items of the row container
/// `idx` whose writing mode is horizontal as their content height, and every percentage minimum and maximum
/// height as `auto`: the cross size of the container they would refer to is
/// not definite.
pub(super) fn unresolved_item_cross_heights(
    tree: &LayoutSession<'_>,
    idx: usize,
) -> Vec<(usize, O)> {
    item_percentages_left_unresolved(tree, idx, false, false)
}

/// The overrides that read the percentage sizes of the items of `idx` along
/// the height, or the width where `vertical`, as unresolved: a minimum or a
/// maximum as `auto`, and a size as `auto` along the main axis, where
/// `with_basis` also reads a percentage flex basis as `content`, or as its
/// content size along the cross axis.
fn item_percentages_left_unresolved(
    tree: &LayoutSession<'_>,
    idx: usize,
    vertical: bool,
    with_basis: bool,
) -> Vec<(usize, O)> {
    let properties = if vertical {
        [P::FlexBasis, P::Width, P::MinWidth, P::MaxWidth]
    } else {
        [P::FlexBasis, P::Height, P::MinHeight, P::MaxHeight]
    };
    let mut entries = Vec::new();
    for child in tree.child_ids(NodeId::from(idx)) {
        let child = usize::from(child);
        let style = tree.physical_style(child);
        if style.position() == Position::Absolute
            || style.box_generation_mode() == BoxGenerationMode::None
        {
            continue;
        }
        // The height of an orthogonal item is its inline size, and a
        // percentage inline size resolves against the container even when
        // the container's height is not definite.
        if !with_basis && tree.has_vertical_writing_mode(child) {
            continue;
        }
        let basis = style.flex_basis();
        let (size, min, max) = (style.size(), style.min_size(), style.max_size());
        let (size, min, max) = if vertical {
            (size.width, min.width, max.width)
        } else {
            (size.height, min.height, max.height)
        };
        // Only a percentage refers to the container's size: a keyword such
        // as `min-content` keeps its own meaning.
        let unresolved = [
            with_basis && basis.uses_percentage(),
            size.uses_percentage(),
            min.uses_percentage(),
            max.uses_percentage(),
        ];
        for (property, unresolved) in properties.into_iter().zip(unresolved) {
            if unresolved {
                // An unresolved basis is `content`, which no override spells:
                // the max-content block size is the content size of a column
                // item.
                // A cross size that does not resolve is still not `auto`,
                // so the item is not stretched: it takes its content size.
                let value = if property == P::FlexBasis
                    || (!with_basis && matches!(property, P::Height | P::Width))
                {
                    L::MaxContent
                } else {
                    L::Auto
                };
                entries.push((
                    child,
                    O::length(property, value, false).expect("unresolved item percentage"),
                ));
            }
        }
    }
    entries
}

/// Run `probe`, which measures the content height of the flex container
/// `node_id` to raise the height its aspect ratio gives it, while the items the
/// container stretches add only their `min-height` to that measure (see
/// `with_ratio_probe_stretch_floor` in [`crate::layout::dispatch`]).
pub(super) fn with_ratio_height_probe<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    probe: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let outer = tree
        .context
        .ratio_height_probe
        .replace(usize::from(node_id));
    let result = catch_unwind(AssertUnwindSafe(|| probe(tree)));
    tree.context.ratio_height_probe = outer;
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

/// The padding and border of the item `style` across its width, when its sizes
/// are those of its content box, and zero when they are those of its border box.
/// A percentage padding is of `basis`, the width of the containing block, and
/// counts as zero where the probe has not got one.
fn horizontal_content_edges(style: &LayoutStyleView<'_>, basis: Option<f32>) -> f32 {
    if style.box_sizing() == BoxSizing::ContentBox {
        let padding = style.padding().resolve_or_zero(basis);
        let border = style.border().resolve_or_zero(basis);
        padding.left + padding.right + border.left + border.right
    } else {
        0.0
    }
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
        (room - margins_sum(margin.top, margin.bottom, None))
            .min(bound(style.max_size().height).unwrap_or(f32::INFINITY))
            .max(bound(style.min_size().height).unwrap_or(0.0))
            .max(edges)
    })
}

/// The sum of two opposite margins of the item, where an `auto` margin and a
/// percentage of a size the probe has not got count as zero. A percentage margin
/// is of `basis`, the width of the containing block.
fn margins_sum(start: LengthPercentageAuto, end: LengthPercentageAuto, basis: Option<f32>) -> f32 {
    [start, end]
        .into_iter()
        .map(|margin| crate::layout::resolve_length_percentage_auto(margin, basis).unwrap_or(0.0))
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

/// The inputs that ask a flex item for its content width under `request`, the
/// min-content or max-content width the container is asked for, measured with
/// the border-box `height` it has before its width is measured, if it has one.
/// The width a `ContentSize` measure gives is the one Taffy takes as the
/// contribution of the item.
fn content_probe(request: AvailableSpace, height: Option<f32>) -> LayoutInput {
    LayoutInput {
        available_space: Size {
            width: request,
            height: AvailableSpace::MaxContent,
        },
        ..min_content_probe(SizingMode::ContentSize, height)
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
/// width instead of installing a bound. The width the item states caps its
/// automatic minimum size, so a width that is lowered would lower that size
/// too: it is measured first and installed as the `min-width` of the item.
/// Where Taffy measures the item it clamps the contribution with the item's
/// margins counted in, so the margins are counted in the upper bound as well
/// and survive it.
///
/// An item whose `width` is a `min-content` or `max-content` keyword states
/// the width the keyword stands for as far as the contribution goes, and has
/// it installed as a length. A scroll container whose flex base size is a
/// length is given the contribution Taffy would find for a box that does not
/// scroll, as its width, minimum width and maximum width together: Taffy
/// takes the flex base size of a scroll container as its contribution, and
/// `request` is the min-content or max-content width that the container is
/// asked for, which the content of the item is measured at.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#intrinsic-item-contributions>
pub(super) fn with_flex_item_basis_bounds<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    cross_room: Option<f32>,
    request: AvailableSpace,
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
            let height = height_before_width(&style, cross_room, single_line, container_align);
            let edges = horizontal_content_edges(&style, None);
            let stated = crate::layout::resolve_dimension(style.size().width, None);
            let finite_basis = crate::layout::resolve_dimension(style.flex_basis(), None)
                .filter(|basis| basis.is_finite());
            // An item whose `width` is a `min-content` or `max-content` keyword
            // contributes the width the keyword stands for, within its
            // `min-width` and `max-width` (CSS Sizing 3 section 5.2). Taffy does
            // not see the keyword: it measures the item as if its width were
            // `auto`, at the min-content size for the min-content width of the
            // container and at the max-content size for the max-content width,
            // so the keyword is replaced by the width it stands for, as a
            // length.
            let keyword = matches!(
                style.size().width.expand(),
                ExpandedDimension::MinContent | ExpandedDimension::MaxContent
            );
            // Taffy gives a scroll container its flex base size as its
            // contribution, whatever its content and its `flex-grow` and
            // `flex-shrink` are, where a box that does not scroll contributes
            // its content within the bounds of the flex base size. A scroll
            // container with a flex base size of a length is measured here
            // instead, at the width of the container's request.
            let scrolls = finite_basis.is_some() && is_scroll_container(style.overflow());
            let keyword_width = keyword.then(|| {
                let width = tree
                    .compute_child_layout(
                        child,
                        min_content_probe(SizingMode::InherentSize, height),
                    )
                    .size
                    .width;
                (width - edges).max(0.0)
            });
            let scroll_content =
                (scrolls && stated.is_none() && keyword_width.is_none()).then(|| {
                    let width = tree
                        .compute_child_layout(child, content_probe(request, height))
                        .size
                        .width;
                    (width - edges).max(0.0)
                });
            let style = tree.physical_style(child_idx);
            let Some(basis) = finite_basis else {
                if style.flex_basis().is_auto()
                    && let Some(width) = keyword_width
                {
                    let entry =
                        O::length(P::Width, L::Px(width), true).expect("physical keyword width");
                    let key = tree
                        .nodes
                        .box_key(child_idx)
                        .expect("flex child has a box key");
                    let cache = tree.take_layout_cache(child_idx);
                    saved.push((child_idx, cache));
                    tree.push_local_overrides(vec![(key, entry)]);
                }
                continue;
            };
            let (min, max) = (style.min_size().width, style.max_size().width);
            let margin = style.margin();
            // A percentage of the container's width, which the probe has not got,
            // bounds nothing, and is no margin either.
            let min_length = crate::layout::resolve_length_percentage_auto(min, None);
            let max_length = crate::layout::resolve_length_percentage_auto(max, None);
            let margins = margins_sum(margin.left, margin.right, None);
            let (cannot_shrink, cannot_grow) =
                (style.flex_shrink() == 0.0, style.flex_grow() == 0.0);
            let preferred = stated.or(keyword_width);
            let automatic_minimum = min.is_auto().then(|| AutomaticMinimumInputs {
                scrolls: is_scroll_container(style.overflow()),
                specified: preferred,
                maximum: max_length,
                height,
                across: edges,
            });
            if scrolls {
                // The flex base size floors the contribution of an item that
                // cannot shrink and caps that of an item that cannot grow, and
                // then the bounds of the item itself clamp it, the minimum over
                // the maximum. A scroll container has no automatic minimum.
                let content = preferred.or(scroll_content).unwrap_or(0.0);
                let floored = if cannot_shrink {
                    content.max(basis)
                } else {
                    content
                };
                let capped = if cannot_grow {
                    floored.min(basis)
                } else {
                    floored
                };
                let capped = max_length.map_or(capped, |max| capped.min(max));
                let width = min_length.map_or(capped, |min| capped.max(min));
                let key = tree
                    .nodes
                    .box_key(child_idx)
                    .expect("flex child has a box key");
                // Taffy takes the width of an item as its contribution once its
                // minimum and maximum width agree on it.
                let mut entries = Vec::new();
                if cannot_shrink {
                    entries.push((key, O::flex_shrink(1.0)));
                }
                for property in [P::Width, P::MinWidth, P::MaxWidth] {
                    entries.push((
                        key,
                        O::length(property, L::Px(width), true).expect("physical pinned width"),
                    ));
                }
                let cache = tree.take_layout_cache(child_idx);
                saved.push((child_idx, cache));
                tree.push_local_overrides(entries);
                continue;
            }
            // The flex base size floors the contribution, and then the item's own
            // bounds clamp it, the minimum over the maximum.
            let new_min = cannot_shrink.then(|| {
                let floor = max_length.map_or(basis, |max| basis.min(max));
                min_length.map_or(floor, |min| min.max(floor))
            });
            let new_max = (cannot_grow && preferred.is_none())
                .then(|| max_length.map_or(basis, |m| m.min(basis)) + margins);
            let bounded_width =
                preferred
                    .filter(|_| cannot_shrink || cannot_grow)
                    .map(|preferred| {
                        let floored = if cannot_shrink {
                            preferred.max(basis)
                        } else {
                            preferred
                        };
                        if cannot_grow {
                            floored.min(basis)
                        } else {
                            floored
                        }
                    });
            // The keyword width is what the item states, and Taffy does not see
            // it: it is installed as the width of the item however the flex base
            // size bounds it.
            let new_width = bounded_width.or(keyword_width).map(Dimension::length);
            let width_lowered = matches!(
                (bounded_width, preferred),
                (Some(bounded), Some(stated)) if bounded < stated
            );
            // A bound installed as `min-width` or `max-width` is an explicit one,
            // which leaves the item no automatic minimum size. The contribution
            // of an item with one is no less than that size however far its flex
            // base size would bound it. So is that of an item whose width is
            // lowered, though no bound is installed for it otherwise.
            let automatic = automatic_minimum
                .filter(|_| new_min.is_some() || new_max.is_some() || width_lowered)
                .map(|inputs| automatic_minimum_width(tree, child, inputs));
            let new_min = new_min
                .map(|value| automatic.map_or(value, |minimum| value.max(minimum)))
                .or(automatic.filter(|_| width_lowered));
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

/// The padding and border of the item `style` across its height, when its sizes
/// are those of its content box, and zero when they are those of its border box.
/// A percentage padding is of `basis`, the width of the containing block.
fn vertical_content_edges(style: &LayoutStyleView<'_>, basis: Option<f32>) -> f32 {
    if style.box_sizing() == BoxSizing::ContentBox {
        let padding = style.padding().resolve_or_zero(basis);
        let border = style.border().resolve_or_zero(basis);
        padding.top + padding.bottom + border.top + border.bottom
    } else {
        0.0
    }
}

/// Run `layout`, a layout of the column flex container `node_id` of one line,
/// whose height follows its items and whose inner width is `inner_width`, with
/// the `height` of each item that is taller than its content left out.
///
/// An item whose flex base size is a length is as tall as that size, within its
/// minimum height; its `height` only caps the automatic minimum height. The
/// contribution of the item to the height of the container is then its content
/// height when that is above the flex base size, though the `height` it states
/// is more. Taffy takes the stated `height` as the contribution, so an item
/// that states a height above its content and a smaller flex base size has the
/// `height` replaced by `auto` while the container finds its height. Its
/// content height is above the automatic minimum height there, whatever the
/// `height` caps it at, and that is all the `height` was for.
///
/// An item that scrolls has no automatic minimum height and Taffy takes its
/// flex base size as its contribution; an item the container does not give a
/// width of its own is left alone, as the height of its content follows from
/// the width it gets.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#intrinsic-item-contributions>
pub(super) fn with_column_item_content_height<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inner_width: Option<f32>,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    tree.sync_layout_cache_mutations();
    let container = tree.physical_style(usize::from(node_id));
    let Some(inner_width) =
        inner_width.filter(|_| !container.flex_direction_is_row() && !container.flex_wraps())
    else {
        return layout(tree);
    };
    let container_align = container.align_items();
    let children: Vec<NodeId> = tree.child_ids(node_id).collect();
    with_saved_probe_caches(tree, children.len(), |tree, saved| {
        for child in children {
            let child_idx = usize::from(child);
            let style = tree.physical_style(child_idx);
            if style.box_generation_mode() == BoxGenerationMode::None
                || style.position() == Position::Absolute
                || is_scroll_container(style.overflow())
            {
                continue;
            }
            let (Some(basis), Some(stated)) = (
                crate::layout::resolve_dimension(style.flex_basis(), None)
                    .filter(|basis| basis.is_finite()),
                crate::layout::resolve_dimension(style.size().height, None),
            ) else {
                continue;
            };
            if stated <= basis {
                continue;
            }
            let margin = style.margin();
            let stretched = style.size().width.is_auto()
                && !margin.left.is_auto()
                && !margin.right.is_auto()
                && matches!(
                    style.align_self().unwrap_or(container_align).keyword,
                    AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch
                );
            let across = horizontal_content_edges(&style, Some(inner_width));
            let width = if stretched {
                Some(
                    (inner_width - margins_sum(margin.left, margin.right, Some(inner_width)))
                        .max(0.0),
                )
            } else {
                crate::layout::resolve_dimension(style.size().width, Some(inner_width))
                    .map(|width| width + across)
            };
            // The width the container gives the item is within its `min-width`
            // and `max-width`, which the content is measured at. A bound that
            // does not resolve leaves the width the item gets unknown.
            let bound = |value: LengthPercentageAuto| {
                if value.is_auto() {
                    Some(None)
                } else {
                    crate::layout::resolve_length_percentage_auto(value, Some(inner_width))
                        .map(|bound| Some(bound + across))
                }
            };
            let (Some(width), Some(min), Some(max)) = (
                width,
                bound(style.min_size().width),
                bound(style.max_size().width),
            ) else {
                continue;
            };
            let width = max
                .map_or(width, |max| width.min(max))
                .max(min.unwrap_or(0.0))
                .max(across);
            let edges = vertical_content_edges(&style, Some(inner_width));
            let content = (tree
                .compute_child_layout(child, content_height_probe(width, inner_width))
                .size
                .height
                - edges)
                .max(0.0);
            if stated <= content {
                continue;
            }
            let key = tree
                .nodes
                .box_key(child_idx)
                .expect("flex child has a box key");
            let entry = O::length(P::Height, L::Auto, false).expect("physical automatic height");
            let cache = tree.take_layout_cache(child_idx);
            saved.push((child_idx, cache));
            tree.push_local_overrides(vec![(key, entry)]);
        }
        layout(tree)
    })
}

/// The inputs that ask a flex item for its height at the border-box `width` the
/// container gives it, whatever height it states.
fn content_height_probe(width: f32, inner_width: f32) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::ContentSize,
        axis: RequestedAxis::Vertical,
        known_dimensions: Size {
            width: Some(width),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(inner_width),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
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
        let margins = margins_sum(margin.left, margin.right, None);
        let height = height_before_width(&style, cross_room, false, container_align);
        let contribution = tree
            .compute_child_layout(child, min_content_probe(SizingMode::InherentSize, height))
            .size
            .width;
        widest = widest.max(contribution + margins);
    }
    widest
}

/// The content-box min-content inline height of a vertical wrapping flex box.
///
/// Each item contributes its own preferred min-content height, within its
/// bounds and with its margins, independently of its flex basis or flexing.
/// For a row, its width is the cross size before main sizing; for a column,
/// it is the unflexed main size before cross sizing. An automatic width stays
/// unknown in both measurements. Physical width percentages keep their
/// definite block-size basis, while percentage edges have no definite inline
/// height and contribute zero. The caller adds the container's insets once.
pub(super) fn wrapping_min_content_height(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    parent_width: Option<f32>,
) -> f32 {
    let children: Vec<NodeId> = tree.child_ids(node_id).collect();
    let mut tallest: f32 = 0.0;
    for child in children {
        let style = tree.physical_style(usize::from(child));
        if style.box_generation_mode() == BoxGenerationMode::None
            || style.position() == Position::Absolute
        {
            continue;
        }
        let margin = style.margin();
        let margins = margins_sum(margin.top, margin.bottom, None);
        let padding = style.padding();
        let border = style.border();
        let insets: f32 = [padding.left, padding.right, border.left, border.right]
            .into_iter()
            .map(|edge| edge.resolve(Some(0.0)).unwrap_or(0.0))
            .sum();
        let edges = if style.box_sizing() == BoxSizing::ContentBox {
            insets
        } else {
            0.0
        };
        // Keep the physical width basis without giving cyclic percentage
        // edges that block-size basis instead of their unknown inline size.
        let edge_overrides = if has_percent_padding_or_border(&style) {
            [
                (P::PaddingTop, padding.top),
                (P::PaddingRight, padding.right),
                (P::PaddingBottom, padding.bottom),
                (P::PaddingLeft, padding.left),
                (P::BorderTop, border.top),
                (P::BorderRight, border.right),
                (P::BorderBottom, border.bottom),
                (P::BorderLeft, border.left),
            ]
            .into_iter()
            .map(|(property, edge)| {
                O::length(
                    property,
                    L::Px(edge.resolve(Some(0.0)).unwrap_or(0.0)),
                    false,
                )
                .expect("resolved intrinsic edge")
            })
            .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let width = crate::layout::resolve_dimension(style.size().width, parent_width)
            .map(|width| width + edges)
            .or_else(|| {
                matches!(style.size().width.expand(), ExpandedDimension::Stretch)
                    .then(|| {
                        parent_width.map(|room| {
                            (room - margins_sum(margin.left, margin.right, None)).max(0.0)
                        })
                    })
                    .flatten()
            });
        // A replaced item's inherent-size probe trusts a known width, so
        // apply its numeric bounds before it derives the height from ratio.
        let width = width.map(|width| {
            let bound = |length| {
                crate::layout::resolve_length_percentage_auto(length, parent_width)
                    .map(|bound| bound + edges)
            };
            width
                .min(bound(style.max_size().width).unwrap_or(f32::INFINITY))
                .max(bound(style.min_size().width).unwrap_or(0.0))
                .max(insets)
        });
        let probe = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Vertical,
            known_dimensions: Size {
                width,
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: width.is_some(),
                height: false,
            },
            parent_size: Size {
                width: parent_width,
                height: None,
            },
            available_space: Size {
                width: width.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
                height: AvailableSpace::MinContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let measure =
            |tree: &mut LayoutSession<'_>| tree.compute_child_layout(child, probe).size.height;
        let contribution = if edge_overrides.is_empty() {
            measure(tree)
        } else {
            with_saved_probe_caches(tree, 1, |tree, saved| {
                let idx = usize::from(child);
                let key = tree.nodes.box_key(idx).expect("flex item has a box key");
                saved.push((idx, tree.take_layout_cache(idx)));
                tree.push_local_overrides(
                    edge_overrides
                        .into_iter()
                        .map(|entry| (key, entry))
                        .collect(),
                );
                measure(tree)
            })
        };
        tallest = tallest.max(contribution + margins);
    }
    tallest
}
