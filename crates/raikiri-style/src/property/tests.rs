use std::sync::Arc;

use super::*;
use crate::Atom;
use cssparser::{ParseError, Parser, ParserInput};
use smol_str::SmolStr;

fn parse(source: &str, name: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_value(name, &mut parser)
}

fn parse_entire(source: &str, name: &str) -> Option<PropertyValue> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|i| -> Result<PropertyValue, ParseError<'_, ()>> {
            parse_value(name, i).ok_or_else(|| i.new_custom_error(()))
        })
        .ok()
}

/// Asserts that each CSS-wide keyword parses to the deferred marker the
/// cascade resolves, for the property spelled `name`.
fn assert_css_wide_keywords_deferred(name: &str) {
    let key = property_key_for_name(name).unwrap_or_else(|| panic!("`{name}` has no key"));
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        let Some(PropertyValue::Deferred(value)) = parse(keyword, name) else {
            panic!("expected a CSS-wide defaulting marker for `{name}: {keyword}`");
        };
        assert_eq!(value.key, key, "{name}: {keyword}");
        assert_eq!(
            value.css_wide_keyword(),
            CssWideKeyword::from_css_ident(keyword),
            "{name}: {keyword}"
        );
    }
}

fn content_items(source: &str) -> Vec<ContentComponent> {
    match parse(source, "content") {
        // Clone the items of PropertyValue::Content(Arc<ContentList>).
        Some(PropertyValue::Content(v)) => v.to_vec(),
        other => panic!("expected PropertyValue::Content, got {other:?}"),
    }
}

mod background_longhands_tests;
mod box_model_tests;
mod color_tests;
mod common_tests;
mod content_tests;
mod hyphenate_limit_chars_tests;
mod keyword_tests;
mod layout_tests;
mod misc_tests;
mod serialize_tests;
mod text_indent_calc_tests;
mod text_tests;
mod visual_tests;

mod css_wide_tests;

mod column_rule_tests;

mod column_span_tests;
