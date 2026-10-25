//! What the corrections of the bridge's second pass read back of settled boxes.
//!
//! The second pass of the bridge corrects the alignment and aspect ratio of
//! flex and grid items by what the item and its container say. Both come from
//! computed values, apart from the few facts that an earlier correction of the
//! first pass may have replaced; [`Settled`] keeps those, so no style has to be
//! stored for a box to be asked about.

use super::*;
use raikiri_layout_style::LengthProperty;
use raikiri_layout_style::values::{
    AlignItems, AlignItemsKeyword, AlignmentSafety, Display, Position, baseline_set_of,
};

/// The facts of a box that a correction replaces before another reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Settled {
    /// Whether the box has a preferred aspect ratio.
    pub(super) has_aspect_ratio: bool,
    /// The `align-self` of the box, `None` for `auto`.
    pub(super) align_self: Option<AlignItems>,
    /// The `justify-self` of the box, `None` for `auto`.
    pub(super) justify_self: Option<AlignItems>,
}

impl Settled {
    /// The alignments the box has by its own computed values, before any
    /// correction.
    pub(super) fn of(cv: &ComputedValues) -> Self {
        Self {
            has_aspect_ratio: false,
            align_self: own_alignment(cv, cv.align_self),
            justify_self: own_alignment(cv, cv.justify_self),
        }
    }
}

/// The alignment that `align-self` or `justify-self` (`value`) gives a box
/// with the computed values `cv`.
///
/// An absolutely positioned box takes no part in baseline alignment, so `last
/// baseline` falls back to `safe self-end` and `baseline` to `safe start` (CSS
/// Box Alignment 3 §9.1). Taffy places `baseline` at the start edge already,
/// but keeps the default overflow alignment.
fn own_alignment(cv: &ComputedValues, value: AlignSelfValue) -> Option<AlignItems> {
    let abspos = matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed);
    match value {
        AlignSelfValue::Value(SelfAlignmentValue::LastBaseline) if abspos => {
            Some(AlignItems::SAFE_SELF_END)
        }
        AlignSelfValue::Value(SelfAlignmentValue::Baseline) if abspos => Some(AlignItems {
            keyword: AlignItemsKeyword::Baseline,
            safety: AlignmentSafety::Safe,
        }),
        value => self_alignment_or_auto_to_taffy(value),
    }
}

/// The `align-items` or `justify-items` of a container with these values.
fn container_alignment(value: SelfAlignmentValue) -> AlignItems {
    self_alignment_to_taffy(value).unwrap_or(AlignItems::NORMAL)
}

/// The alignments that replace the `auto` ones of an absolutely positioned
/// box whose container (`parent`) aligns its items to the last baseline.
///
/// Such a box takes the container's `align-items` / `justify-items` for its
/// static position, where both insets of the axis are `auto`. When that is
/// `last baseline`, the box falls back to `safe self-end` like an explicit
/// `last baseline` does ([`own_alignment`]); taffy alone would inherit the
/// container's plain baseline. With an inset specified, `auto` behaves as
/// `normal` and the box stays at the start.
///
/// Layout reads `align-self` along the physical vertical axis and
/// `justify-self` along the horizontal one, except for a box in a container on
/// vertical axes whose containing block is on vertical axes as well: it reads
/// them the other way round there, so `align-self` is the block axis of the
/// container, the physical horizontal one. `containing_block_vertical` says
/// whether the containing block is on vertical axes, and is only asked for when
/// the container is and an alignment would be replaced.
// spec: https://www.w3.org/TR/css-align-3/#valdef-justify-self-auto
pub(super) fn inherited_last_baseline(
    cv: &ComputedValues,
    parent: &ComputedValues,
    parent_vertical: bool,
    containing_block_vertical: impl FnOnce() -> bool,
) -> (Option<AlignItems>, Option<AlignItems>) {
    let mut overrides = (None, None);
    if !matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed) {
        return overrides;
    }
    let block = matches!(cv.align_self, AlignSelfValue::Auto)
        && parent.align_items == SelfAlignmentValue::LastBaseline;
    let inline = matches!(cv.justify_self, AlignSelfValue::Auto)
        && parent.justify_items == SelfAlignmentValue::LastBaseline;
    if !(block || inline) {
        return overrides;
    }
    let auto = |value: ComputedLengthPercentageOrAuto| {
        matches!(value, ComputedLengthPercentageOrAuto::Auto)
    };
    let (vertical_insets_auto, horizontal_insets_auto) = (
        auto(cv.top) && auto(cv.bottom),
        auto(cv.left) && auto(cv.right),
    );
    let (block_insets_auto, inline_insets_auto) = if parent_vertical && containing_block_vertical()
    {
        (horizontal_insets_auto, vertical_insets_auto)
    } else {
        (vertical_insets_auto, horizontal_insets_auto)
    };
    if block && block_insets_auto {
        overrides.0 = Some(AlignItems::SAFE_SELF_END);
    }
    if inline && inline_insets_auto {
        overrides.1 = Some(AlignItems::SAFE_SELF_END);
    }
    overrides
}

/// The computed values of `parent`, the box that holds an item in its own
/// layout, when that box is a container laid out by `display`.
///
/// Boxes the document does not hold (the initial containing block and the
/// anonymous table boxes) are neither flex nor grid containers.
fn container_laid_out_as<'a>(
    doc: &LayoutSession<'_>,
    cascade: &'a CascadeResult,
    parent: usize,
    display: Display,
) -> Option<&'a ComputedValues> {
    (parent < doc.nodes.len() && doc.nodes[parent].layout_display() == display)
        .then(|| &cascade.computed[parent])
}

/// Whether the cross axis of the flex container `parent`, whose computed
/// values are `flex`, runs from top to bottom: the main axis of a row is the
/// inline axis of the container's writing mode and that of a column its block
/// axis, and a vertical writing mode turns both.
fn flex_cross_axis_is_vertical(
    cascade: &CascadeResult,
    parent: usize,
    flex: &ComputedValues,
) -> bool {
    let row = matches!(
        flex.flex_direction,
        FlexDirectionValue::Row | FlexDirectionValue::RowReverse
    );
    row != lays_out_vertically(cascade, parent)
}

/// Whether the width `cv` of an item is a sizing keyword although the
/// container's alignment would stretch it.
///
/// Only an `auto` cross size stretches (CSS Flexbox 1 §9.4 step 11). Taffy
/// settles that when it places the item, yet it measures the item's height at
/// the stretched width first, so a keyword width that is narrower or wider than
/// the container leaves the height of another line count. Aligning the item at
/// the start of the cross axis measures it at its own width. Taffy places a
/// stretched item that it does not stretch exactly like a `flex-start` one, so
/// nothing else moves.
///
/// The width of a replaced box is `auto` to Taffy ([`replaced_keyword_width`]),
/// so the item is aligned at the start whenever the keyword would be stretched.
/// This applies to an item of a flex container whose cross axis is the
/// physical width.
pub(super) fn is_stretched_by_keyword_width(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    cv: &ComputedValues,
    settled: &[Settled],
) -> bool {
    let keyword_width =
        is_sizing_keyword(cv.width) || replaced_keyword_width(doc.document, cascade, idx);
    if !keyword_width || lays_out_vertically(cascade, idx) {
        return false;
    }
    let Some((parent, container)) = doc.taffy_parent_of(idx).and_then(|parent| {
        container_laid_out_as(doc, cascade, parent, Display::Flex)
            .map(|container| (parent, container))
    }) else {
        return false;
    };
    !flex_cross_axis_is_vertical(cascade, parent, container)
        && matches!(
            settled[idx]
                .align_self
                .unwrap_or_else(|| container_alignment(container.align_items))
                .keyword,
            AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal
        )
}

/// Whether grid item `idx` is a replaced box with a keyword width that its
/// `justify-self` would stretch ([`replaced_keyword_width`]). Aligning it at
/// the start of the row axis keeps the width it sizes itself to.
pub(super) fn is_stretched_by_replaced_keyword_width_in_grid(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    settled: &[Settled],
) -> bool {
    replaced_keyword_width(doc.document, cascade, idx)
        && doc.taffy_parent_of(idx).is_some_and(|parent| {
            container_laid_out_as(doc, cascade, parent, Display::Grid).is_some_and(|container| {
                settled[idx]
                    .justify_self
                    .unwrap_or_else(|| container_alignment(container.justify_items))
                    .keyword
                    == AlignItemsKeyword::Stretch
            })
        })
}

/// The alignment that replaces a stretch of item `idx` when its `height` is
/// `min-content`, `max-content` or `fit-content` and it sits in a grid or in a
/// flex container whose cross axis is the physical height.
///
/// Only an `auto` height is stretched (CSS Box Alignment 3 §6.1), but the cascade
/// computes the keyword to `auto` ([`ComputedValues::height_keyword`] is what is
/// left of it), so Taffy would stretch the box. It sizes itself by its content as
/// it does for `auto`, and is aligned at the start of the block axis instead,
/// which is where a stretch that does not apply falls back to: `start` in a grid
/// and `flex-start` in a flex container, whose cross-start edge `wrap-reverse`
/// moves. `fit-content(<length>)` stays out, as browsers treat it like `auto`.
/// So do an absolutely positioned box, whose size comes from its insets, and an
/// element without a box.
pub(super) fn unstretched_keyword_height_alignment(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    settled: &[Settled],
) -> Option<AlignItems> {
    let values = &cascade.computed[idx];
    if values.height_keyword.is_none()
        || matches!(values.display, DisplayValue::Contents | DisplayValue::None)
    {
        return None;
    }
    if doc.nodes[idx].layout_position() == Position::Absolute {
        return None;
    }
    let parent = doc.taffy_parent_of(idx)?;
    let (container, fallback) =
        if let Some(grid) = container_laid_out_as(doc, cascade, parent, Display::Grid) {
            (grid, AlignItems::START)
        } else {
            let flex = container_laid_out_as(doc, cascade, parent, Display::Flex)?;
            if !flex_cross_axis_is_vertical(cascade, parent, flex) {
                return None;
            }
            (flex, AlignItems::FLEX_START)
        };
    matches!(
        settled[idx]
            .align_self
            .unwrap_or_else(|| container_alignment(container.align_items))
            .keyword,
        AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal
    )
    .then_some(fallback)
}

/// The containing block of the absolutely (`fixed` false) or fixed positioned
/// box `idx`: the nearest ancestor that holds this kind of box. The root element
/// stands in for the initial containing block, and for itself when it is the
/// box.
pub(super) fn positioning_container(doc: &LayoutSession<'_>, idx: usize, fixed: bool) -> usize {
    doc.ancestors(idx)
        .find(|&ancestor| {
            let node = &doc.nodes[ancestor];
            if fixed {
                node.claims_fixed_oof
            } else {
                node.claims_absolute_oof
            }
        })
        .or_else(|| {
            doc.ancestors(idx)
                .filter(|&ancestor| doc.nodes[ancestor].kind() == NodeKind::Element)
                .last()
        })
        .unwrap_or(idx)
}

/// What an out-of-flow box with a content keyword `height` needs of its
/// vertical insets, which Taffy would otherwise stretch it between; see
/// [`keyword_height_insets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeywordHeightInsets {
    /// The box is overconstrained: this inset is dropped, and the box takes the
    /// height of its content from the inset on the opposite side. Its alignment
    /// stays as it is.
    Ignore(LengthProperty),
    /// An `auto` margin takes the free space between the insets, so both stay.
    /// An alignment of the vertical axis that would stretch the box
    /// ([`stretches_between_insets`]) becomes `start`, so that Taffy sizes the
    /// box by its content. `justify` names `justify-self` as the property of
    /// that axis, and `align-self` otherwise.
    FitContent { justify: bool },
}

/// Whether Taffy stretches an out-of-flow box between its insets on the axis
/// that `alignment`, the settled `align-self` or `justify-self` of the box, is
/// for: `normal` and `stretch`, and the `auto` that is left of an unsettled one.
pub(super) fn stretches_between_insets(alignment: Option<AlignItems>) -> bool {
    alignment.is_none_or(|alignment| {
        matches!(
            alignment.keyword,
            AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal
        )
    })
}

/// How out-of-flow box `idx`, whose `height` is `min-content`, `max-content` or
/// `fit-content`, sits between its `top` and `bottom`. The cascade computes
/// the keyword to `auto` ([`ComputedValues::height_keyword`] is what is left of
/// it).
///
/// A box with a `top`, a `bottom` and a `height` that are all not `auto` is
/// overconstrained, and the inset at the end of the inline axis of its
/// containing block is ignored (CSS 2.1 §10.6.4, CSS Position 3): `bottom`
/// unless the containing block lays out vertical lines from bottom to top. Taffy
/// sees an `auto` height instead, which `top` and `bottom` stretch the box
/// between, so the end inset is dropped from its style and the box takes the
/// height of its content from the start inset. `fit-content(<length>)` stays
/// out, as browsers treat it like `auto`, and so does a replaced box, whose
/// natural size Taffy does not stretch.
///
/// An `auto` margin on either side takes the free space between the insets
/// instead (centered with both margins `auto`), which the box is not
/// overconstrained by: it keeps both insets and is sized by its content, with
/// the alignment of the vertical axis replaced by one that is no stretch. The
/// alignment does not matter for where the box goes.
///
/// Without one, a box that aligns itself to the middle or to the end of the
/// vertical axis is placed within the area between both insets, so it keeps them,
/// and it is not stretched: `align-self` for a containing block on horizontal
/// axes and `justify-self` for one on vertical axes, whose inline axis is the
/// vertical one. `last baseline` falls back to the end for a box that is out of
/// flow. The other alignments put the box at the start edge, and the alignment of
/// the container does not apply to it.
// spec: https://www.w3.org/TR/CSS21/visudet.html#abs-non-replaced-height
pub(super) fn keyword_height_insets(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
) -> Option<KeywordHeightInsets> {
    use crate::layout::page::inset_is_auto;
    let values = &cascade.computed[idx];
    let fixed = values.position == PositionValue::Fixed;
    if !(fixed || values.position == PositionValue::Absolute)
        || values.height_keyword.is_none()
        || inset_is_auto(values.top)
        || inset_is_auto(values.bottom)
        || matches!(values.display, DisplayValue::Contents | DisplayValue::None)
        || crate::paint_rules::is_replaced_element(doc.document, idx)
    {
        return None;
    }
    let containing_block = positioning_container(doc, idx, fixed);
    // `sideways-lr` runs its inline axis from bottom to top, `direction: rtl`
    // reverses it, and `text-orientation: upright` does not change either.
    let block = &cascade.computed[containing_block];
    let vertical_axis_is_inline = uses_vertical_axes(cascade, containing_block);
    let auto = |margin| matches!(margin, ComputedLengthPercentageOrAuto::Auto);
    if auto(values.margin.top) || auto(values.margin.bottom) {
        // Taffy swaps the two alignments only where the container that holds
        // the box is on vertical axes as well (`get_oof_item_style`), which is
        // the property that stretches the box here.
        return Some(KeywordHeightInsets::FitContent {
            justify: vertical_axis_is_inline
                && doc
                    .taffy_parent_of(idx)
                    .is_some_and(|parent| uses_vertical_axes(cascade, parent)),
        });
    }
    let alignment = if vertical_axis_is_inline {
        values.justify_self
    } else {
        values.align_self
    };
    if let AlignSelfValue::Value(value) = alignment
        && (matches!(
            value,
            SelfAlignmentValue::LastBaseline | SelfAlignmentValue::AnchorCenter
        ) || matches!(
            value.position(),
            Some(
                SelfPosition::Center
                    | SelfPosition::End
                    | SelfPosition::FlexEnd
                    | SelfPosition::SelfEnd
            )
        ))
    {
        return None;
    }
    let inline_starts_at_bottom = vertical_axis_is_inline
        && (block.direction == Direction::Rtl)
            != (block.cssom_writing_mode == WritingMode::SidewaysLr);
    Some(KeywordHeightInsets::Ignore(if inline_starts_at_bottom {
        LengthProperty::Top
    } else {
        LengthProperty::Bottom
    }))
}

/// The alignment taffy gets for in-flow item `idx` of a vertical flex row or
/// grid that aligns by a baseline against the block direction of its
/// container: `start` in a grid and `flex-start` in a flex container.
///
/// Items share a baseline only when their block flow runs in the same
/// direction (CSS Box Alignment 3 §9.3), and taffy keeps one group. The
/// container's own group stays with taffy, so an item that runs against the
/// block direction is placed at the start of the block axis, where its size is
/// all it adds to the line or the row, and the container moves it to its group
/// of its own afterwards. A flex container that wraps is left out: it has a
/// group for each line, which the container cannot find after the fact. So is
/// a box with an auto margin in the block axis, which the margin takes the free
/// space for instead of aligning the box.
// spec: https://www.w3.org/TR/css-align-3/#baseline-sharing-group
pub(super) fn opposed_baseline_alignment(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
) -> Option<AlignItems> {
    let values = &cascade.computed[idx];
    // Most boxes are horizontal, so that is the first thing to rule out.
    if !uses_vertical_axes(cascade, idx)
        || matches!(values.display, DisplayValue::Contents | DisplayValue::None)
        || doc.nodes[idx].layout_position() == Position::Absolute
        || matches!(values.margin.left, ComputedLengthPercentageOrAuto::Auto)
        || matches!(values.margin.right, ComputedLengthPercentageOrAuto::Auto)
    {
        return None;
    }
    let parent = doc.taffy_parent_of(idx)?;
    let (container, fallback) =
        if let Some(grid) = container_laid_out_as(doc, cascade, parent, Display::Grid) {
            (grid, AlignItems::START)
        } else {
            let flex = container_laid_out_as(doc, cascade, parent, Display::Flex)?;
            if !matches!(
                flex.flex_direction,
                FlexDirectionValue::Row | FlexDirectionValue::RowReverse
            ) || flex.flex_wrap != FlexWrapValue::NoWrap
            {
                return None;
            }
            (flex, AlignItems::FLEX_START)
        };
    let right_to_left = |values: &ComputedValues| {
        matches!(
            values.cssom_writing_mode,
            WritingMode::VerticalRl | WritingMode::SidewaysRl
        )
    };
    (baseline_set_of(values.align_self, container.align_items).is_some()
        && uses_vertical_axes(cascade, parent)
        && right_to_left(values) != right_to_left(container))
    .then_some(fallback)
}

/// Whether in-flow grid item `idx` is sized along both axes without its
/// preferred aspect ratio, in which case the ratio is ignored: each axis is
/// either given a definite size or stretched by an explicit `stretch`
/// alignment, so the item fills its grid area on it (CSS Grid 1 §6.2,
/// csswg-drafts#5713). A stretch alignment in one axis together with a
/// definite size in the other leaves the ratio nothing to do, replaced
/// content included. An auto margin in an axis turns that axis's stretch
/// off, and so does a height that is a sizing keyword computed to `auto`
/// ([`ComputedValues::height_keyword`]).
///
/// The ratio is dropped from the item's style, which also removes it from
/// track sizing, so this applies only when every row track has a fixed size
/// and the item's height contribution cannot size a row.
pub(super) fn stretches_on_both_grid_axes(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    cv: &ComputedValues,
    settled: &[Settled],
) -> bool {
    if !settled[idx].has_aspect_ratio || doc.nodes[idx].layout_position() == Position::Absolute {
        return false;
    }
    let Some(parent) = doc.taffy_parent_of(idx) else {
        return false;
    };
    let Some(container) = container_laid_out_as(doc, cascade, parent, Display::Grid) else {
        return false;
    };
    let stretch = |value: Option<AlignItems>, fallback: SelfAlignmentValue| {
        value
            .unwrap_or_else(|| container_alignment(fallback))
            .keyword
            == AlignItemsKeyword::Stretch
    };
    let any_auto = |margins: [ComputedLengthPercentageOrAuto; 2]| {
        margins
            .iter()
            .any(|margin| matches!(margin, ComputedLengthPercentageOrAuto::Auto))
    };
    let horizontal = !matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
        || (stretch(settled[idx].justify_self, container.justify_items)
            && !any_auto([cv.margin.left, cv.margin.right]));
    let vertical = !matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
        || (cv.height_keyword.is_none()
            && stretch(settled[idx].align_self, container.align_items)
            && !any_auto([cv.margin.top, cv.margin.bottom]));
    horizontal && vertical && has_only_fixed_rows(doc, cascade, parent, container)
}

/// Whether in-flow grid item `idx` has a preferred aspect ratio and `auto`
/// sizes, and its `align-self` is an explicit `stretch` where its
/// `justify-self` is `normal`.
///
/// Both alignments would stretch the box, but with a preferred aspect ratio
/// the `normal` one yields to the explicit one: the box takes the height of
/// its grid area and the width the ratio gives for it, where `normal` alone
/// stretches the width and takes the height from the ratio (CSS Grid 1 §6.2,
/// csswg-drafts#5713). Taffy stretches the width of every box whose
/// `justify-self` is `normal`, so such an item is aligned at the start of the
/// row axis, which leaves the width to the ratio. A top or bottom margin that is
/// `auto` turns that stretch off, as does a height keyword that
/// computes to `auto` ([`ComputedValues::height_keyword`]). An item on
/// vertical axes, or in a grid on them, is left alone.
pub(super) fn is_sized_through_block_stretch(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    idx: usize,
    cv: &ComputedValues,
    settled: &[Settled],
) -> bool {
    if !settled[idx].has_aspect_ratio
        || doc.nodes[idx].layout_position() == Position::Absolute
        || !matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
        || !matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
        || cv.height_keyword.is_some()
        || uses_vertical_axes(cascade, idx)
    {
        return false;
    }
    let Some((parent, container)) = doc.taffy_parent_of(idx).and_then(|parent| {
        container_laid_out_as(doc, cascade, parent, Display::Grid)
            .map(|container| (parent, container))
    }) else {
        return false;
    };
    let keyword = |value: Option<AlignItems>, fallback: SelfAlignmentValue| {
        value
            .unwrap_or_else(|| container_alignment(fallback))
            .keyword
    };
    keyword(settled[idx].justify_self, container.justify_items) == AlignItemsKeyword::Normal
        && keyword(settled[idx].align_self, container.align_items) == AlignItemsKeyword::Stretch
        && ![cv.margin.top, cv.margin.bottom]
            .iter()
            .any(|margin| matches!(margin, ComputedLengthPercentageOrAuto::Auto))
        && !uses_vertical_axes(cascade, parent)
}

/// Whether every row an item of `grid` can land in has a fixed length, so
/// no item contributes to a row's size: the explicit rows are fixed, and so
/// are the implicit ones unless the grid has a single in-flow item with an
/// automatic one-row placement, which puts it in the first explicit row.
fn has_only_fixed_rows(
    doc: &LayoutSession<'_>,
    cascade: &CascadeResult,
    grid: usize,
    values: &ComputedValues,
) -> bool {
    let fixed = |size: &ComputedGridTrackSize| {
        use ComputedGridTrackBreadth::Px;
        matches!(
            size,
            ComputedGridTrackSize::Breadth(Px(_)) | ComputedGridTrackSize::MinMax(Px(_), Px(_))
        )
    };
    let explicit = match &values.grid_template_rows {
        ComputedGridTemplateTracks::None => false,
        ComputedGridTemplateTracks::List(list) => {
            !list.components.is_empty()
                && list.components.iter().all(|row| match row {
                    ComputedGridTrackListComponent::Size(size) => fixed(size),
                    ComputedGridTrackListComponent::Repeat(_) => false,
                })
        }
    };
    let implicit = !values.grid_auto_rows.is_empty() && values.grid_auto_rows.iter().all(fixed);
    let mut items = doc
        .child_ids(protocol::NodeId::from(grid))
        .map(usize::from)
        .filter(|&child| {
            doc.nodes[child].layout_display() != Display::None
                && doc.nodes[child].layout_position() != Position::Absolute
        });
    let lone_auto_placed_item = match (items.next(), items.next()) {
        (Some(item), None) => {
            // A box that is not an element, and a `display: contents` one,
            // is placed by no `grid-row`.
            let node = &doc.nodes[item];
            let cv = (node.kind() == NodeKind::Element && node.display() != DisplayValue::Contents)
                .then(|| &cascade.computed[item]);
            let (start, end) = cv.map_or((&GridLineValue::Auto, &GridLineValue::Auto), |cv| {
                (&cv.grid_row_start, &cv.grid_row_end)
            });
            use GridLineValue::{Auto, Span};
            matches!(
                (start, end),
                (Auto, Auto) | (Auto, Span(1)) | (Span(1), Auto)
            )
        }
        _ => false,
    };
    explicit && (implicit || lone_auto_placed_item)
}
