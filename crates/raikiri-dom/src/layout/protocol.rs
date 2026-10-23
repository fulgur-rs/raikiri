//! Names for the layout call protocol.
//!
//! A layout algorithm is called with a [`LayoutInput`] (known dimensions, the
//! available space, the sizing and run modes, the margin collapsing state of
//! the parent) and answers with a [`LayoutOutput`] (the size of the box, its
//! content size, baselines, collapsed margins and the out-of-flow candidates
//! found inside it). The rest of the layout code names these types through
//! this module instead of through the layout library, so the library's types
//! are only named where raikiri-dom talks to the library.
//!
//! Today the names are re-exports, so values pass between the layout code and
//! the library without conversion. The definitions will move here and the
//! conversion to and from the library's types will live in the adapter; code
//! that uses the names and fields through this module keeps compiling when that
//! happens.
//!
//! These types are internal to raikiri-dom: no other crate reads a
//! [`LayoutInput`] or a [`LayoutOutput`], so they are deliberately not exported.

// Not every name is imported through this module yet.
#[allow(unused_imports)]
// cov:ignore: re-exports have no executable mapping to cover.
pub(crate) use taffy::{
    AbsoluteAxis, AvailableSpace, AxisStaticAlign, AxisStaticEdge, AxisStaticPosition, Baselines,
    CollapsibleMarginSet, LayoutInput, LayoutOutput, NodeId, OofCandidate, OofCandidates,
    OofPositioningArea, RequestedAxis, RunMode, SizingMode,
};
