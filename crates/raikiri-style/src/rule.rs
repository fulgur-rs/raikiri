//! CSS rule と declaration の shape。M1.4 では type/universal selector を含む
//! qualified rule (StyleRule) のみサポート。at-rule (@page / @media 等) は
//! ruletree.rs 側で skip。

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use selectors::parser::SelectorList;

use crate::RaikiriSelectorImpl;
use crate::property::{PropertyValue, parse_value};

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
/// `origin` は CSS Cascading L4 §6.2 の origin (M1.4a、m1.22)。cascade tuple
/// の rank 化 (`!important` 反転扱い) に使用。
/// Future field (specificity cache / invalidation hint 等) は M4+ で追加、
/// `#[non_exhaustive]` の恩恵で non-breaking。
#[non_exhaustive]
pub struct StyleRule {
    /// Parse 済 selector list。M1.4 では type + universal のみ受理 (他は build 段で drop)。
    pub selectors: SelectorList<RaikiriSelectorImpl>,
    /// このルールの declaration list (invalid は含まない)。
    pub declarations: Vec<Declaration>,
    /// RuleTree 全体を通した 0-indexed source order。
    pub source_order: u32,
    /// この rule が属する cascade origin (raikiri-spike-m1.22)。
    pub origin: crate::ruletree::Origin,
}

/// declaration-list を消費して `Vec<Declaration>` を produce。
/// 認識できない property name / invalid value は silently drop。
///
/// # Shorthand expansion (raikiri-spike-0vv.5)
///
/// spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim: "A shorthand
/// property sets all of its longhand sub-properties, exactly as if expanded
/// in place." に準拠して、[`PropertyValue::Margin`] 系の shorthand declaration
/// は本関数の出口で 4 longhand declaration に展開される。cascade 段の
/// per-side winner selection が自然に成立 (HashMap iteration 順に依存しない
/// determinism) を担保するための spec-correct な expansion — 詳細は
/// [`expand_shorthand_into`] doc 参照。
pub(crate) fn parse_declaration_block(input: &mut Parser<'_, '_>) -> Vec<Declaration> {
    let mut parser = DeclParser;
    let mut out = Vec::new();
    for decl in RuleBodyParser::new(input, &mut parser).flatten() {
        expand_shorthand_into(decl, &mut out);
    }
    out
}

/// Shorthand declaration を対応する longhand declaration 列に展開して
/// `out` に in-place push する。non-shorthand はそのまま 1 個 push される。
///
/// # Rationale (per-key cascade determinism)
///
/// [`crate::cascade::apply_value`] は [`crate::cascade::pick_winners`] 結果を
/// `HashMap::into_values()` で iterate する。std [`std::collections::HashMap`] の
/// iteration 順は per-process randomly seeded で decl 適用順が nondeterministic
/// になる。既存 property は全て key と field が 1:1 disjoint のため apply 順
/// に依存しなかったが、`margin` shorthand + `margin-*` longhand の cross-key
/// dependency (`margin: 0; margin-top: 10px` は spec 上 top=10、他=0) では
/// apply 順が結果を左右する。
///
/// spec CSS Cascading L4 §3 "Shorthand Properties"
/// <https://www.w3.org/TR/css-cascade-4/#shorthand> は shorthand を "sets all
/// of its longhand sub-properties, exactly as if expanded in place" と定義し
/// shorthand を longhand の syntactic sugar と扱う。本関数は parse 直後に spec
/// のこの等価変換を実行することで、cascade 段には longhand のみが伝わる不変を
/// 確立する — HashMap iteration 順に依存しない per-side cascade を得る
/// (`static ordering after cascade` の deterministic な source of truth)。
///
/// # `important` flag propagation
///
/// shorthand の `!important` は各 longhand にそのまま copy される (spec §3
/// verbatim: "Declaring a shorthand property to be !important is equivalent to
/// declaring all of its sub-properties to be !important.")。
///
/// # Allocation shape
///
/// caller が保持する `out: &mut Vec<Declaration>` に直接 push する — 従来の
/// `flat_map + vec![d].into_iter()` は non-shorthand path で per-decl の 1-slot
/// heap Vec を alloc していた (common case regression、reviewer:quality F6)、
/// in-place push で除去。shorthand path は 4 longhand を 4 回 push (同 alloc
/// budget、shape のみ変更)。
fn expand_shorthand_into(d: Declaration, out: &mut Vec<Declaration>) {
    match d.value {
        PropertyValue::Margin(sides) => {
            out.push(Declaration {
                value: PropertyValue::MarginTop(sides.top),
                important: d.important,
            });
            out.push(Declaration {
                value: PropertyValue::MarginRight(sides.right),
                important: d.important,
            });
            out.push(Declaration {
                value: PropertyValue::MarginBottom(sides.bottom),
                important: d.important,
            });
            out.push(Declaration {
                value: PropertyValue::MarginLeft(sides.left),
                important: d.important,
            });
        }
        PropertyValue::Padding(sides) => {
            out.push(Declaration {
                value: PropertyValue::PaddingTop(sides.top),
                important: d.important,
            });
            out.push(Declaration {
                value: PropertyValue::PaddingRight(sides.right),
                important: d.important,
            });
            out.push(Declaration {
                value: PropertyValue::PaddingBottom(sides.bottom),
                important: d.important,
            });
            out.push(Declaration {
                value: PropertyValue::PaddingLeft(sides.left),
                important: d.important,
            });
        }
        _ => out.push(d),
    }
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
        let value = parse_value(name.as_ref(), input).ok_or_else(|| input.new_custom_error(()))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        // Exhaustive consumption: trailing garbage after the value (and optional
        // `!important`) must reject the whole declaration rather than silently
        // accepting a prefix (e.g. `color: red garbage` / `font-size: 16px 20px`).
        input.expect_exhausted().map_err(
            |e: cssparser::BasicParseError<'i>| -> ParseError<'i, Self::Error> { e.into() },
        )?;
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
    use crate::property::{CssColor, Length, LengthOrAuto};
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
            PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
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
        // width: 未対応 property → drop (0vv.5 以前は `margin` を dropped 例に
        // 使っていたが、margin は 0vv.5 で認識対象になったため差し替え。width
        // は現行 milestone subset 外)
        // font-size: 1em → em 未対応 → drop
        // color: red → 残す
        let decls = parse_block("width: 10px; font-size: 1em; color: red;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::Color(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
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

    #[test]
    fn rejects_trailing_garbage_after_value() {
        // "red garbage" — value 後に余計な token があるので declaration ごと drop。
        let decls = parse_block("color: red garbage;");
        assert!(decls.is_empty());
    }

    #[test]
    fn rejects_extra_length_after_font_size() {
        // "16px 20px" — 2 つ目の length は exhaust しない garbage 扱いで drop。
        let decls = parse_block("font-size: 16px 20px;");
        assert!(decls.is_empty());
    }

    #[test]
    fn still_accepts_important_after_value() {
        // regression guard: !important は exhaustive-consumption check の後でも
        // 引き続き受理されなければならない。
        let decls = parse_block("color: red !important;");
        assert_eq!(decls.len(), 1);
        assert!(decls[0].important);
    }

    #[test]
    fn font_family_leaves_important_alone() {
        // parse_font_family の loop が `!` (from `!important`) を garbage として
        // 拒否してしまうと、declaration ごと drop される (Finding 3)。
        let decls = parse_block("font-family: Arial !important;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::FontFamily(vec![crate::Atom::from("Arial")])
        );
        assert!(decls[0].important);
    }

    // ── margin shorthand expansion (CSS Cascading L4 §3、raikiri-spike-0vv.5) ──
    //
    // `parse_declaration_block` は shorthand `margin` を 4 longhand
    // (`MarginTop` / `MarginRight` / `MarginBottom` / `MarginLeft`) に展開する。
    // spec §3 "Shorthand Properties"
    // <https://www.w3.org/TR/css-cascade-4/#shorthand> の "sets all of its
    // longhand sub-properties, exactly as if expanded in place" 準拠、cascade 段
    // の HashMap 順非依存 determinism を parse-time で担保する。

    #[test]
    fn margin_shorthand_expands_into_four_longhand_declarations() {
        // `margin: 10px 20px` → 4 longhand (top=10, right=20, bottom=10, left=20)。
        let decls = parse_block("margin: 10px 20px;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(
            decls[0].value,
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
        );
        assert_eq!(
            decls[1].value,
            PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(20.0)))
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(10.0)))
        );
        assert_eq!(
            decls[3].value,
            PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(20.0)))
        );
    }

    #[test]
    fn margin_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L4 §3 "Shorthand Properties"
        // <https://www.w3.org/TR/css-cascade-4/#shorthand> verbatim:
        // "Declaring a shorthand property to be !important is equivalent to
        // declaring all of its sub-properties to be !important." — shorthand の
        // `!important` は全 longhand に copy される。
        let decls = parse_block("margin: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn margin_shorthand_five_values_declaration_dropped() {
        // 5+ value shorthand: parse_margin_shorthand は 4 value 消費、5th 残り
        // token は expect_exhausted で declaration drop。end-to-end で 0 decl
        // になることを pin (property.rs の
        // `margin_shorthand_leaves_extra_values_for_caller_exhausted_check` と complementary)。
        let decls = parse_block("margin: 10px 20px 30px 40px 50px;");
        assert!(
            decls.is_empty(),
            "5-value shorthand must be dropped by expect_exhausted, got {decls:?}"
        );
    }

    #[test]
    fn margin_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand の match arm を no-op で通過 (1 decl のまま)。
        // shorthand-only expansion の scope を pin する negative test。
        let decls = parse_block("margin-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(
            decls[0].value,
            PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
        );
    }

    // ── padding shorthand expansion (CSS Cascading L5、raikiri-spike-5nc) ──

    #[test]
    fn padding_shorthand_expands_into_four_longhand_declarations() {
        // `padding: 10px 20px` → 4 longhand (top=10, right=20, bottom=10, left=20)。
        // (margin 0vv.5 の parse-time expansion model を padding に migrate: raikiri-spike-5nc)
        let decls = parse_block("padding: 10px 20px;");
        assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
        assert_eq!(
            decls[1].value,
            PropertyValue::PaddingRight(Length::Px(20.0))
        );
        assert_eq!(
            decls[2].value,
            PropertyValue::PaddingBottom(Length::Px(10.0))
        );
        assert_eq!(decls[3].value, PropertyValue::PaddingLeft(Length::Px(20.0)));
    }

    #[test]
    fn padding_shorthand_important_flag_propagates_to_all_longhand() {
        // spec CSS Cascading L5 §3: shorthand の `!important` は全 longhand に
        // copy される (margin important 拡張と同じ)。
        let decls = parse_block("padding: 5px !important;");
        assert_eq!(decls.len(), 4);
        for d in &decls {
            assert!(d.important, "important must propagate to every longhand");
        }
    }

    #[test]
    fn padding_longhand_declaration_not_expanded() {
        // longhand は expand_shorthand の match arm を no-op で通過 (1 decl のまま)。
        let decls = parse_block("padding-top: 10px;");
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
    }
}
