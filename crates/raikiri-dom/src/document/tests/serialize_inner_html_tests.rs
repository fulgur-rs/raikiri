use super::*;
use taffy::Style;

#[test]
fn serialize_inner_html_uses_live_attributes_escapes_content_and_omits_void_end_tags() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let paragraph = doc.append_element(Some(host), "p", Style::default(), None::<&str>);
    doc.set_element_attributes(
        paragraph,
        vec![(SmolStr::new("data-empty"), SmolStr::new(""))],
    );
    doc.set_element_attribute(paragraph, "title", "old")
        .unwrap();
    doc.set_element_attribute(paragraph, "title", "a&b\"c'd")
        .unwrap();
    doc.set_element_attribute(paragraph, "style", "color: red")
        .unwrap();
    doc.set_element_attributes(
        paragraph,
        vec![
            (SmolStr::new("data-empty"), SmolStr::new("")),
            (SmolStr::new("title"), SmolStr::new("a&b\"c'd")),
            (SmolStr::new("style"), SmolStr::new("legacy style entry")),
        ],
    );
    let text = doc.append_text(paragraph, "A < B & C > D");
    doc.append_comment(Some(paragraph), "note");
    doc.append_processing_instruction(Some(paragraph), "target", "data");
    doc.append_element(Some(paragraph), "br", Style::default(), None::<&str>);

    assert_eq!(
        doc.serialize_inner_html(host).unwrap(),
        "<p data-empty=\"\" title=\"a&amp;b&quot;c&#39;d\" style=\"color: red\">A &lt; B &amp; C &gt; D<!--note--><?target data?><br></p>",
        "serialize attributes, text, comments, processing instructions, and void elements in order" // cov:ignore: assert_eq! formats this diagnostic only on failure.
    );
    assert!(doc.serialize_inner_html(doc.nodes.len()).is_err());
    assert!(doc.serialize_inner_html(text).is_err());
    let serialized_document = doc.serialize_inner_html(doc.root_index()).unwrap();
    assert!(serialized_document.starts_with("<div>"));
    assert!(serialized_document.ends_with("</div>"));
}

#[test]
fn serializes_raw_text_and_rejects_invalid_attribute_names() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let script = doc.append_element(Some(host), "script", Style::default(), None::<&str>);
    doc.append_text(script, "if (a < b && c > d) {}");

    assert_eq!(
        doc.serialize_inner_html(script).unwrap(),
        "if (a < b && c > d) {}"
    );
    assert_eq!(
        doc.serialize_inner_html(host).unwrap(),
        "<script>if (a < b && c > d) {}</script>"
    );
    assert!(
        doc.set_element_attribute(host, "x\" onmouseover=\"bad", "1")
            .is_err()
    );
    assert_eq!(
        doc.serialize_inner_html(host).unwrap(),
        "<script>if (a < b && c > d) {}</script>"
    );
    let malformed = doc.append_element(Some(host), "span", Style::default(), None::<&str>);
    doc.set_element_attributes(
        malformed,
        vec![(SmolStr::new("bad name"), SmolStr::new("value"))],
    );
    assert!(
        doc.serialize_inner_html(host)
            .unwrap_err()
            .contains("invalid attribute name")
    );
}

#[test]
fn attribute_names_follow_html_and_foreign_content_case_rules() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    doc.set_element_attribute(html, "DATA-Key", "one").unwrap();
    doc.set_element_attribute(html, "data-key", "two").unwrap();
    assert_eq!(doc.element_attribute(html, "DATA-KEY"), Some("two"));

    let svg = doc.append_element(Some(0), "svg", Style::default(), None::<&str>);
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attribute(svg, "viewBox", "0 0 10 10")
        .unwrap();
    doc.set_element_attribute(svg, "viewbox", "lowercase")
        .unwrap();
    assert_eq!(doc.element_attribute(svg, "viewBox"), Some("0 0 10 10"));
    assert_eq!(doc.element_attribute(svg, "viewbox"), Some("lowercase"));
    assert_eq!(doc.element_attribute(svg, "VIEWBOX"), None);
}

#[test]
fn replace_children_from_targets_template_contents() {
    let mut target = Document::new();
    let template = target.append_element(Some(0), "template", Style::default(), None::<&str>);
    let template_contents = target.allocate_template_fragment_root(template);
    target.append_text(template_contents, "old");

    let mut source = Document::new();
    let span = source.append_element(Some(0), "span", Style::default(), None::<&str>);
    source.append_text(span, "new");

    target.replace_children_from(template, &source, source.root_index());
    assert!(target.nodes[template].children.is_empty());
    assert_eq!(target.nodes[template_contents].children.len(), 1);
    assert_eq!(
        target.serialize_inner_html(template).unwrap(),
        "<span>new</span>"
    );
}

#[test]
fn serialize_inner_html_reads_template_contents_fragment() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let template = doc.append_element(Some(host), "template", Style::default(), None::<&str>);
    let contents = doc.allocate_template_fragment_root(template);
    doc.append_text(contents, "<template text>");

    assert_eq!(
        doc.serialize_inner_html(template).unwrap(),
        "&lt;template text&gt;"
    );
    assert_eq!(
        doc.serialize_inner_html(host).unwrap(),
        "<template>&lt;template text&gt;</template>"
    );
    assert_eq!(
        doc.serialize_inner_html(contents).unwrap(),
        "&lt;template text&gt;"
    );
}

#[test]
fn serialize_inner_html_reports_malformed_template_fragment_links() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let parent_template = doc.append_element(Some(0), "template", Style::default(), None::<&str>);
    let nested_template =
        doc.append_element(Some(host), "template", Style::default(), None::<&str>);
    let document_root = doc.root_index();
    let wrong_kind = doc.append_text(document_root, "not a fragment");

    doc.nodes[parent_template]
        .data
        .as_element_mut()
        .unwrap()
        .template_contents = Some(wrong_kind);
    assert!(
        doc.serialize_inner_html(parent_template)
            .unwrap_err()
            .contains("not a fragment")
    );
    doc.nodes[parent_template]
        .data
        .as_element_mut()
        .unwrap()
        .template_contents = Some(usize::MAX);
    assert!(
        doc.serialize_inner_html(parent_template)
            .unwrap_err()
            .contains("out of range")
    );

    doc.nodes[nested_template]
        .data
        .as_element_mut()
        .unwrap()
        .template_contents = Some(usize::MAX);
    assert!(
        doc.serialize_inner_html(host)
            .unwrap_err()
            .contains("out of range")
    );
    doc.nodes[nested_template]
        .data
        .as_element_mut()
        .unwrap()
        .template_contents = Some(wrong_kind);
    assert!(
        doc.serialize_inner_html(host)
            .unwrap_err()
            .contains("not a fragment")
    );
}

#[test]
fn serialize_inner_html_flattens_fragment_children_and_rejects_document_children() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let fragment = doc.nodes.len();
    doc.nodes.push(Node::new_document_fragment());
    doc.nodes[host].children.push(fragment);
    doc.append_text(fragment, "fragment child");
    assert_eq!(doc.serialize_inner_html(host).unwrap(), "fragment child");

    let document_root = doc.root_index();
    doc.nodes[host].children.push(document_root);
    assert!(
        doc.serialize_inner_html(host)
            .unwrap_err()
            .contains("Document node")
    );
}
