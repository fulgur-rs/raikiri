//! Shared font layer for the inline formatting context path.
//!
//! A shared [`FontCollection`] holds the installed or bundled faces. Every
//! generic family is remapped onto the registered bundle, in registration
//! order, so results do not depend on the fonts installed on the host.

use crate::fonts::{
    FONT_SIZE_CAP, FontError, FontWarn, FontWarnObserver, PREFERRED_FIRST, emit_warn,
    read_font_file, walk_fonts,
};
use shodo::font::{FontCollection, FontOptions};
use shodo::limits::Limits;
use shodo::style::GenericFamily;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};
use std::collections::HashSet;
use std::path::Path;

const GENERICS: [GenericFamily; 6] = [
    GenericFamily::Serif,
    GenericFamily::SansSerif,
    GenericFamily::Monospace,
    GenericFamily::SystemUi,
    GenericFamily::Cursive,
    GenericFamily::Fantasy,
];

pub(crate) mod face;

/// One font file registered under an authored family name.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct BundledFace {
    /// CSS family name the face is registered under. Empty (or blank) for a
    /// font whose faces are registered under the family names they record,
    /// which is how a TrueType/OpenType collection is bundled; see
    /// `build_bundled_font_collection`.
    pub family: String,
    /// Encoded sfnt bytes: a font file, or a collection when `family` is
    /// empty.
    pub bytes: Vec<u8>,
}

/// One font source of a bundle.
///
/// A face is registered under the family name its supplier chose. A
/// collection is registered under the family names its faces record, which is
/// how a TrueType/OpenType collection (`.ttc` / `.otc`) is bundled: its faces
/// belong to different families and only the font knows them.
enum BundledSource {
    /// One font file registered under an authored family name.
    Face(BundledFace),
    /// A font file or collection whose every face is registered under the
    /// primary family name its name table records. A face without a name is
    /// kept as a last resort only, and takes no part in the generic families.
    Collection(Vec<u8>),
}

/// The CSS descriptor of the first face of `bytes`, registered under
/// `family`: the face's own weight, width and style, only the family name
/// being overridden. Bytes that do not parse
/// keep the defaults; registering them fails later with shodo's own error.
fn bundled_descriptor(family: String, bytes: &[u8]) -> shodo::font::FontFaceDescriptor {
    let defaults = shodo::font::FontFaceDescriptor {
        family,
        ..shodo::font::FontFaceDescriptor::default()
    };
    let Ok(font) = FontRef::from_index(bytes, 0) else {
        return defaults;
    };
    let attributes = font.attributes();
    // A face may state values outside the ranges a CSS descriptor accepts
    // (a weight class above 1000, a zero width class); the values are
    // brought into range instead of refusing the face.
    let weight = finite_or(attributes.weight.value(), 400.0).clamp(1.0, 1000.0);
    let width = finite_or(attributes.stretch.ratio() * 100.0, 100.0);
    let width = if width > 0.0 { width } else { 100.0 };
    let style = match attributes.style {
        skrifa::attribute::Style::Normal => shodo::style::FontStyle::Normal,
        skrifa::attribute::Style::Italic => shodo::style::FontStyle::Italic,
        skrifa::attribute::Style::Oblique(angle) => shodo::style::FontStyle::Oblique(
            finite_or(
                angle.unwrap_or(OBLIQUE_DEFAULT_ANGLE),
                OBLIQUE_DEFAULT_ANGLE,
            )
            .clamp(-90.0, 90.0),
        ),
    };
    shodo::font::FontFaceDescriptor {
        weight: (weight, weight),
        width: (width, width),
        style,
        ..defaults
    }
}

/// The angle of an oblique face that does not state one (CSS Fonts 4
/// `font-style: oblique`).
const OBLIQUE_DEFAULT_ANGLE: f32 = 14.0;

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// The primary family name in a font's name table, preferring the typographic
/// family. Only the first face of a collection is read.
pub(crate) fn family_name(bytes: &[u8]) -> Option<String> {
    face_family_name(bytes, 0)
}

/// [`family_name`] of the face at `index` of a font file or collection.
fn face_family_name(bytes: &[u8], index: u32) -> Option<String> {
    let font = FontRef::from_index(bytes, index).ok()?;
    [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
        .into_iter()
        .find_map(|id| {
            font.localized_strings(id)
                .english_or_first()
                .map(|name| name.to_string())
        })
}

/// The number of faces `bytes` declares: the header count of a collection,
/// one for any other font.
fn declared_face_count(bytes: &[u8]) -> u32 {
    if bytes.get(..4) != Some(b"ttcf".as_slice()) {
        return 1;
    }
    bytes
        .get(8..12)
        .and_then(|count| count.try_into().ok())
        .map_or(0, u32::from_be_bytes)
}

/// The primary family name of each face of a font file or collection, in face
/// order, without the faces that have none.
///
/// A face whose preferred name is present but empty has none, even when the
/// legacy family name is set: the font library does not register it under
/// either name.
///
/// A header can declare more faces than the file can hold, so the walk is
/// bounded by the collection face limit and by the offsets that fit in the
/// file.
fn recorded_families(bytes: &[u8], limits: &Limits) -> Vec<String> {
    let fitting = u64::try_from(bytes.len() / 4).unwrap_or(u64::MAX);
    let count = u64::from(declared_face_count(bytes))
        .min(limits.max_ttc_faces.unwrap_or(u64::MAX))
        .min(fitting);
    (0..count)
        .filter_map(|index| face_family_name(bytes, u32::try_from(index).ok()?))
        .filter(|name| !name.is_empty())
        .collect()
}

fn map_generics(collection: &FontCollection, families: &[String]) {
    for generic in GENERICS {
        collection.set_generic_families(generic, families.to_vec());
    }
}

/// Build a shared layer from consumer-supplied faces.
///
/// Registration order is the fallback order for every generic family. System
/// fonts are only consulted when `system_fonts` is set.
///
/// Each face is registered with a CSS descriptor under the given family name,
/// carrying the face's own weight, width and style, so the faces of one
/// family are told apart by weight and style. A face with an empty family is a
/// font file or collection registered under the names it records (see
/// `bundled_sources`).
/// `local()` only resolves installed faces (by full or PostScript name), so a
/// face registered here under a chosen name is not visible to a `local()`
/// source. The faces of a font registered under its recorded names are
/// registered as installed faces, and are visible to it, as are those of
/// [`wpt_collection`].
///
/// # Errors
/// An empty list, a face shodo rejects, or a resource limit.
pub(crate) fn bundled_collection(
    limits: &Limits,
    faces: Vec<BundledFace>,
    system_fonts: bool,
) -> Result<FontCollection, shodo::font::FontError> {
    let sources = faces
        .into_iter()
        .map(|face| {
            if face.family.trim().is_empty() {
                BundledSource::Collection(face.bytes)
            } else {
                BundledSource::Face(face)
            }
        })
        .collect();
    bundled_sources(limits, sources, system_fonts)
}

/// [`bundled_collection`] for sources that are not all one face under a chosen
/// name.
///
/// A [`BundledSource::Collection`] is registered once, so shodo checks and
/// budgets its bytes once however many faces it holds, and each face is
/// registered under the family name its name table records, with the weight,
/// width and style the font states. Its families join the generic families in
/// face order, after the families registered before it.
///
/// # Errors
/// An empty list, a source shodo rejects, or a resource limit.
fn bundled_sources(
    limits: &Limits,
    sources: Vec<BundledSource>,
    system_fonts: bool,
) -> Result<FontCollection, shodo::font::FontError> {
    if sources.is_empty() {
        return Err(shodo::font::FontError::Malformed(
            "no bundled fonts were supplied",
        ));
    }
    let collection = FontCollection::with_options(
        limits,
        FontOptions {
            system_fonts,
            whole_pixel_metrics: true,
            ..FontOptions::default()
        },
    );
    let mut families: Vec<String> = Vec::new();
    for source in sources {
        let registered = match source {
            BundledSource::Face(face) => {
                let family = face.family.trim().to_owned();
                let descriptor = bundled_descriptor(family.clone(), &face.bytes);
                collection.register_face(face.bytes, 0, descriptor)?;
                vec![family]
            }
            BundledSource::Collection(bytes) => {
                let first = collection.register(bytes)?;
                // Read the names from the registered blob, so that bytes the
                // font library refuses are never walked.
                collection.font_data(first).map_or_else(Vec::new, |data| {
                    recorded_families(data.data.as_ref(), limits)
                })
            }
        };
        for family in registered {
            if !families.contains(&family) {
                families.push(family);
            }
        }
    }
    map_generics(&collection, &families);
    Ok(collection)
}

/// Build a shared layer from the WPT bundled fonts directory.
///
/// System fonts are disabled. Fonts are read with the same bounds and
/// warn-and-skip policy as the directory walk ([`crate::fonts`]), registered as installed faces
/// (so `local()` can find them), and `Ahem.ttf` must be among them.
///
/// # Errors
/// See [`FontError`]: a missing or empty directory, an I/O error, no
/// registrable font, or a missing preferred font.
pub(crate) fn wpt_collection(
    fonts_dir: &Path,
    limits: &Limits,
    mut observer: FontWarnObserver<'_>,
) -> Result<FontCollection, FontError> {
    if !fonts_dir.exists() {
        return Err(FontError::DirNotFound(fonts_dir.to_path_buf()));
    }
    let canonical_root = std::fs::canonicalize(fonts_dir).map_err(|source| FontError::Io {
        path: fonts_dir.to_path_buf(),
        source,
    })?;
    let paths = walk_fonts(fonts_dir, &mut observer)?;
    if paths.is_empty() {
        return Err(FontError::EmptyDir(fonts_dir.to_path_buf()));
    }
    let collection = FontCollection::with_options(
        limits,
        FontOptions {
            system_fonts: false,
            whole_pixel_metrics: true,
            ..FontOptions::default()
        },
    );
    let mut families: Vec<String> = Vec::new();
    let mut registered_preferred: HashSet<String> = HashSet::new();
    for path in paths {
        let read = read_font_file(&path, &canonical_root, FONT_SIZE_CAP, &mut observer)?;
        let Some(bytes) = read else { continue };
        let family = family_name(&bytes);
        if collection.register(bytes).is_err() {
            emit_warn(&mut observer, FontWarn::RegisterEmpty { path: &path });
            continue;
        }
        if let Some(basename) = path.file_name().and_then(|name| name.to_str())
            && PREFERRED_FIRST.contains(&basename)
        {
            registered_preferred.insert(basename.to_owned());
        }
        if let Some(family) = family
            && !families.contains(&family)
        {
            families.push(family);
        }
    }
    for expected in PREFERRED_FIRST {
        if !registered_preferred.contains(*expected) {
            return Err(FontError::PreferredFontUnavailable {
                name: (*expected).to_owned(),
                dir: fonts_dir.to_path_buf(),
            });
        }
    }
    if families.is_empty() {
        return Err(FontError::NoFontsRegistered(fonts_dir.to_path_buf()));
    }
    map_generics(&collection, &families);
    Ok(collection)
}

#[cfg(test)]
mod tests;
