//! The text of each longhand a `background` value sets, for CSSOM
//! declaration blocks, which hold a shorthand as its longhands (CSSOM
//! "set a CSS declaration" for a shorthand).

use cssparser::{ParseError, Parser, ParserInput};

use super::super::serialize::serialize_value;
use super::super::types::PropertyKey;
use super::color::parse_contextual_color;
use super::visual::{
    parse_background_attachment, parse_background_image, parse_background_repeat,
    parse_background_shorthand, parse_background_size, parse_bg_position, parse_clip_box,
};

/// The longhands of `background`, in the order of its grammar (CSS
/// Backgrounds 3 §2.11), which CSSOM uses to list them.
pub const BACKGROUND_LONGHANDS: [&str; 8] = [
    "background-image",
    "background-position",
    "background-size",
    "background-repeat",
    "background-attachment",
    "background-origin",
    "background-clip",
    "background-color",
];

/// The components of one layer whose parsed value forgets how they were
/// written, as their source text.
#[derive(Default)]
struct LayerText {
    image: Option<String>,
    position: Option<String>,
    color: Option<String>,
}

/// Each longhand of [`BACKGROUND_LONGHANDS`] with the value `value` (a
/// `background` value) sets for it, in that order, or `None` when `value`
/// is not a valid `background`. A component a layer omits takes its
/// initial value. Image, position, and color keep their source text, for
/// the caller to serialize as values of their own longhand; the rest are
/// serialized here.
pub fn background_longhand_texts(value: &str) -> Option<Vec<(&'static str, String)>> {
    let mut input = ParserInput::new(value);
    let shorthand = Parser::new(&mut input)
        .parse_entirely(|i| {
            parse_background_shorthand(i).ok_or_else(|| i.new_custom_error::<_, ()>(()))
        })
        .ok()?;
    let mut input = ParserInput::new(value);
    let layers = Parser::new(&mut input)
        .parse_entirely(|i| i.parse_comma_separated(layer_text))
        .ok()?;
    let joined = |text: fn(&LayerText) -> &Option<String>, initial: &str| {
        layers
            .iter()
            .map(|layer| text(layer).as_deref().unwrap_or(initial))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let serialized =
        |key| serialize_value(&shorthand.longhand(key)?).filter(|text| !text.is_empty());
    let color = layers
        .last()
        .and_then(|layer| layer.color.clone())
        .unwrap_or_else(|| "transparent".to_owned());
    Some(vec![
        ("background-image", joined(|layer| &layer.image, "none")),
        (
            "background-position",
            joined(|layer| &layer.position, "0% 0%"),
        ),
        ("background-size", serialized(PropertyKey::BackgroundSize)?),
        (
            "background-repeat",
            serialized(PropertyKey::BackgroundRepeat)?,
        ),
        (
            "background-attachment",
            serialized(PropertyKey::BackgroundAttachment)?,
        ),
        (
            "background-origin",
            serialized(PropertyKey::BackgroundOrigin)?,
        ),
        ("background-clip", serialized(PropertyKey::BackgroundClip)?),
        ("background-color", color),
    ])
}

/// The source text of one layer's image, position, and color. The value
/// was already parsed as a whole, and the token sets of the components are
/// disjoint, so each component is recognized as it was there.
fn layer_text<'i>(input: &mut Parser<'i, '_>) -> Result<LayerText, ParseError<'i, ()>> {
    let mut text = LayerText::default();
    loop {
        let start = input.position();
        let source = |input: &Parser<'_, '_>| input.slice_from(start).trim().to_owned();
        if text.image.is_none() && parses(input, parse_background_image) {
            text.image = Some(source(input));
        } else if parses(input, parse_background_repeat)
            || parses(input, parse_background_attachment)
            || parses(input, parse_clip_box)
        {
            // Serialized from the parsed value instead.
            continue;
        } else if text.position.is_none() && parses(input, parse_bg_position) {
            text.position = Some(source(input));
            input
                .try_parse(|i| -> Result<(), ParseError<'_, ()>> {
                    i.expect_delim('/')?;
                    parse_background_size(i)
                        .map(drop)
                        .ok_or_else(|| i.new_custom_error(()))
                })
                .ok();
        } else if parses(input, parse_contextual_color) {
            text.color = Some(source(input));
        } else {
            return Ok(text);
        }
    }
}

/// Whether `parse` accepts the next component of `input`, which it then
/// consumes.
fn parses<T>(input: &mut Parser<'_, '_>, parse: fn(&mut Parser<'_, '_>) -> Option<T>) -> bool {
    input
        .try_parse(|i| parse(i).ok_or_else(|| i.new_custom_error::<_, ()>(())))
        .is_ok()
}
