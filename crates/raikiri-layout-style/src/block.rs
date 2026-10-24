use crate::{LayoutStyleView, StyleFlags};
use raikiri_style::property::FloatValue;

impl LayoutStyleView<'_> {
    /// Alignment read by project block primitives, independent of trait defaults.
    #[inline]
    pub fn bridge_align_self(&self) -> Option<taffy::AlignSelf> {
        self.self_alignment(false)
    }
    /// Inline alignment read by project block primitives.
    #[inline]
    pub fn bridge_justify_self(&self) -> Option<taffy::AlignSelf> {
        self.self_alignment(true)
    }
    /// Content alignment read by project block primitives.
    #[inline]
    pub fn bridge_align_content(&self) -> taffy::AlignContent {
        crate::flex::content(self.prepared.values.align_content)
    }
}

impl taffy::BlockContainerStyle for LayoutStyleView<'_> {
    fn align_content(&self) -> taffy::AlignContent {
        self.bridge_align_content()
    }
    fn justify_items(&self) -> taffy::AlignItems {
        crate::flex::items(self.prepared.values.justify_items)
    }
}

impl taffy::BlockItemStyle for LayoutStyleView<'_> {
    fn is_table(&self) -> bool {
        self.flags().contains(StyleFlags::BLOCK_TABLE)
    }
    fn float(&self) -> taffy::Float {
        match self.prepared.values.float {
            FloatValue::Left => taffy::Float::Left,
            FloatValue::Right => taffy::Float::Right,
            _ => taffy::Float::None,
        }
    }
    fn clear(&self) -> taffy::Clear {
        LayoutStyleView::clear(self)
    }
}

impl taffy::OofItemStyle for LayoutStyleView<'_> {
    fn align_self(&self) -> Option<taffy::AlignSelf> {
        self.bridge_align_self()
    }
    fn justify_self(&self) -> Option<taffy::AlignSelf> {
        self.bridge_justify_self()
    }
    fn is_table(&self) -> bool {
        self.flags().contains(StyleFlags::BLOCK_TABLE)
            || self.flags().contains(StyleFlags::OOF_TABLE)
    }
    fn grid_row(&self) -> taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>> {
        taffy::GridItemStyle::grid_row(self)
    }
    fn grid_column(&self) -> taffy::Line<taffy::GridPlacement<raikiri_style::SmolStr>> {
        taffy::GridItemStyle::grid_column(self)
    }
}
