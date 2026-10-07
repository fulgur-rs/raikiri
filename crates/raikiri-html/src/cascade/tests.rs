use super::*;

#[test]
fn stylesheet_kind_to_origin_matches_spec() {
    assert_eq!(
        stylesheet_kind_to_origin(StylesheetKind::UserAgent),
        Origin::UserAgent,
    );
    assert_eq!(
        stylesheet_kind_to_origin(StylesheetKind::User),
        Origin::User,
    );
    assert_eq!(
        stylesheet_kind_to_origin(StylesheetKind::Author),
        Origin::Author,
    );
}

fn parsed_extra_stylesheet(source: &str) -> UncascadedDocument {
    crate::parse(
        &b"<p>x</p>"[..],
        &crate::ParseOptions {
            extra_stylesheets: &[source],
            network: None,
            base_url: None,
        },
    )
    .unwrap()
}

fn paragraph_display(doc: &UncascadedDocument) -> raikiri_style::DisplayValue {
    let index = (0..doc.dom.node_count())
        .find(|&index| doc.dom.get_node(index).and_then(|node| node.tag_name()) == Some("p"))
        .unwrap();
    build_cascaded(doc).computed[index].display
}

#[test]
fn later_dom_user_stylesheets_follow_parsed_extra_stylesheets() {
    let mut doc = parsed_extra_stylesheet("p {display:block}");
    doc.dom
        .add_stylesheet("p {display:none}", StylesheetKind::User);
    assert_eq!(paragraph_display(&doc), raikiri_style::DisplayValue::None);
}

#[test]
fn parsed_extra_layer_order_precedes_later_dom_user_stylesheets() {
    let mut doc = parsed_extra_stylesheet("@layer a,b;");
    doc.dom.add_stylesheet(
        "@layer b,a; @layer a {p {display:none}} @layer b {p {display:inline}}",
        StylesheetKind::User,
    );
    assert_eq!(paragraph_display(&doc), raikiri_style::DisplayValue::Inline);
}
