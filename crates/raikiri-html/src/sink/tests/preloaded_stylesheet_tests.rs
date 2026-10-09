use super::*;

#[test]
fn preloaded_user_sheets_precede_extras_and_later_appended_user_sheets_follow() {
    let sink = RaikiriTreeSink::default();
    sink.document
        .borrow_mut()
        .add_stylesheet("p {display:none}", raikiri_traits::StylesheetKind::User);
    let mut doc = crate::parse_with_sink(
        &b"<p>x</p>"[..],
        sink,
        &crate::ParseOptions {
            extra_stylesheets: &["p {display:inline}"],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let index = (0..doc.dom.node_count())
        .find(|&index| doc.dom.get_node(index).and_then(|node| node.tag_name()) == Some("p"))
        .unwrap();
    assert_eq!(
        crate::build_cascaded(&doc)
            .expect("the cascade succeeds")
            .computed[index]
            .display,
        raikiri_style::DisplayValue::Inline
    );
    doc.dom
        .add_stylesheet("p {display:block}", raikiri_traits::StylesheetKind::User);
    assert_eq!(
        crate::build_cascaded(&doc)
            .expect("the cascade succeeds")
            .computed[index]
            .display,
        raikiri_style::DisplayValue::Block
    );
}
