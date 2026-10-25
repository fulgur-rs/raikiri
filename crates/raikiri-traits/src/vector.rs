//! Vector paint scenes: resolution independent drawing commands.
//!
//! A [`VectorImage`] is what a producer such as an SVG builder hands to a
//! consumer that draws it. It is plain data: no parser, rasterizer or output
//! format type appears here, and the scene never names a file or a URL. Text
//! carries the bytes of the fonts it uses, shared, and not a font name.
//!
//! ## Space and size
//!
//! Coordinates are CSS pixels in the scene's own space. `(0, 0)` to
//! [`VectorImage::size`] is the viewport box the scene was built for, after
//! the source's own view box and aspect ratio rules have been applied. A
//! consumer places the scene in the page with one affine transform and a clip
//! to that box.
//!
//! ## What a scene carries
//!
//! The scene carries the opacity of its own content only. The opacity and
//! visibility of the element that shows it belong to the page's paint events,
//! so the consumer composites them once around the host box and the scene
//! together.
//!
//! Paths are already flattened to lines, quadratic and cubic curves: arcs,
//! basic shapes and instanced content are resolved by the producer.
//!
//! Gradients and clip paths are resolved to the user space of the path or
//! group that uses them, so a consumer needs no bounding box of its own.
//!
//! Text is already shaped and positioned: a [`TextRun`] lists glyphs of
//! [`FontFace`]s with the transform that places each one. A consumer never
//! selects a font, wraps a line or applies a script's rules.
//!
//! Every public enum and struct is `#[non_exhaustive]`. Producers build the
//! structs with their `new` constructors and set the public fields.

use crate::paint::{PaintColor, PaintTransform};

mod text;

pub use text::{FontFace, FontVariation, Glyph, TextRun};

/// A vector drawing of a fixed size.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct VectorImage {
    /// The viewport box the scene was built for, in CSS pixels.
    pub size: (f32, f32),
    /// The content, in the scene's space.
    pub root: Group,
}

impl VectorImage {
    /// A scene of `size` with no content.
    pub fn new(size: (f32, f32)) -> Self {
        Self {
            size,
            root: Group::new(PaintTransform::IDENTITY),
        }
    }

    /// The number of nodes in the scene, groups and the root included.
    pub fn node_count(&self) -> usize {
        1 + self.root.descendant_count()
    }

    /// An estimate of the heap memory the scene holds, in bytes, for charging
    /// it to a cache budget. It counts the storage of nodes, path segments,
    /// dash arrays and glyphs, not allocator overhead. The bytes of a font are
    /// shared with the producer and not counted.
    pub fn estimated_heap_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.root.heap_bytes()
    }
}

/// One drawing command or a container of them.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Node {
    /// A group with its own transform and opacity.
    Group(Group),
    /// A filled and/or stroked path.
    Path(Path),
    /// Shaped text.
    Text(TextRun),
}

/// A set of nodes drawn together.
///
/// The children are drawn in order, after `transform` has been applied to
/// their coordinates. The children are first composited with each other,
/// then clipped by `clip`, and the result is drawn with `opacity`. A group
/// with neither a clip nor an opacity below one only moves its children.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Group {
    /// Maps the children's coordinates to the enclosing space.
    pub transform: PaintTransform,
    /// Opacity of the composited children, in `0.0..=1.0`.
    pub opacity: f32,
    /// Limits where the children draw, in the same space as the children
    /// (after `transform`).
    pub clip: Option<Box<ClipPath>>,
    /// The children, back to front.
    pub children: Vec<Node>,
}

impl Group {
    /// An opaque, unclipped group with no children.
    pub fn new(transform: PaintTransform) -> Self {
        Self {
            transform,
            opacity: 1.0,
            clip: None,
            children: Vec::new(),
        }
    }

    fn descendant_count(&self) -> usize {
        let clip = self.clip.as_ref().map_or(0, |clip| clip.node_count());
        clip + self
            .children
            .iter()
            .map(|child| match child {
                Node::Group(group) => 1 + group.descendant_count(),
                Node::Path(_) | Node::Text(_) => 1,
            })
            .sum::<usize>()
    }

    fn heap_bytes(&self) -> usize {
        let own = self.children.capacity() * std::mem::size_of::<Node>();
        let clip = self.clip.as_ref().map_or(0, |clip| clip.heap_bytes());
        own + clip
            + self
                .children
                .iter()
                .map(|child| match child {
                    Node::Group(group) => group.heap_bytes(),
                    Node::Path(path) => path.heap_bytes(),
                    Node::Text(run) => run.heap_bytes(),
                })
                .sum::<usize>()
    }
}

/// An area that limits where a group draws.
///
/// The area is the union of what the paths of `root` cover, each filled by
/// its own fill rule; their paints, opacities and strokes do not count. A
/// group inside `root` moves, and may clip, the paths below it like any
/// group, but its opacity does not count either. When `clip` is set, the area
/// is the part of it that `clip` covers as well.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ClipPath {
    /// Maps the coordinates of `root` to the space of the clipped group's
    /// children.
    pub transform: PaintTransform,
    /// The shapes that make up the area.
    pub root: Group,
    /// A second clip that the area is intersected with. Its transform maps
    /// to the same space as this clip's.
    pub clip: Option<Box<ClipPath>>,
}

impl ClipPath {
    /// A clip of no shapes, which clips everything away.
    pub fn new(transform: PaintTransform) -> Self {
        Self {
            transform,
            root: Group::new(PaintTransform::IDENTITY),
            clip: None,
        }
    }

    /// The number of nodes in the clip: its root, what is below it, and the
    /// clips chained to it.
    pub fn node_count(&self) -> usize {
        1 + self.root.descendant_count() + self.clip.as_ref().map_or(0, |clip| clip.node_count())
    }

    fn heap_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.root.heap_bytes()
            + self.clip.as_ref().map_or(0, |clip| clip.heap_bytes())
    }
}

/// A path with its paint.
///
/// A path with neither a fill nor a stroke draws nothing.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Path {
    /// The outline, in the space of the enclosing group.
    pub segments: Vec<PathSegment>,
    /// How the inside is painted, if it is.
    pub fill: Option<Fill>,
    /// How the outline is painted, if it is.
    pub stroke: Option<Stroke>,
    /// Whether the fill or the stroke is drawn first.
    pub paint_order: PaintOrder,
    /// Whether the edges are smoothed. `false` asks for hard edges, as
    /// `shape-rendering: crispEdges` and `optimizeSpeed` do; a consumer that
    /// cannot control smoothing may ignore it.
    pub anti_alias: bool,
}

impl Path {
    /// An unpainted, smoothed path along `segments`.
    pub fn new(segments: Vec<PathSegment>) -> Self {
        Self {
            segments,
            fill: None,
            stroke: None,
            paint_order: PaintOrder::FillThenStroke,
            anti_alias: true,
        }
    }

    fn heap_bytes(&self) -> usize {
        self.segments.capacity() * std::mem::size_of::<PathSegment>()
            + self.fill.as_ref().map_or(0, |fill| fill.paint.heap_bytes())
            + self.stroke.as_ref().map_or(0, Stroke::heap_bytes)
    }
}

/// One step of a path.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PathSegment {
    /// Starts a new subpath at the point.
    MoveTo((f32, f32)),
    /// A straight line to the point.
    LineTo((f32, f32)),
    /// A quadratic curve.
    QuadTo {
        /// The control point.
        control: (f32, f32),
        /// The end point.
        to: (f32, f32),
    },
    /// A cubic curve.
    CubicTo {
        /// The first control point.
        control1: (f32, f32),
        /// The second control point.
        control2: (f32, f32),
        /// The end point.
        to: (f32, f32),
    },
    /// Closes the current subpath with a line to its start.
    Close,
}

/// The paint of the inside of a path.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Fill {
    /// What covers the inside.
    pub paint: Paint,
    /// Opacity of the paint, in `0.0..=1.0`.
    pub opacity: f32,
    /// Which points are inside.
    pub rule: FillRule,
}

impl Fill {
    /// An opaque non-zero fill with `paint`.
    pub fn new(paint: Paint) -> Self {
        Self {
            paint,
            opacity: 1.0,
            rule: FillRule::NonZero,
        }
    }
}

/// The paint of the outline of a path.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Stroke {
    /// What covers the outline.
    pub paint: Paint,
    /// Opacity of the paint, in `0.0..=1.0`.
    pub opacity: f32,
    /// Width of the outline, in the space of the path.
    pub width: f32,
    /// How open subpaths end.
    pub cap: LineCap,
    /// How segments meet.
    pub join: LineJoin,
    /// The longest miter, as a multiple of half the width.
    pub miter_limit: f32,
    /// The dash pattern; `None` draws a solid line.
    pub dash: Option<Dash>,
}

impl Stroke {
    /// An opaque solid stroke of `width` with butt caps and miter joins.
    pub fn new(paint: Paint, width: f32) -> Self {
        Self {
            paint,
            opacity: 1.0,
            width,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
            dash: None,
        }
    }

    fn heap_bytes(&self) -> usize {
        self.paint.heap_bytes()
            + self
                .dash
                .as_ref()
                .map_or(0, |dash| dash.array.capacity() * std::mem::size_of::<f32>())
    }
}

/// A dash pattern.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Dash {
    /// Alternating lengths of dashes and gaps, starting with a dash. It has
    /// an even number of positive entries.
    pub array: Vec<f32>,
    /// Where in the pattern the line starts.
    pub offset: f32,
}

impl Dash {
    /// A pattern of alternating dash and gap lengths.
    pub fn new(array: Vec<f32>, offset: f32) -> Self {
        Self { array, offset }
    }
}

/// What a fill or a stroke is painted with.
///
/// The alpha of a color, including the alpha of the stops of a gradient, is
/// part of the paint, on top of the opacity of the fill or stroke that holds
/// it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Paint {
    /// A single color.
    Color(PaintColor),
    /// Colors that change along a line.
    LinearGradient(Box<LinearGradient>),
    /// Colors that change outwards from a circle.
    RadialGradient(Box<RadialGradient>),
}

impl Paint {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Color(_) => 0,
            Self::LinearGradient(gradient) => {
                std::mem::size_of::<LinearGradient>() + gradient.stops_heap_bytes()
            }
            Self::RadialGradient(gradient) => {
                std::mem::size_of::<RadialGradient>() + gradient.stops_heap_bytes()
            }
        }
    }
}

/// One color of a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct GradientStop {
    /// Where along the gradient the color is reached, in `0.0..=1.0`.
    pub offset: f32,
    /// The color, with the opacity of the stop as its alpha.
    pub color: PaintColor,
}

impl GradientStop {
    /// A stop of `color` at `offset`.
    pub fn new(offset: f32, color: PaintColor) -> Self {
        Self { offset, color }
    }
}

/// What a gradient does outside the range of its stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpreadMethod {
    /// The first and last colors extend.
    Pad,
    /// The gradient repeats, alternately forwards and backwards.
    Reflect,
    /// The gradient repeats from its start.
    Repeat,
}

/// A gradient along the line from `start` to `end`.
///
/// The stops have at least two entries with non-decreasing offsets, and
/// `start` differs from `end`; a producer turns anything else into a solid
/// color or leaves the paint out.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LinearGradient {
    /// Where the gradient begins, in the gradient's own space.
    pub start: (f32, f32),
    /// Where the gradient ends, in the gradient's own space.
    pub end: (f32, f32),
    /// Maps the gradient's own space to the space of the path it paints.
    /// It can be inverted.
    pub transform: PaintTransform,
    /// What happens outside the range of the stops.
    pub spread: SpreadMethod,
    /// The colors, in the order of their offsets.
    pub stops: Vec<GradientStop>,
}

impl LinearGradient {
    /// A padded gradient from `start` to `end` in the space of the path.
    pub fn new(start: (f32, f32), end: (f32, f32), stops: Vec<GradientStop>) -> Self {
        Self {
            start,
            end,
            transform: PaintTransform::IDENTITY,
            spread: SpreadMethod::Pad,
            stops,
        }
    }

    fn stops_heap_bytes(&self) -> usize {
        self.stops.capacity() * std::mem::size_of::<GradientStop>()
    }
}

/// A gradient between a focal circle and an outer circle.
///
/// The color at offset zero lies on the circle of `focal_radius` around
/// `focal`, and the color at offset one on the circle of `radius` around
/// `center`. The stops are as for [`LinearGradient`], and `radius` is
/// positive.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RadialGradient {
    /// Center of the outer circle, in the gradient's own space.
    pub center: (f32, f32),
    /// Radius of the outer circle.
    pub radius: f32,
    /// Center of the focal circle.
    pub focal: (f32, f32),
    /// Radius of the focal circle.
    pub focal_radius: f32,
    /// Maps the gradient's own space to the space of the path it paints.
    /// It can be inverted.
    pub transform: PaintTransform,
    /// What happens outside the range of the stops.
    pub spread: SpreadMethod,
    /// The colors, in the order of their offsets.
    pub stops: Vec<GradientStop>,
}

impl RadialGradient {
    /// A padded gradient around `center` whose focal circle is the point at
    /// `center`.
    pub fn new(center: (f32, f32), radius: f32, stops: Vec<GradientStop>) -> Self {
        Self {
            center,
            radius,
            focal: center,
            focal_radius: 0.0,
            transform: PaintTransform::IDENTITY,
            spread: SpreadMethod::Pad,
            stops,
        }
    }

    fn stops_heap_bytes(&self) -> usize {
        self.stops.capacity() * std::mem::size_of::<GradientStop>()
    }
}

/// Which points of a path are inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FillRule {
    /// A point is inside when the path winds around it.
    NonZero,
    /// A point is inside when a ray from it crosses the path an odd number of times.
    EvenOdd,
}

/// How an open subpath ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineCap {
    /// Flat at the end point.
    Butt,
    /// A half circle around the end point.
    Round,
    /// Flat, half a width beyond the end point.
    Square,
}

/// How two segments of an outline meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineJoin {
    /// A sharp corner, cut to a bevel past the miter limit.
    Miter,
    /// A sharp corner, clipped at the miter limit instead of bevelled.
    MiterClip,
    /// A rounded corner.
    Round,
    /// A flat cut across the corner.
    Bevel,
}

/// The order in which the fill and the stroke of a path are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaintOrder {
    /// The stroke is drawn over the fill.
    FillThenStroke,
    /// The fill is drawn over the stroke.
    StrokeThenFill,
}

#[cfg(test)]
mod tests;
