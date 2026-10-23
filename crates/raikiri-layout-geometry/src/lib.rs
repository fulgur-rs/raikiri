//! Geometry and box-layout result types for raikiri's layout engine.
//!
//! The paint walk, the page pipeline and the layout algorithms all pass sizes,
//! points, edge widths and the placement of a box around. This crate is where
//! those types are named, so that code outside the layout adapter does not have
//! to name the layout library's types. It has no dependency on the DOM, the
//! style system or the layout algorithms.
//!
//! - [`Size`] is a `width` / `height` pair.
//! - [`Point`] is an `x` / `y` pair.
//! - [`Line`] is a `start` / `end` pair, for example the two edges of a grid
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
//!
//! The field names above are the contract. Today the names are re-exports of
//! the corresponding Taffy types, so values pass between this crate's types and
//! Taffy's without conversion. The definitions will move into this crate and
//! the dependency on Taffy will go away; code that sticks to the names and
//! fields listed here keeps compiling when that happens.

pub use taffy::{Layout as BoxLayout, Line, Point, Rect as Edges, Size};

#[cfg(test)]
mod tests;
