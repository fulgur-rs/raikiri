use super::*;

#[test]
fn column_rule_accepts_border_grammar_in_each_order() {
    for value in [
        "2px solid red",
        "red solid 2px",
        "solid",
        "currentcolor",
        "thick dashed",
        "0",
        "none",
        "hidden",
        "2em double rgb(0, 128, 0)",
    ] {
        assert!(
            parse_entire(value, "column-rule").is_some(),
            "valid rule {value}"
        );
    }
}

#[test]
fn column_rule_longhands_accept_width_style_and_color() {
    for (name, values) in [
        (
            "column-rule-width",
            &["0", "2px", "thin", "medium", "thick", "1em"][..],
        ),
        (
            "column-rule-style",
            &[
                "none", "hidden", "solid", "double", "dotted", "dashed", "ridge", "groove",
                "inset", "outset",
            ][..],
        ),
        (
            "column-rule-color",
            &[
                "red",
                "currentcolor",
                "transparent",
                "#abcdef",
                "rgb(0, 128, 0)",
            ][..],
        ),
    ] {
        for value in values {
            assert!(parse_entire(value, name).is_some(), "valid {name}: {value}");
        }
    }
}

#[test]
fn column_rule_rejects_percentages_negative_widths_and_duplicate_components() {
    for value in [
        "-1px solid red",
        "10% solid red",
        "2px 3px solid",
        "solid dashed",
        "red blue",
        "solid trailing",
        "inherit solid",
    ] {
        assert!(
            parse_entire(value, "column-rule").is_none(),
            "invalid rule {value}"
        );
    }
    for value in ["-1px", "10%", "auto", "thin thick"] {
        assert!(
            parse_entire(value, "column-rule-width").is_none(),
            "invalid width {value}"
        );
    }
}

#[test]
fn column_rule_preserves_css_wide_and_variable_values_until_cascade() {
    for name in [
        "column-rule",
        "column-rule-width",
        "column-rule-style",
        "column-rule-color",
    ] {
        for value in [
            "inherit",
            "initial",
            "unset",
            "revert",
            "revert-layer",
            "var(--rule)",
        ] {
            assert!(
                parse_entire(value, name).is_some(),
                "defaulting {name}: {value}"
            );
        }
    }
}
