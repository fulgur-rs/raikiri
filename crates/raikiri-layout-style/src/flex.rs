use crate::overrides::OverrideKind;
use crate::*;
use raikiri_style::{
    ComputedFlexBasis,
    property::{
        AlignSelfValue, ContentAlignmentValue as C, FlexDirectionValue as D, FlexWrapValue as W,
        PositionValue, SelfAlignmentValue as A,
    },
};

pub(crate) fn content(value: C) -> taffy::AlignContent {
    match value {
        C::Stretch => taffy::AlignContent::STRETCH,
        C::SpaceBetween => taffy::AlignContent::SPACE_BETWEEN,
        C::SpaceEvenly => taffy::AlignContent::SPACE_EVENLY,
        C::SpaceAround => taffy::AlignContent::SPACE_AROUND,
        C::Center => taffy::AlignContent::CENTER,
        C::Start => taffy::AlignContent::START,
        C::End => taffy::AlignContent::END,
        C::FlexStart => taffy::AlignContent::FLEX_START,
        C::FlexEnd => taffy::AlignContent::FLEX_END,
        _ => taffy::AlignContent::NORMAL,
    }
}
pub(crate) fn items(value: A) -> taffy::AlignItems {
    match value {
        A::Stretch => taffy::AlignItems::STRETCH,
        A::Center => taffy::AlignItems::CENTER,
        A::Start => taffy::AlignItems::START,
        A::End => taffy::AlignItems::END,
        A::FlexStart => taffy::AlignItems::FLEX_START,
        A::FlexEnd => taffy::AlignItems::FLEX_END,
        A::Baseline | A::LastBaseline => taffy::AlignItems::BASELINE,
        _ => taffy::AlignItems::NORMAL,
    }
}
impl LayoutStyleView<'_> {
    pub(crate) fn self_alignment(&self, inline: bool) -> Option<taffy::AlignSelf> {
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
                Some(taffy::AlignItems::SAFE_SELF_END)
            }
            AlignSelfValue::Value(A::Baseline)
                if matches!(c.position, PositionValue::Absolute | PositionValue::Fixed) =>
            {
                Some(taffy::AlignItems {
                    keyword: taffy::AlignItemsKeyword::Baseline,
                    safety: taffy::AlignmentSafety::Safe,
                })
            }
            AlignSelfValue::Value(A::Normal) => Some(taffy::AlignItems::NORMAL),
            AlignSelfValue::Value(value) => Some(items(value)),
            _ => None,
        }
    }
    pub(crate) fn gap_value(&self) -> taffy::Size<taffy::LengthPercentage> {
        // Grid columns/rows and gaps keep their CSS inline/block semantics.
        taffy::Size {
            width: self
                .length(LengthProperty::ColumnGap)
                .lp(PercentBasis::Taffy),
            height: self.length(LengthProperty::RowGap).lp(PercentBasis::Taffy),
        }
    }
}
impl taffy::FlexboxContainerStyle for LayoutStyleView<'_> {
    #[inline]
    fn flex_direction(&self) -> taffy::FlexDirection {
        match self.prepared.values.flex_direction {
            D::RowReverse => taffy::FlexDirection::RowReverse,
            D::Column => taffy::FlexDirection::Column,
            D::ColumnReverse => taffy::FlexDirection::ColumnReverse,
            _ => taffy::FlexDirection::Row,
        }
    }
    #[inline]
    fn flex_wrap(&self) -> taffy::FlexWrap {
        match self.prepared.values.flex_wrap {
            W::Wrap => taffy::FlexWrap::Wrap,
            W::WrapReverse => taffy::FlexWrap::WrapReverse,
            _ => taffy::FlexWrap::NoWrap,
        }
    }
    #[inline]
    fn gap(&self) -> taffy::Size<taffy::LengthPercentage> {
        self.gap_value()
    }
    #[inline]
    fn align_content(&self) -> taffy::AlignContent {
        content(self.prepared.values.align_content)
    }
    #[inline]
    fn justify_content(&self) -> taffy::JustifyContent {
        content(self.prepared.values.justify_content)
    }
    #[inline]
    fn align_items(&self) -> taffy::AlignItems {
        items(self.prepared.values.align_items)
    }
}
impl taffy::FlexboxItemStyle for LayoutStyleView<'_> {
    #[inline]
    fn flex_basis(&self) -> taffy::Dimension {
        if matches!(self.prepared.values.flex_basis, ComputedFlexBasis::Content) {
            if self
                .overrides()
                .any(|entry| matches!(entry, OverrideKind::Length(LengthProperty::FlexBasis, _)))
            {
                self.length(LengthProperty::FlexBasis).dimension()
            } else {
                taffy::Dimension::content()
            }
        } else {
            self.length(LengthProperty::FlexBasis).dimension()
        }
    }
    #[inline]
    fn flex_grow(&self) -> f32 {
        self.prepared.values.flex_grow
    }
    #[inline]
    fn flex_shrink(&self) -> f32 {
        self.overrides()
            .find_map(|o| match o {
                OverrideKind::FlexShrink(v) => Some(v),
                _ => None,
            })
            .unwrap_or(self.prepared.values.flex_shrink)
    }
    #[inline]
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.self_alignment(self.context.align_axis_is_inline)
    }
}
