use std::borrow::Cow;
use std::collections::HashMap;

use crate::error::CascadeError;
use crate::property::{Length, LengthOrAuto, PropertyValue};
use crate::rule::Declaration;
use crate::ruletree::Origin;
use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

use super::candidate::{CandidateSink, Precedence};
use super::collect::{MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER, MARGIN_COLLAPSING_QUIRK_SPECIFICITY};

/// Promote HTML `<img width>` / `<img height>` to presentational hints.
///
/// # Spec mapping (verbatim, live HTML Standard)
///
/// Map HTML `img` `width` and `height` attributes to presentational hints,
/// following HTML Living Standard §15.4.3:
/// <https://html.spec.whatwg.org/multipage/rendering.html#dimRendering>.
/// Values use the HTML dimension-value parser; invalid attributes produce no
/// hint, while zero is a valid value.
///
/// The hints use CSS Cascading Level 5's author-presentational-hint origin:
/// <https://drafts.csswg.org/css-cascade-5/#preshint>. They lose to normal
/// author declarations and beat normal user-origin declarations. This helper
/// handles `img` dimensions only; `aspect-ratio` and other element mappings
/// are outside its scope.
pub(crate) fn push_img_dimension_hints(
    elem: &impl StyleElement,
    sink: &mut CandidateSink<'_>,
) -> Result<(), CascadeError> {
    // HTML-namespace gate:
    // this mapping is HTML LS's own presentational hint, scoped to the HTML
    // namespace — a foreign-namespace element that merely shares the local
    // name "img" (SVG has no `img` element today, but `StyleElement` is a
    // generic trait not tied to any one DOM/parser, so this stays
    // defensive rather than relying on "SVG doesn't currently define one").
    // `namespace_uri()` returns `None` for the HTML default namespace
    // (`style_dom.rs`'s "optimized path" doc) — same check/shape as
    // `ruletree.rs`'s `<template>` HTML-only gate
    // (`tag.eq_ignore_ascii_case("template") && elem.namespace_uri().is_none()`)
    // and the same principle applies to attribute-selector matching.
    if !elem.tag_name().eq_ignore_ascii_case("img") || elem.namespace_uri().is_some() {
        return Ok(());
    }
    if let Some(width) = elem.attr("width").and_then(parse_html_dimension_value) {
        sink.push_hint(PropertyValue::Width(LengthOrAuto::Length(width)))?;
    }
    if let Some(height) = elem.attr("height").and_then(parse_html_dimension_value) {
        sink.push_hint(PropertyValue::Height(LengthOrAuto::Length(height)))?;
    }
    Ok(())
}

/// The "elements with default margins" HTML LS §15.3.9 "Margin collapsing
/// quirks"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>)
/// names by an explicit list, verbatim: blockquote, dir, dl, h1, h2, h3,
/// h4, h5, h6, listing, menu, ol, p, plaintext, pre, ul, xmp (17 elements).
///
/// `eq_ignore_ascii_case`, not plain `==` — matched against a *hardcoded
/// literal* set, not against another DOM-sourced tag name (unlike
/// [`super::selector_match::sibling_position`]'s sibling-vs-sibling comparison, whose "html5ever
/// already normalises" reasoning only covers two DOM-sourced tag names
/// meeting each other). Nothing in [`StyleElement::tag_name`]'s own
/// contract requires lower-casing — the crate's own test-only `TestDoc`
/// mock explicitly does not lower-case — so this stays defensive the
/// same way the other same-file examples of
/// "DOM tag name vs. hardcoded literal" already are:
/// `elem.tag_name().eq_ignore_ascii_case("img")`
/// ([`push_img_dimension_hints`]) and
/// `tag.eq_ignore_ascii_case("template")` (`ruletree.rs`'s `<template>`
/// gate).
fn is_element_with_default_margins(tag_name: &str) -> bool {
    const NAMES: &[&str] = &[
        "blockquote",
        "dir",
        "dl",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "listing",
        "menu",
        "ol",
        "p",
        "plaintext",
        "pre",
        "ul",
        "xmp",
    ];
    NAMES.iter().any(|name| tag_name.eq_ignore_ascii_case(name))
}

/// HTML LS §15.3.9 defines a node as "substantial" if it is a text node
/// that is not [inter-element
/// whitespace](https://html.spec.whatwg.org/multipage/dom.html#inter-element-whitespace),
/// or if it is an element node.
///
/// "Inter-element whitespace" (HTML LS §3.2.5 "Content models", same URL
/// as above, verbatim): "Empty Text nodes and Text nodes consisting of
/// just \[...\] ASCII whitespace \[...\] are considered inter-element
/// whitespace" — "ASCII whitespace" itself links to Infra's
/// <https://infra.spec.whatwg.org/#ascii-whitespace> dfn there, the
/// 5-character set {tab, LF, FF, CR, space}. Same set this crate already
/// uses for HTML attribute-value tokenisation (`class_token_matches` in
/// [`crate::style_dom`]) and dimension-value leading-whitespace skipping
/// ([`parse_html_dimension_value`]).
///
/// This is deliberately a *different* character set from CSS Selectors
/// L4's `:empty` "document white space characters" ([`super::selector_match::matches_empty`]
/// doc) — that set is {space, tab, segment break/LF} and excludes form
/// feed; HTML LS's "ASCII whitespace" includes form feed and carriage
/// return too. The two predicates ([`super::selector_match::matches_empty`] for `:empty`, this
/// function's caller [`is_blank_element`] for HTML LS "blank") are
/// spec-distinct features and must not share one whitespace set even
/// though they look similar.
///
/// Comment / processing-instruction / document-fragment / document nodes
/// are none of "text node" or "element node", so they are never
/// substantial — matching how [`super::selector_match::matches_empty`] treats the same kinds as
/// not affecting `:empty` (verbatim spec text quoted on that function:
/// "comments, processing instructions, and other nodes must not affect").
pub(crate) fn is_substantial_node<D: StyleDom>(dom: &D, id: StyleNodeId) -> bool {
    match dom.node(id) {
        Some(node) => match node.kind() {
            StyleNodeKind::Element => true,
            StyleNodeKind::Text => !node
                .text_content()
                .unwrap_or("")
                .chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')),
            StyleNodeKind::Comment
            | StyleNodeKind::ProcessingInstruction
            | StyleNodeKind::DocumentFragment
            | StyleNodeKind::Document => false,
        },
        // cov:ignore: `child_ids` only ever yields ids that `dom.node`
        // resolves (`StyleDom` trait doc: "child_ids(id) returns an empty
        // iterator for invalid id", implying ids it does yield are
        // valid) — same defensive posture as `matches_empty`'s own `None
        // => true` fallback. This function's only two call sites
        // (`is_blank_element`, `SubstantialBounds::scan`) always
        // derive `id` from `dom.child_ids`.
        None => false,
    }
}

/// HTML LS §15.3.9 defines an element as "blank" if it "contains no
/// substantial nodes"
/// (<https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>).
///
/// Only `elem_id`'s **direct** children need checking, not the full
/// descendant subtree: [`is_substantial_node`] already counts *any*
/// element node as substantial regardless of what that element itself
/// contains, so the moment `elem_id` has one element child anywhere in its
/// subtree, that child is itself either a substantial direct child, or
/// the child leading to it is — "does any direct child qualify" and "does
/// any descendant at any depth qualify" always agree for this particular
/// definition of substantial.
fn is_blank_element<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> bool {
    !dom.child_ids(elem_id)
        .any(|child_id| is_substantial_node(dom, child_id))
}

/// The first and last substantial child of one parent, keyed by the parent.
///
/// HTML LS §15.3.9's "has no substantial previous siblings" / "has no
/// substantial following siblings" conditions
/// (<https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>)
/// only ever ask about an element child, which is itself substantial
/// ([`is_substantial_node`]). So an element has no substantial sibling
/// before it exactly when it is its parent's first substantial child, and
/// none after it exactly when it is the last one. Recording those two ids
/// once per parent turns a scan of every sibling per child (quadratic in
/// the number of children of a `body`/`td`/`th`) into one scan per parent.
///
/// Entries are only valid while the DOM is unchanged, which holds for one
/// cascade walk (the walk borrows the DOM immutably).
pub(crate) type SubstantialBoundsCache = HashMap<StyleNodeId, SubstantialBounds>;

/// See [`SubstantialBoundsCache`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SubstantialBounds {
    first: Option<StyleNodeId>,
    last: Option<StyleNodeId>,
}

impl SubstantialBounds {
    /// One pass over `parent_id`'s direct children.
    fn scan<D: StyleDom>(dom: &D, parent_id: StyleNodeId) -> Self {
        let mut bounds = Self::default();
        for child_id in dom.child_ids(parent_id) {
            if is_substantial_node(dom, child_id) {
                bounds.first.get_or_insert(child_id);
                bounds.last = Some(child_id);
            }
        }
        bounds
    }
}

/// For `target_id` among `parent_id`'s direct children, whether it has no
/// substantial sibling **before** it and no substantial sibling **after**
/// it in document order (HTML LS §15.3.9's "has no substantial previous
/// siblings" / "has no substantial following siblings" conditions,
/// <https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>).
///
/// `target_id` must be a substantial child of `parent_id` (an element);
/// see [`SubstantialBoundsCache`] for why the parent's first and last
/// substantial child then answer both questions. The only caller
/// ([`push_margin_collapsing_quirk_declarations`]) passes an element whose
/// `parent_id` is `ancestor_path.last()`, i.e. the real DOM parent
/// [`super::inherit::walk_from`] walked through `dom.child_ids(parent_id)` to
/// reach `target_id` in the first place.
fn substantial_sibling_bounds<D: StyleDom>(
    dom: &D,
    cache: &mut SubstantialBoundsCache,
    parent_id: StyleNodeId,
    target_id: StyleNodeId,
) -> (bool, bool) {
    let bounds = *cache
        .entry(parent_id)
        .or_insert_with(|| SubstantialBounds::scan(dom, parent_id));
    (
        bounds.first == Some(target_id),
        bounds.last == Some(target_id),
    )
}

/// Margin-collapsing quirks (HTML LS §15.3.9 "Margin collapsing quirks",
/// <https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>
///
///
/// Four verbatim rules, all gated on [`StyleQuirksMode::Quirks`] (the DOM
/// Standard's full "quirks mode", distinct from "limited-quirks mode" —
/// same distinction [`crate::style_dom::StyleQuirksMode`]'s own doc
/// draws):
///
/// 1. Any "element with default margins"
///    ([`is_element_with_default_margins`]) that is the child of a
///    `body`/`td`/`th` element and has no substantial previous siblings:
///    `margin-block-start` (this crate's `margin-top` — no writing-mode
///    support, so physical == logical unconditionally here, same posture
///    minimal.css's own `margin-block` substitution already takes)
///    zeroed.
/// 2. Same as 1, plus the element is "blank" ([`is_blank_element`]):
///    `margin-block-end` (`margin-bottom`) also zeroed.
/// 3. Any such element that is the child of a `td`/`th` element, has no
///    substantial following siblings, and is blank: `margin-block-start`
///    zeroed.
/// 4. Any `p` that is the child of a `td`/`th` element and has no
///    substantial following siblings: `margin-block-end` zeroed.
///
/// # Why this enters cascade as a candidate declaration rather than
/// overriding the computed value directly
///
/// The spec frames all four rules as "is expected to have a user-agent
/// level style sheet rule that sets \[...\] to zero" — i.e. a real UA
/// stylesheet declaration participating in normal cascade, not an
/// unconditional override. Concretely: real author CSS (any specificity,
/// `Origin::Author`) must still be able to give the element a nonzero
/// margin again; zeroing the resolved computed value after cascade
/// (bypassing origin/specificity entirely) would incorrectly clobber
/// that. So this function pushes an [`Origin::UserAgent`] candidate
/// declaration into the same candidate list [`super::collect::Collector::collect`]
/// builds for this node from stylesheet rules and inline style,
/// and lets the normal [`super::collect::pick_winners`]/[`super::collect::beats`] machinery decide —
/// [`super::collect::cascade_rank`] guarantees any `Origin::Author` declaration for the
/// same property outranks this regardless of specificity. See
/// [`MARGIN_COLLAPSING_QUIRK_SPECIFICITY`]'s own doc for why this
/// declaration's specificity still needs to be chosen carefully (to
/// out-rank *other* `Origin::UserAgent` declarations for the same
/// property, e.g. minimal.css's default-margin rule).
///
/// # Why the structural predicates can't be plain CSS selectors
///
/// "No substantial previous/following sibling" counts text-node content
/// (ignoring only inter-element whitespace), which CSS Selectors L4's
/// `:first-child`/`:last-child` do not — those ignore *all* non-element
/// siblings regardless of text content ([`super::selector_match::sibling_position`] doc,
/// verbatim: "Standalone text and other non-element nodes are not counted
/// \[...\]"). A document like `<body>Hello<p>...</p></body>` has `<p>` as
/// CSS's `:first-child` (no earlier *element* sibling) but HTML LS denies
/// it "no substantial previous siblings" (the text "Hello" is
/// substantial) — a selector-based UA rule built on `:first-child` would
/// zero this `<p>`'s margin-block-start incorrectly. Hence the dedicated
/// structural predicates ([`is_substantial_node`],
/// [`substantial_sibling_bounds`], [`is_blank_element`]), computed
/// directly against the [`StyleDom`] tree rather than expressed as
/// selector components.
///
/// # HTML-namespace gate
///
/// Both `elem` and its `body`/`td`/`th` container are gated on
/// `namespace_uri().is_none()` — same posture
/// [`push_img_dimension_hints`]'s own doc explains for the same reason:
/// HTML LS's rendering rules (§15.2's own `@namespace
/// "http://www.w3.org/1999/xhtml";` scoping, which §15.3.9 falls under)
/// are scoped to the HTML namespace, and [`StyleElement`] is a generic
/// trait not tied to any one DOM/parser, so this stays defensive rather
/// than relying on "no foreign-namespace vocabulary defines an element
/// that happens to share one of these local names today".
pub(crate) fn push_margin_collapsing_quirk_declarations<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    elem: &impl StyleElement,
    ancestor_path: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    bounds_cache: &mut SubstantialBoundsCache,
    sink: &mut CandidateSink<'_>,
) -> Result<(), CascadeError> {
    if quirks_mode != StyleQuirksMode::Quirks || elem.namespace_uri().is_some() {
        return Ok(());
    }
    let tag_name = elem.tag_name();
    if !is_element_with_default_margins(tag_name) {
        return Ok(());
    }
    let Some(&parent_id) = ancestor_path.last() else {
        return Ok(());
    };
    // cov:ignore: `parent_id` came from `ancestor_path`, which
    // the cascade walk only ever pushes `Element`-kind ids onto (this
    // module's doc on `ancestor_path`), so `dom.node(parent_id)` resolving
    // to a node whose `as_element()` is `Some` is guaranteed by that same
    // invariant, not by anything local to this function. Defensive
    // fallback kept anyway rather than an unchecked index, same posture
    // as `sibling_position`'s own `dom.node(child_id)`/`as_element()`
    // guards over `child_ids`.
    let Some(parent_node) = dom.node(parent_id) else {
        return Ok(());
    };
    // cov:ignore: see the comment on the `dom.node(parent_id)` guard above
    // — same invariant covers this arm too.
    let Some(parent_elem) = parent_node.as_element() else {
        return Ok(());
    };
    if parent_elem.namespace_uri().is_some() {
        return Ok(());
    }
    let parent_tag = parent_elem.tag_name();
    let is_body = parent_tag.eq_ignore_ascii_case("body");
    let is_td_or_th =
        parent_tag.eq_ignore_ascii_case("td") || parent_tag.eq_ignore_ascii_case("th");
    if !is_body && !is_td_or_th {
        return Ok(());
    }

    let (no_substantial_before, no_substantial_after) =
        substantial_sibling_bounds(dom, bounds_cache, parent_id, id);
    let is_blank = is_blank_element(dom, id);

    let mut zero_start = false;
    let mut zero_end = false;

    // Rules 1 + 2: body/td/th child, no substantial previous sibling.
    if no_substantial_before {
        zero_start = true;
        if is_blank {
            zero_end = true;
        }
    }
    // Rule 3: td/th child, no substantial following sibling, blank.
    if is_td_or_th && no_substantial_after && is_blank {
        zero_start = true;
    }
    // Rule 4: p child of td/th, no substantial following sibling.
    if is_td_or_th && tag_name.eq_ignore_ascii_case("p") && no_substantial_after {
        zero_end = true;
    }

    let mut quirk = |value| {
        sink.push_local(
            Cow::Owned(Declaration::new(value, false)),
            Precedence::new(
                Origin::UserAgent,
                false,
                MARGIN_COLLAPSING_QUIRK_SPECIFICITY,
                MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER,
                crate::layer::LayerPosition::default(),
            ),
        )
    };
    if zero_start {
        quirk(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            0.0,
        ))))?;
    }
    if zero_end {
        quirk(PropertyValue::MarginBottom(LengthOrAuto::Length(
            Length::Px(0.0),
        )))?;
    }
    Ok(())
}

/// HTML LS "rules for parsing dimension values"
/// (<https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-dimension-values>,
/// verbatim algorithm).
///
/// 1. Skip leading ASCII whitespace.
/// 2. If the next character is not an ASCII digit (including at end of input),
///    fail with `None`.
/// 3. Collect consecutive digits for the integer part and interpret in base 10.
/// 4. If the next character is `.`, consume it and collect subsequent digits
///    for the fractional part. If no digit follows `.`, leave the fractional
///    part empty and continue from the position after `.`.
/// 5. Classify by the character immediately after the number: `%` means a
///    percentage ([`Length::Percent`]); anything else, including garbage or
///    end of input, means a length ([`Length::Px`]).
///
/// Garbage following the number does not cause failure: `"42px"` gives
/// `Px(42.0)`, and `"10.5%rest"` gives `Percent(10.5)`. This permissive UA
/// parsing rule differs from the *author-facing* conformance requirement
/// ("must have values that are valid non-negative integers") in
/// `embedded-content-other.html#dimension-attributes`: dimension values can
/// include decimals and percentages. A "non-negative integers" summary would
/// mislead, so this implementation follows the full spec algorithm. The
/// algorithm has no branch consuming `-` or `+`, so negative values need no
/// separate rejection step.
pub(crate) fn parse_html_dimension_value(input: &str) -> Option<Length> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    // Infra "ASCII whitespace": TAB / LF / FF / CR / SPACE.
    while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b'\n' | b'\x0C' | b'\r') {
        pos += 1;
    }
    if pos >= bytes.len() || !bytes[pos].is_ascii_digit() {
        return None;
    }
    let mut value: f64 = 0.0;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        value = value * 10.0 + f64::from(bytes[pos] - b'0');
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'.' {
        pos += 1;
        if pos < bytes.len() && bytes[pos].is_ascii_digit() {
            let mut divisor: f64 = 1.0;
            loop {
                divisor *= 10.0;
                value += f64::from(bytes[pos] - b'0') / divisor;
                pos += 1;
                if pos >= bytes.len() || !bytes[pos].is_ascii_digit() {
                    break;
                }
            }
        }
    }
    Some(if pos < bytes.len() && bytes[pos] == b'%' {
        Length::Percent(value as f32)
    } else {
        Length::Px(value as f32)
    })
}

#[cfg(test)]
mod tests;
