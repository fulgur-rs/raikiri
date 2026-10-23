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
//! - [`MaybeResolve`] and [`ResolveOrZero`] resolve the library's style lengths
//!   against the layout's geometry.
//! - [`algorithms`] calls the library's layout algorithms with protocol types.
//! - [`tree`] is the library's view of the layout session: the trait
//!   implementations the algorithms call back into. Each one converts its
//!   arguments, calls the session, and converts the result.

pub(crate) mod algorithms;
mod cache;
pub(crate) mod convert;
mod grid_info;
mod margin_set;
mod oof_candidates;
mod resolve;
mod tree;

pub(crate) use cache::LayoutCache;
pub(crate) use grid_info::{GridInfo, GridRowCounts};
pub(crate) use margin_set::CollapsibleMarginSet;
pub(crate) use oof_candidates::{OofBatch, OofCandidates, OofSpan};
pub(crate) use resolve::{IntoEdges, MaybeResolve, ResolveOrZero};

#[cfg(test)]
mod tests;
