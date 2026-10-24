//! Vector scenes: the parsed SVG as resolution independent paint commands.
//!
//! The scene is the drawing the raster path would produce, kept as geometry:
//! paths in the coordinates of nested groups, with the document's `viewBox`,
//! `preserveAspectRatio` and `transform` attributes folded into group
//! transforms. This step covers shapes and paths with solid fills and strokes,
//! group opacity and visibility. Gradients, patterns, clip paths, masks, blend
//! modes and images are reported as [`SvgSceneError::Unsupported`], so the
//! caller draws the raster of the whole image instead.

use crate::{
    SourceRootStyle, SvgDocument, SvgError, SvgResolvedStyles, SvgRootStyle, SvgViewport,
    resolved_source_style, within_four_ulps_of_one,
};
use raikiri_traits::vector::{
    Dash, Fill, FillRule, Group, LineCap, LineJoin, Node, Paint, PaintOrder, Path, PathSegment,
    Stroke, VectorImage,
};
use raikiri_traits::{PaintColor, PaintTransform};
use usvg::tiny_skia_path;

/// Most nodes, groups included, in one scene.
const MAX_SCENE_NODES: usize = 1_000_000;
/// Most path segments in one scene.
const MAX_SCENE_SEGMENTS: usize = 4_000_000;
/// Deepest nesting of groups in one scene.
///
/// The XML depth limit is 128; `use` elements can double the depth of the
/// tree that results.
const MAX_SCENE_GROUP_DEPTH: usize = 256;
/// The deepest chain of single-child wrappers searched for the group of the
/// root `<svg>` element: the view box transform and the element itself.
const ROOT_GROUP_SEARCH_DEPTH: usize = 3;

fn unsupported<T>(feature: SvgSceneFeature) -> Result<T, SvgSceneError> {
    Err(SvgSceneError::Unsupported(feature))
}

/// A reason the document is outside the vector subset.
///
/// The picture would change if the content were left out, so the caller draws
/// the document's raster instead. The set grows as the subset does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SvgSceneFeature {
    /// A linear or radial gradient paint.
    Gradient,
    /// A pattern paint.
    Pattern,
    /// A clip path.
    ClipPath,
    /// A mask.
    Mask,
    /// A blend mode other than normal.
    BlendMode,
    /// An embedded image.
    Image,
    /// A text element.
    Text,
    /// The group of the root `<svg>` element could not be told from the
    /// groups of its content, so the host opacity could not be left out.
    RootOpacity,
}

impl SvgSceneFeature {
    fn name(self) -> &'static str {
        match self {
            Self::Gradient => "gradients",
            Self::Pattern => "patterns",
            Self::ClipPath => "clip paths",
            Self::Mask => "masks",
            Self::BlendMode => "blend modes",
            Self::Image => "images",
            Self::Text => "text",
            Self::RootOpacity => "separating the root opacity",
        }
    }
}

/// A budget of the scene builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SvgSceneLimit {
    /// Nodes, groups included.
    Nodes,
    /// Path segments.
    PathSegments,
    /// Nesting depth of groups.
    GroupDepth,
}

impl SvgSceneLimit {
    fn name(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::PathSegments => "path segments",
            Self::GroupDepth => "group nesting levels",
        }
    }
}

/// Why a vector scene could not be built.
#[derive(Debug)]
#[non_exhaustive]
pub enum SvgSceneError {
    /// The request or the document is invalid, or over a parser or style
    /// preparation limit. The raster path fails the same way.
    Svg(SvgError),
    /// The document uses content outside the vector subset. Draw the raster
    /// of the whole image instead of mixing the two renderings.
    Unsupported(SvgSceneFeature),
    /// The scene would exceed a budget. The raster path would exceed its own
    /// limit as well, so this is an error and not a fallback.
    LimitExceeded {
        /// The budget.
        limit: SvgSceneLimit,
        /// The largest value it permits.
        max: usize,
    },
}

impl SvgSceneError {
    /// Whether the raster of the same document is a valid substitute.
    pub fn falls_back_to_raster(&self) -> bool {
        matches!(self, Self::Unsupported(_))
    }
}

impl std::fmt::Display for SvgSceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Svg(error) => error.fmt(f),
            Self::Unsupported(feature) => {
                write!(f, "SVG vector rendering does not cover {}", feature.name())
            }
            Self::LimitExceeded { limit, max } => {
                write!(f, "SVG scene exceeds the limit of {max} {}", limit.name())
            }
        }
    }
}

impl std::error::Error for SvgSceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Svg(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SvgError> for SvgSceneError {
    fn from(error: SvgError) -> Self {
        Self::Svg(error)
    }
}

/// The budgets of one scene; a field for each [`SvgSceneLimit`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct SceneLimits {
    pub(crate) nodes: usize,
    pub(crate) segments: usize,
    pub(crate) group_depth: usize,
}

impl Default for SceneLimits {
    fn default() -> Self {
        Self {
            nodes: MAX_SCENE_NODES,
            segments: MAX_SCENE_SEGMENTS,
            group_depth: MAX_SCENE_GROUP_DEPTH,
        }
    }
}

impl SvgDocument {
    /// Builds the vector scene of the document for `viewport`.
    ///
    /// The scene is `viewport` in size, in CSS pixels. The root `viewBox` and
    /// `preserveAspectRatio` are folded into its transforms, as they are for
    /// [`Self::rasterize`], so a consumer places the scene with one transform
    /// and a clip to its box.
    ///
    /// The host opacity is never part of the scene: the page composites it
    /// around the host box and the scene together. `root_style.opacity` still
    /// feeds inherited `opacity` declarations, as if
    /// `root_style.neutralize_root_opacity` were set, whatever its value.
    /// `root_style.visible` hides the root like it does for the raster, so a
    /// descendant that sets `visibility: visible` itself still paints.
    ///
    /// Content outside the vector subset gives
    /// [`SvgSceneError::Unsupported`]; see
    /// [`SvgSceneError::falls_back_to_raster`].
    pub fn scene(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
    ) -> Result<VectorImage, SvgSceneError> {
        self.build_scene(viewport, root_style, None, SceneLimits::default())
    }

    /// Builds the vector scene like [`Self::scene`] from the source that
    /// [`Self::styled_source_with_resolved_styles`] prepares: the host's
    /// resolved root color and font, its `visibility` and the descendant
    /// declarations apply exactly as they do for the raster.
    pub fn scene_with_resolved_styles(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        resolved: SvgResolvedStyles<'_>,
    ) -> Result<VectorImage, SvgSceneError> {
        let source_style = resolved_source_style(root_style, resolved)?;
        self.build_scene(
            viewport,
            root_style,
            Some(source_style),
            SceneLimits::default(),
        )
    }

    pub(crate) fn build_scene(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        source_style: Option<SourceRootStyle<'_>>,
        limits: SceneLimits,
    ) -> Result<VectorImage, SvgSceneError> {
        if !root_style.opacity.is_finite() || !(0.0..=1.0).contains(&root_style.opacity) {
            return Err(SvgError::InvalidOpacity.into());
        }
        check_viewport(viewport)?;
        let size = (viewport.width, viewport.height);
        // The host opacity is composited outside the scene.
        let root_style = SvgRootStyle {
            neutralize_root_opacity: true,
            ..root_style
        };
        // Malformed descendant styles are rejected whatever the root's
        // opacity or visibility.
        let prepared = source_style
            .map(|style| self.prepare_source(viewport, root_style, style, false))
            .transpose()?;
        // A host that draws nothing leaves nothing to build.
        if root_style.opacity <= 0.0 {
            return Ok(VectorImage::new(size));
        }
        let parsed = match self.prepared_tree(viewport, root_style, prepared) {
            Ok(parsed) => parsed,
            // A root that is not visible keeps the empty picture it always
            // had when its descendants cannot be drawn.
            Err(_) if !root_style.visible => return Ok(VectorImage::new(size)),
            Err(error) => return Err(error.into()),
        };
        let tree = parsed.as_ref().unwrap_or(&self.tree);
        let tree_size = tree.size();
        let fit = PaintTransform::new(
            viewport.width / tree_size.width(),
            0.0,
            0.0,
            viewport.height / tree_size.height(),
            0.0,
            0.0,
        );
        let host_opacity = root_style.opacity;
        let strips_opacity = host_opacity < 1.0 && !within_four_ulps_of_one(host_opacity);
        let host_group = if strips_opacity {
            host_group_depth(tree.root(), host_opacity)
        } else {
            None
        };
        let mut builder = Builder::new(limits);
        // The root of the scene counts as a node.
        builder.charge_node()?;
        let mut root = Group::new(fit * transform(tree.root().transform()));
        root.children = builder.children(tree.root(), 0, host_group)?;
        // A document that draws nothing has no group for the host opacity
        // either, and needs none.
        if strips_opacity && host_group.is_none() && !root.children.is_empty() {
            return unsupported(SvgSceneFeature::RootOpacity);
        }
        let mut image = VectorImage::new(size);
        image.root = root;
        Ok(image)
    }
}

fn check_viewport(viewport: SvgViewport) -> Result<(), SvgError> {
    let extent =
        |value: f32| value.is_finite() && value > 0.0 && value.ceil() <= i32::MAX as f32 / 4.0;
    if extent(viewport.width) && extent(viewport.height) {
        Ok(())
    } else {
        Err(SvgError::InvalidViewport)
    }
}

/// The depth of the group of the root `<svg>` element, the one that carries
/// `host_opacity`, counted from `root` (whose children are at depth one).
/// Only single-child wrappers of opacity one may lead to it. `None` when the
/// tree does not have that shape.
fn host_group_depth(root: &usvg::Group, host_opacity: f32) -> Option<usize> {
    let mut group = root;
    for depth in 1..=ROOT_GROUP_SEARCH_DEPTH {
        let [usvg::Node::Group(child)] = group.children() else {
            return None;
        };
        let opacity = child.opacity().get();
        if (opacity - host_opacity).abs() <= f32::EPSILON {
            return Some(depth);
        }
        // Only the wrapper of the view box transform may come first.
        if opacity != 1.0 {
            return None;
        }
        group = child;
    }
    None
}

struct Builder {
    limits: SceneLimits,
    nodes: usize,
    segments: usize,
}

impl Builder {
    fn new(limits: SceneLimits) -> Self {
        Self {
            limits,
            nodes: 0,
            segments: 0,
        }
    }

    fn charge_node(&mut self) -> Result<(), SvgSceneError> {
        self.nodes += 1;
        if self.nodes > self.limits.nodes {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::Nodes,
                max: self.limits.nodes,
            });
        }
        Ok(())
    }

    fn charge_segments(&mut self, count: usize) -> Result<(), SvgSceneError> {
        self.segments = self.segments.saturating_add(count);
        if self.segments > self.limits.segments {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::PathSegments,
                max: self.limits.segments,
            });
        }
        Ok(())
    }

    /// The nodes of `group`'s children. `host_group` is the depth of the group
    /// that holds the host opacity, while `group` is on the chain of
    /// single-child wrappers that leads to it.
    fn children(
        &mut self,
        group: &usvg::Group,
        depth: usize,
        host_group: Option<usize>,
    ) -> Result<Vec<Node>, SvgSceneError> {
        let mut nodes = Vec::with_capacity(group.children().len());
        for child in group.children() {
            match child {
                usvg::Node::Group(child) => {
                    nodes.extend(self.group(child, depth + 1, host_group)?);
                }
                usvg::Node::Path(path) => {
                    nodes.extend(self.path(path)?.map(Node::Path));
                }
                usvg::Node::Image(_) => return unsupported(SvgSceneFeature::Image),
                usvg::Node::Text(_) => return unsupported(SvgSceneFeature::Text),
            }
        }
        Ok(nodes)
    }

    /// The nodes that stand for `group`: the group itself, or its children
    /// when the group changes nothing, or none when it draws nothing.
    fn group(
        &mut self,
        group: &usvg::Group,
        depth: usize,
        host_group: Option<usize>,
    ) -> Result<Vec<Node>, SvgSceneError> {
        if depth > self.limits.group_depth {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::GroupDepth,
                max: self.limits.group_depth,
            });
        }
        if group.clip_path().is_some() {
            return unsupported(SvgSceneFeature::ClipPath);
        }
        if group.mask().is_some() {
            return unsupported(SvgSceneFeature::Mask);
        }
        if !group.filters().is_empty() {
            return Err(SvgError::UnsupportedFilterEffects.into());
        }
        if group.blend_mode() != usvg::BlendMode::Normal {
            return unsupported(SvgSceneFeature::BlendMode);
        }
        self.charge_node()?;
        let mut node = Group::new(transform(group.transform()));
        // The host composites its own opacity around the scene.
        node.opacity = if host_group == Some(depth) {
            1.0
        } else {
            group.opacity().get()
        };
        // Every group on the chain is the only child of its parent, so the
        // groups reached with a host depth are exactly those on it.
        let on_chain = host_group.filter(|&host| host > depth);
        node.children = self.children(group, depth, on_chain)?;
        if node.children.is_empty() {
            self.nodes -= 1;
            return Ok(Vec::new());
        }
        if node.opacity == 1.0 && node.transform == PaintTransform::IDENTITY {
            self.nodes -= 1;
            return Ok(node.children);
        }
        Ok(vec![Node::Group(node)])
    }

    fn path(&mut self, path: &usvg::Path) -> Result<Option<Path>, SvgSceneError> {
        if !path.is_visible() {
            return Ok(None);
        }
        let fill = path.fill().map(fill).transpose()?;
        let stroke = path.stroke().map(stroke).transpose()?;
        if fill.is_none() && stroke.is_none() {
            return Ok(None);
        }
        self.charge_node()?;
        self.charge_segments(path.data().len())?;
        let mut node = Path::new(segments(path.data()));
        node.fill = fill;
        node.stroke = stroke;
        node.paint_order = match path.paint_order() {
            usvg::PaintOrder::FillAndStroke => PaintOrder::FillThenStroke,
            usvg::PaintOrder::StrokeAndFill => PaintOrder::StrokeThenFill,
        };
        Ok(Some(node))
    }
}

fn transform(transform: usvg::tiny_skia_path::Transform) -> PaintTransform {
    PaintTransform::new(
        transform.sx,
        transform.ky,
        transform.kx,
        transform.sy,
        transform.tx,
        transform.ty,
    )
}

fn segments(path: &tiny_skia_path::Path) -> Vec<PathSegment> {
    let point = |point: tiny_skia_path::Point| (point.x, point.y);
    path.segments()
        .map(|segment| match segment {
            tiny_skia_path::PathSegment::MoveTo(to) => PathSegment::MoveTo(point(to)),
            tiny_skia_path::PathSegment::LineTo(to) => PathSegment::LineTo(point(to)),
            tiny_skia_path::PathSegment::QuadTo(control, to) => PathSegment::QuadTo {
                control: point(control),
                to: point(to),
            },
            tiny_skia_path::PathSegment::CubicTo(control1, control2, to) => PathSegment::CubicTo {
                control1: point(control1),
                control2: point(control2),
                to: point(to),
            },
            tiny_skia_path::PathSegment::Close => PathSegment::Close,
        })
        .collect()
}

fn paint(paint: &usvg::Paint) -> Result<Paint, SvgSceneError> {
    match paint {
        usvg::Paint::Color(color) => Ok(Paint::Color(PaintColor::new(
            f32::from(color.red) / 255.0,
            f32::from(color.green) / 255.0,
            f32::from(color.blue) / 255.0,
            1.0,
        ))),
        usvg::Paint::LinearGradient(_) | usvg::Paint::RadialGradient(_) => {
            unsupported(SvgSceneFeature::Gradient)
        }
        usvg::Paint::Pattern(_) => unsupported(SvgSceneFeature::Pattern),
    }
}

fn fill(fill: &usvg::Fill) -> Result<Fill, SvgSceneError> {
    let mut node = Fill::new(paint(fill.paint())?);
    node.opacity = fill.opacity().get();
    node.rule = match fill.rule() {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    };
    Ok(node)
}

fn stroke(stroke: &usvg::Stroke) -> Result<Stroke, SvgSceneError> {
    let mut node = Stroke::new(paint(stroke.paint())?, stroke.width().get());
    node.opacity = stroke.opacity().get();
    node.cap = match stroke.linecap() {
        usvg::LineCap::Butt => LineCap::Butt,
        usvg::LineCap::Round => LineCap::Round,
        usvg::LineCap::Square => LineCap::Square,
    };
    node.join = match stroke.linejoin() {
        usvg::LineJoin::Miter => LineJoin::Miter,
        usvg::LineJoin::MiterClip => LineJoin::MiterClip,
        usvg::LineJoin::Round => LineJoin::Round,
        usvg::LineJoin::Bevel => LineJoin::Bevel,
    };
    node.miter_limit = stroke.miterlimit().get();
    // A pattern the stroker cannot use draws a solid line, like the raster.
    let offset = stroke.dashoffset();
    node.dash = stroke
        .dasharray()
        .filter(|array| tiny_skia_path::StrokeDash::new(array.to_vec(), offset).is_some())
        .map(|array| Dash::new(array.to_vec(), offset));
    Ok(node)
}

#[cfg(test)]
mod tests;
