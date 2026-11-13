//! Flow-relative margin, padding, inset, border and intrinsic size longhands.
//!
//! The parser stores `margin-inline-start` and its siblings (the padding,
//! inset and border ones too) under the physical longhand they map to in `horizontal-tb`
//! with `direction: ltr`, and tags the declaration with the flow-relative side
//! it was written for. `contain-intrinsic-inline-size` and
//! `contain-intrinsic-block-size` are stored the same way, as
//! `contain-intrinsic-width` and `contain-intrinsic-height`, tagged with the
//! start side of their axis: only the axis of a side matters to them.
//! The cascade moves a tagged declaration to the physical side that side maps
//! to under the element's (or page context's) own writing mode and
//! `direction` before it picks winners, so that a flow-relative declaration
//! and a physical one compete for the same physical side by the usual cascade
//! order (CSS Logical Properties 1 §4, "Flow-Relative Box Model Properties",
//! <https://www.w3.org/TR/css-logical-1/#box>, and CSS Writing Modes 4 §6.4
//! "Abstract-to-Physical Mappings",
//! <https://www.w3.org/TR/css-writing-modes-4/#logical-to-physical>).
//!
//! Elements map through the writing mode block layout stacks their boxes
//! in: `vertical-rl` and `vertical-lr` as specified, the sideways modes as
//! `horizontal-tb`. The page context maps through its specified writing
//! mode, because the page box's margins and padding do not depend on block
//! layout. The intrinsic sizes are physical quantities in layout, so they map
//! through the writing mode the element has ([`FlowMapping::size_writing_mode`]),
//! which the placement of the children of a table box or a multicol flow does
//! not change.
//!
//! `text-orientation: upright` makes the used direction `ltr` in vertical
//! writing modes (CSS Writing Modes 4 §5.1), and both mappings use that used
//! direction.

use smol_str::SmolStr;

use crate::property::{Direction, PropertyKey, PropertyValue, WritingMode};

/// The flow-relative side a margin, padding, inset, border or intrinsic size
/// declaration was written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LogicalSide {
    InlineStart,
    InlineEnd,
    BlockStart,
    BlockEnd,
}

/// A physical box side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// The writing mode and direction that map flow-relative sides to physical
/// ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlowMapping {
    pub(crate) writing_mode: WritingMode,
    pub(crate) direction: Direction,
    /// The writing mode that orients a flow-relative size
    /// (`contain-intrinsic-inline-size`, `contain-intrinsic-block-size`). A
    /// size is a physical quantity in layout, so it follows the writing mode the
    /// element has even where [`Self::writing_mode`] is `horizontal-tb` because
    /// block layout places the children of a table box or a multicol flow on
    /// horizontal axes.
    pub(crate) size_writing_mode: WritingMode,
}

impl FlowMapping {
    /// `horizontal-tb` with `direction: ltr`.
    #[cfg(test)]
    pub(crate) const HORIZONTAL: Self = Self::new(WritingMode::HorizontalTb, Direction::Ltr);

    /// The mapping of a box whose sides and sizes both follow `writing_mode`.
    pub(crate) const fn new(writing_mode: WritingMode, direction: Direction) -> Self {
        Self {
            writing_mode,
            direction,
            size_writing_mode: writing_mode,
        }
    }

    /// This mapping with its flow-relative sizes oriented by `size_writing_mode`.
    pub(crate) const fn with_size_writing_mode(self, size_writing_mode: WritingMode) -> Self {
        Self {
            size_writing_mode,
            ..self
        }
    }

    /// Whether the inline axis is vertical.
    pub(crate) fn is_vertical(self) -> bool {
        self.writing_mode.is_vertical()
    }

    /// The physical side `side` of a `property` maps to: the sizes follow
    /// [`Self::size_writing_mode`], the other properties [`Self::writing_mode`].
    fn physical_of(self, property: BoxProperty, side: LogicalSide) -> PhysicalSide {
        match property {
            BoxProperty::ContainIntrinsic => Self {
                writing_mode: self.size_writing_mode,
                ..self
            }
            .physical(side),
            _ => self.physical(side),
        }
    }

    /// The physical side `side` maps to (CSS Writing Modes 4 §6.4).
    fn physical(self, side: LogicalSide) -> PhysicalSide {
        let ltr = self.direction == Direction::Ltr;
        match (self.writing_mode, side) {
            (WritingMode::HorizontalTb, LogicalSide::BlockStart) => PhysicalSide::Top,
            (WritingMode::HorizontalTb, LogicalSide::BlockEnd) => PhysicalSide::Bottom,
            (WritingMode::HorizontalTb, LogicalSide::InlineStart) if ltr => PhysicalSide::Left,
            (WritingMode::HorizontalTb, LogicalSide::InlineStart) => PhysicalSide::Right,
            (WritingMode::HorizontalTb, LogicalSide::InlineEnd) if ltr => PhysicalSide::Right,
            (WritingMode::HorizontalTb, LogicalSide::InlineEnd) => PhysicalSide::Left,
            (WritingMode::VerticalRl | WritingMode::SidewaysRl, LogicalSide::BlockStart) => {
                PhysicalSide::Right
            }
            (WritingMode::VerticalRl | WritingMode::SidewaysRl, LogicalSide::BlockEnd) => {
                PhysicalSide::Left
            }
            (WritingMode::VerticalLr | WritingMode::SidewaysLr, LogicalSide::BlockStart) => {
                PhysicalSide::Left
            }
            (WritingMode::VerticalLr | WritingMode::SidewaysLr, LogicalSide::BlockEnd) => {
                PhysicalSide::Right
            }
            // `sideways-lr` turns the line upside down: its inline-start is
            // the bottom edge in `ltr`.
            (WritingMode::SidewaysLr, LogicalSide::InlineStart) if ltr => PhysicalSide::Bottom,
            (WritingMode::SidewaysLr, LogicalSide::InlineStart) => PhysicalSide::Top,
            (WritingMode::SidewaysLr, LogicalSide::InlineEnd) if ltr => PhysicalSide::Top,
            (WritingMode::SidewaysLr, LogicalSide::InlineEnd) => PhysicalSide::Bottom,
            (_, LogicalSide::InlineStart) if ltr => PhysicalSide::Top,
            (_, LogicalSide::InlineStart) => PhysicalSide::Bottom,
            (_, LogicalSide::InlineEnd) if ltr => PhysicalSide::Bottom,
            (_, LogicalSide::InlineEnd) => PhysicalSide::Top,
        }
    }

    /// The physical longhand key a declaration of `key` written for `side`
    /// competes for. `key` is the physical key the parser stored it under.
    pub(crate) fn key(self, key: PropertyKey, side: LogicalSide) -> PropertyKey {
        let Some(property) = BoxProperty::of_key(key) else {
            return key;
        };
        property.key(self.physical_of(property, side))
    }

    /// `value`, declared for `side`, moved to the physical side this mapping
    /// sends `side` to.
    pub(crate) fn value(self, value: PropertyValue, side: LogicalSide) -> PropertyValue {
        // The side of a margin, padding or inset; a size maps through `physical_of`.
        let to = self.physical(side);
        match value {
            PropertyValue::MarginTop(v)
            | PropertyValue::MarginRight(v)
            | PropertyValue::MarginBottom(v)
            | PropertyValue::MarginLeft(v) => match to {
                PhysicalSide::Top => PropertyValue::MarginTop(v),
                PhysicalSide::Right => PropertyValue::MarginRight(v),
                PhysicalSide::Bottom => PropertyValue::MarginBottom(v),
                PhysicalSide::Left => PropertyValue::MarginLeft(v),
            },
            PropertyValue::PaddingTop(v)
            | PropertyValue::PaddingRight(v)
            | PropertyValue::PaddingBottom(v)
            | PropertyValue::PaddingLeft(v) => match to {
                PhysicalSide::Top => PropertyValue::PaddingTop(v),
                PhysicalSide::Right => PropertyValue::PaddingRight(v),
                PhysicalSide::Bottom => PropertyValue::PaddingBottom(v),
                PhysicalSide::Left => PropertyValue::PaddingLeft(v),
            },
            PropertyValue::Top(v)
            | PropertyValue::Right(v)
            | PropertyValue::Bottom(v)
            | PropertyValue::Left(v) => match to {
                PhysicalSide::Top => PropertyValue::Top(v),
                PhysicalSide::Right => PropertyValue::Right(v),
                PhysicalSide::Bottom => PropertyValue::Bottom(v),
                PhysicalSide::Left => PropertyValue::Left(v),
            },
            PropertyValue::BorderTopWidth(v)
            | PropertyValue::BorderRightWidth(v)
            | PropertyValue::BorderBottomWidth(v)
            | PropertyValue::BorderLeftWidth(v) => match to {
                PhysicalSide::Top => PropertyValue::BorderTopWidth(v),
                PhysicalSide::Right => PropertyValue::BorderRightWidth(v),
                PhysicalSide::Bottom => PropertyValue::BorderBottomWidth(v),
                PhysicalSide::Left => PropertyValue::BorderLeftWidth(v),
            },
            PropertyValue::BorderTopStyle(v)
            | PropertyValue::BorderRightStyle(v)
            | PropertyValue::BorderBottomStyle(v)
            | PropertyValue::BorderLeftStyle(v) => match to {
                PhysicalSide::Top => PropertyValue::BorderTopStyle(v),
                PhysicalSide::Right => PropertyValue::BorderRightStyle(v),
                PhysicalSide::Bottom => PropertyValue::BorderBottomStyle(v),
                PhysicalSide::Left => PropertyValue::BorderLeftStyle(v),
            },
            PropertyValue::BorderTopColor(v)
            | PropertyValue::BorderRightColor(v)
            | PropertyValue::BorderBottomColor(v)
            | PropertyValue::BorderLeftColor(v) => match to {
                PhysicalSide::Top => PropertyValue::BorderTopColor(v),
                PhysicalSide::Right => PropertyValue::BorderRightColor(v),
                PhysicalSide::Bottom => PropertyValue::BorderBottomColor(v),
                PhysicalSide::Left => PropertyValue::BorderLeftColor(v),
            },
            PropertyValue::BorderTopWidthCssWide(k)
            | PropertyValue::BorderRightWidthCssWide(k)
            | PropertyValue::BorderBottomWidthCssWide(k)
            | PropertyValue::BorderLeftWidthCssWide(k) => match to {
                PhysicalSide::Top => PropertyValue::BorderTopWidthCssWide(k),
                PhysicalSide::Right => PropertyValue::BorderRightWidthCssWide(k),
                PhysicalSide::Bottom => PropertyValue::BorderBottomWidthCssWide(k),
                PhysicalSide::Left => PropertyValue::BorderLeftWidthCssWide(k),
            },
            PropertyValue::BorderTopStyleCssWide(k)
            | PropertyValue::BorderRightStyleCssWide(k)
            | PropertyValue::BorderBottomStyleCssWide(k)
            | PropertyValue::BorderLeftStyleCssWide(k) => match to {
                PhysicalSide::Top => PropertyValue::BorderTopStyleCssWide(k),
                PhysicalSide::Right => PropertyValue::BorderRightStyleCssWide(k),
                PhysicalSide::Bottom => PropertyValue::BorderBottomStyleCssWide(k),
                PhysicalSide::Left => PropertyValue::BorderLeftStyleCssWide(k),
            },
            PropertyValue::BorderTopColorCssWide(k)
            | PropertyValue::BorderRightColorCssWide(k)
            | PropertyValue::BorderBottomColorCssWide(k)
            | PropertyValue::BorderLeftColorCssWide(k) => match to {
                PhysicalSide::Top => PropertyValue::BorderTopColorCssWide(k),
                PhysicalSide::Right => PropertyValue::BorderRightColorCssWide(k),
                PhysicalSide::Bottom => PropertyValue::BorderBottomColorCssWide(k),
                PhysicalSide::Left => PropertyValue::BorderLeftColorCssWide(k),
            },
            PropertyValue::ContainIntrinsicWidth(v) | PropertyValue::ContainIntrinsicHeight(v) => {
                match self.physical_of(BoxProperty::ContainIntrinsic, side) {
                    PhysicalSide::Left | PhysicalSide::Right => {
                        PropertyValue::ContainIntrinsicWidth(v)
                    }
                    PhysicalSide::Top | PhysicalSide::Bottom => {
                        PropertyValue::ContainIntrinsicHeight(v)
                    }
                }
            }
            PropertyValue::MarginTopInherit
            | PropertyValue::MarginRightInherit
            | PropertyValue::MarginBottomInherit
            | PropertyValue::MarginLeftInherit => match to {
                PhysicalSide::Top => PropertyValue::MarginTopInherit,
                PhysicalSide::Right => PropertyValue::MarginRightInherit,
                PhysicalSide::Bottom => PropertyValue::MarginBottomInherit,
                PhysicalSide::Left => PropertyValue::MarginLeftInherit,
            },
            PropertyValue::Deferred(mut deferred) => {
                if let Some(property) = BoxProperty::of_key(deferred.key) {
                    let to = self.physical_of(property, side);
                    deferred.key = property.key(to);
                    deferred.property = SmolStr::new_static(property.name(to));
                }
                PropertyValue::Deferred(deferred)
            }
            other => other,
        }
    }
}

/// The box properties that have flow-relative longhands.
#[derive(Clone, Copy)]
enum BoxProperty {
    Margin,
    Padding,
    Inset,
    BorderWidth,
    BorderStyle,
    BorderColor,
    /// `contain-intrinsic-width` and `contain-intrinsic-height`: a side stands
    /// for its axis, the width for a left or right side and the height for a
    /// top or bottom one.
    ContainIntrinsic,
}

impl BoxProperty {
    fn of_key(key: PropertyKey) -> Option<Self> {
        match key {
            PropertyKey::MarginTop
            | PropertyKey::MarginRight
            | PropertyKey::MarginBottom
            | PropertyKey::MarginLeft => Some(Self::Margin),
            PropertyKey::PaddingTop
            | PropertyKey::PaddingRight
            | PropertyKey::PaddingBottom
            | PropertyKey::PaddingLeft => Some(Self::Padding),
            PropertyKey::Top | PropertyKey::Right | PropertyKey::Bottom | PropertyKey::Left => {
                Some(Self::Inset)
            }
            PropertyKey::BorderTopWidth
            | PropertyKey::BorderRightWidth
            | PropertyKey::BorderBottomWidth
            | PropertyKey::BorderLeftWidth => Some(Self::BorderWidth),
            PropertyKey::BorderTopStyle
            | PropertyKey::BorderRightStyle
            | PropertyKey::BorderBottomStyle
            | PropertyKey::BorderLeftStyle => Some(Self::BorderStyle),
            PropertyKey::BorderTopColor
            | PropertyKey::BorderRightColor
            | PropertyKey::BorderBottomColor
            | PropertyKey::BorderLeftColor => Some(Self::BorderColor),
            PropertyKey::ContainIntrinsicWidth | PropertyKey::ContainIntrinsicHeight => {
                Some(Self::ContainIntrinsic)
            }
            _ => None,
        }
    }

    fn key(self, side: PhysicalSide) -> PropertyKey {
        match (self, side) {
            (Self::Margin, PhysicalSide::Top) => PropertyKey::MarginTop,
            (Self::Margin, PhysicalSide::Right) => PropertyKey::MarginRight,
            (Self::Margin, PhysicalSide::Bottom) => PropertyKey::MarginBottom,
            (Self::Margin, PhysicalSide::Left) => PropertyKey::MarginLeft,
            (Self::Padding, PhysicalSide::Top) => PropertyKey::PaddingTop,
            (Self::Padding, PhysicalSide::Right) => PropertyKey::PaddingRight,
            (Self::Padding, PhysicalSide::Bottom) => PropertyKey::PaddingBottom,
            (Self::Padding, PhysicalSide::Left) => PropertyKey::PaddingLeft,
            (Self::Inset, PhysicalSide::Top) => PropertyKey::Top,
            (Self::Inset, PhysicalSide::Right) => PropertyKey::Right,
            (Self::Inset, PhysicalSide::Bottom) => PropertyKey::Bottom,
            (Self::Inset, PhysicalSide::Left) => PropertyKey::Left,
            (Self::BorderWidth, PhysicalSide::Top) => PropertyKey::BorderTopWidth,
            (Self::BorderWidth, PhysicalSide::Right) => PropertyKey::BorderRightWidth,
            (Self::BorderWidth, PhysicalSide::Bottom) => PropertyKey::BorderBottomWidth,
            (Self::BorderWidth, PhysicalSide::Left) => PropertyKey::BorderLeftWidth,
            (Self::BorderStyle, PhysicalSide::Top) => PropertyKey::BorderTopStyle,
            (Self::BorderStyle, PhysicalSide::Right) => PropertyKey::BorderRightStyle,
            (Self::BorderStyle, PhysicalSide::Bottom) => PropertyKey::BorderBottomStyle,
            (Self::BorderStyle, PhysicalSide::Left) => PropertyKey::BorderLeftStyle,
            (Self::BorderColor, PhysicalSide::Top) => PropertyKey::BorderTopColor,
            (Self::BorderColor, PhysicalSide::Right) => PropertyKey::BorderRightColor,
            (Self::BorderColor, PhysicalSide::Bottom) => PropertyKey::BorderBottomColor,
            (Self::BorderColor, PhysicalSide::Left) => PropertyKey::BorderLeftColor,
            (Self::ContainIntrinsic, PhysicalSide::Left | PhysicalSide::Right) => {
                PropertyKey::ContainIntrinsicWidth
            }
            (Self::ContainIntrinsic, PhysicalSide::Top | PhysicalSide::Bottom) => {
                PropertyKey::ContainIntrinsicHeight
            }
        }
    }

    fn name(self, side: PhysicalSide) -> &'static str {
        match (self, side) {
            (Self::Margin, PhysicalSide::Top) => "margin-top",
            (Self::Margin, PhysicalSide::Right) => "margin-right",
            (Self::Margin, PhysicalSide::Bottom) => "margin-bottom",
            (Self::Margin, PhysicalSide::Left) => "margin-left",
            (Self::Padding, PhysicalSide::Top) => "padding-top",
            (Self::Padding, PhysicalSide::Right) => "padding-right",
            (Self::Padding, PhysicalSide::Bottom) => "padding-bottom",
            (Self::Padding, PhysicalSide::Left) => "padding-left",
            (Self::Inset, PhysicalSide::Top) => "top",
            (Self::Inset, PhysicalSide::Right) => "right",
            (Self::Inset, PhysicalSide::Bottom) => "bottom",
            (Self::Inset, PhysicalSide::Left) => "left",
            (Self::BorderWidth, PhysicalSide::Top) => "border-top-width",
            (Self::BorderWidth, PhysicalSide::Right) => "border-right-width",
            (Self::BorderWidth, PhysicalSide::Bottom) => "border-bottom-width",
            (Self::BorderWidth, PhysicalSide::Left) => "border-left-width",
            (Self::BorderStyle, PhysicalSide::Top) => "border-top-style",
            (Self::BorderStyle, PhysicalSide::Right) => "border-right-style",
            (Self::BorderStyle, PhysicalSide::Bottom) => "border-bottom-style",
            (Self::BorderStyle, PhysicalSide::Left) => "border-left-style",
            (Self::BorderColor, PhysicalSide::Top) => "border-top-color",
            (Self::BorderColor, PhysicalSide::Right) => "border-right-color",
            (Self::BorderColor, PhysicalSide::Bottom) => "border-bottom-color",
            (Self::BorderColor, PhysicalSide::Left) => "border-left-color",
            (Self::ContainIntrinsic, PhysicalSide::Left | PhysicalSide::Right) => {
                "contain-intrinsic-width"
            }
            (Self::ContainIntrinsic, PhysicalSide::Top | PhysicalSide::Bottom) => {
                "contain-intrinsic-height"
            }
        }
    }
}

/// The flow-relative side the longhand `name` is written for, when `name`
/// is one of the margin, padding, inset, border or intrinsic size
/// flow-relative longhands, or one of the single-edge border shorthands
/// (`border-block-start` and its siblings), whose longhands all share the
/// side.
pub(crate) fn longhand_side(name: &str) -> Option<LogicalSide> {
    const LONGHANDS: [(&str, LogicalSide); 30] = [
        ("margin-inline-start", LogicalSide::InlineStart),
        ("margin-inline-end", LogicalSide::InlineEnd),
        ("margin-block-start", LogicalSide::BlockStart),
        ("margin-block-end", LogicalSide::BlockEnd),
        ("padding-inline-start", LogicalSide::InlineStart),
        ("padding-inline-end", LogicalSide::InlineEnd),
        ("padding-block-start", LogicalSide::BlockStart),
        ("padding-block-end", LogicalSide::BlockEnd),
        ("inset-inline-start", LogicalSide::InlineStart),
        ("inset-inline-end", LogicalSide::InlineEnd),
        ("inset-block-start", LogicalSide::BlockStart),
        ("inset-block-end", LogicalSide::BlockEnd),
        ("contain-intrinsic-inline-size", LogicalSide::InlineStart),
        ("contain-intrinsic-block-size", LogicalSide::BlockStart),
        ("border-block-start", LogicalSide::BlockStart),
        ("border-block-end", LogicalSide::BlockEnd),
        ("border-inline-start", LogicalSide::InlineStart),
        ("border-inline-end", LogicalSide::InlineEnd),
        ("border-block-start-width", LogicalSide::BlockStart),
        ("border-block-end-width", LogicalSide::BlockEnd),
        ("border-inline-start-width", LogicalSide::InlineStart),
        ("border-inline-end-width", LogicalSide::InlineEnd),
        ("border-block-start-style", LogicalSide::BlockStart),
        ("border-block-end-style", LogicalSide::BlockEnd),
        ("border-inline-start-style", LogicalSide::InlineStart),
        ("border-inline-end-style", LogicalSide::InlineEnd),
        ("border-block-start-color", LogicalSide::BlockStart),
        ("border-block-end-color", LogicalSide::BlockEnd),
        ("border-inline-start-color", LogicalSide::InlineStart),
        ("border-inline-end-color", LogicalSide::InlineEnd),
    ];
    // Every declaration passes through here; most names are shorter or
    // longer than any of the thirty, so their length rules them out first.
    if !matches!(name.len(), 15..=25 | 28..=29) {
        return None;
    }
    LONGHANDS
        .iter()
        .find(|(longhand, _)| longhand.eq_ignore_ascii_case(name))
        .map(|&(_, side)| side)
}

#[cfg(test)]
mod tests;
