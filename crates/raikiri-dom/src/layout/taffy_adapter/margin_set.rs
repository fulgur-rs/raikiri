use super::convert::{FromTaffy, IntoTaffy};

/// The margins that are available for collapsing with the margins of a block.
///
/// The library keeps the largest positive and the smallest negative margin
/// private, so a set cannot be taken apart and put together again field by
/// field. The set is the library's value, wrapped: converting it to and from
/// the library's type moves it unchanged, which is what a cached layout output
/// needs to come back as it went in.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct CollapsibleMarginSet(taffy::CollapsibleMarginSet);

impl CollapsibleMarginSet {
    /// No collapsible margins.
    pub(crate) const ZERO: Self = Self(taffy::CollapsibleMarginSet::ZERO);

    /// A set holding a single margin.
    #[inline]
    pub(crate) fn from_margin(margin: f32) -> Self {
        Self(taffy::CollapsibleMarginSet::from_margin(margin))
    }

    /// The set after `margin` is collapsed into it.
    #[inline]
    pub(crate) fn collapse_with_margin(self, margin: f32) -> Self {
        Self(self.0.collapse_with_margin(margin))
    }

    /// The set after the set `other` is collapsed into it.
    #[inline]
    pub(crate) fn collapse_with_set(self, other: CollapsibleMarginSet) -> Self {
        Self(self.0.collapse_with_set(other.0))
    }

    /// The margin the set collapses to.
    #[inline]
    pub(crate) fn resolve(&self) -> f32 {
        self.0.resolve()
    }
}

impl IntoTaffy for CollapsibleMarginSet {
    type Taffy = taffy::CollapsibleMarginSet;

    #[inline]
    fn into_taffy(self) -> taffy::CollapsibleMarginSet {
        self.0
    }
}

impl FromTaffy for CollapsibleMarginSet {
    type Taffy = taffy::CollapsibleMarginSet;

    #[inline]
    fn from_taffy(value: taffy::CollapsibleMarginSet) -> Self {
        Self(value)
    }
}
