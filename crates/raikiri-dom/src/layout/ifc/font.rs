//! Shared font layer for the inline formatting context path.
//!
//! A shared [`FontCollection`] holds the installed or bundled faces. Every
//! generic family is remapped onto the registered bundle, in registration
//! order, so results do not depend on the fonts installed on the host.

use crate::fonts::{
    FONT_SIZE_CAP, FontError, FontReadReject, FontWarn, FontWarnObserver, PREFERRED_FIRST,
    emit_warn, read_bounded_font_file, read_reject_to_warn, walk_fonts,
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
pub struct BundledFace {
    /// CSS family name the face is registered under.
    pub family: String,
    /// Encoded sfnt bytes.
    pub bytes: Vec<u8>,
}

/// The primary family name in a font's name table, preferring the typographic
/// family. Only the first face of a collection is read.
pub(crate) fn family_name(bytes: &[u8]) -> Option<String> {
    let font = FontRef::from_index(bytes, 0).ok()?;
    [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
        .into_iter()
        .find_map(|id| {
            font.localized_strings(id)
                .english_or_first()
                .map(|name| name.to_string())
        })
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
/// Each face is registered with a CSS descriptor under the given family name.
/// `local()` only resolves installed faces (by full or PostScript name), so a
/// face registered here is not visible to a `local()` source. [`wpt_collection`]
/// registers installed faces and is visible to `local()`.
///
/// # Errors
/// An empty list, a face shodo rejects, or a resource limit.
pub(crate) fn bundled_collection(
    limits: &Limits,
    faces: Vec<BundledFace>,
    system_fonts: bool,
) -> Result<FontCollection, shodo::font::FontError> {
    if faces.is_empty() {
        return Err(shodo::font::FontError::Malformed(
            "no bundled fonts were supplied",
        ));
    }
    let collection = FontCollection::with_options(
        limits,
        FontOptions {
            system_fonts,
            ..FontOptions::default()
        },
    );
    let mut families: Vec<String> = Vec::new();
    for face in faces {
        let family = face.family.trim().to_owned();
        collection.register_face(
            face.bytes,
            0,
            shodo::font::FontFaceDescriptor {
                family: family.clone(),
                ..shodo::font::FontFaceDescriptor::default()
            },
        )?;
        if !families.contains(&family) {
            families.push(family);
        }
    }
    map_generics(&collection, &families);
    Ok(collection)
}

/// Build a shared layer from the WPT bundled fonts directory.
///
/// System fonts are disabled. Fonts are read with the same bounds and
/// warn-and-skip policy as the parley builder, registered as installed faces
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
            ..FontOptions::default()
        },
    );
    let mut families: Vec<String> = Vec::new();
    let mut registered_preferred: HashSet<String> = HashSet::new();
    for path in paths {
        let bytes = match read_bounded_font_file(&path, &canonical_root, FONT_SIZE_CAP) {
            Ok(bytes) => bytes,
            Err(FontReadReject::Io(source)) => {
                return Err(FontError::Io {
                    path: path.clone(),
                    source,
                });
            }
            Err(reason) => {
                emit_warn(&mut observer, read_reject_to_warn(&path, &reason));
                continue;
            }
        };
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
