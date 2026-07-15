//! CSS rule と declaration の shape。M1.4 では type/universal selector を含む
//! qualified rule (StyleRule) のみサポート。at-rule (@page / @media 等) は
//! ruletree.rs 側で skip。

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::parser::SelectorList;

use crate::property::{parse_value, PropertyValue};
use crate::RaikiriSelectorImpl;

/// 1 property declaration = value + `!important` flag。
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    /// resolved property value。
    pub value: PropertyValue,
    /// `!important` flag (true なら importance 上げ)。
    pub important: bool,
}

/// Qualified style rule (`selectors { declarations }`)。
///
/// `source_order` は同一 `RuleTree` 内で 0 から通し番号。cascade tie-break
/// (同 specificity 時に「後勝ち」) に使う。
pub struct StyleRule {
    /// Parse 済 selector list。M1.4 では type + universal のみ受理 (他は build 段で drop)。
    pub selectors: SelectorList<RaikiriSelectorImpl>,
    /// このルールの declaration list (invalid は含まない)。
    pub declarations: Vec<Declaration>,
    /// RuleTree 全体を通した 0-indexed source order。
    pub source_order: u32,
}

/// declaration-list を消費して `Vec<Declaration>` を produce。
/// 認識できない property name / invalid value は silently drop。
///
/// pub(crate) の caller (ruletree.rs の qualified-rule block parser) は M1.4
/// 後続 task で追加される。それまでの間、非 test build では (このモジュールが
/// 呼ぶ property.rs の parser chain ごと) dead code になるため `#[allow]` を
/// 付与している — ruletree.rs から呼ばれるようになれば不要になる。
#[allow(dead_code)]
pub(crate) fn parse_declaration_block(input: &mut Parser<'_, '_>) -> Vec<Declaration> {
    let mut parser = DeclParser;
    RuleBodyParser::new(input, &mut parser).flatten().collect()
}

/// Per-declaration parser for cssparser::RuleBodyParser。
struct DeclParser;

impl<'i> DeclarationParser<'i> for DeclParser {
    type Declaration = Declaration;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &ParserState,
    ) -> Result<Declaration, ParseError<'i, Self::Error>> {
        let value =
            parse_value(name.as_ref(), input).ok_or_else(|| input.new_custom_error(()))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        Ok(Declaration { value, important })
    }
}

// At-rule parser は M1.4 では no-op (block 内で @rule が現れた場合は drop)。
impl<'i> AtRuleParser<'i> for DeclParser {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = ();
}

// Qualified-rule parser (nested rule) も no-op — block 内 nested rule は M1.4 では drop。
impl<'i> QualifiedRuleParser<'i> for DeclParser {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Declaration, ()> for DeclParser {
    fn parse_qualified(&self) -> bool {
        false
    }
    fn parse_declarations(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::{CssColor, Length};
    use cssparser::ParserInput;

    fn parse_block(source: &str) -> Vec<Declaration> {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        parse_declaration_block(&mut parser)
    }

    #[test]
    fn parses_single_declaration() {
        let decls = parse_block("color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor { r: 255, g: 0, b: 0, a: 255 })
        );
        assert!(!decls[0].important);
    }

    #[test]
    fn captures_important_flag() {
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn drops_invalid_property_and_value() {
        // margin: 未対応 property → drop
        // font-size: 1em → em 未対応 → drop
        // color: red → 残す
        let decls = parse_block("margin: 10px; font-size: 1em; color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor { r: 255, g: 0, b: 0, a: 255 })
        );
    }

    #[test]
    fn multiple_declarations_in_order() {
        let decls = parse_block("color: red; font-size: 16px; font-weight: 700;");
        assert_eq!(decls.len(), 3);
        assert!(matches!(decls[0].value, PropertyValue::Color(_)));
        assert_eq!(decls[1].value, PropertyValue::FontSize(Length::Px(16.0)));
        assert_eq!(decls[2].value, PropertyValue::FontWeight(700));
    }

    #[test]
    fn empty_block_returns_empty() {
        assert!(parse_block("").is_empty());
        assert!(parse_block("   ").is_empty());
    }
}
