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
mod empty_leaf;
mod float;
pub(crate) use empty_leaf::empty_leaf_collapses_on_transposed_axes;
mod grid_info;
mod grid_rows;
mod margin_set;
mod oof_candidates;
#[cfg(test)]
pub(crate) mod track_calcs;
mod tree;

pub(crate) use cache::LayoutCache;
pub(crate) use float::{FloatContext, FloatIntrinsicWidthCalculator};
pub(crate) use grid_info::{GridInfo, GridItemLines, GridRowCounts};
pub(crate) use grid_rows::columns_follow_their_items;
pub(crate) use grid_rows::{RowSizing, columns_have_no_content_minimum, rows_size_to_content};
pub(crate) use margin_set::CollapsibleMarginSet;

/// Run `layout`, the layout library's layout of the grid container `node`,
/// with `node` noted as the innermost grid container being laid out.
///
/// The library reads each item's style once to build the item it places and
/// sizes the tracks with, and again later to align it. The first read of an
/// item, found by [`placement_read`], may then tell the track sizing what the
/// alignment does not see. The note is taken off again even when `layout`
/// panics, as the other stacks around native layout are.
pub(crate) fn placing_grid<T, R>(
    tree: &mut T,
    context: for<'a> fn(&'a T) -> &'a crate::layout::context::LayoutContext,
    node: usize,
    layout: impl FnOnce(&mut T) -> R,
) -> R {
    context(tree)
        .grid_placements
        .borrow_mut()
        .push((node, std::collections::HashSet::new()));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layout(tree)));
    context(tree).grid_placements.borrow_mut().pop();
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Whether this read of the style of `item`, a child of `parent`, is the one
/// the layout library builds the placed item from: the first read of it
/// while its parent is the innermost grid container being laid out.
pub(crate) fn placement_read(
    context: &crate::layout::context::LayoutContext,
    parent: Option<usize>,
    item: usize,
) -> bool {
    let mut placements = context.grid_placements.borrow_mut();
    match placements.last_mut() {
        Some((container, read)) if parent == Some(*container) => read.insert(item),
        _ => false,
    }
}
pub(crate) use oof_candidates::{OofBatch, OofCandidates, OofSpan};

#[cfg(test)]
mod tests;
