//! Keywords and small value types of the used style.
//!
//! These describe what a box uses, not what its author wrote: `overflow: auto`
//! and `overflow: scroll` are the same [`Overflow`], and a `direction` that a
//! vertical writing mode reverses is the reversed [`Direction`].

/// The layout of the children of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// Block layout.
    Block,
    /// Block layout that establishes a new block formatting context.
    FlowRoot,
    /// Flex layout.
    Flex,
    /// Grid layout.
    Grid,
    /// The box and its descendants generate no boxes.
    None,
}

/// Whether a box generates boxes at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxGenerationMode {
    /// The box generates boxes as usual.
    Normal,
    /// The box and its descendants generate no boxes.
    None,
}

/// The position scheme of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Position {
    /// In normal flow; insets have no effect.
    #[default]
    Static,
    /// In normal flow, then offset by its insets.
    Relative,
    /// Out of flow, placed against its containing block.
    Absolute,
    /// Out of flow, placed against the viewport.
    Fixed,
    /// In normal flow like a static box, and a containing block for
    /// absolutely positioned descendants.
    Sticky,
}

impl Position {
    /// Whether the position takes the box out of normal flow.
    #[inline(always)]
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }

    /// Whether the box is a containing block for absolutely positioned
    /// descendants.
    #[inline(always)]
    pub fn is_positioned(self) -> bool {
        !matches!(self, Self::Static)
    }
}

/// Which out-of-flow positions a box is the containing block of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainingBlockClaims {
    /// Whether the box is the containing block of `position: absolute` boxes.
    pub absolute: bool,
    /// Whether the box is the containing block of `position: fixed` boxes.
    pub fixed: bool,
}

impl ContainingBlockClaims {
    /// Claims nothing.
    pub const NONE: Self = Self {
        absolute: false,
        fixed: false,
    };

    /// Whether the box is the containing block of a box with `position`.
    #[inline(always)]
    pub fn for_position(self, position: Position) -> bool {
        match position {
            Position::Absolute => self.absolute,
            Position::Fixed => self.fixed,
            Position::Static | Position::Relative | Position::Sticky => false,
        }
    }
}

/// What the size properties of a box measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BoxSizing {
    /// The border box.
    #[default]
    BorderBox,
    /// The content box.
    ContentBox,
}

/// How overflowing content affects the layout of its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overflow {
    /// Content spills out and counts for the scrollable overflow of ancestors.
    #[default]
    Visible,
    /// Content is clipped and does not count for ancestors.
    Clip,
    /// Content is clipped; the box is a scroll container.
    Hidden,
    /// Content is clipped and scrolls; space is reserved for a scrollbar.
    Scroll,
}

impl Overflow {
    /// Whether the box contains its content: everything but `visible` and `clip`.
    #[inline(always)]
    pub fn is_scroll_container(self) -> bool {
        match self {
            Self::Visible | Self::Clip => false,
            Self::Hidden | Self::Scroll => true,
        }
    }
}

/// The layout-relevant parts of `contain`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Contain(u8);

impl Contain {
    /// No containment.
    pub const NONE: Self = Self(0);
    /// Layout containment.
    pub const LAYOUT: Self = Self(1 << 0);
    /// Paint containment.
    pub const PAINT: Self = Self(1 << 1);
    /// Layout and paint containment.
    pub const CONTENT: Self = Self(Self::LAYOUT.0 | Self::PAINT.0);

    /// Whether every kind of containment in `other` is set.
    #[inline(always)]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether the box establishes an independent formatting context.
    #[inline(always)]
    pub const fn establishes_independent_formatting_context(self) -> bool {
        self.0 & Self::CONTENT.0 != 0
    }

    /// Whether the containment keeps overflowing content out of the scrollable
    /// overflow of the ancestors: layout containment makes it ink overflow and
    /// paint containment clips it.
    #[inline(always)]
    pub const fn contains_scrollable_overflow(self) -> bool {
        self.0 & Self::CONTENT.0 != 0
    }
}

/// The inline base direction of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction {
    /// Left to right.
    #[default]
    Ltr,
    /// Right to left.
    Rtl,
}

impl Direction {
    /// Whether the direction is right to left.
    #[inline]
    pub fn is_rtl(&self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// The float side of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Float {
    /// Floats to the left.
    Left,
    /// Floats to the right.
    Right,
    /// Does not float.
    #[default]
    None,
}

impl Float {
    /// Whether the box floats.
    #[inline]
    pub fn is_floated(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// The floats a box clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Clear {
    /// Clears left floats.
    Left,
    /// Clears right floats.
    Right,
    /// Clears floats on both sides.
    Both,
    /// Clears nothing.
    #[default]
    None,
}
