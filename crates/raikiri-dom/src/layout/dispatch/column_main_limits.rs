//! The automatic main size of a column with finite flex bases.

use super::intrinsic_scopes::with_saved_probe_caches;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode};
use raikiri_layout_style::values::{
    BoxGenerationMode, ExpandedLengthPercentage, MaybeResolve, Position, ResolveOrZero,
};
use raikiri_layout_style::{LayoutStyleView, LengthProperty as P, OwnedPropertyOverride as O};
use raikiri_style::ComputedLengthPercentageOrAuto as L;

/// Measure bounded hypothetical main sizes without changing the bases or
/// flexibility used by the ordinary layout. Unresolved item minima leave
/// the container on its existing path.
pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    main_is_definite: bool,
    algorithm: impl Fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let vertical = super::block_right_to_left(tree, idx).is_some();
    // Choose one main-size correction before any probe changes the item styles.
    let fallback = |tree: &mut LayoutSession<'_>| {
        super::intrinsic_scopes::with_hypothetical_column_main_size(
            tree,
            node_id,
            inputs,
            main_is_definite,
            vertical,
            |tree, inputs| algorithm(tree, node_id, inputs),
        )
    };
    let main_axis = if vertical {
        RequestedAxis::Horizontal
    } else {
        RequestedAxis::Vertical
    };
    let known_main = if vertical {
        inputs.known_dimensions.width
    } else {
        inputs.known_dimensions.height
    };
    let style = tree.physical_style(idx);
    if !super::column_cross_measure::supported(tree, idx)
        || main_is_definite
        || style.flex_direction_is_row()
        || style.flex_wraps()
        || style.block_aspect_ratio().is_some()
        || (inputs.run_mode == RunMode::ComputeSize
            && inputs.axis != main_axis
            && inputs.axis != RequestedAxis::Both)
    {
        return fallback(tree);
    }
    let needs_probe = tree
        .child_ids(node_id)
        .try_fold(false, |needs_probe, child| {
            let item = tree.physical_style(usize::from(child));
            if item.position() == Position::Absolute
                || item.box_generation_mode() == BoxGenerationMode::None
            {
                return Some(needs_probe);
            }
            let (basis, bounded) = finite_basis(&item, vertical)?;
            Some(needs_probe || bounded < basis)
        });
    if needs_probe != Some(true) {
        return fallback(tree);
    }

    let gap = style.gap().height;
    let percentage_gap = match gap.expand() {
        ExpandedLengthPercentage::Percent(_) => true,
        ExpandedLengthPercentage::Calc(value) => value.has_percentage(),
        _ => false,
    };
    let mut stable = Vec::new();
    if percentage_gap {
        // A content-derived known size must not resolve a cyclic main gap.
        stable.push((
            idx,
            O::length(P::RowGap, L::Px(gap.resolve_or_zero(Some(0.0))), true)
                .expect("resolved row gap"),
        ));
    }
    let mut probe = Vec::new();
    for child in tree.child_ids(node_id) {
        let child = usize::from(child);
        let item = tree.physical_style(child);
        if item.position() == Position::Absolute
            || item.box_generation_mode() == BoxGenerationMode::None
        {
            continue;
        }
        let (basis, bounded) = finite_basis(&item, vertical).expect("qualified finite basis");
        if bounded != basis {
            probe.push((
                child,
                O::length(P::FlexBasis, L::Px(bounded), true).expect("bounded flex basis"),
            ));
        }
        if item.flex_grow() != 0.0 {
            probe.push((child, O::flex_grow(0.0)));
        }
        if item.flex_shrink() != 0.0 {
            probe.push((child, O::flex_shrink(0.0)));
        }
        if item.position() == Position::Relative {
            let inset = item.inset();
            let insets = if vertical {
                [(P::Left, inset.left), (P::Right, inset.right)]
            } else {
                [(P::Top, inset.top), (P::Bottom, inset.bottom)]
            };
            for (property, value) in insets {
                if !value.is_auto() && value.maybe_resolve(None::<f32>).is_none() {
                    // Preserve unresolved insets, including the opposite-side fallback.
                    stable.push((
                        child,
                        O::length(property, L::Auto, false).expect("automatic inset"),
                    ));
                }
            }
        }
    }
    scoped(tree, idx, stable, inputs.run_mode, |tree| {
        if known_main.is_some() {
            return algorithm(tree, node_id, inputs);
        }
        #[cfg(test)]
        crate::layout::test_support::COLUMN_MAIN_PROBES.with(|count| count.set(count.get() + 1));
        let measured = scoped(tree, idx, probe, RunMode::ComputeSize, |tree| {
            algorithm(
                tree,
                node_id,
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    axis: main_axis,
                    ..inputs
                },
            )
        });
        if inputs.run_mode == RunMode::ComputeSize && inputs.axis == main_axis {
            return measured;
        }
        let mut settled = inputs;
        if vertical {
            settled.known_dimensions.width = Some(measured.size.width);
            settled.known_dimensions_are_definite.width = false;
        } else {
            settled.known_dimensions.height = Some(measured.size.height);
            settled.known_dimensions_are_definite.height = false;
        }
        algorithm(tree, node_id, settled)
    })
}

/// The authored basis and its bounded value use the same box-sizing space.
/// Native layout still applies the automatic minimum and padding/border floor.
fn finite_basis(style: &LayoutStyleView<'_>, vertical: bool) -> Option<(f32, f32)> {
    let basis = style.flex_basis().maybe_resolve(None::<f32>)?;
    let min = style.min_size();
    let max = style.max_size();
    let (min, max) = if vertical {
        (min.width, max.width)
    } else {
        (min.height, max.height)
    };
    let resolved_min = min.maybe_resolve(None::<f32>);
    if !min.is_auto() && resolved_min.is_none() {
        return None;
    }
    let min = resolved_min;
    let max = max.maybe_resolve(None::<f32>);
    if !basis.is_finite()
        || basis < 0.0
        || min.is_some_and(|v| !v.is_finite())
        || max.is_some_and(|v| !v.is_finite())
    {
        return None;
    }
    let bounded = max.map_or(basis, |max| basis.min(max));
    let bounded = min.map_or(bounded, |min| bounded.max(min));
    (bounded >= 0.0).then_some((basis, bounded))
}

/// Keep every changed box's cache separate, including the probe root.
fn scoped<R>(
    tree: &mut LayoutSession<'_>,
    root: usize,
    entries: Vec<(usize, O)>,
    run_mode: RunMode,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    if entries.is_empty() {
        return f(tree);
    }
    let mut boxes: Vec<_> = std::iter::once(root)
        .chain(entries.iter().map(|(idx, _)| *idx))
        .collect();
    boxes.sort_unstable();
    boxes.dedup();
    with_saved_probe_caches(tree, boxes.len(), |tree, saved| {
        for idx in boxes {
            let mut cache = tree.take_layout_cache(idx);
            if run_mode == RunMode::PerformLayout {
                cache.forget_final_layout();
            }
            saved.push((idx, cache));
        }
        let entries = entries
            .into_iter()
            .map(|(idx, entry)| (tree.nodes.box_key(idx).expect("live flex box"), entry))
            .collect();
        tree.push_local_overrides(entries);
        f(tree)
    })
}
