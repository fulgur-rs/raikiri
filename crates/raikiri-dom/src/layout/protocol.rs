//! The layout call protocol.
//!
//! A layout algorithm is called with a [`LayoutInput`] (known dimensions, the
//! available space, the sizing and run modes, the margin collapsing state of
//! the parent) and answers with a [`LayoutOutput`] (the size of the box, its
//! content size, baselines, collapsed margins and the out-of-flow candidates
//! found inside it). The layout code names these types through this module.
//! The layout library's own types are only named in the adapter that talks to
//! the library (`layout::taffy_adapter`), which converts to and from these.
//!
//! The types have the shape of the library's types of the same name: the same
//! fields and the same variants. Two parts of the library's types are opaque
//! and stay behind the adapter: the margin sets and the list of out-of-flow
//! candidates are re-exported from it ([`CollapsibleMarginSet`],
//! [`OofCandidates`]).
//!
//! These types are internal to raikiri-dom: no other crate reads a
//! [`LayoutInput`] or a [`LayoutOutput`], so they are deliberately not exported.

use raikiri_layout_geometry::{Edges, Line, MaybeMath, Point, Size};

pub(crate) use super::taffy_adapter::{CollapsibleMarginSet, OofCandidates};

/// The identity of a node in the layout: its index in the document arena.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct NodeId(usize);

impl From<usize> for NodeId {
    #[inline]
    fn from(index: usize) -> Self {
        Self(index)
    }
}

impl From<NodeId> for usize {
    #[inline]
    fn from(id: NodeId) -> Self {
        id.0
    }
}

/// The amount of space available to a box in one axis.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum AvailableSpace {
    /// The space is the given number of pixels.
    Definite(f32),
    /// The space is indefinite and the box is laid out under a min-content
    /// constraint.
    MinContent,
    /// The space is indefinite and the box is laid out under a max-content
    /// constraint.
    MaxContent,
}

impl AvailableSpace {
    /// An indefinite space under a max-content constraint.
    pub(crate) const MAX_CONTENT: AvailableSpace = AvailableSpace::MaxContent;
    /// An indefinite space under a min-content constraint.
    pub(crate) const MIN_CONTENT: AvailableSpace = AvailableSpace::MinContent;

    /// The number of pixels of a definite space; `None` for a constraint.
    #[inline]
    pub(crate) const fn into_option(self) -> Option<f32> {
        match self {
            AvailableSpace::Definite(value) => Some(value),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        }
    }
}

impl MaybeMath<f32, AvailableSpace> for AvailableSpace {
    #[inline]
    fn maybe_min(self, rhs: f32) -> AvailableSpace {
        match self {
            AvailableSpace::Definite(val) => AvailableSpace::Definite(val.min(rhs)),
            AvailableSpace::MinContent => AvailableSpace::Definite(rhs),
            AvailableSpace::MaxContent => AvailableSpace::Definite(rhs),
        }
    }

    #[inline]
    fn maybe_max(self, rhs: f32) -> AvailableSpace {
        match self {
            AvailableSpace::Definite(val) => AvailableSpace::Definite(val.max(rhs)),
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_clamp(self, min: f32, max: f32) -> AvailableSpace {
        match self {
            AvailableSpace::Definite(val) => AvailableSpace::Definite(val.min(max).max(min)),
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_add(self, rhs: f32) -> AvailableSpace {
        match self {
            AvailableSpace::Definite(val) => AvailableSpace::Definite(val + rhs),
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_sub(self, rhs: f32) -> AvailableSpace {
        match self {
            AvailableSpace::Definite(val) => AvailableSpace::Definite(val - rhs),
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }
}

impl MaybeMath<Option<f32>, AvailableSpace> for AvailableSpace {
    #[inline]
    fn maybe_min(self, rhs: Option<f32>) -> AvailableSpace {
        match (self, rhs) {
            (AvailableSpace::Definite(val), Some(rhs)) => AvailableSpace::Definite(val.min(rhs)),
            (AvailableSpace::Definite(val), None) => AvailableSpace::Definite(val),
            (AvailableSpace::MinContent, Some(rhs)) => AvailableSpace::Definite(rhs),
            (AvailableSpace::MinContent, None) => AvailableSpace::MinContent,
            (AvailableSpace::MaxContent, Some(rhs)) => AvailableSpace::Definite(rhs),
            (AvailableSpace::MaxContent, None) => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_max(self, rhs: Option<f32>) -> AvailableSpace {
        match (self, rhs) {
            (AvailableSpace::Definite(val), Some(rhs)) => AvailableSpace::Definite(val.max(rhs)),
            (AvailableSpace::Definite(val), None) => AvailableSpace::Definite(val),
            (AvailableSpace::MinContent, _) => AvailableSpace::MinContent,
            (AvailableSpace::MaxContent, _) => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_clamp(self, min: Option<f32>, max: Option<f32>) -> AvailableSpace {
        match (self, min, max) {
            (AvailableSpace::Definite(val), Some(min), Some(max)) => {
                AvailableSpace::Definite(val.min(max).max(min))
            }
            (AvailableSpace::Definite(val), None, Some(max)) => {
                AvailableSpace::Definite(val.min(max))
            }
            (AvailableSpace::Definite(val), Some(min), None) => {
                AvailableSpace::Definite(val.max(min))
            }
            (AvailableSpace::Definite(val), None, None) => AvailableSpace::Definite(val),
            (AvailableSpace::MinContent, _, _) => AvailableSpace::MinContent,
            (AvailableSpace::MaxContent, _, _) => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_add(self, rhs: Option<f32>) -> AvailableSpace {
        match (self, rhs) {
            (AvailableSpace::Definite(val), Some(rhs)) => AvailableSpace::Definite(val + rhs),
            (AvailableSpace::Definite(val), None) => AvailableSpace::Definite(val),
            (AvailableSpace::MinContent, _) => AvailableSpace::MinContent,
            (AvailableSpace::MaxContent, _) => AvailableSpace::MaxContent,
        }
    }

    #[inline]
    fn maybe_sub(self, rhs: Option<f32>) -> AvailableSpace {
        match (self, rhs) {
            (AvailableSpace::Definite(val), Some(rhs)) => AvailableSpace::Definite(val - rhs),
            (AvailableSpace::Definite(val), None) => AvailableSpace::Definite(val),
            (AvailableSpace::MinContent, _) => AvailableSpace::MinContent,
            (AvailableSpace::MaxContent, _) => AvailableSpace::MaxContent,
        }
    }
}

/// Whether a box is laid out in full or only measured.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// The box and all its descendants are laid out and placed.
    PerformLayout,
    /// Only the size of the box is determined; steps that do not affect it may
    /// be skipped.
    ComputeSize,
    /// The box is hidden (`display: none`) and gets a zero layout.
    PerformHiddenLayout,
}

/// Whether the style sizes of a box are taken into account.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum SizingMode {
    /// Only the contributions of the content are taken into account.
    ContentSize,
    /// The size properties of the box are taken into account as well.
    InherentSize,
}

/// An axis a layout algorithm is asked to compute a size for.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum RequestedAxis {
    /// The horizontal axis.
    Horizontal,
    /// The vertical axis.
    Vertical,
    /// Both axes.
    Both,
}

/// A single physical axis.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum AbsoluteAxis {
    /// The horizontal axis.
    Horizontal,
    /// The vertical axis.
    Vertical,
}

impl AbsoluteAxis {
    /// The other axis.
    #[inline]
    pub(crate) const fn other_axis(&self) -> AbsoluteAxis {
        match *self {
            AbsoluteAxis::Horizontal => AbsoluteAxis::Vertical,
            AbsoluteAxis::Vertical => AbsoluteAxis::Horizontal,
        }
    }
}

impl From<AbsoluteAxis> for RequestedAxis {
    #[inline]
    fn from(value: AbsoluteAxis) -> Self {
        match value {
            AbsoluteAxis::Horizontal => RequestedAxis::Horizontal,
            AbsoluteAxis::Vertical => RequestedAxis::Vertical,
        }
    }
}

impl TryFrom<RequestedAxis> for AbsoluteAxis {
    type Error = ();

    #[inline]
    fn try_from(value: RequestedAxis) -> Result<Self, Self::Error> {
        match value {
            RequestedAxis::Horizontal => Ok(AbsoluteAxis::Horizontal),
            RequestedAxis::Vertical => Ok(AbsoluteAxis::Vertical),
            RequestedAxis::Both => Err(()),
        }
    }
}

/// What a parent asks a box to lay itself out under.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct LayoutInput {
    /// Whether to lay the box out in full or only to measure it.
    pub run_mode: RunMode,
    /// Whether the style sizes of the box are taken into account.
    pub sizing_mode: SizingMode,
    /// The axis the size is needed for.
    pub axis: RequestedAxis,
    /// Dimensions the parent has fixed.
    ///
    /// With `known_dimensions.width` set to `Some(w)` the question is what the
    /// height would be if the width were `w`. With both set, the box is laid
    /// out at exactly that size.
    pub known_dimensions: Size<Option<f32>>,
    /// Whether each known dimension is a definite size for the content of the
    /// box (percentages of children resolve against it).
    ///
    /// `false` for a dimension that a parent imposed on a box and that is
    /// derived from the content of the box, which makes it indefinite. Ignored
    /// for an axis without a known dimension.
    pub known_dimensions_are_definite: Size<bool>,
    /// The size percentages resolve against.
    pub parent_size: Size<Option<f32>>,
    /// The space the box is laid out into, a soft constraint for wrapping. A
    /// definite value is the space for the border box: the parent has
    /// subtracted the margins already.
    pub available_space: Size<AvailableSpace>,
    /// Whether the top and the bottom margin of the box can collapse with
    /// those of its parent. `Line::FALSE` unless the parent is a block.
    pub vertical_margins_are_collapsible: Line<bool>,
}

impl LayoutInput {
    /// The input that requests hidden layout; only `run_mode` matters.
    pub(crate) const HIDDEN: LayoutInput = LayoutInput {
        run_mode: RunMode::PerformHiddenLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size::NONE,
        available_space: Size {
            width: AvailableSpace::MAX_CONTENT,
            height: AvailableSpace::MAX_CONTENT,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
}

/// The first and last baseline of a box, as offsets from the top edge of its
/// border box, for horizontal text.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct Baselines {
    /// The first baseline, if any.
    pub first: Option<f32>,
    /// The last baseline, if any.
    pub last: Option<f32>,
}

impl Baselines {
    /// Neither a first nor a last baseline.
    pub(crate) const NONE: Self = Self {
        first: None,
        last: None,
    };
}

/// The physical alignment of the margin box of an out-of-flow box within its
/// static-position area in one axis.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum AxisStaticEdge {
    /// The start (left or top) edge of the margin box at the start of the area.
    Start,
    /// The margin box centered in the area.
    Center,
    /// The end (right or bottom) edge of the margin box at the end of the area.
    End,
}

/// The overflow-position keyword of an alignment: what happens when the margin
/// box does not fit the area.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum StaticAlignSafety {
    /// No keyword: as `Unsafe`, except that an absolutely positioned box is
    /// adjusted to overflow its containing block as little as possible.
    Default,
    /// The requested alignment is kept even when the box overflows the area.
    Unsafe,
    /// The `fallback` alignment is used when the box would overflow the area.
    Safe,
}

/// The alignment applied to the margin box of an out-of-flow box within its
/// static-position area in one axis.
///
/// The keywords are physical: the emitting algorithm has resolved
/// `justify-content` / `align-self`, the direction of the container, and
/// `self-start` / `self-end` already.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct AxisStaticAlign {
    /// The physical alignment keyword.
    pub keyword: AxisStaticEdge,
    /// The overflow-position keyword.
    pub safety: StaticAlignSafety,
    /// The alignment to use instead of `keyword` when `safety` is `Safe` and
    /// the margin box overflows the area.
    pub fallback: AxisStaticEdge,
}

impl AxisStaticAlign {
    /// An alignment by `keyword` with no safe fallback.
    pub(crate) const fn from_keyword(keyword: AxisStaticEdge) -> Self {
        Self {
            keyword,
            safety: StaticAlignSafety::Default,
            fallback: keyword,
        }
    }
}

/// The static position of an out-of-flow box in one axis: the alignment area
/// (start and end coordinate) and the alignment within it.
///
/// The area is measured from the border box of the node that holds the
/// candidate; it is translated by the location of each node the candidate
/// passes on its way up. An emitter without an alignment area in the axis (a
/// block, where the static position is a point) emits a degenerate area with
/// `start == end`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct AxisStaticPosition {
    /// The alignment area in this axis.
    pub area: Line<f32>,
    /// The alignment of the margin box within `area`.
    pub align: AxisStaticAlign,
}

impl AxisStaticPosition {
    /// A degenerate area at `anchor` aligned by `edge`, with no safe fallback.
    pub(crate) const fn from_edge(anchor: f32, edge: AxisStaticEdge) -> Self {
        Self {
            area: Line {
                start: anchor,
                end: anchor,
            },
            align: AxisStaticAlign::from_keyword(edge),
        }
    }
}

/// The position of an out-of-flow box.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum OofPosition {
    /// `position: absolute`.
    Absolute,
    /// `position: fixed`.
    Fixed,
}

/// An out-of-flow box that is not laid out yet because its containing block is
/// an ancestor of its parent.
///
/// The parent's layout algorithm computes the static position and emits the
/// candidate; it bubbles up through [`LayoutOutput`] until it reaches the node
/// that is its containing block, which lays it out.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct OofCandidate {
    /// The out-of-flow box.
    pub node: NodeId,
    /// The `order` its layout is given: its child index in its parent.
    pub order: u32,
    /// Its position.
    pub position: OofPosition,
    /// Its static position in each axis.
    pub static_position: Point<AxisStaticPosition>,
}

/// The area of a container the insets of its out-of-flow boxes resolve
/// against: the border box less the borders and the scrollbar gutters.
#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) struct OofPositioningArea {
    /// The size of the area.
    pub size: Size<f32>,
    /// The offset of the area from the origin of the border box.
    pub offset: Point<f32>,
}

/// What positioning out-of-flow candidates against a containing block returns.
#[derive(Debug)]
pub(crate) struct OofLayoutResult {
    /// The candidates the pass laid out, in document order.
    pub hoisted: Vec<NodeId>,
    /// The candidates the pass is not the containing block of, which continue
    /// to bubble up.
    pub unclaimed: OofCandidates,
    /// The scrollable overflow the laid out candidates contribute, measured
    /// from the origin of the positioning area.
    pub scrollable_overflow_rect: Edges<f32>,
}

/// What laying out one box returns to its parent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LayoutOutput {
    /// The size of the box.
    pub size: Size<f32>,
    /// How far the content of the box reaches on each side.
    pub scrollable_overflow_rect: Edges<f32>,
    /// The first and last baseline.
    pub baselines: Baselines,
    /// The top margin that can collapse with the margin of the parent.
    pub top_margin: CollapsibleMarginSet,
    /// The bottom margin that can collapse with the margin of the parent.
    pub bottom_margin: CollapsibleMarginSet,
    /// Whether margins can collapse through the box.
    pub margins_can_collapse_through: bool,
    /// The out-of-flow descendants whose containing block is not this box. They
    /// bubble up until they reach it. Only a run that performs layout produces
    /// them, and an empty list does not allocate.
    ///
    /// A container holds the full candidate list in document order here; the
    /// positioning pass lays out the ones it is the containing block of and
    /// replaces the list with the rest.
    pub oof_candidates: OofCandidates,
    /// The area the insets of the out-of-flow boxes resolve against. Set for
    /// containers by runs that perform layout; `None` for leaves and for
    /// measuring runs, for which the positioning pass does nothing.
    pub oof_positioning_area: Option<OofPositioningArea>,
}

impl LayoutOutput {
    /// A zero output for a hidden box.
    pub(crate) const HIDDEN: Self = Self {
        size: Size::ZERO,
        scrollable_overflow_rect: Edges::ZERO,
        baselines: Baselines::NONE,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: false,
        oof_candidates: OofCandidates::NONE,
        oof_positioning_area: None,
    };

    /// An output with a size, a scrollable overflow rectangle and baselines
    /// only.
    pub(crate) fn from_sizes_and_baselines(
        size: Size<f32>,
        scrollable_overflow_rect: Edges<f32>,
        baselines: Baselines,
    ) -> Self {
        Self {
            size,
            scrollable_overflow_rect,
            baselines,
            ..Self::HIDDEN
        }
    }

    /// An output with a size and a scrollable overflow rectangle only.
    pub(crate) fn from_sizes(size: Size<f32>, scrollable_overflow_rect: Edges<f32>) -> Self {
        Self::from_sizes_and_baselines(size, scrollable_overflow_rect, Baselines::NONE)
    }

    /// An output with a size only.
    pub(crate) fn from_outer_size(size: Size<f32>) -> Self {
        Self::from_sizes(size, Edges::ZERO)
    }
}
