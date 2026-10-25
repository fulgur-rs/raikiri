//! `parse_xhtml_with_resources` returns the document `parse_html_with_resources`
//! returns for the same markup and takes its configuration from the same
//! [`RenderResources`].

use std::sync::Mutex;

use raikiri_traits::{
    FetchOutcome, FetchedResource, LimitKind, NetworkError, NetworkProvider, ParseError,
    RenderError, RenderLimits, Request,
};

use crate::{HtmlDocument, RenderResources, parse_html_with_resources, parse_xhtml_with_resources};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn by_id(document: &HtmlDocument, id: &str) -> usize {
    let dom = document.dom();
    (0..dom.node_count())
        .find(|&node| dom.element_attribute(node, "id") == Some(id))
        .expect("element with the id")
}

fn xhtml(head: &str, body: &str) -> String {
    format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head>{head}</head><body>{body}</body></html>"
    )
}

fn limit_error(result: Result<HtmlDocument, RenderError>) -> (LimitKind, u64, u64) {
    match result {
        Err(RenderError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => (kind, limit, actual),
        Err(error) => panic!("not a limit error: {error:?}"),
        Ok(_) => panic!("the parse succeeded"),
    }
}

#[test]
fn the_document_is_cascaded_like_the_html_document() {
    let style = "#a { color: rgb(1, 2, 3) } p { font-size: 20px }";
    let xml = xhtml(
        &format!("<style><![CDATA[{style}]]></style>"),
        "<p id=\"a\">hi<br/></p>",
    );
    let html = format!("<style>{style}</style><p id=\"a\">hi<br></p>");
    let resources = RenderResources::new();
    let from_xhtml = parse_xhtml_with_resources(xml.as_bytes(), &resources).unwrap();
    let from_html = parse_html_with_resources(html.as_bytes(), &resources).unwrap();

    let xhtml_style = &from_xhtml.cascade().computed[by_id(&from_xhtml, "a")];
    let html_style = &from_html.cascade().computed[by_id(&from_html, "a")];
    assert_eq!(xhtml_style.color, html_style.color);
    assert_eq!(xhtml_style.font_size, html_style.font_size);
    assert_eq!(xhtml_style.font_size.0, 20.0);
    assert_eq!(
        from_xhtml.cascade().computed.len(),
        from_xhtml.dom().node_count(),
        "every node is cascaded"
    );
    assert_eq!(from_xhtml.stylesheet_sources().len(), 1);
}

#[test]
fn extra_stylesheets_apply_to_the_xhtml_document() {
    let xml = xhtml("", "<p id=\"a\">hi</p>");
    let resources = RenderResources::new().stylesheet("p { font-size: 33px }");
    let document = parse_xhtml_with_resources(xml.as_bytes(), &resources).unwrap();
    let style = &document.cascade().computed[by_id(&document, "a")];
    assert_eq!(style.font_size.0, 33.0);
}

#[test]
fn the_base_url_is_the_resources_fallback_or_the_documents_own() {
    let fallback = url::Url::parse("https://example.test/dir/doc.xhtml").unwrap();
    let resources = RenderResources::new().base_url(fallback.clone());

    let plain = parse_xhtml_with_resources(xhtml("", "").as_bytes(), &resources).unwrap();
    assert_eq!(plain.effective_base_url(), Some(&fallback));

    let own = xhtml("<base href=\"https://other.test/base/\"/>", "");
    let based = parse_xhtml_with_resources(own.as_bytes(), &resources).unwrap();
    assert_eq!(
        based.effective_base_url().map(url::Url::as_str),
        Some("https://other.test/base/")
    );
}

#[test]
fn fonts_reach_the_dom_before_the_cascade() {
    let xml = xhtml("", "<p>hi</p>");
    let without = parse_xhtml_with_resources(xml.as_bytes(), &RenderResources::new()).unwrap();
    assert!(!without.dom().has_font_collection());

    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let with =
        parse_xhtml_with_resources(xml.as_bytes(), &RenderResources::new().fonts(fonts)).unwrap();
    assert!(with.dom().has_font_collection());
}

#[test]
fn the_border_width_device_scale_reaches_the_cascade() {
    let xml = xhtml(
        "",
        "<div id=\"a\" style=\"border-top-width:49.75px;border-top-style:solid\"/>",
    );
    let width = |resources: &RenderResources<'_>| {
        let document = parse_xhtml_with_resources(xml.as_bytes(), resources).unwrap();
        document.cascade().computed[by_id(&document, "a")]
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

fn parsed_within(
    source: &str,
    change: impl FnOnce(&mut RenderLimits),
) -> Result<HtmlDocument, RenderError> {
    let mut limits = RenderLimits::default();
    change(&mut limits);
    parse_xhtml_with_resources(
        source.as_bytes(),
        &RenderResources::new().render_limits(limits),
    )
}

#[test]
fn the_parse_limits_of_the_resources_apply() {
    let source = xhtml("", "<div/>");
    let length = source.len() as u64;
    assert!(parsed_within(&source, |limits| limits.max_input_bytes = Some(length)).is_ok());
    assert_eq!(
        limit_error(parsed_within(&source, |limits| limits.max_input_bytes = Some(length - 1))),
        (LimitKind::InputBytes, length - 1, length)
    );
    assert_eq!(
        limit_error(parsed_within(&source, |limits| limits.max_dom_nodes = Some(3))).0,
        LimitKind::DomNodes
    );

    let mut deep = String::from("<html xmlns=\"http://www.w3.org/1999/xhtml\">");
    deep.push_str(&"<div>".repeat(256));
    assert_eq!(
        limit_error(parse_xhtml_with_resources(
            deep.as_bytes(),
            &RenderResources::new()
        )),
        (LimitKind::DomDepth, 256, 257)
    );
}

#[test]
fn input_that_is_not_utf8_is_an_encoding_error() {
    let error = parse_xhtml_with_resources(&b"<p>\xFF</p>"[..], &RenderResources::new())
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(
        error,
        RenderError::Parse(ParseError::Encoding { .. })
    ));
}

/// Serves one stylesheet for every request and records the URLs asked for.
#[derive(Default)]
struct StylesheetProvider {
    requests: Mutex<Vec<url::Url>>,
}

impl NetworkProvider for StylesheetProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let final_url = request.url.clone();
        self.requests.lock().unwrap().push(request.url);
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: b"p { font-size: 44px }".as_slice().into(),
            content_type: Some("text/css".into()),
            final_url,
            encoding: None,
        }))
    }
}

#[test]
fn the_network_provider_fetches_linked_stylesheets_of_the_xhtml_document() {
    let provider = StylesheetProvider::default();
    let resources = RenderResources::new().network_provider(&provider);
    let xml = xhtml(
        "<link rel=\"stylesheet\" href=\"https://css.test/print.css\"/>",
        "<p id=\"a\">hi</p>",
    );
    let document = parse_xhtml_with_resources(xml.as_bytes(), &resources).unwrap();
    assert_eq!(
        *provider.requests.lock().unwrap(),
        [url::Url::parse("https://css.test/print.css").unwrap()]
    );
    let style = &document.cascade().computed[by_id(&document, "a")];
    assert_eq!(style.font_size.0, 44.0);
}
