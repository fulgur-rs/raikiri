//! Unified rule tree — cascade 側と GCPM 解決側 (M5+) が共有する index。
//! M1.4 では style_rules のみ populate。at-rule (@page / @media / @import 等) は
//! silently skip。

use cssparser::{Parser, ParserInput, StyleSheetParser};
use selectors::parser::{ParseRelative, SelectorList};

use raikiri_traits::{Dom, Element, Node, NodeKind};

use crate::rule::{parse_declaration_block, Declaration, StyleRule};
use crate::{RaikiriSelectorImpl, RaikiriSelectorParser};

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
        Self { style_rules: Vec::new() }
    }
}

/// DOM を DFS walk して全 `<style>` element の text を parse、単一 RuleTree に集約。
///
/// - `<style>` element の子 Text node を concat して stylesheet 文字列を構成
/// - cssparser の StyleSheetParser で top-level rule list として parse
/// - 各 qualified rule について:
///   - prelude を SelectorList として parse — 失敗 or type/universal 外を含む rule は
///     silently drop (spec 準拠)
///   - block を declaration-list として parse (Task 4 の parse_declaration_block を使用)
///   - StyleRule を produce、source_order は通し番号
/// - at-rule は cssparser 側で skip
pub fn build_rule_tree<D: Dom>(dom: &D) -> RuleTree {
    let mut rules: Vec<StyleRule> = Vec::new();
    let mut source_order: u32 = 0;

    walk_and_collect(dom, dom.root_id(), &mut |source| {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = StyleRuleParser;
        for (selectors, declarations) in
            StyleSheetParser::new(&mut parser, &mut rule_parser).flatten()
        {
            if !is_type_or_universal_only(&selectors) {
                continue; // class/id/attr/combinator selector は m1.4 では drop
            }
            rules.push(StyleRule { selectors, declarations, source_order });
            source_order = source_order.wrapping_add(1);
        }
    });

    RuleTree { style_rules: rules }
}

fn walk_and_collect<D: Dom, F: FnMut(&str)>(
    dom: &D,
    id: raikiri_traits::NodeId,
    on_style_text: &mut F,
) {
    if let Some(node) = dom.node(id) {
        if node.kind() == NodeKind::Element
            && let Some(elem) = node.as_element()
            && elem.tag_name().eq_ignore_ascii_case("style")
        {
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
        // 全 kind で children を再帰
        for child_id in dom.child_ids(id) {
            walk_and_collect(dom, child_id, on_style_text);
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
}
