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

/// Matches one compound selector against `elem`, stopping when `iter` reaches the
/// next combinator or the end of the selector.
///
/// Both the initial rightmost-compound check in [`match_complex_selector_list`]
/// and candidate checks across combinators ([`match_combinator_chain`] /
/// [`match_from_element`]) use this function. Ancestor checks for descendant/child
/// combinators and sibling checks for NextSibling/LaterSibling share this helper.
/// It extracts the former `match_simple_selectors` body without changing any
/// per-component matching logic.
///
/// Iterating `iter: &mut SelectorIter` with `for component in iter` stops
/// at a combinator by the `selectors` crate's own contract (`Selector::iter`
/// documentation, verbatim:
/// "Returns an iterator over this selector in matching order
/// (right-to-left). When a combinator is reached, the iterator will return
/// None, and next_sequence() may be called to continue to the next
/// sequence."). `Component::Combinator` never appears as a component in this
/// loop: `SelectorIter::next()` stores the combinator internally and returns
/// `None`. The caller must distinguish a `false` result from a successful
/// compound followed by another combinator. It calls `iter.next_sequence()`
/// for the latter case; `true` only means no component of this compound failed.
///
/// Returns `true` if every component in this compound matches:
/// - `Component::LocalName(name)` — compare `elem.tag_name()` with
///   `eq_ignore_ascii_case`.
/// - `Component::ExplicitUniversalType` — always matches.
/// - `Component::ID` — compare with `elem.id()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#id-selectors>, verbatim: "When
///   matching against a document which is in quirks mode, IDs must be
///   matched ASCII case-insensitively; ID selectors are otherwise
///   case-sensitive"). Use `eq_ignore_ascii_case` only when `quirks_mode` is
///   [`StyleQuirksMode::Quirks`]; [`StyleQuirksMode::NoQuirks`] and
///   [`StyleQuirksMode::LimitedQuirks`] use exact matching. "Limited-quirks"
///   has a distinct DOM Standard definition from "quirks mode" and is not folded.
/// - `Component::Class` — `elem.has_class()` /
///   `elem.has_class_ascii_case_insensitive()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#class-html>, verbatim: "When matching
///   against a document which is in quirks mode, class names must be matched
///   ASCII case-insensitively; class selectors are otherwise case-sensitive").
///   Use the same `quirks_mode` branch as for IDs. Both variants share the same
///   HTML-spec ASCII whitespace tokenisation; see [`StyleElement::has_class`].
/// - `Component::AttributeInNoNamespaceExists` / `Component::AttributeInNoNamespace`
///   — use `elem.attr()` (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#attribute-selectors>). For an existence
///   selector (`[foo]`), choose the `local_name` or `local_name_lower` lookup key
///   according to the element's namespace (see the relevant match arm).
///   For selectors with values, see [`resolve_case_sensitivity`].
/// - `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
///   — resolve inherited effective language or directionality through
///   [`super::lang::language_range_matches`] / [`resolve_directionality`]
///   using `dom` and `ancestors` (the element's ancestor chain). This arm still
///   returns `false` for `PseudoClass::Hover` / `PseudoClass::Active`; dynamic
///   pseudo-classes remain outside the scope of this implementation.
/// - `Component::Root` (`:root`, CSS Selectors L4
///   §13.1 <https://www.w3.org/TR/selectors-4/#the-root-pseudo>) — matches
///   iff [`is_document_root_element`] (no element ancestor, in-document, and
///   immediate parent is the `Document` node). The matcher passes `ancestors`
///   root-first/immediate-parent-last — [`super::collect::collect_cascaded`]'s
///   doc establishes that only `StyleNodeKind::Element` nodes are ever
///   pushed onto `ancestor_path`, so an empty `ancestors` slice alone would
///   also hold for any disconnected root or fragment top-level child; the
///   parent + in-document checks exclude those, leaving exactly the spec's
///   "root of the document" (the `html` element in HTML documents).
/// - `Component::Scope` (`:scope`, CSS Selectors L4 §14.3.3
///   <https://www.w3.org/TR/selectors-4/#the-scope-pseudo>) — matches iff
///   `elem_id == scope` when a scope element is given, or falls back to
///   `:root` semantics ([`is_document_root_element`]) when it is not — the same
///   two-way rule the upstream `selectors` crate's own matcher applies to
///   this component (its `matching.rs`, `Component::Scope | Component::
///   ImplicitScope => match context.shared.scope_element { Some(e) => ...,
///   None => element.is_root() }`; this crate does not use that matcher,
///   but mirrors its rule here). `scope` is always `None` in a stylesheet
///   cascade (nothing here ever supplies one), so this arm never actually
///   changes cascade matching; it only takes effect through
///   [`crate::SelectorQuery::matches_scoped`], the DOM query entry point
///   `Element.matches`/`closest`/`querySelector`/`querySelectorAll` use
///   when their `this` is itself an `Element` (DOM §4.2.6, DOM §4.9).
/// - `Component::Empty` (`:empty`, CSS Selectors
///   L4 §13.2 <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>) — see
///   [`matches_empty`] doc for the whitespace handling.
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
/// `Component::RelativeSelectorAnchor` matches only when the enclosing
/// `:has()` matcher supplies the subject id; an anchor cannot match in an
/// ordinary stylesheet selector. Other components include namespace-qualified
/// attribute selectors (always `Component::AttributeOther`) and valued
/// attribute selectors with non-lowercase local names (also
/// `Component::AttributeOther`; an unvalued existence selector with a
/// non-lowercase name remains `AttributeInNoNamespaceExists` if it has no
/// explicit namespace; see the comment on `is_supported_selector_list` in
/// `ruletree.rs`). The `is_supported_selector_list` gate should already have
/// dropped these components when building the rule tree, but they still fail
/// matching as a safety net.
#[allow(clippy::too_many_arguments)] // one component-matching entry point threading element identity, quirks mode, and two independent optional binding contexts (`:has()` anchor, `:scope` element)
pub(crate) fn compound_matches<D: StyleDom, E: StyleElement>(
    dom: &D,
    iter: &mut SelectorIter<'_, RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> bool {
    use selectors::parser::Component;

    for component in iter {
        let component_matches = match component {
            Component::LocalName(local) => {
                // `local.name` is an Atom (`raikiri-style::Atom`); compare it to the tag name.
                elem.tag_name().eq_ignore_ascii_case(local.name.0.as_str())
            }
            Component::ExplicitUniversalType
            | Component::ExplicitAnyNamespace
            | Component::ExplicitNoNamespace
            | Component::DefaultNamespace(_) => {
                // Always match; namespaces are currently treated as always matching.
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
                // Using `local_name` directly is safe only in this with-value arm; unlike
                // the `AttributeInNoNamespaceExists` arm above, the element's namespace does
                // not matter. The `selectors` crate parser chooses `AttributeInNoNamespace`
                // only when the *selector's own* local name is already ASCII-lowercase.
                // Other names become `Component::AttributeOther` and are dropped by
                // `is_supported_selector_list`. This lowercase guarantee comes from parsing
                // the selector, independent of whether the matched element is HTML or foreign,
                // so no namespace branch is needed here.
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
                // `:hover` and `:active` remain unsupported dynamic pseudo-classes.
                // `is_supported_selector_list` drops them when building the rule tree (pinned
                // by `ruletree::tests::pseudo_class_selector_still_dropped`). The test
                // `match_complex_selector_list_rejects_unsupported_component_via_safety_net`
                // invokes this function directly to check the defensive failure.
                crate::PseudoClass::Hover | crate::PseudoClass::Active => false,
            },
            Component::Root => is_document_root_element(dom, elem_id, ancestors),
            Component::Scope => match scope {
                Some(scope_id) => elem_id == scope_id,
                None => is_document_root_element(dom, elem_id, ancestors),
            },
            Component::Empty => matches_empty(dom, elem_id),
            Component::Negation(selectors) => !selector_slice_matches_with_anchor(
                selectors.slice(),
                dom,
                elem,
                elem_id,
                ancestors,
                quirks_mode,
                relative_anchor,
                scope,
                allow_detached,
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
                    scope,
                    allow_detached,
                )
            }
            Component::Has(_) if relative_anchor.is_some() => false,
            Component::Has(relative_selectors) => has_relative_selector_matches(
                dom,
                relative_selectors,
                elem_id,
                ancestors,
                quirks_mode,
                scope,
                allow_detached,
            ),
            Component::Nth(data) => {
                // Sibling-list parent: the element parent when present,
                // otherwise the true immediate parent (Document for the
                // document element, a DocumentFragment for fragment
                // top-level children) via `StyleDom::parent_id`. A detached
                // root with no parent has no sibling list at all.
                let Some(sibling_parent) =
                    ancestors.last().copied().or_else(|| dom.parent_id(elem_id))
                else {
                    return false;
                };
                matches_nth(
                    dom,
                    sibling_parent,
                    elem_id,
                    elem.tag_name(),
                    data,
                    ancestors,
                    quirks_mode,
                    scope,
                    allow_detached,
                )
            }
            Component::NthOf(data) => {
                let Some(sibling_parent) =
                    ancestors.last().copied().or_else(|| dom.parent_id(elem_id))
                else {
                    return false;
                };
                matches_nth_of(
                    dom,
                    sibling_parent,
                    elem_id,
                    elem.tag_name(),
                    data,
                    ancestors,
                    quirks_mode,
                    scope,
                    allow_detached,
                )
            }
            Component::RelativeSelectorAnchor => relative_anchor == Some(elem_id),
            _ => {
                // Other components (`AttributeOther`) should have been dropped during
                // rule-tree construction, but fail matching here as a safety net.
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
/// CSS Selectors Level 4 allows document whitespace characters in an empty
/// element. The implementation treats space, tab, line feed, and carriage
/// return as document whitespace, while non-empty text and element children
/// make the element non-empty. Comments and processing instructions do not.
///
/// CSS Text Level 4 defines document whitespace as spaces, tabs, and segment
/// breaks; HTML normalizes segment breaks to line feeds and treats carriage
/// return as a space.
/// <https://drafts.csswg.org/css-text-4/#white-space-rules>
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
    allow_detached: bool,
    selector_filter: Option<&'a [Selector<RaikiriSelectorImpl>]>,
    ancestors: &'a [StyleNodeId],
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
}

/// 1-based sibling position of `elem_id` among `parent_id`'s element children
/// (in-document only when `allow_detached` is false; same-tree elements when
/// true), both from the start and from the end, plus the total count of such
/// siblings — shared arithmetic behind `Component::Nth` and `Component::NthOf`
/// matching ([`matches_nth`] / [`matches_nth_of`]).
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
/// always-true (`Component::DefaultNamespace(_) => true`, "namespaces
/// currently always match") for the equivalent
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
            continue; // cov:ignore: defensive against a detached/inert child_id that dom.node() can't resolve; no StyleDom impl in this crate's test corpus produces one.
        };
        // Candidates are already constrained to the subject's own parent (same
        // tree); `allow_detached` decides whether inert siblings count. See
        // `is_candidate_element` doc for the cascade vs query distinction.
        if !is_candidate_element(dom, child_id, context.allow_detached) {
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
                context.scope,
                context.allow_detached,
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
/// The structural sibling-position rules are defined by Selectors Level 3
/// §6.6 and retained by Level 4.
///
/// Selectors **Level 3** §6.6
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
#[allow(clippy::too_many_arguments)] // mirrors compound_matches's own Nth/NthOf parameter set (sibling position plus the shared matching context)
fn matches_nth<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    data: &NthSelectorData,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> bool {
    let (from_start, from_end, total) = sibling_position(
        dom,
        parent_id,
        elem_id,
        elem_tag,
        data.ty.is_of_type(),
        SiblingMatchContext {
            allow_detached,
            selector_filter: None,
            ancestors,
            quirks_mode,
            scope,
        },
    );
    matches_nth_position(from_start, from_end, total, data)
}

/// `Component::NthOf` matching for CSS Selectors L4's `:nth-child(An+B of S)`
/// and `:nth-last-child(An+B of S)` forms. The specification defines the
/// position among the inclusive siblings that match `S`; the selector-list
/// matcher therefore runs for every direct element child before the normal
/// `An+B` arithmetic is applied.
#[allow(clippy::too_many_arguments)] // mirrors matches_nth's own parameter set, plus the An+B-of-S selector-list data
fn matches_nth_of<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    elem_id: StyleNodeId,
    elem_tag: &str,
    data: &NthOfSelectorData<RaikiriSelectorImpl>,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> bool {
    let nth_data = data.nth_data();
    let (from_start, from_end, total) = sibling_position(
        dom,
        parent_id,
        elem_id,
        elem_tag,
        nth_data.ty.is_of_type(),
        SiblingMatchContext {
            allow_detached,
            selector_filter: Some(data.selectors()),
            ancestors,
            quirks_mode,
            scope,
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

/// Matches a selector list against `elem` (and, for combinators, ancestor
/// elements in `ancestors` or preceding siblings found from `elem_id`).
///
/// # Combinator support
///
/// Initially only single-element (compound-only) matching was supported, and
/// the `is_supported_selector_list` gate in `ruletree.rs` dropped selectors
/// with combinators before they reached the rule tree. Descendant (space, CSS
/// Selectors L4 <https://www.w3.org/TR/selectors-4/#descendant-combinators>)
/// and child (`>`, <https://www.w3.org/TR/selectors-4/#child-combinators>) were
/// added first. Adjacent sibling (`+`,
/// <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>) and
/// general sibling (`~`,
/// <https://www.w3.org/TR/selectors-4/#general-sibling-combinators>) followed.
/// All four combinators are supported; see [`match_combinator_chain`]. The
/// CSSWG Editor's Draft <https://drafts.csswg.org/selectors-4/#complex>
/// describes matching a complex selector (verbatim; see the provenance note
/// in [`match_combinator_chain`]): "A given element ... is said to match a
/// complex selector when it matches the final compound selector ... in the
/// sequence, and every preceding unit of the sequence also matches an
/// element ..., with the correct relationship between consecutive units as
/// expressed by the combinators separating them". This function implements
/// that rule right-to-left, from `elem` to ancestors or siblings, by iterating
/// with `Selector::iter` and `SelectorIter::next_sequence`:
///
/// 1. Check the rightmost compound against `elem` with [`compound_matches`].
/// 2. If it fails, move to the next selector.
/// 3. Otherwise, inspect the next combinator with `iter.next_sequence()`:
///    - `None` means the entire selector matches.
///    - `Some(combinator)` delegates to [`match_combinator_chain`], which checks
///      the next compound against the immediate parent (child), any ancestor
///      (descendant), the immediate preceding sibling (next-sibling), or any
///      preceding sibling (later-sibling).
///
/// `ancestors` runs from the root to the immediate parent (`ancestors.last()`
/// is `elem`'s parent), following the DFS order of
/// [`super::collect::collect_cascaded`] (see its documentation). `elem_id` is
/// the id of `elem`. Sibling combinators use it as the stopping point when
/// searching [`StyleDom::child_ids`] for children before `elem` (see the
/// `NextSibling`/`LaterSibling` arms of [`match_combinator_chain`]). It is
/// also passed to [`compound_matches`]: structural pseudo-classes such as
/// `:root`, `:empty`, and `:nth-child()` need `dom`, `elem_id`, and
/// `ancestors.last()` (the parent), which the borrowed `elem` alone cannot
/// provide.
///
/// Returns the greatest specificity of any matching selector, or `None` if
/// none matches. `specificity_of` computes specificity for the whole complex
/// selector, across combinators; adding combinators did not change this call
/// because the `selectors` crate computes it from the complete selector.
#[allow(clippy::too_many_arguments)] // matching context plus allow_detached for cascade vs query gating
pub(crate) fn match_complex_selector_list<D: StyleDom, E: StyleElement>(
    list: &SelectorList<RaikiriSelectorImpl>,
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> Option<Specificity> {
    let mut best: Option<Specificity> = None;
    for selector in list.slice() {
        if selector_matches(
            dom,
            selector,
            elem,
            elem_id,
            ancestors,
            quirks_mode,
            scope,
            allow_detached,
        ) {
            let spec = specificity_of(selector);
            best = Some(match best {
                Some(prev) => prev.max(spec),
                None => spec,
            });
        }
    }
    best
}

#[allow(clippy::too_many_arguments)] // thin passthrough carrying matching context plus allow_detached
fn selector_slice_matches<D: StyleDom, E: StyleElement>(
    selectors: &[Selector<RaikiriSelectorImpl>],
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> bool {
    selector_slice_matches_with_anchor(
        selectors,
        dom,
        elem,
        elem_id,
        ancestors,
        quirks_mode,
        None,
        scope,
        allow_detached,
    )
}

#[allow(clippy::too_many_arguments)] // thin passthrough carrying every matching-context parameter down to selector_matches_with_anchor
fn selector_slice_matches_with_anchor<D: StyleDom, E: StyleElement>(
    selectors: &[Selector<RaikiriSelectorImpl>],
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
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
            scope,
            allow_detached,
        )
    })
}

#[allow(clippy::too_many_arguments)] // thin passthrough carrying matching context plus allow_detached
fn selector_matches<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> bool {
    selector_matches_with_anchor(
        dom,
        selector,
        elem,
        elem_id,
        ancestors,
        quirks_mode,
        None,
        scope,
        allow_detached,
    )
}

/// Matches a selector while optionally binding the internal
/// `RelativeSelectorAnchor` component generated for a `:has()` argument to a
/// particular element. Ordinary stylesheet selectors use [`selector_matches`]
/// and therefore cannot match that internal component.
#[allow(clippy::too_many_arguments)] // same matching context as compound_matches, plus the selector and iterator it drives
fn selector_matches_with_anchor<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
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
        scope,
        allow_detached,
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
            scope,
            allow_detached,
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
    scope: Option<StyleNodeId>,
    allow_detached: bool,
) -> bool {
    for relative_selector in relative_selectors {
        let leading_combinator = relative_selector.selector.combinator_at_parse_order(1);
        let include_descendants = relative_selector.match_hint.is_subtree();

        let (roots, candidate_ancestors) = match leading_combinator {
            Combinator::Child | Combinator::Descendant => {
                let roots = in_document_element_children(dom, anchor_id, allow_detached);
                let mut candidate_ancestors = anchor_ancestors.to_vec();
                candidate_ancestors.push(anchor_id);
                (roots, candidate_ancestors)
            }
            Combinator::NextSibling | Combinator::LaterSibling => {
                let Some(parent_id) = anchor_ancestors
                    .last()
                    .copied()
                    .or_else(|| dom.parent_id(anchor_id))
                else {
                    continue;
                };
                let roots = following_sibling_elements(
                    dom,
                    parent_id,
                    anchor_id,
                    relative_selector.match_hint.is_next_sibling(),
                    allow_detached,
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
            scope,
            allow_detached,
        ) {
            return true;
        }
    }
    false
}

/// Searches the candidate roots and, when the relative selector can reach
/// deeper nodes, their element subtrees. The explicit stack avoids consuming
/// the native call stack on deeply nested untrusted markup.
#[allow(clippy::too_many_arguments)] // the :has() candidate search needs its own region/anchor data plus the shared matching context
fn relative_selector_matches_in_regions<D: StyleDom>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    roots: &[StyleNodeId],
    base_ancestors: &[StyleNodeId],
    include_descendants: bool,
    anchor_id: StyleNodeId,
    quirks_mode: StyleQuirksMode,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
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
        if !is_candidate_element(dom, candidate_id, allow_detached) {
            continue; // cov:ignore: roots are pre-filtered; only a malformed custom DOM can reach this branch
        }
        let Some(node) = dom.node(candidate_id) else {
            continue; // cov:ignore: is_candidate_element already required a node for this id
        };
        let Some(elem) = node.as_element() else {
            continue; // cov:ignore: is_candidate_element already required an element node
        };
        if selector_matches_with_anchor(
            dom,
            selector,
            &elem,
            candidate_id,
            &ancestor_path,
            quirks_mode,
            Some(anchor_id),
            scope,
            allow_detached,
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

/// Returns direct element children in document order (`allow_detached` decides
/// whether inert siblings count; see `is_candidate_element`).
fn in_document_element_children<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    allow_detached: bool,
) -> Vec<StyleNodeId> {
    dom.child_ids(parent_id)
        .filter(|&id| is_candidate_element(dom, id, allow_detached))
        .collect()
}

/// Returns the following element siblings of `anchor_id`, optionally limited
/// to the first one for the adjacent-sibling relative combinator.
fn following_sibling_elements<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    anchor_id: StyleNodeId,
    only_first: bool,
    allow_detached: bool,
) -> Vec<StyleNodeId> {
    let mut following = false;
    let mut result = Vec::new();
    for child_id in dom.child_ids(parent_id) {
        if child_id == anchor_id {
            following = true;
            continue;
        }
        if following && is_candidate_element(dom, child_id, allow_detached) {
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
/// # Why `Component::PseudoElement` is restricted to the leading compound
///
/// The parser rejects a pseudo-class, another pseudo-element, or a following
/// compound after `::before`/`::after`. A successfully parsed selector
/// therefore contains at most one pseudo-element, in its own leading compound.
pub(crate) fn selector_matches_pseudo_element<D: StyleDom, E: StyleElement>(
    dom: &D,
    selector: &Selector<RaikiriSelectorImpl>,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    quirks_mode: StyleQuirksMode,
    allow_detached: bool,
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
    let matches = compound_matches(
        dom,
        &mut iter,
        elem,
        elem_id,
        ancestors,
        quirks_mode,
        None,
        None,
        allow_detached,
    ) && match iter.next_sequence() {
        None => true,
        Some(next_combinator) => match_combinator_chain(
            dom,
            next_combinator,
            elem_id,
            ancestors,
            iter,
            quirks_mode,
            None,
            None,
            allow_detached,
        ),
    };
    matches.then_some(pseudo)
}

/// After [`match_complex_selector_list`] matches the rightmost compound
/// against `elem`, check the remaining combinator/compound sequence by walking
/// back through `ancestors` or preceding siblings found from `current_id`.
///
/// - [`Combinator::Child`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim: "A
///   child combinator describes a childhood relationship between two
///   elements") — try exactly one candidate: the last entry in `ancestors`
///   (the immediate parent). It must match the next compound; if more
///   combinators remain to the left, the rest must match from that parent's
///   ancestors. No backtracking is needed: `>` uniquely identifies the parent.
/// - [`Combinator::Descendant`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#descendant-combinators>, verbatim:
///   "A selector of the form A B represents an element B that is an
///   arbitrary descendant of some ancestor element A") — try each ancestor
///   from the immediate parent toward the root. For a candidate that matches
///   the next compound, continue matching the remaining sequence from that
///   candidate. If the rest fails, retry with the next, more distant ancestor.
///   `iter.clone()` gives each candidate an independent iterator copy
///   (`selectors::parser::SelectorIter` implements `Clone`). This follows the
///   CSSWG Editor's Draft <https://drafts.csswg.org/selectors-4/#complex>
///   definition of complex matching: each unit matches some element while
///   adjacent units satisfy their combinator relationship.
///
///   The retry is required when a descendant combinator is followed by a
///   child combinator: each candidate may have a different immediate parent.
///   For example, `.x > .y .target` can require trying an outer `.y` after
///   the nearer `.y` fails the child relation. The result is independent of
///   candidate order because matching asks whether any candidate succeeds.
/// - [`Combinator::NextSibling`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>
///   §14.3, verbatim: "The elements represented by the two compound
///   selectors share the same parent in the document tree and the element
///   represented by the first compound selector immediately precedes the
///   element represented by the second one. Non-element nodes (e.g. text
///   between elements) are ignored when considering the adjacency of
///   elements.") — try only the element immediately before `current_id` in
///   the child list of its parent (`ancestors.last()`, or
///   [`StyleDom::root_id`] when absent; see the note below), obtained through
///   [`immediate_preceding_sibling`]. There is no backtracking: `+` identifies
///   exactly one preceding sibling, as [`Combinator::Child`] does for parents.
/// - [`Combinator::LaterSibling`] (CSS Selectors L4
///   <https://www.w3.org/TR/selectors-4/#general-sibling-combinators> §14.4,
///   verbatim: "The elements represented by the two compound selectors
///   share the same parent in the document tree and the element
///   represented by the first compound selector precedes (not necessarily
///   immediately) the element represented by the second one.") — try the
///   parent's children from the beginning, stopping at `current_id`. When a
///   candidate matches, continue checking the sequence to its left from
///   there. As with [`Combinator::Descendant`], retry candidates along a
///   single linear list. Sibling lists do not branch, so search order cannot
///   affect correctness; this uses the natural `child_ids` order (earliest
///   sibling first).
///
/// # Resolving the parent when `ancestors.last()` is empty
///
/// `ancestor_path` contains only nodes of `Element` kind (see
/// [`super::collect::collect_cascaded`]). If `current_id`'s parent is the
/// [`StyleNodeKind::Document`] root itself, as with an element directly under
/// the document (`<html>`, for example), `ancestors` is empty. `Child` and
/// `Descendant` correctly treat this as no parent capable of matching a
/// compound (`ancestors.split_last() => None`). A sibling combinator does not
/// match the parent against a compound; it only needs the parent id as the
/// lookup key for [`StyleDom::child_ids`]. Even under the root, siblings can
/// exist: `<h2>` and `<p>` can both be direct document children, as in the
/// acceptance test. Only the `NextSibling`/`LaterSibling` arms therefore look
/// up [`StyleDom::parent_id`] when `ancestors.last()` is `None` (the
/// `Document` node for the document element, a `DocumentFragment` for
/// fragment top-level children, `None` for a detached root with no siblings).
/// `Child` and `Descendant` must not fall back: the root never matches a
/// compound.
///
/// Other combinators ([`Combinator::PseudoElement`] /
/// [`Combinator::SlotAssignment`] / [`Combinator::Part`]) are outside this
/// function's supported set. [`Combinator::SlotAssignment`] and
/// [`Combinator::Part`] remain pseudo-element-only combinators. This crate's
/// `parse_selector_list` (`RaikiriSelectorImpl`) rejects their `::slotted()`
/// and `::part()` syntax because `parse_slotted` and `parse_part` are not
/// overridden (see the `is_supported_selector` doc in `ruletree.rs`). Thus a
/// `SelectorList` produced by this crate cannot reach them.
/// [`Combinator::PseudoElement`] (`::before`/`::after`) differs: it now parses,
/// enters a `SelectorList`, and survives the `is_supported_selector` rule-tree
/// gate. But [`super::collect::collect_cascaded`] dispatches to a separate
/// matcher, [`selector_matches_pseudo_element`], *before* this function. Neither
/// caller of [`match_combinator_chain`] ([`selector_matches`] or the explicit
/// continuation of [`match_from_element`]) passes it this combinator. As with
/// the `_ => false` safety net in [`compound_matches`], reaching any of these
/// combinators here fails matching.
///
/// # Implementation: explicit `Vec` stack, not native recursion
///
/// The matcher uses an explicit heap-backed stack so selector depth does not
/// consume the native call stack.
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
/// The "continue matching" descriptions above refer to the *logical*
/// structure of the search: CSS complex selectors are themselves defined
/// recursively (CSSWG ED `#complex`, cited above). The implementation uses
/// explicit stack push/pop for that logical recursion, never the native call
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
/// *not* hit this bound: it matches via the nearest-candidate-first path.
/// An unsatisfiable selector with one more compound than available ancestors
/// reaches the `S(k) = 2^(k-1)` bound. This is an untrusted-depth CPU-exhaustion
/// risk,
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
#[allow(clippy::too_many_arguments)] // drives the explicit-stack combinator search: its own combinator/candidate state plus the shared matching context
fn match_combinator_chain<D: StyleDom>(
    dom: &D,
    combinator: Combinator,
    current_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    iter: SelectorIter<'_, RaikiriSelectorImpl>,
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
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
        candidates: pending_candidates_for(dom, combinator, current_id, ancestors, allow_detached),
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
            frame
                .candidates
                .next(dom, frame.ancestors_unchanged, allow_detached)
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
            scope,
            allow_detached,
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
                        allow_detached,
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
        allow_detached: bool,
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
                    if is_candidate_element(dom, candidate_id, allow_detached) {
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
/// (unsupported combinators, see this module's "Other combinators" doc note)
/// yields an already-exhausted `Child(None)` cursor, the same "no candidate
/// ever succeeds" outcome the pre-fix `_ => false` arm produced.
fn pending_candidates_for<'a, D: StyleDom + 'a>(
    dom: &'a D,
    combinator: Combinator,
    current_id: StyleNodeId,
    ancestors: &'a [StyleNodeId],
    allow_detached: bool,
) -> PendingCandidates<'a, D> {
    match combinator {
        Combinator::Child => PendingCandidates::Child(
            ancestors
                .split_last()
                .map(|(&parent_id, rest)| (parent_id, rest)),
        ),
        Combinator::Descendant => PendingCandidates::Descendant(ancestors),
        Combinator::NextSibling => {
            let Some(parent_id) = ancestors
                .last()
                .copied()
                .or_else(|| dom.parent_id(current_id))
            else {
                return PendingCandidates::Child(None);
            };
            PendingCandidates::NextSibling(immediate_preceding_sibling(
                dom,
                parent_id,
                current_id,
                allow_detached,
            ))
        }
        Combinator::LaterSibling => {
            let Some(parent_id) = ancestors
                .last()
                .copied()
                .or_else(|| dom.parent_id(current_id))
            else {
                return PendingCandidates::Child(None);
            };
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
        // See this module's "Other combinators" doc note, above
        // `match_combinator_chain`, for the fuller argument. Yields an
        // already-exhausted `Child(None)` cursor — same "no candidate ever
        // succeeds" outcome the pre-fix `_ => false` arm produced.
        _ => PendingCandidates::Child(None),
    }
}

/// Shared predicate: whether `id` is an `Element` candidate for sibling and
/// structural matching (sibling combinators, `:first-child` etc., `:has()`
/// regions).
///
/// When `allow_detached` is false (stylesheet cascade), both the element kind
/// and [`StyleNode::is_in_document`] are required, so inert siblings (e.g. a
/// `set_in_document(false)` mock, or any detached node) are ignored — this is
/// what keeps `li:nth-child(2 of .featured)` ignoring inert siblings and what
/// keeps template-contents inertness in the cascade alongside
/// [`super::collect::collect_cascaded`]'s top-level `!is_in_document` skip
/// (inert subtrees are never visited as subjects) and the structural fact that
/// a `<template>` element's contents live in a separate detached fragment
/// (never in the template element's own `child_ids`).
///
/// When true (DOM query entry points via [`crate::SelectorQuery`]), only the
/// element kind is required. Candidates are enumerated from the subject's own
/// parent (`parent_id` derived from `ancestors` or [`StyleDom::parent_id`]), so
/// they are by construction in the same tree as the subject; detached and
/// `DocumentFragment` trees therefore match correctly.
fn is_candidate_element<D: StyleDom>(dom: &D, id: StyleNodeId, allow_detached: bool) -> bool {
    dom.node(id).is_some_and(|node| {
        node.kind() == StyleNodeKind::Element && (allow_detached || node.is_in_document())
    })
}

/// Whether `elem_id` is the document element for `:root` (CSS Selectors L4
/// §13.1, "root of the document"; in HTML documents the `html` element).
///
/// Requires all three: no element ancestor (`ancestors.is_empty()`),
/// flat-tree membership (`is_in_document`), and an immediate parent that is
/// the `Document` node itself ([`StyleDom::parent_id`] == [`StyleDom::root_id`]).
/// Any disconnected root or `DocumentFragment` top-level child fails at least
/// one of the last two: detached nodes are not in-document, and fragment
/// children parent to the fragment rather than the `Document` node.
fn is_document_root_element<D: StyleDom>(
    dom: &D,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
) -> bool {
    if !ancestors.is_empty() {
        return false;
    }
    let in_doc = dom.node(elem_id).is_some_and(|node| node.is_in_document());
    in_doc && dom.parent_id(elem_id) == Some(dom.root_id())
}

/// Returns the element id immediately preceding `current_id` among the
/// direct children of `parent_id` (for [`Combinator::NextSibling`]). Scan
/// [`StyleDom::child_ids`] once from the start; upon reaching `current_id`,
/// return the last element candidate seen. This does not allocate a `Vec`,
/// following the other helpers' policy of avoiding disposable vectors.
///
/// Exclude non-element nodes (such as text), as CSS Selectors L4 says of the
/// next-sibling combinator (verbatim): "Non-element nodes (e.g. text
/// between elements) are ignored when considering the adjacency of
/// elements" (<https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>).
fn immediate_preceding_sibling<D: StyleDom>(
    dom: &D,
    parent_id: StyleNodeId,
    current_id: StyleNodeId,
    allow_detached: bool,
) -> Option<StyleNodeId> {
    let mut last_element = None;
    for candidate_id in dom.child_ids(parent_id) {
        if candidate_id == current_id {
            return last_element;
        }
        if is_candidate_element(dom, candidate_id, allow_detached) {
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

/// Resolve the element identified by `elem_id` and match the compound at
/// `iter` against it using [`compound_matches`]. Ancestor (`Child` /
/// `Descendant`) and sibling (`NextSibling` / `LaterSibling`) candidates share
/// this helper: resolving an id and checking a compound does not depend on
/// the combinator that produced the candidate. Sibling jumps keep the same
/// `ancestors` because siblings have the same parent; ancestor jumps pass the
/// remaining `ancestors` after `split_last` or backtracking truncates it.
/// Formerly named `match_from_ancestor`, it was renamed `match_from_element`
/// when sibling candidates began using it too.
///
/// Compound matching returns the advanced iterator to
/// [`match_combinator_chain`], which drives the remaining selector with an
/// explicit stack. This keeps the selector walk bounded by the selector and
/// ancestor-chain lengths rather than the native call stack.
///
/// The element is resolved from its [`StyleNodeId`] for each candidate; the
/// same helper serves ancestor and sibling combinators.
///
/// Returns `Some` with `iter` advanced beyond the matching compound so the
/// caller can check any remaining compounds to the left, or `None` if the
/// compound does not match.
#[allow(clippy::too_many_arguments)] // same matching context as compound_matches plus allow_detached
fn match_from_element<'s, D: StyleDom>(
    dom: &D,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
    mut iter: SelectorIter<'s, RaikiriSelectorImpl>,
    quirks_mode: StyleQuirksMode,
    relative_anchor: Option<StyleNodeId>,
    scope: Option<StyleNodeId>,
    allow_detached: bool,
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
        scope,
        allow_detached,
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

/// Resolve `Component::AttributeInNoNamespace`'s `ParsedCaseSensitivity`
/// (a spec-level "language depends on this" placeholder for the
/// `AsciiCaseInsensitiveIfInHtmlElementInHtmlDocument` case) to a
/// `CaseSensitivity` suitable for `AttrSelectorOperator::eval_str`.
///
/// This models the same three-way branch as upstream
/// `selectors::matching::to_unconditional_case_sensitivity`, but cannot call
/// that function: it requires the full tree-walking `selectors::Element`
/// trait, while raikiri's `StyleElement` is a reduced trait for single-element
/// matching. This private helper reimplements the branch without crossing
/// the `wall/traits` boundary. Raikiri currently supports only HTML documents
/// (not XML/XHTML), so "in html document" is always true. It uses the existing
/// [`StyleElement::namespace_uri`] contract (style_dom.rs: "HTML default
/// namespace returns None (optimized path)") as a proxy for "is html
/// element"; SVG and other non-HTML namespaces remain case-sensitive.
///
/// # "In html document" differs from quirks mode
///
/// Here, "in html document" concerns the document **language** (HTML versus
/// XML) under CSS Selectors L4 §3.7/§6.3. This differs from the quirks-mode
/// axis (`StyleQuirksMode`, CSS Selectors L4 §6.6/§6.7) used for id/class
/// matching. Do not conflate them. `raikiri-html` uses only an HTML5 tree
/// builder and cannot produce an XML document, so this axis is currently
/// unconditionally true. XML document parsing would require passing the
/// document language to this function.
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
mod tests;
