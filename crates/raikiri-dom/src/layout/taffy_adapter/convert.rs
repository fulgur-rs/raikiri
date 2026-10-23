//! Conversions between the layout call protocol and the layout library's types.
//!
//! Every conversion names all fields and all variants of the type it converts
//! (no `..` in a pattern, no wildcard arm in a `match`), so a field or a variant
//! that the library adds is a compile error here instead of a value that is
//! silently dropped or mapped to something else.

use crate::layout::protocol::{
    AbsoluteAxis, AvailableSpace, AxisStaticAlign, AxisStaticEdge, AxisStaticPosition, Baselines,
    LayoutInput, LayoutOutput, NodeId, OofCandidate, OofPosition, OofPositioningArea,
    RequestedAxis, RunMode, SizingMode, StaticAlignSafety,
};
use raikiri_layout_geometry::{BoxLayout, Edges, Line, Point, Size};

/// Convert a value of the layout call protocol (or the geometry types) into the
/// layout library's type of the same name.
pub(crate) trait IntoTaffy {
    /// The library's type.
    type Taffy;

    /// The library's value.
    fn into_taffy(self) -> Self::Taffy;
}

/// Convert a value of the layout library into the type of the layout call
/// protocol (or the geometry types) of the same name.
pub(crate) trait FromTaffy {
    /// The library's type.
    type Taffy;

    /// The protocol's value.
    fn from_taffy(value: Self::Taffy) -> Self;
}

macro_rules! same_type {
    ($($ty:ty),+ $(,)?) => {$(
        impl IntoTaffy for $ty {
            type Taffy = $ty;

            #[inline]
            fn into_taffy(self) -> $ty {
                self
            }
        }

        impl FromTaffy for $ty {
            type Taffy = $ty;

            #[inline]
            fn from_taffy(value: $ty) -> $ty {
                value
            }
        }
    )+};
}

same_type!(f32, bool, u32);

impl<T: IntoTaffy> IntoTaffy for Option<T> {
    type Taffy = Option<T::Taffy>;

    #[inline]
    fn into_taffy(self) -> Self::Taffy {
        self.map(IntoTaffy::into_taffy)
    }
}

impl<T: FromTaffy> FromTaffy for Option<T> {
    type Taffy = Option<T::Taffy>;

    #[inline]
    fn from_taffy(value: Self::Taffy) -> Self {
        value.map(T::from_taffy)
    }
}

impl<T: IntoTaffy> IntoTaffy for Size<T> {
    type Taffy = taffy::Size<T::Taffy>;

    #[inline]
    fn into_taffy(self) -> Self::Taffy {
        let Size { width, height } = self;
        taffy::Size {
            width: width.into_taffy(),
            height: height.into_taffy(),
        }
    }
}

impl<T: FromTaffy> FromTaffy for Size<T> {
    type Taffy = taffy::Size<T::Taffy>;

    #[inline]
    fn from_taffy(value: Self::Taffy) -> Self {
        let taffy::Size { width, height } = value;
        Size {
            width: T::from_taffy(width),
            height: T::from_taffy(height),
        }
    }
}

impl<T: IntoTaffy> IntoTaffy for Point<T> {
    type Taffy = taffy::Point<T::Taffy>;

    #[inline]
    fn into_taffy(self) -> Self::Taffy {
        let Point { x, y } = self;
        taffy::Point {
            x: x.into_taffy(),
            y: y.into_taffy(),
        }
    }
}

impl<T: FromTaffy> FromTaffy for Point<T> {
    type Taffy = taffy::Point<T::Taffy>;

    #[inline]
    fn from_taffy(value: Self::Taffy) -> Self {
        let taffy::Point { x, y } = value;
        Point {
            x: T::from_taffy(x),
            y: T::from_taffy(y),
        }
    }
}

impl<T: IntoTaffy> IntoTaffy for Line<T> {
    type Taffy = taffy::Line<T::Taffy>;

    #[inline]
    fn into_taffy(self) -> Self::Taffy {
        let Line { start, end } = self;
        taffy::Line {
            start: start.into_taffy(),
            end: end.into_taffy(),
        }
    }
}

impl<T: FromTaffy> FromTaffy for Line<T> {
    type Taffy = taffy::Line<T::Taffy>;

    #[inline]
    fn from_taffy(value: Self::Taffy) -> Self {
        let taffy::Line { start, end } = value;
        Line {
            start: T::from_taffy(start),
            end: T::from_taffy(end),
        }
    }
}

impl<T: IntoTaffy> IntoTaffy for Edges<T> {
    type Taffy = taffy::Rect<T::Taffy>;

    #[inline]
    fn into_taffy(self) -> Self::Taffy {
        let Edges {
            left,
            right,
            top,
            bottom,
        } = self;
        taffy::Rect {
            left: left.into_taffy(),
            right: right.into_taffy(),
            top: top.into_taffy(),
            bottom: bottom.into_taffy(),
        }
    }
}

impl<T: FromTaffy> FromTaffy for Edges<T> {
    type Taffy = taffy::Rect<T::Taffy>;

    #[inline]
    fn from_taffy(value: Self::Taffy) -> Self {
        let taffy::Rect {
            left,
            right,
            top,
            bottom,
        } = value;
        Edges {
            left: T::from_taffy(left),
            right: T::from_taffy(right),
            top: T::from_taffy(top),
            bottom: T::from_taffy(bottom),
        }
    }
}

impl IntoTaffy for BoxLayout {
    type Taffy = taffy::Layout;

    #[inline]
    fn into_taffy(self) -> taffy::Layout {
        let BoxLayout {
            order,
            location,
            size,
            scrollable_overflow_rect,
            scrollbar_size,
            border,
            padding,
            margin,
        } = self;
        taffy::Layout {
            order,
            location: location.into_taffy(),
            size: size.into_taffy(),
            scrollable_overflow_rect: scrollable_overflow_rect.into_taffy(),
            scrollbar_size: scrollbar_size.into_taffy(),
            border: border.into_taffy(),
            padding: padding.into_taffy(),
            margin: margin.into_taffy(),
        }
    }
}

impl FromTaffy for BoxLayout {
    type Taffy = taffy::Layout;

    #[inline]
    fn from_taffy(value: taffy::Layout) -> Self {
        let taffy::Layout {
            order,
            location,
            size,
            scrollable_overflow_rect,
            scrollbar_size,
            border,
            padding,
            margin,
        } = value;
        BoxLayout {
            order,
            location: Point::from_taffy(location),
            size: Size::from_taffy(size),
            scrollable_overflow_rect: Edges::from_taffy(scrollable_overflow_rect),
            scrollbar_size: Size::from_taffy(scrollbar_size),
            border: Edges::from_taffy(border),
            padding: Edges::from_taffy(padding),
            margin: Edges::from_taffy(margin),
        }
    }
}

impl IntoTaffy for NodeId {
    type Taffy = taffy::NodeId;

    #[inline]
    fn into_taffy(self) -> taffy::NodeId {
        taffy::NodeId::from(usize::from(self))
    }
}

impl FromTaffy for NodeId {
    type Taffy = taffy::NodeId;

    #[inline]
    fn from_taffy(value: taffy::NodeId) -> Self {
        NodeId::from(usize::from(value))
    }
}

impl IntoTaffy for RunMode {
    type Taffy = taffy::RunMode;

    #[inline]
    fn into_taffy(self) -> taffy::RunMode {
        match self {
            RunMode::PerformLayout => taffy::RunMode::PerformLayout,
            RunMode::ComputeSize => taffy::RunMode::ComputeSize,
            RunMode::PerformHiddenLayout => taffy::RunMode::PerformHiddenLayout,
        }
    }
}

impl FromTaffy for RunMode {
    type Taffy = taffy::RunMode;

    #[inline]
    fn from_taffy(value: taffy::RunMode) -> Self {
        match value {
            taffy::RunMode::PerformLayout => RunMode::PerformLayout,
            taffy::RunMode::ComputeSize => RunMode::ComputeSize,
            taffy::RunMode::PerformHiddenLayout => RunMode::PerformHiddenLayout,
        }
    }
}

impl IntoTaffy for SizingMode {
    type Taffy = taffy::SizingMode;

    #[inline]
    fn into_taffy(self) -> taffy::SizingMode {
        match self {
            SizingMode::ContentSize => taffy::SizingMode::ContentSize,
            SizingMode::InherentSize => taffy::SizingMode::InherentSize,
        }
    }
}

impl FromTaffy for SizingMode {
    type Taffy = taffy::SizingMode;

    #[inline]
    fn from_taffy(value: taffy::SizingMode) -> Self {
        match value {
            taffy::SizingMode::ContentSize => SizingMode::ContentSize,
            taffy::SizingMode::InherentSize => SizingMode::InherentSize,
        }
    }
}

impl IntoTaffy for RequestedAxis {
    type Taffy = taffy::RequestedAxis;

    #[inline]
    fn into_taffy(self) -> taffy::RequestedAxis {
        match self {
            RequestedAxis::Horizontal => taffy::RequestedAxis::Horizontal,
            RequestedAxis::Vertical => taffy::RequestedAxis::Vertical,
            RequestedAxis::Both => taffy::RequestedAxis::Both,
        }
    }
}

impl FromTaffy for RequestedAxis {
    type Taffy = taffy::RequestedAxis;

    #[inline]
    fn from_taffy(value: taffy::RequestedAxis) -> Self {
        match value {
            taffy::RequestedAxis::Horizontal => RequestedAxis::Horizontal,
            taffy::RequestedAxis::Vertical => RequestedAxis::Vertical,
            taffy::RequestedAxis::Both => RequestedAxis::Both,
        }
    }
}

impl IntoTaffy for AbsoluteAxis {
    type Taffy = taffy::AbsoluteAxis;

    #[inline]
    fn into_taffy(self) -> taffy::AbsoluteAxis {
        match self {
            AbsoluteAxis::Horizontal => taffy::AbsoluteAxis::Horizontal,
            AbsoluteAxis::Vertical => taffy::AbsoluteAxis::Vertical,
        }
    }
}

impl FromTaffy for AbsoluteAxis {
    type Taffy = taffy::AbsoluteAxis;

    #[inline]
    fn from_taffy(value: taffy::AbsoluteAxis) -> Self {
        match value {
            taffy::AbsoluteAxis::Horizontal => AbsoluteAxis::Horizontal,
            taffy::AbsoluteAxis::Vertical => AbsoluteAxis::Vertical,
        }
    }
}

impl IntoTaffy for AvailableSpace {
    type Taffy = taffy::AvailableSpace;

    #[inline]
    fn into_taffy(self) -> taffy::AvailableSpace {
        match self {
            AvailableSpace::Definite(value) => taffy::AvailableSpace::Definite(value),
            AvailableSpace::MinContent => taffy::AvailableSpace::MinContent,
            AvailableSpace::MaxContent => taffy::AvailableSpace::MaxContent,
        }
    }
}

impl FromTaffy for AvailableSpace {
    type Taffy = taffy::AvailableSpace;

    #[inline]
    fn from_taffy(value: taffy::AvailableSpace) -> Self {
        match value {
            taffy::AvailableSpace::Definite(value) => AvailableSpace::Definite(value),
            taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
            taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }
}

impl IntoTaffy for LayoutInput {
    type Taffy = taffy::LayoutInput;

    #[inline]
    fn into_taffy(self) -> taffy::LayoutInput {
        let LayoutInput {
            run_mode,
            sizing_mode,
            axis,
            known_dimensions,
            known_dimensions_are_definite,
            parent_size,
            available_space,
            vertical_margins_are_collapsible,
        } = self;
        taffy::LayoutInput {
            run_mode: run_mode.into_taffy(),
            sizing_mode: sizing_mode.into_taffy(),
            axis: axis.into_taffy(),
            known_dimensions: known_dimensions.into_taffy(),
            known_dimensions_are_definite: known_dimensions_are_definite.into_taffy(),
            parent_size: parent_size.into_taffy(),
            available_space: available_space.into_taffy(),
            vertical_margins_are_collapsible: vertical_margins_are_collapsible.into_taffy(),
        }
    }
}

impl FromTaffy for LayoutInput {
    type Taffy = taffy::LayoutInput;

    #[inline]
    fn from_taffy(value: taffy::LayoutInput) -> Self {
        let taffy::LayoutInput {
            run_mode,
            sizing_mode,
            axis,
            known_dimensions,
            known_dimensions_are_definite,
            parent_size,
            available_space,
            vertical_margins_are_collapsible,
        } = value;
        LayoutInput {
            run_mode: RunMode::from_taffy(run_mode),
            sizing_mode: SizingMode::from_taffy(sizing_mode),
            axis: RequestedAxis::from_taffy(axis),
            known_dimensions: Size::from_taffy(known_dimensions),
            known_dimensions_are_definite: Size::from_taffy(known_dimensions_are_definite),
            parent_size: Size::from_taffy(parent_size),
            available_space: Size::from_taffy(available_space),
            vertical_margins_are_collapsible: Line::from_taffy(vertical_margins_are_collapsible),
        }
    }
}

impl IntoTaffy for Baselines {
    type Taffy = taffy::Baselines;

    #[inline]
    fn into_taffy(self) -> taffy::Baselines {
        let Baselines { first, last } = self;
        taffy::Baselines { first, last }
    }
}

impl FromTaffy for Baselines {
    type Taffy = taffy::Baselines;

    #[inline]
    fn from_taffy(value: taffy::Baselines) -> Self {
        let taffy::Baselines { first, last } = value;
        Baselines { first, last }
    }
}

impl IntoTaffy for AxisStaticEdge {
    type Taffy = taffy::AxisStaticEdge;

    #[inline]
    fn into_taffy(self) -> taffy::AxisStaticEdge {
        match self {
            AxisStaticEdge::Start => taffy::AxisStaticEdge::Start,
            AxisStaticEdge::Center => taffy::AxisStaticEdge::Center,
            AxisStaticEdge::End => taffy::AxisStaticEdge::End,
        }
    }
}

impl FromTaffy for AxisStaticEdge {
    type Taffy = taffy::AxisStaticEdge;

    #[inline]
    fn from_taffy(value: taffy::AxisStaticEdge) -> Self {
        match value {
            taffy::AxisStaticEdge::Start => AxisStaticEdge::Start,
            taffy::AxisStaticEdge::Center => AxisStaticEdge::Center,
            taffy::AxisStaticEdge::End => AxisStaticEdge::End,
        }
    }
}

impl IntoTaffy for StaticAlignSafety {
    type Taffy = taffy::AlignmentSafety;

    #[inline]
    fn into_taffy(self) -> taffy::AlignmentSafety {
        match self {
            StaticAlignSafety::Default => taffy::AlignmentSafety::Default,
            StaticAlignSafety::Unsafe => taffy::AlignmentSafety::Unsafe,
            StaticAlignSafety::Safe => taffy::AlignmentSafety::Safe,
        }
    }
}

impl FromTaffy for StaticAlignSafety {
    type Taffy = taffy::AlignmentSafety;

    #[inline]
    fn from_taffy(value: taffy::AlignmentSafety) -> Self {
        match value {
            taffy::AlignmentSafety::Default => StaticAlignSafety::Default,
            taffy::AlignmentSafety::Unsafe => StaticAlignSafety::Unsafe,
            taffy::AlignmentSafety::Safe => StaticAlignSafety::Safe,
        }
    }
}

impl IntoTaffy for AxisStaticAlign {
    type Taffy = taffy::AxisStaticAlign;

    #[inline]
    fn into_taffy(self) -> taffy::AxisStaticAlign {
        let AxisStaticAlign {
            keyword,
            safety,
            fallback,
        } = self;
        taffy::AxisStaticAlign {
            keyword: keyword.into_taffy(),
            safety: safety.into_taffy(),
            fallback: fallback.into_taffy(),
        }
    }
}

impl FromTaffy for AxisStaticAlign {
    type Taffy = taffy::AxisStaticAlign;

    #[inline]
    fn from_taffy(value: taffy::AxisStaticAlign) -> Self {
        let taffy::AxisStaticAlign {
            keyword,
            safety,
            fallback,
        } = value;
        AxisStaticAlign {
            keyword: AxisStaticEdge::from_taffy(keyword),
            safety: StaticAlignSafety::from_taffy(safety),
            fallback: AxisStaticEdge::from_taffy(fallback),
        }
    }
}

impl IntoTaffy for AxisStaticPosition {
    type Taffy = taffy::AxisStaticPosition;

    #[inline]
    fn into_taffy(self) -> taffy::AxisStaticPosition {
        let AxisStaticPosition { area, align } = self;
        taffy::AxisStaticPosition {
            area: area.into_taffy(),
            align: align.into_taffy(),
        }
    }
}

impl FromTaffy for AxisStaticPosition {
    type Taffy = taffy::AxisStaticPosition;

    #[inline]
    fn from_taffy(value: taffy::AxisStaticPosition) -> Self {
        let taffy::AxisStaticPosition { area, align } = value;
        AxisStaticPosition {
            area: Line::from_taffy(area),
            align: AxisStaticAlign::from_taffy(align),
        }
    }
}

impl IntoTaffy for OofPosition {
    type Taffy = taffy::Position;

    #[inline]
    fn into_taffy(self) -> taffy::Position {
        match self {
            OofPosition::Absolute => taffy::Position::Absolute,
            OofPosition::Fixed => taffy::Position::Fixed,
        }
    }
}

impl FromTaffy for OofPosition {
    type Taffy = taffy::Position;

    /// The library emits a candidate only for a box that is out of flow, whose
    /// position is `Absolute` or `Fixed`. The positions that are in flow are
    /// not candidates and have no counterpart; they read as `Absolute`.
    #[inline]
    fn from_taffy(value: taffy::Position) -> Self {
        match value {
            taffy::Position::Fixed => OofPosition::Fixed,
            taffy::Position::Absolute
            | taffy::Position::Static
            | taffy::Position::Relative
            | taffy::Position::Sticky => OofPosition::Absolute,
        }
    }
}

impl IntoTaffy for OofCandidate {
    type Taffy = taffy::OofCandidate;

    #[inline]
    fn into_taffy(self) -> taffy::OofCandidate {
        let OofCandidate {
            node,
            order,
            position,
            static_position,
        } = self;
        taffy::OofCandidate {
            node: node.into_taffy(),
            order,
            position: position.into_taffy(),
            static_position: static_position.into_taffy(),
        }
    }
}

impl FromTaffy for OofCandidate {
    type Taffy = taffy::OofCandidate;

    #[inline]
    fn from_taffy(value: taffy::OofCandidate) -> Self {
        let taffy::OofCandidate {
            node,
            order,
            position,
            static_position,
        } = value;
        OofCandidate {
            node: NodeId::from_taffy(node),
            order,
            position: OofPosition::from_taffy(position),
            static_position: Point::from_taffy(static_position),
        }
    }
}

impl IntoTaffy for OofPositioningArea {
    type Taffy = taffy::OofPositioningArea;

    #[inline]
    fn into_taffy(self) -> taffy::OofPositioningArea {
        let OofPositioningArea { size, offset } = self;
        taffy::OofPositioningArea {
            size: size.into_taffy(),
            offset: offset.into_taffy(),
        }
    }
}

impl FromTaffy for OofPositioningArea {
    type Taffy = taffy::OofPositioningArea;

    #[inline]
    fn from_taffy(value: taffy::OofPositioningArea) -> Self {
        let taffy::OofPositioningArea { size, offset } = value;
        OofPositioningArea {
            size: Size::from_taffy(size),
            offset: Point::from_taffy(offset),
        }
    }
}

impl IntoTaffy for LayoutOutput {
    type Taffy = taffy::LayoutOutput;

    #[inline]
    fn into_taffy(self) -> taffy::LayoutOutput {
        let LayoutOutput {
            size,
            scrollable_overflow_rect,
            baselines,
            top_margin,
            bottom_margin,
            margins_can_collapse_through,
            oof_candidates,
            oof_positioning_area,
        } = self;
        taffy::LayoutOutput {
            size: size.into_taffy(),
            scrollable_overflow_rect: scrollable_overflow_rect.into_taffy(),
            baselines: baselines.into_taffy(),
            top_margin: top_margin.into_taffy(),
            bottom_margin: bottom_margin.into_taffy(),
            margins_can_collapse_through,
            oof_candidates: oof_candidates.into_taffy(),
            oof_positioning_area: oof_positioning_area.into_taffy(),
        }
    }
}

impl FromTaffy for LayoutOutput {
    type Taffy = taffy::LayoutOutput;

    #[inline]
    fn from_taffy(value: taffy::LayoutOutput) -> Self {
        let taffy::LayoutOutput {
            size,
            scrollable_overflow_rect,
            baselines,
            top_margin,
            bottom_margin,
            margins_can_collapse_through,
            oof_candidates,
            oof_positioning_area,
        } = value;
        LayoutOutput {
            size: Size::from_taffy(size),
            scrollable_overflow_rect: Edges::from_taffy(scrollable_overflow_rect),
            baselines: Baselines::from_taffy(baselines),
            top_margin: FromTaffy::from_taffy(top_margin),
            bottom_margin: FromTaffy::from_taffy(bottom_margin),
            margins_can_collapse_through,
            oof_candidates: FromTaffy::from_taffy(oof_candidates),
            oof_positioning_area: Option::from_taffy(oof_positioning_area),
        }
    }
}
