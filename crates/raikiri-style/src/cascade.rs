//! CSS cascade + inheritance walk (M1.4)。
//!
//! 2 phase:
//! 1. per-node "cascaded values" 決定 — matching rule + inline style の候補集合から
//!    specificity + !important + source order で winner を選択
//! 2. inheritance walk — top-down DFS で親の computed value を継承 + 自 node の
//!    cascaded value で override

use std::collections::HashMap;

use cssparser::{Parser, ParserInput};
use selectors::parser::{Selector, SelectorList};

use crate::RaikiriSelectorImpl;
use crate::computed::ComputedValues;
use crate::error::CascadeError;
use crate::property::{PropertyKey, PropertyValue};
use crate::rule::parse_declaration_block;
use crate::ruletree::Origin;
use crate::ruletree::RuleTree;
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

/// Cascade 結果。
///
/// M1.4 では `computed` のみ populate。future field
/// (gcpm_directives / running_templates) は M5 で追加、`#[non_exhaustive]` の
/// 恩恵で non-breaking。
#[derive(Debug)]
#[non_exhaustive]
pub struct CascadeResult {
    /// Per-node computed values (NodeId.0 as usize で index)。
    /// Element / Text / Document 全 kind に populate、範囲外は panic (caller 責任)。
    pub computed: Vec<ComputedValues>,
}

/// DOM + RuleTree から per-node ComputedValues を produce。
///
/// M1.4 では常に `Ok` を返す (invalid CSS は既に build_rule_tree 段で silently
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
/// let result = cascade(dom, &rule_tree).expect("m1.4 では常に Ok");
/// let root_style: &ComputedValues = &result.computed[0];
/// # }
/// ```
pub fn cascade<D: StyleDom>(dom: &D, rule_tree: &RuleTree) -> Result<CascadeResult, CascadeError> {
    let mut cascaded: HashMap<StyleNodeId, Vec<CascadedDecl>> = HashMap::new();

    // Phase 1: per-node cascaded values を収集
    collect_cascaded(dom, dom.root_id(), rule_tree, &mut cascaded);

    // Phase 2: inheritance walk。
    //
    // raikiri-spike-37c roborev job 293 M1 finding: computed を Dom::node_count()
    // で pre-allocate する。resolve_inheritance の DFS は root reachable な node
    // のみを訪問するため、detached / unreachable node (foster-parenting transient、
    // strip 後の孤児 stub 等) には entry を作らない。しかし raikiri-spike-m1.23
    // contract `computed.len() == document.node_count()` は arena 全体を要求する
    // (`raikiri-dom::layout::preshape_text` / `raikiri-paint::text::draw_text_node`
    // が node_id で `computed[idx]` に直接 index する)。事前に initial() で埋めて
    // おき、DFS で visited slot を上書きする実装。
    let mut computed: Vec<ComputedValues> = vec![ComputedValues::initial(); dom.node_count()];
    resolve_inheritance(
        dom,
        dom.root_id(),
        &ComputedValues::initial(),
        &cascaded,
        &mut computed,
    );

    Ok(CascadeResult { computed })
}

/// selectors 由来の 32-bit specificity。u32 で完全順序比較。
type Specificity = u32;

/// inline style の specificity — spec §6.3 で `(1, 0, 0, 0)` に相当。
/// selectors crate は 32-bit packed で `id << 20 | class << 10 | element` を使うので、
/// inline の 1,0,0,0 相当は 1 << 30 とみなす (どの selector 由来 spec より大)。
const INLINE_SPECIFICITY: Specificity = 1 << 30;
/// inline style の source_order — 全 stylesheet rule より後 (最終出現扱い)。
const INLINE_SOURCE_ORDER: u32 = u32::MAX;

/// 1 candidate declaration = `(value, important, origin, specificity, source_order)`。
/// `collect_cascaded` が populate、`pick_winners` が rank 化して winner を選ぶ
/// (raikiri-spike-m1.22 で `Origin` を追加、clippy::type_complexity 回避のため alias 化)。
type CascadedDecl = (PropertyValue, bool, Origin, Specificity, u32);

/// Cascade origin + `!important` flag に基づく優先度 rank (raikiri-spike-m1.22)。
///
/// 高いほど勝つ。CSS Cascading L4 §6.4.4 の origin 反転扱いを表現:
/// - Normal   : UA < User < Author (Author が最強、UA が最弱)
/// - Important: UA > User > Author (反転、UA が最強)
///
/// M1 では User origin を扱わないので UA + Author の 2 段。
fn cascade_rank(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::Author, false) => 1,
        (Origin::Author, true) => 2,
        (Origin::UserAgent, true) => 3,
    }
}

/// `collect_cascaded` は本来 DFS で node を訪れるが、per-node の処理は他の
/// node の状態に依存しないため訪問順は無関係。overflow 回避のため explicit
/// `Vec` stack で iterative に書き換え (roborev job 199)。
fn collect_cascaded<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    rule_tree: &RuleTree,
    out: &mut HashMap<StyleNodeId, Vec<CascadedDecl>>,
) {
    let mut stack: Vec<StyleNodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            // silent bug fix: 従来 template 内 element にも rule matching が走り
            // Vec<CascadedDecl> が waste で膨らんでいた。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let mut per_node = Vec::new();
                // stylesheet rule matching
                let tag = elem.tag_name();
                for rule in &rule_tree.style_rules {
                    if let Some(spec) = match_by_tag(&rule.selectors, tag) {
                        for decl in &rule.declarations {
                            per_node.push((
                                decl.value.clone(),
                                decl.important,
                                rule.origin,
                                spec,
                                rule.source_order,
                            ));
                        }
                    }
                }
                // inline style
                if let Some(source) = elem.inline_style_source() {
                    let mut input = ParserInput::new(source);
                    let mut parser = Parser::new(&mut input);
                    for decl in parse_declaration_block(&mut parser) {
                        per_node.push((
                            decl.value,
                            decl.important,
                            Origin::Author,
                            INLINE_SPECIFICITY,
                            INLINE_SOURCE_ORDER,
                        ));
                    }
                }
                if !per_node.is_empty() {
                    out.insert(id, per_node);
                }
            }
            // stack は LIFO なので document order で push するため reverse。
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
}

/// tag_name 文字列と selector list を突き合わせる簡易 matcher (M1.4 scope)。
///
/// Returns: matching した selector の最大 specificity。1 つも match しなければ None。
/// - `Component::LocalName(name)` — name eq_ignore_ascii_case で判定
/// - `Component::ExplicitUniversalType` — 常に match
fn match_by_tag(list: &SelectorList<RaikiriSelectorImpl>, tag_name: &str) -> Option<Specificity> {
    use selectors::parser::Component;

    let mut best: Option<Specificity> = None;
    for selector in list.slice() {
        let mut matches = true;
        for component in selector.iter_raw_match_order() {
            match component {
                Component::LocalName(local) => {
                    // local.name は Atom (raikiri-style::Atom)、tag_name 文字列と比較
                    if !tag_name.eq_ignore_ascii_case(local.name.0.as_str()) {
                        matches = false;
                        break;
                    }
                }
                Component::ExplicitUniversalType
                | Component::ExplicitAnyNamespace
                | Component::ExplicitNoNamespace
                | Component::DefaultNamespace(_) => {
                    // 常に match / namespace は m1.4 では常に true 扱い
                }
                _ => {
                    // 他 component (class/id/attr/combinator/pseudo) は m1.4 では
                    // ruletree build 段で drop 済のはずだが safety net で match fail
                    matches = false;
                    break;
                }
            }
        }
        if matches {
            let spec = specificity_of(selector);
            best = Some(match best {
                Some(prev) => prev.max(spec),
                None => spec,
            });
        }
    }
    best
}

fn specificity_of(selector: &Selector<RaikiriSelectorImpl>) -> Specificity {
    // selectors crate の Selector::specificity は 32-bit packed integer を返す。
    selector.specificity()
}

/// Top-down inheritance walk。子 node は親の computed value を必要とするため
/// (再帰の call stack で暗黙に運んでいた context)、iterative 化には各 stack
/// entry に `(StyleNodeId, 親の computed value)` を明示的に持たせる — Approach A
/// (roborev job 199 対応)。clone は各 entry ごとに発生するが m1.4 scope では
/// 許容 (hot path 化した場合は将来 `Arc<ComputedValues>` で削減を検討)。
fn resolve_inheritance<D: StyleDom>(
    dom: &D,
    id: StyleNodeId,
    parent_computed: &ComputedValues,
    cascaded: &HashMap<StyleNodeId, Vec<CascadedDecl>>,
    out: &mut Vec<ComputedValues>,
) {
    let mut stack: Vec<(StyleNodeId, ComputedValues)> = vec![(id, parent_computed.clone())];
    while let Some((id, parent_computed)) = stack.pop() {
        // raikiri-spike-37c roborev job 294 M2 finding: is_in_document()==false
        // の node は subtree ごと早期 continue する。
        //
        // 以前は resize + write + children push を unconditional に行い computed
        // 長を node_count() に揃えていた (m1.23 contract)。今 `cascade()` が
        // `dom.node_count()` で `computed` を pre-allocate + initial() で埋める
        // ように変わったため、visited しないままの slot は自然に initial()
        // として残る。これにより:
        //   - detached / template descendants は inherit_from(parent) の
        //     継承値ではなく initial() となる (`<template style="color:red">`
        //     配下は red を継承しない)
        //   - template subtree の walk が省ける (パフォーマンス改善)
        //
        // 未知 NodeId (dom.node が None) の場合も skip: initial() のままにする
        // 方が defensive (旧コードは inherit_from してから書いていた)。
        if !dom.node(id).is_some_and(|n| n.is_in_document()) {
            continue;
        }

        // 親からの inheritance walk 開始値: inherited のみコピー、非継承は
        // initial() (spec §M1.4a、raikiri-spike-m1.22)
        let mut computed = ComputedValues::inherit_from(&parent_computed);

        // 自 node の cascaded winners を apply
        if let Some(candidates) = cascaded.get(&id) {
            let winners = pick_winners(candidates);
            for value in winners.into_values() {
                apply_value(value, &mut computed);
            }
        }

        // out を id+1 サイズに resize してから index 書き込み。
        // `cascade()` の pre-allocation で通常 out.len() == node_count() のため
        // resize は no-op、defensive safety net として維持 (Dom impl の
        // node_count() 過小報告に対する保険)。
        let idx = id.0 as usize;
        if out.len() <= idx {
            out.resize(idx + 1, ComputedValues::initial());
        }
        out[idx] = computed.clone();

        // 子を stack に push (own computed value を parent_computed として渡す)。
        // stack は LIFO なので document order で push するため reverse。
        let children: Vec<_> = dom.child_ids(id).collect();
        for child_id in children.into_iter().rev() {
            stack.push((child_id, computed.clone()));
        }
    }
}

/// property key ごとに勝者 declaration を pick (specificity + !important + source order)。
fn pick_winners(candidates: &[CascadedDecl]) -> HashMap<PropertyKey, PropertyValue> {
    // best entry: (rank, spec, source_order, value)
    let mut best: HashMap<PropertyKey, (u8, Specificity, u32, PropertyValue)> = HashMap::new();

    for (value, important, origin, spec, order) in candidates {
        let rank = cascade_rank(*origin, *important);
        let key = value.key();
        let candidate = (rank, *spec, *order, value.clone());
        match best.get(&key) {
            Some(existing) => {
                if beats(&candidate, existing) {
                    best.insert(key, candidate);
                }
            }
            None => {
                best.insert(key, candidate);
            }
        }
    }

    best.into_iter().map(|(k, (_, _, _, v))| (k, v)).collect()
}

fn beats(
    candidate: &(u8, Specificity, u32, PropertyValue),
    existing: &(u8, Specificity, u32, PropertyValue),
) -> bool {
    // Tuple compare: (rank, specificity, source_order)
    // - rank 高い方が勝つ (Important UA > Important Author > Normal Author > Normal UA)
    // - 同 rank なら specificity 高い方が勝つ
    // - 同 rank + spec なら source_order 大 (=後ろ) が勝つ
    // `>=` は同一 rule 内 duplicate property の後方勝ち (spec §6.4.4) のため
    // 意図的。cross-rule では source_order が異なるので `>=` でも安全。
    (candidate.0, candidate.1, candidate.2) >= (existing.0, existing.1, existing.2)
}

fn apply_value(value: PropertyValue, target: &mut ComputedValues) {
    match value {
        PropertyValue::Color(c) => target.color = c,
        PropertyValue::FontFamily(f) => target.font_family = f,
        PropertyValue::FontSize(s) => target.font_size = s,
        PropertyValue::FontWeight(w) => target.font_weight = w,
        PropertyValue::Display(d) => target.display = d,
        // counter-* は M5 pre-work (raikiri-spike-s85) — parse 結果をそのまま
        // computed value に格納。counter tree resolve は M5 本編。
        PropertyValue::CounterReset(v) => target.counter_reset = v,
        PropertyValue::CounterIncrement(v) => target.counter_increment = v,
        PropertyValue::CounterSet(v) => target.counter_set = v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::CssColor;
    use crate::property::DisplayValue;
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

    #[test]
    fn inline_style_beats_type_selector() {
        let cv = cascade_doc("p { color: red }", "p", Some("color: blue"));
        assert_eq!(cv.color, BLUE);
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
        // 同一 rule 内で同じ property が 2 回 — CSS §6.4.4: 後方の declaration が勝つ。
        let cv = cascade_doc("p { color: red; color: blue }", "p", None);
        assert_eq!(cv.color, BLUE);
    }

    #[test]
    fn later_duplicate_in_inline_wins() {
        // inline style 内で同じ property が 2 回 — 同様に後方が勝つ。
        let cv = cascade_doc("", "p", Some("color: red; color: blue"));
        assert_eq!(cv.color, BLUE);
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

    /// roborev job 199 (medium): `collect_cascaded` and `resolve_inheritance`
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

    // ── UA origin + display cascade (M1.4a、raikiri-spike-m1.22) ──

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

        // build_rule_tree (Author 集約) + UA add_stylesheet (m1.22 経路)
        let mut tree = build_rule_tree(&doc);
        // UA CSS を先頭に inject するのではなく、既存の Author rule の後ろに
        // add してから rank 化で origin 順序を担保する (source_order より rank
        // が優位)
        // ただし M1.4a では add_stylesheet の呼び出し順で source_order が振られ
        // Author が先 (source_order 小)、UA が後 (source_order 大) となる。
        // rank 化により Origin::UserAgent の Normal は rank=0 (最弱)、
        // Origin::Author の Normal は rank=1 なので UA rule が Author を上書き
        // することはない (source_order に関わらず rank が優先)。
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
        // Normal Author > Normal UA (rank 1 > rank 0)
        let cv = cascade_with_ua("p { display: block }", "p { display: inline }", "p", None);
        assert_eq!(cv.display, DisplayValue::Inline);
    }

    #[test]
    fn important_ua_beats_important_author_display() {
        // Important UA > Important Author (rank 3 > rank 2、!important 反転)
        let cv = cascade_with_ua(
            "p { display: block !important }",
            "p { display: inline !important }",
            "p",
            None,
        );
        assert_eq!(cv.display, DisplayValue::Block);
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

    // ── counter-* wire-through (CSS Lists 3 §3、raikiri-spike-s85 M5 pre-work) ──

    #[test]
    fn counter_reset_wired_through_cascade_from_inline_style() {
        // <div style="counter-reset: chapter"> → ComputedValues.counter_reset
        // に [("chapter", 0)] が届く。parser → PropertyValue → apply_value →
        // ComputedValues の end-to-end 疎通 smoke。
        let cv = cascade_doc("", "div", Some("counter-reset: chapter"));
        assert_eq!(cv.counter_reset, vec![(SmolStr::new("chapter"), 0)]);
        // 他 counter property は non-inherited の initial (empty) のまま
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
    }

    #[test]
    fn cascade_with_ua_deterministic_across_10_runs() {
        // determinism regression (spec §M1 acceptance criteria)
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
