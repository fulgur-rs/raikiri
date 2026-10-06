use super::*;

#[test]
fn not_accepts_newlines_and_comments() {
    for condition in ["not\n(display: invalid)", "not/**/(display: invalid)"] {
        assert!(supports_condition(condition), "{condition}");
    }
}

#[test]
fn not_keyword_is_case_insensitive() {
    assert!(supports_condition("NOT (display: invalid)"));
}

#[test]
fn and_keyword_is_case_insensitive() {
    assert!(supports_condition("(display: block) AND (color: red)"));
}

#[test]
fn or_keyword_is_case_insensitive() {
    assert!(supports_condition("(display: invalid) OR (color: red)"));
}

#[test]
fn nested_parentheses_preserve_condition_value() {
    assert!(supports_condition("((display: block))"));
    assert!(supports_condition("(((display: block)))"));
    assert!(!supports_condition("((display: invalid))"));
}

#[test]
fn leading_comments_do_not_hide_a_supported_declaration() {
    assert!(supports_condition("/* comment */ (display: block)"));
}

#[test]
fn important_does_not_change_declaration_support() {
    for condition in [
        "(color: red !important)",
        "(color: red !IMPORTANT)",
        "(color: red !/**/important)",
    ] {
        assert!(supports_condition(condition), "{condition}");
    }
    assert!(!supports_condition("(color: invalid !important)"));
}

#[test]
fn unparenthesized_operator_mixtures_are_invalid() {
    for condition in [
        "(color: red) or (display: block) and (color: blue)",
        "(color: red) and (display: block) or (color: blue)",
        "not (display: invalid) and (color: red)",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
    assert!(supports_condition(
        "(color: red) or ((display: block) and (color: blue))"
    ));
}

#[test]
fn declarations_require_parentheses() {
    for condition in ["display: block", "not display: invalid"] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn malformed_operands_do_not_become_true_through_negation() {
    for condition in [
        "not",
        "not (display: invalid) trailing",
        "(color: red) or",
        "(color: red) and",
        "not (display: invalid))",
        "(display: block) (color: red)",
        "not not (display: invalid)",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn general_enclosed_is_false_and_can_be_negated() {
    for condition in ["future(foo)", "(future syntax)", "()", "(color: invalid)"] {
        assert!(!supports_condition(condition), "{condition}");
        assert!(
            supports_condition(&format!("not {condition}")),
            "{condition}"
        );
    }
    assert!(!supports_condition("not(display: invalid)"));
    assert!(supports_condition("(color: red) or future(foo)"));
    assert!(!supports_condition("(color: red) and future(foo)"));
}

#[test]
fn tokenizer_errors_in_any_operand_invalidate_the_condition() {
    for condition in [
        "not (font-family: \"bad\nstring\")",
        "not future(\"bad\nstring\")",
        "(color: red) or future(url(bad url))",
        "not (color: url(bad url))",
        "not future([)])",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn declarations_accept_css_escapes_and_unicode_strings() {
    assert!(supports_condition("(\\64 isplay: block)"));
    assert!(supports_condition("\\6e ot (display: invalid)"));
    assert!(supports_condition("(font-family: \"日本語\")"));
    for condition in [
        "(color: red garbage)",
        "(color: red !invalid)",
        "(color: red !important garbage)",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn selector_function_reports_supported_complex_selectors() {
    for condition in [
        "selector(p)",
        "SELECTOR(.a > .b)",
        "selector(:is(.a, .b))",
        "selector(:has(> .a))",
    ] {
        assert!(supports_condition(condition), "{condition}");
    }
    for condition in [
        "selector()",
        "selector(.a, .b)",
        "selector(:unknown)",
        "selector(.a || .b)",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn selector_function_rejects_invalid_forgiving_branches_recursively() {
    for condition in [
        "selector(:is(.a, :unknown))",
        "selector(:where(.a, :unknown))",
        "selector(:not(:is(.a, :unknown)))",
        "selector(:has(> :is(.a, :unknown)))",
        "selector(:nth-child(2n of :where(.a, :unknown)))",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn accepted_conditions_expose_style_rules_and_keep_the_opaque_view() {
    for condition in [
        "not\n(display: invalid)",
        "NOT (display: invalid)",
        "/* c */ ((display: block))",
        "(color: red !important)",
        "selector(:is(p, div))",
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            &format!("@supports {condition} {{ p {{ color: red }} }}"),
            Origin::Author,
        );
        assert_eq!(tree.style_rules().len(), 1, "{condition}");
        assert_eq!(tree.opaque_at_rules().len(), 1, "{condition}");
    }
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        "@supports (color: red) or (display: block) and (color: blue) { p { color: red } }",
        Origin::Author,
    );
    assert!(tree.style_rules().is_empty());
    assert_eq!(tree.opaque_at_rules().len(), 1);
}

#[test]
fn important_is_separate_from_css_wide_and_calc_values() {
    for declaration in [
        "color: inherit",
        "font-size: inherit",
        "width: calc(1px + 2px)",
    ] {
        assert!(
            supports_condition(&format!("({declaration})")),
            "{declaration}"
        );
        assert!(
            supports_condition(&format!("({declaration} !important)")),
            "{declaration}"
        );
    }
}

#[test]
fn negated_forgiving_selectors_cannot_hide_tokenizer_errors() {
    for condition in [
        "not selector(:is(.a, [)]))",
        "not selector(:where(.a, [)]))",
        "(color: red) or selector(:is(.a, [)]))",
    ] {
        assert!(!supports_condition(condition), "{condition}");
    }
}

#[test]
fn deeply_nested_supports_conditions_are_bounded() {
    let nested = |depth: usize| format!("{}display: block{}", "(".repeat(depth), ")".repeat(depth));
    assert!(supports_condition(&nested(128)));
    assert!(!supports_condition(&nested(129)));
    assert!(!supports_condition(&nested(4000)));
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        &format!("@supports {} {{ p {{ color: red }} }}", nested(4000)),
        Origin::Author,
    );
    assert!(tree.style_rules().is_empty());
}
