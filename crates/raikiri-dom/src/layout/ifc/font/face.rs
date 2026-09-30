//! Document layer and `@font-face` registration.
//!
//! Each document gets its own layer on top of the shared one. `@font-face`
//! rules are registered only there, so one document's faces are never visible
//! to another. Faces are registered under the authored family name, which
//! makes rewriting computed `font-family` lists unnecessary.

use crate::fonts::{FONT_SIZE_CAP, FontFaceApplyReport, FontFaceLoader, decode_web_font};
use raikiri_style::{
    FontFaceRegistry, FontFaceRule, FontFaceSource, FontFaceStyle, FontFaceWeight,
};
use shodo::font::{FontCollection, FontFaceDescriptor, FontSource};
use shodo::limits::Limits;
use shodo::style::FontStyle;

/// A fresh, empty document layer over `shared`.
pub(crate) fn document_layer(shared: &FontCollection, limits: &Limits) -> FontCollection {
    FontCollection::for_document(shared, limits)
}

fn descriptor(rule: &FontFaceRule) -> FontFaceDescriptor {
    let (low, high) = match rule.weight {
        FontFaceWeight::Bold => (700.0, 700.0),
        FontFaceWeight::Number(value) => (value, value),
        FontFaceWeight::Range(low, high) => (low, high),
        // `normal`, and any descriptor added later, take the initial weight.
        _ => (400.0, 400.0),
    };
    let style = match rule.style {
        FontFaceStyle::Italic => FontStyle::Italic,
        FontFaceStyle::Oblique => FontStyle::Oblique(14.0),
        _ => FontStyle::Normal,
    };
    FontFaceDescriptor {
        family: rule.family.to_string(),
        weight: (low, high),
        style,
        unicode_ranges: rule.unicode_range.clone(),
        ..FontFaceDescriptor::default()
    }
}

/// Register the faces of `faces` into `document_fonts`.
///
/// Rules are visited in family-name order and sources in author order; the
/// first source that registers wins and the rest are not fetched. A `url()`
/// source is fetched through `loader`, size-capped, and decoded from a
/// WOFF/WOFF2 container to sfnt first. A `local()` source resolves against the
/// shared layer by full name or PostScript name (CSS Fonts 4 §4.3), not by
/// family name. Unavailable, oversized, undecodable, or rejected sources only
/// leave the family in [`FontFaceApplyReport::skipped`].
///
/// `font-stretch` is not passed on yet; the descriptor keeps its initial
/// width.
pub(crate) fn register_font_faces(
    document_fonts: &FontCollection,
    faces: &FontFaceRegistry,
    loader: &dyn FontFaceLoader,
) -> FontFaceApplyReport {
    let mut report = FontFaceApplyReport::default();
    let mut ordered: Vec<_> = faces.iter().collect();
    ordered.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    for (family, rule) in ordered {
        let descriptor = descriptor(rule);
        for source in &rule.src {
            match source {
                FontFaceSource::Local(name) => {
                    let sources = vec![FontSource::Local(name.to_string())];
                    if document_fonts
                        .register_sources(descriptor.clone(), sources)
                        .is_ok()
                    {
                        report.aliased.push((family.to_string(), name.to_string()));
                        break;
                    }
                }
                FontFaceSource::Url { url, .. } => {
                    let Some(bytes) = loader.load(url.as_str()) else {
                        continue;
                    };
                    if bytes.len() as u64 > FONT_SIZE_CAP {
                        continue;
                    }
                    let Some(bytes) = decode_web_font(bytes) else {
                        continue;
                    };
                    if document_fonts
                        .register_face(bytes, 0, descriptor.clone())
                        .is_ok()
                    {
                        report.applied.push(family.to_string());
                        break;
                    }
                }
                // A source kind added later never resolves here.
                _ => continue,
            }
        }
    }
    report.applied.sort();
    report.aliased.sort();
    let mut skipped: Vec<String> = faces
        .iter()
        .map(|(family, _)| family.to_string())
        .filter(|family| {
            !report.applied.contains(family)
                && !report.aliased.iter().any(|(face, _)| face == family)
        })
        .collect();
    skipped.sort();
    report.skipped = skipped;
    report
}

#[cfg(test)]
mod tests;
