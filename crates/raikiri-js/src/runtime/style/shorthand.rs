//! Shorthands a declaration block holds as their longhands (CSSOM "set a
//! CSS declaration" for a shorthand), and their serialization back from
//! those longhands (CSSOM "serialize a CSS value" for a shorthand).
//!
//! Only `background` is held this way so far; every other shorthand stays
//! one declaration as specified.

use cssparser::{Parser, ParserInput};
use raikiri_style::property::{BACKGROUND_LONGHANDS, background_longhand_texts};

use super::{css_wide_keyword, declaration_value};

/// The longhands a declaration block holds in place of the shorthand
/// `name`, or `None` when it holds `name` itself.
pub(super) fn longhands_of(name: &str) -> Option<&'static [&'static str]> {
    name.eq_ignore_ascii_case("background")
        .then_some(&BACKGROUND_LONGHANDS[..])
}

/// The shorthand whose longhands include `name`, when a declaration block
/// holds that shorthand as its longhands.
pub(super) fn shorthand_of(name: &str) -> Option<&'static str> {
    BACKGROUND_LONGHANDS
        .iter()
        .any(|longhand| name.eq_ignore_ascii_case(longhand))
        .then_some("background")
}

/// The longhand declarations the shorthand `name` with the stored value
/// `value` sets, or `None` when the block keeps it as one declaration: a
/// value with a substitution function, which only the cascade can split.
pub(super) fn expand(name: &str, value: &str, page: bool) -> Option<Vec<(String, String)>> {
    let longhands = longhands_of(name)?;
    if let Some(keyword) = css_wide_keyword(value) {
        return Some(
            longhands
                .iter()
                .map(|longhand| ((*longhand).to_owned(), keyword.to_owned()))
                .collect(),
        );
    }
    background_longhand_texts(value)?
        .into_iter()
        .map(|(longhand, text)| {
            declaration_value(longhand, &text, page).map(|value| (longhand.to_owned(), value))
        })
        .collect()
}

/// The value of the shorthand `name` from `values`, the values of its
/// longhands in [`longhands_of`] order, or `None` when no value of the
/// shorthand sets them all to those values.
pub(super) fn assemble(name: &str, values: &[&str]) -> Option<String> {
    longhands_of(name)?;
    if let Some(keyword) = css_wide_keyword(values[0]) {
        return values
            .iter()
            .all(|value| css_wide_keyword(value) == Some(keyword))
            .then(|| keyword.to_owned());
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        return None;
    }
    assemble_background(values)
}

/// `background` from its longhands, in [`BACKGROUND_LONGHANDS`] order.
/// Every list must have one item per image layer, and each layer leaves
/// out what equals its initial value, as the shortest form browsers agree
/// on (CSS Backgrounds 3 §2.11 sets omitted components to their initial
/// value).
fn assemble_background(values: &[&str]) -> Option<String> {
    let [
        images,
        positions,
        sizes,
        repeats,
        attachments,
        origins,
        clips,
        color,
    ]: [&str; 8] = std::array::from_fn(|index| values[index]);
    let lists = [
        images,
        positions,
        sizes,
        repeats,
        attachments,
        origins,
        clips,
    ]
    .map(comma_separated);
    let layer_count = lists[0].len();
    if lists.iter().any(|list| list.len() != layer_count) {
        return None;
    }
    let mut layers = Vec::with_capacity(layer_count);
    for index in 0..layer_count {
        let [image, position, size, repeat, attachment, origin, clip] =
            lists.each_ref().map(|list| list[index].as_str());
        let mut parts = Vec::new();
        if index + 1 == layer_count && !matches!(color, "transparent" | "rgba(0, 0, 0, 0)") {
            parts.push(color.to_owned());
        }
        if image != "none" {
            parts.push(image.to_owned());
        }
        if size != "auto" {
            parts.push(format!("{position} / {size}"));
        } else if position != "0% 0%" {
            parts.push(position.to_owned());
        }
        if repeat != "repeat" {
            parts.push(repeat.to_owned());
        }
        if attachment != "scroll" {
            parts.push(attachment.to_owned());
        }
        let clip_only = matches!(clip, "text" | "border-area" | "border-area text");
        if clip_only && origin == "border-box" || origin == clip {
            parts.push(clip.to_owned());
        } else if (origin, clip) != ("padding-box", "border-box") {
            parts.push(format!("{origin} {clip}"));
        }
        layers.push(if parts.is_empty() {
            "none".to_owned()
        } else {
            parts.join(" ")
        });
    }
    Some(layers.join(", "))
}

/// The items of the comma-separated list `value`, as their source text.
fn comma_separated(value: &str) -> Vec<String> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    parser.parse_comma_separated_ignoring_errors(|item| {
        let start = item.position();
        while item.next().is_ok() {}
        Ok::<_, cssparser::ParseError<'_, ()>>(item.slice_from(start).trim().to_owned())
    })
}
