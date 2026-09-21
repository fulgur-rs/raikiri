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
//!   [`crate::specified::SpecifiedValues`] を seed し、その node の全 winner を `apply_value` で
//!   適用する。この段では length は specified 表現のまま。
//! - **phase 2: font-size の絶対化** — **親の** computed font-size 基準。
//! - **phase 2.5: line-height の絶対化** — 自 node の (今確定した) font-size
//!   基準。`lh`/`rlh` の自己参照基準の非対称は
//!   [`crate::resolve::resolve_line_height`] doc が canonical。
//! - **phase 3: 残り全 length の絶対化** — **自 node の** computed font-size /
//!   line-height 基準。
//!
//! 2 / 2.5 / 3 は [`crate::specified::SpecifiedValues::finalize`] に閉じている。分離が必要な理由は
//! [`crate::specified`] の module doc を参照 (`padding: 2em` の基準となる
//! `font-size` はその node の**全** winner を適用し終えるまで確定しないため、
//! winner 適用の途中で絶対化することはできない)。

use std::collections::HashMap;

use crate::PseudoElem;
use crate::computed::ComputedValues;
use crate::counter_style::CounterStyleRegistry;
use crate::error::CascadeError;
use crate::media::MediaContext;
use crate::page::{PageCascadeResult, PageContextQuery, PageInheritance, cascade_page};
use crate::property::{Sides, WritingMode};
#[cfg_attr(not(test), allow(unused_imports))]
use crate::ruletree::Origin;
use crate::ruletree::RuleTree;
use crate::style_dom::{StyleDom, StyleNode, StyleNodeId, StyleNodeKind};

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
mod lang;
mod selector_match;
// Re-exported purely so intra-doc links elsewhere in the crate (e.g.
// `crate::cascade::language_range_matches` in lib.rs) can resolve — this
// module's own top-level API never calls into `lang` directly.
#[allow(unused_imports)]
pub(crate) use lang::*;
mod directionality;
pub(crate) use directionality::*;
mod custom_property;
mod html_quirks;
pub(crate) use custom_property::*;
mod inherit;
pub(crate) use inherit::*;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::test_support::*;
    use crate::ruletree::build_rule_tree;
    use crate::test_dom::TestDoc;

    #[test]
    fn empty_dom_root_has_initial() {
        let doc = TestDoc::new();
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).unwrap();
        assert_eq!(r.computed[0], ComputedValues::initial());
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
    fn cascade_result_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<CascadeResult>();
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

    #[test]
    fn deep_nesting_5000_cascade_no_overflow() {
        let (doc, deepest) = deep_chain_doc(5000);
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(result.computed.len(), doc.nodes.len());
        assert_eq!(result.computed[deepest].color, RED);
    }

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
}
