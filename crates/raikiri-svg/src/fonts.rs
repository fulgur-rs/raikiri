//! Fonts for SVG text.
//!
//! The parser lays text out, but this crate gives it no fonts of its own:
//! its font database stays empty. Whenever the text needs a face, the parser
//! asks a [`SvgFontSource`] for one, and the source is the same font
//! collection the HTML around the SVG uses. Shaping and positioning stay with
//! the parser, so a face is selected by the same rules as for HTML text and
//! never by a second font database.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

mod key;
use key::{ContextKey, FaceIdentity};

use raikiri_traits::vector::FontFace;
use skrifa::MetadataProvider;
use usvg::fontdb;

/// A family in a `font-family` list.
///
/// CSS fixes the generic families, so a source matches on this without a
/// catch-all arm. A generic family that CSS adds later becomes a new variant,
/// which is a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgFontFamily {
    /// The generic family `serif`.
    Serif,
    /// The generic family `sans-serif`.
    SansSerif,
    /// The generic family `cursive`.
    Cursive,
    /// The generic family `fantasy`.
    Fantasy,
    /// The generic family `monospace`.
    Monospace,
    /// A family by name.
    Named(String),
}

/// A `font-style` keyword. The set is closed, like [`SvgFontFamily`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgFontStyle {
    /// Upright.
    Normal,
    /// The italic face.
    Italic,
    /// A slanted face.
    Oblique,
}

/// What a run of text asks of a font.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SvgFontRequest {
    /// The families to try, in order. The generic `serif` family is appended
    /// last when the list does not name it, so a request always ends in a
    /// family a collection can answer.
    pub families: Vec<SvgFontFamily>,
    /// The CSS weight, `1..=1000`.
    pub weight: u16,
    /// The CSS width as a percentage, `100.0` for normal.
    pub stretch: f32,
    /// The slant.
    pub style: SvgFontStyle,
}

impl SvgFontRequest {
    /// A request for `families` in the regular face: weight 400, normal width
    /// and upright.
    pub fn new(families: Vec<SvgFontFamily>) -> Self {
        Self {
            families,
            weight: 400,
            stretch: 100.0,
            style: SvgFontStyle::Normal,
        }
    }
}

/// Where SVG text gets its faces.
///
/// An implementation answers from the collection that HTML text uses, so SVG
/// and HTML select a face by the same rules, with the same generic families
/// and the same system-font policy. It must hand out the same
/// [`FontFace`] (see [`FontFace::is_same_face`]) every time it answers with
/// the same face, so the face is loaded once and a consumer embeds it once.
pub trait SvgFontSource: Send + Sync {
    /// The face for text with `request`, whose characters are not known yet.
    /// A source that matches by coverage may probe with a space.
    fn select(&self, request: &SvgFontRequest) -> Option<FontFace>;

    /// A face that has `ch`, as close to `request` as possible, for a
    /// character the selected face lacks.
    fn fallback(&self, request: &SvgFontRequest, ch: char) -> Option<FontFace>;
}

/// The faces a parse has asked for, with the requests that selected them.
///
/// The parser knows a face by its id in a database, and a fallback asks only
/// for a character and the ids already used. So an id stands for a face in the
/// context of one request: the same face selected by two requests with
/// different fallback families, weights or styles has two ids, and the
/// fallback of each finds its own request. The font bytes are loaded into the
/// database once and shared by all the ids of a face.
pub(crate) struct Faces<'a> {
    source: &'a dyn SvgFontSource,
    entries: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    list: Vec<Entry>,
    contexts: HashMap<ContextKey<'static>, fontdb::ID>,
    faces: HashMap<FaceIdentity, fontdb::ID>,
    ids: HashMap<fontdb::ID, usize>,
}

struct Entry {
    face: FontFace,
    id: fontdb::ID,
    request: Arc<SvgFontRequest>,
}

/// The face behind each database id of a parsed tree.
#[derive(Default)]
pub(crate) struct FaceMap(HashMap<fontdb::ID, MappedFace>);

struct MappedFace {
    face: FontFace,
    /// Whether the face has an optical size axis.
    has_opsz: bool,
}

impl FaceMap {
    pub(crate) fn get(&self, id: fontdb::ID) -> Option<&FontFace> {
        self.entry(id).map(|entry| &entry.face)
    }

    /// Whether the face behind `id` has an `opsz` axis, which the parser sets
    /// to the font size unless the text turns optical sizing off.
    pub(crate) fn has_opsz(&self, id: fontdb::ID) -> bool {
        self.entry(id).is_some_and(|entry| entry.has_opsz)
    }

    fn entry(&self, id: fontdb::ID) -> Option<&MappedFace> {
        #[cfg(test)]
        tests::record_entry_visit();
        self.0.get(&id)
    }
}

impl<'a> Faces<'a> {
    pub(crate) fn new(source: &'a dyn SvgFontSource) -> Self {
        Self {
            source,
            entries: Mutex::default(),
        }
    }

    fn entries(&self) -> MutexGuard<'_, Entries> {
        // User font-data reads finish before publishing entries and indexes.
        // Publication uses owned standard keys and makes no user callbacks.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The parser's font resolver: it selects and falls back through the
    /// source.
    pub(crate) fn resolver(&self) -> usvg::FontResolver<'_> {
        usvg::FontResolver {
            select_font: Box::new(move |font, database| {
                let request = request(font);
                let face = self.source.select(&request)?;
                self.load(face, Cow::Owned(request), database)
            }),
            select_fallback: Box::new(move |ch, used, database| {
                // The first face used is the one the text was selected with.
                let base = *used.first()?;
                let request = {
                    let entries = self.entries();
                    #[cfg(test)]
                    tests::record_entry_visit();
                    let index = *entries.ids.get(&base)?;
                    Arc::clone(&entries.list[index].request)
                };
                let face = self.source.fallback(&request, ch)?;
                let id = self.load(face, Cow::Borrowed(&request), database)?;
                // The parser asks for a face it has not used yet.
                (!used.contains(&id)).then_some(id)
            }),
        }
    }

    /// The database id of `face` selected by `request`, loading the face on
    /// its first use.
    fn load(
        &self,
        face: FontFace,
        request: Cow<'_, SvgFontRequest>,
        database: &mut Arc<fontdb::Database>,
    ) -> Option<fontdb::ID> {
        let mut entries = self.entries();
        let identity = FaceIdentity::of(&face);
        #[cfg(test)]
        tests::record_entry_visit();
        if let Some(&id) = entries
            .contexts
            .get(&ContextKey::borrowed(identity, &request))
        {
            return Some(id);
        }
        #[cfg(test)]
        tests::record_entry_visit();
        let id = match entries.faces.get(&identity).copied() {
            Some(loaded) => {
                let mut info = database.face(loaded)?.clone();
                info.id = fontdb::ID::dummy();
                Arc::make_mut(database).push_face_info(info)
            }
            None => {
                let ids = Arc::make_mut(database)
                    .load_font_source(fontdb::Source::Binary(Arc::clone(face.data())));
                ids.into_iter().find(|&id| {
                    database
                        .face(id)
                        .is_some_and(|info| info.index == face.index())
                })?
            }
        };
        // A context owns its request only after the database load succeeds.
        // Selection misses move fresh requests; fallback misses clone as before.
        let request = Arc::new(request.into_owned());
        let key = ContextKey::shared(identity, Arc::clone(&request));
        let index = entries.list.len();
        entries.list.push(Entry { face, id, request });
        entries.contexts.insert(key, id);
        entries.faces.entry(identity).or_insert(id);
        entries.ids.insert(id, index);
        Some(id)
    }

    /// The faces that were loaded, by database id.
    pub(crate) fn into_map(self) -> FaceMap {
        let entries = self
            .entries
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        FaceMap(
            entries
                .list
                .into_iter()
                .map(|entry| {
                    (
                        entry.id,
                        MappedFace {
                            has_opsz: has_opsz_axis(&entry.face),
                            face: entry.face,
                        },
                    )
                })
                .collect(),
        )
    }
}

fn request(font: &usvg::Font) -> SvgFontRequest {
    let mut families: Vec<SvgFontFamily> = font
        .families()
        .iter()
        .map(|family| match family {
            usvg::FontFamily::Serif => SvgFontFamily::Serif,
            usvg::FontFamily::SansSerif => SvgFontFamily::SansSerif,
            usvg::FontFamily::Cursive => SvgFontFamily::Cursive,
            usvg::FontFamily::Fantasy => SvgFontFamily::Fantasy,
            usvg::FontFamily::Monospace => SvgFontFamily::Monospace,
            usvg::FontFamily::Named(name) => SvgFontFamily::Named(name.clone()),
        })
        .collect();
    if !families.contains(&SvgFontFamily::Serif) {
        families.push(SvgFontFamily::Serif);
    }
    SvgFontRequest {
        families,
        weight: font.weight().clamp(1, 1000),
        stretch: match font.stretch() {
            usvg::FontStretch::UltraCondensed => 50.0,
            usvg::FontStretch::ExtraCondensed => 62.5,
            usvg::FontStretch::Condensed => 75.0,
            usvg::FontStretch::SemiCondensed => 87.5,
            usvg::FontStretch::Normal => 100.0,
            usvg::FontStretch::SemiExpanded => 112.5,
            usvg::FontStretch::Expanded => 125.0,
            usvg::FontStretch::ExtraExpanded => 150.0,
            usvg::FontStretch::UltraExpanded => 200.0,
        },
        style: match font.style() {
            usvg::FontStyle::Normal => SvgFontStyle::Normal,
            usvg::FontStyle::Italic => SvgFontStyle::Italic,
            usvg::FontStyle::Oblique => SvgFontStyle::Oblique,
        },
    }
}

/// Whether `face` varies with the optical size.
fn has_opsz_axis(face: &FontFace) -> bool {
    skrifa::FontRef::from_index(face.bytes(), face.index())
        .is_ok_and(|font| font.axes().get_by_tag(skrifa::Tag::new(b"opsz")).is_some())
}

#[cfg(test)]
mod tests;
