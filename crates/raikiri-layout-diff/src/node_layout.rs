//! The one place that reads a node's laid-out box.
//!
//! The tool is built from a single copy of its sources against both the
//! merge-base and the head checkout, so everything it reads from the layout
//! engine has to exist, with the same meaning, in both. The page API does
//! (it is the surface a consumer such as fulgur depends on). A node's box is
//! internal to the DOM crate and may be renamed or retyped by a change under
//! test, so the access lives here and nowhere else: when the accessor changes,
//! this file is the only one to adapt, and it must then still compile against
//! the merge-base accessor.

use raikiri_dom::Node;

/// A node's laid-out box, as plain numbers.
///
/// Lengths are CSS px. Four-sided values are in `left, right, top, bottom`
/// order, whatever the writing mode: they are the physical sides of the box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NodeBox {
    /// Paint order among siblings.
    pub(crate) order: u32,
    /// Top-left corner, relative to the layout parent's border box.
    pub(crate) location: [f32; 2],
    /// Border-box width and height.
    pub(crate) size: [f32; 2],
    /// Scrollable overflow rectangle edges, from the scroll origin.
    pub(crate) scrollable_overflow: [f32; 4],
    /// Scrollbar width and height.
    pub(crate) scrollbar: [f32; 2],
    /// Used border widths.
    pub(crate) border: [f32; 4],
    /// Used padding.
    pub(crate) padding: [f32; 4],
    /// Used margins.
    pub(crate) margin: [f32; 4],
}

impl NodeBox {
    /// The box layout stored on `node`.
    pub(crate) fn of(node: &Node) -> Self {
        let layout = &node.unrounded_layout;
        Self {
            order: layout.order,
            location: [layout.location.x, layout.location.y],
            size: [layout.size.width, layout.size.height],
            scrollable_overflow: [
                layout.scrollable_overflow_rect.left,
                layout.scrollable_overflow_rect.right,
                layout.scrollable_overflow_rect.top,
                layout.scrollable_overflow_rect.bottom,
            ],
            scrollbar: [layout.scrollbar_size.width, layout.scrollbar_size.height],
            border: [
                layout.border.left,
                layout.border.right,
                layout.border.top,
                layout.border.bottom,
            ],
            padding: [
                layout.padding.left,
                layout.padding.right,
                layout.padding.top,
                layout.padding.bottom,
            ],
            margin: [
                layout.margin.left,
                layout.margin.right,
                layout.margin.top,
                layout.margin.bottom,
            ],
        }
    }
}
