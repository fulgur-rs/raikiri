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
/// HTML Living Standard §15.4.3 "Attributes for embedded content and
/// images" (<https://html.spec.whatwg.org/multipage/rendering.html#dimRendering>):
///
/// > The `width` and `height` attributes on an `img` element's dimension
/// > attribute source map to the dimension properties 'width' and 'height'
/// > on the `img` element respectively.
///
/// "maps to the dimension property" (**not** "…(ignoring zero)" — cf. e.g.
/// `<table width>`) は
/// <https://html.spec.whatwg.org/multipage/rendering.html#maps-to-the-dimension-property>
/// が定義: 属性値を [`parse_html_dimension_value`] (HTML LS "rules for
/// parsing dimension values") で parse し、失敗しなければその結果を
/// presentational hint の値として使う。`0` は "ignoring zero" ではないため
/// 有効な hint 値になる。parse 失敗時は hint 自体を作らない (cascade 的には
/// 属性が存在しないのと同じ — 他の source があればそれが勝ち、無ければ
/// property の initial value `auto` のまま)。
///
/// # Non-goals (spec が定義するがこの関数が扱わないこと)
///
/// - **`aspect-ratio` mapping**: 同じ spec 段落が続けて "They similarly map
///   to the aspect-ratio property (using dimension rules) of the `img`
///   element" と述べるが、raikiri-style は `aspect-ratio` property を
///   まだ実装していない ([`crate::property::PropertyValue`] に該当 variant
///   なし) — mapping 先が存在しないので実装しようがない、spec 逸脱ではなく
///   「まだ生えていない property への言及」。
/// - **`dimension attribute source` 間接**:
///   <https://html.spec.whatwg.org/multipage/embedded-content.html#concept-img-dimension-attribute-source>
///   は "initially set to the element itself" で、`<picture>`/`srcset`
///   選択があった場合のみ選ばれた `<source>` 側に切り替わる。raikiri は
///   `<picture>` source 選択を未実装なので、この関数は常に `img` 要素自身の
///   属性を読む — 上記 default と一致する straightforward な subset。
/// - **`embed` / `iframe` / `object` / `video` / `input[type=image]`**: 同じ
///   spec 段落の後続文が他要素にも同じ mapping を適用するが、本関数の
///   scope narrowing は `img` のみに限定している (最小実装、将来の拡張の
///   土台という位置づけ)。
///
/// # Cascade origin (`Origin::AuthorPresentationalHint` へ retag 済み、
/// spec text 再確認済み)
///
/// CSS Cascading L5 §6.5 "Precedence of Non-CSS Presentational Hints"
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>, verbatim) はこの種の
/// hint を "a special-purpose author presentational hint origin between the
/// regular user origin and the author origin" — user origin と author
/// origin の間に位置する独立 origin — に置くことを定め、"Presentational
/// hints entering the cascade as author presentational hint origin rules
/// can be overridden by author-origin styles, but not by non-important
/// user-origin styles" と続ける。本関数は専用 variant
/// [`Origin::AuthorPresentationalHint`] を採る — [`super::collect::cascade_rank`] はこれを
/// `(User, false) => 1` より上、`(Author, false) => 3` より下に置く
/// ([`Origin::User`] 挿入後の値 — [`super::collect::cascade_rank`] doc
/// 参照)。この rank 差は `beats` の tuple compare `(rank, specificity, source_order)` の
/// **第一要素**なので、真の UA-origin rule には specificity/source_order を
/// 問わず常に勝ち、real author-origin 宣言 (stylesheet rule でも inline
/// style でも) には specificity/source_order を問わず常に負ける — かつて
/// (retag 前、旧版は hint も real 宣言も同じ `Origin::Author`
/// に tag していた) は後者の保証を「hint の specificity を 0 に固定し、
/// real 宣言が zero-specificity かつ stylesheet 先頭 rule の場合に限り
/// 発生する exact tie を push 順序 (hint を先に push) で決着させる」という
/// 同一 origin 内 tie-break に依存していた — 3rd tier 導入によりその依存は
/// 解消され、origin rank だけで無条件に決着する。[`Origin::User`] の挿入は
/// この結論を変えない — hint の rank は挿入後も
/// 依然として real `Author` rank と等しくなることが無い (`AuthorPresentationalHint`
/// と `Author` は常に隣接する別 rank 値のまま、[`super::collect::cascade_rank`] doc の rank
/// 表参照) ため、[`super::collect::collect_cascaded`] が今も stylesheet rule matching /
/// inline style より先にこの関数を push する呼び出し順は残っているが、
/// 上記の通りもう correctness の必要条件ではない (無害な残置、re-verify 済み)。
///
/// テスト
/// `img_width_attribute_overridable_by_author_stylesheet_regardless_of_specificity`
/// はこの「specificity を問わず real author 宣言が勝つ」性質を、かつては
/// exact-tie 経由で、今は origin rank 差で直接 exercise する ([`Origin::User`]
/// 挿入後も rank 差の大小関係は変わらないため、この test は無変更で check
/// し続ける)。
///
/// `Origin::User` no-producer 残差の解消: raikiri-style 内の
/// [`Origin::User`] variant 自体を追加した時点 ([`super::collect::cascade_rank`] の 4-tier
/// 化) では、consumer が渡す `extra_stylesheets` を実際に [`Origin::User`]
/// へ route する producer がまだ無く、今も `StylesheetKind::Author` 経由で
/// [`Origin::Author`] として届いていた。spec の完全な順序では hint は
/// 「user origin より強い」はずだが、当時の実装は user stylesheet 宣言を
/// hint より強い [`Origin::Author`] rank に一律 fold していた — user
/// stylesheet が img の width/height を上書きできるという結果自体は spec と
/// 一致していたが (`Author` rank は hint より常に上)、独立した User origin
/// へ実際に route されていない分、モデルの精度としては不完全だった。
///
/// raikiri-traits 側の `StylesheetKind` に独立
/// `User` variant を追加し raikiri-html で retag、umbrella 側の
/// `stylesheet_kind_to_origin` を拡張する genuine multi-crate diff
/// (raikiri-style 単体では完結しない) が着地し、この残差は解消された —
/// `extra_stylesheets` は今は実際に [`Origin::User`] へ route される。**normal
/// 宣言同士なら** user stylesheet の宣言はもう [`Origin::Author`] rank に
/// fold されず、spec 通り hint ([`Origin::AuthorPresentationalHint`]、normal
/// rank 2) より弱い ([`Origin::User`] normal rank 1) — つまり `<img width>`
/// hint は今や normal な `extra_stylesheets` 由来の宣言に specificity を
/// 問わず勝つ (real author-origin 宣言、たとえば in-document `<style>`、には
/// normal 同士なら今も負ける — umbrella crate の `build_cascaded` doc の
/// "DOM `<style>` vs `extra_stylesheets`" 節参照)。ただし
/// `extra_stylesheets` 側が `!important` を持つ場合はこの勝敗も反転する:
/// [`Origin::User`] の important rank (6) は hint の (常に normal で push
/// される、[`super::collect::cascade_rank`] doc 参照) rank (2) より高いため、`!important`
/// 付きの `extra_stylesheets` 宣言は hint に specificity を問わず勝つ。この
/// normal-tier の振る舞いの umbrella 越し end-to-end check は
/// `crates/raikiri/tests/build_cascaded.rs`'s
/// `img_width_presentational_hint_beats_extra_stylesheets_user_origin_via_umbrella`
/// 参照。
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
/// — fetched as raw spec HTML directly rather than through a summarizing
/// fetch, the same precaution [`super::selector_match::matches_empty`]'s doc explains: this
/// section sits deep inside one very long single-page spec, where
/// summarized fetches have been observed to truncate before reaching the
/// relevant section).
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
