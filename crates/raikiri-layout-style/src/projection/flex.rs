//! The flex getters of a view that answer in the layout library's vocabulary.

use crate::LayoutStyleView;
use raikiri_style::property::{FlexDirectionValue as D, FlexWrapValue as W};

impl LayoutStyleView<'_> {
    /// The main-axis direction of a flex container.
    #[inline]
    pub fn flex_direction(&self) -> taffy::FlexDirection {
        match self.prepared.values.flex_direction {
            D::RowReverse => taffy::FlexDirection::RowReverse,
            D::Column => taffy::FlexDirection::Column,
            D::ColumnReverse => taffy::FlexDirection::ColumnReverse,
            _ => taffy::FlexDirection::Row,
        }
    }
    /// Whether and how a flex container wraps its lines.
    #[inline]
    pub fn flex_wrap(&self) -> taffy::FlexWrap {
        match self.prepared.values.flex_wrap {
            W::Wrap => taffy::FlexWrap::Wrap,
            W::WrapReverse => taffy::FlexWrap::WrapReverse,
            _ => taffy::FlexWrap::NoWrap,
        }
    }
}
