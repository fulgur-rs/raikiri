//! The adapter's conversions, cache and wrappers.
//!
//! Three properties are pinned here:
//!
//! 1. A value of the protocol survives the trip to the library's type and back
//!    bit for bit, including the floats that compare unequal to themselves or
//!    equal to a different bit pattern (`NaN`, infinities, `-0.0`) and the
//!    bounds of the value range (`±1e7`).
//! 2. Every variant of every enum the library defines for the protocol is
//!    listed. Each list matches on the library's type without a catch-all arm,
//!    so a variant the library gains stops the build here.
//! 3. The cache answers the same lookups in the same way as the library's
//!    cache does for the same inputs.

use super::convert::{FromTaffy, IntoTaffy};
use super::{LayoutCache, OofBatch, OofCandidates};
use crate::layout::protocol::{
    AbsoluteAxis, AvailableSpace, AxisStaticAlign, AxisStaticEdge, AxisStaticPosition, Baselines,
    CollapsibleMarginSet, LayoutInput, LayoutOutput, NodeId, OofCandidate, OofPosition,
    OofPositioningArea, RequestedAxis, RunMode, SizingMode, StaticAlignSafety,
};
use raikiri_layout_geometry::{BoxLayout, Edges, Line, Point, Size};

mod cache_keys;
mod resolve;
mod round_trip;
mod variants;
mod wrappers;

/// The floats that conversions must not change: every bit pattern that takes a
/// special path in a comparison or a sign test, and the range bounds.
fn extreme_floats() -> [f32; 9] {
    [
        f32::NAN,
        // A quiet `NaN` with a payload, which `==` and `Debug` cannot tell
        // from any other `NaN`.
        f32::from_bits(0x7fc0_1234),
        f32::INFINITY,
        f32::NEG_INFINITY,
        -0.0,
        0.0,
        1e7,
        -1e7,
        0.1,
    ]
}

/// A supply of the extreme floats, one after another, so that every field of a
/// value gets a different one.
struct Floats(usize);

impl Floats {
    fn next(&mut self) -> f32 {
        let floats = extreme_floats();
        let value = floats[self.0 % floats.len()];
        self.0 += 1;
        value
    }

    fn option(&mut self) -> Option<f32> {
        let value = self.next();
        // Every third one is missing, so `None` sits beside each extreme.
        (!self.0.is_multiple_of(3)).then_some(value)
    }
}

/// The bits of every float in a value, and whether each `Option` holds one.
trait Bits {
    fn bits(&self, out: &mut Vec<u32>);
}

impl Bits for f32 {
    fn bits(&self, out: &mut Vec<u32>) {
        out.push(self.to_bits());
    }
}

impl Bits for bool {
    fn bits(&self, out: &mut Vec<u32>) {
        out.push(u32::from(*self));
    }
}

impl Bits for u32 {
    fn bits(&self, out: &mut Vec<u32>) {
        out.push(*self);
    }
}

impl<T: Bits> Bits for Option<T> {
    fn bits(&self, out: &mut Vec<u32>) {
        match self {
            Some(value) => {
                out.push(1);
                value.bits(out);
            }
            None => out.push(0),
        }
    }
}

impl<T: Bits> Bits for Size<T> {
    fn bits(&self, out: &mut Vec<u32>) {
        self.width.bits(out);
        self.height.bits(out);
    }
}

impl<T: Bits> Bits for Point<T> {
    fn bits(&self, out: &mut Vec<u32>) {
        self.x.bits(out);
        self.y.bits(out);
    }
}

impl<T: Bits> Bits for Line<T> {
    fn bits(&self, out: &mut Vec<u32>) {
        self.start.bits(out);
        self.end.bits(out);
    }
}

impl<T: Bits> Bits for Edges<T> {
    fn bits(&self, out: &mut Vec<u32>) {
        self.left.bits(out);
        self.right.bits(out);
        self.top.bits(out);
        self.bottom.bits(out);
    }
}

impl Bits for BoxLayout {
    fn bits(&self, out: &mut Vec<u32>) {
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
        order.bits(out);
        location.bits(out);
        size.bits(out);
        scrollable_overflow_rect.bits(out);
        scrollbar_size.bits(out);
        border.bits(out);
        padding.bits(out);
        margin.bits(out);
    }
}

impl Bits for AvailableSpace {
    fn bits(&self, out: &mut Vec<u32>) {
        match self {
            AvailableSpace::Definite(value) => {
                out.push(0);
                value.bits(out);
            }
            AvailableSpace::MinContent => out.push(1),
            AvailableSpace::MaxContent => out.push(2),
        }
    }
}

impl Bits for Baselines {
    fn bits(&self, out: &mut Vec<u32>) {
        let Baselines { first, last } = self;
        first.bits(out);
        last.bits(out);
    }
}

impl Bits for AxisStaticPosition {
    fn bits(&self, out: &mut Vec<u32>) {
        let AxisStaticPosition { area, align } = self;
        area.bits(out);
        // The alignment holds no float; `Debug` tells its variants apart.
        out.extend(format!("{align:?}").bytes().map(u32::from));
    }
}

impl Bits for OofCandidate {
    fn bits(&self, out: &mut Vec<u32>) {
        let OofCandidate {
            node,
            order,
            position,
            static_position,
        } = self;
        out.push(usize::from(*node) as u32);
        order.bits(out);
        out.push(matches!(position, OofPosition::Fixed).into());
        static_position.bits(out);
    }
}

impl Bits for OofPositioningArea {
    fn bits(&self, out: &mut Vec<u32>) {
        let OofPositioningArea { size, offset } = self;
        size.bits(out);
        offset.bits(out);
    }
}

impl Bits for LayoutInput {
    fn bits(&self, out: &mut Vec<u32>) {
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
        out.extend(
            format!("{run_mode:?}{sizing_mode:?}{axis:?}")
                .bytes()
                .map(u32::from),
        );
        known_dimensions.bits(out);
        known_dimensions_are_definite.bits(out);
        parent_size.bits(out);
        available_space.width.bits(out);
        available_space.height.bits(out);
        vertical_margins_are_collapsible.bits(out);
    }
}

impl Bits for CollapsibleMarginSet {
    fn bits(&self, out: &mut Vec<u32>) {
        // The set keeps its two margins private; `resolve` reads both, and the
        // sign of the zero it can return is part of the answer.
        self.resolve().bits(out);
        out.extend(format!("{self:?}").bytes().map(u32::from));
    }
}

impl Bits for OofCandidates {
    fn bits(&self, out: &mut Vec<u32>) {
        for candidate in self.iter() {
            candidate.bits(out);
        }
        out.push(self.iter().len() as u32);
    }
}

impl Bits for LayoutOutput {
    fn bits(&self, out: &mut Vec<u32>) {
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
        size.bits(out);
        scrollable_overflow_rect.bits(out);
        baselines.bits(out);
        top_margin.bits(out);
        bottom_margin.bits(out);
        margins_can_collapse_through.bits(out);
        oof_candidates.bits(out);
        oof_positioning_area.bits(out);
    }
}

fn bits_of(value: &impl Bits) -> Vec<u32> {
    let mut out = Vec::new();
    value.bits(&mut out);
    out
}

/// `value` after a trip to the library's type and back.
fn round_trip<T>(value: T) -> T
where
    T: IntoTaffy + FromTaffy<Taffy = <T as IntoTaffy>::Taffy>,
{
    T::from_taffy(value.into_taffy())
}

/// A trip to the library's type and back leaves every bit of `value` as it was.
fn assert_round_trip<T>(value: T)
where
    T: IntoTaffy + FromTaffy<Taffy = <T as IntoTaffy>::Taffy> + Bits + Clone + std::fmt::Debug,
{
    let before = bits_of(&value);
    let debug_before = format!("{value:?}");
    let back = round_trip(value);
    assert_eq!(bits_of(&back), before, "bits of {debug_before}");
    assert_eq!(format!("{back:?}"), debug_before);
}

fn size(floats: &mut Floats) -> Size<f32> {
    Size {
        width: floats.next(),
        height: floats.next(),
    }
}

fn point(floats: &mut Floats) -> Point<f32> {
    Point {
        x: floats.next(),
        y: floats.next(),
    }
}

fn edges(floats: &mut Floats) -> Edges<f32> {
    Edges {
        left: floats.next(),
        right: floats.next(),
        top: floats.next(),
        bottom: floats.next(),
    }
}

fn option_size(floats: &mut Floats) -> Size<Option<f32>> {
    Size {
        width: floats.option(),
        height: floats.option(),
    }
}

fn available(floats: &mut Floats) -> AvailableSpace {
    match floats.0 % 3 {
        0 => AvailableSpace::Definite(floats.next()),
        1 => {
            floats.0 += 1;
            AvailableSpace::MinContent
        }
        _ => {
            floats.0 += 1;
            AvailableSpace::MaxContent
        }
    }
}

fn static_position(floats: &mut Floats, edge: AxisStaticEdge) -> AxisStaticPosition {
    AxisStaticPosition {
        area: Line {
            start: floats.next(),
            end: floats.next(),
        },
        align: AxisStaticAlign {
            keyword: edge,
            safety: StaticAlignSafety::Safe,
            fallback: AxisStaticEdge::End,
        },
    }
}

fn candidate(floats: &mut Floats, node: usize, position: OofPosition) -> OofCandidate {
    OofCandidate {
        node: NodeId::from(node),
        order: u32::MAX - node as u32,
        position,
        static_position: Point {
            x: static_position(floats, AxisStaticEdge::Start),
            y: static_position(floats, AxisStaticEdge::Center),
        },
    }
}

fn box_layout(floats: &mut Floats) -> BoxLayout {
    BoxLayout {
        order: 7,
        location: point(floats),
        size: size(floats),
        scrollable_overflow_rect: edges(floats),
        scrollbar_size: size(floats),
        border: edges(floats),
        padding: edges(floats),
        margin: edges(floats),
    }
}

fn layout_input(floats: &mut Floats) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::ContentSize,
        axis: RequestedAxis::Vertical,
        known_dimensions: option_size(floats),
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: option_size(floats),
        available_space: Size {
            width: available(floats),
            height: available(floats),
        },
        vertical_margins_are_collapsible: Line {
            start: false,
            end: true,
        },
    }
}

fn layout_output(floats: &mut Floats) -> LayoutOutput {
    let mut oof_candidates = OofCandidates::new();
    oof_candidates.push(candidate(floats, 3, OofPosition::Absolute));
    oof_candidates.push(candidate(floats, 4, OofPosition::Fixed));
    LayoutOutput {
        size: size(floats),
        scrollable_overflow_rect: edges(floats),
        baselines: Baselines {
            first: floats.option(),
            last: floats.option(),
        },
        top_margin: CollapsibleMarginSet::from_margin(floats.next()).collapse_with_margin(-3.5),
        bottom_margin: CollapsibleMarginSet::from_margin(-1e7).collapse_with_margin(1e7),
        margins_can_collapse_through: true,
        oof_candidates,
        oof_positioning_area: Some(OofPositioningArea {
            size: size(floats),
            offset: point(floats),
        }),
    }
}
