//! Text in a vector scene: glyphs that carry the font they come from.

use std::fmt;
use std::sync::Arc;

use super::{Fill, PaintOrder, Stroke};
use crate::paint::PaintTransform;

/// One face of a font file: shared bytes and the face's position in them.
///
/// The bytes are shared, never copied per glyph or per scene, so a consumer
/// that embeds fonts can tell that two glyphs use the same face with
/// [`FontFace::is_same_face`] and embed it once.
#[derive(Clone)]
pub struct FontFace {
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    index: u32,
}

impl FontFace {
    /// The face at `index` of the font file or collection in `data`.
    pub fn new(data: Arc<dyn AsRef<[u8]> + Send + Sync>, index: u32) -> Self {
        Self { data, index }
    }

    /// The bytes of the whole font file, which a collection shares between
    /// its faces.
    pub fn bytes(&self) -> &[u8] {
        AsRef::<[u8]>::as_ref(&*self.data)
    }

    /// The shared bytes, for a consumer that keeps them without copying.
    pub fn data(&self) -> &Arc<dyn AsRef<[u8]> + Send + Sync> {
        &self.data
    }

    /// Which face of the file this is; zero for a file of one face.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Whether `other` is this face, by the identity of its shared bytes. This
    /// is the cheap test a consumer keys its font cache on: it never compares
    /// the bytes themselves.
    pub fn is_same_face(&self, other: &Self) -> bool {
        self.index == other.index
            && std::ptr::addr_eq(Arc::as_ptr(&self.data), Arc::as_ptr(&other.data))
    }
}

impl PartialEq for FontFace {
    /// Faces are equal when they are the same face, or when they name the same
    /// face of two copies of the same bytes. The bytes are compared only in
    /// the second case.
    fn eq(&self, other: &Self) -> bool {
        self.is_same_face(other) || (self.index == other.index && self.bytes() == other.bytes())
    }
}

impl fmt::Debug for FontFace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontFace")
            .field("index", &self.index)
            .field("byte_len", &self.bytes().len())
            .finish()
    }
}

/// A variable-font axis value, as `font-variation-settings` sets it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct FontVariation {
    /// The four-byte axis tag, such as `b"wght"`.
    pub tag: [u8; 4],
    /// The value on that axis.
    pub value: f32,
}

impl FontVariation {
    /// A setting of `value` on the axis named `tag`.
    pub fn new(tag: [u8; 4], value: f32) -> Self {
        Self { tag, value }
    }
}

/// Glyphs drawn with one paint.
///
/// The run is painted as one shape: its fill covers all the glyphs, then its
/// stroke does, or the other way round for [`PaintOrder::StrokeThenFill`], so
/// the stroke of a glyph never lies over the fill of the next when they
/// overlap. A consumer that has a text API draws the glyphs as selectable
/// text in one pass for each of the fill and the stroke; one that has only
/// outlines takes them from each glyph's font.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TextRun {
    /// The size the glyphs were scaled to, in the space of the enclosing
    /// group. Each glyph's transform already includes it.
    pub font_size: f32,
    /// The variation settings applied to the glyphs' outlines.
    pub variations: Vec<FontVariation>,
    /// The glyphs, in drawing order.
    pub glyphs: Vec<Glyph>,
    /// How the inside of the glyphs is painted, if it is.
    pub fill: Option<Fill>,
    /// How the outline of the glyphs is painted, if it is.
    pub stroke: Option<Stroke>,
    /// Whether the fill or the stroke is drawn first.
    pub paint_order: PaintOrder,
    /// Whether the edges are smoothed; see [`Path::anti_alias`](super::Path::anti_alias).
    pub anti_alias: bool,
}

impl TextRun {
    /// An unpainted, smoothed run of `glyphs` at `font_size`.
    pub fn new(font_size: f32, glyphs: Vec<Glyph>) -> Self {
        Self {
            font_size,
            variations: Vec::new(),
            glyphs,
            fill: None,
            stroke: None,
            paint_order: PaintOrder::FillThenStroke,
            anti_alias: true,
        }
    }

    pub(super) fn heap_bytes(&self) -> usize {
        self.variations.capacity() * std::mem::size_of::<FontVariation>()
            + self.glyphs.capacity() * std::mem::size_of::<Glyph>()
            + self
                .glyphs
                .iter()
                .map(|glyph| glyph.text.capacity())
                .sum::<usize>()
            + self.fill.as_ref().map_or(0, |fill| fill.paint.heap_bytes())
            + self.stroke.as_ref().map_or(0, Stroke::heap_bytes)
    }
}

/// One glyph of a [`TextRun`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Glyph {
    /// The face the glyph is taken from. A run can mix faces where the text
    /// fell back to another font.
    pub font: FontFace,
    /// The glyph's index in `font`.
    pub id: u32,
    /// The characters the glyph stands for, for selectable and searchable
    /// text. Empty for a glyph that belongs to a cluster another glyph spells.
    pub text: String,
    /// Maps the glyph's outline, in the font's design units with `y` pointing
    /// down, to the space of the enclosing group. The font size is already
    /// applied: a design unit becomes `font_size` over the font's units per
    /// em. A text API that sizes the glyph itself takes this transform
    /// followed by a scale of units per em over `font_size`.
    pub transform: PaintTransform,
}

impl Glyph {
    /// Glyph `id` of `font`, standing for `text`, placed by `transform`.
    pub fn new(
        font: FontFace,
        id: u32,
        text: impl Into<String>,
        transform: PaintTransform,
    ) -> Self {
        Self {
            font,
            id,
            text: text.into(),
            transform,
        }
    }
}
