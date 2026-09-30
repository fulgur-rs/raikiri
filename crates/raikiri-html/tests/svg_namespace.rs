//! Integration coverage for inline SVG namespace data.
//!
//! Parses a full HTML document with an inline `<svg>` subtree, checks that
//! the SVG namespace, `viewBox`, plain `href`, and namespaced `xlink:href`
//! survive parsing, and proves the neutral serialized source rebuilds an
//! equivalent SVG tree. The boundary stays renderer-neutral: only plain
//! strings and `raikiri_svg` (no usvg, Krilla, PageScene, or PDF types).

use raikiri_html::{ParseOptions, parse};
use raikiri_traits::{Dom, Element, Node};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

fn empty_options() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

fn find_all_by_tag(doc: &raikiri_dom::Document, tag: &str) -> Vec<raikiri_traits::NodeId> {
    fn recur(
        doc: &raikiri_dom::Document,
        id: raikiri_traits::NodeId,
        tag: &str,
        out: &mut Vec<raikiri_traits::NodeId>,
    ) {
        if let Some(node) = doc.node(id) {
            let is_match = node
                .as_element()
                .is_some_and(|element| element.tag_name() == tag);
            if is_match {
                out.push(id);
            }
            for child in doc.child_ids(id) {
                recur(doc, child, tag, out);
            }
        }
    }
    let mut out = Vec::new();
    let root = doc.root_id();
    recur(doc, root, tag, &mut out);
    out
}

#[test]
fn svg_namespace_viewbox_and_both_href_forms_survive_parsing() {
    let html = concat!(
        r##"<!doctype html><html><body>"##,
        r##"<svg viewBox="0 0 4 2">"##,
        r##"<defs><g id="shape"><rect width="4" height="2" fill="red"></rect></g></defs>"##,
        r##"<use href="#shape"></use>"##,
        r##"<use xlink:href="#shape"></use>"##,
        r##"</svg></body></html>"##,
    );
    let opts = empty_options();
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");

    let svg_ids = find_all_by_tag(&uncascaded.dom, "svg");
    assert_eq!(svg_ids.len(), 1);
    let svg_node = uncascaded.dom.node(svg_ids[0]).expect("svg node exists");
    let svg = svg_node.as_element().expect("svg is element");
    assert_eq!(svg.namespace_uri(), Some(SVG_NS));
    assert_eq!(svg.attr("viewBox"), Some("0 0 4 2"));
    // Foreign elements keep case; lowercased lookup finds nothing.
    assert_eq!(svg.attr("viewbox"), None);

    let use_ids = find_all_by_tag(&uncascaded.dom, "use");
    assert_eq!(use_ids.len(), 2);
    let mut saw_plain_href = false;
    let mut saw_xlink_href = false;
    for id in &use_ids {
        let node = uncascaded.dom.node(*id).expect("use node exists");
        let element = node.as_element().expect("use is element");
        assert_eq!(element.namespace_uri(), Some(SVG_NS));
        if element.attr("href") == Some("#shape") {
            saw_plain_href = true;
        }
        if element.attr_ns(XLINK_NS, "href") == Some("#shape") {
            saw_xlink_href = true;
        }
        // Direct document accessors agree with the trait view.
        let idx = id.0 as usize;
        if uncascaded.dom.element_attribute(idx, "href") == Some("#shape") {
            assert_eq!(element.attr("href"), Some("#shape"));
        }
        if uncascaded.dom.element_attribute_ns(idx, XLINK_NS, "href") == Some("#shape") {
            assert_eq!(element.attr_ns(XLINK_NS, "href"), Some("#shape"));
        }
    }
    assert!(saw_plain_href, "plain href must survive on one <use>");
    assert!(saw_xlink_href, "xlink:href must survive on one <use>");
}

#[test]
fn serialized_svg_source_preserves_namespaces_and_reparses() {
    let html = concat!(
        r##"<!doctype html><html><body>"##,
        r##"<svg viewBox="0 0 4 2">"##,
        r##"<defs><g id="shape"><rect width="4" height="2" fill="red"></rect></g></defs>"##,
        r##"<use href="#shape"></use>"##,
        r##"<use xlink:href="#shape"></use>"##,
        r##"</svg></body></html>"##,
    );
    let opts = empty_options();
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse ok");
    let svg_ids = find_all_by_tag(&uncascaded.dom, "svg");
    assert_eq!(svg_ids.len(), 1);
    let source = uncascaded
        .dom
        .serialize_svg_subtree(svg_ids[0].0 as usize)
        .expect("SVG subtree serializes")
        .expect("SVG root produces XML source");

    assert!(source.contains(r##"viewBox="0 0 4 2""##));
    assert!(source.contains(r##"href="#shape""##));
    assert!(source.contains(r##"xlink:href="#shape""##));
    assert!(source.contains(r##"xmlns:xlink="http://www.w3.org/1999/xlink""##));
    assert!(source.contains(r##"width="4""##));
    assert!(source.contains(r##"height="2""##));
    assert!(source.contains(r##"fill="red""##));

    // Neutral source rebuilds a parseable SVG tree without renderer types.
    raikiri_svg::SvgDocument::parse(source.as_bytes()).expect("SVG source reparses");
}
