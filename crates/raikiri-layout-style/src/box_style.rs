//! The style of a box, reduced to what the block primitives and leaf sizing read.

use crate::values::{
    AlignContent, AlignItems, AlignSelf, BoxGenerationMode, BoxSizing, Clear, Contain, Dimension,
    Direction, Display, Float, LengthPercentage, LengthPercentageAuto, Overflow, Position,
};
use raikiri_layout_geometry::{Edges, Point, Size};

/// A snapshot of the used style of one box.
///
/// It holds the properties that block flow, the sizing of replaced boxes and
/// leaf measurement read, and none of the flex or grid properties. A caller
/// that needs to adjust a style for one measurement takes a copy and writes
/// the copy.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxStyle {
    /// The layout of the children. Block flow tells hidden, block and other
    /// apart, and a snapshot of a view holds `Flex` for every box that is none
    /// of the first two.
    pub display: Display,
    /// Whether the box is a table in a block formatting context.
    pub item_is_table: bool,
    /// Whether the box is a replaced element.
    pub item_is_replaced: bool,
    /// Whether the box is a replaced element whose automatic size may shrink.
    pub item_is_compressible_replaced: bool,
    /// What the size properties measure.
    pub box_sizing: BoxSizing,
    /// The inline direction.
    pub direction: Direction,
    /// The overflow of both axes.
    pub overflow: Point<Overflow>,
    /// The layout and paint containment.
    pub contain: Contain,
    /// The position scheme.
    pub position: Position,
    /// The inset edges.
    pub inset: Edges<LengthPercentageAuto>,
    /// The preferred size.
    pub size: Size<Dimension>,
    /// The minimum size.
    pub min_size: Size<LengthPercentageAuto>,
    /// The maximum size.
    pub max_size: Size<LengthPercentageAuto>,
    /// The preferred aspect ratio.
    pub aspect_ratio: Option<f32>,
    /// The margin edges.
    pub margin: Edges<LengthPercentageAuto>,
    /// The padding edges.
    pub padding: Edges<LengthPercentage>,
    /// The border edges.
    pub border: Edges<LengthPercentage>,
    /// The float side.
    pub float: Float,
    /// The floats the box clears.
    pub clear: Clear,
    /// The default inline alignment of the items of the box.
    pub justify_items: AlignItems,
    /// The content alignment of the box.
    pub align_content: AlignContent,
    /// The block-axis alignment of the box as an item.
    pub align_self: Option<AlignSelf>,
    /// The inline-axis alignment of the box as an item.
    pub justify_self: Option<AlignSelf>,
}

impl BoxStyle {
    /// Whether the box lays its children out as a block: `display: block`, and
    /// not `flow-root`, which establishes a formatting context of its own.
    #[inline(always)]
    pub fn is_block(&self) -> bool {
        matches!(self.display, Display::Block)
    }

    /// Whether the box generates boxes at all.
    #[inline(always)]
    pub fn box_generation_mode(&self) -> BoxGenerationMode {
        match self.display {
            Display::None => BoxGenerationMode::None,
            Display::Block | Display::FlowRoot | Display::Flex | Display::Grid => {
                BoxGenerationMode::Normal
            }
        }
    }

    /// Whether the box is a replaced element, or one whose automatic size may
    /// shrink.
    #[inline(always)]
    pub fn is_compressible_replaced(&self) -> bool {
        self.item_is_replaced || self.item_is_compressible_replaced
    }
}

impl Default for BoxStyle {
    /// An ordinary box: no containment, no insets or edges, automatic size.
    fn default() -> Self {
        let zero = Edges {
            left: LengthPercentage::ZERO,
            right: LengthPercentage::ZERO,
            top: LengthPercentage::ZERO,
            bottom: LengthPercentage::ZERO,
        };
        let auto = Edges {
            left: LengthPercentageAuto::auto(),
            right: LengthPercentageAuto::auto(),
            top: LengthPercentageAuto::auto(),
            bottom: LengthPercentageAuto::auto(),
        };
        let auto_size = Size {
            width: LengthPercentageAuto::auto(),
            height: LengthPercentageAuto::auto(),
        };
        Self {
            display: Display::Flex,
            item_is_table: false,
            item_is_replaced: false,
            item_is_compressible_replaced: false,
            box_sizing: BoxSizing::BorderBox,
            direction: Direction::Ltr,
            overflow: Point {
                x: Overflow::Visible,
                y: Overflow::Visible,
            },
            contain: Contain::NONE,
            position: Position::Static,
            inset: auto,
            size: Size {
                width: Dimension::auto(),
                height: Dimension::auto(),
            },
            min_size: auto_size,
            max_size: auto_size,
            aspect_ratio: None,
            margin: Edges {
                left: LengthPercentageAuto::ZERO,
                right: LengthPercentageAuto::ZERO,
                top: LengthPercentageAuto::ZERO,
                bottom: LengthPercentageAuto::ZERO,
            },
            padding: zero,
            border: zero,
            float: Float::None,
            clear: Clear::None,
            justify_items: AlignItems::NORMAL,
            align_content: AlignContent::NORMAL,
            align_self: None,
            justify_self: None,
        }
    }
}
