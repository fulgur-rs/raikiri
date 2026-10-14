//! Measure the CSS `ch` and `lh` units with the inline engine's own fonts.

use super::style::map_font_families;
use raikiri_style::ChFontKey;
use raikiri_style::property::FontStyle as CssFontStyle;
use shodo::font::{FontCollection, FontQuery};
use shodo::style::{FontStyle, InlineStyle};

/// The inline engine's query for the font `key` selects, or `None` when its
/// family list does not map.
fn font_query(key: &ChFontKey) -> Option<FontQuery> {
    let families = map_font_families(&key.family).ok()?;
    let style = match key.style {
        CssFontStyle::Italic => FontStyle::Italic,
        CssFontStyle::Oblique => FontStyle::Oblique(14.0),
        _ => FontStyle::Normal,
    };
    Some(FontQuery {
        families,
        weight: key.weight,
        style,
        ..FontQuery::default()
    })
}

/// Advance of U+0030 in the font `key` selects, in px.
///
/// The key carries the declaring element's family list, size, weight and
/// style, so a value inherited in `ch` keeps the font it was declared with.
/// When no face is selected, or the face has no `0`, the result is half the
/// size (CSS Values 4 §6.1.1), which is also what style resolution uses when
/// it cannot measure.
pub(crate) fn ch_advance(fonts: &FontCollection, key: &ChFontKey) -> f32 {
    let size = key.size.0;
    let Some(query) = font_query(key) else {
        return size * 0.5;
    };
    fonts.resolve_ch(&query, size).advance
}

/// Advance of U+0030 in the font `key` selects from `fonts`, in px: the
/// `ch` unit as the inline engine measures it.
pub fn measure_ch_advance(fonts: &FontCollection, key: &ChFontKey) -> f32 {
    ch_advance(fonts, key)
}

/// Used length of `line-height: normal` in the first available font `key`
/// selects from `fonts`, in px: the sum of its ascent, descent and line gap.
///
/// The inline engine measures it the way it sizes a `normal` line, so the
/// face is the [first available font](https://www.w3.org/TR/css-fonts-4/#first-available-font)
/// at the variation instance the key's weight and size select. The key
/// carries no `font-variation-settings`, so explicit axis settings are not
/// applied. `None` when the family list does not map.
pub fn measure_normal_line_height(fonts: &FontCollection, key: &ChFontKey) -> Option<f32> {
    let query = font_query(key)?;
    Some(fonts.normal_line_height(&InlineStyle {
        font_families: query.families,
        font_size: key.size.0,
        font_weight: query.weight,
        font_style: query.style,
        ..InlineStyle::default()
    }))
}

#[cfg(test)]
mod tests;
