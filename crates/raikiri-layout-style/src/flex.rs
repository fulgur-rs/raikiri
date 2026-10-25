use crate::overrides::OverrideKind;
use crate::*;
use raikiri_layout_geometry::Size;
use raikiri_style::{
    ComputedFlexBasis,
    property::{
        AlignSelfValue, ContentAlignmentValue as C, FlexDirectionValue as D, FlexWrapValue as W,
        PositionValue, SelfAlignmentValue as A,
    },
};

pub(crate) fn content(value: C) -> values::AlignContent {
    match value {
        C::Stretch => values::AlignContent::STRETCH,
        C::SpaceBetween => values::AlignContent::SPACE_BETWEEN,
        C::SpaceEvenly => values::AlignContent::SPACE_EVENLY,
        C::SpaceAround => values::AlignContent::SPACE_AROUND,
        C::Center => values::AlignContent::CENTER,
        C::Start => values::AlignContent::START,
        C::End => values::AlignContent::END,
        C::FlexStart => values::AlignContent::FLEX_START,
        C::FlexEnd => values::AlignContent::FLEX_END,
        _ => values::AlignContent::NORMAL,
    }
}
/// The item alignment that the self alignment `value` asks for.
#[doc(hidden)]
pub fn items(value: A) -> values::AlignItems {
    match value {
        A::Stretch => values::AlignItems::STRETCH,
        A::Center => values::AlignItems::CENTER,
        A::Start => values::AlignItems::START,
        A::End => values::AlignItems::END,
        A::FlexStart => values::AlignItems::FLEX_START,
        A::FlexEnd => values::AlignItems::FLEX_END,
        A::SelfStart => values::AlignItems::SELF_START,
        A::SelfEnd => values::AlignItems::SELF_END,
        A::Safe(position) => values::AlignItems {
            safety: values::AlignmentSafety::Safe,
            ..items(position.alignment())
        },
        A::Unsafe(position) => values::AlignItems {
            safety: values::AlignmentSafety::Unsafe,
            ..items(position.alignment())
        },
        A::Baseline | A::LastBaseline => values::AlignItems::BASELINE,
        _ => values::AlignItems::NORMAL,
    }
}
impl LayoutStyleView<'_> {
    pub(crate) fn self_alignment(&self, inline: bool) -> Option<values::AlignSelf> {
        if let Some(value) = self.overrides().find_map(|o| match o {
            OverrideKind::AlignSelf(v) if !inline => Some(v),
            OverrideKind::JustifySelf(v) if inline => Some(v),
            _ => None,
        }) {
            return value;
        }
        let c = &*self.prepared.values;
        let value = if inline { c.justify_self } else { c.align_self };
        match value {
            AlignSelfValue::Value(A::LastBaseline)
                if matches!(c.position, PositionValue::Absolute | PositionValue::Fixed) =>
            {
                Some(values::AlignItems::SAFE_SELF_END)
            }
            AlignSelfValue::Value(A::Baseline)
                if matches!(c.position, PositionValue::Absolute | PositionValue::Fixed) =>
            {
                Some(values::AlignItems {
                    keyword: values::AlignItemsKeyword::Baseline,
                    safety: values::AlignmentSafety::Safe,
                })
            }
            AlignSelfValue::Value(A::Normal) => Some(values::AlignItems::NORMAL),
            AlignSelfValue::Value(value) => Some(items(value)),
            _ => None,
        }
    }
    /// The row and column gaps.
    #[inline]
    pub fn gap(&self) -> Size<values::LengthPercentage> {
        // Grid columns/rows and gaps keep their CSS inline/block semantics.
        Size {
            width: self
                .length(LengthProperty::ColumnGap)
                .lp(PercentBasis::Taffy),
            height: self.length(LengthProperty::RowGap).lp(PercentBasis::Taffy),
        }
    }
    /// Whether the main axis of a flex container runs along its rows, as
    /// `row` and `row-reverse` do, rather than along its columns.
    #[inline]
    pub fn flex_direction_is_row(&self) -> bool {
        !matches!(
            self.prepared.values.flex_direction,
            D::Column | D::ColumnReverse
        )
    }
    /// Whether a flex container wraps its items onto more lines when they do
    /// not fit.
    #[inline]
    pub fn flex_wraps(&self) -> bool {
        self.prepared.values.flex_wrap != W::NoWrap
    }
    /// How a container distributes free space along its main axis.
    #[inline]
    pub fn justify_content(&self) -> values::AlignContent {
        content(self.prepared.values.justify_content)
    }
    /// The default cross-axis alignment of the items of a container.
    #[inline]
    pub fn align_items(&self) -> values::AlignItems {
        items(self.prepared.values.align_items)
    }
    /// The flex basis of a flex item.
    #[inline]
    pub fn flex_basis(&self) -> values::Dimension {
        if matches!(self.prepared.values.flex_basis, ComputedFlexBasis::Content) {
            if self
                .overrides()
                .any(|entry| matches!(entry, OverrideKind::Length(LengthProperty::FlexBasis, _)))
            {
                self.length(LengthProperty::FlexBasis).dimension()
            } else {
                values::Dimension::content()
            }
        } else {
            self.length(LengthProperty::FlexBasis).dimension()
        }
    }
    /// The flex grow factor of a flex item.
    #[inline]
    pub fn flex_grow(&self) -> f32 {
        self.prepared.values.flex_grow
    }
    /// The flex shrink factor of a flex item, including an intrinsic-probe override.
    #[inline]
    pub fn flex_shrink(&self) -> f32 {
        self.overrides()
            .find_map(|o| match o {
                OverrideKind::FlexShrink(v) => Some(v),
                _ => None,
            })
            .unwrap_or(self.prepared.values.flex_shrink)
    }
    /// The self alignment of a flex item along the axis `align_axis_is_inline` selects.
    ///
    /// Grid and out-of-flow items read [`Self::bridge_align_self`] and
    /// [`Self::bridge_justify_self`] instead.
    #[inline]
    pub fn align_self(&self) -> Option<values::AlignSelf> {
        self.self_alignment(self.context.align_axis_is_inline)
    }
}
