use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::protocol::{AvailableSpace, BfcSlot, FloatDirection};
use raikiri_layout_geometry::{Point, Size};
use raikiri_layout_style::ToTaffy;
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
pub(crate) struct FloatContext {
    inner: taffy::FloatContext,
    /// The width last set with [`Self::set_width`], which the library keeps
    /// private.
    width: f32,
    /// The parts of floats that only lines are broken against: the library
    /// knows nothing of them, so neither floats nor boxes placed after them
    /// move for them (see [`Self::extend_for_lines`]).
    line_extensions: Vec<LineExtension>,
}

/// A band below a float that the lines of the flow are broken against as if
/// the float reached down through it, in the coordinates of the floats.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LineExtension {
    /// The side the float sits on.
    pub(crate) direction: FloatDirection,
    /// The edge of the float's margin box that faces the lines: its right
    /// edge for a left float, its left edge for a right one.
    pub(crate) edge: f32,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
}

impl FloatContext {
    /// A context that holds no float and has no width.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            inner: taffy::FloatContext::new(),
            width: 0.0,
            line_extensions: Vec::new(),
        }
    }

    /// Breaks the lines between `top` and `bottom` against the float placed
    /// at `position` with the margin-box `size` as if it reached down that
    /// far, without moving any float or box placed after it.
    pub(crate) fn extend_for_lines(
        &mut self,
        position: Point<f32>,
        size: Size<f32>,
        direction: FloatDirection,
        bottom: f32,
    ) {
        let top = position.y + size.height;
        if bottom <= top {
            return;
        }
        let edge = match direction {
            FloatDirection::Left => position.x + size.width,
            FloatDirection::Right => position.x,
        };
        self.line_extensions.push(LineExtension {
            direction,
            edge,
            top,
            bottom,
        });
    }

    /// The bands below floats that lines are broken against.
    pub(crate) fn line_extensions(&self) -> &[LineExtension] {
        &self.line_extensions
    }

    /// Set the width of the formatting context, which the room beside the
    /// floats is measured in.
    #[inline]
    pub(crate) fn set_width(&mut self, width: f32) {
        self.width = width;
        self.inner.set_width(width);
    }

    /// The width of the formatting context.
    #[inline]
    pub(crate) fn width(&self) -> f32 {
        self.width
    }

    /// Whether the context holds any float.
    #[inline]
    pub(crate) fn has_floats(&self) -> bool {
        self.inner.has_floats()
    }

    /// Whether the context holds a float that reaches `min_y` or below.
    #[inline]
    pub(crate) fn has_active_floats(&self, min_y: f32) -> bool {
        self.inner.has_active_floats(min_y)
    }

    /// Place a floated box of the border-box size `floated_box` no higher than
    /// `min_y`, and return its position.
    ///
    /// `containing_block_insets` are how far the containing block is inset from
    /// the left and the right edge of the formatting context.
    ///
    /// `clear` moves the float below the bottom of the floats on the sides it
    /// names, and no further (CSS 2 §9.5.2): it may sit beside an earlier float
    /// on the other side, even one that is itself below a cleared float. The
    /// library also skips the band of segments after the last float on the
    /// cleared side, which can move the float below a float on the side it
    /// does not clear, so the clearance is passed to it as the least `y`
    /// instead (see [`Self::clearance_floor`]).
    #[inline]
    pub(crate) fn place_floated_box(
        &mut self,
        floated_box: Size<f32>,
        min_y: f32,
        containing_block_insets: [f32; 2],
        direction: FloatDirection,
        clear: Clear,
    ) -> Point<f32> {
        Point::from_taffy(self.inner.place_floated_box(
            floated_box.into_taffy(),
            self.clearance_floor(min_y, clear),
            containing_block_insets,
            direction.into_taffy(),
            taffy::Clear::None,
        ))
    }

    /// The room for a box that establishes a formatting context, starting no
    /// higher than `min_y`, after the segment `after` if there is one.
    ///
    /// `margins` are the side margins of the box, which the room is measured
    /// inside of, and `direction` is its direction.
    ///
    /// `clear` moves the room below the bottom of the floats on the sides it
    /// names, and no further, as for [`Self::place_floated_box`]: the room may
    /// be beside a float on the other side.
    pub(crate) fn find_bfc_slot(
        &self,
        min_y: f32,
        containing_block_insets: [f32; 2],
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        after: Option<usize>,
    ) -> BfcSlot {
        BfcSlot::from_taffy(self.inner.find_bfc_slot(
            self.clearance_floor(min_y, clear),
            containing_block_insets,
            margins,
            direction.to_taffy(),
            taffy::Clear::None,
            after,
        ))
    }

    /// The least `y` that clears the floats `clear` names, given the least `y`
    /// the caller asks for.
    ///
    /// The bottoms of the cleared floats are all that clearance needs: the
    /// library raises the least `y` to them as well, but it also starts the
    /// search after the segment range it last recorded for the cleared side,
    /// plus one more segment. That range is half-open, so the extra segment is
    /// the one right below the cleared float, and a float on the other side
    /// that sits there makes the box skip past it. Passing the bottoms as the
    /// least `y` and no clearance to the library leaves only the bottoms to
    /// decide.
    #[inline]
    pub(super) fn clearance_floor(&self, min_y: f32, clear: Clear) -> f32 {
        self.inner
            .cleared_threshold(clear.to_taffy())
            .map_or(min_y, |bottom| min_y.max(bottom))
    }

    /// How low the floats that `clear` clears reach, if there are any.
    #[inline]
    pub(crate) fn cleared_threshold(&self, clear: Clear) -> Option<f32> {
        self.inner.cleared_threshold(clear.to_taffy())
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
        self.0
            .add_float(width, direction.into_taffy(), clear.to_taffy());
    }

    /// The width the floats added so far contribute.
    #[inline]
    pub(crate) fn result(&self) -> f32 {
        self.0.result()
    }
}
