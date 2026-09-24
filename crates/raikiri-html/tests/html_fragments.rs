//! Tests for the context-sensitive HTML fragment parser.

use raikiri_html::{ParseOptions, parse_fragment};

const HTML_NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

fn parse(markup: &str, context: &str, namespace: &str) -> raikiri_html::UncascadedDocument {
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    parse_fragment(markup.as_bytes(), &options, context, namespace, true).unwrap()
}

#[test]
fn fragment_parser_uses_table_context_insertion_modes() {
    let document = parse("<tr><td>cell</td></tr>", "tbody", HTML_NAMESPACE);
    let root = document.dom.root_index();
    let children = &document.dom.get_node(root).unwrap().children;
    assert_eq!(children.len(), 1);
    let tr = children[0];
    assert_eq!(document.dom.get_node(tr).unwrap().tag_name(), Some("tr"));
    let td = document.dom.get_node(tr).unwrap().children[0];
    assert_eq!(document.dom.get_node(td).unwrap().tag_name(), Some("td"));
}

#[test]
fn fragment_parser_populates_template_content() {
    let document = parse("<span>inside</span>", "template", HTML_NAMESPACE);
    let root = document.dom.root_index();
    let children = &document.dom.get_node(root).unwrap().children;
    assert_eq!(children.len(), 1);
    assert_eq!(
        document.dom.get_node(children[0]).unwrap().tag_name(),
        Some("span")
    );
}

#[test]
fn fragment_parser_preserves_svg_namespace_and_attribute_case() {
    let document = parse(
        r#"<g viewBox="0 0 1 1"><circle></circle></g>"#,
        "svg",
        SVG_NAMESPACE,
    );
    let root = document.dom.root_index();
    let g = document.dom.get_node(root).unwrap().children[0];
    assert_eq!(document.dom.element_namespace_uri(g), Some(SVG_NAMESPACE));
    assert_eq!(
        document.dom.element_attribute(g, "viewBox"),
        Some("0 0 1 1")
    );
    assert_eq!(document.dom.element_attribute(g, "viewbox"), None);
    let circle = document.dom.get_node(g).unwrap().children[0];
    assert_eq!(
        document.dom.get_node(circle).unwrap().tag_name(),
        Some("circle")
    );
    assert_eq!(
        document.dom.element_namespace_uri(circle),
        Some(SVG_NAMESPACE)
    );
}
