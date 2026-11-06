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

/// Resolve the percentage `flex-basis` of the items of the column flex
/// container `node_id` that run in the other writing direction, against the
/// initial containing block, while the main size of the container is open.
///
/// The main size of such an item is its inline size. Browsers resolve a
/// percentage of it against the initial containing block along that axis, the
/// size an orthogonal flow is offered when its container has none (CSS Writing
/// Modes 4 section 7.3.1), rather than taking the content size. Along the main
/// axis of a column container that is the page area's height, or its width
/// where the container itself runs in a vertical writing mode. An item in the
/// writing direction of its container keeps the percentage, which is the
/// content size while the container leaves its main size open (CSS Flexbox 1
/// section 9.2.3).
///
/// `main_size_is_definite` tells whether the main size of the container is
/// definite; the percentage refers to it then, and nothing is overridden.
pub(super) fn with_orthogonal_percentage_flex_bases<R>(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    main_size_is_definite: bool,
    run_mode: RunMode,
    layout: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    use super::vertical_oof::used_writing_mode;
    let idx = usize::from(node_id);
    let container_vertical = used_writing_mode(tree, idx).is_vertical();
    let room = tree.fixed_page_area.and_then(|(width, height)| {
        let room = if container_vertical { width } else { height };
        (room > 0.0).then_some(room)
    });
    let Some(room) = room
        .filter(|_| !main_size_is_definite && !tree.physical_style(idx).flex_direction_is_row())
    else {
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
            let basis = tree.physical_style(idx).flex_basis();
            let percentage = match basis.expand() {
                ExpandedDimension::Percent(_) => true,
                ExpandedDimension::Calc(calc) => calc.has_percentage(),
                _ => false,
            };
            percentage
                .then(|| {
                    O::length(
                        P::FlexBasis,
                        L::Px(basis.maybe_resolve(Some(room)).expect("percentage basis")),
                        true,
                    )
                    .expect("physical flex basis")
                })
                .map(|entry| (idx, entry))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return layout(tree);
    }
    with_saved_probe_caches(tree, items.len(), |tree, saved| {
        for (idx, entry) in items {
            let key = tree.nodes.box_key(idx).expect("orthogonal item box");
            let mut cache = tree.take_layout_cache(idx);
            if run_mode == RunMode::PerformLayout {
                // A different final layout replaces descendant geometry, so
                // the old final entry can no longer stand for that geometry.
                cache.forget_final_layout();
            }
            saved.push((idx, cache));
            tree.push_local_overrides(vec![(key, entry)]);
        }
        layout(tree)
    })
}

/// Lay out the column flex container `node_id` with the height it gets from
/// its items' hypothetical main sizes, when its own height is open and some
/// item has a flex base size that is a length.
///
/// A browser finds the height of such a container as the sum of the
/// hypothetical main sizes of its items: the flex base size, kept within the
/// minimum and maximum height of the item (the automatic minimum height where
/// it states none). Taffy takes the max-content contribution of the item
/// instead, which is the content height of an item whose automatic minimum is
/// below it (a scroll container, an item with `min-height: 0`, a box of the
/// other writing direction), and which a `max-height` below the flex base
/// size does not bound. A flex factor then lets the container grow past the
/// sum as well.
///
/// Each such item is therefore made rigid while the container finds its
/// height: no flex factor, and a flex base size bounded by its `max-height`,
/// which gives Taffy the hypothetical size as the contribution. Items whose
/// flex base size is their content are left alone; their contribution is the
/// hypothetical size already.
///
/// With no `min-height` or `max-height` on the container the sum is the height,
/// and nothing is left to flex, so the rigid items are laid out as they are.
/// Otherwise the height the rigid items found is given to the container, which
/// lays the items out at their own flex factors within it.
///
/// spec: <https://www.w3.org/TR/css-flexbox-1/#algo-main-container>
pub(super) fn with_hypothetical_column_height(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    main_size_is_definite: bool,
    layout: impl Fn(&mut LayoutSession<'_>, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    use super::vertical_oof::used_writing_mode;
    let idx = usize::from(node_id);
    let container = tree.physical_style(idx);
    if main_size_is_definite
        || container.flex_direction_is_row()
        || container.flex_wraps()
        || inputs.known_dimensions.height.is_some()
        || (inputs.run_mode == RunMode::ComputeSize && inputs.axis == RequestedAxis::Horizontal)
        || used_writing_mode(tree, idx).is_vertical()
    {
        return layout(tree, inputs);
    }
    let bounded = container
        .max_size()
        .height
        .maybe_resolve(None::<f32>)
        .is_some()
        || container
            .min_size()
            .height
            .maybe_resolve(None::<f32>)
            .is_some_and(|min| min > 0.0);
    let rigid = tree
        .child_ids(node_id)
        .filter_map(|child| {
            let item = usize::from(child);
            if tree.nodes[item].layout_position() == Position::Absolute {
                return None;
            }
            let style = tree.physical_style(item);
            if style.box_generation_mode() == BoxGenerationMode::None {
                return None;
            }
            let basis = crate::layout::resolve_dimension(style.flex_basis(), None)
                .filter(|basis| basis.is_finite())?;
            let bounded_basis = style
                .max_size()
                .height
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
    if rigid.is_empty() {
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
    if inputs.run_mode == RunMode::ComputeSize || !bounded {
        return run(tree, rigid, inputs);
    }
    let height = run(
        tree,
        rigid,
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            axis: RequestedAxis::Vertical,
            ..inputs
        },
    )
    .size
    .height;
    let mut inputs = inputs;
    inputs.known_dimensions.height = Some(height);
    inputs.known_dimensions_are_definite.height = false;
    layout(tree, inputs)
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
