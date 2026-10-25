use super::*;
use raikiri_traits::LimitKind;

const NAMESPACE: &str = "http://www.w3.org/1999/xhtml";

fn xhtml(body: &str) -> String {
    format!("<html xmlns=\"{NAMESPACE}\"><body>{body}</body></html>")
}

fn page_count(document: &HtmlDocument, resources: &RenderResources<'_>) -> usize {
    let LayoutStatus::Completed(laid_out) = layout(
        document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    laid_out.pages().count()
}

fn limit_error(result: Result<HtmlDocument, RenderError>) -> (LimitKind, u64, u64) {
    match result {
        Err(RenderError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => (kind, limit, actual),
        Err(other) => panic!("expected a limit error, got {other:?}"),
        Ok(_) => panic!("expected a limit error, got a document"),
    }
}

#[test]
fn xhtml_with_resources_cascades_the_document_and_applies_extra_stylesheets() {
    let source = xhtml("<div>one</div><div>two</div><div>three</div>");

    let plain = RenderResources::new();
    let document = parse_xhtml_with_resources(source.as_bytes(), &plain).expect("parse");
    assert_eq!(
        document.cascade().computed.len(),
        document.dom().node_count()
    );
    assert_eq!(page_count(&document, &plain), 1);

    let paged = RenderResources::new().stylesheet("div { break-after: page }");
    let document = parse_xhtml_with_resources(source.as_bytes(), &paged).expect("parse");
    assert_eq!(page_count(&document, &paged), 3);
}

#[test]
fn xhtml_with_resources_lays_out_like_the_same_html() {
    let resources = RenderResources::new().stylesheet("div { break-after: page }");
    let body = "<div>one</div><div>two <b>bold</b></div><div>three</div>";
    let from_xhtml =
        parse_xhtml_with_resources(xhtml(body).as_bytes(), &resources).expect("xhtml parse");
    let from_html =
        parse_html_with_resources(format!("<body>{body}</body>").as_bytes(), &resources)
            .expect("html parse");
    assert_eq!(page_count(&from_xhtml, &resources), 3);
    assert_eq!(
        page_count(&from_xhtml, &resources),
        page_count(&from_html, &resources)
    );
}

#[test]
fn xhtml_with_resources_reads_the_document_as_xml() {
    // A self-closing `div` is an empty element in XML, so the two divs are
    // siblings; HTML would open the first and nest the second inside it.
    let resources = RenderResources::new();
    let document =
        parse_xhtml_with_resources(xhtml("<div/><div/>").as_bytes(), &resources).expect("parse");
    let serialized = document
        .dom()
        .serialize_inner_html(document.dom().root_index())
        .expect("serialize");
    assert!(
        serialized.contains("<body><div></div><div></div></body>"),
        "{serialized}"
    );
}

#[test]
fn xhtml_with_resources_enforces_the_configured_budgets() {
    let source = "<html xmlns='http://www.w3.org/1999/xhtml'><body><div/></body></html>";

    let limits = RenderLimits::builder()
        .max_input_bytes(Some(source.len() as u64))
        .max_dom_nodes(Some(4))
        .build();
    let at_the_boundary = RenderResources::new().render_limits(limits);
    assert!(parse_xhtml_with_resources(source.as_bytes(), &at_the_boundary).is_ok());

    let limits = RenderLimits::builder()
        .max_input_bytes(Some(source.len() as u64 - 1))
        .build();
    let resources = RenderResources::new().render_limits(limits);
    assert_eq!(
        limit_error(parse_xhtml_with_resources(source.as_bytes(), &resources)),
        (
            LimitKind::InputBytes,
            source.len() as u64 - 1,
            source.len() as u64
        )
    );

    let limits = RenderLimits::builder().max_dom_nodes(Some(3)).build();
    let resources = RenderResources::new().render_limits(limits);
    assert_eq!(
        limit_error(parse_xhtml_with_resources(source.as_bytes(), &resources)),
        (LimitKind::DomNodes, 3, 4)
    );
}

#[test]
fn xhtml_with_resources_stops_at_the_nesting_depth_limit() {
    let nested = |depth: usize| "<a>".repeat(depth) + &"</a>".repeat(depth);
    let resources = RenderResources::new();
    assert_eq!(
        limit_error(parse_xhtml_with_resources(
            nested(257).as_bytes(),
            &resources
        )),
        (LimitKind::DomDepth, 256, 257)
    );
}
