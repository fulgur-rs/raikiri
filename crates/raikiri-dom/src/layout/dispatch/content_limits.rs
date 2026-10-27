//! Intrinsic sizing keywords on `min-width`, `max-width`, `min-height` and
//! `max-height` (CSS Sizing 3 §4 and §5).
//!
//! Taffy's `min_size` and `max_size` hold lengths, percentages and `calc()`,
//! not the keywords. The bridge therefore leaves them `auto` and records each
//! keyword in [`ContentLimits`]; [`with_content_limits`] measures the box it
//! names when the box is laid out and puts the resulting lengths in the style
//! for the duration of that layout. Every algorithm that clamps the box to its
//! `min_size` and `max_size` then reads the sizes the keywords stand for.

use raikiri_layout_style::values::{
    BoxSizing, Dimension, Display, ExpandedDimension, LengthIdentity, LengthPercentage,
    LengthPercentageAuto, MaybeResolve, Position, ResolveOrZero,
};
use std::collections::HashMap;

use crate::layout::context::LayoutSession;
use crate::layout::dispatch::fit_content_room;
use crate::layout::ifc::boxes::{box_sizing_adjustment, box_sizing_height_adjustment};
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode, SizingMode,
};
use crate::layout::taffy_adapter::LayoutCache;
use crate::node::FormattingContext;
use raikiri_layout_geometry::{Line, Size};
use raikiri_layout_style::BoxStyle as Style;
use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
use raikiri_style::property::PositionValue;

/// The `min-*` and `max-*` sizes of a box that are intrinsic sizing keywords
/// (`min-content`, `max-content`, `fit-content`, `fit-content(<length-percentage>)`),
/// as the keyword [`Dimension`]s the `width` bridge produces. A size that is no
/// such keyword is `auto` here; Taffy's style carries every other size.
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

/// Run `f` with the `properties` of the box `node` standing for `auto` or no
/// limit, as they do while the size they refer to is not known.
pub(crate) fn with_neutral_sizes<R>(
    tree: &mut LayoutSession<'_>,
    node: usize,
    properties: &[LengthProperty],
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    with_size_overrides(tree, node, neutral_size_overrides(properties), f)
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
    let limits = tree.nodes[idx].content_limits;
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

/// Resolve percentage fit-content limits on grid items against the grid area
/// sizes from a preliminary track layout, then lay out the grid with those
/// resolved limits installed.
pub(super) fn with_grid_item_content_limits(
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
        applies_to_parent_item(tree, idx, &limits) && has_percentage_fit_content(&limits)
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

fn has_percentage_fit_content(limits: &ContentLimits) -> bool {
    [
        limits.min.width,
        limits.max.width,
        limits.min.height,
        limits.max.height,
    ]
    .into_iter()
    .any(|dimension| {
        matches!(
            dimension.expand(),
            ExpandedDimension::FitContentPercent(_) | ExpandedDimension::FitContentCalc(_)
        ) && dimension.uses_percentage()
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

/// The border-box widths the keyword `min-width` and `max-width` of `node_id`
/// stand for in the room `inputs` offers it.
fn replaced_keyword_bounds(
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
/// `node_id` stand for: along the block axis every keyword is the automatic
/// height of the box at the width `inputs` give it, within the `width` bounds
/// already installed in its style (CSS Sizing 3 §5.1).
fn height_bounds(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    limits: &ContentLimits,
    inputs: &LayoutInput,
    width: Bounds,
) -> Bounds {
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
    Bounds {
        min: (!limits.min.height.is_auto()).then_some(height),
        max: (!limits.max.height.is_auto()).then_some(height),
    }
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

/// The border-box width the box `idx` would stretch to in the room `inputs`
/// offers it: the width of its containing block less its margins, an `auto`
/// margin counting for nothing, and no more than the `room` the parent leaves
/// it (CSS Sizing 3 §3.1.1). Without a containing block width, that room.
fn stretch_fit_width(tree: &LayoutSession<'_>, idx: usize, inputs: &LayoutInput, room: f32) -> f32 {
    let Some(basis) = inputs.parent_size.width else {
        return room.max(0.0);
    };
    let margin = tree.physical_style(idx).margin();
    let side = |side: LengthPercentageAuto| side.resolve(Some(basis)).unwrap_or(0.0);
    (basis - side(margin.left) - side(margin.right))
        .min(room)
        .max(0.0)
}

#[cfg(test)]
mod tests;
