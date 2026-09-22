use crate::property::{Length, LengthOrAuto, PropertyValue};
use crate::ruletree::Origin;
use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

use super::collect::{
    CascadedDecl, MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER, MARGIN_COLLAPSING_QUIRK_SPECIFICITY,
    PRESENTATIONAL_HINT_SOURCE_ORDER, PRESENTATIONAL_HINT_SPECIFICITY,
};

/// `<img width>` / `<img height>` の HTML presentational-hint 昇格。
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
pub(crate) fn push_img_dimension_hints(elem: &impl StyleElement, decls: &mut Vec<CascadedDecl>) {
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
        return;
    }
    if let Some(width) = elem.attr("width").and_then(parse_html_dimension_value) {
        decls.push((
            PropertyValue::Width(LengthOrAuto::Length(width)),
            false,
            Origin::AuthorPresentationalHint,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
        ));
    }
    if let Some(height) = elem.attr("height").and_then(parse_html_dimension_value) {
        decls.push((
            PropertyValue::Height(LengthOrAuto::Length(height)),
            false,
            Origin::AuthorPresentationalHint,
            PRESENTATIONAL_HINT_SPECIFICITY,
            PRESENTATIONAL_HINT_SOURCE_ORDER,
        ));
    }
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
        // (`is_blank_element`, `substantial_sibling_bounds`) always
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

/// For `target_id` among `parent_id`'s direct children, whether it has no
/// substantial sibling **before** it and no substantial sibling **after**
/// it in document order (HTML LS §15.3.9's "has no substantial previous
/// siblings" / "has no substantial following siblings" conditions,
/// <https://html.spec.whatwg.org/multipage/rendering.html#margin-collapsing-quirks>).
/// Single pass over `parent_id`'s children — same shape as
/// [`super::selector_match::sibling_position`]'s combined start/end computation.
///
/// `target_id` not found among `parent_id`'s children never happens for
/// this function's only caller
/// ([`push_margin_collapsing_quirk_declarations`]): `parent_id` there is
/// always `ancestor_path.last()`, i.e. the real DOM parent
/// [`super::collect::collect_cascaded`] walked through `dom.child_ids(parent_id)` to reach
/// `target_id` in the first place — the same ancestor-path invariant
/// [`super::selector_match::sibling_position`]'s own callers rely on.
fn substantial_sibling_bounds<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    target_id: StyleNodeId,
) -> (bool, bool) {
    let mut no_substantial_before = true;
    let mut no_substantial_after = true;
    let mut seen_target = false;
    for child_id in dom.child_ids(parent_id) {
        if child_id == target_id {
            seen_target = true;
            continue;
        }
        if is_substantial_node(dom, child_id) {
            if seen_target {
                no_substantial_after = false;
            } else {
                no_substantial_before = false;
            }
        }
    }
    (no_substantial_before, no_substantial_after)
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
/// declaration into the same flat candidate list [`super::collect::collect_cascaded`]
/// already builds for this node from stylesheet rules and inline style,
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
    decls: &mut Vec<CascadedDecl>,
) {
    if quirks_mode != StyleQuirksMode::Quirks || elem.namespace_uri().is_some() {
        return;
    }
    let tag_name = elem.tag_name();
    if !is_element_with_default_margins(tag_name) {
        return;
    }
    let Some(&parent_id) = ancestor_path.last() else {
        return;
    };
    // cov:ignore: `parent_id` came from `ancestor_path`, which
    // `collect_cascaded` only ever pushes `Element`-kind ids onto (this
    // module's doc on `ancestor_path`), so `dom.node(parent_id)` resolving
    // to a node whose `as_element()` is `Some` is guaranteed by that same
    // invariant, not by anything local to this function. Defensive
    // fallback kept anyway rather than an unchecked index, same posture
    // as `sibling_position`'s own `dom.node(child_id)`/`as_element()`
    // guards over `child_ids`.
    let Some(parent_node) = dom.node(parent_id) else {
        return;
    };
    // cov:ignore: see the comment on the `dom.node(parent_id)` guard above
    // — same invariant covers this arm too.
    let Some(parent_elem) = parent_node.as_element() else {
        return;
    };
    if parent_elem.namespace_uri().is_some() {
        return;
    }
    let parent_tag = parent_elem.tag_name();
    let is_body = parent_tag.eq_ignore_ascii_case("body");
    let is_td_or_th =
        parent_tag.eq_ignore_ascii_case("td") || parent_tag.eq_ignore_ascii_case("th");
    if !is_body && !is_td_or_th {
        return;
    }

    let (no_substantial_before, no_substantial_after) =
        substantial_sibling_bounds(dom, parent_id, id);
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

    if zero_start {
        decls.push((
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(0.0))),
            false,
            Origin::UserAgent,
            MARGIN_COLLAPSING_QUIRK_SPECIFICITY,
            MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER,
        ));
    }
    if zero_end {
        decls.push((
            PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(0.0))),
            false,
            Origin::UserAgent,
            MARGIN_COLLAPSING_QUIRK_SPECIFICITY,
            MARGIN_COLLAPSING_QUIRK_SOURCE_ORDER,
        ));
    }
}

/// HTML LS "rules for parsing dimension values"
/// (<https://html.spec.whatwg.org/multipage/common-microsyntaxes.html#rules-for-parsing-dimension-values>,
/// verbatim algorithm)。
///
/// 1. 先頭の ASCII whitespace を skip。
/// 2. 直後が ASCII digit でなければ (末尾も含め) 失敗 → `None`。
/// 3. 整数部の連続 digit を集めて 10 進数として解釈。
/// 4. 直後が `.` なら、消費してから続く digit を小数部として集める
///    (`.` の直後が digit でなければ小数部なしとして扱い、位置は `.` の
///    次で確定)。
/// 5. 最終的に「数値の直後の 1 文字」で分類: `%` なら percentage
///    ([`Length::Percent`])、それ以外 (garbage でも文字列終端でも) は
///    length ([`Length::Px`])。
///
/// 数値直後の garbage は失敗にならない — `"42px"` → `Px(42.0)`、
/// `"10.5%rest"` → `Percent(10.5)`。この寛容さは
/// `embedded-content-other.html#dimension-attributes` にある**著者向け**
/// conformance 要件 ("must have values that are valid non-negative
/// integers") とは別物で、UA 側の実際の parse 規則はこちら (dimension
/// value 一般、非負整数だけでなく小数・percentage も受理) —
/// "非負整数" という短い要約だけでは誤解を招きうるため、実装はこの
/// spec 本文の algorithm に忠実にした
/// (percentage / 小数を含む)。負値を作る分岐 (`-`/`+` の読み取り) は
/// algorithm 自体に存在しないため、別途の負値拒否は不要。
pub(crate) fn parse_html_dimension_value(input: &str) -> Option<Length> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    // Infra "ASCII whitespace": TAB / LF / FF / CR / SPACE。
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
mod tests {
    use super::*;
    use crate::cascade::cascade;
    use crate::resolve::ComputedLengthPercentageOrAuto;
    use crate::ruletree::{RuleTree, build_rule_tree};
    use crate::test_dom::TestDoc;

    #[test]
    fn img_width_and_height_attributes_promoted_to_computed_style() {
        // Acceptance: `<img width="100"
        // height="50">` の HTML attribute が author CSS 無しでも computed
        // width/height に届く。
        let mut doc = TestDoc::new();
        let img =
            doc.push_element_with_attrs(0, "img", None, &[("width", "100"), ("height", "50")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(
            r.computed[img].height,
            ComputedLengthPercentageOrAuto::Px(50.0)
        );
    }

    #[test]
    fn img_width_attribute_alone_does_not_set_height() {
        // 独立 mapping — `width` だけ指定した場合 `height` は initial のまま
        // (spec は 2 属性を "respectively" と個別に mapping する)。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(0, "img", None, &[("width", "100")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(r.computed[img].height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn img_width_attribute_zero_is_a_valid_hint() {
        // 「maps to the dimension property」であり「…(ignoring zero)」では
        // ないことの check (cf. `<table width>` は ignoring-zero) — `width="0"`
        // は 0px という有効な hint になる。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(0, "img", None, &[("width", "0")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn img_width_attribute_percentage_and_decimal_accepted() {
        // "非負整数" とだけ要約すると誤解を招くが、
        // spec 本文の "rules for parsing dimension values" は percentage /
        // 小数も受理する — 実装はその本文どおり (`parse_html_dimension_value`
        // doc 参照)。
        let mut doc = TestDoc::new();
        let pct = doc.push_element_with_attrs(0, "img", None, &[("width", "50%")]);
        let dec = doc.push_element_with_attrs(0, "img", None, &[("width", "10.5")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[pct].width,
            ComputedLengthPercentageOrAuto::Percent(50.0)
        );
        assert_eq!(
            r.computed[dec].width,
            ComputedLengthPercentageOrAuto::Px(10.5)
        );
    }

    #[test]
    fn img_width_attribute_trailing_garbage_does_not_fail_parse() {
        // legacy dimension-value microsyntax の寛容さ check: 数値直後の garbage
        // は失敗にならない (`"42px"` → 42px, naive integer parse ならここで
        // 失敗していたはず)。先頭 whitespace の skip も同時に確認。
        let mut doc = TestDoc::new();
        let px = doc.push_element_with_attrs(0, "img", None, &[("width", "  42px")]);
        let pct_dot = doc.push_element_with_attrs(0, "img", None, &[("width", "10.%")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[px].width,
            ComputedLengthPercentageOrAuto::Px(42.0)
        );
        // cov:ignore: the message-format branch of this `assert_eq!` only
        // executes on failure; this assertion passes on every run, so
        // llvm-cov reports the message-string line as an uncovered added
        // line even though the assertion itself runs (same shape as
        // `counter_style.rs`'s `reserved_rule_names_are_dropped` cov:ignore).
        assert_eq!(
            r.computed[pct_dot].width,
            ComputedLengthPercentageOrAuto::Percent(10.0),
            "trailing `.` with no fractional digit still checks the following `%`"
        );
    }

    #[test]
    fn img_width_attribute_invalid_or_negative_value_produces_no_hint() {
        // 失敗 (parse failure) は「hint を作らない」に落ちる — 属性が無いのと
        // 同じ扱いで width は initial `auto` のまま。`-5` は algorithm に
        // `-`/`+` 分岐が無いため即失敗 (先頭が ASCII digit でない)。
        let mut doc = TestDoc::new();
        let garbage = doc.push_element_with_attrs(0, "img", None, &[("width", "abc")]);
        let negative = doc.push_element_with_attrs(0, "img", None, &[("width", "-5")]);
        let empty = doc.push_element_with_attrs(0, "img", None, &[("width", "")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[garbage].width,
            ComputedLengthPercentageOrAuto::Auto
        );
        assert_eq!(
            r.computed[negative].width,
            ComputedLengthPercentageOrAuto::Auto
        );
        assert_eq!(
            r.computed[empty].width,
            ComputedLengthPercentageOrAuto::Auto
        );
        // Independently check the *other* rejection layer for the empty-
        // string case: `TestDoc`'s own `TestElementRef::attr()` override
        // normalises `""` to `None` as a simplification local to that mock
        // — unlike the real `ElementRef::attr()` (`raikiri-dom::dom_impl`),
        // which returns `Some("")` for a present-but-empty attribute (see
        // `StyleElement::attr`'s trait doc). Against `TestDoc`,
        // `push_img_dimension_hints` never even calls
        // `parse_html_dimension_value` for `width=""`, so the `Auto` result
        // above is `TestDoc`-only "attribute absent" behavior here, not
        // (only) a parse-failure outcome. Against the real DOM the same
        // `Auto` result still holds, but for a different reason:
        // `attr("width")` returns `Some("")`, and
        // `parse_html_dimension_value("")` itself rejects the empty string
        // at its first-digit check (both DOM implementations agree on the
        // end result here, just not on why).
        let node = doc
            .node(StyleNodeId::new(empty as u64))
            .expect("node exists");
        let elem = node.as_element().expect("element node");
        assert_eq!(elem.attr("width"), None);
    }

    #[test]
    fn non_img_element_width_height_attributes_not_promoted() {
        // scope narrowing: mapping は `img` のみ。
        // 同じ attribute を持つ `div` は影響を受けない。
        let mut doc = TestDoc::new();
        let div =
            doc.push_element_with_attrs(0, "div", None, &[("width", "100"), ("height", "50")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(r.computed[div].height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn img_width_attribute_overridable_by_inline_author_style() {
        // Cascade-origin check: presentational hint は
        // `Origin::AuthorPresentationalHint`、inline style は `Origin::Author`
        // (`push_img_dimension_hints` doc の "Cascade origin" 節) — 別 origin
        // tier なので `cascade_rank` の rank 差だけで無条件に決着し、
        // inline style の specificity (`INLINE_SPECIFICITY` = `1 << 30`) を
        // 参照するまでもなく勝つ。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(
            0,
            "img",
            Some("width: 50px"),
            &[("width", "100"), ("height", "100")],
        );
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: message-format branch of this `assert_eq!` only
        // executes on failure (see the `pct_dot` cov:ignore above for the
        // full explanation of this line-coverage false positive).
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(50.0),
            "author inline style must override the HTML presentational hint"
        );
        // cov:ignore: same false positive, second assertion in this test.
        assert_eq!(
            r.computed[img].height,
            ComputedLengthPercentageOrAuto::Px(100.0),
            "height has no author override, so the hint still applies"
        );
    }

    #[test]
    fn img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity() {
        // Origin-rank check (`push_img_dimension_hints`
        // doc's "Cascade origin" section): the hint is
        // `Origin::AuthorPresentationalHint` (rank below `Origin::Author`
        // per `cascade_rank`), while this `* { width: 30px }` rule is a
        // real `Origin::Author` rule with zero specificity (universal
        // selector). Before this, both sides shared
        // `Origin::Author` and this exact zero-specificity/zero-source-order
        // case only resolved via `collect_cascaded`'s push-order (hint
        // pushed first, so the later-scanned real rule won the `beats` tie).
        // Now the rank difference alone decides it, independent of
        // specificity or push order — this test still pins "real author
        // rule wins regardless of specificity", just via a different
        // mechanism.
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, "* { width: 30px }");
        let img = doc.push_element_with_attrs(0, "img", None, &[("width", "100")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: same false positive as the other `img_width_attribute_*`
        // tests above (message-format branch of `assert_eq!` only executes
        // on failure).
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(30.0),
            "real author rule must win over the hint via origin rank, regardless of specificity"
        );
    }

    #[test]
    fn img_tag_name_match_is_ascii_case_insensitive() {
        // `push_img_dimension_hints` 自身の `elem.tag_name().eq_ignore_ascii_case`
        // 判定を確認 — `compound_matches` の `Component::LocalName` 判定
        // (旧名 `match_by_tag`、その後
        // `match_simple_selectors` → `compound_matches`/
        // `match_complex_selector_list` に分割 rename) と同じ寛容さの、独立
        // した別実装。real DOM (html5ever) は tag name を常に lowercase に
        // 正規化するので実運用では観測されないが、`StyleElement` は特定 DOM
        // 実装に紐付かない generic trait なので defensive に確認しておく。
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_attrs(0, "IMG", None, &[("width", "100")]);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
    }

    #[test]
    fn foreign_namespace_img_local_name_does_not_get_the_hint() {
        // The mapping is
        // HTML-namespace-specific (`push_img_dimension_hints` doc's
        // namespace-gate comment). A foreign-namespace element that merely
        // shares the local name "img" must not pick up the presentational
        // hint, even though `tag_name()` alone would match.
        let mut doc = TestDoc::new();
        let img = doc.push_element_with_namespace(
            0,
            "img",
            "http://example.com/not-html",
            &[("width", "100")],
        );
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: message-format branch of this `assert_eq!` only
        // executes on failure (same false positive as the other
        // `img_width_attribute_*` tests above).
        assert_eq!(
            r.computed[img].width,
            ComputedLengthPercentageOrAuto::Auto,
            "foreign-namespace element sharing the \"img\" local name must not get the hint"
        );
    }

    #[test]
    fn test_dom_attr_style_matches_inline_style_source_contract() {
        // `push_img_dimension_hints` は `elem.attr("width")`/`attr("height")`
        // 経由で `TestElementRef::attr()` の override (`crate::test_dom`
        // に追加済み) を叩く。`StyleElement::attr` の
        // trait doc ("Default handles `style` by delegating to
        // `inline_style_source`; overrides must preserve that contract")
        // をこの override が守っていることを直接確認する — real DOM
        // (`crates/raikiri-dom/src/dom_impl.rs`) の `ElementRef::attr()` も
        // 同じ `"style"` 特別扱いを持つので、ここが崩れると実 DOM との
        // 挙動差が生まれる。
        let mut doc = TestDoc::new();
        let id = doc.push_element(0, "div", Some("color: red"));
        let node = doc.node(StyleNodeId::new(id as u64)).expect("node exists");
        let elem = node.as_element().expect("element node");
        assert_eq!(elem.attr("style"), Some("color: red"));
        assert_eq!(elem.attr("style"), elem.inline_style_source());
    }

    #[test]
    fn parse_html_dimension_value_matches_spec_algorithm_directly() {
        // `parse_html_dimension_value` の unit-level check — 上の end-to-end
        // test 群と違い、cascade を経由せず algorithm 自体の分岐を直接叩く。
        assert_eq!(parse_html_dimension_value("100"), Some(Length::Px(100.0)));
        assert_eq!(parse_html_dimension_value("0"), Some(Length::Px(0.0)));
        assert_eq!(
            parse_html_dimension_value("50%"),
            Some(Length::Percent(50.0))
        );
        assert_eq!(parse_html_dimension_value("10.5"), Some(Length::Px(10.5)));
        assert_eq!(
            parse_html_dimension_value("10.5%"),
            Some(Length::Percent(10.5))
        );
        // 2 桁以上の小数部 — fractional-digit loop が 1 回で `break` せず
        // 「まだ digit が続く」経路 (`parse_html_dimension_value` 内 `loop`
        // の non-break iteration) を通ることを pin。1 桁小数
        // (上の `10.5` / `10.5%`) だけでは exercise されない分岐。
        assert_eq!(
            parse_html_dimension_value("12.345"),
            Some(Length::Px(12.345))
        );
        // cov:ignore: message-format branch of this `assert_eq!` only
        // executes on failure (see `img_width_attribute_trailing_garbage_
        // does_not_fail_parse`'s `pct_dot` cov:ignore for the full
        // explanation of this line-coverage false positive).
        assert_eq!(
            parse_html_dimension_value("  42px"),
            Some(Length::Px(42.0)),
            "leading whitespace skipped, trailing garbage after the number ignored"
        );
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("10.%"),
            Some(Length::Percent(10.0)),
            "trailing `.` with no fractional digit still advances past it before the % check"
        );
        assert_eq!(parse_html_dimension_value("abc"), None);
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("-5"),
            None,
            "no sign branch in the algorithm"
        );
        assert_eq!(parse_html_dimension_value(""), None);
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("   "),
            None,
            "whitespace-only input never reaches a digit"
        );
        // cov:ignore: same false positive.
        assert_eq!(
            parse_html_dimension_value("."),
            None,
            "a lone `.` is not a leading digit"
        );
    }

    fn margin_quirk_ua_tree(css: &str) -> RuleTree {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::UserAgent);
        tree
    }

    #[test]
    fn margin_collapsing_quirk_zeroes_start_for_first_child_of_body() {
        // `<body><p>text</p></body>`, quirks mode: `p` is the first
        // (only) child of `body` and has no substantial previous
        // siblings → rule 1 zeroes margin-top. `p` is not blank (has a
        // substantial text child), so rule 2 does not also zero
        // margin-bottom.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_zeroes_both_sides_when_blank() {
        // `<body><p></p></body>` — `p` is additionally blank (no
        // substantial children at all) → rule 2 also zeroes
        // margin-bottom.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element(body, "p", None);

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_does_not_apply_outside_quirks_mode() {
        // Same structure as the first-child test above, but each of the
        // two non-`Quirks` `StyleQuirksMode` states — the quirk is gated
        // on full quirks mode only. `LimitedQuirks` gets its own case
        // (not folded into `NoQuirks`) because it's a DOM Standard dfn
        // distinct from full quirks mode
        // (`crate::style_dom::StyleQuirksMode`'s own doc draws the same
        // distinction, and `class_selector_case_sensitivity_across_quirks_modes`
        // above pins the equivalent distinction for selector matching) —
        // a broken gate that folded `LimitedQuirks` in with `Quirks`
        // would not be caught by testing `NoQuirks` alone.
        for mode in [StyleQuirksMode::NoQuirks, StyleQuirksMode::LimitedQuirks] {
            let mut doc = TestDoc::new();
            doc.quirks_mode = mode;
            let body = doc.push_element(0, "body", None);
            let p = doc.push_element(body, "p", None);
            doc.push_text(p, "text");

            let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
            let r = cascade(&doc, &tree).expect("cascade Ok");
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                r.computed[p].margin.top,
                ComputedLengthPercentageOrAuto::Px(16.0),
                "{mode:?} must not trigger margin-collapsing-quirks zeroing"
            );
        }
    }

    #[test]
    fn margin_collapsing_quirk_substantial_text_sibling_blocks_zeroing() {
        // `<body>Hello<p>text</p></body>` — `p` IS CSS's `:first-child`
        // (no earlier *element* sibling), but HTML LS denies it "no
        // substantial previous siblings" because the text node "Hello" is
        // substantial (non-inter-element-whitespace). A selector-based UA
        // rule built on `:first-child` would zero this incorrectly; the
        // dedicated structural predicate must not.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_text(body, "Hello");
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a substantial (non-whitespace) previous text sibling must block zeroing"
        );
    }

    #[test]
    fn margin_collapsing_quirk_whitespace_only_previous_sibling_does_not_block_zeroing() {
        // `<body>   <p>text</p></body>` — the leading text node is
        // whitespace-only (inter-element whitespace per HTML LS §3.2.5),
        // so it is not substantial and must not block "no substantial
        // previous siblings".
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_text(body, "   \t\n");
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_comment_previous_sibling_does_not_block_zeroing() {
        // `<body><!--c--><p>text</p></body>` — comment nodes are never
        // substantial (HTML LS §15.3.9's "substantial" dfn only counts
        // text/element nodes).
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_comment(body, "c");
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_rule3_zeroes_start_for_blank_last_child_of_td() {
        // `<td>Hello<pre></pre></td>` — `pre` has a substantial previous
        // sibling ("Hello", rules 1/2 do not apply) but is the last
        // (only trailing) child of `td` and is blank → rule 3 zeroes
        // margin-top. `pre` (not `p`) is used deliberately so rule 4
        // (which only ever fires for `p`) cannot also zero margin-bottom
        // here, keeping rule 3 isolated.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let td = doc.push_element(0, "td", None);
        doc.push_text(td, "Hello");
        let pre = doc.push_element(td, "pre", None);

        let tree = margin_quirk_ua_tree("pre { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[pre].margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[pre].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "rule 3 only zeroes margin-block-start, not margin-block-end"
        );
    }

    #[test]
    fn margin_collapsing_quirk_rule4_zeroes_end_for_p_last_child_of_td_even_when_not_blank() {
        // `<td>Hello<p>text</p></td>` — `p` has a substantial previous
        // sibling (rules 1/2 do not apply) and is not blank (rule 3 does
        // not apply either, it requires blank), but is `p` specifically
        // and has no substantial following sibling → rule 4 zeroes
        // margin-bottom regardless of blankness.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let td = doc.push_element(0, "td", None);
        doc.push_text(td, "Hello");
        let p = doc.push_element(td, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "no rule zeroes margin-top here"
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_th_parent_triggers_rule4_same_as_td() {
        // Same fixture as
        // `margin_collapsing_quirk_rule4_zeroes_end_for_p_last_child_of_td_even_when_not_blank`
        // but with a `th` container instead of `td` — every other test in
        // this group uses `td` for the `td`/`th`-only rules (3 and 4), so
        // this pins that the `th` half of that `is_td_or_th` check is
        // exercised too.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let th = doc.push_element(0, "th", None);
        doc.push_text(th, "Hello");
        let p = doc.push_element(th, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(0.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_container_that_is_not_body_td_th_does_not_gate_at_all() {
        // `<div><p>text</p></div>` — `p` is the first (only) child of a
        // `div`, which is none of `body`/`td`/`th`, so none of the four
        // rules can apply (rules 1/2 require a `body`/`td`/`th` parent;
        // rules 3/4 require `td`/`th` specifically) — the
        // `!is_body && !is_td_or_th` early-return path.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let div = doc.push_element(0, "div", None);
        let p = doc.push_element(div, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a div container (neither body nor td/th) must never trigger any \
             margin-collapsing-quirks rule"
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_rules_3_and_4_are_td_th_only_not_body() {
        // `<body>Hello<p></p></body>` — `p` is blank and has no
        // substantial following sibling, which would trigger rule 3 (if
        // blank+trailing were enough regardless of parent) or rule 4 (if
        // it applied to any parent) — but rules 3/4 are gated on a
        // `td`/`th` parent specifically, and `body` must not qualify.
        // Rule 1/2 also don't apply here (substantial previous sibling
        // "Hello"), so nothing should be zeroed.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        doc.push_text(body, "Hello");
        let p = doc.push_element(body, "p", None);

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
        assert_eq!(
            r.computed[p].margin.bottom,
            ComputedLengthPercentageOrAuto::Px(16.0)
        );
    }

    #[test]
    fn margin_collapsing_quirk_author_declaration_still_overrides_zeroing() {
        // The spec frames this as a real UA-origin stylesheet rule
        // participating in normal cascade, not an unconditional override
        // — a real author declaration (any specificity) must still be
        // able to give the element a nonzero margin.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "p { margin-top: 16px; margin-bottom: 16px; }",
            Origin::UserAgent,
        );
        tree.add_stylesheet("p { margin-top: 5px; }", Origin::Author);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(5.0),
            "a real author declaration must beat the quirks zeroing regardless of \
             its specificity, since Origin::Author always outranks Origin::UserAgent"
        );
    }

    #[test]
    fn margin_collapsing_quirk_does_not_apply_to_elements_outside_default_margin_list() {
        // `<body><div>text</div></body>` — `div` is not one of HTML LS
        // §15.3.9's 17 "elements with default margins", so the quirk
        // must never fire for it even though it would otherwise satisfy
        // every structural condition rule 1 checks.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let div = doc.push_element(body, "div", None);
        doc.push_text(div, "text");

        let tree = margin_quirk_ua_tree("div { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[div].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "div is not an \"element with default margins\" — must be unaffected"
        );
    }

    #[test]
    fn margin_collapsing_quirk_foreign_namespace_element_is_not_gated() {
        // Same shape as `push_img_dimension_hints`'s own namespace-gate
        // regression (`foreign_namespace_img_local_name_does_not_get_the_hint`
        // above) — a foreign-namespace element that merely shares the
        // local name "p" must not pick up the quirks zeroing, even though
        // it otherwise satisfies every structural condition rule 1
        // checks (first child of `body`, no substantial previous
        // sibling).
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element(0, "body", None);
        let p = doc.push_element_with_namespace(body, "p", "http://example.com/not-html", &[]);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a foreign-namespace element sharing the local name \"p\" must not \
             get the quirks zeroing"
        );
    }

    #[test]
    fn margin_collapsing_quirk_foreign_namespace_parent_is_not_gated() {
        // Mirror of the previous test on the *parent* side: a
        // foreign-namespace element sharing the local name "body" must
        // not count as the real HTML `body` this quirk is scoped to,
        // even though the child otherwise satisfies every structural
        // condition rule 1 checks.
        let mut doc = TestDoc::new();
        doc.quirks_mode = StyleQuirksMode::Quirks;
        let body = doc.push_element_with_namespace(0, "body", "http://example.com/not-html", &[]);
        let p = doc.push_element(body, "p", None);
        doc.push_text(p, "text");

        let tree = margin_quirk_ua_tree("p { margin-top: 16px; margin-bottom: 16px; }");
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].margin.top,
            ComputedLengthPercentageOrAuto::Px(16.0),
            "a foreign-namespace container sharing the local name \"body\" must \
             not count as the real body/td/th for the quirk"
        );
    }
}
