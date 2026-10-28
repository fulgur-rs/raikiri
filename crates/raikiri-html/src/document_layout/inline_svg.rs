//! What an inline `<svg>` root is drawn with: a vector scene, or pixels where
//! the scene cannot stand for the picture.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use raikiri_svg::{SvgDocument, SvgFontSource, SvgResources, SvgRootStyle, SvgSceneError};
use raikiri_svg::{SvgError, SvgViewport};
use raikiri_traits::vector::{Group, Node};
use raikiri_traits::{PaintRect, PaintTransform};
use rustc_hash::FxHasher;

mod storage;

use super::ImageContent;
use crate::SvgRendering;

/// Upper bound on the heap bytes of the scenes that stay cached.
const MAX_KEPT_SCENE_BYTES: usize = 32 * 1024 * 1024;

/// Upper bound on the pixel bytes of the rasters that stay cached.
const MAX_KEPT_RASTER_BYTES: usize = 64 * 1024 * 1024;

/// Accounted XML, Arc overhead allowance and fixed-capacity metadata storage.
const MAX_KEPT_STORAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PREPARED_ROOTS: usize = 256;
const MAX_PICTURES: usize = 256;

fn fingerprint<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

/// The tolerance within which two linear transforms count as the same one.
const LINEAR_EPSILON: f32 = 1e-4;

/// An SVG root with its prepared source.
#[derive(Debug, Clone)]
pub(super) struct PreparedSvg {
    /// Standalone XML with the viewport, root color, and root font resolved.
    /// Shared, so a cache hit does not copy it.
    pub(super) source: Arc<str>,
    source_hash: u64,
    pub(super) host_opacity: Option<f32>,
    /// What the root asks to be drawn as, which its ancestors decide.
    pub(super) rendering: SvgRendering,
}

impl PreparedSvg {
    pub(super) fn new(source: String, host_opacity: Option<f32>, rendering: SvgRendering) -> Self {
        let source_hash = fingerprint(source.as_str());
        Self {
            source: source.into(),
            source_hash,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PictureKey {
    source: Arc<str>,
    source_hash: u64,
    fingerprint: u64,
    host_opacity: Option<u32>,
    size: (u32, u32),
    rendering: SvgRendering,
}

impl PictureKey {
    pub(super) fn new(prepared: &PreparedSvg, viewport: PaintRect) -> Self {
        let host_opacity = prepared.host_opacity.map(f32::to_bits);
        let size = (viewport.width.to_bits(), viewport.height.to_bits());
        Self {
            source: Arc::clone(&prepared.source),
            source_hash: prepared.source_hash,
            fingerprint: fingerprint(&(
                prepared.source_hash,
                host_opacity,
                size,
                prepared.rendering,
            )),
            host_opacity,
            size,
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
/// once while retained, and roots with the same markup share XML and pictures.
/// Count/storage pressure can evict entries, which are rebuilt on demand.
/// Placement is taken from each fragment.
#[derive(Debug)]
pub(crate) struct InlineSvgCache {
    storage: RefCell<Option<storage::Storage>>,
    limits: storage::Limits,
    /// The faces handed out to the SVG text of the document. The prepared
    /// sources belong to one laid-out document, but the faces belong to the
    /// whole [`super::DocumentLayout`]: its pages laid out at another width
    /// and its running elements share them, so a face is one allocation
    /// wherever its text is.
    faces: Arc<crate::svg_fonts::FaceCache>,
}

impl InlineSvgCache {
    /// An empty cache that hands out its SVG text faces from `faces`.
    pub(super) fn new(faces: Arc<crate::svg_fonts::FaceCache>) -> Self {
        Self::with_limits(faces, storage::Limits::default())
    }

    fn with_limits(faces: Arc<crate::svg_fonts::FaceCache>, limits: storage::Limits) -> Self {
        Self {
            storage: RefCell::new(None),
            limits,
            faces,
        }
    }

    fn with_storage<R>(&self, operation: impl FnOnce(&mut storage::Storage) -> R) -> R {
        // Documents and running elements without SVG allocate no slot arrays.
        let mut storage = self.storage.borrow_mut();
        operation(storage.get_or_insert_with(|| storage::Storage::new(self.limits)))
    }

    pub(super) fn faces(&self) -> &Arc<crate::svg_fonts::FaceCache> {
        &self.faces
    }

    pub(super) fn get(&self, key: &SvgKey) -> Option<PreparedSvg> {
        self.storage
            .borrow()
            .as_ref()
            .and_then(|storage| storage.get(key))
    }

    pub(super) fn insert(&self, key: SvgKey, prepared: PreparedSvg) -> PreparedSvg {
        self.with_storage(|storage| storage.insert(key, prepared))
    }

    /// The picture kept for `key`.
    pub(super) fn picture(&self, key: &PictureKey) -> Option<ImageContent> {
        self.storage
            .borrow()
            .as_ref()
            .and_then(|storage| storage.picture(key))
    }

    /// Keeps `content` as the picture for `key`, when it fits the budget of
    /// its kind. A picture that does not is built again each time it is asked
    /// for.
    pub(super) fn keep(&self, key: PictureKey, content: &ImageContent) {
        self.with_storage(|storage| storage.keep(key, content));
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.storage
            .borrow()
            .as_ref()
            .map_or(0, |storage| storage.roots_len())
    }

    #[cfg(test)]
    pub(super) fn kept_pictures(&self) -> usize {
        self.storage
            .borrow()
            .as_ref()
            .map_or(0, |storage| storage.pictures_len())
    }

    #[cfg(test)]
    pub(super) fn charges(&self) -> (usize, usize, usize, usize) {
        self.storage
            .borrow()
            .as_ref()
            .map_or((std::mem::size_of::<Self>(), 0, 0, 0), |storage| {
                storage.charges()
            })
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
/// document outside the vector subset (a `filter`, a `mask` or a `pattern`),
/// one over a budget of the scene (the raster path has limits of its own, not
/// these), and one with stroked glyphs placed by a transform that is not a
/// rotation, a reflection and a uniform scale (`textLength` stretching them),
/// which a backend strokes in glyph space and so distorts. When pixels cannot
/// be had either, the scene is returned if there is one (a stroked glyph then
/// comes out distorted). Without one, the error is the raster's for a document
/// outside the subset, whose only picture it is, and the scene's own for a
/// scene over a budget. An invalid document has no picture.
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
        // The raster is the only picture of this document, so what stops it is
        // the reason there is none.
        Err(SvgSceneError::Unsupported(_)) => pixels().map_err(SvgSceneError::Svg),
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
