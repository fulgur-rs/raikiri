//! Focus: `HTMLElement.focus()` / `blur()` / `tabIndex` and
//! `Document.activeElement` / `hasFocus()`.
//!
//! The focused element lives on the DOM ([`raikiri_dom::Document`]), where
//! the cascade reads it for `:focus`, `:focus-within` and `:focus-visible`.
//! There is no user input here, so focus only ever moves by script.

use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::{Document, NodeKind};

use super::dispatch::fire_focus_event;
use super::interfaces::{HTML_NS, Members, SVG_NS, wrap_optional};
use super::node::{mark_dirty, write_attribute};
use super::style::ensure_flushed;
use super::webidl::{host_failure, this_document, this_element, with_state};

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// HTML "rules for parsing integers": optional ASCII whitespace, an optional
/// sign, then ASCII digits; `None` when no digit follows or it overflows.
///
/// Spec: <https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-integers>
fn parse_integer(value: &str) -> Option<i32> {
    let rest = value.trim_start_matches(['\t', '\n', '\x0C', '\r', ' ']);
    let (negative, rest) = match rest.as_bytes().first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let magnitude: i64 = rest[..digits].parse().ok()?;
    i32::try_from(if negative { -magnitude } else { magnitude }).ok()
}

fn tag_in<'a>(document: &'a Document, index: usize, namespace: &str) -> Option<&'a str> {
    let node = document.get_node(index)?;
    (node.kind() == NodeKind::Element && document.element_namespace_uri(index) == Some(namespace))
        .then(|| node.tag_name())
        .flatten()
}

fn html_tag(document: &Document, index: usize) -> Option<&str> {
    tag_in(document, index, HTML_NS)
}

/// Whether `index` is an SVG `<a>` with a link, focusable by default.
///
/// Spec: <https://svgwg.org/svg2-draft/interact.html#Focus>
fn is_svg_link(document: &Document, index: usize) -> bool {
    tag_in(document, index, SVG_NS) == Some("a")
        && (document.element_attribute(index, "href").is_some()
            || document
                .element_attribute_ns(index, XLINK_NS, "href")
                .is_some())
}

/// Whether a `contenteditable` value is in the true or plaintext-only
/// state, making its element an editing host. Other values, invalid ones
/// included, are the false or inherit state.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#attr-contenteditable>
fn is_editing_host_value(value: &str) -> bool {
    value.is_empty()
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("plaintext-only")
}

/// Whether `index` is a `<summary>` that is the first `<summary>` child of
/// a `<details>` (HTML "summary for its parent details").
fn is_details_summary(document: &Document, index: usize) -> bool {
    let Some(parent) = document.parent_of(index) else {
        return false;
    };
    html_tag(document, parent) == Some("details")
        && document.get_node(parent).is_some_and(|details| {
            details
                .children
                .iter()
                .copied()
                .find(|&child| html_tag(document, child) == Some("summary"))
                == Some(index)
        })
}

/// Whether the HTML element `index` is focusable on its own, without a
/// `tabindex` attribute: the elements whose default `tabIndex` is 0.
fn focusable_by_default(document: &Document, index: usize) -> bool {
    let attr = |name| document.element_attribute(index, name);
    match html_tag(document, index) {
        Some("a" | "area") => attr("href").is_some(),
        Some("input") => !attr("type").is_some_and(|kind| kind.eq_ignore_ascii_case("hidden")),
        Some("button" | "select" | "textarea" | "iframe" | "object" | "frame") => true,
        Some("audio" | "video") => attr("controls").is_some(),
        Some("summary") => is_details_summary(document, index),
        _ => false,
    }
}

/// The `tabIndex` default of an element without a valid `tabindex`: 0 for
/// a fixed list of elements, -1 for the rest. The list is historical and
/// differs from which elements are focusable by default.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#dom-tabindex>
fn default_tab_index(document: &Document, index: usize) -> i32 {
    let zero = match html_tag(document, index) {
        Some(
            "a" | "area" | "button" | "frame" | "iframe" | "input" | "object" | "select"
            | "textarea",
        ) => true,
        Some("summary") => is_details_summary(document, index),
        Some(_) => false,
        None => tag_in(document, index, SVG_NS) == Some("a"),
    };
    if zero { 0 } else { -1 }
}

/// Whether the form control `index` is disabled: by its own `disabled`
/// attribute, or by a `<fieldset disabled>` ancestor unless it is inside
/// that fieldset's first `<legend>` child.
///
/// Spec: <https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#concept-fe-disabled>
fn is_disabled_control(document: &Document, index: usize) -> bool {
    if !matches!(
        html_tag(document, index),
        Some("button" | "input" | "select" | "textarea")
    ) {
        return false;
    }
    if document.element_attribute(index, "disabled").is_some() {
        return true;
    }
    let mut child = index;
    while let Some(parent) = document.parent_of(child) {
        if html_tag(document, parent) == Some("fieldset")
            && document.element_attribute(parent, "disabled").is_some()
        {
            let first_legend = document.get_node(parent).and_then(|fieldset| {
                fieldset
                    .children
                    .iter()
                    .copied()
                    .find(|&c| html_tag(document, c) == Some("legend"))
            });
            if first_legend != Some(child) {
                return true;
            }
        }
        child = parent;
    }
    false
}

/// Whether the HTML or SVG element `index` is a focusable area: connected,
/// not inert, not a disabled form control, and focusable by default (an
/// SVG link counts), through a valid `tabindex`, or as an HTML editing
/// host. Whether it is being rendered is
/// not checked, since that needs a layout.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focusable-area>
fn is_focusable(document: &Document, index: usize) -> bool {
    let svg = tag_in(document, index, SVG_NS).is_some();
    if html_tag(document, index).is_none() && !svg {
        return false;
    }
    let attr = |name| document.element_attribute(index, name);
    if is_disabled_control(document, index) {
        return false;
    }
    let mut ancestor = Some(index);
    let mut connected = false;
    while let Some(current) = ancestor {
        if current == document.root_index() {
            connected = true;
        } else if document.element_attribute(current, "inert").is_some() {
            return false;
        }
        ancestor = document.parent_of(current);
    }
    connected
        && (attr("tabindex").and_then(parse_integer).is_some()
            || is_svg_link(document, index)
            || focusable_by_default(document, index)
            || !svg && attr("contenteditable").is_some_and(is_editing_host_value))
}

/// HTML "focus update steps" for a move from the current focus to `next`
/// (`None` to clear it): `blur` then `focusout` at the old element, then
/// `focus` then `focusin` at the new one, each a `FocusEvent` whose
/// `relatedTarget` is the other element.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focus-update-steps>
fn move_focus(context: &mut Context, next: Option<usize>) -> JsResult<()> {
    let previous = with_state(context, |s| s.host.document().focused_element())?;
    if previous == next {
        return Ok(());
    }
    with_state(context, |s| s.host.document_mut().set_focused_element(next))?;
    // `:focus` and `:focus-within` change with it.
    mark_dirty(context)?;
    if let Some(previous) = previous {
        fire_focus_event(context, previous, "blur", false, next)?;
        fire_focus_event(context, previous, "focusout", true, next)?;
    }
    if let Some(next) = next {
        fire_focus_event(context, next, "focus", false, previous)?;
        fire_focus_event(context, next, "focusin", true, previous)?;
    }
    Ok(())
}

/// Whether the element `index` is being rendered, after bringing style and
/// layout up to date. A focusable area must be.
fn is_being_rendered(context: &mut Context, index: usize) -> JsResult<bool> {
    ensure_flushed(context)?;
    let rendered = with_state(context, |s| s.host.is_being_rendered(index))?;
    rendered.map_err(|error| host_failure(context, error))
}

/// Whether `index` is a focusable area that is being rendered.
fn can_take_focus(context: &mut Context, index: usize) -> JsResult<bool> {
    Ok(
        with_state(context, |s| is_focusable(s.host.document(), index))?
            && is_being_rendered(context, index)?,
    )
}

/// The rendering update's focus fixup: clear focus, firing `blur` and
/// `focusout`, when the focused element is no longer a focusable area.
/// Leaving the document already cleared it without events.
///
/// Spec: <https://html.spec.whatwg.org/multipage/webappapis.html#update-the-rendering>
pub(crate) fn fix_up_focus(context: &mut Context) -> JsResult<()> {
    let focused = with_state(context, |s| s.host.document().focused_element())?;
    if let Some(focused) = focused
        && !can_take_focus(context, focused)?
    {
        move_focus(context, None)?;
    }
    Ok(())
}

/// `HTMLElement.focus(options)`: focus the element if it is a focusable
/// area; otherwise do nothing. `preventScroll` has nothing to prevent.
fn focus(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    if can_take_focus(context, index)? {
        move_focus(context, Some(index))?;
    }
    Ok(JsValue::undefined())
}

/// `HTMLElement.blur()`: unfocus the element if it has focus.
fn blur(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    if with_state(context, |s| s.host.document().focused_element())? == Some(index) {
        move_focus(context, None)?;
    }
    Ok(JsValue::undefined())
}

/// `tabIndex`: the parsed `tabindex` attribute, else the element's
/// default.
fn tab_index(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let value = with_state(context, |s| {
        let document = s.host.document();
        document
            .element_attribute(index, "tabindex")
            .and_then(parse_integer)
            .unwrap_or_else(|| default_tab_index(document, index))
    })?;
    Ok(JsValue::from(value))
}

fn set_tab_index(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    // WebIDL `long`.
    let value = args.first().cloned().unwrap_or_default().to_i32(context)?;
    write_attribute(context, index, "tabindex", &value.to_string())?;
    Ok(JsValue::undefined())
}

pub(crate) const HTML_ELEMENT_FOCUS_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[("tabIndex", tab_index, set_tab_index)],
    methods: &[("focus", 0, focus), ("blur", 0, blur)],
};

/// `Document.activeElement`: the focused element, else the body, else the
/// document element.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#dom-document-activeelement>
fn active_element(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let active = with_state(context, |s| {
        let document = s.host.document();
        document.focused_element().or_else(|| {
            let html = document
                .get_node(document.root_index())?
                .children
                .iter()
                .copied()
                .find(|&child| {
                    document
                        .get_node(child)
                        .is_some_and(|node| node.kind() == NodeKind::Element)
                })?;
            let body = document
                .get_node(html)?
                .children
                .iter()
                .copied()
                .find(|&child| matches!(html_tag(document, child), Some("body" | "frameset")));
            Some(body.unwrap_or(html))
        })
    })?;
    wrap_optional(context, active)
}

/// `Document.hasFocus()`: the page is always the focused top-level
/// browsing context here.
fn has_focus(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    Ok(JsValue::from(true))
}

pub(crate) const DOCUMENT_FOCUS_MEMBERS: Members = Members {
    getters: &[("activeElement", active_element)],
    accessors: &[],
    methods: &[("hasFocus", 0, has_focus)],
};

#[cfg(test)]
mod tests;
