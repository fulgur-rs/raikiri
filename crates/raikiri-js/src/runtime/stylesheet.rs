//! The live inline stylesheet surface used by DOM scripts.

use std::ops::Range;

use boa_engine::object::JsObject;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, Finalize, JsData, JsError, JsNativeError, JsResult, JsString, JsValue, Trace,
    js_string,
};
use cssparser::{Delimiter, Parser, ParserInput, Token};
use raikiri_dom::{Document, NodeKind};

use super::indexed::{IndexedSource, indexed_object, this_indexed};
use super::interfaces::function;
use super::webidl::{arg_unsigned_long, this_document, with_state};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CssRuleInfo {
    pub(crate) prelude: String,
    pub(crate) source: Range<usize>,
    pub(crate) body: Option<Range<usize>>,
}

/// Parse the top-level CSS rules while retaining source ranges for live edits.
pub(crate) fn parse_css_rules(source: &str) -> Vec<CssRuleInfo> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut rules = Vec::new();

    loop {
        parser.skip_whitespace();
        let start = parser.position().byte_index();
        if start >= source.len() {
            break;
        }
        let prelude = parser.parse_until_before(
            Delimiter::CurlyBracketBlock | Delimiter::Semicolon,
            |prelude| {
                let start = prelude.position();
                while prelude.next_including_whitespace_and_comments().is_ok() {}
                Ok::<_, cssparser::ParseError<'_, ()>>(
                    prelude.slice(start..prelude.position()).to_owned(),
                )
            },
        );
        let Ok(prelude) = prelude else {
            break;
        };
        let prelude = prelude.trim().to_owned();
        let token = parser.next_including_whitespace_and_comments().cloned();
        match token {
            Ok(Token::CurlyBracketBlock) => {
                let body_start = parser.position().byte_index();
                let body_end = parser.parse_nested_block(|body| {
                    let start = body.position();
                    while body.next_including_whitespace_and_comments().is_ok() {}
                    Ok::<_, cssparser::ParseError<'_, ()>>(
                        body.position().byte_index().max(start.byte_index()),
                    )
                });
                let Ok(body_end) = body_end else {
                    break;
                };
                let end = parser.position().byte_index();
                rules.push(CssRuleInfo {
                    prelude,
                    source: start..end,
                    body: Some(body_start..body_end),
                });
            }
            Ok(Token::Semicolon) => {
                let end = parser.position().byte_index();
                rules.push(CssRuleInfo {
                    prelude,
                    source: start..end,
                    body: None,
                });
            }
            _ => break,
        }
    }

    rules
}

pub(crate) fn stylesheet_text(document: &Document, style_element: usize) -> String {
    document
        .element_text_content(style_element)
        .unwrap_or_default()
}

fn stylesheet_nodes(document: &Document) -> Vec<usize> {
    let root = document.root_index();
    let Some(root_node) = document.get_node(root) else {
        return Vec::new();
    };
    let mut pending: Vec<usize> = root_node.children.iter().rev().copied().collect();
    let mut nodes = Vec::new();
    while let Some(index) = pending.pop() {
        let Some(node) = document.get_node(index) else {
            continue;
        };
        if node.kind() != NodeKind::Element {
            continue;
        }
        if node.tag_name() == Some("style")
            && matches!(
                document.element_namespace_uri(index),
                Some("http://www.w3.org/1999/xhtml") | Some("http://www.w3.org/2000/svg")
            )
        {
            nodes.push(index);
            continue;
        }
        pending.extend(node.children.iter().rev().copied());
    }
    nodes
}

struct StyleSheetListSource;

impl IndexedSource for StyleSheetListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        with_state(context, |state| {
            stylesheet_nodes(state.host.document()).len()
        })
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        let node = with_state(context, |state| {
            stylesheet_nodes(state.host.document()).get(index).copied()
        })?;
        node.map(|node| style_sheet_object(context, node).map(JsValue::from))
            .transpose()
    }
}

struct CssRuleListSource {
    style_element: usize,
}

impl IndexedSource for CssRuleListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        let style_element = self.style_element;
        let text = with_state(context, |state| {
            stylesheet_text(state.host.document(), style_element)
        })?;
        Ok(parse_css_rules(&text).len())
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        let style_element = self.style_element;
        let exists = with_state(context, |state| {
            let text = stylesheet_text(state.host.document(), style_element);
            index < parse_css_rules(&text).len()
        })?;
        if exists {
            css_style_rule_object(context, style_element, index).map(|rule| Some(rule.into()))
        } else {
            Ok(None)
        }
    }
}

fn list_prototype(
    context: &mut Context,
    type_name: &str,
    length: boa_engine::native_function::NativeFunctionPointer,
    item: boa_engine::native_function::NativeFunctionPointer,
) -> JsResult<JsObject> {
    let prototype = JsObject::from_proto_and_data(
        Some(context.intrinsics().constructors().object().prototype()),
        (),
    );
    let length_getter = function(context, "get length", 0, length)?;
    let length_descriptor = PropertyDescriptor::builder()
        .get(length_getter)
        .enumerable(true)
        .configurable(true)
        .build();
    prototype.define_property_or_throw(js_string!("length"), length_descriptor, context)?;
    let item_function = function(context, "item", 1, item)?;
    let item_descriptor = PropertyDescriptor::builder()
        .value(item_function)
        .writable(true)
        .enumerable(true)
        .configurable(true)
        .build();
    prototype.define_property_or_throw(js_string!("item"), item_descriptor, context)?;
    super::interfaces::set_to_string_tag(&prototype, type_name, context)?;
    Ok(prototype)
}

fn stylesheet_list_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<StyleSheetListSource>(this, context, "StyleSheetList")?;
    Ok(JsValue::from(source.length(context)? as u32))
}

fn stylesheet_list_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<StyleSheetListSource>(this, context, "StyleSheetList")?;
    let index = arg_unsigned_long(args, 0, context)?;
    Ok(source.item(index, context)?.unwrap_or_else(JsValue::null))
}

fn css_rule_list_length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_indexed::<CssRuleListSource>(this, context, "CSSRuleList")?;
    Ok(JsValue::from(source.length(context)? as u32))
}

fn css_rule_list_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<CssRuleListSource>(this, context, "CSSRuleList")?;
    let index = arg_unsigned_long(args, 0, context)?;
    Ok(source.item(index, context)?.unwrap_or_else(JsValue::null))
}

pub(crate) fn document_style_sheets(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_document(this, context)?;
    if let Some(existing) = with_state(context, |state| state.style_sheet_list.clone())? {
        return Ok(existing.into());
    }
    let prototype = list_prototype(
        context,
        "StyleSheetList",
        stylesheet_list_length,
        stylesheet_list_item,
    )?;
    let list = indexed_object(context, prototype, StyleSheetListSource)?;
    with_state(context, |state| state.style_sheet_list = Some(list.clone()))?;
    Ok(list.into())
}

#[derive(Trace, Finalize, JsData)]
struct StyleSheetData {
    #[unsafe_ignore_trace]
    style_element: usize,
}

#[derive(Trace, Finalize, JsData)]
struct CssStyleRuleData {
    #[unsafe_ignore_trace]
    style_element: usize,
    #[unsafe_ignore_trace]
    rule_index: usize,
}

fn define_getter(
    object: &JsObject,
    name: &str,
    getter: boa_engine::native_function::NativeFunctionPointer,
    context: &mut Context,
) -> JsResult<()> {
    let function = function(context, &format!("get {name}"), 0, getter)?;
    let descriptor = PropertyDescriptor::builder()
        .get(function)
        .enumerable(true)
        .configurable(true)
        .build();
    object.define_property_or_throw(JsString::from(name), descriptor, context)?;
    Ok(())
}

fn style_sheet_object(context: &mut Context, style_element: usize) -> JsResult<JsObject> {
    if let Some(existing) = with_state(context, |state| {
        state.style_sheets.get(&style_element).cloned()
    })? {
        return Ok(existing);
    }
    let prototype = context.intrinsics().constructors().object().prototype();
    let object = JsObject::from_proto_and_data(Some(prototype), StyleSheetData { style_element });
    define_getter(&object, "cssRules", style_sheet_rules, context)?;
    let to_string_tag = PropertyDescriptor::builder()
        .value(JsString::from("CSSStyleSheet"))
        .writable(false)
        .enumerable(false)
        .configurable(true)
        .build();
    object.define_property_or_throw(
        boa_engine::JsSymbol::to_string_tag(),
        to_string_tag,
        context,
    )?;
    with_state(context, |state| {
        state.style_sheets.insert(style_element, object.clone())
    })?;
    Ok(object)
}

fn style_sheet_node(this: &JsValue) -> JsResult<usize> {
    this.as_object()
        .and_then(|object| {
            object
                .downcast_ref::<StyleSheetData>()
                .map(|data| data.style_element)
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a CSSStyleSheet")
                .into()
        })
}

fn style_sheet_rules(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let style_element = style_sheet_node(this)?;
    if let Some(existing) = with_state(context, |state| {
        state.css_rule_lists.get(&style_element).cloned()
    })? {
        return Ok(existing.into());
    }
    let prototype = list_prototype(
        context,
        "CSSRuleList",
        css_rule_list_length,
        css_rule_list_item,
    )?;
    let list = indexed_object(context, prototype, CssRuleListSource { style_element })?;
    with_state(context, |state| {
        state.css_rule_lists.insert(style_element, list.clone())
    })?;
    Ok(list.into())
}

pub(crate) fn css_style_rule_object(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
) -> JsResult<JsObject> {
    let key = (style_element, rule_index);
    if let Some(existing) = with_state(context, |state| state.css_style_rules.get(&key).cloned())? {
        return Ok(existing);
    }
    let prototype = context.intrinsics().constructors().object().prototype();
    let object = JsObject::from_proto_and_data(
        Some(prototype),
        CssStyleRuleData {
            style_element,
            rule_index,
        },
    );
    define_getter(&object, "cssText", css_rule_text, context)?;
    define_getter(&object, "selectorText", css_rule_selector, context)?;
    define_getter(&object, "style", css_rule_style, context)?;
    let to_string_tag = PropertyDescriptor::builder()
        .value(JsString::from("CSSStyleRule"))
        .writable(false)
        .enumerable(false)
        .configurable(true)
        .build();
    object.define_property_or_throw(
        boa_engine::JsSymbol::to_string_tag(),
        to_string_tag,
        context,
    )?;
    with_state(context, |state| {
        state.css_style_rules.insert(key, object.clone())
    })?;
    Ok(object)
}

fn css_rule_data(this: &JsValue) -> JsResult<(usize, usize)> {
    this.as_object()
        .and_then(|object| {
            object
                .downcast_ref::<CssStyleRuleData>()
                .map(|data| (data.style_element, data.rule_index))
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a CSSStyleRule")
                .into()
        })
}

fn rule_info(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
) -> JsResult<(String, CssRuleInfo)> {
    with_state(context, |state| {
        let text = stylesheet_text(state.host.document(), style_element);
        parse_css_rules(&text)
            .get(rule_index)
            .cloned()
            .map(|rule| (text, rule))
    })?
    .ok_or_else(|| {
        JsNativeError::typ()
            .with_message("CSS rule no longer exists")
            .into()
    })
}

fn css_rule_text(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (style_element, rule_index) = css_rule_data(this)?;
    let (text, rule) = rule_info(context, style_element, rule_index)?;
    Ok(JsValue::from(JsString::from(&text[rule.source])))
}

fn css_rule_selector(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (style_element, rule_index) = css_rule_data(this)?;
    let (_, rule) = rule_info(context, style_element, rule_index)?;
    Ok(JsValue::from(JsString::from(rule.prelude)))
}

fn css_rule_style(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (style_element, rule_index) = css_rule_data(this)?;
    let (_, rule) = rule_info(context, style_element, rule_index)?;
    if rule.prelude.starts_with('@') || rule.body.is_none() {
        return Ok(JsValue::undefined());
    }
    let key = (style_element, rule_index);
    if let Some(existing) = with_state(context, |state| state.css_rule_styles.get(&key).cloned())? {
        return Ok(existing.into());
    }
    let style = super::style::stylesheet_style_object(context, style_element, rule_index)?;
    with_state(context, |state| {
        state.css_rule_styles.insert(key, style.clone())
    })?;
    Ok(style.into())
}

pub(crate) fn parent_rule(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
) -> JsResult<JsValue> {
    Ok(css_style_rule_object(context, style_element, rule_index)?.into())
}

pub(crate) fn stylesheet_rule_body(
    document: &Document,
    style_element: usize,
    rule_index: usize,
) -> Option<String> {
    let text = stylesheet_text(document, style_element);
    let rule = parse_css_rules(&text).get(rule_index)?.clone();
    Some(
        rule.body
            .map_or_else(String::new, |range| text[range].to_owned()),
    )
}

pub(crate) fn write_stylesheet_rule_body(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
    body: &str,
) -> JsResult<()> {
    let result = with_state(context, |state| {
        let text = stylesheet_text(state.host.document(), style_element);
        let Some(rule) = parse_css_rules(&text).get(rule_index).cloned() else {
            return Err("CSS rule no longer exists".to_owned());
        };
        let Some(body_range) = rule.body else {
            return Err("CSS rule has no declaration block".to_owned());
        };
        let mut updated = String::with_capacity(text.len() + body.len());
        updated.push_str(&text[..body_range.start]);
        updated.push_str(body);
        updated.push_str(&text[body_range.end..]);
        state
            .host
            .document_mut()
            .set_element_text_content(style_element, updated)
    })?;
    result.map_err(|_| {
        JsError::from(JsNativeError::typ().with_message("unable to update stylesheet rule"))
    })?;
    super::node::mark_dirty(context)
}

#[cfg(test)]
mod tests;
