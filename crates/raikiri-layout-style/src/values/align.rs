//! Alignment keywords.
//!
//! An alignment is a position keyword and an overflow-position modifier, so
//! `safe center` and `center` are [`AlignItems`] values with the same
//! [`AlignItemsKeyword`] and a different [`AlignmentSafety`].

/// The position keyword of an item alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[doc(hidden)]
pub enum AlignItemsKeyword {
    /// `normal`.
    #[default]
    Normal,
    /// `start`.
    Start,
    /// `end`.
    End,
    /// `flex-start`.
    FlexStart,
    /// `flex-end`.
    FlexEnd,
    /// `self-start`.
    SelfStart,
    /// `self-end`.
    SelfEnd,
    /// `center`.
    Center,
    /// `baseline`.
    Baseline,
    /// `stretch`.
    Stretch,
}

/// The position keyword of a content alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[doc(hidden)]
pub enum AlignContentKeyword {
    /// `normal`.
    #[default]
    Normal,
    /// `start`.
    Start,
    /// `end`.
    End,
    /// `flex-start`.
    FlexStart,
    /// `flex-end`.
    FlexEnd,
    /// `center`.
    Center,
    /// `stretch`.
    Stretch,
    /// `space-between`.
    SpaceBetween,
    /// `space-evenly`.
    SpaceEvenly,
    /// `space-around`.
    SpaceAround,
}

/// What an alignment does when the aligned item overflows its alignment area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[doc(hidden)]
pub enum AlignmentSafety {
    /// No overflow-position keyword was written.
    #[default]
    Default,
    /// The item may overflow on either side (the keyword that opts out of `safe`).
    Unsafe,
    /// `safe`: an overflowing item falls back to `start`.
    Safe,
}

/// An item alignment: `align-items`, `justify-items` or a self alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[doc(hidden)]
pub struct AlignItems {
    /// The position keyword.
    pub keyword: AlignItemsKeyword,
    /// The overflow-position modifier.
    pub safety: AlignmentSafety,
}

/// A self alignment, which is an item alignment of one item.
#[doc(hidden)]
pub type AlignSelf = AlignItems;

impl AlignItems {
    /// `normal`.
    pub const NORMAL: Self = Self {
        keyword: AlignItemsKeyword::Normal,
        safety: AlignmentSafety::Default,
    };
    /// `start`.
    pub const START: Self = Self {
        keyword: AlignItemsKeyword::Start,
        safety: AlignmentSafety::Default,
    };
    /// `end`.
    pub const END: Self = Self {
        keyword: AlignItemsKeyword::End,
        safety: AlignmentSafety::Default,
    };
    /// `flex-start`.
    pub const FLEX_START: Self = Self {
        keyword: AlignItemsKeyword::FlexStart,
        safety: AlignmentSafety::Default,
    };
    /// `flex-end`.
    pub const FLEX_END: Self = Self {
        keyword: AlignItemsKeyword::FlexEnd,
        safety: AlignmentSafety::Default,
    };
    /// `self-start`.
    pub const SELF_START: Self = Self {
        keyword: AlignItemsKeyword::SelfStart,
        safety: AlignmentSafety::Default,
    };
    /// `self-end`.
    pub const SELF_END: Self = Self {
        keyword: AlignItemsKeyword::SelfEnd,
        safety: AlignmentSafety::Default,
    };
    /// `center`.
    pub const CENTER: Self = Self {
        keyword: AlignItemsKeyword::Center,
        safety: AlignmentSafety::Default,
    };
    /// `baseline`.
    pub const BASELINE: Self = Self {
        keyword: AlignItemsKeyword::Baseline,
        safety: AlignmentSafety::Default,
    };
    /// `stretch`.
    pub const STRETCH: Self = Self {
        keyword: AlignItemsKeyword::Stretch,
        safety: AlignmentSafety::Default,
    };
    /// `safe self-end`.
    pub const SAFE_SELF_END: Self = Self {
        keyword: AlignItemsKeyword::SelfEnd,
        safety: AlignmentSafety::Safe,
    };

    /// The alignment with `self-start` and `self-end` resolved against the
    /// direction of the container.
    ///
    /// Along the inline axis, an item whose direction differs from its
    /// container's has its `self-start` at the container's end.
    #[inline]
    pub fn resolve_self_relative(
        self,
        item_direction: super::Direction,
        container_direction: super::Direction,
        axis_is_inline: bool,
    ) -> Self {
        let flip = axis_is_inline && item_direction != container_direction;
        let keyword = match self.keyword {
            AlignItemsKeyword::SelfStart if flip => AlignItemsKeyword::End,
            AlignItemsKeyword::SelfStart => AlignItemsKeyword::Start,
            AlignItemsKeyword::SelfEnd if flip => AlignItemsKeyword::Start,
            AlignItemsKeyword::SelfEnd => AlignItemsKeyword::End,
            other => other,
        };
        Self {
            keyword,
            safety: self.safety,
        }
    }
}

/// A content alignment: `align-content` or `justify-content`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[doc(hidden)]
pub struct AlignContent {
    /// The position keyword.
    pub keyword: AlignContentKeyword,
    /// The overflow-position modifier.
    pub safety: AlignmentSafety,
}

impl AlignContent {
    /// `normal`.
    pub const NORMAL: Self = Self {
        keyword: AlignContentKeyword::Normal,
        safety: AlignmentSafety::Default,
    };
    /// `start`.
    pub const START: Self = Self {
        keyword: AlignContentKeyword::Start,
        safety: AlignmentSafety::Default,
    };
    /// `end`.
    pub const END: Self = Self {
        keyword: AlignContentKeyword::End,
        safety: AlignmentSafety::Default,
    };
    /// `flex-start`.
    pub const FLEX_START: Self = Self {
        keyword: AlignContentKeyword::FlexStart,
        safety: AlignmentSafety::Default,
    };
    /// `flex-end`.
    pub const FLEX_END: Self = Self {
        keyword: AlignContentKeyword::FlexEnd,
        safety: AlignmentSafety::Default,
    };
    /// `center`.
    pub const CENTER: Self = Self {
        keyword: AlignContentKeyword::Center,
        safety: AlignmentSafety::Default,
    };
    /// `stretch`.
    pub const STRETCH: Self = Self {
        keyword: AlignContentKeyword::Stretch,
        safety: AlignmentSafety::Default,
    };
    /// `space-between`.
    pub const SPACE_BETWEEN: Self = Self {
        keyword: AlignContentKeyword::SpaceBetween,
        safety: AlignmentSafety::Default,
    };
    /// `space-evenly`.
    pub const SPACE_EVENLY: Self = Self {
        keyword: AlignContentKeyword::SpaceEvenly,
        safety: AlignmentSafety::Default,
    };
    /// `space-around`.
    pub const SPACE_AROUND: Self = Self {
        keyword: AlignContentKeyword::SpaceAround,
        safety: AlignmentSafety::Default,
    };
}
