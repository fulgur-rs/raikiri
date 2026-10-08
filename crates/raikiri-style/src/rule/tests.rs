use std::sync::Arc;

use super::*;
use crate::property::{
    BorderColor, BorderStyle, CssColor, FontStyle, FontVariantCaps, FontWeightValue, Length,
    LengthOrAuto, LineHeight, OutlineColor, OutlineStyle, OverflowValue, RelativeFontSize,
};
use cssparser::ParserInput;

fn parse_block(source: &str) -> Vec<Declaration> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_declaration_block(&mut parser)
}

/// `parse_declaration_block` must not emit shorthand keys. Shorthands are
/// expanded before the cascade so longhand precedence follows the CSS
/// shorthand contract.
#[test]
fn declaration_block_never_emits_shorthand_keys() {
    use crate::property::PropertyKey;

    let decls = parse_block(
        "margin: 1px; padding: 2px; border: 3px solid red; outline: auto 2px red; \
             margin-top: 4px; padding-left: 5px; border-top-width: 6px; \
             border-top: 1px solid; border-right: 2px dashed; border-bottom: blue; \
             border-left: thick; \
             text-decoration: underline overline; color: red; font-size: 10px; \
             font: italic small-caps bold 12px/1.5 serif",
    );
    assert!(
        !decls.is_empty(),
        "parse が空 — test corpus 側の regression"
    );

    for decl in &decls {
        let key = decl.value.key();
        assert!(
            !matches!(
                key,
                PropertyKey::Margin
                    | PropertyKey::Padding
                    | PropertyKey::Border
                    | PropertyKey::BorderTop
                    | PropertyKey::BorderRight
                    | PropertyKey::BorderBottom
                    | PropertyKey::BorderLeft
                    | PropertyKey::BorderStyle
                    | PropertyKey::BorderWidth
                    | PropertyKey::BorderColor
                    | PropertyKey::Outline
                    | PropertyKey::Font
                    | PropertyKey::TextDecoration
            ),
            "shorthand key {key:?} が cascade 段へ漏れている — \
                 `expand_shorthand_into` の展開 arm は exhaustive match により \
                 存在するはずなので、疑うのは `parse_declaration_block` が \
                 同関数を通さなくなったか、当該 variant が展開 arm ではなく \
                 non-shorthand 側の or-pattern に書かれているか (どちらも \
                 compile は通る)"
        );
    }
}

#[test]
fn deferred_shorthand_expands_to_each_longhand_key() {
    use crate::property::{DeferredValue, PropertyKey};

    let cases: &[(PropertyKey, &[PropertyKey])] = &[
        (
            PropertyKey::Padding,
            &[
                PropertyKey::PaddingTop,
                PropertyKey::PaddingRight,
                PropertyKey::PaddingBottom,
                PropertyKey::PaddingLeft,
            ],
        ),
        (
            PropertyKey::Margin,
            &[
                PropertyKey::MarginTop,
                PropertyKey::MarginRight,
                PropertyKey::MarginBottom,
                PropertyKey::MarginLeft,
            ],
        ),
        (
            PropertyKey::Border,
            &[
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
            PropertyKey::BorderTop,
            &[
                PropertyKey::BorderTopWidth,
                PropertyKey::BorderTopStyle,
                PropertyKey::BorderTopColor,
            ],
        ),
        (
            PropertyKey::BorderRight,
            &[
                PropertyKey::BorderRightWidth,
                PropertyKey::BorderRightStyle,
                PropertyKey::BorderRightColor,
            ],
        ),
        (
            PropertyKey::BorderBottom,
            &[
                PropertyKey::BorderBottomWidth,
                PropertyKey::BorderBottomStyle,
                PropertyKey::BorderBottomColor,
            ],
        ),
        (
            PropertyKey::BorderLeft,
            &[
                PropertyKey::BorderLeftWidth,
                PropertyKey::BorderLeftStyle,
                PropertyKey::BorderLeftColor,
            ],
        ),
        (
            PropertyKey::Overflow,
            &[PropertyKey::OverflowX, PropertyKey::OverflowY],
        ),
        (
            PropertyKey::BorderStyle,
            &[
                PropertyKey::BorderTopStyle,
                PropertyKey::BorderRightStyle,
                PropertyKey::BorderBottomStyle,
                PropertyKey::BorderLeftStyle,
            ],
        ),
        (
            PropertyKey::BorderWidth,
            &[
                PropertyKey::BorderTopWidth,
                PropertyKey::BorderRightWidth,
                PropertyKey::BorderBottomWidth,
                PropertyKey::BorderLeftWidth,
            ],
        ),
        (
            PropertyKey::BorderColor,
            &[
                PropertyKey::BorderTopColor,
                PropertyKey::BorderRightColor,
                PropertyKey::BorderBottomColor,
                PropertyKey::BorderLeftColor,
            ],
        ),
        (
            PropertyKey::TextDecoration,
            &[
                PropertyKey::TextDecorationLine,
                PropertyKey::TextDecorationThickness,
                PropertyKey::TextDecorationStyle,
                PropertyKey::TextDecorationColor,
            ],
        ),
        (
            PropertyKey::TextEmphasis,
            &[
                PropertyKey::TextEmphasisStyle,
                PropertyKey::TextEmphasisColor,
            ],
        ),
        (
            PropertyKey::Outline,
            &[
                PropertyKey::OutlineWidth,
                PropertyKey::OutlineStyle,
                PropertyKey::OutlineColor,
            ],
        ),
        (
            PropertyKey::Font,
            &[
                PropertyKey::FontStyle,
                PropertyKey::FontVariantCaps,
                PropertyKey::FontWeight,
                PropertyKey::FontSize,
                PropertyKey::LineHeight,
                PropertyKey::FontFamily,
                PropertyKey::FontKerning,
                PropertyKey::FontLanguageOverride,
                PropertyKey::FontOpticalSizing,
                PropertyKey::FontVariantEastAsian,
                PropertyKey::FontVariantEmoji,
                PropertyKey::FontVariantLigatures,
                PropertyKey::FontVariantNumeric,
                PropertyKey::FontVariantPosition,
                PropertyKey::FontVariationSettings,
                PropertyKey::FontFeatureSettings,
            ],
        ),
        (
            PropertyKey::Flex,
            &[
                PropertyKey::FlexGrow,
                PropertyKey::FlexShrink,
                PropertyKey::FlexBasis,
            ],
        ),
        (
            PropertyKey::Columns,
            &[PropertyKey::ColumnWidth, PropertyKey::ColumnCount],
        ),
        (
            PropertyKey::Gap,
            &[PropertyKey::RowGap, PropertyKey::ColumnGap],
        ),
        (
            PropertyKey::PlaceContent,
            &[PropertyKey::AlignContent, PropertyKey::JustifyContent],
        ),
        (
            PropertyKey::GridRow,
            &[PropertyKey::GridRowStart, PropertyKey::GridRowEnd],
        ),
        (
            PropertyKey::GridColumn,
            &[PropertyKey::GridColumnStart, PropertyKey::GridColumnEnd],
        ),
        (
            PropertyKey::PlaceItems,
            &[PropertyKey::AlignItems, PropertyKey::JustifyItems],
        ),
        (
            PropertyKey::PlaceSelf,
            &[PropertyKey::AlignSelf, PropertyKey::JustifySelf],
        ),
    ];

    for (shorthand, expected) in cases {
        let declaration = Declaration {
            value: PropertyValue::Deferred(DeferredValue {
                property: "test".into(),
                value: "raw".into(),
                key: *shorthand,
            }),
            important: true,
        };
        let mut expanded = Vec::new();
        expand_shorthand_into(&declaration, |declaration| {
            expanded.push((declaration.value.key(), declaration.important));
        });
        assert_eq!(
            expanded,
            expected
                .iter()
                .copied()
                .map(|key| (key, true))
                .collect::<Vec<_>>()
        );
    }

    let declaration = Declaration {
        value: PropertyValue::Deferred(DeferredValue {
            property: "width".into(),
            value: "raw".into(),
            key: PropertyKey::Width,
        }),
        important: false,
    };
    let mut expanded = Vec::new();
    expand_shorthand_into(&declaration, |declaration| {
        expanded.push((declaration.value.key(), declaration.important));
    });
    assert_eq!(expanded, vec![(PropertyKey::Width, false)]);
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
    // `cursor: pointer` is unsupported (CSS Basic User Interface
    // Module Level 3 <https://www.w3.org/TR/css-ui-3/#cursor>) → drop
    // (`margin` / `width` / `float` were previously used as dropped examples,
    // but were later recognized and replaced here; `cursor` remains
    // unsupported, matching the canary in property.rs's
    // `unknown_property_returns_none`).
    // `font-size: math` drops because the MathML scaling algorithm is not implemented
    // (`medium` was formerly used as a dropped example, but `<absolute-size>` /
    // `<relative-size>` keywords became recognized and were replaced here; see
    // the "keep examples synchronized when replacing them" section of the
    // `PropertyValue` docs in property.rs).
    // Keep `color: red`.
    let decls = parse_block("cursor: pointer; font-size: math; color: red;");
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
    assert_eq!(
        decls[2].value,
        PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))
    );
}

#[test]
fn empty_block_returns_empty() {
    assert!(parse_block("").is_empty());
    assert!(parse_block("   ").is_empty());
}

#[test]
fn rejects_trailing_garbage_after_value() {
    // "red garbage": the extra token after the value drops the whole declaration.
    let decls = parse_block("color: red garbage;");
    assert!(decls.is_empty());
}

#[test]
fn rejects_extra_length_after_font_size() {
    // "16px 20px": the second length is unconsumed garbage, so the declaration drops.
    let decls = parse_block("font-size: 16px 20px;");
    assert!(decls.is_empty());
}

#[test]
fn rejects_font_style_oblique_with_angle() {
    // CSS Fonts 4 §2.4's `oblique <angle [-90deg,90deg]>?` grammar lets
    // `oblique` take an optional angle, but this crate accepts `oblique`
    // only as a bare keyword (`FontStyle` doc's "Scope carving"
    // section) — the angle argument is out of scope.
    // `parse_font_style` consumes just the `oblique` ident and succeeds,
    // leaving `14deg` unconsumed, so the whole declaration is dropped by
    // `DeclParser::expect_exhausted` — mirrors
    // `rejects_extra_length_after_font_size` above.
    let decls = parse_block("font-style: oblique 14deg;");
    assert!(decls.is_empty());
}

#[test]
fn accepts_text_transform_keyword_combinations_in_either_order() {
    let decls = parse_block("text-transform: uppercase full-width;");
    assert_eq!(decls.len(), 1);
    let decls = parse_block("text-transform: full-width uppercase;");
    assert_eq!(decls.len(), 1);
}

#[test]
fn accepts_hanging_after_text_indent() {
    // "2em hanging" — now accepted (CSS Text 3 §8.1 hanging keyword).
    let decls = parse_block("text-indent: 2em hanging;");
    assert_eq!(decls.len(), 1);
}

#[test]
fn rejects_extra_length_after_vertical_align() {
    // "10px 20px" — `parse_vertical_align`'s `<length>` fallback doesn't
    // consume to end of declaration itself (mirrors every other
    // single-value property parser); the trailing token is unconsumed
    // garbage from `expect_exhausted`'s point of view and the whole
    // declaration drops — mirrors `rejects_extra_length_after_font_size`.
    let decls = parse_block("vertical-align: 10px 20px;");
    assert!(decls.is_empty());
}

#[test]
fn rejects_extra_ident_after_vertical_align() {
    // "middle top" — `top` is spec-valid (CSS 2.1 §10.8.1) but out of
    // this crate's scope (`VerticalAlign` doc's "Scope carving"
    // section), so it is unconsumed garbage from `expect_exhausted`'s
    // point of view and the whole declaration drops, same shape as
    // `rejects_extra_ident_after_text_indent` above.
    let decls = parse_block("vertical-align: middle top;");
    assert!(decls.is_empty());
}

#[test]
fn still_accepts_important_after_value() {
    // Regression guard: `!important` must still be accepted after the
    // exhaustive-consumption check.
    let decls = parse_block("color: red !important;");
    assert_eq!(decls.len(), 1);
    assert!(decls[0].important);
}

#[test]
fn font_family_leaves_important_alone() {
    // If the `parse_font_family` loop rejects `!` (from `!important`) as garbage,
    // it drops the entire declaration (Finding 3).
    let decls = parse_block("font-family: Arial !important;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::FontFamily(Arc::new(vec![crate::property::FontFamilyName::named(
            "Arial"
        )]))
    );
    assert!(decls[0].important);
}

// ── margin shorthand expansion (CSS Cascading L4 §3) ──
//
// `parse_declaration_block` expands the `margin` shorthand into four longhands
// (`MarginTop` / `MarginRight` / `MarginBottom` / `MarginLeft`).
// spec §3 "Shorthand Properties"
// Per "sets all of its longhand sub-properties, exactly as if expanded
// in place" in <https://www.w3.org/TR/css-cascade-4/#shorthand>, this prevents
// a shorthand key from reaching the cascade by expanding at parse time.

#[test]
fn margin_shorthand_expands_into_four_longhand_declarations() {
    // `margin: 10px 20px` → four longhands (top=10, right=20, bottom=10, left=20).
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
    // declaring all of its sub-properties to be !important": the shorthand's
    // `!important` is copied to all longhands.
    let decls = parse_block("margin: 5px !important;");
    assert_eq!(decls.len(), 4);
    for d in &decls {
        assert!(d.important, "important must propagate to every longhand");
    }
}

#[test]
fn margin_shorthand_five_values_declaration_dropped() {
    // For a 5+ value shorthand, `parse_margin_shorthand` consumes four values; the
    // remaining fifth token causes `expect_exhausted` to drop the declaration.
    // Check that this yields zero declarations end to end (complementing
    // `margin_shorthand_leaves_extra_values_for_caller_exhausted_check` in property.rs).
    let decls = parse_block("margin: 10px 20px 30px 40px 50px;");
    assert!(
        decls.is_empty(),
        "5-value shorthand must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn margin_inline_shorthand_three_values_declaration_dropped() {
    // End-to-end sibling of `margin_shorthand_five_values_declaration_dropped`
    // for the 2-value logical shorthand: 3+ value `margin-inline` must
    // be dropped by `expect_exhausted`, not silently truncated to the
    // first 2 values (property.rs's
    // `margin_inline_shorthand_leaves_extra_values_for_caller_exhausted_check`
    // pins the bare `parse_value`-level behavior; this pins the
    // end-to-end declaration-block outcome).
    let decls = parse_block("margin-inline: 10px 20px 30px;");
    // cov:ignore: the failure-message branch of this `assert!` only
    // executes when the assertion fails; it passes here, so llvm-cov
    // reports the macro's condition-false region as an uncovered added
    // line even though the assertion itself runs and does its job.
    assert!(
        decls.is_empty(),
        "3-value margin-inline shorthand must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn margin_block_shorthand_three_values_declaration_dropped() {
    // Sibling of `margin_inline_shorthand_three_values_declaration_dropped`
    // for the block-axis 2-value shorthand.
    let decls = parse_block("margin-block: 10px 20px 30px;");
    // cov:ignore: the failure-message branch of this `assert!` only
    // executes when the assertion fails; it passes here, so llvm-cov
    // reports the macro's condition-false region as an uncovered added
    // line even though the assertion itself runs and does its job.
    assert!(
        decls.is_empty(),
        "3-value margin-block shorthand must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn padding_inline_shorthand_three_values_declaration_dropped() {
    // Sibling of `margin_inline_shorthand_three_values_declaration_dropped`
    // for `padding-inline`.
    let decls = parse_block("padding-inline: 10px 20px 30px;");
    // cov:ignore: the failure-message branch of this `assert!` only
    // executes when the assertion fails; it passes here, so llvm-cov
    // reports the macro's condition-false region as an uncovered added
    // line even though the assertion itself runs and does its job.
    assert!(
        decls.is_empty(),
        "3-value padding-inline shorthand must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn padding_block_shorthand_three_values_declaration_dropped() {
    // Sibling of `margin_inline_shorthand_three_values_declaration_dropped`
    // for `padding-block`, the last of the 4 logical 2-value shorthands.
    let decls = parse_block("padding-block: 10px 20px 30px;");
    // cov:ignore: the failure-message branch of this `assert!` only
    // executes when the assertion fails; it passes here, so llvm-cov
    // reports the macro's condition-false region as an uncovered added
    // line even though the assertion itself runs and does its job.
    assert!(
        decls.is_empty(),
        "3-value padding-block shorthand must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn margin_longhand_declaration_not_expanded() {
    // A longhand passes through the `expand_shorthand_into` match arm unchanged (one declaration).
    // Negative test checking the scope of shorthand-only expansion.
    let decls = parse_block("margin-top: 10px;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(10.0)))
    );
}

// ── padding shorthand expansion (CSS Cascading L4 §3) ──

#[test]
fn padding_shorthand_expands_into_four_longhand_declarations() {
    // `padding: 10px 20px` → four longhands (top=10, right=20, bottom=10, left=20).
    // Migrate margin's parse-time expansion model to padding.
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
    // CSS Cascading L4 §3: a shorthand's `!important` is copied to every longhand,
    // as in the margin important extension.
    let decls = parse_block("padding: 5px !important;");
    assert_eq!(decls.len(), 4);
    for d in &decls {
        assert!(d.important, "important must propagate to every longhand");
    }
}

#[test]
fn padding_longhand_declaration_not_expanded() {
    // A longhand passes through the `expand_shorthand_into` match arm unchanged (one declaration).
    let decls = parse_block("padding-top: 10px;");
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].value, PropertyValue::PaddingTop(Length::Px(10.0)));
}

// ── border shorthand expansion (CSS Cascading L4 §3) ──
//
// `parse_declaration_block` expands the `border` shorthand into 12 longhands
// (four sides × three sub-properties: width / style / color).
// Under spec §3 "Shorthand Properties", which "sets all of its longhand sub-properties,
// exactly as if expanded in place", parse-time expansion prevents shorthand
// keys from reaching the cascade. Extends the margin / padding
// precedent to a 12-longhand shape.

#[test]
fn border_shorthand_expands_into_twelve_longhand_declarations() {
    // `border: 1px solid red` → 12 longhands (four sides × {width, style, color}).
    // order: top-w / top-s / top-c / right-w / right-s / right-c / bottom-* /
    // Check the handwritten order of the `left-*` longhands in
    // `expand_shorthand_into` to catch copy-paste regressions.
    let decls = parse_block("border: 1px solid red;");
    assert_eq!(
        decls.len(),
        12,
        "border shorthand must expand to 12 longhand decls"
    );
    let red = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    // The color longhand reaches the cascade as `BorderColor::Resolved(red)`:
    // the author's color slot in the shorthand is passed as the
    // Resolved variant (pin for hazard case 3).
    let red_bc = BorderColor::Resolved(red);
    assert_eq!(
        decls[0].value,
        PropertyValue::BorderTopWidth(Length::Px(1.0))
    );
    assert_eq!(
        decls[1].value,
        PropertyValue::BorderTopStyle(BorderStyle::Solid)
    );
    assert_eq!(decls[2].value, PropertyValue::BorderTopColor(red_bc));
    assert_eq!(
        decls[3].value,
        PropertyValue::BorderRightWidth(Length::Px(1.0))
    );
    assert_eq!(
        decls[4].value,
        PropertyValue::BorderRightStyle(BorderStyle::Solid)
    );
    assert_eq!(decls[5].value, PropertyValue::BorderRightColor(red_bc));
    assert_eq!(
        decls[6].value,
        PropertyValue::BorderBottomWidth(Length::Px(1.0))
    );
    assert_eq!(
        decls[7].value,
        PropertyValue::BorderBottomStyle(BorderStyle::Solid)
    );
    assert_eq!(decls[8].value, PropertyValue::BorderBottomColor(red_bc));
    assert_eq!(
        decls[9].value,
        PropertyValue::BorderLeftWidth(Length::Px(1.0))
    );
    assert_eq!(
        decls[10].value,
        PropertyValue::BorderLeftStyle(BorderStyle::Solid)
    );
    assert_eq!(decls[11].value, PropertyValue::BorderLeftColor(red_bc));
}

#[test]
fn border_shorthand_important_flag_propagates_to_all_longhand() {
    // CSS Cascading L4 §3: a shorthand's `!important` is copied to all longhands,
    // as in the margin / padding important extensions; check all 12 longhands.
    // as a regression check.
    let decls = parse_block("border: 5px dashed blue !important;");
    assert_eq!(decls.len(), 12);
    for d in &decls {
        assert!(d.important, "important must propagate to every longhand");
    }
}

#[test]
fn border_longhand_declaration_not_expanded() {
    // A longhand passes through the `expand_shorthand_into` match arm unchanged (one declaration).
    // Negative test checking shorthand-only expansion scope, as in the margin /
    // padding sibling tests.
    let decls = parse_block("border-top-width: 10px;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::BorderTopWidth(Length::Px(10.0))
    );
}

#[test]
fn border_side_family_shorthands_expand_into_four_longhands() {
    use crate::property::PropertyKey as K;
    let decls = parse_block(
        "border-style: solid dashed; border-width: 1px 2px 3px 4px; \
             border-color: red !important;",
    );
    let keys: Vec<_> = decls.iter().map(|d| d.value.key()).collect();
    assert_eq!(
        keys,
        [
            K::BorderTopStyle,
            K::BorderRightStyle,
            K::BorderBottomStyle,
            K::BorderLeftStyle,
            K::BorderTopWidth,
            K::BorderRightWidth,
            K::BorderBottomWidth,
            K::BorderLeftWidth,
            K::BorderTopColor,
            K::BorderRightColor,
            K::BorderBottomColor,
            K::BorderLeftColor,
        ]
    );
    assert_eq!(
        decls[1].value,
        PropertyValue::BorderRightStyle(BorderStyle::Dashed)
    );
    assert_eq!(
        decls[7].value,
        PropertyValue::BorderLeftWidth(Length::Px(4.0))
    );
    assert!(decls[..8].iter().all(|d| !d.important));
    assert!(decls[8..].iter().all(|d| d.important));
}

#[test]
fn border_shorthand_two_widths_declaration_dropped() {
    // property.rs `border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check`
    // End-to-end check: with `border: 1px 2px`, the shorthand helper puts 1px in
    // the width slot, then 2px matches neither style nor color. It falls through
    // and remains unconsumed, so the caller's `expect_exhausted` drops the whole
    // declaration (zero declarations).
    let decls = parse_block("border: 1px 2px;");
    assert!(
        decls.is_empty(),
        "border shorthand with leftover token must be dropped, got {decls:?}"
    );
}

// ── outline shorthand expansion (CSS Basic User Interface Module Level 3 §4.1) ──

#[test]
fn outline_shorthand_expands_into_three_longhand_declarations() {
    // CSS Cascading 4 §3: a shorthand sets each longhand as if expanded in
    // place. The outline-only `auto` style must stay an `OutlineStyle`.
    let decls = parse_block("outline: auto 2px red;");
    assert_eq!(decls.len(), 3, "shorthand must expand to 3 longhand decls");
    assert_eq!(decls[0].value, PropertyValue::OutlineWidth(Length::Px(2.0)));
    assert_eq!(
        decls[1].value,
        PropertyValue::OutlineStyle(OutlineStyle::Auto)
    );
    assert_eq!(
        decls[2].value,
        PropertyValue::OutlineColor(OutlineColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }))
    );
}

#[test]
fn outline_shorthand_important_flag_propagates_to_all_longhand() {
    // CSS Cascading 4 §3: `!important` on a shorthand applies to all of
    // its longhands.
    let decls = parse_block("outline: auto !important;");
    assert_eq!(decls.len(), 3);
    assert!(decls.iter().all(|declaration| declaration.important));
}

// ── font shorthand expansion (CSS Fonts 4 §2.1) ──

#[test]
fn font_shorthand_expands_into_sixteen_longhand_declarations() {
    // The first 6 declarations are grammar longhands. CSS Fonts 4 §2.1
    // then requires the 10 modeled reset-only subproperties to use their
    // initial values; `font-variant-caps` is already a grammar longhand.
    let decls = parse_block("font: italic small-caps bold 12px/1.5 serif;");
    assert_eq!(decls.len(), 16);
    assert_eq!(decls[0].value, PropertyValue::FontStyle(FontStyle::Italic));
    assert_eq!(
        decls[1].value,
        PropertyValue::FontVariantCaps(FontVariantCaps::SmallCaps)
    );
    assert_eq!(
        decls[2].value,
        PropertyValue::FontWeight(FontWeightValue::Absolute(700.0))
    );
    assert_eq!(decls[3].value, PropertyValue::FontSize(Length::Px(12.0)));
    assert_eq!(
        decls[4].value,
        PropertyValue::LineHeight(LineHeight::Number(1.5))
    );
    assert_eq!(
        decls[5].value,
        PropertyValue::FontFamily(Arc::new(vec![crate::property::FontFamilyName::generic(
            "serif"
        )]))
    );
    assert_eq!(
        decls[6].value,
        PropertyValue::FontKerning(FontKerning::Auto)
    );
    assert_eq!(
        decls[7].value,
        PropertyValue::FontLanguageOverride(FontLanguageOverride::Normal)
    );
    assert_eq!(
        decls[8].value,
        PropertyValue::FontOpticalSizing(FontOpticalSizing::Auto)
    );
    assert_eq!(
        decls[9].value,
        PropertyValue::FontVariantEastAsian(FontVariantEastAsian::initial())
    );
    assert_eq!(
        decls[10].value,
        PropertyValue::FontVariantEmoji(FontVariantEmoji::Normal)
    );
    assert_eq!(
        decls[11].value,
        PropertyValue::FontVariantLigatures(FontVariantLigatures::Normal)
    );
    assert_eq!(
        decls[12].value,
        PropertyValue::FontVariantNumeric(FontVariantNumeric::initial())
    );
    assert_eq!(
        decls[13].value,
        PropertyValue::FontVariantPosition(FontVariantPosition::Normal)
    );
    assert_eq!(
        decls[14].value,
        PropertyValue::FontVariationSettings(FontVariationSettings::Normal)
    );
    assert_eq!(
        decls[15].value,
        PropertyValue::FontFeatureSettings(FontFeatureSettings::Normal)
    );
}

#[test]
fn font_shorthand_omitted_components_expand_to_initial_values() {
    // CSS Fonts 4 §2.1 fills omitted grammar components and resets modeled
    // reset-only subproperties to their initial values.
    let decls = parse_block("font: 12px serif;");
    assert_eq!(decls.len(), 16);
    assert_eq!(decls[0].value, PropertyValue::FontStyle(FontStyle::Normal));
    assert_eq!(
        decls[1].value,
        PropertyValue::FontVariantCaps(FontVariantCaps::Normal)
    );
    assert_eq!(
        decls[2].value,
        PropertyValue::FontWeight(FontWeightValue::Absolute(400.0))
    );
    assert_eq!(
        decls[4].value,
        PropertyValue::LineHeight(LineHeight::Normal)
    );
    assert_eq!(
        decls[6].value,
        PropertyValue::FontKerning(FontKerning::Auto)
    );
    assert_eq!(
        decls[7].value,
        PropertyValue::FontLanguageOverride(FontLanguageOverride::Normal)
    );
    assert_eq!(
        decls[8].value,
        PropertyValue::FontOpticalSizing(FontOpticalSizing::Auto)
    );
    assert_eq!(
        decls[9].value,
        PropertyValue::FontVariantEastAsian(FontVariantEastAsian::initial())
    );
    assert_eq!(
        decls[10].value,
        PropertyValue::FontVariantEmoji(FontVariantEmoji::Normal)
    );
    assert_eq!(
        decls[11].value,
        PropertyValue::FontVariantLigatures(FontVariantLigatures::Normal)
    );
    assert_eq!(
        decls[12].value,
        PropertyValue::FontVariantNumeric(FontVariantNumeric::initial())
    );
    assert_eq!(
        decls[13].value,
        PropertyValue::FontVariantPosition(FontVariantPosition::Normal)
    );
    assert_eq!(
        decls[14].value,
        PropertyValue::FontVariationSettings(FontVariationSettings::Normal)
    );
}

#[test]
fn font_shorthand_relative_size_expands_to_font_size_relative_longhand() {
    // `larger` keeps its `FontSizeRelative` carrier (same `PropertyKey`
    // as `FontSize`, so cascade still sees a single slot).
    let decls = parse_block("font: italic larger serif;");
    assert_eq!(decls.len(), 16);
    assert_eq!(
        decls[3].value,
        PropertyValue::FontSizeRelative(RelativeFontSize::Larger)
    );
    assert_eq!(
        decls[3].value.key(),
        PropertyValue::FontSize(Length::Px(16.0)).key()
    );
}

#[test]
fn font_shorthand_important_flag_propagates_to_all_longhand() {
    // CSS Cascading 4 §3: a shorthand's `!important` is copied to all longhands,
    // as in the outline / overflow important extensions.
    let decls = parse_block("font: italic 12px serif !important;");
    assert_eq!(decls.len(), 16);
    for d in &decls {
        assert!(d.important, "important must propagate to every longhand");
    }
}

#[test]
fn font_shorthand_invalid_declaration_expands_to_nothing() {
    // System-font keywords / a missing family drop the whole declaration,
    // leaving nothing at the block exit.
    assert!(parse_block("font: menu;").is_empty());
    assert!(parse_block("font: italic 12px;").is_empty());
}

// ── overflow shorthand expansion (CSS Overflow 3 §3.1) ──

#[test]
fn overflow_shorthand_expands_into_two_longhand_declarations() {
    // `overflow: hidden scroll` → 2 longhand (x=hidden, y=scroll), spec
    // order per §3.1 "sets the specified values of overflow-x and
    // overflow-y in that order".
    let decls = parse_block("overflow: hidden scroll;");
    assert_eq!(decls.len(), 2, "shorthand must expand to 2 longhand decls");
    assert_eq!(
        decls[0].value,
        PropertyValue::OverflowX(OverflowValue::Hidden)
    );
    assert_eq!(
        decls[1].value,
        PropertyValue::OverflowY(OverflowValue::Scroll)
    );
}

#[test]
fn overflow_shorthand_one_value_expands_to_both_axes() {
    // §3.1 "If the second value is omitted, it is copied from the first."
    let decls = parse_block("overflow: auto;");
    assert_eq!(decls.len(), 2);
    assert_eq!(
        decls[0].value,
        PropertyValue::OverflowX(OverflowValue::Auto)
    );
    assert_eq!(
        decls[1].value,
        PropertyValue::OverflowY(OverflowValue::Auto)
    );
}

#[test]
fn overflow_shorthand_important_flag_propagates_to_all_longhand() {
    // CSS Cascading L4 §3: a shorthand's `!important` is copied to all longhands,
    // as in the margin / padding / border important extensions.
    let decls = parse_block("overflow: hidden !important;");
    assert_eq!(decls.len(), 2);
    for d in &decls {
        assert!(d.important, "important must propagate to every longhand");
    }
}

#[test]
fn overflow_shorthand_three_values_declaration_dropped() {
    // property.rs
    // `overflow_shorthand_leaves_extra_values_for_caller_exhausted_check`
    // End-to-end check: `parse_overflow_shorthand` consumes two values. The third,
    // unconsumed token causes `expect_exhausted` to drop the entire declaration
    // (zero declarations, as in the margin 5-value sibling test).
    let decls = parse_block("overflow: hidden scroll auto;");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        decls.is_empty(),
        "3-value shorthand must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn overflow_longhand_declaration_not_expanded() {
    // A longhand passes through the `expand_shorthand_into` match arm unchanged
    // (one declaration). Negative test checking the scope of shorthand-only expansion,
    // as in the margin / padding / border sibling tests.
    let decls = parse_block("overflow-x: hidden;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::OverflowX(OverflowValue::Hidden)
    );
}

// ── text-decoration shorthand expansion (CSS Text Decoration Module
// Level 3 §2.4) ──
//
// `parse_declaration_block` expands `text-decoration` into three longhands
// (`TextDecorationLine` / `TextDecorationStyle` / `TextDecorationColor`)
// following the margin / padding / border / overflow precedent.
// parse-time expansion model.

#[test]
fn text_decoration_shorthand_expands_into_four_longhand_declarations() {
    use crate::property::{
        TextDecorationColor, TextDecorationLine, TextDecorationStyle, TextDecorationThickness,
    };

    // `text-decoration: underline` produces four longhands; omitted components
    // (thickness/style/color) get spec initial values (see "Initial value fill" in
    // `parse_text_decoration_shorthand` in property.rs).
    let decls = parse_block("text-decoration: underline;");
    assert_eq!(decls.len(), 4, "shorthand must expand to 4 longhand decls");
    assert_eq!(
        decls[0].value,
        PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE)
    );
    assert_eq!(
        decls[1].value,
        PropertyValue::TextDecorationThickness(TextDecorationThickness::Auto)
    );
    assert_eq!(
        decls[2].value,
        PropertyValue::TextDecorationStyle(TextDecorationStyle::Solid)
    );
    assert_eq!(
        decls[3].value,
        PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor)
    );
}

#[test]
fn text_decoration_shorthand_important_flag_propagates_to_all_longhand() {
    // CSS Cascading L4 §3: a shorthand's `!important` is copied to all longhands,
    // as in the margin / padding / border / overflow important extensions.
    // pattern).
    let decls = parse_block("text-decoration: underline !important;");
    assert_eq!(decls.len(), 4);
    for d in &decls {
        assert!(d.important, "important must propagate to every longhand");
    }
}

#[test]
fn text_decoration_shorthand_two_style_components_declaration_dropped() {
    // property.rs
    // `text_decoration_shorthand_two_style_components_leaves_leftover_for_caller_exhausted_check`
    // End-to-end check: a second style keyword is unconsumed; `expect_exhausted`
    // detects it and drops the entire declaration (zero declarations).
    let decls = parse_block("text-decoration: solid wavy;");
    assert!(
        decls.is_empty(),
        "2 style components must be dropped by expect_exhausted, got {decls:?}"
    );
}

#[test]
fn text_decoration_longhand_declarations_not_expanded() {
    use crate::property::{TextDecorationColor, TextDecorationLine, TextDecorationStyle};

    // A longhand passes through the `expand_shorthand_into` match arm unchanged
    // (one declaration). Negative test checking shorthand-only expansion scope,
    // as in the margin / padding / border / overflow sibling tests.
    let decls = parse_block("text-decoration-line: underline;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::TextDecorationLine(TextDecorationLine::UNDERLINE)
    );

    let decls = parse_block("text-decoration-style: wavy;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy)
    );

    let decls = parse_block("text-decoration-color: red;");
    assert_eq!(decls.len(), 1);
    assert_eq!(
        decls[0].value,
        PropertyValue::TextDecorationColor(TextDecorationColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }))
    );
}

#[test]
fn text_decoration_shorthand_always_overwrites_all_four_longhand() {
    // Shorthand-resets-omitted-longhands: per CSS Cascading L4 §3's
    // "exactly as if expanded in place", `text-decoration: underline`
    // (style/color omitted) still emits a `TextDecorationStyle::Solid` /
    // `TextDecorationColor::CurrentColor` declaration alongside the
    // line one — it does not "leave the other two alone". This is what
    // lets a later bare `text-decoration: underline` reset an earlier
    // `text-decoration-style: wavy` back to `solid` through ordinary
    // cascade order-of-appearance (see
    // `crate::cascade::tests::text_decoration_shorthand_resets_earlier_longhand_declarations` // doc-pointer-lint:ignore: opt-out-3, #[test]-item body (test doc) — invisible to rustdoc, confirmed by deliberately breaking the link
    // for the end-to-end cascade pin).
    use crate::property::{TextDecorationColor, TextDecorationStyle};

    let decls = parse_block("text-decoration-style: wavy; text-decoration: underline;");
    assert_eq!(decls.len(), 5, "1 longhand + 4 expanded, in source order");
    assert_eq!(
        decls[0].value,
        PropertyValue::TextDecorationStyle(TextDecorationStyle::Wavy)
    );
    // The 2nd declaration is the shorthand's TextDecorationLine, the 3rd
    // its thickness — the 4th is the discriminating one: the shorthand's
    // own Solid, appearing *after* the earlier explicit Wavy.
    assert_eq!(
        decls[3].value,
        PropertyValue::TextDecorationStyle(TextDecorationStyle::Solid)
    );
    assert_eq!(
        decls[4].value,
        PropertyValue::TextDecorationColor(TextDecorationColor::CurrentColor)
    );
}

#[test]
fn text_decoration_line_duplicate_and_none_combination_declarations_dropped() {
    // End-to-end check for the leftover-token cases property.rs's
    // `text_decoration_line_two_underlines_leaves_leftover_for_caller_exhausted_check`
    // and `text_decoration_line_none_combined_with_a_keyword_leaves_leftover`
    // exercise at the `parse_text_decoration_line` helper level: the
    // leftover token they leave unconsumed is caught here by
    // `expect_exhausted` (`rule.rs`'s `DeclParser`), dropping the whole
    // declaration (0 decl), same shape as
    // `rejects_trailing_garbage_after_value`.
    assert!(parse_block("text-decoration-line: underline underline;").is_empty());
    assert!(parse_block("text-decoration-line: none underline;").is_empty());
    assert!(parse_block("text-decoration-line: underline none;").is_empty());
}

#[test]
fn list_style_shorthand_expands_every_component_in_any_order() {
    use crate::property::{BackgroundImage, ListStylePosition, ListStyleType};
    for value in [
        "square inside url(marker.png)",
        "square url(marker.png) inside",
        "inside square url(marker.png)",
        "inside url(marker.png) square",
        "url(marker.png) square inside",
        "url(marker.png) inside square",
    ] {
        let decls = parse_block(&format!("list-style: {value} !important"));
        assert_eq!(decls.len(), 3, "{value}");
        assert!(decls.iter().all(|d| d.important));
        assert_eq!(
            decls[0].value(),
            &PropertyValue::ListStyleType(ListStyleType::Named("square".into()))
        );
        assert_eq!(
            decls[1].value(),
            &PropertyValue::ListStylePosition(ListStylePosition::Inside)
        );
        assert_eq!(
            decls[2].value(),
            &PropertyValue::ListStyleImage(BackgroundImage::Url("marker.png".into()))
        );
    }
}

#[test]
fn list_style_none_resolves_only_unspecified_components() {
    use crate::property::{BackgroundImage, ListStylePosition, ListStyleType};
    for (value, kind, image) in [
        ("none", ListStyleType::None, BackgroundImage::None),
        ("none none", ListStyleType::None, BackgroundImage::None),
        ("disc none", ListStyleType::Disc, BackgroundImage::None),
        (
            "none url(marker.png)",
            ListStyleType::None,
            BackgroundImage::Url("marker.png".into()),
        ),
        ("inside", ListStyleType::Disc, BackgroundImage::None),
    ] {
        let decls = parse_block(&format!("list-style: {value}"));
        assert_eq!(decls.len(), 3, "{value}");
        assert_eq!(decls[0].value(), &PropertyValue::ListStyleType(kind));
        assert_eq!(
            decls[1].value(),
            &PropertyValue::ListStylePosition(if value == "inside" {
                ListStylePosition::Inside
            } else {
                ListStylePosition::Outside
            })
        );
        assert_eq!(decls[2].value(), &PropertyValue::ListStyleImage(image));
    }
    for value in [
        "inside outside",
        "square circle",
        "none none none",
        "disc url(a) none",
        "disc none none",
        "none none url(a)",
        "inherit inside",
        "",
        "url(a) url(b)",
    ] {
        assert!(
            parse_block(&format!("list-style: {value}")).is_empty(),
            "{value}"
        );
    }
}

#[test]
fn opacity_leaves_importance_for_the_declaration_parser() {
    for source in [
        "opacity:.25!important",
        "opacity:25% !important",
        "opacity:inherit!important",
    ] {
        let declarations = parse_block(source);
        assert_eq!(declarations.len(), 1, "{source}");
        assert!(declarations[0].important, "{source}");
        assert_eq!(
            declarations[0].value.key(),
            crate::property::PropertyKey::Opacity
        );
    }
    for source in [
        "opacity:.25 extra!important",
        "opacity:.25!other",
        "opacity:.25!important extra",
    ] {
        assert!(parse_block(source).is_empty(), "{source}");
    }
}
