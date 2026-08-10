//! Unified rule tree — cascade 側と GCPM 解決側 (M5+) が共有する index。
//! M1.4 では style_rules を populate。raikiri-spike-rbo で @page at-rule も
//! 非-skip 化して [`RuleTree::page_rules`] に格納する (cascade 適用は M4 defer)。
//! それ以外の at-rule (@media / @supports / @import 等) は引き続き silently skip。

use cssparser::{Parser, ParserInput, StyleSheetParser};
use selectors::parser::{ParseRelative, SelectorList};

use crate::counter_style::{CounterStyleRegistry, parse_counter_style_rules};
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
/// 追加 (parse のみ、cascade は M4 defer)。bd raikiri-spike-gce8 で
/// `counter_styles` ([`CounterStyleRegistry`]) を追加 — `@counter-style`
/// at-rule の registry 化のみで、`generate a counter` 算出
/// ([`crate::counter_style::resolve_custom_counter`]) の呼び出しは consumer
/// 側の責務のまま (bd raikiri-spike-cvxe が raikiri-traits 側の配線を担当)。
/// future field (font_face_rules / media_rules / supports_rules /
/// import_rules) は M4+ で追加、`#[non_exhaustive]` の恩恵で非破壊的に
/// 拡張可能。
#[non_exhaustive]
pub struct RuleTree {
    /// Qualified style rules (`selectors { declarations }`)、source order 保持。
    pub(crate) style_rules: Vec<StyleRule>,
    /// `@page` at-rules。source_order は `style_rules` とは独立の 0-index。
    /// cascade は [`crate::page::cascade_page`] が適用する; per-page `PageBox`
    /// derivation と margin-box slot layout は M4 defer。
    ///
    /// `style_rules` と違い `pub` のまま — 意図的で、bd raikiri-spike-qzn3 の
    /// approved scope 外である。非対称の帰結の canonical な記述 (docs.rs から
    /// 到達可能) は [`crate::page::PageRule::declarations`] の doc の
    /// 「Why this field ... is still `pub`」節にある (bd raikiri-spike-ykee)。
    /// 旧 pointer 先だった [`crate::rule::expand_shorthand_into`] は
    /// `pub(crate)` で docs.rs に出ないため dead end だった。
    pub page_rules: Vec<PageRule>,
    /// `@counter-style` at-rule の name → rule registry (bd raikiri-spike-gce8、
    /// origin-aware 化は bd raikiri-spike-f7vg)。
    ///
    /// [`RuleTree::add_stylesheet`] が呼ばれるたび (origin を問わず)、同じ
    /// 文字列に対して [`crate::counter_style::parse_counter_style_rules`] を
    /// 独立にもう一度走らせ、得られた各 [`crate::counter_style::CounterStyleRule`]
    /// を呼び出し時の `origin` と一緒に
    /// [`CounterStyleRegistry::insert_with_origin`] へ渡す。同名 rule 間の
    /// 勝敗は `CounterStyleRegistry` 自体が origin ごとに追跡して解決する
    /// ([`CounterStyleRegistry`] 型 doc の解決表参照) — CSS Counter Styles L3
    /// §3 の "standard cascade rules" (origin が第一基準、同一 origin 内は
    /// source order) をこの 2-origin モデルの下でそのまま実装しており、
    /// 呼び出し側 (`add_stylesheet`) は origin でフィルタする必要がない。
    /// 詳細は [`RuleTree::add_stylesheet`] doc 参照。
    ///
    /// `style_rules` 用の parser とは意図的に別 pass ([`crate::counter_style`] module doc
    /// の "What's implemented" 節が元々の設計意図として明記) — このフィールドを
    /// 追加した bd raikiri-spike-gce8 は「同じ source 文字列を追加でもう一度
    /// `counter_style` 側の entry point に渡す」配線のみを担い、2 つの parser を
    /// 1 pass に融合する話ではない。
    pub(crate) counter_styles: CounterStyleRegistry,
}

impl RuleTree {
    /// Qualified style rules への read-only accessor (source order 順)。
    ///
    /// **qualified style rules に関しては**書き込み経路が
    /// [`RuleTree::add_stylesheet`] のみになった (bd raikiri-spike-qzn3)。
    /// `page_rules` 側は approved scope 外で `pub` のまま — 同 field の doc 参照。
    ///
    /// # `style_rules` field 自体への到達不能性 (bd raikiri-spike-ejia)
    ///
    /// `style_rules` field は `pub(crate)` — external crate から届くのは
    /// この accessor だけである。`RuleTree` は `#[non_exhaustive]` かつ
    /// `Clone` を derive していないので、struct literal / functional-update
    /// による構築も、所有値としての複製も external crate からはできない。
    /// 以下は field 名そのものが private であることの compile-fail pin —
    /// `pub` に戻れば compile が通るようになる:
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.style_rules;
    /// ```
    pub fn style_rules(&self) -> &[StyleRule] {
        &self.style_rules
    }

    /// `@counter-style` registry への read-only accessor (bd raikiri-spike-gce8)。
    ///
    /// [`RuleTree::add_stylesheet`] が呼ばれるたびに (origin を問わず) populate
    /// される — 同名 rule 間の origin 優先順位の解決は `CounterStyleRegistry`
    /// 自体が担う (field doc 参照)。空の `RuleTree` ([`RuleTree::empty`]) では
    /// [`CounterStyleRegistry::is_empty`] が `true`。`generate a counter` の実行
    /// (`counter()`/`counters()` の値 resolve) はこの registry を読む consumer 側の
    /// 責務 — [`crate::counter_style::resolve_custom_counter`] にこの registry から
    /// [`CounterStyleRegistry::get`] した [`crate::counter_style::CounterStyleRule`]
    /// を渡す配線は raikiri-traits 側 (bd raikiri-spike-cvxe) が担う。
    ///
    /// # `counter_styles` field 自体への到達不能性
    ///
    /// `style_rules`/[`RuleTree::style_rules`] (bd raikiri-spike-ejia) と同じ
    /// pin — `counter_styles` field は `pub(crate)` で、external crate から
    /// 届くのはこの accessor だけである。以下は field 名そのものが private で
    /// あることの compile-fail pin — `pub` に戻れば compile が通るようになる:
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.counter_styles;
    /// ```
    pub fn counter_styles(&self) -> &CounterStyleRegistry {
        &self.counter_styles
    }

    /// 空の RuleTree (0 rule)。
    pub fn empty() -> Self {
        Self {
            style_rules: Vec::new(),
            page_rules: Vec::new(),
            counter_styles: CounterStyleRegistry::new(),
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
    /// - `@counter-style` at-rule は `origin` を問わず、
    ///   [`crate::counter_style::parse_counter_style_rules`] が同じ `source` に対して
    ///   独立にもう一度 parse し、得られた各 rule を呼び出し時の `origin` と共に
    ///   [`CounterStyleRegistry::insert_with_origin`] へ渡す (bd raikiri-spike-gce8、
    ///   origin-aware 化は bd raikiri-spike-f7vg)。
    ///
    ///   CSS Counter Styles L3 §3 は同名 `@counter-style` の勝者決定を "according
    ///   to standard cascade rules" (origin が第一基準、UA は常に他 origin に負ける)
    ///   と規定する。この解決自体は [`CounterStyleRegistry`] が origin ごとに
    ///   entry を追跡して実装している (型 doc の解決表参照) — 呼び出し側の
    ///   `add_stylesheet` は origin でフィルタする必要がなく、単に origin を
    ///   そのまま伝播するだけでよい。結果として、他 origin との同名衝突がない
    ///   単独の `Origin::UserAgent` `@counter-style` も (bd raikiri-spike-gce8 時点
    ///   では Author-only gate により無条件 drop されていたが) `counter_styles`
    ///   に反映されるようになった (bd raikiri-spike-f7vg)。
    ///
    /// spec: raikiri-spike-m1.22 (m1.21 spec addition の実装)、
    /// raikiri-spike-rbo (@page scaffolding)、raikiri-spike-gce8 (counter_styles wiring)、
    /// raikiri-spike-f7vg (origin-aware 化、standalone UA-origin 定義の反映)
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
        for rule in parse_counter_style_rules(source) {
            self.counter_styles.insert_with_origin(rule, origin);
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
/// で iterative DFS (roborev job 199 対応)。children を reverse push してから
/// LIFO で pop するため (下記 stack push 箇所参照)、sibling 間の訪問順は素朴な
/// recursion 版と一致する — document order の保持は単なる互換目的ではなく、
/// [`RuleTree::add_stylesheet`] が呼び出し順で `source_order` を単調採番する
/// (`build_rule_tree` がこの walk の callback から呼ぶ) ため正しさ上の要請。
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
            // order を保つため reverse push。`child_ids` イテレータを直接
            // `stack` へ `extend` し、今回追加した末尾スライスだけを in-place
            // `reverse()` する — 都度捨てる中間 `Vec` を経由しない (bd
            // raikiri-spike-o53w、cascade.rs 側の bd raikiri-spike-75ch と同型
            // の技法)。`stack` 自体の capacity growth は元の
            // `for .. { stack.push(..) }` と同じ amortized pattern のままで、
            // ここで削れるのは「今回だけの捨て Vec」1 本分のみ。
            //
            // なぜここでは document order 保持が正しさ上の要請か: 本関数冒頭
            // のコメントの通り `RuleTree::add_stylesheet` は呼び出し順で
            // source_order を単調採番する。`build_rule_tree` はこの walk の
            // callback から `<style>` element 訪問順にそれを呼ぶため、訪問順
            // がそのまま source_order — ひいては cascade tie-break — に反映
            // される。cascade.rs の同型 2 箇所 (collect_cascaded /
            // resolve_inheritance) は訪問順に依存しない挙動保持のみが目的
            // だったのと対照的。
            // `walk_style_elements_pub_visits_all_style_texts_in_document_order`
            // test が兄弟 `<style>` 2 個 (同一 depth) の text を visit 順で固定
            // している。ただし同一 depth の兄弟だけでは DFS/BFS を判別できない
            // (同深度なら両戦略の visit 順が一致してしまう) — mixed depth の
            // regression net は
            // `walk_and_collect_preserves_document_order_across_mixed_sibling_descendant_depths`
            // test (bd raikiri-spike-spju) が別途固定している。
            let start = stack.len();
            stack.extend(dom.child_ids(id));
            stack[start..].reverse();
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

    /// bd raikiri-spike-spju: regression net for document-order preservation
    /// across MIXED sibling/descendant depths — the existing sibling-only
    /// test above (2 flat `<style>` at the same depth) cannot distinguish a
    /// depth-first walk from a naive breadth-first one, because same-depth
    /// visit order happens to coincide for both strategies. This test uses
    /// a shape where the doc-order-earlier `<style>` sits *deeper* than the
    /// doc-order-later one:
    ///
    /// ```text
    /// root
    /// ├── section          (depth 1)
    /// │     └── mid        (depth 2)
    /// │           └── style A   (depth 3, text "p { color: red }")
    /// └── aside             (depth 1, later sibling of `section`)
    ///       └── style B    (depth 2, text "div { background-color: blue }")
    /// ```
    ///
    /// (A and B intentionally use different properties — `color` vs.
    /// `background-color` — so the `build_rule_tree` half below can pin
    /// *which* rule landed at which `source_order`, not just that 2 rules
    /// exist.)
    ///
    /// Correct document order is A then B (pre-order: all of `section`'s
    /// subtree, including the depth-3 A, precedes `aside`'s subtree)
    /// regardless of A being deeper than B. A level-order (BFS) walk would
    /// instead visit the shallower B (depth 2) before the deeper A
    /// (depth 3), flipping the order — this is exactly the class of bug
    /// `walk_and_collect`'s doc comment warns is a correctness requirement,
    /// not just a behavior-compat nicety, because `source_order` feeds the
    /// cascade order-of-appearance tie-break (CSS Cascading L4
    /// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>).
    ///
    /// Empirically confirmed (bd raikiri-spike-spju filing): this test is
    /// the only one of the 57 tests in this module that goes red when
    /// `walk_and_collect` is mutated from `Vec`/LIFO-pop DFS to
    /// `VecDeque`/`pop_front` BFS — the pre-existing
    /// `walk_style_elements_pub_visits_all_style_texts_in_document_order`
    /// test (2 flat siblings, same depth) stays green under that mutation
    /// because same-depth visit order happens to coincide for DFS and BFS.
    #[test]
    fn walk_and_collect_preserves_document_order_across_mixed_sibling_descendant_depths() {
        use crate::property::PropertyValue;

        let mut doc = TestDoc::new();
        let section = doc.push_element(0, "section", None);
        let mid = doc.push_element(section, "mid", None);
        let style_a = doc.push_element(mid, "style", None);
        // A uses `color` — distinguishable from B's `background-color` below
        // so the build_rule_tree assertions can pin *which* rule landed at
        // which source_order, not just that 2 rules exist.
        doc.push_text(style_a, "p { color: red }");

        let aside = doc.push_element(0, "aside", None); // later sibling of `section`
        let style_b = doc.push_element(aside, "style", None);
        doc.push_text(style_b, "div { background-color: blue }");

        // Entry point 1: raw text collection order via `walk_style_elements`.
        let mut collected: Vec<String> = Vec::new();
        super::walk_style_elements(&doc, |css| collected.push(css.to_string()));
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], "p { color: red }");
        assert_eq!(collected[1], "div { background-color: blue }");

        // Entry point 2: `source_order` assigned via `build_rule_tree`, which
        // is the value that actually feeds the cascade tie-break — pin it
        // too so a regression here is caught even if a future change routes
        // rule extraction through `build_rule_tree` without going through
        // the raw-text collection path in the same way. Distinguish A vs B
        // by declaration kind (Color vs BackgroundColor) rather than just
        // counting, so a swap is actually detected.
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
        assert!(matches!(
            tree.style_rules[0].declarations()[0].value(),
            PropertyValue::Color(_)
        ));
        assert!(matches!(
            tree.style_rules[1].declarations()[0].value(),
            PropertyValue::BackgroundColor(_)
        ));
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
    // Spec: CSS Paged Media Level 3, §4.3 "@page rule grammar"
    // <https://www.w3.org/TR/css-page-3/#syntax-page-selector>
    //
    // Test で使う body は M1.4 の property.rs でサポート済み (color / font-*) を
    // 選ぶ — parse_declaration_block を reuse しているので @page descriptor
    // (`size` / `marks` / `bleed` 等) や未サポート property は現時点で silent drop
    // され declaration 0 個になる (下の
    // page_body_unsupported_property_drops_declaration がその regression guard)。
    //
    // NB (raikiri-spike-0vv.5): `margin` は 0vv.5 で author scope の supported
    // property になった (parse_declaration_block 出口で 4 longhand に展開)。
    // @page context での margin-box descriptor 挙動 (L3 §5) は依然 M4+ scope、
    // 通常の longhand `margin-top` 等の parse は @page body 内でも成立するが
    // page-context specific な意味付けは持たない。

    use crate::page::{PagePseudo, PageSelector, PageSelectorEntry};
    use crate::{Atom, PageRule};

    /// Test helper — build a `PageSelector` with a single compound entry
    /// containing exactly the given ident and pseudo-page list. Reduces the
    /// verbosity of `PageSelector { entries: vec![PageSelectorEntry { ident,
    /// pseudos, .. }] }` at every assertion site (raikiri-spike-mvu).
    fn ps_single(ident: Option<Atom>, pseudos: Vec<PagePseudo>) -> PageSelector {
        PageSelector {
            entries: vec![PageSelectorEntry {
                ident,
                pseudos,
                ..PageSelectorEntry::default()
            }],
        }
    }

    fn page_rules(source: &str) -> Vec<PageRule> {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        tree.page_rules
    }

    #[test]
    fn page_default_selector_no_prelude() {
        // `@page { color: red }` → empty prelude represented as one default
        // entry (raikiri-spike-mvu: PageSelector is now a Vec<Entry> shape,
        // uniform for M4 cascade iteration). declarations は 1 個、origin は
        // `page_rules(...)` helper が Author hardcode (raikiri-spike-jzv)。
        let rules = page_rules("@page { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![]));
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].source_order, 0);
        assert_eq!(rules[0].origin, Origin::Author);
    }

    #[test]
    fn page_pseudo_first() {
        let rules = page_rules("@page :first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
    }

    #[test]
    fn page_pseudo_left_right_blank() {
        let rules = page_rules(
            "@page :left { color: red } \
             @page :right { color: red } \
             @page :blank { color: red }",
        );
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::Left]));
        assert_eq!(rules[1].selector, ps_single(None, vec![PagePseudo::Right]));
        assert_eq!(rules[2].selector, ps_single(None, vec![PagePseudo::Blank]));
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
            ps_single(Some(Atom::from("my-cover")), vec![])
        );
    }

    #[test]
    fn page_multi_pseudo_is_accepted() {
        // raikiri-spike-mvu: L3 `<page-selector>` = `[ <ident-token>?
        // <pseudo-page>* ]!` permits any number of adjacent pseudo-pages.
        // Compound rule ("No whitespace is allowed between the productions
        // in `<page-selector>` or `<pseudo-page>`") — the input must be
        // written without whitespace between the two pseudos, hence
        // `:first:left`, not `:first :left`. The spaced form is pinned by
        // `page_multi_pseudo_with_whitespace_between_is_dropped` below.
        let rules = page_rules("@page :first:left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(None, vec![PagePseudo::First, PagePseudo::Left])
        );
    }

    #[test]
    fn page_functional_pseudo_is_dropped() {
        // Spec-outside functional pseudo (e.g. `:nth-page(...)`) は CSS Paged
        // Media L3 (anchor `#syntax-page-selector`) も L4 Editor's Draft も
        // 定義していないため、raikiri-style としては未知 pseudo として rule ごと
        // drop する。もし raikiri-local な拡張として実装する日が来れば、そのときは
        // 明示的に variant を追加し (現在のこの guard test を反転) スコープを
        // 人間 ledger で決める。
        let rules = page_rules("@page :nth-page(2n+1) { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_ident_plus_pseudo_is_accepted() {
        // raikiri-spike-mvu: L3 `<page-selector>` allows an ident followed
        // by pseudo-pages (`named:first`). No whitespace between them per
        // the compound rule — the spaced form (`named :first`) is dropped
        // by `page_named_with_whitespace_before_pseudo_is_dropped` below.
        let rules = page_rules("@page named:first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(Some(Atom::from("named")), vec![PagePseudo::First])
        );
    }

    #[test]
    fn page_selector_list_with_comma_is_accepted() {
        // raikiri-spike-mvu: L3 `<page-selector-list>` = `<page-selector>#`
        // — a comma-separated list of compound page-selectors. Whitespace
        // around the `,` is spec-permitted (the list is not itself a
        // compound). Expected: one rule with two entries.
        let rules = page_rules("@page :first, :left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            PageSelector {
                entries: vec![
                    PageSelectorEntry {
                        ident: None,
                        pseudos: vec![PagePseudo::First],
                        ..PageSelectorEntry::default()
                    },
                    PageSelectorEntry {
                        ident: None,
                        pseudos: vec![PagePseudo::Left],
                        ..PageSelectorEntry::default()
                    },
                ],
            }
        );
    }

    #[test]
    fn page_unknown_pseudo_is_dropped() {
        // `:cover` は the L3 grammar (spec anchor `#syntax-page-selector`) に存在しないため drop。
        let rules = page_rules("@page :cover { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_pseudo_is_case_insensitive() {
        // CSS keyword は ASCII case-insensitive (`match_ignore_ascii_case!` 経由)。
        let rules = page_rules("@page :FIRST { color: red } @page :Left { color: red }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
        assert_eq!(rules[1].selector, ps_single(None, vec![PagePseudo::Left]));
    }

    // ── Compound whitespace tightening (raikiri-spike-mvu, codex §8.3 F3) ──
    //
    // CSS Paged Media L3 (anchor `#syntax-page-selector`) states: "No
    // whitespace is allowed between the productions in `<page-selector>` or
    // `<pseudo-page>` (similar to the rule for `<compound-selector>`)".
    // Whitespace *around* the `,` separator of `<page-selector-list>` is
    // allowed (the list is not a compound); that variant is exercised by
    // `page_comma_list_with_whitespace_around_comma_is_accepted` below.

    #[test]
    fn page_whitespace_between_colon_and_pseudo_is_dropped() {
        // `@page : left` — whitespace between `:` and pseudo ident violates
        // the `<pseudo-page>` compound rule. Whole rule dropped.
        let rules = page_rules("@page : left { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_multi_pseudo_with_whitespace_between_is_dropped() {
        // `@page :first :left` — whitespace between two pseudo-pages of the
        // same compound violates the `<page-selector>` compound rule.
        let rules = page_rules("@page :first :left { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_named_with_whitespace_before_pseudo_is_dropped() {
        // `@page my-cover :first` — whitespace between ident and pseudo of
        // the same compound violates the `<page-selector>` compound rule.
        let rules = page_rules("@page my-cover :first { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_comma_list_with_whitespace_around_comma_is_accepted() {
        // Whitespace around the `,` of `<page-selector-list>` is spec-
        // permitted (the list is not a compound). Both `,` and ` , ` and
        // `, ` MUST all yield the same two-entry result.
        for src in [
            "@page :first, :left { color: red }",
            "@page :first , :left { color: red }",
            "@page :first ,:left { color: red }",
        ] {
            let rules = page_rules(src);
            assert_eq!(rules.len(), 1, "source: {src:?}");
            assert_eq!(rules[0].selector.entries.len(), 2, "source: {src:?}");
        }
    }

    #[test]
    fn page_ident_with_multi_pseudo_is_accepted() {
        // Ident + multiple pseudo-pages, all adjacent — the fullest shape
        // the L3 compound grammar permits.
        let rules = page_rules("@page named:first:left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(
                Some(Atom::from("named")),
                vec![PagePseudo::First, PagePseudo::Left]
            )
        );
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
        // M1.4 property.rs は size / marks 等 @page descriptor を未サポート。
        // parse_declaration_block reuse により silent drop され declaration 0 個。
        // M4 で @page descriptor が入るまで cascade 側は空 declarations を扱える
        // ことを保証する regression guard。
        //
        // NB (raikiri-spike-0vv.5): pre-0vv.5 では `margin: 1cm` を dropped
        // 例に使っていたが (`margin` property 自体が未認識だった)、0vv.5 で
        // `margin` は author scope で認識されるようになった (unit `cm` は依然
        // 未サポート = drop するが、drop 経路が「property 未認識」から
        // 「unit 未サポート」に変わる)。@page-specific descriptor のみで例を
        // 組み直し、意図する "@page descriptor drop" の regression guard に集約。
        let rules = page_rules("@page { size: A4; marks: crop }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![]));
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

    // ── Comment / whitespace transparency within compound (raikiri-spike-mvu spec F2) ──
    //
    // CSS Syntax L3 §4.3.2 Consume comments specifies that a /*…*/
    // sequence is consumed and the algorithm returns nothing — no token
    // is emitted into the token stream (see
    // <https://www.w3.org/TR/css-syntax-3/#consume-comment>). Within a
    // `<page-selector>` compound, therefore, a comment between two
    // components MUST behave as if absent —
    // `:first/*x*/:left` == `:first:left`. Whitespace, by contrast, IS
    // emitted as a <whitespace-token> and still breaks the compound per
    // L3 §4.3.

    #[test]
    fn page_comment_between_pseudos_is_transparent() {
        // `@page :first/*sep*/:left` — comment only, no whitespace within
        // compound. Per CSS Syntax L3 tokenization, comments vanish, so this
        // is equivalent to `@page :first:left` → 1 rule with two pseudos.
        let rules = page_rules("@page :first/*sep*/:left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(None, vec![PagePseudo::First, PagePseudo::Left])
        );
    }

    #[test]
    fn page_comment_between_colon_and_pseudo_is_transparent() {
        // `@page :/*x*/first` — comment between `:` and pseudo ident.
        // Comment vanishes at tokenization, so this equals `@page :first` →
        // 1 rule. Contrast with `@page : first` (whitespace-separated),
        // which is dropped by `page_whitespace_between_colon_and_pseudo_is_dropped`.
        let rules = page_rules("@page :/*x*/first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
    }

    #[test]
    fn page_whitespace_plus_comment_between_pseudos_is_dropped() {
        // `@page :first /*sep*/:left` — comment is transparent, but the
        // leading whitespace is a real compound-boundary token per L3 §4.3.
        // Whole rule dropped (matches `page_multi_pseudo_with_whitespace_between_is_dropped`).
        let rules = page_rules("@page :first /*sep*/:left { color: red }");
        assert!(rules.is_empty());
    }

    // ── <custom-ident> case-sensitivity for named-page ident (raikiri-spike-mvu spec F3) ──
    //
    // The named-page ident in a `<page-selector>` derives from the `page`
    // property (CSS Paged Media L3 §8.1 `#using-named-pages`), which types
    // its value as `<custom-ident>`. Per CSS Values L4 §4.2
    // `#custom-idents`: "Such identifiers are fully case-sensitive
    // (meaning they're compared using the 'identical to' operation), even
    // in the ASCII range (e.g. example and EXAMPLE are two different,
    // unrelated user-defined identifiers)." So `Cover` and `cover` MUST be
    // preserved verbatim and treated as distinct named-pages.

    #[test]
    fn page_named_ident_is_case_sensitive() {
        let rules = page_rules("@page Cover { color: red } @page cover { color: blue }");
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].selector,
            ps_single(Some(Atom::from("Cover")), vec![])
        );
        assert_eq!(
            rules[1].selector,
            ps_single(Some(Atom::from("cover")), vec![])
        );
    }

    // ── <page-selector># list-boundary invariants (raikiri-spike-mvu spec F4) ──
    //
    // `<page-selector-list> = <page-selector>#` per CSS Paged Media L3 §4.3
    // (anchor `#syntax-page-selector`). The `#` multiplier is "one or more,
    // comma-separated" per CSS Values L4 `#component-multipliers`, so each
    // list entry must be a *non-empty* `<page-selector>`. Trailing,
    // leading, and empty-middle commas violate this and drop the whole
    // `@page` rule. These 3 tests close 3 of the 6 malformed-prelude
    // debt cases enumerated in bd raikiri-spike-rm8; the remaining 3
    // (trailing colon on named page, adjacent idents, etc.) stay in rm8.

    #[test]
    fn page_trailing_comma_is_dropped() {
        let rules = page_rules("@page :first, { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_leading_comma_is_dropped() {
        let rules = page_rules("@page ,:first { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_empty_middle_entry_is_dropped() {
        let rules = page_rules("@page :first, , :left { color: red }");
        assert!(rules.is_empty());
    }

    // ── Malformed prelude — trailing/isolated colon and adjacent idents (raikiri-spike-rm8) ──
    //
    // Companion to the F4 banner above. F4 pinned the 3 list-boundary
    // cases (trailing / leading / empty-middle commas) of `<page-selector>#`;
    // rm8 pins the 3 compound-internal cases against the CSS Paged Media L3
    // §4.3 (anchor `#syntax-page-selector`) compound grammar
    // `<page-selector> = [ <ident-token>? <pseudo-page>* ]!` with
    // `<pseudo-page> = ':' [ left | right | first | blank ]`. Together the two
    // banners close the 6-case malformed-prelude debt set enumerated in
    // bd raikiri-spike-rm8:
    //   - `named:`      — trailing colon, missing required left|right|first|blank keyword
    //   - `:`           — bare colon, same
    //   - `named other` — two adjacent idents, compound allows only one
    // Each case drops the whole `@page` rule (declarations not captured).

    #[test]
    fn page_named_trailing_colon_is_dropped() {
        // `@page named:` — named-page ident に `:` が付いて次に来るべき
        // `<pseudo-page>` の keyword (left/right/first/blank) が来ずに block
        // が始まる shape。compound grammar `[ <ident-token>? <pseudo-page>* ]!`
        // の `<pseudo-page> = ':' [ left | right | first | blank ]` が
        // required keyword を欠くため drop。
        let rules = page_rules("@page named: { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_bare_colon_is_dropped() {
        // `@page :` — colon 単独。`<pseudo-page> = ':' [ left | right | first |
        // blank ]` が required keyword を欠くため drop。
        let rules = page_rules("@page : { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_two_adjacent_idents_are_dropped() {
        // `@page named other` — compound 内で `<ident-token>` は先頭 1 個のみ。
        // 2 個目の ident は compound 継続でも次 entry (comma がない) でもない
        // ため、`parse_comma_separated` の entry-parses-entirely 検査が leftover
        // を見て Err → whole rule drop。
        let rules = page_rules("@page named other { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_rules_captured_via_build_rule_tree_from_dom() {
        // build_rule_tree (DOM 経由) でも page_rules が populate される。
        let doc = dom_with_style("@page :first { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.page_rules.len(), 1);
        assert_eq!(
            tree.page_rules[0].selector,
            ps_single(None, vec![PagePseudo::First])
        );
        assert_eq!(tree.style_rules.len(), 1);
    }

    // ── counter_styles wiring (bd raikiri-spike-gce8, origin-aware since bd raikiri-spike-f7vg) ──

    #[test]
    fn empty_rule_tree_has_empty_counter_styles() {
        assert!(RuleTree::empty().counter_styles().is_empty());
    }

    #[test]
    fn add_stylesheet_populates_counter_styles() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 1);
    }

    #[test]
    fn add_stylesheet_invalid_counter_style_rule_is_dropped() {
        // `system: cyclic` に symbols 0 個 — is_valid() が false になり drop
        // される (counter_style.rs の同型 test と同じ shape)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@counter-style foo { system: cyclic; }", Origin::Author);
        assert!(tree.counter_styles().is_empty());
    }

    #[test]
    fn add_stylesheet_counter_style_alongside_style_and_page_rules() {
        // 1 回の add_stylesheet 呼び出しで 3 種の rule が同じ source から
        // それぞれ populate されることを確認 — 独立 2nd pass であって
        // style_rules/page_rules の収集を妨げない。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"
            @counter-style thumbs { system: cyclic; symbols: "*"; }
            @page :first { color: red }
            p { color: blue }
            "#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        assert_eq!(tree.page_rules.len(), 1);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn counter_styles_same_name_later_author_call_replaces_earlier_entirely() {
        // CounterStyleRegistry 型 doc が明記する「同名は atomically 後勝ち」を、
        // 複数 add_stylesheet(Origin::Author) 呼び出しをまたいで確認する
        // (page_source_order_monotonic_across_add_stylesheet_calls の
        // counter-style 版)。両方 Author origin — 同一 origin 内での source
        // order tie-break が CSS Counter Styles L3 §3 の "standard cascade
        // rules" と一致する場合。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::Author,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_useragent_origin_alone_populates_counter_styles() {
        // CSS Counter Styles L3 §3: defining an @counter-style makes it
        // available unconditionally — the "only one wins, according to
        // standard cascade rules" sentence only applies when there IS a
        // same-name conflict. A standalone Origin::UserAgent rule with no
        // competing Origin::Author rule has no conflict, so it must be
        // available. Before bd raikiri-spike-f7vg, add_stylesheet's
        // Author-only gate dropped this unconditionally regardless of
        // conflict — that was the bug this test now pins the fix for
        // (previously named *_does_not_populate_counter_styles and asserted
        // the opposite). style_rules 側が origin を問わず populate される
        // ことは既存の add_stylesheet_ua_and_author_populate_rule_tree が
        // 別途 pin 済み。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 1);
    }

    #[test]
    fn add_stylesheet_useragent_after_author_does_not_override_counter_styles() {
        // CSS Counter Styles L3 §3: "only one wins, according to standard
        // cascade rules" — origin が第一基準で UA は常に Author に負ける。
        // Author を先に定義し、同名 @counter-style を UA 側で後から
        // add_stylesheet しても、Author の定義が生き残ることを確認する。
        // bd raikiri-spike-f7vg 以降、この保証は add_stylesheet 側の
        // Origin::Author ゲート (UA を無条件 drop) ではなく、
        // CounterStyleRegistry::insert_with_origin が同名 entry の origin を
        // 個別に追跡して行う origin-precedence 解決 (型 doc の解決表) が担う
        // — 「flat call-order last-wins だと UA が後から Author を上書きし
        // 得る」spec 違反の regression pin (bd raikiri-spike-gce8) は変わらず
        // 有効。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::Author,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::UserAgent,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        // UA 側の 2-symbol 定義ではなく、Author 側の 1-symbol 定義のまま。
        assert_eq!(rule.symbols.len(), 1);
    }

    #[test]
    fn add_stylesheet_author_after_useragent_overrides_counter_styles() {
        // 上のテストの call-order を反転させた版 — UA を先に定義し、同名
        // @counter-style を Author 側で後から add_stylesheet する。origin
        // 優先順位 (Author > UserAgent) は call order 非依存であるべきなので、
        // こちらも Author が勝つ (CSS Counter Styles L3 §3, "standard cascade
        // rules" は origin が第一基準 — bd raikiri-spike-f7vg)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        // Author 側の 2-symbol 定義が勝つ。
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_useragent_same_name_later_call_replaces_earlier() {
        // counter_styles_same_name_later_author_call_replaces_earlier_entirely
        // の Origin::UserAgent 版 — 同一 origin (UserAgent) 内での
        // source-order tie-break (「後勝ち」) が Author 側と対称に効くことを
        // 確認する (CounterStyleRegistry::insert_with_origin の型 doc 解決表、
        // bd raikiri-spike-f7vg)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::UserAgent,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_useragent_and_author_different_names_both_populate_counter_styles() {
        // The headline spec claim this whole fix (bd raikiri-spike-f7vg) is
        // about: CSS Counter Styles L3 §3 makes defining an @counter-style
        // available unconditionally — availability, not just same-name
        // conflict resolution. Every other origin-mixing test above reuses
        // the same rule name ("thumbs") specifically to exercise conflict
        // resolution; this one pins the non-conflicting case those can't:
        // two differently-named rules from different origins must both
        // survive together in the registry.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style ua-thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        tree.add_stylesheet(
            r#"@counter-style author-thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 2);
        assert!(tree.counter_styles().get("ua-thumbs").is_some());
        assert!(tree.counter_styles().get("author-thumbs").is_some());
    }

    #[test]
    fn build_rule_tree_populates_counter_styles_from_dom_style_element() {
        let doc = dom_with_style(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; } p { color: red }"#,
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.counter_styles().len(), 1);
        assert!(tree.counter_styles().get("thumbs").is_some());
        // 同 source の style rule も引き続き populate される (2nd pass が既存
        // walk を妨げないことの確認)。
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn build_rule_tree_counter_styles_across_multiple_style_elements_last_wins() {
        // 兄弟 <style> 2 個、同名 @counter-style — walk_and_collect の
        // document order 呼び出しに従い、後の <style> の定義が勝つ。
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(
            s1,
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        );
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(
            s2,
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 2);
    }
}
