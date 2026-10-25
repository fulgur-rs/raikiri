//! SVG text set in the faces of the font collection that HTML text uses.

use std::collections::HashMap;
use std::sync::Mutex;

use raikiri_svg::{SvgFontFamily, SvgFontRequest, SvgFontSource, SvgFontStyle};
use raikiri_traits::vector::FontFace;
use shodo::font::{FontCollection, FontId, FontQuery};
use shodo::style::{FontFamily, FontStyle, GenericFamily};

/// The angle of the slant that `font-style: oblique` takes without one.
const DEFAULT_OBLIQUE_DEGREES: f32 = 14.0;

/// The faces handed out so far.
///
/// A scene shares one allocation of font bytes between all its glyphs of a
/// face, and a consumer tells two glyphs of one face apart from those of
/// another by it, so a face is wrapped once and then reused.
#[derive(Debug, Default)]
pub(crate) struct FaceCache(Mutex<HashMap<FontId, FontFace>>);

impl FaceCache {
    /// The face of `id` in `collection`.
    fn face(&self, collection: &FontCollection, id: FontId) -> Option<FontFace> {
        // A panic while the lock is held cannot leave a half-written entry.
        let mut faces = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(face) = faces.get(&id) {
            return Some(face.clone());
        }
        let data = collection.font_data(id)?;
        let face = FontFace::new(std::sync::Arc::new(data.data), data.index);
        faces.insert(id, face.clone());
        Some(face)
    }
}

/// The face of `collection` that matches `request` for `cluster`.
pub(crate) fn matching_face(
    collection: &FontCollection,
    cache: &FaceCache,
    request: &SvgFontRequest,
    cluster: &str,
) -> Option<FontFace> {
    let query = FontQuery {
        families: request
            .families
            .iter()
            .map(|family| match family {
                SvgFontFamily::Serif => FontFamily::Generic(GenericFamily::Serif),
                SvgFontFamily::SansSerif => FontFamily::Generic(GenericFamily::SansSerif),
                SvgFontFamily::Cursive => FontFamily::Generic(GenericFamily::Cursive),
                SvgFontFamily::Fantasy => FontFamily::Generic(GenericFamily::Fantasy),
                SvgFontFamily::Monospace => FontFamily::Generic(GenericFamily::Monospace),
                SvgFontFamily::Named(name) => FontFamily::Named(name.clone()),
            })
            .collect(),
        weight: f32::from(request.weight),
        width: request.stretch,
        style: match request.style {
            SvgFontStyle::Normal => FontStyle::Normal,
            SvgFontStyle::Italic => FontStyle::Italic,
            SvgFontStyle::Oblique => FontStyle::Oblique(DEFAULT_OBLIQUE_DEGREES),
        },
        script: cluster.chars().next().map_or(*b"Latn", script),
        ..FontQuery::default()
    };
    let matched = collection.match_cluster(&query, cluster)?;
    cache.face(collection, matched.id)
}

/// The ISO 15924 tag of the script of `ch`; characters shared by scripts are
/// taken as Latin, the default of a query.
fn script(ch: char) -> [u8; 4] {
    use unicode_script::{Script, UnicodeScript};
    match ch.script() {
        Script::Common | Script::Inherited | Script::Unknown => *b"Latn",
        script => {
            let mut tag = [0; 4];
            tag.copy_from_slice(script.short_name().as_bytes());
            tag
        }
    }
}

impl SvgFontSource for crate::RenderFonts {
    /// Selects with a space as the probe: the text is not known yet, and a
    /// face without one is not a face for running text.
    fn select(&self, request: &SvgFontRequest) -> Option<FontFace> {
        matching_face(self.collection(), self.svg_faces(), request, " ")
    }

    fn fallback(&self, request: &SvgFontRequest, ch: char) -> Option<FontFace> {
        let mut buffer = [0; 4];
        matching_face(
            self.collection(),
            self.svg_faces(),
            request,
            ch.encode_utf8(&mut buffer),
        )
    }
}

/// The fonts SVG text of a page is set in: the font collection its document
/// was laid out with, so SVG text sees the same `@font-face` faces, bundled
/// faces and generic-family mapping as the HTML around it.
///
/// Get it from [`crate::Page::svg_fonts`] and pass it to
/// [`raikiri_svg::SvgResources::with_fonts`]. Every scene built for the pages
/// of one [`crate::DocumentLayout`] shares the faces it hands out, whether the
/// text is on a page, on a page laid out at another width, or in a running
/// element, so a consumer that embeds fonts embeds each once.
#[derive(Clone, Copy)]
pub struct PageSvgFonts<'a> {
    collection: &'a FontCollection,
    cache: &'a FaceCache,
}

impl<'a> PageSvgFonts<'a> {
    pub(crate) fn new(collection: &'a FontCollection, cache: &'a FaceCache) -> Self {
        Self { collection, cache }
    }
}

impl std::fmt::Debug for PageSvgFonts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageSvgFonts").finish_non_exhaustive()
    }
}

impl SvgFontSource for PageSvgFonts<'_> {
    /// Selects with a space as the probe, like [`RenderFonts`](crate::RenderFonts).
    fn select(&self, request: &SvgFontRequest) -> Option<FontFace> {
        matching_face(self.collection, self.cache, request, " ")
    }

    fn fallback(&self, request: &SvgFontRequest, ch: char) -> Option<FontFace> {
        let mut buffer = [0; 4];
        matching_face(
            self.collection,
            self.cache,
            request,
            ch.encode_utf8(&mut buffer),
        )
    }
}
