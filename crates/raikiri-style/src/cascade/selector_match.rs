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
///   — [`language_range_matches`] /
///   [`resolve_directionality`] 経由、`dom` + `ancestors` (自身の祖先 chain)
///   を使って ancestor-inherited な effective language / directionality を
///   解決する。`PseudoClass::Hover` / `PseudoClass::Active` はこの arm 内で
///   引き続き `false` (dynamic pseudo-class の対応は本実装の scope 外のまま)。
/// - `Component::Root` (`:root`, CSS Selectors L4
///   §13.1 <https://www.w3.org/TR/selectors-4/#the-root-pseudo>) — matches
///   iff `ancestors.is_empty()`. Both call sites
///   ([`match_complex_selector_list`] for the rightmost compound,
///   [`match_from_element`] for compounds reached by crossing a combinator)
///   pass `ancestors` root-first/immediate-parent-last — [`collect_cascaded`]'s
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
fn compound_matches<D: StyleDom, E: StyleElement>(
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
fn matches_empty<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> bool {
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
struct SiblingMatchContext<'a> {
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
fn sibling_position<D: StyleDom>(
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
/// `elem` の親) — [`collect_cascaded`] の DFS 訪問順から構築される
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
/// ([`collect_cascaded`] doc 参照) ので、`current_id` の親が
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
/// [`collect_cascaded`] が [`selector_matches_pseudo_element`] という
/// 独立した matcher へ**この関数を経由させる前に**振り分けるため、
/// [`match_combinator_chain`] のどちらの呼び出し元 ([`selector_matches`] /
/// [`match_from_element`] 自身の再帰) もこの combinator を渡すことは無い。
/// [`compound_matches`] の `_ => false` safety net と
/// 同じ姿勢で、いずれの combinator も (到達すれば) ここでは match fail 扱い
/// にする。
///
/// # Spec provenance note
///
/// この doc および [`match_complex_selector_list`] / [`collect_cascaded`]
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
/// `LaterSibling` arm の両方から使う — [`collect_cascaded`] が
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
/// 明らかに軽量。[`collect_cascaded`] 側で既に解決済みの `elem` を再利用
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
