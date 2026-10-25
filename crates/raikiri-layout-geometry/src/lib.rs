//! Geometry and box-layout result types for raikiri's layout engine.
//!
//! The paint walk, the page pipeline and the layout algorithms all pass sizes,
//! points, edge widths and the placement of a box around. This crate defines
//! those types, so that code outside the layout adapter does not have to name
//! the layout library's types. It has no dependency on the DOM, the style
//! system, the layout algorithms or the layout library.
//!
//! - [`Size`] is a `width` / `height` pair.
//! - [`Point`] is an `x` / `y` pair.
//! - `Line` is a `start` / `end` pair, for example the two edges of a grid
//!   placement or a pair of flags for the two axes.
//! - [`Edges`] holds one value for each of `left`, `right`, `top` and `bottom`:
//!   padding, border or margin widths, or the extent of scrollable overflow on
//!   each side. It is not a rectangle with a position; the rectangles used for
//!   painting and fragmentation are separate types with `x`, `y`, `width` and
//!   `height`.
//! - [`BoxLayout`] is the placement of one box relative to its layout parent:
//!   `location`, `size`, `border`, `padding`, `margin`, `scrollbar_size`,
//!   `scrollable_overflow_rect`, `order`, plus the content-box and scroll-extent
//!   helpers.
//! - [`MaybeMath`] does minimum, maximum, clamp, addition and subtraction on
//!   sizes and lengths of which some may be missing.
//!
//! The field names above are the contract: they are the names the layout
//! library uses, so code that reads or builds these values by field does not
//! change when it moves between the two.

#![forbid(unsafe_code)]

mod box_layout;
mod geometry;
mod math;

pub use box_layout::BoxLayout;
pub use geometry::{Edges, Line, Point, Size};
pub use math::MaybeMath;

#[cfg(test)]
mod tests;
