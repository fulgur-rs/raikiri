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

fn content_items(source: &str) -> Vec<ContentComponent> {
    match parse(source, "content") {
        // PropertyValue::Content(Arc<Vec<..>>) を expose するため
        // (*v).clone() で Vec を deref-clone。tests は既存 shape のまま検証。
        Some(PropertyValue::Content(v)) => (*v).clone(),
        other => panic!("expected PropertyValue::Content, got {other:?}"),
    }
}

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
