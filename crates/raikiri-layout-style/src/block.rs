use crate::{LayoutStyleView, StyleFlags};
use raikiri_style::property::{AlignSelfValue, FloatValue, SelfAlignmentValue};

/// The set of baselines a box aligns to when it asks for baseline alignment
/// (CSS Box Alignment 3 §9.1): `baseline` or `first baseline` aligns the first
/// baselines of the boxes, `last baseline` the last ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineSet {
    /// The first baseline of the box.
    First,
    /// The last baseline of the box.
    Last,
}

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
    /// The set of baselines the in-flow box aligns to along an axis of its
    /// container `parent`, if it asks for baseline alignment there: `justify`
    /// selects `justify-self`, falling back to the `justify-items` of the
    /// container, and otherwise `align-self` and `align-items` decide.
    pub fn self_baseline_set(&self, parent: &Self, justify: bool) -> Option<BaselineSet> {
        let (own, inherited) = if justify {
            (
                self.prepared.values.justify_self,
                parent.prepared.values.justify_items,
            )
        } else {
            (
                self.prepared.values.align_self,
                parent.prepared.values.align_items,
            )
        };
        match match own {
            AlignSelfValue::Value(value) => value,
            _ => inherited,
        } {
            SelfAlignmentValue::Baseline => Some(BaselineSet::First),
            SelfAlignmentValue::LastBaseline => Some(BaselineSet::Last),
            _ => None,
        }
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
