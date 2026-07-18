//! Unified rule tree — cascade 側と GCPM 解決側 (M5+) が共有する index。
//! M1.4 では style_rules のみ populate。at-rule (@page / @media / @import 等) は
//! silently skip。

use cssparser::{Parser, ParserInput, StyleSheetParser};
use selectors::parser::{ParseRelative, SelectorList};

use crate::rule::{Declaration, StyleRule, parse_declaration_block};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};
use crate::{RaikiriSelectorImpl, RaikiriSelectorParser};

/// Cascade origin (CSS Cascading L4 §6.2)。M1 では UserAgent + Author の
/// 2 段のみ。User origin は Consumer が `extra_stylesheets` 経由で Author
/// として渡す想定 (spec §M1.4a Non-goals、raikiri-spike-m1.22)。
///
/// `StylesheetKind` (dom-level tag) との対応は raikiri umbrella crate が
/// cascade orchestration の一部として map する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    UserAgent,
    Author,
}

/// Unified rule tree。cascade + GCPM (M5+) が消費する index。
///
/// M1.4 では `style_rules` のみ populate。future field
/// (page_rules / font_face_rules / counter_style_rules / media_rules /
///  supports_rules / import_rules) は M4 で追加、`#[non_exhaustive]` の恩恵で
/// 非破壊的に拡張可能。
#[non_exhaustive]
pub struct RuleTree {
    /// Qualified style rules (`selectors { declarations }`)、source order 保持。
    pub style_rules: Vec<StyleRule>,
}

impl RuleTree {
    /// 空の RuleTree (0 rule)。
    pub fn empty() -> Self {
        Self {
            style_rules: Vec::new(),
        }
    }

    /// Stylesheet 文字列を parse して rule を append する。
    ///
    /// - `source_order` は既存 rule 数を起点に呼び出し順で自動採番
    /// - `origin` は各 rule に紐付き、cascade rank 化 (`!important` 反転
    ///   扱い) で使用される
    /// - Invalid selector / 未サポート property は既存の silent-drop 挙動を
    ///   継承 (spec §M1.4a)
    ///
    /// spec: raikiri-spike-m1.22 (m1.21 spec addition の実装)
    pub fn add_stylesheet(&mut self, source: &str, origin: Origin) {
        let start_order = self.style_rules.len() as u32;
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = StyleRuleParser;
        let mut order = start_order;
        for (selectors, declarations) in
            StyleSheetParser::new(&mut parser, &mut rule_parser).flatten()
        {
            if !is_type_or_universal_only(&selectors) {
                continue; // class/id/attr/combinator selector は m1.4 では drop
            }
            self.style_rules.push(StyleRule {
                selectors,
                declarations,
                source_order: order,
                origin,
            });
            order = order.wrapping_add(1);
        }
    }
}

/// DOM を DFS walk して全 `<style>` element の text を Author stylesheet として
/// 集約する convenience。UA CSS は含めない (`raikiri-html::parse` が
/// Document.add_stylesheet 経由で inject 済み、`Document.stylesheets()` を
/// raikiri umbrella が RuleTree に流し込む責務)。
///
/// 詳細は spec §M1.4a (raikiri-spike-m1.22)。
pub fn build_rule_tree<D: StyleDom>(dom: &D) -> RuleTree {
    let mut tree = RuleTree::empty();
    walk_and_collect(dom, dom.root_id(), &mut |source| {
        tree.add_stylesheet(source, Origin::Author);
    });
    tree
}

/// DOM を root から DFS walk して全 `<style>` element の text を callback に渡す。
///
/// UA CSS は含まれない — `raikiri-html::parse` が `Document::add_stylesheet` 経由で
/// UA を注入しており、`Document::stylesheets()` 経路で raikiri umbrella が別途消費
/// する契約 (spec §M1.4a、raikiri-spike-m1.23)。本 walker は DOM `<style>` element
/// の text 収集のみを担当する。
///
/// 呼び出し順は `walk_and_collect` の iterative DFS に従い document order。
/// stack overflow 保護は `walk_and_collect` と共有 (roborev job 199)。
pub fn walk_style_elements<D: StyleDom, F: FnMut(&str)>(dom: &D, mut on_style_text: F) {
    walk_and_collect(dom, dom.root_id(), &mut on_style_text);
}

/// DOM walk 本体。深いネストで stack overflow しないよう explicit `Vec` stack
/// で iterative DFS (roborev job 199 対応)。訪問順は sibling 間で recursion 版
/// と異なり得るが、`<style>` は独立に text を emit するだけで他 node の状態に
/// 依存しないため source_order (呼び出し側で採番) は不変。
fn walk_and_collect<D: StyleDom, F: FnMut(&str)>(dom: &D, id: StyleNodeId, on_style_text: &mut F) {
    let mut stack: Vec<StyleNodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // raikiri-spike-37c: <template> 子孫 + 将来の inert subtree を統一 skip。
            // 実 Document (sink 経由 populate 済) では is_in_document() bit が
            // primary skip 経路。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let tag = elem.tag_name();
                // NOTE: raikiri-spike-37c contract — 通常経路 (sink 経由 populate
                // 済 Document) では上の is_in_document() gate で subsumed。本 arm
                // は TestDoc 等の default true な Node trait 実装からの呼び出しで
                // template 内 <style> が cascade に流れ込むのを防ぐ safety net。
                // 実本番経路の "1 か所集約" contract は sink 側の判定を primary
                // とし、この safety net は 2nd-line defense として明示的に維持する。
                //
                // roborev job 292 L2 finding: namespace check を追加し HTML
                // `<template>` のみを対象とする (SVG element `<template>` は spec
                // 定義が無いが raw parser で local="template" になり得る)。sink 側
                // の判定 (`namespace.is_none()`) と一貫。
                if tag.eq_ignore_ascii_case("template") && elem.namespace_uri().is_none() {
                    continue;
                }
                if tag.eq_ignore_ascii_case("style") {
                    // 子 Text node を concat
                    let mut concat = String::new();
                    for child_id in dom.child_ids(id) {
                        if let Some(child) = dom.node(child_id)
                            && let Some(t) = child.text_content()
                        {
                            concat.push_str(t);
                        }
                    }
                    if !concat.is_empty() {
                        on_style_text(&concat);
                    }
                }
            }
            // 全 kind で children を stack に push。stack は LIFO なので document
            // order (source_order 割り当てに影響) を保つため reverse push。
            let children: Vec<_> = dom.child_ids(id).collect();
            for child_id in children.into_iter().rev() {
                stack.push(child_id);
            }
        }
    }
}

/// StyleSheetParser 実装。qualified rule のみ受理、at-rule は default drop。
struct StyleRuleParser;

impl<'i> cssparser::AtRuleParser<'i> for StyleRuleParser {
    type Prelude = ();
    type AtRule = (SelectorList<RaikiriSelectorImpl>, Vec<Declaration>);
    type Error = ();
}

impl<'i> cssparser::QualifiedRuleParser<'i> for StyleRuleParser {
    type Prelude = SelectorList<RaikiriSelectorImpl>;
    type QualifiedRule = (SelectorList<RaikiriSelectorImpl>, Vec<Declaration>);
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        SelectorList::parse(&RaikiriSelectorParser, input, ParseRelative::No)
            .map_err(|_| input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let declarations = parse_declaration_block(input);
        Ok((prelude, declarations))
    }
}

/// SelectorList 内全 selector が type or universal のみで構成されているか判定。
/// class/id/attribute/combinator/pseudo-class を 1 つでも含めば false → drop。
fn is_type_or_universal_only(list: &SelectorList<RaikiriSelectorImpl>) -> bool {
    use selectors::parser::Component;

    for selector in list.slice() {
        for component in selector.iter_raw_match_order() {
            match component {
                Component::LocalName(_)
                | Component::ExplicitUniversalType
                | Component::ExplicitAnyNamespace
                | Component::ExplicitNoNamespace
                | Component::DefaultNamespace(_) => {}
                _ => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dom::TestDoc;

    fn dom_with_style(css: &str) -> TestDoc {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, css);
        doc
    }

    #[test]
    fn empty_dom_returns_empty_ruletree() {
        let doc = TestDoc::new();
        assert!(build_rule_tree(&doc).style_rules.is_empty());
    }

    #[test]
    fn dom_without_style_returns_empty_ruletree() {
        let mut doc = TestDoc::new();
        doc.push_element(0, "p", None);
        assert!(build_rule_tree(&doc).style_rules.is_empty());
    }

    #[test]
    fn single_style_type_selector_captured() {
        let doc = dom_with_style("p { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[0].declarations.len(), 1);
    }

    #[test]
    fn class_selector_silently_dropped() {
        let doc = dom_with_style(".foo { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        // .foo が drop、p が残る (source_order は 0 のまま = drop された rule は order を消費しない)
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn universal_selector_captured() {
        let doc = dom_with_style("* { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn multiple_style_elements_source_order() {
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(s1, "p { color: red }");
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(s2, "div { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn invalid_rule_silently_dropped() {
        // @nope; は at-rule として parse され drop、bogus_selector.. rule は selector parse fail で drop
        let doc = dom_with_style("@nope; p { color: red } ;;garbage;;");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn important_flag_captured() {
        let doc = dom_with_style("p { color: red !important }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert!(tree.style_rules[0].declarations[0].important);
    }

    /// roborev job 199 (medium): `walk_and_collect` was recursive DFS —
    /// a deeply nested DOM (e.g. approaching `max_dom_nodes = 1M`) could
    /// stack-overflow the process. 5000-level linear chain with `<style>`
    /// at the deepest level (forcing the walk all the way down before
    /// finding rule text) must complete without overflow and still find
    /// the rule.
    #[test]
    fn deep_nesting_5000_build_rule_tree_no_overflow() {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        for _ in 0..5000 {
            parent = doc.push_element(parent, "div", None);
        }
        let style = doc.push_element(parent, "style", None);
        doc.push_text(style, "div { color: red }");

        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].declarations.len(), 1);
    }

    // ── Origin + add_stylesheet (M1.4a、raikiri-spike-m1.22) ──

    #[test]
    fn origin_is_copy_eq() {
        fn assert_copy<T: Copy + PartialEq + Eq>() {}
        assert_copy::<Origin>();
        assert_ne!(Origin::UserAgent, Origin::Author);
    }

    #[test]
    fn add_stylesheet_ua_and_author_populate_rule_tree() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p { color: red }", Origin::UserAgent);
        tree.add_stylesheet("p { color: blue }", Origin::Author);

        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].origin, Origin::UserAgent);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].origin, Origin::Author);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn add_stylesheet_source_order_monotonic_across_calls() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p { color: red }", Origin::UserAgent);
        tree.add_stylesheet("div { color: green }", Origin::UserAgent);
        tree.add_stylesheet("span { color: blue }", Origin::Author);
        let orders: Vec<u32> = tree.style_rules.iter().map(|r| r.source_order).collect();
        assert_eq!(orders, vec![0, 1, 2]);
    }

    #[test]
    fn add_stylesheet_dropped_selectors_do_not_consume_source_order() {
        // .foo (class selector) は M1.4 では drop、p は残る
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(".foo { color: red } p { color: blue }", Origin::Author);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    #[test]
    fn build_rule_tree_produces_author_origin_for_dom_style_elements() {
        let doc = dom_with_style("p { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].origin, Origin::Author);
    }

    #[test]
    fn walk_style_elements_pub_visits_all_style_texts_in_document_order() {
        // 兄弟の <style> 2 個 → 呼び出し順で collected される。
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(s1, "p { color: red }");
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(s2, "div { color: blue }");

        let mut collected: Vec<String> = Vec::new();
        super::walk_style_elements(&doc, |css| collected.push(css.to_string()));

        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], "p { color: red }");
        assert_eq!(collected[1], "div { color: blue }");
    }

    #[test]
    fn style_inside_template_is_skipped_per_html_spec_inertness() {
        // <template> は spec 上 inert (HTML spec)。内部の <style> は cascade に流れない。
        // raikiri-html/src/sink.rs:315-318 の invariant と consistent。
        let mut doc = TestDoc::new();
        let template = doc.push_element(0, "template", None);
        let style_in_template = doc.push_element(template, "style", None);
        doc.push_text(style_in_template, "p { color: red }");

        // <template> 外の <style> は拾う必要がある (baseline)。
        let style_outer = doc.push_element(0, "style", None);
        doc.push_text(style_outer, "div { color: blue }");

        let tree = build_rule_tree(&doc);
        // <style> outer の 1 rule のみ (div{...})、template 内は skip。
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }
}
