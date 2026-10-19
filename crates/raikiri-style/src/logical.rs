//! Flow-relative margin, padding and inset longhands.
//!
//! The parser stores `margin-inline-start` and its siblings (the padding and
//! inset ones too) under the physical longhand they map to in `horizontal-tb`
//! with `direction: ltr`, and tags the declaration with the flow-relative side
//! it was written for.
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
//! layout.
//!
//! `text-orientation: upright` makes the used direction `ltr` in vertical
//! writing modes (CSS Writing Modes 4 §5.1), and both mappings use that used
//! direction.

use smol_str::SmolStr;

use crate::property::{Direction, PropertyKey, PropertyValue, WritingMode};

/// The flow-relative side a margin, padding or inset declaration was written for.
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
}

impl FlowMapping {
    /// `horizontal-tb` with `direction: ltr`.
    #[cfg(test)]
    pub(crate) const HORIZONTAL: Self = Self {
        writing_mode: WritingMode::HorizontalTb,
        direction: Direction::Ltr,
    };

    /// Whether the inline axis is vertical.
    pub(crate) fn is_vertical(self) -> bool {
        self.writing_mode.is_vertical()
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
        property.key(self.physical(side))
    }

    /// `value`, declared for `side`, moved to the physical side this mapping
    /// sends `side` to.
    pub(crate) fn value(self, value: PropertyValue, side: LogicalSide) -> PropertyValue {
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
        }
    }
}

/// The flow-relative side the longhand `name` is written for, when `name`
/// is one of the margin, padding or inset flow-relative longhands.
pub(crate) fn longhand_side(name: &str) -> Option<LogicalSide> {
    const LONGHANDS: [(&str, LogicalSide); 12] = [
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
    ];
    // Every declaration passes through here; most names are shorter or
    // longer than any of the twelve, so their length rules them out first.
    if !(15..=20).contains(&name.len()) {
        return None;
    }
    LONGHANDS
        .iter()
        .find(|(longhand, _)| longhand.eq_ignore_ascii_case(name))
        .map(|&(_, side)| side)
}

#[cfg(test)]
mod tests;
