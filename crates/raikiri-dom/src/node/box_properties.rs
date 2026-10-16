//! Canonical CSS box identity shared by layout and the Taffy adapter.

use raikiri_style::ComputedValues;
use raikiri_style::property::{DisplayValue, PositionValue};
use raikiri_traits::NodeKind;

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

    /// Preserve the display and position accepted by programmatic constructors.
    pub(crate) fn from_taffy_style(style: &taffy::Style) -> Self {
        let display = match style.display {
            taffy::Display::None => DisplayValue::None,
            taffy::Display::Block => DisplayValue::Block,
            taffy::Display::FlowRoot => DisplayValue::FlowRoot,
            taffy::Display::Flex => DisplayValue::Flex,
            taffy::Display::Grid => DisplayValue::Grid,
        };
        let position = match style.position {
            taffy::Position::Static => PositionValue::Static,
            taffy::Position::Relative => PositionValue::Relative,
            taffy::Position::Fixed => PositionValue::Fixed,
            taffy::Position::Sticky => PositionValue::Sticky,
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

    pub(crate) fn borrowed(&self) -> BoxPropertiesRef<'_> {
        BoxPropertiesRef {
            display: self.display,
            position: &self.position,
        }
    }

    /// Lower CSS display to the box types supported by Taffy.
    pub(crate) fn taffy_display(
        &self,
        multicol: bool,
        kind: NodeKind,
        root: bool,
    ) -> taffy::Display {
        // Non-elements have no CSS display property. Preserve the native
        // default used by programmatic document layout instead of lowering
        // their inline text identity as an element's block box.
        if kind != NodeKind::Element && self.display == DisplayValue::Inline {
            return taffy::Display::DEFAULT;
        }
        let display = match self.display {
            DisplayValue::None => taffy::Display::None,
            DisplayValue::FlowRoot => taffy::Display::FlowRoot,
            display if display.is_flex_container() => taffy::Display::Flex,
            display if display.is_grid_container() => taffy::Display::Grid,
            _ => taffy::Display::Block,
        };
        if (multicol || root) && display == taffy::Display::Block {
            taffy::Display::FlowRoot
        } else {
            display
        }
    }

    pub(crate) fn taffy_position(&self) -> taffy::Position {
        if self.display == DisplayValue::Contents {
            return taffy::Position::Static;
        }
        match self.position() {
            PositionValue::Absolute | PositionValue::Fixed => taffy::Position::Absolute,
            PositionValue::Relative => taffy::Position::Relative,
            PositionValue::Sticky => taffy::Position::Sticky,
            _ => taffy::Position::Static,
        }
    }

    /// Refresh the derived identity exported to Taffy and the public style API.
    pub(crate) fn apply_to_style(
        &self,
        style: &mut taffy::Style,
        multicol: bool,
        kind: NodeKind,
        root: bool,
    ) {
        style.display = self.taffy_display(multicol, kind, root);
        style.position = self.taffy_position();
    }
}

/// A borrowed identity for one explicit style query or a captured layout box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxPropertiesRef<'a> {
    display: DisplayValue,
    position: &'a PositionValue,
}

impl<'a> BoxPropertiesRef<'a> {
    pub(crate) fn from_computed(values: &'a ComputedValues) -> Self {
        Self {
            display: values.display,
            position: &values.position,
        }
    }

    pub(crate) fn display(self) -> DisplayValue {
        self.display
    }

    pub(crate) fn position(self) -> &'a PositionValue {
        self.position
    }
}
