use super::*;
use raikiri_dom::Document;
use raikiri_style::property::OverflowValue;

fn svg_document() -> (Document, usize, usize) {
    let mut document = Document::new();
    let root = document.append_element(Some(0), "svg", None::<&str>);
    let nested = document.append_element(Some(root), "svg", None::<&str>);
    for node in [root, nested] {
        document.set_element_namespace(node, Some("http://www.w3.org/2000/svg".into()));
    }
    document.mark_in_document_flags();
    (document, root, nested)
}

#[test]
fn svg_overflow_default_excludes_the_document_element() {
    let (document, root, nested) = svg_document();
    let tree = prepared_rule_tree().expect("the bundled UA remains preparable");
    let cascade = raikiri_style::cascade(&document, &tree).unwrap();
    for value in [
        cascade.computed[root].overflow.x,
        cascade.computed[root].overflow.y,
    ] {
        assert_eq!(value, OverflowValue::Visible);
    }
    for value in [
        cascade.computed[nested].overflow.x,
        cascade.computed[nested].overflow.y,
    ] {
        assert_eq!(value, OverflowValue::Hidden);
    }
}

#[test]
fn author_stylesheet_can_override_the_svg_overflow_default() {
    let (mut document, root, nested) = svg_document();
    document
        .set_element_attribute(nested, "overflow", "hidden")
        .unwrap();
    let mut tree = prepared_rule_tree().expect("the bundled UA remains preparable");
    tree.add_stylesheet("svg { overflow: visible }", raikiri_style::Origin::Author);
    let cascade = raikiri_style::cascade(&document, &tree).unwrap();
    for node in [root, nested] {
        assert_eq!(cascade.computed[node].overflow.x, OverflowValue::Visible);
        assert_eq!(cascade.computed[node].overflow.y, OverflowValue::Visible);
    }
}

#[test]
fn svg_overflow_presentation_attribute_can_override_the_default() {
    for (attribute, expected) in [
        ("visible", OverflowValue::Visible),
        ("hidden", OverflowValue::Hidden),
        ("clip", OverflowValue::Clip),
        ("auto", OverflowValue::Auto),
        ("invalid", OverflowValue::Hidden),
        ("inherit", OverflowValue::Visible),
    ] {
        let (mut document, _, nested) = svg_document();
        document
            .set_element_attribute(nested, "overflow", attribute)
            .unwrap();
        let tree = prepared_rule_tree().unwrap();
        let cascade = raikiri_style::cascade(&document, &tree).unwrap();
        assert_eq!(cascade.computed[nested].overflow.x, expected);
        assert_eq!(cascade.computed[nested].overflow.y, expected);
    }
}

#[test]
fn svg_overflow_hint_axes_yield_independently_to_author_longhands() {
    let (mut document, _, nested) = svg_document();
    document
        .set_element_attribute(nested, "overflow", "hidden")
        .unwrap();
    let mut tree = prepared_rule_tree().unwrap();
    tree.add_stylesheet("svg { overflow-x: auto }", raikiri_style::Origin::Author);
    let cascade = raikiri_style::cascade(&document, &tree).unwrap();
    assert_eq!(cascade.computed[nested].overflow.x, OverflowValue::Auto);
    assert_eq!(cascade.computed[nested].overflow.y, OverflowValue::Hidden);
}
