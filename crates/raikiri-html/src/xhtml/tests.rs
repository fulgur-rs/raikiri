use std::io::Read;

use raikiri_traits::{
    FetchOutcome, LimitKind, NetworkError, NetworkProvider, ParseError, RenderLimits, Request,
    WarningKind,
};

use super::{MAX_ELEMENT_DEPTH, parse_with_limits};
use crate::{ParseOptions, UncascadedDocument, parse_xhtml as parse};

const XHTML: &str = "http://www.w3.org/1999/xhtml";

fn options() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

fn find(parsed: &UncascadedDocument, predicate: impl Fn(usize) -> bool) -> usize {
    (0..parsed.dom.node_count())
        .find(|&id| predicate(id))
        .expect("node in the document")
}

fn by_id(parsed: &UncascadedDocument, id: &str) -> usize {
    find(parsed, |node| {
        parsed.dom.element_attribute(node, "id") == Some(id)
    })
}

fn by_tag(parsed: &UncascadedDocument, tag: &str) -> usize {
    find(parsed, |node| {
        parsed.dom.get_node(node).unwrap().tag_name() == Some(tag)
    })
}

fn has_tag(parsed: &UncascadedDocument, tag: &str) -> bool {
    (0..parsed.dom.node_count())
        .any(|node| parsed.dom.get_node(node).unwrap().tag_name() == Some(tag))
}

fn serialized(parsed: &UncascadedDocument) -> String {
    parsed
        .dom
        .serialize_inner_html(parsed.dom.root_index())
        .unwrap()
}

fn messages(parsed: &UncascadedDocument) -> Vec<&str> {
    parsed
        .warnings
        .iter()
        .filter_map(|warning| match &warning.kind {
            WarningKind::HtmlParseError { message } => Some(message.as_str()),
            _ => None,
        })
        .collect()
}

fn limit_error(result: Result<UncascadedDocument, ParseError>) -> (LimitKind, u64, u64) {
    match result {
        Err(ParseError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => (kind, limit, actual),
        Err(other) => panic!("expected a limit error, got {other:?}"),
        Ok(_) => panic!("expected a limit error, but the parse succeeded"),
    }
}

#[test]
fn self_closing_xhtml_elements_are_siblings() {
    let source = br#"<html xmlns="http://www.w3.org/1999/xhtml"><head/><body><div id="a"/><div id="b"/></body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    let a = by_id(&parsed, "a");
    let b = by_id(&parsed, "b");
    assert_eq!(parsed.dom.parent_of(a), parsed.dom.parent_of(b));
    assert!(parsed.dom.get_node(a).unwrap().children.is_empty());
    assert!(messages(&parsed).is_empty());
}

#[test]
fn html_self_closing_elements_keep_html5_behavior() {
    let parsed = crate::parse(
        &b"<!doctype html><div id='a'/><div id='b'/>"[..],
        &options(),
    )
    .unwrap();
    assert_eq!(
        parsed.dom.parent_of(by_id(&parsed, "b")),
        Some(by_id(&parsed, "a"))
    );
}

#[test]
fn xml_text_namespaces_and_lexical_prefixes_are_preserved() {
    let source = br#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:a="urn:test" xmlns:b="urn:test"><head><style><![CDATA[div > span { color: green }]]></style></head><body><a:Box id="foreign" b:Name="a &amp; b"/><div id="text" style="color:green"><![CDATA[a < b]]>&amp;&#x41;</div></body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    let foreign = by_id(&parsed, "foreign");
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
    let text = by_id(&parsed, "text");
    assert_eq!(
        parsed.dom.element_text_content(text).as_deref(),
        Some("a < b&A")
    );
    assert!(serialized(&parsed).contains("style=\"color:green\""));
    assert_eq!(
        parsed.stylesheet_sources[0].parts[0].source,
        "div > span { color: green }"
    );
    assert_eq!(parsed.quirks_mode, raikiri_traits::QuirksMode::NoQuirks);
    assert!(messages(&parsed).is_empty());
}

#[test]
fn xml_parser_preserves_comments_processing_instructions_and_no_namespace() {
    let parsed = parse(
        &b"<?test value?><root><!--comment--><child/></root>"[..],
        &options(),
    )
    .unwrap();
    let serialized = serialized(&parsed);
    assert!(serialized.contains("<!--comment-->"));
    assert!(serialized.contains("<?test value?>"));
    assert_eq!(
        parsed.dom.element_namespace_uri(by_tag(&parsed, "root")),
        Some("")
    );
}

#[test]
fn the_xml_declaration_is_a_processing_instruction() {
    let parsed = parse(&b"<?xml version=\"1.0\"?><root/>"[..], &options()).unwrap();
    assert!(serialized(&parsed).starts_with("<?xml version=\"1.0\"?>"));
}

#[test]
fn namespace_declarations_scope_elements_and_do_not_become_attributes() {
    let source = br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:p="urn&#58;test"><head/><body><p:Box xmlns:p="urn:other"/><p:Child/><root xmlns=""/></body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
    for tag in ["html", "Box", "root"] {
        let node = by_tag(&parsed, tag);
        assert_eq!(parsed.dom.element_attribute_ns(node, XMLNS, "xmlns"), None);
        assert_eq!(parsed.dom.element_attribute_ns(node, XMLNS, "p"), None);
        assert_eq!(parsed.dom.element_attribute(node, "xmlns"), None);
    }
    let namespace = |tag| parsed.dom.element_namespace_uri(by_tag(&parsed, tag));
    assert_eq!(namespace("html"), Some(XHTML));
    assert_eq!(namespace("Box"), Some("urn:other"));
    assert_eq!(namespace("Child"), Some("urn:test"));
    assert_eq!(namespace("root"), Some(""));
}

#[test]
fn embedded_svg_keeps_its_namespace() {
    let source = br##"<html xmlns="http://www.w3.org/1999/xhtml"><body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" id="s"><use xlink:href="#a"/></svg></body></html>"##;
    let parsed = parse(&source[..], &options()).unwrap();
    assert_eq!(
        parsed.dom.element_namespace_uri(by_id(&parsed, "s")),
        Some("http://www.w3.org/2000/svg")
    );
    assert_eq!(
        parsed.dom.element_attribute_ns(
            by_tag(&parsed, "use"),
            "http://www.w3.org/1999/xlink",
            "href"
        ),
        Some("#a")
    );
}

#[test]
fn malformed_xhtml_is_repaired_and_reported() {
    let parsed = parse(&b"<html><body><div></body></html>"[..], &options()).unwrap();
    let div = by_tag(&parsed, "div");
    assert_eq!(parsed.dom.parent_of(div), Some(by_tag(&parsed, "body")));
    assert!(messages(&parsed).contains(&"Current node doesn't match tag"));
}

#[test]
fn xml_errors_are_recovered_the_way_xml5ever_does() {
    // An element left open at the end of the input is closed.
    let parsed = parse(&b"<html><body><p id='p'>text"[..], &options()).unwrap();
    assert_eq!(
        parsed
            .dom
            .element_text_content(by_id(&parsed, "p"))
            .as_deref(),
        Some("text")
    );

    // The first of two equal attributes wins.
    let parsed = parse(&b"<root a='1' a='2'/>"[..], &options()).unwrap();
    assert_eq!(
        parsed.dom.element_attribute(by_tag(&parsed, "root"), "a"),
        Some("1")
    );
    assert!(messages(&parsed).contains(&"Duplicate attribute"));

    // An undeclared prefix stays lexical, with no namespace.
    let parsed = parse(&b"<root><u:x id='x'/></root>"[..], &options()).unwrap();
    let x = by_id(&parsed, "x");
    assert_eq!(parsed.dom.element_prefix(x), Some("u"));
    assert_eq!(parsed.dom.element_namespace_uri(x), Some(""));
    assert!(messages(&parsed).contains(&"No appropriate namespace found"));

    // Text before the root and everything after it are dropped.
    let parsed = parse(&b"junk<html>a</html>trailing<x/>"[..], &options()).unwrap();
    assert!(!has_tag(&parsed, "x"));
    assert_eq!(
        parsed
            .dom
            .element_text_content(by_tag(&parsed, "html"))
            .as_deref(),
        Some("a")
    );
    assert!(!messages(&parsed).is_empty());

    // Empty input has no root element and is not an error.
    let parsed = parse(&b""[..], &options()).unwrap();
    assert!(!has_tag(&parsed, "html"));
}

#[test]
fn an_internal_subset_defines_nothing_and_leaves_the_document_alone() {
    let source = br#"<!DOCTYPE html [
  <!ENTITY e "expanded">
  <!ENTITY row "<tr><td/></tr>">
  <!-- ] -->
  <?pi ?>
]><html xmlns="http://www.w3.org/1999/xhtml"><body id="b">&e;|&row;</body></html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    // The text of the entities does not appear, and neither does the markup
    // inside their values.
    assert_eq!(
        parsed
            .dom
            .element_text_content(by_id(&parsed, "b"))
            .as_deref(),
        Some("&e;|&row;")
    );
    assert!(!has_tag(&parsed, "tr") && !has_tag(&parsed, "td"));
    // The document element is still the root's only element child.
    let root = parsed.dom.get_node(parsed.dom.root_index()).unwrap();
    let elements: Vec<_> = root
        .children
        .iter()
        .filter_map(|&child| parsed.dom.get_node(child).unwrap().tag_name())
        .collect();
    assert_eq!(elements, ["html"]);
    assert_eq!(
        messages(&parsed)
            .iter()
            .filter(|message| **message == "Internal DTD subset ignored")
            .count(),
        1
    );
}

#[test]
fn external_dtds_and_entities_are_never_fetched() {
    struct PanicIfCalledProvider;
    impl NetworkProvider for PanicIfCalledProvider {
        fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
            panic!("fetched {}", request.url);
        }
    }
    let source = br#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://example.invalid/xhtml1-strict.dtd"><html xmlns="http://www.w3.org/1999/xhtml"><body id="b">&external;&nbsp;&copy;</body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: Some(&PanicIfCalledProvider),
        base_url: Some("http://example.invalid/".parse().unwrap()),
    };
    let parsed = parse(&source[..], &options).unwrap();
    // The HTML named references resolve; an unknown name stays literal text.
    assert_eq!(
        parsed
            .dom
            .element_text_content(by_id(&parsed, "b"))
            .as_deref(),
        Some("&external;\u{a0}\u{a9}")
    );
}

#[test]
fn doctype_markers_inside_text_comments_and_literals_do_not_hide_content() {
    let source = br#"<?test <!DOCTYPE html [?>
<!-- <!DOCTYPE html [ -->
<!DOCTYPE html SYSTEM "http://example.invalid/[unused].dtd">
<html value="&lt;!DOCTYPE html [" id="h"><![CDATA[<!DOCTYPE html [literal]]>literal</html>"#;
    let parsed = parse(&source[..], &options()).unwrap();
    assert_eq!(
        parsed
            .dom
            .element_text_content(by_id(&parsed, "h"))
            .as_deref(),
        Some("<!DOCTYPE html [literalliteral")
    );
    assert!(!messages(&parsed).contains(&"Internal DTD subset ignored"));
}

#[test]
fn invalid_utf8_is_an_encoding_error() {
    for source in [&b"<html>\xff</html>"[..], &b"<html>\xe3\x81"[..]] {
        assert!(matches!(
            parse(source, &options()),
            Err(ParseError::Encoding { .. })
        ));
    }
}

#[test]
fn input_read_errors_preserve_the_diagnostic_and_io_cause() {
    use std::error::Error;
    struct BrokenReader;
    impl Read for BrokenReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "fixture read refused",
            ))
        }
    }
    let error = parse(BrokenReader, &options()).unwrap_err();
    assert!(error.to_string().contains("source read error"));
    let io_error = error
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(io_error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(io_error.to_string(), "fixture read refused");
}

#[test]
fn input_and_node_budgets_accept_the_boundary() {
    let source = b"<html xmlns='http://www.w3.org/1999/xhtml'><body><div/></body></html>";
    let mut limits = RenderLimits::default();
    limits.max_input_bytes = Some(source.len() as u64);
    limits.max_dom_nodes = Some(4);
    assert!(parse_with_limits(&source[..], &options(), &limits).is_ok());

    limits.max_input_bytes = Some(source.len() as u64 - 1);
    assert_eq!(
        limit_error(parse_with_limits(&source[..], &options(), &limits)),
        (
            LimitKind::InputBytes,
            source.len() as u64 - 1,
            source.len() as u64
        )
    );

    limits.max_input_bytes = None;
    limits.max_dom_nodes = Some(3);
    assert_eq!(
        limit_error(parse_with_limits(&source[..], &options(), &limits)),
        (LimitKind::DomNodes, 3, 4)
    );
}

#[test]
fn the_node_budget_counts_elements_and_the_text_between_them() {
    let mut limits = RenderLimits::default();
    limits.max_dom_nodes = Some(2);
    for source in [&b"<root><child/></root>"[..], &b"<root>text</root>"[..]] {
        assert_eq!(
            limit_error(parse_with_limits(source, &options(), &limits)),
            (LimitKind::DomNodes, 2, 3)
        );
    }
    limits.max_dom_nodes = Some(3);
    assert!(parse_with_limits(&b"<root>text</root>"[..], &options(), &limits).is_ok());
}

#[test]
fn the_warning_budget_caps_what_a_malformed_document_records() {
    let source = "<root>".to_owned() + &"</x>".repeat(100);
    let mut limits = RenderLimits::default();
    limits.max_parse_warnings = Some(5);
    let parsed = parse_with_limits(source.as_bytes(), &options(), &limits).unwrap();
    assert_eq!(messages(&parsed).len(), 5);
    assert!(messages(&parsed)[4].contains("further parse errors suppressed"));
}

fn nested(depth: u32) -> String {
    let mut source = String::new();
    for _ in 0..depth {
        source.push_str("<a>");
    }
    for _ in 0..depth {
        source.push_str("</a>");
    }
    source
}

#[test]
fn nesting_up_to_the_depth_limit_is_accepted() {
    let parsed = parse(nested(MAX_ELEMENT_DEPTH).as_bytes(), &options()).unwrap();
    assert_eq!(parsed.dom.node_count(), MAX_ELEMENT_DEPTH as usize + 1);
    assert!(messages(&parsed).is_empty());
}

#[test]
fn nesting_over_the_depth_limit_is_a_limit_error() {
    assert_eq!(
        limit_error(parse(nested(MAX_ELEMENT_DEPTH + 1).as_bytes(), &options())),
        (
            LimitKind::DomDepth,
            u64::from(MAX_ELEMENT_DEPTH),
            u64::from(MAX_ELEMENT_DEPTH) + 1
        )
    );
}

#[test]
fn closed_and_empty_elements_do_not_add_to_the_depth() {
    // A thousand siblings of every kind never stack up.
    let source = format!(
        "<root>{}{}{}</root>",
        "<a/>".repeat(1000),
        "<b></b>".repeat(1000),
        "<c>text</c>".repeat(1000)
    );
    assert!(parse(source.as_bytes(), &options()).is_ok());
}

#[test]
fn unclosed_elements_count_until_the_end_tag_that_pops_them() {
    // `</root>` pops all the open `<a>`s, so the next sibling starts shallow.
    let source = format!("<root>{}</root>", "<a>".repeat(200));
    assert!(parse(source.as_bytes(), &options()).is_ok());
    let source = format!("<root>{}</root>", "<a>".repeat(MAX_ELEMENT_DEPTH as usize));
    assert!(matches!(
        parse(source.as_bytes(), &options()),
        Err(ParseError::LimitExceeded {
            kind: LimitKind::DomDepth,
            ..
        })
    ));
}

/// A reader that never ends and counts the bytes it hands out.
struct Endless<'a> {
    unit: &'a [u8],
    served: &'a std::cell::Cell<usize>,
}

impl Read for Endless<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        for (index, byte) in buffer.iter_mut().enumerate() {
            *byte = self.unit[(self.served.get() + index) % self.unit.len()];
        }
        self.served.set(self.served.get() + buffer.len());
        Ok(buffer.len())
    }
}

#[test]
fn a_budget_stops_the_read_without_buffering_the_input() {
    // The reader has no end, so only a parser that stops by itself returns.
    let served = std::cell::Cell::new(0);
    let reader = |unit| Endless {
        unit,
        served: &served,
    };
    let chunks = |served: &std::cell::Cell<usize>| served.get() / (64 * 1024);

    // The depth limit.
    let error = limit_error(parse(reader(b"<a>"), &options()));
    assert_eq!(error.0, LimitKind::DomDepth);
    assert!(chunks(&served) <= 2, "{} bytes read", served.get());

    // The node budget.
    served.set(0);
    let mut limits = RenderLimits::default();
    limits.max_dom_nodes = Some(100);
    let source = reader(b"<a/>");
    let error = limit_error(parse_with_limits(
        // The root element is the first of the endless siblings' parent.
        std::io::Read::chain(&b"<root>"[..], source),
        &options(),
        &limits,
    ));
    assert_eq!(error.0, LimitKind::DomNodes);
    assert!(chunks(&served) <= 2, "{} bytes read", served.get());

    // The input-byte budget.
    served.set(0);
    limits.max_dom_nodes = None;
    limits.max_input_bytes = Some(200_000);
    let error = limit_error(parse_with_limits(
        std::io::Read::chain(&b"<root>"[..], reader(b"x")),
        &options(),
        &limits,
    ));
    assert_eq!(error.0, LimitKind::InputBytes);
    assert!(served.get() <= 200_000 + 64 * 1024);
}

/// Hands the bytes out one at a time.
struct OneByteAtATime<'a>(&'a [u8]);

impl Read for OneByteAtATime<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let Some((&first, rest)) = self.0.split_first() else {
            return Ok(0);
        };
        buffer[0] = first;
        self.0 = rest;
        Ok(1)
    }
}

#[test]
fn input_split_anywhere_gives_the_same_document() {
    let source = "<?xml version=\"1.0\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"x.dtd\" [<!ENTITY é \"é]\">]>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:a=\"urn:a\"><head><style><![CDATA[p > b { color: red }]]></style></head><body><!-- c --><p id=\"p\" a:k=\"v &amp; w\">日本語 &nbsp;&amp;&#x41; <![CDATA[<>]]><?pi data?></p><a:x/></body></html>";
    let whole = parse(source.as_bytes(), &options()).unwrap();
    let split = parse(OneByteAtATime(source.as_bytes()), &options()).unwrap();
    assert_eq!(serialized(&whole), serialized(&split));
    assert_eq!(whole.dom.node_count(), split.dom.node_count());
    assert_eq!(messages(&whole), messages(&split));
    assert_eq!(
        whole.stylesheet_sources[0].parts[0].source,
        split.stylesheet_sources[0].parts[0].source
    );
    assert_eq!(
        split
            .dom
            .element_text_content(by_id(&split, "p"))
            .as_deref(),
        Some("日本語 \u{a0}&A <>")
    );
}
