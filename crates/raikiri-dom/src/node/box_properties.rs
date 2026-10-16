//! Canonical CSS box identity shared by layout and the Taffy adapter.

use raikiri_style::property::{DisplayValue, PositionValue};

/// The computed display and position captured before layout classifies a box.
#[derive(Debug, Clone)]
pub(crate) struct BoxProperties {
    display: DisplayValue,
    position: PositionValue,
}

impl BoxProperties {
    pub(crate) fn new(display: DisplayValue, position: PositionValue) -> Self {
        Self { display, position }
    }

    /// Preserve the style accepted by the programmatic document constructors.
    /// Taffy merges static and relative positioning; retain the CSS initial
    /// value until the bridge captures the computed position.
    pub(crate) fn from_taffy_style(style: &taffy::Style) -> Self {
        let display = match style.display {
            taffy::Display::None => DisplayValue::None,
            taffy::Display::Block => DisplayValue::Block,
            taffy::Display::FlowRoot => DisplayValue::FlowRoot,
            taffy::Display::Flex => DisplayValue::Flex,
            taffy::Display::Grid => DisplayValue::Grid,
        };
        let position = match style.position {
            taffy::Position::Relative => PositionValue::Static,
            taffy::Position::Absolute => PositionValue::Absolute,
        };
        Self::new(display, position)
    }

    pub(crate) fn display(&self) -> DisplayValue {
        self.display
    }

    pub(crate) fn position(&self) -> &PositionValue {
        &self.position
    }

    /// Lower CSS display to the box types supported by Taffy.
    pub(crate) fn taffy_display(&self, multicol: bool) -> taffy::Display {
        let display = match self.display {
            DisplayValue::None => taffy::Display::None,
            DisplayValue::FlowRoot => taffy::Display::FlowRoot,
            display if display.is_flex_container() => taffy::Display::Flex,
            display if display.is_grid_container() => taffy::Display::Grid,
            _ => taffy::Display::Block,
        };
        if multicol && display == taffy::Display::Block {
            taffy::Display::FlowRoot
        } else {
            display
        }
    }

    pub(crate) fn taffy_position(&self) -> taffy::Position {
        if self.display == DisplayValue::Contents {
            return taffy::Position::Relative;
        }
        match self.position() {
            PositionValue::Absolute | PositionValue::Fixed => taffy::Position::Absolute,
            _ => taffy::Position::Relative,
        }
    }

    /// Refresh the derived identity exported to Taffy and the public style API.
    pub(crate) fn apply_to_style(&self, style: &mut taffy::Style, multicol: bool) {
        style.display = self.taffy_display(multicol);
        style.position = self.taffy_position();
    }
}
