//! The live inline stylesheet surface used by DOM scripts.

use std::ops::Range;

use boa_engine::object::JsObject;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, Finalize, JsData, JsError, JsNativeError, JsResult, JsString, JsValue, Trace,
    js_string,
};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss, Token,
};
use raikiri_dom::{Document, NodeKind};

use super::indexed::{IndexedSource, indexed_object, this_indexed};
use super::interfaces::function;
use super::webidl::{
    arg_unsigned_long, dom_string, this_document, throw_dom_exception, with_state,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CssRuleInfo {
    pub(crate) prelude: String,
    pub(crate) source: Range<usize>,
    pub(crate) body: Option<Range<usize>>,
}

/// Parse the top-level CSS rules while retaining source ranges for live edits.
///
/// Rules are found by cssparser's [`cssparser::StyleSheetParser`], the same
/// top-level algorithm the cascade uses, so rule boundaries (and therefore
/// `cssRules` indices) agree with it: a qualified rule's prelude runs up to
/// its `{` block (a `;` does not end it), an at-rule without a block may end
/// at EOF, and top-level `<!--`/`-->` are skipped.
pub(crate) fn parse_css_rules(source: &str) -> Vec<CssRuleInfo> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut ranges = RuleRangeParser { source };
    let mut rules = StyleSheetParser::new(&mut parser, &mut ranges);
    let mut out = Vec::new();
    while let Some(rule) = rules.next() {
        let Ok((prelude, start, body)) = rule else {
            continue; // cov:ignore: RuleRangeParser accepts every rule cssparser hands it.
        };
        out.push(CssRuleInfo {
            prelude,
            source: start..rules.input.position().byte_index(),
            body,
        });
    }
    out
}

/// Whether `prelude` is a valid `@page` rule's prelude. The at-keyword is
/// compared after tokenization, so escaped spellings match like the cascade
/// sees them (ASCII case-insensitive), and the selector must parse as the
/// cascade requires; a rule the cascade drops gets no `CSSPageRule`.
pub(crate) fn is_page_prelude(prelude: &str) -> bool {
    let mut input = ParserInput::new(prelude);
    let mut parser = Parser::new(&mut input);
    if !matches!(
        parser.next_including_whitespace(),
        Ok(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("page")
    ) {
        return false;
    }
    // Validate the source after the at-keyword rather than a re-serialization,
    // so comments keep separating tokens exactly as they do for the cascade.
    let selector = &prelude[parser.position().byte_index()..];
    raikiri_style::is_valid_page_selector(selector)
}

/// The page selector list of an `@page` prelude, serialized from its tokens:
/// comments dropped, whitespace runs collapsed to one space, and the result
/// trimmed. `None` when `prelude` does not start with the `page` at-keyword.
fn page_selector_text(prelude: &str) -> Option<String> {
    let mut input = ParserInput::new(prelude);
    let mut parser = Parser::new(&mut input);
    match parser.next_including_whitespace() {
        Ok(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("page") => {}
        _ => return None,
    }
    let mut out = String::new();
    while let Ok(token) = parser.next_including_whitespace() {
        if matches!(token, Token::WhiteSpace(_)) {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push_str(&token.to_css_string());
        }
    }
    Some(out.trim_end().to_owned())
}

/// The page margin box names (CSS Paged Media 3 §4.2) that `insertRule` on a
/// `CSSPageRule` accepts.
const MARGIN_RULE_NAMES: [&str; 16] = [
    "top-left-corner",
    "top-left",
    "top-center",
    "top-right",
    "top-right-corner",
    "right-top",
    "right-middle",
    "right-bottom",
    "bottom-right-corner",
    "bottom-right",
    "bottom-center",
    "bottom-left",
    "bottom-left-corner",
    "left-bottom",
    "left-middle",
    "left-top",
];

/// Source ranges of the margin rules in an `@page` body, in source order:
/// a margin at-keyword (compared after tokenization, ASCII
/// case-insensitive) with an empty prelude and a block. Declarations and any
/// other nested at-rule are skipped, as the `@page` grammar drops them.
pub(crate) fn nested_at_rule_ranges(body: &str) -> Vec<Range<usize>> {
    let mut input = ParserInput::new(body);
    let mut parser = Parser::new(&mut input);
    let mut ranges_parser = NestedRuleRangeParser;
    let mut items = RuleBodyParser::new(&mut parser, &mut ranges_parser);
    let mut out = Vec::new();
    while let Some(item) = items.next() {
        if let Ok(Some(start)) = item {
            out.push(start..items.input.position().byte_index());
        }
    }
    out
}

/// Records where each nested at-rule of a rule body starts; declarations
/// parse to `None` so they are skipped without being reported as errors.
struct NestedRuleRangeParser;

impl<'i> DeclarationParser<'i> for NestedRuleRangeParser {
    type Declaration = Option<usize>;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<Option<usize>, ParseError<'i, ()>> {
        while input.next().is_ok() {}
        Ok(None)
    }
}

impl<'i> AtRuleParser<'i> for NestedRuleRangeParser {
    type Prelude = ();
    type AtRule = Option<usize>;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        if !MARGIN_RULE_NAMES
            .iter()
            .any(|margin| name.eq_ignore_ascii_case(margin))
        {
            return Err(input.new_custom_error(()));
        }
        input.expect_exhausted()?;
        Ok(())
    }

    fn rule_without_block(&mut self, _: (), _: &ParserState) -> Result<Option<usize>, ()> {
        Err(())
    }

    fn parse_block<'t>(
        &mut self,
        _: (),
        start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Option<usize>, ParseError<'i, ()>> {
        while input.next().is_ok() {}
        Ok(Some(start.position().byte_index()))
    }
}

impl<'i> QualifiedRuleParser<'i> for NestedRuleRangeParser {
    type Prelude = ();
    type QualifiedRule = Option<usize>;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Option<usize>, ()> for NestedRuleRangeParser {
    fn parse_qualified(&self) -> bool {
        false
    }

    fn parse_declarations(&self) -> bool {
        true
    }
}

/// `rule` as a single margin rule (`@<margin-box-name> { ... }`), serialized
/// from its tokens so that every block, string, and comment the input left
/// open at EOF is closed, or `None` when it is anything else.
fn parse_margin_rule(rule: &str) -> Option<String> {
    let [range] = <[Range<usize>; 1]>::try_from(nested_at_rule_ranges(rule)).ok()?;
    // Like "parse a rule", only whitespace and comments may surround it.
    let blank = |text: &str| {
        let mut input = ParserInput::new(text);
        Parser::new(&mut input).is_exhausted()
    };
    if !(blank(&rule[..range.start]) && blank(&rule[range.end..])) {
        return None;
    }
    let mut input = ParserInput::new(&rule[range]);
    let mut out = String::new();
    serialize_tokens(&mut Parser::new(&mut input), &mut out, 0).then_some(out)
}

/// Deepest block nesting [`serialize_tokens`] follows before giving up.
const MAX_SERIALIZED_BLOCK_DEPTH: usize = 256;

/// Appends the tokens of `input` to `out`, writing each block's closing token
/// whether or not the source had it (CSS Syntax 3 §5.4.8: EOF closes a
/// block). Returns `false` when blocks nest deeper than
/// [`MAX_SERIALIZED_BLOCK_DEPTH`].
fn serialize_tokens(input: &mut Parser<'_, '_>, out: &mut String, depth: usize) -> bool {
    while let Ok(token) = input.next_including_whitespace_and_comments() {
        let closer = match token {
            Token::Function(_) | Token::ParenthesisBlock => ")",
            Token::SquareBracketBlock => "]",
            Token::CurlyBracketBlock => "}",
            _ => {
                out.push_str(&token.to_css_string());
                continue;
            }
        };
        out.push_str(&token.to_css_string());
        if depth >= MAX_SERIALIZED_BLOCK_DEPTH {
            return false;
        }
        let nested = input.parse_nested_block(|nested| {
            if serialize_tokens(nested, out, depth + 1) {
                Ok(())
            } else {
                Err(nested.new_custom_error::<(), ()>(()))
            }
        });
        if nested.is_err() {
            return false;
        }
        out.push_str(closer);
    }
    true
}

/// A rule as [`RuleRangeParser`] reports it: the trimmed prelude text, the
/// rule's start offset, and its block body's range, if any. The rule's end
/// offset is read from the parser once the whole rule has been consumed.
type RuleRange = (String, usize, Option<Range<usize>>);

/// Accepts every top-level rule, recording where it is rather than what it
/// means. The prelude is the byte offset where the prelude ends.
struct RuleRangeParser<'s> {
    source: &'s str,
}

impl RuleRangeParser<'_> {
    fn prelude(&self, start: &ParserState, prelude_end: usize) -> String {
        self.source[start.position().byte_index()..prelude_end]
            .trim()
            .to_owned()
    }

    fn block<'i>(
        &self,
        prelude_end: usize,
        start: &ParserState,
        input: &mut Parser<'i, '_>,
    ) -> RuleRange {
        let body_start = input.position().byte_index();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let body_end = input.position().byte_index().max(body_start);
        (
            self.prelude(start, prelude_end),
            start.position().byte_index(),
            Some(body_start..body_end),
        )
    }
}

fn consume_prelude<'i>(input: &mut Parser<'i, '_>) -> usize {
    while input.next_including_whitespace_and_comments().is_ok() {}
    input.position().byte_index()
}

impl<'i> AtRuleParser<'i> for RuleRangeParser<'_> {
    type Prelude = usize;
    type AtRule = RuleRange;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<usize, ParseError<'i, ()>> {
        Ok(consume_prelude(input))
    }

    fn rule_without_block(
        &mut self,
        prelude_end: usize,
        start: &ParserState,
    ) -> Result<RuleRange, ()> {
        Ok((
            self.prelude(start, prelude_end),
            start.position().byte_index(),
            None,
        ))
    }

    fn parse_block<'t>(
        &mut self,
        prelude_end: usize,
        start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<RuleRange, ParseError<'i, ()>> {
        Ok(self.block(prelude_end, start, input))
    }
}

impl<'i> QualifiedRuleParser<'i> for RuleRangeParser<'_> {
    type Prelude = usize;
    type QualifiedRule = RuleRange;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<usize, ParseError<'i, ()>> {
        Ok(consume_prelude(input))
    }

    fn parse_block<'t>(
        &mut self,
        prelude_end: usize,
        start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<RuleRange, ParseError<'i, ()>> {
        Ok(self.block(prelude_end, start, input))
    }
}

pub(crate) fn stylesheet_text(document: &Document, style_element: usize) -> String {
    document
        .element_text_content(style_element)
        .unwrap_or_default()
}

fn stylesheet_nodes(document: &Document) -> Vec<usize> {
    let root = document.root_index();
    let Some(root_node) = document.get_node(root) else {
        return Vec::new(); // cov:ignore: A Document always retains its initialized root node.
    };
    let mut pending: Vec<usize> = root_node.children.iter().rev().copied().collect();
    let mut nodes = Vec::new();
    while let Some(index) = pending.pop() {
        let Some(node) = document.get_node(index) else {
            continue; // cov:ignore: Child indices are created and retained by the same append-only document arena.
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
            // HTML "update a style block": a `type` other than empty or
            // `text/css` (ASCII case-insensitive) creates no style sheet.
            let is_css = document
                .element_attribute(index, "type")
                .is_none_or(|kind| kind.is_empty() || kind.eq_ignore_ascii_case("text/css"));
            if is_css {
                nodes.push(index);
            }
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
    )?; // cov:ignore: The fresh realm's built-in object prototype accepts these fixed list members.
    let list = indexed_object(context, prototype, StyleSheetListSource)?;
    with_state(context, |state| state.style_sheet_list = Some(list.clone()))?;
    Ok(list.into())
}

/// `HTMLStyleElement.sheet` (HTML §4.2.6, CSSOM §6.1): the element's
/// `CSSStyleSheet` while it is in the document, otherwise `null`.
fn style_element_sheet(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = super::webidl::this_element(this, context)?;
    let is_style = with_state(context, |state| {
        let document = state.host.document();
        document.get_node(index).and_then(|node| node.tag_name()) == Some("style")
            && matches!(
                document.element_namespace_uri(index),
                Some("http://www.w3.org/1999/xhtml") | Some("http://www.w3.org/2000/svg")
            )
    })?;
    if !is_style {
        return Err(JsNativeError::typ()
            .with_message("'this' is not an HTMLStyleElement or SVGStyleElement")
            .into());
    }
    let connected = with_state(context, |state| {
        stylesheet_nodes(state.host.document()).contains(&index)
    })?;
    if connected {
        Ok(style_sheet_object(context, index)?.into())
    } else {
        Ok(JsValue::null())
    }
}

pub(crate) const HTML_STYLE_ELEMENT_MEMBERS: super::interfaces::Members =
    super::interfaces::Members {
        getters: &[("sheet", style_element_sheet)],
        accessors: &[],
        methods: &[],
    };

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
    /// Whether the object was built as a `CSSPageRule`.
    #[unsafe_ignore_trace]
    is_page: bool,
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

fn define_accessor(
    object: &JsObject,
    name: &str,
    getter: boa_engine::native_function::NativeFunctionPointer,
    setter: boa_engine::native_function::NativeFunctionPointer,
    context: &mut Context,
) -> JsResult<()> {
    let get = function(context, &format!("get {name}"), 0, getter)?;
    let set = function(context, &format!("set {name}"), 1, setter)?;
    let descriptor = PropertyDescriptor::builder()
        .get(get)
        .set(set)
        .enumerable(true)
        .configurable(true)
        .build();
    object.define_property_or_throw(JsString::from(name), descriptor, context)?;
    Ok(())
}

fn define_method(
    object: &JsObject,
    name: &str,
    length: usize,
    method: boa_engine::native_function::NativeFunctionPointer,
    context: &mut Context,
) -> JsResult<()> {
    let function = function(context, name, length, method)?;
    let descriptor = PropertyDescriptor::builder()
        .value(function)
        .writable(true)
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
    // `rules` is the legacy alias of `cssRules` (CSSOM §6.1.2.1).
    define_getter(&object, "rules", style_sheet_rules, context)?;
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
    )?; // cov:ignore: The newly created ordinary object is extensible and has no conflicting own symbol.
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
    )?; // cov:ignore: The fresh realm's built-in object prototype accepts these fixed list members.
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
    let is_page = rule_info(context, style_element, rule_index)
        .is_ok_and(|(_, rule)| is_page_prelude(&rule.prelude));
    let cached = with_state(context, |state| state.css_style_rules.get(&key).cloned())?;
    if let Some(existing) = cached {
        let same_kind = existing
            .downcast_ref::<CssStyleRuleData>()
            .is_some_and(|data| data.is_page == is_page);
        if same_kind {
            return Ok(existing);
        }
        // The sheet text was replaced and the rule at this index changed
        // between a style rule and a page rule: build a fresh object of the
        // right interface, with a fresh declaration object.
        with_state(context, |state| {
            state.css_style_rules.remove(&key);
            state.css_rule_styles.remove(&key);
        })?;
    }
    let prototype = context.intrinsics().constructors().object().prototype();
    let object = JsObject::from_proto_and_data(
        Some(prototype),
        CssStyleRuleData {
            style_element,
            rule_index,
            is_page,
        },
    );
    define_getter(&object, "cssText", css_rule_text, context)?;
    if is_page {
        define_accessor(
            &object,
            "selectorText",
            css_rule_selector,
            page_rule_selector_set,
            context,
        )?;
    } else {
        define_getter(&object, "selectorText", css_rule_selector, context)?;
    }
    define_getter(&object, "style", css_rule_style, context)?;
    if is_page {
        define_method(&object, "insertRule", 1, page_rule_insert_rule, context)?;
        define_method(&object, "deleteRule", 1, page_rule_delete_rule, context)?;
        let prototype = list_prototype(
            context,
            "CSSRuleList",
            margin_rule_list_length,
            margin_rule_list_item,
        )?; // cov:ignore: The fresh realm's built-in object prototype accepts these fixed list members.
        let list = indexed_object(
            context,
            prototype,
            MarginRuleListSource {
                style_element,
                rule_index,
            },
        )?;
        let descriptor = PropertyDescriptor::builder()
            .value(list)
            .writable(false)
            .enumerable(true)
            .configurable(true)
            .build();
        object.define_property_or_throw(js_string!("cssRules"), descriptor, context)?;
    }
    let tag = if is_page {
        "CSSPageRule"
    } else {
        "CSSStyleRule"
    };
    let to_string_tag = PropertyDescriptor::builder()
        .value(JsString::from(tag))
        .writable(false)
        .enumerable(false)
        .configurable(true)
        .build();
    object.define_property_or_throw(
        boa_engine::JsSymbol::to_string_tag(),
        to_string_tag,
        context,
    )?; // cov:ignore: The newly created ordinary object is extensible and has no conflicting own symbol.
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
    // A `CSSPageRule`'s selector is the page selector list after `@page`.
    if let Some(selector) = page_selector_text(&rule.prelude) {
        return Ok(JsValue::from(JsString::from(selector)));
    }
    Ok(JsValue::from(JsString::from(rule.prelude)))
}

fn css_rule_style(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (style_element, rule_index) = css_rule_data(this)?;
    let (_, rule) = rule_info(context, style_element, rule_index)?;
    if (rule.prelude.starts_with('@') && !is_page_prelude(&rule.prelude)) || rule.body.is_none() {
        return Ok(JsValue::undefined());
    }
    let key = (style_element, rule_index);
    if let Some(existing) = with_state(context, |state| state.css_rule_styles.get(&key).cloned())? {
        return Ok(existing.into());
    }
    let style = super::style::stylesheet_style_object(context, style_element, rule_index)?;
    if is_page_prelude(&rule.prelude) {
        super::style::install_page_descriptor_accessors(context, &style)?;
    }
    with_state(context, |state| {
        state.css_rule_styles.insert(key, style.clone())
    })?;
    Ok(style.into())
}

/// `CSSPageRule.selectorText`, on setting (CSSOM §6.4.3): the value is
/// parsed as a page selector list and, when valid, replaces the rule's
/// prelude; an invalid value changes nothing.
fn page_rule_selector_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (style_element, rule_index) = css_rule_data(this)?;
    let selector = dom_string(args, 0, context)?;
    if !raikiri_style::is_valid_page_selector(&selector) {
        return Ok(JsValue::undefined());
    }
    let (_, rule) = rule_info(context, style_element, rule_index)?;
    let Some(body) = rule.body.filter(|_| is_page_prelude(&rule.prelude)) else {
        return Ok(JsValue::undefined());
    };
    // The prelude runs from the rule's start up to the `{` that opens its
    // body, one byte before the body.
    let prelude = rule.source.start..body.start - 1;
    let selector = page_selector_text(&format!("@page {selector}")).unwrap_or_default();
    let replacement = if selector.is_empty() {
        "@page ".to_owned()
    } else {
        format!("@page {selector} ")
    };
    splice_rule_body(context, style_element, 0, prelude, &replacement)?;
    Ok(JsValue::undefined())
}

pub(crate) fn parent_rule(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
) -> JsResult<JsValue> {
    Ok(css_style_rule_object(context, style_element, rule_index)?.into())
}

/// Whether the top-level rule `rule_index` of `style_element`'s sheet is an
/// `@page` rule.
pub(crate) fn is_page_rule(document: &Document, style_element: usize, rule_index: usize) -> bool {
    let text = stylesheet_text(document, style_element);
    parse_css_rules(&text)
        .get(rule_index)
        .is_some_and(|rule| is_page_prelude(&rule.prelude))
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
        if is_page_prelude(&rule.prelude) {
            // The declaration view of an `@page` rule never sees its margin
            // rules, so carry them over when its declarations are rewritten.
            let old_body = &text[body_range.clone()];
            for range in nested_at_rule_ranges(old_body) {
                updated.push(' ');
                updated.push_str(&old_body[range]);
            }
        }
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

/// Replace `range` (relative to the rule body starting at `body_start`) with
/// `replacement`, then mark the document dirty.
fn splice_rule_body(
    context: &mut Context,
    style_element: usize,
    body_start: usize,
    range: Range<usize>,
    replacement: &str,
) -> JsResult<()> {
    let result = with_state(context, |state| {
        let mut text = stylesheet_text(state.host.document(), style_element);
        text.replace_range(
            body_start + range.start..body_start + range.end,
            replacement,
        );
        state
            .host
            .document_mut()
            .set_element_text_content(style_element, text)
    })?;
    result.map_err(|_| {
        JsError::from(JsNativeError::typ().with_message("unable to update stylesheet rule"))
    })?;
    super::node::mark_dirty(context)
}

/// The `@page` rule `this` names, with its body range and the ranges of its
/// margin rules relative to that body.
fn page_rule_parts(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<(usize, Range<usize>, Vec<Range<usize>>)> {
    let (style_element, rule_index) = css_rule_data(this)?;
    let (text, rule) = rule_info(context, style_element, rule_index)?;
    let body = rule
        .body
        .filter(|_| is_page_prelude(&rule.prelude))
        .ok_or_else(|| JsNativeError::typ().with_message("'this' is not a CSSPageRule"))?;
    let margins = nested_at_rule_ranges(&text[body.clone()]);
    Ok((style_element, body, margins))
}

/// `CSSPageRule.insertRule(rule, index = 0)` (CSSOM §6.4.3, "insert a CSS
/// rule" on the page rule's child margin rules).
fn page_rule_insert_rule(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (style_element, body, margins) = page_rule_parts(this, context)?;
    let rule = dom_string(args, 0, context)?;
    let index = match args.get(1) {
        Some(value) if !value.is_undefined() => value.to_u32(context)? as usize,
        _ => 0,
    };
    if index > margins.len() {
        return Err(throw_dom_exception(
            context,
            "IndexSizeError",
            "rule index is out of range",
        ));
    }
    let Some(rule) = parse_margin_rule(&rule) else {
        return Err(throw_dom_exception(
            context,
            "SyntaxError",
            "only a margin rule can be inserted into a page rule",
        ));
    };
    let (at, inserted) = match margins.get(index) {
        Some(range) => (range.start, format!(" {rule} ")),
        // At the end of the body the last declaration may lack its `;`;
        // close it so the rule is not read as part of that value.
        None => (body.end - body.start, format!("; {rule} ")),
    };
    splice_rule_body(context, style_element, body.start, at..at, &inserted)?;
    Ok(JsValue::from(index as u32))
}

/// `CSSPageRule.deleteRule(index)` (CSSOM §6.4.3, "remove a CSS rule").
fn page_rule_delete_rule(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (style_element, body, margins) = page_rule_parts(this, context)?;
    let index = arg_unsigned_long(args, 0, context)?;
    let Some(range) = margins.get(index).cloned() else {
        return Err(throw_dom_exception(
            context,
            "IndexSizeError",
            "rule index is out of range",
        ));
    };
    splice_rule_body(context, style_element, body.start, range, "")?;
    Ok(JsValue::undefined())
}

/// The live `cssRules` of a `CSSPageRule`: its nested margin rules, in
/// source order.
struct MarginRuleListSource {
    style_element: usize,
    rule_index: usize,
}

impl IndexedSource for MarginRuleListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        Ok(margin_rule_texts(context, self.style_element, self.rule_index)?.len())
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        if index >= self.length(context)? {
            return Ok(None);
        }
        Ok(Some(
            margin_rule_object(context, self.style_element, self.rule_index, index)?.into(),
        ))
    }
}

/// The `CSSMarginRule` for margin rule `margin_index` of the page rule
/// `rule_index`, the same object each time it is asked for.
pub(crate) fn margin_rule_object(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
    margin_index: usize,
) -> JsResult<JsObject> {
    let key = (style_element, rule_index, margin_index);
    if let Some(existing) = with_state(context, |state| state.css_margin_rules.get(&key).cloned())?
    {
        return Ok(existing);
    }
    let object = JsObject::from_proto_and_data(
        Some(context.intrinsics().constructors().object().prototype()),
        MarginRuleData {
            style_element,
            rule_index,
            margin_index,
        },
    );
    define_getter(&object, "name", margin_rule_name, context)?;
    define_getter(&object, "cssText", margin_rule_text, context)?;
    define_getter(&object, "style", margin_rule_style, context)?;
    super::interfaces::set_to_string_tag(&object, "CSSMarginRule", context)?;
    with_state(context, |state| {
        state.css_margin_rules.insert(key, object.clone())
    })?;
    Ok(object)
}

/// `CSSMarginRule.style`: a declaration object over the margin rule's own
/// block (CSSOM §6.4.4, `CSSMarginDescriptors`).
fn margin_rule_style(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = margin_rule_data(this)?;
    if let Some(existing) = with_state(context, |state| {
        state.css_margin_rule_styles.get(&key).cloned()
    })? {
        return Ok(existing.into());
    }
    let (style_element, rule_index, margin_index) = key;
    let style =
        super::style::margin_rule_style_object(context, style_element, rule_index, margin_index)?;
    with_state(context, |state| {
        state.css_margin_rule_styles.insert(key, style.clone())
    })?;
    Ok(style.into())
}

/// The range of a margin rule's declarations, relative to `margin_rule`:
/// the contents of its block, up to its closing `}` when it has one.
fn margin_rule_body_range(margin_rule: &str) -> Option<Range<usize>> {
    let mut input = ParserInput::new(margin_rule);
    let mut parser = Parser::new(&mut input);
    // `nested_at_rule_ranges` only reports an at-keyword followed by a block.
    if !matches!(parser.next(), Ok(Token::AtKeyword(_)))
        || !matches!(parser.next(), Ok(Token::CurlyBracketBlock))
    {
        return None; // cov:ignore: Every margin rule range starts with an at-keyword and its block.
    }
    let start = parser.position().byte_index();
    let end = parser
        .parse_nested_block(|block| {
            while block.next_including_whitespace_and_comments().is_ok() {}
            Ok::<_, ParseError<'_, ()>>(block.position().byte_index())
        })
        .ok()?; // cov:ignore: The closure never returns an error.
    Some(start..end.max(start))
}

/// The range of the declarations of margin rule `margin_index` in the page
/// rule `rule_index` of the sheet `text`.
fn margin_body_range(text: &str, rule_index: usize, margin_index: usize) -> Option<Range<usize>> {
    let rule = parse_css_rules(text).get(rule_index)?.clone();
    let body = rule.body.filter(|_| is_page_prelude(&rule.prelude))?;
    let margin = nested_at_rule_ranges(&text[body.clone()])
        .into_iter()
        .nth(margin_index)?;
    let start = body.start + margin.start;
    let inner = margin_rule_body_range(&text[start..body.start + margin.end])?;
    Some(start + inner.start..start + inner.end)
}

/// The declaration text of margin rule `margin_index` in the page rule
/// `rule_index`, or `None` once that margin rule is gone.
pub(crate) fn stylesheet_margin_rule_body(
    document: &Document,
    style_element: usize,
    rule_index: usize,
    margin_index: usize,
) -> Option<String> {
    let text = stylesheet_text(document, style_element);
    let range = margin_body_range(&text, rule_index, margin_index)?;
    Some(text[range].to_owned())
}

/// Replaces the declarations of margin rule `margin_index` in the page rule
/// `rule_index` with `body`.
pub(crate) fn write_stylesheet_margin_rule_body(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
    margin_index: usize,
    body: &str,
) -> JsResult<()> {
    let range = with_state(context, |state| {
        let text = stylesheet_text(state.host.document(), style_element);
        margin_body_range(&text, rule_index, margin_index)
    })?
    .ok_or_else(|| JsNativeError::typ().with_message("CSS rule no longer exists"))?;
    splice_rule_body(context, style_element, 0, range, body)
}

#[derive(Trace, Finalize, JsData)]
struct MarginRuleData {
    #[unsafe_ignore_trace]
    style_element: usize,
    #[unsafe_ignore_trace]
    rule_index: usize,
    #[unsafe_ignore_trace]
    margin_index: usize,
}

/// The source text of each margin rule nested in the page rule
/// `rule_index`; empty once that rule is gone or no longer an `@page` rule.
fn margin_rule_texts(
    context: &mut Context,
    style_element: usize,
    rule_index: usize,
) -> JsResult<Vec<String>> {
    let Ok((text, rule)) = rule_info(context, style_element, rule_index) else {
        return Ok(Vec::new());
    };
    let Some(body) = rule.body.filter(|_| is_page_prelude(&rule.prelude)) else {
        return Ok(Vec::new());
    };
    let body = &text[body];
    Ok(nested_at_rule_ranges(body)
        .into_iter()
        .map(|range| body[range].to_owned())
        .collect())
}

fn margin_rule_list_length(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<MarginRuleListSource>(this, context, "CSSRuleList")?;
    Ok(JsValue::from(source.length(context)? as u32))
}

fn margin_rule_list_item(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_indexed::<MarginRuleListSource>(this, context, "CSSRuleList")?;
    let index = arg_unsigned_long(args, 0, context)?;
    Ok(source.item(index, context)?.unwrap_or_else(JsValue::null))
}

/// The sheet, page rule index, and margin rule index `this` names.
fn margin_rule_data(this: &JsValue) -> JsResult<(usize, usize, usize)> {
    this.as_object()
        .and_then(|object| {
            object
                .downcast_ref::<MarginRuleData>()
                .map(|data| (data.style_element, data.rule_index, data.margin_index))
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a CSSMarginRule")
                .into()
        })
}

/// The current text of the margin rule `this` names, or `""` once it is gone.
fn this_margin_rule_text(this: &JsValue, context: &mut Context) -> JsResult<String> {
    let (style_element, rule_index, margin_index) = margin_rule_data(this)?;
    Ok(margin_rule_texts(context, style_element, rule_index)?
        .into_iter()
        .nth(margin_index)
        .unwrap_or_default())
}

/// `CSSMarginRule.name`: the margin rule's at-keyword, ASCII-lowercased.
fn margin_rule_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = this_margin_rule_text(this, context)?;
    let mut input = ParserInput::new(&text);
    let mut parser = Parser::new(&mut input);
    let name = match parser.next() {
        Ok(Token::AtKeyword(name)) => name.to_ascii_lowercase(),
        _ => String::new(),
    };
    Ok(JsValue::from(JsString::from(name)))
}

fn margin_rule_text(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(JsString::from(this_margin_rule_text(
        this, context,
    )?)))
}

#[cfg(test)]
mod tests;
