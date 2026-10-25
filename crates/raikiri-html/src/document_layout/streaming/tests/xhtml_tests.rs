use super::*;
use crate::parse_xhtml_with_resources;

const NAMESPACE: &str = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// A three-page document with the XML declaration and a DOCTYPE whose
/// internal subset the parser has to skip.
fn paged_xhtml() -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE html [<!ENTITY note \"a > b\"> <!-- ] -->]>\n\
         <html xmlns=\"{NAMESPACE}\"><head><title>t</title><style><![CDATA[\
         div {{ break-after: page }}]]></style></head><body>\
         <div id=\"a\">one</div><div id=\"b\">two <a href=\"#c\">to three</a></div>\
         <div id=\"c\">three <br/></div></body></html>"
    )
}

/// A long document mixing the constructs the frontier treats specially, as
/// the HTML tests' `long_document` does.
fn long_xhtml() -> String {
    let mut xhtml = format!(
        "<html xmlns=\"{NAMESPACE}\"><head><style><![CDATA[\
         h2 {{ break-after: avoid }} .keep {{ break-inside: avoid }}]]></style></head><body>"
    );
    for section in 0..12 {
        xhtml.push_str(&format!("<h2>Section {section}</h2>"));
        for paragraph in 0..6 {
            xhtml.push_str("<p>");
            for word in 0..60 {
                xhtml.push_str(&format!("word{section}x{paragraph}x{word} "));
            }
            xhtml.push_str("<b>bold</b> tail</p>\n");
        }
        match section % 4 {
            0 => xhtml.push_str(
                "<table><tr><td>a</td><td>b</td></tr><tr><td>long cell text</td><td>c</td></tr></table>",
            ),
            1 => xhtml.push_str(
                "<div style=\"display:flex\"><div>one</div><div>two</div></div>",
            ),
            2 => xhtml.push_str("<div class=\"keep\"><p>kept</p><p>together</p></div>"),
            _ => xhtml.push_str(&format!(
                "<div><div><p>nested</p></div> trailing text \
                 <svg xmlns=\"{SVG_NAMESPACE}\" width=\"20\" height=\"20\">\
                 <rect width=\"20\" height=\"20\"/></svg></div>"
            )),
        }
    }
    xhtml.push_str("</body></html>");
    xhtml
}

/// The pages of a batch layout of `xhtml`.
fn batch_xhtml(xhtml: &str) -> (PageLog, Vec<RenderWarning>) {
    batch_xhtml_with(xhtml, &RenderResources::new())
}

/// The pages of a batch layout of `xhtml` with `resources`.
fn batch_xhtml_with(xhtml: &str, resources: &RenderResources<'_>) -> (PageLog, Vec<RenderWarning>) {
    let document = parse_xhtml_with_resources(xhtml.as_bytes(), resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    let pages = laid_out
        .pages()
        .map(|page| (page.index(), page.fragments().map(|f| f.node()).collect()))
        .collect();
    (pages, laid_out.warnings().to_vec())
}

/// Stream `xhtml` in `chunk`-byte pieces with a checkpoint every
/// `checkpoint` bytes. Returns all pages, how many arrived before `finish`,
/// and the summary.
fn stream_xhtml(
    xhtml: &str,
    resources: &RenderResources<'_>,
    chunk: usize,
    checkpoint: usize,
) -> (PageLog, usize, StreamSummary) {
    let shared = Shared::default();
    let mut stream = StreamingLayout::new_xhtml(
        resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        shared.clone(),
    )
    // A checkpoint smaller than a piece falls after each piece.
    .checkpoint_bytes(checkpoint.max(chunk));
    for piece in xhtml.as_bytes().chunks(chunk) {
        stream.feed(piece).expect("feed");
    }
    let early = shared.0.borrow().len();
    let summary = completed(stream.finish().expect("finish"));
    let pages = shared.0.borrow().clone();
    (pages, early, summary)
}

fn parse_error_messages(warnings: &[RenderWarning]) -> Vec<&str> {
    warnings
        .iter()
        .filter_map(|warning| match &warning.kind {
            WarningKind::HtmlParseError { message } => Some(message.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn xhtml_pages_match_batch_layout_for_any_chunk_size() {
    let xhtml = paged_xhtml();
    let (expected, _) = batch_xhtml(&xhtml);
    assert_eq!(expected.len(), 3);
    let resources = RenderResources::new();
    for chunk in [1, 7, 64, xhtml.len()] {
        let (pages, _, summary) = stream_xhtml(&xhtml, &resources, chunk, 1);
        assert_eq!(pages, expected, "chunk {chunk}");
        assert_eq!(summary.page_count, 3);
        assert_eq!(summary.anchors.get("c").map(|a| a.page_index), Some(2));
        assert!(
            parse_error_messages(&summary.warnings).contains(&"Internal DTD subset ignored"),
            "chunk {chunk}"
        );
    }
}

#[test]
fn progressive_xhtml_pages_match_batch_layout() {
    let xhtml = long_xhtml();
    let (expected, _) = batch_xhtml(&xhtml);
    assert!(expected.len() > 8, "the fixture spans many pages");
    let resources = RenderResources::new();
    for (chunk, checkpoint) in [(2048, 1), (300, 4096), (5000, 1)] {
        let (pages, early, _) = stream_xhtml(&xhtml, &resources, chunk, checkpoint);
        assert_eq!(pages, expected, "chunk {chunk}, checkpoint {checkpoint}");
        assert!(
            early >= expected.len() / 2,
            "chunk {chunk}, checkpoint {checkpoint}: only {early} pages before finish"
        );
    }
}

#[test]
fn an_open_xhtml_table_holds_its_pages() {
    let mut xhtml = format!("<html xmlns=\"{NAMESPACE}\"><body><table>");
    for row in 0..400 {
        xhtml.push_str(&format!("<tr><td>row {row}</td><td>cell</td></tr>"));
    }
    let open_part = xhtml.len();
    xhtml.push_str("</table><p>after</p><!--");
    // Pad the rest to the length of the open part, so that each feed below
    // ends at a checkpoint.
    xhtml.push_str(&" ".repeat(2 * open_part - xhtml.len() - 3));
    xhtml.push_str("--></body></html>");
    let (expected, _) = batch_xhtml(&xhtml);
    assert!(expected.len() > 2);

    let resources = RenderResources::new();
    let shared = Shared::default();
    let mut stream = StreamingLayout::new_xhtml(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        shared.clone(),
    )
    .checkpoint_bytes(open_part);
    stream.feed(&xhtml.as_bytes()[..open_part]).expect("feed");
    assert!(
        shared.0.borrow().is_empty(),
        "nothing is final in an open table"
    );
    stream.feed(&xhtml.as_bytes()[open_part..]).expect("feed");
    assert!(
        !shared.0.borrow().is_empty(),
        "closing the table finalises pages"
    );
    completed(stream.finish().expect("finish"));
    assert_eq!(*shared.0.borrow(), expected);
}

#[test]
fn repaired_xhtml_streams_like_the_batch_layout() {
    // A stray end tag, an element the end tag of its parent closes, and a
    // mismatched end tag, all before the pages that are delivered early.
    let mut xhtml = format!(
        "<html xmlns=\"{NAMESPACE}\"><head><style><![CDATA[p {{ margin: 0 }}]]></style></head><body>\
         </p><div class=\"unclosed\"><b>bold</i> text</div>"
    );
    for paragraph in 0..120 {
        xhtml.push_str(&format!("<p>paragraph {paragraph} of the document</p>"));
    }
    xhtml.push_str("</body></html>");
    let (expected, batch_warnings) = batch_xhtml(&xhtml);
    assert!(expected.len() > 2);
    let resources = RenderResources::new();
    let (pages, early, summary) = stream_xhtml(&xhtml, &resources, 256, 1);
    assert_eq!(pages, expected);
    assert!(early > 0, "pages arrive before the input ends");
    let streamed = parse_error_messages(&summary.warnings);
    assert!(!streamed.is_empty(), "the repairs are reported");
    assert_eq!(streamed, parse_error_messages(&batch_warnings));
}

#[test]
fn xhtml_nesting_over_the_depth_limit_stops_streaming() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new_xhtml(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );
    let open = format!("<html xmlns=\"{NAMESPACE}\">{}", "<div>".repeat(255));
    stream.feed(open.as_bytes()).expect("256 levels are open");
    let error = stream.feed(b"<div>").expect_err("the 257th level");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::DomDepth,
            limit: 256,
            actual: 257,
        }
    ));
    assert!(matches!(
        stream.finish(),
        Err(RenderError::Configuration(_))
    ));
}

#[test]
fn xhtml_invalid_utf8_is_an_encoding_error() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new_xhtml(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );
    let error = stream.feed(b"<p>\xFF</p>").expect_err("not UTF-8");
    assert!(matches!(
        error,
        RenderError::Parse(ParseError::Encoding { .. })
    ));
}

#[test]
fn xhtml_pages_match_batch_layout_with_fonts_and_a_border_scale() {
    let ahem = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
    ));
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", ahem.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .border_width_device_scale(Some(1.5));
    let mut xhtml = format!(
        "<html xmlns=\"{NAMESPACE}\"><head><style><![CDATA[\
         body {{ font: 16px Ahem }} \
         p {{ border-top: 1.3px solid; margin: 0 }}]]></style></head><body>"
    );
    for paragraph in 0..80 {
        xhtml.push_str(&format!("<p>paragraph {paragraph} of the document</p>"));
    }
    xhtml.push_str("</body></html>");
    let (expected, _) = batch_xhtml_with(&xhtml, &resources);
    assert!(expected.len() > 1, "the fixture spans pages");
    let (pages, _, _) = stream_xhtml(&xhtml, &resources, 256, 1);
    assert_eq!(pages, expected);
}
