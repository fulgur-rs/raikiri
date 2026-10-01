//! Measure the CSS `ch` unit with the inline engine's own fonts.

use super::style::map_font_families;
use raikiri_style::ChFontKey;
use raikiri_style::property::FontStyle as CssFontStyle;
use shodo::font::{FontCollection, FontQuery};
use shodo::style::FontStyle;

/// Advance of U+0030 in the font `key` selects, in px.
///
/// The key carries the declaring element's family list, size, weight and
/// style, so a value inherited in `ch` keeps the font it was declared with.
/// When no face is selected, or the face has no `0`, the result is half the
/// size (CSS Values 4 §6.1.1), which is also what style resolution uses when
/// it cannot measure.
pub(crate) fn ch_advance(fonts: &FontCollection, key: &ChFontKey) -> f32 {
    let size = key.size.0;
    let Ok(families) = map_font_families(&key.family) else {
        return size * 0.5;
    };
    let style = match key.style {
        CssFontStyle::Italic => FontStyle::Italic,
        CssFontStyle::Oblique => FontStyle::Oblique(14.0),
        _ => FontStyle::Normal,
    };
    let query = FontQuery {
        families,
        weight: key.weight,
        style,
        ..FontQuery::default()
    };
    fonts.resolve_ch(&query, size).advance
}

/// Advance of U+0030 in the font `key` selects from `fonts`, in px: the
/// `ch` unit as the inline engine measures it.
pub fn measure_ch_advance(fonts: &FontCollection, key: &ChFontKey) -> f32 {
    ch_advance(fonts, key)
}

#[cfg(test)]
mod tests;
