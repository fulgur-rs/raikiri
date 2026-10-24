use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::protocol::{AvailableSpace, BfcSlot, FloatDirection};
use raikiri_layout_geometry::{Point, Size};
use raikiri_layout_style::values::{Clear, Direction};

/// Where the floats of one block formatting context are.
///
/// The placement state is the library's, wrapped: the library keeps it private
/// and extends it as floats are placed, and the layout asks it for the room
/// that is left. Placement and queries take and return the protocol's types
/// and convert them on the way in and out.
///
/// A clone copies every list the context holds, so the layout shares one value
/// between the snapshots of a flow and clones it only when a float is placed
/// or the width changes.
#[derive(Debug, Clone)]
pub(crate) struct FloatContext(taffy::FloatContext);

impl FloatContext {
    /// A context that holds no float and has no width.
    #[inline]
    pub(crate) fn new() -> Self {
        Self(taffy::FloatContext::new())
    }

    /// Set the width of the formatting context, which the room beside the
    /// floats is measured in.
    #[inline]
    pub(crate) fn set_width(&mut self, width: f32) {
        self.0.set_width(width);
    }

    /// Whether the context holds any float.
    #[inline]
    pub(crate) fn has_floats(&self) -> bool {
        self.0.has_floats()
    }

    /// Whether the context holds a float that reaches `min_y` or below.
    #[inline]
    pub(crate) fn has_active_floats(&self, min_y: f32) -> bool {
        self.0.has_active_floats(min_y)
    }

    /// Place a floated box of the border-box size `floated_box` no higher than
    /// `min_y`, and return its position.
    ///
    /// `containing_block_insets` are how far the containing block is inset from
    /// the left and the right edge of the formatting context.
    #[inline]
    pub(crate) fn place_floated_box(
        &mut self,
        floated_box: Size<f32>,
        min_y: f32,
        containing_block_insets: [f32; 2],
        direction: FloatDirection,
        clear: Clear,
    ) -> Point<f32> {
        Point::from_taffy(self.0.place_floated_box(
            floated_box.into_taffy(),
            min_y,
            containing_block_insets,
            direction.into_taffy(),
            clear,
        ))
    }

    /// The room for a box that establishes a formatting context, starting no
    /// higher than `min_y`, after the segment `after` if there is one.
    ///
    /// `margins` are the side margins of the box, which the room is measured
    /// inside of, and `direction` is its direction.
    #[inline]
    pub(crate) fn find_bfc_slot(
        &self,
        min_y: f32,
        containing_block_insets: [f32; 2],
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        after: Option<usize>,
    ) -> BfcSlot {
        BfcSlot::from_taffy(self.0.find_bfc_slot(
            min_y,
            containing_block_insets,
            margins,
            direction,
            clear,
            after,
        ))
    }

    /// How low the floats that `clear` clears reach, if there are any.
    #[inline]
    pub(crate) fn cleared_threshold(&self, clear: Clear) -> Option<f32> {
        self.0.cleared_threshold(clear)
    }
}

/// A sum of the widths that a run of floats adds to the intrinsic width of
/// their block.
///
/// The calculation is the library's, wrapped: floats are added one by one in
/// document order and the contribution is read at the end.
pub(crate) struct FloatIntrinsicWidthCalculator(taffy::FloatIntrinsicWidthCalculator);

impl FloatIntrinsicWidthCalculator {
    /// A calculator for a block laid out in `available_width`.
    #[inline]
    pub(crate) fn new(available_width: AvailableSpace) -> Self {
        Self(taffy::FloatIntrinsicWidthCalculator::new(
            available_width.into_taffy(),
        ))
    }

    /// Add a float of the border-box `width`, placed on the side `direction`
    /// and clearing the floats that `clear` names.
    #[inline]
    pub(crate) fn add_float(&mut self, width: f32, direction: FloatDirection, clear: Clear) {
        self.0.add_float(width, direction.into_taffy(), clear);
    }

    /// The width the floats added so far contribute.
    #[inline]
    pub(crate) fn result(&self) -> f32 {
        self.0.result()
    }
}
