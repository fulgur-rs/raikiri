//! Inline style, computed style, `CSS.supports`, and geometry members.

use boa_engine::object::builtins::JsProxyBuilder;
use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction,
    Trace, js_string,
};
use cssparser::{Parser, ParserInput};
use raikiri_dom::NodeKind;

use super::host::DomRect;
use super::interfaces::{Members, closure_function, function};
use super::node::mark_dirty;
use super::webidl::{arg_node, dom_string, host_failure, this_element, with_state};

// ---- inline style text ---------------------------------------------------

#[derive(Default)]
struct RawStyleDeclarationParser;

impl<'i> cssparser::DeclarationParser<'i> for RawStyleDeclarationParser {
    type Declaration = (String, String);
    type Error = ();
    fn parse_value<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut cssparser::Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, cssparser::ParseError<'i, Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        Ok((
            name.to_string(),
            input.slice(start..input.position()).trim().to_owned(),
        ))
    }
}

impl<'i> cssparser::AtRuleParser<'i> for RawStyleDeclarationParser {
    type Prelude = ();
    type AtRule = (String, String);
    type Error = ();
}

impl<'i> cssparser::QualifiedRuleParser<'i> for RawStyleDeclarationParser {
    type Prelude = ();
    type QualifiedRule = (String, String);
    type Error = ();
}

impl<'i> cssparser::RuleBodyItemParser<'i, (String, String), ()> for RawStyleDeclarationParser {
    fn parse_qualified(&self) -> bool {
        false
    }

    fn parse_declarations(&self) -> bool {
        true
    }
}

fn parse_inline_style(source: &str) -> Vec<(String, String)> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut declarations = RawStyleDeclarationParser;
    cssparser::RuleBodyParser::new(&mut parser, &mut declarations)
        .flatten()
        .collect()
}

fn strip_important(value: &str) -> &str {
    let trimmed = value.trim_end();
    let lower = trimmed.to_ascii_lowercase();
    lower
        .rfind("!important")
        .filter(|&start| lower[start + "!important".len()..].trim().is_empty())
        .map_or(trimmed, |start| trimmed[..start].trim_end())
}

fn same_property(declared: &str, property: &str) -> bool {
    if property.starts_with("--") {
        declared == property
    } else {
        declared.eq_ignore_ascii_case(property)
    }
}

/// The last declared value of `property` in a `style` attribute, without `!important`.
pub(crate) fn inline_style_value(style_attr: Option<&str>, property: &str) -> String {
    let Some(style) = style_attr else {
        return String::new();
    };
    parse_inline_style(style)
        .into_iter()
        .rev()
        .find(|(name, _)| same_property(name, property))
        .map(|(_, value)| strip_important(&value).trim().to_owned())
        .unwrap_or_default()
}

/// A `style` attribute with `property` replaced by `value` (removed when blank).
pub(crate) fn with_inline_style_property(
    style_attr: Option<&str>,
    property: &str,
    value: &str,
) -> String {
    let property = property.trim();
    let mut declarations = style_attr.map(parse_inline_style).unwrap_or_default();
    if property.is_empty() {
        return declarations
            .into_iter()
            .map(|(n, v)| format!("{n}: {v};"))
            .collect::<Vec<_>>()
            .join(" ");
    }
    declarations.retain(|(name, _)| !same_property(name, property));
    if !value.trim().is_empty() {
        declarations.push((property.to_owned(), value.to_owned()));
    }
    declarations
        .into_iter()
        .map(|(n, v)| format!("{n}: {v};"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// CSSOM "camel-cased attribute" -> CSS property name (CSSOM §7.3): every
/// ASCII uppercase letter becomes a `-` followed by its lowercase form.
fn css_name(property: &str) -> String {
    let mut out = String::with_capacity(property.len() + 4);
    for c in property.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

// ---- flush and geometry ---------------------------------------------------

/// Flush the host when a mutation happened since the last successful flush.
pub(crate) fn ensure_flushed(context: &mut Context) -> JsResult<()> {
    let result = with_state(context, |s| {
        if !s.dirty {
            return Ok(());
        }
        let result = s.host.flush();
        if result.is_ok() {
            s.dirty = false;
        }
        result
    })?;
    result.map_err(|error| host_failure(context, error))
}

fn border_box(context: &mut Context, index: usize) -> JsResult<DomRect> {
    ensure_flushed(context)?;
    let geometry = with_state(context, |s| s.host.box_geometry(index))?;
    match geometry {
        Ok(geometry) => Ok(geometry.map(|g| g.border_box).unwrap_or_default()),
        Err(error) => Err(host_failure(context, error)),
    }
}

fn offset_height(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    Ok(JsValue::from(border_box(context, index)?.height.round()))
}

fn offset_width(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    Ok(JsValue::from(border_box(context, index)?.width.round()))
}

/// `Element.getBoundingClientRect` (CSSOM View §5): the border box, in the
/// coordinate space this runtime uses (relative to the initial containing
/// block; there is no scroll or transform to account for yet).
pub(crate) fn get_bounding_client_rect(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let r = border_box(context, index)?;
    Ok(ObjectInitializer::new(context)
        .property(js_string!("x"), r.left, Attribute::all())
        .property(js_string!("left"), r.left, Attribute::all())
        .property(js_string!("y"), r.top, Attribute::all())
        .property(js_string!("top"), r.top, Attribute::all())
        .property(js_string!("right"), r.right, Attribute::all())
        .property(js_string!("bottom"), r.bottom, Attribute::all())
        .property(js_string!("width"), r.width, Attribute::all())
        .property(js_string!("height"), r.height, Attribute::all())
        .build()
        .into())
}

// ---- inline `style` object ------------------------------------------------

/// Proxy target of an element's `style` object: the arena index it reads and
/// writes, and whether it is a read-only computed-style view.
#[derive(Debug, Trace, Finalize, JsData)]
struct StyleTarget {
    #[unsafe_ignore_trace]
    index: usize,
    #[unsafe_ignore_trace]
    computed: bool,
}

// cov:ignore: a proxy's traps are internal (never exposed to script) and
// `style_object` always builds a `StyleTarget`, so no test can reach this;
// it exists only so a future proxy misuse throws instead of panicking.
fn proxy_target_error() -> JsError {
    JsNativeError::typ()
        .with_message("style proxy called on an unexpected target")
        .into()
}

/// The target object plus the `(index, computed)` pair it carries, or a
/// `TypeError` when `value` is not one (a trap should never see this, but a
/// binding must never panic on an unexpected argument).
fn style_target(value: &JsValue) -> JsResult<(JsObject, usize, bool)> {
    let object = value.as_object().ok_or_else(proxy_target_error)?;
    let (index, computed) = object
        .downcast_ref::<StyleTarget>()
        .map(|t| (t.index, t.computed))
        .ok_or_else(proxy_target_error)?;
    Ok((object.clone(), index, computed))
}

fn read_inline(context: &mut Context, index: usize, property: &str) -> JsResult<String> {
    with_state(context, |s| {
        inline_style_value(
            s.host.document().element_attribute(index, "style"),
            property,
        )
    })
}

fn write_inline(context: &mut Context, index: usize, property: &str, value: &str) -> JsResult<()> {
    if property.trim().is_empty() {
        return Ok(());
    }
    with_state(context, |s| {
        let next = with_inline_style_property(
            s.host.document().element_attribute(index, "style"),
            property,
            value,
        );
        s.host
            .document_mut()
            .set_element_inline_style(index, Some(next.into()));
    })?;
    mark_dirty(context)
}

/// The computed value of `property`, or `None` for an unsupported property
/// name (checked before any flush, so an unsupported name never triggers one).
fn read_computed(context: &mut Context, index: usize, property: &str) -> JsResult<Option<String>> {
    if raikiri_style::ComputedProperty::from_name(property).is_none() {
        return Ok(None);
    }
    ensure_flushed(context)?;
    let value = with_state(context, |s| s.host.computed_value(index, property))?;
    value.map_err(|error| host_failure(context, error))
}

fn style_object(context: &mut Context, index: usize, computed: bool) -> JsResult<JsObject> {
    let target = JsObject::from_proto_and_data(
        Some(context.intrinsics().constructors().object().prototype()),
        StyleTarget { index, computed },
    );
    let get_value = NativeFunction::from_copy_closure(move |_, args, ctx| {
        let name = dom_string(args, 0, ctx)?;
        let value = if computed {
            read_computed(ctx, index, &name)?.unwrap_or_default()
        } else {
            read_inline(ctx, index, &name)?
        };
        Ok(JsValue::from(JsString::from(value)))
    });
    let get_value = closure_function(context, "getPropertyValue", 1, get_value)?;
    target.set(js_string!("getPropertyValue"), get_value, true, context)?;
    if !computed {
        let set_value = NativeFunction::from_copy_closure(move |_, args, ctx| {
            let name = dom_string(args, 0, ctx)?;
            let value = dom_string(args, 1, ctx)?;
            write_inline(ctx, index, &name, &value)?;
            Ok(JsValue::undefined())
        });
        let set_value = closure_function(context, "setProperty", 2, set_value)?;
        let remove_value = NativeFunction::from_copy_closure(move |_, args, ctx| {
            let name = dom_string(args, 0, ctx)?;
            let previous = read_inline(ctx, index, &name)?;
            write_inline(ctx, index, &name, "")?;
            Ok(JsValue::from(JsString::from(previous)))
        });
        let remove_value = closure_function(context, "removeProperty", 1, remove_value)?;
        target.set(js_string!("setProperty"), set_value, true, context)?;
        target.set(js_string!("removeProperty"), remove_value, true, context)?;
    }
    let proxy = JsProxyBuilder::new(target)
        .get(style_get_trap)
        .set(style_set_trap)
        .has(style_has_trap)
        .build(context)?;
    Ok(proxy.into())
}

fn string_key(key: &JsValue) -> Option<String> {
    key.as_string().map(|s| s.to_std_string_escaped())
}

/// Proxy `[[Get]]` trap (`« target, key, receiver »`): a property the target
/// already has — one of the methods `style_object` defined, or one inherited
/// from `Object.prototype` (`toString`, `valueOf`, `hasOwnProperty`, ...) —
/// wins; otherwise the key is read as a camelCase or dashed style property
/// name. Checking the whole prototype chain, not just own properties, is
/// what lets `String(el.style)`, `'' + el.style`, and
/// `el.style.hasOwnProperty(...)` work like a normal object instead of
/// throwing `TypeError` on a non-callable `""`.
fn style_get_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target_value = args.first().cloned().unwrap_or_default();
    let (target, index, computed) = style_target(&target_value)?;
    let key = args.get(1).cloned().unwrap_or_default();
    let property_key = key.to_property_key(context)?;
    if target.has_property(property_key.clone(), context)? {
        return target.get(property_key, context);
    }
    let Some(name) = string_key(&key) else {
        return Ok(JsValue::undefined());
    };
    let property = css_name(&name);
    let value = if computed {
        read_computed(context, index, &property)?.unwrap_or_default()
    } else {
        read_inline(context, index, &property)?
    };
    Ok(JsValue::from(JsString::from(value)))
}

/// Proxy `[[Set]]` trap (`« target, key, value, receiver »`): a property
/// write updates the inline `style` attribute; computed style is read-only.
fn style_set_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target_value = args.first().cloned().unwrap_or_default();
    let (_, index, computed) = style_target(&target_value)?;
    if computed {
        return Ok(JsValue::from(false));
    }
    let Some(name) = args.get(1).and_then(string_key) else {
        return Ok(JsValue::from(false));
    };
    let value = args
        .get(2)
        .cloned()
        .unwrap_or_default()
        .to_string(context)?
        .to_std_string_escaped();
    write_inline(context, index, &css_name(&name), &value)?;
    Ok(JsValue::from(true))
}

/// Proxy `[[HasProperty]]` trap (`« target, key »`): an own property of the
/// target wins; a computed style otherwise reports a supported property name
/// as present, matching a real `CSSStyleDeclaration`'s indexed-property view.
fn style_has_trap(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target_value = args.first().cloned().unwrap_or_default();
    let (target, index, computed) = style_target(&target_value)?;
    let key = args.get(1).cloned().unwrap_or_default();
    let property_key = key.to_property_key(context)?;
    if target.has_property(property_key, context)? {
        return Ok(JsValue::from(true));
    }
    if !computed {
        return Ok(JsValue::from(false));
    }
    let Some(name) = string_key(&key) else {
        return Ok(JsValue::from(false));
    };
    Ok(JsValue::from(
        read_computed(context, index, &css_name(&name))?.is_some(),
    ))
}

fn style(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    if let Some(existing) = with_state(context, |s| s.style_objects.get(&index).cloned())? {
        return Ok(existing.into());
    }
    let object = style_object(context, index, false)?;
    with_state(context, |s| s.style_objects.insert(index, object.clone()))?;
    Ok(object.into())
}

fn get_computed_style(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = arg_node(args, 0, context)?;
    let is_element = with_state(context, |s| {
        s.host.document().get_node(index).map(|n| n.kind()) == Some(NodeKind::Element)
    })?;
    if !is_element {
        return Err(JsNativeError::typ()
            .with_message("argument is not an Element")
            .into());
    }
    Ok(style_object(context, index, true)?.into())
}

fn css_supports(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let property = dom_string(args, 0, context)?;
    let value = dom_string(args, 1, context)?;
    let mut input = ParserInput::new(&value);
    let mut parser = Parser::new(&mut input);
    let parsed =
        parser
            .parse_entirely(
                |input| -> Result<
                    raikiri_style::property::PropertyValue,
                    cssparser::ParseError<'_, ()>,
                > {
                    raikiri_style::property::parse_value(&property, input)
                        .ok_or_else(|| input.new_custom_error(()))
                },
            )
            .is_ok();
    let wide = ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|k| value.trim().eq_ignore_ascii_case(k));
    Ok(JsValue::from(
        parsed || (wide && raikiri_style::property::is_supported_property_name(&property)),
    ))
}

pub(crate) const HTML_ELEMENT_MEMBERS: Members = Members {
    getters: &[
        ("style", style),
        ("offsetHeight", offset_height),
        ("offsetWidth", offset_width),
    ],
    accessors: &[],
    methods: &[],
};

/// `getComputedStyle` and the `CSS` namespace object.
pub(crate) fn install_globals(context: &mut Context) -> JsResult<()> {
    let attr = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    let gcs = function(context, "getComputedStyle", 1, get_computed_style)?;
    context.register_global_property(js_string!("getComputedStyle"), gcs, attr)?;
    let supports = function(context, "supports", 2, css_supports)?;
    let css = ObjectInitializer::new(context)
        .property(js_string!("supports"), supports, attr)
        .build();
    context.register_global_property(js_string!("CSS"), css, attr)?;
    Ok(())
}

#[cfg(test)]
mod tests;
