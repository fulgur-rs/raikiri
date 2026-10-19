//! Intrinsic sizing keywords on `min-width`, `max-width`, `min-height` and
//! `max-height` (CSS Sizing 3 §4 and §5).
//!
//! Taffy's `min_size` and `max_size` hold lengths, percentages and `calc()`,
//! not the keywords. The bridge therefore leaves them `auto` and records each
//! keyword in [`ContentLimits`]; [`with_content_limits`] measures the box it
//! names when the box is laid out and puts the resulting lengths in the style
//! for the duration of that layout. Every algorithm that clamps the box to its
//! `min_size` and `max_size` then reads the sizes the keywords stand for.

use std::collections::HashMap;

use crate::layout::context::LayoutSession;
use crate::layout::ifc::boxes::{box_sizing_adjustment, box_sizing_height_adjustment};
use crate::node::FormattingContext;
use crate::taffy_impl::{fit_content_room, resolve_calc};
use raikiri_style::property::PositionValue;
use taffy::{
    AvailableSpace, BoxSizing, Cache, Dimension, ExpandedDimension, LayoutInput, LayoutOutput,
    LengthPercentageAuto, Line, MaybeResolve, NodeId, RequestedAxis, ResolveOrZero, RunMode, Size,
    SizingMode, Style,
};

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
        min: Size::auto(),
        max: Size::auto(),
    };

    fn has_width(&self) -> bool {
        !self.min.width.is_auto() || !self.max.width.is_auto()
    }

    fn has_height(&self) -> bool {
        !self.min.height.is_auto() || !self.max.height.is_auto()
    }
}

/// The sizes of `style` that a box's content measure must not read.
#[derive(Clone, Copy)]
struct SavedSizes {
    size: Size<Dimension>,
    min_size: Size<LengthPercentageAuto>,
    max_size: Size<LengthPercentageAuto>,
}

impl SavedSizes {
    fn of(style: &Style) -> Self {
        Self {
            size: style.size,
            min_size: style.min_size,
            max_size: style.max_size,
        }
    }

    fn restore(self, style: &mut Style) {
        style.size = self.size;
        style.min_size = self.min_size;
        style.max_size = self.max_size;
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
    crate::taffy_impl::is_table_box(tree.nodes[idx].display())
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

fn neutralize_all_sizes(style: &mut Style) {
    style.size = Size::auto();
    style.min_size = Size::auto();
    style.max_size = Size::auto();
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
    let result = f(tree);
    tree.context.content_limit_depth -= 1;
    result
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
        let saved = SavedSizes::of(tree.layout_node_with_style(idx).unwrap().style);
        // A parent hands its child a size already within the child's limits;
        // the keywords are the limits it could not read.
        let mut inputs = inputs;
        let mut width = Bounds::default();
        if limits.has_width() || (is_replaced_content(tree, idx) && limits.has_height()) {
            width = resolved_width_bounds(tree, node_id, &limits, &inputs);
            let adjustment =
                box_sizing_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
            width.install_width(tree.layout_style_for_key_mut(idx).unwrap(), adjustment);
            if let Some(known) = inputs.known_dimensions.width {
                let known = width.apply(known);
                inputs.known_dimensions.width = Some(known);
                // The room a parent offers a box it sizes is that size.
                inputs.available_space.width = AvailableSpace::Definite(known);
            }
        }
        if limits.has_height() {
            let height = resolved_height_bounds(tree, node_id, &limits, &inputs, width);
            let adjustment =
                box_sizing_height_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
            height.install_height(tree.layout_style_for_key_mut(idx).unwrap(), adjustment);
            inputs.known_dimensions.height = inputs
                .known_dimensions
                .height
                .map(|known| height.apply(known));
        }
        let output = dispatch(tree, node_id, inputs);
        saved.restore(tree.layout_style_for_key_mut(idx).unwrap());
        output
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
    if limits == ContentLimits::NONE
        || !is_oof_box(tree, idx)
        || is_table_content_limits(tree, idx)
        || tree.context.content_limit_probes.contains(&idx)
    {
        return dispatch(tree, node_id, inputs);
    }

    scoped(tree, |tree| {
        let cache = std::mem::replace(&mut tree.nodes[idx].cache, Cache::new());
        let saved = SavedSizes::of(tree.layout_node_with_style(idx).unwrap().style);
        let mut width = Bounds::default();
        if limits.has_width() || (is_replaced_content(tree, idx) && limits.has_height()) {
            width = resolved_width_bounds(tree, node_id, &limits, &inputs);
            let adjustment =
                box_sizing_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
            width.install_width(tree.layout_style_for_key_mut(idx).unwrap(), adjustment);
        }
        if let Some(known) = inputs.known_dimensions.width {
            let known =
                clamp_known_width_to_style(tree, idx, inputs.parent_size.width, width.apply(known));
            inputs.known_dimensions.width = Some(known);
            inputs.available_space.width = AvailableSpace::Definite(known);
        }
        if limits.has_height() {
            let height = resolved_height_bounds(tree, node_id, &limits, &inputs, width);
            let adjustment =
                box_sizing_height_adjustment(tree, idx, inputs.parent_size.width.unwrap_or(0.0));
            height.install_height(tree.layout_style_for_key_mut(idx).unwrap(), adjustment);
            inputs.known_dimensions.height = inputs.known_dimensions.height.map(|known| {
                clamp_known_height_to_style(
                    tree,
                    idx,
                    inputs.parent_size.height,
                    inputs.parent_size.width,
                    height.apply(known),
                )
            });
        }
        let output = dispatch(tree, node_id, inputs);
        saved.restore(tree.layout_style_for_key_mut(idx).unwrap());
        tree.nodes[idx].cache = cache;
        output
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
            .map(|&idx| {
                (
                    idx,
                    std::mem::replace(&mut tree.nodes[idx].cache, Cache::new()),
                )
            })
            .collect::<Vec<_>>();

        let suppress_nested_prepasses = candidate_ids.is_some();
        let saved_prepass_caches = suppress_nested_prepasses.then(|| {
            tree.nodes
                .iter()
                .map(|node| node.cache.clone())
                .collect::<Vec<_>>()
        });
        let constrained_inputs =
            capture_oof_layout_inputs(tree, &mut dispatch, suppress_nested_prepasses);
        let raw_targets = targets
            .iter()
            .copied()
            .filter(|idx| constrained_inputs.contains_key(idx))
            .filter(|&idx| {
                let style = tree.layout_node_with_style(idx).unwrap().style;
                !style.min_size.width.is_auto()
                    || !style.min_size.height.is_auto()
                    || !style.max_size.width.is_auto()
                    || !style.max_size.height.is_auto()
            })
            .collect::<Vec<_>>();
        let raw_inputs = if raw_targets.is_empty() {
            None
        } else {
            let saved_sizes = raw_targets
                .iter()
                .map(|&idx| {
                    let style = tree.layout_node_with_style(idx).unwrap().style;
                    (idx, SavedSizes::of(style))
                })
                .collect::<Vec<_>>();
            for &idx in &raw_targets {
                let style = tree.layout_style_for_key_mut(idx).unwrap();
                style.min_size = Size::auto();
                style.max_size = Size::auto();
            }
            let inputs = capture_oof_layout_inputs(tree, &mut dispatch, suppress_nested_prepasses);
            for (idx, saved) in saved_sizes.into_iter().rev() {
                saved.restore(tree.layout_style_for_key_mut(idx).unwrap());
            }
            Some(inputs)
        };

        let mut prepared = Vec::new();
        for &idx in &targets {
            let Some(positioned_inputs) = constrained_inputs.get(&idx) else {
                continue;
            };
            let limits = tree.nodes[idx].content_limits;
            let node_id = NodeId::from(idx);
            let width =
                if limits.has_width() || (is_replaced_content(tree, idx) && limits.has_height()) {
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
            let saved = {
                let style = tree.layout_style_for_key_mut(idx).unwrap();
                let saved = SavedSizes::of(style);
                width.install_width(style, width_adjustment);
                saved
            };

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
                let height = resolved_height_bounds(tree, node_id, &limits, &height_inputs, width);
                let height_adjustment = box_sizing_height_adjustment(
                    tree,
                    idx,
                    positioned_inputs.parent_size.width.unwrap_or(0.0),
                );
                height.install_height(
                    tree.layout_style_for_key_mut(idx).unwrap(),
                    height_adjustment,
                );
            }
            prepared.push((idx, saved));
        }

        if let Some(caches) = saved_prepass_caches {
            // Discard cache entries produced while descendant OOF wrappers
            // were bypassed for this candidate's preliminary measurements.
            debug_assert_eq!(caches.len(), tree.nodes.len());
            for (node, cache) in tree.nodes.iter_mut().zip(caches) {
                node.cache = cache;
            }
        }
        for &node_id in clear_cached_nodes {
            tree.nodes[usize::from(node_id)].cache.clear();
        }
        for &idx in &targets {
            tree.nodes[idx].cache = Cache::new();
            tree.context.content_limit_probes.push(idx);
        }
        let output = dispatch(tree);
        for _ in &targets {
            tree.context.content_limit_probes.pop();
        }
        for (idx, saved) in prepared.into_iter().rev() {
            saved.restore(tree.layout_style_for_key_mut(idx).unwrap());
        }
        for (idx, cache) in saved_caches.into_iter().rev() {
            tree.nodes[idx].cache = cache;
        }
        output
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
    let style = tree.layout_node_with_style(idx).unwrap().style;
    let adjustment = box_sizing_adjustment(tree, idx, parent_width.unwrap_or(0.0));
    let minimum = style
        .min_size
        .width
        .maybe_resolve(parent_width, resolve_calc)
        .map(|width| width + adjustment)
        .unwrap_or(0.0);
    let maximum = style
        .max_size
        .width
        .maybe_resolve(parent_width, resolve_calc)
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
    let style = tree.layout_node_with_style(idx).unwrap().style;
    let adjustment = box_sizing_height_adjustment(tree, idx, parent_width.unwrap_or(0.0));
    let minimum = style
        .min_size
        .height
        .maybe_resolve(parent_height, resolve_calc)
        .map(|height| height + adjustment)
        .unwrap_or(0.0);
    let maximum = style
        .max_size
        .height
        .maybe_resolve(parent_height, resolve_calc)
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
    tree.context.oof_content_limit_inputs.push(HashMap::new());
    if suppress_nested_prepasses {
        tree.context.oof_content_limit_prepass_depth += 1;
    }
    let _ = dispatch(tree);
    if suppress_nested_prepasses {
        tree.context.oof_content_limit_prepass_depth -= 1;
    }
    tree.context
        .oof_content_limit_inputs
        .pop()
        .expect("OOF layout input capture was pushed above")
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
    bounds.install_width(style, adjustment);
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
    let style = tree.layout_style_for_key_mut(node).unwrap();
    let saved = SavedSizes::of(style);
    bounds.install_width(style, adjustment);
    let result = f(tree);
    saved.restore(tree.layout_style_for_key_mut(node).unwrap());
    result
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
    let children = super::TaffyChildIter::new_for_session(tree, node_id).collect::<Vec<_>>();
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
        .map(|idx| {
            (
                idx,
                std::mem::replace(&mut tree.nodes[idx].cache, Cache::new()),
            )
        })
        .collect::<Vec<_>>();

    tree.context.grid_layout_info.remove(&usize::from(node_id));
    let _ = dispatch(tree, node_id, inputs);
    let area_bases = grid_item_area_bases(tree, node_id);
    for &(idx, _) in &saved_caches {
        tree.nodes[idx].cache = Cache::new();
    }

    let output = with_item_content_limits_for_parent_sizes(
        tree,
        node_id,
        inputs,
        |idx| area_bases.get(&idx).copied().unwrap_or(parent_size),
        dispatch,
    );
    for (idx, cache) in saved_caches.into_iter().rev() {
        tree.nodes[idx].cache = cache;
    }
    output
}

fn with_item_content_limits_for_parent_sizes(
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
    mut parent_size_for_item: impl FnMut(usize) -> Size<Option<f32>>,
    mut dispatch: impl FnMut(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    let has_item_limits = super::TaffyChildIter::new_for_session(tree, node_id).any(|child| {
        let idx = usize::from(child);
        applies_to_parent_item(tree, idx, &tree.nodes[idx].content_limits)
    });
    if !has_item_limits {
        return dispatch(tree, node_id, inputs);
    }
    let children = super::TaffyChildIter::new_for_session(tree, node_id).collect::<Vec<_>>();

    scoped(tree, |tree| {
        let mut prepared = Vec::new();
        for child in children {
            let idx = usize::from(child);
            let limits = tree.nodes[idx].content_limits;
            if !applies_to_parent_item(tree, idx, &limits) {
                continue;
            }
            let parent_size = parent_size_for_item(idx);
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

            let saved = SavedSizes::of(tree.layout_node_with_style(idx).unwrap().style);
            let width =
                if limits.has_width() || (is_replaced_content(tree, idx) && limits.has_height()) {
                    resolved_width_bounds(tree, child, &limits, &item_inputs)
                } else {
                    Bounds::default()
                };
            let width_adjustment =
                box_sizing_adjustment(tree, idx, parent_size.width.unwrap_or(0.0));
            width.install_width(
                tree.layout_style_for_key_mut(idx).unwrap(),
                width_adjustment,
            );

            if limits.has_height() {
                let height = resolved_height_bounds(tree, child, &limits, &item_inputs, width);
                let height_adjustment =
                    box_sizing_height_adjustment(tree, idx, parent_size.width.unwrap_or(0.0));
                height.install_height(
                    tree.layout_style_for_key_mut(idx).unwrap(),
                    height_adjustment,
                );
            }

            let cache = std::mem::replace(&mut tree.nodes[idx].cache, Cache::new());
            tree.context.content_limit_probes.push(idx);
            prepared.push((idx, saved, cache));
        }

        let output = dispatch(tree, node_id, inputs);
        for (idx, saved, cache) in prepared.into_iter().rev() {
            tree.context.content_limit_probes.pop();
            saved.restore(tree.layout_style_for_key_mut(idx).unwrap());
            tree.nodes[idx].cache = cache;
        }
        output
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
        ) && dimension.into_raw().uses_percentage()
    })
}

fn grid_item_area_bases(
    tree: &LayoutSession<'_>,
    node_id: NodeId,
) -> std::collections::HashMap<usize, Size<Option<f32>>> {
    let parent = usize::from(node_id);
    let Some(taffy::DetailedLayoutInfo::Grid(grid)) = tree.context.grid_layout_info.get(&parent)
    else {
        return std::collections::HashMap::new();
    };
    let children = super::TaffyChildIter::new_for_session(tree, node_id)
        .filter(|&child| {
            let style = tree
                .layout_node_with_style(usize::from(child))
                .unwrap()
                .style;
            style.display != taffy::Display::None && style.position != taffy::Position::Absolute
        })
        .collect::<Vec<_>>();
    children
        .into_iter()
        .enumerate()
        .filter_map(|(item_index, child)| {
            let (_, area) = grid.item_grid_area(item_index)?;
            Some((
                usize::from(child),
                Size {
                    width: Some(area.width),
                    height: Some(area.height),
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
    /// `size` within the bounds, the minimum winning over the maximum.
    fn apply(self, size: f32) -> f32 {
        let size = self.max.map_or(size, |max| size.min(max));
        self.min.map_or(size, |min| size.max(min))
    }

    /// The box-sizing box a style names for a border-box size, given what
    /// `box-sizing` leaves out of it.
    fn length(border_box: f32, adjustment: f32) -> LengthPercentageAuto {
        LengthPercentageAuto::length((border_box - adjustment).max(0.0))
    }

    fn install_width(self, style: &mut Style, adjustment: f32) {
        if let Some(min) = self.min {
            style.min_size.width = Self::length(min, adjustment);
        }
        if let Some(max) = self.max {
            style.max_size.width = Self::length(max, adjustment);
        }
    }

    fn install_height(self, style: &mut Style, adjustment: f32) {
        if let Some(min) = self.min {
            style.min_size.height = Self::length(min, adjustment);
        }
        if let Some(max) = self.max {
            style.max_size.height = Self::length(max, adjustment);
        }
    }
}

/// What a measure of a box is keyed by: the box, the axis measured, and
/// everything the layout it runs reads besides the box's contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MeasureKey {
    node: usize,
    width: bool,
    /// The basis a flex item's percentage edges were resolved against.
    edge_basis: Option<u32>,
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
    ) -> Self {
        let bits = |size: Option<f32>| size.map(f32::to_bits);
        let space = |space: AvailableSpace| match space {
            AvailableSpace::Definite(size) => (0, size.to_bits()),
            AvailableSpace::MinContent => (1, 0),
            AvailableSpace::MaxContent => (2, 0),
        };
        Self {
            node,
            width,
            edge_basis: bits(tree.nodes[node].flex_edge_basis),
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
    neutral: impl FnOnce(&mut Style),
) -> f32 {
    if let Some(&size) = tree.context.content_limit_measures.get(&key) {
        return size;
    }
    let idx = usize::from(node_id);
    let saved = {
        let style = tree.layout_style_for_key_mut(idx).unwrap();
        let saved = SavedSizes::of(style);
        neutral(style);
        saved
    };
    tree.context.content_limit_probes.push(idx);
    let cache = std::mem::replace(&mut tree.nodes[idx].cache, Cache::new());
    let output = tree.compute_child_layout_with_block_ctx(node_id, inputs, None);
    tree.nodes[idx].cache = cache;
    tree.context.content_limit_probes.pop();
    saved.restore(tree.layout_style_for_key_mut(idx).unwrap());
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
    let Some(mut style) = tree
        .layout_node_with_style(idx)
        .map(|node| node.style.clone())
    else {
        return (Bounds::default(), Bounds::default()); // cov:ignore: every source node retains a layout style for its lifetime; this guards malformed arena state.
    };
    if style.aspect_ratio.is_none() {
        style.aspect_ratio = super::leaf_intrinsic_ratio(&tree.nodes[idx]);
    }
    let Some(bounds) = super::replaced_size::keyword_bounds(&style, inputs, natural, *limits)
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
                let key = MeasureKey::new(tree, idx, true, &along, Bounds::default());
                probe(tree, node_id, key, along, |style| {
                    if flex_grid_container {
                        neutralize_all_sizes(style);
                    } else {
                        style.size.width = Dimension::auto();
                        style.min_size.width = LengthPercentageAuto::auto();
                        style.max_size.width = LengthPercentageAuto::auto();
                    }
                })
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
    let key = MeasureKey::new(tree, usize::from(node_id), false, &along, width);
    let percentage_height_basis = (is_oof_box(tree, idx)
        || is_flex_grid_container(tree, idx)
        || is_flex_grid_item(tree, idx))
    .then(|| content_percentage_height_basis(tree, idx, inputs))
    .flatten();
    if let Some(height) = percentage_height_basis {
        tree.context
            .content_limit_percentage_height_bases
            .push((idx, height));
    }
    let height = probe(tree, node_id, key, along, |style| {
        if flex_grid_container {
            let preferred_width = style.size.width;
            neutralize_all_sizes(style);
            style.size.width = preferred_width;
            width.install_width(style, width_adjustment);
        } else {
            style.size.height = Dimension::auto();
            style.min_size.height = LengthPercentageAuto::auto();
            style.max_size.height = LengthPercentageAuto::auto();
        }
    });
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
    let style = tree.layout_node_with_style(idx)?.style;
    let parent_width = inputs.parent_size.width.unwrap_or(0.0);
    let edges = (style
        .padding
        .resolve_or_zero(Some(parent_width), resolve_calc)
        + style
            .border
            .resolve_or_zero(Some(parent_width), resolve_calc))
    .sum_axes()
    .height;
    if let Some(height) = inputs.known_dimensions.height {
        return Some((height - edges).max(0.0));
    }

    let box_sizing_adjustment = if style.box_sizing == BoxSizing::ContentBox {
        edges
    } else {
        0.0
    };
    let preferred = style
        .size
        .height
        .maybe_resolve(inputs.parent_size.height, resolve_calc)?
        + box_sizing_adjustment;
    let minimum = style
        .min_size
        .height
        .maybe_resolve(inputs.parent_size.height, resolve_calc)
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
    let margin = &tree.layout_node_with_style(idx).unwrap().style.margin;
    let side =
        |side: LengthPercentageAuto| side.resolve_to_option(basis, resolve_calc).unwrap_or(0.0);
    (basis - side(margin.left) - side(margin.right))
        .min(room)
        .max(0.0)
}

#[cfg(test)]
mod tests;
