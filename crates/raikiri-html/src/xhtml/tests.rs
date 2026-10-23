use crate::{ParseOptions, parse_xhtml as parse};

fn options() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

#[test]
fn self_closing_xhtml_elements_are_siblings() {
    let source = br#"<html xmlns="http://www.w3.org/1999/xhtml"><head/><body><div id="a"/><div id="b"/></body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    let by_id = |id| {
        (0..parsed.dom.node_count())
            .find(|&node| parsed.dom.element_attribute(node, "id") == Some(id))
            .unwrap()
    };
    let a = by_id("a");
    let b = by_id("b");
    assert_eq!(parsed.dom.parent_of(a), parsed.dom.parent_of(b));
    assert!(parsed.dom.get_node(a).unwrap().children.is_empty());
}

#[test]
fn malformed_xhtml_is_rejected() {
    assert!(parse(&b"<html><body><div></body></html>"[..], &options()).is_err());
}

#[test]
fn xml_text_namespaces_and_lexical_prefixes_are_preserved() {
    let source = br#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:a="urn:test" xmlns:b="urn:test"><head><style><![CDATA[div > span { color: green }]]></style></head><body><a:Box id="foreign" b:Name="a &amp; b"/><div id="text" style="color:green"><![CDATA[a < b]]>&amp;&#x41;</div></body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    let foreign = (0..parsed.dom.node_count())
        .find(|&id| parsed.dom.element_attribute(id, "id") == Some("foreign"))
        .unwrap();
    assert_eq!(
        parsed.dom.get_node(foreign).unwrap().tag_name(),
        Some("Box")
    );
    assert_eq!(parsed.dom.element_namespace_uri(foreign), Some("urn:test"));
    assert_eq!(parsed.dom.element_prefix(foreign), Some("a"));
    assert_eq!(
        parsed.dom.element_attribute_ns(foreign, "urn:test", "Name"),
        Some("a & b")
    );
    let text = (0..parsed.dom.node_count())
        .find(|&id| parsed.dom.element_attribute(id, "id") == Some("text"))
        .unwrap();
    assert_eq!(
        parsed.dom.element_text_content(text).as_deref(),
        Some("a < b&A")
    );
    assert!(
        parsed
            .dom
            .serialize_inner_html(parsed.dom.root_index())
            .unwrap()
            .contains("style=\"color:green\"")
    );
    assert_eq!(
        parsed.stylesheet_sources[0].parts[0].source,
        "div > span { color: green }"
    );
    assert_eq!(parsed.quirks_mode, raikiri_traits::QuirksMode::NoQuirks);
}

#[test]
fn xml_parser_preserves_comments_processing_instructions_and_no_namespace() {
    let parsed = parse(
        &b"<?test value?><root><!--comment--><child/></root>"[..],
        &options(),
    )
    .unwrap();
    let serialized = parsed
        .dom
        .serialize_inner_html(parsed.dom.root_index())
        .unwrap();
    assert!(serialized.contains("<!--comment-->"));
    assert!(serialized.contains("<?test value?>"));
    let root = (0..parsed.dom.node_count())
        .find(|&id| parsed.dom.get_node(id).unwrap().tag_name() == Some("root"))
        .unwrap();
    assert_eq!(parsed.dom.element_namespace_uri(root), Some(""));
}

#[test]
fn html_self_closing_elements_keep_html5_behavior() {
    let parsed = crate::parse(
        &b"<!doctype html><div id='a'/><div id='b'/>"[..],
        &options(),
    )
    .unwrap();
    let a = (0..parsed.dom.node_count())
        .find(|&id| parsed.dom.element_attribute(id, "id") == Some("a"))
        .unwrap();
    let b = (0..parsed.dom.node_count())
        .find(|&id| parsed.dom.element_attribute(id, "id") == Some("b"))
        .unwrap();
    assert_eq!(parsed.dom.parent_of(b), Some(a));
}

#[test]
fn xml_input_and_node_budgets_accept_the_boundary() {
    use super::{XhtmlParseError, parse_with_limits};
    use raikiri_traits::{LimitKind, ParseError, RenderLimits};
    let source = b"<html xmlns='http://www.w3.org/1999/xhtml'><body><div/></body></html>";
    let mut limits = RenderLimits::default();
    limits.max_input_bytes = Some(source.len() as u64);
    limits.max_dom_nodes = Some(4);
    assert!(parse_with_limits(&source[..], &options(), &limits).is_ok());
    limits.max_input_bytes = Some(source.len() as u64 - 1);
    assert!(matches!(
        parse_with_limits(&source[..], &options(), &limits),
        Err(XhtmlParseError::Input(ParseError::LimitExceeded {
            kind: LimitKind::InputBytes,
            ..
        }))
    ));
    limits.max_input_bytes = None;
    limits.max_dom_nodes = Some(3);
    assert!(matches!(
        parse_with_limits(&source[..], &options(), &limits),
        Err(XhtmlParseError::Input(ParseError::LimitExceeded {
            kind: LimitKind::DomNodes,
            limit: 3,
            actual: 4
        }))
    ));
}

#[test]
fn invalid_utf8_and_unresolved_external_entities_are_terminal() {
    assert!(matches!(
        parse(&b"<html>\xff</html>"[..], &options()),
        Err(super::XhtmlParseError::Input(
            raikiri_traits::ParseError::Encoding { .. }
        ))
    ));
    assert!(parse(&b"<!DOCTYPE html SYSTEM 'http://example.invalid/xhtml.dtd'><html>&external;</html>"[..], &options()).is_err());
    assert!(
        parse(
            &b"<!DOCTYPE html SYSTEM 'http://example.invalid/xhtml.dtd'><html/>"[..],
            &options()
        )
        .is_ok()
    );
}

#[test]
fn namespace_declarations_remain_attributes_without_copying_inherited_ones() {
    let source = br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:p="urn&#58;test"><head/><body><p:Box xmlns:p="urn:test"/><p:Child/><root xmlns=""/></body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    let find = |tag| {
        (0..parsed.dom.node_count())
            .find(|&id| parsed.dom.get_node(id).unwrap().tag_name() == Some(tag))
            .unwrap()
    };
    const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
    assert_eq!(
        parsed
            .dom
            .element_attribute_ns(find("html"), XMLNS, "xmlns"),
        Some("http://www.w3.org/1999/xhtml")
    );
    assert_eq!(
        parsed.dom.element_attribute_ns(find("html"), XMLNS, "p"),
        Some("urn:test")
    );
    assert_eq!(
        parsed.dom.element_attribute_ns(find("Box"), XMLNS, "p"),
        Some("urn:test")
    );
    assert_eq!(
        parsed.dom.element_attribute_ns(find("Child"), XMLNS, "p"),
        None
    );
    assert_eq!(
        parsed
            .dom
            .element_attribute_ns(find("root"), XMLNS, "xmlns"),
        Some("")
    );
}

#[test]
fn internal_dtd_is_rejected_before_any_entity_expansion() {
    let source = br#"<!DOCTYPE html [<!ENTITY e "expanded">]><html>&e;</html>"#;
    assert!(parse(&source[..], &options()).is_err());
}

#[test]
fn doctype_markers_inside_text_comments_and_literals_do_not_trigger_dtd_rejection() {
    let source = br#"<?test <!DOCTYPE html [?>
<!-- <!DOCTYPE html [ -->
<!DOCTYPE html SYSTEM "http://example.invalid/[unused].dtd">
<html value="&lt;!DOCTYPE html ["><![CDATA[<!DOCTYPE html [literal]]>literal</html>"#;
    assert!(parse(&source[..], &options()).is_ok());
}
