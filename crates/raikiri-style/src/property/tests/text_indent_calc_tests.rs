//! Focused tests for the retained linear `text-indent` calc path.

use super::*;
use crate::resolve::ComputedTextIndent;

#[test]
fn mixed_percentage_and_px_calc_stays_mixed_and_keeps_flags() {
    let value = parse_entire("calc(50% + 60px) hanging each-line", "text-indent");
    assert_eq!(
        value,
        Some(PropertyValue::TextIndent(TextIndentValue {
            length: TextIndentLength::Calc(LengthPercentageCalc {
                percent: 50.0,
                px: 60.0,
                em: 0.0,
            }),
            hanging: true,
            each_line: true,
        }))
    );

    let Some(PropertyValue::TextIndent(value)) = value else {
        panic!("expected a parsed text-indent calc")
    };
    let TextIndentLength::Calc(calc) = value.length else {
        panic!("expected retained percentage and px coefficients")
    };
    assert_eq!(
        crate::resolve::resolve_text_indent_calc(calc, crate::resolve::ComputedLength(40.0)),
        ComputedTextIndent::Calc(CalcLengthPercentage {
            percent: 50.0,
            px: 60.0,
        }),
    );
}

#[test]
fn calc_subtracts_em_after_resolving_the_element_font_size() {
    let Some(PropertyValue::TextIndent(value)) = parse_entire("calc(10px - 0.5em)", "text-indent")
    else {
        panic!("expected a parsed text-indent calc")
    };
    let TextIndentLength::Calc(calc) = value.length else {
        panic!("expected retained px and em coefficients")
    };
    assert_eq!(
        calc,
        LengthPercentageCalc {
            percent: 0.0,
            px: 10.0,
            em: -0.5
        }
    );
    assert_eq!(
        crate::resolve::resolve_text_indent_calc(calc, crate::resolve::ComputedLength(40.0)),
        ComputedTextIndent::Px(-10.0),
    );
}

#[test]
fn calc_adds_em_after_resolving_the_element_font_size() {
    let Some(PropertyValue::TextIndent(value)) = parse_entire("calc(10px + 0.5em)", "text-indent")
    else {
        panic!("expected a parsed text-indent calc")
    };
    let TextIndentLength::Calc(calc) = value.length else {
        panic!("expected retained px and em coefficients")
    };
    assert_eq!(
        crate::resolve::resolve_text_indent_calc(calc, crate::resolve::ComputedLength(40.0)),
        ComputedTextIndent::Px(30.0),
    );
}
