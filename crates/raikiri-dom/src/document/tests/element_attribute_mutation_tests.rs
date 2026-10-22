use super::*;
use raikiri_traits::{Dom as _, Element as _, Node as _, NodeId};

fn element_attributes(doc: &Document, id: usize) -> Vec<(String, String)> {
    let NodeData::Element(element) = &doc.nodes[id].data else {
        panic!("expected element node"); // cov:ignore: test helper callers construct this node as an Element.
    };
    element
        .attributes
        .iter()
        .map(|attr| (attr.local.to_string(), attr.value.to_string()))
        .collect()
}

#[test]
fn xml_name_validation_accepts_the_non_ascii_name_ranges() {
    for ch in [
        'À', 'Ø', 'ø', 'Ͱ', 'Ϳ', '\u{200c}', '⁰', 'Ⰰ', '々', '豈', 'ﷰ', '𐀀',
    ] {
        assert!(is_xml_name_start(ch));
    }
    for ch in ['0', '-', '.', '·', '\u{0300}', '\u{203f}'] {
        assert!(is_xml_name_char(ch));
    }
    assert!(is_valid_xml_name("π\u{0300}:name"));
    assert!(!is_valid_xml_name("0name"));
}

#[test]
fn dom_name_rules_follow_the_validate_and_extract_definitions() {
    for name in ["a", "a@b", "x\u{e9}/", "Ab-1", "a:b", "a\u{1}"] {
        assert_eq!(
            is_valid_element_local_name(name),
            !name.ends_with('/'),
            "{name:?}"
        );
    }
    for name in [":a", "_a.b", "\u{e9}x", "\u{e9}\u{1F196}", "_-.:9"] {
        assert!(is_valid_element_local_name(name), "{name:?}");
    }
    for name in ["", "1a", "-a", "_a b", ":a@", "a b", "a\0", "a>", "\u{e9}="] {
        assert!(!is_valid_element_local_name(name), "{name:?}");
    }
    for name in ["1a", "data@x", "a:b", "\u{e9}", "@"] {
        assert!(is_valid_attribute_local_name(name), "{name:?}");
        assert!(is_valid_namespace_prefix(name), "{name:?}");
    }
    for name in [
        "", "a b", "a\tb", "a\nb", "a\u{c}b", "a\rb", "a\0", "a/b", "a>b",
    ] {
        assert!(!is_valid_attribute_local_name(name), "{name:?}");
        assert!(!is_valid_namespace_prefix(name), "{name:?}");
    }
    assert!(!is_valid_attribute_local_name("a=b"));
    assert!(is_valid_namespace_prefix("a=b"));

    let mut doc = Document::new();
    assert!(doc.create_detached_element("1a").is_err());
    let element = doc.create_detached_element("a\u{e9}").unwrap();
    doc.set_element_attribute(element, "data@x", "1").unwrap();
    doc.set_element_namespaced_attribute(element, "urn:x", None, "1a", "2")
        .unwrap();
    assert!(doc.set_element_attribute(element, "a=b", "3").is_err());
    assert_eq!(
        doc.remove_element_attribute(element, "data@x"),
        Ok(Some("1".into()))
    );
    assert!(doc.remove_element_attribute(element, "a b").is_err());
}

#[test]
#[should_panic(expected = "set_element_attribute called on non-Element")]
fn set_element_attribute_panics_for_a_non_element() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let _ = doc.set_element_attribute(root, "id", "not-an-element");
}

#[test]
#[should_panic(expected = "remove_element_attribute called on non-Element")]
fn remove_element_attribute_panics_for_a_non_element() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let _ = doc.remove_element_attribute(root, "id");
}

#[test]
fn remove_element_attribute_validates_names_and_preserves_foreign_case() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "div", None::<&str>);
    let svg = doc.append_element(Some(0), "svg", None::<&str>);
    doc.set_element_namespace(svg, Some(SmolStr::new("http://www.w3.org/2000/svg")));

    assert!(doc.remove_element_attribute(html, "bad name").is_err());
    doc.set_element_attribute(svg, "viewBox", "0 0 10 10")
        .unwrap();
    assert_eq!(doc.remove_element_attribute(svg, "viewbox").unwrap(), None);
    assert_eq!(
        doc.remove_element_attribute(svg, "viewBox").unwrap(),
        Some(SmolStr::new("0 0 10 10"))
    );
}

#[test]
fn element_attribute_accessors_return_none_for_non_elements_and_match_html_case() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "div", None::<&str>);
    let text = doc.append_text(html, "text");
    doc.set_element_attribute(html, "title", "heading").unwrap();
    assert_eq!(doc.element_namespace_uri(html), Some(XHTML_NAMESPACE_URI));
    assert_eq!(doc.element_namespace_uri(text), None);
    assert_eq!(doc.element_namespace_uri(usize::MAX), None);
    assert_eq!(doc.element_attribute(html, "TITLE"), Some("heading"));
    assert_eq!(doc.element_attribute(text, "title"), None);
    assert_eq!(doc.element_attribute(usize::MAX, "title"), None);
}

#[test]
fn set_and_remove_element_attribute_preserve_order_and_route_style_separately() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "div", None::<&str>);
    doc.set_element_attributes(
        id,
        vec![
            (SmolStr::new("id"), SmolStr::new("old")),
            (SmolStr::new("data-empty"), SmolStr::new("")),
            (SmolStr::new("id"), SmolStr::new("duplicate")),
        ],
    );

    doc.set_element_attribute(id, "id", "new").unwrap();
    doc.set_element_attribute(id, "title", "heading").unwrap();
    assert_eq!(
        element_attributes(&doc, id),
        vec![
            ("id".into(), "new".into()),
            ("data-empty".into(), "".into()),
            ("title".into(), "heading".into()),
        ],
        "updating an attribute keeps its position and a new attribute appends" // cov:ignore: assert_eq! formats this diagnostic only on failure.
    );

    doc.set_element_attribute(id, "style", "color: red")
        .unwrap();
    assert_eq!(
        element_attributes(&doc, id),
        vec![
            ("id".into(), "new".into()),
            ("data-empty".into(), "".into()),
            ("title".into(), "heading".into()),
        ],
        "style must not enter the null-namespace attribute list" // cov:ignore: assert_eq! formats this diagnostic only on failure.
    );
    let NodeData::Element(element) = &doc.nodes[id].data else {
        unreachable!(); // cov:ignore: the setup above creates this node as an Element.
    };
    assert_eq!(element.inline_style.as_deref(), Some("color: red"));

    let node = doc.node(NodeId::new(id as u64)).expect("element exists");
    let element = node.as_element().expect("node is an element");
    assert_eq!(element.attr("id"), Some("new"));
    assert_eq!(element.attr("style"), Some("color: red"));

    assert_eq!(
        doc.remove_element_attribute(id, "data-empty").unwrap(),
        Some(SmolStr::new(""))
    );
    assert_eq!(
        doc.remove_element_attribute(id, "style").unwrap(),
        Some(SmolStr::new("color: red"))
    );
    assert_eq!(doc.remove_element_attribute(id, "missing").unwrap(), None);
    assert_eq!(
        element_attributes(&doc, id),
        vec![
            ("id".into(), "new".into()),
            ("title".into(), "heading".into()),
        ]
    );
    let NodeData::Element(element) = &doc.nodes[id].data else {
        unreachable!(); // cov:ignore: the setup above creates this node as an Element.
    };
    assert_eq!(element.inline_style, None);
}
