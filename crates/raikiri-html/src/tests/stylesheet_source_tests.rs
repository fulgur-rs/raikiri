use super::*;

#[test]
fn author_projection_reads_cssom_sources_and_preserves_original_dom() {
    let options = ParseOptions::new();
    let mut doc = parse(
        &b"<!DOCTYPE html><head><style>p { color: red }</style></head><body><style></style>"[..],
        &options,
    )
    .unwrap();
    let styles: Vec<_> = (0..doc.dom.node_count())
        .filter(|&id| doc.dom.get_node(id).unwrap().tag_name() == Some("style"))
        .collect();
    doc.dom
        .set_stylesheet_source(styles[0], String::new())
        .unwrap();
    doc.dom
        .set_stylesheet_source(styles[1], "<![CDATA[p { color: blue }]]>".into())
        .unwrap();
    refresh_stylesheets(&mut doc, &options);
    assert_eq!(doc.stylesheet_sources.len(), 1);
    assert_eq!(
        doc.stylesheet_sources[0].parts[0].source,
        "<![CDATA[p { color: blue }]]>"
    );
    assert_eq!(
        doc.dom.element_text_content(styles[0]).as_deref(),
        Some("p { color: red }")
    );
}
