use super::*;
use crate::{DisplayValue, SelectorQuery, StyleNodeId};

const HTML_NS: &str = "http://www.w3.org/1999/xhtml";
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const MATH_NS: &str = "http://www.w3.org/1998/Math/MathML";

fn fixture() -> (TestDoc, [usize; 5]) {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "rect", None);
    let explicit_html = doc.push_element_with_namespace(0, "rect", HTML_NS, &[]);
    let svg = doc.push_element_with_namespace(0, "rect", SVG_NS, &[]);
    let math = doc.push_element_with_namespace(0, "rect", MATH_NS, &[]);
    let no_namespace = doc.push_element_with_namespace(0, "rect", "", &[]);
    let ids = [html, explicit_html, svg, math, no_namespace];
    for id in ids {
        doc.set_attr(id, "class", "target");
        doc.set_attr(id, "data-target", "yes");
    }
    (doc, ids)
}

fn assert_namespace_cascade(css: &str, expected: [bool; 5]) {
    let (doc, ids) = fixture();
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(css, Origin::Author);
    assert_eq!(tree.style_rules().len(), 1, "rule must parse: {css}");
    let result = cascade(&doc, &tree).unwrap();
    for (id, matches) in ids.into_iter().zip(expected) {
        assert_eq!(
            result.computed[id].display,
            if matches {
                DisplayValue::Block
            } else {
                DisplayValue::Inline
            },
            "namespace {:?}, stylesheet {css}",
            doc.nodes[id].namespace,
        );
    }
}

#[test]
fn prefixed_namespace_selectors_match_uri_and_reject_other_namespaces() {
    for selector in ["svg|rect", "svg|*", ":is(svg|rect)", ":where(svg|rect)"] {
        assert_namespace_cascade(
            &format!("@namespace svg '{SVG_NS}'; {selector} {{display:block}}"),
            [false, false, true, false, false],
        );
    }
    assert_namespace_cascade(
        &format!("@namespace alias '{SVG_NS}'; alias|rect {{display:block}}"),
        [false, false, true, false, false],
    );
    assert_namespace_cascade(
        "@namespace svg 'HTTP://www.w3.org/2000/svg'; svg|rect {display:block}",
        [false; 5],
    );
    assert_namespace_cascade(
        &format!("@namespace svg '{SVG_NS}'; :not(svg|rect) {{display:block}}"),
        [true, true, false, true, true],
    );
    assert_namespace_cascade(
        &format!("@namespace html '{HTML_NS}'; html|rect {{display:block}}"),
        [true, true, false, false, false],
    );
}

#[test]
fn default_namespace_restricts_type_universal_and_implicit_universal_selectors() {
    for selector in ["rect", "*", ".target", "[data-target]"] {
        assert_namespace_cascade(
            &format!("@namespace '{SVG_NS}'; {selector} {{display:block}}"),
            [false, false, true, false, false],
        );
    }
    assert_namespace_cascade(
        &format!("@namespace '{HTML_NS}'; rect {{display:block}}"),
        [true, true, false, false, false],
    );
    assert_namespace_cascade(
        "@namespace ''; rect {display:block}",
        [false, false, false, false, true],
    );
}

#[test]
fn no_namespace_selectors_exclude_html_and_foreign_elements() {
    for selector in ["|rect", "|*"] {
        assert_namespace_cascade(
            &format!("{selector} {{display:block}}"),
            [false, false, false, false, true],
        );
    }
    assert_namespace_cascade(
        "@namespace empty ''; empty|rect {display:block}",
        [false, false, false, false, true],
    );
}

#[test]
fn any_namespace_selectors_override_default_namespace() {
    for selector in ["*|rect", "*|*"] {
        assert_namespace_cascade(
            &format!("@namespace '{SVG_NS}'; {selector} {{display:block}}"),
            [true; 5],
        );
    }
    for selector in ["rect", "*"] {
        assert_namespace_cascade(&format!("{selector} {{display:block}}"), [true; 5]);
    }
}

#[test]
fn query_no_namespace_and_any_namespace_share_cascade_semantics() {
    let (doc, ids) = fixture();
    for (selector, expected) in [
        ("|rect", [false, false, false, false, true]),
        ("*|rect", [true; 5]),
    ] {
        let query = SelectorQuery::parse(selector).unwrap();
        for (id, matches) in ids.into_iter().zip(expected) {
            assert_eq!(
                query.matches(&doc, StyleNodeId::new(id as u64), &[]),
                matches
            );
        }
    }
}
