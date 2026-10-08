use super::super::*;
use taffy::Style;

#[test]
fn hyperlink_cascade_and_query_preserve_empty_html_and_legacy_svg_href() {
    for (tag, namespace, attribute_namespace, value, expected) in [
        ("a", None, None, Some(""), true),
        ("area", None, None, Some(""), true),
        ("a", None, None, None, false),
        ("div", None, None, Some("/target"), false),
        ("a", Some("urn:custom"), None, Some("/target"), false),
        (
            "a",
            Some("http://www.w3.org/2000/svg"),
            None,
            Some(""),
            true,
        ),
        (
            "a",
            Some("http://www.w3.org/2000/svg"),
            Some("http://www.w3.org/1999/xlink"),
            Some(""),
            true,
        ),
        (
            "area",
            Some("http://www.w3.org/2000/svg"),
            None,
            Some("/target"),
            false,
        ),
        (
            "A",
            Some("http://www.w3.org/2000/svg"),
            None,
            Some("/target"),
            false,
        ),
        (
            "a",
            None,
            Some("http://www.w3.org/1999/xlink"),
            Some("/target"),
            false,
        ),
    ] {
        let mut doc = Document::new();
        let id = doc.append_element(Some(0), tag, Style::default(), None::<&str>);
        if let Some(namespace) = namespace {
            doc.set_element_namespace_info(id, Some(namespace.into()), None);
        }
        if let Some(value) = value {
            if let Some(namespace) = attribute_namespace {
                doc.set_element_namespaced_attribute(id, namespace, None, "href", value)
                    .unwrap();
            } else {
                doc.set_element_attribute(id, "href", value).unwrap();
            }
        }
        for selector in [":link", ":any-link"] {
            let mut tree = raikiri_style::RuleTree::empty();
            tree.add_stylesheet(
                &format!("{selector} {{ font-weight:700 }}"),
                raikiri_style::Origin::Author,
            );
            let cascade = raikiri_style::cascade(&doc, &tree).unwrap();
            assert_eq!(
                cascade.computed[id].font_weight,
                if expected { 700.0 } else { 400.0 }
            );
            assert_eq!(
                raikiri_style::SelectorQuery::parse(selector)
                    .unwrap()
                    .matches(&doc, raikiri_style::StyleNodeId::new(id as u64), &[]),
                expected
            );
        }
    }
}
