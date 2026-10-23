use crate::{Edges, Point, Size};

/// The placement of one box, as the layout engine computed it.
///
/// Every `f32` field is finite and within `[-1e7, 1e7]`: the layout engine
/// replaces `NaN` with `0.0` and clamps the rest before it stores a layout, so
/// a reader never sees an infinity or a value that overflows when summed with a
/// few of its neighbours. Only finiteness and the magnitude limit are
/// guaranteed. The box model containment of CSS Box 3 (content inside padding
/// inside border) is not preserved, because the fields are clamped
/// independently: if `size.width` and `padding.left` + `padding.right` both
/// saturate, `size.width - padding.left - padding.right` can be negative.
/// `order` cannot be non-finite and is outside the contract.
///
/// The fields use the same coordinate convention as the rest of the layout
/// result: `location` is the top-left corner of the border box measured from
/// the top-left corner of the border box of the layout parent, `size` is the
/// border box, and `border`, `padding` and `margin` are the used widths of
/// those edges.
///
/// The origin that `scrollable_overflow_rect` is measured from is not settled
/// for boxes in a vertical writing mode or with `direction: rtl`. For a
/// horizontal left-to-right box it is the top-left corner of the padding box:
/// `left` and `top` are at most zero and `right` and `bottom` at least zero,
/// and `right` and `bottom` are the reachable extent of the content.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BoxLayout {
    /// The paint order of the box among its siblings: a box with a higher
    /// value is painted on top of one with a lower value.
    pub order: u32,
    /// The top-left corner of the border box.
    pub location: Point<f32>,
    /// The border box.
    pub size: Size<f32>,
    /// How far the content of the box reaches on each side, from the origin
    /// described above.
    pub scrollable_overflow_rect: Edges<f32>,
    /// The size of the scrollbars; zero for an axis that has none.
    pub scrollbar_size: Size<f32>,
    /// The widths of the border.
    pub border: Edges<f32>,
    /// The widths of the padding.
    pub padding: Edges<f32>,
    /// The widths of the margin.
    pub margin: Edges<f32>,
}

impl Default for BoxLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl BoxLayout {
    /// A layout with `order` zero and every other value zero.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_order(0)
    }

    /// A layout with the given `order` and every other value zero.
    #[must_use]
    pub const fn with_order(order: u32) -> Self {
        Self {
            order,
            location: Point::ZERO,
            size: Size::ZERO,
            scrollable_overflow_rect: Edges::ZERO,
            scrollbar_size: Size::ZERO,
            border: Edges::ZERO,
            padding: Edges::ZERO,
            margin: Edges::ZERO,
        }
    }

    /// The width of the content box.
    #[inline]
    pub fn content_box_width(&self) -> f32 {
        self.size.width
            - self.padding.left
            - self.padding.right
            - self.border.left
            - self.border.right
    }

    /// The height of the content box.
    #[inline]
    pub fn content_box_height(&self) -> f32 {
        self.size.height
            - self.padding.top
            - self.padding.bottom
            - self.border.top
            - self.border.bottom
    }

    /// The size of the content box.
    #[inline]
    pub fn content_box_size(&self) -> Size<f32> {
        Size {
            width: self.content_box_width(),
            height: self.content_box_height(),
        }
    }

    /// The x offset of the content box from the border box of the layout
    /// parent.
    #[inline]
    pub fn content_box_x(&self) -> f32 {
        self.location.x + self.border.left + self.padding.left
    }

    /// The y offset of the content box from the border box of the layout
    /// parent.
    #[inline]
    pub fn content_box_y(&self) -> f32 {
        self.location.y + self.border.top + self.padding.top
    }

    /// The largest horizontal scroll offset: the reachable extent of the
    /// content less the width of the padding box, never below zero.
    #[inline]
    pub fn scroll_width(&self) -> f32 {
        0.0_f32.max(
            self.scrollable_overflow_rect.right + self.scrollbar_size.width.min(self.size.width)
                - self.size.width
                + self.border.left
                + self.border.right,
        )
    }

    /// The largest vertical scroll offset: the reachable extent of the content
    /// less the height of the padding box, never below zero.
    #[inline]
    pub fn scroll_height(&self) -> f32 {
        0.0_f32.max(
            self.scrollable_overflow_rect.bottom + self.scrollbar_size.height.min(self.size.height)
                - self.size.height
                + self.border.top
                + self.border.bottom,
        )
    }
}
