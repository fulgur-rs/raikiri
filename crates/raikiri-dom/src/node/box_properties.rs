//! Canonical CSS box identity shared by layout and the Taffy adapter.

use raikiri_layout_style::values::{Display, Position};
use raikiri_style::ComputedValues;
use raikiri_style::property::{DisplayValue, PositionValue};
use raikiri_traits::NodeKind;

/// CSS Display 3 §2.8: blockify the document element while preserving its
/// inner formatting algorithm. Unlike ordinary contents boxes, a contents
/// root computes to block and retains its own box properties.
fn blockify_root_display(display: DisplayValue) -> DisplayValue {
    match display {
        DisplayValue::Inline | DisplayValue::Contents => DisplayValue::Block,
        DisplayValue::InlineBlock => DisplayValue::FlowRoot,
        DisplayValue::InlineFlex => DisplayValue::Flex,
        DisplayValue::InlineGrid => DisplayValue::Grid,
        DisplayValue::InlineTable => DisplayValue::Table,
        DisplayValue::Ruby => DisplayValue::BlockRuby,
        DisplayValue::TableRowGroup
        | DisplayValue::TableHeaderGroup
        | DisplayValue::TableFooterGroup
        | DisplayValue::TableRow
        | DisplayValue::TableColumnGroup
        | DisplayValue::TableColumn
        | DisplayValue::TableCell
        | DisplayValue::TableCaption
        | DisplayValue::RubyBase
        | DisplayValue::RubyText
        | DisplayValue::RubyBaseContainer
        | DisplayValue::RubyTextContainer => DisplayValue::Block,
        other => other,
    }
}

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

    pub(crate) fn blockify_root(&mut self) {
        self.display = blockify_root_display(self.display);
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

    /// Lower CSS display to the box types layout supports.
    pub(crate) fn layout_display(&self, multicol: bool, kind: NodeKind, root: bool) -> Display {
        // Non-elements have no CSS display property. Preserve the native
        // default used by programmatic document layout instead of lowering
        // their inline text identity as an element's block box.
        if kind != NodeKind::Element && self.display == DisplayValue::Inline {
            return Display::Flex;
        }
        let display = match self.display {
            DisplayValue::None => Display::None,
            DisplayValue::FlowRoot => Display::FlowRoot,
            display if display.is_flex_container() => Display::Flex,
            display if display.is_grid_container() => Display::Grid,
            _ => Display::Block,
        };
        if (multicol || root) && display == Display::Block {
            Display::FlowRoot
        } else {
            display
        }
    }

    pub(crate) fn layout_position(&self) -> Position {
        if self.display == DisplayValue::Contents {
            return Position::Static;
        }
        match self.position() {
            PositionValue::Absolute | PositionValue::Fixed => Position::Absolute,
            PositionValue::Relative => Position::Relative,
            PositionValue::Sticky => Position::Sticky,
            _ => Position::Static,
        }
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

    pub(crate) fn blockify_root(mut self) -> Self {
        self.display = blockify_root_display(self.display);
        self
    }

    pub(crate) fn display(self) -> DisplayValue {
        self.display
    }

    pub(crate) fn position(self) -> &'a PositionValue {
        self.position
    }
}
