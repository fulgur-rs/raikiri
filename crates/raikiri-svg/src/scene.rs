//! Vector scenes: the parsed SVG as resolution independent paint commands.
//!
//! The scene is the drawing the raster path would produce, kept as geometry:
//! paths in the coordinates of nested groups, with the document's `viewBox`,
//! `preserveAspectRatio` and `transform` attributes folded into group
//! transforms. It covers shapes and paths with solid and gradient fills and
//! strokes, clip paths, group opacity and visibility, and text when the caller
//! supplies fonts, and raster pictures when the caller supplies them. Patterns,
//! masks and blend modes are reported as [`SvgSceneError::Unsupported`], so the
//! caller draws the raster of the whole image instead.
//!
//! ## Text
//!
//! The parser shapes and positions text; the faces come from the caller's
//! [`SvgFontSource`](crate::SvgFontSource), so SVG text is set in the fonts of the HTML around it.
//! Each styled span becomes a [`TextRun`] of glyphs, with its decorations as
//! paths around it. Text inside a clip path is flattened to outlines, because
//! a clip is geometry. Without fonts the text is left out, as the raster path
//! leaves it out.
//!
//! ## Pictures
//!
//! An `<image>` is a [`RasterImage`] of the pixels that the caller's
//! [`SvgImageSource`](crate::SvgImageSource) decoded, in a group that carries the size, the position
//! and the `preserveAspectRatio` clip of the element. A picture the source
//! does not give is left out, like a broken picture in HTML, and a document
//! built without a source has no pictures in its scene. The pictures count
//! toward a budget of their pixels, so a scene is no bigger than the raster of
//! the same document could be.

use std::sync::Arc;

use crate::fonts::FaceMap;
use crate::images::{ImageMap, MAX_SCENE_IMAGES};
use crate::{
    SourceRootStyle, SvgDocument, SvgError, SvgResolvedStyles, SvgResources, SvgRootStyle,
    SvgViewport, resolved_source_style, within_four_ulps_of_one,
};
use raikiri_traits::vector::{
    ClipPath, Dash, Fill, FillRule, FontVariation, Glyph, GradientStop, Group, LineCap, LineJoin,
    LinearGradient, Node, Paint, PaintOrder, Path, PathSegment, RadialGradient, RasterImage,
    SpreadMethod, Stroke, TextRun, VectorImage,
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
/// Deepest nesting of clip paths in one scene: a clip inside a clip, or
/// chained to one.
const MAX_SCENE_CLIP_DEPTH: usize = 64;
/// Most stops in one gradient.
const MAX_GRADIENT_STOPS: usize = 1_024;
/// Most gradient stops in one scene, over all its gradients.
const MAX_SCENE_GRADIENT_STOPS: usize = 1_000_000;
/// Most glyphs in one scene.
const MAX_SCENE_GLYPHS: usize = 1_000_000;
/// Most pixel bytes the pictures of one document may hold together, each
/// distinct picture counting once: the output limit of the raster path.
const MAX_SCENE_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// The length below which the two points of a gradient count as one, as the
/// raster path counts them.
const DEGENERATE_GRADIENT_LENGTH: f32 = 1.0 / 32_768.0;
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
    /// A pattern paint.
    Pattern,
    /// A mask.
    Mask,
    /// A filter effect.
    Filter,
    /// A blend mode other than normal.
    BlendMode,
    /// An image that is not a raster picture of the scene's source, such as a
    /// nested SVG document.
    Image,
    /// The group of the root `<svg>` element could not be told from the
    /// groups of its content, so the host opacity could not be left out.
    RootOpacity,
}

impl SvgSceneFeature {
    fn name(self) -> &'static str {
        match self {
            Self::Pattern => "patterns",
            Self::Mask => "masks",
            Self::Filter => "filter effects",
            Self::BlendMode => "blend modes",
            Self::Image => "nested images",
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
    /// Nesting depth of clip paths.
    ClipDepth,
    /// Stops in one gradient.
    GradientStops,
    /// Stops in all the gradients of the scene together.
    SceneGradientStops,
    /// Glyphs in the text of the scene.
    Glyphs,
    /// Different pictures the document asks its source for.
    Images,
    /// Pixel bytes the pictures of the document hold together, each distinct
    /// picture counted once, whether the scene draws it or not.
    ImageBytes,
}

impl SvgSceneLimit {
    fn name(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::PathSegments => "path segments",
            Self::GroupDepth => "group nesting levels",
            Self::ClipDepth => "clip path nesting levels",
            Self::GradientStops => "stops in one gradient",
            Self::SceneGradientStops => "gradient stops",
            Self::Glyphs => "glyphs",
            Self::Images => "pictures",
            Self::ImageBytes => "picture bytes",
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
    /// The scene would exceed a budget. This is an error of the scene and not
    /// a signal that the raster is a valid substitute
    /// ([`SvgSceneError::falls_back_to_raster`] is `false`); a caller that can
    /// draw pixels may still try, within the limits of the raster path.
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
    pub(crate) clip_depth: usize,
    pub(crate) gradient_stops: usize,
    pub(crate) scene_gradient_stops: usize,
    pub(crate) glyphs: usize,
    pub(crate) image_bytes: usize,
}

impl Default for SceneLimits {
    fn default() -> Self {
        Self {
            nodes: MAX_SCENE_NODES,
            segments: MAX_SCENE_SEGMENTS,
            group_depth: MAX_SCENE_GROUP_DEPTH,
            clip_depth: MAX_SCENE_CLIP_DEPTH,
            gradient_stops: MAX_GRADIENT_STOPS,
            scene_gradient_stops: MAX_SCENE_GRADIENT_STOPS,
            glyphs: MAX_SCENE_GLYPHS,
            image_bytes: MAX_SCENE_IMAGE_BYTES,
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
    /// The exception is a document with no host, which is the whole picture:
    /// with `root_style.opacity` one, `neutralize_root_opacity` unset and no
    /// resolved styles, the root's own `opacity` stays part of the scene, as
    /// it stays part of [`Self::rasterize`]'s output.
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
        self.build_scene(
            viewport,
            root_style,
            None,
            SceneLimits::default(),
            SvgResources::new(),
        )
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
            SvgResources::new(),
        )
    }

    /// Builds the vector scene like [`Self::scene`], or like
    /// [`Self::scene_with_resolved_styles`] when `resolved` is given, with
    /// the document's text and pictures taken from `resources`.
    ///
    /// The scene holds the text as glyphs of the faces of `resources.fonts`,
    /// so the faces must outlive whatever shows the scene; the scene shares
    /// their bytes and does not copy them. Text that no face can set is left
    /// out, like it is for [`Self::scene`], and a character that the selected
    /// face lacks is set in the face that the font source gives as its
    /// fallback.
    ///
    /// The scene holds each `<image>` as the shared pixels that the picture
    /// source gave for it. A picture it did not give is left out. A document
    /// that asks for more than 256 different pictures, or whose pictures
    /// total more than 32 MiB of pixels, is an error and no fallback, because
    /// the raster path does not draw pictures at all.
    pub fn scene_with_resources(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        resolved: Option<SvgResolvedStyles<'_>>,
        resources: SvgResources<'_>,
    ) -> Result<VectorImage, SvgSceneError> {
        let source_style = resolved
            .map(|resolved| resolved_source_style(root_style, resolved))
            .transpose()?;
        self.build_scene(
            viewport,
            root_style,
            source_style,
            SceneLimits::default(),
            resources,
        )
    }

    pub(crate) fn build_scene(
        &self,
        viewport: SvgViewport,
        root_style: SvgRootStyle,
        source_style: Option<SourceRootStyle<'_>>,
        limits: SceneLimits,
        resources: SvgResources<'_>,
    ) -> Result<VectorImage, SvgSceneError> {
        if !root_style.opacity.is_finite() || !(0.0..=1.0).contains(&root_style.opacity) {
            return Err(SvgError::InvalidOpacity.into());
        }
        check_viewport(viewport)?;
        let size = (viewport.width, viewport.height);
        // The host opacity is composited outside the scene. A document that
        // has no host (opacity one, no host styles, nothing asked to be
        // neutralized) is the whole picture, and its own root opacity is part
        // of it, as it is of its raster.
        let has_host = source_style.is_some()
            || root_style.neutralize_root_opacity
            || (root_style.opacity < 1.0 && !within_four_ulps_of_one(root_style.opacity));
        let root_style = SvgRootStyle {
            neutralize_root_opacity: has_host,
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
        let parsed = match self.prepared_tree_with(
            viewport,
            root_style,
            prepared,
            resources,
            limits.image_bytes,
        ) {
            Ok(parsed) => parsed,
            // A root that is not visible keeps the empty picture it always
            // had when its descendants cannot be drawn.
            Err(_) if !root_style.visible => return Ok(VectorImage::new(size)),
            Err(error) => return Err(error.into()),
        };
        let (no_faces, no_images) = (FaceMap::default(), ImageMap::default());
        let (tree, faces, images) = parsed.as_ref().map_or_else(
            || {
                (
                    self.tree
                        .as_ref()
                        .expect("source-only documents always prepare a tree"),
                    &no_faces,
                    &no_images,
                )
            },
            |parsed| (&parsed.tree, &parsed.faces, &parsed.images),
        );
        if images.overflowed() {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::Images,
                max: MAX_SCENE_IMAGES,
            });
        }
        if images.bytes_exceeded() {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::ImageBytes,
                max: limits.image_bytes,
            });
        }
        scene_from_tree(tree, faces, images, viewport, root_style.opacity, limits)
    }
}

/// The scene of `tree` placed in `viewport`. The root `<svg>` element of the
/// tree carries `host_opacity` on behalf of the host; the scene leaves that
/// opacity out.
fn scene_from_tree(
    tree: &usvg::Tree,
    faces: &FaceMap,
    images: &ImageMap,
    viewport: SvgViewport,
    host_opacity: f32,
    limits: SceneLimits,
) -> Result<VectorImage, SvgSceneError> {
    let tree_size = tree.size();
    let fit = PaintTransform::new(
        viewport.width / tree_size.width(),
        0.0,
        0.0,
        viewport.height / tree_size.height(),
        0.0,
        0.0,
    );
    let strips_opacity = host_opacity < 1.0 && !within_four_ulps_of_one(host_opacity);
    let host_group = if strips_opacity {
        host_group_depth(tree.root(), host_opacity)
    } else {
        None
    };
    let mut builder = Builder::new(limits, faces, images);
    // The root of the scene counts as a node.
    builder.charge_node()?;
    let mut root = Group::new(fit * transform(tree.root().transform()));
    root.children = builder.children(tree.root(), 0, host_group)?;
    // A document that draws nothing has no group for the host opacity
    // either, and needs none.
    if strips_opacity && host_group.is_none() && !root.children.is_empty() {
        return unsupported(SvgSceneFeature::RootOpacity);
    }
    let mut image = VectorImage::new((viewport.width, viewport.height));
    image.root = root;
    Ok(image)
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

struct Builder<'a> {
    limits: SceneLimits,
    /// The face behind each font id of the tree's glyphs.
    faces: &'a FaceMap,
    /// The pixels behind each picture node.
    images: &'a ImageMap,
    nodes: usize,
    segments: usize,
    stops: usize,
    glyphs: usize,
    clip_depth: usize,
}

impl<'a> Builder<'a> {
    fn new(limits: SceneLimits, faces: &'a FaceMap, images: &'a ImageMap) -> Self {
        Self {
            limits,
            faces,
            images,
            nodes: 0,
            segments: 0,
            stops: 0,
            glyphs: 0,
            clip_depth: 0,
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

    fn charge_glyphs(&mut self, count: usize) -> Result<(), SvgSceneError> {
        self.glyphs = self.glyphs.saturating_add(count);
        if self.glyphs > self.limits.glyphs {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::Glyphs,
                max: self.limits.glyphs,
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
                usvg::Node::Text(text) => {
                    nodes.extend(self.text(text, depth)?);
                }
                usvg::Node::Image(image) => {
                    nodes.extend(self.image(image)?);
                }
            }
        }
        Ok(nodes)
    }

    /// The node for `image`, or `None` when it is not visible. The group the
    /// parser wraps it in scales it from its pixels to its box.
    fn image(&mut self, image: &usvg::Image) -> Result<Option<Node>, SvgSceneError> {
        // Only the pictures of the source are known; a nested SVG document
        // or any other picture keeps the document on the raster.
        let found = match image.kind() {
            usvg::ImageKind::PNG(header) => self.images.get(header),
            _ => None,
        };
        let Some(pixels) = found else {
            return unsupported(SvgSceneFeature::Image);
        };
        if !image.is_visible() {
            return Ok(None);
        }
        self.charge_node()?;
        let mut picture = RasterImage::new(Arc::clone(pixels));
        picture.smooth = !matches!(
            image.rendering_mode(),
            usvg::ImageRendering::OptimizeSpeed
                | usvg::ImageRendering::CrispEdges
                | usvg::ImageRendering::Pixelated
        );
        Ok(Some(Node::Image(picture)))
    }

    /// The nodes that stand for `text`: for each visible span its decorations
    /// and a run of its glyphs. Text that no face could set has no spans.
    fn text(&mut self, text: &usvg::Text, depth: usize) -> Result<Vec<Node>, SvgSceneError> {
        // A clip is geometry, so the outlines the parser flattened the glyphs
        // to take the place of the run.
        if self.clip_depth > 0 {
            return self.children(text.flattened(), depth, None);
        }
        // The parser draws `optimizeSpeed` text with crisp edges.
        let anti_alias = text.rendering_mode() != usvg::TextRendering::OptimizeSpeed;
        let mut nodes = Vec::new();
        for span in text.layouted().iter().filter(|span| span.visible) {
            for decoration in [&span.overline, &span.underline] {
                nodes.extend(self.decoration(decoration.as_ref())?);
            }
            nodes.extend(self.run(span, anti_alias)?.map(Node::Text));
            nodes.extend(self.decoration(span.line_through.as_ref())?);
        }
        Ok(nodes)
    }

    fn decoration(&mut self, line: Option<&usvg::Path>) -> Result<Option<Node>, SvgSceneError> {
        line.map_or(Ok(None), |line| self.path(line))
            .map(|path| path.map(Node::Path))
    }

    /// The run for the glyphs of `span`, or `None` when it has no paint. A
    /// glyph of a face this scene does not know is left out.
    fn run(
        &mut self,
        span: &usvg::layout::Span,
        anti_alias: bool,
    ) -> Result<Option<TextRun>, SvgSceneError> {
        let fill = span
            .fill
            .as_ref()
            .map(|fill| self.fill(fill))
            .transpose()?
            .flatten();
        let stroke = span
            .stroke
            .as_ref()
            .map(|stroke| self.stroke(stroke))
            .transpose()?
            .flatten();
        if fill.is_none() && stroke.is_none() {
            return Ok(None);
        }
        // Charge before the copy, which is as large as the parser's own.
        self.charge_glyphs(span.positioned_glyphs.len())?;
        let glyphs: Vec<Glyph> = span
            .positioned_glyphs
            .iter()
            .filter_map(|glyph| {
                let font = self.faces.get(glyph.font)?;
                Some(Glyph::new(
                    font.clone(),
                    glyph.id.0,
                    glyph.text.as_str(),
                    transform(glyph.transform()),
                ))
            })
            .collect();
        self.charge_node()?;
        let mut run = TextRun::new(span.font_size.get(), glyphs);
        run.variations = span
            .variations
            .iter()
            .map(|variation| FontVariation::new(variation.tag, variation.value))
            .collect();
        // The parser shaped and flattened the glyphs with the optical size set
        // to the font size, unless the text turns that off or sets the axis
        // itself. A face without the axis ignores the setting.
        if span.font_optical_sizing == usvg::FontOpticalSizing::Auto
            && !run
                .variations
                .iter()
                .any(|variation| &variation.tag == b"opsz")
            && span
                .positioned_glyphs
                .iter()
                .any(|glyph| self.faces.has_opsz(glyph.font))
        {
            run.variations
                .push(FontVariation::new(*b"opsz", span.font_size.get()));
        }
        run.fill = fill;
        run.stroke = stroke;
        run.paint_order = paint_order(span.paint_order);
        run.anti_alias = anti_alias;
        Ok(Some(run))
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
        if group.mask().is_some() {
            return unsupported(SvgSceneFeature::Mask);
        }
        if !group.filters().is_empty() {
            return unsupported(SvgSceneFeature::Filter);
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
        node.clip = group
            .clip_path()
            .map(|clip| self.clip(clip, depth).map(Box::new))
            .transpose()?;
        if node.opacity == 1.0 && node.transform == PaintTransform::IDENTITY && node.clip.is_none()
        {
            self.nodes -= 1;
            return Ok(node.children);
        }
        Ok(vec![Node::Group(node)])
    }

    /// The clip for a group at `depth`. What is drawn inside the clip nests
    /// below that depth, so clips share the budget of the groups.
    fn clip(&mut self, clip: &usvg::ClipPath, depth: usize) -> Result<ClipPath, SvgSceneError> {
        if self.clip_depth >= self.limits.clip_depth {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::ClipDepth,
                max: self.limits.clip_depth,
            });
        }
        self.clip_depth += 1;
        self.charge_node()?;
        let mut node = ClipPath::new(transform(clip.transform()));
        node.root.children = self.children(clip.root(), depth, None)?;
        // The chained clip is converted after the shapes, not inside them, so
        // only the clips that really nest add to the depth.
        node.clip = clip
            .clip_path()
            .map(|chained| self.clip(chained, depth).map(Box::new))
            .transpose()?;
        self.clip_depth -= 1;
        Ok(node)
    }

    fn path(&mut self, path: &usvg::Path) -> Result<Option<Path>, SvgSceneError> {
        if !path.is_visible() {
            return Ok(None);
        }
        let fill = path
            .fill()
            .map(|fill| self.fill(fill))
            .transpose()?
            .flatten();
        let stroke = path
            .stroke()
            .map(|stroke| self.stroke(stroke))
            .transpose()?
            .flatten();
        // A paint that cannot be drawn leaves nothing to paint.
        if fill.is_none() && stroke.is_none() {
            return Ok(None);
        }
        self.charge_node()?;
        self.charge_segments(path.data().len())?;
        let mut node = Path::new(segments(path.data()));
        node.fill = fill;
        node.stroke = stroke;
        node.anti_alias = path.rendering_mode().use_shape_antialiasing();
        node.paint_order = paint_order(path.paint_order());
        Ok(Some(node))
    }

    /// The paint for `paint`, or `None` when it paints nothing.
    fn paint(&mut self, paint: &usvg::Paint) -> Result<Option<Paint>, SvgSceneError> {
        match paint {
            usvg::Paint::Color(color) => Ok(Some(Paint::Color(paint_color(*color, 1.0)))),
            usvg::Paint::LinearGradient(gradient) => self.linear_gradient(gradient),
            usvg::Paint::RadialGradient(gradient) => self.radial_gradient(gradient),
            usvg::Paint::Pattern(_) => unsupported(SvgSceneFeature::Pattern),
        }
    }

    fn linear_gradient(
        &mut self,
        gradient: &usvg::LinearGradient,
    ) -> Result<Option<Paint>, SvgSceneError> {
        let Some((stops, spread, transform)) = self.gradient_base(gradient)? else {
            return Ok(None);
        };
        let start = (gradient.x1(), gradient.y1());
        let end = (gradient.x2(), gradient.y2());
        // A gradient without a length paints the color of its last stop.
        if (end.0 - start.0).hypot(end.1 - start.1) <= DEGENERATE_GRADIENT_LENGTH {
            return Ok(Some(Paint::Color(stops[stops.len() - 1].color)));
        }
        let mut node = LinearGradient::new(start, end, stops);
        node.transform = transform;
        node.spread = spread;
        Ok(Some(Paint::LinearGradient(Box::new(node))))
    }

    fn radial_gradient(
        &mut self,
        gradient: &usvg::RadialGradient,
    ) -> Result<Option<Paint>, SvgSceneError> {
        let Some((stops, spread, transform)) = self.gradient_base(gradient)? else {
            return Ok(None);
        };
        let mut node =
            RadialGradient::new((gradient.cx(), gradient.cy()), gradient.r().get(), stops);
        node.focal = (gradient.fx(), gradient.fy());
        node.focal_radius = gradient.fr().get();
        node.transform = transform;
        node.spread = spread;
        Ok(Some(Paint::RadialGradient(Box::new(node))))
    }

    /// The stops, spread and transform that gradients share. `None` when the
    /// gradient cannot be drawn, which paints nothing like the raster.
    fn gradient_base(
        &mut self,
        gradient: &usvg::BaseGradient,
    ) -> Result<Option<(Vec<GradientStop>, SpreadMethod, PaintTransform)>, SvgSceneError> {
        let count = gradient.stops().len();
        if count > self.limits.gradient_stops {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::GradientStops,
                max: self.limits.gradient_stops,
            });
        }
        self.stops = self.stops.saturating_add(count);
        if self.stops > self.limits.scene_gradient_stops {
            return Err(SvgSceneError::LimitExceeded {
                limit: SvgSceneLimit::SceneGradientStops,
                max: self.limits.scene_gradient_stops,
            });
        }
        if gradient.transform().invert().is_none() {
            return Ok(None);
        }
        let stops = gradient
            .stops()
            .iter()
            .map(|stop| {
                GradientStop::new(
                    stop.offset().get(),
                    paint_color(stop.color(), stop.opacity().get()),
                )
            })
            .collect();
        let spread = match gradient.spread_method() {
            usvg::SpreadMethod::Pad => SpreadMethod::Pad,
            usvg::SpreadMethod::Reflect => SpreadMethod::Reflect,
            usvg::SpreadMethod::Repeat => SpreadMethod::Repeat,
        };
        Ok(Some((stops, spread, transform(gradient.transform()))))
    }

    fn fill(&mut self, fill: &usvg::Fill) -> Result<Option<Fill>, SvgSceneError> {
        let Some(paint) = self.paint(fill.paint())? else {
            return Ok(None);
        };
        let mut node = Fill::new(paint);
        node.opacity = fill.opacity().get();
        node.rule = match fill.rule() {
            usvg::FillRule::NonZero => FillRule::NonZero,
            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
        };
        Ok(Some(node))
    }

    fn stroke(&mut self, stroke: &usvg::Stroke) -> Result<Option<Stroke>, SvgSceneError> {
        let Some(paint) = self.paint(stroke.paint())? else {
            return Ok(None);
        };
        let mut node = Stroke::new(paint, stroke.width().get());
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
        Ok(Some(node))
    }
}

fn paint_order(order: usvg::PaintOrder) -> PaintOrder {
    match order {
        usvg::PaintOrder::FillAndStroke => PaintOrder::FillThenStroke,
        usvg::PaintOrder::StrokeAndFill => PaintOrder::StrokeThenFill,
    }
}

fn paint_color(color: usvg::Color, alpha: f32) -> PaintColor {
    PaintColor::new(
        f32::from(color.red) / 255.0,
        f32::from(color.green) / 255.0,
        f32::from(color.blue) / 255.0,
        alpha,
    )
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

#[cfg(test)]
mod tests;
