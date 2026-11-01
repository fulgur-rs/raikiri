//! Intrinsic sizing keywords and `stretch` on `min-width`, `max-width`,
//! `min-height` and `max-height` (CSS Sizing 3 §4 and §5, CSS Sizing 4 §5.1).
//!
//! Taffy's `min_size` and `max_size` hold lengths, percentages and `calc()`,
//! not the keywords. The bridge therefore leaves them `auto` and records each
//! keyword in [`ContentLimits`]; [`with_content_limits`] measures the box it
//! names when the box is laid out and puts the resulting lengths in the style
//! for the duration of that layout. Every algorithm that clamps the box to its
//! `min_size` and `max_size` then reads the sizes the keywords stand for.
//!
//! `stretch` needs no measure: it is the room the containing block offers, less
//! the margins of the box, which is what `width: stretch` and `height: stretch`
//! fill. Where that room is indefinite the keyword has no effect, as a
//! percentage that cannot resolve has none: a minimum is `0` and a maximum is
//! `none`.

use raikiri_layout_style::values::{
    BoxSizing, Dimension, Display, ExpandedDimension, ExpandedLengthPercentageAuto, LengthIdentity,
    LengthPercentage, LengthPercentageAuto, MaybeResolve, Position, ResolveOrZero,
};
use std::collections::HashMap;

use crate::layout::context::LayoutSession;
use crate::layout::dispatch::fit_content_room;
use crate::layout::dispatch::intrinsic_scopes::with_saved_probe_caches;
use crate::layout::ifc::boxes::{box_sizing_adjustment, box_sizing_height_adjustment};
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::taffy_adapter::{LayoutCache, columns_have_no_content_minimum};
use crate::node::FormattingContext;
use crate::node::arena::BoxKey;
use raikiri_layout_geometry::{Line, Size};
use raikiri_layout_style::BoxStyle as Style;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::property::{PositionValue, TableLayoutValue};

/// The `min-*` and `max-*` sizes of a box that are intrinsic sizing keywords
/// (`min-content`, `max-content`, `fit-content`, `fit-content(<length-percentage>)`)
/// or `stretch`, as the keyword [`Dimension`]s the `width` bridge produces. A
/// size that is no such keyword is `auto` here; Taffy's style carries every
/// other size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContentLimits {
    pub min: Size<Dimension>,
    pub max: Size<Dimension>,
}

impl ContentLimits {
    pub(crate) const NONE: Self = Self {
        min: Size::splat(Dimension::auto()),
        max: Size::splat(Dimension::auto()),
    };

    fn has_width(&self) -> bool {
        !self.min.width.is_auto() || !self.max.width.is_auto()
    }

    fn has_height(&self) -> bool {
        !self.min.height.is_auto() || !self.max.height.is_auto()
    }

    /// These limits without the `stretch` widths, which a parent that places
    /// the box beside floats resolves in the room it finds there.
    fn without_stretch_widths(mut self) -> Self {
        for width in [&mut self.min.width, &mut self.max.width] {
            if is_stretch(*width) {
                *width = Dimension::auto();
            }
        }
        self
    }
}

/// Whether `dimension` is the `stretch` keyword.
fn is_stretch(dimension: Dimension) -> bool {
    matches!(dimension.expand(), ExpandedDimension::Stretch)
}

/// Whether node `idx`, whose keyword sizes are `limits`, is laid out with
/// them resolved. The measures below lay the box out again; they see the style
/// without the limits.
fn applies(tree: &LayoutSession<'_>, idx: usize, limits: &ContentLimits) -> bool {
    let context = tree.nodes[idx].formatting_context;
    *limits != ContentLimits::NONE
        && !is_table_content_limits(tree, idx)
        && !tree.context.content_limit_probes.contains(&idx)
        && !is_oof_box(tree, idx)
        && matches!(
            context,
            FormattingContext::Block
                | FormattingContext::Inline
                | FormattingContext::Flex
                | FormattingContext::Grid
        )
}

fn is_oof_box(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.nodes[idx].fixed_position
        || matches!(
            *tree.nodes[idx].position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
}

fn is_table_content_limits(tree: &LayoutSession<'_>, idx: usize) -> bool {
    crate::layout::dispatch::is_table_box(tree.nodes[idx].display())
}

fn is_replaced_content(tree: &LayoutSession<'_>, idx: usize) -> bool {
    crate::paint_rules::is_replaced_element(tree.document, idx)
}

/// Whether `idx` has intrinsic min/max sizes that its flex/grid parent must
/// resolve before the parent algorithm reads the child's style.
fn applies_to_parent_item(tree: &LayoutSession<'_>, idx: usize, limits: &ContentLimits) -> bool {
    *limits != ContentLimits::NONE
        && !is_table_content_limits(tree, idx)
        && !tree.context.content_limit_probes.contains(&idx)
}

fn is_flex_grid_container(tree: &LayoutSession<'_>, idx: usize) -> bool {
    matches!(
        tree.nodes[idx].formatting_context,
        FormattingContext::Flex | FormattingContext::Grid
    )
}

fn is_flex_grid_item(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.nodes[idx].parent.is_some_and(|parent| {
        matches!(
            tree.nodes[parent].formatting_context,
            FormattingContext::Flex | FormattingContext::Grid
        )
    })
}

fn neutral_size_overrides(properties: &[LengthProperty]) -> Vec<OwnedPropertyOverride> {
    properties
        .iter()
        .map(|&property| {
            OwnedPropertyOverride::length(
                property,
                raikiri_style::ComputedLengthPercentageOrAuto::Auto,
                false,
            )
            .expect("neutral sizing property")
        })
        .collect()
}

fn with_size_overrides<R>(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    entries: Vec<OwnedPropertyOverride>,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let key = tree.nodes.box_key(idx).expect("live content-limit box");
    tree.with_overrides(entries.into_iter().map(|entry| (key, entry)).collect(), f)
}

fn push_size_overrides(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    entries: Vec<OwnedPropertyOverride>,
) {
    let key = tree.nodes.box_key(idx).expect("live content-limit box");
    tree.push_overrides(entries.into_iter().map(|entry| (key, entry)).collect());
}

/// Run `f`, a layout that resolves keyword sizes, with the measures of the
/// boxes inside it remembered for as long as the outermost such layout runs.
///
/// A box measured for its keyword sizes lays its contents out again for each
/// measure, and the boxes inside it do the same, so without the memory the
/// work doubles with every level of boxes that carry keyword sizes.
fn scoped<R>(tree: &mut LayoutSession<'_>, f: impl FnOnce(&mut LayoutSession<'_>) -> R) -> R {
    if tree.context.content_limit_depth == 0 {
        tree.context.content_limit_measures.clear();
    }
    tree.context.content_limit_depth += 1;
    let checkpoint = tree.override_checkpoint();
    let probe_depth = tree.context.content_limit_probes.len();
    let height_basis_depth = tree.context.content_limit_percentage_height_bases.len();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tree)));
    tree.restore_overrides_to(checkpoint);
    tree.context.content_limit_probes.truncate(probe_depth);
    tree.context
        .content_limit_percentage_height_bases
        .truncate(height_basis_depth);
    if result.is_err() {
        tree.layout_dirty = true;
        tree.context.content_limit_measures.clear();
    }
    tree.context.content_limit_depth -= 1;
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Lay out node `node_id` with `dispatch` after resolving its keyword `min-*`
/// and `max-*` sizes to lengths for the room `inputs` offers it.
///
/// The widths the keywords stand for are the min-content and max-content
/// widths of the box, measured without its `width`, `min-width` and
/// `max-width`; the heights are its automatic height at the width the limits
/// leave it. Both are border-box sizes and become the box-sizing box the
/// style names.
pub(super) fn with_content_limits(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    dispatch: impl FnOnce(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let limits = tree.nodes[idx].content_limits;
    if !applies(tree, idx, &limits) {
        return dispatch(tree, node_id, inputs);
    }
    scoped(tree, |tree| {
        // Resolve height only after the width bounds have been published.
        let mut inputs = inputs;
        let has_width =
            limits.has_width() || (is_replaced_content(tree, idx) && limits.has_height());
        let width = if has_width {
            resolved_width_bounds(tree, node_id, &limits, &inputs)
        } else {
            Bounds::default()
        };
        let adjustment = box_sizing_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
        with_size_overrides(tree, idx, width.overrides(true, adjustment), |tree| {
            if has_width && let Some(known) = inputs.known_dimensions.width {
                let known = width.apply(known);
                inputs.known_dimensions.width = Some(known);
                inputs.available_space.width = AvailableSpace::Definite(known);
            }
            let height = if limits.has_height() {
                resolved_height_bounds(tree, node_id, &limits, &inputs, width)
            } else {
                Bounds::default()
            };
            let adjustment =
                box_sizing_height_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
            with_size_overrides(tree, idx, height.overrides(false, adjustment), |tree| {
                inputs.known_dimensions.height = inputs
                    .known_dimensions
                    .height
                    .map(|known| height.apply(known));
                dispatch(tree, node_id, inputs)
            })
        })
    })
}

/// Lay out an out-of-flow box whose containing block is an inline element.
/// That placement path supplies definite inset sizes itself, so apply the
/// intrinsic bounds to those sizes before dispatching the child layout.
pub(crate) fn with_oof_child_content_limits(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    mut inputs: LayoutInput,
    dispatch: impl FnOnce(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let limits = tree.nodes[idx].content_limits;
    if limits == ContentLimits::NONE || !is_oof_box(tree, idx) || is_table_content_limits(tree, idx)
    {
        return dispatch(tree, node_id, inputs);
    }

    if tree.context.content_limit_probes.contains(&idx) {
        // The outer OOF wrapper has already installed numeric bounds. Avoid
        // measuring keywords again, but constrain inline-CB known dimensions
        // even when a fresh cache miss dispatches this final placement.
        if let Some(known) = inputs.known_dimensions.width {
            let known = clamp_known_width_to_style(tree, idx, inputs.parent_size.width, known);
            inputs.known_dimensions.width = Some(known);
            inputs.available_space.width = AvailableSpace::Definite(known);
        }
        inputs.known_dimensions.height = inputs.known_dimensions.height.map(|known| {
            clamp_known_height_to_style(
                tree,
                idx,
                inputs.parent_size.height,
                inputs.parent_size.width,
                known,
            )
        });
        return dispatch(tree, node_id, inputs);
    }

    scoped(tree, |tree| {
        let cache = tree.take_layout_cache(idx);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let width =
                if limits.has_width() || (is_replaced_content(tree, idx) && limits.has_height()) {
                    resolved_width_bounds(tree, node_id, &limits, &inputs)
                } else {
                    Bounds::default()
                };
            let adjustment =
                box_sizing_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
            with_size_overrides(tree, idx, width.overrides(true, adjustment), |tree| {
                if let Some(known) = inputs.known_dimensions.width {
                    let known = clamp_known_width_to_style(
                        tree,
                        idx,
                        inputs.parent_size.width,
                        width.apply(known),
                    );
                    inputs.known_dimensions.width = Some(known);
                    inputs.available_space.width = AvailableSpace::Definite(known);
                }
                let height = if limits.has_height() {
                    resolved_height_bounds(tree, node_id, &limits, &inputs, width)
                } else {
                    Bounds::default()
                };
                let adjustment = box_sizing_height_adjustment(
                    tree,
                    idx,
                    inputs.parent_size.width.unwrap_or(0.0),
                );
                with_size_overrides(tree, idx, height.overrides(false, adjustment), |tree| {
                    if limits.has_height() {
                        inputs.known_dimensions.height =
                            inputs.known_dimensions.height.map(|known| {
                                clamp_known_height_to_style(
                                    tree,
                                    idx,
                                    inputs.parent_size.height,
                                    inputs.parent_size.width,
                                    height.apply(known),
                                )
                            });
                    }
                    dispatch(tree, node_id, inputs)
                })
            })
        }));
        tree.restore_layout_cache(idx, cache);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
}

/// Resolve intrinsic limits before Taffy's out-of-flow algorithm reads a
/// candidate's style. The preliminary pass supplies the exact available
/// inset room, including static-position and grid-area adjustments; a second
/// pass with numeric min/max constraints removed supplies that unconstrained
/// room when those numeric constraints would have clamped it.
pub(crate) fn with_oof_content_limits<R>(
    tree: &mut LayoutSession<'_>,
    candidate_ids: Option<&[NodeId]>,
    clear_cached_nodes: &[NodeId],
    mut dispatch: impl FnMut(&mut LayoutSession<'_>) -> R,
) -> R {
    if candidate_ids.is_some() && tree.context.oof_content_limit_prepass_depth > 0 {
        return dispatch(tree);
    }
    let targets = candidate_ids
        .map(|candidates| {
            candidates
                .iter()
                .map(|&candidate| usize::from(candidate))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            (0..tree.nodes.len())
                .filter(|&idx| is_oof_box(tree, idx))
                .collect()
        })
        .into_iter()
        .filter(|&idx| {
            tree.nodes[idx].content_limits != ContentLimits::NONE
                && is_oof_box(tree, idx)
                && !is_table_content_limits(tree, idx)
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return dispatch(tree);
    }

    scoped(tree, |tree| {
        let saved_caches = targets
            .iter()
            .map(|&idx| (idx, tree.take_layout_cache(idx)))
            .collect::<Vec<_>>();

        let suppress_nested_prepasses = candidate_ids.is_some();
        let saved_prepass_caches = suppress_nested_prepasses.then(|| {
            (0..tree.nodes.len())
                .map(|idx| tree.clone_layout_cache(idx))
                .collect::<Vec<_>>()
        });
        let mut saved_prepass_caches = saved_prepass_caches;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.with_overrides(Vec::new(), |tree| {
                let constrained_inputs =
                    capture_oof_layout_inputs(tree, &mut dispatch, suppress_nested_prepasses);
                let raw_targets = targets
                    .iter()
                    .copied()
                    .filter(|idx| constrained_inputs.contains_key(idx))
                    .filter(|&idx| {
                        let style = tree.physical_style(idx);
                        let (min, max) = (style.min_size(), style.max_size());
                        !min.width.is_auto()
                            || !min.height.is_auto()
                            || !max.width.is_auto()
                            || !max.height.is_auto()
                    })
                    .collect::<Vec<_>>();
                let raw_inputs = if raw_targets.is_empty() {
                    None
                } else {
                    use LengthProperty::*;
                    let entries = raw_targets
                        .iter()
                        .flat_map(|&idx| {
                            let key = tree.nodes.box_key(idx).expect("live OOF candidate");
                            neutral_size_overrides(&[MinWidth, MinHeight, MaxWidth, MaxHeight])
                                .into_iter()
                                .map(move |entry| (key, entry))
                        })
                        .collect();
                    Some(tree.with_overrides(entries, |tree| {
                        capture_oof_layout_inputs(tree, &mut dispatch, suppress_nested_prepasses)
                    }))
                };

                for &idx in &targets {
                    let Some(positioned_inputs) = constrained_inputs.get(&idx) else {
                        continue;
                    };
                    let limits = tree.nodes[idx].content_limits;
                    let node_id = NodeId::from(idx);
                    let width = if limits.has_width()
                        || (is_replaced_content(tree, idx) && limits.has_height())
                    {
                        let inputs = raw_inputs
                            .as_ref()
                            .and_then(|inputs| inputs.get(&idx))
                            .unwrap_or(positioned_inputs);
                        resolved_width_bounds(tree, node_id, &limits, inputs)
                    } else {
                        Bounds::default()
                    };
                    let width_adjustment = box_sizing_adjustment(
                        tree,
                        idx,
                        positioned_inputs.parent_size.width.unwrap_or(0.0),
                    );
                    push_size_overrides(tree, idx, width.overrides(true, width_adjustment));

                    if limits.has_height() {
                        let mut height_inputs = *positioned_inputs;
                        if limits.has_width()
                            && let Some(known) = height_inputs.known_dimensions.width
                        {
                            let known = clamp_known_width_to_style(
                                tree,
                                idx,
                                height_inputs.parent_size.width,
                                known,
                            );
                            height_inputs.known_dimensions.width = Some(known);
                            height_inputs.available_space.width = AvailableSpace::Definite(known);
                        }
                        let height =
                            resolved_height_bounds(tree, node_id, &limits, &height_inputs, width);
                        let height_adjustment = box_sizing_height_adjustment(
                            tree,
                            idx,
                            positioned_inputs.parent_size.width.unwrap_or(0.0),
                        );
                        push_size_overrides(tree, idx, height.overrides(false, height_adjustment));
                    }
                }

                if let Some(caches) = saved_prepass_caches.take() {
                    // Discard cache entries produced while descendant OOF wrappers
                    // were bypassed for this candidate's preliminary measurements.
                    debug_assert_eq!(caches.len(), tree.nodes.len());
                    for (idx, cache) in caches.into_iter().enumerate() {
                        tree.restore_layout_cache(idx, cache);
                    }
                }
                for &node_id in clear_cached_nodes {
                    tree.nodes[usize::from(node_id)].cache.clear();
                }
                for &idx in &targets {
                    tree.nodes[idx].cache = LayoutCache::new();
                    tree.context.content_limit_probes.push(idx);
                }
                let output = dispatch(tree);
                for _ in &targets {
                    tree.context.content_limit_probes.pop();
                }
                output
            })
        }));
        if let Some(caches) = saved_prepass_caches {
            for (idx, cache) in caches.into_iter().enumerate() {
                tree.restore_layout_cache(idx, cache);
            }
        }
        for (idx, cache) in saved_caches.into_iter().rev() {
            tree.restore_layout_cache(idx, cache);
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
}

/// Used percentage padding and intrinsic width floors for tables positioned
/// against a physical containing area. Padding refers to its inline size;
/// width constraints continue to refer to its physical width.
pub(super) fn table_size_overrides(
    tree: &mut LayoutSession<'_>,
    candidates: &[NodeId],
    containing_size: Size<f32>,
    vertical: bool,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    let mut overrides = Vec::new();
    if vertical {
        for &candidate in candidates {
            let idx = usize::from(candidate);
            if !is_table_content_limits(tree, idx) {
                continue;
            }
            let padding = tree.physical_style(idx).padding();
            let key = tree.nodes.box_key(idx).expect("live out-of-flow table");
            for (property, edge) in [
                (LengthProperty::PaddingTop, padding.top),
                (LengthProperty::PaddingRight, padding.right),
                (LengthProperty::PaddingBottom, padding.bottom),
                (LengthProperty::PaddingLeft, padding.left),
            ] {
                if super::edge_depends_on_basis(edge) {
                    let used = edge.resolve_or_zero(Some(containing_size.height));
                    let entry = OwnedPropertyOverride::length(
                        property,
                        raikiri_style::ComputedLengthPercentageOrAuto::Px(used),
                        true,
                    )
                    .expect("resolved pixel padding");
                    overrides.push((key, entry));
                }
            }
        }
    }
    let floors = tree.with_overrides(overrides.clone(), |tree| {
        table_width_floors(tree, candidates, containing_size)
    });
    overrides.extend(floors);
    overrides
}

/// Width bounds that preserve the intrinsic width of positioned tables.
/// Horizontal tables whose used padding differs also need a minimum before
/// the parent fixes an authored width; their intrinsic border box is kept
/// while final column layout subtracts the used edges.
///
/// A table is never narrower than its columns can be, whatever its `max-width`
/// says (CSS Tables 3 §3, csswg-drafts#5336), but the library clamps every
/// out-of-flow box to its `max-width`, so a table with a `max-width` below its
/// min-content width came out as wide as the limit. The table is measured at
/// its min-content width, without its own `width`, `min-width` and `max-width`,
/// and a `max-width` below that width is raised to it. A `max-width` the
/// content fits in stays as it is: the library also offers the table that room
/// to wrap its content in. Width percentages use the physical width of
/// `containing_size`; percentage padding has already been resolved by
/// [`table_size_overrides`].
fn table_width_floors(
    tree: &mut LayoutSession<'_>,
    candidates: &[NodeId],
    containing_size: Size<f32>,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    let containing_width = containing_size.width;
    let limited = candidates
        .iter()
        .copied()
        .filter_map(|candidate| {
            let idx = usize::from(candidate);
            if !is_table_content_limits(tree, idx) || ignores_content_for_width(tree, idx) {
                return None;
            }
            let style = tree.physical_style(idx);
            let minimum_floor = !tree.has_vertical_writing_mode(idx)
                && style.padding() != style.intrinsic_table_padding();
            let adjustment = box_sizing_adjustment(tree, idx, containing_width);
            let maximum = style
                .max_size()
                .width
                .maybe_resolve(Some(containing_width))
                .map(|maximum| maximum + adjustment);
            if maximum.is_none() && !minimum_floor {
                return None;
            }
            Some((candidate, maximum, adjustment, minimum_floor))
        })
        .collect::<Vec<_>>();
    if limited.is_empty() {
        return Vec::new();
    }
    scoped(tree, |tree| {
        limited
            .into_iter()
            .flat_map(|(node_id, maximum, adjustment, minimum_floor)| {
                let idx = usize::from(node_id);
                let minimum =
                    min_content_width_without_limits(tree, node_id, containing_width, Vec::new());
                use LengthProperty::*;
                let key = tree.nodes.box_key(idx).expect("live out-of-flow table");
                let mut floors = Vec::new();
                if minimum_floor {
                    let authored_minimum = tree
                        .physical_style(idx)
                        .min_size()
                        .width
                        .maybe_resolve(Some(containing_width))
                        .unwrap_or(0.0)
                        + adjustment;
                    if minimum > authored_minimum {
                        floors.push((
                            key,
                            OwnedPropertyOverride::length(
                                MinWidth,
                                raikiri_style::ComputedLengthPercentageOrAuto::Px(
                                    (minimum - adjustment).max(0.0),
                                ),
                                true,
                            )
                            .expect("resolved pixel bound"),
                        ));
                    }
                }
                if maximum.is_some_and(|maximum| minimum > maximum) {
                    floors.push((
                        key,
                        OwnedPropertyOverride::length(
                            MaxWidth,
                            raikiri_style::ComputedLengthPercentageOrAuto::Px(
                                (minimum - adjustment).max(0.0),
                            ),
                            true,
                        )
                        .expect("resolved pixel bound"),
                    ));
                }
                floors
            })
            .collect()
    })
}

/// The min-content border-box width of the out-of-flow box `node_id`, measured
/// without its `width`, `min-width` and `max-width` and with the `extra`
/// overrides, as a box in a containing block `containing_width` wide.
fn min_content_width_without_limits(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    containing_width: f32,
    extra: Vec<OwnedPropertyOverride>,
) -> f32 {
    let idx = usize::from(node_id);
    let along = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size {
            width: Some(containing_width),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::MinContent,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let key = MeasureKey::new(tree, idx, true, &along, Bounds::default(), None);
    use LengthProperty::*;
    let mut neutral = neutral_size_overrides(&[Width, MinWidth, MaxWidth]);
    neutral.extend(extra);
    probe(tree, node_id, key, along, neutral)
}

/// The padding and border of the box `idx` summed along each axis, whatever its
/// `box-sizing`: the smallest border box it can have.
fn padding_border_sum(tree: &LayoutSession<'_>, idx: usize, basis: Option<f32>) -> Size<f32> {
    let style = tree.physical_style(idx);
    (style.padding().resolve_or_zero(basis) + style.border().resolve_or_zero(basis)).sum_axes()
}

/// Lay out a vertical flex or grid container with any authored ratio source
/// size raised to its padding and border before its axes are transposed.
/// Parent-imposed dimensions and intrinsic content measurements stay intact.
pub(super) fn with_vertical_ratio_stated_size_clamp(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    layout: impl FnOnce(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    if !tree.has_vertical_writing_mode(idx)
        || inputs.sizing_mode != SizingMode::InherentSize
        || tree.nodes[idx].ratio_in_content_box
    {
        return layout(tree, node_id, inputs);
    }
    let padding_basis = super::vertical_block::containing_inline_size(tree, idx, &inputs);
    let Some(clamp) = ratio_stated_size_clamp(tree, idx, inputs.parent_size, padding_basis) else {
        return layout(tree, node_id, inputs);
    };
    with_saved_probe_caches(tree, 1, |tree, saved| {
        saved.push((idx, tree.take_layout_cache(idx)));
        tree.push_local_overrides(vec![clamp]);
        layout(tree, node_id, inputs)
    })
}

/// The border-box sizes of the out-of-flow `candidates` with an aspect ratio
/// that are smaller than the padding and border they hold, raised to them.
///
/// A border box is no smaller than its padding and border, and the ratio
/// relates the box that results. The library derives the size the box does not
/// state from the one it states before it raises that to the padding and
/// border, so `aspect-ratio: 2 / 1; height: 40px` with 25px of border would be
/// 80px wide, not 100px as the 50px-tall box is.
pub(super) fn ratio_stated_size_clamps(
    tree: &LayoutSession<'_>,
    candidates: &[NodeId],
    containing_size: Size<f32>,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    candidates
        .iter()
        .filter_map(|&candidate| {
            ratio_stated_size_clamp(
                tree,
                usize::from(candidate),
                containing_size.map(Some),
                Some(containing_size.width),
            )
        })
        .collect()
}

/// One authored physical ratio source below its edge floor, resolved with
/// independent physical size and containing-inline padding bases.
fn ratio_stated_size_clamp(
    tree: &LayoutSession<'_>,
    idx: usize,
    size_basis: Size<Option<f32>>,
    padding_basis: Option<f32>,
) -> Option<(BoxKey, OwnedPropertyOverride)> {
    let style = tree.physical_style(idx);
    if style.stated_aspect_ratio().is_none() || style.box_sizing() == BoxSizing::ContentBox {
        return None;
    }
    let edges = padding_border_sum(tree, idx, padding_basis);
    let size = style.size();
    // The ratio derives a size only from the one the box states in the
    // other axis, so a box that states both has nothing to relate.
    for (property, stated, other, floor, basis) in [
        (
            LengthProperty::Width,
            size.width,
            size.height,
            edges.width,
            size_basis.width,
        ),
        (
            LengthProperty::Height,
            size.height,
            size.width,
            edges.height,
            size_basis.height,
        ),
    ] {
        if other.is_auto()
            && stated
                .maybe_resolve(basis)
                .is_some_and(|stated| stated < floor)
        {
            let entry = OwnedPropertyOverride::length(
                property,
                raikiri_style::ComputedLengthPercentageOrAuto::Px(floor),
                true,
            )
            .expect("resolved pixel size");
            let key = tree.nodes.box_key(idx).expect("live aspect-ratio box");
            return Some((key, entry));
        }
    }
    None
}

/// The automatic minimum width of the out-of-flow `candidates` whose width
/// follows from a height they state through their aspect ratio: their
/// min-content width, which the content raises that width to unless the box
/// scrolls or states a `min-width` (CSS Sizing 4 §5.1). The `max-width` of the
/// box caps it.
///
/// The library takes the width the ratio gives for an out-of-flow box as final,
/// so a box whose content needs more room is narrower than its content. It
/// clamps that width to the `min-width` of the box, so the floor goes there. A
/// minimum width carries across the ratio into a minimum height, which a box
/// that states its height must not get. An automatic minimum height is set
/// to `0`, even when an authored minimum width opts out of the content floor.
/// An authored minimum height remains intact, including the legitimate width
/// minimum it transfers through the ratio.
///
/// The width is measured without the `width`, `min-width`, `max-width` and
/// aspect ratio of the box, at its min-content size.
pub(super) fn ratio_width_floors(
    tree: &mut LayoutSession<'_>,
    candidates: &[NodeId],
    containing_size: Size<f32>,
) -> Vec<(BoxKey, OwnedPropertyOverride)> {
    let containing_width = containing_size.width;
    let following = candidates
        .iter()
        .copied()
        .filter(|&candidate| {
            let idx = usize::from(candidate);
            if tree.nodes[idx].content_limits != ContentLimits::NONE
                || is_table_content_limits(tree, idx)
                || is_replaced_content(tree, idx)
                || tree.has_vertical_writing_mode(idx)
            {
                return false;
            }
            let style = tree.physical_style(idx);
            style.stated_aspect_ratio().is_some()
                && style.size().width.is_auto()
                && style
                    .size()
                    .height
                    .maybe_resolve(Some(containing_size.height))
                    .is_some()
        })
        .collect::<Vec<_>>();
    if following.is_empty() {
        return Vec::new();
    }
    scoped(tree, |tree| {
        following
            .into_iter()
            .flat_map(|node_id| {
                let idx = usize::from(node_id);
                let style = tree.physical_style(idx);
                let content_minimum =
                    style.min_size().width.is_auto() && !style.overflow().x.is_scroll_container();
                let automatic_height = style.min_size().height.is_auto();
                let mut floors = Vec::with_capacity(2);
                if content_minimum {
                    // Intrinsic width is measured without height constraints:
                    // a wrapping column breaks at the room it is offered.
                    let mut unlimited = vec![OwnedPropertyOverride::aspect_ratio(None)];
                    unlimited.extend(neutral_size_overrides(&[
                        LengthProperty::Height,
                        LengthProperty::MinHeight,
                        LengthProperty::MaxHeight,
                    ]));
                    let minimum = min_content_width_without_limits(
                        tree,
                        node_id,
                        containing_width,
                        unlimited,
                    );
                    let style = super::block_flow::style_input::physical(tree, idx);
                    let metrics = super::block_flow::Metrics::resolve(
                        &style,
                        false,
                        tree.nodes[idx].ratio_in_content_box,
                        LayoutInput {
                            parent_size: containing_size.map(Some),
                            ..LayoutInput::HIDDEN
                        },
                    );
                    // Installing a content floor must preserve a minimum
                    // legitimately transferred from an authored min-height.
                    let minimum = minimum.max(metrics.min.width.unwrap_or(0.0));
                    let adjustment = box_sizing_adjustment(tree, idx, containing_width);
                    let floor = metrics
                        .own_max
                        .width
                        .map_or(minimum, |max| minimum.min(max));
                    floors.push((LengthProperty::MinWidth, (floor - adjustment).max(0.0)));
                }
                if automatic_height {
                    floors.push((LengthProperty::MinHeight, 0.0));
                }
                let key = tree.nodes.box_key(idx).expect("live out-of-flow box");
                floors.into_iter().map(move |(property, size)| {
                    let entry = OwnedPropertyOverride::length(
                        property,
                        raikiri_style::ComputedLengthPercentageOrAuto::Px(size),
                        true,
                    )
                    .expect("resolved pixel bound");
                    (key, entry)
                })
            })
            .collect()
    })
}

/// Whether the table `idx` takes its width from its style alone: a table with
/// `table-layout: fixed` and a `width` sizes its columns from that width and the
/// first row, whatever its cells hold (CSS 2.1 §17.5.2.1). Measuring it without
/// its `width` would use the automatic algorithm instead, so the width of its
/// content says nothing about the narrowest it can be.
fn ignores_content_for_width(tree: &LayoutSession<'_>, idx: usize) -> bool {
    tree.nodes[idx].table_layout == TableLayoutValue::Fixed
        && !tree.physical_style(idx).size().width.is_auto()
}

/// Clamp an out-of-flow preliminary width to its installed min/max sizes,
/// including box-sizing adjustments.
fn clamp_known_width_to_style(
    tree: &LayoutSession<'_>,
    idx: usize,
    parent_width: Option<f32>,
    known: f32,
) -> f32 {
    let style = tree.physical_style(idx);
    let adjustment = box_sizing_adjustment(tree, idx, parent_width.unwrap_or(0.0));
    let minimum = style
        .min_size()
        .width
        .maybe_resolve(parent_width)
        .map(|width| width + adjustment)
        .unwrap_or(0.0);
    let maximum = style
        .max_size()
        .width
        .maybe_resolve(parent_width)
        .map(|width| width + adjustment)
        .unwrap_or(f32::INFINITY)
        .max(minimum);
    known.max(minimum).min(maximum)
}

/// Clamp an out-of-flow preliminary height to its installed min/max sizes,
/// including box-sizing adjustments.
fn clamp_known_height_to_style(
    tree: &LayoutSession<'_>,
    idx: usize,
    parent_height: Option<f32>,
    parent_width: Option<f32>,
    known: f32,
) -> f32 {
    let style = tree.physical_style(idx);
    let adjustment = box_sizing_height_adjustment(tree, idx, parent_width.unwrap_or(0.0));
    let minimum = style
        .min_size()
        .height
        .maybe_resolve(parent_height)
        .map(|height| height + adjustment)
        .unwrap_or(0.0);
    let maximum = style
        .max_size()
        .height
        .maybe_resolve(parent_height)
        .map(|height| height + adjustment)
        .unwrap_or(f32::INFINITY)
        .max(minimum);
    known.max(minimum).min(maximum)
}

fn capture_oof_layout_inputs<R>(
    tree: &mut LayoutSession<'_>,
    dispatch: &mut impl FnMut(&mut LayoutSession<'_>) -> R,
    suppress_nested_prepasses: bool,
) -> HashMap<usize, LayoutInput> {
    let capture_depth = tree.context.oof_content_limit_inputs.len();
    let prepass_depth = tree.context.oof_content_limit_prepass_depth;
    tree.context.oof_content_limit_inputs.push(HashMap::new());
    if suppress_nested_prepasses {
        tree.context.oof_content_limit_prepass_depth += 1;
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = dispatch(tree);
    }));
    tree.context.oof_content_limit_prepass_depth = prepass_depth;
    let captured = tree
        .context
        .oof_content_limit_inputs
        .pop()
        .expect("OOF layout input capture was pushed above");
    tree.context
        .oof_content_limit_inputs
        .truncate(capture_depth);
    match result {
        Ok(()) => captured,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Give `style`, a copy of the style of the block-flow child `child`, the
/// lengths the keyword `min-width` and `max-width` of the child stand for when
/// its parent offers it the room `inputs` describe.
///
/// The parent sizes the child, and prepares the block context the child's
/// lines are laid out in, before the child is laid out; it must see the width
/// the limits leave the child.
pub(super) fn resolve_width_in_style(
    tree: &mut LayoutSession<'_>,
    child: NodeId,
    style: &mut Style,
    inputs: &LayoutInput,
) {
    let idx = usize::from(child);
    // The parent resolves a `stretch` width itself, in the room it finds for
    // the child.
    let limits = tree.nodes[idx].content_limits.without_stretch_widths();
    if !limits.has_width() || !applies(tree, idx, &limits) {
        return;
    }
    let bounds = scoped(tree, |tree| {
        resolved_width_bounds(tree, child, &limits, inputs)
    });
    let adjustment = box_sizing_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
    for entry in bounds.overrides(true, adjustment) {
        entry
            .apply_physical_sizing(style)
            .expect("local sizing bound");
    }
}

/// Put the lengths a `stretch` `min-height` and `max-height` of the block child
/// `child` stand for in `style`, when its parent, whose content box is `basis`
/// wide, offers it `room` for a `stretch` height (see [`stretch_fit_height`]).
///
/// The box lays itself out with these lengths in its style; a parent that
/// sizes an `auto` width through the aspect ratio of the box needs them too,
/// as the ratio carries a bound across to the width (CSS Sizing 4 §5.1).
pub(crate) fn resolve_stretch_height_limits_in_style(
    tree: &LayoutSession<'_>,
    child: NodeId,
    style: &mut Style,
    room: Option<f32>,
    basis: f32,
) {
    let idx = usize::from(child);
    let limits = tree.nodes[idx].content_limits;
    let Some(room) = room else {
        return;
    };
    if style.aspect_ratio.is_none()
        || !(is_stretch(limits.min.height) || is_stretch(limits.max.height))
        || !applies(tree, idx, &limits)
    {
        return;
    }
    let bounds = Bounds {
        min: is_stretch(limits.min.height).then_some(room),
        max: is_stretch(limits.max.height).then_some(room),
    };
    let adjustment = box_sizing_height_adjustment(tree, idx, basis);
    for entry in bounds.overrides(false, adjustment) {
        entry
            .apply_physical_sizing(style)
            .expect("local sizing bound");
    }
}

/// Run `f` with the lengths the keyword `min-width` and `max-width` of the
/// block child `node` stand for in its style, when its parent, whose content
/// box `parent_size` describes, offers it `room` for an `auto` width.
///
/// A paragraph root sizes its block children itself, from the child's style
/// and before the child is laid out.
pub(crate) fn with_width_limits<R>(
    tree: &mut LayoutSession<'_>,
    node: usize,
    parent_size: Size<Option<f32>>,
    room: f32,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let limits = tree.nodes[node].content_limits;
    if !limits.has_width() || !applies(tree, node, &limits) {
        return f(tree);
    }
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size,
        available_space: Size {
            width: AvailableSpace::Definite(room),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    let bounds = scoped(tree, |tree| {
        resolved_width_bounds(tree, NodeId::from(node), &limits, &inputs)
    });
    let adjustment = box_sizing_adjustment(tree, node, parent_size.width.unwrap_or(0.0));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_size_overrides(tree, node, bounds.overrides(true, adjustment), f)
    }));
    match result {
        Ok(value) => value,
        Err(payload) => {
            tree.layout_dirty = true;
            tree.context.content_limit_measures.clear();
            std::panic::resume_unwind(payload)
        }
    }
}

/// Resolve intrinsic min/max constraints on the in-flow children of a
/// flex/grid container before Taffy reads their styles to build item records.
/// The resolved values stay installed while the parent algorithm lays out the
/// children, then their styles and caches are restored.
pub(super) fn with_item_content_limits(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    parent_size: Size<Option<f32>>,
    dispatch: impl FnMut(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    with_item_content_limits_for_parent_sizes(tree, node_id, inputs, |_| parent_size, dispatch)
}

/// The in-flow items of the grid container `node_id` that a measure of the
/// container's min-content width must not widen past their `min-width`, each
/// with that `min-width` in the box-sizing box of the item's style.
///
/// Sized under a min-content constraint, a track takes the min-content
/// contribution of an item in place of its minimum contribution (CSS Grid 2
/// §11.5). Browsers do this only for an item whose `min-width` is `auto`: one
/// that states a `min-width` contributes no more than that, so a grid of
/// `min-width: 0` items can be as narrow as `0`. Taffy takes the min-content
/// contribution of every item that is not a scroll container, so the items
/// here are measured as if their `width` were their `min-width`, which makes
/// both contributions the minimum one.
fn items_held_to_minimum_width(
    tree: &LayoutSession<'_>,
    node_id: NodeId,
    inputs: &LayoutInput,
) -> Vec<(usize, f32)> {
    if inputs.run_mode != RunMode::ComputeSize
        || inputs.known_dimensions.width.is_some()
        || !matches!(inputs.available_space.width, AvailableSpace::MinContent)
        || super::grid_oof::right_to_left(tree, usize::from(node_id)).is_some()
        || !columns_have_no_content_minimum(&tree.physical_style(usize::from(node_id)))
    {
        return Vec::new();
    }
    tree.child_ids(node_id)
        .filter_map(|child| {
            let idx = usize::from(child);
            if tree.nodes[idx].layout_display() == Display::None
                || is_oof_box(tree, idx)
                || tree.context.content_limit_probes.contains(&idx)
            {
                return None;
            }
            let style = tree.physical_style(idx);
            // This probe measures only the minimum width contribution.
            // Replaced items keep their natural ratio in the authored-style
            // layout that follows, even when that size overflows the track.
            if !style.size().width.is_auto() || super::is_scroll_container(style.overflow()) {
                return None;
            }
            // Cyclic minimum-size percentages resolve against zero for the
            // intrinsic contribution, including those inside calc(). The
            // authored expression is restored for the final track-based size
            // (CSS Sizing 3 section 5.2.1).
            let minimum = match style.min_size().width.expand() {
                ExpandedLengthPercentageAuto::Length(minimum) => minimum,
                ExpandedLengthPercentageAuto::Percent(_) => 0.0,
                ExpandedLengthPercentageAuto::Calc(_) => {
                    crate::layout::used_style_length_percentage_auto(style.min_size().width, 0.0)?
                        .max(0.0)
                }
                _ => return None,
            };
            Some((idx, minimum))
        })
        .collect()
}

/// Lay out the grid `node_id` with `dispatch`, measuring its min-content width
/// with the items [`items_held_to_minimum_width`] names held to their
/// `min-width`.
///
/// The width is measured on its own, without the rows, since an item held to
/// its minimum is narrower than the track it stands in whenever another item
/// of the track is wider. Every other measure of the grid then runs at that
/// width, with the items as the author styled them.
pub(super) fn with_grid_item_content_limits(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    parent_size: Size<Option<f32>>,
    mut dispatch: impl FnMut(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let held = items_held_to_minimum_width(tree, node_id, &inputs);
    if held.is_empty() {
        return with_resolved_grid_item_limits(tree, node_id, inputs, parent_size, dispatch);
    }

    let width_inputs = LayoutInput {
        axis: RequestedAxis::Horizontal,
        ..inputs
    };
    // The override changes the inputs the box inside an item is asked with and
    // the item's own sizes, so it stays out of the cache generation and the
    // item's caches are kept apart while it lasts. Moving the generation would
    // have a chain of nested grids measure the one inside it again at each width
    // its parent tries.
    let width_output = scoped(tree, |tree| {
        with_saved_probe_caches(tree, held.len(), |tree, saved| {
            for &(idx, minimum) in &held {
                let key = tree.nodes.box_key(idx).expect("live grid item");
                let width = OwnedPropertyOverride::length(
                    LengthProperty::Width,
                    raikiri_style::ComputedLengthPercentageOrAuto::Px(minimum),
                    true,
                )
                .expect("resolved pixel width");
                saved.push((idx, tree.take_layout_cache(idx)));
                tree.push_local_overrides(vec![(key, width)]);
            }
            with_resolved_grid_item_limits(tree, node_id, width_inputs, parent_size, &mut dispatch)
        })
    });
    if inputs.axis == RequestedAxis::Horizontal {
        return width_output;
    }

    let width = width_output.size.width;
    let mut inputs = inputs;
    inputs.known_dimensions.width = Some(width);
    inputs.known_dimensions_are_definite.width = true;
    inputs.available_space.width = AvailableSpace::Definite(width);
    with_resolved_grid_item_limits(tree, node_id, inputs, parent_size, dispatch)
}

/// Resolve percentage fit-content and `stretch` limits on grid items against
/// the grid area sizes from a preliminary track layout, then lay out the grid
/// with those resolved limits installed.
fn with_resolved_grid_item_limits(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    parent_size: Size<Option<f32>>,
    mut dispatch: impl FnMut(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let children = tree.child_ids(node_id).collect::<Vec<_>>();
    let has_percentage_limit = children.iter().any(|&child| {
        let idx = usize::from(child);
        let limits = tree.nodes[idx].content_limits;
        applies_to_parent_item(tree, idx, &limits) && has_area_relative_limit(&limits)
    });
    if !has_percentage_limit {
        return with_item_content_limits(tree, node_id, inputs, parent_size, dispatch);
    }

    let cache_indices = children
        .iter()
        .map(|&child| usize::from(child))
        .filter(|&idx| {
            let limits = tree.nodes[idx].content_limits;
            applies_to_parent_item(tree, idx, &limits)
        })
        .collect::<Vec<_>>();
    let saved_caches = cache_indices
        .into_iter()
        .map(|idx| (idx, tree.take_layout_cache(idx)))
        .collect::<Vec<_>>();

    tree.context.grid_layout_info.remove(&usize::from(node_id));
    let _ = dispatch(tree, node_id, inputs);
    let area_bases = grid_item_area_bases(tree, node_id);
    for &(idx, _) in &saved_caches {
        tree.nodes[idx].cache = LayoutCache::new();
    }

    let output = with_item_content_limits_for_parent_sizes(
        tree,
        node_id,
        inputs,
        |idx| area_bases.get(&idx).copied().unwrap_or(parent_size),
        dispatch,
    );
    for (idx, cache) in saved_caches.into_iter().rev() {
        tree.restore_layout_cache(idx, cache);
    }
    output
}

/// The sizes of a flex or grid item `idx` that its container resolves before
/// reading the item's style.
struct ItemLimits {
    /// The keyword `min-*` and `max-*` sizes the item states, and the
    /// `min-content` minimum a flex item with an aspect ratio stands for when
    /// it states none ([`RatioMinimum`]).
    limits: ContentLimits,
    ratio_minimum: Option<RatioMinimum>,
}

impl ItemLimits {
    fn new(
        tree: &LayoutSession<'_>,
        container: usize,
        idx: usize,
        parent_size: Size<Option<f32>>,
    ) -> Self {
        let mut limits = tree.nodes[idx].content_limits;
        let ratio_minimum = (tree.nodes[container].formatting_context == FormattingContext::Flex)
            .then(|| ratio_minimum(tree, container, idx, parent_size, &limits))
            .flatten();
        match ratio_minimum {
            Some(RatioMinimum { width: true, .. }) => limits.min.width = Dimension::min_content(),
            Some(RatioMinimum { width: false, .. }) => limits.min.height = Dimension::min_content(),
            None => {}
        }
        Self {
            limits,
            ratio_minimum,
        }
    }
}

/// The automatic minimum size of a flex item that has a preferred aspect ratio
/// and states its size across the main axis of its container.
///
/// Such an item takes its size along the main axis from the ratio, and the
/// layout library holds its automatic minimum size down to that size. The
/// minimum is the min-content size of the item along the main axis, which
/// grows past the size the ratio gives when the content is larger (CSS
/// Flexbox 1 §4.5, CSS Sizing 4 §5.1). The item is therefore given a
/// `min-content` minimum along the main axis, capped by the size it states
/// there and by its `max-*` (and, along a column, by what the limits across
/// the item carry over the ratio), and a minimum of zero across it so that
/// the first does not carry across the ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RatioMinimum {
    /// Whether the main axis of the container is the width.
    width: bool,
    /// Whether the item leaves its minimum across the main axis `auto`.
    cross_is_auto: bool,
    /// The border-box size along the main axis that the ratio gives the size
    /// the item states across it, within its `min-*` and `max-*` there.
    transferred: f32,
    /// The same for the `min-*` and the `max-*` across the item alone. They
    /// bound the height of the content of a column item, but not the width of
    /// the content of a row item, which is laid out wider than the ratio gives.
    transferred_bounds: (Option<f32>, Option<f32>),
    /// The size the item states along the main axis and its `max-*` there, in
    /// the box-sizing box of the style, where they resolve.
    stated: Option<f32>,
    max: Option<f32>,
}

/// The [`RatioMinimum`] of the flex item `idx` of `container`, if the item
/// has one. `limits` are the keyword sizes the item states.
fn ratio_minimum(
    tree: &LayoutSession<'_>,
    container: usize,
    idx: usize,
    parent_size: Size<Option<f32>>,
    limits: &ContentLimits,
) -> Option<RatioMinimum> {
    if tree.nodes[idx].layout_display() == Display::None || is_oof_box(tree, idx) {
        return None;
    }
    let item = tree.physical_style(idx);
    let ratio = item.stated_aspect_ratio()?;
    if is_replaced_content(tree, idx) || tree.container_is_vertical(idx) {
        return None;
    }
    // A scroll container has no content-based minimum size.
    if super::is_scroll_container(item.overflow()) {
        return None;
    }
    let width = tree.physical_style(container).flex_direction_is_row();
    let (size, min_size, max_size) = (item.size(), item.min_size(), item.max_size());
    let (main, cross, cross_bounds, main_minimum, cross_minimum, bases) = if width {
        (
            (size.width, max_size.width, limits.max.width),
            size.height,
            (min_size.height, max_size.height),
            (min_size.width.is_auto(), limits.min.width.is_auto()),
            (min_size.height.is_auto(), limits.min.height.is_auto()),
            (parent_size.width, parent_size.height),
        )
    } else {
        (
            (size.height, max_size.height, limits.max.height),
            size.width,
            (min_size.width, max_size.width),
            (min_size.height.is_auto(), limits.min.height.is_auto()),
            (min_size.width.is_auto(), limits.min.width.is_auto()),
            (parent_size.height, parent_size.width),
        )
    };
    let (main_basis, cross_basis) = bases;
    let (main, main_max, main_max_keyword) = main;
    let states_main = matches!(
        main.expand(),
        ExpandedDimension::Auto
            | ExpandedDimension::Length(_)
            | ExpandedDimension::Percent(_)
            | ExpandedDimension::Calc(_)
    );
    // A keyword `max-*` is measured with the box, which the minimum would
    // have to be capped by.
    if !states_main || main_minimum != (true, true) || !main_max_keyword.is_auto() {
        return None;
    }
    let mut cross = cross.maybe_resolve(cross_basis)?;
    let bounds = (
        cross_bounds.0.maybe_resolve(cross_basis),
        cross_bounds.1.maybe_resolve(cross_basis),
    );
    if let Some(max) = bounds.1 {
        cross = cross.min(max);
    }
    if let Some(min) = bounds.0 {
        cross = cross.max(min);
    }
    // The ratio relates the content boxes when the box sizes its content, or
    // the ratio says `auto`, and the border boxes otherwise.
    let content_sized = item.box_sizing() == BoxSizing::ContentBox;
    let relates_content = content_sized || tree.nodes[idx].ratio_in_content_box;
    let basis = Some(parent_size.width.unwrap_or(0.0));
    let edges =
        (item.padding().resolve_or_zero(basis) + item.border().resolve_or_zero(basis)).sum_axes();
    let (edges_main, edges_cross) = if width {
        (edges.width, edges.height)
    } else {
        (edges.height, edges.width)
    };
    let carry = |cross: f32| {
        let border_box = cross + if content_sized { edges_cross } else { 0.0 };
        let related = if relates_content {
            (border_box - edges_cross).max(0.0)
        } else {
            border_box
        };
        let main = if width {
            related * ratio
        } else {
            related / ratio
        };
        if relates_content {
            main + edges_main
        } else {
            main
        }
    };
    let box_sizing_edges = if content_sized { edges_main } else { 0.0 };
    Some(RatioMinimum {
        width,
        cross_is_auto: cross_minimum == (true, true),
        stated: main
            .maybe_resolve(main_basis)
            .map(|stated| stated + box_sizing_edges),
        max: main_max
            .maybe_resolve(main_basis)
            .map(|max| max + box_sizing_edges),
        transferred: carry(cross),
        transferred_bounds: if width {
            (None, None)
        } else {
            (bounds.0.map(carry), bounds.1.map(carry))
        },
    })
}

/// `minimum`, the content-based automatic minimum size of a flex item along
/// its main axis, no larger than the size the item states there or its
/// `max-*` (CSS Flexbox 1 §4.5). Sizes are border-box sizes.
///
/// An item that states both its sizes gets no size from the ratio, but its
/// content is still at least the size the ratio carries across from the size
/// it states there, as browsers lay it out.
fn capped_ratio_minimum(ratio_minimum: RatioMinimum, minimum: Option<f32>) -> Option<f32> {
    let minimum = match ratio_minimum.stated {
        Some(_) => minimum.map(|minimum| minimum.max(ratio_minimum.transferred)),
        None => minimum,
    };
    let (transferred_min, transferred_max) = ratio_minimum.transferred_bounds;
    // The content is bounded by what the limits across the item carry over the
    // ratio to the main axis; the lower bound wins.
    let minimum = minimum.map(|minimum| {
        let minimum = transferred_max.map_or(minimum, |max| minimum.min(max));
        transferred_min.map_or(minimum, |min| minimum.max(min))
    });
    [ratio_minimum.stated, ratio_minimum.max]
        .into_iter()
        .flatten()
        .fold(minimum, |minimum, cap| {
            minimum.map(|minimum| minimum.min(cap))
        })
}

fn zero_length(property: LengthProperty) -> OwnedPropertyOverride {
    OwnedPropertyOverride::length(
        property,
        raikiri_style::ComputedLengthPercentageOrAuto::Px(0.0),
        true,
    )
    .expect("resolved pixel bound")
}

fn with_item_content_limits_for_parent_sizes(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    mut parent_size_for_item: impl FnMut(usize) -> Size<Option<f32>>,
    mut dispatch: impl FnMut(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let items = tree
        .child_ids(node_id)
        .filter_map(|child| {
            let idx = usize::from(child);
            let parent_size = parent_size_for_item(idx);
            let item = ItemLimits::new(tree, usize::from(node_id), idx, parent_size);
            applies_to_parent_item(tree, idx, &item.limits).then_some((child, parent_size, item))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return dispatch(tree, node_id, inputs);
    }

    scoped(tree, |tree| {
        let mut saved_caches = Vec::with_capacity(items.len());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.with_overrides(Vec::new(), |tree| {
                for (child, parent_size, item) in items {
                    let idx = usize::from(child);
                    let limits = item.limits;
                    let item_inputs = LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        sizing_mode: SizingMode::InherentSize,
                        axis: RequestedAxis::Both,
                        known_dimensions: Size::NONE,
                        known_dimensions_are_definite: Size {
                            width: false,
                            height: false,
                        },
                        parent_size,
                        available_space: Size {
                            width: parent_size
                                .width
                                .map(AvailableSpace::Definite)
                                .unwrap_or(inputs.available_space.width),
                            height: parent_size
                                .height
                                .map(AvailableSpace::Definite)
                                .unwrap_or(inputs.available_space.height),
                        },
                        vertical_margins_are_collapsible: Line::FALSE,
                    };

                    let mut width = if limits.has_width()
                        || (is_replaced_content(tree, idx) && limits.has_height())
                    {
                        resolved_width_bounds(tree, child, &limits, &item_inputs)
                    } else {
                        Bounds::default()
                    };
                    let width_adjustment =
                        box_sizing_adjustment(tree, idx, parent_size.width.unwrap_or(0.0));
                    if let Some(minimum) = item.ratio_minimum.filter(|minimum| minimum.width) {
                        width.min = capped_ratio_minimum(minimum, width.min);
                    }
                    push_size_overrides(tree, idx, width.overrides(true, width_adjustment));

                    if limits.has_height() {
                        let mut height =
                            resolved_height_bounds(tree, child, &limits, &item_inputs, width);
                        let height_adjustment = box_sizing_height_adjustment(
                            tree,
                            idx,
                            parent_size.width.unwrap_or(0.0),
                        );
                        if let Some(minimum) = item.ratio_minimum.filter(|minimum| !minimum.width) {
                            height.min = capped_ratio_minimum(minimum, height.min);
                        }
                        push_size_overrides(tree, idx, height.overrides(false, height_adjustment));
                    }
                    if let Some(minimum) =
                        item.ratio_minimum.filter(|minimum| minimum.cross_is_auto)
                    {
                        // Left `auto`, the minimum along the main axis would be
                        // carried across the aspect ratio into this one.
                        let cross = if minimum.width {
                            LengthProperty::MinHeight
                        } else {
                            LengthProperty::MinWidth
                        };
                        push_size_overrides(tree, idx, vec![zero_length(cross)]);
                    }

                    let cache = tree.take_layout_cache(idx);
                    tree.context.content_limit_probes.push(idx);
                    saved_caches.push((idx, cache));
                }

                dispatch(tree, node_id, inputs)
            })
        }));
        for (idx, cache) in saved_caches.into_iter().rev() {
            tree.restore_layout_cache(idx, cache);
        }
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
}

/// Whether a limit of `limits` resolves against the size of the area a grid
/// item sits in: a percentage `fit-content()` limit, or `stretch`.
fn has_area_relative_limit(limits: &ContentLimits) -> bool {
    [
        limits.min.width,
        limits.max.width,
        limits.min.height,
        limits.max.height,
    ]
    .into_iter()
    .any(|dimension| {
        is_stretch(dimension)
            || (matches!(
                dimension.expand(),
                ExpandedDimension::FitContentPercent(_) | ExpandedDimension::FitContentCalc(_)
            ) && dimension.uses_percentage())
    })
}

fn grid_item_area_bases(
    tree: &LayoutSession<'_>,
    node_id: NodeId,
) -> std::collections::HashMap<usize, Size<Option<f32>>> {
    let parent = usize::from(node_id);
    let Some(grid) = tree.context.grid_layout_info.get(&parent) else {
        return std::collections::HashMap::new();
    };
    let children = tree
        .child_ids(node_id)
        .filter(|&child| {
            let node = &tree.nodes[usize::from(child)];
            node.layout_display() != Display::None && node.layout_position() != Position::Absolute
        })
        .collect::<Vec<_>>();
    // The areas are on the layout axes of the grid, which are the physical
    // ones swapped in a vertical writing mode.
    let vertical = super::grid_oof::right_to_left(tree, parent).is_some();
    children
        .into_iter()
        .enumerate()
        .filter_map(|(item_index, child)| {
            let area = grid.item_area_size(item_index)?;
            let (width, height) = if vertical {
                (area.height, area.width)
            } else {
                (area.width, area.height)
            };
            Some((
                usize::from(child),
                Size {
                    width: Some(width),
                    height: Some(height),
                },
            ))
        })
        .collect()
}

/// The border-box bounds a keyword `min-*` and `max-*` size resolved to along
/// one axis.
#[derive(Clone, Copy, Default)]
struct Bounds {
    min: Option<f32>,
    max: Option<f32>,
}

impl Bounds {
    fn overrides(self, width: bool, adjustment: f32) -> Vec<OwnedPropertyOverride> {
        use LengthProperty::*;
        let (min, max) = if width {
            (MinWidth, MaxWidth)
        } else {
            (MinHeight, MaxHeight)
        };
        [(min, self.min), (max, self.max)]
            .into_iter()
            .filter_map(|(property, bound)| {
                bound.map(|bound| {
                    OwnedPropertyOverride::length(
                        property,
                        raikiri_style::ComputedLengthPercentageOrAuto::Px(
                            (bound - adjustment).max(0.0),
                        ),
                        true,
                    )
                    .expect("resolved pixel bound")
                })
            })
            .collect()
    }

    /// `size` within the bounds, the minimum winning over the maximum.
    fn apply(self, size: f32) -> f32 {
        let size = self.max.map_or(size, |max| size.min(max));
        self.min.map_or(size, |min| size.max(min))
    }
}

/// What a measure of a box is keyed by: the box, the axis measured, and
/// everything the layout it runs reads besides the box's contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MeasureKey {
    node: usize,
    publication_epoch: crate::layout::cache_context::CacheEpoch,
    pagination_transposed: bool,
    flex_intrinsic_probe: Option<usize>,
    width: bool,
    /// The basis a flex item's percentage edges were resolved against.
    edge_basis: Option<u32>,
    percentage_height_basis: Option<u32>,
    insets: [LengthIdentity; 8],
    cross_sizes: [Option<LengthIdentity>; 3],
    empty: bool,
    orthogonal_guard: bool,
    /// The minimum and maximum width the height is measured under.
    bounds: [Option<u32>; 2],
    known: [Option<u32>; 2],
    available: [(u8, u32); 2],
    parent: [Option<u32>; 2],
    /// The sizing mode is the content size, and the known sizes are definite.
    flags: [bool; 3],
}

impl MeasureKey {
    fn new(
        tree: &LayoutSession<'_>,
        node: usize,
        width: bool,
        inputs: &LayoutInput,
        bounds: Bounds,
        percentage_height_basis: Option<f32>,
    ) -> Self {
        let style = tree.physical_style(node);
        let (size, min_size, max_size) = (style.size(), style.min_size(), style.max_size());
        let (padding, border) = (style.padding(), style.border());
        let flex_grid = is_flex_grid_container(tree, node);
        // Key only the cross-axis sizes that the probe deliberately preserves.
        let cross_sizes = if width && flex_grid {
            [None; 3]
        } else if width {
            [
                Some(size.height.identity()),
                Some(min_size.height.identity()),
                Some(max_size.height.identity()),
            ]
        } else if flex_grid {
            [Some(size.width.identity()), None, None]
        } else {
            [
                Some(size.width.identity()),
                Some(min_size.width.identity()),
                Some(max_size.width.identity()),
            ]
        };
        let edge_basis = if tree.nodes[node].formatting_context == FormattingContext::Inline {
            super::ifc_parent_inline_size(tree, node, inputs.parent_size)
        } else {
            tree.resolved_edge_basis(node)
        };
        let bits = |size: Option<f32>| size.map(f32::to_bits);
        let space = |space: AvailableSpace| match space {
            AvailableSpace::Definite(size) => (0, size.to_bits()),
            AvailableSpace::MinContent => (1, 0),
            AvailableSpace::MaxContent => (2, 0),
        };
        Self {
            node,
            publication_epoch: tree.layout_cache_context.publication_epoch(),
            pagination_transposed: tree.context.pagination_on_transposed_axes,
            flex_intrinsic_probe: tree.context.flex_intrinsic_probe,
            width,
            edge_basis: bits(edge_basis),
            percentage_height_basis: bits(percentage_height_basis),
            insets: [
                padding.top,
                padding.right,
                padding.bottom,
                padding.left,
                border.top,
                border.right,
                border.bottom,
                border.left,
            ]
            .map(LengthPercentage::identity),
            cross_sizes,
            empty: tree.context.size_contained_measure.last() == Some(&node),
            orthogonal_guard: tree.context.orthogonal_block_in_layout == Some(node),
            bounds: [bits(bounds.min), bits(bounds.max)],
            known: [
                bits(inputs.known_dimensions.width),
                bits(inputs.known_dimensions.height),
            ],
            available: [
                space(inputs.available_space.width),
                space(inputs.available_space.height),
            ],
            parent: [
                bits(inputs.parent_size.width),
                bits(inputs.parent_size.height),
            ],
            flags: [
                inputs.sizing_mode == SizingMode::ContentSize,
                inputs.known_dimensions_are_definite.width,
                inputs.known_dimensions_are_definite.height,
            ],
        }
    }
}

/// Measure `node_id` as the layout the probe `inputs` describe, on an empty
/// cache that gives way to the entries the box had before, with the style
/// `neutral` leaves it. The measure is the width of the box when `key` is a
/// width measure, its height otherwise.
fn probe(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    key: MeasureKey,
    inputs: LayoutInput,
    neutral: Vec<OwnedPropertyOverride>,
) -> f32 {
    if let Some(&size) = tree.context.content_limit_measures.get(&key) {
        return size;
    }
    let idx = usize::from(node_id);
    let output = with_size_overrides(tree, idx, neutral, |tree| {
        tree.context.content_limit_probes.push(idx);
        let cache = tree.take_layout_cache(idx);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tree.compute_child_layout_with_block_ctx(node_id, inputs, None)
        }));
        tree.restore_layout_cache(idx, cache);
        tree.context.content_limit_probes.pop();
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    });
    let size = if key.width {
        output.size.width
    } else {
        output.size.height
    };
    tree.context.content_limit_measures.insert(key, size);
    size
}

/// Whether the box `idx` is a flex container that wraps its items onto lines
/// of a vertical main axis, on the axes of a horizontal writing mode.
pub(super) fn is_wrapping_column_flex(tree: &LayoutSession<'_>, idx: usize) -> bool {
    if tree.nodes[idx].formatting_context != FormattingContext::Flex
        || tree.has_vertical_writing_mode(idx)
    {
        return false;
    }
    let style = tree.physical_style(idx);
    !style.flex_direction_is_row() && style.flex_wraps()
}

/// The border-box min-content width of the flex container `node_id` when it
/// wraps its items onto lines of a vertical main axis, which `inputs` ask for.
///
/// Taffy answers a min-content request with the sum of the widths of the
/// lines, which are as many as the height of the container makes them. A line
/// of such a container is as wide as its widest item, and the min-content
/// width of the container is the widest item of all, as if the height left
/// room for a single line (Chromium: a container of two 26px items in a 100px
/// height is 26px wide at min-content and 52px at max-content). The container
/// is measured with its height and its width limits left open, and the caller
/// clamps the width by the limits. A container whose width, or the width of an
/// item, follows its height (see [`width_follows_container_height`]) keeps the
/// sum of its lines, which is measured with the height.
pub(super) fn wrapping_column_min_content_width(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: &LayoutInput,
) -> Option<f32> {
    let idx = usize::from(node_id);
    if inputs.run_mode != RunMode::ComputeSize
        || inputs.axis == RequestedAxis::Vertical
        || !matches!(inputs.available_space.width, AvailableSpace::MinContent)
        || inputs.known_dimensions.width.is_some()
        || tree.context.content_limit_probes.contains(&idx)
        || !is_wrapping_column_flex(tree, idx)
    {
        return None;
    }
    // A length is the width of the box whatever its items ask for, and
    // `max-content` is the sum of the lines; the other widths (`auto`, a
    // percentage, `fit-content`, `stretch`, `min-content`) take the widest
    // item into account.
    if matches!(
        tree.physical_style(idx).size().width.expand(),
        ExpandedDimension::Length(_) | ExpandedDimension::Calc(_) | ExpandedDimension::MaxContent
    ) || width_follows_container_height(tree, node_id)
    {
        return None;
    }
    let mut along = *inputs;
    along.axis = RequestedAxis::Horizontal;
    along.known_dimensions.height = None;
    along.known_dimensions_are_definite.height = false;
    along.available_space.height = AvailableSpace::MaxContent;
    let key = MeasureKey::new(tree, idx, true, &along, Bounds::default(), None);
    use LengthProperty::*;
    Some(probe(
        tree,
        node_id,
        key,
        along,
        neutral_size_overrides(&[Width, MinWidth, MaxWidth, Height, MinHeight, MaxHeight]),
    ))
}

/// Whether the width of the flex container `node_id` or of one of its items
/// can follow the height of the container: the container has a ratio, or an
/// item takes a percentage of the height (or all of it), as a box with a ratio
/// and a percentage height does.
///
/// [`wrapping_column_min_content_width`] leaves the height of the container
/// open, so such a width would be measured without what it follows. An item
/// that grows to the height does not count: Chromium leaves the grown height
/// out of the min-content contribution of the item as well.
fn width_follows_container_height(tree: &LayoutSession<'_>, node_id: NodeId) -> bool {
    let takes_height = |idx: usize| {
        let style = tree.physical_style(idx);
        matches!(
            style.size().height.expand(),
            ExpandedDimension::Percent(_)
                | ExpandedDimension::Calc(_)
                | ExpandedDimension::FitContentPercent(_)
                | ExpandedDimension::FitContentCalc(_)
                | ExpandedDimension::Stretch
        ) || [style.min_size().height, style.max_size().height]
            .into_iter()
            .any(|limit| {
                matches!(
                    limit.expand(),
                    ExpandedLengthPercentageAuto::Percent(_)
                        | ExpandedLengthPercentageAuto::Calc(_)
                )
            })
    };
    tree.physical_style(usize::from(node_id))
        .stated_aspect_ratio()
        .is_some()
        || tree
            .child_ids(node_id)
            .any(|item| takes_height(usize::from(item)))
}

/// The border-box widths the keyword `min-width` and `max-width` of `node_id`
/// stand for in the room `inputs` offers it.
fn replaced_keyword_bounds(
    tree: &LayoutSession<'_>,
    idx: usize,
    limits: &ContentLimits,
    inputs: &LayoutInput,
) -> (Bounds, Bounds) {
    let (mut width, mut height) = intrinsic_replaced_bounds(tree, idx, limits, inputs);
    // `stretch` fills the room the same way for a replaced box; it takes no
    // part in the ratio the intrinsic keywords are carried across, which the
    // layout of the box does with the length it gets.
    if is_replaced_content(tree, idx) {
        let (min, max) = (limits.min.width, limits.max.width);
        if is_stretch(min) || is_stretch(max) {
            let stretch = stretch_width(tree, idx, inputs);
            if is_stretch(min) {
                width.min = stretch;
            }
            if is_stretch(max) {
                width.max = stretch;
            }
        }
        let stretch = stretch_fit_height(tree, idx, inputs);
        if is_stretch(limits.min.height) {
            height.min = stretch;
        }
        if is_stretch(limits.max.height) {
            height.max = stretch;
        }
    }
    (width, height)
}

/// The border-box bounds the intrinsic keywords among the `min-*` and `max-*`
/// sizes of a replaced box stand for.
fn intrinsic_replaced_bounds(
    tree: &LayoutSession<'_>,
    idx: usize,
    limits: &ContentLimits,
    inputs: &LayoutInput,
) -> (Bounds, Bounds) {
    if !is_replaced_content(tree, idx) {
        return (Bounds::default(), Bounds::default());
    }
    let Some(natural) = super::leaf_intrinsic_size(&tree.nodes[idx]) else {
        return (Bounds::default(), Bounds::default());
    };
    let mut style = super::block_flow::style_input::physical(tree, idx);
    if style.aspect_ratio.is_none() {
        super::apply_local_sizing(
            &mut style,
            &[OwnedPropertyOverride::aspect_ratio(
                tree.nodes[idx].keyword_aspect_ratio,
            )],
        );
    }
    // A `stretch` height whose room is definite is a stated height, which the
    // ratio carries across to the width the keywords stand for.
    if matches!(style.size.height.expand(), ExpandedDimension::Stretch)
        && let Some(room) = stretch_fit_height(tree, idx, inputs)
    {
        let adjustment =
            box_sizing_height_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
        super::apply_local_sizing(
            &mut style,
            &[OwnedPropertyOverride::length(
                LengthProperty::Height,
                raikiri_style::ComputedLengthPercentageOrAuto::Px((room - adjustment).max(0.0)),
                true,
            )
            .expect("resolved pixel height")],
        );
    }
    let content_box_ratio = tree.nodes[idx].ratio_in_content_box;
    let Some(bounds) =
        super::replaced_size::keyword_bounds(&style, inputs, natural, *limits, content_box_ratio)
    else {
        return (Bounds::default(), Bounds::default());
    };
    (
        Bounds {
            min: bounds.width.min,
            max: bounds.width.max,
        },
        Bounds {
            min: bounds.height.min,
            max: bounds.height.max,
        },
    )
}

fn resolved_width_bounds(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    limits: &ContentLimits,
    inputs: &LayoutInput,
) -> Bounds {
    let idx = usize::from(node_id);
    if is_replaced_content(tree, idx) {
        return replaced_keyword_bounds(tree, idx, limits, inputs).0;
    }
    width_bounds(tree, node_id, limits, inputs)
}

fn resolved_height_bounds(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    limits: &ContentLimits,
    inputs: &LayoutInput,
    width: Bounds,
) -> Bounds {
    let idx = usize::from(node_id);
    if is_replaced_content(tree, idx) {
        return replaced_keyword_bounds(tree, idx, limits, inputs).1;
    }
    height_bounds(tree, node_id, limits, inputs, width)
}

fn width_bounds(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    limits: &ContentLimits,
    inputs: &LayoutInput,
) -> Bounds {
    let idx = usize::from(node_id);
    let flex_grid_container = is_flex_grid_container(tree, idx);
    let mut measures: Option<(f32, f32)> = None;
    let mut measure_contents = |tree: &mut LayoutSession<'_>| {
        *measures.get_or_insert_with(|| {
            let mut measure = |space| {
                let along = LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: Size::NONE,
                    known_dimensions_are_definite: Size {
                        width: false,
                        height: false,
                    },
                    parent_size: inputs.parent_size,
                    available_space: Size {
                        width: space,
                        height: AvailableSpace::MaxContent,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                };
                let key = MeasureKey::new(tree, idx, true, &along, Bounds::default(), None);
                use LengthProperty::*;
                let properties: &[LengthProperty] = if flex_grid_container {
                    &[Width, Height, MinWidth, MinHeight, MaxWidth, MaxHeight]
                } else {
                    &[Width, MinWidth, MaxWidth]
                };
                probe(
                    tree,
                    node_id,
                    key,
                    along,
                    neutral_size_overrides(properties),
                )
            };
            (
                measure(AvailableSpace::MinContent),
                measure(AvailableSpace::MaxContent),
            )
        })
    };
    let mut resolved = [None, None];
    for (slot, keyword) in resolved
        .iter_mut()
        .zip([limits.min.width, limits.max.width])
    {
        let expanded = keyword.expand();
        if matches!(expanded, ExpandedDimension::Stretch) {
            *slot = stretch_width(tree, idx, inputs);
            continue;
        }
        if !matches!(
            expanded,
            ExpandedDimension::MinContent
                | ExpandedDimension::MaxContent
                | ExpandedDimension::FitContent
                | ExpandedDimension::FitContentPx(_)
                | ExpandedDimension::FitContentPercent(_)
                | ExpandedDimension::FitContentCalc(_)
        ) {
            continue;
        }
        let (min_content, max_content) = measure_contents(tree);
        let fit = |room: f32| max_content.min(room).max(min_content);
        // The limit of `fit-content(<length-percentage>)` stands in for the
        // room the parent offers; one that cannot resolve leaves the box
        // sized by the room, as `fit-content` is.
        let limit = fit_content_room(tree, idx, expanded, inputs.parent_size.width, true);
        *slot = Some(match (expanded, limit) {
            (ExpandedDimension::MinContent, _) => min_content,
            (ExpandedDimension::MaxContent, _) => max_content,
            (_, Some(limit)) => fit(limit),
            (_, None) => match inputs.available_space.width {
                AvailableSpace::MinContent => min_content,
                AvailableSpace::MaxContent => max_content,
                AvailableSpace::Definite(room) => fit(stretch_fit_width(tree, idx, inputs, room)),
            },
        });
    }
    Bounds {
        min: resolved[0],
        max: resolved[1],
    }
}

/// The border-box heights the keyword `min-height` and `max-height` of
/// `node_id` stand for: along the block axis every intrinsic keyword is the
/// automatic height of the box at the width `inputs` give it, within the
/// `width` bounds already installed in its style (CSS Sizing 3 §5.1), and
/// `stretch` is the definite height of the containing block less the margins
/// the box takes off it.
fn height_bounds(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    limits: &ContentLimits,
    inputs: &LayoutInput,
    width: Bounds,
) -> Bounds {
    let idx = usize::from(node_id);
    let stretch = stretch_fit_height(tree, idx, inputs);
    let is_measured = |size: Dimension| !size.is_auto() && !is_stretch(size);
    let measured = (is_measured(limits.min.height) || is_measured(limits.max.height))
        .then(|| automatic_height(tree, node_id, inputs, width));
    let bound = |size: Dimension| {
        if is_stretch(size) {
            stretch
        } else if size.is_auto() {
            None
        } else {
            measured
        }
    };
    Bounds {
        min: bound(limits.min.height),
        max: bound(limits.max.height),
    }
}

/// The automatic border-box height of `node_id` at the width `inputs` give it,
/// within the `width` bounds already installed in its style.
fn automatic_height(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: &LayoutInput,
    width: Bounds,
) -> f32 {
    let idx = usize::from(node_id);
    let flex_grid_container = is_flex_grid_container(tree, idx);
    let width_adjustment =
        box_sizing_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
    let mut along = *inputs;
    along.run_mode = RunMode::ComputeSize;
    along.axis = RequestedAxis::Both;
    along.known_dimensions.height = None;
    along.known_dimensions_are_definite.height = false;
    along.vertical_margins_are_collapsible = Line::FALSE;
    let percentage_height_basis = (is_oof_box(tree, idx)
        || is_flex_grid_container(tree, idx)
        || is_flex_grid_item(tree, idx))
    .then(|| content_percentage_height_basis(tree, idx, inputs))
    .flatten();
    let key = MeasureKey::new(tree, idx, false, &along, width, percentage_height_basis);
    if let Some(height) = percentage_height_basis {
        tree.context
            .content_limit_percentage_height_bases
            .push((idx, height));
    }
    use LengthProperty::*;
    let properties: &[LengthProperty] = if flex_grid_container {
        &[Height, MinWidth, MinHeight, MaxWidth, MaxHeight]
    } else {
        &[Height, MinHeight, MaxHeight]
    };
    let mut entries = neutral_size_overrides(properties);
    if flex_grid_container {
        entries.extend(width.overrides(true, width_adjustment));
    }
    let height = probe(tree, node_id, key, along, entries);
    if percentage_height_basis.is_some() {
        tree.context.content_limit_percentage_height_bases.pop();
    }
    height
}

/// The content-box height the measured box's authored size or its caller's
/// definite size offers to percentage-sized children during intrinsic sizing.
fn content_percentage_height_basis(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
) -> Option<f32> {
    let style = tree.physical_style(idx);
    let parent_width = inputs.parent_size.width.unwrap_or(0.0);
    let edges = (style.padding().resolve_or_zero(Some(parent_width))
        + style.border().resolve_or_zero(Some(parent_width)))
    .sum_axes()
    .height;
    if let Some(height) = inputs.known_dimensions.height {
        return Some((height - edges).max(0.0));
    }

    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        edges
    } else {
        0.0
    };
    let preferred = style
        .size()
        .height
        .maybe_resolve(inputs.parent_size.height)?
        + box_sizing_adjustment;
    let minimum = style
        .min_size()
        .height
        .maybe_resolve(inputs.parent_size.height)
        .map(|height| height + box_sizing_adjustment)
        .unwrap_or(0.0);
    Some((preferred.max(minimum) - edges).max(0.0))
}

/// The border-box width the box `idx` stretches to in the width `inputs`
/// offer it, which an intrinsic sizing pass leaves open (see
/// [`stretch_fit_width`]).
fn stretch_width(tree: &LayoutSession<'_>, idx: usize, inputs: &LayoutInput) -> Option<f32> {
    match inputs.available_space.width {
        AvailableSpace::Definite(room) => Some(stretch_fit_width(tree, idx, inputs, room)),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
    }
}

/// The border-box width the box `idx` would stretch to in the room `inputs`
/// offers it: the width of its containing block less its margins, an `auto`
/// margin counting for nothing, and no more than the `room` the parent leaves
/// it (CSS Sizing 3 §3.1.1). Without a containing block width, that room.
pub(super) fn stretch_fit_width(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
    room: f32,
) -> f32 {
    let Some(basis) = inputs.parent_size.width else {
        return room.max(0.0);
    };
    let margin = tree.physical_style(idx).margin();
    let side = |side: LengthPercentageAuto| side.resolve(Some(basis)).unwrap_or(0.0);
    (basis - side(margin.left) - side(margin.right))
        .min(room)
        .max(0.0)
}

/// The border-box height the box `idx` would stretch to in the room `inputs`
/// offers it: the definite height of its containing block less the margins it
/// takes off it (CSS Sizing 4 §5.1). A box that is not a block child keeps all
/// its margins out of the room; a block child keeps those its parent holds in
/// (see [`HeldMargins`]). Without a definite height the keyword has no effect.
/// An out-of-flow box stretches to the room between its insets instead.
/// Shared with table sizing before the grid's height limits are applied.
pub(in crate::layout) fn stretch_fit_height(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
) -> Option<f32> {
    // The room of an out-of-flow box is what its containing block leaves it
    // between its insets and margins, which is the space it is offered.
    if is_oof_box(tree, idx) {
        return match inputs.available_space.height {
            AvailableSpace::Definite(room) => Some(room.max(0.0)),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        };
    }
    let room = inputs.parent_size.height?;
    let margin = tree.physical_style(idx).margin();
    let side = |side: LengthPercentageAuto| side.resolve(inputs.parent_size.width).unwrap_or(0.0);
    let held = tree
        .context
        .held_margins
        .iter()
        .rev()
        .find(|held| held.node == idx)
        .map_or(Line::TRUE, |held| held.holds);
    let taken = if held.start { side(margin.top) } else { 0.0 }
        + if held.end { side(margin.bottom) } else { 0.0 };
    Some((room - taken).max(0.0))
}

/// The vertical margins of a block child that its parent holds in, which a
/// `stretch` height of the child leaves out of the room, while the parent lays
/// the child out. A margin that would escape the parent takes none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeldMargins {
    node: usize,
    holds: Line<bool>,
}

impl HeldMargins {
    pub(crate) fn new(node: usize, holds: Line<bool>) -> Self {
        Self { node, holds }
    }
}

/// Whether sizing needs the parent's held-margin context for a stretch
/// height limit, or for a table's own stretch height before its grid limits.
pub(crate) fn needs_stretch_height_room(tree: &LayoutSession<'_>, child: usize) -> bool {
    let limits = tree.nodes[child].content_limits;
    is_stretch(limits.min.height)
        || is_stretch(limits.max.height)
        || (is_table_content_limits(tree, child)
            && is_stretch(tree.physical_style(child).size().height))
}

/// Whether the box `child` states a `stretch` width as a `min-width` and as a
/// `max-width`. A parent that places the box beside floats finds the room it
/// stretches to there, so it applies these itself (see [`StretchWidths::clamp`]).
pub(crate) fn stretch_widths(tree: &LayoutSession<'_>, child: usize) -> StretchWidths {
    let limits = tree.nodes[child].content_limits;
    StretchWidths {
        min: is_stretch(limits.min.width),
        max: is_stretch(limits.max.width),
    }
}

/// Which of the `min-width` and `max-width` of a box are `stretch`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StretchWidths {
    min: bool,
    max: bool,
}

impl StretchWidths {
    pub(crate) fn any(self) -> bool {
        self.min || self.max
    }

    /// `width`, a border-box width that already respects the lengths stated as
    /// `min-width` and `max-width`, within the `stretch` ones: the border-box
    /// `minimum` and `maximum` that `bounds` hold, and the room `stretch` fills.
    /// The minimum wins over the maximum.
    pub(crate) fn clamp(self, width: f32, bounds: (Option<f32>, Option<f32>), stretch: f32) -> f32 {
        let minimum = if self.min {
            Some(bounds.0.map_or(stretch, |minimum| minimum.max(stretch)))
        } else {
            bounds.0
        };
        let maximum = if self.max {
            Some(bounds.1.map_or(stretch, |maximum| maximum.min(stretch)))
        } else {
            bounds.1
        };
        let width = maximum.map_or(width, |maximum| width.min(maximum));
        minimum.map_or(width, |minimum| width.max(minimum))
    }
}

#[cfg(test)]
mod tests;
