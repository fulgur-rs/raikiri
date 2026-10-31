//! Focus: `HTMLElement.focus()` / `blur()` / `tabIndex` and
//! `Document.activeElement` / `hasFocus()`.
//!
//! The focused element lives on the DOM ([`raikiri_dom::Document`]), where
//! the cascade reads it for `:focus`, `:focus-within` and `:focus-visible`.
//! There is no user input here, so focus only ever moves by script.

use std::collections::HashMap;

use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};
use raikiri_dom::{Document, NodeKind};

use super::dispatch::{dict_arg, fire_focus_event};
use super::events::dict_flag;
use super::interfaces::{HTML_NS, MATHML_NS, Members, SVG_NS, wrap_optional};
use super::node::{mark_dirty, write_attribute};
use super::style::ensure_flushed;
use super::webidl::{host_failure, this_document, this_element, with_state};

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// HTML "rules for parsing integers": optional ASCII whitespace, an optional
/// sign, then ASCII digits; `None` when no digit follows or it overflows.
///
/// Spec: <https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-integers>
fn parse_integer(value: &str) -> Option<i32> {
    let (negative, digits) = integer_digits(value)?;
    let magnitude: i64 = digits.parse().ok()?;
    i32::try_from(if negative { -magnitude } else { magnitude }).ok()
}

/// The sign and leading digit run of `value` under HTML's rules for
/// parsing integers, or `None` when it has no digits. A value with digits
/// is a valid integer however large it is.
fn integer_digits(value: &str) -> Option<(bool, &str)> {
    let rest = value.trim_start_matches(['\t', '\n', '\x0C', '\r', ' ']);
    let (negative, rest) = match rest.as_bytes().first() {
        Some(b'-') => (true, &rest[1..]),
        Some(b'+') => (false, &rest[1..]),
        _ => (false, rest),
    };
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (digits > 0).then(|| (negative, &rest[..digits]))
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

/// Whether `index` is an SVG or MathML `<a>` with a link, focusable by
/// default. SVG also accepts the legacy XLink `href`.
///
/// Spec: <https://svgwg.org/svg2-draft/interact.html#Focus>,
/// <https://w3c.github.io/mathml-core/#dom-and-javascript>
fn is_foreign_link(document: &Document, index: usize) -> bool {
    let href = document.element_attribute(index, "href").is_some();
    if tag_in(document, index, MATHML_NS) == Some("a") {
        return href;
    }
    tag_in(document, index, SVG_NS) == Some("a")
        && (href
            || document
                .element_attribute_ns(index, XLINK_NS, "href")
                .is_some())
}

/// The document element: the first element child of the document.
fn document_element(document: &Document) -> Option<usize> {
    document
        .get_node(document.root_index())?
        .children
        .iter()
        .copied()
        .find(|&child| {
            document
                .get_node(child)
                .is_some_and(|node| node.kind() == NodeKind::Element)
        })
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

/// Whether the element `index` is editable: its nearest HTML ancestor or
/// self with a `contenteditable` in the true, plaintext-only or false
/// state decides, and invalid values inherit.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#editing-host>
fn is_editable(document: &Document, index: usize) -> bool {
    let mut current = Some(index);
    while let Some(element) = current {
        let value =
            html_tag(document, element).and(document.element_attribute(element, "contenteditable"));
        match value {
            Some(value) if is_editing_host_value(value) => return true,
            Some(value) if value.eq_ignore_ascii_case("false") => return false,
            _ => current = document.parent_of(element),
        }
    }
    false
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
        None => {
            tag_in(document, index, SVG_NS) == Some("a")
                || tag_in(document, index, MATHML_NS) == Some("a")
        }
    };
    if zero { 0 } else { -1 }
}

/// Whether `index` is actually disabled. A `button`, `input`, `select`,
/// `textarea` or `fieldset` is disabled by its own `disabled` attribute,
/// or by a `<fieldset disabled>` ancestor unless it is inside that
/// fieldset's first `<legend>` child; an `optgroup` by its own attribute;
/// an `option` by its own or its parent `optgroup`'s.
///
/// Spec: <https://html.spec.whatwg.org/multipage/semantics-other.html#concept-element-disabled>
fn is_actually_disabled(document: &Document, index: usize) -> bool {
    let disabled = |element| document.element_attribute(element, "disabled").is_some();
    match html_tag(document, index) {
        Some("optgroup") => return disabled(index),
        Some("option") => {
            return disabled(index)
                || document.parent_of(index).is_some_and(|parent| {
                    html_tag(document, parent) == Some("optgroup") && disabled(parent)
                });
        }
        Some("button" | "input" | "select" | "textarea" | "fieldset") => {}
        _ => return false,
    }
    if disabled(index) {
        return true;
    }
    let mut child = index;
    while let Some(parent) = document.parent_of(child) {
        if html_tag(document, parent) == Some("fieldset") && disabled(parent) {
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

/// Whether `index` is in the document and neither it nor an ancestor is
/// `inert`.
fn is_connected_and_not_inert(document: &Document, index: usize) -> bool {
    let mut ancestor = Some(index);
    while let Some(current) = ancestor {
        if current == document.root_index() {
            return true;
        }
        if document.element_attribute(current, "inert").is_some() {
            return false;
        }
        ancestor = document.parent_of(current);
    }
    false
}

/// Whether the HTML, SVG or MathML element `index` can be the DOM anchor
/// of a focusable area: connected, not inert and not actually disabled.
/// Whether it is being rendered is not checked, since that needs a layout.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focusable-area>
fn can_anchor_focus(document: &Document, index: usize) -> bool {
    let html = html_tag(document, index).is_some();
    let foreign = tag_in(document, index, SVG_NS).or(tag_in(document, index, MATHML_NS));
    (html || foreign.is_some())
        && !is_actually_disabled(document, index)
        && is_connected_and_not_inert(document, index)
}

/// Whether the element `index` is focusable on its own, without layout:
/// through a valid `tabindex`, by default (an SVG link counts), or as an
/// HTML editing host.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focusable-area>
fn is_focusable(document: &Document, index: usize) -> bool {
    let attr = |name| document.element_attribute(index, name);
    attr("tabindex").and_then(integer_digits).is_some()
        || is_foreign_link(document, index)
        || focusable_by_default(document, index)
        || html_tag(document, index).is_some()
            && attr("contenteditable").is_some_and(is_editing_host_value)
            && !document
                .parent_of(index)
                .is_some_and(|parent| is_editable(document, parent))
}

/// The element children of `index`, then their descendants, in tree
/// order.
fn elements_in_tree_order(document: &Document) -> Vec<usize> {
    let mut elements = Vec::new();
    let mut stack = vec![document.root_index()];
    while let Some(index) = stack.pop() {
        let node = document.get_node(index);
        if node.is_some_and(|node| node.kind() == NodeKind::Element) {
            elements.push(index);
        }
        stack.extend(node.into_iter().flat_map(|node| node.children.iter().rev()));
    }
    elements
}

/// The `<img>` elements whose image map is the `<map>` ancestor of the
/// `<area>` element `area`. An image's `usemap` is a hash-name reference to
/// the first `<map>` in tree order whose `id` or `name` matches.
///
/// Spec: <https://html.spec.whatwg.org/multipage/image-maps.html#image-map>
fn image_map_images(document: &Document, area: usize) -> Vec<usize> {
    let mut map = document.parent_of(area);
    while let Some(candidate) = map
        && html_tag(document, candidate) != Some("map")
    {
        map = document.parent_of(candidate);
    }
    let Some(map) = map else {
        return Vec::new();
    };
    let elements = elements_in_tree_order(document);
    let mut map_names = HashMap::new();
    for &element in &elements {
        #[cfg(test)]
        tests::record_map_work(0);
        if html_tag(document, element) == Some("map") {
            for attribute in ["id", "name"] {
                if let Some(name) = document.element_attribute(element, attribute) {
                    map_names.entry(name).or_insert(element);
                }
            }
        }
    }
    elements
        .iter()
        .copied()
        .filter(|&image| {
            html_tag(document, image) == Some("img")
                && document
                    .element_attribute(image, "usemap")
                    .and_then(|usemap| usemap.split_once('#'))
                    .is_some_and(|(_, name)| {
                        if name.is_empty() {
                            return false;
                        }
                        #[cfg(test)]
                        tests::record_map_work(1);
                        map_names.get(name).copied() == Some(map)
                    })
        })
        .collect()
}

/// Make `focused` the focused element (`None` clears it), indicating focus
/// when `visible`, then fire the two `events` at `target` with `related`
/// as `relatedTarget` while focus stays there. Returns whether it still
/// does: a listener that moves focus ends the transition, since what is
/// left of it no longer holds.
fn focus_phase(
    context: &mut Context,
    (focused, visible): (Option<usize>, bool),
    target: usize,
    related: Option<usize>,
    events: [(&str, bool); 2],
) -> JsResult<bool> {
    let current = |context: &mut Context| {
        with_state(context, |s| s.host.document().focused_element() == focused)
    };
    with_state(context, |s| {
        let document = s.host.document_mut();
        document.set_focused_element(focused);
        document.set_focus_visible(visible);
    })?;
    // `:focus` and `:focus-within` change with it.
    mark_dirty(context)?;
    for (kind, bubbles) in events {
        if !current(context)? {
            return Ok(false);
        }
        fire_focus_event(context, target, kind, bubbles, related)?;
    }
    current(context)
}

/// HTML "focus update steps" for a move from the current focus to `next`
/// (`None` to clear it): with focus cleared, `blur` then `focusout` at the
/// old element; then with focus on the new one, `focus` then `focusin`
/// there. Each is a `FocusEvent` whose `relatedTarget` is the other
/// element. `visible` decides whether the new element indicates focus,
/// already while its events run; `None` keeps the indication of an
/// element that already has focus and indicates it on a new one.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focus-update-steps>
fn move_focus(context: &mut Context, next: Option<usize>, visible: Option<bool>) -> JsResult<()> {
    let previous = with_state(context, |s| s.host.document().focused_element())?;
    if previous == next {
        let changed = with_state(context, |s| {
            let document = s.host.document_mut();
            let before = document.focus_visible_element();
            if let Some(visible) = visible {
                document.set_focus_visible(visible);
            }
            document.focus_visible_element() != before
        })?;
        if changed {
            mark_dirty(context)?;
        }
        return Ok(());
    }
    if let Some(previous) = previous {
        let blur = [("blur", false), ("focusout", true)];
        let cleared = focus_phase(context, (None, true), previous, next, blur);
        if !cleared? {
            return Ok(());
        }
    }
    if let Some(next) = next {
        // A blur listener may have made the new element unfocusable.
        if previous.is_some() && !can_take_focus(context, next)? {
            return Ok(());
        }
        let focus = [("focus", false), ("focusin", true)];
        let state = (Some(next), visible.unwrap_or(true));
        let focused = focus_phase(context, state, next, previous, focus);
        focused?;
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

/// Whether the element `index` computes `visibility: visible`, after
/// bringing style up to date. An invisible box cannot take focus.
fn is_visible(context: &mut Context, index: usize) -> JsResult<bool> {
    ensure_flushed(context)?;
    let visible = with_state(context, |s| s.host.is_visible(index))?;
    visible.map_err(|error| host_failure(context, error))
}

/// Whether the element `index` has a scrollable region: a box whose
/// scrollable overflow exceeds its padding box on an axis the user can
/// scroll (`overflow` `auto` or `scroll`).
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focusable-area>
fn is_scrollable_region(context: &mut Context, index: usize) -> JsResult<bool> {
    ensure_flushed(context)?;
    let geometry = with_state(context, |s| s.host.box_geometry(index))?;
    let Some(geometry) = geometry.map_err(|error| host_failure(context, error))? else {
        return Ok(false);
    };
    let padding = geometry.padding_box;
    let axes = [
        ("overflow-x", geometry.scroll_width > padding.width),
        ("overflow-y", geometry.scroll_height > padding.height),
    ];
    for (property, overflows) in axes {
        if !overflows {
            continue;
        }
        let value = with_state(context, |s| s.host.computed_value(index, property))?;
        let value = value.map_err(|error| host_failure(context, error))?;
        if matches!(value.as_deref(), Some("auto" | "scroll")) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `index` is a focusable area that is being rendered. An
/// `<area>` is focusable through its shape in an image map, so it needs an
/// associated `<img>` that is being rendered and is not inert.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focusable-area>
fn can_take_focus(context: &mut Context, index: usize) -> JsResult<bool> {
    let (focusable, images) = with_state(context, |s| {
        let document = s.host.document();
        let images = (html_tag(document, index) == Some("area")).then(|| {
            let mut images = image_map_images(document, index);
            images.retain(|&image| is_connected_and_not_inert(document, image));
            images
        });
        let anchor = can_anchor_focus(document, index);
        (anchor.then(|| is_focusable(document, index)), images)
    })?;
    let focusable = match focusable {
        Some(true) => true,
        Some(false) => is_scrollable_region(context, index)?,
        None => false,
    };
    if !focusable {
        return Ok(false);
    }
    // An image-map shape has no box of its own; only its image must show.
    let Some(images) = images else {
        return Ok(is_visible(context, index)? && is_being_rendered(context, index)?);
    };
    for image in images {
        if is_being_rendered(context, image)? && is_visible(context, image)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The rendering update's focus fixup: clear focus, firing `blur` and
/// `focusout`, when the focused element is no longer a focusable area.
/// Leaving the document already cleared it without events. With `events`
/// false the focus is cleared silently instead. Returns whether the focus
/// needed fixing.
///
/// Spec: <https://html.spec.whatwg.org/multipage/webappapis.html#update-the-rendering>
pub(crate) fn fix_up_focus(context: &mut Context, events: bool) -> JsResult<bool> {
    let focused = with_state(context, |s| s.host.document().focused_element())?;
    let Some(focused) = focused else {
        return Ok(false);
    };
    if can_take_focus(context, focused)? {
        return Ok(false);
    }
    if events {
        move_focus(context, None, None)?;
    } else {
        with_state(context, |s| s.host.document_mut().set_focused_element(None))?;
        mark_dirty(context)?;
    }
    Ok(true)
}

/// `focus(options)`'s `FocusOptions` dictionary, converted in member
/// order: `focusVisible` (absent as `None`), then `preventScroll`, which
/// has nothing to prevent here.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#focusoptions>
fn focus_visible_option(args: &[JsValue], context: &mut Context) -> JsResult<Option<bool>> {
    let Some(options) = dict_arg(args, 0)? else {
        return Ok(None);
    };
    let visible = options.get(js_string!("focusVisible"), context)?;
    dict_flag(context, &options, "preventScroll")?;
    Ok((!visible.is_undefined()).then(|| visible.to_boolean()))
}

/// `this` as an HTML, SVG or MathML element, the interfaces that include
/// the focus members. Any other element is a `TypeError`, per WebIDL's
/// check that `this` implements the member's interface.
fn this_focus_element(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_element(this, context)?;
    let supported = with_state(context, |s| {
        matches!(
            s.host.document().element_namespace_uri(index),
            Some(HTML_NS | SVG_NS | MATHML_NS)
        )
    })?;
    if !supported {
        return Err(JsNativeError::typ()
            .with_message("'this' is not an HTML, SVG or MathML element")
            .into());
    }
    Ok(index)
}

/// `HTMLElement.focus(options)`: focus the element if it is a focusable
/// area; otherwise do nothing. An explicit `focusVisible` decides whether
/// the element indicates focus (`:focus-visible`); without one, script
/// focus always indicates it.
///
/// Spec: <https://html.spec.whatwg.org/multipage/interaction.html#dom-focus>
fn focus(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_focus_element(this, context)?;
    let visible = focus_visible_option(args, context)?;
    // Focusing the document element focuses the viewport instead.
    if with_state(context, |s| document_element(s.host.document()))? == Some(index) {
        move_focus(context, None, None)?;
        return Ok(JsValue::undefined());
    }
    if !can_take_focus(context, index)? {
        return Ok(JsValue::undefined());
    }
    move_focus(context, Some(index), visible)?;
    Ok(JsValue::undefined())
}

/// `HTMLElement.blur()`: unfocus the element if it has focus.
fn blur(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_focus_element(this, context)?;
    if with_state(context, |s| s.host.document().focused_element())? == Some(index) {
        move_focus(context, None, None)?;
    }
    Ok(JsValue::undefined())
}

/// `tabIndex`: the parsed `tabindex` attribute, else the element's
/// default.
fn tab_index(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_focus_element(this, context)?;
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
    let index = this_focus_element(this, context)?;
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
            let html = document_element(document)?;
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
