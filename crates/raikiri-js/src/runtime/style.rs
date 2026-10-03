//! Inline style, computed style, `CSS.supports`, and geometry members.

use std::rc::Rc;

use boa_engine::object::builtins::JsFunction;
use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction, js_string,
};
use cssparser::{Parser, ParserInput};
use raikiri_dom::NodeKind;

use super::indexed::{IndexedSource, indexed_object, this_indexed};
use super::interfaces::{Members, closure_function, function, protos};
use super::node::mark_dirty;
use super::webidl::{
    arg_node, arg_unsigned_long, dom_string, host_failure, host_failure_with_message, this_element,
    throw_dom_exception, with_state,
};

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

/// CSSOM §6.7 step 2.1 validated name: `None` for an empty name or an
/// unsupported non-custom name (which the caller drops); otherwise the name
/// to store and enumerate -- ASCII-lowercased for non-custom names, exactly
/// as given (trimmed) for custom names.
fn validated_property_name(raw_name: &str) -> Option<String> {
    let trimmed = raw_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("--") {
        if trimmed.len() <= 2 {
            return None;
        }
        return Some(trimmed.to_owned());
    }
    let lower = trimmed.to_ascii_lowercase();
    if !raikiri_style::property::is_supported_property_name(&lower) {
        return None;
    }
    Some(lower)
}

/// One validated inline declaration: `value` without any trailing
/// `!important`, `important` carrying that flag separately.
struct ValidatedDeclaration {
    name: String,
    value: String,
    important: bool,
}

/// The validated view of a `style` attribute (CSSOM §6.7): raw declarations
/// parsed with [`parse_inline_style`], then each dropped when its name is
/// empty or unsupported, its value is empty, or its value does not parse
/// for its name (via [`declaration_value`); a trailing `!important` in the
/// raw text becomes `important`, not part of `value`). Duplicates collapse
/// to the last occurrence (removed then re-pushed, so the survivor sits at
/// the last position), matching [`with_inline_style_property`]'s write path.
/// Shorthands are kept as specified (a single entry, not expanded into
/// longhands); expanding `margin: 0` into four longhands for reads remains
/// future work.
fn validated_inline_declarations(style_attr: Option<&str>) -> Vec<ValidatedDeclaration> {
    let Some(style) = style_attr else {
        return Vec::new();
    };
    let mut out: Vec<ValidatedDeclaration> = Vec::new();
    for (raw_name, raw_value) in parse_inline_style(style) {
        let Some(name) = validated_property_name(&raw_name) else {
            continue;
        };
        let important = strip_important(&raw_value) != raw_value.trim_end();
        let value_part = strip_important(&raw_value).trim();
        if value_part.is_empty() {
            continue;
        }
        let Some(stored) = declaration_value(&name, value_part) else {
            continue;
        };
        if stored.trim().is_empty() {
            continue;
        }
        out.retain(|d| !same_property(&d.name, &name));
        out.push(ValidatedDeclaration {
            name,
            value: stored,
            important,
        });
    }
    out
}

/// CSSOM shorthand names among
/// [`raikiri_style::property::supported_property_names`]: excluded from the
/// computed `length`/`item` view (longhands only, lexicographic). Kept as a
/// plain list (rather than derived from `expand_shorthand_into`, which is
/// `pub(crate)` to `raikiri-style`) so the computed view stays a pure
/// name filter over the already-sorted supported list.
pub(crate) fn is_computed_shorthand(name: &str) -> bool {
    matches!(
        name,
        "background"
            | "border"
            | "border-color"
            | "border-style"
            | "border-width"
            | "border-right"
            | "border-radius"
            | "columns"
            | "flex"
            | "flex-flow"
            | "font"
            | "gap"
            | "grid"
            | "grid-area"
            | "grid-column"
            | "grid-row"
            | "grid-gap"
            | "margin"
            | "margin-inline"
            | "margin-block"
            | "padding"
            | "padding-inline"
            | "padding-block"
            | "overflow"
            | "outline"
            | "text-decoration"
            | "text-emphasis"
            | "text-spacing"
            | "place-content"
            | "place-items"
            | "place-self"
    )
}

/// The computed `length`/`item` view (CSSOM §6.7): supported longhands only,
/// lexicographic. [`raikiri_style::property::supported_property_names`] is
/// already lowercase and sorted, so filtering out [`is_computed_shorthand`]
/// preserves both properties.
pub(crate) fn computed_longhand_names() -> Vec<&'static str> {
    raikiri_style::property::supported_property_names()
        .iter()
        .copied()
        .filter(|name| !is_computed_shorthand(name))
        .collect()
}

/// The last declared value of `property` in a `style` attribute, without `!important`.
/// Reads through the validated view ([`validated_inline_declarations`]), so
/// an unsupported name or an unparseable value reads as `""` rather than
/// the raw text.
pub(crate) fn inline_style_value(style_attr: Option<&str>, property: &str) -> String {
    let trimmed = property.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lookup = if trimmed.starts_with("--") {
        if trimmed.len() <= 2 {
            return String::new();
        }
        trimmed.to_owned()
    } else {
        trimmed.to_ascii_lowercase()
    };
    validated_inline_declarations(style_attr)
        .into_iter()
        .rev()
        .find(|d| same_property(&d.name, &lookup))
        .map(|d| d.value)
        .unwrap_or_default()
}

/// Serializes `declarations` back to `style` attribute text (`"prop:
/// value;"` per declaration, space-joined) -- the inverse of
/// [`parse_inline_style`], and CSSOM's "serialize a CSS declaration
/// block". Shared by [`with_inline_style_property`]'s raw-preserving
/// "replace one declaration" and the validated `cssText` getter/serializer
/// below.
fn serialize_declarations(declarations: &[(String, String)]) -> String {
    declarations
        .iter()
        .map(|(n, v)| format!("{n}: {v};"))
        .collect::<Vec<_>>()
        .join(" ")
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
        return serialize_declarations(&declarations);
    }
    declarations.retain(|(name, _)| !same_property(name, property));
    if !value.trim().is_empty() {
        declarations.push((property.to_owned(), value.to_owned()));
    }
    serialize_declarations(&declarations)
}

/// `getPropertyPriority`'s return value (CSSOM): `"important"` when the
/// validated declaration for `property` carries `!important`, otherwise
/// `""` -- including when `property` is not declared at all, is unsupported,
/// or holds an unparseable value (all dropped from the validated view).
fn property_priority(style_attr: Option<&str>, property: &str) -> &'static str {
    let trimmed = property.trim();
    if trimmed.is_empty() {
        return "";
    }
    let lookup = if trimmed.starts_with("--") {
        if trimmed.len() <= 2 {
            return "";
        }
        trimmed.to_owned()
    } else {
        trimmed.to_ascii_lowercase()
    };
    match validated_inline_declarations(style_attr)
        .into_iter()
        .rev()
        .find(|d| same_property(&d.name, &lookup))
    {
        Some(d) if d.important => "important",
        _ => "",
    }
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

/// `Element.getBoundingClientRect` (CSSOM View §5): the border box relative
/// to the viewport, as a real `DOMRect`. The host reports boxes relative to
/// the initial containing block, so the viewport offset is subtracted here,
/// except for fixed-position boxes which are already viewport-relative and
/// stay put. There is still no transform to account for.
pub(crate) fn get_bounding_client_rect(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let geometry = super::geometry::box_geometry(context, index)?;
    let Some(g) = geometry else {
        return Ok(super::geometry::new_dom_rect(context, Default::default())?.into());
    };
    let mut r = g.border_box;
    if g.position != super::host::PositionKind::Fixed {
        let (scroll_x, scroll_y) =
            with_state(context, |s| (s.viewport_scroll_x, s.viewport_scroll_y))?;
        r.left -= scroll_x;
        r.right -= scroll_x;
        r.top -= scroll_y;
        r.bottom -= scroll_y;
    }
    Ok(super::geometry::new_dom_rect(context, r)?.into())
}

// ---- CSSStyleDeclaration ---------------------------------------------------

/// Native data behind one `CSSStyleDeclaration` instance: the arena index
/// of the element it reads/writes, and whether it is `getComputedStyle`'s
/// read-only view or an element's own (writable) inline `style`.
struct StyleSource {
    index: usize,
    computed: bool,
    rule: Option<usize>,
}

impl StyleSource {
    /// The declared property name at `index` -- the WebIDL "supported
    /// property indices" this object exposes (CSSOM §6.7's `item()` and
    /// indexed access).
    ///
    /// An inline declaration enumerates its validated declarations
    /// ([`validated_inline_declarations`]) in declaration order (last
    /// occurrence wins, survivor at the last position): unsupported names
    /// and unparseable values are dropped, duplicates collapse to one.
    /// Shorthands are kept as specified (not expanded into longhands).
    ///
    /// A computed declaration enumerates supported longhands only (see
    /// [`is_computed_shorthand`]), in the sorted order
    /// [`raikiri_style::property::supported_property_names`] already
    /// returns. A computed read here never has to flush the host, unlike
    /// [`read_property_value`]'s per-name lookup.
    fn declared_name_at(&self, index: usize, context: &mut Context) -> JsResult<Option<String>> {
        if self.computed {
            return Ok(computed_longhand_names()
                .get(index)
                .map(|name| (*name).to_owned()));
        }
        let index_ = self.index;
        let rule = self.rule;
        with_state(context, |s| {
            let text = match rule {
                Some(rule) => {
                    super::stylesheet::stylesheet_rule_body(s.host.document(), index_, rule)
                }
                None => s
                    .host
                    .document()
                    .element_attribute(index_, "style")
                    .map(str::to_owned),
            };
            validated_inline_declarations(text.as_deref())
                .into_iter()
                .nth(index)
                .map(|d| d.name)
        })
    }

    fn declared_len(&self, context: &mut Context) -> JsResult<usize> {
        if self.computed {
            return Ok(computed_longhand_names().len());
        }
        let index_ = self.index;
        let rule = self.rule;
        with_state(context, |s| {
            let text = match rule {
                Some(rule) => {
                    super::stylesheet::stylesheet_rule_body(s.host.document(), index_, rule)
                }
                None => s
                    .host
                    .document()
                    .element_attribute(index_, "style")
                    .map(str::to_owned),
            };
            validated_inline_declarations(text.as_deref()).len()
        })
    }
}

impl IndexedSource for StyleSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        self.declared_len(context)
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        Ok(self
            .declared_name_at(index, context)?
            .map(|name| JsValue::from(JsString::from(name))))
    }
}

/// Brand check for `CSSStyleDeclaration` members.
fn this_style(this: &JsValue, context: &mut Context) -> JsResult<Rc<StyleSource>> {
    this_indexed::<StyleSource>(this, context, "CSSStyleDeclaration")
}

fn read_declared(context: &mut Context, source: &StyleSource, property: &str) -> JsResult<String> {
    with_state(context, |s| {
        let text = match source.rule {
            Some(rule) => {
                super::stylesheet::stylesheet_rule_body(s.host.document(), source.index, rule)
            }
            None => s
                .host
                .document()
                .element_attribute(source.index, "style")
                .map(str::to_owned),
        };
        inline_style_value(text.as_deref(), property)
    })
}

/// `value` parsed as `property` with the same grammar the cascade uses, or
/// `None` when it does not parse.
fn parse_property_value(
    property: &str,
    value: &str,
) -> Option<raikiri_style::property::PropertyValue> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(
            |input| -> Result<raikiri_style::property::PropertyValue, cssparser::ParseError<'_, ()>> {
                raikiri_style::property::parse_value(property, input)
                    .ok_or_else(|| input.new_custom_error(()))
            },
        )
        .ok()
}

/// CSSOM `setProperty`/`removeProperty`/`getPropertyPriority` step 2.1: a
/// non-custom `property` name is used ASCII-lowercased; a custom property
/// name (`--`-prefixed) is used exactly as given.
fn ascii_lowercase_property_name(property: String) -> String {
    if property.starts_with("--") {
        property
    } else {
        property.to_ascii_lowercase()
    }
}

fn is_css_wide_keyword(value: &str) -> bool {
    ["inherit", "initial", "unset", "revert", "revert-layer"]
        .iter()
        .any(|keyword| value.trim().eq_ignore_ascii_case(keyword))
}

/// CSSOM `setProperty` steps 2-6 ("parse a CSS value"): the text to store
/// for a non-empty `value` of `property`, or `None` when the value does
/// not parse and the declaration must be left untouched.
///
/// A non-custom `property` outside
/// [`raikiri_style::property::supported_property_names`] is unsupported
/// (CSSOM step 2.2): the caller must return without a write. This helper
/// reports that the same way as an unparseable value (`None`), so both
/// paths leave the existing declaration untouched.
///
/// A `value` with a trailing `!important` is invalid here (CSSOM keeps
/// `value` and `priority` separate; `red !important` as a `value` must not
/// become a priority). Callers that parse a full declaration block
/// (`cssText`, the inline `length`/`item` view) strip that suffix before
/// calling, and re-attach it as the declaration's priority.
///
/// A parsed value is stored in its canonical serialization where
/// `raikiri_style` can produce one, and as given otherwise. CSS-wide
/// keywords serialize ASCII-lowercased.
fn declaration_value(property: &str, value: &str) -> Option<String> {
    let is_custom = property.starts_with("--");
    if !is_custom && !raikiri_style::property::is_supported_property_name(property) {
        return None;
    }
    let trimmed = value.trim();
    if strip_important(trimmed) != trimmed {
        return None;
    }
    if !is_custom && is_css_wide_keyword(value) {
        return Some(value.trim().to_ascii_lowercase());
    }
    let parsed = parse_property_value(property, value)?;
    Some(
        raikiri_style::property::serialize_value(&parsed)
            .or_else(|| {
                raikiri_style::property::serialize_color_value(
                    &property.to_ascii_lowercase(),
                    value,
                )
            })
            .unwrap_or_else(|| value.trim().to_owned()),
    )
}

/// Set (or, for an empty or whitespace-only `value`, remove) one inline
/// declaration, dropping a non-empty `value` that does not parse for
/// `property` (including an unsupported non-custom name, reported as `None`
/// by [`declaration_value`).
fn set_declaration(
    context: &mut Context,
    source: &StyleSource,
    property: &str,
    value: &str,
    important: bool,
) -> JsResult<()> {
    if value.trim().is_empty() {
        return write_declared(context, source, property, "");
    }
    let Some(stored) = declaration_value(property.trim(), value) else {
        return Ok(());
    };
    let stored = if important {
        format!("{stored} !important")
    } else {
        stored
    };
    write_declared(context, source, property, &stored)
}

fn write_declared(
    context: &mut Context,
    source: &StyleSource,
    property: &str,
    value: &str,
) -> JsResult<()> {
    if property.trim().is_empty() {
        return Ok(());
    }
    if let Some(rule) = source.rule {
        let text = with_state(context, |s| {
            super::stylesheet::stylesheet_rule_body(s.host.document(), source.index, rule)
                .unwrap_or_default()
        })?;
        let next = with_inline_style_property(Some(&text), property, value);
        return super::stylesheet::write_stylesheet_rule_body(context, source.index, rule, &next);
    }
    with_state(context, |s| {
        let next = with_inline_style_property(
            s.host.document().element_attribute(source.index, "style"),
            property,
            value,
        );
        s.host
            .document_mut()
            .set_element_inline_style(source.index, Some(next.into()));
    })?;
    mark_dirty(context)
}

/// The computed value of `property`, or `None` for an unsupported property
/// name (checked before any flush, so an unsupported name never triggers
/// one). A host's own `computed_value` failure message can name the arena
/// index it failed on -- an implementation detail that must never be
/// observable from script -- so the exception thrown into script carries a
/// fixed message instead, while the harness-facing host failure keeps the
/// original, more specific one (the same shape `node.rs`'s `inner_html`
/// getter uses for its own host-failure path).
fn read_computed(context: &mut Context, index: usize, property: &str) -> JsResult<Option<String>> {
    if raikiri_style::ComputedProperty::from_name(property).is_none() {
        return Ok(None);
    }
    ensure_flushed(context)?;
    let value = with_state(context, |s| s.host.computed_value(index, property))?;
    value.map_err(|error| host_failure_with_message(context, error, "computed style lookup failed"))
}

/// `getPropertyValue`/an attribute accessor's shared read: the raw declared
/// value for an inline declaration, or the resolved value (`""` when
/// unsupported) for a computed one.
fn read_property_value(
    context: &mut Context,
    source: &StyleSource,
    name: &str,
) -> JsResult<String> {
    if source.computed {
        Ok(read_computed(context, source.index, name)?.unwrap_or_default())
    } else {
        read_declared(context, source, name)
    }
}

/// `NoModificationAllowedError` for a write attempted on a computed
/// (read-only) `CSSStyleDeclaration` (CSSOM's "readonly flag").
fn no_modification_allowed(context: &mut Context) -> JsError {
    throw_dom_exception(
        context,
        "NoModificationAllowedError",
        "computed style is read-only",
    )
}

/// `DOMString` conversion of an *optional* argument that defaults to `""`:
/// a genuinely missing trailing argument, or one explicitly passed as
/// `undefined` or `null` (`setProperty`'s `priority` is
/// `[LegacyNullToEmptyString]`), takes `""` directly; anything else runs
/// `ToString` as usual.
fn optional_string(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<String> {
    match args.get(i) {
        None => Ok(String::new()),
        Some(v) if v.is_undefined() || v.is_null() => Ok(String::new()),
        Some(_) => dom_string(args, i, context),
    }
}

/// `DOMString` conversion of a `[LegacyNullToEmptyString]` argument (CSSOM:
/// `setProperty`'s `value`, `cssFloat`, and every generated camel/dashed/
/// webkit-cased attribute setter): an explicit `null` converts to `""`
/// directly; anything else -- including a missing argument or an explicit
/// `undefined` -- runs the ordinary `ToString` conversion via
/// [`dom_string`] as usual. Unlike [`optional_string`] (an *optional*
/// argument with a default), a required `[LegacyNullToEmptyString]`
/// argument does not also treat `undefined` as `""`: only `null` gets the
/// special conversion; `s.color = undefined` stores the literal string
/// `"undefined"`, the same as any other required `DOMString` argument.
fn legacy_null_to_empty_string(
    args: &[JsValue],
    i: usize,
    context: &mut Context,
) -> JsResult<String> {
    match args.get(i) {
        Some(v) if v.is_null() => Ok(String::new()),
        _ => dom_string(args, i, context),
    }
}

fn style_length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    Ok(JsValue::from(source.length(context)? as u32))
}

fn style_item(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    let index = arg_unsigned_long(args, 0, context)?;
    let name = source.declared_name_at(index, context)?.unwrap_or_default();
    Ok(JsValue::from(JsString::from(name)))
}

/// `parentRule` for a declaration from a rule, or `null` for inline style.
fn style_parent_rule(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    match source.rule {
        Some(rule) => super::stylesheet::parent_rule(context, source.index, rule),
        None => Ok(JsValue::null()),
    }
}

fn style_get_property_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    let name = dom_string(args, 0, context)?;
    let value = read_property_value(context, &source, &name)?;
    Ok(JsValue::from(JsString::from(value)))
}

fn style_get_property_priority(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    let name = ascii_lowercase_property_name(dom_string(args, 0, context)?);
    let priority = if source.computed {
        String::new()
    } else {
        let index = source.index;
        let rule = source.rule;
        with_state(context, |s| {
            let text = match rule {
                Some(rule) => {
                    super::stylesheet::stylesheet_rule_body(s.host.document(), index, rule)
                }
                None => s
                    .host
                    .document()
                    .element_attribute(index, "style")
                    .map(str::to_owned),
            };
            property_priority(text.as_deref(), &name).to_owned()
        })?
    };
    Ok(JsValue::from(JsString::from(priority)))
}

/// `setProperty(property, value, priority = "")` (CSSOM §6.7). `value` and
/// `priority` are both converted before either is inspected (WebIDL
/// converts every argument before an operation's own algorithm runs, so a
/// `priority` whose conversion throws must abort the call even when
/// `value` is empty) -- but the empty-`value` check itself still runs
/// *before* validating `priority`, so `setProperty(name, "", "important")`
/// removes the declaration rather than storing a lone `" !important"`. A
/// `priority` that is neither `""` nor an ASCII case-insensitive match for
/// `"important"` is spec-silent (return, no error, no write) rather than
/// rejected as an argument error.
///
/// A non-custom `property` outside
/// [`raikiri_style::property::supported_property_names`] makes the method
/// return without a write (CSSOM step 2.2); the name is ASCII-lowercased
/// per step 2.1 before that check, and custom names (`--`-prefixed) are
/// used exactly as given. Only the per-property accessors
/// (`s.marginTop = …`) stay scoped to the supported list, becoming ordinary
/// expandos otherwise.
///
/// A `value` with a trailing `!important` never becomes a priority here:
/// [`declaration_value`] reports it as invalid, so the declaration is left
/// untouched (the `!important` suffix only becomes a priority when parsing
/// a full block such as `cssText`). A whitespace-only `value` counts as
/// empty and removes the declaration.
fn style_set_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    if source.computed {
        return Err(no_modification_allowed(context));
    }
    let raw_name = dom_string(args, 0, context)?;
    let name = ascii_lowercase_property_name(raw_name);
    let trimmed_name = name.trim().to_owned();
    let value = legacy_null_to_empty_string(args, 1, context)?;
    let priority = optional_string(args, 2, context)?;
    let value_is_empty = value.trim().is_empty();
    if !value_is_empty && !priority.is_empty() && !priority.eq_ignore_ascii_case("important") {
        return Ok(JsValue::undefined());
    }
    if !trimmed_name.is_empty()
        && !trimmed_name.starts_with("--")
        && !raikiri_style::property::is_supported_property_name(&trimmed_name)
    {
        return Ok(JsValue::undefined());
    }
    set_declaration(
        context,
        &source,
        &trimmed_name,
        &value,
        !priority.is_empty(),
    )?;
    Ok(JsValue::undefined())
}

fn style_remove_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    if source.computed {
        return Err(no_modification_allowed(context));
    }
    let name = ascii_lowercase_property_name(dom_string(args, 0, context)?);
    let previous = read_declared(context, &source, &name)?;
    write_declared(context, &source, &name, "")?;
    Ok(JsValue::from(JsString::from(previous)))
}

/// Serialize one validated declaration, re-attaching `!important` when the
/// raw block carried it.
fn serialize_validated(declarations: &[ValidatedDeclaration]) -> String {
    declarations
        .iter()
        .map(|d| {
            if d.important {
                format!("{}: {} !important;", d.name, d.value)
            } else {
                format!("{}: {};", d.name, d.value)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `cssText`, on getting: `""` for a computed declaration (CSSOM); the
/// validated inline declarations serialized otherwise (unsupported names
/// and unparseable values dropped, non-custom names lowercased, values in
/// canonical serialization, CSS-wide keywords lowercased).
fn style_css_text_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    if source.computed {
        return Ok(JsValue::from(JsString::from("")));
    }
    let index = source.index;
    let rule = source.rule;
    let text = with_state(context, |s| {
        let style_text = match rule {
            Some(rule) => super::stylesheet::stylesheet_rule_body(s.host.document(), index, rule),
            None => s
                .host
                .document()
                .element_attribute(index, "style")
                .map(str::to_owned),
        };
        serialize_validated(&validated_inline_declarations(style_text.as_deref()))
    })?;
    Ok(JsValue::from(JsString::from(text)))
}

/// `cssText`, on setting (CSSOM): parse the given text as a declaration
/// block, dropping invalid declarations (empty or unsupported names,
/// empty or unparseable values), then replace the whole inline `style`
/// attribute with that validated serialization. Unlike
/// `setAttribute("style", …)` (which stores verbatim), an invalid
/// declaration here never reaches storage.
fn style_css_text_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    if source.computed {
        return Err(no_modification_allowed(context));
    }
    let text = dom_string(args, 0, context)?;
    let validated = serialize_validated(&validated_inline_declarations(Some(&text)));
    if let Some(rule) = source.rule {
        super::stylesheet::write_stylesheet_rule_body(context, source.index, rule, &validated)?;
    } else {
        with_state(context, |s| {
            s.host
                .document_mut()
                .set_element_inline_style(source.index, Some(validated.into()));
        })?;
        mark_dirty(context)?;
    }
    Ok(JsValue::undefined())
}

/// `cssFloat` (CSSOM `CSSStyleProperties`): a fixed alias for the `float`
/// property, kept separate from the generated per-property accessors
/// because `float` is a JavaScript reserved word in older engines --
/// `getPropertyValue`/`setProperty` with `"float"` as the property name
/// still works.
fn style_css_float_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    let value = read_property_value(context, &source, "float")?;
    Ok(JsValue::from(JsString::from(value)))
}

fn style_css_float_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let source = this_style(this, context)?;
    if source.computed {
        return Err(no_modification_allowed(context));
    }
    let value = legacy_null_to_empty_string(args, 0, context)?;
    set_declaration(context, &source, "float", &value, false)?;
    Ok(JsValue::undefined())
}

pub(crate) const CSS_STYLE_DECLARATION_MEMBERS: Members = Members {
    getters: &[("length", style_length), ("parentRule", style_parent_rule)],
    accessors: &[
        ("cssText", style_css_text_get, style_css_text_set),
        ("cssFloat", style_css_float_get, style_css_float_set),
    ],
    methods: &[
        ("item", 1, style_item),
        ("getPropertyValue", 1, style_get_property_value),
        ("getPropertyPriority", 1, style_get_property_priority),
        ("setProperty", 2, style_set_property),
        ("removeProperty", 1, style_remove_property),
    ],
};

/// CSSOM §6.7.1 "CSS property to IDL attribute": every `-` (U+002D)
/// triggers uppercasing the character right after it. `lowercase_first`
/// additionally drops `property`'s own first character before that loop
/// runs -- used only for the `-webkit-` prefix's lowercase-`w`
/// "webkit-cased attribute" (its ordinary, non-dropped camel-cased form
/// already yields the capital-`W` form, "Webkit...", for the same reason:
/// the leading `-` itself triggers uppercasing the `w` right after it).
fn css_property_to_idl_attribute(property: &str, lowercase_first: bool) -> String {
    let mut chars = property.chars();
    if lowercase_first {
        chars.next();
    }
    let mut output = String::with_capacity(property.len());
    let mut uppercase_next = false;
    for c in chars {
        if c == '-' {
            uppercase_next = true;
        } else if uppercase_next {
            output.push(c.to_ascii_uppercase());
            uppercase_next = false;
        } else {
            output.push(c);
        }
    }
    output
}

/// The CSSOM §6.7.1 attribute names generated for `property`: its dashed
/// form (`property` itself) always; its camel-cased form, only when that
/// differs from the dashed form (a property with no `-` camel-cases to
/// itself, so CSSOM does not generate a second, redundant attribute for
/// it); and, only when `property` begins with `-webkit-`, its
/// webkit-cased form too (the matching capital-`W` "Webkit..." form is
/// already produced by the camel-cased step above, applied to the leading
/// `-` itself).
fn attribute_names(property: &str) -> Vec<String> {
    let mut names = vec![property.to_owned()];
    let camel = css_property_to_idl_attribute(property, false);
    if camel != property {
        names.push(camel);
    }
    if property.starts_with("-webkit-") {
        names.push(css_property_to_idl_attribute(property, true));
    }
    names
}

/// One `property` attribute's getter/setter pair, named after `key` (the
/// attribute spelling it is about to be installed under).
fn property_accessor_pair(
    context: &mut Context,
    property: &'static str,
    key: &str,
) -> JsResult<(JsFunction, JsFunction)> {
    let getter = NativeFunction::from_copy_closure(move |this, _args, ctx| {
        let source = this_style(this, ctx)?;
        let value = read_property_value(ctx, &source, property)?;
        Ok(JsValue::from(JsString::from(value)))
    });
    let getter = closure_function(context, &format!("get {key}"), 0, getter)?;
    let setter = NativeFunction::from_copy_closure(move |this, args, ctx| {
        let source = this_style(this, ctx)?;
        if source.computed {
            return Err(no_modification_allowed(ctx));
        }
        let value = legacy_null_to_empty_string(args, 0, ctx)?;
        set_declaration(ctx, &source, property, &value, false)?;
        Ok(JsValue::undefined())
    });
    let setter = closure_function(context, &format!("set {key}"), 1, setter)?;
    Ok((getter, setter))
}

fn define_accessor(
    prototype: &JsObject,
    key: &str,
    getter: JsFunction,
    setter: JsFunction,
    context: &mut Context,
) -> JsResult<()> {
    let descriptor = PropertyDescriptor::builder()
        .get(getter)
        .set(setter)
        .enumerable(true)
        .configurable(true)
        .build();
    prototype.define_property_or_throw(JsString::from(key), descriptor, context)?;
    Ok(())
}

/// Define, on `prototype`, every dashed / camelCase / (for `-webkit-`
/// names) webkit-cased attribute CSSOM §6.7.1 generates for each of
/// [`raikiri_style::property::supported_property_names`]. `cssFloat` is
/// not generated by that algorithm -- it is its own fixed member, defined
/// through [`CSS_STYLE_DECLARATION_MEMBERS`] instead.
pub(crate) fn install_property_accessors(
    context: &mut Context,
    prototype: &JsObject,
) -> JsResult<()> {
    for &property in raikiri_style::property::supported_property_names() {
        for key in attribute_names(property) {
            let (getter, setter) = property_accessor_pair(context, property, &key)?;
            define_accessor(prototype, &key, getter, setter, context)?;
        }
    }
    Ok(())
}

fn style_object(
    context: &mut Context,
    index: usize,
    computed: bool,
    rule: Option<usize>,
) -> JsResult<JsObject> {
    let prototype = protos(context).css_style_declaration.clone();
    indexed_object(
        context,
        prototype,
        StyleSource {
            index,
            computed,
            rule,
        },
    )
}

fn style(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    if let Some(existing) = with_state(context, |s| s.style_objects.get(&index).cloned())? {
        return Ok(existing.into());
    }
    let object = style_object(context, index, false, None)?;
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
    if let Some(existing) = with_state(context, |s| s.computed_style_objects.get(&index).cloned())?
    {
        return Ok(existing.into());
    }
    let object = style_object(context, index, true, None)?;
    with_state(context, |s| {
        s.computed_style_objects.insert(index, object.clone())
    })?;
    Ok(object.into())
}

pub(crate) fn stylesheet_style_object(
    context: &mut Context,
    style_element: usize,
    rule: usize,
) -> JsResult<JsObject> {
    style_object(context, style_element, false, Some(rule))
}

fn css_supports(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let property = dom_string(args, 0, context)?;
    let value = dom_string(args, 1, context)?;
    let parsed = parse_property_value(&property, &value).is_some();
    let wide = is_css_wide_keyword(&value);
    Ok(JsValue::from(
        parsed || (wide && raikiri_style::property::is_supported_property_name(&property)),
    ))
}

pub(crate) const HTML_ELEMENT_MEMBERS: Members = Members {
    getters: &[("style", style)],
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
