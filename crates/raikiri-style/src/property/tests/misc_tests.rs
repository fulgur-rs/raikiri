//! Tests for cross-cutting `parse_value` behavior: unknown names, deferred values and math.

use super::*;

#[test]
fn unknown_property_returns_none() {
    // `background-color` / `padding` / `margin` / `width` / `height` /
    // `float` が順次実装済 = ここから除外。
    // `cursor` (CSS Basic User Interface Module Level 3
    // <https://www.w3.org/TR/css-ui-3/#cursor>) は現時点で
    // parse_value dispatch に未登録 → fall-through で None が返る
    // canonical unknown-property canary。実装され次第、別の未実装
    // property 名へ再び移設すること。
    assert_eq!(parse("pointer", "cursor"), None);
}

#[test]
fn deferred_function_scanner_skips_literals_and_handles_bounds() {
    assert!(contains_deferred_function_in_source("foo(VAR(--x))"));
    assert!(!contains_deferred_function_in_source(
        r#""var(--x)" /* calc(1px) */"#
    ));
    assert!(!contains_deferred_function_in_source("#var(--x)"));
    assert!(!contains_deferred_function_in_source("@calc(1px)"));
    assert!(!contains_deferred_function_in_source("\"unterminated"));
    assert!(!contains_deferred_function_in_source("/* unterminated"));
    assert_eq!(skip_deferred_string(r#""a\"b""#, 0), Some(6));
    assert_eq!(skip_deferred_string("\"unterminated", 0), None);
    assert_eq!(skip_deferred_comment("/* comment */", 0), Some(13));
    assert_eq!(skip_deferred_comment("/* unterminated", 0), None);
    assert!(contains_deferred_function_in_source("[var(--x)]"));
    assert!(contains_function_in_source("[var(--x)]", "var"));
}

#[test]
fn deferred_value_capture_rejects_oversized_input() {
    let source = format!("calc(1px){}", "x".repeat(64 * 1024));
    assert!(parse(&source, "width").is_none());
    assert!(contains_deferred_function_in_source(&source));
    let mut parser_input = ParserInput::new(&source);
    let mut parser = Parser::new(&mut parser_input);
    assert!(contains_deferred_function(&mut parser));

    let oversized = "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES + 1);
    let mut oversized_input = ParserInput::new(&oversized);
    let mut oversized_parser = Parser::new(&mut oversized_input);
    assert!(contains_deferred_function(&mut oversized_parser));
}

#[test]
fn deferred_value_capture_rejects_oversized_trailing_comment() {
    let source = format!("var(--x)/*{}*/", "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES));
    assert!(parse(&source, "width").is_none());
}

#[test]
fn deferred_value_capture_rejects_oversized_comment_before_important() {
    let source = format!(
        "var(--x)/*{}*/!important",
        "x".repeat(MAX_SUBSTITUTED_VALUE_BYTES)
    );
    assert!(parse(&source, "width").is_none());
}

#[test]
fn deferred_value_capture_rejects_bad_url_before_important() {
    assert!(parse(r#"var(--x) url(foo"bar)!important"#, "width").is_none());
}

#[test]
fn deferred_value_capture_rejects_excessive_component_nesting() {
    let depth = 129;
    let source = format!("{}var(--x){}", "[".repeat(depth), "]".repeat(depth));
    assert!(parse(&source, "width").is_none());
}

#[test]
fn deferred_value_capture_does_not_nest_unquoted_url_contents() {
    let depth = 129;
    let source = format!(
        "var(--image) url(data:image/svg+xml,{}{}x{}{})",
        "[".repeat(depth),
        "{".repeat(depth),
        "}".repeat(depth),
        "]".repeat(depth),
    );
    assert!(parse(&source, "width").is_some());
}

#[test]
fn math_without_var_is_validated_during_declaration_parsing() {
    assert!(parse("calc(foo)", "width").is_none());
    assert!(parse("min(10px, 20px)", "width").is_some());
}

#[test]
fn math_dummy_selection_is_type_aware() {
    // `math_source_has_dimension_or_percentage`: dimensions and
    // percentages count even through nesting and across comma-separated
    // arguments (no early exit — `parse_nested_block` runs its closure
    // via `parse_entirely`).
    assert!(math_source_has_dimension_or_percentage("calc(10px)"));
    assert!(math_source_has_dimension_or_percentage("min(20px, 10px)"));
    assert!(math_source_has_dimension_or_percentage("calc((1px))"));
    assert!(math_source_has_dimension_or_percentage("calc(10% + 1)"));
    assert!(!math_source_has_dimension_or_percentage("calc(0)"));
    assert!(!math_source_has_dimension_or_percentage("calc(3 - 3)"));

    // Percentage-bearing math is invalid in box-shadow length slots, even
    // when the percentage is nested in a math function or block token.
    assert!(math_source_has_percentage("calc(10%)"));
    assert!(math_source_has_percentage("min(10px, 20%)"));
    assert!(!math_source_has_percentage("foo(10%)"));
    assert!(math_source_has_percentage("calc((10%))"));
    assert!(math_source_has_percentage("calc([10%])"));
    assert!(math_source_has_percentage("calc({10%})"));
    assert!(math_source_has_percentage("calc(10% + 1px)"));
    assert_eq!(parse_entire("calc(10% + 1px) 2px", "box-shadow"), None);
    assert!(matches!(
        parse_entire("calc(1px + 2px) 2px", "box-shadow"),
        Some(PropertyValue::BoxShadow(_))
    ));

    // `deferred_dummy_is_valid_for_property`: pure-number math validates
    // only `<number>` positions (WPT `flex: 1 2 calc(0)` invalid), while
    // dimension-carrying math keeps the `1px` behavior.
    assert!(deferred_dummy_is_valid_for_property(
        "calc(-1)",
        "flex-grow"
    ));
    assert!(!deferred_dummy_is_valid_for_property(
        "calc(0)",
        "flex-basis"
    ));
    assert!(deferred_dummy_is_valid_for_property(
        "calc(2em + 3ex)",
        "width"
    ));
}

#[test]
fn keyword_and_grid_shorthand_parsers_are_reachable_from_parse_value() {
    let accepted = [
        ("start", "text-align-all"),
        ("match-parent", "text-align-all"),
        ("all", "text-combine-upright"),
        ("none", "text-combine-upright"),
        ("upright", "text-orientation"),
        ("sideways", "text-orientation"),
        ("isolate", "unicode-bidi"),
        ("plaintext", "unicode-bidi"),
        ("1 / 2 / 3 / 4", "grid-area"),
        ("100px / 50px", "grid"),
    ];
    for (source, property) in accepted {
        assert!(parse(source, property).is_some(), "{property}: {source}");
    }
    for property in [
        "text-align-all",
        "text-combine-upright",
        "text-orientation",
        "unicode-bidi",
    ] {
        assert_eq!(parse("bogus", property), None, "{property}");
    }
}
