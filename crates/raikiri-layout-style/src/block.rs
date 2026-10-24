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
    /// The content alignment of a container.
    #[inline]
    pub fn align_content(&self) -> taffy::AlignContent {
        self.bridge_align_content()
    }
    /// The default inline alignment of the items of a container.
    pub fn justify_items(&self) -> taffy::AlignItems {
        crate::flex::items(self.prepared.values.justify_items)
    }
    /// Whether the box is a table in a block formatting context.
    pub fn is_table(&self) -> bool {
        self.flags().contains(StyleFlags::BLOCK_TABLE)
    }
    /// Whether the box is a table when positioned out of flow.
    pub(crate) fn is_oof_table(&self) -> bool {
        self.flags().contains(StyleFlags::BLOCK_TABLE)
            || self.flags().contains(StyleFlags::OOF_TABLE)
    }
    /// The physical float side.
    pub fn float(&self) -> taffy::Float {
        match self.prepared.values.float {
            FloatValue::Left => taffy::Float::Left,
            FloatValue::Right => taffy::Float::Right,
            _ => taffy::Float::None,
        }
    }
}
