//! Original container constraints for hypothetical column cross measurements.

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode,
};
use raikiri_layout_style::{
    LayoutStyleView,
    values::{BoxGenerationMode, BoxSizing, MaybeResolve, Position, ResolveOrZero},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::Arc,
};

/// Physical constraints retained before a content-derived known size exists.
#[derive(Clone, Debug)]
pub(crate) struct Constraints {
    owner: usize,
    percentage_basis: Option<f32>,
    content_room: Option<f32>,
    measure_cross: bool,
    supported: bool,
    /// Whether some item has a percentage margin along the main axis, which
    /// refers to the cross size of the container that it measures.
    cyclic_main_margins: bool,
    /// These child caches have already been saved by the enclosing phase.
    isolated_items: Option<Arc<[usize]>>,
}

/// Whether the minimum main size of an item resolves before the container's
/// main size is known. A percentage main margin does not hold the measurement
/// back: it refers to the cross size being measured, counts as zero in the
/// item's contribution to that size, and resolves against the size once it is
/// settled (CSS Sizing 3 section 5.2.1).
fn outer_constraints_resolve(
    style: &LayoutStyleView<'_>,
    vertical: bool,
    main_basis: Option<f32>,
) -> bool {
    let minimum = if vertical {
        style.min_size().width
    } else {
        style.min_size().height
    };
    minimum.is_auto() || minimum.maybe_resolve(main_basis).is_some()
}

fn constraints(
    tree: &LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    main_is_definite: bool,
) -> Option<Constraints> {
    let idx = usize::from(node);
    let style = tree.physical_style(idx);
    if style.flex_direction_is_row() {
        return None;
    }
    let vertical = super::block_right_to_left(tree, idx).is_some();
    let (size, edges, _) = tree.block_definite_size(idx, &inputs);
    let measure_cross = if vertical {
        size.height.is_none()
    } else {
        size.width.is_none()
    };
    // A known cross can itself come from an earlier intrinsic measurement.
    // Require a stated size before resolving cyclic main percentage margins.
    let stated_cross = if vertical {
        style.size().height.maybe_resolve(inputs.parent_size.height)
    } else {
        style.size().width.maybe_resolve(inputs.parent_size.width)
    };
    let inline_basis = stated_cross.and_then(|_| {
        if vertical {
            size.height.map(|v| (v - edges.height).max(0.0))
        } else {
            size.width.map(|v| (v - edges.width).max(0.0))
        }
    });
    let (main_size, main_edges, parent, min, max) = if vertical {
        (
            size.width,
            edges.width,
            inputs.parent_size.width,
            style.min_size().width,
            style.max_size().width,
        )
    } else {
        (
            size.height,
            edges.height,
            inputs.parent_size.height,
            style.min_size().height,
            style.max_size().height,
        )
    };
    let percentage_basis = main_is_definite
        .then_some(main_size)
        .flatten()
        .map(|v| (v - main_edges).max(0.0));
    let content_room = percentage_basis.or_else(|| {
        // A percentage maximum does not constrain this intrinsic contribution.
        let maximum = max.maybe_resolve(None::<f32>)?;
        let minimum = min.maybe_resolve(parent).unwrap_or(0.0);
        let limit = maximum.max(minimum);
        Some(if style.box_sizing() == BoxSizing::ContentBox {
            limit.max(0.0)
        } else {
            (limit - main_edges).max(0.0)
        })
    });
    let supported = tree.child_ids(node).all(|child| {
        let item = tree.physical_style(usize::from(child));
        item.position() == Position::Absolute
            || item.box_generation_mode() == BoxGenerationMode::None
            || outer_constraints_resolve(&item, vertical, percentage_basis)
    });
    let cyclic_main_margins = measure_cross
        && inline_basis.is_none()
        && tree.child_ids(node).any(|child| {
            let item = tree.physical_style(usize::from(child));
            let margin = item.margin();
            let margins = if vertical {
                [margin.left, margin.right]
            } else {
                [margin.top, margin.bottom]
            };
            item.position() != Position::Absolute
                && item.box_generation_mode() != BoxGenerationMode::None
                && margins
                    .into_iter()
                    .any(|margin| !margin.is_auto() && margin.maybe_resolve(None::<f32>).is_none())
        });
    Some(Constraints {
        owner: idx,
        percentage_basis,
        content_room,
        measure_cross,
        supported,
        cyclic_main_margins,
        isolated_items: None,
    })
}

fn with_entry<R>(
    tree: &mut LayoutSession<'_>,
    entry: Constraints,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let depth = tree.context.column_cross_constraints.len();
    tree.context.column_cross_constraints.push(entry);
    let result = catch_unwind(AssertUnwindSafe(|| f(tree)));
    tree.context.column_cross_constraints.truncate(depth);
    match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}

/// Settle an automatic container cross size before ordinary item placement.
pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    main_is_definite: bool,
    algorithm: impl Fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let Some(original) = constraints(tree, node, inputs, main_is_definite) else {
        return algorithm(tree, node, inputs);
    };
    let wraps = tree.physical_style(usize::from(node)).flex_wraps();
    if !original.supported && !wraps {
        // Retain legacy behavior for unresolved minima. A wrapping container
        // still settles its cross size first, so its items keep their own
        // cross sizes in its lines.
        return with_entry(tree, original, |tree| algorithm(tree, node, inputs));
    }
    let idx = usize::from(node);
    let vertical = super::block_right_to_left(tree, idx).is_some();
    let has_orthogonal_paragraph = tree
        .child_ids(node)
        .any(|child| super::orthogonal_column_paragraph(tree, usize::from(child)).is_some());
    if !has_orthogonal_paragraph && !original.cyclic_main_margins {
        return with_entry(tree, original, |tree| algorithm(tree, node, inputs));
    }
    // The native pass visits every direct child, including replaced boxes.
    let items: Arc<[usize]> = tree
        .child_ids(node)
        .map(usize::from)
        .collect::<Vec<_>>()
        .into();
    let run = |tree: &mut LayoutSession<'_>, current: LayoutInput, isolated: bool| {
        let mut entry = original.clone();
        let cross_known = if vertical {
            current.known_dimensions.height.is_some()
        } else {
            current.known_dimensions.width.is_some()
        };
        entry.measure_cross &= !cross_known;
        entry.isolated_items = isolated.then(|| Arc::clone(&items));
        with_entry(tree, entry, |tree| algorithm(tree, node, current))
    };
    let cross_axis = if vertical {
        RequestedAxis::Vertical
    } else {
        RequestedAxis::Horizontal
    };
    // A main size measured on its own takes the percentage main margins of
    // the items, which refer to the cross size, from the cross size it
    // settles first, as the final layout does.
    let measures_main = inputs.run_mode == RunMode::ComputeSize
        && inputs.axis != cross_axis
        && original.cyclic_main_margins;
    if (inputs.run_mode != RunMode::PerformLayout && !measures_main) || !original.measure_cross {
        return isolate(tree, idx, &items, inputs.run_mode, |tree| {
            run(tree, inputs, true)
        });
    }
    let probe = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: cross_axis,
        ..inputs
    };
    let measured = isolate(tree, idx, &items, RunMode::ComputeSize, |tree| {
        run(tree, probe, true)
    });
    let mut settled = inputs;
    if vertical {
        settled.known_dimensions.height = Some(measured.size.height);
        settled.known_dimensions_are_definite.height = false;
    } else {
        settled.known_dimensions.width = Some(measured.size.width);
        settled.known_dimensions_are_definite.width = false;
    }
    isolate(tree, idx, &items, inputs.run_mode, |tree| {
        run(tree, settled, true)
    })
}

fn isolate<R>(
    tree: &mut LayoutSession<'_>,
    root: usize,
    items: &[usize],
    mode: RunMode,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    super::intrinsic_scopes::with_saved_probe_caches(tree, items.len() + 1, |tree, saved| {
        for idx in std::iter::once(root).chain(items.iter().copied()) {
            let mut cache = tree.take_layout_cache(idx);
            if mode == RunMode::PerformLayout {
                cache.forget_final_layout();
            }
            saved.push((idx, cache));
        }
        f(tree)
    })
}

fn entry<'a>(tree: &'a LayoutSession<'_>, parent: usize) -> Option<&'a Constraints> {
    tree.context
        .column_cross_constraints
        .last()
        .filter(|entry| entry.owner == parent)
}

pub(super) fn supported(tree: &LayoutSession<'_>, parent: usize) -> bool {
    entry(tree, parent).is_none_or(|entry| entry.supported)
}

pub(super) fn measures_cross(tree: &LayoutSession<'_>, parent: usize) -> bool {
    entry(tree, parent).is_none_or(|entry| entry.measure_cross)
}

pub(super) fn percentage_basis(tree: &LayoutSession<'_>, parent: usize) -> Option<Option<f32>> {
    entry(tree, parent).map(|entry| entry.percentage_basis)
}

/// Local edge resolution is valid only for a child saved by this phase.
pub(super) fn item_cache_isolated(tree: &LayoutSession<'_>, item: usize) -> bool {
    tree.taffy_parent_of(item)
        .and_then(|parent| entry(tree, parent))
        .and_then(|entry| entry.isolated_items.as_ref())
        .is_some_and(|items| items.contains(&item))
}

/// Replace only the main-axis inputs of a direct item's cross contribution.
pub(super) fn constrain_inputs(
    tree: &LayoutSession<'_>,
    item: usize,
    parent: usize,
    vertical: bool,
    inputs: &mut LayoutInput,
) {
    let mut available = AvailableSpace::MaxContent;
    if let Some(entry) = entry(tree, parent) {
        let inline_basis = super::ifc_parent_inline_size(tree, item, inputs.parent_size);
        let margin = tree
            .physical_style(item)
            .margin()
            .resolve_or_zero(inline_basis);
        let margins = if vertical {
            margin.left + margin.right
        } else {
            margin.top + margin.bottom
        };
        available = entry
            .content_room
            .map(|room| AvailableSpace::Definite((room - margins).max(0.0)))
            .unwrap_or(AvailableSpace::MaxContent);
        if vertical {
            inputs.parent_size.width = entry.percentage_basis;
        } else {
            inputs.parent_size.height = entry.percentage_basis;
        }
    }
    if vertical {
        inputs.available_space.width = available;
    } else {
        inputs.available_space.height = available;
    }
}

#[cfg(test)]
mod tests;
