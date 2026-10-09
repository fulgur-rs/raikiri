use super::*;

fn span(values: &ComputedValues) -> String {
    crate::ComputedProperty::from_name("column-span")
        .expect("computed column-span")
        .serialize(values, &mut |_| 10.0)
        .unwrap()
}

#[test]
fn column_span_cascade_keeps_none_initial_and_explicit_values_and_variables() {
    for (source, expected) in [
        ("", "none"),
        ("column-span:all", "all"),
        ("column-span:all;column-span:none", "none"),
        ("column-span:none;column-span:all", "all"),
        ("--span:all;column-span:var(--span)", "all"),
        ("column-span:all;column-span:bogus", "all"),
        ("column-span:all;column-span:initial", "none"),
        ("column-span:all;column-span:unset", "none"),
        ("column-span:all;column-span:revert", "none"),
        (
            "--span:initial;column-span:all;column-span:var(--span)",
            "none",
        ),
    ] {
        assert_eq!(
            span(&cascade_doc("", "div", Some(source))),
            expected,
            "{source}"
        );
    }
}

#[test]
fn column_span_does_not_inherit_without_explicit_inherit() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("column-span:all;--inherit:inherit"));
    let ordinary = doc.push_element(parent, "div", None);
    let inherited = doc.push_element(parent, "div", Some("column-span:inherit"));
    let unset = doc.push_element(parent, "div", Some("column-span:unset"));
    let variable = doc.push_element(parent, "div", Some("column-span:var(--inherit)"));
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(span(&result.computed[parent]), "all");
    assert_eq!(span(&result.computed[ordinary]), "none");
    assert_eq!(span(&result.computed[inherited]), "all");
    assert_eq!(span(&result.computed[unset]), "none");
    assert_eq!(span(&result.computed[variable]), "all");
}
