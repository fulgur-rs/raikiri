//! Text that is not part of a paragraph (page margin boxes, generated
//! pseudo-element text, list markers), shaped by the inline engine when the
//! document has one.

use raikiri_dom::{Document, StandaloneAlign, StandaloneStyle, StandaloneText};

/// The family names of a CSS `font-family` string, in order. Quotes are kept:
/// the engine side tells a quoted `"serif"` (a named family) from the generic
/// keyword.
fn families(list: &str) -> Vec<String> {
    list.split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The size the parley path uses for an unusable value.
pub(crate) fn usable_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    }
}

/// The shaped run when the document's engine takes `content`, else `None`:
/// the caller then uses the parley path.
pub(crate) fn shape(
    document: Option<&Document>,
    content: &str,
    font_size: f32,
    font_family: &str,
    width: Option<f32>,
    align: StandaloneAlign,
) -> Option<StandaloneText> {
    let style = StandaloneStyle {
        families: families(font_family),
        font_size: usable_size(font_size),
    };
    document?.shape_standalone_text(content, &style, width, align)
}

#[cfg(test)]
mod tests;
