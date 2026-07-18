//! Unified rule tree — cascade 側と GCPM 解決側 (M5+) が共有する index。
//! M1.4 では style_rules を populate。raikiri-spike-rbo で @page at-rule も
//! 非-skip 化して [`RuleTree::page_rules`] に格納する (cascade 適用は M4 defer)。
//! それ以外の at-rule (@media / @supports / @import 等) は引き続き silently skip。

use cssparser::{Parser, ParserInput, StyleSheetParser};
use selectors::parser::{ParseRelative, SelectorList};

use crate::page::{PageRule, PageSelector, parse_page_prelude};
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
/// M1.4 では `style_rules` を populate。raikiri-spike-rbo で `page_rules` を
/// 追加 (parse のみ、cascade は M4 defer)。future field
/// (font_face_rules / counter_style_rules / media_rules /
///  supports_rules / import_rules) は M4+ で追加、`#[non_exhaustive]` の恩恵で
/// 非破壊的に拡張可能。
#[non_exhaustive]
pub struct RuleTree {
    /// Qualified style rules (`selectors { declarations }`)、source order 保持。
    pub style_rules: Vec<StyleRule>,
    /// `@page` at-rules。source_order は `style_rules` とは独立の 0-index。
    /// cascade 適用は M4 defer — 現在は parse 結果を parked しているだけ。
    pub page_rules: Vec<PageRule>,
}

impl RuleTree {
    /// 空の RuleTree (0 rule)。
    pub fn empty() -> Self {
        Self {
            style_rules: Vec::new(),
            page_rules: Vec::new(),
        }
    }

    /// Stylesheet 文字列を parse して rule を append する。
    ///
    /// - `source_order` は既存 rule 数を起点に呼び出し順で自動採番
    ///   (`style_rules` / `page_rules` は別カウンタ — [`PageRule::source_order`]
    ///   の doc 参照)
    /// - `origin` は style rule と `@page` rule の両方に伝播する。cascade
    ///   rank 化 (`!important` 反転扱い) は M4 で wire — M4 pre-work
    ///   (raikiri-spike-jzv) で `PageRule` にも origin を保持することで
    ///   cascade 側が re-index せずに済むようになった。CSS Cascading L4
    ///   §"cascade-origin" (<https://www.w3.org/TR/css-cascade-4/#cascade-origin>)
    /// - Invalid selector / 未サポート property は既存の silent-drop 挙動を
    ///   継承 (spec §M1.4a)
    ///
    /// spec: raikiri-spike-m1.22 (m1.21 spec addition の実装)、
    /// raikiri-spike-rbo (@page scaffolding)
    pub fn add_stylesheet(&mut self, source: &str, origin: Origin) {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = StyleRuleParser;
        let mut style_order = self.style_rules.len() as u32;
        let mut page_order = self.page_rules.len() as u32;
        for rule in StyleSheetParser::new(&mut parser, &mut rule_parser).flatten() {
            match rule {
                ParsedRule::Style(selectors, declarations) => {
                    if !is_type_or_universal_only(&selectors) {
                        continue; // class/id/attr/combinator selector は m1.4 では drop
                    }
                    self.style_rules.push(StyleRule {
                        selectors,
                        declarations,
                        source_order: style_order,
                        origin,
                    });
                    style_order = style_order.wrapping_add(1);
                }
                ParsedRule::Page(selector, declarations) => {
                    self.page_rules.push(PageRule {
                        selector,
                        declarations,
                        source_order: page_order,
                        origin,
                    });
                    page_order = page_order.wrapping_add(1);
                }
            }
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

/// Top-level parsed rule shape emitted by [`StyleRuleParser`].
///
/// cssparser requires `AtRuleParser::AtRule` と `QualifiedRuleParser::QualifiedRule`
/// を同一型にする必要があるため、両方をこの enum に流し込む
/// (`StyleSheetParser::next` の `Item = R` 制約)。@media / @supports / @import
/// は default `parse_prelude` の `Err` に落ちて cssparser 側で silent drop。
enum ParsedRule {
    Style(SelectorList<RaikiriSelectorImpl>, Vec<Declaration>),
    Page(PageSelector, Vec<Declaration>),
}

/// StyleSheetParser 実装。qualified rule + `@page` を受理、他 at-rule は drop。
struct StyleRuleParser;

/// `@page` prelude を parse する。他 at-rule (@media / @supports / @import 等) は
/// default `Err` に落として cssparser に silent drop させる。
impl<'i> cssparser::AtRuleParser<'i> for StyleRuleParser {
    type Prelude = PageSelector;
    type AtRule = ParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("page") {
            parse_page_prelude(input)
        } else {
            // @media / @supports / @import 等は今 slot 未サポート — cssparser 側で
            // block をまるごと skip させるため Err を返す。
            Err(input.new_custom_error(()))
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, Self::Error>> {
        // @page body = declaration list (M4 で margin-box at-rule 追加予定)。
        // 未サポート property は既存の silent-drop で 0 declaration 化する。
        let declarations = parse_declaration_block(input);
        Ok(ParsedRule::Page(prelude, declarations))
    }
}

impl<'i> cssparser::QualifiedRuleParser<'i> for StyleRuleParser {
    type Prelude = SelectorList<RaikiriSelectorImpl>;
    type QualifiedRule = ParsedRule;
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
        Ok(ParsedRule::Style(prelude, declarations))
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

    // ── @page at-rule scaffolding (raikiri-spike-rbo) ──
    //
    // Spec: CSS Paged Media Level 3, "Page selectors syntax"
    // <https://www.w3.org/TR/css-page-3/#page-selectors-syntax>
    //
    // Test で使う body は M1.4 の property.rs でサポート済み (color / font-*) を
    // 選ぶ — parse_declaration_block を reuse しているので margin / size 等は
    // 現時点で silent drop され declaration 0 個になる (下の
    // page_body_unsupported_property_drops_declaration がその regression guard)。

    use crate::page::PageSelector;
    use crate::{Atom, PageRule};

    fn page_rules(source: &str) -> Vec<PageRule> {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        tree.page_rules
    }

    #[test]
    fn page_default_selector_no_prelude() {
        // `@page { color: red }` → PageSelector::Default、declarations 1 個。
        // origin は `page_rules(...)` helper が `Origin::Author` を hardcode
        // しているため Author が期待値 (raikiri-spike-jzv、M4 pre-work)。
        let rules = page_rules("@page { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, PageSelector::Default);
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].source_order, 0);
        assert_eq!(rules[0].origin, Origin::Author);
    }

    #[test]
    fn page_pseudo_first() {
        let rules = page_rules("@page :first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, PageSelector::First);
    }

    #[test]
    fn page_pseudo_left_right_blank() {
        let rules = page_rules(
            "@page :left { color: red } \
             @page :right { color: red } \
             @page :blank { color: red }",
        );
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, PageSelector::Left);
        assert_eq!(rules[1].selector, PageSelector::Right);
        assert_eq!(rules[2].selector, PageSelector::Blank);
        // page_order は @page 独立の counter。
        assert_eq!(rules[0].source_order, 0);
        assert_eq!(rules[1].source_order, 1);
        assert_eq!(rules[2].source_order, 2);
    }

    #[test]
    fn page_named_selector() {
        let rules = page_rules("@page my-cover { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            PageSelector::Named(Atom::from("my-cover"))
        );
    }

    #[test]
    fn page_multi_pseudo_is_dropped() {
        // Scaffolding narrowing (see page.rs module docs): the L3 grammar (spec anchor `#page-selectors-syntax`) では
        // `<pseudo-page>*` で複数許可だが、raikiri-spike-rbo では単数のみ受理。
        // `@page :first :left` は入 rule ごと drop。
        let rules = page_rules("@page :first :left { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_functional_pseudo_is_dropped() {
        // Spec-outside functional pseudo (e.g. `:nth-page(...)`) は CSS Paged
        // Media L3 (anchor `#page-selectors-syntax`) も L4 Editor's Draft も
        // 定義していないため、raikiri-style としては未知 pseudo として rule ごと
        // drop する。もし raikiri-local な拡張として実装する日が来れば、そのときは
        // 明示的に variant を追加し (現在のこの guard test を反転) スコープを
        // 人間 ledger で決める。
        let rules = page_rules("@page :nth-page(2n+1) { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_ident_plus_pseudo_is_dropped() {
        // 同じく scaffolding narrowing。spec は `<ident>? <pseudo-page>*` で
        // `named:first` を許可するが、rbo では drop。
        let rules = page_rules("@page named:first { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_selector_list_with_comma_is_dropped() {
        // `page-selector-list = <page-selector>#` の comma-list も scaffolding
        // では未対応で drop。
        let rules = page_rules("@page :first, :left { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_unknown_pseudo_is_dropped() {
        // `:cover` は the L3 grammar (spec anchor `#page-selectors-syntax`) に存在しないため drop。
        let rules = page_rules("@page :cover { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_pseudo_is_case_insensitive() {
        // CSS keyword は ASCII case-insensitive (`match_ignore_ascii_case!` 経由)。
        let rules = page_rules("@page :FIRST { color: red } @page :Left { color: red }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, PageSelector::First);
        assert_eq!(rules[1].selector, PageSelector::Left);
    }

    #[test]
    fn page_margin_box_at_rule_body_is_skipped_declaration_survives() {
        // reviewer-spec §8.2 Finding 4 regression guard: parse_declaration_block
        // reuse は margin-box at-rules (`@top-left { … }` per L3 §5) を DeclParser
        // の default AtRuleParser::parse_prelude が Err で返して cssparser の
        // error-recovery で block ごと silent skip する。その前後の通常宣言は
        // 生き残ることを pin する。M4 で margin-box を wire するときは PageDeclParser
        // に本物の AtRuleParser を実装する予定。
        let rules = page_rules("@page :first { @top-left { content: 'x' } color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 1);
    }

    #[test]
    fn page_source_order_independent_from_style_rules() {
        // page_rules の source_order は style_rules と独立の counter。
        // 全 rule が同一 add_stylesheet call の origin (Author) を継承する
        // ことも同時に pin する (raikiri-spike-jzv M4 pre-work: `PageRule.origin`)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "p { color: red } \
             @page :first { color: red } \
             div { color: blue } \
             @page :left { color: red }",
            Origin::Author,
        );
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
        assert_eq!(tree.page_rules.len(), 2);
        assert_eq!(tree.page_rules[0].source_order, 0);
        assert_eq!(tree.page_rules[0].origin, Origin::Author);
        assert_eq!(tree.page_rules[1].source_order, 1);
        assert_eq!(tree.page_rules[1].origin, Origin::Author);
    }

    #[test]
    fn page_source_order_monotonic_across_add_stylesheet_calls() {
        // 複数 add_stylesheet 呼び出し間で page_order は継続する。
        // 各 rule の origin は当該 add_stylesheet call の引数に一致することを
        // pin する (raikiri-spike-jzv M4 pre-work: `PageRule.origin` は
        // per-call の origin を保持し、cascade 側 M4 code が re-index せず
        // per-origin cascade を組めるようにする — CSS Cascading L4
        // <https://www.w3.org/TR/css-cascade-4/#cascade-origin>)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page :first { color: red }", Origin::UserAgent);
        tree.add_stylesheet("@page :left { color: blue }", Origin::Author);
        assert_eq!(tree.page_rules.len(), 2);
        assert_eq!(tree.page_rules[0].source_order, 0);
        assert_eq!(tree.page_rules[0].origin, Origin::UserAgent);
        assert_eq!(tree.page_rules[1].source_order, 1);
        assert_eq!(tree.page_rules[1].origin, Origin::Author);
    }

    #[test]
    fn page_body_unsupported_property_drops_declaration() {
        // M1.4 property.rs は margin / size 等 @page descriptor を未サポート。
        // parse_declaration_block reuse により silent drop され declaration 0 個。
        // M4 で @page descriptor が入るまで cascade 側は空 declarations を扱える
        // ことを保証する regression guard。
        let rules = page_rules("@page { margin: 1cm; size: A4 }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, PageSelector::Default);
        assert!(rules[0].declarations.is_empty());
    }

    #[test]
    fn other_at_rules_still_silently_dropped() {
        // @media / @supports / @import は default `Err` に落ちて silent drop。
        // (raikiri-spike-rbo scope 外 — @page のみ非-skip 化)
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@media print { p { color: red } } \
             @supports (display: block) { p { color: red } } \
             p { color: red }",
            Origin::Author,
        );
        assert_eq!(tree.page_rules.len(), 0);
        // @media / @supports 内の p { color: red } は body parse されず drop、
        // 末尾の p { color: red } のみ残る。
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn page_rules_captured_via_build_rule_tree_from_dom() {
        // build_rule_tree (DOM 経由) でも page_rules が populate される。
        let doc = dom_with_style("@page :first { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.page_rules.len(), 1);
        assert_eq!(tree.page_rules[0].selector, PageSelector::First);
        assert_eq!(tree.style_rules.len(), 1);
    }
}
