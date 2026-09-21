//! CSS cascade + inheritance walk。
//!
//! 2 phase:
//! 1. per-node "cascaded values" 決定 — matching rule + inline style の候補集合から
//!    specificity + !important + source order で winner を選択
//! 2. inheritance walk — top-down DFS で親の computed value を継承 + 自 node の
//!    cascaded value で override
//!
//! # inheritance walk 内部の 4 段階
//!
//! 上記 phase 2 の per-node 処理は、さらに 4 段に分かれる (順に phase 1 /
//! 2 / 2.5 / 3 と呼ぶ):
//!
//! - **phase 1: winner の staging** — 親の [`ComputedValues`] から
//!   [`SpecifiedValues`] を seed し、その node の全 winner を `apply_value` で
//!   適用する。この段では length は specified 表現のまま。
//! - **phase 2: font-size の絶対化** — **親の** computed font-size 基準。
//! - **phase 2.5: line-height の絶対化** — 自 node の (今確定した) font-size
//!   基準。`lh`/`rlh` の自己参照基準の非対称は
//!   [`crate::resolve::resolve_line_height`] doc が canonical。
//! - **phase 3: 残り全 length の絶対化** — **自 node の** computed font-size /
//!   line-height 基準。
//!
//! 2 / 2.5 / 3 は [`SpecifiedValues::finalize`] に閉じている。分離が必要な理由は
//! [`crate::specified`] の module doc を参照 (`padding: 2em` の基準となる
//! `font-size` はその node の**全** winner を適用し終えるまで確定しないため、
//! winner 適用の途中で絶対化することはできない)。

use std::collections::HashMap;
use std::collections::HashSet;

use cssparser::{Parser, ParserInput};
use selectors::parser::SelectorList;

use crate::computed::{ComputedValues, CustomPropertyEnvironment};
use crate::counter_style::CounterStyleRegistry;
use crate::error::CascadeError;
use crate::media::MediaContext;
use crate::page::{PageCascadeResult, PageContextQuery, PageInheritance, cascade_page};
use crate::property::{
    CustomProperty, DeferredValue, FontWeightValue, MAX_SUBSTITUTED_VALUE_BYTES, RelativeFontSize,
    Sides, WritingMode, parse_value,
};
use crate::resolve::ResolveContext;
use crate::ruletree::Origin;
use crate::ruletree::RuleTree;
use crate::specified::SpecifiedValues;
use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};
use crate::{PseudoElem, RaikiriSelectorImpl};

/// Cascade 結果。
///
/// 将来の GCPM (paged media generated content) static-side 実装では、per-node
/// ComputedValues 内で content / string_set / running_templates を保持する
/// canonical taxonomy に落ち着く見込みで、CascadeResult-level の
/// `gcpm_directives` / `running_templates` は下流 (raikiri-dom) で
/// per-document に concatenate される責務に移る。`#[non_exhaustive]` は将来
/// field 追加のために維持。
#[derive(Debug)]
#[non_exhaustive]
pub struct CascadeResult {
    /// Per-node computed values (NodeId.0 as usize で index)。
    /// Element / Text / Document 全 kind に populate、範囲外は panic (caller 責任)。
    pub computed: Vec<ComputedValues>,
    /// Per-node flags identifying margin sides whose winning declaration came
    /// from an origin other than the user-agent stylesheet.  The paged DOM
    /// adapter uses this to distinguish an authored `margin: 8px` from the
    /// minimal UA body's default `margin: 8px` before it builds the synthetic
    /// page root.  The side order is [`Sides`] top/right/bottom/left and the
    /// vector follows the same node-index contract as [`Self::computed`].
    pub non_ua_margin_sides: Vec<Sides<bool>>,
    /// Authored `writing-mode` winners before the computed-value normalization
    /// that currently collapses vertical modes to `horizontal-tb`.  This keeps
    /// paged consumers able to apply physical page-context mapping without
    /// changing ordinary element layout semantics.
    pub authored_writing_modes: Vec<Option<WritingMode>>,
    /// The `@page` cascade for the page query supplied to the cascade entry
    /// point. The compatibility entry point uses the unnamed/default query;
    /// paged consumers should use [`cascade_with_media_context_for_page`].
    pub page: PageCascadeResult,
    /// The winning `@counter-style` registry captured from the rule tree.
    ///
    /// Counter styles are stylesheet-level rules, not per-node computed
    /// values, but marker and generated-content paint runs only receive the
    /// cascade result. Cloning this small registry at cascade time keeps the
    /// rule-tree ownership boundary while making custom counter formatting
    /// available to those consumers.
    pub counter_styles: CounterStyleRegistry,
    /// Per-node computed page type (`page: auto | <custom-ident>`).
    ///
    /// This is kept beside, rather than inside, `ComputedValues` because the
    /// page property is consumed by the pagination driver and is not part of
    /// the element-to-taffy computed style bag.  The vector has the same arena
    /// indexing contract as [`Self::computed`].
    pub page_values: Vec<crate::property::PageValue>,
    /// `::before` / `::after` — sparse, keyed by `(originating element's own
    /// NodeId, which pseudo)`. An entry exists **iff** at least one
    /// stylesheet rule's selector targets that pseudo-element and matches
    /// that element (CSS Pseudo-Elements Module Level 4 §4.1
    /// <https://drafts.csswg.org/css-pseudo-4/#generated-content>) — most
    /// elements have neither `::before` nor `::after` rules, so this stays
    /// empty for them (no wasted entry).
    ///
    /// Each value is a **full** [`ComputedValues`], computed the same way a
    /// real element's is — cascaded winners from matching `::before`/
    /// `::after` selectors applied over inherited-from-the-real-element
    /// values, then absolutized — not just a lone `content` value. Two
    /// distinct spec statements justify this, and neither alone would:
    /// §4 `#treelike` (tree-abiding pseudo-elements, which `::before`/
    /// `::after` are a subcase of) states the *unconditioned* inheritance
    /// model this crate actually implements — "They inherit any inheritable
    /// properties from their originating element; non-inheritable
    /// properties take their initial values as usual" — independent of what
    /// `content` computes to. §4.1 `#generated-content` separately states
    /// the *content-conditioned* box-generation model — "When their
    /// computed 'content' value is not 'none', these pseudo-elements
    /// generate boxes as if they were immediate children of their
    /// originating element" — which is a downstream (`raikiri-dom`)
    /// decision, not something this crate's cascade computes. So a pseudo
    /// inherits its `color`/`font-*`/etc. from the real element exactly
    /// like a real child would (§4, unconditionally), and can have those
    /// (and any other property) overridden by its own `::before`/`::after`
    /// rule; whether a box is actually generated from the result (§4.1,
    /// conditioned on `content`) is left to the consumer, per this doc's own
    /// content representation note below. See [`resolve_inheritance`]'s
    /// pseudo-element section for the derivation.
    ///
    /// This crate represents `content: normal` as an empty `content` list and
    /// explicit `content: none` with [`crate::property::ContentComponent::None`] inside the
    /// list ([`ComputedValues::content`] doc) — a present
    /// map entry with an empty `content` list means "a `::before`/`::after`
    /// rule matched, but its (or the initial) `content` value is `normal`";
    /// an explicit `none` carries the sentinel. CSS Content Module Level 3
    /// <https://www.w3.org/TR/css-content-3/#content-property> is the
    /// primary source for what a non-empty `content` value implies for box
    /// generation — this crate stops at exposing the computed value; the
    /// consumer decides box generation from whether the list is empty or
    /// carries [`crate::property::ContentComponent::None`], not from
    /// map presence (map presence only means "some `::before`/`::after`
    /// rule matched this element", independent of what that rule set
    /// `content` to). This content representation ⇒ suppress-box reading is
    /// scoped to **this** (`pseudo`) map's entries specifically — it does
    /// not carry over to [`Self::computed`] (real elements). CSS Content 3
    /// §1 <https://www.w3.org/TR/css-content-3/#content-property> draws
    /// exactly this real-element/pseudo-element split itself: "For
    /// elements, \[`content`\] has only one purpose: specifying that the
    /// element renders as normal, or replacing the element with an image
    /// [...]. For pseudo-elements [...], it is more powerful. It controls
    /// whether the element renders at all, can replace the element with an
    /// image, or replace it with arbitrary inline content" — an empty
    /// `content` list on a **real** element's own [`Self::computed`] entry
    /// means "renders as normal" (no box-suppression meaning at all), while
    /// the same empty list on a **pseudo**'s [`Self::pseudo`] entry means
    /// "no box" (§4.1's `content: not none` condition above).
    pub pseudo: HashMap<(StyleNodeId, PseudoElem), ComputedValues>,
}

/// DOM + RuleTree から per-node ComputedValues を produce。
///
/// 現時点では常に `Ok` を返す (invalid CSS は既に build_rule_tree 段で silently
/// drop されており、cascade は construct され得ない)。`Result` signature は
/// 将来 fail-hard mode 用に維持。
///
/// # Example
///
/// ```ignore
/// // ignore: raikiri-dom crate は raikiri-style の doc-test から使えないため
/// // (crate cycle 回避)、実 code は integration test で確認。ここは shape のみ。
/// use raikiri_style::{build_rule_tree, cascade, ComputedValues};
///
/// # fn demo<D: raikiri_style::StyleDom>(dom: &D) {
/// let rule_tree = build_rule_tree(dom);
/// let result = cascade(dom, &rule_tree).expect("現時点では常に Ok");
/// let root_style: &ComputedValues = &result.computed[0];
/// # }
/// ```
pub fn cascade<D: StyleDom>(dom: &D, rule_tree: &RuleTree) -> Result<CascadeResult, CascadeError> {
    cascade_with_media_context(dom, rule_tree, &MediaContext::default())
}

/// Run the cascade for an explicit media context.
///
/// [`cascade`] remains the compatibility entry point and uses the default
/// paged (`print`) context.
pub fn cascade_with_media_context<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
) -> Result<CascadeResult, CascadeError> {
    cascade_with_media_context_for_page(dom, rule_tree, media_context, &PageContextQuery::default())
}

/// Run the element and `@page` cascades for one page-context query.
///
/// This is the page-aware sibling of [`cascade_with_media_context`]. It keeps
/// the ordinary element cascade and the page-context cascade in one result so
/// a paged consumer cannot accidentally render with a page box selected from a
/// different stylesheet or root inheritance context.
pub fn cascade_with_media_context_for_page<D: StyleDom>(
    dom: &D,
    rule_tree: &RuleTree,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> Result<CascadeResult, CascadeError> {
    let mut cascaded = CascadedArena::new();

    // Phase 1: per-node cascaded values を収集
    collect_cascaded_with_media_context(
        dom,
        dom.root_id(),
        rule_tree,
        &mut cascaded,
        media_context,
    );

    // Phase 2: inheritance walk。
    //
    // computed を Dom::node_count() で pre-allocate する。resolve_inheritance の
    // DFS は root reachable な node のみを訪問するため、detached / unreachable
    // node (foster-parenting transient、strip 後の孤児 node 等) には entry を
    // 作らない。しかし `computed.len() == document.node_count()` という contract
    // は arena 全体を要求する (`raikiri-dom::layout::preshape_text` /
    // `raikiri-paint::text::draw_text_node` が node_id で `computed[idx]` に
    // 直接 index する)。事前に initial() で埋めておき、DFS で visited slot を
    // 上書きする実装。
    let mut computed: Vec<ComputedValues> = vec![ComputedValues::initial(); dom.node_count()];
    let mut non_ua_margin_sides = vec![Sides::all(false); dom.node_count()];
    let mut authored_writing_modes = vec![None; dom.node_count()];
    let mut page_values = vec![crate::property::PageValue::Auto; dom.node_count()];
    let mut pseudo: HashMap<(StyleNodeId, PseudoElem), ComputedValues> = HashMap::new();
    resolve_inheritance(
        dom,
        dom.root_id(),
        &ComputedValues::initial(),
        &cascaded,
        &mut computed,
        &mut non_ua_margin_sides,
        &mut authored_writing_modes,
        &mut page_values,
        &mut pseudo,
    );

    // `StyleDom::root_id()` is the Document node.  Page properties inherit
    // from the first direct element child (the root element), not from that
    // Document node's initial values.
    let root_element_id = dom.child_ids(dom.root_id()).find(|id| {
        dom.node(*id)
            .is_some_and(|node| node.kind() == StyleNodeKind::Element)
    });
    let root_computed = root_element_id
        .and_then(|id| computed.get(id.0 as usize))
        .unwrap_or(&computed[dom.root_id().0 as usize]);
    let page = cascade_page(
        rule_tree,
        page_query,
        PageInheritance::FromRoot(root_computed),
    );

    Ok(CascadeResult {
        computed,
        non_ua_margin_sides,
        authored_writing_modes,
        page,
        counter_styles: rule_tree.counter_styles().clone(),
        page_values,
        pseudo,
    })
}

mod collect;
pub(crate) use collect::*;
mod selector_match;
pub(crate) use selector_match::*;
mod lang;
pub(crate) use lang::*;
mod directionality;
pub(crate) use directionality::*;
mod html_quirks;
pub(crate) use html_quirks::*;
mod custom_property;
pub(crate) use custom_property::*;
mod inherit;
pub(crate) use inherit::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::INITIAL_FONT_SIZE_PX;
    use crate::property::CssColor;
    use crate::property::DisplayValue;
    use crate::property::{
        Border, BorderColor, BorderStyle, CalcLengthPercentage, ContentComponent,
        GridAreaShorthand, GridAutoFlowValue, GridLineValue, GridShorthand, GridTemplateAreasValue,
        GridTemplateTracks, Length, LengthOrAuto, ListStylePosition, ListStyleType, Outline,
        OutlineColor, OutlineStyle, OverflowValue, OverflowXY, PageValue, PositionValue,
        PropertyKey, PropertyValue, Sides, TextDecorationColor, TextDecorationLine,
        TextDecorationShorthand, TextDecorationStyle, TextDecorationThickness, TextShadowColor,
        empty_counter_entries, initial_grid_auto_track_list,
    };
    use crate::resolve::{
        ComputedBorder, ComputedBorderRadius, ComputedBoxShadowItem, ComputedLength,
        ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLineHeight,
        ComputedTabSize, ComputedTextShadow,
    };
    use crate::ruletree::build_rule_tree;
    use crate::test_dom::TestDoc;
    use smol_str::SmolStr;

    fn cascade_doc(css: &str, tag: &str, inline: Option<&str>) -> ComputedValues {
        let mut doc = TestDoc::new();
        if !css.is_empty() {
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, css);
        }
        let e = doc.push_element(0, tag, inline);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        result.computed[e].clone()
    }

    const RED: CssColor = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    const BLUE: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };

    #[test]
    fn empty_dom_root_has_initial() {
        let doc = TestDoc::new();
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        assert_eq!(r.computed[0], ComputedValues::initial());
    }

    #[test]
    fn type_selector_applies_color() {
        let cv = cascade_doc("p { color: red }", "p", None);
        assert_eq!(cv.color, RED);
    }

    fn context_cascade_doc(css: &str, context: MediaContext) -> (TestDoc, usize, CascadeResult) {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, css);
        let element = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let result = cascade_with_media_context(&doc, &tree, &context).expect("cascade Ok");
        (doc, element, result)
    }

    #[test]
    fn media_context_selects_print_and_screen_rules() {
        let css = "@media print { p { color: red } } @media screen { p { color: blue } }";
        let (_, element, print_result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(print_result.computed[element].color, RED);
        let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
        assert_eq!(screen_result.computed[element].color, BLUE);
    }

    #[test]
    fn media_all_matches_both_contexts_and_default_is_print() {
        let css = "@media all { p { color: red } }";
        let (_, element, default_result) = context_cascade_doc(css, MediaContext::default());
        assert_eq!(default_result.computed[element].color, RED);
        let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
        assert_eq!(screen_result.computed[element].color, RED);
    }

    #[test]
    fn media_rules_keep_source_order_against_direct_rules() {
        let css = "p { color: red } @media print { p { color: blue } } p { color: red }";
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(result.computed[element].color, RED);

        let css = "p { color: red } @media print { p { color: blue } }";
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(result.computed[element].color, BLUE);
    }

    #[test]
    fn nested_media_conditions_are_conjoined_and_unknown_wrappers_do_not_leak() {
        let css =
            "@media print { @media all { p { color: red } } @media screen { p { color: blue } } }";
        let (_, element, print_result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(print_result.computed[element].color, RED);
        let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
        assert_ne!(screen_result.computed[element].color, RED);

        let css = "@supports (display: block) { @media print { p { color: red } } }";
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_ne!(result.computed[element].color, RED);
    }

    #[test]
    fn media_comma_list_and_invalid_features_are_safe() {
        let css = "@media print, projection { p { color: red } }";
        let (_, element, print_result) = context_cascade_doc(css, MediaContext::print());
        assert_eq!(print_result.computed[element].color, RED);
        let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
        assert_ne!(screen_result.computed[element].color, RED);

        let css = "@media print and (color) { p { color: red } }";
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_ne!(result.computed[element].color, RED);

        let css = "@media print, { p { color: red } }";
        let (_, element, result) = context_cascade_doc(css, MediaContext::print());
        assert_ne!(result.computed[element].color, RED);
    }

    #[test]
    fn media_condition_is_applied_to_pseudo_elements() {
        let css = "@media screen { p::before { content: \"x\"; color: red } }";
        let (doc, element, print_result) = context_cascade_doc(css, MediaContext::print());
        let element_id = StyleNodeId::new(element as u64);
        assert!(
            !print_result
                .pseudo
                .contains_key(&(element_id, PseudoElem::Before))
        );
        let (_, element, screen_result) = context_cascade_doc(css, MediaContext::screen());
        let element_id = StyleNodeId::new(element as u64);
        assert!(
            screen_result
                .pseudo
                .contains_key(&(element_id, PseudoElem::Before))
        );
        assert_eq!(doc.node_count(), 4);
    }

    #[test]
    fn inline_style_beats_type_selector() {
        let cv = cascade_doc("p { color: red }", "p", Some("color: blue"));
        assert_eq!(cv.color, BLUE);
    }

    // ── class / id / attribute selector matching ──
    //
    // Spec: CSS Selectors Level 4 — class selector
    // <https://www.w3.org/TR/selectors-4/#class-html>, ID selector
    // <https://www.w3.org/TR/selectors-4/#id-selectors>, attribute selector
    // <https://www.w3.org/TR/selectors-4/#attribute-selectors>.
    //
    // `cascade_doc` (above) has no way to set `class`/`id`/arbitrary attrs —
    // it only threads `inline_style` through `push_element` — so these tests
    // build the `TestDoc` directly via `push_element` + `TestDoc::set_attr`.

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
    fn comma_separated_selector_list_uses_max_specificity_across_matches() {
        // `match_complex_selector_list` tracks the *best* (highest)
        // specificity across every selector in a comma-separated list that
        // matches the element — not just the first hit. An element matched
        // by 2+ selectors in the same rule's list must exercise the
        // `Some(prev) => prev.max(spec)` fold, not just its `None => spec`
        // base case.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".a, div.b { font-weight: bold }");
        let p = doc.push_element(0, "div", None);
        doc.set_attr(p, "class", "a b");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
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

    /// Acceptance: quirks-mode 3 状態 × class-selector
    /// case variation の regression matrix. CSS Selectors L4 class-html
    /// (<https://www.w3.org/TR/selectors-4/#class-html>, verbatim): "When
    /// matching against a document which is in quirks mode, class names must
    /// be matched ASCII case-insensitively; class selectors are otherwise
    /// case-sensitive". `NoQuirks`'s different-case branch overlaps
    /// `class_selector_is_case_sensitive` above (kept as its own smaller,
    /// standalone regression test); this table adds `LimitedQuirks` /
    /// `Quirks` plus a same-case control per mode so a regression that stops
    /// matching entirely (rather than over-folding) would also be caught.
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

    /// `has_class_ascii_case_insensitive`'s empty-query guard. A real CSS
    /// class selector can never lexically produce an empty class name, so
    /// `compound_matches` can't reach this branch — hence a direct
    /// call on `TestElementRef`, mirroring
    /// `match_complex_selector_list_rejects_unsupported_component_via_safety_net`
    /// below.
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

    /// Acceptance: quirks-mode 3 状態 × id-selector case
    /// variation の regression matrix — [`class_selector_case_sensitivity_across_quirks_modes`]
    /// の id-selector 版。CSS Selectors L4 id-selectors
    /// (<https://www.w3.org/TR/selectors-4/#id-selectors>, verbatim): "When
    /// matching against a document which is in quirks mode, IDs must be
    /// matched ASCII case-insensitively; ID selectors are otherwise
    /// case-sensitive".
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
    fn id_selector_specificity_beats_class_selector() {
        // CSS Cascading L4 §6.1 sort criterion (b): higher specificity wins.
        // ID (0,1,0,0) > class (0,0,1,0) — both target the same element via
        // separate rules, later source order for the loser to make sure the
        // win is attributable to specificity, not source order.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".foo { color: blue } #bar { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "foo");
        doc.set_attr(div, "id", "bar");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[div].color, RED,
            "id selector must win over class selector"
        );
    }

    /// CSS Cascading and Inheritance Level 4 §6.1 "Cascade Sorting Order"
    /// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>, Specificity step
    /// verbatim: "declarations that do not belong to a style rule (such as
    /// the contents of a style attribute) are considered to have a
    /// specificity higher than any selector." Unlike
    /// [`inline_specificity_exceeds_max_reachable_packed_specificity`],
    /// which pins the `INLINE_SPECIFICITY` constant against a
    /// packed-specificity numeric ceiling, this drives the full
    /// [`cascade()`] pipeline end to end: a real `build_rule_tree` +
    /// `cascade` run against a deliberately high-specificity author
    /// selector (id + 3 classes) matched against an inline `style`
    /// attribute on the same element.
    #[test]
    fn inline_style_beats_maximally_specific_selector_via_cascade() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "#a.b.c.d { color: red }");
        let p = doc.push_element(0, "p", Some("color: blue"));
        doc.set_attr(p, "id", "a");
        doc.set_attr(p, "class", "b c d");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].color, BLUE,
            "inline style must win over #a.b.c.d despite its high selector specificity"
        );
    }

    // --- descendant / child combinator ---
    //
    // CSS Selectors L4 descendant combinator
    // (<https://www.w3.org/TR/selectors-4/#descendant-combinators>, verbatim:
    // "A selector of the form A B represents an element B that is an
    // arbitrary descendant of some ancestor element A") and child combinator
    // (<https://www.w3.org/TR/selectors-4/#child-combinators>, verbatim: "A
    // child combinator describes a childhood relationship between two
    // elements") — see `match_combinator_chain`'s "Spec provenance note" doc
    // section for how this verbatim text was confirmed.

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

    // ── Sibling combinators ──

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
    fn descendant_combinator_selector_specificity_includes_ancestor_compound() {
        // Specificity of a complex selector accounts for every compound in
        // the chain, not just the rightmost one matched against `elem` —
        // `.chapter h2` (0,1,1,0) must beat a plain `h2` (0,0,0,1) rule
        // targeting the same element, CSS Cascading L4 §6.1 sort criterion
        // (b). Both rules are given later source order for the loser so the
        // win is attributable to specificity, not source order.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "h2 { color: blue } .chapter h2 { color: red }");
        let div = doc.push_element(0, "div", None);
        doc.set_attr(div, "class", "chapter");
        let h2 = doc.push_element(div, "h2", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h2].color, RED,
            ".chapter h2 must win over plain h2 on specificity"
        );
    }

    #[test]
    fn descendant_combinator_after_backtracking_past_a_sibling_subtree() {
        // Regression for the `ancestor_path` depth-truncation technique in
        // `collect_cascaded`: after the DFS finishes a `.wrap` subtree and
        // returns to process a *sibling* `.target`, `.target`'s own
        // ancestor_path must not still contain anything pushed while
        // visiting the sibling's subtree. `.wrap p` must not leak onto
        // `.target`'s child.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ".wrap p { color: red }");
        let root = doc.push_element(0, "div", None);
        let wrap = doc.push_element(root, "div", None);
        doc.set_attr(wrap, "class", "wrap");
        let wrapped_p = doc.push_element(wrap, "p", None);
        let target = doc.push_element(root, "div", None);
        doc.set_attr(target, "class", "target"); // sibling of `wrap`, NOT `.wrap`
        let p_under_target = doc.push_element(target, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // Positive control: without this, a matcher
        // that never matches `.wrap p` anywhere would also make the
        // negative assertion below pass vacuously — assert the rule
        // actually fired where it should have before asserting it didn't
        // leak where it shouldn't.
        assert_eq!(
            r.computed[wrapped_p].color, RED,
            ".wrap p must match its own direct target under .wrap"
        );
        assert_eq!(
            r.computed[p_under_target].color,
            ComputedValues::initial().color,
            "p under .target must not match .wrap p just because a sibling .wrap subtree was visited earlier"
        );
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

    /// Builds a `depth`-deep uniformly-tagged `<div>` chain (root's first
    /// child down to the deepest, returned as `(doc, deepest_id,
    /// ancestors_of_deepest)`) and a `div`-only descendant-combinator
    /// selector with `compounds` compounds — the shape
    /// [`match_combinator_chain`]'s "Memoization" doc empirically measures.
    /// `compounds == depth` is the "exact fit" case (every compound has an
    /// ancestor slot; matches almost immediately via the greedy
    /// nearest-candidate path). `compounds == depth + 1` is the minimal
    /// *unsatisfiable* case (one ancestor short of what the selector
    /// needs) — this is the one that drove `2^(depth-1)`
    /// [`match_from_element`] calls before this test's fix (see that doc
    /// for the derivation and measured pre-fix timings at smaller depths).
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

    /// This is the DoS this branch fixes: an attacker-controlled markup
    /// depth matched against an attacker-controlled (or even entirely
    /// ordinary, e.g. deeply nested `<div>`s) stylesheet could drive
    /// [`Combinator::Descendant`] backtracking to `2^(depth-1)`
    /// [`match_from_element`] calls — CPU exhaustion with no crash, no
    /// stack limit involved (distinct from `deep_child_combinator_chain_
    /// small_stack_no_overflow`'s native-stack-overflow concern above,
    /// which the explicit-`Vec`-stack rewrite already fixed independently
    /// of this test). Depth 200 with the pre-fix (unmemoized) backtracking
    /// search would need on the order of `2^198` `match_from_element`
    /// calls to conclude "no match" — not slow, simply never finishing on
    /// any real machine (`match_combinator_chain`'s "Memoization" doc has
    /// the exact recurrence, confirmed empirically at smaller depths to
    /// match `2^(k-1)` scaling).
    ///
    /// Post-fix (memoized), the same 200-deep unsatisfiable case is bounded
    /// polynomially in the ancestor/compound counts and completes in a
    /// small fraction of a second — asserted here with a generous 5s
    /// bound (not a tight one) so this doesn't flake under a loaded gate
    /// run; the point is "no longer exponential", not a precise benchmark
    /// (formal benchmarking is out of scope for this change).
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

    /// Sibling case of the test directly above — same pathological shape
    /// ([`match_combinator_chain`]'s "Memoization" doc notes the memo
    /// bounds `Combinator::LaterSibling` backtracking identically, as a
    /// consequence of being keyed generically rather than per-combinator).
    /// `* ~ * ~ ... ~ *` (general-sibling combinators) against a run of
    /// uniformly-matching siblings, with one compound more than there are
    /// preceding siblings to satisfy them — the minimal unsatisfiable case
    /// on the sibling axis instead of the ancestor axis.
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

    // ── `:lang()` / `:dir()` ──

    /// Acceptance: `:lang(ja) { font-family: serif-ja }` applies to an
    /// element that has `lang="ja"` **directly on itself**.
    #[test]
    fn lang_matches_own_lang_attribute() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "lang", "ja");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "serif-ja");
    }

    /// Acceptance (the actual "top gap" this closes): `:lang(ja)`
    /// must apply to an element with **no lang attribute of its own**, whose
    /// language is inherited from an ancestor (`<html lang="ja">` in the
    /// example below) — the ancestor-walk this reuses from the
    /// descendant/child combinator matching.
    #[test]
    fn lang_matches_via_ancestor_inherited_language() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "ja");
        let body = doc.push_element(html, "body", None);
        let p = doc.push_element(body, "p", None); // no lang of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_family[0].to_string(),
            "serif-ja",
            ":lang(ja) must match an element with no lang attribute of its \
             own when an ancestor carries lang=\"ja\""
        );
    }

    /// Negative counterpart of the two tests above: neither the element nor
    /// any ancestor carries a `lang` attribute at all, so the content
    /// language resolves to the empty string (`effective_language`'s
    /// exhausted-ancestor-chain terminal case) — a non-empty range like
    /// `ja` must not match that (`subtags_match`'s first-subtag comparison
    /// step, CSS Selectors L4 §7.2 — see `effective_language`'s doc). This
    /// is `:lang(ja)`-range-specific: contrast with
    /// `lang_empty_string_range_matches_when_no_lang_anywhere_in_ancestor_chain`,
    /// where the same no-lang-anywhere element correctly *does* match the
    /// literal empty-string range `:lang("")`.
    #[test]
    fn lang_ja_range_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) { font-family: serif-ja }");
        let html = doc.push_element(0, "html", None); // no lang
        let body = doc.push_element(html, "body", None); // no lang
        let p = doc.push_element(body, "p", None); // no lang

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_family,
            ComputedValues::initial().font_family,
            ":lang(ja) must not match when no element in the chain has a lang attribute"
        );
    }

    /// A closer ancestor's `lang` must win over a farther one — regression
    /// check for `effective_language`'s "own first, then nearest ancestor"
    /// (not "any ancestor") walk order.
    #[test]
    fn lang_prefers_nearest_ancestor_lang_over_farther_one() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(en) { font-family: serif-en }");
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "ja");
        let section = doc.push_element(html, "section", None);
        doc.set_attr(section, "lang", "en");
        let p = doc.push_element(section, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "serif-en");
    }

    /// HTML LS §3.2.6.2's own-`lang`
    /// step is scoped to "an HTML element or an element in the SVG
    /// namespace" (quoted in full on `effective_language`'s doc) — a MathML
    /// element's own `lang` attribute must NOT be consulted, unlike SVG's
    /// (`own_html_or_svg_lang_attribute`'s 2-element allowlist). Concrete
    /// spec example: `<html lang="en"><math lang="ja">…`
    /// — `<math>`'s own `lang="ja"` is ignored, so its effective language
    /// falls through to the `<html>` ancestor's `"en"`.
    #[test]
    fn lang_on_mathml_namespace_element_is_ignored_falls_through_to_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":lang(en) { font-family: en-font } :lang(ja) { font-family: ja-font }",
        );
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "en");
        let math = doc.push_element_with_namespace(
            html,
            "math",
            "http://www.w3.org/1998/Math/MathML",
            &[("lang", "ja")],
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[math].font_family[0].to_string(),
            "en-font",
            "MathML element's own lang attribute must be ignored per HTML \
             LS §3.2.6.2 (HTML/SVG only), falling through to the <html \
             lang=\"en\"> ancestor — :lang(en) must match, :lang(ja) must not"
        );
    }

    /// `:lang()`/`:dir()` on a non-rightmost compound (low severity, optional) —
    /// the left side of a
    /// combinator, matched via `match_from_ancestor` rather than the
    /// rightmost-element entry point in `match_complex_selector_list` — was
    /// reviewed by hand and judged structurally correct but untested.
    /// `:lang(ja) p` matches a `<p>` whose *grandparent* `<html>` (not its
    /// immediate parent `<body>`) carries `lang="ja"`, exercising both
    /// `match_from_ancestor`'s own `compound_matches` call (the `:lang(ja)`
    /// compound tested against the `<html>` ancestor candidate) and that
    /// call's `ancestors` slice (the further-out ancestors above `<html>` —
    /// here empty, but exercised as a real argument rather than `&[]`).
    #[test]
    fn lang_pseudo_class_matches_on_non_rightmost_compound_via_descendant_combinator() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(ja) p { font-family: serif-ja }");
        let html = doc.push_element(0, "html", None);
        doc.set_attr(html, "lang", "ja");
        let body = doc.push_element(html, "body", None);
        let p = doc.push_element(body, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_family[0].to_string(),
            "serif-ja",
            ":lang(ja) p must match <p> whose ancestor <html> (not <p> \
             itself) satisfies :lang(ja), via the ancestor-compound path \
             (match_from_ancestor) rather than the rightmost-element path"
        );
    }

    /// RFC 4647 §3.3.2 extended filtering: `:lang(en)` must match a more
    /// specific tagged element (`lang="en-US"`) — subtag prefix matching,
    /// not exact string equality.
    #[test]
    fn lang_range_matches_more_specific_tag_via_extended_filtering() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(en) { font-family: serif-en }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "lang", "en-US");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "serif-en");
    }

    /// `:dir(ltr)` matches an element with an explicit `dir="ltr"` attribute.
    #[test]
    fn dir_matches_explicit_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "dir", "ltr");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
    }

    /// `:dir(rtl)` matches an element with an explicit `dir="rtl"` attribute,
    /// and — the actual differentiator from `[dir=rtl]` — a descendant with
    /// no `dir` of its own inherits that directionality
    /// (`resolve_directionality`'s ancestor walk).
    #[test]
    fn dir_matches_explicit_rtl_and_inherits_to_descendant_without_own_dir() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(rtl) { font-family: rtl-font }");
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None); // no dir of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[article].font_family[0].to_string(),
            "rtl-font",
            ":dir(rtl) must match the element with the explicit attribute"
        );
        assert_eq!(
            r.computed[span].font_family[0].to_string(),
            "rtl-font",
            ":dir(rtl) must match a descendant with no dir attribute of its \
             own, inheriting from its dir=\"rtl\" ancestor"
        );
    }

    /// Mirror of the `rtl`-ancestor case above with an `ltr` ancestor
    /// instead — [`own_explicit_direction`]'s `Ltr` arm is only reachable
    /// via [`resolve_directionality`]'s ancestor-walk loop (an element's
    /// *own* `ltr`/`rtl` state is read via [`own_dir_attribute_state`]
    /// directly), so this is the only test that exercises it; the `rtl`
    /// sibling test above only exercises the loop's `Rtl` arm. An `rtl`
    /// *grandparent* wraps the `ltr` parent specifically so this test can
    /// fail: without it, a broken `Ltr` arm would still leave `span`
    /// resolving to `'ltr'` via the loop-exhausted default, indistinguishable
    /// from the arm working — with the `rtl` grandparent present, a broken
    /// `Ltr` arm instead lets the walk continue past the `ltr` parent to the
    /// `rtl` grandparent, flipping `span` to `rtl`.
    #[test]
    fn dir_matches_explicit_ltr_and_inherits_to_descendant_without_own_dir() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let section = doc.push_element(0, "section", None);
        doc.set_attr(section, "dir", "rtl"); // grandparent, see doc above
        let article = doc.push_element(section, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None); // no dir of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            ":dir(ltr) must match a descendant with no dir attribute of its \
             own, inheriting from its dir=\"ltr\" ancestor rather than \
             continuing past it to the dir=\"rtl\" grandparent"
        );
    }

    /// Default directionality (no `dir` attribute anywhere in the chain) is
    /// `ltr` — HTML LS "parent directionality" §3.2.6.4's root fallback
    /// (`resolve_directionality`'s doc). `:dir(rtl)` must not match; `:dir(ltr)`
    /// must.
    #[test]
    fn dir_defaults_to_ltr_when_no_dir_attribute_anywhere() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { font-family: rtl-font } :dir(ltr) { font-family: ltr-font }",
        );
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
    }

    /// RFC 4647 §3.3.2 extended filtering, direct check (bypassing the cascade
    /// pipeline) of the examples the RFC itself gives for range `de-*-DE` /
    /// its synonym `de-DE` — [`language_range_matches`]'s doc quotes the
    /// algorithm this exercises.
    #[test]
    fn language_range_matches_rfc4647_de_de_examples() {
        // Matches (RFC 4647 §3.3.2, "matches all of the following tags"):
        for tag in [
            "de-DE",
            "de-de",
            "de-Latn-DE",
            "de-Latf-DE",
            "de-DE-x-goethe",
            "de-Latn-DE-1996",
            "de-Deva-DE",
        ] {
            assert!(
                language_range_matches("de-*-DE", tag),
                "de-*-DE must match {tag}"
            );
            assert!(
                language_range_matches("de-DE", tag),
                "de-DE (synonym) must match {tag}"
            );
        }
        // Does not match (RFC 4647 §3.3.2, "does not match any of the
        // following tags"):
        assert!(
            !language_range_matches("de-*-DE", "de"),
            "missing 'DE' subtag entirely"
        );
        assert!(
            !language_range_matches("de-*-DE", "de-x-DE"),
            "singleton 'x' occurs before 'DE', blocking the skip"
        );
        assert!(
            !language_range_matches("de-*-DE", "de-Deva"),
            "'Deva' present but 'DE' subtag never appears"
        );
    }

    #[test]
    fn language_range_matches_is_ascii_case_insensitive() {
        assert!(language_range_matches("EN", "en-us"));
        assert!(language_range_matches("en", "EN-US"));
    }

    /// Selectors L4 §7.2's own example, quoted on [`language_range_matches`]'s
    /// doc: "`:lang(åå)` would not match, because it contain[s] non-ASCII
    /// characters so is ill-formed." Checked against several `lang` values,
    /// including `åå` itself, to check "never matches any element, regardless
    /// of its lang attribute value" (not merely "doesn't happen to match
    /// this particular content language").
    #[test]
    fn language_range_matches_rejects_non_ascii_ill_formed_range() {
        for content_language in ["en", "en-US", "åå", ""] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                !language_range_matches("åå", content_language),
                "ill-formed range :lang(åå) must never match content \
                 language {content_language:?}"
            );
        }
    }

    /// The other half of Selectors L4 §7.2's paired example (same fetch as
    /// above): "`:lang(qq)` could match, even though qq is not a
    /// registered language code." `qq` is well-formed (2 ASCII alpha
    /// characters) but not IANA-registered — pins that this
    /// implementation's bar is well-formedness (RFC 5646 §2.1's ABNF), not
    /// full BCP47 validity (well-formed *and* every subtag registered),
    /// matching the spec's own worked example rather than a stricter
    /// registry check a future change might otherwise "helpfully" add.
    #[test]
    fn language_range_matches_well_formed_unregistered_subtag_can_match() {
        assert!(language_range_matches("qq", "qq"));
        assert!(language_range_matches("qq", "qq-Latn"));
    }

    /// Well-formedness rejects an overlong subtag (RFC 5646 §2.1: "All
    /// subtags have a maximum length of eight characters") on either side —
    /// covers [`is_well_formed_language_tag`]'s later-subtag branch, which
    /// the non-ASCII range test above does not reach (that one fails on the
    /// first-subtag check instead).
    #[test]
    fn language_range_matches_rejects_overlong_subtag_on_either_side() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !language_range_matches("en", "en-abcdefghi"),
            "9-character subtag exceeds RFC 5646's 8-character maximum"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !language_range_matches("en-abcdefghi", "en-US"),
            "same overlong-subtag rejection, this time on the range side"
        );
    }

    /// A double hyphen splits into an empty subtag, which fits no BCP47
    /// subtag production regardless of position.
    #[test]
    fn language_range_matches_rejects_empty_subtag_from_double_hyphen() {
        assert!(!language_range_matches("en", "en--US"));
    }

    /// RFC 5646 §4.5 step 3 canonicalization (deprecated primary language
    /// subtag -> registry `Preferred-Value`) is a genuine false-negative
    /// source without it: `iw` is the deprecated form of `he` (IANA
    /// Language Subtag Registry, `Type: language`, `Subtag: iw`,
    /// `Deprecated`, `Preferred-Value: he`), so a conformant UA must match
    /// `:lang(he)` against `lang="iw"` **and** `:lang(iw)` against
    /// `lang="he"` — canonicalization runs on both the range and the
    /// content language before comparison, so the match is symmetric.
    #[test]
    fn language_range_matches_canonicalizes_deprecated_primary_language_subtag() {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            language_range_matches("he", "iw"),
            ":lang(he) must match the deprecated-but-still-in-the-wild lang=\"iw\""
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            language_range_matches("iw", "he"),
            ":lang(iw) must also match lang=\"he\" — the :lang() argument \
             itself is canonicalized too, per Selectors L4 §7.2"
        );
    }

    /// Regression coverage for the rest of [`DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS`]
    /// beyond the `iw`/`he` pair pinned above, including `bh` -> `bih` — the
    /// one entry where the `Preferred-Value` changes the subtag's *length*
    /// (2 letters -> 3), which is exactly where an off-by-one in the
    /// canonicalize-then-compare ordering would surface.
    #[test]
    fn language_range_matches_canonicalizes_remaining_deprecated_subtag_table_entries() {
        for (deprecated, preferred) in [
            ("bh", "bih"),
            ("in", "id"),
            ("ji", "yi"),
            ("jw", "jv"),
            ("mo", "ro"),
        ] {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                language_range_matches(preferred, deprecated),
                ":lang({preferred}) must match lang=\"{deprecated}\""
            );
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert!(
                language_range_matches(deprecated, preferred),
                ":lang({deprecated}) must match lang=\"{preferred}\""
            );
        }
    }

    /// Regression check: a well-formed, multi-subtag tag/range pair that does
    /// NOT involve any deprecated subtag must still match exactly as before
    /// this well-formedness/canonicalization pass — exercises a `script`
    /// subtag (`Hans`) through the new validation pipeline, distinct from
    /// the plain-primary-language pairs the pre-existing tests use.
    #[test]
    fn language_range_matches_well_formed_non_canonicalized_tag_still_matches() {
        assert!(language_range_matches("zh-Hans", "zh-Hans-CN"));
        assert!(!language_range_matches("zh-Hant", "zh-Hans-CN"));
    }

    /// RFC 5646 §2.1's `privateuse = "x" 1*("-" (1*8alphanum))` is a live,
    /// still-usable `Language-Tag` alternative on its own (distinct from the
    /// frozen `grandfathered` list) — a bare top-level `privateuse` tag like
    /// `x-foo` must be well-formed and must match itself.
    #[test]
    fn language_range_matches_privateuse_tag_matches_itself() {
        assert!(language_range_matches("x-foo", "x-foo"));
        // The ABNF's `1*` requires at least one value subtag after the `x`
        // introducer — a bare `x` alone is ill-formed and must not match.
        assert!(!language_range_matches("x", "x"));
    }

    /// `dir="auto"` resolves via the element's own contained-text scan
    /// (HTML LS §3.2.6.4's "auto directionality" — see
    /// [`resolve_directionality`]'s doc), *not* by falling through to the
    /// nearest ancestor's directionality — even when that ancestor has an
    /// explicit `dir` of its own. `background-color` (not inherited, unlike
    /// `font-family`) proves the span itself resolved `ltr` rather than
    /// merely failing to match `:dir(rtl)` while still visually inheriting
    /// the ancestor's rtl-associated styling.
    #[test]
    fn dir_auto_scans_own_text_ltr_first_strong_overrides_rtl_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // "Hello" is wrapped in an ordinary (non-excluded) nested <b>, not a
        // direct text child of `span` — exercises the recursive descent
        // into an un-excluded element subtree in
        // `auto_text_scan_subtree`, not just the direct-text-child case.
        let bold = doc.push_element(span, "b", None);
        doc.push_text(bold, "Hello"); // first strong character 'H' is type L

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[article].background_color, RED,
            ":dir(rtl) must still match the article's own explicit dir=\"rtl\""
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "dir=\"auto\" must resolve via the element's own text scan (first \
             strong character 'H' is type L → ltr), not by inheriting the \
             rtl ancestor's directionality"
        );
    }

    /// Mirror of the LTR-first-strong case above, with the ancestor now
    /// `ltr` and the `dir=\"auto\"` element's own text starting with an
    /// Arabic (bidirectional type AL) character — resolves `rtl` despite
    /// the `ltr` ancestor.
    #[test]
    fn dir_auto_with_arabic_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}"); // "السلام"

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is Arabic (type AL) → rtl"
        );
    }

    /// Same shape as the Arabic case, but with a Hebrew (bidirectional type
    /// R, the non-Arabic right-to-left branch of
    /// [`strong_bidi_type`]) first strong character.
    #[test]
    fn dir_auto_with_hebrew_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{05E9}\u{05DC}\u{05D5}\u{05DD}"); // "שלום"

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is Hebrew (type R) → rtl"
        );
    }

    /// [`strong_bidi_type`]'s `R_RANGES` table, Hebrew Presentation Forms
    /// case (U+FB1D..U+FB4F, distinct from the main Hebrew block
    /// U+0590..U+05FF the test above exercises) — a separate
    /// `DerivedBidiClass.txt` `@missing` range and therefore a separate
    /// table entry that needs its own coverage.
    #[test]
    fn dir_auto_with_hebrew_presentation_forms_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{FB1D}"); // HEBREW LETTER YOD WITH HIRIQ

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "Hebrew presentation forms character is type R → rtl"
        );
    }

    /// U+200E LEFT-TO-RIGHT MARK (`Cf`, General Punctuation block) is one of
    /// [`strong_bidi_type`]'s explicit top-of-function exceptions (type `L`,
    /// checked before either range table) — its own doc's history matters
    /// here: this exact case (U+200E as text's first strong character,
    /// followed by unrelated Arabic text) previously resolved wrongly
    /// (`rtl` instead of the spec-correct `ltr`) before that check existed,
    /// because `is_alphabetic('\u{200E}') == false` fell through to `None`.
    #[test]
    fn dir_auto_with_leading_ltr_mark_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+200E LEFT-TO-RIGHT MARK, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{200E}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "first strong character is U+200E (type L) → ltr, despite \
             unrelated Arabic text right after it"
        );
    }

    /// U+200F RIGHT-TO-LEFT MARK (`Cf`, General Punctuation block) is
    /// [`strong_bidi_type`]'s other explicit top-of-function exception
    /// (type `R`) — mirrors the U+200E case above with the roles reversed.
    #[test]
    fn dir_auto_with_leading_rtl_mark_before_latin_text_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+200F RIGHT-TO-LEFT MARK, then unrelated Latin (type L) text.
        doc.push_text(span, "\u{200F}Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is U+200F (type R) → rtl, despite \
             unrelated Latin text right after it"
        );
    }

    /// [`strong_bidi_type`]'s `NON_STRONG_ALPHABETIC_RANGES` table, `NSM`
    /// case: U+0941 DEVANAGARI VOWEL SIGN U is `Alphabetic=Yes`
    /// (`char::is_alphabetic() == true`) but its real `Bidi_Class` is
    /// `NSM` (non-strong), not `L`. Without the exclusion table this
    /// leading combining mark would be misclassified as strong `L` and
    /// stop the scan immediately, resolving `ltr`; the correct scan skips
    /// it and continues to the following Hebrew (type `R`) text, resolving
    /// `rtl`.
    #[test]
    fn dir_auto_skips_non_strong_alphabetic_combining_mark_before_hebrew_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0941 DEVANAGARI VOWEL SIGN U, then "שלום" (Hebrew, type R).
        doc.push_text(span, "\u{0941}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "leading non-strong combining mark must be skipped (not \
             misclassified as strong L) so the scan reaches the Hebrew \
             text and resolves rtl"
        );
    }

    /// Same table, `ON` case (a spacing modifier letter, not a combining
    /// mark): U+02C6 MODIFIER LETTER CIRCUMFLEX ACCENT is also
    /// `Alphabetic=Yes` but its real `Bidi_Class` is `ON`, not `L`.
    #[test]
    fn dir_auto_skips_non_strong_alphabetic_modifier_letter_before_arabic_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+02C6 MODIFIER LETTER CIRCUMFLEX ACCENT, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{02C6}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "leading non-strong alphabetic modifier letter must be skipped \
             so the scan reaches the Arabic text and resolves rtl"
        );
    }

    /// [`strong_bidi_type`]'s `NON_STRONG_WITHIN_AL_R_RANGES` table: U+0664
    /// ARABIC-INDIC DIGIT FOUR falls inside `AL_RANGES`'s main Arabic span
    /// (U+0600..U+07BF) but its real `Bidi_Class` is `AN` (Arabic Number,
    /// weak), not `AL`. Without this exclusion table the range-table
    /// lookup alone would over-classify it as strong `AL` and stop the
    /// scan there, wrongly resolving `rtl` — not merely stopping one code
    /// point early, but resolving the *opposite* direction from the
    /// following Latin (type `L`) text's true first-strong result.
    #[test]
    fn dir_auto_skips_arabic_indic_digit_before_latin_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0664 ARABIC-INDIC DIGIT FOUR, then Latin "H".
        doc.push_text(span, "\u{0664}H");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Arabic-Indic digit (weak AN, not strong AL) must be \
             skipped so the scan reaches the Latin text and resolves ltr"
        );
    }

    /// [`strong_bidi_type`]'s `STRONG_L_NON_ALPHABETIC_RANGES` table, `Nd`
    /// case: U+0966 DEVANAGARI DIGIT ZERO has real `Bidi_Class=L` but
    /// `is_alphabetic() == false` (it is a decimal digit, not a letter).
    /// Without this table the digit would fall through to `None`
    /// (non-strong) and the scan would skip past it, wrongly resolving via
    /// the later Arabic (type `AL`) text instead — the opposite direction
    /// from the spec, since the digit is the text's true first strong
    /// (`L`) character.
    #[test]
    fn dir_auto_with_devanagari_digit_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0966 DEVANAGARI DIGIT ZERO, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{0966}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Devanagari digit (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall through \
             to the later Arabic text"
        );
    }

    /// Same table, `Po` case: U+055A ARMENIAN APOSTROPHE has real
    /// `Bidi_Class=L` but `is_alphabetic() == false` (it is punctuation,
    /// not a letter). Mirrors the Devanagari-digit test above with a
    /// Hebrew (type `R`) character standing in for the later strong
    /// character that must NOT be reached.
    #[test]
    fn dir_auto_with_armenian_punctuation_before_hebrew_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+055A ARMENIAN APOSTROPHE, then "שלום" (Hebrew, type R).
        doc.push_text(span, "\u{055A}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Armenian punctuation (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall \
             through to the later Hebrew text"
        );
    }

    /// Same table, `Mc` case: U+1B44 BALINESE ADEG ADEG is a virama (a
    /// spacing combining mark that suppresses the inherent vowel of the
    /// preceding consonant) with real `Bidi_Class=L`, but Unicode does not
    /// consider a virama `Alphabetic` the way it considers an ordinary
    /// vowel sign alphabetic, so `is_alphabetic() == false` here too.
    /// Mirrors the two tests above with Arabic (type `AL`) standing in for
    /// the later strong character that must NOT be reached.
    #[test]
    fn dir_auto_with_balinese_virama_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+1B44 BALINESE ADEG ADEG, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{1B44}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Balinese virama (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall \
             through to the later Arabic text"
        );
    }

    /// Same table, `Mc` case, Sharada Vowel Signs Supplement (new in Unicode
    /// 17.0.0): U+11B61 SHARADA VOWEL SIGN OOE has real `Bidi_Class=L` and
    /// is `Alphabetic=Yes` per UCD 17.0.0, but this workspace's pinned rustc
    /// 1.89.0 predates that Unicode version, so `char::is_alphabetic()`
    /// still returns `false` for it. Without its own table entry (as
    /// opposed to the Balinese virama above, which is never `Alphabetic`
    /// under any Unicode version) the vowel sign would fall through to
    /// `None` exactly the way the `is_alphabetic()`-fallback gap this table
    /// exists to close would predict.
    #[test]
    fn dir_auto_with_sharada_vowel_sign_ooe_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+11B61 SHARADA VOWEL SIGN OOE, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{11B61}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Sharada vowel sign (strong L, despite not being \
             is_alphabetic() under this workspace's pinned rustc) must \
             resolve ltr on its own, not fall through to the later Arabic \
             text"
        );
    }

    #[test]
    fn strong_bidi_range_tables_preserve_classification_invariants() {
        fn overlaps(a: (u32, u32), b: (u32, u32)) -> bool {
            a.0 <= b.1 && b.0 <= a.1
        }

        fn assert_table_is_internally_disjoint(name: &str, table: &[(u32, u32)]) {
            for (index, &left) in table.iter().enumerate() {
                assert!(left.0 <= left.1, "{name}[{index}] has an inverted range");
                for (other_index, &right) in table.iter().enumerate().skip(index + 1) {
                    // cov:ignore: the failure-message branch of this `assert!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert!(
                        !overlaps(left, right),
                        "{name}[{index}] {left:#x?} overlaps {name}[{other_index}] {right:#x?}"
                    );
                }
            }
        }

        fn assert_tables_are_disjoint(
            left_name: &str,
            left: &[(u32, u32)],
            right_name: &str,
            right: &[(u32, u32)],
        ) {
            for (left_index, &left_range) in left.iter().enumerate() {
                for (right_index, &right_range) in right.iter().enumerate() {
                    // cov:ignore: the failure-message branch of this `assert!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert!(
                        !overlaps(left_range, right_range),
                        "{left_name}[{left_index}] {left_range:#x?} overlaps \
                         {right_name}[{right_index}] {right_range:#x?}"
                    );
                }
            }
        }

        let tables = [
            ("AL_RANGES", AL_RANGES),
            ("R_RANGES", R_RANGES),
            (
                "NON_STRONG_WITHIN_AL_R_RANGES",
                NON_STRONG_WITHIN_AL_R_RANGES,
            ),
            ("NON_STRONG_ALPHABETIC_RANGES", NON_STRONG_ALPHABETIC_RANGES),
            (
                "STRONG_L_NON_ALPHABETIC_RANGES",
                STRONG_L_NON_ALPHABETIC_RANGES,
            ),
        ];

        for &(name, table) in &tables {
            assert_table_is_internally_disjoint(name, table);
        }
        assert_tables_are_disjoint("AL_RANGES", AL_RANGES, "R_RANGES", R_RANGES);
        assert_tables_are_disjoint(
            "AL_RANGES",
            AL_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "R_RANGES",
            R_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "AL_RANGES",
            AL_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "R_RANGES",
            R_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );

        for &(lo, hi) in NON_STRONG_WITHIN_AL_R_RANGES {
            // cov:ignore: the failure-message branch of this `assert!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert!(
                AL_RANGES
                    .iter()
                    .chain(R_RANGES)
                    .any(|&outer| outer.0 <= lo && hi <= outer.1),
                "non-strong exception {lo:#x}..={hi:#x} is outside AL/R defaults"
            );
        }

        for &(lo, hi) in NON_STRONG_ALPHABETIC_RANGES {
            for cp in lo..=hi {
                let character = char::from_u32(cp).expect("range is a Unicode scalar");
                let resolved = strong_bidi_type(character);
                if character.is_alphabetic() {
                    // cov:ignore: the failure-message branch of this `assert_eq!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert_eq!(
                        resolved, None,
                        "alphabetic NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must be excluded from L"
                    );
                }
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    resolved != Some(StrongBidiType::L),
                    "NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must never resolve as L"
                );
            }
        }
        for &(lo, hi) in STRONG_L_NON_ALPHABETIC_RANGES {
            for cp in lo..=hi {
                let character = char::from_u32(cp).expect("range is a Unicode scalar");
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    !character.is_alphabetic(),
                    "STRONG_L_NON_ALPHABETIC_RANGES contains alphabetic U+{cp:04X}"
                );
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(character),
                    Some(StrongBidiType::L),
                    "STRONG_L_NON_ALPHABETIC_RANGES entry U+{cp:04X} must resolve as L"
                );
            }
        }

        for &(lo, hi) in AL_RANGES {
            for cp in lo..=hi {
                let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                    .iter()
                    .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
                {
                    None
                } else {
                    Some(StrongBidiType::Al)
                };
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                    expected,
                    "AL_RANGES classification changed for U+{cp:04X}"
                );
            }
        }
        for &(lo, hi) in R_RANGES {
            for cp in lo..=hi {
                let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                    .iter()
                    .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
                {
                    None
                } else {
                    Some(StrongBidiType::R)
                };
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                    expected,
                    "R_RANGES classification changed for U+{cp:04X}"
                );
            }
        }
    }

    /// No strongly-directional character anywhere in the `dir=\"auto\"`
    /// element's contained text (digits, spaces, and punctuation are all
    /// non-strong) — HTML LS's own `Auto`-state fallback is `'ltr'`
    /// unconditionally, never the ancestor's directionality (contrast with
    /// `dir_undefined_still_falls_through_to_ancestor_via_background_color`
    /// below, where an *absent* `dir` attribute does fall through to this
    /// same `rtl` ancestor).
    #[test]
    fn dir_auto_with_no_strong_directional_text_falls_back_to_ltr_despite_rtl_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "123 456!");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "no strong L/AL/R character anywhere → 'ltr' fallback, not the \
             rtl ancestor's directionality"
        );
    }

    /// A missing `dir` attribute (HTML LS "Undefined" state) is a
    /// *different* case from `dir=\"auto\"` above and must keep falling
    /// through to the ancestor's directionality — `background-color`
    /// isolates the element's own `:dir()` match from ordinary (unrelated)
    /// property inheritance the same way the tests above do.
    #[test]
    fn dir_undefined_still_falls_through_to_ancestor_via_background_color() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None); // no dir attribute at all

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "missing dir attribute must still fall through to the ancestor's \
             directionality"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s `dir`-attribute-state branch: a
    /// descendant with its own non-`Undefined` `dir` (here `rtl`) resolves
    /// its *own* directionality independently and must not leak its text
    /// into an ancestor's auto-directionality scan. Tree order would visit
    /// the nested Arabic text before the outer element's own trailing
    /// English text — if the exclusion were missing, the scan would
    /// (wrongly) stop at the Arabic text and resolve `rtl`.
    #[test]
    fn auto_directionality_skips_descendant_with_own_dir_attribute() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let inner = doc.push_element(outer, "span", None);
        doc.set_attr(inner, "dir", "rtl");
        doc.push_text(inner, "\u{0627}"); // Arabic alef — must be skipped
        doc.push_text(outer, "Hello"); // outer's own trailing text

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "the dir=\"rtl\" descendant's text must be excluded from the \
             outer element's own auto-directionality scan"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s tag-name branch, `bdi` case,
    /// isolated from the `dir`-attribute branch above (this `bdi` element
    /// carries no `dir` attribute of its own — it is excluded purely by
    /// being a `bdi` element, per HTML LS's exclusion list).
    #[test]
    fn auto_directionality_skips_bdi_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let bdi = doc.push_element(outer, "bdi", None); // no dir attribute
        doc.push_text(bdi, "\u{0627}"); // Arabic alef — must be skipped
        doc.push_text(outer, "Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "a bdi descendant's text must be excluded from the outer \
             element's own auto-directionality scan, regardless of its own \
             dir state"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s tag-name branch, `script`/`style`
    /// case — stylesheet/script text must never be treated as page content
    /// for directionality purposes.
    #[test]
    fn auto_directionality_skips_script_and_style_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let nested_script = doc.push_element(outer, "script", None);
        doc.push_text(nested_script, "\u{0627}");
        let nested_style = doc.push_element(outer, "style", None);
        doc.push_text(nested_style, "\u{0627}");
        doc.push_text(outer, "Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "script/style descendant text must be excluded from the outer \
             element's own auto-directionality scan"
        );
    }

    /// [`excluded_from_auto_text_scan`]'s namespace-gate branch: a
    /// foreign-namespace (SVG) descendant is not one of the 4 HTML-only
    /// excluded tag names (it cannot be, per HTML LS's own dfns), so its
    /// text must still be scanned like any ordinary descendant's — unlike
    /// the `bdi`/`script`/`style`/`textarea` cases above, this is a
    /// "must **not** be excluded" regression test.
    #[test]
    fn auto_directionality_scans_into_foreign_namespace_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let svg_text =
            doc.push_element_with_namespace(outer, "text", "http://www.w3.org/2000/svg", &[]);
        doc.push_text(svg_text, "\u{0627}"); // Arabic alef

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, RED,
            "a foreign-namespace descendant's text must still be scanned, \
             not excluded"
        );
    }

    /// Comment nodes are neither "text node" nor "element node" and must
    /// not contribute to the auto-directionality scan (mirrors
    /// [`matches_empty`] / [`is_substantial_node`]'s treatment of the same
    /// node kind). Deliberately has **no** other text anywhere in the
    /// element, so a wrong implementation that read the comment's own text
    /// content would resolve `rtl`, not merely fail to resolve `ltr` for an
    /// unrelated reason.
    #[test]
    fn auto_directionality_ignores_comment_node_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        doc.push_comment(outer, "\u{0627}"); // Arabic alef inside a comment

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "a comment node's text must never be scanned; with no other \
             text present this must resolve via the 'ltr' no-strong-\
             character fallback"
        );
    }

    /// HTML LS §3.2.6.4 (quoted on `own_explicit_direction`'s doc): "the
    /// `dir` attribute is only defined for HTML elements \[...\] elements
    /// from other namespaces always end up using the parent
    /// directionality." A `dir="rtl"` attribute on a foreign-namespace
    /// (SVG) element must therefore be ignored, falling through to the
    /// ancestor walk — same shape as
    /// `resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive`'s
    /// SVG regression above.
    #[test]
    fn dir_attribute_on_foreign_namespace_element_is_ignored_falls_through_to_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
        let html = doc.push_element(0, "html", None); // no dir -> default ltr
        let svg = doc.push_element_with_namespace(
            html,
            "svg",
            "http://www.w3.org/2000/svg",
            &[("dir", "rtl")],
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[svg].font_family[0].to_string(),
            "ltr-font",
            "dir on a foreign-namespace element must be ignored, not treated \
             as an explicit directionality"
        );
    }

    #[test]
    fn language_range_matches_wildcard_matches_any_tagged_language() {
        // The CSS-spec-level "wildcard doesn't match untagged" rule
        // (`lang_pseudo_matches` doc) is enforced directly by this
        // function's own empty-`content_language` early return, which
        // requires exact string equality against `range` and so yields
        // `false` for a bare `*` range against an empty `content_language`
        // — this function itself just needs to accept any non-empty tag
        // for a bare `*` range (RFC 4647 §3.3.2 step 2's wildcard-subtag
        // clause), which is what the assertions below check.
        assert!(language_range_matches("*", "ja"));
        assert!(language_range_matches("*", "en-US"));
        assert!(language_range_matches("*", "und"));
    }

    /// Selectors L4 §7.2 (quoted in full on `lang_pseudo_matches`'s doc): "A
    /// language range consisting of an empty string (`:lang(\"\")`) matches
    /// (only) elements whose language is not tagged." An element with no
    /// `lang`/`xml:lang` anywhere in its ancestor chain is exactly that case
    /// — `effective_language`'s exhausted-ancestor-chain terminal case
    /// resolves to `String::new()` per HTML LS §3.2.6.2's own final fallback
    /// ("the corresponding language tag is the empty string").
    #[test]
    fn lang_empty_string_range_matches_when_no_lang_anywhere_in_ancestor_chain() {
        // `background-color`, not `font-family`: `font-family` is inherited
        // (CSS Fonts 4 §2), so a rule that only matched `html` or `body`
        // would still show up on `untagged` via ordinary inheritance,
        // making font-family unable to distinguish "matched `untagged`
        // itself" from "matched an ancestor and inherited down" (same
        // pitfall `root_pseudo_class_matches_the_document_root_element_only`'s
        // own comment documents). `background-color` is not inherited (CSS
        // Backgrounds 3 §2.2), so red on `untagged` can only mean
        // `:lang("")` matched `untagged` itself. `tagged` (an explicit
        // `lang="en"` sibling) is the negative control proving the rule
        // isn't matching unconditionally.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(\"\") { background-color: red }");
        let html = doc.push_element(0, "html", None); // no lang
        let body = doc.push_element(html, "body", None); // no lang
        let untagged = doc.push_element(body, "p", None); // no lang
        let tagged = doc.push_element(body, "p", None);
        doc.set_attr(tagged, "lang", "en");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[untagged].background_color, RED,
            ":lang(\"\") must match an element with no lang/xml:lang anywhere \
             in its ancestor chain, per Selectors L4 §7.2"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[tagged].background_color,
            ComputedValues::initial().background_color,
            ":lang(\"\") must not match an element with an explicit lang \
             attribute in its own chain"
        );
    }

    /// Companion regression check for the previous test: `:lang(*)` must NOT
    /// match the same no-lang-anywhere element (Selectors L4 §7.2: "a
    /// wildcard language range (\"*\") does not match elements whose
    /// language is not tagged"). Distinct from the existing
    /// `language_range_matches_wildcard_matches_any_tagged_language` test,
    /// which exercises `language_range_matches` directly as a Rust
    /// function call — this one goes through the full CSS parse + cascade
    /// path, exercising `effective_language`'s terminal
    /// ancestor-chain-exhausted case directly, where the resolved content
    /// language is the empty string.
    ///
    /// The range must be **quoted** (`:lang("*")`, not bare `:lang(*)`) —
    /// `*` is a CSS delimiter token, not a valid `<ident>` character, so the
    /// unquoted form fails `expect_ident_or_string()`
    /// (`parse_non_ts_functional_pseudo_class`'s `:lang()` arm) and the
    /// whole rule is dropped as an invalid selector.
    #[test]
    fn lang_wildcard_range_does_not_match_when_no_lang_anywhere_in_ancestor_chain() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":lang(\"*\") { font-family: wildcard-font }");
        let html = doc.push_element(0, "html", None); // no lang
        let body = doc.push_element(html, "body", None); // no lang
        let untagged = doc.push_element(body, "p", None); // no lang
        let tagged = doc.push_element(body, "p", None);
        doc.set_attr(tagged, "lang", "en");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[untagged].font_family,
            ComputedValues::initial().font_family,
            ":lang(*) must not match an element with no lang/xml:lang \
             anywhere in its ancestor chain, per Selectors L4 §7.2"
        );
        // Positive control: without this, a silently-dropped/unparsed
        // `:lang(*)` rule would make the negative assertion above pass
        // vacuously.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[tagged].font_family[0].to_string(),
            "wildcard-font",
            ":lang(*) must match an element with a real, non-empty lang \
             attribute"
        );
    }

    // --- structural pseudo-classes ---
    //
    // `:root` (CSS Selectors L4 §13.1
    // <https://www.w3.org/TR/selectors-4/#the-root-pseudo>), `:empty`
    // (§13.2 <https://www.w3.org/TR/selectors-4/#the-empty-pseudo>),
    // `:first-child`/`:last-child`/`:only-child`/`:nth-child()`/
    // `:nth-last-child()` (§13.3 area) and `:first-of-type`/`:last-of-type`/
    // `:only-of-type`/`:nth-of-type()`/`:nth-last-of-type()` (§13.4 area).
    // See `matches_empty`/`sibling_position`/`matches_nth` docs for the
    // full spec-provenance notes (L4's own TR anchors truncated on
    // WebFetch; fell back to Selectors Level 3, verbatim-quoted there).

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

    /// A failed descendant search over a deep, branched subtree must not copy
    /// the full ancestor path into every pending sibling. This shape keeps one
    /// sibling pending at each depth, which is the peak retained-path case for
    /// the explicit search stack.
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

    /// `INLINE_SPECIFICITY` (cascade.rs doc, CSS Cascading L4 §6.1
    /// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>: "declarations that
    /// do not belong to a style rule ... are considered to have a specificity
    /// higher than any selector") check.
    ///
    /// # なぜ hardcoded 算術 assert ではなく実 parse なのか
    ///
    /// upstream `selectors` crate (v0.39) は `id_selectors << 20 |
    /// class_like_selectors << 10 | element_selectors` の 32-bit packed
    /// specificity を持ち、各 field は `cmp::min(field, MAX_10BIT)` で
    /// **10-bit に飽和** する
    /// (`selectors-0.39.0/builder.rs` の `MAX_10BIT` / `impl From<Specificity>
    /// for u32` — 挙動理解のための参照であり、raikiri-style 側の実装判断は
    /// この private 定数からではなく selectors の**公開 API** から導いている)。
    /// `MAX_10BIT` はモジュール private (`pub(crate)` にすらなっていない) で
    /// 外部 crate から import できないため、`MAX_PACKED_SPECIFICITY` を
    /// upstream の型から直接 const-derive することは**そもそも不可能**
    /// (許可/禁止の問題ではなく、単に import できる定数が存在しない)。
    ///
    /// 代わりに `Selector::specificity()` を使って現実に到達可能な最大値を
    /// 実測する: 現行の 10-bit 飽和点 (1023) を大きく超える数の ID / class
    /// selector を含む compound selector を構築し、飽和後の実値を読み戻す。
    /// こうすれば upstream が将来 field 幅を広げても (例: issue 本文が挙げる
    /// 11-bit 化)、本 test は**その時点の upstream 実装が実際に返す値**を
    /// 再測定し続けるので、`INLINE_SPECIFICITY` を超えた瞬間に fail する —
    /// 「今の幅を前提にした算術の pin」より頑丈 (hardcoded const assert では
    /// なく実測 test を採る設計)。
    ///
    /// # 未 cover: cascade 経由の end-to-end check (本 test 執筆時点では実装不可だった)
    ///
    /// もう 1 つの選択肢 (`<p id class>` に対する高 specificity
    /// selector と inline style を実際に cascade させ、inline が勝つことを
    /// 見る e2e test) は本 test 執筆時点では構築できなかった: 当時の
    /// `ruletree.rs` `is_type_or_universal_only` が id/class を含む selector
    /// を rule tree 構築時点で drop し、当時の `match_by_tag` も
    /// type/universal 以外の component を持つ selector を一致させなかった
    /// (当時は type + universal selector のみ対応だった)。したがって本 test は
    /// 「numeric な不変条件そのもの」を `crate::parse_selector_list` 経由で // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 直接 check するに留めた。
    ///
    /// class/id/attribute selector matching の実装後
    /// (`is_type_or_universal_only` → `is_supported_selector_list`
    /// rename、`match_by_tag` → `match_simple_selectors` rename +
    /// `StyleElement` 対応)、上記の e2e test を阻んでいたブロッカーは解消
    /// 済み。e2e test 自体の追加は本 test の scope 外のまま、将来の課題
    /// として残す。
    ///
    /// selector 内の class / pseudo-class 数も併せて増やし
    /// (class_like_selectors field)、id field 単独ではなく複数 field が
    /// 同時に飽和する構成にしている。element_selectors field (type selector /
    /// pseudo-element 由来) は 1 compound selector につき type selector を
    /// 1 つしか持てないが、descendant combinator で compound を連結すれば
    /// compound ごとに `Component::LocalName` が積み上がるため、この field も
    /// 公開 API 経由で飽和させられる (`RaikiriSelectorParser` は
    /// `::before`/`::after` (`crate::PseudoElem`) をサポートするが、それらは // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// selector の最右端に 1 個だけしか現れない (後続の combinator/component
    /// を一切許さない、`selector_matches_pseudo_element` doc 参照) ため
    /// combinator 連結による飽和には使えず、この field の飽和経路とは無関係)。
    /// 3 field 全てを飽和させると理論上の
    /// packed 最大値 `0x3FFF_FFFF` (margin 1、実測値) に一致する — これは
    /// 本 const 直上の doc の「margin はちょうど 1」と整合する。
    #[test]
    fn inline_specificity_exceeds_max_reachable_packed_specificity() {
        // 現行 10-bit 飽和点 (1023) を十分に超える数。id field は 4096 個、
        // element field は type-chain 1201 個 (下記) で現行 field を確実に
        // 飽和させる。この余裕はあくまで「現行 10-bit field を確実に飽和
        // させる」ためのものであり、upstream が将来 field 幅を広げた場合に
        // **その新しい幅でも飽和し続ける**ことまでは保証しない (例えば
        // 12-bit = 4095 まで広がれば、この個数では飽和しきらない)。
        // それでも `measured_specificity` は selectors crate の公開 API から
        // 都度実測する値なので、幅が変わって挙動が変化したこと自体は
        // 検知できる — 「理論上の最大値と一致し続ける」のではなく
        // 「upstream の実装変化を都度観測する」ことが本 test の check 機構。
        const FIELD_REPEAT: usize = 4096;
        // element_selectors field は 1 compound selector につき type
        // selector を 1 つしか持てないが、descendant combinator で compound
        // を連結すれば compound ごとに LocalName 分が積み上がる。
        // `"div "` (末尾 space = descendant combinator) を 1200 回連結した
        // 直後に最終 compound `div#a#a...#a.b.b...:hover:active` を置き、
        // id / class / element の 3 field を同時に飽和させる。
        const TYPE_CHAIN_REPEAT: usize = 1200;
        let type_chain: String = "div ".repeat(TYPE_CHAIN_REPEAT);
        let ids: String = "#a".repeat(FIELD_REPEAT);
        let classes: String = ".b".repeat(FIELD_REPEAT);
        let selector_str = format!("{type_chain}div{ids}{classes}:hover:active");
        let list = crate::parse_selector_list(&selector_str).expect("maximal selector must parse");
        let measured_specificity: Specificity = list
            .slice()
            .iter()
            .map(|s| s.specificity())
            .max()
            .expect("selector list is non-empty");

        assert!(
            INLINE_SPECIFICITY > measured_specificity,
            "INLINE_SPECIFICITY ({INLINE_SPECIFICITY:#x}) は selectors crate の \
             公開 API (Selector::specificity) で実測した到達可能最大値 \
             ({measured_specificity:#x}) を上回らなければならない — CSS Cascading L4 \
             §6.1 の 'higher than any selector' 要件。selectors crate の \
             packed-specificity field 幅が変わった signal (cascade.rs \
             INLINE_SPECIFICITY doc 参照)。"
        );
    }

    #[test]
    fn type_selector_beats_universal() {
        let cv = cascade_doc("* { color: red } p { color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn important_beats_normal_across_specificity() {
        // universal !important が type normal に勝つ
        let cv = cascade_doc("p { color: red } * { color: blue !important }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn source_order_tiebreak_later_wins() {
        let cv = cascade_doc("p { color: red } p { color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn later_duplicate_in_same_rule_wins() {
        // 同一 rule 内で同じ property が 2 回 — CSS Cascading L4 §6.1 "Order of
        // Appearance" <https://www.w3.org/TR/css-cascade-4/#cascade-sort>:
        // "The last declaration in document order wins."
        let cv = cascade_doc("p { color: red; color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn later_duplicate_in_inline_wins() {
        // inline style 内で同じ property が 2 回 — 同様に後方が勝つ。
        let cv = cascade_doc("", "p", Some("color: red; color: blue"));
        assert_eq!(cv.color, BLUE);
    }

    /// `pick_winners` の scratch buffer は walk loop の外で確保され全 node で
    /// 共有される。**この共有が持ち込む唯一の新しい
    /// 失敗様式が「前 node の winner slot が drain されずに残り、次 node へ
    /// 漏れる」**であり、本 test がそれを check する。
    ///
    /// 兄弟 2 つに **互いに素な property** を当てるのが要点:
    /// `<p>` は `background-color` slot (discriminant 1) だけを、`<span>` は
    /// `font-weight` slot (discriminant 4) だけを埋める。
    ///
    /// # なぜ `font-weight: bolder` なのか
    ///
    /// slot が持つのは値そのものではなく **`candidates` 内 index** なので、
    /// 漏れた slot は「次 node の candidate list を誤った index で読む」形で
    /// 顕在化する。ここでは `<span>` の `candidates[0]` が `font-weight:
    /// bolder` なので、`<p>` の残した slot 1 と自分の slot 4 が**同じ
    /// declaration を 2 回**適用する。`bolder` は `apply_value` 中で唯一の
    /// read-modify-write arm であるため 400 → 700 → **900** と複合し、
    /// 期待値 700 とずれる。単純代入 property を選ぶと二重適用が冪等になって
    /// leak を素通ししてしまう (実際 `padding` で書いた初版は、drain の
    /// `take()` を `*slot` に落とす mutant を release build で検出できなかった)。
    ///
    /// `background_color` 側の assertion は構造的な control で、こちらは
    /// **non-inherited** であることが効いている — `color` のような inherited
    /// property では「親から継承した値」と「兄弟から漏れた値」が区別できない。
    ///
    /// # 本 test の射程
    ///
    /// - leak の**向き**は traversal 順に依存するので、検出できるのは
    ///   document order で先行する `<p>` → 後続 `<span>` の向きだけ。
    /// - `cargo test` は debug build なので、実際に leak すると
    ///   `pick_winners` 冒頭の debug_assert が先に落ちる。`bolder` の二重適用
    ///   機構が単独で load-bearing になるのは **release build** (debug_assert が
    ///   消える) のみ。逆に言えば本 test の価値は release build での検出可能性と、
    ///   失敗時の診断 message の明示性にある。
    #[test]
    fn winner_does_not_leak_into_next_sibling() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            "p { background-color: red } span { font-weight: bolder }",
        );
        let wrapper = doc.push_element(0, "div", None);
        let p = doc.push_element(wrapper, "p", None);
        let span = doc.push_element(wrapper, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();

        let initial = ComputedValues::initial();
        assert_eq!(r.computed[p].background_color, RED);
        assert_eq!(
            r.computed[span].background_color, initial.background_color,
            "span に p の background-color winner が漏れた"
        );
        // 親 <div> は initial の 400。CSS Fonts 4 §2.2.1 の表で
        // 350 <= 400 < 550 → bolder = 700。二重適用なら 900 になる。
        assert_eq!(
            r.computed[span].font_weight, 700.0,
            "font-weight: bolder が 2 回適用された (slot leak による二重 drain)"
        );
    }

    /// `collect_cascaded` の出力が per-node
    /// `HashMap<StyleNodeId, Vec<CascadedDecl>>` から flat arena +
    /// `HashMap<StyleNodeId, Range<usize>>` (`CascadedArena`) に変わったあと、
    /// per-node grouping / 内部順序 / 「候補 0 件なら entry 無し」の 3 つが
    /// 旧実装と不変であることを直接 check する。
    ///
    /// 3 兄弟 `<p>` を作り、うち 2 つ (`p1`/`p3`) には inline style も足す —
    /// 「stylesheet 2 rule → inline 1 件」の混在順序 (旧実装のまま:
    /// stylesheet 由来が先、inline が最後) を見るため。`<hr>` は
    /// マッチする rule も inline style も持たない — 旧実装の
    /// `if !per_node.is_empty() { out.insert(..) }` と同じ「候補が無ければ
    /// map に entry を作らない」契約を CascadedArena も引き継いでいることを
    /// 確認する。
    ///
    /// ranges の非重複性は flat arena 特有の新しい不変条件 — 個別 `Vec` には
    /// 存在しなかった「他 node の区間と重なってはいけない」という要求で、
    /// `CascadedArena::candidates` が正しい slice を返す前提そのもの
    /// (「global index space を渡すと壊れる」という既知のハザードの、
    /// arena 版の再発防止)。2 つの assertion で役割が分かれる:
    /// `windows(2)` が三者間の pairwise overlap を検査し、末尾の
    /// `assert_eq!` (arena 全長 == 3 区間長の和) が「どの named range にも
    /// 属さない迷子 slot」の有無を検査する — 前者だけでは後者は捕まらない。
    #[test]
    fn collect_cascaded_groups_are_unchanged_by_flat_arena_refactor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red } p { background-color: blue }");

        let p1 = doc.push_element(0, "p", Some("display: block"));
        let p2 = doc.push_element(0, "p", None);
        let empty = doc.push_element(0, "hr", None);
        let p3 = doc.push_element(0, "p", Some("display: inline"));

        let tree = build_rule_tree(&doc);
        let mut arena = CascadedArena::new();
        collect_cascaded(&doc, doc.root_id(), &tree, &mut arena);

        let id = |i: usize| StyleNodeId::new(i as u64);

        // `hr` matches no rule and has no inline style — old code's
        // `if !per_node.is_empty()` guard meant no map entry at all; the
        // arena must not create a zero-length range for it either.
        assert!(
            arena.candidates(id(empty)).is_none(),
            "element with zero candidate declarations must get no arena entry"
        );

        // p2: 2 stylesheet decls, no inline — order = rule/source order.
        let p2c = arena.candidates(id(p2)).expect("p2 has 2 stylesheet decls");
        assert_eq!(p2c.len(), 2);
        assert_eq!(p2c[0].0, PropertyValue::Color(RED));
        assert_eq!(p2c[1].0, PropertyValue::BackgroundColor(BLUE));
        assert_eq!(p2c[0].4, 0, "first rule keeps its source_order");
        assert_eq!(p2c[1].4, 1, "second rule keeps its source_order");

        // p1 / p3: same 2 stylesheet decls, PLUS inline style appended last
        // (collect_cascaded pushes stylesheet rules before inline style).
        let p1c = arena.candidates(id(p1)).expect("p1 has decls");
        assert_eq!(p1c.len(), 3, "2 stylesheet decls + 1 inline, inline last");
        assert_eq!(p1c[0].0, PropertyValue::Color(RED));
        assert_eq!(p1c[1].0, PropertyValue::BackgroundColor(BLUE));
        assert_eq!(p1c[2].0, PropertyValue::Display(DisplayValue::Block));
        assert_eq!(p1c[2].3, INLINE_SPECIFICITY);
        assert_eq!(p1c[2].4, INLINE_SOURCE_ORDER);

        let p3c = arena.candidates(id(p3)).expect("p3 has decls");
        assert_eq!(p3c.len(), 3);
        assert_eq!(p3c[2].0, PropertyValue::Display(DisplayValue::Inline));

        // Stylesheet-only decls (p1/p2/p3 all matched the same 2 `p` rules)
        // carry identical specificity to each other — cross-node consistency
        // the old shared-selector-per-rule code guaranteed too.
        assert_eq!(p1c[0].3, p2c[0].3);
        assert_eq!(p2c[0].3, p3c[0].3);

        // Ranges must not overlap — a flat arena has to hold this invariant
        // that per-node `Vec`s never needed to: if two nodes' ranges ever
        // overlapped, `candidates(id)` would silently hand `pick_winners` a
        // slice containing another node's declarations too
        // (a known "global index space" hazard, arena-shaped).
        //
        // The `windows(2)` loop below only checks pairwise overlap among the
        // three explicitly-named nodes (p1/p2/p3) — it would miss a stray
        // arena slot that belongs to no range, or one double-counted across
        // two ranges. The load-bearing check for "no slot unaccounted for"
        // is the trailing `assert_eq!` after the loop: it compares the
        // arena's total length against the sum of every named range's
        // length, so any leaked/duplicated/orphaned slot shows up as a
        // length mismatch even if no two of the three named ranges overlap
        // each other directly.
        let mut ranges: Vec<_> = [p1, p2, p3]
            .iter()
            .map(|&n| arena.ranges.get(&id(n)).unwrap().clone())
            .collect();
        ranges.sort_by_key(|r| r.start);
        for w in ranges.windows(2) {
            assert!(
                w[0].end <= w[1].start,
                "per-node ranges must not overlap: {:?} vs {:?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(
            arena.decls.len(),
            ranges.iter().map(|r| r.len()).sum::<usize>(),
            "every arena slot belongs to exactly one node's range"
        );
    }

    #[test]
    fn inheritance_walk_child_from_parent_element() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red }");
        let p = doc.push_element(0, "p", None);
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        // <p> が red、<span> は inherit で red
        assert_eq!(r.computed[p].color, RED);
        assert_eq!(r.computed[span].color, RED);
    }

    #[test]
    fn inheritance_falls_through_to_initial_when_no_rule_matches() {
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.color, ComputedValues::initial().color);
    }

    #[test]
    fn text_node_inherits_from_element_parent() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("color: red"));
        let t = doc.push_text(p, "Hi");
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        assert_eq!(r.computed[p].color, RED);
        assert_eq!(r.computed[t].color, RED);
    }

    #[test]
    fn cascade_result_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<CascadeResult>();
    }

    // ── 絶対化 (Phase 2) ───────────────────
    //
    // ここから下の test 群は「cascade を抜けた時点で length が px に解決されて
    // いる」ことを check する。従来は specified value が
    // `ComputedValues` に素通りし、`em` / `rem` は下流 (raikiri-dom layout.rs)
    // で黙って 0px に潰れていた。

    /// 2 段の element を作り、両者の [`ComputedValues`] を返す。
    ///
    /// `cascade_doc` は `<style>` と対象 element を **兄弟**として Document 直下に
    /// 置くため、親子関係を要する test (inheritance / `rem` の root element 判定)
    /// には使えない。
    fn cascade_parent_child(
        parent_tag: &str,
        parent_inline: Option<&str>,
        child_tag: &str,
        child_inline: Option<&str>,
    ) -> (ComputedValues, ComputedValues) {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, parent_tag, parent_inline);
        let child = doc.push_element(parent, child_tag, child_inline);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        (r.computed[parent].clone(), r.computed[child].clone())
    }

    /// Rationale 1 — **本 task の存在理由**。
    ///
    /// 従来はどちらの `<span>` も `font_size == Length::Em(1.5)` になり、
    /// 「宣言由来の `em`」と「inherit 由来の値」が区別できなかった (下流から
    /// 修復不能な live bug)。CSS Cascade 5 §7.2
    /// (<https://www.w3.org/TR/css-cascade-5/#inheriting>) が「inheritance が
    /// 運ぶのは computed value である」と規定するため、両者は別の値でなければ
    /// ならない。
    ///
    /// - (i) `<div style="font-size:1.5em"><span style="font-size:1.5em">` →
    ///   span は **36px** (自 declaration が親 24px に対して解決)
    /// - (ii) `<div style="font-size:1.5em"><span>` → span は **24px**
    ///   (親の computed value を継承)
    #[test]
    fn declared_em_and_inherited_em_produce_different_computed_font_sizes() {
        let (div_i, span_i) = cascade_parent_child(
            "div",
            Some("font-size: 1.5em"),
            "span",
            Some("font-size: 1.5em"),
        );
        let (div_ii, span_ii) = cascade_parent_child("div", Some("font-size: 1.5em"), "span", None);

        // 親はどちらも initial 16px に対する 1.5em = 24px。
        assert_eq!(div_i.font_size, ComputedLength(24.0));
        assert_eq!(div_ii.font_size, ComputedLength(24.0));

        // (i) 宣言由来 — 自 node で再度 1.5 倍される。
        assert_eq!(span_i.font_size, ComputedLength(36.0));
        // (ii) inherit 由来 — 親の computed value がそのまま。
        assert_eq!(span_ii.font_size, ComputedLength(24.0));
        assert_ne!(
            span_i.font_size, span_ii.font_size,
            "declared em と inherited em が同値になるのが 082k Rationale 1 の live bug"
        );
    }

    /// `em` の compounding が cascade 経由で成立する (16px → 1.5em → 1.5em)。
    /// CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#em>。
    #[test]
    fn em_font_size_compounds_across_cascade_levels() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-size: 1.5em"));
        let b = doc.push_element(a, "div", Some("font-size: 1.5em"));
        let c = doc.push_element(b, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].font_size, ComputedLength(24.0));
        assert_eq!(r.computed[b].font_size, ComputedLength(36.0));
        // 孫は宣言が無いので親の computed value を継承 (再乗算しない)。
        assert_eq!(r.computed[c].font_size, ComputedLength(36.0));
    }

    /// `font-size` 以外の length は **自 node の** computed font-size 基準
    /// (CSS Values 4 §6.1.1 `em`)。従来は下流で 0px に潰れていた経路。
    #[test]
    fn box_property_em_resolves_against_own_computed_font_size() {
        let cv = cascade_doc(
            "",
            "div",
            Some(
                "font-size: 20px; padding: 2em; margin-left: 1.5em; border-top-width: 0.5em; border-top-style: solid; width: 3em; line-height: 1.2em",
            ),
        );
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(30.0));
        assert_eq!(cv.border.top.width, ComputedLength(10.0));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(60.0));
        assert_eq!(
            cv.line_height,
            ComputedLineHeight::Length(ComputedLength(24.0))
        );
    }

    /// **root element 自身の `Nrem`** は initial value (16px) 基準。
    ///
    /// CSS Values 4 §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>): "…or
    /// against the computed metrics corresponding to the initial values of the
    /// font and line-height properties, **if the element has no parent**."
    ///
    /// この case は `resolve_inheritance` が root element を過小に special-case
    /// した場合 (tree 全体に単一の `ResolveContext::new(root_font_size)` を配る
    /// 誤実装) に落ちる — 自己参照になり `2rem` が発散/固定点に落ちる。
    #[test]
    fn rem_on_root_element_resolves_against_initial_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: 2rem"));
        assert_eq!(cv.font_size, ComputedLength(32.0));
    }

    /// **root element の子の `Nrem`** は root element の computed font-size 基準。
    ///
    /// CSS Values 4 §6.1.1 (<https://www.w3.org/TR/css-values-4/#rem>): `rem` —
    /// "Equal to the computed value of the em unit on the root element."
    ///
    /// この case は root element を **過剰に** special-case した場合 (子まで
    /// `ResolveContext::initial()` を配る誤実装) に落ちる — 40px ではなく 32px に
    /// なる。上の test と対で初めて閉じる。
    #[test]
    fn rem_below_root_element_resolves_against_root_computed_font_size() {
        let (root, child) = cascade_parent_child(
            "html",
            Some("font-size: 20px"),
            "p",
            Some("font-size: 2rem"),
        );
        assert_eq!(root.font_size, ComputedLength(20.0));
        assert_eq!(child.font_size, ComputedLength(40.0));
    }

    /// root element 上の **box property** の `rem` は自 font-size 基準
    /// (font-\* property ではないので CSS Values 4 §6.1.1 の parent-metrics
    /// 条項は発火せず、`rem` は素の定義「root element の computed font-size」に
    /// なる)。
    ///
    /// 上 2 test と合わせて root element の `rem` を 2 方向から挟む — phase 2 は
    /// initial 16px 基準 (32px)、phase 3 は自 20px 基準 (40px)。
    /// **root 全体に `ResolveContext::initial()` を配る実装ではここが 32px に
    /// なって落ちる。**
    #[test]
    fn rem_on_root_element_box_property_uses_own_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: 20px; padding: 2rem"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
    }

    // ── `lh` / `rlh` (CSS Values 4 §6.1.1) ──────────

    /// `padding: 1lh` は **自 node の** used line-height (own font-size ×
    /// `<number>`) を基準にする — CSS Values 4 §6.1.1 `lh`。
    #[test]
    fn lh_resolves_against_own_computed_line_height() {
        // font-size は initial 16px、line-height: 2 (Number) → used = 32px。
        let cv = cascade_doc("", "div", Some("line-height: 2; padding: 1.5lh"));
        assert_eq!(
            cv.line_height,
            ComputedLineHeight::Number(2.0),
            "line-height 自身は Number のまま computed 層に残る (spec 上 load-bearing)"
        );
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(48.0))); // 1.5 * 32
    }

    /// `line-height: normal` (initial value) の下で `1lh` を使うのは common
    /// case — real font metrics が無いので padding の spec initial `0` に
    /// 倒す (`crate::resolve::resolve_length_percentage` doc、独立実装: // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// 比率を捏造しない)。
    #[test]
    fn lh_falls_back_to_zero_when_own_line_height_is_normal() {
        let cv = cascade_doc("", "div", Some("padding: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    /// Regression check: `margin-top: 1lh`
    /// under the extremely common `line-height: normal` configuration must
    /// compute to `Px(0.0)` — margin's true spec initial (CSS Box 3 §3.1) —
    /// **not** `Auto`. `Auto` would silently trigger real taffy auto-margin
    /// layout (space distribution / centering) with no spec basis, which is
    /// the concrete failure mode the fix (`resolve_margin_length_or_auto`)
    /// closes.
    #[test]
    fn margin_lh_falls_back_to_zero_not_auto_when_line_height_normal() {
        let cv = cascade_doc("", "div", Some("margin-top: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(0.0),
            "margin-top: 1lh under line-height: normal must be Px(0.0), not Auto \
             (Auto would trigger real auto-margin layout with no spec basis)"
        );
    }

    /// `rlh` = "the value of the lh unit on the root element" (CSS Values 4
    /// §6.1.1) — a **tree-global** constant, unaffected by the consuming
    /// node's own font-size / line-height. The root element's own `1rlh`
    /// usage and a descendant's must agree on the same basis; this is the
    /// `root_line_height` / `used_line_height_length` derivation this issue
    /// added `ResolveContext::with_root_line_height` for (mirrors
    /// `rem_on_root_element_box_property_uses_own_font_size`'s `rem` pin).
    #[test]
    fn rlh_on_root_element_matches_child_root_line_height_basis() {
        let mut doc = TestDoc::new();
        let html = doc.push_element(
            0,
            "html",
            Some("font-size: 20px; line-height: 2; padding: 1rlh"),
        );
        // Deliberately different own font-size/line-height, to prove `rlh`
        // does not read the child's own metrics.
        let p = doc.push_element(
            html,
            "p",
            Some("font-size: 100px; line-height: 5; padding: 1rlh"),
        );
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // root's own used line-height: 2 * 20px = 40px.
        assert_eq!(
            r.computed[html].line_height,
            ComputedLineHeight::Number(2.0)
        );
        assert_eq!(
            r.computed[html].padding,
            Sides::all(ComputedLengthPercentage::Px(40.0)),
        );
        // child's `1rlh` uses the *same* 40px basis, not its own (100px,
        // Number(5) → 500px) line-height.
        assert_eq!(
            r.computed[p].padding,
            Sides::all(ComputedLengthPercentage::Px(40.0)),
            "rlh must be the same tree-global constant on the root and on a descendant"
        );
    }

    /// The default (initial) `line-height: normal` on the root propagates the
    /// same "no font metrics" wall through `rlh` to every descendant.
    #[test]
    fn rlh_falls_back_to_zero_when_root_line_height_is_normal() {
        let (root, child) = cascade_parent_child("html", None, "p", Some("padding: 1rlh"));
        assert_eq!(root.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.padding, Sides::all(ComputedLengthPercentage::Px(0.0)),);
    }

    /// `line-height: 1lh` is self-referential (CSS Values 4 §6.1.1, spec
    /// quote canonically documented on
    /// `crate::resolve::resolve_line_height`) — it must use the **parent's** // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// used line-height, not the declaring element's own font-size. The
    /// child's own font-size (50px) is deliberately different from the
    /// parent's (16px, initial) so a bug that leaks the child's own metrics
    /// in would be caught.
    #[test]
    fn line_height_lh_self_reference_uses_parent_not_own_metrics() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("line-height: 2"), // own font-size 16px (initial) → used 32px
            "span",
            Some("font-size: 50px; line-height: 1lh"),
        );
        assert_eq!(parent.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(
            child.line_height,
            ComputedLineHeight::Length(ComputedLength(32.0)),
            "1lh on line-height itself must resolve against the parent's used \
             line-height (32px), not the child's own font-size (50px)"
        );
    }

    /// When the parent's own line-height is unresolvable (`normal`), a
    /// child's self-referential `line-height: 1lh` falls back to
    /// `line-height`'s own initial value `normal` — not a fabricated length.
    #[test]
    fn line_height_lh_self_reference_falls_back_to_normal_when_parent_is_normal() {
        let (parent, child) = cascade_parent_child("div", None, "span", Some("line-height: 1lh"));
        assert_eq!(parent.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.line_height, ComputedLineHeight::Normal);
    }

    /// `line-height: 1rlh`, unlike `1lh` above, is **not** self-referential
    /// in this crate (`rlh`'s own definition, "the
    /// lh unit on the root element", is a tree-global constant that does not
    /// depend on the declaring element's position; see
    /// `crate::resolve::resolve_line_height`'s doc for why the literal // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// "Similarly, lh or rlh" spec wording is not followed for `rlh` on
    /// non-root elements). Three levels (root / middle / leaf) with
    /// **different** line-heights at the root and the immediate parent
    /// discriminate this: if `rlh` were (wrongly) treated as
    /// self-referential like `lh`, the leaf would pick up the *middle*
    /// element's used line-height (48px) instead of the root's (40px).
    #[test]
    fn line_height_rlh_in_line_height_uses_root_not_immediate_parent() {
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2")); // root used = 40px
        let middle = doc.push_element(root, "div", Some("font-size: 12px; line-height: 4")); // middle used = 48px
        let leaf = doc.push_element(middle, "span", Some("line-height: 1rlh"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(
            r.computed[root].line_height,
            ComputedLineHeight::Number(2.0)
        );
        assert_eq!(
            r.computed[middle].line_height,
            ComputedLineHeight::Number(4.0)
        );
        assert_eq!(
            r.computed[leaf].line_height,
            ComputedLineHeight::Length(ComputedLength(40.0)),
            "1rlh on line-height itself must use the root's used line-height \
             (40px), not the immediate parent's (48px) — rlh is not self-referential"
        );
    }

    /// The root element has no parent, so CSS Values 4 §6.1.1's "if the
    /// element has no parent" clause applies: the self-reference basis is
    /// the *initial* line-height, which is `normal` — always unresolvable.
    /// Mirrors `rem_on_root_element_resolves_against_initial_font_size` for
    /// `em`/`rem` on `font-size`.
    #[test]
    fn line_height_lh_self_reference_on_root_element_is_always_normal() {
        let cv = cascade_doc("", "html", Some("line-height: 1lh"));
        assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    }

    // ── `font-size: 1lh` / `1rlh` (CSS Values 4 §6.1.1) ──

    /// **The core bug this issue tracks.** Before the fix, `parse_font_size`
    /// dropped `font-size: 1lh` at *parse* time — not merely computing it
    /// wrong, but making the declaration invisible to cascade winner
    /// selection. `p { font-size: 1lh }` (specificity 0,0,1) must beat
    /// `* { font-size: 12px }` (specificity 0,0,0) under ordinary CSS
    /// cascade rules; before the fix, the `1lh` declaration was silently
    /// discarded and the lower-specificity `12px` declaration won by
    /// default (there being no competing candidate left).
    #[test]
    fn font_size_lh_declaration_is_no_longer_parse_dropped_and_can_win_cascade() {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, "* { font-size: 12px } p { font-size: 1lh }");
        // `* { font-size: 12px }` also matches `html`, but the inline
        // declaration below beats it (inline specificity exceeds any
        // selector's). html: font-size 20px, line-height: 2 → used
        // line-height 40px.
        let html = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2"));
        let p = doc.push_element(html, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[p].font_size,
            ComputedLength(40.0), // 1 * parent's (html's) used line-height (2 * 20px)
            "p's own `font-size: 1lh` (specificity 0,0,1) must win over \
             `* {{ font-size: 12px }}` (specificity 0,0,0); before the fix, \
             1lh was parse-dropped, silently leaving only the universal-selector \
             declaration as a cascade candidate"
        );
    }

    /// `font-size: 1lh` is self-referential (CSS Values 4 §6.1.1, "or font-*
    /// properties on the element they refer to") — it resolves against the
    /// **parent's** used line-height, mirroring `line-height: 1lh`'s own
    /// self-reference (`line_height_lh_self_reference_uses_parent_not_own_metrics`
    /// above).
    #[test]
    fn font_size_lh_resolves_against_parent_used_line_height() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("line-height: 2"), // own font-size 16px (initial) → used 32px
            "span",
            Some("font-size: 1.5lh"),
        );
        assert_eq!(parent.line_height, ComputedLineHeight::Number(2.0));
        assert_eq!(child.font_size, ComputedLength(48.0)); // 1.5 * 32
    }

    /// When the parent's own line-height is unresolvable (`normal`), a
    /// child's self-referential `font-size: 1lh` falls back to `font-size`'s
    /// own spec initial (`medium` = 16px) — not a fabricated ratio
    /// (独立実装), and not `0px` either: unlike `border-width: 1lh` /
    /// `padding: 1lh` under `line-height: normal`
    /// (`lh_falls_back_to_zero_when_own_line_height_is_normal` above, which
    /// share a *generic* resolver with a known `0px` compromise),
    /// `resolve_font_size` is a dedicated
    /// single-property resolver and falls back to its own true initial
    /// directly (`resolve_font_size` doc, "`lh` / `rlh` の自己参照" section).
    #[test]
    fn font_size_lh_falls_back_to_initial_when_parent_line_height_is_normal() {
        let (parent, child) = cascade_parent_child("div", None, "span", Some("font-size: 1lh"));
        assert_eq!(parent.line_height, ComputedLineHeight::Normal);
        assert_eq!(child.font_size, ComputedLength(INITIAL_FONT_SIZE_PX));
    }

    /// The root element has no parent, so `font-size: 1lh` on the root
    /// itself is always unresolvable (CSS Values 4 §6.1.1 "if the element
    /// has no parent" → initial values → `line-height: normal`). Mirrors
    /// `line_height_lh_self_reference_on_root_element_is_always_normal`.
    /// Falls back to `font-size`'s own spec initial, same as the non-root
    /// case above.
    #[test]
    fn font_size_lh_on_root_element_falls_back_to_initial() {
        let cv = cascade_doc("", "html", Some("font-size: 1lh"));
        assert_eq!(cv.font_size, ComputedLength(INITIAL_FONT_SIZE_PX));
    }

    /// `font-size: 1rlh`, unlike `1lh` above, is **not** self-referential
    /// for a non-root element (same asymmetry as `line-height: 1rlh`,
    /// `line_height_rlh_in_line_height_uses_root_not_immediate_parent`
    /// above) — `rlh` always refers to the tree-global root line-height, not
    /// the immediate parent's. Three levels with **different** line-heights
    /// at the root and the immediate parent discriminate this: if `rlh`
    /// were (wrongly) treated as self-referential like `lh`, the leaf would
    /// pick up the *middle* element's used line-height (48px) instead of
    /// the root's (40px).
    #[test]
    fn font_size_rlh_uses_root_not_immediate_parent() {
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2")); // root used = 40px
        let middle = doc.push_element(root, "div", Some("font-size: 12px; line-height: 4")); // middle used = 48px
        let leaf = doc.push_element(middle, "span", Some("font-size: 1rlh"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(
            r.computed[root].line_height,
            ComputedLineHeight::Number(2.0)
        );
        assert_eq!(
            r.computed[middle].line_height,
            ComputedLineHeight::Number(4.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[leaf].font_size,
            ComputedLength(40.0),
            "font-size: 1rlh must use the root's used line-height (40px), not \
             the immediate parent's (48px) — rlh is not self-referential"
        );
    }

    /// Same 3-level tree as `font_size_rlh_uses_root_not_immediate_parent`,
    /// but with `1lh` on the leaf instead of `1rlh` — the two tests together
    /// discriminate a `Lh`/`Rlh` basis swap in `resolve_font_size` (48px vs
    /// 40px, the two possible wrong answers for each other's unit).
    #[test]
    fn font_size_lh_uses_immediate_parent_not_root() {
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("font-size: 20px; line-height: 2")); // root used = 40px
        let middle = doc.push_element(root, "div", Some("font-size: 12px; line-height: 4")); // middle used = 48px
        let leaf = doc.push_element(middle, "span", Some("font-size: 1lh"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[leaf].font_size,
            ComputedLength(48.0),
            "font-size: 1lh must use the *immediate parent's* used line-height \
             (48px), not the root's (40px) — lh is self-referential, unlike rlh"
        );
    }

    /// Document 直下の **非 element** node は rem context を確定させない
    /// (`resolve_inheritance` の `child_ctx` の `None => None` arm)。
    ///
    /// `StyleDom::root_id` は Document node であって root element ではないので、
    /// element を 1 つも通っていない経路では `rem` の参照値が未確定のままで
    /// なければならない。Text / Comment node が誤って「root element」扱いされると
    /// 兄弟 element より先に walk された場合に rem 基準が汚染される。
    #[test]
    fn non_element_node_under_document_does_not_establish_rem_context() {
        let mut doc = TestDoc::new();
        let text = doc.push_text(0, "bare text");
        let html = doc.push_element(0, "html", Some("font-size: 20px"));
        let child = doc.push_element(html, "p", Some("padding: 1rem"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        // 非 element node は cascade winner を持たないので全 field が initial。
        assert_eq!(r.computed[text], ComputedValues::initial());
        // 兄弟 element 側の subtree は自身の root element (html) 基準で解決される
        // — text node の存在に影響されない。
        assert_eq!(r.computed[html].font_size, ComputedLength(20.0));
        assert_eq!(
            r.computed[child].padding,
            Sides::all(ComputedLengthPercentage::Px(20.0))
        );
    }

    /// `rem` は **root element** 基準であって直近の親基準ではない。
    #[test]
    fn rem_ignores_intermediate_font_sizes() {
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", Some("font-size: 20px"));
        let mid = doc.push_element(html, "div", Some("font-size: 40px"));
        let leaf = doc.push_element(mid, "span", Some("padding: 1rem"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[leaf].padding,
            Sides::all(ComputedLengthPercentage::Px(20.0)),
            "rem は root element (20px) 基準 — 直近の親 (40px) ではない"
        );
    }

    /// **D5 invariant**。
    ///
    /// `bolder` は `SpecifiedValues` の staging 上で解決されるが、その基準は
    /// **親の computed font-weight** でなければならない (CSS Fonts 4 §2.2.1
    /// <https://www.w3.org/TR/css-fonts-4/#relative-weights>)。
    /// `SpecifiedValues::inherit_from` が `font_weight` を親からではなく
    /// `initial()` (400) から seed すると 400 → 700 になり、この test だけが
    /// 落ちる (compile error にはならない)。
    #[test]
    fn bolder_resolves_against_parent_computed_weight_through_staging() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-weight: 700"),
            "span",
            Some("font-weight: bolder"),
        );
        assert_eq!(parent.font_weight, 700.0);
        assert_eq!(
            child.font_weight, 900.0,
            "bolder は親の computed 700 に対して解決される (initial 400 起点なら 700 になる)"
        );

        // lighter 側も同じ経路を通る (700 → 400)。
        let (_, lighter) = cascade_parent_child(
            "div",
            Some("font-weight: 700"),
            "span",
            Some("font-weight: lighter"),
        );
        assert_eq!(lighter.font_weight, 400.0);
    }

    /// 3 段の `bolder` chain — staging を経ても compounding が spec table どおり
    /// 進む (400 → 700 → 900 → 900)。
    #[test]
    fn bolder_chain_compounds_through_staging() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-weight: bolder"));
        let b = doc.push_element(a, "div", Some("font-weight: bolder"));
        let c = doc.push_element(b, "div", Some("font-weight: bolder"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].font_weight, 700.0);
        assert_eq!(r.computed[b].font_weight, 900.0);
        assert_eq!(r.computed[c].font_weight, 900.0);
    }

    /// **D5 invariant** — `font-size` 版 (`bolder` の
    /// `bolder_resolves_against_parent_computed_weight_through_staging` と同型)。
    ///
    /// `larger` / `smaller` は `SpecifiedValues` の staging 上で解決されるが、
    /// その基準は**親の computed font-size** でなければならない (CSS Fonts 4
    /// §2.5 <https://www.w3.org/TR/css-fonts-4/#font-size-prop>)。
    /// `SpecifiedValues::inherit_from` が `font_size` を親からではなく
    /// `initial()` (16px) から seed すると 16 → 19.2 になり、この test だけが
    /// 落ちる (compile error にはならない)。
    #[test]
    fn larger_resolves_against_parent_computed_font_size_through_staging() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-size: 20px"),
            "span",
            Some("font-size: larger"),
        );
        assert_eq!(parent.font_size, ComputedLength(20.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            child.font_size,
            ComputedLength(24.0),
            "larger は親の computed 20px に対して解決される (initial 16px 起点なら 19.2px になる)"
        );

        // smaller 側も同じ経路を通る (20px → 20/1.2px)。
        let (_, smaller) = cascade_parent_child(
            "div",
            Some("font-size: 20px"),
            "span",
            Some("font-size: smaller"),
        );
        assert_eq!(smaller.font_size, ComputedLength(20.0 / 1.2));
    }

    /// 3 段の `larger` chain — staging を経ても compounding する
    /// (16 → 19.2 → 23.04)。`bolder_chain_compounds_through_staging` の
    /// font-size 版。
    #[test]
    fn larger_chain_compounds_through_staging() {
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "div", Some("font-size: larger"));
        let b = doc.push_element(a, "div", Some("font-size: larger"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // literal (`19.2`) ではなく式 (`16.0 * 1.2`) で期待値を書く — 実装の
        // 計算式をそのまま mirror し、f32 の最終 bit まで一致させる
        // (`resolve_relative_font_size` の `RATIO` 定数と同じ乗算)。
        assert_eq!(r.computed[a].font_size, ComputedLength(16.0 * 1.2));
        assert_eq!(r.computed[b].font_size, ComputedLength(16.0 * 1.2 * 1.2));
    }

    /// root element の `font-size: larger` — 親が無いので initial (16px) 基準
    /// (`crate::specified::SpecifiedValues::finalize_as_root` doc の // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// "if the element has no parent" 条項、SPEC-6 derivation と同じ pattern)。
    #[test]
    fn larger_on_root_element_resolves_against_initial_font_size() {
        let cv = cascade_doc("", "html", Some("font-size: larger"));
        assert_eq!(cv.font_size, ComputedLength(19.2));
    }

    /// `pt` は cascade 段で px に絶対化される (CSS Values 4 §6.2
    /// <https://www.w3.org/TR/css-values-4/#absolute-lengths>、`1pt = 4/3px`)。
    ///
    /// **font-size の期待値は initial (16px) と一致させてはならない** — 一致させると
    /// `parse_font_size` が `pt` を drop しても (= 本 commit が受理可能にした経路が
    /// 壊れても) initial が残って pass してしまう。`15pt = 20px` を使う。
    #[test]
    fn pt_is_absolutized_at_cascade() {
        let cv = cascade_doc("", "div", Some("font-size: 15pt; padding-top: 9pt"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_ne!(
            cv.font_size,
            ComputedValues::initial().font_size,
            "initial と一致する期待値は parse 側の drop を検出できない"
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(12.0));
    }

    /// `line-height: <percentage>` は **宣言要素**で絶対化され、子は length を
    /// そのまま継承する (CSS Inline 3 §5.1
    /// <https://www.w3.org/TR/css-inline-3/#propdef-line-height>
    /// "Percentages: computed relative to 1em" + "Computed value: … a computed
    /// `<length>` value")。子の font-size で再解決してはならない。
    #[test]
    fn line_height_percentage_is_resolved_at_declaring_element() {
        let (parent, child) = cascade_parent_child(
            "div",
            Some("font-size: 20px; line-height: 150%"),
            "span",
            Some("font-size: 10px"),
        );
        assert_eq!(
            parent.line_height,
            ComputedLineHeight::Length(ComputedLength(30.0))
        );
        assert_eq!(
            child.line_height,
            ComputedLineHeight::Length(ComputedLength(30.0)),
            "子は 30px をそのまま継承する (15px に再解決しない)"
        );
    }

    /// `line-height: <number>` は computed 層でも number のまま継承され、
    /// **子自身の** font-size に掛かる余地を残す (§5.1 の special behavior)。
    #[test]
    fn line_height_number_stays_unitless_through_computed_layer() {
        let (_, child) = cascade_parent_child(
            "div",
            Some("font-size: 20px; line-height: 1.5"),
            "span",
            Some("font-size: 10px"),
        );
        assert_eq!(child.line_height, ComputedLineHeight::Number(1.5));
    }

    /// cascade winner の適用順が絶対化の基準に影響しない
    /// — decision 082k の拘束事項 (絶対化を winner 適用と別 phase にした理由)。
    /// declaration の並び順を入れ替えても `padding: 2em` は同じ 40px になる。
    #[test]
    fn absolutization_is_independent_of_declaration_order() {
        let a = cascade_doc("", "div", Some("font-size: 20px; padding: 2em"));
        let b = cascade_doc("", "div", Some("padding: 2em; font-size: 20px"));
        assert_eq!(a.padding, Sides::all(ComputedLengthPercentage::Px(40.0)));
        assert_eq!(a.padding, b.padding);
        assert_eq!(a.font_size, b.font_size);
    }

    #[test]
    fn cascade_deterministic_across_10_runs() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red } * { color: blue !important }");
        let p = doc.push_element(0, "p", Some("font-size: 20px"));
        doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);

        let baseline = cascade(&doc, &tree).unwrap();
        for _ in 0..9 {
            let run = cascade(&doc, &tree).unwrap();
            assert_eq!(run.computed.len(), baseline.computed.len());
            for i in 0..run.computed.len() {
                assert_eq!(run.computed[i], baseline.computed[i], "differ at node {i}");
            }
        }
    }

    fn deep_chain_doc(depth: usize) -> (TestDoc, usize) {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        for _ in 0..depth {
            parent = doc.push_element(parent, "div", None);
        }
        let style = doc.push_element(parent, "style", None);
        doc.push_text(style, "div { color: red }");
        (doc, parent)
    }

    /// `collect_cascaded` and `resolve_inheritance`
    /// were recursive DFS — a deeply nested DOM could stack-overflow the
    /// process. 5000-level linear chain must cascade without overflow and
    /// produce a correct (non-initial) computed value at the deepest node.
    #[test]
    fn deep_nesting_5000_cascade_no_overflow() {
        let (doc, deepest) = deep_chain_doc(5000);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed.len(), doc.nodes.len());
        assert_eq!(result.computed[deepest].color, RED);
    }

    /// Same regression, but forces the overflow deterministically: run on a
    /// thread with a small, fixed stack size so recursion depth needed to
    /// blow the stack is low and hardware/platform-independent. Before the
    /// iterative-DFS fix this thread aborts with a stack overflow; after the
    /// fix it completes cleanly and returns the correct computed color.
    #[test]
    fn deep_nesting_small_stack_no_overflow() {
        let handle = std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let (doc, deepest) = deep_chain_doc(500);
                let tree = build_rule_tree(&doc);
                let result = cascade(&doc, &tree).expect("cascade Ok");
                result.computed[deepest].color
            })
            .expect("spawn thread");
        let color = handle
            .join()
            .expect("thread must not stack-overflow on deep DOM");
        assert_eq!(color, RED);
    }

    /// Same DOM shape as [`deep_chain_doc`], but the stylesheet's selector is
    /// a `depth`-compound **child**-combinator chain (`div > div > ... > div`)
    /// instead of a single compound (`div`). Child combinator is chosen over
    /// descendant here because `Combinator::Child` has exactly one candidate
    /// per level (`ancestors.split_last()`, no backtracking), so cost is
    /// O(`depth`) — this isolates the pure *stack-depth* question this test
    /// answers. A descendant-combinator version was tried
    /// first and rejected for an unrelated reason, not stack depth:
    /// `Combinator::Descendant`'s backtracking was
    /// separately exponential on uniformly-matching chains (discovered by
    /// that attempt) — since fixed by memoization, see
    /// [`match_combinator_chain`]'s "Memoization" doc and
    /// `descendant_combinator_deep_unsatisfiable_chain_does_not_explode`
    /// below for that fix and its regression test.
    ///
    /// Before this fix, this forced the
    /// `match_combinator_chain`/`match_from_element` mutual recursion
    /// (renamed from `match_from_ancestor`) to a
    /// depth of `depth - 1` (one combinator per ancestor level), 2 native
    /// stack frames per level, independently of `collect_cascaded`'s own
    /// (already-iterative, job 199) DOM-DFS depth. Post-fix, this same
    /// `depth - 1`-long chain instead drives the explicit `Vec<Frame>` stack
    /// [`match_combinator_chain`]'s "Implementation" doc describes, with no
    /// native call-stack recursion involved at all.
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

    /// Empirical answer to the question: does a long, uniformly-
    /// matching child-combinator chain stack-overflow
    /// `match_combinator_chain`/`match_from_element`'s mutual recursion? Same
    /// small-fixed-stack technique as [`deep_nesting_small_stack_no_overflow`]
    /// (job 199 precedent) — deterministic, hardware/platform-independent.
    /// Unlike that test, `doc`/`tree` are built on the *main* (default-size)
    /// stack and only borrowed into the constrained-stack scoped thread —
    /// selector parsing (`build_rule_tree`, `selectors`/`cssparser` crate
    /// internals, outside this crate's implementation review surface) is
    /// deliberately excluded from the measured region, so a crash here can
    /// only be attributed to `cascade`'s own matching path.
    ///
    /// Confirmed empirically at depth 500 / 128 KiB stack against the
    /// mutually-recursive pre-fix implementation: the spawned thread
    /// aborted the whole process with `thread '<unknown>' has overflowed
    /// its stack` / SIGABRT, the same crash shape
    /// [`deep_nesting_small_stack_no_overflow`]'s doc describes for
    /// `collect_cascaded` pre-job-199 — i.e. this was a real, not
    /// hypothetical, overflow risk. After converting the recursion to the
    /// explicit-stack form
    /// below, this test completes cleanly and returns the correct computed
    /// color.
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

    // ── UA origin + display cascade ──

    fn cascade_with_ua(
        ua_css: &str,
        author_css: &str,
        target_tag: &str,
        inline: Option<&str>,
    ) -> ComputedValues {
        // UA rule + Author rule + inline を一気に組み立てて cascade 実行
        let mut doc = TestDoc::new();
        // Author の <style> は DOM 側から build_rule_tree に読ませる
        if !author_css.is_empty() {
            let s = doc.push_element(0, "style", None);
            doc.push_text(s, author_css);
        }
        let e = doc.push_element(0, target_tag, inline);

        // build_rule_tree (Author 集約) + UA add_stylesheet
        let mut tree = build_rule_tree(&doc);
        // UA CSS を先頭に inject するのではなく、既存の Author rule の後ろに
        // add してから rank 化で origin 順序を担保する (source_order より rank
        // が優位)
        // ただし現状 add_stylesheet の呼び出し順で source_order が振られ
        // Author が先 (source_order 小)、UA が後 (source_order 大) となる。
        // rank 化により Origin::UserAgent の Normal は Origin::Author の
        // Normal より常に低い rank になる (`cascade_rank` doc に正確な値
        // あり) ので UA rule が Author を上書きすることはない (source_order
        // に関わらず rank が優先)。
        if !ua_css.is_empty() {
            tree.add_stylesheet(ua_css, Origin::UserAgent);
        }
        let result = cascade(&doc, &tree).expect("cascade Ok");
        result.computed[e].clone()
    }

    #[test]
    fn ua_display_block_applied_when_no_author_rule() {
        // UA CSS のみで <p> の display が Block になる
        let cv = cascade_with_ua("p { display: block }", "", "p", None);
        assert_eq!(cv.display, DisplayValue::Block);
    }

    #[test]
    fn author_display_inline_overrides_ua_block() {
        // Normal Author > Normal UA (`cascade_rank` doc has the exact values)
        let cv = cascade_with_ua("p { display: block }", "p { display: inline }", "p", None);
        assert_eq!(cv.display, DisplayValue::Inline);
    }

    #[test]
    fn author_display_flex_and_grid_compute_through_cascade() {
        // Cascade output (`ComputedValues.display`) is read directly by
        // raikiri-paint's `walk.rs` (independent of raikiri-dom's taffy
        // bridge), so `display: flex` / `display: grid` reaching a
        // `DisplayValue::Flex` / `DisplayValue::Grid` computed value here is
        // observable to that consumer on its own, not only once the taffy
        // bridge exists.
        let flex_cv = cascade_with_ua("", "div { display: flex }", "div", None);
        assert_eq!(flex_cv.display, DisplayValue::Flex);
        let grid_cv = cascade_with_ua("", "div { display: grid }", "div", None);
        assert_eq!(grid_cv.display, DisplayValue::Grid);
    }

    #[test]
    fn author_display_flow_root_computes_through_cascade() {
        // End-to-end pipeline check (parse -> cascade -> ComputedValues) for
        // the standalone CSS Display 3 `flow-root` keyword.
        let cv = cascade_with_ua("", "div { display: flow-root }", "div", None);
        assert_eq!(cv.display, DisplayValue::FlowRoot);
    }

    #[test]
    fn author_display_list_item_computes_through_cascade() {
        // End-to-end pipeline check (parse -> cascade -> ComputedValues) for
        // `display: list-item` — keyword-acceptance only, sibling of
        // `author_display_flex_and_grid_compute_through_cascade` above.
        let cv = cascade_with_ua("", "li { display: list-item }", "li", None);
        assert_eq!(cv.display, DisplayValue::ListItem);
    }

    #[test]
    fn author_display_contents_computes_through_cascade() {
        // CSS Display 3 §2.5 `contents` — same end-to-end pipeline check as
        // `author_display_flex_and_grid_compute_through_cascade` above.
        // `ComputedValues.display` reaching `DisplayValue::Contents` here
        // is this crate's whole scope for this keyword — see
        // `DisplayValue::Contents`'s doc for the known consumer-side box
        // generation gap this does not (and should not) work around.
        let cv = cascade_with_ua("", "div { display: contents }", "div", None);
        assert_eq!(cv.display, DisplayValue::Contents);
    }

    #[test]
    fn author_flex_container_longhands_compute_through_cascade() {
        // End-to-end pipeline check (parse -> cascade -> ComputedValues) for
        // the individual flex-container properties — sibling of
        // `author_display_flex_and_grid_compute_through_cascade` above.
        let cv = cascade_with_ua(
            "",
            "div { display: flex; flex-direction: column; flex-wrap: wrap; \
             justify-content: space-between; align-items: center; \
             align-content: flex-end; row-gap: 10px; column-gap: 5%; }",
            "div",
            None,
        );
        assert_eq!(
            cv.flex_direction,
            crate::property::FlexDirectionValue::Column
        );
        assert_eq!(cv.flex_wrap, crate::property::FlexWrapValue::Wrap);
        assert_eq!(
            cv.justify_content,
            crate::property::ContentAlignmentValue::SpaceBetween
        );
        assert_eq!(cv.align_items, crate::property::SelfAlignmentValue::Center);
        assert_eq!(
            cv.align_content,
            crate::property::ContentAlignmentValue::FlexEnd
        );
        assert_eq!(
            cv.row_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Px(10.0)
        );
        assert_eq!(
            cv.column_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Percent(5.0)
        );
    }

    #[test]
    fn author_flex_item_longhands_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { flex-grow: 2; flex-shrink: 0; flex-basis: 50%; align-self: flex-end; }",
            "div",
            None,
        );
        assert_eq!(cv.flex_grow, 2.0);
        assert_eq!(cv.flex_shrink, 0.0);
        assert_eq!(
            cv.flex_basis,
            crate::resolve::ComputedFlexBasis::Percent(50.0)
        );
        assert_eq!(
            cv.align_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::FlexEnd)
        );
    }

    #[test]
    fn author_multicol_longhands_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { column-count: 3; column-width: 2em; }",
            "div",
            None,
        );
        assert_eq!(cv.column_count, crate::property::ColumnCountValue::Count(3));
        assert_eq!(
            cv.column_width,
            crate::resolve::ComputedColumnWidth::Px(32.0)
        );
    }

    #[test]
    fn author_multicol_shorthand_fans_out_through_cascade() {
        let cv = cascade_with_ua("", "div { columns: 3 2em; }", "div", None);
        assert_eq!(cv.column_count, crate::property::ColumnCountValue::Count(3));
        assert_eq!(
            cv.column_width,
            crate::resolve::ComputedColumnWidth::Px(32.0)
        );
    }

    #[test]
    fn author_flex_basis_intrinsic_keywords_compute_through_cascade() {
        // `min-content` / `max-content` / bare `fit-content` survive the
        // full stylesheet -> cascade path as distinct computed keywords
        // (WPT `flex-basis-valid.html`; rendering approximation lives at
        // the taffy bridge, `bridge_flex` doc).
        let cv = cascade_doc("", "div", Some("flex-basis: min-content"));
        assert_eq!(cv.flex_basis, crate::resolve::ComputedFlexBasis::MinContent);
        let cv = cascade_doc("", "div", Some("flex-basis: max-content"));
        assert_eq!(cv.flex_basis, crate::resolve::ComputedFlexBasis::MaxContent);
        let cv = cascade_doc("", "div", Some("flex-basis: fit-content"));
        assert_eq!(cv.flex_basis, crate::resolve::ComputedFlexBasis::FitContent);
    }

    #[test]
    fn author_flex_shorthand_expands_into_3_longhands_through_cascade() {
        // Proves `crate::rule::expand_shorthand_into`'s `Flex` arm is
        // actually wired into the real parse -> cascade pipeline (not just
        // unit-tested at the parser/expansion-function level) — same
        // end-to-end intent as `flex_shorthand_*` tests in `property.rs`,
        // but through the full stylesheet -> cascade path.
        let cv = cascade_with_ua("", "div { flex: 2 3 10%; }", "div", None);
        assert_eq!(cv.flex_grow, 2.0);
        assert_eq!(cv.flex_shrink, 3.0);
        assert_eq!(
            cv.flex_basis,
            crate::resolve::ComputedFlexBasis::Percent(10.0)
        );
    }

    #[test]
    fn author_gap_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua("", "div { gap: 10px 20px; }", "div", None);
        assert_eq!(
            cv.row_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Px(10.0)
        );
        assert_eq!(
            cv.column_gap,
            crate::resolve::ComputedLengthPercentageOrNormal::Px(20.0)
        );
    }

    #[test]
    fn author_place_content_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { place-content: center space-between; }",
            "div",
            None,
        );
        assert_eq!(
            cv.align_content,
            crate::property::ContentAlignmentValue::Center
        );
        assert_eq!(
            cv.justify_content,
            crate::property::ContentAlignmentValue::SpaceBetween
        );
    }

    #[test]
    fn author_grid_template_columns_track_list_absolutizes_through_cascade() {
        // `2em` at `font-size: 20px` → `40px` — proves phase 3 absolutizes
        // `<length-percentage>` inside the track list (not just passes the
        // specified value through), sibling of `author_flex_item_longhands_compute_through_cascade`
        // above.
        let cv = cascade_with_ua(
            "",
            "div { font-size: 20px; grid-template-columns: 2em 1fr auto; }",
            "div",
            None,
        );
        let crate::resolve::ComputedGridTemplateTracks::List(list) = cv.grid_template_columns
        else {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            panic!("expected a track list");
        };
        assert_eq!(
            list.components,
            vec![
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Px(40.0)
                    )
                ),
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Flex(1.0)
                    )
                ),
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Auto
                    )
                ),
            ]
        );
    }

    #[test]
    fn author_grid_template_rows_and_grid_auto_rows_compute_through_cascade() {
        // Sibling of `author_grid_template_columns_track_list_absolutizes_through_cascade`
        // above — `grid-template-rows`/`grid-auto-rows` share the parser and
        // `apply_value` arm shape with their `-columns` counterparts but
        // were never independently exercised through the cascade pipeline.
        let cv = cascade_with_ua(
            "",
            "div { grid-template-rows: 1fr 2fr; grid-auto-rows: min-content; }",
            "div",
            None,
        );
        let crate::resolve::ComputedGridTemplateTracks::List(rows) = cv.grid_template_rows else {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            panic!("expected a track list");
        };
        assert_eq!(
            rows.components,
            vec![
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Flex(1.0)
                    )
                ),
                crate::resolve::ComputedGridTrackListComponent::Size(
                    crate::resolve::ComputedGridTrackSize::Breadth(
                        crate::resolve::ComputedGridTrackBreadth::Flex(2.0)
                    )
                ),
            ]
        );
        assert_eq!(
            cv.grid_auto_rows,
            std::sync::Arc::new(vec![crate::resolve::ComputedGridTrackSize::Breadth(
                crate::resolve::ComputedGridTrackBreadth::MinContent
            )])
        );
    }

    #[test]
    fn author_grid_template_areas_computes_through_cascade() {
        let cv = cascade_with_ua(
            "",
            r#"div { grid-template-areas: "header header" "nav main"; }"#,
            "div",
            None,
        );
        let crate::property::GridTemplateAreasValue::Areas(areas) = cv.grid_template_areas else {
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            panic!("expected parsed areas");
        };
        assert_eq!(areas.row_count, 2);
        assert_eq!(areas.column_count, 2);
        assert!(areas.areas.iter().any(|a| a.name == "header"));
        assert!(areas.areas.iter().any(|a| a.name == "nav"));
        assert!(areas.areas.iter().any(|a| a.name == "main"));
    }

    #[test]
    fn author_grid_auto_flow_and_placement_longhands_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { grid-auto-flow: column dense; grid-row-start: 2; \
             grid-column-start: span 3; }",
            "div",
            None,
        );
        assert_eq!(
            cv.grid_auto_flow,
            crate::property::GridAutoFlowValue::ColumnDense
        );
        assert_eq!(cv.grid_row_start, crate::property::GridLineValue::Line(2));
        assert_eq!(
            cv.grid_column_start,
            crate::property::GridLineValue::Span(3)
        );
    }

    #[test]
    fn author_grid_row_shorthand_expands_into_2_longhands_through_cascade() {
        // Proves `crate::rule::expand_shorthand_into`'s `GridRow` arm is
        // wired into the real parse -> cascade pipeline, sibling of
        // `author_flex_shorthand_expands_into_3_longhands_through_cascade`
        // above.
        let cv = cascade_with_ua("", "div { grid-row: 2 / 5; }", "div", None);
        assert_eq!(cv.grid_row_start, crate::property::GridLineValue::Line(2));
        assert_eq!(cv.grid_row_end, crate::property::GridLineValue::Line(5));
    }

    #[test]
    fn author_grid_column_shorthand_omitted_second_copies_ident_through_cascade() {
        let cv = cascade_with_ua("", "div { grid-column: content; }", "div", None);
        assert_eq!(
            cv.grid_column_start,
            crate::property::GridLineValue::Named("content".into())
        );
        assert_eq!(
            cv.grid_column_end,
            crate::property::GridLineValue::Named("content".into())
        );
    }

    #[test]
    fn author_justify_items_and_justify_self_compute_through_cascade() {
        let cv = cascade_with_ua(
            "",
            "div { justify-items: center; justify-self: end; }",
            "div",
            None,
        );
        assert_eq!(
            cv.justify_items,
            crate::property::SelfAlignmentValue::Center
        );
        assert_eq!(
            cv.justify_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::End)
        );
    }

    #[test]
    fn author_place_items_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua("", "div { place-items: start end; }", "div", None);
        assert_eq!(cv.align_items, crate::property::SelfAlignmentValue::Start);
        assert_eq!(cv.justify_items, crate::property::SelfAlignmentValue::End);
    }

    #[test]
    fn author_place_self_shorthand_expands_into_2_longhands_through_cascade() {
        let cv = cascade_with_ua("", "div { place-self: center; }", "div", None);
        assert_eq!(
            cv.align_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::Center)
        );
        assert_eq!(
            cv.justify_self,
            crate::property::AlignSelfValue::Value(crate::property::SelfAlignmentValue::Center)
        );
    }

    #[test]
    fn important_ua_beats_important_author_display() {
        // Important UA > Important Author (!important 反転、`cascade_rank`
        // doc has the exact values)
        let cv = cascade_with_ua(
            "p { display: block !important }",
            "p { display: inline !important }",
            "p",
            None,
        );
        assert_eq!(cv.display, DisplayValue::Block);
    }

    // ── cascade_rank 4-tier ordering (3rd tier: CSS
    // Cascading L5 §6.5 "author presentational hint origin"; 4th tier:
    // CSS Cascading L4 §6.2 "user origin") ──

    #[test]
    fn cascade_rank_orders_ua_user_hint_author_normal_then_reverses_for_important() {
        // Direct unit exercise of `cascade_rank`'s full 8-arm match
        // (`cascade_rank`'s doc has the full re-derivation and rank table).
        // Pinned via a chain of relative-order assertions rather than exact
        // `assert_eq!` values: the chain below establishes a complete total
        // order over all 8 values (each value related to its neighbor, no
        // gaps) — it does *not* prove the exact literal values 0-7 (e.g.
        // ranks 10,11,12,14,15,16,17,18 would satisfy the same chain), but
        // that's the right level of strength here, since none of
        // `cascade_rank`'s 3 call sites — `counter_style.rs`'s
        // `insert_with_origin`, `page.rs`'s `cascade_page`, this file's
        // `beats` — switch on the literal `u8`, only compare/order it.
        //
        // The Normal-tier ordering (UA < User < hint < Author) combines two
        // spec-verbatim facts: CSS Cascading L4 §6.1's origin list gives
        // UA < User < Author directly, and CSS Cascading L5 §6.5's verbatim
        // text places the hint strictly between User and Author.
        //
        // The Important-tier ordering (Author < hint < User < UA) also
        // combines two facts, but only one is spec-verbatim: §6.1 directly
        // gives Author < User < UA for the important tier — exactly the
        // reverse of the normal-tier UA < User < Author order, read
        // straight off §6.1's list, not an analogy. The hint's position in
        // that reversal is *not* spec-verbatim: §6.5 never defines an
        // important presentational hint (host languages only ever emit
        // normal-tier hints), so this half pins `cascade_rank`'s own
        // minimal, non-arbitrary extension of that same reversal mechanism
        // to the hint (origin independence + CSS Cascading L4 §6.3's
        // importance reversal, <https://www.w3.org/TR/css-cascade-4/#importance>)
        // and its status as a total function, not an external requirement.
        // No production code path emits an `!important` presentational
        // hint (`push_img_dimension_hints` always pushes
        // `important = false`), so `(AuthorPresentationalHint, true)`
        // stays production-unreached. `(User, false)` / `(User, true)`
        // used to be production-unreached too (no code path routed any
        // declaration to `Origin::User`) until consumer `extra_stylesheets`
        // was wired to it — this test remains the only
        // place `(AuthorPresentationalHint, true)` is exercised, but the
        // two `User` arms now also have real end-to-end coverage via
        // `crates/raikiri/tests/build_cascaded.rs`'s
        // `extra_stylesheets_user_rule_overrides_ua_via_umbrella` (normal)
        // and `user_important_beats_normal_ua_via_umbrella` (important).
        let normal_ua = cascade_rank(Origin::UserAgent, false);
        let normal_user = cascade_rank(Origin::User, false);
        let normal_hint = cascade_rank(Origin::AuthorPresentationalHint, false);
        let normal_author = cascade_rank(Origin::Author, false);
        let important_author = cascade_rank(Origin::Author, true);
        let important_hint = cascade_rank(Origin::AuthorPresentationalHint, true);
        let important_user = cascade_rank(Origin::User, true);
        let important_ua = cascade_rank(Origin::UserAgent, true);

        // Spec-verbatim (§6.1): Normal UA < Normal User < Normal Author.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_ua < normal_user,
            "normal UA must lose to normal user"
        );
        // Spec-verbatim (§6.5): the hint sits strictly between Normal User
        // and Normal Author. `normal_user < normal_author` follows
        // transitively from this assert and the next one, so it isn't
        // pinned separately.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_user < normal_hint,
            "normal user must lose to the hint"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_hint < normal_author,
            "normal hint must lose to a real author declaration"
        );
        // Spec-verbatim (§6.1/§6.3): any important declaration beats any
        // normal declaration.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            normal_author < important_author,
            "any important declaration must beat any normal declaration"
        );
        // Spec-verbatim (§6.1): Important Author < Important User < Important UA.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            important_user < important_ua,
            "important user must lose to important UA"
        );
        // Derived (not spec-verbatim, see comment above): symmetry of
        // origin independence under importance reversal places the hint
        // strictly between Important Author and Important User, mirroring
        // its Normal-tier position between Normal User and Normal Author.
        //
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            important_author < important_hint,
            "derived symmetry: important hint ranks above important author"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            important_hint < important_user,
            "derived symmetry: important user ranks above important hint"
        );
    }

    #[test]
    fn non_inherited_display_child_starts_from_initial_not_parent() {
        // <div> が UA CSS で display: block、その子 <span> は自身 rule がなく、
        // display は non-inherited なので initial (Inline) となる
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ""); // author 空
        let div = doc.push_element(0, "div", None);
        let span = doc.push_element(div, "span", None);

        let mut tree = build_rule_tree(&doc);
        tree.add_stylesheet("div { display: block } span { }", Origin::UserAgent);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].display, DisplayValue::Block);
        assert_eq!(r.computed[span].display, DisplayValue::Inline);
    }

    // ── background-color wire-through (CSS Backgrounds 3 §2.2) ──

    #[test]
    fn background_color_wired_through_cascade_from_inline_style() {
        // <div style="background-color: red"> → ComputedValues.background_color
        // に RED が届く。parser → PropertyValue::BackgroundColor → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。sibling `color` の wire-through
        // pattern (`type_selector_applies_color`) を踏襲。
        let cv = cascade_doc("", "div", Some("background-color: red"));
        assert_eq!(cv.background_color, RED);
    }

    #[test]
    fn background_color_is_non_inherited_child_starts_from_initial_transparent() {
        // CSS Backgrounds 3 §2.2 "Inheritance: no"。<p style='background-color:red'>
        // の子 <span> は自身 rule がなく、background_color は initial (transparent)。
        // sibling pattern (display / counter-* / content / string-set /
        // position の non-inheritance test 群を踏襲)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-color: red"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_color, RED,
            "parent should carry its own background-color"
        );
        assert_eq!(
            r.computed[span].background_color,
            CssColor::TRANSPARENT,
            "child should not inherit background-color (initial: transparent)"
        );
    }

    #[test]
    fn background_color_transparent_keyword_resolves_to_zero_alpha() {
        // CSS Color 4 §6.3 "The transparent keyword": `transparent`
        // = rgba(0, 0, 0, 0)。CssColor::TRANSPARENT が cascade winner として
        // per-node に到達することを check (parse_color の transparent Ident branch
        // と CssColor::TRANSPARENT const の regression canary)。
        let cv = cascade_doc("", "div", Some("background-color: transparent"));
        assert_eq!(cv.background_color, CssColor::TRANSPARENT);
        assert_eq!(cv.background_color.a, 0);
    }

    // ── line-height wire-through (CSS Inline 3 §5.1) ──

    #[test]
    fn line_height_wired_through_cascade_from_inline_style() {
        // <p style="line-height: 1.5"> → ComputedValues.line_height に
        // LineHeight::Number(1.5) が届く。parser → PropertyValue::LineHeight
        // → apply_value → ComputedValues の end-to-end 疎通 smoke
        // (font-size / color と同じ inherited property pattern)。
        let cv = cascade_doc("", "p", Some("line-height: 1.5"));
        assert_eq!(cv.line_height, ComputedLineHeight::Number(1.5));
    }

    #[test]
    fn line_height_is_inherited_child_carries_parent_number() {
        // spec §5.1 "Inheritance: Yes"。<p style="line-height: 1.5"> の子 <span>
        // は自身 rule 無しでも parent の LineHeight::Number(1.5) を継承する
        // (unitless number の specified-value inherit special behavior は
        // cascade static side では raw value 継承として観測される)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("line-height: 1.5"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].line_height, ComputedLineHeight::Number(1.5));
        assert_eq!(
            r.computed[span].line_height,
            ComputedLineHeight::Number(1.5),
            "line-height must be inherited (§5.1 Yes)"
        );
    }

    // ── counter-* wire-through (CSS Lists 3 §4、将来の GCPM 対応に向けた足場) ──

    #[test]
    fn counter_reset_wired_through_cascade_from_inline_style() {
        // <div style="counter-reset: chapter"> → ComputedValues.counter_reset
        // に `[("chapter", 0)]` が届く。parser → PropertyValue → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。
        // counter_reset は Arc<Vec<..>>、`*cv.counter_reset` で deref-compare。
        let cv = cascade_doc("", "div", Some("counter-reset: chapter"));
        assert_eq!(*cv.counter_reset, vec![(SmolStr::new("chapter"), 0)]);
        // 他 counter property は non-inherited の initial (empty) のまま
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
    }

    // ── quotes wire-through (CSS Content 3 §2.4.1) ──

    #[test]
    fn quotes_wired_through_cascade_from_inline_style() {
        // <p style='quotes: "«" "»"'> → ComputedValues.quotes に `[("«", "»")]`
        // が届く。parser → PropertyValue::Quotes → apply_value →
        // ComputedValues の end-to-end 疎通 smoke (`counter_reset` wire-through
        // pattern を踏襲)。quotes は Arc<Vec<..>>、`*cv.quotes` で deref-compare。
        let cv = cascade_doc("", "p", Some(r#"quotes: "«" "»""#));
        assert_eq!(*cv.quotes, vec![(SmolStr::new("«"), SmolStr::new("»"))]);
    }

    #[test]
    fn quotes_is_inherited_child_carries_parent_pairs() {
        // CSS Content 3 §2.4.1 "Inherited: yes"。<div style='quotes: ...'> の
        // 子 <span> は自身 rule 無しでも parent の quotes pairs を継承する
        // (`line_height_is_inherited_child_carries_parent_number` と同型 —
        // counter-* (non-inherited) と対照的に、こちらは real cascade tree を
        // 組んで inheritance walk 自体を通す)。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some(r#"quotes: "«" "»" "‹" "›""#));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let expected = vec![
            (SmolStr::new("«"), SmolStr::new("»")),
            (SmolStr::new("‹"), SmolStr::new("›")),
        ];
        assert_eq!(*r.computed[div].quotes, expected);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            *r.computed[span].quotes, expected,
            "quotes must be inherited (CSS Content 3 §2.4.1 Inherited: yes)"
        );
    }

    // ── content wire-through (CSS Content 3 §2) ──

    #[test]
    fn content_wired_through_cascade_from_inline_style() {
        // <p style='content: "hello"'> → ComputedValues.content に
        // `[Literal("hello")]` が届く。parser → PropertyValue::Content →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // counter-* wire-through pattern を踏襲。
        // content は Arc<Vec<..>>、`*cv.content` で deref-compare。
        // Literal は SmolStr payload (owned String → SmolStr conversion)。
        use crate::property::ContentComponent;
        use smol_str::SmolStr;
        let cv = cascade_doc("", "p", Some(r#"content: "hello""#));
        assert_eq!(
            *cv.content,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    // ── string-set wire-through (CSS GCPM 3 §1.1.1) ──

    #[test]
    fn string_set_wired_through_cascade_from_inline_style() {
        // <p style='string-set: chapter_title "hello"'> → ComputedValues.string_set
        // に `[(chapter_title, [Literal("hello")])]` が届く。
        // parser → PropertyValue::StringSet → apply_value → ComputedValues の
        // end-to-end 疎通 smoke。counter-* / content wire-through pattern を踏襲。
        // string_set は Arc<Vec<..>>、Literal は SmolStr。indexing +
        // field access は Arc<Vec<T>> の Deref chain (`&[T]`) 経由でそのまま
        // 通る (dom/paint consumer 波及 0)。
        use crate::property::ContentComponent;
        let cv = cascade_doc("", "p", Some(r#"string-set: chapter_title "hello""#));
        assert_eq!(cv.string_set.len(), 1);
        assert_eq!(cv.string_set[0].0, SmolStr::new("chapter_title"));
        assert_eq!(
            cv.string_set[0].1,
            vec![ContentComponent::Literal(SmolStr::new("hello"))]
        );
    }

    #[test]
    fn string_set_is_non_inherited_child_starts_from_initial_empty() {
        // CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>:
        // string-set は non-inherited。<p style='string-set: a "x"'>
        // の子 <span> は自身 rule がなく、string_set は initial (empty Vec)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some(r#"string-set: a "x""#));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].string_set.len(),
            1,
            "parent should carry its own string-set"
        );
        assert!(
            r.computed[span].string_set.is_empty(),
            "child should not inherit string-set"
        );
    }

    #[test]
    fn content_is_non_inherited_child_starts_from_initial_empty() {
        // spec §2.1: content は non-inherited。<p style="content: 'x'">
        // の子 <span> は自身 rule がなく、content は initial (empty Vec)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some(r#"content: "parent""#));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].content.len(),
            1,
            "parent should carry its own content"
        );
        assert!(
            r.computed[span].content.is_empty(),
            "child should not inherit content"
        );
    }

    #[test]
    fn list_style_values_inherit_and_author_override() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(
            0,
            "ol",
            Some("list-style-type: upper-roman; list-style-position: inside"),
        );
        let child = doc.push_element(parent, "li", None);
        let override_child = doc.push_element(
            parent,
            "li",
            Some("list-style-type: none; list-style-position: outside"),
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[parent].list_style_type,
            ListStyleType::Named("upper-roman".into())
        );
        assert_eq!(
            r.computed[child].list_style_type,
            ListStyleType::Named("upper-roman".into())
        );
        assert_eq!(
            r.computed[child].list_style_position,
            ListStylePosition::Inside
        );
        assert_eq!(
            r.computed[override_child].list_style_type,
            ListStyleType::None
        );
        assert_eq!(
            r.computed[override_child].list_style_position,
            ListStylePosition::Outside
        );
    }

    #[test]
    fn marker_selector_populates_pseudo_map_and_inherits_list_style() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            r##"li { display: list-item; list-style-type: decimal } li::marker { content: "#"; color: blue }"##,
        );
        let li = doc.push_element(0, "li", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let marker = r
            .pseudo
            .get(&(StyleNodeId(li as u64), PseudoElem::Marker))
            .expect("::marker entry must exist");
        assert_eq!(marker.color, BLUE);
        assert_eq!(*marker.content, vec![ContentComponent::Literal("#".into())]);
        assert_eq!(
            marker.list_style_type,
            ListStyleType::Named("decimal".into())
        );
        assert_eq!(marker.list_style_position, ListStylePosition::Outside);
    }

    // ── ::before/::after pseudo-element cascade (CSS Pseudo-Elements
    //    Module Level 4 §4/§4.1, CSS Content Module Level 3 §2) ──

    #[test]
    fn before_selector_populates_pseudo_map_with_content() {
        // `.foo::before { content: "x" }` on `<p class="foo">` — the real
        // element's own `computed` must NOT carry `content` (that selector
        // never matches `p` itself, only its `::before`), while
        // `result.pseudo[&(p, PseudoElem::Before)]` must.
        use crate::property::ContentComponent;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#".foo::before { content: "x" }"#);
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            r.computed[p].content.is_empty(),
            "the ::before rule must not leak onto the real element itself"
        );
        let before = r
            .pseudo
            .get(&(StyleNodeId(p as u64), PseudoElem::Before))
            .expect("::before entry must exist");
        assert_eq!(*before.content, vec![ContentComponent::Literal("x".into())]);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            !r.pseudo
                .contains_key(&(StyleNodeId(p as u64), PseudoElem::After)),
            "no ::after rule matched — no entry"
        );
    }

    #[test]
    fn after_selector_populates_pseudo_map_independently_of_before() {
        use crate::property::ContentComponent;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p::before { content: "b" } p::after { content: "a" }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        let after = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::After)];
        assert_eq!(*before.content, vec![ContentComponent::Literal("b".into())]);
        assert_eq!(*after.content, vec![ContentComponent::Literal("a".into())]);
    }

    #[test]
    fn pseudo_element_computed_values_inherit_from_real_element_not_initial() {
        // CSS Pseudo-Elements Module Level 4 §4
        // <https://drafts.csswg.org/css-pseudo-4/#treelike>: "They inherit
        // any inheritable properties from their originating element" — so
        // `color` (inherited) on `::before` must come from the real
        // element's own computed color, not the document's initial black,
        // when the `::before` rule itself sets nothing for `color`.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p { color: red } p::before { content: "x" }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            before.color, RED,
            "::before must inherit color from its real originating element"
        );
    }

    #[test]
    fn pseudo_element_own_declaration_overrides_inherited_value() {
        // A `::before` rule can still set its own `color`, overriding what
        // it would otherwise inherit from the real element.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            r#"p { color: red } p::before { content: "x"; color: blue }"#,
        );
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(before.color, BLUE);
    }

    #[test]
    fn pseudo_element_custom_property_wired_through_cascade() {
        // Exercises `CascadedArena`'s custom-property counterpart of the
        // pseudo arena (`pseudo_custom_decls`/`pseudo_custom_ranges`,
        // `collect_cascaded`'s `pseudo_before_custom`/`pseudo_after_custom`
        // scratch buffers) — every other pseudo-element test above only
        // ever pushes through the plain-property side
        // (`pseudo_decls`/`pseudo_ranges`). A `::before` rule can declare a
        // custom property exactly like a real element can; this pins that
        // it actually reaches the pseudo's own `custom_properties`
        // environment (via `resolve_custom_properties` in
        // `resolve_inheritance`'s pseudo-element section), not just the
        // parent's inherited one.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"p::before { content: "x"; --accent: red }"#);
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(before.custom_properties.get("--accent"), Some("red".into()));
    }

    #[test]
    fn pseudo_element_entry_exists_with_empty_content_when_rule_sets_no_content() {
        // `CascadeResult::pseudo` doc's contract to the consumer: map
        // presence only means "some ::before/::after rule matched" — it says
        // nothing about whether that rule actually set `content`. A
        // `::before` rule that sets only `color` (forgets `content`, an easy
        // authoring mistake) must still produce an entry, with an empty
        // `content` list (this crate's `normal`/`none` representation) —
        // not a missing entry, and not a panic.
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p::before { color: blue }");
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        assert_eq!(before.color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            before.content.is_empty(),
            "no `content` declaration on the rule — must compute to the \
             empty (normal/none) representation, not panic or default to \
             something else"
        );
    }

    #[test]
    fn pseudo_element_selector_specificity_participates_in_cascade_ranking() {
        // The pseudo arena runs through the same `pick_winners`/`beats`
        // ranking as the real-element arena — this pins that
        // `specificity_of(selector)` (not some constant) is actually what
        // gets passed through for the `::before` path specifically.
        // `.foo::before` (0,1,0 + pseudo-element count) must beat plain
        // `p::before` (0,0,1 + pseudo-element count) *despite* losing on
        // source order — `.foo::before` is declared **first** here
        // deliberately, so that if the specificity value were accidentally
        // dropped (e.g. a constant passed instead of
        // `specificity_of(selector)`), the later, lower-specificity
        // `p::before` would win on the source-order tie-break instead and
        // this assertion would catch it.
        use crate::property::ContentComponent;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            r#".foo::before { content: "higher-specificity" } p::before { content: "later-source-order" }"#,
        );
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "class", "foo");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let before = &r.pseudo[&(StyleNodeId(p as u64), PseudoElem::Before)];
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            *before.content,
            vec![ContentComponent::Literal("higher-specificity".into())],
            "higher-specificity .foo::before must win over the \
             later-declared, lower-specificity p::before"
        );
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

    // ── position: running() wire-through (CSS GCPM 3 §1.2.1) ──

    #[test]
    fn running_template_wired_through_cascade_from_inline_style() {
        // <div style="position: running(header)"> → ComputedValues.running_templates
        // に `[RunningTemplate{name:"header"}]` が届く。parser → PropertyValue::Position
        // → apply_value → ComputedValues の end-to-end 疎通 smoke。
        // counter-* / content / string-set wire-through pattern を踏襲。
        use crate::computed::RunningTemplate;
        let cv = cascade_doc("", "div", Some("position: running(header)"));
        assert_eq!(
            cv.running_templates,
            vec![RunningTemplate {
                name: SmolStr::new("header")
            }]
        );
    }

    #[test]
    fn running_template_is_non_inherited_child_starts_from_initial_empty() {
        // CSS GCPM 3 §1.2.1。position が non-inherited であることは CSS
        // Positioned Layout 3 §2 <https://www.w3.org/TR/css-position-3/#position-property>
        // の propdef "Inherited: no"。<div style="position: running(hdr)"> の
        // 子 <span> は自身の rule がなく running_templates は initial (empty)。
        // sibling: string_set / content non-inherited と同じ shape。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("position: running(hdr)"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].running_templates.len(),
            1,
            "parent should carry its own running_templates seed"
        );
        assert!(
            r.computed[span].running_templates.is_empty(),
            "child should not inherit running_templates"
        );
    }

    #[test]
    fn position_static_yields_empty_running_templates() {
        // position: static (spec baseline) の場合 apply_value は no-op、
        // running_templates は initial の空 Vec が残る。標準 pattern の pin。
        let cv = cascade_doc("", "div", Some("position: static"));
        assert!(cv.running_templates.is_empty());
    }

    #[test]
    fn static_position_wins_over_running_via_source_order() {
        // `Static` variant の load-bearing 検証。
        // 同一 declaration block 内で `position: running(hdr); position: static`
        // → CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> で後方
        // declaration が同 rank/spec/order で勝つ (source_order
        // が同じでも `beats` の `>=` で最後の候補が上書きする)。winner は
        // Position(Static)、apply_value は no-op → running_templates 空。
        let cv = cascade_doc("", "div", Some("position: running(hdr); position: static"));
        assert!(
            cv.running_templates.is_empty(),
            "later `position: static` must suppress earlier `running(hdr)` — \
             running_templates should stay empty when Static wins the cascade"
        );
    }

    // ── text-justify / text-align-last wire-through + inheritance ──

    #[test]
    fn text_justify_wired_through_cascade_from_inline_style() {
        // <p style="text-justify: inter-word"> → ComputedValues.text_justify。
        // 上記 text-align pattern を踏襲。
        use crate::property::TextJustify;
        let cv = cascade_doc("", "p", Some("text-justify: inter-word"));
        assert_eq!(cv.text_justify, TextJustify::InterWord);
    }

    #[test]
    fn text_justify_distribute_parses() {
        // legacy `distribute` を受理する (WPT text-justify-distribute-001)。
        use crate::property::TextJustify;
        let cv = cascade_doc("", "p", Some("text-justify: distribute"));
        assert_eq!(cv.text_justify, TextJustify::Distribute);
    }

    #[test]
    fn text_justify_inherits_from_parent_element() {
        // CSS Text 3 §6.2: text-justify は **inherited**。
        use crate::property::TextJustify;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-justify: none"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_justify, TextJustify::None);
        assert_eq!(r.computed[span].text_justify, TextJustify::None);
    }

    #[test]
    fn text_align_last_wired_through_cascade_from_inline_style() {
        // <p style="text-align-last: justify"> → ComputedValues.text_align_last。
        use crate::property::TextAlignLast;
        let cv = cascade_doc("", "p", Some("text-align-last: justify"));
        assert_eq!(cv.text_align_last, TextAlignLast::Justify);
    }

    #[test]
    fn text_align_last_inherits_from_parent_element() {
        // CSS Text 3 §6.1: text-align-last は **inherited**。
        use crate::property::TextAlignLast;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align-last: center"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_align_last, TextAlignLast::Center);
        assert_eq!(r.computed[span].text_align_last, TextAlignLast::Center);
    }

    // ── text-wrap wire-through + inheritance (CSS Text 4 §5 subset) ──

    #[test]
    fn text_wrap_nowrap_wired_through_cascade_from_inline_style() {
        use crate::property::TextWrapMode;
        let cv = cascade_doc("", "p", Some("text-wrap: nowrap"));
        assert_eq!(cv.text_wrap, TextWrapMode::Nowrap);
    }

    #[test]
    fn text_wrap_wrap_is_default_and_inherited() {
        use crate::property::TextWrapMode;
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.text_wrap, TextWrapMode::Wrap);
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-wrap: nowrap"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_wrap, TextWrapMode::Nowrap);
        assert_eq!(r.computed[span].text_wrap, TextWrapMode::Nowrap);
    }

    // ── text-align wire-through + inheritance (CSS Text 3 §6.1) ──

    #[test]
    fn text_align_wired_through_cascade_from_inline_style() {
        // <p style="text-align: center"> → ComputedValues.text_align に
        // TextAlign::Center が届く。parser → PropertyValue::TextAlign →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // counter-* / content / string-set / position wire-through
        // pattern を踏襲 (原則 1 前例主義)。
        use crate::property::TextAlign;
        let cv = cascade_doc("", "p", Some("text-align: center"));
        assert_eq!(cv.text_align, TextAlign::Center);
    }

    #[test]
    fn text_align_inherits_from_parent_element() {
        // CSS Text 3 §6.1: text-align は **inherited** (color と同じ handling)。
        // <p style="text-align: center"> の子 <span> は自身 rule 無しでも
        // 親の text_align (Center) を引き継ぐ。inheritance walk が
        // inherit_from 経由で text_align を copy することを pin。
        //
        // Verification #7 の中核 assertion (parent center → child Center を確認)。
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align: center"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_align, TextAlign::Center);
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Center,
            "child should inherit text-align from parent (CSS Text 3 §6.1 inherited property)"
        );
    }

    #[test]
    fn text_align_inheritance_contrasts_with_display_non_inheritance() {
        // Verification #7 (contrast): text-align (inherited) と display
        // (non-inherited) を同一 fixture で対比 — inheritance discipline を明示。
        // parent が両 property を持ち、child は inherit で text-align のみ引き継ぐ、
        // display は initial (Inline) に落ちる。inherit_from の inherited /
        // non-inherited 分類が正しく機能していることの pin。
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        // p に UA-like rule として display: block を Author 側で置く (現状
        // UA rule も同 rank に居るので、child が inherit しない性質だけを見る)
        doc.push_text(s, "p { display: block; text-align: right }");
        let p = doc.push_element(0, "p", None);
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // parent: 両 property が Author rule で set される。
        assert_eq!(r.computed[p].display, DisplayValue::Block);
        assert_eq!(r.computed[p].text_align, TextAlign::Right);
        // child: 自身 rule 無し。text-align (inherited) は Right を引き継ぐが、
        // display (non-inherited) は initial (Inline) に落ちる。
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Right,
            "text-align must inherit (CSS Text 3 §6.1 inherited)"
        );
        assert_eq!(
            r.computed[span].display,
            DisplayValue::Inline,
            "display must NOT inherit (CSS Display 3 §2 Inherited: no) — initial Inline"
        );
    }

    // ── text-indent wire-through + inheritance (CSS Text 3 §8.1) ──

    #[test]
    fn text_indent_wired_through_cascade_from_inline_style() {
        // <p style="text-indent: 20px"> → ComputedValues.text_indent に
        // ComputedLengthPercentage::Px(20.0) が届く。parser →
        // PropertyValue::TextIndent → apply_value → ComputedValues の
        // end-to-end 疎通 smoke (`text_align_wired_through_cascade_from_inline_style`
        // と同 pattern)。
        let cv = cascade_doc("", "p", Some("text-indent: 20px"));
        assert_eq!(cv.text_indent, ComputedLengthPercentage::Px(20.0));
    }

    #[test]
    fn text_indent_flags_wired_through_cascade_from_inline_style() {
        // <p style="text-indent: 2em hanging each-line"> → length plus flags.
        let cv = cascade_doc("", "p", Some("text-indent: 2em hanging each-line"));
        assert!(cv.text_indent_hanging);
        assert!(cv.text_indent_each_line);
    }

    #[test]
    fn text_indent_percentage_stays_unresolved_in_computed_layer() {
        // CSS Text 3 §8.1 "Computed value: computed <length-percentage>
        // value, plus any specified keywords" — `%` は block container 自身の
        // inline-axis inner size 依存 (used value 層) なので、この crate の
        // computed 層では `Percent` のまま残る (`padding` / `width` と同じ
        // 扱い、`ComputedValues::padding` doc 参照)。
        let cv = cascade_doc("", "p", Some("text-indent: 10%"));
        assert_eq!(cv.text_indent, ComputedLengthPercentage::Percent(10.0));
    }

    #[test]
    fn text_indent_inherits_from_parent_element() {
        // CSS Text 3 §8.1: text-indent は **inherited**。<p> の `2em` は親の
        // font-size (20px) 基準で 40px に絶対化され、子 <span> はその**絶対化
        // 済み 40px を再解決せず継承**する (`lift_line_height` doc が
        // line-height について説明する挙動と同型 — 子が独自の font-size
        // (10px) を持っていても 40px のままであることで、この
        // "re-resolve しない" 性質を子の font-size を変えて check する)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-size: 20px; text-indent: 2em"));
        let span = doc.push_element(p, "span", Some("font-size: 10px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].text_indent,
            ComputedLengthPercentage::Px(40.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_indent,
            ComputedLengthPercentage::Px(40.0),
            "child should inherit text-indent's already-absolutized 40px \
             unchanged (CSS Text 3 §8.1 inherited property), not re-resolve \
             `2em` against its own 10px font-size"
        );
    }

    #[test]
    fn text_indent_ch_preserves_source_font_through_inheritance_and_override() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-size: 20px; text-indent: 1ch"));
        let inherited = doc.push_element(p, "span", Some("font-size: 40px"));
        let own = doc.push_element(p, "strong", Some("font-size: 40px; text-indent: 2ch"));
        let own_child = doc.push_element(own, "i", Some("font-size: 10px"));
        let cleared = doc.push_element(p, "em", Some("font-size: 40px; text-indent: 2em"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");

        assert_eq!(r.computed[p].text_indent_ch_factor, Some(1.0));
        assert_eq!(r.computed[inherited].text_indent_ch_factor, Some(1.0));
        assert_eq!(
            r.computed[inherited]
                .text_indent_ch_font
                .as_ref()
                .expect("inherited source font")
                .size,
            ComputedLength(20.0)
        );
        assert_eq!(r.computed[own].text_indent_ch_factor, Some(2.0));
        assert_eq!(
            r.computed[own]
                .text_indent_ch_font
                .as_ref()
                .expect("own source font")
                .size,
            ComputedLength(40.0)
        );
        assert_eq!(
            r.computed[own_child]
                .text_indent_ch_font
                .as_ref()
                .expect("inherited own source font")
                .size,
            ComputedLength(40.0)
        );
        assert_eq!(r.computed[cleared].text_indent_ch_factor, None);
        assert_eq!(r.computed[cleared].text_indent_ch_font, None);
    }

    #[test]
    fn text_indent_inheritance_contrasts_with_padding_non_inheritance() {
        // Verification (contrast): text-indent (inherited) と padding-top
        // (non-inherited) を同一 fixture で対比 —
        // `text_align_inheritance_contrasts_with_display_non_inheritance` と
        // 同 pattern。child は自身 rule 無し、text-indent のみ引き継ぎ、
        // padding-top は initial (0) に落ちる。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-indent: 15px; padding-top: 15px"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].text_indent,
            ComputedLengthPercentage::Px(15.0)
        );
        assert_eq!(
            r.computed[p].padding.top,
            ComputedLengthPercentage::Px(15.0)
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_indent,
            ComputedLengthPercentage::Px(15.0),
            "text-indent must inherit (CSS Text 3 §8.1 Inherited: yes)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].padding.top,
            ComputedLengthPercentage::Px(0.0),
            "padding-top must NOT inherit (CSS Box 3 §4.1 Inherited: no) — initial 0"
        );
    }

    #[test]
    fn text_align_child_own_value_wins_over_inherited() {
        // parent center + child left → child は自身 rule の Left が cascade winner。
        // inheritance は「rule 無し fallback」であって override 元ではないことを pin。
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align: center"));
        let span = doc.push_element(p, "span", Some("text-align: left"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_align, TextAlign::Center);
        assert_eq!(r.computed[span].text_align, TextAlign::Left);
    }

    // ── direction wire-through (CSS Writing Modes 4 §2.1) ──

    #[test]
    fn direction_wired_through_cascade_from_inline_style() {
        use crate::property::Direction;
        let cv = cascade_doc("", "p", Some("direction: rtl"));
        assert_eq!(cv.direction, Direction::Rtl);
    }

    #[test]
    fn direction_inherits_from_parent_element() {
        // CSS Writing Modes 4 §2.1: direction は **inherited**.
        use crate::property::Direction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Rtl);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].direction,
            Direction::Rtl,
            "child should inherit direction from parent (CSS Writing Modes 4 §2.1 Inherited: yes)"
        );
    }

    #[test]
    fn direction_child_own_value_wins_over_inherited() {
        use crate::property::Direction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl"));
        let span = doc.push_element(p, "span", Some("direction: ltr"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Rtl);
        assert_eq!(r.computed[span].direction, Direction::Ltr);
    }

    // ── font-style wire-through (CSS Fonts 4 §2.4) ──

    #[test]
    fn font_style_wired_through_cascade_from_inline_style() {
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: italic"));
        assert_eq!(cv.font_style, FontStyle::Italic);
    }

    #[test]
    fn font_style_oblique_wired_through_cascade_from_inline_style() {
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: oblique"));
        assert_eq!(cv.font_style, FontStyle::Oblique);
    }

    #[test]
    fn font_style_inherits_from_parent_element() {
        // CSS Fonts 4 §2.4: font-style は **inherited**.
        use crate::property::FontStyle;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-style: italic"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_style, FontStyle::Italic);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].font_style,
            FontStyle::Italic,
            "child should inherit font-style from parent (CSS Fonts 4 §2.4 Inherited: yes)"
        );
    }

    #[test]
    fn font_style_child_own_value_wins_over_inherited() {
        use crate::property::FontStyle;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-style: italic"));
        let span = doc.push_element(p, "span", Some("font-style: normal"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_style, FontStyle::Italic);
        assert_eq!(r.computed[span].font_style, FontStyle::Normal);
    }

    // ── font-style scope-limited cascade consequence ──
    //
    // `font-style`'s spec-valid but unimplemented keywords (`left` / `right`
    // and `oblique <angle>`) are silent-dropped at parse time
    // (`crate::property::FontStyle` doc's "Scope carving" section,
    // `parse_font_style` returning `None` → `DeclParser` dropping the whole
    // declaration like any other unhandled ident
    // — same mechanism the `word-break: break-word` Non-goal uses).
    // At cascade level this means the dropped declaration never participates
    // as a winner, so an earlier valid declaration for the same property
    // stays the winner instead of the property falling back to its initial
    // value or applying the spec-mandated semantics. This is the crate-wide
    // "scope-limited-but-spec-valid" gap; these tests check the current
    // declaration-drop → prior-wins behaviour so a future fix is visible.

    #[test]
    fn font_style_scope_cut_left_is_dropped_and_prior_wins_via_stylesheet() {
        // `left` / `right` are spec-valid `font-style` keywords (CSS Fonts 4
        // §2.4) but this crate's scope excludes them (`FontStyle` doc).
        // Second rule is parse-dropped, so first rule stays winner.
        use crate::property::FontStyle;
        let cv = cascade_doc("p { font-style: italic } p { font-style: left }", "p", None);
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            cv.font_style,
            FontStyle::Italic,
            "scope-limited `font-style: left` is dropped, prior `italic` must remain winner (current crate behaviour)"
        );
    }

    #[test]
    fn font_style_scope_cut_right_is_dropped_and_prior_wins_via_inline() {
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: italic; font-style: right"));
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            cv.font_style,
            FontStyle::Italic,
            "scope-limited `font-style: right` is dropped, prior `italic` must remain winner"
        );
    }

    #[test]
    fn font_style_scope_cut_oblique_angle_is_dropped_and_prior_wins() {
        // `oblique <angle>` is spec-valid (CSS Fonts 4 §2.4) but this crate
        // accepts only bare `oblique` (`FontStyle` doc's Scope carving).
        // `oblique 14deg` is consumed as `oblique` then rejected by
        // `DeclParser::expect_exhausted` for the leftover `<angle>` token,
        // so the whole declaration is dropped — prior wins.
        use crate::property::FontStyle;
        let cv = cascade_doc(
            "",
            "p",
            Some("font-style: italic; font-style: oblique 14deg"),
        );
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(
            cv.font_style,
            FontStyle::Italic,
            "scope-limited `font-style: oblique 14deg` is dropped, prior `italic` must remain winner"
        );
        // Same via stylesheet ordering.
        let cv = cascade_doc(
            "p { font-style: italic } p { font-style: oblique 14deg }",
            "p",
            None,
        );
        // cov:ignore: assertion text is only evaluated when this test fails
        assert_eq!(cv.font_style, FontStyle::Italic);
    }

    #[test]
    fn font_style_scope_cut_alone_falls_back_to_initial() {
        // A lone scope-limited declaration never wins, so the property stays at
        // its initial value (`normal`), not the scope-limited keyword.
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: left"));
        assert_eq!(cv.font_style, FontStyle::Normal);
        let cv = cascade_doc("", "p", Some("font-style: oblique 14deg"));
        assert_eq!(cv.font_style, FontStyle::Normal);
    }

    #[test]
    fn font_style_bare_oblique_is_not_scope_cut_and_wins() {
        // Control: bare `oblique` IS implemented and must win over a prior
        // declaration, proving the prior-wins above is due to the scope-limited
        // drop, not a generic cascade bug.
        use crate::property::FontStyle;
        let cv = cascade_doc("", "p", Some("font-style: italic; font-style: oblique"));
        assert_eq!(cv.font_style, FontStyle::Oblique);
        let cv = cascade_doc(
            "p { font-style: italic } p { font-style: oblique }",
            "p",
            None,
        );
        assert_eq!(cv.font_style, FontStyle::Oblique);
    }

    // ── font-variant-caps wire-through (CSS Fonts Module Level 3 §6.6) ──

    #[test]
    fn font_variant_caps_wired_through_cascade_from_inline_style() {
        use crate::property::FontVariantCaps;
        let cv = cascade_doc("", "p", Some("font-variant-caps: small-caps"));
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
    }

    #[test]
    fn font_variant_caps_inherits_from_parent_element() {
        // CSS Fonts Module Level 3 §6.6: font-variant-caps は **inherited**.
        use crate::property::FontVariantCaps;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-variant-caps: small-caps"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_variant_caps, FontVariantCaps::SmallCaps);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].font_variant_caps,
            FontVariantCaps::SmallCaps,
            "child should inherit font-variant-caps from parent (CSS Fonts Module Level 3 §6.6 Inherited: yes)"
        );
    }

    #[test]
    fn font_variant_caps_child_own_value_wins_over_inherited() {
        use crate::property::FontVariantCaps;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-variant-caps: small-caps"));
        let span = doc.push_element(p, "span", Some("font-variant-caps: normal"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(r.computed[span].font_variant_caps, FontVariantCaps::Normal);
    }

    // ── text-transform wire-through (CSS Text Module Level 3 §2.1) ──

    #[test]
    fn text_transform_wired_through_cascade_from_inline_style() {
        use crate::property::TextTransform;
        let cv = cascade_doc("", "p", Some("text-transform: uppercase"));
        assert_eq!(cv.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn text_transform_inherits_from_parent_element() {
        // CSS Text Module Level 3 §2.1: text-transform は **inherited**.
        use crate::property::TextTransform;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-transform: uppercase"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_transform, TextTransform::Uppercase);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_transform,
            TextTransform::Uppercase,
            "child should inherit text-transform from parent (CSS Text Module Level 3 §2.1 Inherited: yes)"
        );
    }

    // ── visibility wire-through (CSS Display 3 §4) ──

    #[test]
    fn visibility_wired_through_cascade_from_inline_style() {
        use crate::property::Visibility;
        let cv = cascade_doc("", "p", Some("visibility: hidden"));
        assert_eq!(cv.visibility, Visibility::Hidden);
    }

    #[test]
    fn visibility_inherits_from_parent_element() {
        // CSS Display 3 §4: visibility は **inherited**.
        use crate::property::Visibility;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("visibility: hidden"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].visibility, Visibility::Hidden);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].visibility,
            Visibility::Hidden,
            "child should inherit visibility from parent (CSS Display 3 §4 Inherited: yes)"
        );
    }

    // ── table-layout / border-collapse wire-through (CSS Tables 3 §4/§6) ──

    #[test]
    fn table_layout_wired_through_cascade_from_inline_style() {
        use crate::property::TableLayoutValue;
        let cv = cascade_doc("", "table", Some("table-layout: fixed"));
        assert_eq!(cv.table_layout, TableLayoutValue::Fixed);
    }

    #[test]
    fn table_layout_does_not_inherit_from_parent_element() {
        // CSS Tables 3 §4: table-layout は **non-inherited**.
        use crate::property::TableLayoutValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("table-layout: fixed"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[table].table_layout, TableLayoutValue::Fixed);
        assert_eq!(
            r.computed[td].table_layout,
            TableLayoutValue::Auto,
            "child should reset table-layout to initial (CSS Tables 3 §4 Inherited: no)"
        );
    }

    #[test]
    fn border_collapse_wired_through_cascade_from_inline_style() {
        use crate::property::BorderCollapseValue;
        let cv = cascade_doc("", "table", Some("border-collapse: collapse"));
        assert_eq!(cv.border_collapse, BorderCollapseValue::Collapse);
    }

    #[test]
    fn border_collapse_inherits_from_parent_element() {
        // CSS Tables 3 §6: border-collapse は **inherited**.
        use crate::property::BorderCollapseValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("border-collapse: collapse"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[table].border_collapse,
            BorderCollapseValue::Collapse
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[td].border_collapse,
            BorderCollapseValue::Collapse,
            "child should inherit border-collapse from parent (CSS Tables 3 §6 Inherited: yes)"
        );
    }

    // ── border-spacing wire-through (CSS Tables 3 §6.1) ──
    //
    // WPT css/css-tables/parsing/border-spacing-computed.html の
    // plain-length 3 case の check (`"10px 20px"` / `"0"` → `"0px"` /
    // single-doubles)。`calc()` + relative-unit 混じり 2 case は本 engine
    // の math evaluator が mixed-unit calc を解決しない
    // (`mixed_length_percentage_math_is_intentionally_not_supported` 参照)
    // ため対象外 — baseline check 側の注記参照。

    #[test]
    fn border_spacing_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "table", Some("border-spacing: 10px 20px"));
        assert_eq!(
            cv.border_spacing.horizontal,
            crate::resolve::ComputedLength(10.0)
        );
        assert_eq!(
            cv.border_spacing.vertical,
            crate::resolve::ComputedLength(20.0)
        );
        // WPT computed: `"10px 20px"` stays two lengths.
        assert_eq!(cv.border_spacing.serialized(), "10px 20px");
    }

    #[test]
    fn border_spacing_zero_serializes_shortest() {
        // WPT computed: `"0"` → `"0px"` (not `"0px 0px"`, CSSOM §2.1
        // shortest serialization).
        let cv = cascade_doc("", "table", Some("border-spacing: 0"));
        assert_eq!(cv.border_spacing.serialized(), "0px");
    }

    #[test]
    fn border_spacing_single_value_doubles_to_both_axes() {
        let cv = cascade_doc("", "table", Some("border-spacing: 10px"));
        assert_eq!(cv.border_spacing.serialized(), "10px");
    }

    #[test]
    fn border_spacing_resolves_em_against_own_font_size() {
        // font-size 40px の node での `0.5em` → 20px (WPT computed file が
        // `#target` に `font-size: 40px` を指定するのと同じ基準)。
        let cv = cascade_doc(
            "",
            "table",
            Some("font-size: 40px; border-spacing: 0.5em 10px"),
        );
        assert_eq!(cv.border_spacing.serialized(), "20px 10px");
    }

    #[test]
    fn border_spacing_inherits_from_parent_element() {
        // CSS Tables 3 §6.1: border-spacing は **inherited**.
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("border-spacing: 10px 20px"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[td].border_spacing.serialized(),
            "10px 20px",
            "child should inherit border-spacing from parent (CSS Tables 3 §6.1 Inherited: yes)"
        );
    }

    // ── caption-side wire-through (CSS Tables 3 §7) ──

    #[test]
    fn caption_side_wired_through_cascade_from_inline_style() {
        use crate::property::CaptionSideValue;
        let cv = cascade_doc("", "table", Some("caption-side: bottom"));
        assert_eq!(cv.caption_side, CaptionSideValue::Bottom);
        // WPT caption-side-computed.html: single keyword serializes as-is.
        let cv_top = cascade_doc("", "table", Some("caption-side: top"));
        assert_eq!(cv_top.caption_side, CaptionSideValue::Top);
    }

    #[test]
    fn caption_side_inherits_from_parent_element() {
        // CSS Tables 3 §7: caption-side は **inherited**.
        use crate::property::CaptionSideValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("caption-side: bottom"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[td].caption_side,
            CaptionSideValue::Bottom,
            "child should inherit caption-side from parent (CSS Tables 3 §7 Inherited: yes)"
        );
    }

    // ── empty-cells wire-through (CSS Tables 3 §8) ──

    #[test]
    fn empty_cells_wired_through_cascade_from_inline_style() {
        use crate::property::EmptyCellsValue;
        let cv = cascade_doc("", "table", Some("empty-cells: hide"));
        assert_eq!(cv.empty_cells, EmptyCellsValue::Hide);
        // WPT empty-cells-computed.html: single keyword serializes as-is.
        let cv_show = cascade_doc("", "table", Some("empty-cells: show"));
        assert_eq!(cv_show.empty_cells, EmptyCellsValue::Show);
    }

    #[test]
    fn empty_cells_inherits_from_parent_element() {
        // CSS Tables 3 §8: empty-cells は **inherited**.
        use crate::property::EmptyCellsValue;
        let mut doc = TestDoc::new();
        let table = doc.push_element(0, "table", Some("empty-cells: hide"));
        let td = doc.push_element(table, "td", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[td].empty_cells,
            EmptyCellsValue::Hide,
            "child should inherit empty-cells from parent (CSS Tables 3 §8 Inherited: yes)"
        );
    }

    // ── word-break wire-through (CSS Text 3 §5.1) ──

    #[test]
    fn word_break_wired_through_cascade_from_inline_style() {
        use crate::property::WordBreak;
        let cv = cascade_doc("", "p", Some("word-break: break-all"));
        assert_eq!(cv.word_break, WordBreak::BreakAll);
    }

    #[test]
    fn word_break_inherits_from_parent_element() {
        // CSS Text 3 §5.1: word-break は **inherited**.
        use crate::property::WordBreak;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("word-break: break-all"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].word_break, WordBreak::BreakAll);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].word_break,
            WordBreak::BreakAll,
            "child should inherit word-break from parent (CSS Text 3 §5.1 Inherited: yes)"
        );
    }

    #[test]
    fn text_transform_child_own_value_wins_over_inherited() {
        use crate::property::TextTransform;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-transform: uppercase"));
        let span = doc.push_element(p, "span", Some("text-transform: lowercase"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_transform, TextTransform::Uppercase);
        assert_eq!(r.computed[span].text_transform, TextTransform::Lowercase);
    }

    #[test]
    fn visibility_child_own_value_wins_over_inherited() {
        use crate::property::Visibility;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("visibility: hidden"));
        let span = doc.push_element(p, "span", Some("visibility: visible"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].visibility, Visibility::Hidden);
        assert_eq!(r.computed[span].visibility, Visibility::Visible);
    }

    #[test]
    fn word_break_child_own_value_wins_over_inherited() {
        use crate::property::WordBreak;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("word-break: break-all"));
        let span = doc.push_element(p, "span", Some("word-break: keep-all"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].word_break, WordBreak::BreakAll);
        assert_eq!(r.computed[span].word_break, WordBreak::KeepAll);
    }

    // ── word-break scope-limited cascade consequence ──
    //
    // `word-break: break-word` is spec-valid (CSS Text 3 §5.1) but
    // intentionally unimplemented (`WordBreak` doc's "Scope carving" section:
    // deprecated cross-property `normal` + `overflow-wrap: anywhere`
    // semantics have no slot in this crate's per-property cascade model).
    // The declaration is silent-dropped at parse time
    // (`parse_word_break` → `None` → `DeclParser` drops), so at cascade level
    // it never becomes a winner. The consequence is that an earlier valid
    // declaration for `word-break` stays the winner, instead of the property
    // falling back to its initial `normal` (or applying the spec-mandated
    // `break-word` → `normal` + `anywhere` equivalent as browsers do).
    // These tests check that current prior-wins behaviour.

    #[test]
    fn word_break_break_word_is_now_accepted_and_wins_via_stylesheet() {
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "p { word-break: break-all } p { word-break: break-word }",
            "p",
            None,
        );
        assert_eq!(
            cv.word_break,
            WordBreak::BreakWord,
            "break-word is now implemented and must win over break-all"
        );
    }

    #[test]
    fn word_break_break_word_is_now_accepted_and_wins_via_inline() {
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "",
            "p",
            Some("word-break: break-all; word-break: break-word"),
        );
        assert_eq!(
            cv.word_break,
            WordBreak::BreakWord,
            "break-word is now implemented and must win over break-all"
        );
    }

    #[test]
    fn word_break_break_word_wins_over_keep_all_in_same_rule() {
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "p { word-break: keep-all; word-break: break-word }",
            "p",
            None,
        );
        assert_eq!(cv.word_break, WordBreak::BreakWord);
    }

    #[test]
    fn word_break_break_word_alone_is_accepted() {
        use crate::property::WordBreak;
        let cv = cascade_doc("", "p", Some("word-break: break-word"));
        assert_eq!(cv.word_break, WordBreak::BreakWord);
        let cv = cascade_doc("p { word-break: break-word }", "p", None);
        assert_eq!(cv.word_break, WordBreak::BreakWord);
    }

    #[test]
    fn word_break_valid_later_overrides_prior_scope_cut_alone() {
        // Control: `break-word` being dropped must not poison a later valid
        // declaration in the same element. `break-word` (dropped) then
        // `break-all` (valid) → `break-all` wins, proving the drop is
        // per-declaration, not per-property.
        use crate::property::WordBreak;
        let cv = cascade_doc(
            "",
            "p",
            Some("word-break: break-word; word-break: break-all"),
        );
        assert_eq!(cv.word_break, WordBreak::BreakAll);
    }

    // ── overflow-wrap / word-wrap legacy alias wire-through (CSS Text 3 §5.4) ──

    #[test]
    fn overflow_wrap_wired_through_cascade_from_inline_style() {
        use crate::property::OverflowWrap;
        let cv = cascade_doc("", "p", Some("overflow-wrap: anywhere"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::Anywhere);
    }

    #[test]
    fn word_wrap_legacy_alias_wired_through_cascade_same_as_overflow_wrap() {
        // CSS Text 3 §5.4 verbatim: "For legacy reasons, UAs must treat
        // word-wrap as a legacy name alias of the overflow-wrap property."
        use crate::property::OverflowWrap;
        let cv = cascade_doc("", "p", Some("word-wrap: break-word"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::BreakWord);
    }

    #[test]
    fn word_wrap_and_overflow_wrap_cascade_against_each_other_as_one_property() {
        // `OverflowWrap` doc's "legacy alias" section: the two names share
        // one `PropertyKey`, so — unlike two genuinely different properties
        // — a later declaration under either name overrides an earlier
        // declaration under the *other* name (CSS Cascading L4 §6.1 "Order
        // of Appearance": "The last declaration in document order wins.",
        // same rule pinned for a single property name by the
        // `later_duplicate_in_inline_wins` sibling test above).
        use crate::property::OverflowWrap;
        let cv = cascade_doc("", "p", Some("overflow-wrap: normal; word-wrap: anywhere"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::Anywhere);
        let cv = cascade_doc("", "p", Some("word-wrap: anywhere; overflow-wrap: normal"));
        assert_eq!(cv.overflow_wrap, OverflowWrap::Normal);
    }

    #[test]
    fn overflow_wrap_inherits_from_parent_element() {
        // CSS Text 3 §5.4: overflow-wrap は **inherited**.
        use crate::property::OverflowWrap;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("overflow-wrap: anywhere"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].overflow_wrap, OverflowWrap::Anywhere);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].overflow_wrap,
            OverflowWrap::Anywhere,
            "child should inherit overflow-wrap from parent (CSS Text 3 §5.4 Inherited: yes)"
        );
    }

    #[test]
    fn overflow_wrap_child_own_value_wins_over_inherited() {
        use crate::property::OverflowWrap;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("overflow-wrap: anywhere"));
        let span = doc.push_element(p, "span", Some("overflow-wrap: normal"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].overflow_wrap, OverflowWrap::Anywhere);
        assert_eq!(r.computed[span].overflow_wrap, OverflowWrap::Normal);
    }

    // ── generic scope-limited cascade consequence ──
    //
    // The word-break / font-style cases above are not unique: every
    // scope-limited-but-spec-valid keyword in this crate follows the same
    // declaration-drop → prior-wins path because the per-property parser
    // returns `None` and `DeclParser` drops the declaration before cascade
    // ever sees it. These three spot-checks check that the same prior-wins
    // behaviour holds for unrelated properties, proving the gap is crate-wide
    // and not a single-property quirk.

    #[test]
    fn white_space_break_spaces_is_now_accepted_and_wins() {
        use crate::property::WhiteSpace;
        let cv = cascade_doc(
            "",
            "p",
            Some("white-space: pre-wrap; white-space: break-spaces"),
        );
        assert_eq!(
            cv.white_space,
            WhiteSpace::BreakSpaces,
            "break-spaces is now implemented and must win over pre-wrap"
        );
        let cv = cascade_doc("", "p", Some("white-space: break-spaces"));
        assert_eq!(cv.white_space, WhiteSpace::BreakSpaces);
    }

    #[test]
    fn vertical_align_top_and_bottom_cascade_as_computed_values() {
        use crate::property::VerticalAlign;
        let cv = cascade_doc(
            "",
            "span",
            Some("vertical-align: baseline; vertical-align: top"),
        );
        assert_eq!(cv.vertical_align, VerticalAlign::Top);
        let cv = cascade_doc("", "span", Some("vertical-align: bottom"));
        assert_eq!(cv.vertical_align, VerticalAlign::Bottom);
    }

    #[test]
    fn text_transform_width_keywords_cascade_as_computed_values() {
        use crate::property::TextTransform;
        let cv = cascade_doc(
            "",
            "p",
            Some("text-transform: uppercase; text-transform: full-width"),
        );
        assert_eq!(cv.text_transform, TextTransform::FullWidth);
        let cv = cascade_doc("", "p", Some("text-transform: uppercase full-width"));
        assert_eq!(cv.text_transform, TextTransform::UppercaseFullWidth);
    }

    // ── letter-spacing / word-spacing wire-through (CSS Text 3 §7.2 / §7.1) ──

    #[test]
    fn letter_spacing_wired_through_cascade_from_inline_style() {
        // `em` (not `px`) so this also exercises phase 3 absolutization
        // (`resolve_length_or_normal`), not just the `apply_value` arm's
        // pass-through assignment.
        let cv = cascade_doc("", "p", Some("letter-spacing: 0.5em"));
        assert_eq!(cv.letter_spacing, ComputedLength(8.0));
    }

    #[test]
    fn word_spacing_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "p", Some("word-spacing: 4px"));
        assert_eq!(cv.word_spacing, ComputedLength(4.0));
    }

    #[test]
    fn word_spacing_ch_provenance_survives_inheritance() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("word-spacing: 1ch"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].word_spacing_ch_factor, Some(1.0));
        assert_eq!(r.computed[span].word_spacing_ch_factor, Some(1.0));
    }

    #[test]
    fn letter_spacing_ch_provenance_survives_inheritance() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("letter-spacing: 1ch"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing_ch_factor, Some(1.0));
        assert_eq!(r.computed[span].letter_spacing_ch_factor, Some(1.0));
    }

    #[test]
    fn letter_spacing_and_word_spacing_inherit_from_parent_element() {
        // CSS Text 3 §7.2 / §7.1: both are **inherited**.
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("letter-spacing: 2px; word-spacing: normal"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing, ComputedLength(2.0));
        assert_eq!(r.computed[p].word_spacing, ComputedLength::ZERO);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].letter_spacing,
            ComputedLength(2.0),
            "child should inherit letter-spacing from parent (CSS Text 3 §7.2 Inherited: yes)"
        );
        assert_eq!(r.computed[span].word_spacing, ComputedLength::ZERO);
    }

    #[test]
    fn letter_spacing_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("letter-spacing: 2px"));
        let span = doc.push_element(p, "span", Some("letter-spacing: -1px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing, ComputedLength(2.0));
        assert_eq!(r.computed[span].letter_spacing, ComputedLength(-1.0));
    }

    /// Inheritance carries the parent's already-**computed** length, not the
    /// specified `em` re-resolved against the child's own font-size (CSS
    /// Cascade 5 §7.2 "Inheritance": inherited values are the parent's
    /// computed values; CSS Text 3 §7.2 "Computed value: an absolute
    /// length"). A px-only fixture (the sibling tests above) can't
    /// distinguish "inherit the computed px" from "inherit the specified
    /// `em`/`px` and re-resolve" — those only diverge when the child's
    /// font-size differs from the parent's, which requires an `em` value.
    #[test]
    fn letter_spacing_inherited_em_value_does_not_re_resolve_against_child_font_size() {
        let mut doc = TestDoc::new();
        // parent: font-size 16px, letter-spacing 0.5em -> computed 8px.
        let p = doc.push_element(0, "p", Some("font-size: 16px; letter-spacing: 0.5em"));
        // child: font-size 32px, no letter-spacing declaration of its own.
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].letter_spacing, ComputedLength(8.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].letter_spacing,
            ComputedLength(8.0),
            "child must inherit the parent's already-computed 8px, not re-resolve \
             0.5em against its own 32px font-size (which would wrongly yield 16px)"
        );
    }

    // ── tab-size wire-through (CSS Text Module Level 3 §4.2) ──

    #[test]
    fn tab_size_number_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "p", Some("tab-size: 4"));
        assert_eq!(cv.tab_size, ComputedTabSize::Number(4.0));
    }

    #[test]
    fn tab_size_length_wired_through_cascade_from_inline_style() {
        // `em` (not `px`) so this also exercises phase 3 absolutization
        // (`resolve_tab_size`), not just the `apply_value` arm's
        // pass-through assignment.
        let cv = cascade_doc("", "p", Some("font-size: 20px; tab-size: 2em"));
        assert_eq!(cv.tab_size, ComputedTabSize::Length(ComputedLength(40.0)));
    }

    #[test]
    fn tab_size_defaults_to_initial_8_when_undeclared() {
        // CSS Text Module Level 3 §4.2: "Initial: 8".
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.tab_size, ComputedTabSize::Number(8.0));
    }

    #[test]
    fn tab_size_inherits_from_parent_element() {
        // CSS Text Module Level 3 §4.2: "Inherited: yes".
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("tab-size: 6"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].tab_size, ComputedTabSize::Number(6.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].tab_size,
            ComputedTabSize::Number(6.0),
            "child should inherit tab-size from parent (CSS Text Module Level 3 §4.2 Inherited: yes)"
        );
    }

    #[test]
    fn tab_size_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("tab-size: 6"));
        let span = doc.push_element(p, "span", Some("tab-size: 2"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].tab_size, ComputedTabSize::Number(6.0));
        assert_eq!(r.computed[span].tab_size, ComputedTabSize::Number(2.0));
    }

    /// Inheritance carries the parent's already-**computed** length, not the
    /// specified `em` re-resolved against the child's own font-size — same
    /// shape as
    /// `letter_spacing_inherited_em_value_does_not_re_resolve_against_child_font_size`
    /// above (CSS Cascade 5 §7.2 "Inheritance": inherited values are the
    /// parent's computed values; CSS Text Module Level 3 §4.2 "Computed
    /// value: the specified number or absolute length").
    #[test]
    fn tab_size_inherited_em_value_does_not_re_resolve_against_child_font_size() {
        let mut doc = TestDoc::new();
        // parent: font-size 16px, tab-size 2em -> computed 32px.
        let p = doc.push_element(0, "p", Some("font-size: 16px; tab-size: 2em"));
        // child: font-size 32px, no tab-size declaration of its own.
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].tab_size,
            ComputedTabSize::Length(ComputedLength(32.0))
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].tab_size,
            ComputedTabSize::Length(ComputedLength(32.0)),
            "child must inherit the parent's already-computed 32px, not re-resolve \
             2em against its own 32px font-size (which would wrongly yield 64px)"
        );
    }

    #[test]
    fn tab_size_number_inherited_by_child_multiplies_own_font_size() {
        // Mirrors `line-height`'s unitless-number inheritance special
        // behavior shape (CSS Text Module Level 3 §4.2's `<number>`
        // alternative carries no length, so unlike the `<length>` case
        // above there is nothing to "re-resolve" — the raw number just
        // passes through unchanged regardless of the child's own font-size).
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-size: 16px; tab-size: 4"));
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].tab_size, ComputedTabSize::Number(4.0));
        assert_eq!(r.computed[span].tab_size, ComputedTabSize::Number(4.0));
    }

    // ── text-shadow wire-through (CSS Text Decoration Module Level 3 §4) ──

    #[test]
    fn text_shadow_wired_through_cascade_from_inline_style() {
        use crate::property::{CssColor, TextShadowColor};
        // `em` (not `px`) so this also exercises phase 3 absolutization
        // (`resolve_text_shadow_item`), not just the `apply_value` arm's
        // pass-through assignment — same rationale as
        // `letter_spacing_wired_through_cascade_from_inline_style`.
        let cv = cascade_doc("", "p", Some("text-shadow: 0.5em 1em red"));
        assert_eq!(
            *cv.text_shadow,
            vec![ComputedTextShadow {
                offset_x: ComputedLength(8.0),
                offset_y: ComputedLength(16.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }]
        );
    }

    #[test]
    fn text_shadow_none_is_empty_computed_list() {
        let cv = cascade_doc("", "p", Some("text-shadow: none"));
        assert!(cv.text_shadow.is_empty());
    }

    #[test]
    fn text_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade() {
        // `0e999` collapses to `NaN` internally during cssparser
        // tokenization (`raikiri-style/src/property.rs` module doc's
        // "Numeric-token NaN stabilization" section), but the acquisition
        // layer recovers the spec-correct `0.0` before
        // `parse_shadow_length_reject_nan`'s `!is_nan()` guard ever runs —
        // so `text-shadow: 0e999px 1px red` parses and cascades
        // successfully, instead of being dropped
        // (`property::tests::text_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero_but_preserves_infinity`
        // pins the parse-layer half of this). This is the end-to-end pin,
        // through the real parse -> cascade pipeline, that `0e999`
        // resolves to `0.0` all the way to `ComputedValues::text_shadow`.
        let cv = cascade_doc("", "p", Some("text-shadow: 0e999px 1px red"));
        assert_eq!(
            *cv.text_shadow,
            vec![ComputedTextShadow {
                offset_x: ComputedLength(0.0),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }]
        );
    }

    #[test]
    fn text_shadow_infinite_offset_passes_through_unclamped_through_real_cascade() {
        // `+Inf` is a *different* hazard class from `0e999`'s NaN collapse
        // above — a spec-valid `<length>` magnitude overflow (CSS Values 4
        // §5), not a `0 * Infinity` collapse — so unlike NaN it must reach
        // `ComputedValues::text_shadow` unrejected (dropping it here would
        // be the same class of regression the opacity guard's own
        // `is_finite()`-vs-`!is_nan()` history warns against).
        let cv = cascade_doc("", "p", Some("text-shadow: 1e40px 1px red"));
        assert_eq!(
            *cv.text_shadow,
            vec![ComputedTextShadow {
                offset_x: ComputedLength(f32::INFINITY),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
            }]
        );

        // Both signs, same reason
        // `opacity_infinite_literal_clamps_through_real_cascade_instead_of_being_dropped`
        // checks both `1e40`/`-1e40`.
        let neg = cascade_doc("", "p", Some("text-shadow: -1e40px 1px red"));
        assert_eq!(
            neg.text_shadow[0].offset_x,
            ComputedLength(f32::NEG_INFINITY)
        );
    }

    // ── border-radius / box-shadow / outline wire-through ────────────────

    #[test]
    fn border_radius_box_shadow_and_outline_compute_through_cascade() {
        let cv = cascade_doc(
            "",
            "p",
            Some(
                "border-radius: 1em 2em 3em 4em; \
                 box-shadow: red 0.5em -1em 0.25em 0.125em, 2px 3px; \
                 outline: solid 2em red",
            ),
        );

        assert_eq!(
            cv.border_radius,
            ComputedBorderRadius {
                top_left: ComputedLengthPercentage::Px(16.0),
                top_right: ComputedLengthPercentage::Px(32.0),
                bottom_right: ComputedLengthPercentage::Px(48.0),
                bottom_left: ComputedLengthPercentage::Px(64.0),
            }
        );
        assert_eq!(
            *cv.box_shadow,
            vec![
                ComputedBoxShadowItem {
                    offset_x: ComputedLength(8.0),
                    offset_y: ComputedLength(-16.0),
                    blur_radius: ComputedLength(4.0),
                    spread_radius: ComputedLength(2.0),
                    color: TextShadowColor::Resolved(CssColor {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    }),
                    inset: false,
                },
                ComputedBoxShadowItem {
                    offset_x: ComputedLength(2.0),
                    offset_y: ComputedLength(3.0),
                    blur_radius: ComputedLength::ZERO,
                    spread_radius: ComputedLength::ZERO,
                    color: TextShadowColor::CurrentColor,
                    inset: false,
                },
            ]
        );
        assert_eq!(cv.outline.width(), ComputedLength(32.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Solid);
        assert_eq!(
            cv.outline.color,
            OutlineColor::Resolved(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            })
        );
    }

    #[test]
    fn box_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade() {
        // Same recovery as
        // `text_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade`
        // above, for `box-shadow`
        // (`property::tests::box_shadow_zero_mantissa_huge_exponent_offset_or_spread_resolves_to_zero_but_preserves_infinity`
        // pins the parse-layer half).
        let cv = cascade_doc("", "p", Some("box-shadow: 0e999px 1px red"));
        assert_eq!(
            *cv.box_shadow,
            vec![ComputedBoxShadowItem {
                offset_x: ComputedLength(0.0),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength::ZERO,
                spread_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                inset: false,
            }]
        );
    }

    #[test]
    fn box_shadow_infinite_offset_passes_through_unclamped_through_real_cascade() {
        // `+Inf` is a *different* hazard class from `0e999`'s NaN collapse
        // above — a spec-valid `<length>` magnitude overflow (CSS Values 4
        // §5), not a `0 * Infinity` collapse — so unlike NaN it must reach
        // `ComputedValues::box_shadow` unrejected.
        let cv = cascade_doc("", "p", Some("box-shadow: 1e40px 1px 1px 1e40px red"));
        assert_eq!(
            *cv.box_shadow,
            vec![ComputedBoxShadowItem {
                offset_x: ComputedLength(f32::INFINITY),
                offset_y: ComputedLength(1.0),
                blur_radius: ComputedLength(1.0),
                spread_radius: ComputedLength(f32::INFINITY),
                color: TextShadowColor::Resolved(CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                }),
                inset: false,
            }]
        );

        // Both signs, same reason
        // `opacity_infinite_literal_clamps_through_real_cascade_instead_of_being_dropped`
        // checks both `1e40`/`-1e40`.
        let neg = cascade_doc("", "p", Some("box-shadow: -1e40px 1px 1px -1e40px red"));
        assert_eq!(
            neg.box_shadow[0].offset_x,
            ComputedLength(f32::NEG_INFINITY)
        );
        assert_eq!(
            neg.box_shadow[0].spread_radius,
            ComputedLength(f32::NEG_INFINITY)
        );
    }

    /// CSS Basic User Interface Module Level 3 §4.3: `outline-style: auto` is
    /// an outline-only keyword. It must survive cascade/computed propagation
    /// without changing the border style type or parser behavior.
    #[test]
    fn outline_auto_cascades_as_distinct_style_from_border() {
        let cv = cascade_doc(
            "",
            "p",
            Some("border-top-width: 2px; border-top-style: solid; outline: auto 2px red"),
        );

        assert_eq!(cv.outline.width(), ComputedLength(2.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Auto);
        assert_eq!(cv.border.top.width, ComputedLength(2.0));
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
    }

    #[test]
    fn outline_none_gates_computed_width_to_zero() {
        let cv = cascade_doc("", "p", Some("outline: none 2em red"));
        assert_eq!(cv.outline.width(), ComputedLength::ZERO);
        assert_eq!(cv.outline.style(), OutlineStyle::None);
        assert_eq!(cv.outline.color, OutlineColor::Resolved(RED));
    }

    #[test]
    fn outline_color_invert_cascades_as_a_distinct_keyword() {
        let cv = cascade_doc("", "p", Some("outline-color: invert"));
        assert_eq!(cv.outline.color, OutlineColor::Invert);

        let cv = cascade_doc("", "p", Some("outline-color: currentcolor"));
        assert_eq!(cv.outline.color, OutlineColor::CurrentColor);
    }

    #[test]
    fn border_radius_box_shadow_and_outline_are_non_inherited() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(
            0,
            "p",
            Some("border-radius: 1px; box-shadow: 1px 2px red; outline: solid 3px red"),
        );
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        let initial = ComputedValues::initial();

        assert_ne!(result.computed[parent].border_radius, initial.border_radius);
        assert!(!result.computed[parent].box_shadow.is_empty());
        assert_ne!(result.computed[parent].outline, initial.outline);
        assert_eq!(result.computed[child].border_radius, initial.border_radius);
        assert_eq!(result.computed[child].box_shadow, initial.box_shadow);
        assert_eq!(result.computed[child].outline, initial.outline);
    }

    #[test]
    fn text_shadow_inherits_from_parent_element() {
        // CSS Text Decoration Module Level 3 §4: **inherited**.
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-shadow: 1px 1px black"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_shadow.len(), 1);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_shadow, r.computed[p].text_shadow,
            "child should inherit text-shadow from parent (CSS Text Decoration \
             Module Level 3 §4 Inherited: yes)"
        );
    }

    #[test]
    fn text_shadow_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-shadow: 1px 1px black"));
        let span = doc.push_element(p, "span", Some("text-shadow: none"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_shadow.len(), 1);
        assert!(r.computed[span].text_shadow.is_empty());
    }

    /// Inheritance carries the parent's already-**computed** lengths, not the
    /// specified `em` re-resolved against the child's own font-size — same
    /// shape as `letter_spacing_inherited_em_value_does_not_re_resolve_against_child_font_size`.
    #[test]
    fn text_shadow_inherited_em_value_does_not_re_resolve_against_child_font_size() {
        let mut doc = TestDoc::new();
        // parent: font-size 16px, text-shadow 0.5em -> computed 8px.
        let p = doc.push_element(0, "p", Some("font-size: 16px; text-shadow: 0.5em 0.5em"));
        // child: font-size 32px, no text-shadow declaration of its own.
        let span = doc.push_element(p, "span", Some("font-size: 32px"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].text_shadow[0].offset_x, ComputedLength(8.0));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_shadow[0].offset_x,
            ComputedLength(8.0),
            "child must inherit the parent's already-computed 8px, not re-resolve \
             0.5em against its own 32px font-size (which would wrongly yield 16px)"
        );
    }

    // ── white-space wire-through (CSS Text 3 §3) ──

    #[test]
    fn white_space_wired_through_cascade_from_inline_style() {
        use crate::property::WhiteSpace;
        let cv = cascade_doc("", "p", Some("white-space: pre"));
        assert_eq!(cv.white_space, WhiteSpace::Pre);
    }

    #[test]
    fn white_space_inherits_from_parent_element() {
        // CSS Text 3 §3: white-space は **inherited**.
        use crate::property::WhiteSpace;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("white-space: pre"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].white_space, WhiteSpace::Pre);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].white_space,
            WhiteSpace::Pre,
            "child should inherit white-space from parent (CSS Text 3 §3 Inherited: yes)"
        );
    }

    #[test]
    fn white_space_child_own_value_wins_over_inherited() {
        use crate::property::WhiteSpace;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("white-space: pre"));
        let span = doc.push_element(p, "span", Some("white-space: nowrap"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].white_space, WhiteSpace::Pre);
        assert_eq!(r.computed[span].white_space, WhiteSpace::Nowrap);
    }

    // ── hyphens wire-through (CSS Text 3 §5.3) ──

    #[test]
    fn hyphens_wired_through_cascade_from_inline_style() {
        use crate::property::Hyphens;
        let cv = cascade_doc("", "p", Some("hyphens: auto"));
        assert_eq!(cv.hyphens, Hyphens::Auto);
    }

    #[test]
    fn hyphens_inherits_from_parent_element() {
        // CSS Text 3 §5.3: hyphens は **inherited**.
        use crate::property::Hyphens;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("hyphens: auto"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].hyphens, Hyphens::Auto);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].hyphens,
            Hyphens::Auto,
            "child should inherit hyphens from parent (CSS Text 3 §5.3 Inherited: yes)"
        );
    }

    #[test]
    fn hyphens_child_own_value_wins_over_inherited() {
        use crate::property::Hyphens;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("hyphens: auto"));
        let span = doc.push_element(p, "span", Some("hyphens: none"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].hyphens, Hyphens::Auto);
        assert_eq!(r.computed[span].hyphens, Hyphens::None);
    }

    // ── orphans / widows wire-through (CSS Fragmentation Module Level 3
    //    §3.3) ──

    // ── writing-mode wire-through (CSS Writing Modes 4 §3.2) ──
    // Future work: vertical writing-mode 実装時に
    // `resolve_writing_mode` の collapse を削除したら、本 test の期待値を
    // `VerticalRl` へ戻すこと。

    #[test]
    fn writing_mode_wired_through_cascade_from_inline_style() {
        use crate::property::WritingMode;
        let cv = cascade_doc("", "p", Some("writing-mode: vertical-rl"));
        // `apply_value`'s `WritingMode` arm assigns the raw specified
        // keyword; the `HorizontalTb` collapse for non-horizontal keywords
        // happens later in `absolutize_with` (see `apply_value`'s
        // `PropertyValue::WritingMode` arm doc comment), so the computed
        // value here is always `HorizontalTb` even for `vertical-rl`.
        // Future work: vertical writing 実装時に
        // `HorizontalTb` 期待値を `VerticalRl` へ戻すこと。
        assert_eq!(cv.writing_mode, WritingMode::HorizontalTb);
    }

    // ── background-repeat/attachment/clip/origin/size/position wire-through
    // (CSS Backgrounds and Borders 3 §2.4-§2.9) ──

    #[test]
    fn authored_writing_mode_is_retained_before_computed_normalization() {
        use crate::property::WritingMode;
        let mut doc = TestDoc::new();
        let root = doc.push_element(0, "html", Some("writing-mode: vertical-rl"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.authored_writing_modes[root],
            Some(WritingMode::VerticalRl)
        );
        assert_eq!(
            result.computed[root].writing_mode,
            WritingMode::HorizontalTb
        );
    }

    #[test]
    fn page_margin_inherit_uses_the_html_root_element() {
        let mut doc = TestDoc::new();
        let html = doc.push_element(0, "html", Some("margin: 0.5in"));
        let style = doc.push_element(html, "style", None);
        doc.push_text(style, "@page { margin: 13px; margin: inherit }");
        let tree = build_rule_tree(&doc);
        let result = cascade_with_media_context_for_page(
            &doc,
            &tree,
            &MediaContext::default(),
            &crate::page::PageContextQuery::default(),
        )
        .expect("page cascade Ok");
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginTop),
            Some(&PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
                48.0
            ))))
        );
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginRight),
            Some(&PropertyValue::MarginRight(LengthOrAuto::Length(
                Length::Px(48.0)
            )))
        );
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginBottom),
            Some(&PropertyValue::MarginBottom(LengthOrAuto::Length(
                Length::Px(48.0)
            )))
        );
        assert_eq!(
            result.page.declarations().get(&PropertyKey::MarginLeft),
            Some(&PropertyValue::MarginLeft(LengthOrAuto::Length(
                Length::Px(48.0)
            )))
        );
    }

    #[test]
    fn page_margin_inherit_preserves_computed_margin_value_shapes() {
        let mut root = ComputedValues::initial();
        root.margin = Sides {
            top: ComputedLengthPercentageOrAuto::Auto,
            right: ComputedLengthPercentageOrAuto::Px(12.0),
            bottom: ComputedLengthPercentageOrAuto::Percent(25.0),
            left: ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
                percent: 10.0,
                px: 2.0,
            }),
        };
        let ctx = ResolveContext::new(root.font_size);
        let cases = [
            (
                PropertyValue::MarginTopInherit,
                PropertyValue::MarginTop(LengthOrAuto::Auto),
            ),
            (
                PropertyValue::MarginRightInherit,
                PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(12.0))),
            ),
            (
                PropertyValue::MarginBottomInherit,
                PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Percent(25.0))),
            ),
            (
                PropertyValue::MarginLeftInherit,
                PropertyValue::MarginLeft(LengthOrAuto::Calc(CalcLengthPercentage {
                    percent: 10.0,
                    px: 2.0,
                })),
            ),
        ];
        for (marker, expected) in cases {
            assert_eq!(
                resolve_against_inherited(marker, &root, &ctx).into_property_value(),
                expected
            );
        }
        assert_eq!(
            resolve_against_inherited(PropertyValue::MarginInherit, &root, &ctx)
                .into_property_value(),
            PropertyValue::Margin(root.margin.map(|value| match value {
                ComputedLengthPercentageOrAuto::Px(px) => LengthOrAuto::Length(Length::Px(px)),
                ComputedLengthPercentageOrAuto::Percent(percent) => {
                    LengthOrAuto::Length(Length::Percent(percent))
                }
                ComputedLengthPercentageOrAuto::Calc(calc) => LengthOrAuto::Calc(calc),
                ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            }))
        );
    }

    #[test]
    fn background_repeat_attachment_clip_origin_wired_through_cascade_from_inline_style() {
        use crate::property::{
            BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, VisualBox,
        };
        let cv = cascade_doc(
            "",
            "div",
            Some(
                "background-repeat: repeat-x; background-attachment: fixed; \
                 background-clip: content-box; background-origin: border-box",
            ),
        );
        assert_eq!(
            cv.background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::NoRepeat,
            }
        );
        assert_eq!(cv.background_attachment, BackgroundAttachment::Fixed);
        assert_eq!(cv.background_clip, VisualBox::ContentBox);
        assert_eq!(cv.background_origin, VisualBox::BorderBox);
    }

    #[test]
    fn background_repeat_attachment_clip_origin_are_non_inherited() {
        use crate::property::{
            BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, VisualBox,
        };
        let mut doc = TestDoc::new();
        let p = doc.push_element(
            0,
            "p",
            Some(
                "background-repeat: round; background-attachment: local; \
                 background-clip: content-box; background-origin: content-box",
            ),
        );
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Round,
                y: BackgroundRepeatKeyword::Round,
            }
        );
        // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8 "Inherited: no" — the
        // child without its own winner resets to each property's spec
        // initial, not the parent's value.
        assert_eq!(
            r.computed[span].background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            }
        );
        assert_eq!(
            r.computed[p].background_attachment,
            BackgroundAttachment::Local
        );
        assert_eq!(
            r.computed[span].background_attachment,
            BackgroundAttachment::Scroll
        );
        assert_eq!(r.computed[p].background_clip, VisualBox::ContentBox);
        assert_eq!(r.computed[span].background_clip, VisualBox::BorderBox);
        assert_eq!(r.computed[p].background_origin, VisualBox::ContentBox);
        // `background-origin`'s initial (`padding-box`) differs from
        // `background-clip`'s (`border-box`) — check both distinctly.
        assert_eq!(r.computed[span].background_origin, VisualBox::PaddingBox);
    }

    #[test]
    fn background_size_wired_through_cascade_and_absolutizes_em() {
        use crate::resolve::{ComputedBackgroundSize, ComputedLengthPercentageOrAuto};
        // `2em` at the default 16px font-size absolutizes to 32px; the 2nd
        // axis is omitted so it fills with `auto` (not a duplicate of the
        // 1st, per `BackgroundSize` doc's fill-rule note).
        let cv = cascade_doc("", "div", Some("background-size: 2em"));
        assert_eq!(
            cv.background_size,
            ComputedBackgroundSize::Explicit {
                width: ComputedLengthPercentageOrAuto::Px(32.0),
                height: ComputedLengthPercentageOrAuto::Auto,
            }
        );
    }

    #[test]
    fn background_size_is_non_inherited() {
        use crate::resolve::ComputedBackgroundSize;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-size: cover"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].background_size, ComputedBackgroundSize::Cover);
        assert_eq!(
            r.computed[span].background_size,
            ComputedValues::initial().background_size
        );
    }

    #[test]
    fn background_position_wired_through_cascade_and_absolutizes_edge_offset() {
        use crate::resolve::{
            ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
        };
        // `bottom 1em right` — `1em` absolutizes to 16px at the default
        // font-size, `right`'s omitted offset defaults to 0 and normalizes
        // to `Start(100%)` (`CssPositionOffset` doc's normalization note).
        let cv = cascade_doc("", "div", Some("background-position: bottom 1em right"));
        assert_eq!(
            cv.background_position,
            ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    100.0
                )),
                vertical: ComputedCssPositionOffset::End(ComputedLengthPercentage::Px(16.0)),
            }
        );
    }

    #[test]
    fn background_position_is_non_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-position: right bottom"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_ne!(
            r.computed[p].background_position,
            ComputedValues::initial().background_position
        );
        assert_eq!(
            r.computed[span].background_position,
            ComputedValues::initial().background_position
        );
    }

    // ── object-fit / object-position wire-through
    // (CSS Images Module Level 3 §5.1/§5.2) ──

    #[test]
    fn object_fit_wired_through_cascade_from_inline_style() {
        use crate::property::ObjectFit;
        let cv = cascade_doc("", "img", Some("object-fit: contain"));
        assert_eq!(cv.object_fit, ObjectFit::Contain);
    }

    #[test]
    fn object_fit_defaults_to_fill_without_declaration() {
        use crate::property::ObjectFit;
        let cv = cascade_doc("", "img", None);
        assert_eq!(cv.object_fit, ObjectFit::Fill);
    }

    #[test]
    fn object_fit_is_non_inherited() {
        use crate::property::ObjectFit;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("object-fit: cover"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].object_fit, ObjectFit::Cover);
        // CSS Images Module Level 3 §5.1 "Inherited: no" — the child without
        // its own winner resets to the spec initial, not the parent's value
        // (`background_position_is_non_inherited` sibling shape above).
        assert_eq!(r.computed[span].object_fit, ObjectFit::Fill);
    }

    #[test]
    fn object_position_wired_through_cascade_and_absolutizes_em() {
        use crate::resolve::{
            ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
        };
        // `2em` at the default 16px font-size absolutizes to 32px.
        let cv = cascade_doc("", "img", Some("object-position: 2em 10%"));
        assert_eq!(
            cv.object_position,
            ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(32.0)),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(10.0)),
            }
        );
    }

    #[test]
    fn object_position_defaults_to_50_percent_50_percent_without_declaration() {
        // CSS Images Module Level 3 §5.2 "Initial: 50% 50%" — distinct from
        // `background-position`'s `0% 0%` initial
        // (`background_position_is_non_inherited` sibling above resets to
        // `0% 0%`; this property resets to `50% 50%` instead).
        use crate::resolve::{
            ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
        };
        let cv = cascade_doc("", "img", None);
        assert_eq!(
            cv.object_position,
            ComputedCssPosition {
                horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(
                    50.0
                )),
                vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(50.0)),
            }
        );
    }

    #[test]
    fn object_position_is_non_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("object-position: right bottom"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_ne!(
            r.computed[p].object_position,
            ComputedValues::initial().object_position
        );
        // CSS Images Module Level 3 §5.2 "Inherited: no" — the child without
        // its own winner resets to the spec initial (`50% 50%`), not the
        // parent's value.
        assert_eq!(
            r.computed[span].object_position,
            ComputedValues::initial().object_position
        );
    }

    #[test]
    fn object_position_3_value_edge_offset_form_dropped_through_real_cascade() {
        // `right 10px center` is `<bg-position>`'s (CSS Backgrounds 3 §2.6)
        // 3-value extension, not valid for `object-position`'s plain
        // `<position>` (CSS Values 4 §8.3) —
        // `object_position_rejects_bg_position_only_3_value_edge_offset_forms`
        // (property.rs) pins this at the `parse_value` level; this is the
        // end-to-end sibling through the real parse -> cascade pipeline
        // (`rule::DeclParser`'s `expect_exhausted` drops the whole
        // declaration once `parse_position_strict`'s fallback alternative
        // leaves `center` as leftover, same mechanism as the
        // `background_shorthand_*_dropped_through_real_cascade` tests).
        let cv = cascade_doc("", "img", Some("object-position: right 10px center"));
        assert_eq!(
            cv.object_position,
            ComputedValues::initial().object_position
        );
    }

    // ── opacity wire-through (CSS Color 4 §3.3) ──

    #[test]
    fn opacity_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "div", Some("opacity: 0.5"));
        assert_eq!(cv.opacity, 0.5);
    }

    #[test]
    fn opacity_defaults_to_1_without_declaration() {
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.opacity, 1.0);
    }

    #[test]
    fn opacity_is_non_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("opacity: 0.3"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].opacity, 0.3);
        // CSS Color 4 §3.3 "Inherited: no" — the child without its own
        // winner resets to the spec initial (`1`), not the parent's value
        // (`object_fit_is_non_inherited` sibling shape above).
        assert_eq!(r.computed[span].opacity, 1.0);
    }

    #[test]
    fn opacity_out_of_range_clamps_to_0_1_through_real_cascade() {
        // CSS Color 4 §3.3: "clamped to the range `[0, 1]` in computed
        // values" — the clamp is a phase-3 transform
        // (`SpecifiedValues::absolutize_with`), pinned end-to-end here
        // through the real parse -> cascade pipeline (unit-level check at
        // `SpecifiedValues::finalize` itself is
        // `opacity_out_of_range_specified_clamps_at_finalize` in
        // `specified.rs`).
        let over = cascade_doc("", "div", Some("opacity: 2"));
        assert_eq!(over.opacity, 1.0);
        let under = cascade_doc("", "div", Some("opacity: -3"));
        assert_eq!(under.opacity, 0.0);
    }

    #[test]
    fn opacity_percentage_wired_through_cascade_and_clamps() {
        // `150%` exercises the percentage branch of `<opacity-value>` (CSS
        // Color 4 §3.3) through the same clamp, a distinct code path from
        // the bare-`<number>` case above.
        let cv = cascade_doc("", "div", Some("opacity: 150%"));
        assert_eq!(cv.opacity, 1.0);
        let half = cascade_doc("", "div", Some("opacity: 50%"));
        assert_eq!(half.opacity, 0.5);
    }

    #[test]
    fn opacity_zero_mantissa_huge_exponent_resolves_through_real_cascade() {
        // `0e999` collapses to `NaN` internally during cssparser
        // tokenization (`raikiri-style/src/property.rs` module doc's
        // "Numeric-token NaN stabilization" section), but the acquisition
        // layer recovers the spec-correct `0.0` before `parse_opacity_value`'s
        // `!is_nan()` guard ever runs — so `opacity: 0e999` parses and
        // cascades successfully to `0.0`, not dropped back to the initial
        // `1.0`
        // (`property::tests::opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity`
        // pins the parse-layer half of this). This is the end-to-end pin,
        // through the real parse -> cascade pipeline, that `0e999` reaches
        // `ComputedValues::opacity` as `0.0`.
        let cv = cascade_doc("", "div", Some("opacity: 0e999"));
        assert_eq!(cv.opacity, 0.0);
        assert!(!cv.opacity.is_nan());
    }

    #[test]
    fn opacity_infinite_literal_clamps_through_real_cascade_instead_of_being_dropped() {
        // `1e40`/`-1e40` overflow to `+Inf`/`-Inf` during tokenization — a
        // *different* hazard class from `0e999`'s NaN collapse above
        // (`parse_opacity_value` doc's "`!is_nan()` guard" section). Unlike
        // NaN, these are spec-valid `<number>` values that must reach the
        // phase-3 clamp and become `1.0`/`0.0` — not be dropped and fall
        // back to the initial `1.0` (which would silently turn a
        // fully-transparent `-1e40` into fully opaque, a regression an
        // earlier iteration of the parse-time guard introduced by using
        // `is_finite()` instead of `!is_nan()`).
        let over = cascade_doc("", "div", Some("opacity: 1e40"));
        assert_eq!(over.opacity, 1.0);
        let under = cascade_doc("", "div", Some("opacity: -1e40"));
        assert_eq!(under.opacity, 0.0);
    }

    // ── isolation / mix-blend-mode wire-through (CSS Compositing and
    // Blending Level 1 §3.4.1/§3.4.2) ──

    #[test]
    fn isolation_wired_through_cascade_from_inline_style() {
        use crate::property::Isolation;
        let cv = cascade_doc("", "div", Some("isolation: isolate"));
        assert_eq!(cv.isolation, Isolation::Isolate);
    }

    #[test]
    fn isolation_defaults_to_auto_without_declaration() {
        use crate::property::Isolation;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.isolation, Isolation::Auto);
    }

    #[test]
    fn isolation_is_non_inherited() {
        use crate::property::Isolation;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("isolation: isolate"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].isolation, Isolation::Isolate);
        assert_eq!(r.computed[span].isolation, Isolation::Auto);
    }

    #[test]
    fn mix_blend_mode_wired_through_cascade_from_inline_style() {
        use crate::property::MixBlendMode;
        let cv = cascade_doc("", "div", Some("mix-blend-mode: multiply"));
        assert_eq!(cv.mix_blend_mode, MixBlendMode::Multiply);
    }

    #[test]
    fn mix_blend_mode_defaults_to_normal_without_declaration() {
        use crate::property::MixBlendMode;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.mix_blend_mode, MixBlendMode::Normal);
    }

    #[test]
    fn mix_blend_mode_is_non_inherited() {
        use crate::property::MixBlendMode;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("mix-blend-mode: screen"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].mix_blend_mode, MixBlendMode::Screen);
        assert_eq!(r.computed[span].mix_blend_mode, MixBlendMode::Normal);
    }

    // ── mask-image / clip-path wire-through (CSS Masking Level 1
    // §7.1/§5.1) ──

    #[test]
    fn mask_image_wired_through_cascade_from_inline_style() {
        use crate::property::MaskImage;
        let cv = cascade_doc("", "div", Some("mask-image: url(mask.svg)"));
        assert_eq!(cv.mask_image, MaskImage::Url("mask.svg".to_string()));
    }

    #[test]
    fn mask_image_defaults_to_none_without_declaration() {
        use crate::property::MaskImage;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.mask_image, MaskImage::None);
    }

    #[test]
    fn mask_image_is_non_inherited() {
        use crate::property::MaskImage;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("mask-image: url(mask.svg)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].mask_image,
            MaskImage::Url("mask.svg".to_string())
        );
        assert_eq!(r.computed[span].mask_image, MaskImage::None);
    }

    #[test]
    fn clip_path_wired_through_cascade_from_inline_style() {
        use crate::property::{ClipPath, GeometryBox};
        let cv = cascade_doc("", "div", Some("clip-path: padding-box"));
        assert_eq!(cv.clip_path, ClipPath::GeometryBox(GeometryBox::PaddingBox));
    }

    #[test]
    fn clip_path_defaults_to_none_without_declaration() {
        use crate::property::ClipPath;
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.clip_path, ClipPath::None);
    }

    #[test]
    fn clip_path_is_non_inherited() {
        use crate::property::{ClipPath, GeometryBox};
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("clip-path: border-box"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].clip_path,
            ClipPath::GeometryBox(GeometryBox::BorderBox)
        );
        assert_eq!(r.computed[span].clip_path, ClipPath::None);
    }

    // ── transform / filter wire-through (CSS Transforms Level 1 §4, CSS
    // Filter Effects Level 1 §5) ──

    #[test]
    fn transform_wired_through_cascade_from_inline_style() {
        use crate::property::Angle;
        use crate::resolve::ComputedTransformFunction;
        let cv = cascade_doc("", "div", Some("transform: rotate(45deg)"));
        assert_eq!(
            *cv.transform,
            vec![ComputedTransformFunction::Rotate(Angle(45.0))]
        );
    }

    #[test]
    fn transform_defaults_to_none_without_declaration() {
        let cv = cascade_doc("", "div", None);
        assert!(cv.transform.is_empty());
    }

    #[test]
    fn transform_is_non_inherited() {
        use crate::property::Angle;
        use crate::resolve::ComputedTransformFunction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("transform: rotate(45deg)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            *r.computed[p].transform,
            vec![ComputedTransformFunction::Rotate(Angle(45.0))]
        );
        assert!(r.computed[span].transform.is_empty());
    }

    #[test]
    fn filter_wired_through_cascade_from_inline_style() {
        use crate::property::FilterFunction;
        let cv = cascade_doc("", "div", Some("filter: blur(2px)"));
        assert_eq!(
            *cv.filter,
            vec![FilterFunction::Blur(crate::property::Length::Px(2.0))]
        );
    }

    #[test]
    fn filter_defaults_to_none_without_declaration() {
        let cv = cascade_doc("", "div", None);
        assert!(cv.filter.is_empty());
    }

    #[test]
    fn filter_is_non_inherited() {
        use crate::property::FilterFunction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("filter: blur(2px)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            *r.computed[p].filter,
            vec![FilterFunction::Blur(crate::property::Length::Px(2.0))]
        );
        assert!(r.computed[span].filter.is_empty());
    }

    #[test]
    fn filter_drop_shadow_zero_mantissa_huge_exponent_offset_resolves_through_real_cascade() {
        use crate::property::{FilterFunction, TextShadowItem};
        // `0e999` collapses to `NaN` internally during cssparser
        // tokenization, but the acquisition layer recovers the
        // spec-correct `0.0` before `parse_shadow_length_reject_nan`'s
        // `!is_nan()` guard ever runs (reused verbatim by
        // `parse_drop_shadow_args`) — so `filter: drop-shadow(0e999px 2px)`
        // parses and cascades successfully, instead of being dropped
        // (`property::tests::filter_drop_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero`
        // pins the parse-layer half of this). This is the end-to-end pin,
        // through the real parse -> cascade pipeline, that `0e999`
        // resolves to `0.0` all the way to `ComputedValues::filter`.
        let cv = cascade_doc("", "div", Some("filter: drop-shadow(0e999px 2px)"));
        assert_eq!(
            *cv.filter,
            vec![FilterFunction::DropShadow(TextShadowItem {
                offset_x: Length::Px(0.0),
                offset_y: Length::Px(2.0),
                blur_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
            })]
        );
    }

    #[test]
    fn filter_drop_shadow_infinite_offset_passes_through_unclamped_through_real_cascade() {
        use crate::property::{FilterFunction, TextShadowItem};
        // `+Inf` is a *different* hazard class from `0e999`'s NaN collapse
        // above — a spec-valid `<length>` magnitude overflow (CSS Values 4
        // §5), not a `0 * Infinity` collapse — so unlike NaN it must reach
        // `ComputedValues::filter` unrejected. `filter` is not
        // independently absolutized at phase 3 (unlike `text-shadow`/
        // `box-shadow`), so the payload stays the specified `Length`/
        // `TextShadowItem` shape all the way through.
        let cv = cascade_doc("", "div", Some("filter: drop-shadow(1e40px 2px)"));
        assert_eq!(
            *cv.filter,
            vec![FilterFunction::DropShadow(TextShadowItem {
                offset_x: Length::Px(f32::INFINITY),
                offset_y: Length::Px(2.0),
                blur_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
            })]
        );
    }

    // ── background-image wire-through (CSS Backgrounds and Borders 3 §2.3) ──

    #[test]
    fn background_image_wired_through_cascade_from_inline_style() {
        use crate::property::BackgroundImage;
        let cv = cascade_doc("", "div", Some("background-image: url(marble.svg)"));
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("marble.svg".to_string())
        );
    }

    #[test]
    fn background_image_defaults_to_none_without_declaration() {
        // No `background-image` declaration at all — spec initial (`none`).
        // Complements `background_image_explicit_none_overrides_an_earlier_url`
        // below, which exercises the *parsed* `none` keyword end-to-end
        // rather than just the no-winner default.
        use crate::property::BackgroundImage;
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.background_image, BackgroundImage::None);
    }

    #[test]
    fn background_image_explicit_none_overrides_an_earlier_url() {
        // Same-block later-declaration-wins (CSS Cascading L4 §"Cascade
        // Sort Order", same mechanism `cascade_doc("p { color: red; color:
        // blue }")` pins for `color`) — this is the only test that reaches
        // `parse_background_image`'s `none` keyword branch *and* observes
        // it survive the cascade, rather than merely matching the initial
        // value that a missing declaration would also produce.
        use crate::property::BackgroundImage;
        let cv = cascade_doc(
            "",
            "div",
            Some("background-image: url(a.png); background-image: none"),
        );
        assert_eq!(cv.background_image, BackgroundImage::None);
    }

    #[test]
    fn background_image_is_non_inherited() {
        use crate::property::BackgroundImage;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("background-image: url(marble.svg)"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].background_image,
            BackgroundImage::Url("marble.svg".to_string())
        );
        // CSS Backgrounds and Borders 3 §2.3 "Inherited: no" — the child
        // without its own winner resets to the spec initial (`none`), not
        // the parent's value.
        assert_eq!(r.computed[span].background_image, BackgroundImage::None);
    }

    #[test]
    fn background_image_wired_through_cascade_from_inline_style_with_gradient() {
        use crate::property::{BackgroundImage, Gradient};
        // `<gradient>` (`linear-gradient()` etc.) wires through the cascade
        // like any other `BackgroundImage` payload — no dedicated cascade.rs
        // match arm exists for it (`apply_value`'s `BackgroundImage(v) =>
        // target.background_image = v` arm takes any payload via wildcard),
        // so this pins the integration rather than exercising new cascade logic.
        let cv = cascade_doc(
            "",
            "div",
            Some("background-image: linear-gradient(red, blue)"),
        );
        assert!(matches!(
            cv.background_image,
            BackgroundImage::Gradient(Gradient::Linear(_))
        ));
    }

    #[test]
    fn background_image_invalid_gradient_does_not_overwrite_an_earlier_url() {
        // Mirrors `background_image_explicit_none_overrides_an_earlier_url`'s
        // same-block later-declaration-wins setup, but with a *syntactically
        // invalid* later declaration (a single-stop `linear-gradient()` —
        // the CSS Images 3 baseline grammar this crate implements requires
        // 2+ stops, `BackgroundImage` doc's scope-carving section) instead
        // of a valid one: the whole invalid declaration must drop, leaving
        // the earlier `url(...)` winner untouched, not coerced to the
        // property's initial value.
        use crate::property::BackgroundImage;
        let cv = cascade_doc(
            "",
            "div",
            Some("background-image: url(a.png); background-image: linear-gradient(red)"),
        );
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("a.png".to_string())
        );
    }

    #[test]
    fn background_image_radial_gradient_position_before_shape_does_not_overwrite_an_earlier_url() {
        // Same shape as `background_image_invalid_gradient_does_not_overwrite_an_earlier_url`,
        // but with a different flavor of syntactically-invalid gradient:
        // `at center circle` violates CSS Images 4 §3.2.1's `[ [
        // <radial-shape> || <radial-size> ]? [ at <position> ]? ]`
        // sequencing (`at <position>` may only follow the shape/size group,
        // never precede it) — without this test, a regression that widens
        // `parse_radial_gradient_body` back to a flat any-order loop over
        // shape/size/position (rather than treating shape/size/position as
        // one ordered group) would *accept* this declaration and overwrite
        // the earlier `url(...)` winner with a spec-invalid gradient,
        // silently corrupting the cascade result instead of failing loudly.
        use crate::property::BackgroundImage;
        let cv = cascade_doc(
            "",
            "div",
            Some(
                "background-image: url(a.png); background-image: radial-gradient(at center circle, red, blue)",
            ),
        );
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("a.png".to_string())
        );
    }

    #[test]
    fn orphans_widows_wired_through_cascade_from_inline_style() {
        let cv = cascade_doc("", "p", Some("orphans: 4; widows: 3"));
        assert_eq!(cv.orphans, 4);
        assert_eq!(cv.widows, 3);
    }

    #[test]
    fn orphans_widows_inherit_from_parent_element() {
        // CSS Fragmentation Module Level 3 §3.3: orphans / widows は
        // **inherited**.
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("orphans: 4; widows: 3"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].orphans, 4);
        assert_eq!(r.computed[p].widows, 3);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].orphans, 4,
            "child should inherit orphans from parent (CSS Fragmentation \
             Module Level 3 §3.3 Inherited: yes)"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].widows, 3,
            "child should inherit widows from parent (CSS Fragmentation \
             Module Level 3 §3.3 Inherited: yes)"
        );
    }

    #[test]
    fn orphans_widows_child_own_value_wins_over_inherited() {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("orphans: 4; widows: 3"));
        let span = doc.push_element(p, "span", Some("orphans: 6; widows: 5"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].orphans, 4);
        assert_eq!(r.computed[p].widows, 3);
        assert_eq!(r.computed[span].orphans, 6);
        assert_eq!(r.computed[span].widows, 5);
    }

    #[test]
    fn orphans_widows_default_to_initial_value_2_without_declaration() {
        // CSS Fragmentation Module Level 3 §3.3: Initial は共に `2`。
        let cv = cascade_doc("", "p", None);
        assert_eq!(cv.orphans, 2);
        assert_eq!(cv.widows, 2);
    }

    #[test]
    fn orphans_widows_reject_zero_and_negative_leaving_initial_value() {
        // "Negative values and zero are invalid and must cause the
        // declaration to be ignored" — the whole declaration drops, so the
        // property stays at its initial value `2` rather than being clamped.
        let cv = cascade_doc("", "p", Some("orphans: 0; widows: -1"));
        assert_eq!(cv.orphans, 2);
        assert_eq!(cv.widows, 2);
    }

    #[test]
    fn orphans_widows_invalid_declaration_is_dropped_independently_of_sibling() {
        // The spec says "the declaration" (singular) is ignored — this pins
        // that an invalid `orphans` doesn't also take down a syntactically
        // valid, separately-declared `widows` in the same block (per-
        // declaration drop, not per-block).
        let cv = cascade_doc("", "p", Some("orphans: 0; widows: 3"));
        assert_eq!(cv.orphans, 2);
        assert_eq!(cv.widows, 3);
    }

    // ── text-align: match-parent (CSS Text 3 §6.1) ──
    //
    // Before
    // this, `TextAlign::MatchParent` reached `ComputedValues.text_align`
    // unresolved (raikiri had no `direction` in the computed layer). These
    // tests exercise the *full* `cascade()` pipeline end-to-end, complementing
    // the `SpecifiedValues::finalize` unit tests in `specified.rs`.

    #[test]
    fn text_align_match_parent_resolves_start_against_ltr_parent_to_left() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        // Parent: `text-align: start` explicit, `direction` defaults to `ltr`.
        let p = doc.push_element(0, "p", Some("text-align: start"));
        let span = doc.push_element(p, "span", Some("text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Left,
            "start + ltr → left (CSS Text 3 §6.1 match-parent table)"
        );
    }

    /// **The end-to-end check for the whole `direction` + `text-align:
    /// match-parent` design.** The child declares *both* `direction: rtl`
    /// and `text-align: match-parent` on itself. Per CSS Text 3 §6.1
    /// `#valdef-text-align-match-parent` ("interpreted against **the
    /// parent's** direction value"), the resolution must use the parent's
    /// `ltr`, not the child's own `rtl`. This is the same invariant
    /// `specified::tests::finalize_match_parent_uses_parent_direction_not_own_direction_winner`
    /// pins at the `SpecifiedValues` unit level; this test additionally
    /// proves the integration through `PropertyKey` winner selection and
    /// `apply_winners`' declaration-order walk, so a regression that
    /// resurfaces the same-node winner-order hazard through a different path
    /// (not just `SpecifiedValues::finalize`) would be caught here too.
    #[test]
    fn text_align_match_parent_uses_parent_direction_not_own_declared_direction() {
        use crate::property::{Direction, TextAlign};
        let mut doc = TestDoc::new();
        // Parent: direction defaults to ltr, text-align defaults to start.
        let p = doc.push_element(0, "p", None);
        // Child: declares its own (conflicting) direction *and* match-parent.
        let span = doc.push_element(p, "span", Some("direction: rtl; text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Ltr);
        assert_eq!(r.computed[p].text_align, TextAlign::Start);
        // Own `direction: rtl` still applies to the child normally — it's a
        // separate property, unaffected by the match-parent resolution.
        assert_eq!(r.computed[span].direction, Direction::Rtl);
        // But `text-align: match-parent` must resolve against the *parent's*
        // ltr (→ left), not the child's own rtl (which would give right).
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Left,
            "match-parent must use the parent's direction, not the node's own direction winner (CSS Text 3 §6.1 verbatim: \"the parent's direction value\")"
        );
    }

    #[test]
    fn text_align_match_parent_resolves_end_against_rtl_parent_to_left() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl; text-align: end"));
        let span = doc.push_element(p, "span", Some("text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].text_align,
            TextAlign::Left,
            "end + rtl → left (CSS Text 3 §6.1 match-parent table)"
        );
    }

    #[test]
    fn text_align_match_parent_copies_center_parent_verbatim() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("text-align: center"));
        let span = doc.push_element(p, "span", Some("text-align: match-parent"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[span].text_align, TextAlign::Center);
    }

    /// CSS Text 3 §6.1 verbatim: "Computes to start when specified on the
    /// root element." This is **not** the same rule as the parent-direction
    /// table — a root element declaring `direction: rtl` on itself must not
    /// affect its own `match-parent` resolution (there is no parent to
    /// consult at all).
    #[test]
    fn text_align_match_parent_on_root_element_resolves_to_start() {
        use crate::property::TextAlign;
        let cv = cascade_doc("", "html", Some("direction: rtl; text-align: match-parent"));
        assert_eq!(cv.text_align, TextAlign::Start);
    }

    /// Three-level chain: pins the induction that a computed `text_align` is
    /// never observed as `MatchParent` — the grandchild resolves against the
    /// *child's* already-resolved computed value (`Left`), not against
    /// `MatchParent` itself.
    #[test]
    fn text_align_match_parent_resolves_against_already_resolved_parent() {
        use crate::property::TextAlign;
        let mut doc = TestDoc::new();
        let a = doc.push_element(0, "a", Some("text-align: start")); // ltr default → resolves nowhere (not match-parent itself)
        let b = doc.push_element(a, "b", Some("text-align: match-parent")); // start+ltr → left
        let c = doc.push_element(b, "c", Some("text-align: match-parent")); // inherits `left` verbatim
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[a].text_align, TextAlign::Start);
        assert_eq!(r.computed[b].text_align, TextAlign::Left);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[c].text_align,
            TextAlign::Left,
            "grandchild's match-parent must copy the child's *resolved* Left, not re-interpret MatchParent"
        );
    }

    // ── box-sizing wire-through (CSS Sizing 3 §3.3) ──

    #[test]
    fn box_sizing_wired_through_cascade_from_inline_style() {
        // <p style="box-sizing: border-box"> → ComputedValues.box_sizing に
        // BoxSizing::BorderBox が届く。parser → PropertyValue::BoxSizing →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。
        // sibling (background-color / line-height / counter-* / content /
        // string-set / position / text-align) の wire-through pattern を踏襲
        // (原則 1 前例主義)。
        use crate::property::BoxSizing;
        let cv = cascade_doc("", "p", Some("box-sizing: border-box"));
        assert_eq!(cv.box_sizing, BoxSizing::BorderBox);
    }

    // ── vertical-align wire-through (CSS 2.1 §10.8.1) ──

    #[test]
    fn vertical_align_new_keywords_wired_through_cascade_from_inline_style() {
        // parser → PropertyValue::VerticalAlign → apply_value →
        // SpecifiedValues::finalize → ComputedValues の end-to-end 疎通 —
        // `middle`/`text-top`/`text-bottom` (`box_sizing_wired_through_cascade_from_inline_style`
        // と同じ pattern)。
        use crate::property::VerticalAlign;
        assert_eq!(
            cascade_doc("", "span", Some("vertical-align: middle")).vertical_align,
            VerticalAlign::Middle
        );
        assert_eq!(
            cascade_doc("", "span", Some("vertical-align: text-top")).vertical_align,
            VerticalAlign::TextTop
        );
        assert_eq!(
            cascade_doc("", "span", Some("vertical-align: text-bottom")).vertical_align,
            VerticalAlign::TextBottom
        );
    }

    #[test]
    fn vertical_align_length_absolutizes_against_own_font_size_through_cascade() {
        // `font-size: 20px; vertical-align: 2em` on the same element →
        // phase 3 (`resolve_vertical_align`, called from
        // `SpecifiedValues::absolutize_with`) absolutizes against this
        // element's own (already phase-2-resolved) `font-size`, not the
        // inherited parent's — 2 * 20 = 40px.
        use crate::property::{Length, VerticalAlign};
        let cv = cascade_doc("", "span", Some("font-size: 20px; vertical-align: 2em"));
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.vertical_align, VerticalAlign::Length(Length::Px(40.0)));
    }

    // ── z-index wire-through (CSS2 §9.9.1) ──

    #[test]
    fn z_index_wired_through_cascade_from_inline_style() {
        // <p style="z-index: 3"> → ComputedValues.z_index に
        // ZIndexValue::Integer(3) が届く。parser → PropertyValue::ZIndex →
        // apply_value → ComputedValues の end-to-end 疎通 smoke。sibling
        // (box-sizing / font-style) の wire-through pattern を踏襲。
        use crate::property::ZIndexValue;
        let cv = cascade_doc("", "p", Some("z-index: 3"));
        assert_eq!(cv.z_index, ZIndexValue::Integer(3));
    }

    // ── order wire-through (CSS Flexible Box Layout Module Level 1 §4.2) ──

    #[test]
    fn order_wired_through_cascade_from_inline_style() {
        // <p style="order: 2"> → ComputedValues.order に 2 が届く。parser →
        // PropertyValue::Order → apply_value → ComputedValues の end-to-end
        // 疎通 smoke (`z_index_wired_through_cascade_from_inline_style` と
        // 同 pattern)。
        let cv = cascade_doc("", "p", Some("order: 2"));
        assert_eq!(cv.order, 2);
    }

    #[test]
    fn order_non_inherited_child_starts_from_initial() {
        // CSS Flexible Box Layout Module Level 1 §4.2 propdef:
        // "Inherited: no" (`z_index_non_inherited_child_starts_from_initial`
        // と同じ pattern)。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("order: 5"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].order, 5);
        assert_eq!(
            r.computed[span].order, 0,
            "order must not inherit from parent (CSS Flexbox 1 §4.2 Inherited: no)"
        );
    }

    #[test]
    fn flex_flow_shorthand_wired_through_cascade_from_inline_style() {
        // <p style="flex-flow: column wrap"> → ComputedValues.flex_direction /
        // flex_wrap に Column / Wrap が届く (parse → expand → apply_value の
        // end-to-end 疎通 smoke)。
        use crate::property::{FlexDirectionValue, FlexWrapValue};
        let cv = cascade_doc("", "p", Some("flex-flow: column wrap"));
        assert_eq!(cv.flex_direction, FlexDirectionValue::Column);
        assert_eq!(cv.flex_wrap, FlexWrapValue::Wrap);
    }

    #[test]
    fn z_index_non_inherited_child_starts_from_initial() {
        // CSS2 §9.9.1 propdef: "Inherited: no". sibling:
        // `text_decoration_non_inherited_child_starts_from_initial` と同じ
        // pattern。
        use crate::property::ZIndexValue;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("z-index: 5"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].z_index, ZIndexValue::Integer(5));
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].z_index,
            ZIndexValue::Auto,
            "z-index must not inherit from parent (CSS2 §9.9.1 Inherited: no)"
        );
    }

    // ── break-before / break-after / break-inside wire-through
    // (CSS Fragmentation Module Level 3 §3.1 / §3.2 / §3.4) ──

    #[test]
    fn break_before_wired_through_cascade_from_inline_style() {
        // <p style="break-before: avoid-page"> → ComputedValues.break_before
        // に BreakBetween::AvoidPage が届く。parser → PropertyValue::BreakBefore
        // → apply_value → ComputedValues の end-to-end 疎通 smoke。sibling
        // (box-sizing / z-index) の wire-through pattern を踏襲。
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("break-before: avoid-page"));
        assert_eq!(cv.break_before, BreakBetween::AvoidPage);
    }

    #[test]
    fn break_after_wired_through_cascade_from_inline_style() {
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("break-after: page"));
        assert_eq!(cv.break_after, BreakBetween::Page);
    }

    #[test]
    fn break_inside_wired_through_cascade_from_inline_style() {
        use crate::property::BreakInside;
        let cv = cascade_doc("", "p", Some("break-inside: avoid"));
        assert_eq!(cv.break_inside, BreakInside::Avoid);
    }

    #[test]
    fn break_before_non_inherited_child_starts_from_initial() {
        // CSS Fragmentation Module Level 3 §3.1 propdef: "Inherited: no".
        // sibling: `z_index_non_inherited_child_starts_from_initial` と同じ
        // pattern。
        use crate::property::BreakBetween;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("break-before: page"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].break_before, BreakBetween::Page);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].break_before,
            BreakBetween::Auto,
            "break-before must not inherit from parent (CSS Fragmentation \
             Module Level 3 §3.1 Inherited: no)"
        );
    }

    #[test]
    fn page_break_before_legacy_shorthand_wired_through_cascade_remaps_to_page() {
        // <p style="page-break-before: always"> → ComputedValues.break_before
        // に BreakBetween::Page が届く (CSS Fragmentation Module Level 3 §3.4
        // mapping table: `always` -> `page`, `BreakBetween` doc's "legacy
        // shorthand" section) — end-to-end check that the non-identity remap
        // survives the full parse -> cascade -> ComputedValues pipeline, not
        // just the `property::tests` parser-level check.
        use crate::property::BreakBetween;
        let cv = cascade_doc("", "p", Some("page-break-before: always"));
        assert_eq!(cv.break_before, BreakBetween::Page);
    }

    // ── font-weight keyword + inheritance (CSS Fonts 4 §2.2) ──

    #[test]
    fn font_weight_keyword_bold_wired_through_cascade_from_inline_style() {
        // <p style="font-weight: bold"> → ComputedValues.font_weight = 700。
        // parser Ident arm → PropertyValue::FontWeight(Absolute(700)) → apply_value →
        // ComputedValues の end-to-end 疎通 smoke (既存 wire-through test と同じ
        // pattern を踏襲)。
        let cv = cascade_doc("", "p", Some("font-weight: bold"));
        assert_eq!(cv.font_weight, 700.0);
    }

    #[test]
    fn font_weight_keyword_normal_wired_through_cascade_from_inline_style() {
        // <p style="font-weight: normal"> → ComputedValues.font_weight = 400。
        let cv = cascade_doc("", "p", Some("font-weight: normal"));
        assert_eq!(cv.font_weight, 400.0);
    }

    #[test]
    fn font_weight_is_inherited_child_carries_parent_bold() {
        // CSS Fonts 4 §2.2 "Inheritance: Yes"。<p style="font-weight: bold"> の
        // 子 <span> は自身 rule 無しでも parent の 700 を継承する。
        // Verification #7: parent bold + child 未指定 = child 700。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bold"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_weight, 700.0);
        assert_eq!(
            r.computed[span].font_weight, 700.0,
            "font-weight must be inherited (CSS Fonts 4 §2.2 Yes) — \
             parent bold keyword → child inherits 700"
        );
    }

    // ── font-weight bolder / lighter (CSS Fonts 4 §2.2) ──

    /// `<p style="font-weight: {parent}"><span style="font-weight: {child}">` を
    /// cascade して span の computed font-weight を返す。
    ///
    /// **親の computed value** を経由することが本 helper の主眼 — child は
    /// literal な spec 値ではなく、親が cascade を通して確定させた weight に
    /// 対して relative resolution される (Verification #7)。
    fn relative_weight_through_cascade(parent_decl: &str, child_decl: &str) -> f32 {
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some(parent_decl));
        let span = doc.push_element(p, "span", Some(child_decl));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        r.computed[span].font_weight
    }

    #[test]
    fn font_weight_bolder_lighter_table_all_six_rows() {
        // CSS Fonts 4 §2.2.1 の bolder/lighter table を 6 行 × 2 列すべて直接
        // 検証する。unit 関数を叩くことで cascade test setup に依存せず表の
        // 境界 (半開区間) を網羅する。
        //
        // | inherited w    | bolder | lighter |
        // | w < 100        | 400    | w       |
        // | 100 <= w < 350 | 400    | 100     |
        // | 350 <= w < 550 | 700    | 100     |
        // | 550 <= w < 750 | 900    | 400     |
        // | 750 <= w < 900 | 900    | 700     |
        // | 900 <= w       | w      | 700     |
        let bolder = |w| resolve_relative_weight(FontWeightValue::Bolder, w);
        let lighter = |w| resolve_relative_weight(FontWeightValue::Lighter, w);

        // row 1: w < 100 (lighter = no change)
        assert_eq!(bolder(1.0), 400.0);
        assert_eq!(bolder(99.0), 400.0);
        assert_eq!(lighter(1.0), 1.0);
        assert_eq!(lighter(99.0), 99.0);
        // row 2: 100 <= w < 350
        assert_eq!(bolder(100.0), 400.0);
        assert_eq!(bolder(349.0), 400.0);
        assert_eq!(lighter(100.0), 100.0);
        assert_eq!(lighter(349.0), 100.0);
        // row 3: 350 <= w < 550
        assert_eq!(bolder(350.0), 700.0);
        assert_eq!(bolder(549.0), 700.0);
        assert_eq!(lighter(350.0), 100.0);
        assert_eq!(lighter(549.0), 100.0);
        // row 4: 550 <= w < 750
        assert_eq!(bolder(550.0), 900.0);
        assert_eq!(bolder(749.0), 900.0);
        assert_eq!(lighter(550.0), 400.0);
        assert_eq!(lighter(749.0), 400.0);
        // row 5: 750 <= w < 900
        assert_eq!(bolder(750.0), 900.0);
        assert_eq!(bolder(899.0), 900.0);
        assert_eq!(lighter(750.0), 700.0);
        assert_eq!(lighter(899.0), 700.0);
        // row 6: 900 <= w (bolder = no change)
        assert_eq!(bolder(900.0), 900.0);
        assert_eq!(bolder(1000.0), 1000.0);
        assert_eq!(lighter(900.0), 700.0);
        assert_eq!(lighter(1000.0), 700.0);
    }

    #[test]
    fn font_weight_table_no_change_rows_are_not_clamps() {
        // 表の両端 2 行は "no change" であって clamp ではない。算術近似
        // (`min(w + 300, 900)` / `max(w - 300, 100)`) を書くとここが壊れる。
        // この 2 行は `font-weight` の受理 range を `[1,1000]` に広げて初めて
        // author から到達可能になったため、regression guard として置いている。
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Bolder, 1000.0),
            1000.0,
            "900 <= w row is no-change: bolder(1000) must stay 1000, not clamp to 900"
        );
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Lighter, 50.0),
            50.0,
            "w < 100 row is no-change: lighter(50) must stay 50, not rise to 100"
        );
    }

    #[test]
    fn font_weight_absolute_ignores_inherited_weight() {
        // `<font-weight-absolute>` は継承値と無関係にそのまま computed になる。
        assert_eq!(
            resolve_relative_weight(FontWeightValue::Absolute(250.0), 900.0),
            250.0
        );
    }

    /// `resolve_relative_weight` の doc 「非有限 `inherited` — 本関数は
    /// guard しない」節が記述する非対称処理を check する。
    ///
    /// 「guard は sink 境界に置く、resolve
    /// 層には置かない」という既存方針に従い、**本関数自体は変更しない** — 非対称は
    /// バグとして修正されるものではなく、非有限 `inherited` (通常経路では
    /// 型/parse guard により到達しないが `ComputedValues` の直接構築からは
    /// 到達しうる) に対する現状の table 分岐の帰結として、以降の regression
    /// で挙動が変わらないことを保証するために check する。sink 側の guard は
    /// `crates/raikiri-dom/src/layout.rs` の `sanitize_font_weight`
    /// (`preshape_text` が `parley::FontWeight::new` に渡す直前) に別途ある。
    #[test]
    fn resolve_relative_weight_non_finite_inherited_is_asymmetric() {
        let bolder = |w| resolve_relative_weight(FontWeightValue::Bolder, w);
        let lighter = |w| resolve_relative_weight(FontWeightValue::Lighter, w);

        // NaN: `<` 比較は常に false なので両 arm とも catch-all に落ちる。
        // catch-all の中身が違うので結果も違う —
        // `Bolder` の catch-all は `w => w` (900 <= w 行の "no change") な
        // ので NaN がそのまま伝播する。
        // Bolder(NaN) must propagate NaN as-is (catch-all is `w => w`).
        assert!(bolder(f32::NAN).is_nan());
        // `Lighter` の catch-all は `_ => 700.0` なので NaN は 700.0 に丸め
        // られる。
        // Lighter(NaN) must round to 700.0 (catch-all is `_ => 700.0`, not `w => w`).
        assert_eq!(lighter(f32::NAN), 700.0);

        // +Inf: `<` 比較は NaN と同じく常に false なので、同じ catch-all
        // 経路 (NaN と同型の非対称)。
        // Bolder(+Inf) must propagate +Inf as-is.
        assert_eq!(bolder(f32::INFINITY), f32::INFINITY);
        // Lighter(+Inf) must round to 700.0.
        assert_eq!(lighter(f32::INFINITY), 700.0);

        // -Inf: `w < 100.0` の最初の guard に一致するので、Bolder/Lighter
        // どちらも catch-all を経ない唯一の非有限入力 — ただし row 1 に
        // ヒットした後の結果は arm ごとに違う。`Bolder` の row 1 は `w if w <
        // 100.0 => 400.0` なので、有限の `w < 100` と同じく 400.0 に解決
        // される (正常な値)。`Lighter` の row 1 は "no change" arm (`w if w <
        // 100.0 => w`) なので `-Inf` はそのまま伝播する — row にヒットする
        // ことと結果が正常な有限値になることは同じではない。
        // Bolder(-Inf) hits row 1 (w < 100) like any finite w < 100 and
        // resolves to 400.0.
        assert_eq!(bolder(f32::NEG_INFINITY), 400.0);
        // Lighter(-Inf) hits row 1's no-change arm (`w if w < 100.0 => w`) and propagates -Inf.
        assert_eq!(lighter(f32::NEG_INFINITY), f32::NEG_INFINITY);
    }

    /// `resolve_relative_font_size` を unit 関数として直接叩く — cascade
    /// test setup に依存せず ratio (1.2) の適用を検証する
    /// (`font_weight_bolder_lighter_table_all_six_rows` の font-size 版)。
    #[test]
    fn resolve_relative_font_size_applies_1_2_ratio() {
        assert_eq!(
            resolve_relative_font_size(RelativeFontSize::Larger, 16.0),
            19.2
        );
        assert_eq!(
            resolve_relative_font_size(RelativeFontSize::Smaller, 16.0),
            16.0 / 1.2
        );
        // `×1.2` の後 `÷1.2` は f32 の丸めにより **bit 一致はしない** —
        // spec が round-trip を要求しているわけではなく、単に実装が table
        // lookup ではなく単純 ratio の合成であることの pin。浮動小数誤差の
        // 範囲 (`< 0.0001`) では元の値に戻ることを確認する。
        let round_tripped = resolve_relative_font_size(
            RelativeFontSize::Smaller,
            resolve_relative_font_size(RelativeFontSize::Larger, 16.0),
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert!(
            (round_tripped - 16.0).abs() < 0.0001,
            "×1.2 の後 ÷1.2 すれば浮動小数誤差の範囲で元に戻るはず: {round_tripped}"
        );
    }

    /// `resolve_against_inherited` の戻り値 `ResolvedAgainstInherited` が
    /// 中身を無損失で運ぶこと — 型を足したことで解決結果そのものが変わって
    /// いないことの pin。`as_property_value` (覗き見)
    /// と `into_property_value` (消費) の両方を、resolve 対象・pass-through
    /// 対象の 2 パターンで確認する。
    #[test]
    fn resolved_against_inherited_carries_the_value_without_loss() {
        let inherited = ComputedValues::initial();
        let ctx = ResolveContext::new(inherited.font_size);

        // 解決される側 (payload が変わる) — `bolder` は継承元 400 に対して
        // 700 に解決される。
        let resolved = resolve_against_inherited(
            PropertyValue::FontWeight(FontWeightValue::Bolder),
            &inherited,
            &ctx,
        );
        assert_eq!(
            resolved.as_property_value(),
            &PropertyValue::FontWeight(FontWeightValue::Absolute(700.0)),
            "as_property_value は所有権を取らずに中身を覗けること",
        );
        assert_eq!(
            resolved.into_property_value(),
            PropertyValue::FontWeight(FontWeightValue::Absolute(700.0)),
            "into_property_value は同じ値を消費して取り出せること",
        );

        // pass-through 側 (payload は変わらない) — `Color` はこの関数の対象外
        // なので `v` がそのまま返る。
        let passthrough =
            resolve_against_inherited(PropertyValue::Color(CssColor::BLACK), &inherited, &ctx);
        assert_eq!(
            passthrough.into_property_value(),
            PropertyValue::Color(CssColor::BLACK),
        );

        // `background-position` — added `v @ (...)` pass-through arm (CSS
        // Backgrounds 3 §2.6). This arm is only reachable via the `@page`
        // path (`crate::page::cascade_page`) in practice — the element
        // path goes through `apply_value` directly — so this direct call
        // is this arm's only coverage of the 7 `Background*` variants in
        // that bucket (same shape as the `Color` assertion above, which
        // covers `background_color`'s sibling arm the same way).
        let position = PropertyValue::BackgroundPosition(crate::property::CssPosition {
            horizontal: crate::property::CssPositionOffset::Start(Length::Px(5.0)),
            vertical: crate::property::CssPositionOffset::Start(Length::Px(5.0)),
        });
        let passthrough = resolve_against_inherited(position.clone(), &inherited, &ctx);
        assert_eq!(passthrough.into_property_value(), position);
    }

    #[test]
    fn font_weight_bolder_wired_through_cascade_from_parent_computed() {
        // Verification #3 / #4 / #5。親の **computed** weight に対して
        // resolve される (parse → PropertyValue::FontWeight(Bolder) →
        // apply_value → ComputedValues の end-to-end 疎通)。
        assert_eq!(
            relative_weight_through_cascade("font-weight: 400", "font-weight: bolder"),
            700.0
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 700", "font-weight: bolder"),
            900.0
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 900", "font-weight: bolder"),
            900.0,
            "900 <= w row: bolder is a no-op at 900"
        );
    }

    #[test]
    fn font_weight_lighter_wired_through_cascade_from_parent_computed() {
        // Verification #6。
        assert_eq!(
            relative_weight_through_cascade("font-weight: 100", "font-weight: lighter"),
            100.0,
            "100 <= w < 350 row: lighter(100) = 100"
        );
        assert_eq!(
            relative_weight_through_cascade("font-weight: 700", "font-weight: lighter"),
            400.0
        );
    }

    #[test]
    fn font_weight_relative_resolves_against_computed_not_literal_parent_value() {
        // Verification #7 の核心。親の declaration は `bold` keyword
        // (literal な spec 値は "bold" であって数値ではない) だが、resolution は
        // 親の **computed** 700 に対して行われる → bolder(700) = 900。
        assert_eq!(
            relative_weight_through_cascade("font-weight: bold", "font-weight: bolder"),
            900.0,
            "parent keyword `bold` must be computed to 700 first, then bolder(700) = 900"
        );
        // 親自身が bolder の場合は連鎖する: 親 = bolder(400 initial) = 700、
        // 子 = bolder(700) = 900。継承値が「親の computed」であることの証明。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bolder"));
        let span = doc.push_element(p, "span", Some("font-weight: bolder"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[p].font_weight, 700.0,
            "root-level bolder resolves against the initial 400"
        );
        assert_eq!(
            r.computed[span].font_weight, 900.0,
            "nested bolder must chain off the parent's computed 700, not off 400"
        );
    }

    #[test]
    fn font_weight_relative_is_inherited_as_resolved_absolute() {
        // bolder を解決した親の値は、以降 absolute weight として通常どおり
        // inherit される (computed side に sentinel が漏れない証明)。
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("font-weight: bolder"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[span].font_weight, 700.0);
    }

    #[test]
    fn font_weight_full_range_wired_through_cascade() {
        // spec range `[1,1000]` の両端が cascade まで届く。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1")).font_weight,
            1.0
        );
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 1000")).font_weight,
            1000.0
        );
        // fractional は f32 格上げ以降、丸めずそのまま computed value まで届く
        // (旧実装は round-half-away-from-zero で 101 に
        // 丸めていた)。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 100.5")).font_weight,
            100.5
        );
    }

    #[test]
    fn font_weight_wpt_font_weight_computed_150_25() {
        // WPT css/css-fonts/parsing/font-weight-computed.html:
        // `test_computed_value('font-weight', '150.25')` を cascade を経由した
        // computed side で check する (parse 側の同値 check は
        // `crate::property::tests::font_weight_wpt_font_weight_computed_150_25`)。
        assert_eq!(
            cascade_doc("", "p", Some("font-weight: 150.25")).font_weight,
            150.25
        );
    }

    #[test]
    fn bolder_lighter_resolve_against_unrounded_fractional_parent_weight() {
        // 実際に起きていた origin failure scenario (3 件、以下そのまま pin)。
        // 丸めが `u16` computed 表現に
        // 起因していた頃は、350 単位の relative-weight table 行選択そのものが
        // ずれていた:
        //
        // - `p { font-weight: 349.5 } span { font-weight: bolder }`
        //   spec: 349.5 は `100 <= w < 350` 行 → bolder = **400**
        //   旧実装: parse が 350 に丸め → `350 <= w < 550` 行 → **700** (誤り)
        // - `549.5` + `bolder`: spec **700** / 旧実装 **900** (誤り)
        // - `749.5` + `lighter`: spec **400** / 旧実装 **700** (誤り)
        //
        // payload / `ComputedValues.font_weight` を `f32` に格上げしたことで
        // 丸め自体が無くなり、以下は spec どおりの行に解決される。
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            relative_weight_through_cascade("font-weight: 349.5", "font-weight: bolder"),
            400.0,
            "349.5 is in the `100 <= w < 350` row, not `350 <= w < 550`"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            relative_weight_through_cascade("font-weight: 549.5", "font-weight: bolder"),
            700.0,
            "549.5 is in the `350 <= w < 550` row, not `550 <= w < 750`"
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            relative_weight_through_cascade("font-weight: 749.5", "font-weight: lighter"),
            400.0,
            "749.5 is in the `550 <= w < 750` row, not `750 <= w < 900`"
        );
    }

    #[test]
    fn multiple_elements_each_carry_own_running_template() {
        // 複数 element がそれぞれ異なる running(name) を持つ →
        // per-node で seed が独立に格納される (per-document concat は下流責務)。
        use crate::computed::RunningTemplate;
        let mut doc = TestDoc::new();
        let h = doc.push_element(0, "header", Some("position: running(hdr)"));
        let f = doc.push_element(0, "footer", Some("position: running(ftr)"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[h].running_templates,
            vec![RunningTemplate {
                name: SmolStr::new("hdr")
            }]
        );
        assert_eq!(
            r.computed[f].running_templates,
            vec![RunningTemplate {
                name: SmolStr::new("ftr")
            }]
        );
    }

    // ── Cascade memory DoS regression (SEC HIGH) ──
    //
    // 従来 `PropertyValue::Content(Vec<ContentComponent>)` /
    // `ComputedValues.content: Vec<ContentComponent>` は cascade 段の `decl.value.clone()`、
    // `apply_winners` の drain の `value.clone()`、`resolve_inheritance` の stack push + write と
    // 段階ごとに deep-clone を経由し、`* { content: "<large>" }` × N element で
    // O(N × M) 相当の heap 消費を招いていた。この修正で outer `Vec` を
    // `Arc<Vec<ContentComponent>>` に wrap、全 clone 経路が Arc reference-count increment に落ちた。
    //
    // 実 heap 計測は環境依存 (allocator hook が必要) のため、behavioral proxy として
    // `Arc::ptr_eq` で「複数 element が同 rule から同一 underlying `Vec` を共有」を
    // 確認する。Regression 時 (deep clone に戻る) はここが false となり test fail する。

    /// `* { content: "<literal>" }` × N element の cascade で、matching 全 element
    /// の `ComputedValues.content` Arc は **同 underlying Vec** を指す
    /// (`Arc::ptr_eq` = true)。deep-clone regression の behavioral canary。
    #[test]
    fn cascade_shares_content_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        // 攻撃 vector そのものの縮小版: universal selector + 単一 literal payload。
        doc.push_text(s, r#"* { content: "shared payload" }"#);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // Sanity: 両 element とも content が届いている。
        assert_eq!(r.computed[p1].content.len(), 1);
        assert_eq!(r.computed[p2].content.len(), 1);
        // Regression assert: Arc pointer identity で shallow-shared を証明。
        // deep-clone 復活時は underlying alloc が別、ptr_eq = false → fail。
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].content, &r.computed[p2].content),
            "cascade must Arc-share content across universal-selector matches \
             (SEC HIGH DoS regression)"
        );
    }

    /// `string-set` も content と同じ cascade path を辿るため、同種 Arc 共有が
    /// 成立している必要がある (同じ修正で `PropertyValue::StringSet` も Arc wrap)。
    #[test]
    fn cascade_shares_string_set_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, r#"* { string-set: k "shared payload" }"#);
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].string_set.len(), 1);
        assert_eq!(r.computed[p2].string_set.len(), 1);
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].string_set, &r.computed[p2].string_set),
            "cascade must Arc-share string_set across universal-selector matches"
        );
    }

    // ── Cascade memory DoS regression (SEC HIGH) ──
    //
    // `counter-reset` /
    // `counter-increment` / `counter-set` は content/string-set と同じ
    // 3 段 clone 経路 (`decl.value.clone`、`apply_winners` の drain の `value.clone`、
    // `resolve_inheritance` の stack push + write) を辿るため
    // `PropertyValue::Counter*(Vec<..>)` × universal selector × N element で
    // O(N × M) 相当の heap 消費を招いていた。この修正で全 3 property の outer
    // `Vec` を `Arc<Vec<(SmolStr, i32)>>` に wrap、clone 経路が Arc reference-count increment に
    // 落ちた (asymptotic は O(N + M))。short-circuit (child stack entry で
    // counter-* を skip) は **意図的に採用せず** — content/string-set の修正も
    // 同じ non-inherited Arc field でありながら short-circuit していないため、
    // counter-* のみ特別扱いすると仕上げが非対称になる。Arc wrap 単独で DoS は
    // 塞がる (stack 上に転がるのは Arc reference-count increment 1 個ずつだけで、直後の
    // `inherit_from` で empty slot に落ちる)。

    /// `* { counter-reset: <list> }` × N element の cascade で、matching 全
    /// element の `ComputedValues.counter_reset` Arc は **同 underlying Vec** を
    /// 指す (`Arc::ptr_eq` = true)。deep-clone regression の behavioral canary。
    #[test]
    fn cascade_shares_counter_reset_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        // 攻撃 vector そのものの縮小版: universal selector + 3-name payload。
        doc.push_text(s, "* { counter-reset: c0 c1 c2 }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // Sanity: 両 element とも counter_reset が届いている。
        assert_eq!(r.computed[p1].counter_reset.len(), 3);
        assert_eq!(r.computed[p2].counter_reset.len(), 3);
        // Regression assert: Arc pointer identity で shallow-shared を証明。
        // deep-clone 復活時は underlying alloc が別、ptr_eq = false → fail。
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].counter_reset, &r.computed[p2].counter_reset),
            "cascade must Arc-share counter_reset across universal-selector matches \
             (SEC HIGH DoS regression)"
        );
    }

    /// `counter-increment` も counter-reset と同じ cascade path を辿るため、
    /// 同種 Arc 共有が成立している必要がある (同じ修正)。
    #[test]
    fn cascade_shares_counter_increment_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "* { counter-increment: c0 c1 c2 }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].counter_increment.len(), 3);
        assert_eq!(r.computed[p2].counter_increment.len(), 3);
        assert!(
            std::sync::Arc::ptr_eq(
                &r.computed[p1].counter_increment,
                &r.computed[p2].counter_increment
            ),
            "cascade must Arc-share counter_increment across universal-selector matches"
        );
    }

    /// `counter-set` も同じ cascade path を辿るため Arc 共有が必要 (同じ修正)。
    #[test]
    fn cascade_shares_counter_set_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "* { counter-set: c0 c1 c2 }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].counter_set.len(), 3);
        assert_eq!(r.computed[p2].counter_set.len(), 3);
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].counter_set, &r.computed[p2].counter_set),
            "cascade must Arc-share counter_set across universal-selector matches"
        );
    }

    /// counter-* は non-inherited — 親 element に counter 値があっても child は
    /// inherit_from で shared empty Arc slot に落ちる。この check が「Arc wrap 単独
    /// (short-circuit 無し) でも child stack entry の parent Arc reference-count increment が即 empty
    /// slot に置換される」ことを保証する。short-circuit 不採用の正当化
    /// assertion。
    ///
    /// CSS Lists 3 §4: 3 property とも property table が `Inherited: no`。
    /// See <https://www.w3.org/TR/css-lists-3/#auto-numbering>.
    #[test]
    fn resolve_inheritance_uses_initial_arc_for_non_inherited_counter_on_child() {
        let mut doc = TestDoc::new();
        // 親 <parent> に counter-reset を付け、child <child> は counter rule 無し。
        let parent = doc.push_element(0, "parent", Some("counter-reset: c 1"));
        let child = doc.push_element(parent, "child", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // 親は counter_reset を持つ、child は non-inherited のため empty。
        assert_eq!(r.computed[parent].counter_reset.len(), 1);
        assert!(r.computed[child].counter_reset.is_empty());
        // child の counter_reset Arc は shared empty slot と ptr_eq (empty Arc
        // slot 再利用の behavioral proxy)。ここが false になる regression:
        // (a) inherit_from が parent の Arc をそのまま渡してしまう
        // (b) inherit_from が per-node `Arc::new(Vec::new())` を alloc する
        // どちらも上記の memory 目標を破る。
        let shared = crate::property::empty_counter_entries();
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[child].counter_reset, &shared),
            "child counter_reset must point to shared empty Arc slot \
             (non-inherited short-circuit-equivalent canary)"
        );
    }

    /// Empty (initial / inherit_from) の content/string_set も **shared Arc slot**
    /// を再利用する — cascade fix の副作用で「per-node empty Arc allocation
    /// regression」に陥っていないことを check する。
    #[test]
    fn initial_empty_content_and_string_set_share_arc_slot() {
        let mut doc = TestDoc::new();
        // rule なし、element 2 個 (両者 empty content / string_set)。
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // どちらも empty (initial)。
        assert!(r.computed[p1].content.is_empty());
        assert!(r.computed[p2].content.is_empty());
        assert!(r.computed[p1].string_set.is_empty());
        assert!(r.computed[p2].string_set.is_empty());
        // Shared empty Arc slot を指しているので ptr_eq = true。
        // Arc::new(Vec::new()) を initial/inherit_from で直に呼ぶ regression が
        // 出た瞬間ここが false になり、DoS fix が memory alloc regression に
        // 転じたことを検知する。
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].content, &r.computed[p2].content),
            "empty content must reuse shared Arc slot — per-node empty Arc \
             allocation regression detected (side-effect canary)"
        );
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].string_set, &r.computed[p2].string_set),
            "empty string_set must reuse shared Arc slot (side-effect canary)"
        );
    }

    // ── font-family per-node malloc regression ──
    //
    // pre-existing finding (out-of-diff, unrelated to this change):
    // `ComputedValues.font_family` は本 crate で最後に
    // Arc-share されていなかった `Vec` payload (前述の修正群が Content/StringSet
    // と counter-* を対応済み)。あちら (non-inherited) と違い
    // `font-family` は **inherited** なので、支配的な per-node cost は
    // 「毎 node で initial にリセットする」コストではなく inheritance walk
    // (`SpecifiedValues::inherit_from` の `parent.font_family.clone()`) の
    // コスト — 同じ `Arc` fix でもコストの形が違う。上記と同じ
    // behavioral-proxy methodology: `heap` 計測は allocator hook 依存なので、
    // `Arc::ptr_eq` を deep-clone regression の canary として使う。

    /// **独立した** 2 回の `cascade()` 呼び出し (別々の `TestDoc`、共有する
    /// inheritance 祖先なし) でも、root の computed value は**同一**の
    /// `initial_font_family()` Arc slot を指さなければならない。
    ///
    /// この test は「同一 document 内の 2 sibling element」ではなく
    /// **document をまたぐ** 形でなければならない — 単一 document 内では
    /// `font-family` が **inherited** なので、rule 無しの sibling 2 つは
    /// `SpecifiedValues::inherit_from` の「親の Arc をコピーする」経路で
    /// 既に font_family Arc を共有してしまい、`initial_font_family()` 自体は
    /// その walk を seed するために `cascade()` 1 回あたり高々 1 回しか
    /// 呼ばれない。そのため同一 document 版の test では、
    /// `initial_font_family()` 内の `OnceLock` を削除しても green のまま
    /// になってしまう (この test の実装時に perturbation で確認済み —
    /// 同一 document 形は下記
    /// `resolve_inheritance_shares_font_family_arc_from_parent_when_child_has_no_declaration`
    /// を再度 test しているだけで、shared slot 自体は check していない)。
    /// 単一 call site での直接 check は
    /// `property::tests::initial_font_family_shares_arc_slot_across_calls`
    /// を参照。本 test はそれに加えて、slot が別々の `cascade()` 実行間
    /// (例: 1 process 内での複数 page rendering) をまたいで生存することを
    /// 確認する — 意図は `initial_empty_content_and_string_set_share_arc_slot`
    /// と同じだが、対象は非空の initial 値 (`[Atom::from("serif")]`) であり、
    /// この field の inherited 性質に合わせて adapt してある。
    #[test]
    fn initial_font_family_shares_arc_slot_across_independent_cascade_runs() {
        let mut doc_a = TestDoc::new();
        let a = doc_a.push_element(0, "p", None);
        let tree_a = build_rule_tree(&doc_a);
        let r_a = cascade(&doc_a, &tree_a).expect("cascade Ok");

        let mut doc_b = TestDoc::new();
        let b = doc_b.push_element(0, "p", None);
        let tree_b = build_rule_tree(&doc_b);
        let r_b = cascade(&doc_b, &tree_b).expect("cascade Ok");

        assert_eq!(r_a.computed[a].font_family.len(), 1);
        assert_eq!(r_b.computed[b].font_family.len(), 1);
        assert!(
            std::sync::Arc::ptr_eq(&r_a.computed[a].font_family, &r_b.computed[b].font_family),
            "initial font_family must reuse the shared `initial_font_family()` Arc \
             slot across independent cascade() runs"
        );
    }

    /// `* { font-family: ... }` × N element で、matching 全 element の
    /// `Vec` は勝者 declaration の Arc を共有しなければならない — mirrors
    /// `cascade_shares_content_arc_across_universal_selector_matches`。
    #[test]
    fn cascade_shares_font_family_arc_across_universal_selector_matches() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "* { font-family: Arial, sans-serif }");
        let p1 = doc.push_element(0, "p", None);
        let p2 = doc.push_element(0, "p", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p1].font_family.len(), 2);
        assert_eq!(r.computed[p2].font_family.len(), 2);
        assert!(
            std::sync::Arc::ptr_eq(&r.computed[p1].font_family, &r.computed[p2].font_family),
            "cascade must Arc-share font_family across universal-selector matches"
        );
    }

    /// `font-family` declaration を持たない child は、親と**同一**の Arc を
    /// (`Arc::ptr_eq`) 継承しなければならない — 中身を再 clone したものでは
    /// ならない。これはこの (inherited) field の支配的コストである
    /// inheritance-walk cost そのものであり、前述の
    /// non-inherited「shared empty slot にリセットする」形とは異なる。
    #[test]
    fn resolve_inheritance_shares_font_family_arc_from_parent_when_child_has_no_declaration() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "parent", Some("font-family: Georgia, serif"));
        let child = doc.push_element(parent, "child", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[parent].font_family.len(), 2);
        assert_eq!(r.computed[child].font_family.len(), 2);
        assert!(
            std::sync::Arc::ptr_eq(
                &r.computed[parent].font_family,
                &r.computed[child].font_family
            ),
            "child with no font-family declaration must inherit the parent's \
             Arc by identity, not a re-cloned Vec"
        );
    }

    // ── padding wire-through (CSS Box 3 §4.1 + §4.2) ──
    //
    // Verification #7 (cascade wire-through + non-inheritance):
    // <div style="padding: 10px 5%"> の cascade 結果が populate、initial value 0
    // が inheritance walk で child に伝播しない (padding は non-inherited)。

    #[test]
    fn padding_shorthand_wired_through_cascade_from_inline_style() {
        // Verification #7: `padding: 10px 5%` → 2-value form expansion で
        // top/bottom=10px, left/right=5% を pin。counter-* / content / string_set
        // wire-through pattern を踏襲。
        use crate::property::Sides;
        let cv = cascade_doc("", "div", Some("padding: 10px 5%"));
        assert_eq!(
            cv.padding,
            Sides {
                top: ComputedLengthPercentage::Px(10.0),
                right: ComputedLengthPercentage::Percent(5.0),
                bottom: ComputedLengthPercentage::Px(10.0),
                left: ComputedLengthPercentage::Percent(5.0),
            }
        );
    }

    #[test]
    fn padding_longhand_wired_through_cascade_from_inline_style() {
        // 4 longhand も端から端まで届くことを smoke で pin。
        use crate::property::Sides;
        let cv = cascade_doc(
            "",
            "div",
            Some("padding-top: 1px; padding-right: 2px; padding-bottom: 3px; padding-left: 4px"),
        );
        assert_eq!(
            cv.padding,
            Sides {
                top: ComputedLengthPercentage::Px(1.0),
                right: ComputedLengthPercentage::Px(2.0),
                bottom: ComputedLengthPercentage::Px(3.0),
                left: ComputedLengthPercentage::Px(4.0),
            }
        );
    }

    #[test]
    fn padding_is_non_inherited_child_starts_from_initial_zero() {
        // Verification #7 の後半: <div style="padding: 10px 5%"> の子 <span> は
        // 自身 rule がなく padding は initial (Sides::all(0px))。
        // sibling: string_set / content / display / counter-* / running_templates
        // の non-inheritance test と同じ shape。
        use crate::property::Sides;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("padding: 10px 5%"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // 親は shorthand 由来の値を持つ。
        assert_eq!(
            r.computed[div].padding,
            Sides {
                top: ComputedLengthPercentage::Px(10.0),
                right: ComputedLengthPercentage::Percent(5.0),
                bottom: ComputedLengthPercentage::Px(10.0),
                left: ComputedLengthPercentage::Percent(5.0),
            }
        );
        // 子は inherit_from 経由で initial (Sides::all(0px)) — 継承しない。
        assert_eq!(
            r.computed[span].padding,
            Sides::all(ComputedLengthPercentage::Px(0.0))
        );
    }

    #[test]
    fn padding_negative_declaration_dropped_at_cascade() {
        // spec (CSS Box 3) §4.1 negative reject の end-to-end smoke: cascade まで負値が
        // 到達せず、initial (0) が残る。property.rs test は parse_value 単体
        // の drop、本 test は rule.rs → cascade の一貫 drop を pin。
        use crate::property::Sides;
        let cv = cascade_doc("", "div", Some("padding-top: -5px"));
        // 負値 → declaration drop → padding は cascade 未 override → initial 0 が残る。
        assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    }

    #[test]
    fn padding_shorthand_then_longhand_longhand_wins() {
        // CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand>: shorthand は
        // parse-time で longhand に expand してから cascade する。
        // `padding: 10px; padding-top: 5px;` →
        // top=5, others=10 (source-order-independent、spec-correct)。
        // (margin で実装済みの parse-time expansion model に migrate 済み)
        let cv = cascade_doc("", "div", Some("padding: 10px; padding-top: 5px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    #[test]
    fn padding_longhand_then_shorthand_shorthand_wins() {
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で check: `padding-top: 5px; padding: 10px;`
        // → 全 side = 10px (後段 shorthand が top も含めて上書き)。
        let cv = cascade_doc("", "div", Some("padding-top: 5px; padding: 10px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(10.0));
    }

    // ── margin longhand + shorthand cascade (CSS Box 3 §3.1/§3.2) ──

    #[test]
    fn margin_shorthand_wired_through_cascade_two_value_expansion() {
        // Verification 6-a: `<div style="margin: 10px 20px">` → ComputedValues.margin
        // に top=10, right=20, bottom=10, left=20 が届く。
        // parser → parse_declaration_block (shorthand expand) → 4 longhand
        // PropertyValue → apply_value → ComputedValues の end-to-end 疎通 smoke。
        let cv = cascade_doc("", "div", Some("margin: 10px 20px"));
        assert_eq!(
            cv.margin,
            Sides {
                top: ComputedLengthPercentageOrAuto::Px(10.0),
                right: ComputedLengthPercentageOrAuto::Px(20.0),
                bottom: ComputedLengthPercentageOrAuto::Px(10.0),
                left: ComputedLengthPercentageOrAuto::Px(20.0),
            }
        );
    }

    #[test]
    fn margin_longhand_wired_through_cascade_single_side() {
        // longhand direct path — `<p style="margin-left: 2em">` → left = Em(2)、
        // 他 side は initial (0)。
        let cv = cascade_doc("", "p", Some("margin-left: 2em"));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(32.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_auto_wired_through_cascade_horizontal_centering() {
        // Verification 5: `margin: 0 auto` (block-level horizontal centering の
        // 慣用形) が全 4 side に正しく落ちる。cascade で LengthOrAuto::Auto の
        // wire-through を pin。
        let cv = cascade_doc("", "div", Some("margin: 0px auto"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn margin_shorthand_then_longhand_later_longhand_wins() {
        // spec (CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort>): 同一 declaration
        // block 内で shorthand + longhand が declared された場合、後方
        // declaration が同 rank/spec/order で勝つ。
        // `margin: 0px; margin-top: 10px;` → top=10, others=0。
        //
        // 本 test は本 architecture の load-bearing case: expansion 前 shorthand
        // を単一 key で cascade してしまうと、`PropertyKey` 宣言順では `Margin`
        // が `MarginTop` より後に来るため `margin` が必ず後勝ちし top=0 に
        // 上書きされる (spec と逆)。expand_shorthand_into が parse-time で longhand
        // 化するため per-key の cascade winner が top=10 に確定する。
        let cv = cascade_doc("", "div", Some("margin: 0px; margin-top: 10px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_longhand_then_shorthand_later_shorthand_wins() {
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で check: `margin-top: 10px; margin: 0px;`
        // → 全 side = 0px (後段 shorthand が top も含めて上書き)。
        // expand_shorthand_into の 4 longhand 展開が source_order を保持したまま
        // cascade に届き、後段が per-side 勝ち抜けする証拠。
        let cv = cascade_doc("", "div", Some("margin-top: 10px; margin: 0px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_margin_shorthand_preserves_later_longhand_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin: var(--space); margin-left: 20px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(20.0));
    }

    #[test]
    fn var_in_outline_shorthand_projects_each_deferred_longhand() {
        // `outline` expands before cascade; after the custom property is
        // substituted, each deferred longhand must project its component from
        // the reparsed `Outline` shorthand rather than being dropped.
        let cv = cascade_doc(
            "",
            "div",
            Some("--outline: auto 2px red; outline: var(--outline)"),
        );
        assert_eq!(cv.outline.width(), ComputedLength(2.0));
        assert_eq!(cv.outline.style(), OutlineStyle::Auto);
        assert_eq!(cv.outline.color, OutlineColor::Resolved(RED));
    }

    #[test]
    fn margin_non_inherited_child_starts_from_initial() {
        // Verification 6-b: CSS Box 3 §3.1 "Inherited: no"。<div style="margin:
        // 20px"> の子 <span> は自身 rule 無しで margin = initial (0 spread)。
        // sibling: display / string_set / content non-inherited と同 shape。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("margin: 20px"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].margin,
            Sides::all(ComputedLengthPercentageOrAuto::Px(20.0))
        );
        assert_eq!(
            r.computed[span].margin,
            Sides::all(ComputedLengthPercentageOrAuto::Px(0.0)),
            "margin must not inherit from parent"
        );
    }

    #[test]
    fn margin_negative_length_accepted() {
        // Task Non-goals: negative margin は spec-valid (§3.1)、cascade の end-to-end
        // で受理されることを check (parser 側 check `margin_side_accepts_negative_length`
        // と complementary、下流 layout 側で negative 意味付け)。
        let cv = cascade_doc("", "div", Some("margin-top: -5px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(-5.0));
    }

    // ── CSS Logical Properties and Values 1 §4.2/§4.4 margin-inline-*/
    //    margin-block-*/padding-inline-*/padding-block-* longhand +
    //    margin-inline/margin-block/padding-inline/padding-block shorthand
    //    wire-through ──
    //
    // raikiri は縦書きレンダリングパイプライン未実装のため writing-mode を
    // 常に HorizontalTb に潰し、かつ inline axis は `direction: ltr` を
    // 仮定した固定物理写像
    // (`PropertyValue::PaddingInline` doc 参照) — margin-inline-start/end →
    // margin-left/margin-right、margin-block-start/end →
    // margin-top/margin-bottom (padding も同型)。

    #[test]
    fn padding_inline_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("padding-inline-start: 5px; padding-inline-end: 10px"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        // untouched block axis stays at initial (0).
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn padding_block_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "div",
            Some("padding-block-start: 5px; padding-block-end: 10px"),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn margin_inline_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "p",
            Some("margin-inline-start: 5px; margin-inline-end: auto"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_block_longhand_wired_through_cascade() {
        let cv = cascade_doc(
            "",
            "p",
            Some("margin-block-start: auto; margin-block-end: 5px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn padding_inline_shorthand_one_value_wired_through_cascade() {
        // 1-value form spreads to both start/end (CSS Logical Properties and
        // Values 1 §4.4 "If only one value is given, it applies to both the
        // start and end edges").
        let cv = cascade_doc("", "div", Some("padding-inline: 12px"));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(12.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(12.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn padding_inline_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("padding-inline: 5px 10px"));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
    }

    #[test]
    fn padding_block_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("padding-block: 5px 10px"));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(5.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn margin_inline_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("margin-inline: 5px auto"));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_block_shorthand_two_value_wired_through_cascade() {
        let cv = cascade_doc("", "div", Some("margin-block: auto 5px"));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn margin_inline_start_and_margin_left_compete_on_the_same_cascade_key() {
        // Physical `margin-left` and logical `margin-inline-start` are
        // fixed-mapped to the exact same `PropertyValue::MarginLeft` variant
        // at parse time (`PropertyValue::PaddingInline` doc's "なぜ 8
        // longhand が専用 variant を持たないか" section) — so, per CSS
        // Logical Properties and Values 1 §4 ("corresponding flow-relative
        // and physical properties are paired"), the two compete for the
        // *same* cascade winner, with the later declaration winning (CSS
        // Cascading L4 §6.1 "Order of Appearance"). Same shape as
        // `margin_shorthand_then_longhand_later_longhand_wins`.
        let later_logical_wins = cascade_doc(
            "",
            "div",
            Some("margin-left: 1px; margin-inline-start: 2px"),
        );
        assert_eq!(
            later_logical_wins.margin.left,
            ComputedLengthPercentageOrAuto::Px(2.0)
        );
        let later_physical_wins = cascade_doc(
            "",
            "div",
            Some("margin-inline-start: 2px; margin-left: 1px"),
        );
        assert_eq!(
            later_physical_wins.margin.left,
            ComputedLengthPercentageOrAuto::Px(1.0)
        );
    }

    #[test]
    fn var_in_margin_inline_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_shorthand_preserves_later_longhand_cascade`
        // — `margin-inline` only fans out to `margin-left`/`margin-right`
        // (unlike `margin`'s 4-side fan-out), so the untouched block axis
        // must stay at initial (0), not at the `var()`-substituted value.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin-inline: var(--space); margin-left: 20px"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_margin_block_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for the block-axis 2-value shorthand — exercises
        // `crate::rule::expand_deferred`'s `MarginBlock` arm (only the
        // inline-axis sibling was previously covered by an end-to-end
        // var() test).
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; margin-block: var(--space); margin-top: 20px"),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    #[test]
    fn var_in_padding_inline_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for `padding-inline` — exercises `crate::rule::expand_deferred`'s
        // `PaddingInline` arm.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; padding-inline: var(--space); padding-left: 20px"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(20.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn var_in_padding_block_shorthand_preserves_later_longhand_cascade() {
        // Sibling of `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // for `padding-block` — exercises `crate::rule::expand_deferred`'s
        // `PaddingBlock` arm, the last of the 4 logical 2-value shorthands.
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 10px; padding-block: var(--space); padding-top: 20px"),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(20.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(0.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(0.0));
    }

    #[test]
    fn var_in_padding_inline_start_longhand_preserves_cascade() {
        // `property_key_for_name`'s `"padding-inline-start" =>
        // PropertyKey::PaddingLeft` arm is only exercised by the deferred
        // (`var()`) path when `deferred.property` gets re-parsed by
        // `resolve_deferred_value` — this pins that round-trip for a single
        // logical longhand (the `margin-inline`/`padding-inline` shorthand
        // var() round-trip is covered by
        // `var_in_margin_inline_shorthand_preserves_later_longhand_cascade`
        // above).
        let cv = cascade_doc(
            "",
            "div",
            Some("--space: 7px; padding-inline-start: var(--space)"),
        );
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(7.0));
    }

    #[test]
    fn margin_inline_shorthand_important_beats_later_normal_physical_longhand() {
        // CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim:
        // "Declaring a shorthand property to be !important is equivalent to
        // declaring all of its sub-properties to be !important." —
        // `crate::rule::expand_shorthand_into`'s `important` threading
        // (`expand_margin_inline` etc. each take and propagate an
        // `important: bool`) must hold for the 2-value logical shorthands
        // too, not just the physical `margin`/`padding` shorthands this same
        // guarantee already covers.
        //
        // Source order alone would make the later `margin-left: 20px` win
        // (CSS Cascading L4 §6.1 "Order of Appearance"), but Origin and
        // Importance rank higher than order (§6.1) — so this only comes out
        // to 5px if the `!important` flag actually survived the `margin-inline`
        // → `MarginLeft`/`MarginRight` fan-out.
        let cv = cascade_doc(
            "",
            "div",
            Some("margin-inline: 5px !important; margin-left: 20px"),
        );
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(5.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(5.0));
    }

    #[test]
    fn margin_inline_shorthand_three_values_declaration_dropped() {
        // End-to-end sibling of `margin_shorthand_five_values_declaration_dropped`
        // for the 2-value logical shorthand: `property::tests`'s
        // `margin_inline_shorthand_leaves_extra_values_for_caller_exhausted_check`
        // pins the bare `parse_value`-level behavior (2 values consumed,
        // 3rd left unconsumed, `Some` still returned) and *claims* the
        // caller's `expect_exhausted` drops the whole declaration — this
        // test is the actual end-to-end confirmation of that claim, through
        // `cascade_doc`'s real stylesheet-parse path (same shape as
        // `padding_shorthand_rejects_any_negative_value`'s leftover-token
        // drop, but via a real `<div style>` rather than a hand-built
        // `Parser`).
        let cv = cascade_doc("", "div", Some("margin-inline: 5px 10px 15px"));
        // Declaration dropped entirely → margin stays at initial (0), not
        // the would-be start/end pair (5px/10px) the 2-value prefix alone
        // would produce.
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(0.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(0.0));
    }

    // ── height wire-through (CSS Sizing 3 §3.1.1) ──

    #[test]
    fn height_wired_through_cascade_from_inline_style() {
        // <div style="height: 100px"> → ComputedValues.height に
        // LengthOrAuto::Length(Length::Px(100)) が届く。parser →
        // PropertyValue::Height → apply_value → ComputedValues の end-to-end
        // 疎通 smoke (margin / padding wire-through pattern を踏襲)。
        let cv = cascade_doc("", "div", Some("height: 100px"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn height_auto_wired_through_cascade() {
        // `height: auto` は spec initial (§3.1.1) だが cascade winner として
        // declaration が到達した場合の受理 pattern を明示 check
        // (`static_position_wins_over_running_via_source_order` 系の pattern、
        // parser の auto ident branch と apply_value の LengthOrAuto::Auto 経路
        // が疎通することを保証)。
        let cv = cascade_doc("", "div", Some("height: auto"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn height_percentage_wired_through_cascade() {
        // `height: 50%` の end-to-end 疎通。resolve (containing block % → 実寸)
        // は下流責務、cascade は authored value をそのまま保持することを pin。
        let cv = cascade_doc("", "div", Some("height: 50%"));
        assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Percent(50.0));
    }

    #[test]
    fn height_non_inherited_child_starts_from_initial() {
        // Verification 6 (task doc): CSS Sizing 3 §3.1.1 "Inherited: no"。
        // <div style="height: 100px"> の子 <span> は自身 rule 無しで
        // height = initial (`LengthOrAuto::Auto`)。sibling: margin / padding
        // / display / string_set / content non-inherited と同 shape。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("height: 100px"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].height,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(
            r.computed[span].height,
            ComputedLengthPercentageOrAuto::Auto,
            "height must not inherit from parent (§3.1.1 Inherited: no)"
        );
    }

    #[test]
    fn height_negative_length_rejected_at_parse_time() {
        // Non-goal (a) spec-invalid: `height: -10px` は grammar `[0,∞]` 違反、
        // declaration 段で drop → cascade に届かず、height は initial (Auto) の
        // まま。parser 側 check (`height_rejects_negative_length`) と complementary
        // な end-to-end 挙動を確認。
        let cv = cascade_doc("", "div", Some("height: -10px"));
        assert_eq!(
            cv.height,
            ComputedLengthPercentageOrAuto::Auto,
            "negative height declaration must be dropped; height stays at initial"
        );
    }

    // ── <img width>/<img height> presentational hint
    // (HTML LS https://html.spec.whatwg.org/multipage/rendering.html#dimRendering) ──

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

    #[test]
    fn apply_value_direct_counter_reset_inherit() {
        // `counter-reset: revert`-class staging marker — never produced by
        // the element parser's normal `counter-reset: <counter-name>? ...`
        // grammar, but `apply_value` must still reset `target.counter_reset`
        // to the empty entries sentinel if an internal caller supplies one.
        let mut cv = SpecifiedValues::initial();
        cv.counter_reset = std::sync::Arc::new(vec![(SmolStr::new("foo"), 1)]);
        apply_value(PropertyValue::CounterResetInherit, &mut cv);
        assert_eq!(cv.counter_reset, empty_counter_entries());
    }

    #[test]
    fn apply_value_direct_position_sticky_and_fixed() {
        let mut cv = SpecifiedValues::initial();
        apply_value(PropertyValue::Position(PositionValue::Sticky), &mut cv);
        assert_eq!(cv.position, PositionValue::Sticky);
        apply_value(PropertyValue::Position(PositionValue::Fixed), &mut cv);
        assert_eq!(cv.position, PositionValue::Fixed);
    }

    #[test]
    fn apply_value_direct_padding_shorthand_direct_assign() {
        let mut cv = SpecifiedValues::initial();
        let sides = Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        };
        apply_value(PropertyValue::Padding(sides), &mut cv);
        assert_eq!(cv.padding, sides);
    }

    #[test]
    fn apply_value_direct_margin_inherit_marker_is_panic_free() {
        // Page-only inherit markers are never produced by the element
        // parser; `apply_value` must stay a no-op (not panic) if an
        // internal caller supplies one.
        let mut cv = SpecifiedValues::initial();
        let before = cv.margin;
        apply_value(PropertyValue::MarginInherit, &mut cv);
        assert_eq!(cv.margin, before);
    }

    #[test]
    fn apply_value_direct_calc_length_percentage_keys() {
        let value = CalcLengthPercentage {
            percent: 50.0,
            px: 10.0,
        };
        let expect_calc = LengthOrAuto::Calc(value);
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Height,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.height, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MaxWidth,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.max_width, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MaxHeight,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.max_height, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MinWidth,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.min_width, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::MinHeight,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.min_height, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Top,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.top, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Right,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.right, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Bottom,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.bottom, expect_calc);
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Left,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv.left, expect_calc);

        // Non-matching key: the inner match's wildcard fallback is a no-op
        // (this `key` has no `LengthOrAuto`-shaped field to project into).
        let before = cv.clone();
        apply_value(
            PropertyValue::CalcLengthPercentage {
                key: PropertyKey::Color,
                value,
            },
            &mut cv,
        );
        assert_eq!(cv, before);
    }

    #[test]
    fn apply_value_direct_inset_longhands() {
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Top(LengthOrAuto::Length(Length::Px(1.0))),
            &mut cv,
        );
        assert_eq!(cv.top, LengthOrAuto::Length(Length::Px(1.0)));
        apply_value(
            PropertyValue::Right(LengthOrAuto::Length(Length::Px(2.0))),
            &mut cv,
        );
        assert_eq!(cv.right, LengthOrAuto::Length(Length::Px(2.0)));
        apply_value(
            PropertyValue::Bottom(LengthOrAuto::Length(Length::Px(3.0))),
            &mut cv,
        );
        assert_eq!(cv.bottom, LengthOrAuto::Length(Length::Px(3.0)));
    }

    #[test]
    fn apply_value_direct_border_radius_corner_longhands() {
        let mut cv = SpecifiedValues::initial();
        apply_value(PropertyValue::BorderRadiusTopLeft(Length::Px(1.0)), &mut cv);
        assert_eq!(cv.border_radius.top_left, Length::Px(1.0));
        apply_value(
            PropertyValue::BorderRadiusTopRight(Length::Px(2.0)),
            &mut cv,
        );
        assert_eq!(cv.border_radius.top_right, Length::Px(2.0));
        apply_value(
            PropertyValue::BorderRadiusBottomRight(Length::Px(3.0)),
            &mut cv,
        );
        assert_eq!(cv.border_radius.bottom_right, Length::Px(3.0));
        apply_value(
            PropertyValue::BorderRadiusBottomLeft(Length::Px(4.0)),
            &mut cv,
        );
        assert_eq!(cv.border_radius.bottom_left, Length::Px(4.0));
    }

    #[test]
    fn apply_value_direct_outline_and_offset() {
        let mut cv = SpecifiedValues::initial();
        let outline = Outline {
            width: Length::Px(3.0),
            style: OutlineStyle::Dotted,
            color: OutlineColor::CurrentColor,
        };
        apply_value(PropertyValue::Outline(outline), &mut cv);
        assert_eq!(cv.outline, outline);
        apply_value(PropertyValue::OutlineOffset(Length::Px(5.0)), &mut cv);
        assert_eq!(cv.outline_offset, Length::Px(5.0));
    }

    #[test]
    fn apply_value_direct_grid_area_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::GridArea(area)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 4 `GridRowStart`/`GridColumnStart`/
        // `GridRowEnd`/`GridColumnEnd` longhands before `apply_value` ever
        // sees it) — not a safety net, a canary.
        let mut cv = SpecifiedValues::initial();
        let area = GridAreaShorthand {
            row_start: GridLineValue::Line(1),
            column_start: GridLineValue::Line(2),
            row_end: GridLineValue::Line(3),
            column_end: GridLineValue::Line(4),
        };
        apply_value(PropertyValue::GridArea(area.clone()), &mut cv);
        assert_eq!(cv.grid_row_start, area.row_start);
        assert_eq!(cv.grid_column_start, area.column_start);
        assert_eq!(cv.grid_row_end, area.row_end);
        assert_eq!(cv.grid_column_end, area.column_end);
    }

    #[test]
    fn apply_value_direct_grid_shorthand_fall_through() {
        // Sibling of `apply_value_direct_grid_area_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Grid(shorthand)` arm is
        // unreachable via the cascade path for the same reason — not a
        // safety net, a canary that also checks the shorthand's documented
        // sub-property reset behavior.
        let mut cv = SpecifiedValues::initial();
        cv.grid_auto_flow = GridAutoFlowValue::Column;
        cv.grid_row_start = GridLineValue::Line(9);
        let shorthand = GridShorthand {
            rows: GridTemplateTracks::None,
            columns: GridTemplateTracks::None,
        };
        apply_value(PropertyValue::Grid(shorthand), &mut cv);
        assert_eq!(cv.grid_template_rows, GridTemplateTracks::None);
        assert_eq!(cv.grid_template_columns, GridTemplateTracks::None);
        assert_eq!(cv.grid_template_areas, GridTemplateAreasValue::None);
        assert_eq!(cv.grid_auto_columns, initial_grid_auto_track_list());
        assert_eq!(cv.grid_auto_rows, initial_grid_auto_track_list());
        assert_eq!(cv.grid_auto_flow, GridAutoFlowValue::Row);
        assert_eq!(cv.grid_row_start, GridLineValue::Auto);
        assert_eq!(cv.grid_row_end, GridLineValue::Auto);
        assert_eq!(cv.grid_column_start, GridLineValue::Auto);
        assert_eq!(cv.grid_column_end, GridLineValue::Auto);
    }

    #[test]
    fn apply_value_direct_border_style_width_color_shorthand_fall_through() {
        // Sibling of `apply_value_direct_border_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::BorderStyle`/`BorderWidth`/
        // `BorderColor` arms are unreachable via the cascade path
        // (`expand_shorthand_into` expands each to its 4 side longhands
        // before `apply_value` ever sees it) — not a safety net, a canary.
        let mut cv = SpecifiedValues::initial();
        let styles = Sides {
            top: BorderStyle::Solid,
            right: BorderStyle::Dashed,
            bottom: BorderStyle::Dotted,
            left: BorderStyle::Double,
        };
        apply_value(PropertyValue::BorderStyle(styles), &mut cv);
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
        assert_eq!(cv.border.right.style, BorderStyle::Dashed);
        assert_eq!(cv.border.bottom.style, BorderStyle::Dotted);
        assert_eq!(cv.border.left.style, BorderStyle::Double);

        let widths = Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(3.0),
            left: Length::Px(4.0),
        };
        apply_value(PropertyValue::BorderWidth(widths), &mut cv);
        assert_eq!(cv.border.top.width, Length::Px(1.0));
        assert_eq!(cv.border.right.width, Length::Px(2.0));
        assert_eq!(cv.border.bottom.width, Length::Px(3.0));
        assert_eq!(cv.border.left.width, Length::Px(4.0));

        let colors = Sides {
            top: BorderColor::CurrentColor,
            right: BorderColor::Resolved(CssColor::BLACK),
            bottom: BorderColor::CurrentColor,
            left: BorderColor::Resolved(CssColor::TRANSPARENT),
        };
        apply_value(PropertyValue::BorderColor(colors), &mut cv);
        assert_eq!(cv.border.top.color, BorderColor::CurrentColor);
        assert_eq!(
            cv.border.right.color,
            BorderColor::Resolved(CssColor::BLACK)
        );
        assert_eq!(cv.border.bottom.color, BorderColor::CurrentColor);
        assert_eq!(
            cv.border.left.color,
            BorderColor::Resolved(CssColor::TRANSPARENT)
        );
    }

    #[test]
    fn apply_value_direct_page_named() {
        use crate::Atom;
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Page(PageValue::Named(Atom::from("chapter"))),
            &mut cv,
        );
        assert_eq!(cv.page, PageValue::Named(Atom::from("chapter")));
    }

    #[test]
    fn resolve_inheritance_grows_undersized_output_vectors() {
        // Defensive safety net: `cascade()`'s normal pre-allocation always
        // sizes `out`/`non_ua_margin_sides`/`authored_writing_modes` to
        // `dom.node_count()` before calling `resolve_inheritance`, so this
        // resize path is never exercised end-to-end. A direct call with
        // deliberately undersized (empty) vectors verifies the safety net
        // actually grows them instead of panicking on out-of-bounds writes.
        let mut doc = TestDoc::new();
        let e = doc.push_element(0, "div", None);
        let id = StyleNodeId(e as u64);
        let cascaded = CascadedArena::new();
        let mut out: Vec<ComputedValues> = Vec::new();
        let mut non_ua_margin_sides: Vec<Sides<bool>> = Vec::new();
        let mut authored_writing_modes: Vec<Option<WritingMode>> = Vec::new();
        let mut page_values = vec![PageValue::Auto; doc.node_count()];
        let mut pseudo_out = HashMap::new();
        resolve_inheritance(
            &doc,
            id,
            &ComputedValues::initial(),
            &cascaded,
            &mut out,
            &mut non_ua_margin_sides,
            &mut authored_writing_modes,
            &mut page_values,
            &mut pseudo_out,
        );
        assert!(out.len() > e);
        assert!(non_ua_margin_sides.len() > e);
        assert!(authored_writing_modes.len() > e);
    }

    #[test]
    #[should_panic(expected = "root_ctx == None は element 親が居ないことを意味するので")]
    fn resolve_inheritance_panics_when_root_parent_font_size_is_not_initial() {
        // The first stack entry `resolve_inheritance` pushes always has
        // `root_ctx == None` (only `cascade()`'s own `dom.root_id()` entry
        // point does this, and it always pairs `None` with
        // `ComputedValues::initial()`). A direct call that violates that
        // caller-side invariant — a non-initial `parent_computed` on the
        // very first node — must trip the debug_assert_eq guarding it.
        let mut doc = TestDoc::new();
        let e = doc.push_element(0, "div", None);
        let id = StyleNodeId(e as u64);
        let cascaded = CascadedArena::new();
        let mut out = vec![ComputedValues::initial(); doc.node_count()];
        let mut non_ua_margin_sides = vec![Sides::all(false); doc.node_count()];
        let mut authored_writing_modes = vec![None; doc.node_count()];
        let mut page_values = vec![PageValue::Auto; doc.node_count()];
        let mut pseudo_out = HashMap::new();
        let mut non_initial_parent = ComputedValues::initial();
        non_initial_parent.font_size = ComputedLength(999.0);
        resolve_inheritance(
            &doc,
            id,
            &non_initial_parent,
            &cascaded,
            &mut out,
            &mut non_ua_margin_sides,
            &mut authored_writing_modes,
            &mut page_values,
            &mut pseudo_out,
        );
    }

    #[test]
    fn apply_winners_direct_margin_shorthand_marks_all_sides_non_ua() {
        // `apply_winners`'s `non_ua_margin_sides` tracking arm for the
        // `Margin`/`MarginInline`/`MarginBlock` shorthand keys is
        // unreachable via the cascade path (shorthand is expanded to the 4
        // side longhands before candidates are collected) — not a safety
        // net, a canary for the shorthand-payload shape.
        let sides = Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        };
        let candidates: Vec<CascadedDecl> =
            vec![(PropertyValue::Margin(sides), false, Origin::Author, 0, 0)];
        let mut winners: Vec<Option<RankedDecl>> = Vec::new();
        let mut specified = SpecifiedValues::initial();
        let inherited = ComputedValues::initial();
        let custom_properties = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut non_ua_margin_sides = Sides::all(false);
        apply_winners(
            &candidates,
            &mut winners,
            &mut specified,
            &inherited,
            &custom_properties,
            None,
            Some(&mut non_ua_margin_sides),
            None,
        );
        assert!(non_ua_margin_sides.top);
        assert!(non_ua_margin_sides.right);
        assert!(non_ua_margin_sides.bottom);
        assert!(non_ua_margin_sides.left);
    }

    #[test]
    fn apply_winners_direct_border_radius_inherit() {
        let candidates: Vec<CascadedDecl> = vec![(
            PropertyValue::BorderRadiusInherit,
            false,
            Origin::Author,
            0,
            0,
        )];
        let mut winners: Vec<Option<RankedDecl>> = Vec::new();
        let mut specified = SpecifiedValues::initial();
        let mut inherited = ComputedValues::initial();
        inherited.border_radius = ComputedBorderRadius {
            top_left: ComputedLengthPercentage::Percent(10.0),
            top_right: ComputedLengthPercentage::Px(2.0),
            bottom_right: ComputedLengthPercentage::Percent(30.0),
            bottom_left: ComputedLengthPercentage::Px(4.0),
        };
        let custom_properties = CustomPropertyEnvironment::from_map(HashMap::new());
        apply_winners(
            &candidates,
            &mut winners,
            &mut specified,
            &inherited,
            &custom_properties,
            None,
            None,
            None,
        );
        assert_eq!(specified.border_radius.top_left, Length::Percent(10.0));
        assert_eq!(specified.border_radius.top_right, Length::Px(2.0));
        assert_eq!(specified.border_radius.bottom_right, Length::Percent(30.0));
        assert_eq!(specified.border_radius.bottom_left, Length::Px(4.0));
    }

    #[test]
    fn apply_winners_direct_page_value() {
        use crate::Atom;
        let candidates: Vec<CascadedDecl> = vec![(
            PropertyValue::Page(PageValue::Named(Atom::from("chapter"))),
            false,
            Origin::Author,
            0,
            0,
        )];
        let mut winners: Vec<Option<RankedDecl>> = Vec::new();
        let mut specified = SpecifiedValues::initial();
        let inherited = ComputedValues::initial();
        let custom_properties = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut page_value = PageValue::Auto;
        apply_winners(
            &candidates,
            &mut winners,
            &mut specified,
            &inherited,
            &custom_properties,
            Some(&mut page_value),
            None,
            None,
        );
        assert_eq!(page_value, PageValue::Named(Atom::from("chapter")));
    }

    #[test]
    fn apply_value_direct_margin_shorthand_fall_through() {
        // `apply_value` の `PropertyValue::Margin(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 4 longhand に展開する)。**これは
        // safety net ではない** — 万一
        // regression / bypass 経路で到達すると `target.margin = sides` の
        // atomic 上書きが 4 longhand winner を必ず破壊する。到達した時点で
        // 既に bug であり、本 test は arm を直接叩いて `unreachable!` 化 or
        // 空 arm regression を捕捉する canary。
        let mut cv = SpecifiedValues::initial();
        let sides = Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        };
        apply_value(PropertyValue::Margin(sides), &mut cv);
        assert_eq!(cv.margin, sides);
    }

    /// Sibling of `apply_value_direct_margin_shorthand_fall_through` for the
    /// CSS Logical Properties and Values 1 §4.2 `margin-inline` 2-value
    /// shorthand — same "not a safety net" rationale (`crate::property::PropertyValue::MarginInline` doc, // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
    /// which is the canonical record). `start`/`end` deliberately differ so
    /// a `start`/`end` field swap in the `apply_value` arm would fail this
    /// test.
    #[test]
    fn apply_value_direct_margin_inline_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: LengthOrAuto::Length(Length::Px(1.0)),
            end: LengthOrAuto::Length(Length::Px(2.0)),
        };
        apply_value(PropertyValue::MarginInline(pair), &mut cv);
        assert_eq!(cv.margin.left, pair.start);
        assert_eq!(cv.margin.right, pair.end);
        // untouched axis stays at initial (0).
        assert_eq!(cv.margin.top, LengthOrAuto::Length(Length::Px(0.0)));
        assert_eq!(cv.margin.bottom, LengthOrAuto::Length(Length::Px(0.0)));
    }

    /// Sibling of `apply_value_direct_margin_inline_shorthand_fall_through`
    /// for the block-axis `margin-block` shorthand.
    #[test]
    fn apply_value_direct_margin_block_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: LengthOrAuto::Length(Length::Px(3.0)),
            end: LengthOrAuto::Length(Length::Px(4.0)),
        };
        apply_value(PropertyValue::MarginBlock(pair), &mut cv);
        assert_eq!(cv.margin.top, pair.start);
        assert_eq!(cv.margin.bottom, pair.end);
        assert_eq!(cv.margin.left, LengthOrAuto::Length(Length::Px(0.0)));
        assert_eq!(cv.margin.right, LengthOrAuto::Length(Length::Px(0.0)));
    }

    /// Sibling of `apply_value_direct_margin_inline_shorthand_fall_through`
    /// for the CSS Logical Properties and Values 1 §4.4 `padding-inline`
    /// 2-value shorthand.
    #[test]
    fn apply_value_direct_padding_inline_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(6.0),
        };
        apply_value(PropertyValue::PaddingInline(pair), &mut cv);
        assert_eq!(cv.padding.left, pair.start);
        assert_eq!(cv.padding.right, pair.end);
        assert_eq!(cv.padding.top, Length::Px(0.0));
        assert_eq!(cv.padding.bottom, Length::Px(0.0));
    }

    /// Sibling of `apply_value_direct_padding_inline_shorthand_fall_through`
    /// for the block-axis `padding-block` shorthand.
    #[test]
    fn apply_value_direct_padding_block_shorthand_fall_through() {
        use crate::property::StartEnd;
        let mut cv = SpecifiedValues::initial();
        let pair = StartEnd {
            start: Length::Px(7.0),
            end: Length::Px(8.0),
        };
        apply_value(PropertyValue::PaddingBlock(pair), &mut cv);
        assert_eq!(cv.padding.top, pair.start);
        assert_eq!(cv.padding.bottom, pair.end);
        assert_eq!(cv.padding.left, Length::Px(0.0));
        assert_eq!(cv.padding.right, Length::Px(0.0));
    }

    #[test]
    fn apply_value_direct_overflow_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s
        // `PropertyValue::Overflow(pair)` arm is unreachable via the
        // cascade path (`expand_shorthand_into` expands it to the 2
        // `OverflowX`/`OverflowY` longhands before `apply_value` ever
        // sees it) — not a safety net, a canary that catches regression
        // if the arm is ever reached with a stale/wrong pair.
        let mut cv = SpecifiedValues::initial();
        let pair = OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        };
        apply_value(PropertyValue::Overflow(pair), &mut cv);
        assert_eq!(cv.overflow, pair);
    }

    #[test]
    fn apply_value_direct_text_decoration_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::TextDecoration(shorthand)`
        // arm is unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 3 `TextDecorationLine`/`TextDecorationStyle`/
        // `TextDecorationColor` longhands before `apply_value` ever sees
        // it) — not a safety net, a canary that catches regression if the
        // arm is ever reached with a stale/wrong shorthand value.
        let mut cv = SpecifiedValues::initial();
        let shorthand = TextDecorationShorthand {
            line: TextDecorationLine::UNDERLINE,
            style: TextDecorationStyle::Wavy,
            color: TextDecorationColor::Resolved(RED),
            thickness: TextDecorationThickness::Auto,
        };
        apply_value(PropertyValue::TextDecoration(shorthand), &mut cv);
        assert_eq!(cv.text_decoration_line, shorthand.line);
        assert_eq!(cv.text_decoration_style, shorthand.style);
        assert_eq!(cv.text_decoration_color, shorthand.color);
    }

    // ── border longhand + shorthand cascade ──

    #[test]
    fn border_shorthand_then_longhand_later_longhand_wins() {
        // spec (CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort>): 同一 declaration
        // block 内で shorthand + longhand が declared された場合、後方
        // declaration が同 rank/spec/order で勝つ。
        // `border: 1px solid red; border-top-width: 10px;` →
        // top.width=10、他 side の width=1、top.style=Solid、top.color=red 保持。
        //
        // 本 test は本 architecture の load-bearing case:
        // expansion 前 shorthand を単一 key で cascade してしまうと、`PropertyKey`
        // 宣言順では `Border` が `BorderTopWidth` より後に来るため `border` が
        // 必ず後勝ちし top.width=1 に上書きされる (spec と逆)。expand_shorthand_into
        // が parse-time で 12 longhand 化するため per-key の cascade winner が
        // top.width=10 に確定する (margin 0vv.5 precedent の 12-longhand 版)。
        let cv = cascade_doc(
            "",
            "div",
            Some("border: 1px solid red; border-top-width: 10px"),
        );
        assert_eq!(cv.border.top.width, ComputedLength(10.0));
        assert_eq!(cv.border.right.width, ComputedLength(1.0));
        assert_eq!(cv.border.bottom.width, ComputedLength(1.0));
        assert_eq!(cv.border.left.width, ComputedLength(1.0));
        // style / color は shorthand から expand された値のまま (per-side longhand
        // として cascade winner に居座る)。
        let red = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
        // border.color は `BorderColor` enum、shorthand
        // 由来の author-specified red は `Resolved` variant で cascade に届く。
        assert_eq!(cv.border.top.color, BorderColor::Resolved(red));
        assert_eq!(cv.border.right.style, BorderStyle::Solid);
        assert_eq!(cv.border.left.color, BorderColor::Resolved(red));
    }

    #[test]
    fn border_longhand_then_shorthand_later_shorthand_wins() {
        // CSS Cascading L4 §6.1 "Order of Appearance"
        // <https://www.w3.org/TR/css-cascade-4/#cascade-sort> の後方 wins を
        // 逆順で check: `border-top-width: 10px; border: 1px solid red;`
        // → top.width も 1px (後段 shorthand が top も含めて上書き)。
        // expand_shorthand_into の 12 longhand 展開が source_order を保持した
        // まま cascade に届き、後段が per-side / per-sub-property
        // 勝ち抜けする証拠 (margin sibling と対称)。
        let cv = cascade_doc(
            "",
            "div",
            Some("border-top-width: 10px; border: 1px solid red"),
        );
        assert_eq!(cv.border.top.width, ComputedLength(1.0));
        assert_eq!(cv.border.right.width, ComputedLength(1.0));
        assert_eq!(cv.border.bottom.width, ComputedLength(1.0));
        assert_eq!(cv.border.left.width, ComputedLength(1.0));
    }

    #[test]
    fn border_non_inherited_child_starts_from_initial() {
        // CSS Backgrounds 3 §3 "Borders" — border-* propdef は "Inherited: no"。
        // <div style="border: 5px solid red"> の子 <span> は自身 rule 無しで
        // border = initial (medium / none / currentcolor)。
        // sibling: margin / padding non-inherited test
        // を踏襲。color は `BorderColor` enum で保持。
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("border: 5px solid red"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        let red = CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        // parent は shorthand から expand された per-side 値。
        assert_eq!(r.computed[div].border.top.width, ComputedLength(5.0));
        assert_eq!(r.computed[div].border.top.style, BorderStyle::Solid);
        assert_eq!(r.computed[div].border.top.color, BorderColor::Resolved(red));
        // child は inherit_from が initial に戻す (non-inherited)。
        assert_eq!(
            r.computed[span].border,
            Sides::all(ComputedBorder {
                // specified initial は `medium` (3px) だが border-style が
                // `none` なので computed width は 0px (CSS Backgrounds 3 §3.3
                // "Computed value: … zero if the border style is `none` or
                // `hidden`")。
                width: ComputedLength::ZERO,
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            }),
            "border must not inherit from parent"
        );
    }

    #[test]
    fn apply_value_direct_border_shorthand_fall_through() {
        // `apply_value` の `PropertyValue::Border(sides)` arm は cascade 経路
        // では unreachable (`collect_cascaded` が 12 longhand に展開する)。**これは
        // safety net ではない** (margin
        // fall-through と対称) — 万一 regression / bypass 経路で到達すると
        // `target.border = sides` の atomic 上書きが 12 longhand winner を必ず
        // 破壊する。到達した時点で既に bug であり、本 test は arm を直接叩いて
        // `unreachable!` 化 or 空 arm regression を捕捉する canary。
        let mut cv = SpecifiedValues::initial();
        // `BorderColor::CurrentColor` を明示 fixture 化
        // (fall-through arm は payload の shape を保持することを check する)。
        let sides = Sides {
            top: Border {
                width: Length::Px(1.0),
                style: BorderStyle::Solid,
                color: BorderColor::CurrentColor,
            },
            right: Border {
                width: Length::Px(2.0),
                style: BorderStyle::Dashed,
                color: BorderColor::Resolved(CssColor::BLACK),
            },
            bottom: Border {
                width: Length::Px(3.0),
                style: BorderStyle::Dotted,
                color: BorderColor::CurrentColor,
            },
            left: Border {
                width: Length::Px(4.0),
                style: BorderStyle::Double,
                color: BorderColor::Resolved(CssColor::TRANSPARENT),
            },
        };
        apply_value(PropertyValue::Border(sides), &mut cv);
        assert_eq!(cv.border, sides);
    }

    #[test]
    fn apply_value_direct_flex_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Flex(f)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 3 `FlexGrow`/`FlexShrink`/`FlexBasis`
        // longhands before `apply_value` ever sees it) — not a safety
        // net, a canary that catches regression if the arm is ever
        // reached with a stale/wrong shorthand payload.
        use crate::property::{FlexBasisValue, FlexShorthand};
        let mut cv = SpecifiedValues::initial();
        let f = FlexShorthand {
            grow: 2.0,
            shrink: 3.0,
            basis: FlexBasisValue::Length(Length::Px(10.0)),
        };
        apply_value(PropertyValue::Flex(f), &mut cv);
        assert_eq!(cv.flex_grow, 2.0);
        assert_eq!(cv.flex_shrink, 3.0);
        assert_eq!(cv.flex_basis, FlexBasisValue::Length(Length::Px(10.0)));
    }

    #[test]
    fn apply_value_direct_flex_flow_shorthand_fall_through() {
        // Sibling of `apply_value_direct_flex_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::FlexFlow(f)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `FlexDirection`/`FlexWrap` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{FlexDirectionValue, FlexFlow, FlexWrapValue};
        let mut cv = SpecifiedValues::initial();
        let f = FlexFlow {
            direction: FlexDirectionValue::RowReverse,
            wrap: FlexWrapValue::Wrap,
        };
        apply_value(PropertyValue::FlexFlow(f), &mut cv);
        assert_eq!(cv.flex_direction, FlexDirectionValue::RowReverse);
        assert_eq!(cv.flex_wrap, FlexWrapValue::Wrap);
    }

    #[test]
    fn apply_value_direct_gap_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Gap(g)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `RowGap`/`ColumnGap` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{GapShorthand, LengthOrNormal};
        let mut cv = SpecifiedValues::initial();
        let g = GapShorthand {
            row: LengthOrNormal::Length(Length::Px(10.0)),
            column: LengthOrNormal::Length(Length::Px(30.0)),
        };
        apply_value(PropertyValue::Gap(g), &mut cv);
        assert_eq!(cv.row_gap, LengthOrNormal::Length(Length::Px(10.0)));
        assert_eq!(cv.column_gap, LengthOrNormal::Length(Length::Px(30.0)));
    }

    #[test]
    fn apply_value_direct_place_content_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::PlaceContent(p)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `AlignContent`/`JustifyContent`
        // longhands before `apply_value` ever sees it) — not a safety
        // net, a canary.
        use crate::property::{ContentAlignmentValue, PlaceContentShorthand};
        let mut cv = SpecifiedValues::initial();
        let p = PlaceContentShorthand {
            align: ContentAlignmentValue::SpaceBetween,
            justify: ContentAlignmentValue::Center,
        };
        apply_value(PropertyValue::PlaceContent(p), &mut cv);
        assert_eq!(cv.align_content, ContentAlignmentValue::SpaceBetween);
        assert_eq!(cv.justify_content, ContentAlignmentValue::Center);
    }

    #[test]
    fn apply_value_direct_grid_row_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::GridRow(shorthand)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `GridRowStart`/`GridRowEnd` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::GridLineShorthand;
        use crate::property::GridLineValue;
        let mut cv = SpecifiedValues::initial();
        let shorthand = GridLineShorthand {
            start: GridLineValue::Line(2),
            end: GridLineValue::Span(3),
        };
        apply_value(PropertyValue::GridRow(shorthand), &mut cv);
        assert_eq!(cv.grid_row_start, GridLineValue::Line(2));
        assert_eq!(cv.grid_row_end, GridLineValue::Span(3));
    }

    #[test]
    fn apply_value_direct_grid_column_shorthand_fall_through() {
        // Sibling of `apply_value_direct_grid_row_shorthand_fall_through`
        // above, for `PropertyValue::GridColumn(shorthand)`.
        use crate::property::GridLineShorthand;
        use crate::property::GridLineValue;
        let mut cv = SpecifiedValues::initial();
        let shorthand = GridLineShorthand {
            start: GridLineValue::Named("content".into()),
            end: GridLineValue::Auto,
        };
        apply_value(PropertyValue::GridColumn(shorthand), &mut cv);
        assert_eq!(cv.grid_column_start, GridLineValue::Named("content".into()));
        assert_eq!(cv.grid_column_end, GridLineValue::Auto);
    }

    #[test]
    fn apply_value_direct_place_items_shorthand_fall_through() {
        // Sibling of `apply_value_direct_place_content_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::PlaceItems(p)` arm is
        // unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 2 `AlignItems`/`JustifyItems` longhands before
        // `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{PlaceItemsShorthand, SelfAlignmentValue};
        let mut cv = SpecifiedValues::initial();
        let p = PlaceItemsShorthand {
            align: SelfAlignmentValue::Center,
            justify: SelfAlignmentValue::End,
        };
        apply_value(PropertyValue::PlaceItems(p), &mut cv);
        assert_eq!(cv.align_items, SelfAlignmentValue::Center);
        assert_eq!(cv.justify_items, SelfAlignmentValue::End);
    }

    #[test]
    fn apply_value_direct_place_self_shorthand_fall_through() {
        // Sibling of `apply_value_direct_place_items_shorthand_fall_through`
        // above, for `PropertyValue::PlaceSelf(p)`.
        use crate::property::{AlignSelfValue, PlaceSelfShorthand, SelfAlignmentValue};
        let mut cv = SpecifiedValues::initial();
        let p = PlaceSelfShorthand {
            align: AlignSelfValue::Auto,
            justify: AlignSelfValue::Value(SelfAlignmentValue::Start),
        };
        apply_value(PropertyValue::PlaceSelf(p), &mut cv);
        assert_eq!(cv.align_self, AlignSelfValue::Auto);
        assert_eq!(
            cv.justify_self,
            AlignSelfValue::Value(SelfAlignmentValue::Start)
        );
    }

    #[test]
    fn apply_value_direct_background_shorthand_fall_through() {
        // Sibling of `apply_value_direct_margin_shorthand_fall_through`
        // above: `apply_value`'s `PropertyValue::Background(shorthand)` arm
        // is unreachable via the cascade path (`expand_shorthand_into`
        // expands it to the 8 `BackgroundColor`/`BackgroundImage`/
        // `BackgroundRepeat`/`BackgroundAttachment`/`BackgroundPosition`/
        // `BackgroundSize`/`BackgroundClip`/`BackgroundOrigin` longhands
        // before `apply_value` ever sees it) — not a safety net, a canary.
        use crate::property::{
            BackgroundAttachment, BackgroundImage, BackgroundRepeat, BackgroundRepeatKeyword,
            BackgroundShorthand, BackgroundSize, CssPosition, CssPositionOffset, VisualBox,
        };
        let mut cv = SpecifiedValues::initial();
        let shorthand = BackgroundShorthand {
            color: RED,
            image: BackgroundImage::Url("tile.png".to_string()),
            repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Round,
                y: BackgroundRepeatKeyword::Space,
            },
            attachment: BackgroundAttachment::Fixed,
            position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Px(10.0)),
                vertical: CssPositionOffset::End(Length::Px(20.0)),
            },
            size: BackgroundSize::Explicit {
                width: LengthOrAuto::Length(Length::Px(100.0)),
                height: LengthOrAuto::Auto,
            },
            clip: VisualBox::PaddingBox,
            origin: VisualBox::ContentBox,
        };
        apply_value(PropertyValue::Background(shorthand.clone()), &mut cv);
        assert_eq!(cv.background_color, RED);
        assert_eq!(cv.background_image, shorthand.image);
        assert_eq!(cv.background_repeat, shorthand.repeat);
        assert_eq!(cv.background_attachment, shorthand.attachment);
        assert_eq!(cv.background_position, shorthand.position);
        assert_eq!(cv.background_size, shorthand.size);
        assert_eq!(cv.background_clip, shorthand.clip);
        assert_eq!(cv.background_origin, shorthand.origin);
    }

    #[test]
    fn var_in_background_shorthand_projects_each_deferred_longhand() {
        // `background` expands before cascade; after the custom property is
        // substituted, each deferred longhand must project its component
        // from the reparsed `BackgroundShorthand` rather than being dropped
        // (`var_in_outline_shorthand_projects_each_deferred_longhand`
        // sibling, same `project_deferred_value` mechanism).
        use crate::property::{
            BackgroundAttachment, BackgroundRepeat, BackgroundRepeatKeyword, VisualBox,
        };
        let cv = cascade_doc(
            "",
            "div",
            Some("--bg: red round fixed border-box; background: var(--bg)"),
        );
        assert_eq!(cv.background_color, RED);
        assert_eq!(
            cv.background_repeat,
            BackgroundRepeat {
                x: BackgroundRepeatKeyword::Round,
                y: BackgroundRepeatKeyword::Round,
            }
        );
        assert_eq!(cv.background_attachment, BackgroundAttachment::Fixed);
        assert_eq!(cv.background_clip, VisualBox::BorderBox);
        assert_eq!(cv.background_origin, VisualBox::BorderBox);
    }

    #[test]
    fn background_shorthand_expands_to_8_longhands_through_real_cascade() {
        // Unlike `var_in_background_shorthand_projects_each_deferred_longhand`
        // above (which goes through the *deferred* `var()` substitution +
        // `project_deferred_value` path), a literal, non-`var()` `background:`
        // declaration is a real `PropertyValue::Background` straight out of
        // `parse_value`, expanded by `crate::rule::expand_background` (via
        // `expand_shorthand_into`) before it ever reaches `apply_value` —
        // this is the only test that drives that expansion function through
        // the real parse → cascade pipeline rather than calling `apply_value`
        // directly.
        use crate::property::{BackgroundAttachment, BackgroundImage, VisualBox};
        let cv = cascade_doc(
            "",
            "div",
            Some("background: red url(a.png) no-repeat fixed border-box"),
        );
        assert_eq!(cv.background_color, RED);
        assert_eq!(
            cv.background_image,
            BackgroundImage::Url("a.png".to_string())
        );
        assert_eq!(cv.background_attachment, BackgroundAttachment::Fixed);
        assert_eq!(cv.background_clip, VisualBox::BorderBox);
        assert_eq!(cv.background_origin, VisualBox::BorderBox);
    }

    #[test]
    fn background_shorthand_comma_separated_multi_layer_declaration_dropped_through_real_cascade() {
        // `property::tests::background_shorthand_rejects_comma_separated_multi_layer`
        // pins this via the `parse_entire` fixture (`Parser::parse_entirely`,
        // semantically equivalent to the real `DeclParser`'s
        // `expect_exhausted` check but not the real pipeline itself). This
        // is the end-to-end sibling, through the real parse -> cascade path
        // (`expand_shorthand_into` never runs since `parse_value` itself
        // returns `None` for the whole declaration): a prior valid winner
        // must survive untouched, not merely fall back to the initial value
        // (which would also happen if the declaration were simply absent).
        let cv = cascade_doc(
            "",
            "div",
            Some("background: red; background: url(a.png) top, url(b.png) bottom"),
        );
        assert_eq!(cv.background_color, RED);
    }

    // ── font shorthand (CSS Fonts 4 §2.1) wire-through ──

    #[test]
    fn font_shorthand_expands_to_6_longhands_through_real_cascade() {
        // `background_shorthand_expands_to_8_longhands_through_real_cascade`
        // の sibling — literal な `font:` declaration が parse → cascade
        // pipeline を通り 6 longhand に展開されることの pin。
        use crate::property::{FontStyle, FontVariantCaps};
        let cv = cascade_doc(
            "",
            "div",
            Some("font: italic small-caps bold 20px/1.5 serif"),
        );
        assert_eq!(cv.font_style, FontStyle::Italic);
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.line_height, ComputedLineHeight::Number(1.5));
        assert_eq!(cv.font_family[0].to_string(), "serif");
    }

    #[test]
    fn font_shorthand_and_font_style_longhand_interleave_by_source_order() {
        // `background_shorthand_and_background_color_longhand_interleave_by_source_order`
        // の sibling — shorthand と longhand の競合は source order で決まる。
        use crate::property::FontStyle;
        let shorthand_first = cascade_doc(
            "",
            "div",
            Some("font: italic 20px serif; font-style: normal"),
        );
        assert_eq!(shorthand_first.font_style, FontStyle::Normal);

        let longhand_first = cascade_doc("", "div", Some("font-style: italic; font: 20px serif"));
        assert_eq!(longhand_first.font_style, FontStyle::Normal);
    }

    #[test]
    fn font_shorthand_relative_size_and_weight_resolve_against_parent() {
        // `div` は root (16px / 400) の子 — `larger` / `bolder` は親基準で
        // 解決される (longhand arm と同じ roll)。
        let cv = cascade_doc("", "div", Some("font: bolder larger serif"));
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, ComputedLength(19.2));
    }

    #[test]
    fn var_in_font_shorthand_projects_each_deferred_longhand() {
        // `var_in_background_shorthand_projects_each_deferred_longhand` の
        // sibling — `font: var(--f)` は `PropertyKey::Font` の deferred
        // として 6 longhand に fan-out し、各々が substitution 後に
        // re-parse される (`expand_deferred` + `project_deferred_value` 経路)。
        use crate::property::{FontStyle, FontVariantCaps};
        let cv = cascade_doc(
            "",
            "div",
            Some("--f: italic small-caps bold 20px/1.5 serif; font: var(--f)"),
        );
        assert_eq!(cv.font_style, FontStyle::Italic);
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, ComputedLength(20.0));
        assert_eq!(cv.line_height, ComputedLineHeight::Number(1.5));
        assert_eq!(cv.font_family[0].to_string(), "serif");
    }

    #[test]
    fn apply_value_direct_font_shorthand_fall_through() {
        // `apply_value_direct_margin_shorthand_fall_through` の sibling —
        // cascade 経路では unreachable (`expand_shorthand_into` が展開済み)
        // の canary。relative 成分 (`bolder` / `larger`) は longhand arm と
        // 同じく parent seed (ここでは initial: 400 / 16px) 基準で解決される。
        use crate::Atom;
        use crate::property::{
            FontShorthand, FontShorthandSize, FontStyle, FontVariantCaps, FontWeightValue,
            LineHeight, RelativeFontSize,
        };
        use std::sync::Arc;
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Font(FontShorthand {
                style: FontStyle::Italic,
                variant: FontVariantCaps::SmallCaps,
                weight: FontWeightValue::Bolder,
                size: FontShorthandSize::Relative(RelativeFontSize::Larger),
                line_height: LineHeight::Number(1.5),
                family: Arc::new(vec![Atom::from("serif")]),
            }),
            &mut cv,
        );
        assert_eq!(cv.font_style, FontStyle::Italic);
        assert_eq!(cv.font_variant_caps, FontVariantCaps::SmallCaps);
        assert_eq!(cv.font_weight, 700.0);
        assert_eq!(cv.font_size, Length::Px(19.2));
        assert_eq!(cv.line_height, LineHeight::Number(1.5));
        assert_eq!(cv.font_family[0].to_string(), "serif");
        // Absolute size takes the plain `FontSize` longhand arm (same
        // delegation shape as the `Relative` case above).
        let mut cv = SpecifiedValues::initial();
        apply_value(
            PropertyValue::Font(FontShorthand {
                style: FontStyle::Normal,
                variant: FontVariantCaps::Normal,
                weight: FontWeightValue::Absolute(400.0),
                size: FontShorthandSize::Absolute(Length::Px(12.0)),
                line_height: LineHeight::Normal,
                family: Arc::new(vec![Atom::from("serif")]),
            }),
            &mut cv,
        );
        assert_eq!(cv.font_size, Length::Px(12.0));
    }

    #[test]
    fn background_shorthand_and_background_color_longhand_interleave_by_source_order() {
        // Mirror of `var_in_margin_shorthand_preserves_later_longhand_cascade`'s
        // sibling check (`margin-top: 10px; margin: 0px` → all sides 0): the
        // `expand_shorthand_into` doc's entire rationale is that source order,
        // not variant order, decides the winner once a shorthand and a
        // conflicting longhand both target the same field. `background` is
        // the first shorthand whose fan-out targets 8 *pre-existing*
        // longhands rather than a fresh `Sides<T>`-style bundle, so this
        // pins that the same mechanism holds for it in both directions.
        let shorthand_first =
            cascade_doc("", "div", Some("background: red; background-color: blue"));
        assert_eq!(shorthand_first.background_color, BLUE);

        let longhand_first =
            cascade_doc("", "div", Some("background-color: blue; background: red"));
        assert_eq!(longhand_first.background_color, RED);
    }

    // ── text-decoration longhand + shorthand cascade (CSS Text Decoration
    // Module Level 3 §2.1-§2.4) ──

    #[test]
    fn text_decoration_shorthand_expands_to_line_style_and_color() {
        // `text-decoration: underline wavy red` — all 3 longhand winners
        // reach the same node through real parse + cascade.
        let cv = cascade_doc("", "div", Some("text-decoration: underline wavy red"));
        assert_eq!(cv.text_decoration_line, TextDecorationLine::UNDERLINE);
        assert_eq!(cv.text_decoration_style, TextDecorationStyle::Wavy);
        assert_eq!(cv.text_decoration_color, TextDecorationColor::Resolved(RED));
    }

    #[test]
    fn text_decoration_shorthand_resets_earlier_longhand_declarations() {
        // Shorthand-resets-omitted-longhands (CSS Cascading L4 §3 "exactly
        // as if expanded in place"): `text-decoration: underline` omits the
        // style/color components, but `parse_text_decoration_shorthand`
        // fills them with their *own* initial values rather than leaving
        // them unset — so a later bare `text-decoration: underline` still
        // resets an earlier explicit `text-decoration-style: wavy` back to
        // `solid` through ordinary "later declaration in the same block
        // wins" cascade order (CSS Cascading L4 §6.1 "Order of Appearance").
        // This is the test that actually discriminates a spec-correct
        // expansion from one that merely "leaves the others alone" — see
        // `crate::rule::tests::text_decoration_shorthand_always_overwrites_all_four_longhand` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — rustdoc-blind, confirmed via わざと壊して確かめる
        // for the declaration-list-shape version of the same fact.
        let cv = cascade_doc(
            "",
            "div",
            Some("text-decoration-style: wavy; text-decoration: underline"),
        );
        assert_eq!(cv.text_decoration_line, TextDecorationLine::UNDERLINE);
        assert_eq!(
            cv.text_decoration_style,
            TextDecorationStyle::Solid,
            "the later `text-decoration` shorthand must reset style back to \
             its own initial value, not leave the earlier `wavy` in place"
        );
        assert_eq!(cv.text_decoration_color, TextDecorationColor::CurrentColor);
    }

    #[test]
    fn text_decoration_shorthand_then_longhand_later_longhand_wins() {
        // Mirror of `border_shorthand_then_longhand_later_longhand_wins`:
        // `text-decoration: underline wavy; text-decoration-style: dotted;`
        // → style ends up `dotted` (later longhand wins), line stays
        // `underline` (untouched by the longhand declaration).
        let cv = cascade_doc(
            "",
            "div",
            Some("text-decoration: underline wavy; text-decoration-style: dotted"),
        );
        assert_eq!(cv.text_decoration_line, TextDecorationLine::UNDERLINE);
        assert_eq!(cv.text_decoration_style, TextDecorationStyle::Dotted);
    }

    #[test]
    fn text_decoration_non_inherited_child_starts_from_initial() {
        // CSS Text Decoration Module Level 3 §2.1-§2.3 — all 3 propdefs are
        // "Inherited: no". sibling: border / margin / padding non-inherited
        // test pattern.
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("text-decoration: underline wavy red"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[div].text_decoration_line,
            TextDecorationLine::UNDERLINE
        );
        assert_eq!(
            r.computed[span].text_decoration_line,
            TextDecorationLine::NONE,
            "text-decoration-line must not inherit from parent"
        );
        assert_eq!(
            r.computed[span].text_decoration_style,
            TextDecorationStyle::Solid
        );
        assert_eq!(
            r.computed[span].text_decoration_color,
            TextDecorationColor::CurrentColor
        );
    }

    #[test]
    fn width_length_end_to_end() {
        // Verification #7: `div { width: 100px }` が `ComputedValues.width` に
        // Length(Px(100)) として届く。parser → PropertyValue::Width → apply_value
        // → ComputedValues の end-to-end 疎通 smoke (sibling padding/margin と
        // 同 pattern)。
        let cv = cascade_doc("", "div", Some("width: 100px"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn width_auto_end_to_end() {
        // `width: auto` は cascade winner として apply_value で `Auto` に固定される。
        // `inherit_from` initial も Auto なので identity になるが、cascade path が
        // 実際に通っていることを check (silent no-op regression 検知)。
        let cv = cascade_doc("", "div", Some("width: auto"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn width_default_is_initial_auto() {
        // 未指定時は `ComputedValues::initial()` の Auto を維持 (spec §3.1.1
        // "Initial: auto"、non-inherited なので parent も影響しない)。
        let cv = cascade_doc("", "div", None);
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn width_child_does_not_inherit_from_parent() {
        // Verification #8: parent (div) が width: 100px を持っていても child
        // (span、指定 無し) は initial (Auto) を保持する。non-inherited property
        // の end-to-end check (sibling `inherit_from_leaves_*_at_initial` computed
        // 側 test の cascade path 版)。
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div { width: 100px }");
        let parent = doc.push_element(0, "div", None);
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).unwrap();
        // parent (div) は width: 100px を受け取る
        assert_eq!(
            result.computed[parent].width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        // child (span) は non-inherited のため initial (Auto) を保持
        assert_eq!(
            result.computed[child].width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn min_width_length_end_to_end() {
        // CSS Sizing 3 §4: `div { min-width: 100px }` が
        // `ComputedValues.min_width` に Px(100) として届く。sibling
        // `width_length_end_to_end` と同 pattern (parse_min_size →
        // PropertyValue::MinWidth → apply_value → finalize の end-to-end
        // smoke)。
        let cv = cascade_doc("", "div", Some("min-width: 100px"));
        assert_eq!(cv.min_width, ComputedLengthPercentageOrAuto::Px(100.0));
    }

    #[test]
    fn min_height_auto_and_negative_reject() {
        // CSS Sizing 3 §4 initial `auto` の identity round-trip + `[0,∞]`
        // 違反の drop pin。負値は parse 段で declaration drop するため
        // initial (Auto) のまま残る。
        let cv = cascade_doc("", "div", Some("min-height: auto"));
        assert_eq!(cv.min_height, ComputedLengthPercentageOrAuto::Auto);
        let cv_neg = cascade_doc("", "div", Some("min-height: -10px"));
        assert_eq!(cv_neg.min_height, ComputedLengthPercentageOrAuto::Auto);
    }

    #[test]
    fn min_max_child_does_not_inherit_from_parent() {
        // CSS Sizing 3 §4/§5: min/max は **non-inherited**。parent が
        // min-width / max-width を持っていても child は initial を保持する
        // (sibling `width_child_does_not_inherit_from_parent` と同 pattern)。
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "div { min-width: 100px; max-width: 200px }");
        let parent = doc.push_element(0, "div", None);
        let child = doc.push_element(parent, "span", None);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).unwrap();
        assert_eq!(
            result.computed[parent].min_width,
            ComputedLengthPercentageOrAuto::Px(100.0)
        );
        assert_eq!(
            result.computed[parent].max_width,
            ComputedLengthPercentageOrAuto::Px(200.0)
        );
        assert_eq!(
            result.computed[child].min_width,
            ComputedLengthPercentageOrAuto::Auto
        );
        assert_eq!(
            result.computed[child].max_width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn max_width_none_maps_to_auto() {
        // CSS Sizing 3 §5 initial `none` → computed Auto placeholder の連鎖
        // check (specified `parse_max_size` の `none` 分岐 + bridge の
        // `Dimension::auto()` 委譲の上流側)。
        let cv = cascade_doc("", "div", Some("max-width: none"));
        assert_eq!(cv.max_width, ComputedLengthPercentageOrAuto::Auto);
        let cv_px = cascade_doc("", "div", Some("max-height: 50%"));
        assert_eq!(
            cv_px.max_height,
            ComputedLengthPercentageOrAuto::Percent(50.0)
        );
    }

    #[test]
    fn cascade_with_ua_deterministic_across_10_runs() {
        // determinism regression (acceptance criteria for this stage of work)
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, "p { color: red }");
        let p = doc.push_element(0, "p", Some("font-size: 20px"));
        doc.push_element(p, "span", None);

        let mut tree = build_rule_tree(&doc);
        tree.add_stylesheet(
            "p { display: block } span { display: inline }",
            Origin::UserAgent,
        );

        let baseline = cascade(&doc, &tree).unwrap();
        for _ in 0..9 {
            let run = cascade(&doc, &tree).unwrap();
            assert_eq!(run.computed.len(), baseline.computed.len());
            for i in 0..run.computed.len() {
                assert_eq!(run.computed[i], baseline.computed[i], "differ at node {i}");
            }
        }
    }

    // ── post-parse shorthand injection ──────────────
    //
    // `add_stylesheet` の**後**に declaration を shorthand variant へ書き戻す
    // post-parse mutation 経路 — `crate::rule::parse_declaration_block` の
    // parse-time 展開はこの経路を守らない (`declaration_block_never_emits_
    // shorthand_keys` は parse 出口しか見ない)。この
    // 経路は crate 内からのみ到達可能なので `collect_cascaded` 入口の展開は
    // crate 内 invariant guard である。根拠は
    // `crate::rule::expand_shorthand_into` doc が canonical。
    //
    // 守るべき spec は 2 条:
    //
    // - CSS Cascading L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand>
    //   "A shorthand property sets all of its longhand sub-properties, exactly
    //   as if expanded in place."
    // - CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
    //   "Order of Appearance: … the last declaration in document order wins."
    //
    // すなわち勝者は **declaration の並び順**で決まる。`PropertyKey` discriminant
    // 順 (shorthand が longhand より後) に任せると、shorthand が先に書かれた
    // 場合に spec と逆になる。`collect_cascaded` が candidate を積む直前に
    // `expand_shorthand_into` を通すことでこれを塞ぐ。

    /// nqkj の repro を機械的に再現する helper。
    ///
    /// `css` を `RuleTree::add_stylesheet` で parse したあと、
    /// `style_rules[0].declarations[idx].value` を `injected` に差し替え、
    /// `<div>` 1 個の document に cascade して computed value を返す。
    ///
    /// この手つきは **nqkj が報告した当時の Consumer 経路**そのもの。qzn3 で
    /// 3 field を `pub(crate)` に絞ったので、もう crate 外からは書けない。
    ///
    /// `StyleRule` を直接 literal 構築しないのが要点 — `#[non_exhaustive]` は
    /// crate 内構築を妨げないので、literal だと **report された経路とは別の
    /// 経路**を test してしまう。
    ///
    /// ⚠️ 差し替えるのは `.value` **だけ**なので、`css` の `idx` 番目の
    /// declaration は 2 役を持つ:
    ///
    /// 1. **値は捨てられる** — 単に「その位置に declaration を 1 個作る」ための
    ///    placeholder (本 module の呼び出しでは `margin-left: 99px` 等、
    ///    以降の assert に一切現れない値を置いている)。
    /// 2. **`important` flag は生き残り、injected shorthand に載る** — 展開時に各
    ///    longhand へ copy される。これを利用して「注入 shorthand は normal」を
    ///    作っているのが
    ///    `post_parse_important_longhand_survives_later_normal_shorthand`
    ///    である (placeholder に `!important` を付けると逆になる)。
    fn cascade_with_post_parse_injection(
        css: &str,
        idx: usize,
        injected: PropertyValue,
    ) -> ComputedValues {
        let mut doc = TestDoc::new();
        let e = doc.push_element(0, "div", None);
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::Author);
        tree.style_rules[0].declarations[idx].value = injected;
        let result = cascade(&doc, &tree).expect("cascade Ok");
        result.computed[e].clone()
    }

    /// 4 side が全て互いに異なり、かつ initial (0) とも異なる margin fixture。
    /// `Sides::all` だと「1 side しか展開していない」実装と「4 side 展開した」
    /// 実装が区別できず、0 を使うと initial と区別できない。
    fn distinct_margin_sides() -> Sides<LengthOrAuto> {
        Sides {
            top: LengthOrAuto::Length(Length::Px(1.0)),
            right: LengthOrAuto::Length(Length::Px(2.0)),
            bottom: LengthOrAuto::Length(Length::Px(3.0)),
            left: LengthOrAuto::Length(Length::Px(4.0)),
        }
    }

    #[test]
    fn post_parse_margin_shorthand_before_longhand_lets_longhand_win() {
        // 注入後の declaration 列 (= `margin: 1px 2px 3px 4px; margin-top: 10px`):
        //   `[0]` Margin(1,2,3,4)   ← 注入
        //   `[1]` MarginTop(10px)
        // spec §3 + §6.1 → top=10 (後方 longhand)、right/bottom/left=2/3/4。
        //
        // これが nqkj の報告する spec 違反方向。展開しない実装では
        // `PropertyKey::Margin` が `MarginTop` より後に適用されるので
        // 全 side が 1/2/3/4 になり top=10 が破壊される。
        let cv = cascade_with_post_parse_injection(
            "div { margin-left: 99px; margin-top: 10px }",
            0,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(10.0),
            "後方 longhand が order of appearance で勝つこと (§6.1)"
        );
        // 残り 3 side は shorthand 由来の per-side 値。全て assert するのは
        // 「shorthand を単に落とす」実装 (top=10 だが他が initial 0 になる) と
        // 「top arm だけ展開する」実装を弾くため。
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn post_parse_margin_shorthand_after_longhand_lets_shorthand_win() {
        // 鏡像方向 (`margin-top: 10px; margin: 1px 2px 3px 4px`):
        //   `[0]` MarginTop(10px)
        //   `[1]` Margin(1,2,3,4)   ← 注入
        // spec §6.1 → 全 side が shorthand 由来 = 1/2/3/4。
        //
        // 本方向は展開しない実装でも偶然一致するが、fix が「shorthand を
        // 常に負けさせる」誤った非対称化になっていないことを check する。
        let cv = cascade_with_post_parse_injection(
            "div { margin-top: 10px; margin-left: 99px }",
            1,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(1.0));
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn post_parse_padding_shorthand_before_longhand_lets_longhand_win() {
        // margin と同じ形を padding family でも check (展開 arm が family ごとに
        // 独立に書かれているため)。1/2/3/4px の意図は `distinct_margin_sides`
        // doc と同じ。
        let cv = cascade_with_post_parse_injection(
            "div { padding-left: 99px; padding-top: 10px }",
            0,
            PropertyValue::Padding(Sides {
                top: Length::Px(1.0),
                right: Length::Px(2.0),
                bottom: Length::Px(3.0),
                left: Length::Px(4.0),
            }),
        );
        assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(10.0));
        assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(2.0));
        assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(3.0));
        assert_eq!(cv.padding.left, ComputedLengthPercentage::Px(4.0));
    }

    #[test]
    fn post_parse_border_shorthand_before_longhand_lets_longhand_win() {
        // border は 4 side × 3 sub-property = 12 longhand に展開される。
        // width / style を per-side で全て違う値にして、12 arm が sink 経由でも
        // 落ちていないことを check する (style は computed width の gating にも
        // 効くので `None` を混ぜない — CSS Backgrounds 3 §3.3)。width の
        // 1/2/3/4px の意図は `distinct_margin_sides` doc と同じ。
        let cv = cascade_with_post_parse_injection(
            "div { border-left-width: 99px; border-top-width: 10px }",
            0,
            PropertyValue::Border(Sides {
                top: Border {
                    width: Length::Px(1.0),
                    style: BorderStyle::Solid,
                    color: BorderColor::Resolved(RED),
                },
                right: Border {
                    width: Length::Px(2.0),
                    style: BorderStyle::Dashed,
                    color: BorderColor::Resolved(BLUE),
                },
                bottom: Border {
                    width: Length::Px(3.0),
                    style: BorderStyle::Dotted,
                    color: BorderColor::CurrentColor,
                },
                left: Border {
                    width: Length::Px(4.0),
                    style: BorderStyle::Double,
                    color: BorderColor::Resolved(RED),
                },
            }),
        );
        // top.width だけ後方 longhand が勝つ。
        assert_eq!(cv.border.top.width, ComputedLength(10.0));
        assert_eq!(cv.border.right.width, ComputedLength(2.0));
        assert_eq!(cv.border.bottom.width, ComputedLength(3.0));
        assert_eq!(cv.border.left.width, ComputedLength(4.0));
        // style / color は shorthand 由来のまま per-side に残る。
        assert_eq!(cv.border.top.style, BorderStyle::Solid);
        assert_eq!(cv.border.right.style, BorderStyle::Dashed);
        assert_eq!(cv.border.bottom.style, BorderStyle::Dotted);
        assert_eq!(cv.border.left.style, BorderStyle::Double);
        assert_eq!(cv.border.top.color, BorderColor::Resolved(RED));
        assert_eq!(cv.border.right.color, BorderColor::Resolved(BLUE));
        assert_eq!(cv.border.bottom.color, BorderColor::CurrentColor);
        assert_eq!(cv.border.left.color, BorderColor::Resolved(RED));
    }

    #[test]
    fn post_parse_shorthand_injection_propagates_important() {
        // 展開時の `!important` copy (spec §3 "Declaring a shorthand property
        // to be !important is equivalent to declaring all of its sub-properties
        // to be !important.") が element cascade 入口の展開でも保たれること。
        //
        // 注入した shorthand は `!important` を継承する (`[0]` は `!important`
        // 付きで parse される) ので、後方の normal longhand には**負けない**。
        //
        // ⚠️ **本 test は展開の有無を区別しない** (§8.2 spec lens が hunk revert
        // で実測)。展開しない実装でも `PropertyKey` 宣言順 (`Margin` が
        // `MarginTop..MarginLeft` より後) のせいで `Margin` slot が最後に
        // atomic 適用され、偶然同じ 1/2/3/4 になるため。`!important` 方向で
        // 展開の有無を実際に区別するのは
        // `post_parse_important_longhand_survives_later_normal_shorthand` で
        // あり、本 test は「展開が important flag を落としていない」だけを見る
        // 弱い guard である (`..._after_longhand_lets_shorthand_win` と同種)。
        let cv = cascade_with_post_parse_injection(
            "div { margin-left: 99px !important; margin-top: 10px }",
            0,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(1.0),
            "important shorthand 由来の MarginTop が normal longhand に勝つこと"
        );
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    #[test]
    fn post_parse_important_longhand_survives_later_normal_shorthand() {
        // 注入後の declaration 列
        // (= `margin-top: 10px !important; margin: 1px 2px 3px 4px`):
        //   `[0]` MarginTop(10px) !important
        //   `[1]` Margin(1,2,3,4)  normal   ← 注入 (`important` は false のまま)
        //
        // CSS Cascading L4 §6.1 <https://www.w3.org/TR/css-cascade-4/#cascade-sort>
        // の cascade sort は Origin and Importance を Order of Appearance
        // **より上位**に置く。したがって後方の normal shorthand は前方の
        // important longhand に勝てない → top=10。残り 3 side は shorthand 由来
        // = 2/3/4。
        //
        // **`!important` 方向で「展開しない実装」と区別できるのは本 test 群では
        // これだけである** — 展開しないと `PropertyKey::Margin` slot が独立
        // winner になり、discriminant 順 (`Margin` > `MarginTop`) で atomic
        // 上書きして top=1 になる。同じ importance を両者に持たせた
        // `post_parse_shorthand_injection_propagates_important` では
        // 偶然一致してしまい区別できない。
        //
        // (鏡像入力 `margin: 1,2,3,4; margin-top: 10px !important` や
        // padding / border family の同型入力も同様に区別する — 本 test が
        // 唯一というわけではない。coverage 追加は歓迎。)
        let cv = cascade_with_post_parse_injection(
            "div { margin-top: 10px !important; margin-left: 99px }",
            1,
            PropertyValue::Margin(distinct_margin_sides()),
        );
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(10.0),
            "important longhand が後方の normal shorthand 由来 longhand に勝つこと \
             (§6.1 Origin and Importance)"
        );
        assert_eq!(cv.margin.right, ComputedLengthPercentageOrAuto::Px(2.0));
        assert_eq!(cv.margin.bottom, ComputedLengthPercentageOrAuto::Px(3.0));
        assert_eq!(cv.margin.left, ComputedLengthPercentageOrAuto::Px(4.0));
    }

    // ── HTML LS §15.3.9 "Margin collapsing quirks"
    // (push_margin_collapsing_quirk_declarations) ──
    //
    // These tests build a UA stylesheet mimicking minimal.css's real
    // `blockquote, figure, listing, p, plaintext, pre, xmp { margin-top:
    // 1em; margin-bottom: 1em; }` default-margin rule (using an
    // arbitrary-but-nonzero 16px so a zeroed vs. unaffected side is always
    // unambiguous), via `RuleTree::empty()` + `add_stylesheet(_,
    // Origin::UserAgent)` directly rather than `build_rule_tree` (which
    // only ever tags DOM `<style>` elements as `Origin::Author` —
    // `build_rule_tree_produces_author_origin_for_dom_style_elements` in
    // `ruletree.rs` pins that).

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

    // ── float / clear wire-through (CSS2 §9.5, §9.5.2) ──

    #[test]
    fn float_wired_through_cascade_from_inline_style() {
        // <p style="float: left"> → ComputedValues.float に FloatValue::Left
        // が届く。parser → PropertyValue::Float → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。sibling (z-index) の
        // wire-through pattern を踏襲。
        use crate::property::FloatValue;
        let cv = cascade_doc("", "p", Some("float: left"));
        assert_eq!(cv.float, FloatValue::Left);
    }

    #[test]
    fn float_non_inherited_child_starts_from_initial() {
        // CSS2 §9.5.1 propdef: "Inherited: no". sibling:
        // `z_index_non_inherited_child_starts_from_initial` と同じ pattern。
        use crate::property::FloatValue;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("float: right"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].float, FloatValue::Right);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].float,
            FloatValue::None,
            "float must not inherit from parent (CSS2 §9.5.1 Inherited: no)"
        );
    }

    #[test]
    fn clear_wired_through_cascade_from_inline_style() {
        // <p style="clear: both"> → ComputedValues.clear に ClearValue::Both
        // が届く。parser → PropertyValue::Clear → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。sibling (z-index) の
        // wire-through pattern を踏襲。
        use crate::property::ClearValue;
        let cv = cascade_doc("", "p", Some("clear: both"));
        assert_eq!(cv.clear, ClearValue::Both);
    }

    #[test]
    fn clear_non_inherited_child_starts_from_initial() {
        // CSS2 §9.5.2 propdef: "Inherited: no". sibling:
        // `z_index_non_inherited_child_starts_from_initial` と同じ pattern。
        use crate::property::ClearValue;
        let mut doc = TestDoc::new();
        let div = doc.push_element(0, "div", Some("clear: left"));
        let span = doc.push_element(div, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[div].clear, ClearValue::Left);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].clear,
            ClearValue::None,
            "clear must not inherit from parent (CSS2 §9.5.2 Inherited: no)"
        );
    }

    #[test]
    fn deferred_projection_covers_supported_shorthands() {
        use crate::property::PropertyKey;

        fn parse_static(name: &str, source: &str) -> PropertyValue {
            let mut input = ParserInput::new(source);
            let mut parser = Parser::new(&mut input);
            let value = parse_value(name, &mut parser).expect("valid static shorthand");
            parser
                .expect_exhausted()
                .expect("static shorthand is exhaustive");
            value
        }

        let cases = vec![
            (
                "padding",
                "1px 2px 3px 4px",
                vec![
                    PropertyKey::PaddingTop,
                    PropertyKey::PaddingRight,
                    PropertyKey::PaddingBottom,
                    PropertyKey::PaddingLeft,
                ],
            ),
            (
                "margin-inline",
                "1px 2px",
                vec![PropertyKey::MarginLeft, PropertyKey::MarginRight],
            ),
            (
                "margin-block",
                "1px 2px",
                vec![PropertyKey::MarginTop, PropertyKey::MarginBottom],
            ),
            (
                "padding-inline",
                "1px 2px",
                vec![PropertyKey::PaddingLeft, PropertyKey::PaddingRight],
            ),
            (
                "padding-block",
                "1px 2px",
                vec![PropertyKey::PaddingTop, PropertyKey::PaddingBottom],
            ),
            (
                "margin",
                "1px 2px 3px 4px",
                vec![
                    PropertyKey::MarginTop,
                    PropertyKey::MarginRight,
                    PropertyKey::MarginBottom,
                    PropertyKey::MarginLeft,
                ],
            ),
            (
                "border",
                "1px solid red",
                vec![
                    PropertyKey::BorderTopWidth,
                    PropertyKey::BorderTopStyle,
                    PropertyKey::BorderTopColor,
                    PropertyKey::BorderRightWidth,
                    PropertyKey::BorderRightStyle,
                    PropertyKey::BorderRightColor,
                    PropertyKey::BorderBottomWidth,
                    PropertyKey::BorderBottomStyle,
                    PropertyKey::BorderBottomColor,
                    PropertyKey::BorderLeftWidth,
                    PropertyKey::BorderLeftStyle,
                    PropertyKey::BorderLeftColor,
                ],
            ),
            (
                "outline",
                "auto 2px red",
                vec![
                    PropertyKey::OutlineWidth,
                    PropertyKey::OutlineStyle,
                    PropertyKey::OutlineColor,
                ],
            ),
            (
                "overflow",
                "hidden scroll",
                vec![PropertyKey::OverflowX, PropertyKey::OverflowY],
            ),
            (
                "text-decoration",
                "underline wavy red",
                vec![
                    PropertyKey::TextDecorationLine,
                    PropertyKey::TextDecorationThickness,
                    PropertyKey::TextDecorationStyle,
                    PropertyKey::TextDecorationColor,
                ],
            ),
            (
                "flex",
                "2 3 10px",
                vec![
                    PropertyKey::FlexGrow,
                    PropertyKey::FlexShrink,
                    PropertyKey::FlexBasis,
                ],
            ),
            (
                "gap",
                "1px 2px",
                vec![PropertyKey::RowGap, PropertyKey::ColumnGap],
            ),
            (
                "place-content",
                "center space-between",
                vec![PropertyKey::AlignContent, PropertyKey::JustifyContent],
            ),
            (
                "grid-row",
                "2 / 5",
                vec![PropertyKey::GridRowStart, PropertyKey::GridRowEnd],
            ),
            (
                "grid-column",
                "main-start / main-end",
                vec![PropertyKey::GridColumnStart, PropertyKey::GridColumnEnd],
            ),
            (
                "place-items",
                "center stretch",
                vec![PropertyKey::AlignItems, PropertyKey::JustifyItems],
            ),
            (
                "place-self",
                "center stretch",
                vec![PropertyKey::AlignSelf, PropertyKey::JustifySelf],
            ),
            (
                "background",
                "url(a.png) top / cover no-repeat fixed border-box red",
                vec![
                    PropertyKey::BackgroundColor,
                    PropertyKey::BackgroundImage,
                    PropertyKey::BackgroundRepeat,
                    PropertyKey::BackgroundAttachment,
                    PropertyKey::BackgroundPosition,
                    PropertyKey::BackgroundSize,
                    PropertyKey::BackgroundClip,
                    PropertyKey::BackgroundOrigin,
                ],
            ),
            (
                "font",
                "italic small-caps bold 12px/1.5 serif",
                vec![
                    PropertyKey::FontStyle,
                    PropertyKey::FontVariantCaps,
                    PropertyKey::FontWeight,
                    PropertyKey::FontSize,
                    PropertyKey::LineHeight,
                    PropertyKey::FontFamily,
                ],
            ),
            (
                "font",
                "italic larger serif",
                vec![
                    PropertyKey::FontStyle,
                    PropertyKey::FontVariantCaps,
                    PropertyKey::FontWeight,
                    PropertyKey::FontSize,
                    PropertyKey::LineHeight,
                    PropertyKey::FontFamily,
                ],
            ),
        ];

        for (name, source, keys) in cases {
            let value = parse_static(name, source);
            for key in keys {
                assert!(project_deferred_value(value.clone(), key).is_some());
            }
            assert!(project_deferred_value(value, PropertyKey::Color).is_none());
        }
        assert!(project_deferred_value(PropertyValue::Color(RED), PropertyKey::Width).is_none());
    }

    #[test]
    fn direct_apply_ignores_precomputed_custom_values() {
        let initial = SpecifiedValues::initial();
        let mut specified = initial.clone();
        apply_value(
            PropertyValue::CustomProperty(CustomProperty {
                name: "--unused".into(),
                value: "red".into(),
            }),
            &mut specified,
        );
        apply_value(
            PropertyValue::Deferred(DeferredValue {
                property: "color".into(),
                value: "red".into(),
                key: PropertyKey::Color,
            }),
            &mut specified,
        );
        assert_eq!(specified, initial);
    }

    #[test]
    fn math_helpers_cover_nested_and_rejected_forms() {
        assert_eq!(
            simplify_math_functions(r#"rgb(calc(1px + 1px), 0, 0)"#),
            Some("rgb(2px, 0, 0)".into())
        );
        assert_eq!(
            simplify_math_functions(r#""calc(1px)" /* calc(2px) */"#),
            Some(r#""calc(1px)" /* calc(2px) */"#.into())
        );
        assert_eq!(
            simplify_math_functions("calc(calc(1px))"),
            Some("1px".into())
        );
        assert_eq!(
            simplify_math_functions_at_depth("calc(1px)", MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            None
        );
        assert_eq!(simplify_math_functions("(calc(1px))"), Some("(1px)".into()));
        assert_eq!(simplify_math_functions("[calc(1px)]"), Some("[1px]".into()));
        assert_eq!(simplify_math_functions("{calc(1px)}"), Some("{1px}".into()));
        assert!(needs_css_token_separator("-", "a"));
        assert!(needs_css_token_separator("+", "2"));
        assert!(needs_css_token_separator(".", "2"));
        assert!(needs_css_token_separator("#", "a"));
        assert!(needs_css_token_separator("10", "--foo"));
        assert!(needs_css_token_separator("10", "_foo"));
        assert!(needs_css_token_separator("10", r"\66 oo"));
        assert!(needs_css_token_separator("10", "é"));
        assert!(needs_css_token_separator("#", "1"));
        assert_eq!(
            evaluate_math_function("calc", &"x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1)),
            None
        );
        assert_eq!(
            evaluate_math_function("calc", "1 + 2"),
            Some("3".to_owned())
        );
        assert_eq!(evaluate_math_function("calc", "1 + 2px"), None);
        // Additive zero vanishes across units (CSS Values 4 §10.7).
        assert_eq!(
            evaluate_math_function("calc", "50% + 0px"),
            Some("50%".to_owned())
        );
        assert_eq!(
            evaluate_math_function("calc", "0px + 50%"),
            Some("50%".to_owned())
        );
        assert_eq!(
            evaluate_math_function("calc", "10px + 0"),
            Some("10px".to_owned())
        );

        assert_eq!(evaluate_math_function("min", ""), None);
        assert_eq!(evaluate_math_function("min", "1px, 2em"), None);
        assert_eq!(evaluate_math_function("clamp", "1px, 2px"), None);
        assert_eq!(evaluate_math_function("clamp", "1px, 2em, 3px"), None);
        assert_eq!(evaluate_math_function("unknown", "1"), None);
        assert_eq!(
            serialize_math_value(MathValue {
                number: f32::INFINITY,
                unit: None,
            }),
            None
        );
        assert_eq!(
            serialize_math_value(MathValue {
                number: -0.0,
                unit: None,
            }),
            Some("0".to_owned())
        );
        let mut empty_values: Vec<MathValue> = Vec::new();
        assert_eq!(normalize_math_values(&mut empty_values), None);
        let mut overflowing_pair_left = MathValue {
            number: f32::MAX,
            unit: Some("in".to_owned()),
        };
        let mut overflowing_pair_right = MathValue {
            number: 1.0,
            unit: Some("px".to_owned()),
        };
        assert_eq!(
            normalize_math_pair(&mut overflowing_pair_left, &mut overflowing_pair_right),
            None
        );
        let mut overflowing_values = vec![
            MathValue {
                number: f32::MAX,
                unit: Some("in".to_owned()),
            },
            MathValue {
                number: 1.0,
                unit: Some("px".to_owned()),
            },
        ];
        assert_eq!(normalize_math_values(&mut overflowing_values), None);
        assert_eq!(
            serialize_math_value(MathValue {
                number: 1.0,
                unit: Some("x".repeat(MAX_SUBSTITUTED_VALUE_BYTES)),
            }),
            None
        );

        assert!(MathParser::new("1px 2px").parse().is_none());
        assert!(MathParser::new("1px+2px").parse().is_none());
        assert!(MathParser::new("1px +2px").parse().is_none());
        assert!(MathParser::new("1px + 2em").parse().is_none());
        assert_eq!(
            MathParser::new("-2px").parse().map(|value| value.number),
            Some(-2.0)
        );
        assert_eq!(
            MathParser::new("5px - 2px")
                .parse()
                .map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("3e38px + 3e38px").parse().is_none());
        assert!(MathParser::new("2px *").parse().is_none());
        assert_eq!(
            MathParser::new("2 * 3px").parse().map(|value| value.unit),
            Some(Some("px".to_owned()))
        );
        assert_eq!(
            MathParser::new("2px * 3").parse().map(|value| value.number),
            Some(6.0)
        );
        assert_eq!(
            MathParser::new("6px / 2").parse().map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("2px * 3px").parse().is_none());
        assert!(MathParser::new("2px / 0").parse().is_none());
        assert!(MathParser::new("3e38 * 3").parse().is_none());
        assert_eq!(
            MathParser::new("(1px + 2px)")
                .parse()
                .map(|value| value.number),
            Some(3.0)
        );
        assert!(MathParser::new("(1px").parse().is_none());
        assert!(MathParser::new(".").parse().is_none());
        assert_eq!(
            MathParser::new("1.5px").parse().map(|value| value.number),
            Some(1.5)
        );
        assert_eq!(
            MathParser::new("1e-2px").parse().map(|value| value.number),
            Some(0.01)
        );
        assert!(MathParser::new("1e+px").parse().is_none());
        assert_eq!(
            MathParser::new("50%").parse().map(|value| value.unit),
            Some(Some("%".to_owned()))
        );

        assert_eq!(
            split_top_level_commas(r#""a,b", 1px"#),
            Some(vec![r#""a,b""#, "1px"])
        );
        assert_eq!(
            split_top_level_commas("1px /*,*/, 2px"),
            Some(vec!["1px /*,*/", "2px"])
        );
        assert_eq!(
            split_top_level_commas("min(1px, 2px), 3px"),
            Some(vec!["min(1px, 2px)", "3px"])
        );
        assert_eq!(
            split_top_level_commas(r"foo\,bar, baz"),
            Some(vec![r"foo\,bar", "baz"])
        );
        assert_eq!(split_top_level_commas("{a,b}, c"), Some(vec!["{a,b}", "c"]));
        assert_eq!(split_top_level_commas(")"), None);
        assert_eq!(split_top_level_commas("[a,b"), None);
        let nested = format!(
            "{}1{}",
            "(".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            ")".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert!(MathParser::new(&nested).parse().is_none());
        let too_deep = format!(
            "{}1{}",
            "[".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            "]".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert_eq!(split_top_level_commas(&too_deep), None);
    }

    #[test]
    fn variable_scanner_handles_literals_bounds_and_fallbacks() {
        let mut local = HashMap::from([(SmolStr::from("--a"), SmolStr::from("red"))]);
        let inherited = CustomPropertyEnvironment::from_map(HashMap::new());
        {
            let mut resolver = CustomPropertyResolver {
                local: &local,
                inherited: &inherited,
                cycle_members: HashSet::new(),
                memo: HashMap::new(),
                resolving: Vec::new(),
                budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
            };
            assert_eq!(resolver.resolve("--a", 0), Some("red".into()));
            assert_eq!(resolver.resolve("--a", 0), Some("red".into()));
        }
        let mut resolver = CustomPropertyResolver {
            local: &local,
            inherited: &inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: vec!["--a".into()],
            budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
        };
        assert_eq!(resolver.resolve("--a", 0), None);
        let branching_local = HashMap::from([
            (
                SmolStr::from("--root"),
                SmolStr::from("var(--shared) var(--shared)"),
            ),
            (SmolStr::from("--shared"), SmolStr::from("red")),
        ]);
        let branching_inherited = CustomPropertyEnvironment::from_map(HashMap::new());
        let mut resolver = CustomPropertyResolver {
            local: &branching_local,
            inherited: &branching_inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: Vec::new(),
            budget: VariableResolutionBudget::new(MAX_VARIABLE_RESOLUTION_STEPS),
        };
        assert_eq!(resolver.resolve("--root", 0), Some("red red".into()));
        assert!(resolver.memo.contains_key(&(SmolStr::from("--shared"), 1)));

        let budget_local = HashMap::from([
            (SmolStr::from("--a"), SmolStr::from("var(--b)")),
            (SmolStr::from("--b"), SmolStr::from("var(--c)")),
            (SmolStr::from("--c"), SmolStr::from("red")),
        ]);
        let mut resolver = CustomPropertyResolver {
            local: &budget_local,
            inherited: &inherited,
            cycle_members: HashSet::new(),
            memo: HashMap::new(),
            resolving: Vec::new(),
            budget: VariableResolutionBudget::new(2),
        };
        assert_eq!(resolver.resolve("--a", 0), None);
        assert!(resolver.budget.exhausted);
        local.insert("--broken".into(), "\"unterminated".into());
        assert!(find_cycle_members(&local).is_empty());

        let mut references = Vec::new();
        assert!(
            collect_var_references(
                r#""var(--ignored)" /* var(--also-ignored) */ var(--used)"#,
                &mut references
            )
            .is_ok()
        );
        assert_eq!(references, vec![SmolStr::from("--used")]);
        assert!(collect_var_references("\"unterminated", &mut Vec::new()).is_err());
        assert!(collect_var_references("/* unterminated", &mut Vec::new()).is_err());

        fn no_resolution(_: &str) -> Option<SmolStr> {
            None
        }
        assert_eq!(
            substitute_vars(
                r#""var(--ignored)" /* var(--also-ignored) */ blue"#,
                &mut no_resolution,
                0
            ),
            Some(r#""var(--ignored)" /* var(--also-ignored) */ blue"#.into())
        );
        assert_eq!(
            substitute_vars("var(--missing, blue)", &mut no_resolution, 0),
            Some("blue".into())
        );
        assert_eq!(
            substitute_vars(
                "var(--missing, var(--also-missing, red))",
                &mut no_resolution,
                0
            ),
            Some("red".into())
        );
        assert_eq!(
            substitute_vars(
                "var(--x)",
                &mut |_| Some("x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1).into()),
                0
            ),
            None
        );
        assert_eq!(
            substitute_vars("var(--x)", &mut |_| None, MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            None
        );

        assert_eq!(skip_css_string(r#""a\"b""#, 0), Some(6));
        assert_eq!(skip_css_string("\"unterminated", 0), None);
        assert_eq!(skip_css_comment("/* comment */", 0), Some(13));
        assert_eq!(skip_css_comment("/* unterminated", 0), None);
        assert_eq!(skip_css_escape("x", 0), None);
        assert_eq!(skip_css_escape("\\\n", 0), None);
        assert_eq!(skip_css_escape(r"\31 ", 0), Some(4));
        assert_eq!(skip_css_escape("\\31\r\n", 0), Some(5));
        assert_eq!(
            split_var_arguments("--x, var(--y, red)"),
            Some((SmolStr::from("--x"), Some("var(--y, red)")))
        );
        assert_eq!(
            split_var_arguments("--x, \"a,b\" /* comment */"),
            Some((SmolStr::from("--x"), Some("\"a,b\" /* comment */")))
        );
        assert_eq!(
            split_var_arguments("--x, foo(bar)"),
            Some((SmolStr::from("--x"), Some("foo(bar)")))
        );
        assert_eq!(split_var_arguments("color, red"), None);
        assert_eq!(split_var_arguments("\"--x\", red"), None);
        assert_eq!(
            split_var_arguments("--x /* comment */, red"),
            Some((SmolStr::from("--x"), Some("red")))
        );
        assert_eq!(split_var_arguments("--x(foo), red"), None);
        let (escaped_name, escaped_fallback) = split_var_arguments("--x\\,, red").unwrap();
        assert_eq!(escaped_name, "--x,");
        assert_eq!(escaped_fallback, Some("red"));
        assert_eq!(split_var_arguments("--x)"), None);
    }

    #[test]
    fn variable_substitution_preserves_number_identifier_boundary() {
        assert_eq!(
            substitute_vars("var(--n)--foo", &mut |_| Some("10".into()), 0),
            Some("10 --foo".into())
        );
        assert_eq!(
            substitute_vars("var(--missing, [foo)bar])", &mut |_| None, 0),
            Some("[foo)bar]".into())
        );
    }

    #[test]
    fn component_value_scanners_bound_nesting_and_blocks() {
        let nested = format!(
            "{}var(--x){}",
            "[".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1),
            "]".repeat(MAX_VARIABLE_RESOLUTION_DEPTH + 1)
        );
        assert_eq!(find_function_tokens(&nested, &["var"]), None);
        assert_eq!(split_top_level_commas("[a,b], c"), Some(vec!["[a,b]", "c"]));
    }

    #[test]
    fn custom_property_substitutes_into_inherited_color() {
        let cv = cascade_doc("", "p", Some("--accent: red; color: var(--accent)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn var_substitutes_inside_a_nested_function() {
        let cv = cascade_doc("", "p", Some("--red: 255; color: rgb(var(--red), 0, 0)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn custom_property_inherits_to_child() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("--accent: blue"));
        let child = doc.push_element(parent, "p", Some("color: var(--accent, red)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, BLUE);
    }

    /// A deep chain keeps one local custom-property delta per element instead
    /// of cloning the complete inherited map into each computed value.
    #[test]
    fn deep_custom_property_chain_keeps_persistent_environment_deltas() {
        use std::fmt::Write as _;

        const DEPTH: usize = 256;
        let mut doc = TestDoc::new();
        let mut parent = 0;
        let mut ids = Vec::with_capacity(DEPTH);
        for index in 0..DEPTH {
            let mut style = String::new();
            write!(style, "--chain-{index}: {index}px").unwrap();
            let id = doc.push_element(parent, "div", Some(&style));
            ids.push(id);
            parent = id;
        }

        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");

        // Every environment owns exactly this element's declaration. The
        // complete chain is shared through parent Arc pointers, so retained
        // local entries are O(N), not O(N²).
        let mut total_local_entries = 0;
        for (index, id) in ids.iter().copied().enumerate() {
            let environment = &result.computed[id].custom_properties;
            assert_eq!(environment.local_entry_count(), 1);
            total_local_entries += environment.local_entry_count();
            if index > 0 {
                let parent_environment = environment
                    .parent_environment()
                    .expect("non-root custom environment has a parent");
                // cov:ignore: panic-message literal only executed on assertion
                // failure, which does not happen while this test passes.
                assert!(
                    std::sync::Arc::ptr_eq(
                        parent_environment,
                        &result.computed[ids[index - 1]].custom_properties
                    ),
                    "custom environment must share the immediate ancestor's environment"
                );
            }
        }
        assert_eq!(total_local_entries, DEPTH);
        let leaf_environment = &result.computed[*ids.last().unwrap()].custom_properties;
        assert_eq!(leaf_environment.get("--chain-0"), Some("0px".into()));
        assert_eq!(leaf_environment.get("--chain-255"), Some("255px".into()));
    }

    #[test]
    fn invalid_var_keeps_inherited_property_value() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("color: red"));
        let child = doc.push_element(parent, "p", Some("color: var(--missing)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, RED);
    }

    #[test]
    fn invalid_var_uses_initial_for_non_inherited_property() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("width: 20px"));
        let child = doc.push_element(parent, "p", Some("width: var(--missing)"));
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            result.computed[child].width,
            ComputedLengthPercentageOrAuto::Auto
        );
    }

    #[test]
    fn invalid_custom_property_overrides_inherited_value() {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("--accent: red"));
        let child = doc.push_element(
            parent,
            "p",
            Some("--accent: var(--missing); color: var(--accent, blue)"),
        );
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed[child].color, BLUE);
    }

    #[test]
    fn missing_custom_property_uses_var_fallback() {
        let cv = cascade_doc("", "p", Some("color: var(--missing, blue)"));
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn custom_property_rejects_top_level_bang_except_important() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: !not-important; color: var(--accent, blue)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn custom_property_important_wins_and_is_stripped_before_substitution() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: red !important; --accent: blue; color: var(--accent)"),
        );
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn custom_property_names_are_case_sensitive() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--accent: red; --Accent: blue; color: var(--Accent)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn cyclic_custom_property_uses_var_fallback() {
        let cv = cascade_doc(
            "",
            "p",
            Some("--a: var(--b); --b: var(--a); color: var(--a, blue)"),
        );
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn fallback_references_do_not_rescue_a_custom_property_cycle() {
        let cv = cascade_doc(
            "",
            "p",
            Some(
                "--a: var(--b, red); --b: var(--a, blue); \
                 color: var(--a)",
            ),
        );
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn overly_deep_finite_variable_chain_is_bounded() {
        use std::fmt::Write as _;

        let mut css = String::new();
        for index in 0..256 {
            write!(css, "--v{index}: var(--v{}); ", index + 1).unwrap();
        }
        css.push_str("--v256: red; color: var(--v0)");

        let cv = cascade_doc("", "p", Some(&css));
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn overly_deep_nested_var_fallback_is_bounded() {
        let mut fallback = String::from("red");
        for index in 0..256 {
            fallback = format!("var(--missing-{index}, {fallback})");
        }
        let css = format!("color: {fallback}");

        let cv = cascade_doc("", "p", Some(&css));
        assert_eq!(cv.color, CssColor::BLACK);
    }

    #[test]
    fn calc_substituted_custom_property_reaches_width() {
        let cv = cascade_doc(
            "",
            "div",
            Some("--spacing: 10px; width: calc(var(--spacing) + 5px)"),
        );
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(15.0));
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_non_calc_prefix() {
        assert_eq!(parse_simple_calc_length_percentage("foo(50%)"), None);
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_missing_closing_paren() {
        assert_eq!(parse_simple_calc_length_percentage("calc(50%"), None);
    }

    #[test]
    fn parse_simple_calc_length_percentage_handles_nested_parens() {
        assert_eq!(
            parse_simple_calc_length_percentage("calc((50%) + 10px)"),
            Some(CalcLengthPercentage {
                percent: 50.0,
                px: 10.0
            })
        );
    }

    #[test]
    fn parse_simple_calc_length_percentage_handles_single_term() {
        assert_eq!(
            parse_simple_calc_length_percentage("calc(50%)"),
            Some(CalcLengthPercentage {
                percent: 50.0,
                px: 0.0
            })
        );
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_unknown_unit() {
        assert_eq!(
            parse_simple_calc_length_percentage("calc(50deg + 10px)"),
            None
        );
    }

    #[test]
    fn inherited_border_radius_handles_percent() {
        assert_eq!(
            inherited_border_radius(ComputedLengthPercentage::Percent(25.0)),
            Length::Percent(25.0)
        );
        assert_eq!(
            inherited_border_radius(ComputedLengthPercentage::Px(4.0)),
            Length::Px(4.0)
        );
    }

    #[test]
    #[should_panic(expected = "pick_winners は空の scratch buffer を要求する")]
    fn pick_winners_panics_on_non_empty_scratch_buffer() {
        let mut winners: Vec<Option<RankedDecl>> = vec![Some(RankedDecl {
            rank: 0,
            specificity: 0,
            source_order: 0,
            idx: 0,
        })];
        pick_winners(&[], &mut winners);
    }

    #[test]
    fn parse_simple_calc_length_percentage_rejects_non_finite_term() {
        // `1e40` overflows f32 to infinity — the parsed term's `number` is
        // no longer finite, which must be rejected rather than propagated
        // into a `CalcLengthPercentage`.
        assert_eq!(
            parse_simple_calc_length_percentage("calc(1e40% + 10px)"),
            None
        );
    }

    #[test]
    fn invalid_math_declaration_is_dropped_before_cascade() {
        let cv = cascade_doc("", "div", Some("width: 10px; width: calc(foo)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    #[test]
    fn hash_prefixed_var_text_is_not_substituted() {
        let cv = cascade_doc("", "div", Some("color: red; color: #var(--missing)"));
        assert_eq!(cv.color, RED);
    }

    #[test]
    fn min_max_and_clamp_reach_width() {
        let min = cascade_doc("", "div", Some("width: min(20px, 10px)"));
        let max = cascade_doc("", "div", Some("width: max(10px, 20px)"));
        let clamp = cascade_doc("", "div", Some("width: clamp(5px, 20px, 10px)"));
        assert_eq!(min.width, ComputedLengthPercentageOrAuto::Px(10.0));
        assert_eq!(max.width, ComputedLengthPercentageOrAuto::Px(20.0));
        assert_eq!(clamp.width, ComputedLengthPercentageOrAuto::Px(10.0));
    }

    #[test]
    fn var_substitution_does_not_join_adjacent_tokens() {
        // `var(--n)px` is not a valid way to form a dimension: substitution
        // happens at token level, so the result is the two-token sequence
        // `10` + `px`, not a newly reparsed `10px` token.
        let cv = cascade_doc("", "div", Some("--n: 10; width: var(--n)px"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
        assert_eq!(simplify_math_functions("calc(10)px"), Some("10 px".into()));
    }

    #[test]
    fn var_substitution_does_not_create_a_function_token() {
        let cv = cascade_doc("", "div", Some("--fn: rgb; color: var(--fn)(255, 0, 0)"));
        assert_eq!(cv.color, CssColor::BLACK);
        assert_eq!(
            substitute_vars("var(--fn)(255, 0, 0)", &mut |_| Some("rgb".into()), 0),
            Some("rgb (255, 0, 0)".into())
        );
    }

    #[test]
    fn escaped_math_function_names_are_evaluated_after_token_decoding() {
        assert_eq!(simplify_math_functions(r"c\61 lc(1px)"), Some("1px".into()));
        let cv = cascade_doc("", "div", Some(r"width: c\61 lc(1px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(1.0));
    }

    #[test]
    fn css_comments_are_whitespace_inside_math_functions() {
        assert_eq!(
            simplify_math_functions("calc(1px /* comment */ + 2px)"),
            Some("3px".into())
        );
        let cv = cascade_doc("", "div", Some("width: calc(1px /* comment */ + 2px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(3.0));
    }

    #[test]
    fn variable_resolution_budget_rejects_excess_expansions() {
        let mut budget = VariableResolutionBudget::new(2);
        assert!(budget.consume());
        assert!(budget.consume());
        assert!(!budget.consume());

        let mut budget = VariableResolutionBudget::new(0);
        assert_eq!(
            substitute_vars_with_budget("var(--x)", &mut |_| Some("red".into()), 0, &mut budget),
            None
        );
    }

    #[test]
    fn has_css_whitespace_before_at_start_returns_false() {
        assert!(!MathParser::new("1px").has_css_whitespace_before(0));
    }

    #[test]
    fn compatible_absolute_length_units_are_normalized_in_math() {
        let cv = cascade_doc("", "div", Some("width: calc(1in + 96px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(192.0));
        assert_eq!(
            evaluate_math_function("calc", "1in + 96px"),
            Some("192px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("min", "1in, 96px"),
            Some("96px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("max", "1in, 96px"),
            Some("96px".to_owned())
        );
        assert_eq!(
            evaluate_math_function("clamp", "1in, 96px, 2in"),
            Some("96px".to_owned())
        );
    }

    #[test]
    fn mixed_length_percentage_math_is_preserved_until_used_value_resolution() {
        // The computed layer preserves the percentage and px terms; the
        // containing-block basis is only available in the layout bridge.
        let cv = cascade_doc("", "div", Some("width: calc(10px + 5%)"));
        assert_eq!(
            cv.width,
            ComputedLengthPercentageOrAuto::Calc(crate::property::CalcLengthPercentage {
                percent: 5.0,
                px: 10.0,
            })
        );
    }

    #[test]
    fn oversized_variable_and_math_inputs_are_rejected() {
        let oversized = "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1);
        assert!(collect_var_references(&oversized, &mut Vec::new()).is_err());
        assert_eq!(split_top_level_commas(&oversized), None);
        assert_eq!(split_var_arguments(&oversized), None);
    }

    #[test]
    fn cascade_records_non_ua_margin_winners() {
        let mut doc = TestDoc::new();
        let ua_body = doc.push_element(0, "body", None);
        let author_body = doc.push_element(0, "body", Some("margin: 8px"));
        let mut rules = build_rule_tree(&doc);
        rules.add_stylesheet("body { margin: 8px }", Origin::UserAgent);

        let result = cascade(&doc, &rules).expect("cascade Ok");
        assert_eq!(
            result.non_ua_margin_sides[ua_body],
            Sides::all(false),
            "the UA body's default margin must remain identifiable as UA-origin"
        );
        assert_eq!(
            result.non_ua_margin_sides[author_body],
            Sides::all(true),
            "an authored margin equal to the UA value must remain distinguishable"
        );
    }

    #[test]
    fn clamp_min_wins_when_bounds_are_reversed() {
        let cv = cascade_doc("", "div", Some("width: clamp(20px, 0px, 10px)"));
        assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Px(20.0));
    }
}
