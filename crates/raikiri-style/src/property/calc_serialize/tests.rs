use cssparser::ParserInput;

use super::*;

fn flat_expression(term: &str, operator: &str, count: usize) -> String {
    format!("calc({})", vec![term; count].join(operator))
}

fn balanced_expression(levels: usize) -> String {
    if levels == 0 {
        "1".to_owned()
    } else {
        let child = balanced_expression(levels - 1);
        format!("({child} + {child})")
    }
}

#[test]
fn calc_serialization_bounds_shared_work_across_balanced_subtrees() {
    let source = format!("calc({})", balanced_expression(12));
    assert!(parse(&source, CalcUnitKind::Number).is_none());
    let source = format!("calc({})", balanced_expression(10));
    let node = parse(&source, CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(1024)");
}

#[test]
fn calc_serialization_bounds_ast_free_wrappers_and_sign_nodes() {
    let deep = format!("{}1{}", "calc(".repeat(2048), ")".repeat(2048));
    assert!(parse(&deep, CalcUnitKind::Number).is_none());
    let deep = format!("{}1{}", "(".repeat(2048), ")".repeat(2048));
    assert!(parse(&deep, CalcUnitKind::Number).is_none());
    let deep = format!("{}1{}", "sign(".repeat(128), ")".repeat(128));
    assert!(parse(&deep, CalcUnitKind::Number).is_none());
    let bounded = format!("{}1{}", "sign(".repeat(127), ")".repeat(127));
    let node = parse(&bounded, CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(1)");
    let minimum = flat_expression("1", " + ", 32);
    let nested = format!("{}{}{}", "(".repeat(32), minimum, ")".repeat(32));
    let node = parse(&nested, CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(32)");
}

#[test]
fn calc_serialization_rejects_oversized_public_components_and_failed_tails() {
    let sum = flat_expression("1", " + ", 129);
    let unresolved = flat_expression("sign(1em - 10px)", " + ", 129);
    let angle = flat_expression("1deg", " + ", 129);
    for (function, component) in [
        ("lab", &sum),
        ("lch", &sum),
        ("oklab", &sum),
        ("oklch", &sum),
        ("LaB", &unresolved),
        (r"l\61 b", &sum),
    ] {
        for source in [
            format!("{function}({component} 0 0)"),
            format!("{function}(0 {component} 0)"),
            format!("{function}(0 0 0 / {component})"),
        ] {
            assert!(crate::property::serialize_color_value("color", &source).is_none());
            let border = format!("red {source}");
            assert!(crate::property::serialize_color_value("border-color", &border).is_none());
        }
    }
    let source = format!("lch(50 20 {angle})");
    assert!(crate::property::serialize_color_value("color", &source).is_none());
    let bounded = flat_expression("1", " + ", 128);
    let missing_rhs = bounded.replacen(')', " + )", 1);
    assert!(parse(&missing_rhs, CalcUnitKind::Number).is_none());
    let invalid_later = format!("lab({bounded} invalid 0)");
    assert!(crate::property::serialize_color_value("color", &invalid_later).is_none());
    let valid_later = format!("lab({bounded} 0 0)");
    assert_eq!(
        crate::property::serialize_color_value("color", &valid_later).as_deref(),
        Some("lab(calc(128) 0 0)")
    );
}

#[test]
fn calc_serialization_rejects_flat_sum_before_the_ast_becomes_too_deep() {
    let source = flat_expression("1", " + ", 129);
    assert!(parse(&source, CalcUnitKind::Number).is_none());
    let color = format!("lab({source} 0 0)");
    assert!(crate::property::serialize_color_value("color", &color).is_none());
}

#[test]
fn calc_serialization_rejects_flat_product_before_the_ast_becomes_too_deep() {
    let source = flat_expression("1", " * ", 129);
    assert!(parse(&source, CalcUnitKind::Number).is_none());
}

#[test]
fn calc_serialization_preserves_bounded_math_and_unit_fallbacks() {
    let sum = flat_expression("1", " + ", 128);
    let node = parse(&sum, CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(128)");
    let product = flat_expression("1", " * ", 128);
    let node = parse(&product, CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(1)");
    for (input, want) in [
        ("lab(calc(1 + 2) 0 0)", "lab(calc(3) 0 0)"),
        ("lch(50 20 calc(20deg * 2))", "lch(50 20 calc(40deg))"),
        ("lab(calc(50% + 10%) 0 0)", "lab(calc(60) 0 0)"),
        (
            "lab(calc(sign(1em - 10px) * 10) 0 0)",
            "lab(calc(10 * sign(1em - 10px)) 0 0)",
        ),
    ] {
        assert_eq!(
            crate::property::serialize_color_value("color", input).as_deref(),
            Some(want)
        );
    }
}

fn parse(source: &str, unit_kind: CalcUnitKind) -> Option<CalcNode> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_calc_or_plain(&mut parser, unit_kind)
}

#[test]
fn parses_a_bare_number() {
    assert!(matches!(
        parse("50", CalcUnitKind::Number),
        Some(CalcNode::Number(v)) if v == 50.0
    ));
}

#[test]
fn parses_a_bare_percentage() {
    assert!(matches!(
        parse("50%", CalcUnitKind::Percentage),
        Some(CalcNode::Percentage(v)) if v == 50.0
    ));
}

#[test]
fn parses_a_bare_angle_and_normalizes_units_to_degrees() {
    assert!(matches!(
        parse("380deg", CalcUnitKind::Angle),
        Some(CalcNode::Angle(v)) if (v - 380.0).abs() < 1e-9
    ));
    assert!(matches!(
        parse("1.28rad", CalcUnitKind::Angle),
        Some(CalcNode::Angle(v)) if (v - 73.3386).abs() < 1e-3
    ));
}

#[test]
fn parses_calc_with_a_simple_product() {
    let node = parse("calc(50 * 3)", CalcUnitKind::Number).unwrap();
    assert!(matches!(node, CalcNode::Product(..)));
}

#[test]
fn parses_calc_with_a_sign_function() {
    let node = parse("calc(sign(1em - 10px) * 10)", CalcUnitKind::Number).unwrap();
    let CalcNode::Product(left, right, true) = node else {
        panic!("expected a multiplication product");
    };
    assert!(matches!(*left, CalcNode::Sign(_)));
    assert!(matches!(*right, CalcNode::Number(v) if v == 10.0));
}

#[test]
fn parses_infinity_and_nan_keywords() {
    assert!(matches!(
        parse("calc(infinity)", CalcUnitKind::Number),
        Some(CalcNode::Infinity)
    ));
    assert!(matches!(
        parse("calc(-infinity)", CalcUnitKind::Number),
        Some(CalcNode::NegInfinity)
    ));
    assert!(matches!(
        parse("calc(NaN)", CalcUnitKind::Number),
        Some(CalcNode::Nan)
    ));
}

#[test]
fn returns_none_for_unsupported_math_functions() {
    assert!(parse("calc(min(1, 2))", CalcUnitKind::Number).is_none());
}

#[test]
fn folds_a_fully_constant_product() {
    let node = parse("calc(50 * 3)", CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(150)");
}

#[test]
fn folds_a_fully_constant_difference() {
    let node = parse("calc(0.5 - 1)", CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(-0.5)");
}

#[test]
fn reorders_a_sign_call_multiplied_by_a_literal() {
    let node = parse("calc(sign(1em - 10px) * 10)", CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(10 * sign(1em - 10px))");
}

#[test]
fn reorders_a_percentage_variant_the_same_way() {
    let node = parse("calc(sign(1em - 10px) * 10%)", CalcUnitKind::Percentage).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(10% * sign(1em - 10px))");
}

#[test]
fn preserves_addition_operand_order_around_an_unresolved_sign_product() {
    let node = parse(
        "calc(50% + (sign(1em - 10px) * 10%))",
        CalcUnitKind::Percentage,
    )
    .unwrap();
    assert_eq!(
        serialize_calc_node(&node),
        "calc(50% + (10% * sign(1em - 10px)))"
    );
}

#[test]
fn serializes_infinity_and_nan() {
    assert_eq!(serialize_calc_node(&CalcNode::Infinity), "calc(infinity)");
    assert_eq!(
        serialize_calc_node(&CalcNode::NegInfinity),
        "calc(-infinity)"
    );
    assert_eq!(serialize_calc_node(&CalcNode::Nan), "calc(NaN)");
}

#[test]
fn zero_divided_by_zero_folds_to_nan() {
    let node = parse("calc(0 / 0)", CalcUnitKind::Number).unwrap();
    assert_eq!(serialize_calc_node(&node), "calc(NaN)");
}
