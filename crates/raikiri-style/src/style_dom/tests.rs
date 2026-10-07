use super::StyleElement;

struct MinimalElement;

impl StyleElement for MinimalElement {
    fn tag_name(&self) -> &str {
        "div"
    }
}

#[test]
fn animation_style_defaults_to_absent() {
    assert_eq!(MinimalElement.animation_style_source(), None);
}

#[test]
fn hyperlink_default_preserves_empty_attribute_presence_and_namespace() {
    struct Anchor<'a> {
        tag: &'a str,
        namespace: Option<&'a str>,
        href: Option<&'a str>,
    }
    impl StyleElement for Anchor<'_> {
        fn tag_name(&self) -> &str {
            self.tag
        }
        fn namespace_uri(&self) -> Option<&str> {
            self.namespace
        }
        fn attr(&self, name: &str) -> Option<&str> {
            if name == "href" { self.href } else { None }
        }
    }
    for (tag, namespace, href, expected) in [
        ("a", None, Some(""), true),
        ("area", None, Some(""), true),
        ("a", None, None, false),
        ("a", Some("http://www.w3.org/2000/svg"), Some(""), true),
        (
            "A",
            Some("http://www.w3.org/2000/svg"),
            Some("#target"),
            false,
        ),
        (
            "area",
            Some("http://www.w3.org/2000/svg"),
            Some("#target"),
            false,
        ),
        ("a", Some("urn:custom"), Some("#target"), false),
    ] {
        assert_eq!(
            Anchor {
                tag,
                namespace,
                href
            }
            .is_link(),
            expected
        );
    }
    assert!(!MinimalElement.is_link());
}
