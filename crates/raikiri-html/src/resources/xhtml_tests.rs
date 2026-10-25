use super::*;
use raikiri_traits::{
    FetchOutcome, FetchedResource, LimitKind, NetworkError, ParseError, RenderError, Request,
};
use std::sync::Mutex;

const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

fn xhtml_document(head: &str, body: &str) -> String {
    format!("<html xmlns=\"{XHTML_NS}\"><head>{head}</head><body>{body}</body></html>")
}

/// The node of the element with `id` in `document`.
fn element_with_id(document: &HtmlDocument, id: &str) -> usize {
    let dom = document.dom();
    (0..dom.node_count())
        .find(|&node| dom.element_attribute(node, "id") == Some(id))
        .expect("element in the document")
}

fn display_of(document: &HtmlDocument, id: &str) -> DisplayValue {
    document.cascade().computed[element_with_id(document, id)].display
}

/// Serves `p { display: none }` for every request and records the URLs.
#[derive(Default)]
struct HiddenParagraphStylesheet {
    requests: Mutex<Vec<Url>>,
}

impl NetworkProvider for HiddenParagraphStylesheet {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let final_url = request.url.clone();
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.url);
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: b"p { display: none }".as_slice().into(),
            content_type: Some("text/css".into()),
            final_url,
            encoding: None,
        }))
    }
}

#[test]
fn xhtml_document_is_cascaded_like_the_same_html_document() {
    let style = "<style>#hidden { display: none } #flex { display: flex }</style>";
    let body = "<p id=\"hidden\">a</p><div id=\"flex\"><span>b</span></div>";
    let xhtml = xhtml_document(style, body);
    let html = format!("<!DOCTYPE html><html><head>{style}</head><body>{body}</body></html>");
    let resources = RenderResources::new();

    let from_xhtml = parse_xhtml_with_resources(xhtml.as_bytes(), &resources).unwrap();
    let from_html = parse_html_with_resources(html.as_bytes(), &resources).unwrap();

    assert_eq!(from_xhtml.dom().node_count(), from_html.dom().node_count());
    assert_eq!(
        from_xhtml.cascade().computed.len(),
        from_html.cascade().computed.len()
    );
    assert_eq!(display_of(&from_xhtml, "hidden"), DisplayValue::None);
    assert_eq!(display_of(&from_xhtml, "flex"), DisplayValue::Flex);
    for id in ["hidden", "flex"] {
        assert_eq!(display_of(&from_xhtml, id), display_of(&from_html, id));
    }
}

#[test]
fn xhtml_document_takes_the_configured_stylesheets() {
    let xhtml = xhtml_document("", "<p id=\"a\">a</p><div id=\"b\">b</div>");
    let resources = RenderResources::new().stylesheet("p { display: none }");
    let document = parse_xhtml_with_resources(xhtml.as_bytes(), &resources).unwrap();
    assert_eq!(display_of(&document, "a"), DisplayValue::None);
    assert_eq!(display_of(&document, "b"), DisplayValue::Block);

    let plain = parse_xhtml_with_resources(xhtml.as_bytes(), &RenderResources::new()).unwrap();
    assert_eq!(display_of(&plain, "a"), DisplayValue::Block);
}

#[test]
fn xhtml_linked_stylesheet_is_fetched_against_the_configured_base() {
    let provider = HiddenParagraphStylesheet::default();
    let base = Url::parse("https://styles.test/doc/index.xhtml").unwrap();
    let resources = RenderResources::new()
        .network_provider(&provider)
        .base_url(base.clone());
    let xhtml = xhtml_document(
        "<link rel=\"stylesheet\" href=\"../css/print.css\"/>",
        "<p id=\"a\">a</p>",
    );
    let document = parse_xhtml_with_resources(xhtml.as_bytes(), &resources).unwrap();
    assert_eq!(
        *provider.requests.lock().unwrap(),
        [Url::parse("https://styles.test/css/print.css").unwrap()]
    );
    assert_eq!(display_of(&document, "a"), DisplayValue::None);
}

#[test]
fn xhtml_linked_stylesheet_is_ignored_without_a_network_provider() {
    let xhtml = xhtml_document(
        "<link rel=\"stylesheet\" href=\"print.css\"/>",
        "<p id=\"a\">a</p>",
    );
    let document = parse_xhtml_with_resources(xhtml.as_bytes(), &RenderResources::new()).unwrap();
    assert_eq!(display_of(&document, "a"), DisplayValue::Block);
}

#[test]
fn xhtml_border_width_device_scale_reaches_the_parse_cascade() {
    let xhtml = xhtml_document(
        "",
        "<div id=\"bordered\" style=\"border-top-width:49.75px;border-top-style:solid\"/>",
    );
    let width = |resources: &RenderResources<'_>| {
        let document = parse_xhtml_with_resources(xhtml.as_bytes(), resources).unwrap();
        document.cascade().computed[element_with_id(&document, "bordered")]
            .border
            .top
            .width()
            .0
    };
    assert_eq!(width(&RenderResources::new()), 49.75);
    assert_eq!(
        width(&RenderResources::new().border_width_device_scale(Some(1.5))),
        74.0 / 1.5
    );
}

#[test]
fn xhtml_input_is_read_within_the_configured_budgets() {
    let xhtml = xhtml_document("", "<p>one</p><p>two</p>");

    let small = RenderLimits::builder().max_input_bytes(Some(16)).build();
    let error = parse_xhtml_with_resources(
        xhtml.as_bytes(),
        &RenderResources::new().render_limits(small),
    )
    .expect_err("the input is longer than 16 bytes");
    assert!(
        matches!(
            error,
            RenderError::LimitExceeded {
                kind: LimitKind::InputBytes,
                limit: 16,
                ..
            }
        ),
        "{error:?}"
    );

    let few_nodes = RenderLimits::builder().max_dom_nodes(Some(3)).build();
    let error = parse_xhtml_with_resources(
        xhtml.as_bytes(),
        &RenderResources::new().render_limits(few_nodes),
    )
    .expect_err("the document has more than 3 nodes");
    assert!(
        matches!(
            error,
            RenderError::LimitExceeded {
                kind: LimitKind::DomNodes,
                limit: 3,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn xhtml_nesting_over_the_depth_limit_is_an_error() {
    let xhtml = format!(
        "<html xmlns=\"{XHTML_NS}\">{}{}</html>",
        "<div>".repeat(256),
        "</div>".repeat(256)
    );
    let error = parse_xhtml_with_resources(xhtml.as_bytes(), &RenderResources::new())
        .expect_err("257 levels are open");
    assert!(
        matches!(
            error,
            RenderError::LimitExceeded {
                kind: LimitKind::DomDepth,
                limit: 256,
                actual: 257,
            }
        ),
        "{error:?}"
    );
}

#[test]
fn xhtml_input_that_is_not_utf8_is_an_encoding_error() {
    let error = parse_xhtml_with_resources(b"<p>\xFF</p>".as_slice(), &RenderResources::new())
        .expect_err("not UTF-8");
    assert!(
        matches!(error, RenderError::Parse(ParseError::Encoding { .. })),
        "{error:?}"
    );
}
