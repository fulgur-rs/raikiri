//! What an inline `<svg>` root is drawn with: a vector scene, or pixels where
//! the scene cannot stand for the picture.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use raikiri_svg::{SvgDocument, SvgFontSource, SvgResources, SvgRootStyle, SvgSceneError};
use raikiri_svg::{SvgError, SvgViewport};
use raikiri_traits::vector::{Group, Node};
use raikiri_traits::{PaintRect, PaintTransform};
use rustc_hash::FxHashMap;

use super::ImageContent;
use crate::SvgRendering;

/// Upper bound on the heap bytes of the scenes that stay cached.
const MAX_KEPT_SCENE_BYTES: usize = 32 * 1024 * 1024;

/// Upper bound on the pixel bytes of the rasters that stay cached.
const MAX_KEPT_RASTER_BYTES: usize = 64 * 1024 * 1024;

/// The tolerance within which two linear transforms count as the same one.
const LINEAR_EPSILON: f32 = 1e-4;

/// An SVG root with its prepared source.
#[derive(Debug, Clone)]
pub(super) struct PreparedSvg {
    /// Standalone XML with the viewport, root color, and root font resolved.
    /// Shared, so a cache hit does not copy it.
    pub(super) source: Arc<str>,
    pub(super) host_opacity: Option<f32>,
    /// What the root asks to be drawn as, which its ancestors decide.
    pub(super) rendering: SvgRendering,
}

impl PreparedSvg {
    pub(super) fn new(source: String, host_opacity: Option<f32>, rendering: SvgRendering) -> Self {
        Self {
            source: source.into(),
            host_opacity,
            rendering,
        }
    }
}

/// The cache key of an SVG root: its node and the bits of its viewport size.
pub(super) type SvgKey = (usize, u32, u32);

/// What a picture is made of: the prepared source and what else decides how
/// it is drawn. A layout fixes the fonts and the raster scale.
///
/// Roots with the same markup, style and size share a picture, so an icon
/// repeated through a document is drawn once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct PictureKey {
    source: Arc<str>,
    host_opacity: Option<u32>,
    size: (u32, u32),
    rendering: SvgRendering,
}

impl PictureKey {
    pub(super) fn new(prepared: &PreparedSvg, viewport: PaintRect) -> Self {
        Self {
            source: Arc::clone(&prepared.source),
            host_opacity: prepared.host_opacity.map(f32::to_bits),
            size: (viewport.width.to_bits(), viewport.height.to_bits()),
            rendering: prepared.rendering,
        }
    }
}

/// Prepared inline SVG sources and pictures of one laid-out document.
///
/// The prepared source depends only on the root's subtree, its computed
/// styles and its viewport size, so it is kept for each SVG root and viewport
/// size. The picture adds the root's rendering to that, and is kept for each
/// source. So a root painted on several pages (a running header, or an SVG
/// split across a page break) is serialized, parsed, restyled and drawn
/// once, and roots with the same markup draw one picture. Placement is taken
/// from each fragment.
#[derive(Debug)]
pub(crate) struct InlineSvgCache {
    entries: RefCell<FxHashMap<SvgKey, PreparedSvg>>,
    pictures: RefCell<FxHashMap<PictureKey, ImageContent>>,
    /// The faces handed out to the SVG text of the document. The prepared
    /// sources belong to one laid-out document, but the faces belong to the
    /// whole [`super::DocumentLayout`]: its pages laid out at another width
    /// and its running elements share them, so a face is one allocation
    /// wherever its text is.
    faces: Arc<crate::svg_fonts::FaceCache>,
    kept_scene_bytes: Cell<usize>,
    kept_raster_bytes: Cell<usize>,
}

impl InlineSvgCache {
    /// An empty cache that hands out its SVG text faces from `faces`.
    pub(super) fn new(faces: Arc<crate::svg_fonts::FaceCache>) -> Self {
        Self {
            entries: RefCell::default(),
            pictures: RefCell::default(),
            faces,
            kept_scene_bytes: Cell::new(0),
            kept_raster_bytes: Cell::new(0),
        }
    }

    pub(super) fn faces(&self) -> &Arc<crate::svg_fonts::FaceCache> {
        &self.faces
    }

    pub(super) fn get(&self, key: &SvgKey) -> Option<PreparedSvg> {
        self.entries.borrow().get(key).cloned()
    }

    pub(super) fn insert(&self, key: SvgKey, prepared: PreparedSvg) {
        self.entries.borrow_mut().insert(key, prepared);
    }

    /// The picture kept for `key`.
    pub(super) fn picture(&self, key: &PictureKey) -> Option<ImageContent> {
        self.pictures.borrow().get(key).cloned()
    }

    /// Keeps `content` as the picture for `key`, when it fits the budget of
    /// its kind. A picture that does not is built again each time it is asked
    /// for.
    pub(super) fn keep(&self, key: PictureKey, content: &ImageContent) {
        let (kept, bytes, limit) = match content {
            ImageContent::Vector(scene) => (
                &self.kept_scene_bytes,
                scene.estimated_heap_bytes(),
                MAX_KEPT_SCENE_BYTES,
            ),
            ImageContent::Raster(pixels) => (
                &self.kept_raster_bytes,
                pixels.rgba.len(),
                MAX_KEPT_RASTER_BYTES,
            ),
        };
        let mut pictures = self.pictures.borrow_mut();
        if !pictures.contains_key(&key) && kept.get().saturating_add(bytes) <= limit {
            kept.set(kept.get() + bytes);
            pictures.insert(key, content.clone());
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn kept_pictures(&self) -> usize {
        self.pictures.borrow().len()
    }
}

/// What the picture of an SVG root depends on besides its source.
pub(super) struct PictureRequest<'a> {
    pub(super) source: &'a str,
    pub(super) viewport: PaintRect,
    pub(super) host_opacity: Option<f32>,
    pub(super) rendering: SvgRendering,
    /// Device pixels per CSS pixel of a picture drawn as pixels.
    pub(super) raster_scale: f32,
    pub(super) fonts: &'a dyn SvgFontSource,
}

/// The picture of `request`: a scene when the document is inside the vector
/// subset and the root does not ask for pixels, pixels otherwise.
///
/// Pixels also stand in for a scene that cannot be had or drawn faithfully: a
/// document outside the vector subset (a `mask` or a `pattern`), one over a
/// budget of the scene (the raster path has limits of its own, not these), and
/// one with stroked glyphs placed by a transform that is not a rotation, a
/// reflection and a uniform scale (`textLength` stretching them), which a
/// backend strokes in glyph space and so distorts. When pixels cannot be had
/// either, the scene is returned if there is one (a stroked glyph then comes
/// out distorted) and the scene's error is otherwise. An invalid document has
/// no picture.
///
/// The host opacity is in neither: the caller composites it around the
/// picture.
pub(super) fn build_picture(request: &PictureRequest<'_>) -> Result<ImageContent, SvgSceneError> {
    let document = SvgDocument::parse(request.source.as_bytes())?;
    let viewport = SvgViewport {
        width: request.viewport.width,
        height: request.viewport.height,
    };
    let pixels = || -> Result<ImageContent, SvgError> {
        let root_style = SvgRootStyle {
            opacity: request.host_opacity.unwrap_or(1.0),
            // The host opacity is applied around the image, which holds none.
            neutralize_root_opacity: request.host_opacity.is_some(),
            ..SvgRootStyle::default()
        };
        document
            .rasterize_with_fonts(
                viewport,
                root_style,
                Some(request.fonts),
                request.raster_scale,
                None,
            )
            .map(|pixels| ImageContent::Raster(Arc::new(pixels)))
    };
    if request.rendering == SvgRendering::Raster
        && let Ok(content) = pixels()
    {
        return Ok(content);
    }
    let scene = document.scene_with_resources(
        viewport,
        SvgRootStyle {
            opacity: request.host_opacity.unwrap_or(1.0),
            // The host opacity is applied around the scene, which holds none.
            neutralize_root_opacity: request.host_opacity.is_some(),
            ..SvgRootStyle::default()
        },
        None,
        SvgResources::new().with_fonts(request.fonts),
    );
    match scene {
        Ok(scene) if strokes_text_off_similarity(&scene.root) => {
            Ok(pixels().unwrap_or_else(|_| ImageContent::Vector(Arc::new(scene))))
        }
        Ok(scene) => Ok(ImageContent::Vector(Arc::new(scene))),
        // The raster path fails on this document the same way.
        Err(error @ SvgSceneError::Svg(_)) => Err(error),
        Err(error) => pixels().map_err(|_| error),
    }
}

/// Whether some text of `group` is stroked with glyphs whose transform is not
/// a rotation, a reflection and a uniform scale.
fn strokes_text_off_similarity(group: &Group) -> bool {
    group.children.iter().any(|node| match node {
        Node::Group(group) => strokes_text_off_similarity(group),
        Node::Text(run) => {
            run.stroke.is_some()
                && run
                    .glyphs
                    .iter()
                    .any(|glyph| !is_similarity(glyph.transform))
        }
        _ => false,
    })
}

/// Whether `transform` keeps shapes similar: angles stay, and so does the
/// ratio between lengths in every direction.
fn is_similarity(transform: PaintTransform) -> bool {
    let PaintTransform { a, b, c, d, .. } = transform;
    let (x, y) = (a * a + b * b, c * c + d * d);
    (x - y).abs() <= LINEAR_EPSILON * x.max(y) && (a * c + b * d).abs() <= LINEAR_EPSILON * x
}

#[cfg(test)]
mod tests;
