use super::*;
use cssparser::ParserInput;

fn parse(name: &str, text: &str) -> Option<String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let key = property_key_for_name(name)?;
    let value = parse_svg_presentation(key, &mut parser)?;
    parser.expect_exhausted().ok()?;
    assert_eq!(value.key(), key);
    match value {
        PropertyValue::Fill(text)
        | PropertyValue::Stroke(text)
        | PropertyValue::FillOpacity(text)
        | PropertyValue::StrokeOpacity(text)
        | PropertyValue::StrokeWidth(text) => Some(text.into()),
        other => panic!("unexpected value {other:?}"),
    }
}

#[test]
fn a_paint_is_a_keyword_a_color_or_a_local_reference() {
    assert_eq!(parse("fill", "none").as_deref(), Some("none"));
    assert_eq!(parse("fill", "NONE").as_deref(), Some("none"));
    assert_eq!(
        parse("stroke", "currentColor").as_deref(),
        Some("currentColor")
    );
    assert_eq!(
        parse("fill", "context-fill").as_deref(),
        Some("context-fill")
    );
    assert_eq!(
        parse("stroke", "Context-Stroke").as_deref(),
        Some("context-stroke")
    );
    assert_eq!(parse("fill", "blue").as_deref(), Some("rgba(0,0,255,1)"));
    assert_eq!(
        parse("fill", "#ff000080").as_deref(),
        Some("rgba(255,0,0,0.501961)")
    );
    assert_eq!(
        parse("fill", "rgb(1 2 3 / 0)").as_deref(),
        Some("rgba(1,2,3,0)")
    );
    assert_eq!(parse("fill", "url(#g)").as_deref(), Some("url(\"#g\")"));
    assert_eq!(parse("fill", "url('#g')").as_deref(), Some("url(\"#g\")"));
}

#[test]
fn a_reference_may_name_a_fallback() {
    assert_eq!(
        parse("fill", "url(#g) red").as_deref(),
        Some("url(\"#g\") rgba(255,0,0,1)")
    );
    assert_eq!(
        parse("fill", "url(#g) none").as_deref(),
        Some("url(\"#g\") none")
    );
    assert_eq!(
        parse("fill", "url(#g) currentcolor").as_deref(),
        Some("url(\"#g\") currentColor")
    );
    assert_eq!(parse("fill", "url(#g) 1"), None);
}

#[test]
fn a_paint_that_cannot_be_rendered_in_the_svg_is_invalid() {
    assert_eq!(parse("fill", "url(https://example.com/a.svg#g)"), None);
    assert_eq!(parse("fill", "url(a.svg#g)"), None);
    assert_eq!(parse("fill", "url(#)"), None);
    assert_eq!(parse("fill", "color-mix(in srgb, currentcolor, red)"), None);
    assert_eq!(parse("fill", "context-fill red"), None);
    assert_eq!(parse("fill", "12"), None);
    assert_eq!(parse("fill", "red blue"), None);
    assert_eq!(parse("fill", ""), None);
    assert_eq!(parse("fill", "inherit"), None);
}

#[test]
fn an_opacity_is_clamped_and_written_as_a_number() {
    assert_eq!(parse("fill-opacity", "0.25").as_deref(), Some("0.25"));
    assert_eq!(parse("stroke-opacity", "50%").as_deref(), Some("0.5"));
    assert_eq!(parse("fill-opacity", "2").as_deref(), Some("1"));
    assert_eq!(parse("stroke-opacity", "-1").as_deref(), Some("0"));
    assert_eq!(parse("fill-opacity", "red"), None);
    assert_eq!(parse("fill-opacity", "1 2"), None);
}

#[test]
fn a_stroke_width_is_a_non_negative_number_or_length() {
    assert_eq!(parse("stroke-width", "3").as_deref(), Some("3"));
    assert_eq!(parse("stroke-width", "2.5px").as_deref(), Some("2.5px"));
    assert_eq!(parse("stroke-width", "1EM").as_deref(), Some("1em"));
    assert_eq!(parse("stroke-width", "10%").as_deref(), Some("10%"));
    assert_eq!(parse("stroke-width", "0").as_deref(), Some("0"));
    assert_eq!(parse("stroke-width", "-1"), None);
    assert_eq!(parse("stroke-width", "-1px"), None);
    assert_eq!(parse("stroke-width", "2vw"), None);
    assert_eq!(parse("stroke-width", "thick"), None);
    assert_eq!(parse("stroke-width", "1 2"), None);
}

#[test]
fn a_declaration_may_end_in_important() {
    let mut input = ParserInput::new("blue !important");
    let mut parser = Parser::new(&mut input);
    assert!(parse_svg_presentation(PropertyKey::Fill, &mut parser).is_some());
}

#[test]
fn only_the_presentation_keys_parse() {
    let mut input = ParserInput::new("1");
    let mut parser = Parser::new(&mut input);
    assert!(parse_svg_presentation(PropertyKey::Opacity, &mut parser).is_none());
}
