//! The library's layout algorithms, called with the protocol's types.

use super::OofSpan;
use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticPosition, LayoutInput, LayoutOutput, NodeId, OofLayoutResult,
    OofPosition, OofPositioningArea,
};
use raikiri_layout_geometry::{Edges, Line, Size};
use raikiri_layout_style::ToTaffy;
use raikiri_layout_style::values::{
    AlignContent, AlignSelf, ContainingBlockClaims, Direction, Position,
};

/// Lay out a flex container.
#[inline]
pub(crate) fn compute_flexbox_layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    LayoutOutput::from_taffy(taffy::compute_flexbox_layout(
        tree,
        node.into_taffy(),
        inputs.into_taffy(),
    ))
}

/// Lay out a grid container.
#[inline]
pub(crate) fn compute_grid_layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    LayoutOutput::from_taffy(taffy::compute_grid_layout(
        tree,
        node.into_taffy(),
        inputs.into_taffy(),
    ))
}

/// Lay out a leaf whose content is measured by `measure`, which is given the
/// known dimensions and the available space.
#[inline]
pub(crate) fn compute_leaf_layout(
    inputs: LayoutInput,
    style: &impl taffy::CoreStyle,
    resolve_calc_value: impl Fn(*const (), f32) -> f32,
    measure: impl FnOnce(Size<Option<f32>>, Size<AvailableSpace>) -> Size<f32>,
) -> LayoutOutput {
    LayoutOutput::from_taffy(taffy::compute_leaf_layout(
        inputs.into_taffy(),
        style,
        resolve_calc_value,
        |known_dimensions, available_space| {
            measure(
                Size::from_taffy(known_dimensions),
                Size::from_taffy(available_space),
            )
            .into_taffy()
        },
    ))
}

/// Lay out the root of the tree into `available_space`, then position the
/// out-of-flow boxes that no other box is the containing block of.
#[inline]
pub(crate) fn compute_root_layout(
    tree: &mut LayoutSession<'_>,
    root: NodeId,
    available_space: Size<AvailableSpace>,
) {
    taffy::compute_root_layout(tree, root.into_taffy(), available_space.into_taffy());
}

/// Lay out the out-of-flow `candidates` against the positioning `area`, for the
/// node `owner` that owns the geometry.
///
/// `direction` and `claims` describe the containing block the candidates are
/// positioned against: its direction and the positions it is the containing
/// block of.
#[inline]
pub(crate) fn compute_oof_layout_for_area(
    tree: &mut LayoutSession<'_>,
    owner: NodeId,
    candidates: OofSpan<'_>,
    area: OofPositioningArea,
    direction: Direction,
    claims: ContainingBlockClaims,
) -> OofLayoutResult {
    let taffy::OofLayoutResult {
        hoisted,
        unclaimed,
        scrollable_overflow_rect,
    } = taffy::compute::compute_oof_layout_for_area(
        tree,
        owner.into_taffy(),
        candidates.as_taffy(),
        area.into_taffy(),
        direction.to_taffy(),
        claims.to_taffy(),
    );
    OofLayoutResult {
        hoisted: hoisted.into_iter().map(NodeId::from_taffy).collect(),
        unclaimed: FromTaffy::from_taffy(unclaimed),
        scrollable_overflow_rect: Edges::from_taffy(scrollable_overflow_rect),
    }
}

/// Whether a containing block that makes the `claims` is the containing block
/// of a box with the `position`.
#[inline]
pub(crate) fn claims_position(claims: ContainingBlockClaims, position: OofPosition) -> bool {
    claims.for_position(match position {
        OofPosition::Absolute => Position::Absolute,
        OofPosition::Fixed => Position::Fixed,
    })
}

/// The offset along the block axis that `align-content` gives the in-flow
/// content of a block container, from the `free_space` between the content
/// box and the content. A negative offset moves the content up.
///
/// `align_content` must not be `normal`: the container's own default has to
/// be resolved before the offset is asked for.
#[inline]
pub(crate) fn compute_block_align_content_offset(
    align_content: AlignContent,
    free_space: f32,
) -> f32 {
    taffy::compute_block_align_content_offset(align_content.to_taffy(), free_space)
}

/// The static position in one axis of an out-of-flow box whose alignment is
/// `alignment` within `area`.
///
/// `alignment` must already be resolved: an `auto` alignment replaced by the
/// container's, and `self-start` and `self-end` resolved against its
/// direction. `axis_is_rtl` is whether the axis is the inline axis of a
/// right-to-left container.
#[inline]
pub(crate) fn static_position_from_alignment(
    alignment: AlignSelf,
    area: Line<f32>,
    axis_is_rtl: bool,
) -> AxisStaticPosition {
    AxisStaticPosition::from_taffy(taffy::AxisStaticPosition::from_alignment(
        alignment.to_taffy(),
        area.into_taffy(),
        axis_is_rtl,
    ))
}

/// The result of `compute` for `inputs`, from the cache of `node` if it holds
/// one for these inputs, and stored in it otherwise.
#[inline]
pub(crate) fn compute_cached_layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    compute: impl FnOnce(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    if let Some(cached) = tree.cache_get(node, &inputs) {
        return cached;
    }
    let deferred_before = tree.context.columns_deferred_layouts;
    let computed = compute(tree, node, inputs);
    // A layout that left a column container for the layout that follows is not
    // kept, and it drops what the caches of the boxes inside held: that was
    // laid out before they were laid out again, so a later call with the same
    // inputs would take its size and never settle the container.
    if tree.context.columns_deferred_layouts == deferred_before {
        tree.cache_store(node, &inputs, computed.clone());
    } else {
        tree.cache_clear_subtree(node);
    }
    computed
}
