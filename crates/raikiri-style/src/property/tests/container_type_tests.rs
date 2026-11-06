use super::*;

#[test]
fn container_type_recognizes_its_three_size_keywords() {
    for keyword in ["normal", "size", "inline-size", "INLINE-SIZE"] {
        assert!(
            parse_entire(keyword, "container-type").is_some(),
            "{keyword}"
        );
    }
}

#[test]
fn container_type_keyword_values_and_serialization_agree() {
    for &value in ContainerType::ALL {
        let parsed = parse_entire(value.as_css_str(), "container-type").unwrap();
        assert_eq!(parsed, PropertyValue::ContainerType(value));
        assert_eq!(parsed.key(), PropertyKey::ContainerType);
        assert_eq!(
            serialize_value(&parsed).as_deref(),
            Some(value.as_css_str())
        );
    }
    assert!(is_supported_property_name("container-type"));
    assert!(!is_animatable_property_name("container-type"));
    assert_css_wide_keywords_deferred("container-type");
}

#[test]
fn container_type_rejects_unsupported_or_combined_values() {
    for source in [
        "",
        "size inline-size",
        "size size",
        "normal size",
        "scroll-state",
        "size scroll-state",
        "scroll-state inline-size",
        "size, normal",
        "none",
        "block-size",
    ] {
        assert!(parse_entire(source, "container-type").is_none(), "{source}");
    }
}

#[test]
fn container_type_defaulting_and_contain_inheritance_stay_independent() {
    use crate::{ruletree::build_rule_tree, test_dom::TestDoc};
    for (source, expected) in [
        ("", ContainerType::Normal),
        ("container-type:inherit", ContainerType::Size),
        ("container-type:initial", ContainerType::Normal),
        ("container-type:unset", ContainerType::Normal),
        (
            "container-type:inline-size;all:initial",
            ContainerType::Normal,
        ),
        (
            "container-type:inline-size;all:inherit",
            ContainerType::Size,
        ),
        (
            "--kind:inline-size;container-type:var(--kind)",
            ContainerType::InlineSize,
        ),
    ] {
        let mut doc = TestDoc::new();
        let parent = doc.push_element(0, "div", Some("container-type:size;contain:paint"));
        let child = doc.push_element(parent, "p", Some(&format!("contain:inherit;{source}")));
        let cascade = crate::cascade(&doc, &build_rule_tree(&doc)).unwrap();
        let style = &cascade.computed[child];
        assert_eq!(style.container_type, expected, "{source}");
        assert_eq!(cascade.computed[parent].contain, Contain::PAINT);
        if !source.contains("all:") {
            assert_eq!(style.contain, Contain::PAINT, "{source}");
        }
        assert_eq!(
            cascade.computed[parent].effective_containment(),
            Contain::PAINT.union(Contain::STYLE).union(Contain::SIZE)
        );
    }
}

#[test]
fn container_type_rollback_uses_ordinary_cascade_winners() {
    use crate::{
        ruletree::{Origin, RuleTree},
        test_dom::TestDoc,
    };
    for (css, expected) in [
        (
            "p{container-type:size} p{container-type:revert}",
            ContainerType::Normal,
        ),
        (
            "@layer low,high;@layer low{p{container-type:size}}@layer high{p{container-type:inline-size;container-type:revert-layer}}",
            ContainerType::Size,
        ),
        (
            "p{container-type:size!important}p{container-type:inline-size}",
            ContainerType::Size,
        ),
        (
            "p{container-type:size}p{container-type:var(--missing)}",
            ContainerType::Normal,
        ),
        (
            "p{container-type:inline-size}p{container-type:size scroll-state}",
            ContainerType::InlineSize,
        ),
        (
            "p{container-type:inline-size}p{container-type:size size}",
            ContainerType::InlineSize,
        ),
    ] {
        let mut doc = TestDoc::new();
        let child = doc.push_element(0, "p", None);
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(css, Origin::Author);
        let result = crate::cascade(&doc, &tree).unwrap();
        assert_eq!(result.computed[child].container_type, expected, "{css}");
    }
}
