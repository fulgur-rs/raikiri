use super::*;

#[test]
fn column_rule_shorthand_and_longhands_keep_independent_winners() {
    for source in [
        "column-rule:2px solid red;column-rule-color:blue",
        "--rule:2px solid red;column-rule:var(--rule);column-rule-color:blue",
        "column-rule:2px solid red;--shade:blue;column-rule-color:var(--shade)",
        "column-rule-width:9px;column-rule:2px solid blue",
    ] {
        let values = cascade_doc("", "div", Some(source));
        assert_eq!(values.column_rule.width().px(), 2.0);
        assert_eq!(values.column_rule.style(), BorderStyle::Solid);
        assert_eq!(values.column_rule.color, BorderColor::Resolved(BLUE));
    }
}

#[test]
fn column_rule_omitted_components_reset_and_none_hidden_zero_the_width() {
    let values = cascade_doc(
        "",
        "div",
        Some("column-rule:8px dashed red;column-rule:solid"),
    );
    assert_eq!(values.column_rule.width().px(), 3.0);
    assert_eq!(values.column_rule.style(), BorderStyle::Solid);
    assert_eq!(values.column_rule.color, BorderColor::CurrentColor);
    for source in [
        "column-rule:8px none red",
        "column-rule:8px hidden red",
        "column-rule:8px solid red;column-rule:initial",
        "column-rule:8px solid red;column-rule:unset",
    ] {
        let values = cascade_doc("", "div", Some(source));
        assert_eq!(values.column_rule.width().px(), 0.0, "{source}");
    }
}

#[test]
fn column_rules_are_non_inherited_but_explicit_inherit_uses_computed_components() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(
        0,
        "div",
        Some("font-size:20px;color:blue;column-rule:.1em dashed red"),
    );
    let ordinary = doc.push_element(parent, "div", None);
    let inherited = doc.push_element(parent, "div", Some("font-size:40px;column-rule:inherit"));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).unwrap();
    assert_eq!(result.computed[parent].column_rule.width().px(), 2.0);
    assert_eq!(
        result.computed[parent].column_rule.style(),
        BorderStyle::Dashed
    );
    assert_eq!(result.computed[ordinary].column_rule.width().px(), 0.0);
    let rule = result.computed[inherited].column_rule;
    assert_eq!(rule.width().px(), 2.0);
    assert_eq!(rule.style(), BorderStyle::Dashed);
    assert_eq!(rule.color, BorderColor::Resolved(RED));
}

#[test]
fn variable_invalid_rules_reset_only_the_winning_components() {
    let values = cascade_doc(
        "",
        "div",
        Some("column-rule:8px solid red;column-rule:var(--missing)"),
    );
    assert_eq!(values.column_rule.width().px(), 0.0);
    assert_eq!(values.column_rule.style(), BorderStyle::None);
    assert_eq!(values.column_rule.color, BorderColor::CurrentColor);
}

#[test]
fn invalid_variable_longhands_reset_their_component_without_erasing_others() {
    let values = cascade_doc(
        "",
        "div",
        Some("column-rule:2px solid red;column-rule-color:var(--missing);color:blue"),
    );
    assert_eq!(values.column_rule.width().px(), 2.0);
    assert_eq!(values.column_rule.style(), BorderStyle::Solid);
    assert_eq!(values.column_rule.color, BorderColor::CurrentColor);
}

#[test]
fn variable_css_wide_shorthands_use_the_parent_computed_rule() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("font-size:20px;column-rule:.1em dashed red"));
    let child = doc.push_element(
        parent,
        "div",
        Some("font-size:40px;--rule:inherit;column-rule:var(--rule)"),
    );
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(
        result.computed[child].column_rule,
        result.computed[parent].column_rule
    );
}
