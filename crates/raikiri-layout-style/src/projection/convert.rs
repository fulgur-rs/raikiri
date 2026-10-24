//! Conversions from the style values to the layout library's.
//!
//! Every conversion of an enum names all its variants, with no wildcard arm,
//! so a variant that the library adds is a compile error here instead of a
//! value that is mapped to something else. [`Contain`] is a set of the two
//! kinds it holds, and each is converted by name.

use super::{Dimension, LengthPercentage, LengthPercentageAuto};
use crate::values::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignmentSafety,
    BoxGenerationMode, BoxSizing, Clear, Contain, ContainingBlockClaims, Direction, Display, Float,
    Overflow, Position,
};
use raikiri_layout_geometry::{Edges, Point, Size};

/// A style value and the layout library's value of the same meaning.
///
/// The orphan rule keeps `From` from relating two types of other crates, and
/// the library's types are never named by the code that reads these values, so
/// the conversion lives in this trait.
pub trait ToTaffy: Copy {
    /// The library's type.
    type Taffy;

    /// The library's value.
    fn to_taffy(self) -> Self::Taffy;
}

macro_rules! unit_variants {
    ($($ty:ident => $taffy:ty { $($variant:ident),+ $(,)? })+) => {$(
        impl ToTaffy for $ty {
            type Taffy = $taffy;

            #[inline]
            fn to_taffy(self) -> $taffy {
                match self {
                    $($ty::$variant => <$taffy>::$variant,)+
                }
            }
        }
    )+};
}

unit_variants! {
    Display => taffy::Display { Block, FlowRoot, Flex, Grid, None }
    BoxGenerationMode => taffy::BoxGenerationMode { Normal, None }
    Position => taffy::Position { Static, Relative, Absolute, Fixed, Sticky }
    BoxSizing => taffy::BoxSizing { BorderBox, ContentBox }
    Overflow => taffy::Overflow { Visible, Clip, Hidden, Scroll }
    Direction => taffy::Direction { Ltr, Rtl }
    Float => taffy::Float { Left, Right, None }
    Clear => taffy::Clear { Left, Right, Both, None }
    AlignmentSafety => taffy::AlignmentSafety { Default, Unsafe, Safe }
    AlignContentKeyword => taffy::AlignContentKeyword {
        Normal, Start, End, FlexStart, FlexEnd, Center, Stretch, SpaceBetween, SpaceEvenly,
        SpaceAround,
    }
    AlignItemsKeyword => taffy::AlignItemsKeyword {
        Normal, Start, End, FlexStart, FlexEnd, SelfStart, SelfEnd, Center, Baseline, Stretch,
    }
}

impl ToTaffy for AlignItems {
    type Taffy = taffy::AlignItems;

    #[inline]
    fn to_taffy(self) -> taffy::AlignItems {
        let Self { keyword, safety } = self;
        taffy::AlignItems {
            keyword: keyword.to_taffy(),
            safety: safety.to_taffy(),
        }
    }
}

impl ToTaffy for AlignContent {
    type Taffy = taffy::AlignContent;

    #[inline]
    fn to_taffy(self) -> taffy::AlignContent {
        let Self { keyword, safety } = self;
        taffy::AlignContent {
            keyword: keyword.to_taffy(),
            safety: safety.to_taffy(),
        }
    }
}

impl ToTaffy for ContainingBlockClaims {
    type Taffy = taffy::ContainingBlockClaims;

    #[inline]
    fn to_taffy(self) -> taffy::ContainingBlockClaims {
        let Self { absolute, fixed } = self;
        taffy::ContainingBlockClaims { absolute, fixed }
    }
}

impl ToTaffy for Contain {
    type Taffy = taffy::Contain;

    #[inline]
    fn to_taffy(self) -> taffy::Contain {
        let layout = if self.contains(Self::LAYOUT) {
            taffy::Contain::LAYOUT
        } else {
            taffy::Contain::NONE
        };
        let paint = if self.contains(Self::PAINT) {
            taffy::Contain::PAINT
        } else {
            taffy::Contain::NONE
        };
        layout | paint
    }
}

macro_rules! same_bits {
    ($($ty:ident),+ $(,)?) => {$(
        impl ToTaffy for $ty {
            type Taffy = taffy::$ty;

            #[inline(always)]
            fn to_taffy(self) -> taffy::$ty {
                self.0
            }
        }
    )+};
}

same_bits!(Dimension, LengthPercentage, LengthPercentageAuto);

macro_rules! plain {
    ($($ty:ty),+ $(,)?) => {$(
        impl ToTaffy for $ty {
            type Taffy = $ty;

            #[inline(always)]
            fn to_taffy(self) -> $ty {
                self
            }
        }
    )+};
}

plain!(f32, bool);

impl<T: ToTaffy> ToTaffy for Option<T> {
    type Taffy = Option<T::Taffy>;

    #[inline]
    fn to_taffy(self) -> Self::Taffy {
        self.map(ToTaffy::to_taffy)
    }
}

impl<T: ToTaffy> ToTaffy for Size<T> {
    type Taffy = taffy::Size<T::Taffy>;

    #[inline]
    fn to_taffy(self) -> Self::Taffy {
        let Self { width, height } = self;
        taffy::Size {
            width: width.to_taffy(),
            height: height.to_taffy(),
        }
    }
}

impl<T: ToTaffy> ToTaffy for Edges<T> {
    type Taffy = taffy::Rect<T::Taffy>;

    #[inline]
    fn to_taffy(self) -> Self::Taffy {
        let Self {
            left,
            right,
            top,
            bottom,
        } = self;
        taffy::Rect {
            left: left.to_taffy(),
            right: right.to_taffy(),
            top: top.to_taffy(),
            bottom: bottom.to_taffy(),
        }
    }
}

impl<T: ToTaffy> ToTaffy for Point<T> {
    type Taffy = taffy::Point<T::Taffy>;

    #[inline]
    fn to_taffy(self) -> Self::Taffy {
        let Self { x, y } = self;
        taffy::Point {
            x: x.to_taffy(),
            y: y.to_taffy(),
        }
    }
}
