//! `DOMTokenList` (DOM §7.1), the concrete interface backing
//! `Element.classList`.
//!
//! Spec ref: <https://dom.spec.whatwg.org/#interface-domtokenlist>

use std::rc::Rc;

use boa_engine::object::JsObject;
use boa_engine::{Context, JsNativeError, JsResult, JsValue};

use super::indexed::{IndexedSource, indexed_object, this_indexed};
use super::interfaces::{Members, protos};
use super::node::{attribute, js_str, write_attribute};
use super::webidl::{arg_unsigned_long, dom_string, throw_dom_exception};

const CLASS_ATTR: &str = "class";

/// The current token set for the `class` attribute of element `index`: the
/// attribute's value split on ASCII whitespace (DOM §7.1's "ordered set
/// parser"), duplicates removed, first occurrence kept. Recomputed fresh on
/// every access rather than cached, so the list stays live as `class`
/// changes through `setAttribute` or anything else.
fn token_set(context: &mut Context, index: usize) -> JsResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for token in attribute(context, index, CLASS_ATTR)?
        .unwrap_or_default()
        .split_ascii_whitespace()
    {
        if !out.iter().any(|t| t == token) {
            out.push(token.to_owned());
        }
    }
    Ok(out)
}

fn has_ascii_whitespace(token: &str) -> bool {
    token
        .chars()
        .any(|c| matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r'))
}

/// A `DOMTokenList` token (DOM §7.1): non-empty, no ASCII whitespace.
fn validate_token(context: &mut Context, token: &str) -> JsResult<()> {
    if token.is_empty() {
        return Err(throw_dom_exception(
            context,
            "SyntaxError",
            "token is empty",
        ));
    }
    if has_ascii_whitespace(token) {
        return Err(throw_dom_exception(
            context,
            "InvalidCharacterError",
            "token contains whitespace",
        ));
    }
    Ok(())
}

/// `replace`'s own validation (DOM §7.1): both tokens' emptiness is checked
/// before either one's whitespace, so `replace('a b', '')` reports the
/// `SyntaxError` (empty `newToken`) rather than the `InvalidCharacterError`
/// a token-at-a-time check would find first (`'a b'`'s embedded space).
fn validate_replace_tokens(
    context: &mut Context,
    old_token: &str,
    new_token: &str,
) -> JsResult<()> {
    if old_token.is_empty() || new_token.is_empty() {
        return Err(throw_dom_exception(
            context,
            "SyntaxError",
            "token is empty",
        ));
    }
    if has_ascii_whitespace(old_token) || has_ascii_whitespace(new_token) {
        return Err(throw_dom_exception(
            context,
            "InvalidCharacterError",
            "token contains whitespace",
        ));
    }
    Ok(())
}

/// DOM §7.1 "update steps": if the associated element has no `class`
/// attribute and `tokens` is empty, do nothing -- the attribute is never
/// created just to hold an empty value. Otherwise (re)serialize `tokens`
/// (already deduped by [`token_set`]) space-joined into the attribute.
fn run_update_steps(context: &mut Context, index: usize, tokens: &[String]) -> JsResult<()> {
    if attribute(context, index, CLASS_ATTR)?.is_none() && tokens.is_empty() {
        return Ok(());
    }
    write_attribute(context, index, CLASS_ATTR, &tokens.join(" "))
}

/// The source behind an element's `classList`: its arena index. `length`/
/// `item` always recompute [`token_set`], so the list stays live.
pub(crate) struct TokenListSource(pub(crate) usize);

impl IndexedSource for TokenListSource {
    fn length(&self, context: &mut Context) -> JsResult<usize> {
        Ok(token_set(context, self.0)?.len())
    }

    fn item(&self, index: usize, context: &mut Context) -> JsResult<Option<JsValue>> {
        Ok(token_set(context, self.0)?.get(index).map(|t| js_str(t)))
    }
}

/// A new `DOMTokenList` bound to element `index`.
pub(crate) fn token_list(context: &mut Context, index: usize) -> JsResult<JsObject> {
    let prototype = protos(context).dom_token_list.clone();
    indexed_object(context, prototype, TokenListSource(index))
}

fn this_source(this: &JsValue, context: &mut Context) -> JsResult<Rc<TokenListSource>> {
    this_indexed::<TokenListSource>(this, context, "DOMTokenList")
}

fn token_list_length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_source(this, context)?;
    Ok(JsValue::from(source.length(context)? as u32))
}

fn token_list_item(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let source = this_source(this, context)?;
    let index = arg_unsigned_long(args, 0, context)?;
    Ok(source.item(index, context)?.unwrap_or_else(JsValue::null))
}

fn token_list_contains(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    let token = dom_string(args, 0, context)?;
    Ok(JsValue::from(
        token_set(context, index)?.iter().any(|t| t == &token),
    ))
}

/// `add(...tokens)` (DOM §7.1): every argument is `DOMString`-converted and
/// validated up front, before any of them touch the token set, so a later
/// invalid token never leaves an earlier one applied.
fn token_list_add(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    let mut new_tokens = Vec::with_capacity(args.len());
    for i in 0..args.len() {
        new_tokens.push(dom_string(args, i, context)?);
    }
    for token in &new_tokens {
        validate_token(context, token)?;
    }
    let mut tokens = token_set(context, index)?;
    for token in new_tokens {
        if !tokens.iter().any(|t| t == &token) {
            tokens.push(token);
        }
    }
    run_update_steps(context, index, &tokens)?;
    Ok(JsValue::undefined())
}

/// `remove(...tokens)`: same convert-then-validate-then-mutate shape as
/// [`token_list_add`].
fn token_list_remove(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    let mut remove_tokens = Vec::with_capacity(args.len());
    for i in 0..args.len() {
        remove_tokens.push(dom_string(args, i, context)?);
    }
    for token in &remove_tokens {
        validate_token(context, token)?;
    }
    let mut tokens = token_set(context, index)?;
    tokens.retain(|t| !remove_tokens.iter().any(|r| r == t));
    run_update_steps(context, index, &tokens)?;
    Ok(JsValue::undefined())
}

fn token_list_toggle(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    let token = dom_string(args, 0, context)?;
    validate_token(context, &token)?;
    // `force` has no default value, but WebIDL overload resolution still
    // maps an explicitly-`undefined` optional argument to "not present"
    // (the same as a genuinely missing one), not to `ToBoolean(undefined)`
    // == `false` -- that collapsing only fails to apply to arguments that
    // have an actual default value to take instead (e.g. `DOMRect`'s
    // `x`/`y`/`width`/`height`, or `DOMException`'s `message`/`name`).
    let force = args
        .get(1)
        .filter(|v| !v.is_undefined())
        .map(JsValue::to_boolean);
    let mut tokens = token_set(context, index)?;
    let present = tokens.iter().any(|t| t == &token);
    if present {
        if force != Some(true) {
            tokens.retain(|t| t != &token);
            run_update_steps(context, index, &tokens)?;
            return Ok(JsValue::from(false));
        }
        return Ok(JsValue::from(true));
    }
    if force != Some(false) {
        tokens.push(token);
        run_update_steps(context, index, &tokens)?;
        return Ok(JsValue::from(true));
    }
    Ok(JsValue::from(false))
}

/// Infra "replace within a list", specialized to an already-deduplicated
/// token set: `newToken` takes the position of whichever of `oldToken`/
/// `newToken` occurs first, and every other occurrence of either is
/// dropped. For tokens `"a b c"`, replacing `"c"` with `"a"` (already
/// present, before "c") therefore gives `"a b"` -- "a" keeps its own
/// earlier position and "c" is simply dropped, rather than "a" moving to
/// "c"'s later position (which would give "b a").
fn replace_within(tokens: &[String], old_token: &str, new_token: &str) -> Vec<String> {
    let Some(index) = tokens.iter().position(|t| t == old_token || t == new_token) else {
        // cov:ignore: `token_list_replace` (this function's only caller) already
        // confirmed `old_token` is present before calling it, so `position` always
        // finds at least one of the two.
        return tokens.to_vec();
    };
    tokens
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            if i == index {
                Some(new_token.to_owned())
            } else if t != old_token && t != new_token {
                Some(t.clone())
            } else {
                None
            }
        })
        .collect()
}

fn token_list_replace(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    let old_token = dom_string(args, 0, context)?;
    let new_token = dom_string(args, 1, context)?;
    validate_replace_tokens(context, &old_token, &new_token)?;
    let tokens = token_set(context, index)?;
    if !tokens.iter().any(|t| t == &old_token) {
        return Ok(JsValue::from(false));
    }
    let tokens = replace_within(&tokens, &old_token, &new_token);
    run_update_steps(context, index, &tokens)?;
    Ok(JsValue::from(true))
}

/// `supports(token)` (DOM §7.1): the `class` attribute defines no supported
/// tokens, so this always throws, per the generic `DOMTokenList` algorithm
/// for an attribute local name without one.
fn token_list_supports(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Err(JsNativeError::typ()
        .with_message("DOMTokenList.supports: 'class' defines no supported tokens")
        .into())
}

/// `value`'s getter and the stringifier (DOM §7.1: both run "get an
/// attribute value") share this: the raw `class` attribute text, verbatim --
/// not a reserialization of the parsed, whitespace-collapsed, deduplicated
/// [`token_set`] that `length`/`item`/iteration expose.
fn raw_class_attribute(context: &mut Context, index: usize) -> JsResult<String> {
    Ok(attribute(context, index, CLASS_ATTR)?.unwrap_or_default())
}

fn token_list_value(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    Ok(js_str(&raw_class_attribute(context, index)?))
}

/// `value`'s setter (DOM §7.1): unlike `add`/`remove`/etc., this sets the
/// attribute to the given string directly, bypassing the update steps'
/// "don't create an attribute for an empty set" rule -- setting `value = ''`
/// still creates an empty `class` attribute if none existed. `value` is
/// `[LegacyNullToEmptyString]`: an explicit `null` stores `""` directly
/// rather than running the ordinary `ToString` conversion (which would
/// otherwise stringify it to the literal `"null"`); anything else --
/// including a missing argument or an explicit `undefined` -- still runs
/// `ToString` as usual.
fn set_token_list_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_source(this, context)?.0;
    let value = match args.first() {
        Some(v) if v.is_null() => String::new(),
        _ => dom_string(args, 0, context)?,
    };
    write_attribute(context, index, CLASS_ATTR, &value)?;
    Ok(JsValue::undefined())
}

fn token_list_to_string(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    token_list_value(this, args, context)
}

pub(crate) const DOM_TOKEN_LIST_MEMBERS: Members = Members {
    getters: &[("length", token_list_length)],
    accessors: &[("value", token_list_value, set_token_list_value)],
    methods: &[
        ("item", 1, token_list_item),
        ("contains", 1, token_list_contains),
        ("add", 0, token_list_add),
        ("remove", 0, token_list_remove),
        ("toggle", 1, token_list_toggle),
        ("replace", 2, token_list_replace),
        ("supports", 1, token_list_supports),
        ("toString", 0, token_list_to_string),
    ],
};

/// `DOMTokenList`'s `iterable<DOMString>` members (`entries`/`forEach`/
/// `keys`/`values`/`@@iterator`): every value here is already a plain
/// string, never a wrapped `Node`, so [`super::interfaces::
/// install_value_iterable`]'s `%Array.prototype%` functions are exactly
/// the right generated iterable members, the same as they are for
/// `NodeList` (see that function's own doc for why).
pub(crate) fn install_iteration(context: &mut Context, prototype: &JsObject) -> JsResult<()> {
    super::interfaces::install_value_iterable(context, prototype)
}

#[cfg(test)]
mod tests;
