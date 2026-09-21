use std::collections::HashSet;

use selectors::attr::{CaseSensitivity, ParsedCaseSensitivity};
use selectors::parser::{
    Combinator, NthOfSelectorData, NthSelectorData, RelativeSelector, Selector, SelectorIter,
    SelectorList,
};

use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};
use crate::{PseudoElem, RaikiriSelectorImpl};

use super::collect::{Specificity, specificity_of};
use super::lang::lang_pseudo_matches;
use super::resolve_directionality;

/// 1 compound selector 分 — `iter` が次の combinator に達する (または
/// selector 全体の終端に達する) まで — を `elem` 単体に対して判定する。
///
/// [`match_complex_selector_list`] が右端 compound を `elem` 自身に対して
/// 判定する最初の 1 手と、combinator 越しの祖先/兄弟候補判定
/// ([`match_combinator_chain`] / [`match_from_element`] — descendant/child
/// 越しの祖先判定と、NextSibling/LaterSibling 越しの兄弟判定は同じ関数に
/// 統合されている) の両方がこの関数を共有する — 元々の
/// `match_simple_selectors` 本体をそのまま抽出しただけで、per-component の
/// 判定ロジック自体に変更は無い。
///
/// `iter: &mut SelectorIter` を `for component in iter` で回すと、
/// `selectors` crate 自身の contract (`Selector::iter` の doc, verbatim:
/// "Returns an iterator over this selector in matching order
/// (right-to-left). When a combinator is reached, the iterator will return
/// None, and next_sequence() may be called to continue to the next
/// sequence.") により、combinator に達した時点で自動的にループが終わる —
/// `Component::Combinator` 自体がこの for ループの中に component として
/// 出てくることは無い (`SelectorIter::next()` が combinator を internal
/// state に退避して `None` を返す)。呼び出し側は本関数が `false` を返した
/// 場合と「compound は全部一致したが、まだ combinator が続く」場合を
/// 区別する必要があり、後者は呼び出し側が `iter.next_sequence()` で判定する
/// (本関数の戻り値だけでは分からない — 「compound 内で不一致は無かった」を
/// `true` で表すのみ)。
///
/// Returns: この 1 compound 内の全 component が match すれば true。
/// - `Component::LocalName(name)` — `elem.tag_name()` と eq_ignore_ascii_case で判定
/// - `Component::ExplicitUniversalType` — 常に match
/// - `Component::ID` — `elem.id()` と一致比較 (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#id-selectors>、verbatim: "When
///   matching against a document which is in quirks mode, IDs must be
///   matched ASCII case-insensitively; ID selectors are otherwise
///   case-sensitive")。`quirks_mode` 引数が
///   [`StyleQuirksMode::Quirks`] のときのみ `eq_ignore_ascii_case`、それ以外
///   ([`StyleQuirksMode::NoQuirks`] / [`StyleQuirksMode::LimitedQuirks`]) は
///   厳密一致 ("limited-quirks" は DOM Standard上
///   "quirks mode" と別 dfn、fold の対象外)
/// - `Component::Class` — `elem.has_class()` / `elem.has_class_ascii_case_insensitive()`
///   (CSS Selectors L4 <https://www.w3.org/TR/selectors-4/#class-html>、
///   verbatim: "When matching against a document which is in quirks mode,
///   class names must be matched ASCII case-insensitively; class selectors
///   are otherwise case-sensitive")。ID と同じ `quirks_mode` 分岐。
///   both variants share the same HTML-spec ASCII
///   whitespace tokenisation — [`StyleElement::has_class`] の doc 参照
/// - `Component::AttributeInNoNamespaceExists` / `Component::AttributeInNoNamespace`
///   — `elem.attr()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#attribute-selectors>)。存在チェック
///   形態 (`[foo]`) の lookup key は element の namespace に応じて
///   `local_name` / `local_name_lower` を選ぶ (詳細は該当 match arm の
///   コメント)。値付き形態の case-sensitivity 解決は
///   [`resolve_case_sensitivity`] 参照
/// - `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
///   — [`super::lang::language_range_matches`] /
///   [`resolve_directionality`] 経由、`dom` + `ancestors` (自身の祖先 chain)
///   を使って ancestor-inherited な effective language / directionality を
///   解決する。`PseudoClass::Hover` / `PseudoClass::Active` はこの arm 内で
///   引き続き `false` (dynamic pseudo-class の対応は本実装の scope 外のまま)。
/// - `Component::Root` (`:root`, CSS Selectors L4
///   §13.1 <https://www.w3.org/TR/selectors-4/#the-root-pseudo>) — matches
///   iff `ancestors.is_empty()`. Both call sites
///   ([`match_complex_selector_list`] for the rightmost compound,
///   [`match_from_element`] for compounds reached by crossing a combinator)
///   pass `ancestors` root-first/immediate-parent-last — [`super::collect::collect_cascaded`]'s
///   doc establishes that only `StyleNodeKind::Element` nodes are ever
///   pushed onto `ancestor_path`, so an empty `ancestors` slice means "no
///   element ancestor", i.e. this element is the root element of the
///   document tree — exactly the spec's "root of the document" (TR
///   anchor's own prose repeatedly truncated on WebFetch before reaching
///   normative text, same failure mode as [`match_combinator_chain`]'s
///   "Spec provenance note"; what loaded is the summary-table one-liner,
///   "an E element, root of the document").
/// - `Component::Empty` (`:empty`, CSS Selectors
///   L4 §13.2 <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — see
///   [`matches_empty`] doc for the verbatim spec text and its L4-vs-L3
///   whitespace-handling correction history.
/// - `Component::Is` / `Component::Where` (`:is()` / `:where()`) — recursively
///   match their selector lists against the same element. Invalid branches
///   produced by forgiving parsing fail closed in the ordinary matcher, while
///   the rule-tree gate keeps the valid branches available.
/// - `Component::Has` (`:has()`) — evaluate each relative selector by binding
///   its `RelativeSelectorAnchor` to the subject and searching the appropriate
///   descendant or sibling region. Nested `:has()` is rejected by the parser
///   and remains a fail-closed matcher case.
/// - `Component::Nth(data)` (`:first-child`/`:last-child`/`:only-child`/
///   `:nth-child()`/`:nth-last-child()` and their `-of-type` counterparts,
///   CSS Selectors L4 §13.3/§13.4) — see
///   [`matches_nth`] doc for the sibling-position algorithm and its spec
///   citation. Reuses `ancestors.last().copied().unwrap_or_else(||
///   dom.root_id())` for its sibling-list parent — the exact same
///   root-fallback idiom [`match_combinator_chain`]'s `NextSibling`/
///   `LaterSibling` arms already established for
///   an unrelated reason (sibling lookup key, not a compound-match
///   target); both fall back for the same underlying reason ("the root
///   element's parent-in-tree is the Document node, not an `Element`, but
///   `StyleDom::child_ids` still works against it").
/// - `Component::NthOf(data)` (`:nth-child(An+B of S)` /
///   `:nth-last-child(An+B of S)`, CSS Selectors L4 §13.3.1/§13.3.2) — see
///   [`matches_nth_of`] for the filtered sibling-position algorithm. Each
///   direct element child is first matched against the stored selector-list
///   `S` using the same complex-selector matcher as a stylesheet selector;
///   only matching children contribute to the 1-based position.
///
/// `Component::RelativeSelectorAnchor` is only true when the enclosing
/// `:has()` matcher supplies the corresponding subject id; an anchor cannot
/// match in an ordinary stylesheet selector. 他の component (namespace 付き属性 selector = 常に `Component::AttributeOther`、
/// または非小文字 local name **かつ値付き**の属性 selector = 同じく
/// `Component::AttributeOther` — 非小文字でも値なしの存在チェック形態は
/// namespace 無指定なら `AttributeInNoNamespaceExists` のまま、詳細は
/// `ruletree.rs` `is_supported_selector_list` のコメント) は
/// `is_supported_selector_list` が rule tree 構築時点で drop 済のはずだが、
/// safety net として引き続き match fail する。
pub(crate) fn compound_matches<D: StyleDom, E: StyleElement>(
    dom: &D,
    iter: &mut SelectorIter<'_, RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
) -> bool {
    use selectors::parser::Component;

    for component in iter {
        let component_matches = match component {
            Component::LocalName(local) => {
                // local.name は Atom (raikiri-style::Atom)、tag_name 文字列と比較
                elem.tag_name().eq_ignore_ascii_case(local.name.0.as_str())
            }
            Component::ExplicitUniversalType
            | Component::ExplicitAnyNamespace
            | Component::ExplicitNoNamespace
            | Component::DefaultNamespace(_) => {
                // 常に match / namespace は現時点では常に true 扱い
                true
            }
            // CSS Selectors L4 id-selectors / class-html (verbatim
            // quoted on the function doc
            // above): ASCII-case-fold only under full quirks mode.
            // `LimitedQuirks` is a *separate* DOM Standard dfn from
            // "quirks mode" (confirmed via direct fetch of
            // <https://dom.spec.whatwg.org/#concept-document-quirks>) and
            // does not get the fold, matching `NoQuirks`.
            Component::ID(id) => match quirks_mode {
                StyleQuirksMode::Quirks => elem
                    .id()
                    .is_some_and(|elem_id| elem_id.eq_ignore_ascii_case(id.0.as_str())),
                StyleQuirksMode::NoQuirks | StyleQuirksMode::LimitedQuirks => {
                    elem.id() == Some(id.0.as_str())
                }
            },
            Component::Class(class) => match quirks_mode {
                StyleQuirksMode::Quirks => elem.has_class_ascii_case_insensitive(class.0.as_str()),
                StyleQuirksMode::NoQuirks | StyleQuirksMode::LimitedQuirks => {
                    elem.has_class(class.0.as_str())
                }
            },
            Component::AttributeInNoNamespaceExists {
                local_name,
                local_name_lower,
            } => {
                // Which key to look up under `elem.attr()` depends on
                // the *element's* namespace, unlike the with-value arm
                // below (whose lowercase guarantee comes from the
                // selector's own parse, not the element). HTML LS's
                // "attributes on HTML elements in HTML documents are
                // lowercased" scoping
                // (https://html.spec.whatwg.org/multipage/semantics-other.html#case-sensitivity-of-selectors)
                // only covers HTML-namespace elements — html5ever's
                // tokenizer/tree-builder already lower-cases attribute
                // names for those (confirmed via
                // `crates/raikiri-html/src/sink.rs`'s `wire_side_tables`,
                // which stores `a.name.local` verbatim with no extra
                // lowercasing pass of its own), so `local_name_lower` is
                // the correct — and actually-stored — key there. Foreign
                // (SVG/MathML) elements are outside that HTML LS scope:
                // html5ever's "adjust foreign attributes" step can
                // restore specific attributes to their original mixed
                // case (e.g. `viewBox`), and `wire_side_tables` stores
                // whatever case html5ever produced, unmodified — so
                // `local_name` (the selector's own, unmodified case) is
                // the correct key for those.
                let key = if elem.namespace_uri().is_none() {
                    local_name_lower
                } else {
                    local_name
                };
                elem.attr(key.0.as_str()).is_some()
            }
            Component::AttributeInNoNamespace {
                local_name,
                operator,
                value,
                case_sensitivity,
            } => match elem.attr(local_name.0.as_str()) {
                // `local_name` を直に使ってよい理由 (この with-value arm
                // 限定 — 上の `AttributeInNoNamespaceExists` arm とは
                // 対比的に element の namespace を問わない): `selectors`
                // crate の parser (`AttributeInNoNamespace` を作る分岐) は
                // *selector 自身の* local name が既に ASCII-lowercase な
                // 場合にのみこの variant を選ぶ — 非小文字は
                // `Component::AttributeOther` に回る
                // (`is_supported_selector_list` が drop する)。この
                // lowercase 保証は selector の parse 時点で決まり、
                // どの element (HTML/foreign 問わず) に対して matching
                // するかに依存しないため、上の Exists arm と違って
                // namespace 分岐は不要。
                Some(attr_value) => {
                    let case = resolve_case_sensitivity(*case_sensitivity, elem);
                    operator.eval_str(attr_value, value.0.as_str(), case)
                }
                None => false,
            },
            Component::NonTSPseudoClass(pseudo) => match pseudo {
                crate::PseudoClass::Lang(ranges) => {
                    lang_pseudo_matches(ranges, dom, elem, ancestors)
                }
                crate::PseudoClass::Dir(dir) => {
                    resolve_directionality(dom, elem, elem_id, ancestors) == *dir
                }
                // `:hover` / `:active` — dynamic pseudo-class は本実装の
                // scope 外のまま。`is_supported_selector_list` が rule tree 構築時点で
                // drop する契約 (`ruletree::tests::pseudo_class_selector_still_dropped`
                // で pin) だが、`match_complex_selector_list_rejects_unsupported_component_via_safety_net`
                // がこの関数を直接呼んで safety net を確認する — 同じ姿勢を
                // 維持。
                crate::PseudoClass::Hover | crate::PseudoClass::Active => false,
            },
            Component::Root => ancestors.is_empty(),
            Component::Empty => matches_empty(dom, elem_id),
            Component::Negation(selectors) => !selector_slice_matches_with_anchor(
                selectors.slice(),
                dom,
                elem,
                elem_id,
                ancestors,
                quirks_mode,
                relative_anchor,
            ),
            Component::Is(selectors) | Component::Where(selectors) => {
                selector_slice_matches_with_anchor(
                    selectors.slice(),
                    dom,
                    elem,
                    elem_id,
                    ancestors,
                    quirks_mode,
                    relative_anchor,
                )
            }
            Component::Has(_) if relative_anchor.is_some() => false,
            Component::Has(relative_selectors) => has_relative_selector_matches(
                dom,
                relative_selectors,
                elem_id,
                ancestors,
                quirks_mode,
            ),
            Component::Nth(data) => {
                // Root element (`ancestors.is_empty()`) still has a sibling
                // list — the empty set of *element* siblings under
                // `dom.root_id()` — per CSS Selectors L3 §6.6
                // structural-pseudos preamble's sibling-counting framing
                // (`matches_nth` doc, verbatim); it is not itself excluded
                // just because it has no *element* parent, unlike
                // `Component::Root` above. `dom.root_id()` is the Document
                // node `collect_cascaded` never pushes onto `ancestor_path`
                // (see that function's doc), and its `child_ids` already
                // includes the root element — the natural "sibling-list
                // container" for a root element that in the DOM tree has no
                // element parent at all. Same root-fallback idiom
                // `match_combinator_chain`'s `NextSibling`/`LaterSibling`
                // arms already use.
                let sibling_parent = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
                matches_nth(
                    dom,
                    sibling_parent,
                    elem_id,
                    elem.tag_name(),
                    data,
                    ancestors,
                    quirks_mode,
                )
            }
            Component::NthOf(data) => {
                let sibling_parent = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
                matches_nth_of(
                    dom,
                    sibling_parent,
                    elem_id,
                    elem.tag_name(),
                    data,
                    ancestors,
                    quirks_mode,
                )
            }
            Component::RelativeSelectorAnchor => relative_anchor == Some(elem_id),
            _ => {
                // 他 component (AttributeOther) は ruletree build 段で
                // drop 済のはずだが safety net で match fail
                false
            }
        };
        if !component_matches {
            return false;
        }
    }
    true
}

/// CSS Text Module Level 4 "document white space character" (see
/// [`matches_empty`]'s doc for the full verbatim citation and provenance,
/// including the deliberate exclusion of form feed U+000C). For
/// HTML-parsed content this set is `{space, tab, line feed}`; carriage
/// return (U+000D) is also included — CSS Text 3 §4
/// <https://www.w3.org/TR/css-text-3/#white-space-processing> states it is
/// "treated identically to spaces (U+0020) in all respects", even though CR
/// is not itself a segment break (a segment break is line feed (U+000A) for
/// HTML-parsed content, per the same section).
///
/// CR-inclusion is not dead weight against a non-HTML-normalized
/// [`StyleDom`]: an HTML parser folds a *literal* CR/CRLF byte in the source
/// to LF during input-stream preprocessing, but a numeric character
/// reference such as `&#x0D;` is decoded to U+000D during tokenization,
/// *after* that normalization step, so a real DOM text node produced by a
/// conformant parser can and does contain a literal U+000D. §4's own
/// closing sentence on this point: "the character is preserved — and the
/// above rule observable — when encoded using an escape sequence
/// (`&#x0d;`)."
fn is_document_white_space(c: char) -> bool {
    matches!(c, '\u{0020}' | '\u{0009}' | '\u{000A}' | '\u{000D}')
}

/// `:empty` (CSS Selectors L4 §13.2
/// <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — whether
/// `elem_id` has no children that count toward emptiness.
///
/// # Spec provenance and correction
///
/// L4's own TR anchor repeatedly truncated on WebFetch before reaching
/// normative prose — same failure mode [`match_combinator_chain`]'s "Spec
/// provenance note" documents for combinators. The first pass of this
/// function fell back to Selectors **Level 3** prose ("only... content
/// nodes... whose data has a non-zero length must be considered as
/// affecting emptiness") without realizing L4 had *deliberately changed*
/// this from L3, not merely restated it. Corrected after a direct raw
/// fetch of `raw.githubusercontent.com/w3c/csswg-drafts/main/selectors-4/
/// Overview.bs` (bypassing WebFetch's truncation entirely — `curl` the
/// bikeshed source and grep it directly), `#the-empty-pseudo` section,
/// verbatim: "The :empty pseudo-class represents an element that has no
/// children except, optionally, [=document white space characters=]. ...
/// only element nodes and content nodes (such as \[\[DOM\]\] text nodes, and
/// entity references) whose data has a non-zero length must be considered
/// as affecting emptiness; comments, processing instructions, and other
/// nodes must not affect whether an element is considered empty or not."
/// — followed by an explicit changelog note: "In Level 2 and Level 3 of
/// Selectors, :empty did not match elements that contained only white
/// space. This was changed so that... elements which authors perceive of
/// as empty can be selected by this selector, as they expect." The
/// section's own worked examples list `<p> </p>` (whitespace-only) among
/// what `p:empty` matches, and `<div>&nbsp;</div>` among what it does
/// *not* match — pinning both directions.
///
/// "Document white space characters" is itself a CSS Text Module Level 4
/// term (`#the-empty-pseudo`'s own autolink target), verbatim (direct raw
/// fetch of `.../css-text-4/Overview.bs`, `#white-space-rules`): "the
/// [document white space characters]: spaces (U+0020), tabs (U+0009), and
/// segment breaks" — stated a second time nearby, identically: "both
/// include spaces (U+0020), tabs (U+0009), and line feeds (U+000A)". For
/// HTML specifically (same source, `#white-space-rules` preamble), a
/// segment break is exactly line feed (U+000A): "In the case of HTML,
/// newlines are normalized to line feed characters (U+000A)... so... each
/// line feed (U+000A) is treated as a segment break" — and carriage
/// return (U+000D) is separately stated to be "treated identically to
/// spaces (U+0020) in all respects" (same source). CR-inclusion is not
/// dead weight against a non-HTML-normalized [`StyleDom`]: an HTML parser
/// folds a *literal* CR/CRLF byte in the source to LF during input-stream
/// preprocessing, but a numeric character reference such as `&#x0D;` is
/// decoded to U+000D during tokenization, *after* that normalization step,
/// so a real DOM text node produced by a conformant parser can and does
/// contain a literal U+000D. §4's own closing sentence on this point:
/// "the character is preserved — and the above rule observable — when
/// encoded using an escape sequence (`&#x0d;`)." [`is_document_white_space`]
/// is generic over any [`StyleDom`] impl, not just `raikiri-html`'s, but
/// the CR handling is load-bearing for ordinary conformant HTML content
/// (e.g. `<h1>A&#x0d;&#x0d;B</h1>`), not defensive dead code.
///
/// **Deliberately excludes form feed (U+000C)** — unlike Rust's
/// `char::is_ascii_whitespace()` / this crate's own HTML "ASCII
/// whitespace" 5-character set used elsewhere ([`crate::style_dom`]'s
/// `class_token_matches`). Direct search of the css-text-4 raw source
/// (not a WebFetch summary) for "U+000C"/"form feed" returns zero hits
/// anywhere near the "document white space characters" dfn, which is
/// stated explicitly — twice — as exactly {space, tab, segment break/line
/// feed}, no fourth category. This is narrower than an earlier relayed
/// characterization of the set as "U+000A/U+000D/U+000C family" — flagged
/// here as a discrepancy to confirm or correct with a citation, since this
/// function currently follows the directly-verified primary source over
/// the relayed one where they disagree.
///
/// # Node-kind coverage
///
/// [`StyleNodeKind`] has no CDATA/entity-reference variant — HTML parsing
/// produces neither (html5ever folds CDATA-section syntax outside foreign
/// content into a bogus comment per HTML LS tokenization, and HTML has no
/// entity-reference *nodes* the way XML does, only inline character
/// reference expansion during tokenization) — so only `Element` and `Text`
/// need an explicit arm below; `Comment` / `ProcessingInstruction` /
/// `DocumentFragment` (`<template>` contents live in a separate detached
/// tree per that variant's own doc, so they never appear in `child_ids`
/// here regardless) fall through to "does not affect emptiness", matching
/// the spec text's "comments, processing instructions, and other nodes
/// must not affect" clause.
pub(crate) fn matches_empty<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> bool {
    dom.child_ids(elem_id)
        .all(|child_id| match dom.node(child_id) {
            Some(node) => match node.kind() {
                StyleNodeKind::Element => false,
                StyleNodeKind::Text => node
                    .text_content()
                    .unwrap_or("")
                    .chars()
                    .all(is_document_white_space),
                StyleNodeKind::Comment
                | StyleNodeKind::ProcessingInstruction
                | StyleNodeKind::DocumentFragment
                | StyleNodeKind::Document => true,
            },
            // cov:ignore: `child_ids` only ever yields ids that `dom.node`
            // resolves (`StyleDom` trait doc: "child_ids(id) returns an empty
            // iterator for invalid id", implying ids it does yield are valid) —
            // defensive fallback in the same posture as `match_from_element`'s
            // own `dom.node(elem_id)` guard.
            None => true,
        })
}

#[derive(Clone, Copy)]
pub(crate) struct SiblingMatchContext<'a> {
    selector_filter: Option<&'a [Selector<RaikiriSelectorImpl>]>,
    ancestors: &'a [StyleNodeId],
    quirks_mode: StyleQuirksMode,
}

/// 1-based sibling position of `elem_id` among `parent_id`'s **in-document
/// element** children, both from the start and from the end, plus the total
/// count of such siblings — shared arithmetic behind `Component::Nth` and
/// `Component::NthOf` matching ([`matches_nth`] / [`matches_nth_of`]).
///
/// `of_type == false` (`:nth-child`/`:first-child`/`:last-child`/
/// `:only-child`) counts **all** element siblings regardless of tag; CSS
/// Selectors L3 §6.6 structural-pseudos preamble (verbatim, <https://www.w3.org/TR/selectors-3/#structural-pseudos> —
/// same feature, L4 does not change this): "Standalone text and other
/// non-element nodes are not counted when calculating the position of an
/// element in its list of siblings; index numbering starts at 1."
///
/// `of_type == true` (`:nth-of-type`/`:first-of-type`/`:last-of-type`/
/// `:only-of-type`) additionally restricts to siblings sharing `elem_tag`
/// — same source, verbatim: "an+b−1 siblings with the same expanded
/// element name". "Expanded element name" is tag name **and** namespace;
/// this crate's `compound_matches` already treats namespace matching as
/// always-true (`Component::DefaultNamespace(_) => true`, "常に
/// match / namespace は現時点では常に true 扱い") for the equivalent
/// selector-vs-element case, so restricting this sibling-vs-sibling
/// comparison to `tag_name` equality inherits that existing scope
/// simplification rather than introducing a new one. Plain `==` (not
/// `eq_ignore_ascii_case`, unlike the selector-vs-element `LocalName` arm)
/// — html5ever already normalises HTML tag names to lowercase before they
/// ever reach `StyleElement::tag_name`, and "expanded name" comparison for
/// non-HTML (SVG/MathML) content is case-sensitive per XML tag-name rules,
/// so exact comparison is correct for both.
///
/// When `selector_filter` is present, a child contributes only if it matches
/// at least one selector in that list. The candidate is evaluated with the
/// same `ancestors` and `quirks_mode` as the element being matched, because
/// all direct siblings share that parent context.
pub(crate) fn sibling_position<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    of_type: bool,
    context: SiblingMatchContext<'_>,
) -> (i32, i32, i32) {
    let mut total = 0i32;
    let mut index_from_start = 0i32;
    for child_id in dom.child_ids(parent_id) {
        let Some(child_node) = dom.node(child_id) else {
            continue; // cov:ignore: defensive against a detached/inert child_id that dom.node() can't resolve; no StyleDom impl in this crate's test corpus produces one (verified against a main-branch baseline, see raikiri-spike-4nhl.9).
        };
        // `child_ids` is the raw arena view for the style DOM, so detached /
        // inert nodes can still occur in this iterator. Structural
        // pseudo-classes operate on the flat-tree sibling list, matching the
        // gate used by `collect_cascaded` and sibling combinators.
        if !child_node.is_in_document() {
            continue;
        }
        let Some(sibling) = child_node.as_element() else {
            continue;
        };
        if of_type && sibling.tag_name() != elem_tag {
            continue;
        }
        if let Some(selectors) = context.selector_filter
            && !selector_slice_matches(
                selectors,
                dom,
                &sibling,
                child_id,
                context.ancestors,
                context.quirks_mode,
            )
        {
            continue;
        }
        total += 1;
        if child_id == elem_id {
            index_from_start = total;
        }
    }
    let index_from_end = total - index_from_start + 1;
    (index_from_start, index_from_end, total)
}

/// `Component::Nth` — covers `:first-child`/`:last-child`/`:only-child`/
/// `:nth-child()`/`:nth-last-child()` (CSS Selectors L4 §13.3
/// <https://www.w3.org/TR/selectors-4/#the-first-child-pseudo> area) and
/// their `-of-type` counterparts (§13.4
/// <https://www.w3.org/TR/selectors-4/#the-nth-of-type-pseudo> area) — the
/// `selectors` crate itself parses all ten syntaxes into this one
/// `Component` variant, distinguished only by `NthSelectorData::ty`
/// (`selectors` 0.39.0 `parser.rs`'s `NthSelectorData::only`/`first`/`last`
/// constructors and `parse_nth_pseudo_class`, the dependency's own public parse dispatch — not a Stylo reference).
///
/// L4's own per-selector TR anchors truncated the same way documented on
/// [`matches_empty`]; fell back to Selectors **Level 3** §6.6
/// <https://www.w3.org/TR/selectors-3/#structural-pseudos> (verbatim,
/// again the same feature, unchanged by L4
/// except for the `An+B of S` extension handled by [`matches_nth_of`]):
/// "The :nth-child(an+b) pseudo-class notation represents an element that
/// has an+b-1 siblings before it in the document tree... The
/// :nth-last-child(an+b) pseudo-class notation represents an element that
/// has an+b-1 siblings after it... :first-child — Same as :nth-child(1)...
/// :last-child — Same as :nth-last-child(1)... :only-child — represents an
/// element that has no siblings... :nth-of-type(an+b) — an element that
/// has an+b-1 siblings with the same expanded element name before it...
/// :only-of-type — an element that has no siblings with the same expanded
/// element name."
///
/// Implementation: convert "an+b−1 siblings before/after" into a 1-based
/// index ([`sibling_position`]) and let `AnPlusB::matches_index` (the
/// `selectors` crate's own An+B arithmetic, already used as-is — no
/// hand-rolled micro-syntax math here) decide. `:only-*` is `total == 1`
/// directly (an element with exactly one matching sibling — itself — has
/// "no siblings" in the relevant filtered sense) rather than routing
/// through `an_plus_b`, matching how `NthSelectorData::only()` fixes
/// `an_plus_b` at a placeholder `AnPlusB(0, 1)` that was never meant to be
/// evaluated for this `ty`.
///
/// The root element (`parent_id` resolved by the caller to `ancestors.last()
/// .copied().unwrap_or_else(|| dom.root_id())` when there is no element
/// ancestor, see the `Component::Nth` arm's own comment in
/// [`compound_matches`]) trivially satisfies `:first-child`/
/// `:last-child`/`:only-child`/`:nth-child(1)` — it has zero element
/// siblings before or after it, which the "an+b-1 siblings before/after
/// it" framing above does not require a *parent element* to state, only a
/// sibling list (possibly of size 1, itself alone).
fn matches_nth<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    data: &NthSelectorData,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    let (from_start, from_end, total) = sibling_position(
        dom,
        parent_id,
        elem_id,
        elem_tag,
        data.ty.is_of_type(),
        SiblingMatchContext {
            selector_filter: None,
            ancestors,
            quirks_mode,
        },
    );
    matches_nth_position(from_start, from_end, total, data)
}

/// `Component::NthOf` matching for CSS Selectors L4's `:nth-child(An+B of S)`
/// and `:nth-last-child(An+B of S)` forms. The specification defines the
/// position among the inclusive siblings that match `S`; the selector-list
/// matcher therefore runs for every direct element child before the normal
/// `An+B` arithmetic is applied.
fn matches_nth_of<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    data: &NthOfSelectorData<RaikiriSelectorImpl>,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    let nth_data = data.nth_data();
    let (from_start, from_end, total) = sibling_position(
        dom,
        parent_id,
        elem_id,
        elem_tag,
        nth_data.ty.is_of_type(),
        SiblingMatchContext {
            selector_filter: Some(data.selectors()),
            ancestors,
            quirks_mode,
        },
    );
    matches_nth_position(from_start, from_end, total, nth_data)
}

fn matches_nth_position(
    from_start: i32,
    from_end: i32,
    total: i32,
    data: &NthSelectorData,
) -> bool {
    if from_start == 0 {
        // Defensive: `elem_id` was not found among `parent_id`'s (filtered)
        // element children at all — unreachable given `collect_cascaded`'s
        // ancestor-path invariant (every `elem_id`/`parent_id` pair this is
        // ever called with really is a child/parent pair in the walked
        // tree), same posture as `match_from_element`'s own defensive
        // guards. Guards specifically against `AnPlusB(0, 0)`
        // (`:nth-child(0)`, a degenerate but syntactically valid selector)
        // spuriously matching via `matches_index(0) == true` if that
        // invariant were ever violated.
        // cov:ignore: unreachable given the invariant above; would need a
        // `StyleDom` impl that lies about an element's own parent to
        // exercise.
        return false;
    }
    if data.ty.is_only() {
        total == 1
    } else if data.ty.is_from_end() {
        data.an_plus_b.matches_index(from_end)
    } else {
        data.an_plus_b.matches_index(from_start)
    }
}

/// `elem` (と、combinator を跨ぐ場合は `ancestors` で表される祖先 element 列
/// / `elem_id` から辿る兄弟 element 列) と selector list を突き合わせる
/// トップレベル matcher。
///
/// # Combinator 対応
///
/// 当初は single-element (compound-only) matching のみで、combinator を
/// 含む selector は `ruletree.rs` `is_supported_selector_list` の gate で
/// rule tree に乗る前に drop されていた。その後 descendant (space, CSS
/// Selectors L4 <https://www.w3.org/TR/selectors-4/#descendant-combinators>)
/// と child (`>`, <https://www.w3.org/TR/selectors-4/#child-combinators>)
/// の 2 combinator を追加し、続けて adjacent sibling (`+`,
/// <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>) と
/// general sibling (`~`,
/// <https://www.w3.org/TR/selectors-4/#general-sibling-combinators>) を追加
/// した (4 combinator 全対応、詳細は [`match_combinator_chain`] doc)。complex
/// selector の一般的な match 条件は CSSWG Editor's Draft
/// <https://drafts.csswg.org/selectors-4/#complex> (verbatim — provenance の詳細は [`match_combinator_chain`] doc の
/// note 参照) の記述: "A given element ... is said to match a complex
/// selector when it matches the final compound selector ... in the
/// sequence, and every preceding unit of the sequence also matches an
/// element ..., with the correct relationship between consecutive units as
/// expressed by the combinators separating them" — 本関数はこれを右 (elem
/// 自身) から左 (祖先/兄弟) への
/// `Selector::iter`/`SelectorIter::next_sequence` の反復として実装する:
///
/// 1. 一番右の compound を `elem` 自身に対して [`compound_matches`] で判定。
/// 2. 不一致ならこの selector は不一致、次の selector へ。
/// 3. 一致すれば `iter.next_sequence()` で次の combinator を見る:
///    - `None` (もう combinator が無い) → selector 全体が一致。
///    - `Some(combinator)` → [`match_combinator_chain`] に委譲、combinator
///      の意味 (child = 直近の親のみ、descendant = いずれかの祖先、
///      next-sibling = 直前の兄弟のみ、later-sibling = それ以前のいずれかの
///      兄弟) に沿って次の compound を判定する。
///
/// `ancestors` は root 側が先頭、直近の親が末尾の順 (`ancestors.last()` ==
/// `elem` の親) — [`super::collect::collect_cascaded`] の DFS 訪問順から構築される
/// (同関数の doc 参照)。`elem_id` は `elem` 自身の id — sibling combinator
/// が「`elem` の親の子リストの中で `elem` より前にいる
/// のは誰か」を [`StyleDom::child_ids`] から直接求める際の探索終端として
/// 導入され ([`match_combinator_chain`] の `NextSibling`/`LaterSibling` arm
/// 参照)、その後 [`compound_matches`] 自身にも渡すよう
/// 拡張された — `:root`/`:empty`/`:nth-child()` 等の構造的 pseudo-class が
/// `dom`/`elem_id`/`ancestors.last()` (= `elem` の親) を必要とするため、
/// `elem` の借用値だけでは表現できない情報として渡す。
///
/// Returns: matching した selector の最大 specificity。1 つも match しなければ None。
/// `specificity_of` は selector 全体 (combinator を跨いだ複合 selector) に
/// 対する値 — combinator 追加後もこの呼び出しに変更は無い (`selectors`
/// crate 自身が selector 全体から算出する)。
pub(crate) fn match_complex_selector_list<D: StyleDom, E: StyleElement>(
    list: &SelectorList<RaikiriSelectorImpl>,
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> Option<Specificity> {
    let mut best: Option<Specificity> = None;
    for selector in list.slice() {
        if selector_matches(dom, selector, elem, elem_id, ancestors, quirks_mode) {
            let spec = specificity_of(selector);
            best = Some(match best {
                Some(prev) => prev.max(spec),
                None => spec,
            });
        }
    }
    best
}

fn selector_slice_matches<D: StyleDom, E: StyleElement>(
    selectors: &[Selector<RaikiriSelectorImpl>],
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    selector_slice_matches_with_anchor(selectors, dom, elem, elem_id, ancestors, quirks_mode, None)
}

fn selector_slice_matches_with_anchor<D: StyleDom, E: StyleElement>(
    selectors: &[Selector<RaikiriSelectorImpl>],
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
) -> bool {
    selectors.iter().any(|selector| {
        selector_matches_with_anchor(
            dom,
            selector,
            elem,
            elem_id,
            ancestors,
            quirks_mode,
            relative_anchor,
        )
    })
}

fn selector_matches<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    selector_matches_with_anchor(dom, selector, elem, elem_id, ancestors, quirks_mode, None)
}

/// Matches a selector while optionally binding the internal
/// `RelativeSelectorAnchor` component generated for a `:has()` argument to a
/// particular element. Ordinary stylesheet selectors use [`selector_matches`]
/// and therefore cannot match that internal component.
fn selector_matches_with_anchor<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
) -> bool {
    let mut iter = selector.iter();
    compound_matches(
        dom,
        &mut iter,
        elem,
        elem_id,
        ancestors,
        quirks_mode,
        relative_anchor,
    ) && match iter.next_sequence() {
        None => true,
        Some(combinator) => match_combinator_chain(
            dom,
            combinator,
            elem_id,
            ancestors,
            iter,
            quirks_mode,
            relative_anchor,
        ),
    }
}

/// Matches the relative selector list stored by `:has()` against an anchor
/// element. `selectors` stores each relative selector with an internal
/// `RelativeSelectorAnchor` at its left edge, so the normal right-to-left
/// matcher can be reused once that marker is bound to `anchor_id`.
fn has_relative_selector_matches<D: StyleDom>(
    dom: &D,
    relative_selectors: &[RelativeSelector<RaikiriSelectorImpl>],
    anchor_id: StyleNodeId,
    anchor_ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> bool {
    for relative_selector in relative_selectors {
        let leading_combinator = relative_selector.selector.combinator_at_parse_order(1);
        let include_descendants = relative_selector.match_hint.is_subtree();

        let (roots, candidate_ancestors) = match leading_combinator {
            Combinator::Child | Combinator::Descendant => {
                let roots = in_document_element_children(dom, anchor_id);
                let mut candidate_ancestors = anchor_ancestors.to_vec();
                candidate_ancestors.push(anchor_id);
                (roots, candidate_ancestors)
            }
            Combinator::NextSibling | Combinator::LaterSibling => {
                let parent_id = anchor_ancestors
                    .last()
                    .copied()
                    .unwrap_or_else(|| dom.root_id());
                let roots = following_sibling_elements(
                    dom,
                    parent_id,
                    anchor_id,
                    relative_selector.match_hint.is_next_sibling(),
                );
                (roots, anchor_ancestors.to_vec())
            }
            // The parser rejects pseudo-element/slot/part combinators inside
            // `:has()`. Keep the matcher fail-closed if a future parser path
            // constructs one anyway.
            // cov:ignore: `selectors` does not construct these combinators in a :has() relative selector
            Combinator::PseudoElement | Combinator::Part | Combinator::SlotAssignment => {
                continue;
            }
        };

        if relative_selector_matches_in_regions(
            dom,
            &relative_selector.selector,
            &roots,
            &candidate_ancestors,
            include_descendants,
            anchor_id,
            quirks_mode,
        ) {
            return true;
        }
    }
    false
}

/// Searches the candidate roots and, when the relative selector can reach
/// deeper nodes, their element subtrees. The explicit stack avoids consuming
/// the native call stack on deeply nested untrusted markup.
fn relative_selector_matches_in_regions<D: StyleDom>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    roots: &[StyleNodeId],
    base_ancestors: &[StyleNodeId],
    include_descendants: bool,
    anchor_id: StyleNodeId,
    quirks_mode: StyleQuirksMode,
) -> bool {
    // Keep one mutable ancestor path and record only its length in each stack
    // entry. Cloning the full path into every pending child makes a deep,
    // branched `:has()` miss retain O(depth²) ancestor ids at a choice point;
    // truncating on pop gives the same DFS paths with O(depth + pending nodes)
    // storage instead.
    let base_depth = base_ancestors.len();
    let mut ancestor_path = base_ancestors.to_vec();
    let mut stack: Vec<(StyleNodeId, usize)> = Vec::with_capacity(roots.len());
    for &root_id in roots.iter().rev() {
        stack.push((root_id, base_depth));
    }

    while let Some((candidate_id, depth)) = stack.pop() {
        ancestor_path.truncate(depth);
        if !is_in_document_element(dom, candidate_id) {
            continue; // cov:ignore: roots are pre-filtered; only a malformed custom DOM can reach this branch
        }
        let Some(node) = dom.node(candidate_id) else {
            continue; // cov:ignore: is_in_document_element already required a node for this id
        };
        let Some(elem) = node.as_element() else {
            continue; // cov:ignore: is_in_document_element already required an element node
        };
        if selector_matches_with_anchor(
            dom,
            selector,
            &elem,
            candidate_id,
            &ancestor_path,
            quirks_mode,
            Some(anchor_id),
        ) {
            return true;
        }
        if !include_descendants {
            continue;
        }

        ancestor_path.push(candidate_id);
        let child_depth = ancestor_path.len();
        let start = stack.len();
        stack.extend(
            dom.child_ids(candidate_id)
                .map(|child_id| (child_id, child_depth)),
        );
        // The stack is LIFO, but child_ids is in document order. Reverse only
        // the newly appended entries so traversal remains pre-order.
        stack[start..].reverse();
    }
    false
}

/// Returns direct in-document element children in document order.
fn in_document_element_children<D: StyleDom>(dom: &D, parent_id: StyleNodeId) -> Vec<StyleNodeId> {
    dom.child_ids(parent_id)
        .filter(|&id| is_in_document_element(dom, id))
        .collect()
}

/// Returns the following element siblings of `anchor_id`, optionally limited
/// to the first one for the adjacent-sibling relative combinator.
fn following_sibling_elements<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    anchor_id: StyleNodeId,
    only_first: bool,
) -> Vec<StyleNodeId> {
    let mut following = false;
    let mut result = Vec::new();
    for child_id in dom.child_ids(parent_id) {
        if child_id == anchor_id {
            following = true;
            continue;
        }
        if following && is_in_document_element(dom, child_id) {
            result.push(child_id);
            if only_first {
                break;
            }
        }
    }
    result
}

/// `::before`/`::after` counterpart of [`selector_matches`]: if `selector`
/// targets a `::before`/`::after` pseudo-element at all
/// ([`Selector::pseudo_element`], an `O(1)` bitflag-backed check — cheap
/// `None` for the overwhelmingly common non-pseudo selector) *and* the rest
/// of the selector (the originating element's own compound/combinator chain,
/// to the left of the `::before`/`::after`) matches `elem`, returns that
/// pseudo-element. Otherwise `None` — including when `elem` doesn't match
/// the originating-element part.
///
/// `elem` itself is never matched against `Component::PseudoElement`
/// directly — that component only ever reaches [`compound_matches`] through
/// this function's own one-compound skip below, never through
/// [`selector_matches`]'s ordinary rightmost-compound match (which
/// `compound_matches`'s `_ => false` safety net already rejects a bare
/// `Component::PseudoElement` on, so an ordinary selector ending in
/// `::before`/`::after` correctly never matches the real element as itself —
/// this is what keeps [`match_complex_selector_list`] from also matching a
/// `.foo::before` rule directly onto `.foo` the real element, with no
/// changes needed to that function or `compound_matches`).
///
/// # Why `Component::PseudoElement` can only be this function's own leading
/// compound
///
/// `RaikiriSelectorParser` overrides none of `PseudoElement`'s
/// `is_before_or_after` / `accepts_state_pseudo_classes` /
/// `parses_as_element_backed` defaults (all stay `false` — see
/// [`crate::PseudoElem`] doc), so the `selectors` crate's own parser state
/// machine rejects any pseudo-class or further pseudo-element after a
/// `::before`/`::after` (`selectors` v0.39.0 `parser.rs`'s
/// `AFTER_NON_ELEMENT_BACKED_PSEUDO`/`AFTER_BEFORE_OR_AFTER_PSEUDO` state
/// handling), and a compound/combinator *following* `::before`/`::after`
/// (e.g. `a::before b`) is rejected even earlier, as a bare syntax error —
/// empirically confirmed against this crate's actual `parse_selector_list`
/// (not read from `selectors`' source in isolation): `.foo::before` parses
/// to the raw match-order sequence `[PseudoElement(Before),
/// Combinator(PseudoElement), Class("foo")]`; `a::before b`,
/// `::before::after`, `::before:hover`, and `.foo::before.bar` are all
/// parse errors. A `Selector` that parsed successfully at all therefore has
/// **at most one** `Component::PseudoElement`, always as the sole member of
/// its own rightmost compound.
pub(crate) fn selector_matches_pseudo_element<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
) -> Option<PseudoElem> {
    let pseudo = *selector.pseudo_element()?;
    let mut iter = selector.iter();
    // Skip past the (sole) pseudo-element compound — same idiom the
    // `selectors` crate's own `Selector::parts()` uses to skip a leading
    // pseudo-element compound before inspecting the rest of the selector
    // (`selectors` v0.39.0 `parser.rs`).
    for _ in &mut iter {}
    let combinator = iter.next_sequence();
    // cov:ignore: panic-message literal only executed on assertion failure
    // — this crate's whole selector-parsing surface (see this function's
    // doc) guarantees `combinator == Some(Combinator::PseudoElement)`
    // whenever `selector.pseudo_element()` returned `Some` above, so this
    // never fails while any test in this crate runs.
    debug_assert_eq!(
        combinator,
        Some(Combinator::PseudoElement),
        "Selector::pseudo_element() returned Some, so `selectors` must have \
         bridged it with Combinator::PseudoElement — see this function's \
         doc for why that's the only shape a successfully-parsed selector \
         can take here"
    );
    let matches = compound_matches(dom, &mut iter, elem, elem_id, ancestors, quirks_mode, None)
        && match iter.next_sequence() {
            None => true,
            Some(next_combinator) => match_combinator_chain(
                dom,
                next_combinator,
                elem_id,
                ancestors,
                iter,
                quirks_mode,
                None,
            ),
        };
    matches.then_some(pseudo)
}

/// [`match_complex_selector_list`] が右端 compound を `elem` に対して
/// マッチさせたあと、残りの combinator + compound 列を `ancestors`
/// (祖先チェーン) / `current_id` から辿る兄弟列のどちらかを遡って判定する。
///
/// - [`Combinator::Child`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim: "A
///   child combinator describes a childhood relationship between two
///   elements") — 候補は `ancestors` の末尾 (直近の親) **1 つだけ**。それが
///   次の compound に一致し、かつ (さらに左に combinator が続くなら) その
///   親のそのまた祖先から続きが一致すれば全体一致。バックトラックは無い —
///   `>` は「直近の親」を一意に指すため。
/// - [`Combinator::Descendant`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#descendant-combinators>, verbatim:
///   "A selector of the form A B represents an element B that is an
///   arbitrary descendant of some ancestor element A") — `ancestors` を
///   直近の親から根に向かって 1 つずつ試し、次の compound が一致した候補を
///   見つけたら、その候補を起点にさらに左の残りを再帰的に判定する。1 候補で
///   残りの判定まで失敗した場合、次の (さらに外側の) 祖先で再試行する —
///   `iter.clone()` (`selectors::parser::SelectorIter` は `Clone`) で候補
///   ごとに独立した iterator コピーを使う。CSSWG Editor's Draft
///   <https://drafts.csswg.org/selectors-4/#complex> の complex selector
///   定義の「(match の条件は) 各 unit が対応する combinator の関係を
///   満たしながら何らかの element に一致すること」という再帰的な定義を
///   そのまま素直に実装したもの。
///
///   **この再試行は load-bearing — 省略すると壊れる** (訂正: 以前ここには
///   「祖先チェーンは分岐の無い単一の直線なので retry は
///   冗長」という誤った一般化があった。独立したレビューが複数、
///   同型の反例を構築して指摘 — 以下は
///   その反例)。誤りだった論法は「直近候補を選んだ場合の残り
///   `ancestors` は、より遠い候補を選んだ場合の残り `ancestors` を
///   必ず包含する superset になる」という主張だったが、これは**残りが
///   すべて [`Combinator::Descendant`] のとき**にしか成立しない —
///   その場合は次の判定が「残り `ancestors` の**どこかに** compound が
///   一致するか」という集合に対する自由な存在探索で、探索対象が広い
///   ほど (superset ほど) 弱くならないため。しかし残りに
///   [`Combinator::Child`] が 1 つでも混ざると、その段の判定は
///   `ancestors.split_last()` が指す**特定 1 要素**の compound 一致
///   可否であり、候補ごとに「集合の一部を切り詰めたもの」ではなく
///   「そもそも別の要素」を見ることになる — supersetによる包含関係が
///   意味を持たない。
///
///   反例 (`.x > .y .target`、`x`/`y`/`target` は class):
///   `G(.x) → F(.y) → M(no class) → C(.y) → elem(.target)` という祖先
///   チェーンで `elem` を判定する。`elem` の直近の `.y` 候補は `C`
///   だが、`C` の直近の親は `M` で `.x` を持たない — `Child` の判定対象
///   `M` に固定されるため、`C` 候補はここで確定的に失敗する。ここで
///   打ち切ると selector 全体が不一致になってしまうが、正しい答えは
///   一致: より遠い候補 `F` (`.y` を持つ) の直近の親は `G` で `.x` を
///   持つ。`F` を試すこの再試行が無ければ、この (spec 上正当な)
///   selector が静かに一致しなくなる — check 用の regression test
///   `tests::descendant_retry_past_a_failed_child_combinator_candidate_is_required`
///   (この module 内 `#[cfg(test)] mod tests`) がこの具体形をそのまま
///   実行する。
///
///   sibling combinator (`+`/`~`) のような非祖先チェーン型 combinator が
///   同じ complex selector 内に混在するとさらに事情が変わりうる — ただし
///   「事情が変わる」というのは
///   「不正確になる」ではなく「別の軸で load-bearing になる」だった:
///   sibling ジャンプは `ancestors` を不変のまま引き継ぐため、そこから
///   さらに左へ [`Combinator::Descendant`] が続く場合もこの retry は
///   同じ理由でそのまま load-bearing (下記 [`Combinator::NextSibling`] /
///   [`Combinator::LaterSibling`] の説明、および `ruletree.rs`
///   `is_supported_selector_list` doc の "4 combinator 間の混在" note
///   参照)。探索順序 (直近から遠方へ)
///   自体は正しさに影響しない — いずれの順で候補を試しても最終的な
///   一致/不一致の結果 (「一致する候補が存在するか」という真偽値) は
///   変わらない。
/// - [`Combinator::NextSibling`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>
///   §14.3, verbatim: "The elements represented by the two compound
///   selectors share the same parent in the document tree and the element
///   represented by the first compound selector immediately precedes the
///   element represented by the second one. Non-element nodes (e.g. text
///   between elements) are ignored when considering the adjacency of
///   elements.") — 候補は `current_id` の親 (`ancestors.last()`、無ければ
///   [`StyleDom::root_id`]、下記 note 参照) の子リストの中で `current_id`
///   の**直前**の element 1 つだけ ([`immediate_preceding_sibling`])。
///   バックトラックは無い — `+` は「直前の兄弟」を一意に指すため
///   ([`Combinator::Child`] と同じ形)。
/// - [`Combinator::LaterSibling`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#general-sibling-combinators> §14.4,
///   verbatim: "The elements represented by the two compound selectors
///   share the same parent in the document tree and the element
///   represented by the first compound selector precedes (not necessarily
///   immediately) the element represented by the second one.") —
///   `current_id` の親の子リストを先頭から順に試し、`current_id` に達したら
///   打ち切る。一致する候補が見つかり次第、その候補を起点にさらに左の残りを
///   再帰的に判定する ([`Combinator::Descendant`] と同じ「単一の直線を
///   バックトラックする」形 — 兄弟リストも分岐が無いため、探索順序は正しさに
///   影響しない。ここでは `child_ids` が返す自然な順序 (先頭 = 最も遠い兄弟)
///   のまま辿る)。
///
/// # 親の解決: `ancestors.last()` の空スライス fallback
///
/// `ancestor_path` は **Element kind の node のみ**を積む
/// ([`super::collect::collect_cascaded`] doc 参照) ので、`current_id` の親が
/// [`StyleNodeKind::Document`] root 自身であるとき (= document 直下の
/// element、`<html>` 等) `ancestors` は空になる — `Child`/`Descendant` は
/// この場合を「親が compound に一致し得ない」= 不一致として正しく扱う
/// (`ancestors.split_last() => None`) が、sibling combinator は**親自身を
/// compound と照合するわけではなく**、[`StyleDom::child_ids`] の lookup key
/// として親の id が要るだけ — root であっても兄弟は実在しうる (`<h2>` と
/// `<p>` が両方 document 直下の兄弟、という acceptance のケースそのもの)。
/// そのため `NextSibling`/`LaterSibling` の 2 arm だけ `ancestors.last()` が
/// `None` のとき [`StyleDom::root_id`] にフォールバックする — `Child`/
/// `Descendant` 側はこのフォールバックを持たない (持ってはならない — root は
/// 決して compound に一致しない)。
///
/// 他 combinator ([`Combinator::PseudoElement`] / [`Combinator::SlotAssignment`]
/// / [`Combinator::Part`]) はこの関数の対応範囲外。うち
/// [`Combinator::SlotAssignment`]/[`Combinator::Part`] は引き続き
/// pseudo-element 専用の combinator で、本 crate の `parse_selector_list`
/// (`RaikiriSelectorImpl`) が対応する `::slotted()`/`::part()` 構文自体を
/// `parse_slotted`/`parse_part` 未 override のため parse error にする
/// (`ruletree.rs` `is_supported_selector` doc 参照) ので、この crate 内で
/// 生成された `SelectorList` から到達することは無い。
/// [`Combinator::PseudoElement`] (`::before`/`::after`) は事情が異なる —
/// 今はもう parse error ではなく、`SelectorList` に普通に乗って rule tree
/// にも残る (`ruletree.rs` `is_supported_selector` が受理する) が、
/// [`super::collect::collect_cascaded`] が [`selector_matches_pseudo_element`] という
/// 独立した matcher へ**この関数を経由させる前に**振り分けるため、
/// [`match_combinator_chain`] のどちらの呼び出し元 ([`selector_matches`] /
/// [`match_from_element`] 自身の再帰) もこの combinator を渡すことは無い。
/// [`compound_matches`] の `_ => false` safety net と
/// 同じ姿勢で、いずれの combinator も (到達すれば) ここでは match fail 扱い
/// にする。
///
/// # Spec provenance note
///
/// この doc および [`match_complex_selector_list`] / [`super::collect::collect_cascaded`]
/// が引用する verbatim 文言はすべて、`https://www.w3.org/TR/selectors-4/`
/// への直接 WebFetch がページ全体の大きさのため section 14 (Combinators) は
/// おろか `#complex` (§4) にすら到達する前に繰り返し切り詰められたことを
/// 受け、代わりに同一文書の正典 source である CSSWG bikeshed 原稿
/// (`raw.githubusercontent.com/w3c/csswg-drafts/main/selectors-4/Overview.bs`)
/// から確認したもの — TR ページの当該 anchor への
/// 直接到達はできていない。descendant/child/next-sibling/general-sibling
/// combinator の文言 (定義文中心の安定した記述、4 つとも同じ `<h3 id=…>`
/// 形式の見出し直下) はこの ED 原稿の内容が publish 済み TR とも一致して
/// いると見込んで TR anchor (`#descendant-combinators` /
/// `#child-combinators` / `#adjacent-sibling-combinators` /
/// `#general-sibling-combinators`) に紐付けたままにしているが、`#complex`
/// (complex selector 全体の match 条件) は ED 側の周辺記述に
/// pseudo-compound selector 関連の、TR 発行後に追加された可能性のある文言が
/// 混在しており、そちらは "TR と一致しているはず" という前提を置かず ED URL
/// (<https://drafts.csswg.org/selectors-4/#complex>) 自体に紐付けている
/// ([`match_complex_selector_list`] の引用も同様)。§14.3/§14.4 の節番号は
/// 同じ ED 原稿内の `<h2 id="combinators">` 配下の `<h3>` 出現順
/// (descendant, child, adjacent-sibling, general-sibling) から数えたもの。
///
/// # Implementation: explicit `Vec` stack, not native recursion
///
/// Prior to this fix, this function and [`match_from_element`]
/// mutually recursed on the native Rust call stack — one stack frame pair
/// per combinator actually walked while matching successively along the
/// ancestor/sibling chain, with no selector-length/complexity cap anywhere
/// in the parse/build path. That is the same class of problem
/// `collect_cascaded` had pre-job-199 (tree-depth-correlated native
/// recursion on untrusted-depth input) — confirmed empirically here too:
/// regression test `deep_child_combinator_chain_small_stack_no_overflow`
/// (this module's `tests`) reliably aborted the process with a native stack
/// overflow (128 KiB stack, 500-deep uniformly-matching `>` chain) against
/// the prior recursive implementation.
///
/// The fix below uses an explicit `Vec`-based stack, the same *technique*
/// `collect_cascaded` uses for its own job-199 fix — but not the same
/// *shape*: `collect_cascaded` is a plain DFS with no backtracking (visit
/// every node once), whereas [`Combinator::Descendant`] /
/// [`Combinator::LaterSibling`] must try multiple candidates in order and
/// fall back to the next one when a deeper match fails entirely (see this
/// doc's "load-bearing" retry note above). So each stack entry here is a
/// *choice point* (a [`PendingCandidates`] cursor over not-yet-tried
/// candidates for one level of the chain, plus the ancestors to hand a
/// matched candidate and the [`SelectorIter`] to resume from) rather than a
/// bare node id — pushed when a candidate's compound matches and a further
/// combinator remains (going one level deeper/further left), popped when a
/// level's candidates are exhausted (backtracking to the next candidate of
/// the parent choice point). A full match short-circuits immediately
/// (`return true`) without draining the stack; only exhausting the
/// outermost choice point's candidates yields an overall `false`.
///
/// The word "再帰的に" ("recursively") in the per-combinator prose above
/// describes the *logical* structure of the search — CSS complex selectors
/// are themselves defined recursively (CSSWG ED `#complex`, cited above) —
/// not this function's implementation technique; that logical recursion is
/// realized here as the explicit stack's push/pop, never the native call
/// stack.
///
/// # Memoization: bounding backtracking to polynomial time
///
/// The explicit-stack rewrite above removes the native-stack-overflow risk,
/// but on its own does nothing about a second, independent problem: the
/// backtracking itself. [`Combinator::Descendant`]'s retry (this doc's
/// "load-bearing" note above) tries every remaining ancestor as a candidate,
/// and — when a candidate's compound matches but everything further left
/// ultimately fails to complete the match — falls back to the next
/// (farther) candidate. When every remaining ancestor's compound matches
/// (e.g. a `div`-only complex selector against a chain of `<div>`s) and the
/// selector is ultimately unsatisfiable (typically: it needs more ancestor
/// "slots" than the chain actually has, at that point in the search), every
/// combination of candidates gets tried before the search can conclude
/// failure. Writing `S(k)` for the number of [`match_from_element`] calls
/// needed to *prove* failure with `k` ancestors available and always more
/// remaining compounds than ancestors (the unsatisfiable case) gives the
/// recurrence `S(0) = 1`, `S(k) = S(0) + S(1) + ... + S(k-1)` (one recursive
/// call per candidate, ancestors-remaining shrinking from `k-1` down to `0`
/// as the candidate gets farther from the target) — which solves to
/// `S(k) = 2^(k-1)` for `k >= 1`. A `div`-only complex selector with exactly
/// as many compounds as the ancestor chain is deep (the "exact fit" case —
/// every compound has a slot, no unsatisfiable point is ever reached) does
/// *not* hit this bound: it matches via the same greedy
/// nearest-candidate-first path this doc's [`Combinator::Descendant`] note
/// describes, and the retry loop is never actually exercised because the
/// very first candidate at every level already leads to a full match. The
/// same selector with **one extra compound** (so the chain is one ancestor
/// short of what the selector needs, the minimal unsatisfiable case) is the
/// one that hits `S(k) = 2^(k-1)` — confirmed empirically (this crate's
/// `TestDoc` mock, `match_complex_selector_list` timed directly) to match
/// this scaling. This is an untrusted-depth CPU-exhaustion vulnerability,
/// not merely a slow path: a sufficiently deep DOM chain
/// (attacker-controlled markup depth) matched against a same-shape selector
/// (attacker-controlled stylesheet) makes `S(k)` explode long before any
/// stack limit is reached — the *fixed* [`match_combinator_chain`] above
/// still performed 2^(k-1) [`match_from_element`] calls, just on the heap
/// instead of the native stack.
///
/// Content mismatch (not just a numeric shortfall) can trigger the exact
/// same blowup: e.g. a selector whose leftmost compound is `span` matched
/// against a chain where every ancestor is `div`, with the compound *count*
/// otherwise exactly matching the ancestor count. The `span` compound never
/// matches any candidate, so the search is just as unsatisfiable as the
/// "one compound too many" case above, and every combination of candidates
/// for the intervening `div` compounds still gets tried before the `span`
/// failure is reached each time — choosing an ancestor at position `i`
/// (instead of the nearest one) leaves `i - 1` ancestors for `m - 1`
/// remaining compounds, and unless `i` happens to be exactly the position
/// that keeps the count matching, this only defers the failure rather than
/// pruning it. A fix that only compares compound and ancestor *counts*
/// (rejecting whenever remaining compounds exceed remaining ancestors)
/// would close the first shape but not this one — an attacker can always
/// reach it with a one-token substitution in an otherwise-exact-fit
/// selector, e.g. swapping one `div` for a compound the DOM never has.
///
/// The fix memoizes **candidates already proven to fail** — same
/// motivation as memoized backtracking in text-pattern matching (the
/// classic fix for the analogous `a?a?a?...aaa...a` vs `aaa...a`
/// exponential-regex-backtracking shape), applied here to a fixed
/// ancestor/sibling chain instead of a string. The key insight: at the
/// point [`match_combinator_chain`]'s loop is about to try `candidate_id`
/// against the compound `frame` (the current top of the explicit stack)
/// introduces, "does `candidate_id` satisfy this compound *and* everything
/// further left" is a pure function of exactly two things —
///
/// - `candidate_id` itself (which pins down its exact position in the
///   fixed ancestor chain, and hence its own remaining-ancestors slice —
///   see the soundness argument below), and
/// - how many compounds are left to satisfy, i.e. how many frames are
///   currently on `stack` (`stack.len()`, called `depth` below — every
///   combinator, of all four kinds, pushes exactly one frame per level, so
///   this is a stable position marker for "which compound in the fixed
///   original chain are we about to test" regardless of how many
///   backtracking attempts came before).
///
/// So `(candidate_id, depth)` is used as the memo key. **Soundness
/// argument for why `depth` need not be paired with the ancestors slice
/// itself**: every `ancestors`/`candidate_ancestors` value that ever
/// appears anywhere in this search — whether produced by
/// [`Combinator::Child`]/[`Combinator::Descendant`]'s `split_last`-based
/// shrinking or inherited unchanged through a
/// [`Combinator::NextSibling`]/[`Combinator::LaterSibling`] jump (siblings
/// share a parent, hence share the same full ancestor chain — this doc's
/// "load-bearing" retry note establishes this for the shrinking case, and
/// [`match_from_element`]'s own doc establishes it for the sibling case) —
/// is always *some prefix* of the one fixed root-to-target
/// ancestor array this function's own `ancestors` parameter starts with
/// (shrinking only ever drops the array's own tail, and passing it through
/// unchanged is trivially still the same prefix). That original array has
/// no repeated node (it is a single straight ancestor chain in a tree, and
/// a tree has no cycles), so a specific node's position within it — and
/// hence the length of the "everything above this node" prefix — is fixed
/// once and for all by which node it is, independent of which backtracking
/// path reached it. So `candidate_id` alone already determines the
/// ancestors it will be evaluated against; there is no way for the same
/// `(candidate_id, depth)` pair to legitimately mean two different
/// sub-problems.
///
/// Recording: a candidate is memoized as failed either immediately (its
/// compound didn't match at all) or once the frame it was pushed into (for
/// its own further-left continuation) is fully exhausted without a match —
/// at that point `(candidate_id, depth)` is a *proven* dead end, valid for
/// every future backtracking path that might otherwise re-examine the same
/// candidate at the same position. The one exception is `depth == 1` (the
/// outermost frame, this function's own entry point): nothing ever pushes a
/// *second* frame back down to `stack.len() == 1`, so that frame is visited
/// exactly once per call regardless of how many of its own candidates get
/// tried — a `depth == 1` failure can provably never be looked up again, so
/// the immediate-mismatch recording below skips it rather than pay for an
/// `insert` (and, on a query that never backtracks at all, the memo's first
/// heap allocation) that nothing will ever read. Successes are never
/// memoized either: finding one short-circuits the whole search immediately
/// (`return true`), so there is never a later query for it to serve. Every
/// other `(candidate_id, depth)` pair is thus fully resolved (real work,
/// not a cache hit) at most once per [`match_combinator_chain`] call,
/// bounding the total number of [`match_from_element`] calls to a
/// low-degree polynomial in the ancestor/compound counts instead of
/// `2^depth`. The memo (`HashSet`) is created fresh per call and never
/// shared across calls — [`Combinator::Descendant`]'s search space for one
/// element/selector pair has no bearing on any other — and `HashSet::new()`
/// performs no heap allocation until the first `insert`, so the common
/// shape of a shallow selector (few combinators, hence few distinct
/// `depth` values) failing at its very first (`depth == 1`) combinator
/// check — e.g. `nav > a` where `nav` itself doesn't match anything —
/// touches the memo only via lookups against an empty set and pays nothing
/// beyond the empty struct. Selectors with more combinators that still
/// resolve without ever backtracking do perform a handful of `insert`s (at
/// most one per combinator actually walked, not `2^depth` of them) even
/// though nothing reads them back in that particular call.
///
/// This bound is not specific to [`Combinator::Descendant`]: the same memo
/// applies uniformly to every candidate examined in this function's loop,
/// regardless of which combinator produced it, so the identical
/// pathological shape on a [`Combinator::LaterSibling`] chain (`* ~ * ~ *
/// ~ ... ~ *` against a run of uniformly-matching siblings) is bounded the
/// same way, as a direct consequence rather than a separate fix.
fn match_combinator_chain<D: StyleDom>(
    dom: &D,
    combinator: Combinator,
    current_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    iter: SelectorIter<'_, RaikiriSelectorImpl>,
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
) -> bool {
    struct Frame<'a, 's, D: StyleDom> {
        candidates: PendingCandidates<'a, D>,
        /// Ancestors [`Combinator::NextSibling`]/[`Combinator::LaterSibling`]
        /// candidates are evaluated against — unchanged across every
        /// candidate in this frame (siblings share a parent). Unused by
        /// [`Combinator::Child`]/[`Combinator::Descendant`], whose
        /// candidates carry their own (shrinking) ancestors directly out of
        /// `PendingCandidates::next` instead.
        ancestors_unchanged: &'a [StyleNodeId],
        /// Positioned at the compound this frame's combinator introduced;
        /// cloned fresh for every candidate attempt (`Descendant`/
        /// `LaterSibling` mirror the pre-fix code's own per-candidate
        /// `iter.clone()`; `Child`/`NextSibling` are newly cloned here too,
        /// for uniform frame handling across all four combinators — each
        /// has exactly one candidate, so the pre-fix code moved `iter`
        /// instead of cloning it. Negligible cost, no heap allocation; the
        /// broader allocation picture may be revisited in future perf
        /// work).
        iter: SelectorIter<'s, RaikiriSelectorImpl>,
        /// `(candidate_id, depth)` memo key this frame was pushed *for* —
        /// i.e. what to record as a proven dead end in `memo` once this
        /// frame's own candidates are exhausted without a match (see this
        /// function's "Memoization" doc). `None` only for the outermost
        /// frame (pushed directly from this function's own arguments, not
        /// from a candidate choice one level up — there is nothing to
        /// record a failure *against* for it, and its own exhaustion is
        /// this function's overall `false` return, not a sub-result any
        /// other choice point could ever re-query).
        origin: Option<(StyleNodeId, usize)>,
    }

    let mut memo: HashSet<(StyleNodeId, usize)> = HashSet::new();
    // The stack is seeded with one frame and gains one more per combinator
    // crossed. Building it as `vec![frame]` (the previous form) allocates
    // for exactly one element, so pushing a second frame — i.e. a selector
    // with 2+ combinators — already forces a reallocate-and-copy of every
    // frame so far. Reserving 4 slots up front covers up to 3 crossed
    // combinators (stack length 1 through 4) without regrowing; a selector
    // with 4 crossed combinators (5 frames) still regrows once, from
    // capacity 4 to 8.
    let mut stack = Vec::with_capacity(4);
    stack.push(Frame {
        candidates: pending_candidates_for(dom, combinator, current_id, ancestors),
        ancestors_unchanged: ancestors,
        iter,
        origin: None,
    });
    loop {
        // `stack.len()` at this point uniquely identifies "which compound
        // in the fixed original chain the frame about to be examined
        // introduced" — see this function's "Memoization" doc for why this
        // is a stable position marker independent of backtracking history.
        let depth = stack.len();
        let Some(frame) = stack.last_mut() else {
            // Outermost choice point exhausted with no full match found.
            return false;
        };
        let Some((candidate_id, candidate_ancestors)) =
            frame.candidates.next(dom, frame.ancestors_unchanged)
        else {
            // This level's candidates are exhausted — backtrack to the
            // parent choice point's next candidate. Everything this frame
            // could have tried has failed, so the candidate that led here
            // (if any — the outermost frame has none) is now a proven dead
            // end for any other backtracking path that reaches it too.
            let popped = stack.pop().expect("frame just borrowed via last_mut");
            if let Some(key) = popped.origin {
                memo.insert(key);
            }
            continue;
        };
        if memo.contains(&(candidate_id, depth)) {
            // A different backtracking path already proved this exact
            // candidate fails at this exact position in the chain — skip
            // straight to this frame's next candidate without redoing the
            // (possibly deep) exploration.
            continue;
        }
        let candidate_iter = frame.iter.clone();
        let Some(mut matched_iter) = match_from_element(
            dom,
            candidate_id,
            candidate_ancestors,
            candidate_iter,
            quirks_mode,
            relative_anchor,
        ) else {
            // Candidate's compound didn't match — try this frame's next
            // candidate (loop back without push/pop). Immediate failure,
            // same as an exhausted pushed frame would record. Skipped at
            // `depth == 1`: that's the outermost frame, visited exactly
            // once per call (nothing ever pushes a second frame back down
            // to `stack.len() == 1`), so a depth-1 entry can provably never
            // be read back — recording it would just be a wasted `insert`
            // (and, on an otherwise retry-free failure, the first heap
            // allocation this memo would ever make).
            if depth != 1 {
                memo.insert((candidate_id, depth));
            }
            continue;
        };
        match matched_iter.next_sequence() {
            // No further combinator to the left: the whole complex
            // selector matched. Short-circuits immediately — nothing to
            // memoize, there is no later query this result could serve.
            None => return true,
            Some(next_combinator) => {
                // Compound matched and more remains further left — descend
                // one level (push a new choice point) rather than recurse.
                stack.push(Frame {
                    candidates: pending_candidates_for(
                        dom,
                        next_combinator,
                        candidate_id,
                        candidate_ancestors,
                    ),
                    ancestors_unchanged: candidate_ancestors,
                    iter: matched_iter,
                    origin: Some((candidate_id, depth)),
                });
            }
        }
    }
}

/// Not-yet-tried candidates for one [`match_combinator_chain`] choice point
/// — the iterative counterpart of that function's four `match combinator`
/// arms' candidate-generation logic. Each variant
/// corresponds 1:1 to a [`Combinator`] arm; see [`pending_candidates_for`]
/// for the construction side and [`match_combinator_chain`]'s "Implementation"
/// doc for why this needs to be a resumable cursor rather than a one-shot
/// iterator (backtracking may resume a frame after a deeper level failed).
enum PendingCandidates<'a, D: StyleDom + 'a> {
    /// [`Combinator::Child`]: exactly one candidate
    /// (`ancestors.split_last()`'s parent, paired with the remaining
    /// ancestors above it) — `None` once taken, or if there was no parent
    /// to begin with. No backtracking past this single candidate, matching
    /// the pre-fix code's non-looping `match ancestors.split_last() { .. }`.
    Child(Option<(StyleNodeId, &'a [StyleNodeId])>),
    /// [`Combinator::Descendant`]: remaining ancestors to try, closest-first
    /// — mirrors the pre-fix code's `while let Some((&id, further)) =
    /// remaining.split_last()` loop. Each candidate is handed the *further*
    /// ancestors (everything above it) both as its own matching context and
    /// as the next resume point.
    Descendant(&'a [StyleNodeId]),
    /// [`Combinator::NextSibling`]: exactly one candidate (the immediate
    /// preceding sibling, if any), evaluated against the frame's unchanged
    /// `ancestors_unchanged`.
    NextSibling(Option<StyleNodeId>),
    /// [`Combinator::LaterSibling`]: children of the shared parent up to
    /// (excluding) `stop_at`, in document order — mirrors the pre-fix
    /// code's single-pass `for candidate_id in dom.child_ids(parent_id) {
    /// if == current_id { break } .. }` loop. Holds a *live* `D::ChildIter`
    /// (not a re-derived one) so resuming this frame after a deeper level
    /// fails continues exactly where the previous attempt left off — same
    /// single left-to-right pass as the original, no re-scan, no throwaway
    /// `Vec` (same convention [`immediate_preceding_sibling`]'s doc
    /// establishes).
    LaterSibling {
        child_iter: D::ChildIter<'a>,
        stop_at: StyleNodeId,
    },
}

impl<'a, D: StyleDom + 'a> PendingCandidates<'a, D> {
    /// Advances to (and returns) the next untried candidate, paired with
    /// the ancestors it should be evaluated against, or `None` once this
    /// choice point is exhausted. `ancestors_unchanged` is the frame's own
    /// field (not stored on `Self` — only the [`Combinator::NextSibling`]/
    /// [`Combinator::LaterSibling`] variants need it, and every candidate
    /// within a frame needs the *same* value, so the caller threads it
    /// through rather than duplicating it per variant).
    fn next(
        &mut self,
        dom: &D,
        ancestors_unchanged: &'a [StyleNodeId],
    ) -> Option<(StyleNodeId, &'a [StyleNodeId])> {
        match self {
            Self::Child(slot) => slot.take(),
            Self::Descendant(remaining) => {
                let (&candidate_id, further) = remaining.split_last()?;
                *remaining = further;
                Some((candidate_id, further))
            }
            Self::NextSibling(slot) => slot.take().map(|id| (id, ancestors_unchanged)),
            Self::LaterSibling {
                child_iter,
                stop_at,
            } => {
                for candidate_id in child_iter.by_ref() {
                    if candidate_id == *stop_at {
                        // Reached `current_id` itself: no more candidates
                        // are ever valid past this point (mirrors the
                        // pre-fix code's `break`). Not resetting the
                        // iterator is fine — a choice point that returned
                        // `None` once is never queried again by
                        // `match_combinator_chain`'s driving loop.
                        return None;
                    }
                    if is_in_document_element(dom, candidate_id) {
                        return Some((candidate_id, ancestors_unchanged));
                    }
                }
                // cov:ignore: same invariant as `immediate_preceding_sibling`'s
                // own trailing `None` — `stop_at` (`current_id`) is always one
                // of `parent_id`'s own children (it is `pending_candidates_for`'s
                // own `current_id` argument, and `parent_id` is derived from
                // it via `ancestors.last()`), so the loop above always returns
                // via the `*stop_at` branch before `child_iter` is exhausted.
                // Would need a `StyleDom` impl whose `child_ids(parent_id)`
                // omits an id it itself supplied as `current_id` to exercise.
                None
            }
        }
    }
}

/// Builds the [`PendingCandidates`] cursor for one combinator, mirroring
/// [`match_combinator_chain`]'s pre-fix per-combinator candidate-generation
/// logic exactly — this function does no matching
/// itself, only candidate enumeration setup. The `_ => ..` safety-net arm
/// (unsupported combinators, see this module's "他 combinator" doc note)
/// yields an already-exhausted `Child(None)` cursor, the same "no candidate
/// ever succeeds" outcome the pre-fix `_ => false` arm produced.
fn pending_candidates_for<'a, D: StyleDom + 'a>(
    dom: &'a D,
    combinator: Combinator,
    current_id: StyleNodeId,
    ancestors: &'a [StyleNodeId],
) -> PendingCandidates<'a, D> {
    match combinator {
        Combinator::Child => PendingCandidates::Child(
            ancestors
                .split_last()
                .map(|(&parent_id, rest)| (parent_id, rest)),
        ),
        Combinator::Descendant => PendingCandidates::Descendant(ancestors),
        Combinator::NextSibling => {
            let parent_id = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
            PendingCandidates::NextSibling(immediate_preceding_sibling(dom, parent_id, current_id))
        }
        Combinator::LaterSibling => {
            let parent_id = ancestors.last().copied().unwrap_or_else(|| dom.root_id());
            PendingCandidates::LaterSibling {
                child_iter: dom.child_ids(parent_id),
                stop_at: current_id,
            }
        }
        // cov:ignore: `Combinator::SlotAssignment`/`Part` are structurally
        // unconstructible here, and `Combinator::PseudoElement` is
        // constructible in a `SelectorList` but never reaches this function
        // — each for a different reason:
        //   - `Part`: gated by `Parser::parse_part()`, default `false`,
        //     `RaikiriSelectorParser` doesn't override it, so `::part()`
        //     never parses at all.
        //   - `SlotAssignment`: same, gated by `Parser::parse_slotted()`.
        //   - `PseudoElement`: `RaikiriSelectorParser` *does* override
        //     `parse_pseudo_element` (accepts `::before`/`::after`, see
        //     `crate::PseudoElem` doc) — this combinator is real and does
        //     appear in successfully-parsed `Selector`s now. It never
        //     reaches `match_combinator_chain` (and so never reaches this
        //     function) from either of `match_combinator_chain`'s two call
        //     sites: `selector_matches`'s real-element path never gets
        //     past `compound_matches`'s own `_ => false` arm on the
        //     rightmost `Component::PseudoElement` compound (matching a
        //     real element against `.foo::before` fails right there,
        //     before any `next_sequence()`/combinator crossing happens);
        //     `selector_matches_pseudo_element`'s own dedicated path does
        //     cross this exact combinator, but does so itself (its own
        //     `iter.next_sequence()`, asserted via `debug_assert_eq!`)
        //     *before* calling `match_combinator_chain` — by then the
        //     iterator is already positioned past it, and a `Selector` has
        //     at most one `Component::PseudoElement`/`Combinator::
        //     PseudoElement` pair (see that function's doc), so it cannot
        //     appear a second time further down the chain either.
        // See this module's "他 combinator" doc note, above
        // `match_combinator_chain`, for the fuller argument. Yields an
        // already-exhausted `Child(None)` cursor — same "no candidate ever
        // succeeds" outcome the pre-fix `_ => false` arm produced.
        _ => PendingCandidates::Child(None),
    }
}

/// `parent_id`'s direct children (document order) が `Element` kind かつ
/// [`StyleNode::is_in_document`] であるかを判定する共有述語。
/// [`immediate_preceding_sibling`] と [`match_combinator_chain`] の
/// `LaterSibling` arm の両方から使う — [`super::collect::collect_cascaded`] が
/// `ancestor_path` に積む前に行う `!node.is_in_document() => continue` gate
/// (同関数の doc 参照) と同じ基準を、sibling 側の候補選定でも揃えるための
/// 抽出 — 揃えないと `<template>` 子孫のような
/// inert element が sibling combinator の候補として拾われてしまう。
fn is_in_document_element<D: StyleDom>(dom: &D, id: StyleNodeId) -> bool {
    dom.node(id)
        .is_some_and(|node| node.is_in_document() && node.kind() == StyleNodeKind::Element)
}

/// `parent_id` の直接の子のうち、`current_id` の**直前**にいる element の id
/// ([`Combinator::NextSibling`] 用)。[`StyleDom::child_ids`] を先頭から 1
/// パス走査し、`current_id` に達した時点でそれまでに見た最後の element
/// candidate を返す — 割り当ては行わない (`Vec` 不使用、他 helper と
/// 同じ「使い捨て `Vec` を経由しない」方針を踏襲)。
///
/// Non-element node (text 等) は候補から除外 — CSS Selectors L4
/// next-sibling combinator 自身の verbatim: "Non-element nodes (e.g. text
/// between elements) are ignored when considering the adjacency of
/// elements" (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>)。
fn immediate_preceding_sibling<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    current_id: StyleNodeId,
) -> Option<StyleNodeId> {
    let mut last_element = None;
    for candidate_id in dom.child_ids(parent_id) {
        if candidate_id == current_id {
            return last_element;
        }
        if is_in_document_element(dom, candidate_id) {
            last_element = Some(candidate_id);
        }
    }
    // cov:ignore: `current_id` is always one of `parent_id`'s own children
    // when this helper is called — `parent_id` is derived from `current_id`
    // itself (either `current_id`'s own parent via `ancestors.last()`, or —
    // when `current_id` was reached through a prior sibling/ancestor jump —
    // the parent shared with the node that produced it, see
    // `match_combinator_chain`'s callers). Would need a `StyleDom` impl
    // whose `child_ids(parent_id)` omits an id it itself supplied as
    // `ancestors.last()` / a sibling candidate to exercise this branch.
    None
}

/// `elem_id` の element を解決し、[`compound_matches`] で `iter` が指す
/// compound をそれに対して判定する。祖先候補 (`Child`/`Descendant`) と
/// 兄弟候補 (`NextSibling`/`LaterSibling`) の両方がこの 1 つの関数を共有する
/// — 「id を解決して compound を照合する」というロジック自体は候補がどちらの
/// combinator 由来かに依存しない (`ancestors` は
/// 兄弟ジャンプでは不変のまま引き継がれる — 兄弟は親を共有するため — ことが
/// この共有を成立させる。祖先ジャンプでは従来通り `split_last`/バックトラック
/// で truncate 済みの残り `ancestors` を渡す)。旧名 `match_from_ancestor`
/// — 兄弟候補にも使われるようになったため
/// `match_from_element` に rename。
///
/// 以前は、compound が一致した後さらに左の
/// combinator へ**自分で再帰**していた ([`match_combinator_chain`] との
/// 相互再帰、native stack を消費する側)。現在は compound 一致後の `iter`
/// (次の compound の手前まで進んだ状態) を `Some` で返すだけに変わり、
/// 「さらに左の combinator へ進むかどうか」の判断とその実行は
/// [`match_combinator_chain`] の explicit `Vec` stack 駆動ループ側の責務に
/// 一本化されている (同関数の "Implementation" doc 参照) — 呼び出し側が
/// 自分の判断で `matched_iter.next_sequence()` を呼び、`stack.push` するか
/// `return true` するかを選ぶ。
///
/// `elem_id` を [`StyleElement`] の借用値ではなく [`StyleNodeId`] で受け取る
/// 設計: `StyleElement` は [`StyleDom::NodeRef`]/[`StyleNode::Element`] と
/// いう GAT 経由の型で、呼び出しをまたいで別の借用ライフタイムの値を
/// 持ち回るにはシグネチャが煩雑になる — id は `Copy` なのでこの受け渡しには
/// 明らかに軽量。[`super::collect::collect_cascaded`] 側で既に解決済みの `elem` を再利用
/// しない分、候補 1 段ごとに `dom.node()`/`as_element()` を 1 回余分に
/// 呼ぶが、raikiri-style crate-internal な `#[cfg(test)]` 限定 mock
/// (`TestDoc`) / raikiri-dom の実装いずれも arena index 参照相当の安価な
/// lookup (`TestDoc` の宿る module は `#[cfg(test)]` gated のため、ここは
/// あえて intra-doc link 化しない — non-test の `cargo doc` からは解決
/// できない target になる)。
///
/// Returns: compound が一致すれば、その後の compound を指す `iter` を
/// `Some` で返す (呼び出し側がさらに左へ進めるかどうかを判断する)。
/// 一致しなければ `None`。
fn match_from_element<'s, D: StyleDom>(
    dom: &D,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    mut iter: SelectorIter<'s, RaikiriSelectorImpl>,
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
) -> Option<SelectorIter<'s, RaikiriSelectorImpl>> {
    // Both guards below are defensive and not reachable via the real
    // `collect_cascaded` → `match_complex_selector_list` call path: every
    // `elem_id` this function is ever invoked with comes from one of two
    // sources, both already filtered to Element-kind + in-document ids —
    // `ancestors` (built by `collect_cascaded`'s `ancestor_path`, which only
    // ever pushes an id inside its `node.kind() == StyleNodeKind::Element`
    // branch after the `!node.is_in_document() => continue` gate — see that
    // function's doc), or a sibling candidate already passed through
    // `is_in_document_element` (`immediate_preceding_sibling` /
    // `PendingCandidates::LaterSibling`). So `dom.node(elem_id)` is always
    // `Some`, and its `as_element()` is always `Some` too. Kept as an
    // explicit safety net rather than `.unwrap()`/`unreachable!()` — same
    // defensive posture as `compound_matches`'s own `_ => false` arm for
    // unsupported `Component` variants — because `StyleDom`/`StyleElement`
    // are generic traits not owned by this crate; a future non-test
    // implementation could theoretically violate the invariant.
    // cov:ignore: unreachable given the construction invariants above;
    // would need a `StyleDom` impl that returns `None`/non-Element for an id
    // it itself supplied as an ancestor or a filtered sibling candidate to
    // exercise.
    let node = dom.node(elem_id)?;
    // cov:ignore: see the guard immediately above — same invariant.
    let elem = node.as_element()?;
    // `ancestors` here is *this* element's own remaining ancestor chain
    // (root-most first) — `pending_candidates_for`'s `Child`/`Descendant`
    // arms hand out `rest`/`further` (everything left after popping
    // `elem_id` itself off the end), so `ancestors.last()` is `elem_id`'s
    // parent, exactly mirroring `match_complex_selector_list`'s own use of
    // `ancestors` for the rightmost compound —
    // needed so a structural pseudo-class in a non-rightmost compound
    // (e.g. `body > div:only-child p`) resolves against the right parent,
    // not `elem`'s (the search's original caller's) parent. Sibling jumps
    // pass `ancestors` through unchanged (siblings
    // share a parent), so this holds for those candidates too.
    if !compound_matches(
        dom,
        &mut iter,
        &elem,
        elem_id,
        ancestors,
        quirks_mode,
        relative_anchor,
    ) {
        return None;
    }
    Some(iter)
}

// ---------------------------------------------------------------------------
// `:lang()` / `:dir()`.
//
// Both pseudo-classes resolve a property of the element that is NOT a plain
// own-attribute lookup — CSS Selectors L4 explicitly distinguishes them from
// the attribute-selector equivalent (`[lang|=C]` / `[dir=C]`) precisely
// because they consult "the UA's knowledge of the document's semantics"
// (`:dir()`'s own wording, quoted on `resolve_directionality`'s doc) —
// concretely, ancestor inheritance. Both therefore reuse the same
// `ancestors: &[StyleNodeId]` (root-first, immediate-parent-last) that
// `compound_matches` already threads through for descendant/child combinator
// matching — self is checked first, then
// `ancestors` is walked from `.last()` (immediate parent) toward `.first()`
// (document root).
// ---------------------------------------------------------------------------

/// `Component::AttributeInNoNamespace`'s `ParsedCaseSensitivity` (spec-only,
/// "language depends on this" placeholder for the
/// `AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument` case) を、実際に
/// `AttrSelectorOperator::eval_str` へ渡せる `CaseSensitivity` へ解決する。
///
/// upstream `selectors::matching::to_unconditional_case_sensitivity` と同じ
/// 3-way 分岐を model 化しているが、その関数は tree-walk 込みの重い
/// `selectors::Element` trait を要求するため呼べない (raikiri の
/// `StyleElement` は single-element matching 用の縮小 trait —
/// `wall/traits` を跨がない private helper として
/// 再実装)。raikiri は現時点で HTML document のみ対象 (XML/XHTML 未対応) の
/// ため「in html document」は常に true 扱い。「is html element」は
/// [`StyleElement::namespace_uri`] の既存 contract
/// (style_dom.rs: "HTML default namespace returns None (optimized path)") を
/// 代理指標として使う — SVG 等 non-HTML namespace の element は
/// case-sensitive 側に倒す。
///
/// # 「in html document」は quirks-mode と別軸
///
/// ここでの "in html document" は CSS Selectors L4 §3.7/§6.3 が定める
/// document-**language** (HTML vs XML) 軸であり、id/class matching が使う
/// quirks-mode 軸 (`StyleQuirksMode`、CSS Selectors L4 §6.6/§6.7) とは
/// spec 上別概念 — 混同しないこと。raikiri-html は HTML5 tree builder のみで
/// XML document を生成する経路が無いため、この軸は現状 unconditionally true
/// で正しい。XML document parsing が入るときに、document-language 信号を
/// この関数へ渡す配線が必要になる。
fn resolve_case_sensitivity<E: StyleElement>(
    parsed: ParsedCaseSensitivity,
    elem: &E,
) -> CaseSensitivity {
    match parsed {
        ParsedCaseSensitivity::CaseSensitive | ParsedCaseSensitivity::ExplicitCaseSensitive => {
            CaseSensitivity::CaseSensitive
        }
        ParsedCaseSensitivity::AsciiCaseInsensitive => CaseSensitivity::AsciiCaseInsensitive,
        ParsedCaseSensitivity::AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument => {
            if elem.namespace_uri().is_none() {
                CaseSensitivity::AsciiCaseInsensitive
            } else {
                CaseSensitivity::CaseSensitive
            }
        }
    }
}

// cov:ignore: pure test-code relocation (no logic changed). patch-coverage's git-diff-based line classifier treats every moved
// line as newly added, and cargo-llvm-cov does not record hits for
// multi-line string-literal continuation lines inside assert!/panic!
// messages even though the containing statement executes in a
// passing test. Verified against every flagged line in this move:
// all are string-literal fragments or trivial format-arg
// expressions inside already-passing tests, none of them
// production code.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::cascade;
    use crate::cascade::test_support::*;
    use crate::computed::ComputedValues;
    use crate::property::BorderStyle;
    use crate::ruletree::{Origin, RuleTree, build_rule_tree};
    use crate::test_dom::TestDoc;

    #[test]
    fn type_selector_applies_color() {
        let cv = cascade_doc("p { color: red }", "p", None);
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn class_selector_applies_declaration() {
        // Acceptance: `.chapter-title { font-weight:
        // bold }` applied to `<p class="chapter-title">` — `font-weight: bold`
        // computes to 700.0 (property.rs `parse_font_weight`).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter-title { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "chapter-title");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
    }

    #[test]
    fn class_selector_does_not_match_element_without_the_class() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter-title { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "intro"); // different token

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_weight, 400.0,
            "initial, rule must not apply"
        );
    }

    #[test]
    fn class_selector_matches_one_token_among_several() {
        // `class="a b c"` — HTML-spec ASCII whitespace split
        // (`StyleElement::has_class` doc), `.b` must match the middle token.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".b { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "a b c");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
    }

    #[test]
    fn class_selector_is_case_sensitive() {
        // CSS Selectors L4 class-html: HTML class matching in standards mode
        // is case-sensitive (`StyleElement::has_class` default impl does an
        // exact token compare, no ASCII-case-folding).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".Foo { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo"); // different case

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_weight, 400.0,
            "initial, case must not fold"
        );
    }

    #[test]
    fn class_selector_case_sensitivity_across_quirks_modes() {
        struct Case {
            mode: StyleQuirksMode,
            element_class: &'static str,
            expect_match: bool,
        }
        let cases = [
            // NoQuirks (standards mode): always case-sensitive.
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_class: "foo",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_class: "FOO",
                expect_match: false,
            },
            // LimitedQuirks ("almost standards"): distinct DOM Standard dfn
            // from "quirks mode" — must NOT fold, same as NoQuirks.
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_class: "foo",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_class: "FOO",
                expect_match: false,
            },
            // Quirks (full quirks mode): ASCII case-insensitive fold.
            Case {
                mode: StyleQuirksMode::Quirks,
                element_class: "foo",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::Quirks,
                element_class: "FOO",
                expect_match: true,
            },
        ];

        for case in cases {
            let mut doc = TestDoc::new();
            doc.quirks_mode = case.mode;
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, ".foo { font-weight: bold }");
            let p = doc.push_element(0, "p", None);
            doc.set_attr(p, "class", case.element_class);

            let tree = build_rule_tree(&doc);
            let r = cascade(&doc, &tree).expect("cascade Ok");
            let matched = r.computed[p].font_weight == 700.0;
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                matched, case.expect_match,
                "mode={:?} element_class={:?}: expected match={}",
                case.mode, case.element_class, case.expect_match
            );
        }
    }

    #[test]
    fn has_class_ascii_case_insensitive_empty_query_is_false() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo");
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        assert!(!elem.has_class_ascii_case_insensitive(""));
    }

    #[test]
    fn id_selector_applies_declaration() {
        // Acceptance: `#header { ... }` applied to
        // `<div id="header">`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#header { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "id", "header");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn id_selector_does_not_match_different_id() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#header { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "id", "footer");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn id_selector_case_sensitivity_across_quirks_modes() {
        struct Case {
            mode: StyleQuirksMode,
            element_id: &'static str,
            expect_match: bool,
        }
        let cases = [
            // NoQuirks (standards mode): always case-sensitive.
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_id: "header",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::NoQuirks,
                element_id: "HEADER",
                expect_match: false,
            },
            // LimitedQuirks ("almost standards"): distinct DOM Standard dfn
            // from "quirks mode" — must NOT fold, same as NoQuirks.
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_id: "header",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::LimitedQuirks,
                element_id: "HEADER",
                expect_match: false,
            },
            // Quirks (full quirks mode): ASCII case-insensitive fold.
            Case {
                mode: StyleQuirksMode::Quirks,
                element_id: "header",
                expect_match: true,
            },
            Case {
                mode: StyleQuirksMode::Quirks,
                element_id: "HEADER",
                expect_match: true,
            },
        ];

        for case in cases {
            let mut doc = TestDoc::new();
            doc.quirks_mode = case.mode;
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, "#header { color: red }");
            let div = doc.push_element(0, "div", None);
            doc.set_attr(div, "id", case.element_id);

            let tree = build_rule_tree(&doc);
            let r = cascade(&doc, &tree).expect("cascade Ok");
            let matched = r.computed[div].color == RED;
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                matched, case.expect_match,
                "mode={:?} element_id={:?}: expected match={}",
                case.mode, case.element_id, case.expect_match
            );
        }
    }

    #[test]
    fn attribute_exists_selector_applies_declaration() {
        // Acceptance: `[data-foo]` matches any
        // element carrying that attribute, regardless of its value.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "anything");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_exists_selector_does_not_match_when_attr_absent() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo] { color: red }");
        let div = doc.push_element(0, "div", None); // no data-foo at all

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_exists_selector_does_not_match_empty_value_attr() {
        // Pins `TestDoc`'s own `StyleElement::attr` override
        // (`test_dom.rs`), which deliberately keeps the older, stricter
        // "empty value is normalised to `None`" behavior as a
        // simplification local to this mock. Against `TestDoc`,
        // `data-foo=""` reads back as attribute-absent, so `[data-foo]`
        // does not match here.
        //
        // **This is `TestDoc`-only, not the real DOM's behavior.**
        // `raikiri-dom::dom_impl::ElementRef::attr` (the real DOM impl)
        // tracks attribute presence independent of value, so
        // `data-foo=""` does match `[data-foo]` there — this test's name
        // and outcome describe the mock's narrower contract, not a general
        // engine-level accepted-baseline divergence from CSS Selectors L4.
        // See `StyleElement::attr`'s trait doc (style_dom.rs) for the full
        // contract and this divergence's rationale.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_exact_match_selector_does_not_match_empty_value_attr() {
        // Same root cause as
        // `attribute_exists_selector_does_not_match_empty_value_attr` above,
        // pinned separately because it goes through a different
        // `compound_matches` arm (`Component::AttributeInNoNamespace`, not
        // `..Exists`): `TestDoc`'s own `StyleElement::attr` override
        // (`test_dom.rs`) collapses `foo=""` into `None` before the
        // with-value arm's `match elem.attr(...) { Some(..) => ..,
        // None => false }` ever runs, so it takes the `None => false`
        // branch regardless of the selector's own value operand.
        //
        // **This is `TestDoc`-only, not the real DOM's behavior.** Per CSS
        // Selectors L4 (<https://www.w3.org/TR/selectors-4/#attribute-selectors>),
        // `[data-foo=""]` should match an element whose `data-foo` value is
        // exactly the empty string, and `raikiri-dom::dom_impl::ElementRef::attr`
        // (the real DOM impl) does support that — it tracks presence
        // independent of value. Only `TestDoc`'s deliberately-simplified
        // mock still collapses `foo=""` to absent; this test's name and
        // outcome describe that mock, not a general engine-level
        // accepted-baseline divergence. See `StyleElement::attr`'s trait
        // doc (style_dom.rs) for the full contract and this divergence's
        // rationale.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_exists_selector_mixed_case_matches_html_element_via_lowercased_key() {
        // HTML LS "case-sensitivity of selectors"
        // (https://html.spec.whatwg.org/multipage/semantics-other.html#case-sensitivity-of-selectors):
        // attribute names on HTML elements in HTML documents are
        // ASCII-lowercased — html5ever already lower-cases them at parse
        // time (`raikiri-html::sink::wire_side_tables` stores whatever case
        // html5ever produced, unmodified). So a selector written with mixed
        // case, `[Data-Foo]`, must still match an HTML (default-namespace)
        // element whose stored attribute name is already-lowercased
        // `data-foo`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[Data-Foo] { color: red }");
        let div = doc.push_element(0, "div", None); // default namespace = HTML
        doc.set_attr(div, "data-foo", "anything");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_exists_selector_mixed_case_uses_original_case_for_foreign_namespace_element() {
        // Foreign-namespace (SVG/MathML) elements are NOT covered by HTML
        // LS's "attributes on HTML elements in HTML documents" lowercasing
        // scope — html5ever's "adjust foreign attributes" step can restore
        // specific attributes to their original mixed case (e.g. `viewBox`),
        // and `wire_side_tables` stores whatever case html5ever produced,
        // unmodified. A selector written `[Data-Foo]` against such an
        // element must use the *original-case* lookup key, not the
        // lowercased one.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[Data-Foo] { color: red }");
        let svg_el = doc.push_element(0, "rect", None);
        doc.set_namespace(svg_el, "http://www.w3.org/2000/svg");
        doc.set_attr(svg_el, "Data-Foo", "anything"); // original mixed case

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[svg_el].color, RED);
    }

    #[test]
    fn attribute_exists_selector_mixed_case_does_not_fall_back_to_lowercase_for_foreign_namespace_element()
     {
        // Same shape as the sibling test above, but the foreign-namespace
        // element carries only the *lowercased* attribute name — proving
        // the lookup is genuinely gated on the original-case key for
        // foreign elements, not silently trying both keys.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[Data-Foo] { color: red }");
        let svg_el = doc.push_element(0, "rect", None);
        doc.set_namespace(svg_el, "http://www.w3.org/2000/svg");
        doc.set_attr(svg_el, "data-foo", "anything"); // lowercased — wrong key for this element

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[svg_el].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_value_exact_match_selector_applies_declaration() {
        // Acceptance: `[data-foo="bar"]` exact-match
        // variant.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "bar");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_value_exact_match_selector_does_not_match_different_value() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "baz");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    fn assert_attribute_operator_match(selector: &str, value: &str, expected_match: bool) {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, selector);
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", value);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color == RED, expected_match);
    }

    #[test]
    fn attribute_prefix_match_selector_applies_declaration() {
        assert_attribute_operator_match(r#"[data-foo^="pre"] { color: red }"#, "prefix", true);
        assert_attribute_operator_match(r#"[data-foo^="pre"] { color: red }"#, "xprefix", false);
    }

    #[test]
    fn attribute_suffix_match_selector_applies_declaration() {
        assert_attribute_operator_match(r#"[data-foo$="fix"] { color: red }"#, "prefix", true);
        assert_attribute_operator_match(r#"[data-foo$="fix"] { color: red }"#, "fixed", false);
    }

    #[test]
    fn attribute_substring_match_selector_applies_declaration() {
        assert_attribute_operator_match(r#"[data-foo*="ref"] { color: red }"#, "prefix", true);
        assert_attribute_operator_match(r#"[data-foo*="ref"] { color: red }"#, "pfix", false);
    }

    #[test]
    fn attribute_whitespace_token_match_selector_applies_declaration() {
        assert_attribute_operator_match(
            r#"[data-foo~="beta"] { color: red }"#,
            "alpha beta gamma",
            true,
        );
        assert_attribute_operator_match(
            r#"[data-foo~="beta"] { color: red }"#,
            "alphabetagamma",
            false,
        );
    }

    #[test]
    fn attribute_hyphen_prefix_match_selector_applies_declaration() {
        assert_attribute_operator_match(r#"[data-foo|="en"] { color: red }"#, "en-US", true);
        assert_attribute_operator_match(r#"[data-foo|="en"] { color: red }"#, "english", false);
    }

    #[test]
    fn attribute_value_exact_match_is_case_sensitive_for_data_attr() {
        // CSS Selectors L4 attribute-selectors: default case-sensitivity
        // (no `i`/`s` flag) depends on the document language; `data-*` is not
        // in HTML's ASCII-case-insensitive attribute list, so it resolves to
        // `ParsedCaseSensitivity::CaseSensitive` at parse time (selectors
        // crate `AttributeFlags::to_case_sensitivity`) — no
        // `resolve_case_sensitivity` branching is even reached for this case.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\"] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "BAR"); // different case

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, ComputedValues::initial().color);
    }

    #[test]
    fn attribute_value_case_insensitive_flag_i_matches_regardless_of_case() {
        // `[foo="bar" i]` — explicit `i` flag forces ASCII-case-insensitive
        // matching regardless of the attribute's document-language default
        // (CSS Selectors L4 attribute-selectors, `AttributeFlags::AsciiCaseInsensitive`).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[data-foo=\"bar\" i] { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "data-foo", "BAR");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].color, RED);
    }

    #[test]
    fn attribute_selector_style_local_name_matches_element_with_inline_style() {
        // `StyleElement::attr`'s doc contract requires overrides to keep
        // handling `local == "style"` by delegating to
        // `inline_style_source()`; `TestElementRef::attr`
        // does this, so `[style]` — an ordinary
        // existence attribute selector whose local name happens to be
        // `style` — must match any element carrying an inline `style="…"`.
        // `font-weight` (not touched by the inline `color: blue`) is the
        // observable, since inline style otherwise always outranks any
        // stylesheet rule (`INLINE_SPECIFICITY`) regardless of whether
        // `[style]` itself matched.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[style] { font-weight: bold }");
        let p = doc.push_element(0, "p", Some("color: blue"));

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
    }

    #[test]
    fn attribute_selector_style_does_not_match_element_without_inline_style() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[style] { font-weight: bold }");
        let p = doc.push_element(0, "p", None); // no inline style

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 400.0);
    }

    #[test]
    fn compound_type_and_class_selector_requires_both() {
        // `p.chapter-title` — compound selector, AND semantics: both the type
        // and class component must match the same element.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p.chapter-title { font-weight: bold }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "chapter-title");
        let div = doc.push_element(0, "div", None); // wrong tag, same class
        doc.set_attr(div, "class", "chapter-title");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_weight, 700.0,
            "p.chapter-title must match <p class=chapter-title>"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[div].font_weight, 400.0,
            "div.chapter-title selector must not match <div class=chapter-title> (wrong tag)"
        );
    }

    #[test]
    fn descendant_combinator_applies_declaration_to_direct_child() {
        // Acceptance: `.chapter h2` applied to
        // `<div class="chapter"><h2>...</h2></div>`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "chapter");
        let h2 = doc.push_element(div, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[h2].color, RED);
    }

    #[test]
    fn descendant_combinator_applies_to_arbitrary_depth_descendant() {
        // "arbitrary descendant" (spec verbatim above) — must match even
        // through an intermediate <section> that itself matches neither
        // side of the selector.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "chapter");
        let section = doc.push_element(div, "section", None);
        let h2 = doc.push_element(section, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h2].color, RED,
            "descendant combinator must match through an intermediate non-matching ancestor"
        );
    }

    #[test]
    fn descendant_combinator_does_not_match_outside_the_subtree() {
        // Negative case: an <h2> that is not a descendant of any
        // `.chapter` must not pick up the declaration.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None); // no class="chapter"
        let h2 = doc.push_element(div, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[h2].color, ComputedValues::initial().color);
    }

    #[test]
    fn child_combinator_applies_declaration_to_direct_child_only() {
        // Acceptance: `ol > li` applies to a
        // direct `<li>` child of `<ol>`, but NOT to a grandchild `<li>`
        // reached through an intervening `<ul>` (`<ol><li><ul><li>...`).
        //
        // Uses `background-color`, not `color`: `color` is an inherited
        // property (CSS Cascading L4 §5.2 inheritance) — using it here would
        // let the *direct* `<li>` match's computed value leak onto the
        // grandchild via ordinary inheritance (through the intervening
        // `<ul>`), producing a false pass regardless of whether the child
        // combinator itself correctly rejects the grandchild.
        // `background-color` is not inherited (CSS Backgrounds 3 §2.2), so a
        // red grandchild here can only mean the combinator matched it
        // directly.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "ol > li { background-color: red }");
        let ol = doc.push_element(0, "ol", None);
        let direct_li = doc.push_element(ol, "li", None);
        let ul = doc.push_element(direct_li, "ul", None);
        let grandchild_li = doc.push_element(ul, "li", None);
        // Root-level `<li>` with no `<ol>` ancestor at all (its
        // `ancestor_path` is empty, since the document root itself is not
        // an `Element`) — exercises `match_combinator_chain`'s
        // `Combinator::Child => ancestors.split_last() => None => false`
        // arm, distinct from the "wrong parent" case covered by
        // `grandchild_li` above (there `ancestors.split_last()` succeeds
        // but the resolved parent fails `compound_matches`).
        let orphan_li = doc.push_element(0, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[direct_li].background_color, RED,
            "ol > li must match the direct <li> child of <ol>"
        );
        assert_eq!(
            r.computed[grandchild_li].background_color,
            ComputedValues::initial().background_color,
            "ol > li must NOT match a grandchild <li> reached through an intervening <ul>"
        );
        assert_eq!(
            r.computed[orphan_li].background_color,
            ComputedValues::initial().background_color,
            "ol > li must NOT match an <li> with no ancestor at all"
        );
    }

    #[test]
    fn next_sibling_combinator_does_not_match_parent_child_relationship() {
        // `div + p` requires `div`/`p` to be
        // *siblings* (CSS Selectors L4 adjacent-sibling-combinators,
        // "share the same parent"). Here `p` is instead a *child* of
        // `div` — the ancestor relationship must NOT satisfy the sibling
        // combinator, even though `div` is literally `ancestors.last()`.
        // Directly exercises `match_combinator_chain`'s `NextSibling` arm
        // (this test predates sibling-combinator support, when it was named
        // `match_combinator_chain_rejects_unsupported_combinator_via_safety_net`,
        // and `+` fell through the `_ => false` safety net for a different
        // reason — repurposed now that `+` is supported).
        let list = crate::parse_selector_list("div + p").expect("selector parses");
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", None);
        let p = doc.push_element(div, "p", None);
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        assert_eq!(
            match_complex_selector_list(
                &list,
                &doc,
                &elem,
                StyleNodeId::new(p as u64),
                &[StyleNodeId::new(div as u64)],
                StyleQuirksMode::NoQuirks,
            ),
            None,
            "div + p must not match a p that is div's child, not its sibling"
        );
    }

    #[test]
    fn descendant_and_child_combinator_are_distinguished_on_the_same_grandchild() {
        // Same grandchild `<li>` as above, matched instead by a descendant
        // (space) combinator on `ol` — must match, unlike the child (`>`)
        // combinator case, directly exercising "descendant と child の区別"
        // called out in the acceptance criteria. `background-color` again
        // (see the sibling test above) so a match is provably direct, not
        // inherited from the also-matching direct `<li>`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "ol li { background-color: red }");
        let ol = doc.push_element(0, "ol", None);
        let direct_li = doc.push_element(ol, "li", None);
        let ul = doc.push_element(direct_li, "ul", None);
        let grandchild_li = doc.push_element(ul, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[direct_li].background_color, RED);
        assert_eq!(
            r.computed[grandchild_li].background_color, RED,
            "ol li (descendant combinator) must match the grandchild <li> too"
        );
    }

    #[test]
    fn chained_descendant_and_child_combinator_matches_spec_example() {
        // CSS Selectors L4 child-combinators
        // (<https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim
        // example — see `match_combinator_chain`'s "Spec provenance note"
        // for how this text was confirmed): `div ol>li p` "represents a p
        // element that is a descendant of an li element; the li element
        // must be the child of an ol element; the ol element must be a
        // descendant of a div".
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div ol>li p { color: red }");
        let div = doc.push_element(0, "div", None);
        // `ol` is a descendant of `div`, not a direct child — exercises the
        // "arbitrary descendant" half of the chained selector too.
        let wrapper = doc.push_element(div, "section", None);
        let ol = doc.push_element(wrapper, "ol", None);
        let li = doc.push_element(ol, "li", None);
        // `p` is a descendant of `li`, not a direct child.
        let span = doc.push_element(li, "span", None);
        let p = doc.push_element(span, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].color, RED);
    }

    #[test]
    fn chained_descendant_and_child_combinator_rejects_wrong_child_parent() {
        // Same shape as the spec example above, but `li`'s parent is `ul`
        // instead of `ol` — the `>` (child) constraint must reject this
        // even though every other part of the chain still lines up.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div ol>li p { color: red }");
        let div = doc.push_element(0, "div", None);
        let ul = doc.push_element(div, "ul", None); // not `ol`
        let li = doc.push_element(ul, "li", None);
        let p = doc.push_element(li, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].color, ComputedValues::initial().color);
    }

    #[test]
    fn adjacent_sibling_combinator_applies_only_to_immediately_following_sibling() {
        // Acceptance: `h2 + p` applies to the `<p>`
        // immediately following an `<h2>`, but NOT to a second/third `<p>`
        // further along — CSS Selectors L4 next-sibling combinator
        // (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>
        // §14.3, "match_combinator_chain" doc's verbatim quote). Elements
        // are pushed at the **document root** (parent id `0`, no wrapping
        // `<div>`) deliberately — `ancestor_path` only ever contains
        // Element-kind ids, so a root-level sibling pair exercises
        // `match_combinator_chain`'s `ancestors.last() == None →
        // dom.root_id()` fallback; a wrapping element would hide a bug in
        // that fallback entirely.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red }");
        let _h2 = doc.push_element(0, "h2", None);
        let p1 = doc.push_element(0, "p", None); // immediately follows h2
        let p2 = doc.push_element(0, "p", None); // follows p1, not h2
        let p3 = doc.push_element(0, "p", None); // follows p2, not h2

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p1].background_color, RED,
            "h2 + p must match the p immediately following h2"
        );
        assert_eq!(
            r.computed[p2].background_color,
            ComputedValues::initial().background_color,
            "h2 + p must NOT match the second p (not immediately after h2)"
        );
        assert_eq!(
            r.computed[p3].background_color,
            ComputedValues::initial().background_color,
            "h2 + p must NOT match the third p (not immediately after h2)"
        );
    }

    #[test]
    fn general_sibling_combinator_applies_to_every_following_sibling() {
        // Acceptance: `h2 ~ p` applies to every
        // `<p>` that follows an `<h2>`, not just the immediate one — CSS
        // Selectors L4 general-sibling combinator
        // (<https://www.w3.org/TR/selectors-4/#general-sibling-combinators>
        // §14.4). Same root-level layout as the adjacent-sibling test above
        // (same rationale — exercises the `ancestors.last() == None`
        // fallback).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 ~ p { background-color: red }");
        let _h2 = doc.push_element(0, "h2", None);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let p3 = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p1].background_color, RED,
            "h2 ~ p must match the 1st following p"
        );
        assert_eq!(
            r.computed[p2].background_color, RED,
            "h2 ~ p must match the 2nd following p"
        );
        assert_eq!(
            r.computed[p3].background_color, RED,
            "h2 ~ p must match the 3rd following p"
        );
    }

    #[test]
    fn adjacent_and_general_sibling_combinator_are_distinguished_on_the_same_dom() {
        // Acceptance, literal form: both `h2 + p`
        // and `h2 ~ p` active on the same `<h2><p><p><p>` DOM, using two
        // independent non-inherited properties (`background-color`, CSS
        // Backgrounds 3 §2.2; `box-sizing`, CSS Box Sizing dfn "Inherited:
        // no") so each combinator's reach is independently observable on
        // the same elements.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "h2 + p { background-color: red } h2 ~ p { box-sizing: border-box }",
        );
        let _h2 = doc.push_element(0, "h2", None);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let p3 = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // `+` (background-color): only p1.
        assert_eq!(r.computed[p1].background_color, RED);
        assert_eq!(
            r.computed[p2].background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(
            r.computed[p3].background_color,
            ComputedValues::initial().background_color
        );
        // `~` (box-sizing): all three.
        assert_eq!(
            r.computed[p1].box_sizing,
            crate::property::BoxSizing::BorderBox
        );
        assert_eq!(
            r.computed[p2].box_sizing,
            crate::property::BoxSizing::BorderBox
        );
        assert_eq!(
            r.computed[p3].box_sizing,
            crate::property::BoxSizing::BorderBox
        );
    }

    #[test]
    fn sibling_combinator_ignores_non_element_nodes_between_siblings() {
        // CSS Selectors L4 next-sibling combinator, verbatim: "Non-element
        // nodes (e.g. text between elements) are ignored when considering
        // the adjacency of elements"
        // (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>).
        // A text node is pushed to the *root* (same parent as `h2`/`p`)
        // between them — `TestDoc::push_text(parent, ..)` appends to
        // `parent`'s children list in call order, so pushing it between the
        // `h2` and `p` pushes below makes it a genuine root-level sibling
        // positioned between them, not a descendant of either. `h2 + p`
        // must still match `p` despite this — i.e.
        // `match_combinator_chain`'s `NextSibling` arm
        // (`immediate_preceding_sibling`) must skip the non-Element
        // `child_ids` entry rather than treating the text node as "the"
        // immediately preceding sibling (which would make `p` NOT
        // immediately follow `h2` from an all-nodes perspective).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red }");
        let _h2 = doc.push_element(0, "h2", None);
        doc.push_text(0, "root-level text node, sibling of h2 and p, between them");
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_color, RED,
            "h2 + p must match p despite the text node inside h2 (not a sibling at all) \
             and must not be confused by non-element nodes in general"
        );
    }

    #[test]
    fn general_sibling_combinator_skips_a_leading_non_element_candidate() {
        // Same CSS Selectors L4 "non-element nodes... are ignored" rule as
        // `sibling_combinator_ignores_non_element_nodes_between_siblings`
        // above, but for `~` (general sibling) rather than `+` (adjacent
        // sibling) — these exercise different code paths:
        // `PendingCandidates::LaterSibling`'s new resumable scan loop vs
        // `PendingCandidates::NextSibling`'s single-shot
        // `immediate_preceding_sibling`, for the same kind()-based skip
        // (`TestDoc` always reports `is_in_document() == true`; it has no
        // inert/`<template>`-descendant node concept, so this doesn't
        // separately discriminate `is_in_document_element`'s
        // `is_in_document()` conjunct from its `kind() == Element` one). A
        // root-level text node is pushed *before* `h2` (not between `h2`
        // and `p` — `~`'s candidate scan walks the parent's children
        // forward from the start, so the non-element candidate must be
        // reached *before* the eventual matching candidate to exercise the
        // "skip, keep scanning" branch rather than the "reached
        // `current_id`, stop" one).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 ~ p { background-color: red }");
        doc.push_text(0, "root-level text node, sibling of h2 and p, before both");
        let _h2 = doc.push_element(0, "h2", None);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].background_color, RED,
            "h2 ~ p must match p despite a non-element sibling preceding h2 in \
             the same child list"
        );
    }

    #[test]
    fn later_sibling_choice_point_resumes_live_iterator_across_backtrack() {
        // Discriminator for `PendingCandidates::LaterSibling`'s *live,
        // resumable* `D::ChildIter` - not covered by any existing test:
        // doubling `~`
        // is required to exercise one `LaterSibling` choice point being
        // popped-into-and-resumed by a stack.pop() backtrack from a
        // *different, nested* `LaterSibling` choice point (a single `~`
        // combined with `>`/` ` can't discriminate this, since sibling
        // candidates all share the same ancestors, so a Child/Descendant
        // check after a sibling jump can't distinguish "resume mid-scan"
        // from "rescan from the top").
        //
        // Children of the shared parent, document order: b1(.b), a(.a),
        // b2(.b), t(.t). Selector `.a ~ .b ~ .t` requires some `.b` that
        // precedes `t`, itself preceded by some `.a`.
        //
        // b1 is the *first* `.b` candidate tried for `t`'s `~` frame - but
        // b1's own nested `~` scan for `.a` immediately hits its `stop_at`
        // (b1 is the very first child), so that inner frame is exhausted
        // with zero candidates and pops immediately. Correctness requires
        // the *outer* frame (scanning for `.b` before `t`) to resume its
        // live iterator at `a` next (not restart at b1 - that would loop
        // forever / re-fail identically - and not skip past `a` straight to
        // `b2`, which would only find the wrong, but still spec-correct-
        // looking, match via a different `.b` and hide a real skip bug).
        // The only `.b` with a valid `.a` before it is b2 (via `a`), so a
        // match requires both correct resume *and* correct non-skip.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".a ~ .b ~ .t { background-color: red }");
        let b1 = doc.push_element(0, "div", None);
        doc.set_attr(b1, "class", "b");
        let a = doc.push_element(0, "div", None);
        doc.set_attr(a, "class", "a");
        let b2 = doc.push_element(0, "div", None);
        doc.set_attr(b2, "class", "b");
        let t = doc.push_element(0, "div", None);
        doc.set_attr(t, "class", "t");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[t].background_color, RED,
            ".a ~ .b ~ .t must match t via b2 (preceded by a), after b1's own \
             nested .a search (immediately empty) is backtracked past - this \
             requires the LaterSibling choice point's live child iterator to \
             resume correctly rather than restart or skip"
        );
    }

    #[test]
    fn sibling_combinator_does_not_match_preceding_element() {
        // Order matters: CSS Selectors L4 requires the left compound's
        // element to *precede* the right compound's element. A `<p>` placed
        // BEFORE the `<h2>` must not satisfy `h2 + p` / `h2 ~ p` when
        // matching is attempted from that earlier `<p>`'s perspective.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red } h2 ~ p { color: red }");
        let p_before = doc.push_element(0, "p", None);
        let _h2 = doc.push_element(0, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p_before].background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(r.computed[p_before].color, ComputedValues::initial().color);
    }

    #[test]
    fn sibling_combinator_applies_under_a_non_root_parent() {
        // Same as the acceptance tests above but wrapped in a `<div>`
        // parent, so `ancestors` is non-empty when the sibling combinator
        // arms run — exercises the `ancestors.last() == Some(parent)` branch
        // (as opposed to the root-level tests' `None → root_id()` fallback
        // branch) of `match_combinator_chain`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 + p { background-color: red }");
        let wrap = doc.push_element(0, "div", None);
        let _h2 = doc.push_element(wrap, "h2", None);
        let p = doc.push_element(wrap, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].background_color, RED);
    }

    #[test]
    fn sibling_combinator_composes_with_child_combinator_further_left() {
        // Mixed chain, sibling-then-ancestor direction: `.x > .y ~ .z`.
        // `.z` and `.y` are siblings (share parent `.x`); `.y` must in turn
        // be a direct child of `.x`. Exercises the "sibling jump keeps
        // `ancestors` unchanged, so a further-left Child/Descendant combinator
        // composes for free" path documented on
        // `is_supported_selector_list`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".x > .y ~ .z { background-color: red }");
        let x = doc.push_element(0, "div", None);
        doc.set_attr(x, "class", "x");
        let y = doc.push_element(x, "div", None);
        doc.set_attr(y, "class", "y");
        let z = doc.push_element(x, "div", None); // sibling of y, child of x
        doc.set_attr(z, "class", "z");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[z].background_color, RED);
    }

    #[test]
    fn child_combinator_composes_with_sibling_combinator_further_left() {
        // Mixed chain, ancestor-then-sibling direction: `.x ~ .y > .z`.
        // `.z`'s parent is `.y`; `.y` must in turn have a preceding sibling
        // `.x` (sharing `.y`'s own parent). Exercises the opposite
        // composition from the test above — after the `Child` jump to `.y`,
        // the `ancestors` slice `match_from_element` carries onward is
        // already `.y`'s own ancestor chain, so `.last()` correctly resolves
        // to `.y`'s parent for the `LaterSibling` step (see
        // `match_combinator_chain`'s "親の解決" doc note).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".x ~ .y > .z { background-color: red }");
        let container = doc.push_element(0, "div", None);
        let x = doc.push_element(container, "div", None);
        doc.set_attr(x, "class", "x");
        let y = doc.push_element(container, "div", None); // sibling of x
        doc.set_attr(y, "class", "y");
        let z = doc.push_element(y, "div", None); // child of y
        doc.set_attr(z, "class", "z");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[z].background_color, RED);
    }

    #[test]
    fn descendant_retry_past_a_failed_child_combinator_candidate_is_required() {
        // Regression pinned by 3 independently-converging reviewer lenses
        // (spec/quality/debt) on this branch's first draft, which had a
        // *false* doc-comment claim on `match_combinator_chain` that the
        // `Combinator::Descendant` retry loop is provably redundant for an
        // ancestor-chain-only combinator subset. That's only true when
        // *every* subsequent combinator is also `Descendant` (a free
        // existential search over a strictly-growing superset as you pick
        // a nearer anchor). It breaks the moment a `Combinator::Child` sits
        // further left: `Child` pins one *specific* element
        // (`ancestors.split_last()`), not "any element in the remaining
        // set" — different `Descendant` anchor choices check genuinely
        // different elements, not nested subsets of the same free search.
        // See `match_combinator_chain`'s doc for
        // the full argument this test exists to check.
        //
        // Selector `.x > .y .target` against
        // `G(.x) -> F(.y) -> M(no class) -> C(.y) -> elem(.target)`:
        // the *nearest* `.y` candidate is `C`, but `Child` forces checking
        // `C`'s immediate parent `M` specifically, which lacks `.x` — a
        // confirmed dead end. Only the *farther* `.y` candidate `F` works,
        // because `Child` then forces checking `F`'s immediate parent `G`,
        // which does have `.x`. Without the retry (stopping at `C`'s
        // failure), this selector would silently stop matching.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".x > .y .target { background-color: red }");
        let g = doc.push_element(0, "div", None);
        doc.set_attr(g, "class", "x");
        let f = doc.push_element(g, "div", None);
        doc.set_attr(f, "class", "y");
        let m = doc.push_element(f, "div", None); // no class — the dead-end Child target for C
        let c = doc.push_element(m, "div", None);
        doc.set_attr(c, "class", "y"); // nearest .y candidate, but a dead end via Child
        let target = doc.push_element(c, "div", None);
        doc.set_attr(target, "class", "target");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[target].background_color, RED,
            ".x > .y .target must match via the farther .y candidate (F) after the \
             nearer one (C) fails Child's fixed-parent check — the retry is required"
        );
    }

    fn uniform_div_chain_and_selector(
        depth: usize,
        compounds: usize,
    ) -> (
        TestDoc,
        StyleNodeId,
        Vec<StyleNodeId>,
        SelectorList<RaikiriSelectorImpl>,
    ) {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        let mut ids = Vec::with_capacity(depth);
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
            ids.push(parent);
        }
        let target_id = StyleNodeId::new(*ids.last().expect("depth > 0") as u64);
        let ancestors: Vec<StyleNodeId> = ids[..ids.len() - 1]
            .iter()
            .map(|&id| StyleNodeId::new(id as u64))
            .collect();
        let selector = vec!["div"; compounds].join(" ");
        let list = crate::parse_selector_list(&selector).expect("selector parses");
        (doc, target_id, ancestors, list)
    }

    #[test]
    fn descendant_combinator_deep_unsatisfiable_chain_does_not_explode() {
        let depth = 200;
        let (doc, target_id, ancestors, list) = uniform_div_chain_and_selector(depth, depth + 1);
        let node = doc.node(target_id).unwrap();
        let elem = node.as_element().unwrap();

        let start = std::time::Instant::now();
        let result = match_complex_selector_list(
            &list,
            &doc,
            &elem,
            target_id,
            &ancestors,
            StyleQuirksMode::NoQuirks,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            result,
            None,
            "a {}-compound div-only selector against a {depth}-deep div chain is \
             genuinely unsatisfiable (one compound more than there are ancestor \
             slots) — must resolve to no match, not merely resolve fast",
            depth + 1
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "match_combinator_chain's Descendant backtracking must be memoized \
             to polynomial time — took {elapsed:?} for a {depth}-deep unsatisfiable \
             chain, which the pre-fix exponential backtracking could never \
             realistically finish at all"
        );
    }

    #[test]
    fn later_sibling_combinator_deep_unsatisfiable_run_does_not_explode() {
        let sibling_count = 200;
        let mut doc = TestDoc::new();
        for _ in 0..sibling_count {
            doc.push_element(0, "div", None);
        }
        let target = doc.push_element(0, "div", None);
        let target_id = StyleNodeId::new(target as u64);
        // `target` has `sibling_count` preceding `<div>` siblings and no
        // ancestor element (root-level, same layout the existing sibling
        // acceptance tests above use to exercise the `ancestors.last() ==
        // None -> dom.root_id()` fallback).
        let selector = vec!["div"; sibling_count + 2].join(" ~ ");
        let list = crate::parse_selector_list(&selector).expect("selector parses");
        let node = doc.node(target_id).unwrap();
        let elem = node.as_element().unwrap();

        let start = std::time::Instant::now();
        let result = match_complex_selector_list(
            &list,
            &doc,
            &elem,
            target_id,
            &[],
            StyleQuirksMode::NoQuirks,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            result,
            None,
            "a {}-compound div-only general-sibling selector against {sibling_count} \
             preceding siblings is genuinely unsatisfiable (one compound more than \
             there are preceding-sibling slots) — must resolve to no match",
            sibling_count + 2
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "LaterSibling backtracking must be bounded by the same memo as \
             Descendant — took {elapsed:?} for {sibling_count} unsatisfiable \
             siblings"
        );
    }

    #[test]
    fn match_complex_selector_list_rejects_unsupported_component_via_safety_net() {
        // pseudo-class components never reach `match_complex_selector_list`
        // in the real pipeline — `ruletree.rs`'s `is_supported_selector_list`
        // drops any rule containing one at `add_stylesheet` time (pinned by
        // `ruletree::tests::pseudo_class_selector_still_dropped`). This test
        // calls `match_complex_selector_list` directly — both it and
        // `parse_selector_list` are reachable from this `#[cfg(test)] mod
        // tests` (`use super::*` / `crate::parse_selector_list`) — to
        // exercise `compound_matches`'s `_ => false` safety-net arm
        // defensively, per its own doc comment. `p:hover` has no combinator,
        // so `ancestors`/`elem_id` are irrelevant here — `&[]` / `p`'s own id
        // (this test was renamed alongside the function when `dom`/
        // `ancestors` args were added; the `elem_id` arg was added later,
        // matching the current signature).
        let list = crate::parse_selector_list("p:hover").expect("selector parses");
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", None);
        let node = doc.node(StyleNodeId::new(p as u64)).unwrap();
        let elem = node.as_element().unwrap();
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            match_complex_selector_list(
                &list,
                &doc,
                &elem,
                StyleNodeId::new(p as u64),
                &[],
                StyleQuirksMode::NoQuirks,
            ),
            None,
            "NonTSPseudoClass component must fall through the safety net"
        );
    }

    #[test]
    fn root_pseudo_class_matches_the_document_root_element_only() {
        // `background-color`, not `color`: `color` is inherited (CSS
        // Cascading L4 §5.2) — even a correct implementation that matched
        // `:root` on `<html>` alone would show a red `body.color` through
        // ordinary inheritance, making that a false-negative test for the
        // "must not also match a descendant" half (same pitfall
        // `child_combinator_applies_declaration_to_direct_child_only`'s own
        // comment documents). `background-color` is not inherited (CSS
        // Backgrounds 3 §2.2), so a red `body` here can only mean `:root`
        // itself wrongly matched it.
        //
        // `RuleTree::empty()` + `add_stylesheet`, not the usual
        // `push_element(0, "style", None)` + `build_rule_tree` convention
        // (quality-lens finding): that convention parks `<style>` itself as
        // a direct child of the Document node — i.e. an element sibling of
        // `<html>` that *also* has `parent_id.is_none()` and would *also*
        // match `:root`. Since this test never asserted anything about
        // `<style>`'s own computed value, that convention only proved "an
        // element with no element parent matches" (true of `html` here by
        // coincidence of push order), not "`:root` matches the root
        // element and no other top-level node" — the actual claim this
        // test's name makes. `RuleTree::empty()` avoids adding any such
        // ambiguous second candidate.
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", None);
        let body = doc.push_element(html, "body", None);

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(":root { background-color: red }", Origin::Author);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[html].background_color, RED,
            ":root must match the root element"
        );
        assert_eq!(
            r.computed[body].background_color,
            ComputedValues::initial().background_color,
            ":root must not match a non-root descendant"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_element_with_no_children() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match a childless element"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_element_with_zero_length_text_child() {
        // Spec text (`matches_empty` doc, verbatim): "...content nodes...
        // whose data has a non-zero length must be considered as affecting
        // emptiness" — a zero-length text node (`data.len() == 0`) does
        // NOT meet "non-zero length" and so must not disqualify `:empty`,
        // regardless of the L3/L4 whitespace-handling difference (quality
        // lens finding: this branch of `matches_empty`'s `Text` arm was
        // previously untested).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, "");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match an element with a zero-length text child"
        );
    }

    #[test]
    fn empty_pseudo_class_does_not_match_element_with_an_element_child() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_element(p, "span", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            ":empty must not match an element with an element child"
        );
    }

    #[test]
    fn empty_pseudo_class_does_not_match_element_with_a_text_child() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, "hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            ":empty must not match an element with a non-empty text child"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_whitespace_only_text_child() {
        // Acceptance-pinning test for the correction documented in
        // `matches_empty` doc's "Spec provenance and correction" note:
        // CSS Selectors L4 *deliberately changed* `:empty` from L3 so that
        // whitespace-only content — "given white space is largely
        // collapsible in HTML and is therefore used for source code
        // formatting" (L4 changelog note, verbatim) — no longer
        // disqualifies. The L4 spec's own worked example lists `<p> </p>`
        // among what `p:empty` matches, verbatim. This test used to assert
        // the opposite (the pre-correction L3-only reading); inverted, not
        // just renamed, when the bug was fixed.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, " ");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match an element with a document-white-space-only text child (CSS Selectors L4)"
        );
    }

    #[test]
    fn empty_pseudo_class_does_not_match_nbsp_only_text_child() {
        // No-break space (U+00A0) is explicitly NOT a "document white
        // space character" (CSS Text 4, `is_document_white_space` doc) —
        // the L4 spec's own worked example lists `<div>&nbsp;</div>`
        // among what `div:empty` does *not* match, verbatim. Distinguishes
        // this from the (now-passing) plain-space case above: `:empty`'s
        // L4 whitespace carve-out is narrower than "any Unicode
        // whitespace".
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_text(p, "\u{00A0}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            ":empty must not match an element with an NBSP-only text child"
        );
    }

    #[test]
    fn empty_pseudo_class_matches_element_with_only_a_comment_child() {
        // "comments... must not affect whether an element is considered
        // empty" (CSS Selectors L4 §13.2 `#the-empty-pseudo`, verbatim,
        // unchanged from L3) — a comment-only element still matches
        // `:empty`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:empty { color: red }");
        let wrap = doc.push_element(0, "div", None);
        let p = doc.push_element(wrap, "p", None);
        doc.push_comment(p, " note ");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].color, RED,
            ":empty must match an element whose only child is a comment"
        );
    }

    #[test]
    fn first_child_last_child_only_child_ignore_text_node_siblings() {
        // CSS Selectors L3 §6.6 preamble (verbatim, `sibling_position`
        // doc): "Standalone text and other non-element nodes are not
        // counted when calculating the position of an element in its list
        // of siblings" — a text node between two <li> must not shift
        // indices.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "li:first-child { color: red } \
             li:last-child { background-color: red }",
        );
        let ul = doc.push_element(0, "ul", None);
        let first = doc.push_element(ul, "li", None);
        doc.push_text(ul, "\n  "); // whitespace between <li> siblings
        let last = doc.push_element(ul, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[first].color, RED,
            ":first-child must match despite an intervening text node"
        );
        assert_eq!(
            r.computed[last].background_color, RED,
            ":last-child must match despite an intervening text node"
        );
        assert_eq!(
            r.computed[first].background_color,
            ComputedValues::initial().background_color,
            "the first <li> must not also match :last-child"
        );
    }

    #[test]
    fn only_child_matches_the_sole_element_child_and_nothing_else() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "li:only-child { color: red }");
        let solo_wrap = doc.push_element(0, "ul", None);
        let solo = doc.push_element(solo_wrap, "li", None);
        let pair_wrap = doc.push_element(0, "ul", None);
        let pair_a = doc.push_element(pair_wrap, "li", None);
        let pair_b = doc.push_element(pair_wrap, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[solo].color, RED,
            ":only-child must match a sole <li>"
        );
        assert_eq!(
            r.computed[pair_a].color,
            ComputedValues::initial().color,
            ":only-child must not match when a sibling <li> exists"
        );
        assert_eq!(
            r.computed[pair_b].color,
            ComputedValues::initial().color,
            ":only-child must not match when a sibling <li> exists"
        );
    }

    #[test]
    fn nth_child_zebra_striping_acceptance() {
        // Acceptance: `:nth-child(2n+1)` zebra
        // striping. Rows 1/3/5 (1-based) get the declaration, 2/4 don't.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "tr:nth-child(2n+1) { background-color: red }");
        let table = doc.push_element(0, "table", None);
        let rows: Vec<usize> = (0..5)
            .map(|_| doc.push_element(table, "tr", None))
            .collect();

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        for (i, &row) in rows.iter().enumerate() {
            let one_based = i + 1;
            let expect_red = one_based % 2 == 1;
            assert_eq!(
                r.computed[row].background_color,
                if expect_red {
                    RED
                } else {
                    ComputedValues::initial().background_color
                },
                "row {one_based} (0-based index {i}): nth-child(2n+1) zebra stripe mismatch"
            );
        }
    }

    #[test]
    fn nth_child_negative_b_and_explicit_index_forms() {
        // `:nth-child(3)` (a=0) and `:nth-child(-n+2)` (first two only) —
        // exercises `AnPlusB` beyond the simple odd/even case.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "li:nth-child(3) { color: red } li:nth-child(-n+2) { background-color: red }",
        );
        let ul = doc.push_element(0, "ul", None);
        let items: Vec<usize> = (0..4).map(|_| doc.push_element(ul, "li", None)).collect();

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[items[0]].background_color, RED,
            "index 1 in -n+2"
        );
        assert_eq!(
            r.computed[items[1]].background_color, RED,
            "index 2 in -n+2"
        );
        assert_eq!(
            r.computed[items[2]].background_color,
            ComputedValues::initial().background_color,
            "index 3 not in -n+2"
        );
        assert_eq!(
            r.computed[items[2]].color, RED,
            "index 3 matches nth-child(3)"
        );
        assert_eq!(
            r.computed[items[0]].color,
            ComputedValues::initial().color,
            "index 1 does not match nth-child(3)"
        );
    }

    #[test]
    fn nth_last_child_counts_from_the_end() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "li:nth-last-child(1) { color: red }");
        let ul = doc.push_element(0, "ul", None);
        let items: Vec<usize> = (0..3).map(|_| doc.push_element(ul, "li", None)).collect();

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[items[2]].color, RED,
            ":nth-last-child(1) must match the last element sibling"
        );
        assert_eq!(
            r.computed[items[0]].color,
            ComputedValues::initial().color,
            ":nth-last-child(1) must not match the first element sibling"
        );
    }

    #[test]
    fn first_of_type_last_of_type_only_of_type_are_restricted_to_matching_tag() {
        // Mixed-tag sibling list: <h2><p><p><h2> — the -of-type family must
        // count only same-tag siblings (CSS Selectors L3 §6.6, "an+b-1
        // siblings with the same expanded element name"), unlike plain
        // :first-child/:last-child/:only-child.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "h2:first-of-type { color: red } \
             h2:last-of-type { background-color: red } \
             p:only-of-type { border-top-style: solid }",
        );
        let section = doc.push_element(0, "section", None);
        let h2_first = doc.push_element(section, "h2", None);
        let p = doc.push_element(section, "p", None);
        let h2_last = doc.push_element(section, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h2_first].color, RED,
            "h2:first-of-type must match the first <h2> even though a <p> is its actual first-child"
        );
        assert_eq!(
            r.computed[h2_last].background_color, RED,
            "h2:last-of-type must match the second <h2>"
        );
        assert_eq!(
            r.computed[h2_first].background_color,
            ComputedValues::initial().background_color,
            "the first <h2> must not also match :last-of-type"
        );
        assert_eq!(
            r.computed[p].border.top.style,
            BorderStyle::Solid,
            "p:only-of-type must match the sole <p> despite <h2> siblings"
        );
    }

    #[test]
    fn nth_of_type_counts_only_same_tag_siblings() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p:nth-of-type(2) { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.push_element(div, "h2", None);
        let p1 = doc.push_element(div, "p", None);
        doc.push_element(div, "h2", None);
        let p2 = doc.push_element(div, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p2].color, RED,
            "p:nth-of-type(2) must match the 2nd <p>, ignoring interleaved <h2> siblings"
        );
        assert_eq!(
            r.computed[p1].color,
            ComputedValues::initial().color,
            "p:nth-of-type(2) must not match the 1st <p>"
        );
    }

    #[test]
    fn root_element_matches_first_child_last_child_only_child_and_nth_child_1() {
        // The root element has no *element* parent, but per CSS Selectors
        // L3's "an+b-1 siblings before/after it" framing (`matches_nth`
        // doc) it still has a (trivial, size-1) sibling list — itself
        // alone under the Document node.
        //
        // Deliberately does NOT use the usual `push_element(0, "style",
        // None)` + `build_rule_tree` convention: that convention parks the
        // `<style>` element itself as a direct child of the Document node
        // (id 0) — i.e. as an *element sibling of the root element being
        // tested here*, which would make `<style>` the real first element
        // child and `<html>` the second, defeating the point of this test.
        // `RuleTree::empty()` + `add_stylesheet` supplies the CSS without
        // adding any DOM node at all.
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", None);

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "html:first-child { color: red } \
             html:last-child { background-color: red } \
             html:only-child { border-top-style: solid } \
             html:nth-child(1) { border-bottom-style: solid }",
            Origin::Author,
        );
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[html].color, RED,
            "root element must match :first-child"
        );
        assert_eq!(
            r.computed[html].background_color, RED,
            "root element must match :last-child"
        );
        assert_eq!(
            r.computed[html].border.top.style,
            BorderStyle::Solid,
            "root element must match :only-child"
        );
        assert_eq!(
            r.computed[html].border.bottom.style,
            BorderStyle::Solid,
            "root element must match :nth-child(1)"
        );
    }

    #[test]
    fn root_element_does_not_match_nth_child_2() {
        // Negative half of the previous test (WPT reference:
        // `css/selectors/child-indexed-no-parent.html`, per CSS Selectors
        // L3's "an+b-1 siblings before it" framing this crate follows):
        // the root element's sibling list under `dom.root_id()` has size
        // 1 (itself alone), so no `:nth-child(N)`/`:nth-last-child(N)` for
        // `N >= 2` can ever match it. `:root:nth-last-child(2)` is the
        // canonical form of this check.
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", None);

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(":root:nth-last-child(2) { color: red }", Origin::Author);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[html].color,
            ComputedValues::initial().color,
            ":root:nth-last-child(2) must not match — the root element has no siblings at all"
        );
    }

    #[test]
    fn section_gt_p_first_child_acceptance() {
        // Acceptance: `.section > p:first-child { font-weight: bold }`.
        // Combines the child combinator with a
        // structural pseudo-class on the *rightmost* compound.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".section > p:first-child { font-weight: bold }");
        let section = doc.push_element(0, "div", None);
        doc.set_attr(section, "class", "section");
        let first_p = doc.push_element(section, "p", None);
        let second_p = doc.push_element(section, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[first_p].font_weight, 700.0,
            ".section > p:first-child must match the first <p>"
        );
        assert_eq!(
            r.computed[second_p].font_weight,
            ComputedValues::initial().font_weight,
            ".section > p:first-child must not match the second <p>"
        );
    }

    #[test]
    fn is_pseudo_class_matches_any_inner_selector() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "div:is(.featured, [data-kind=selected]) { font-weight: bold }",
        );

        let by_class = doc.push_element(0, "div", None);
        doc.set_attr(by_class, "class", "featured");
        let by_attribute = doc.push_element(0, "div", None);
        doc.set_attr(by_attribute, "data-kind", "selected");
        let no_match = doc.push_element(0, "div", None);

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[by_class].font_weight, 700.0);
        assert_eq!(result.computed[by_attribute].font_weight, 700.0);
        assert_eq!(result.computed[no_match].font_weight, 400.0);
    }

    #[test]
    fn is_and_where_use_selectors_specificity_rules() {
        let mut is_doc = TestDoc::new();
        let is_style = is_doc.push_element(0, "style", None);
        is_doc.push_text(
            is_style,
            ".featured { font-weight: bold } \
             div:is(#unused, .featured) { font-weight: 300 }",
        );
        let is_element = is_doc.push_element(0, "div", None);
        is_doc.set_attr(is_element, "class", "featured");
        let is_tree = build_rule_tree(&is_doc);
        let is_result = cascade(&is_doc, &is_tree).expect("cascade Ok");
        assert_eq!(is_result.computed[is_element].font_weight, 300.0);

        let mut where_doc = TestDoc::new();
        let where_style = where_doc.push_element(0, "style", None);
        where_doc.push_text(
            where_style,
            ".featured { font-weight: bold } \
             div:where(#unused, .featured) { font-weight: 300 }",
        );
        let where_element = where_doc.push_element(0, "div", None);
        where_doc.set_attr(where_element, "class", "featured");
        let where_tree = build_rule_tree(&where_doc);
        let where_result = cascade(&where_doc, &where_tree).expect("cascade Ok");
        assert_eq!(where_result.computed[where_element].font_weight, 700.0);
    }

    #[test]
    fn forgiving_logical_selector_ignores_invalid_branches() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "div:is(.featured, :unknown-pseudo) { font-weight: 300 } \
             div:where(.selected, 123) { font-weight: 500 }",
        );
        let is_element = doc.push_element(0, "div", None);
        doc.set_attr(is_element, "class", "featured");
        let where_element = doc.push_element(0, "div", None);
        doc.set_attr(where_element, "class", "selected");

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[is_element].font_weight, 300.0);
        assert_eq!(result.computed[where_element].font_weight, 500.0);
    }

    #[test]
    fn has_matches_descendants_and_relative_siblings() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "section:has(> .direct:is(.featured, 123)) { font-weight: 700 } \
             article:has(.deep) { font-weight: 600 } \
             div:has(+ p.adjacent) { font-weight: 500 } \
             div:has(~ p.later) { font-weight: 300 }",
        );

        let direct_section = doc.push_element(0, "section", None);
        let direct_child = doc.push_element(direct_section, "div", None);
        doc.set_attr(direct_child, "class", "direct featured");
        let indirect_section = doc.push_element(0, "section", None);
        let wrapper = doc.push_element(indirect_section, "div", None);
        let indirect_child = doc.push_element(wrapper, "div", None);
        doc.set_attr(indirect_child, "class", "direct featured");

        let deep_article = doc.push_element(0, "article", None);
        let deep_wrapper = doc.push_element(deep_article, "div", None);
        doc.push_element(deep_wrapper, "span", None);
        let deep_target = doc.push_element(deep_wrapper, "span", None);
        doc.set_attr(deep_target, "class", "deep");
        let empty_article = doc.push_element(0, "article", None);

        let adjacent_parent = doc.push_element(0, "main", None);
        let adjacent_anchor = doc.push_element(adjacent_parent, "div", None);
        let adjacent = doc.push_element(adjacent_parent, "p", None);
        doc.set_attr(adjacent, "class", "adjacent");
        let no_adjacent_parent = doc.push_element(0, "main", None);
        let no_adjacent = doc.push_element(no_adjacent_parent, "div", None);
        doc.push_element(no_adjacent_parent, "span", None);

        let later_parent = doc.push_element(0, "main", None);
        let later_anchor = doc.push_element(later_parent, "div", None);
        doc.push_element(later_parent, "span", None);
        let later = doc.push_element(later_parent, "p", None);
        doc.set_attr(later, "class", "later");
        let no_later_parent = doc.push_element(0, "main", None);
        let no_later = doc.push_element(no_later_parent, "div", None);
        doc.push_element(no_later_parent, "span", None);

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[direct_section].font_weight, 700.0);
        assert_eq!(result.computed[indirect_section].font_weight, 400.0);
        assert_eq!(result.computed[deep_article].font_weight, 600.0);
        assert_eq!(result.computed[empty_article].font_weight, 400.0);
        assert_eq!(result.computed[adjacent_anchor].font_weight, 500.0);
        assert_eq!(result.computed[no_adjacent].font_weight, 400.0);
        assert_eq!(result.computed[later_anchor].font_weight, 300.0);
        assert_eq!(result.computed[no_later].font_weight, 400.0);
    }

    #[test]
    fn has_matches_mixed_relative_combinator_chains() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "#child-desc:has(> .first .target) { font-weight: 701 } \
             #adj-desc:has(+ .first .target) { font-weight: 702 } \
             #child-sib:has(> .first + .target) { font-weight: 703 } \
             #adj-sib:has(+ .first + .target) { font-weight: 704 } \
             #general-child:has(~ .first > .target) { font-weight: 705 } \
             #general-sib:has(~ .first + .target) { font-weight: 706 }",
        );

        let child_desc = doc.push_element(0, "section", None);
        doc.set_attr(child_desc, "id", "child-desc");
        let first = doc.push_element(child_desc, "div", None);
        doc.set_attr(first, "class", "first");
        let target = doc.push_element(first, "span", None);
        doc.set_attr(target, "class", "target");

        let adj_desc_parent = doc.push_element(0, "main", None);
        let adj_desc = doc.push_element(adj_desc_parent, "section", None);
        doc.set_attr(adj_desc, "id", "adj-desc");
        doc.push_text(adj_desc_parent, "ignored text");
        let first = doc.push_element(adj_desc_parent, "div", None);
        doc.set_attr(first, "class", "first");
        let target = doc.push_element(first, "span", None);
        doc.set_attr(target, "class", "target");

        let child_sib = doc.push_element(0, "section", None);
        doc.set_attr(child_sib, "id", "child-sib");
        let first = doc.push_element(child_sib, "div", None);
        doc.set_attr(first, "class", "first");
        doc.push_comment(child_sib, "ignored comment");
        let target = doc.push_element(child_sib, "span", None);
        doc.set_attr(target, "class", "target");

        let adj_sib_parent = doc.push_element(0, "main", None);
        let adj_sib = doc.push_element(adj_sib_parent, "section", None);
        doc.set_attr(adj_sib, "id", "adj-sib");
        let first = doc.push_element(adj_sib_parent, "div", None);
        doc.set_attr(first, "class", "first");
        let target = doc.push_element(adj_sib_parent, "span", None);
        doc.set_attr(target, "class", "target");

        let general_child_parent = doc.push_element(0, "main", None);
        let general_child = doc.push_element(general_child_parent, "section", None);
        doc.set_attr(general_child, "id", "general-child");
        doc.push_element(general_child_parent, "i", None);
        let first = doc.push_element(general_child_parent, "div", None);
        doc.set_attr(first, "class", "first");
        let target = doc.push_element(first, "span", None);
        doc.set_attr(target, "class", "target");

        let general_sib_parent = doc.push_element(0, "main", None);
        let general_sib = doc.push_element(general_sib_parent, "section", None);
        doc.set_attr(general_sib, "id", "general-sib");
        doc.push_element(general_sib_parent, "i", None);
        let first = doc.push_element(general_sib_parent, "div", None);
        doc.set_attr(first, "class", "first");
        let target = doc.push_element(general_sib_parent, "span", None);
        doc.set_attr(target, "class", "target");

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        for (id, expected) in [
            (child_desc, 701.0),
            (adj_desc, 702.0),
            (child_sib, 703.0),
            (adj_sib, 704.0),
            (general_child, 705.0),
            (general_sib, 706.0),
        ] {
            assert_eq!(result.computed[id].font_weight, expected);
        }
    }

    #[test]
    fn has_deep_branched_miss_uses_bounded_ancestor_storage() {
        const DEPTH: usize = 2_048;
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "section:has(.missing) { font-weight: 700 } section { color: red }",
        );

        let anchor = doc.push_element(0, "section", None);
        let mut current = anchor;
        for _ in 0..DEPTH {
            // The deep child keeps the walk going while the sibling remains
            // pending on the explicit stack at every level.
            let next = doc.push_element(current, "div", None);
            doc.push_element(current, "span", None);
            current = next;
        }

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[anchor].color, RED);
        assert_eq!(
            result.computed[anchor].font_weight,
            ComputedValues::initial().font_weight,
        );
    }

    #[test]
    fn nested_has_in_forgiving_branches_is_ignored() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "#nested:has(:is(:has(*), .valid)) { font-weight: 701 } \
             #fallback:is(:has(:has(*)), .fallback) { font-weight: 702 }",
        );

        let nested = doc.push_element(0, "div", None);
        doc.set_attr(nested, "id", "nested");
        let valid = doc.push_element(nested, "span", None);
        doc.set_attr(valid, "class", "valid");

        let fallback = doc.push_element(0, "div", None);
        doc.set_attr(fallback, "id", "fallback");
        doc.set_attr(fallback, "class", "fallback");

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[nested].font_weight, 701.0);
        assert_eq!(result.computed[fallback].font_weight, 702.0);
    }

    #[test]
    fn logical_selectors_compose_with_has_and_not() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "section:is(:has(> .item), .fallback) { font-weight: 700 } \
             section:not(:has(> .missing)) { color: red }",
        );

        let has_section = doc.push_element(0, "section", None);
        let item = doc.push_element(has_section, "div", None);
        doc.set_attr(item, "class", "item");
        let fallback_section = doc.push_element(0, "section", None);
        doc.set_attr(fallback_section, "class", "fallback");
        let missing_section = doc.push_element(0, "section", None);
        let missing = doc.push_element(missing_section, "div", None);
        doc.set_attr(missing, "class", "missing");

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[has_section].font_weight, 700.0);
        assert_eq!(result.computed[fallback_section].font_weight, 700.0);
        assert_eq!(result.computed[missing_section].font_weight, 400.0);
        assert_eq!(result.computed[has_section].color, RED);
        assert_eq!(result.computed[fallback_section].color, RED);
        assert_eq!(
            result.computed[missing_section].color,
            ComputedValues::initial().color
        );
    }

    #[test]
    fn negation_matches_when_inner_selector_does_not_match() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "section > div:not(:first-child) { font-weight: bold }",
        );
        let section = doc.push_element(0, "section", None);
        let first = doc.push_element(section, "div", None);
        let second = doc.push_element(section, "div", None);

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.computed[first].font_weight,
            ComputedValues::initial().font_weight
        );
        assert_eq!(result.computed[second].font_weight, 700.0);
    }

    #[test]
    fn structural_pseudo_class_on_an_ancestor_compound_uses_that_ancestors_own_parent() {
        // `body > div:only-child p` — the structural pseudo-class sits on
        // the *ancestor* compound (`div:only-child`), reached by crossing
        // the child combinator via `match_from_ancestor`, not on the
        // rightmost compound. This is the one test that would catch a
        // wrong `parent_id` slice at that recursion site (using `elem`'s
        // parent instead of the ancestor-being-matched's own parent).
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "body > div:only-child p { color: red }");
        let body = doc.push_element(0, "body", None);
        let solo_div = doc.push_element(body, "div", None); // only element child of <body>
        let p_under_solo = doc.push_element(solo_div, "p", None);

        let other_body = doc.push_element(0, "body", None);
        let div_a = doc.push_element(other_body, "div", None);
        doc.push_element(other_body, "div", None); // makes div_a NOT an only-child
        let p_under_div_a = doc.push_element(div_a, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p_under_solo].color, RED,
            "body > div:only-child p must match when the <div> really is body's only child"
        );
        assert_eq!(
            r.computed[p_under_div_a].color,
            ComputedValues::initial().color,
            "body > div:only-child p must not match when the <div> has a sibling <div>"
        );
    }

    #[test]
    fn nth_child_of_extended_syntax_is_accepted_as_selector_list_argument() {
        let list = crate::parse_selector_list("p:nth-child(2n+1 of .foo)")
            .expect("the `of S` selector-list syntax must parse");
        assert_eq!(list.slice().len(), 1);
    }

    #[test]
    fn nth_child_of_selector_list_filters_siblings_for_both_directions() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "li:nth-child(2 of .featured, [data-kind=selected]) { color: red } \
             li:nth-last-child(2 of .featured, [data-kind=selected]) { background-color: red }",
        );
        let ul = doc.push_element(0, "ul", None);

        let first_featured = doc.push_element(ul, "li", None);
        doc.set_attr(first_featured, "class", "featured");

        let unfiltered_before_second = doc.push_element(ul, "li", None);

        let second_filtered = doc.push_element(ul, "li", None);
        doc.set_attr(second_filtered, "data-kind", "selected");

        let third_filtered_non_li = doc.push_element(ul, "div", None);
        doc.set_attr(third_filtered_non_li, "class", "featured");

        let unfiltered = doc.push_element(ul, "li", None);

        let second_from_end = doc.push_element(ul, "li", None);
        doc.set_attr(second_from_end, "class", "featured");

        let unfiltered_before_last = doc.push_element(ul, "li", None);

        let last_filtered = doc.push_element(ul, "li", None);
        doc.set_attr(last_filtered, "data-kind", "selected");

        let tree = build_rule_tree(&doc);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            tree.style_rules.len(),
            2,
            "flat selector-list filters must remain captured"
        );
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(r.computed[second_filtered].color, RED);
        assert_eq!(r.computed[second_from_end].background_color, RED);
        assert_eq!(
            r.computed[first_featured].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[third_filtered_non_li].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[unfiltered].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[unfiltered_before_second].color,
            ComputedValues::initial().color
        );
        assert_eq!(
            r.computed[unfiltered_before_last].background_color,
            ComputedValues::initial().background_color
        );
        assert_eq!(
            r.computed[last_filtered].background_color,
            ComputedValues::initial().background_color
        );
    }

    #[test]
    fn nested_nth_child_filter_is_rejected_before_sibling_scan() {
        // Selectors L4 permits a complex-real-selector-list in `of S`, but this
        // implementation rejects nested structural nth components before the
        // outer filter can scan the sibling list. Keep enough siblings here to
        // make accidentally accepting the nested filter observable.
        let selector = "li:nth-child(1 of :nth-child(1))";
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            crate::parse_selector_list(selector).is_ok(),
            "the nested nth-child selector must parse before support filtering"
        );

        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "li:nth-child(1 of :nth-child(1)) { background-color: red }",
        );
        let list = doc.push_element(0, "ul", None);
        let sibling_count = 256;
        let items: Vec<_> = (0..sibling_count)
            .map(|_| doc.push_element(list, "li", None))
            .collect();

        let tree = build_rule_tree(&doc);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            tree.style_rules.is_empty(),
            "nested nth-child filters must be dropped before sibling scans"
        );
        let result = cascade(&doc, &tree).expect("cascade Ok");
        for item in items {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                result.computed[item].background_color,
                ComputedValues::initial().background_color,
                "nested nth-child filter must not style any of {sibling_count} siblings"
            );
        }
    }

    #[test]
    fn nested_nth_last_child_filter_is_rejected_before_sibling_scan() {
        // The from-end form must share the same bounded support boundary as
        // the from-start form; otherwise it could retain a second expensive
        // nested sibling scan path.
        let selector = "li:nth-last-child(1 of :nth-last-child(1))";
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            crate::parse_selector_list(selector).is_ok(),
            "the nested nth-last-child selector must parse before support filtering"
        );

        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(
            style,
            "li:nth-last-child(1 of :nth-last-child(1)) { background-color: red }",
        );
        let list = doc.push_element(0, "ul", None);
        let sibling_count = 256;
        let items: Vec<_> = (0..sibling_count)
            .map(|_| doc.push_element(list, "li", None))
            .collect();

        let tree = build_rule_tree(&doc);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            tree.style_rules.is_empty(),
            "nested nth-last-child filters must be dropped before sibling scans"
        );
        let result = cascade(&doc, &tree).expect("cascade Ok");
        for item in items {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                result.computed[item].background_color,
                ComputedValues::initial().background_color,
                "nested nth-last-child filter must not style any of {sibling_count} siblings"
            );
        }
    }

    #[test]
    fn nth_child_of_ignores_inert_element_siblings() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "li:nth-child(2 of .featured) { color: red }");
        let ul = doc.push_element(0, "ul", None);

        let first_featured = doc.push_element(ul, "li", None);
        doc.set_attr(first_featured, "class", "featured");

        let inert_featured = doc.push_element(ul, "li", None);
        doc.set_attr(inert_featured, "class", "featured");
        doc.set_in_document(inert_featured, false);

        let second_featured = doc.push_element(ul, "li", None);
        doc.set_attr(second_featured, "class", "featured");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(r.computed[second_featured].color, RED);
        assert_eq!(
            r.computed[inert_featured].color,
            ComputedValues::initial().color
        );
    }

    #[test]
    fn resolve_case_sensitivity_html_default_namespace_folds_case_for_html_case_insensitive_attr() {
        // `type` is on HTML's ASCII-case-insensitive attribute list (the
        // selectors crate's generated `ascii_case_insensitive_html_attributes`
        // set) — with no explicit `i`/`s` flag, `[type=...]` parses to
        // `ParsedCaseSensitivity::AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument`,
        // which `resolve_case_sensitivity` must fold to ASCII-case-insensitive
        // for an element in the default (HTML) namespace — `TestElementRef`
        // returns `None` from `namespace_uri()` unless overridden via
        // `TestDoc::set_namespace`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[type=\"text\"] { color: red }");
        let input = doc.push_element(0, "input", None);
        doc.set_attr(input, "type", "TEXT"); // different case than the selector

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[input].color, RED,
            "[type=...] must ASCII-case-fold under the HTML default"
        );
    }

    #[test]
    fn resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive() {
        // Same `[type=...]` shape as the sibling test above, but the element
        // carries an explicit non-HTML namespace (SVG) — `resolve_case_sensitivity`
        // must fall back to case-sensitive matching for it (own doc comment:
        // "SVG 等 non-HTML namespace の element は case-sensitive 側に倒す").
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "[type=\"text\"] { color: red }");
        let input = doc.push_element(0, "input", None);
        doc.set_attr(input, "type", "TEXT");
        doc.set_namespace(input, "http://www.w3.org/2000/svg");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[input].color,
            ComputedValues::initial().color,
            "non-HTML-namespace element must not case-fold [type=...]"
        );
    }

    fn deep_child_combinator_chain_doc(depth: usize) -> (TestDoc, usize) {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
        }
        let style = doc.push_element(parent, "style", None);
        let selector = vec!["div"; depth].join(" > ");
        doc.push_text(style, &format!("{selector} {{ color: red }}"));
        (doc, parent)
    }

    #[test]
    fn deep_child_combinator_chain_small_stack_no_overflow() {
        let (doc, deepest) = deep_child_combinator_chain_doc(500);
        let tree = build_rule_tree(&doc);
        let color = std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .stack_size(128 * 1024)
                .spawn_scoped(scope, || {
                    let result = cascade(&doc, &tree).expect("cascade Ok");
                    result.computed[deepest].color
                })
                .expect("spawn thread");
            handle.join().expect(
                "thread must not stack-overflow matching a long, successively-matching \
                 child-combinator chain",
            )
        });
        assert_eq!(color, RED);
    }

    #[test]
    fn no_pseudo_element_rule_leaves_pseudo_map_empty_for_that_element() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red }");
        doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert!(r.pseudo.is_empty(), "no ::before/::after rule anywhere");
    }

    #[test]
    fn bare_before_pseudo_element_matches_every_element_like_universal() {
        // Bare `::before` (no preceding type/class) parses as an implicit
        // universal originating-element selector — `*::before`. Both `<p>`
        // and `<span>` must get an entry.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"::before { content: "x" }"#);
        let p = doc.push_element(0, "p", None);
        let span = doc.push_element(0, "span", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert!(
            r.pseudo
                .contains_key(&(StyleNodeId(p as u64), PseudoElem::Before))
        );
        assert!(
            r.pseudo
                .contains_key(&(StyleNodeId(span as u64), PseudoElem::Before))
        );
    }

    #[test]
    fn pseudo_element_originating_selector_can_use_a_combinator_chain() {
        // Every other pseudo-element test above uses a single-compound
        // originating-element selector (`.foo::before`, `p::before`, bare
        // `::before`) — this one exercises `selector_matches_pseudo_element`
        // when the part of the selector *before* the pseudo-element itself
        // spans a combinator (`div p::before`, a descendant combinator),
        // which routes through `match_combinator_chain` exactly like an
        // ordinary (non-pseudo) selector's own combinator chain does.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"div p::before { content: "nested" }"#);
        let div = doc.push_element(0, "div", None);
        let p_inside = doc.push_element(div, "p", None);
        let p_outside = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            r.pseudo
                .contains_key(&(StyleNodeId(p_inside as u64), PseudoElem::Before)),
            "`div p::before` must match the `<p>` that is a descendant of `<div>`"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !r.pseudo
                .contains_key(&(StyleNodeId(p_outside as u64), PseudoElem::Before)),
            "a `<p>` outside any `<div>` must not match `div p::before`"
        );
    }

    #[test]
    fn selector_list_can_mix_real_element_and_pseudo_element_targets() {
        // `p, p::before { content: "x" }` — one selector targets the real
        // `<p>`, the other targets its `::before`. Both must apply
        // independently from the same rule.
        use crate::property::ContentComponent;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p, p::before { content: "x" }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            *r.computed[p].content,
            vec![ContentComponent::Literal("x".into())],
            "the `p` branch of the selector list must still apply to the \
             real element"
        );
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(*before.content, vec![ContentComponent::Literal("x".into())]);
    }

    #[test]
    fn pseudo_element_selector_never_matches_real_element_directly() {
        // Safety-net regression: `.foo::before` alone must not also apply
        // its declarations to the real `.foo` element (only to its
        // `::before`) — pins the `compound_matches` `_ => false` interaction
        // `selector_matches_pseudo_element`'s doc describes.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#".foo::before { color: blue }"#);
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].color,
            ComputedValues::initial().color,
            "must stay initial — the ::before rule must not leak onto the \
             real element"
        );
    }
}
