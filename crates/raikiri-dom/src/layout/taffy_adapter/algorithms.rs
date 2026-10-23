//! The library's layout algorithms, called with the protocol's types.

use super::OofSpan;
use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, AxisStaticPosition, LayoutInput, LayoutOutput, NodeId, OofLayoutResult,
    OofPosition, OofPositioningArea,
};
use raikiri_layout_geometry::{Edges, Line, Point, Size};
use smol_str::SmolStr;

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
    direction: taffy::Direction,
    claims: taffy::ContainingBlockClaims,
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
        direction,
        claims,
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
pub(crate) fn claims_position(claims: taffy::ContainingBlockClaims, position: OofPosition) -> bool {
    claims.for_position(position.into_taffy())
}

/// A calculator of the intrinsic width that floats contribute to a block, for a
/// block laid out in `available_width`.
#[inline]
pub(crate) fn float_intrinsic_width_calculator(
    available_width: AvailableSpace,
) -> taffy::FloatIntrinsicWidthCalculator {
    taffy::FloatIntrinsicWidthCalculator::new(available_width.into_taffy())
}

/// Place a floated box of the border-box size `floated_box` no higher than
/// `min_y` among the `floats`, and return its position.
///
/// `containing_block_insets` are how far the containing block is inset from
/// the left and the right edge of the formatting context.
#[inline]
pub(crate) fn place_floated_box(
    floats: &mut taffy::FloatContext,
    floated_box: Size<f32>,
    min_y: f32,
    containing_block_insets: [f32; 2],
    direction: taffy::FloatDirection,
    clear: taffy::Clear,
) -> Point<f32> {
    Point::from_taffy(floats.place_floated_box(
        floated_box.into_taffy(),
        min_y,
        containing_block_insets,
        direction,
        clear,
    ))
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
    alignment: taffy::AlignSelf,
    area: Line<f32>,
    axis_is_rtl: bool,
) -> AxisStaticPosition {
    AxisStaticPosition::from_taffy(taffy::AxisStaticPosition::from_alignment(
        alignment,
        area.into_taffy(),
        axis_is_rtl,
    ))
}

/// The grid area that the placement `row` and `column` of an out-of-flow box
/// resolves to in the grid whose tracks `info` describes, relative to the
/// border box of the container.
///
/// `padding_box` is the padding box of the container and `direction` its
/// inline direction.
#[inline]
pub(crate) fn resolve_absolute_grid_area(
    info: &taffy::DetailedGridInfo<SmolStr>,
    row: taffy::Line<taffy::GridPlacement<SmolStr>>,
    column: taffy::Line<taffy::GridPlacement<SmolStr>>,
    direction: taffy::Direction,
    padding_box: Edges<f32>,
) -> Edges<f32> {
    Edges::from_taffy(info.resolve_absolute_grid_area(
        row,
        column,
        direction,
        padding_box.into_taffy(),
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
    let computed = compute(tree, node, inputs);
    tree.cache_store(node, &inputs, computed.clone());
    computed
}
