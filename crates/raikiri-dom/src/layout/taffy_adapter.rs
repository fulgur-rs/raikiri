//! The adapter between raikiri's layout code and the layout library.
//!
//! This module is where the library's types are named for the call protocol and
//! for the geometry the layout produces. Everything outside it works with
//! [`protocol`](super::protocol) and `raikiri_layout_geometry`:
//!
//! - [`convert`] turns the protocol's types and the geometry types into the
//!   library's and back, field by field.
//! - [`CollapsibleMarginSet`], [`OofCandidates`], [`LayoutCache`] and
//!   [`GridInfo`] wrap the library's values whose contents are private to it,
//!   or that it takes back as they are, so that they move across the boundary
//!   without being copied.
//! - [`FloatContext`] and [`FloatIntrinsicWidthCalculator`] wrap the library's
//!   float placement state and its sum of float widths, which the layout
//!   extends and reads through methods that take the protocol's types.
//! - [`MaybeResolve`] and [`ResolveOrZero`] resolve the library's style lengths
//!   against the layout's geometry.
//! - [`RowSizing`] and [`rows_size_to_content`] read how the rows of a grid are
//!   sized from the library's track sizing functions, and [`GridInfo`] reads the
//!   grid placement of a box from the same projection, so the rest of the
//!   layout asks questions about them instead of naming the library's grid
//!   vocabulary.
//! - [`algorithms`] calls the library's layout algorithms with protocol types.
//! - [`tree`] is the library's view of the layout session: the trait
//!   implementations the algorithms call back into. Each one converts its
//!   arguments, calls the session, and converts the result.

pub(crate) mod algorithms;
mod cache;
pub(crate) mod convert;
mod float;
mod grid_info;
mod grid_rows;
mod margin_set;
mod oof_candidates;
mod resolve;
#[cfg(test)]
pub(crate) mod track_calcs;
mod tree;

pub(crate) use cache::LayoutCache;
pub(crate) use float::{FloatContext, FloatIntrinsicWidthCalculator};
pub(crate) use grid_info::{GridInfo, GridRowCounts};
pub(crate) use grid_rows::{RowSizing, rows_size_to_content};
pub(crate) use margin_set::CollapsibleMarginSet;
pub(crate) use oof_candidates::{OofBatch, OofCandidates, OofSpan};
pub(crate) use resolve::{IntoEdges, MaybeResolve, ResolveOrZero};

#[cfg(test)]
mod tests;
