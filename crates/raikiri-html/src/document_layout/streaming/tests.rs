use std::io::Write;

use raikiri_traits::{AbortController, LimitKind, ParseError, RenderLimits, WarningKind};

use super::*;
use crate::StringFetchMode;
use crate::layout;

mod avoided_chain_tests;
#[cfg(feature = "streaming-diagnostics")]
mod diagnostics_tests;
mod earlier_warnings_tests;
mod head_checkpoint_tests;
mod open_table_skip_tests;
mod snapshot_reuse_tests;
mod xhtml_tests;

const PAGED: &str = "<style>div { break-after: page }</style>\
    <div id=a style='bookmark-level: 1'>one</div>\
    <div id=b style='bookmark-level: 2'>two <a href='#c'>to three</a></div>\
    <div id=c style='bookmark-level: 3'>three</div>\
    <span style='display: none; bookmark-level: 4'>hidden</span>";

/// What a sink saw: per page, its index and the nodes of its fragments.
#[derive(Default)]
struct Record {
    pages: Vec<(u32, Vec<NodeId>)>,
    events: Vec<Vec<NodeId>>,
    finished: bool,
    expected_document: Option<crate::HtmlDocument>,
    offered_table_pages: usize,
    inspect_sample: bool,
    abort_before_sample: Option<AbortController>,
    complete_sample_count: Option<u32>,
    sample_rows: usize,
    paint_lines: Vec<Vec<NodeId>>,
    fragment_rects: Vec<Vec<raikiri_traits::PaintRect>>,
    stage_before: Option<u32>,
    staged: Vec<u32>,
    discarded: usize,
    fail_finish: bool,
    fail_on_page: Option<u32>,
    abort_on_page: Option<(u32, AbortController)>,
}

impl PageSink for &mut Record {
    type Output = StreamSummary;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        if self.fail_on_page == Some(page.index()) {
            return Err(std::io::Error::other("sink failed"));
        }
        if let Some(expected) = &self.expected_document {
            self.paint_lines.push(painted_line_roots(page.page()));
            self.fragment_rects
                .push(page.page().fragments().map(|f| f.rect()).collect());
            let view = page.page().dom();
            for fragment in page.page().fragments() {
                let id = fragment.node();
                let source = expected.dom().get_node(id.0 as usize).unwrap();
                assert_eq!(view.local_name(id), source.tag_name());
                assert_eq!(view.text(id), source.text_content());
                if source.tag_name() == Some("td") {
                    assert!(page.page().computed(id).is_some());
                    assert!(page.page().paint_transform(id).is_some());
                    assert_eq!(
                        view.parent(id).map(|p| p.0 as usize),
                        expected.dom().parent_of(id.0 as usize)
                    );
                }
            }
        }
        let nodes = page.page().fragments().map(|f| f.node()).collect();
        self.pages.push((page.index(), nodes));
        self.events
            .push(events.into_iter().map(|event| event.node_id).collect());
        if let Some((index, controller)) = &self.abort_on_page
            && *index == page.index()
        {
            controller.abort();
        }
        Ok(())
    }

    fn wants_table_sample(&self) -> bool {
        if let Some(controller) = &self.abort_before_sample {
            controller.abort();
        }
        self.inspect_sample
    }

    fn stage_table_sample(
        &mut self,
        sample: &DocumentLayout,
        ending: &DocumentLayout,
        rows: &mut TableTextRows<'_>,
    ) -> std::io::Result<Option<u32>> {
        assert!(
            Shared::default()
                .stage_table_sample(sample, ending, rows)?
                .is_none()
        );
        assert!(sample.pages().count() > 1);
        assert_eq!(ending.pages().len(), 2);
        let first = body_rows(sample.page_at(0));
        let middle = body_rows(sample.page_at(1));
        assert_eq!(
            body_rows(ending.page_at(1)),
            (rows.row_count() - first - 1) % middle + 1
        );
        assert_eq!(rows.row_count(), 600);
        while self.sample_rows < rows.row_count() {
            for row in rows.read_batch()? {
                assert_eq!(row.len(), 1);
                assert_eq!(row[0].text, format!("row & {}", self.sample_rows));
                assert!(row[0].node.unwrap() > row[0].cell);
                self.sample_rows += 1;
            }
            if self.sample_rows >= 256 && self.complete_sample_count.is_none() {
                break;
            }
        }
        self.staged.push(0);
        if self.fail_on_page == Some(0) {
            return Err(std::io::Error::other("sample output failed"));
        }
        if let Some((_, controller)) = &self.abort_on_page {
            controller.abort();
            rows.read_batch()?;
            panic!("read continued after cancellation");
        }
        // A page count offered before the input is exhausted must be discarded.
        Ok(Some(self.complete_sample_count.unwrap_or(1)))
    }

    fn stage_table_page(&mut self, page: StreamPage<'_>) -> std::io::Result<bool> {
        self.offered_table_pages += 1;
        if self.stage_before.is_some_and(|limit| page.index() < limit) {
            self.staged.push(page.index());
            if let Some((index, controller)) = &self.abort_on_page
                && *index == page.index()
            {
                controller.abort();
            }
            if self.fail_on_page == Some(page.index()) {
                return Err(std::io::Error::other("failed after staging"));
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn discard_staged_table_pages(&mut self) {
        self.discarded += self.staged.len();
        self.staged.clear();
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        if self.fail_finish {
            return Err(std::io::Error::other("output finalization failed"));
        }
        self.finished = true;
        Ok(summary)
    }
}

fn painted_line_roots(page: crate::Page<'_>) -> Vec<NodeId> {
    let runs = page.text_runs();
    for run in &runs {
        assert!(page.text_run_opacity_group(run).is_none());
    }
    page.paint_order_for_text_runs(&runs)
        .into_iter()
        .filter_map(|event| match event {
            crate::PaintEvent::TextLine(line) => Some(line.root),
            _ => None,
        })
        .collect()
}

fn batch_layout(html: &str) -> DocumentLayout {
    let resources = RenderResources::new();
    let doc = crate::parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    laid_out
}

fn batch_pages(html: &str) -> Vec<(u32, Vec<NodeId>)> {
    batch_layout(html)
        .pages()
        .map(|page| (page.index(), page.fragments().map(|f| f.node()).collect()))
        .collect()
}

fn stream(
    html: &str,
    chunk: usize,
    record: &mut Record,
    config: LayoutConfig,
) -> Result<StreamStatus<StreamSummary>, RenderError> {
    let resources = RenderResources::new();
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut stream = StreamingLayout::new(&resources, PageDefaults::default(), config, record)
        .consumer_properties(&registrations);
    for piece in html.as_bytes().chunks(chunk) {
        stream.feed(piece)?;
    }
    stream.finish()
}

fn completed(status: StreamStatus<StreamSummary>) -> StreamSummary {
    match status {
        StreamStatus::Completed(summary) => summary,
        StreamStatus::Aborted => panic!("aborted"),
    }
}

#[test]
fn streamed_pages_match_batch_layout_for_any_chunk_size() {
    let expected = batch_pages(PAGED);
    assert_eq!(expected.len(), 3);
    for chunk in [1, 5, 64, PAGED.len()] {
        let mut record = Record::default();
        let summary =
            completed(stream(PAGED, chunk, &mut record, LayoutConfig::default()).expect("stream"));
        assert_eq!(record.pages, expected, "chunk {chunk}");
        assert_eq!(summary.page_count, 3);
        assert!(record.finished);
    }
}

#[test]
fn events_arrive_with_the_page_of_the_first_fragment() {
    let mut record = Record::default();
    let summary =
        completed(stream(PAGED, 16, &mut record, LayoutConfig::default()).expect("stream"));
    // Root and body span the pages; the first content fragment after both
    // differs from page to page.
    let containers = &record.pages[0].1[..2];
    let firsts: Vec<NodeId> = record
        .pages
        .iter()
        .map(|(_, nodes)| {
            *nodes
                .iter()
                .find(|&&node| !containers.contains(&node))
                .expect("content")
        })
        .collect();
    assert_eq!(record.events.len(), 3);
    for (page, events) in record.events.iter().enumerate() {
        assert_eq!(events.len(), 1, "page {page}: {events:?}");
        assert!(record.pages[page].1.contains(&events[0]));
    }
    assert_ne!(firsts[0], firsts[1]);
    assert_eq!(summary.unplaced_events.len(), 1, "display:none element");
}

#[test]
fn summary_carries_anchors_of_the_whole_document() {
    let mut record = Record::default();
    let summary =
        completed(stream(PAGED, 64, &mut record, LayoutConfig::default()).expect("stream"));
    assert_eq!(summary.anchors.get("c").map(|a| a.page_index), Some(2));
}

#[test]
fn write_feeds_the_layout() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );
    for word in ["<p>", "Hello", ", ", "世界", "</p>"] {
        write!(stream, "{word}").expect("write");
    }
    stream.flush().expect("flush");
    let summary = completed(stream.finish().expect("finish"));
    assert_eq!(summary.page_count, 1);
}

#[test]
fn write_errors_are_returned_unchanged_by_finish() {
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );
    assert!(stream.write_all(b"<p>\xFF</p>").is_err());
    assert!(stream.write_all(b"more").is_err(), "input stays failed");
    let error = stream.finish().expect_err("finish fails");
    assert!(matches!(
        error,
        RenderError::Parse(ParseError::Encoding { .. })
    ));
    assert!(!record.finished);
}

#[test]
fn input_cap_stops_feeding() {
    let limits = RenderLimits::builder().max_input_bytes(Some(8)).build();
    let resources = RenderResources::new().render_limits(limits);
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );
    stream.feed(b"<p>abc").expect("under the cap");
    let error = stream.feed(b"def</p>").expect_err("over the cap");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::InputBytes,
            limit: 8,
            actual: 13,
        }
    ));
    assert!(matches!(
        stream.feed(b"x"),
        Err(RenderError::Configuration(_))
    ));
    assert!(matches!(
        stream.finish(),
        Err(RenderError::Configuration(_))
    ));
}

#[test]
fn dom_node_cap_stops_streaming_tree_allocation() {
    let limits = RenderLimits::builder().max_dom_nodes(Some(5)).build();
    let resources = RenderResources::new().render_limits(limits);
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );

    let error = stream
        .feed(b"<p>Hi</p>")
        .expect_err("the text node must exceed a five-node budget");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::DomNodes,
            limit: 5,
            actual: 6,
        }
    ));
    drop(stream);
    assert!(!record.finished);
}

#[test]
fn incomplete_trailing_character_fails_at_finish() {
    let mut record = Record::default();
    let resources = RenderResources::new();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    );
    stream
        .feed(&"日".as_bytes()[..2])
        .expect("partial character waits");
    assert!(matches!(
        stream.finish(),
        Err(RenderError::Parse(ParseError::Encoding { .. }))
    ));
}

#[test]
fn sink_errors_stop_the_layout() {
    let mut record = Record {
        fail_on_page: Some(1),
        ..Record::default()
    };
    let error = stream(PAGED, 64, &mut record, LayoutConfig::default()).expect_err("sink error");
    assert!(matches!(error, RenderError::Io(_)));
    assert_eq!(record.pages.len(), 1);
    assert!(!record.finished);
}

#[test]
fn abort_between_pages_stops_delivery() {
    let controller = AbortController::new();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut record = Record {
        abort_on_page: Some((0, controller)),
        ..Record::default()
    };
    let status = stream(PAGED, 64, &mut record, config).expect("stream");
    assert!(matches!(status, StreamStatus::Aborted));
    assert_eq!(record.pages.len(), 1);
    assert!(!record.finished);
}

#[test]
fn abort_on_the_last_page_skips_finishing_the_sink() {
    let last = batch_pages(PAGED).len() as u32 - 1;
    let controller = AbortController::new();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut record = Record {
        abort_on_page: Some((last, controller)),
        ..Record::default()
    };
    let status = stream(PAGED, 64, &mut record, config).expect("stream");
    assert!(matches!(status, StreamStatus::Aborted));
    assert_eq!(record.pages.len(), last as usize + 1);
    assert!(!record.finished);
}

#[test]
fn running_elements_are_laid_out_from_the_streamed_page() {
    struct Running(Option<f32>);
    impl PageSink for &mut Running {
        type Output = ();
        fn page(
            &mut self,
            page: StreamPage<'_>,
            _events: Vec<ConsumerPropertyEvent>,
        ) -> std::io::Result<()> {
            let node = page
                .page()
                .running_element("title", StringFetchMode::First)
                .expect("running element on the page");
            let laid_out = page
                .layout_running_element(node, 200.0)
                .expect("layout")
                .expect("running element");
            assert!(format!("{laid_out:?}").contains("height"));
            self.0 = Some(laid_out.height());
            Ok(())
        }
        fn finish(self, _summary: StreamSummary) -> std::io::Result<()> {
            Ok(())
        }
    }

    let html = "<style>h1 { position: running(title) }</style><h1>Title</h1><p>body</p>";
    let resources = RenderResources::new();
    let mut running = Running(None);
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut running,
    );
    stream.feed(html.as_bytes()).expect("feed");
    assert!(matches!(
        stream.finish().expect("finish"),
        StreamStatus::Completed(())
    ));
    assert!(running.0.is_some_and(|height| height > 0.0));
}

/// Per page, its index and the nodes of its fragments.
type PageLog = Vec<(u32, Vec<NodeId>)>;

/// Pages a sink saw, shared with the test while the stream owns the sink.
#[derive(Clone, Default)]
struct Shared(std::rc::Rc<std::cell::RefCell<PageLog>>);

impl PageSink for Shared {
    type Output = StreamSummary;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        let nodes = page.page().fragments().map(|f| f.node()).collect();
        self.0.borrow_mut().push((page.index(), nodes));
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        Ok(summary)
    }
}

#[test]
fn discarding_tentative_pages_preserves_delivery_for_a_nonstaging_sink() {
    let mut sink = Shared::default();
    let delivered = vec![(0, vec![NodeId(7)])];
    sink.0.borrow_mut().clone_from(&delivered);

    sink.discard_staged_table_pages();

    assert_eq!(*sink.0.borrow(), delivered);
}

/// Stream `html` in `chunk`-byte pieces with a checkpoint every
/// `checkpoint` bytes. Returns all pages and how many arrived before
/// `finish`.
fn stream_progressively(html: &str, chunk: usize, checkpoint: usize) -> (PageLog, usize) {
    let (pages, early, _) = stream_with_summary(html, chunk, checkpoint);
    (pages, early)
}

/// [`stream_progressively`], also returning the summary.
fn stream_with_summary(
    html: &str,
    chunk: usize,
    checkpoint: usize,
) -> (PageLog, usize, StreamSummary) {
    stream_with_config(html, chunk, checkpoint, LayoutConfig::default())
}

/// [`stream_with_summary`] with a layout configuration.
fn stream_with_config(
    html: &str,
    chunk: usize,
    checkpoint: usize,
    config: LayoutConfig,
) -> (PageLog, usize, StreamSummary) {
    let resources = RenderResources::new();
    let shared = Shared::default();
    let mut stream =
        StreamingLayout::new(&resources, PageDefaults::default(), config, shared.clone())
            // A checkpoint smaller than a piece falls after each piece.
            .checkpoint_bytes(checkpoint.max(chunk));
    for piece in html.as_bytes().chunks(chunk) {
        stream.feed(piece).expect("feed");
    }
    let early = shared.0.borrow().len();
    let summary = completed(stream.finish().expect("finish"));
    let pages = shared.0.borrow().clone();
    (pages, early, summary)
}

/// A long document mixing the constructs the frontier treats specially.
fn long_document() -> String {
    let mut html = String::from(
        "<!doctype html><style>h2 { break-after: avoid } .keep { break-inside: avoid }</style><body>",
    );
    for section in 0..12 {
        html.push_str(&format!("<h2>Section {section}</h2>"));
        for paragraph in 0..6 {
            html.push_str("<p>");
            for word in 0..60 {
                html.push_str(&format!("word{section}x{paragraph}x{word} "));
            }
            html.push_str("<b>bold</b> tail</p>\n");
        }
        match section % 4 {
            0 => html.push_str(
                "<table><tr><td>a</td><td>b</td></tr><tr><td>long cell text</td><td>c</td></tr></table>",
            ),
            1 => html.push_str(
                "<div style='display:flex'><div>one</div><div>two</div></div>",
            ),
            2 => html.push_str("<div class=keep><p>kept</p><p>together</p></div>"),
            _ => html.push_str("<div><div><p>nested</p></div> trailing text</div>"),
        }
    }
    html.push_str("</body>");
    html
}

#[test]
fn progressive_pages_match_batch_layout() {
    let html = long_document();
    let expected = batch_pages(&html);
    assert!(expected.len() > 8, "the fixture spans many pages");
    for (chunk, checkpoint) in [(2048, 1), (300, 4096), (5000, 1)] {
        let (pages, early) = stream_progressively(&html, chunk, checkpoint);
        assert_eq!(pages, expected, "chunk {chunk}, checkpoint {checkpoint}");
        assert!(
            early >= expected.len() / 2,
            "chunk {chunk}, checkpoint {checkpoint}: only {early} pages before finish"
        );
    }
}

#[test]
fn a_checkpoint_before_the_body_starts_delivers_nothing() {
    // A checkpoint after every byte falls in the head, where there is no
    // body to lay out yet.
    let html = "<html><head><style>div { break-after: page }</style></head>\
                <body><div>one</div><div>two</div><div>three</div></body></html>";
    let expected = batch_pages(html);
    assert_eq!(expected.len(), 3);
    let (pages, _) = stream_progressively(html, 1, 1);
    assert_eq!(pages, expected);
}

#[test]
fn forward_dependent_selectors_hold_pages_until_finish() {
    let html = long_document().replace("<style>", "<style>p:last-child { color: red } ");
    let expected = batch_pages(&html);
    let (pages, early) = stream_progressively(&html, 512, 1);
    assert_eq!(pages, expected);
    assert_eq!(early, 0);
}

#[test]
fn an_open_table_holds_its_pages() {
    let mut html = String::from("<table>");
    for row in 0..400 {
        html.push_str(&format!("<tr><td>row {row}</td><td>cell</td></tr>"));
    }
    let open_part = html.len();
    html.push_str("</table><p>after</p><!--");
    // Pad the rest to the length of the open part, so that each feed below
    // ends at a checkpoint.
    html.push_str(&" ".repeat(2 * open_part - html.len() - 3));
    html.push_str("-->");
    let expected = batch_pages(&html);
    assert!(expected.len() > 2);

    let resources = RenderResources::new();
    let shared = Shared::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        shared.clone(),
    )
    .checkpoint_bytes(open_part);
    stream.feed(&html.as_bytes()[..open_part]).expect("feed");
    assert!(
        shared.0.borrow().is_empty(),
        "nothing is final in an open table"
    );
    stream.feed(&html.as_bytes()[open_part..]).expect("feed");
    assert!(
        !shared.0.borrow().is_empty(),
        "closing the table finalises pages"
    );
    completed(stream.finish().expect("finish"));
    assert_eq!(*shared.0.borrow(), expected);
}

#[test]
fn progressive_pages_match_batch_layout_with_page_rules_and_floats() {
    let mut html = String::from(
        "<style>@page :first { margin: 2in } h1 { position: running(title) } \
         .chapter { break-before: page } .side { float: right; width: 30% }</style>\
         <h1>Running title</h1>",
    );
    for chapter in 0..5 {
        html.push_str("<section class=chapter>");
        html.push_str(&format!(
            "<h2>Chapter {chapter}</h2><div class=side>aside {chapter}</div>"
        ));
        for paragraph in 0..8 {
            html.push_str("<p>");
            for word in 0..40 {
                html.push_str(&format!("w{chapter}x{paragraph}x{word} "));
            }
            html.push_str("</p>");
        }
        html.push_str("</section>");
    }
    let expected = batch_pages(&html);
    assert!(expected.len() > 5);
    for (chunk, checkpoint) in [(1500, 1), (700, 2000)] {
        let (pages, early) = stream_progressively(&html, chunk, checkpoint);
        assert_eq!(pages, expected, "chunk {chunk}, checkpoint {checkpoint}");
        assert!(early > 0, "chunk {chunk}, checkpoint {checkpoint}");
    }
}

fn ignored(summary: &StreamSummary) -> Vec<(Option<NodeId>, String)> {
    summary
        .warnings
        .iter()
        .filter(|warning| matches!(warning.kind, WarningKind::StreamingContentIgnored))
        .map(|warning| (warning.node_id, warning.details.clone()))
        .collect()
}

fn paragraphs(count: usize) -> String {
    let mut html = String::new();
    for paragraph in 0..count {
        html.push_str("<p>");
        for word in 0..50 {
            html.push_str(&format!("p{paragraph}w{word} "));
        }
        html.push_str("</p>");
    }
    html
}

#[test]
fn styles_inside_body_are_not_applied() {
    let html = "<p>a</p><style>p { break-before: page }</style><p>b</p>";
    assert_eq!(batch_pages(html).len(), 2);
    let (pages, _, summary) = stream_with_summary(html, html.len(), usize::MAX);
    assert_eq!(pages.len(), 1);
    let ignored = ignored(&summary);
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    assert!(ignored[0].1.contains("<style>"));
}

#[test]
fn styles_in_head_still_apply() {
    let html = "<style>p { break-before: page }</style><p>a</p><p>b</p>";
    let (pages, _, summary) = stream_with_summary(html, html.len(), usize::MAX);
    assert_eq!(pages, batch_pages(html));
    assert!(ignored(&summary).is_empty());
}

#[test]
fn fixed_elements_after_the_first_delivered_page_are_skipped() {
    let fixed = "<div style='position: fixed; top: 0'>stamp</div>";
    let html = format!("{}{fixed}<p>end</p>", paragraphs(60));
    let (pages, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    let ignored = ignored(&summary);
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    assert!(ignored[0].1.contains("position: fixed"));
    // Batch layout repeats the element on every page; the stream does not
    // draw it anywhere.
    let stamp = ignored[0].0.expect("node");
    assert!(pages.iter().all(|(_, nodes)| !nodes.contains(&stamp)));
    assert!(
        batch_pages(&html)
            .iter()
            .all(|(_, nodes)| nodes.contains(&stamp))
    );
}

#[test]
fn fixed_elements_with_malformed_inline_styles_are_skipped() {
    // The unterminated comment would swallow anything appended to the
    // inline style, while the `position` declaration before it still holds.
    let fixed = "<div style='position: fixed; top: 0;/*'>stamp</div>";
    let html = format!("{}{fixed}<p>end</p>", paragraphs(60));
    let (pages, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    let ignored = ignored(&summary);
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    let stamp = ignored[0].0.expect("node");
    assert!(pages.iter().all(|(_, nodes)| !nodes.contains(&stamp)));
}

#[test]
fn late_fixed_elements_are_found_in_the_layout_media_context() {
    let fixed = "<div class=stamp>stamp</div>";
    let html = format!(
        "<style>@media (min-width: 600px) {{ .stamp {{ position: fixed; top: 0 }} }}</style>{}{fixed}<p>end</p>",
        paragraphs(60)
    );
    let config = LayoutConfig::builder()
        .media_context(raikiri_style::MediaContext::with_viewport(
            crate::MediaType::Print,
            1000,
            1400,
        ))
        .build();
    let (pages, early, summary) = stream_with_config(&html, 1000, 1, config);
    assert!(early > 0);
    let ignored = ignored(&summary);
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    let stamp = ignored[0].0.expect("node");
    assert!(pages.iter().all(|(_, nodes)| !nodes.contains(&stamp)));

    // The default context leaves the element in flow, where it is laid out.
    let (pages, _, summary) = stream_with_summary(&html, 1000, 1);
    assert!(self::ignored(&summary).is_empty());
    assert_eq!(pages, batch_pages(&html));
}

#[test]
fn fixed_elements_before_the_first_delivered_page_repeat() {
    let html = format!(
        "<div style='position: fixed; top: 0'>stamp</div>{}",
        paragraphs(60)
    );
    let (pages, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    assert_eq!(pages, batch_pages(&html));
    assert!(ignored(&summary).is_empty());
}

#[test]
fn late_body_attributes_are_ignored_after_delivery() {
    let html = format!("{}<body style='font-size: 40px'><p>end</p>", paragraphs(60));
    let (_, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    let ignored = ignored(&summary);
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    assert!(ignored[0].1.contains("style"));

    // Without a delivered page the attributes still apply.
    let (pages, early, summary) = stream_with_summary(&html, html.len(), usize::MAX);
    assert_eq!(early, 0);
    assert_eq!(pages, batch_pages(&html));
    assert!(self::ignored(&summary).is_empty());
}

#[test]
fn repeated_late_body_tags_are_reported_once() {
    let html = format!(
        "{}{}<p>end</p>",
        paragraphs(60),
        "<body x><body y><html z>".repeat(50)
    );
    let (_, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    let ignored = ignored(&summary);
    // One warning each for <body> and <html>.
    assert_eq!(ignored.len(), 2, "{ignored:?}");
}

#[test]
fn pages_streamed_early_defer_the_page_count() {
    /// Per page: the bottom box's content and its deferred slot texts for
    /// the final page count.
    struct Footers(Vec<(String, Vec<String>)>);
    impl PageSink for &mut Footers {
        type Output = StreamSummary;
        fn page(
            &mut self,
            page: StreamPage<'_>,
            _events: Vec<ConsumerPropertyEvent>,
        ) -> std::io::Result<()> {
            let boxes = page.page().margin_boxes();
            let footer = boxes.first().expect("footer box");
            let deferred = footer
                .deferred
                .iter()
                .map(|slot| footer.content[slot.range.clone()].to_owned())
                .collect();
            self.0.push((footer.content.clone(), deferred));
            Ok(())
        }
        fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
            Ok(summary)
        }
    }

    let html = format!(
        "<style>@page {{ margin: 40px; @bottom-center {{ content: counter(page) ' / ' counter(pages) }} }}</style>{}",
        paragraphs(60)
    );
    let resources = RenderResources::new();
    let mut footers = Footers(Vec::new());
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut footers,
    )
    .checkpoint_bytes(1000);
    for piece in html.as_bytes().chunks(1000) {
        stream.feed(piece).expect("feed");
    }
    let StreamStatus::Completed(summary) = stream.finish().expect("finish") else {
        panic!("aborted");
    };
    let page_count = summary.page_count;
    assert!(page_count > 2);
    let early: Vec<_> = footers
        .0
        .iter()
        .filter(|(_, slots)| !slots.is_empty())
        .collect();
    assert!(
        !early.is_empty(),
        "some pages arrive before the count is known"
    );
    for (index, (content, slots)) in footers.0.iter().enumerate() {
        if slots.is_empty() {
            assert_eq!(*content, format!("{} / {page_count}", index + 1));
        } else {
            // The default page limit of 10000 has five digits.
            assert_eq!(*content, format!("{} / 99999", index + 1));
            assert_eq!(slots, &["99999"]);
        }
    }
    assert!(footers.0.last().is_some_and(|(_, slots)| slots.is_empty()));

    // The summary relays out exactly the pages that showed the placeholder.
    let relaid: Vec<_> = summary
        .page_count_margin_boxes
        .iter()
        .map(|(index, boxes)| {
            let footer = boxes.first().expect("footer box");
            assert!(footer.deferred.is_empty());
            (*index, footer.content.clone())
        })
        .collect();
    let expected: Vec<_> = footers
        .0
        .iter()
        .enumerate()
        .filter(|(_, (_, slots))| !slots.is_empty())
        .map(|(index, _)| (index as u32, format!("{} / {page_count}", index + 1)))
        .collect();
    assert_eq!(relaid, expected);
}

#[test]
fn the_placeholder_has_as_many_digits_as_the_page_limit() {
    let config = |limit| {
        LayoutConfig::builder()
            .limits(RenderLimits::builder().max_document_pages(limit).build())
            .build()
    };
    assert_eq!(page_count_placeholder(&config(Some(10_000))), 99_999);
    assert_eq!(page_count_placeholder(&config(Some(9))), 9);
    assert_eq!(page_count_placeholder(&config(Some(0))), 9);
    assert_eq!(page_count_placeholder(&config(None)), 999_999_999);
}

#[test]
fn streamed_pages_carry_the_base_url() {
    struct BaseUrl(Option<String>);
    impl PageSink for &mut BaseUrl {
        type Output = ();
        fn page(
            &mut self,
            page: StreamPage<'_>,
            _events: Vec<ConsumerPropertyEvent>,
        ) -> std::io::Result<()> {
            self.0 = page.base_url().map(ToString::to_string);
            Ok(())
        }
        fn finish(self, _summary: StreamSummary) -> std::io::Result<()> {
            Ok(())
        }
    }

    let resources =
        RenderResources::new().base_url(url::Url::parse("https://example.com/doc/").unwrap());
    let mut base = BaseUrl(None);
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut base,
    );
    stream
        .feed(b"<base href='https://example.com/other/'><p><a href='x'>x</a></p>")
        .expect("feed");
    assert!(matches!(
        stream.finish().expect("finish"),
        StreamStatus::Completed(())
    ));
    assert_eq!(base.0.as_deref(), Some("https://example.com/other/"));
}

/// Feed `html` in `chunk`-byte pieces with a checkpoint after each piece,
/// stopping at the first error, then finish.
fn stream_with_checkpoints(
    html: &str,
    chunk: usize,
    record: &mut Record,
    config: LayoutConfig,
) -> Result<StreamStatus<StreamSummary>, RenderError> {
    let resources = RenderResources::new();
    let mut stream = StreamingLayout::new(&resources, PageDefaults::default(), config, record)
        .checkpoint_bytes(chunk);
    for piece in html.as_bytes().chunks(chunk) {
        stream.feed(piece)?;
    }
    stream.finish()
}

#[test]
fn an_abort_before_a_checkpoint_stops_the_stream() {
    let controller = AbortController::new();
    controller.abort();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut record = Record::default();
    let status =
        stream_with_checkpoints(&long_document(), 2048, &mut record, config).expect("stream");
    assert!(matches!(status, StreamStatus::Aborted));
    assert!(record.pages.is_empty());
    assert!(!record.finished);
}

#[test]
fn an_abort_during_a_checkpoint_stops_the_stream() {
    let controller = AbortController::new();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut record = Record {
        abort_on_page: Some((0, controller)),
        ..Record::default()
    };
    let status =
        stream_with_checkpoints(&long_document(), 2048, &mut record, config).expect("stream");
    assert!(matches!(status, StreamStatus::Aborted));
    assert_eq!(record.pages.len(), 1);
    assert!(!record.finished);
}

#[test]
fn sink_errors_at_a_checkpoint_stop_the_stream() {
    let mut record = Record {
        fail_on_page: Some(0),
        ..Record::default()
    };
    let error =
        stream_with_checkpoints(&long_document(), 2048, &mut record, LayoutConfig::default())
            .expect_err("sink error");
    assert!(matches!(error, RenderError::Io(_)));
    assert!(record.pages.is_empty());
    assert!(!record.finished);
}

#[test]
fn one_large_feed_still_checkpoints_at_every_boundary() {
    // The sink fails on the first page, which is final after the first
    // checkpoint. Bytes after that boundary are invalid UTF-8; feeding them
    // before the checkpoint would report the input error instead.
    let mut html = long_document().into_bytes();
    html.extend_from_slice(b"\xFF");
    let resources = RenderResources::new();
    let mut record = Record {
        fail_on_page: Some(0),
        ..Record::default()
    };
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    )
    .checkpoint_bytes(html.len() / 2);
    let error = stream.feed(&html).expect_err("sink error");
    assert!(matches!(error, RenderError::Io(_)), "{error:?}");
}

#[test]
fn background_image_preloading_can_be_turned_off() {
    let html = "<div style='background-image: url(missing.png); height: 10px'></div>";
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    )
    .preload_background_images(false);
    stream.feed(html.as_bytes()).expect("feed");
    completed(stream.finish().expect("finish"));
    assert_eq!(record.pages, batch_pages(html));
}

#[test]
fn rebuilding_page_count_margin_boxes_stops_on_abort() {
    let html = format!(
        "<style>@page {{ margin: 40px; @bottom-center {{ content: counter(pages) }} }}</style>{}",
        paragraphs(60)
    );
    let resources = RenderResources::new();
    let doc = crate::parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    let pages = laid_out.page_count();
    let (rebuilt, running) = page_count_margin_boxes(&laid_out, pages, None)
        .expect("layout")
        .expect("not aborted");
    assert_eq!(rebuilt.len(), pages as usize);
    assert!(running.is_empty());
    let controller = AbortController::new();
    controller.abort();
    assert!(
        page_count_margin_boxes(&laid_out, pages, Some(&controller.signal))
            .expect("layout")
            .is_none()
    );
}

#[test]
fn rebuilt_margin_boxes_come_with_their_running_elements() {
    let html = format!(
        "<style>h1 {{ position: running(title) }} \
         @page {{ margin: 40px; @top-left {{ content: element(title) }} \
           @bottom-center {{ content: counter(pages) }} }}</style><h1>Title</h1>{}",
        paragraphs(60)
    );
    let (pages, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    let rebuilt = &summary.page_count_margin_boxes;
    assert_eq!(rebuilt.len(), early);
    assert_eq!(summary.page_count_running_elements.len(), early);
    for element in &summary.page_count_running_elements {
        let (_, boxes) = rebuilt
            .iter()
            .find(|(index, _)| *index == element.page)
            .expect("rebuilt page");
        let margin_box = &boxes[element.margin_box];
        let running = margin_box.running.as_ref().expect("running box");
        assert_eq!(element.layout.width(), running.content_box.width);
        assert!(element.layout.height() > 0.0);
    }
    assert!(pages.len() > early);
}

/// Serves `main.css`, which imports `nested.css`, counting the requests.
#[derive(Default)]
struct SheetProvider {
    requests: std::sync::Mutex<Vec<String>>,
}

impl NetworkProvider for SheetProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.requests
            .lock()
            .unwrap()
            .push(request.url.path().to_owned());
        let css = match request.url.path() {
            "/main.css" => "@import 'nested.css'; p { color: green }",
            "/nested.css" => "p { margin: 4px 0 }",
            _ => return Err(NetworkError::Other("not found".to_owned())),
        };
        Ok(FetchOutcome::Body(
            FetchedResource::new(bytes::Bytes::from(css), request.url)
                .content_type(Some("text/css".to_owned())),
        ))
    }
}

#[test]
fn style_sheets_are_fetched_once_per_stream() {
    let provider = SheetProvider::default();
    let resources = RenderResources::new()
        .network_provider(&provider)
        .base_url(url::Url::parse("https://page.example/").unwrap());
    let html = format!(
        "<head><link rel=stylesheet href=main.css></head><body>{}",
        paragraphs(60)
    );
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut record,
    )
    .checkpoint_bytes(1000);
    stream.feed(html.as_bytes()).expect("feed");
    completed(stream.finish().expect("finish"));
    assert!(record.pages.len() > 1);
    let mut requests = provider.requests.lock().unwrap().clone();
    requests.sort();
    assert_eq!(requests, ["/main.css", "/nested.css"]);
}

#[test]
fn late_absolute_elements_on_delivered_pages_are_skipped() {
    let stamp = "<div style='position: absolute; top: 0'>stamp</div>";
    let html = format!(
        "<body style='position: relative'>{}{stamp}<p>end</p>",
        paragraphs(60)
    );
    let (pages, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    let ignored = ignored(&summary);
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    assert!(ignored[0].1.contains("position: absolute"));
    // Batch layout draws the element on the first page, which the stream
    // delivered before the element arrived.
    let stamp = ignored[0].0.expect("node");
    assert!(pages.iter().all(|(_, nodes)| !nodes.contains(&stamp)));
    assert!(batch_pages(&html)[0].1.contains(&stamp));
}

#[test]
fn late_absolute_elements_on_later_pages_are_kept() {
    let html = format!(
        "{}<div style='position: absolute'>here</div><p>end</p>",
        paragraphs(60)
    );
    let (pages, early, summary) = stream_with_summary(&html, 1000, 1);
    assert!(early > 0);
    assert!(ignored(&summary).is_empty());
    assert_eq!(pages, batch_pages(&html));
}

/// Consumer property events a sink saw, shared with the test.
#[derive(Clone, Default)]
struct SharedEvents(std::rc::Rc<std::cell::RefCell<Vec<ConsumerPropertyEvent>>>);

impl PageSink for SharedEvents {
    type Output = ();

    fn page(
        &mut self,
        _page: StreamPage<'_>,
        events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        self.0.borrow_mut().extend(events);
        Ok(())
    }

    fn finish(self, _summary: StreamSummary) -> std::io::Result<()> {
        Ok(())
    }
}

fn property_event(node: usize, value: ConsumerPropertyValue) -> ConsumerPropertyEvent {
    ConsumerPropertyEvent::new(
        NodeId(node as u64),
        None,
        node as u32,
        "bookmark-label",
        value,
    )
}

#[test]
fn the_text_events_of_open_elements_are_the_ones_listed_in_order() {
    let text = |node| property_event(node, ConsumerPropertyValue::Text(String::new()));
    let events = [
        text(9),
        text(4),
        // Not a text value, and an element that is not open.
        property_event(4, ConsumerPropertyValue::Integer(1)),
        text(7),
        text(4),
    ];
    assert_eq!(text_event_sources_among(vec![4, 5, 9], &events), [9, 4, 4]);
    assert_eq!(
        text_event_sources_among(Vec::new(), &events),
        Vec::<usize>::new()
    );
}

#[test]
fn the_text_events_of_a_deep_stack_of_open_elements_are_found_in_linear_time() {
    let depth = 100_000;
    let open: Vec<usize> = (1..=depth).collect();
    let events: Vec<ConsumerPropertyEvent> = open
        .iter()
        .map(|&node| property_event(node, ConsumerPropertyValue::Text(String::new())))
        .collect();
    let start = std::time::Instant::now();
    let sources = text_event_sources_among(open.clone(), &events);
    let elapsed = start.elapsed();
    assert_eq!(sources, open);
    // Looking each event up among the open elements takes milliseconds;
    // scanning them for every event takes seconds at this depth.
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "{elapsed:?} for {depth} open elements"
    );
}

#[test]
fn text_events_of_open_elements_wait_for_their_content() {
    let open = format!(
        "<section style='bookmark-label: content()'>{}",
        paragraphs(60)
    );
    let html = format!("{open}<p>last words</p></section>");
    let resources = RenderResources::new();
    let registrations = [ConsumerPropertyRegistration::text("bookmark-label")];
    let shared = SharedEvents::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        shared.clone(),
    )
    .consumer_properties(&registrations)
    .checkpoint_bytes(open.len());
    stream.feed(open.as_bytes()).expect("feed");
    assert!(shared.0.borrow().is_empty(), "the section is still open");
    stream.feed(&html.as_bytes()[open.len()..]).expect("feed");
    assert!(matches!(
        stream.finish().expect("finish"),
        StreamStatus::Completed(())
    ));
    let events = shared.0.borrow();
    let [event] = events.as_slice() else {
        panic!("one event: {events:?}");
    };
    let ConsumerPropertyValue::Text(text) = &event.value else {
        panic!("text value: {event:?}");
    };
    assert!(text.ends_with("last words"), "{text:?}");
}

#[test]
fn rejected_table_prefix_replays_with_original_page_and_consumer_events() {
    use crate::sink::row_store::TableInput;
    let html = format!("<table><tr><td>一<tr><td rowspan=2>b</tr></table>{PAGED}");
    let mut direct = Record::default();
    let expected = completed(stream(&html, 7, &mut direct, LayoutConfig::default()).unwrap());
    let mut probe = TableInput::new(&Default::default()).unwrap();
    let mut chunks = html.as_bytes().chunks(7);
    assert!(chunks.by_ref().any(|chunk| !probe.feed(chunk).unwrap()));
    let resources = RenderResources::new();
    let mut recovered = Record::default();
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut fallback = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        &mut recovered,
    )
    .consumer_properties(&registrations);
    std::io::copy(&mut probe.into_replay().unwrap(), &mut fallback).unwrap();
    for chunk in chunks {
        fallback.feed(chunk).unwrap();
    }
    let actual = completed(fallback.finish().unwrap());
    assert_eq!(actual.page_count, expected.page_count);
    assert_eq!(recovered.pages, direct.pages);
    assert_eq!(recovered.events, direct.events);
    assert_eq!(
        actual.anchors.get("c").unwrap().page_index,
        expected.anchors.get("c").unwrap().page_index
    );
}

#[test]
fn table_window_delivery_observes_document_limit_abort_and_sink_failure() {
    use crate::sink::row_store::TableInput;
    let css = "<style>@page{size:240px 150px;margin:0}body{margin:0}table{border-spacing:0}tr{break-inside:avoid}td{height:22px}</style>";
    let html = format!("{css}<table>{}</table>", "<tr><td>x".repeat(47));
    let limits = RenderLimits::default();
    let options = Default::default();
    let mut input = TableInput::new(&limits).unwrap();
    input.feed(html.as_bytes()).unwrap();
    let mut windows = input.finish_windows(&options).unwrap().0.unwrap();
    let assemble =
        |shell| crate::document_parse::assemble_document(shell, &options, &limits, None, None);
    let render = |doc: &crate::HtmlDocument| {
        layout(
            doc,
            Default::default(),
            Default::default(),
            LayoutOptions::new(),
        )
    };
    let plan = windows
        .plan_pages(std::num::NonZeroUsize::new(14).unwrap(), |shell| {
            let doc = assemble(shell)?;
            let LayoutStatus::Completed(result) = render(&doc)? else {
                panic!("unexpected abort")
            };
            Ok((doc, result))
        })
        .unwrap()
        .unwrap();
    assert!(plan.len() > 1);
    let mut output =
        |record: &mut Record, max_pages, signal: Option<&raikiri_traits::AbortSignal>| {
            deliver_table_windows(
                &mut &mut *record,
                &mut windows,
                &plan,
                max_pages,
                signal,
                |shell| render(&assemble(shell)?),
            )
        };
    let mut record = Record::default();
    assert!(matches!(
        output(&mut record, Some(1), None),
        Err(RenderError::LimitExceeded {
            kind: LimitKind::Pages,
            ..
        })
    ));
    assert!(record.pages.is_empty());
    let controller = AbortController::new();
    let signal = controller.signal.clone();
    record.abort_on_page = Some((1, controller));
    assert!(matches!(
        output(&mut record, None, Some(&signal)),
        Ok(StreamStatus::Aborted)
    ));
    assert_eq!(record.pages.len(), 2);
    let mut record = Record {
        fail_on_page: Some(2),
        ..Default::default()
    };
    assert!(matches!(
        output(&mut record, None, None),
        Err(RenderError::Io(_))
    ));
    assert_eq!(record.pages.len(), 2);
    let mut record = Record::default();
    let Ok(StreamStatus::Completed(count)) = output(&mut record, None, None) else {
        panic!("delivery failed")
    };
    assert_eq!(
        record.pages.iter().map(|page| page.0).collect::<Vec<_>>(),
        (0..count).collect::<Vec<_>>()
    );

    let mut record = Record::default();
    let interrupted =
        deliver_table_windows(&mut &mut record, &mut windows, &plan, None, None, |_| {
            Ok(LayoutStatus::Aborted)
        })
        .unwrap();
    assert!(matches!(interrupted, StreamStatus::Aborted));
    assert!(record.pages.is_empty());

    let mut invalid_plan = plan;
    invalid_plan[0].pages.end += 10;
    let result = deliver_table_windows(
        &mut &mut record,
        &mut windows,
        &invalid_plan,
        None,
        None,
        |shell| render(&assemble(shell)?),
    );
    assert!(matches!(result, Err(RenderError::Configuration(_))));
    assert!(record.pages.is_empty());
}

#[test]
fn staged_table_windows_can_be_retained_or_replayed_from_page_zero() {
    let html = format!("<!doctype html><table>{}</table>", "<tr><td>row".repeat(53));
    let css = "@page{size:240px 150px;margin:0}body{margin:0}table{border-spacing:0}tr{break-inside:avoid}td{height:23px}";
    let resources = RenderResources::new().stylesheet(css);
    for limit in [3, u32::MAX] {
        let mut record = Record {
            stage_before: Some(limit),
            ..Default::default()
        };
        let mut stream = StreamingLayout::new(
            &resources,
            Default::default(),
            Default::default(),
            &mut record,
        );
        let mut input = crate::sink::row_store::TableInput::new(&resources.parse_limits()).unwrap();
        input.feed(html.as_bytes()).unwrap();
        let extra = resources.extra_stylesheets();
        let options = resources.parse_options(&extra, None);
        let mut windows = input.finish_windows(&options).unwrap().0.unwrap();
        let StreamStatus::Completed(Some(count)) = stream_table_windows(
            &stream.settings,
            &mut windows,
            std::num::NonZeroUsize::new(18).unwrap(),
            &mut stream.sink,
        )
        .unwrap() else {
            panic!("window plan was rejected")
        };
        assert!(count > 3);
        let expected: Vec<_> = (0..count).collect();
        if limit < count {
            assert_eq!(record.discarded, limit as usize);
            assert!(record.staged.is_empty());
            assert_eq!(
                record.pages.iter().map(|entry| entry.0).collect::<Vec<_>>(),
                expected
            );
        } else {
            assert_eq!(record.staged, expected);
            assert!(record.pages.is_empty());
        }
    }
}

#[test]
fn an_io_error_on_the_first_staged_page_discards_partial_sink_state() {
    let html = format!(
        "<!doctype html><style>td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{}</table>",
        " ".repeat(54000),
        "<tr><td>short</td></tr>".repeat(600)
    );
    let mut record = Record {
        stage_before: Some(u32::MAX),
        fail_on_page: Some(0),
        ..Default::default()
    };
    assert!(matches!(
        stream(&html, 8192, &mut record, Default::default()),
        Err(RenderError::Io(_))
    ));
    assert_eq!(record.discarded, 1);
    assert!(record.staged.is_empty());
    assert!(record.pages.is_empty());
    assert!(!record.finished);
}

#[test]
fn a_late_split_row_discards_staged_pages_before_public_stream_replay() {
    let html = format!(
        "<!doctype html><style>@page{{size:180px 140px;margin:0}}body{{margin:0}}table{{width:120px;border-spacing:0}}td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{}<tr><td>{}</table>",
        " ".repeat(54000),
        "<tr><td>short</td></tr>".repeat(600),
        "word ".repeat(400)
    );
    let expected = batch_pages(&html);
    let mut record = Record {
        stage_before: Some(u32::MAX),
        ..Default::default()
    };
    let summary = completed(stream(&html, 8192, &mut record, Default::default()).unwrap());
    assert!(record.discarded > 0);
    assert!(record.staged.is_empty());
    assert_eq!(record.pages, expected);
    assert_eq!(summary.page_count as usize, expected.len());
}

#[test]
fn window_passes_use_the_streams_configured_stylesheet() {
    let html = format!("<!doctype html><table>{}</table>", "<tr><td>row".repeat(53));
    let css = "@page{size:240px 150px;margin:0}body{margin:0;font-family:Ahem;font-size:12px;line-height:16px}table{border-spacing:0}tr{break-inside:avoid}td{height:23px}";
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
            ))
            .to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts).stylesheet(css);
    let new_record = || Record {
        expected_document: Some(
            crate::parse_html_with_resources(html.as_bytes(), &resources).unwrap(),
        ),
        ..Default::default()
    };
    let document = crate::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(reference) = layout(
        &document,
        Default::default(),
        Default::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("batch layout aborted")
    };
    let pages: PageLog = reference
        .pages()
        .map(|p| (p.index(), p.fragments().map(|f| f.node()).collect()))
        .collect();
    let rects: Vec<Vec<_>> = reference
        .pages()
        .map(|p| p.fragments().map(|f| f.rect()).collect())
        .collect();
    let paint_lines: Vec<_> = reference.pages().map(painted_line_roots).collect();
    assert!(pages.len() > 3);
    assert_eq!(paint_lines.iter().map(Vec::len).sum::<usize>(), 53);

    for width in [14, 18, 31] {
        let mut record = new_record();
        let mut stream = StreamingLayout::new(
            &resources,
            Default::default(),
            Default::default(),
            &mut record,
        );
        let mut input = crate::sink::row_store::TableInput::new(&resources.parse_limits()).unwrap();
        input.feed(html.as_bytes()).unwrap();
        let extra = resources.extra_stylesheets();
        let options = resources.parse_options(&extra, None);
        let mut windows = input.finish_windows(&options).unwrap().0.unwrap();
        let result = stream_table_windows(
            &stream.settings,
            &mut windows,
            std::num::NonZeroUsize::new(width).unwrap(),
            &mut stream.sink,
        )
        .unwrap();
        assert!(
            matches!(result, StreamStatus::Completed(Some(count)) if count as usize == pages.len())
        );
        assert_eq!(record.fragment_rects, rects, "window width {width}");
        assert_eq!(record.pages, pages, "window width {width}");
        assert_eq!(record.paint_lines, paint_lines, "window width {width}");
    }
}

#[test]
fn public_html_stream_automatically_delivers_large_table_windows() {
    let mut html = String::from(
        "<!doctype html><style>@page{size:320px 420px;margin:0}body{margin:0}table{border-spacing:0}td{height:20px}tr{break-inside:avoid}</style><table>",
    );
    for row in 0..1400 {
        use std::fmt::Write;
        write!(html, "<tr><td>{row}</td><td>table window content</td></tr>").unwrap();
    }
    html.push_str("</table>");
    let resources = RenderResources::new()
        .stylesheet("@layer { :where(html) { font-family: sans-serif } } h1 { bookmark-level: 1 }");
    let mut record = Record {
        expected_document: Some(
            crate::parse_html_with_resources(html.as_bytes(), &resources).unwrap(),
        ),
        ..Default::default()
    };
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    );
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    stream = stream.consumer_properties(&registrations);
    for chunk in html.as_bytes().chunks(3001) {
        stream.feed(chunk).unwrap();
    }
    assert!(stream.table_input.is_some());
    let summary = completed(stream.finish().unwrap());
    assert!(summary.page_count > 2);
    assert!(record.offered_table_pages > 0);
    #[cfg(feature = "streaming-diagnostics")]
    assert_eq!(
        summary.diagnostics.table_window_pages, summary.page_count,
        "{:?}",
        summary.warnings
    );
    assert_eq!(record.pages.len(), summary.page_count as usize);
    let document = record.expected_document.take().unwrap();
    let LayoutStatus::Completed(reference) = layout(
        &document,
        Default::default(),
        Default::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("reference layout aborted")
    };
    assert_eq!(reference.pages().len(), record.pages.len());
    for (page, (index, actual)) in reference.pages().zip(&record.pages) {
        assert_eq!(page.index(), *index);
        assert_eq!(
            painted_line_roots(page),
            record.paint_lines[*index as usize]
        );
        let expected: Vec<_> = page.fragments().map(|fragment| fragment.node()).collect();
        assert_eq!(expected.len(), actual.len(), "page {index}");
        for (position, (expected, actual)) in expected.iter().zip(actual).enumerate() {
            assert_eq!(expected, actual, "page {index}, fragment {position}");
        }
    }
}

#[test]
fn failing_public_table_sink_does_not_interrupt_another_document() {
    let ready = std::sync::Barrier::new(2);
    std::thread::scope(|threads| {
        let run = |fail: bool| {
            let text = if fail { "discarded" } else { "retained" };
            let html = format!(
                "<!doctype html><style>@page{{size:240px 150px;margin:0}}body{{margin:0}}table{{border-spacing:0}}tr{{break-inside:avoid}}td{{height:21px}}</style><!--{}--><table>{}</table>",
                " ".repeat(54000),
                format!("<tr><td>{text}</td><td>{text}</td></tr>").repeat(600)
            );
            let resources = RenderResources::new();
            let mut record = Record {
                fail_on_page: fail.then_some(1),
                ..Default::default()
            };
            let mut stream = StreamingLayout::new(
                &resources,
                Default::default(),
                Default::default(),
                &mut record,
            );
            stream.feed(html.as_bytes()).unwrap();
            let selected = stream.table_input.is_some();
            ready.wait();
            assert!(selected);
            let result = stream.finish();
            if fail {
                assert!(matches!(result, Err(RenderError::Io(_))));
                assert_eq!(record.pages.len(), 1);
                assert!(!record.finished);
            } else {
                let summary = completed(result.unwrap());
                assert!(record.finished);
                assert!(summary.page_count > 2);
                assert!(record.offered_table_pages > 0);
                #[cfg(feature = "streaming-diagnostics")]
                assert_eq!(summary.diagnostics.table_window_pages, summary.page_count);
                assert_eq!(record.pages.len(), summary.page_count as usize);
            }
        };
        let failing = threads.spawn(move || run(true));
        run(false);
        failing.join().unwrap();
    });
}

#[test]
fn aborting_at_table_page_and_window_boundaries_preserves_only_final_output() {
    let html = format!(
        "<!doctype html><style>@page{{size:240px 150px;margin:0}}body{{margin:0}}td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{}</table>",
        " ".repeat(54000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let pages = batch_pages(&html);
    let last = pages.last().unwrap().0;
    assert!(last > 1);
    let first_window_end = html.match_indices("</tr>").nth(511).unwrap().0 + "</tr>".len();
    let first_window = batch_pages(&format!("{}</table>", &html[..first_window_end]));
    // The planner withholds the trailing page until it has another window.
    let last_staged_before_next_window = first_window[first_window.len() - 2].0;
    assert!((1..last).contains(&last_staged_before_next_window));
    for (index, staging) in [
        (0, true),
        (last_staged_before_next_window, true),
        (last, true),
        (0, false),
        (last, false),
    ] {
        let controller = AbortController::new();
        let mut config = LayoutConfig::default();
        config.signal = Some(controller.signal.clone());
        let mut record = Record {
            stage_before: staging.then_some(u32::MAX),
            abort_on_page: Some((index, controller)),
            ..Default::default()
        };
        assert!(matches!(
            stream(&html, 8192, &mut record, config).unwrap(),
            StreamStatus::Aborted
        ));
        let delivered = index as usize + 1;
        if staging {
            assert_eq!(record.discarded, delivered);
            assert!(record.pages.is_empty());
        } else {
            assert_eq!(record.discarded, 0);
            assert_eq!(record.pages, pages[..delivered]);
        }
        assert!(record.staged.is_empty());
        assert!(!record.finished);
    }
}

#[test]
fn row_markers_in_a_comment_do_not_select_window_layout() {
    let html = format!(
        "<!doctype html><!--{}{}--><table><tr><td>row</td></tr></table><p id=end>after</p>",
        "<tr>".repeat(128),
        " ".repeat(CHECKPOINT_BYTES)
    );
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    );
    stream.feed(html.as_bytes()).unwrap();
    assert!(stream.table_input.is_none());
    let summary = completed(stream.finish().unwrap());
    assert_eq!(record.pages, batch_pages(&html));
    assert_eq!(record.offered_table_pages, 0);
    assert_eq!(summary.anchors.get("end").unwrap().page_index, 0);
}

#[test]
fn a_stray_end_tag_preserves_parse_warnings_when_replaying_the_table() {
    let html = format!(
        "<!doctype html><style>@page{{size:240px 150px;margin:0}}body{{margin:0}}td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{}</table></span>",
        " ".repeat(54000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let resources = RenderResources::new();
    let reference = crate::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    );
    stream.feed(html.as_bytes()).unwrap();
    assert!(stream.table_input.is_some());
    let summary = completed(stream.finish().unwrap());
    #[cfg(feature = "streaming-diagnostics")]
    assert_eq!(
        summary.diagnostics.table_plans, 1,
        "{:?}",
        summary.diagnostics
    );
    let messages = |warnings: &[RenderWarning]| {
        warnings
            .iter()
            .filter_map(|warning| match &warning.kind {
                WarningKind::HtmlParseError { message } => Some(message.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let expected = messages(&reference.uncascaded.warnings);
    assert!(!expected.is_empty());
    assert_eq!(messages(&summary.warnings), expected);
    assert_eq!(record.offered_table_pages, 0);
    assert_eq!(record.pages, batch_pages(&html));
}

#[test]
fn content_after_a_complete_table_is_replayed_with_its_anchor() {
    let html = format!(
        "<!doctype html><!--{}--><table>{}</table><p id=end>after</p>",
        " ".repeat(54000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    );
    stream.feed(html.as_bytes()).unwrap();
    assert!(stream.table_input.is_some());
    let summary = completed(stream.finish().unwrap());
    let expected = batch_pages(&html);
    assert_eq!(record.pages, expected);
    assert_eq!(record.offered_table_pages, 0);
    assert_eq!(
        summary.anchors.get("end").unwrap().page_index,
        expected.last().unwrap().0
    );
}

#[test]
fn unsupported_row_after_table_selection_replays_before_reading_the_suffix() {
    let prefix = format!(
        "<!doctype html><!--{}--><table>{}",
        " ".repeat(54000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let suffix = "<tr class=late><td>終</td></tr></table><p id=end>after</p>";
    let resources = RenderResources::new();
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    );
    stream.feed(prefix.as_bytes()).unwrap();
    assert!(stream.table_input.is_some());
    for byte in suffix.as_bytes() {
        stream.feed(std::slice::from_ref(byte)).unwrap();
    }
    assert!(stream.table_input.is_none());
    let summary = completed(stream.finish().unwrap());
    let expected = batch_pages(&format!("{prefix}{suffix}"));
    assert_eq!(record.pages, expected);
    assert_eq!(record.offered_table_pages, 0);
    assert_eq!(
        summary.anchors.get("end").unwrap().page_index,
        expected.last().unwrap().0
    );
}

#[test]
fn sinks_without_staging_receive_final_table_pages_once_in_order() {
    for rule in ["", "tr:nth-child(odd) td {height:32px}"] {
        let html = format!(
            "<!doctype html><style>@page{{size:200px 150px;margin:0}}td{{height:18px}}{rule}</style><!--{}--><table>{}</table>",
            " ".repeat(54000),
            "<tr><td>row</td></tr>".repeat(600),
        );
        let resources = RenderResources::new();
        let sink = Shared::default();
        let mut stream = StreamingLayout::new(
            &resources,
            Default::default(),
            Default::default(),
            sink.clone(),
        );
        stream.feed(html.as_bytes()).unwrap();
        assert!(stream.table_input.is_some());
        assert!(sink.0.borrow().is_empty());
        let summary = completed(stream.finish().unwrap());
        let reference = batch_pages(&html);
        assert_eq!(summary.page_count as usize, reference.len());
        assert_eq!(*sink.0.borrow(), reference, "{rule}");
    }
}

#[test]
fn registered_row_events_require_replay_before_any_table_page_is_staged() {
    let html = format!(
        "<!doctype html><style>tr{{bookmark-level:2}}</style><!--{}--><table>{}</table>",
        " ".repeat(56000),
        "<tr><td>x</td></tr>".repeat(600)
    );
    let resources = RenderResources::new();
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut record = Record {
        stage_before: Some(u32::MAX),
        ..Default::default()
    };
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    )
    .consumer_properties(&registrations);
    stream.feed(html.as_bytes()).unwrap();
    assert!(stream.table_input.is_some());
    let summary = completed(stream.finish().unwrap());
    assert_eq!(record.offered_table_pages, 0);
    let document = crate::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let dom = document.dom();
    let rows: Vec<_> = dom
        .preorder(dom.root_index())
        .filter(|&id| dom.get_node(id).unwrap().tag_name() == Some("tr"))
        .map(|id| NodeId(id as u64))
        .collect();
    assert_eq!(rows.len(), 600);
    assert_eq!(
        record.events.into_iter().flatten().collect::<Vec<_>>(),
        rows
    );
    assert_eq!(record.pages.len(), summary.page_count as usize);
}

#[test]
fn window_style_rule_limit_fails_before_page_delivery() {
    let html = format!(
        "<!doctype html><style>td{{height:20px}}</style><!--{}--><table>{}</table>",
        " ".repeat(54000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let resources = RenderResources::new()
        .render_limits(RenderLimits::builder().max_style_rules(Some(1)).build());
    let mut record = Record::default();
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut record,
    );
    stream.feed(html.as_bytes()).unwrap();
    assert!(stream.table_input.is_some());
    let error = stream.finish().unwrap_err();
    assert!(
        matches!(error, RenderError::LimitExceeded { kind: LimitKind::StyleRules, limit: 1, actual } if actual > 1)
    );
    assert_eq!(record.offered_table_pages, 0);
    assert!(record.pages.is_empty());
    assert!(!record.finished);
}

#[test]
fn document_page_limit_discards_staged_table_pages_before_returning_error() {
    let html = format!(
        "<!doctype html><style>@page{{size:240px 150px;margin:0}}td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{}</table>",
        " ".repeat(54000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let mut config = LayoutConfig::default();
    let total = batch_pages(&html).len();
    let limit = u32::try_from(total - 1).unwrap();
    config.limits.max_document_pages = Some(limit);
    let mut record = Record {
        stage_before: Some(u32::MAX),
        ..Default::default()
    };
    let result = stream(&html, 8192, &mut record, config);
    assert!(
        matches!(result, Err(RenderError::LimitExceeded { kind: LimitKind::Pages, limit: reported, actual }) if reported == u64::from(limit) && actual == total as u64)
    );
    assert_eq!(record.offered_table_pages, limit as usize);
    assert_eq!(record.discarded, limit as usize);
    assert!(record.pages.is_empty());
    assert!(record.staged.is_empty());
    assert!(!record.finished);
}

#[test]
fn projected_generated_box_uses_source_ids_for_its_clip_and_text_line() {
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
            ))
            .to_vec(),
        )
        .build()
        .unwrap();
    struct Pixels;
    impl raikiri_traits::ImagePixelSource for Pixels {
        fn get_decoded(
            &self,
            _: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            Some(std::sync::Arc::new(raikiri_traits::DecodedImage::new(
                1,
                1,
                vec![0, 0, 0, 255],
            )))
        }
    }
    let resources = RenderResources::new()
        .fonts(fonts)
        .image_pixel_source(&Pixels);
    let mut doc = crate::parse_html_with_resources(b"<!doctype html><style>body{font:20px/20px Ahem}div::before{content:'B';background:red}div::after{content:url(https://example.test/x.png) / 'label'}p::first-line{color:red}span{border:1px solid currentColor}em{opacity:.5}li{list-style-image:url(https://example.test/marker.png)}article{width:100px;column-count:2;column-gap:20px;column-rule:2px solid red;orphans:1;widows:1}</style><div>A</div><p><span>B</span></p><section><em>C</em></section><ul><li>D</li></ul><article>A<br>B<br>C<br>D</article><table><tr style='box-shadow:2px 2px red;background:blue'><td>E</td></tr></table>".as_slice(), &resources).unwrap();
    let source_ids = (0..doc.dom().node_count()).map(|id| id + 1000).collect();
    doc.uncascaded.dom.set_window_source_ids(source_ids);
    let LayoutStatus::Completed(layout) = layout(
        &doc,
        Default::default(),
        Default::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout aborted")
    };
    let owners = |page: crate::Page<'_>| {
        page.paint_order_for_text_runs(&page.text_runs())
            .into_iter()
            .find_map(|event| match event {
                crate::PaintEvent::GeneratedBox(piece) => {
                    Some([piece.owner.0, piece.clip_owner.0, piece.line.root.0])
                }
                _ => None,
            })
            .unwrap()
    };
    let local = owners(layout.page(0).unwrap());
    let source = owners(StreamPage::from_window(&layout, 0, 0, 1).page());
    assert_eq!(source, local.map(|id| id + 1000));
    let inline_owners = |page: crate::Page<'_>| {
        let mut pieces = Vec::new();
        let mut groups = Vec::new();
        for event in page.paint_order_for_text_runs(&page.text_runs()) {
            match event {
                crate::PaintEvent::LineBox(piece) => {
                    pieces.push([piece.fragment.node().0, piece.line.root.0])
                }
                crate::PaintEvent::GroupTextLine(line, owner) => {
                    groups.push([line.root.0, owner.0])
                }
                _ => {}
            }
        }
        [pieces, groups]
    };
    let expected = inline_owners(layout.page(0).unwrap()).map(|items| {
        assert!(!items.is_empty());
        items
            .into_iter()
            .map(|ids| ids.map(|id| id + 1000))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        inline_owners(StreamPage::from_window(&layout, 0, 0, 1).page()),
        expected
    );
    let decoration_owners = |page: crate::Page<'_>| {
        let mut marker = None;
        let mut column = None;
        let mut shadow = None;
        for event in page.paint_order() {
            match event {
                crate::PaintEvent::MarkerImage(owner) => {
                    let raster = page.raster_marker(owner, &Pixels).expect("marker pixels");
                    marker = Some([owner.0, raster.node.0, raster.clip_owner.unwrap().0]);
                }
                crate::PaintEvent::OuterBoxShadow(fragment) => shadow = Some(fragment.node().0),
                crate::PaintEvent::ColumnRule(rule) => column = Some(rule.owner.0),
                _ => {}
            }
        }
        let [owner, image, clip] = marker.unwrap();
        [owner, image, clip, column.unwrap(), shadow.unwrap()]
    };
    assert_eq!(
        decoration_owners(StreamPage::from_window(&layout, 0, 0, 1).page()),
        decoration_owners(layout.page(0).unwrap()).map(|id| id + 1000)
    );
    let image_owners = |page: crate::Page<'_>| {
        let image = page
            .paint_order_for_text_runs(&page.text_runs())
            .into_iter()
            .find_map(|event| match event {
                crate::PaintEvent::GeneratedImage(image) => Some(image),
                _ => None,
            })
            .unwrap();
        let mut missing = image;
        missing.index += 1;
        assert!(page.generated_image_text_position(missing).is_none());
        let (line, _) = page.generated_image_text_position(image).unwrap();
        let raster = page.raster_generated_image(image, &Pixels).unwrap();
        assert_eq!(
            page.generated_alt_text(image.owner, image.kind).as_deref(),
            Some("label")
        );
        [
            image.owner.0,
            line.root.0,
            raster.node.0,
            raster.clip_owner.unwrap().0,
        ]
    };
    assert_eq!(
        image_owners(StreamPage::from_window(&layout, 0, 0, 1).page()),
        image_owners(layout.page(0).unwrap()).map(|id| id + 1000)
    );
}

#[test]
fn error_finalizing_the_table_sink_reaches_the_public_caller() {
    let html = format!(
        "<!doctype html><!--{}--><table>{}</table>",
        " ".repeat(55000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let mut record = Record {
        fail_finish: true,
        ..Default::default()
    };
    let error = stream(&html, 8192, &mut record, Default::default()).unwrap_err();
    assert!(
        matches!(error, RenderError::Io(error) if error.to_string() == "output finalization failed")
    );
    assert!(record.offered_table_pages > 0);
    assert!(record.pages.len() > 1);
    assert!(!record.finished);
}

#[test]
fn table_page_count_footers_skip_staging_and_use_the_final_count() {
    struct Footers(Vec<String>);
    impl PageSink for &mut Footers {
        type Output = ();
        fn stage_table_page(&mut self, _: StreamPage<'_>) -> std::io::Result<bool> {
            panic!("page-count footer reached tentative painting");
        }
        fn page(
            &mut self,
            page: StreamPage<'_>,
            _: Vec<ConsumerPropertyEvent>,
        ) -> std::io::Result<()> {
            self.0.push(page.page().margin_boxes()[0].content.clone());
            Ok(())
        }
        fn finish(self, _: StreamSummary) -> std::io::Result<()> {
            Ok(())
        }
    }
    let html = format!(
        "<!doctype html><style>@page{{size:240px 150px;margin:12px;@bottom-center{{content:counter(page) '/' counter(pages)}}}}td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{}</table>",
        " ".repeat(55000),
        "<tr><td>row</td></tr>".repeat(600)
    );
    let resources = RenderResources::new();
    let mut sink = Footers(Vec::new());
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut sink,
    );
    stream.feed(html.as_bytes()).unwrap();
    assert!(stream.table_input.is_some());
    assert!(matches!(
        stream.finish().unwrap(),
        StreamStatus::Completed(())
    ));
    let total = batch_pages(&html).len();
    assert!(total > 2);
    assert_eq!(
        sink.0,
        (1..=total)
            .map(|i| format!("{i}/{total}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn window_assembly_preserves_limits_in_the_requested_media_context() {
    let html = format!(
        "<style>@media screen {{{}}}</style><table><tr><td>x</table>",
        "td {color:blue}".repeat(128)
    );
    let resources = RenderResources::new();
    let options = Default::default();
    let assemble = |context, limits: &RenderLimits| {
        let shell = crate::parse(html.as_bytes(), &options).unwrap();
        assemble_resolved(&resources, &context, shell, &options, limits)
    };
    let mut limits = RenderLimits::default();
    limits.max_dom_nodes = Some(1);
    assert!(matches!(
        assemble(Default::default(), &limits),
        Err(RenderError::LimitExceeded {
            kind: LimitKind::DomNodes,
            ..
        })
    ));
    let mut limits = RenderLimits::default();
    limits.max_cascade_candidates_per_element = Some(64);
    assert!(assemble(raikiri_style::MediaContext::print(), &limits).is_ok());
    assert!(matches!(
        assemble(raikiri_style::MediaContext::screen(), &limits),
        Err(RenderError::LimitExceeded {
            kind: LimitKind::CascadeCandidatesPerElement,
            limit: 64,
            ..
        })
    ));
}

fn sample_table_html() -> String {
    let rows: String = (0..600)
        .map(|i| format!("<tr><td>row &amp; {i}</td></tr>"))
        .collect();
    format!(
        "<!doctype html><style>@page{{size:240px 150px;margin:0}}td{{height:22px}}tr{{break-inside:avoid}}</style><!--{}--><table>{rows}</table>",
        " ".repeat(55000)
    )
}

#[test]
fn an_incomplete_table_sample_discards_its_pages_and_replays_the_source() {
    let html = sample_table_html();
    let mut record = Record {
        inspect_sample: true,
        ..Default::default()
    };
    let summary = completed(stream(&html, 8192, &mut record, Default::default()).unwrap());
    assert_eq!(record.sample_rows, 256);
    assert_eq!(record.discarded, 1);
    assert!(record.staged.is_empty());
    assert_eq!(record.pages, batch_pages(&html));
    assert_eq!(summary.page_count as usize, record.pages.len());
}

#[test]
fn cancelling_a_table_sample_stops_row_reads_and_discards_staged_output() {
    let html = sample_table_html();
    let controller = AbortController::new();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut record = Record {
        inspect_sample: true,
        abort_on_page: Some((0, controller)),
        ..Default::default()
    };
    assert!(matches!(
        stream(&html, 8192, &mut record, config).unwrap(),
        StreamStatus::Aborted
    ));
    assert_eq!(record.sample_rows, 256);
    assert_eq!(record.discarded, 1);
    assert!(record.staged.is_empty() && record.pages.is_empty() && !record.finished);
}

#[test]
fn table_sample_respects_page_limit_and_validates_completed_count() {
    let html = sample_table_html();
    let pages = batch_pages(&html);
    let count = u32::try_from(pages.len()).unwrap();
    for (limit, reported) in [(count, count), (count, count - 1), (count - 1, count)] {
        let mut config = LayoutConfig::default();
        config.limits.max_document_pages = Some(limit);
        let mut sink = Record {
            inspect_sample: true,
            complete_sample_count: Some(reported),
            ..Default::default()
        };
        let result = stream(&html, 8192, &mut sink, config);
        if limit < count {
            assert!(matches!(result, Err(RenderError::LimitExceeded {
                kind: LimitKind::Pages, limit: cap, actual
            }) if cap == u64::from(limit) && actual == u64::from(count)));
            assert_eq!(sink.sample_rows, 0);
        } else {
            assert_eq!(completed(result.unwrap()).page_count, count);
            assert_eq!(sink.sample_rows, 600);
            if reported == count {
                assert_eq!(sink.discarded, 0);
                assert!(sink.pages.is_empty());
            } else {
                assert_eq!(sink.discarded, 1);
                assert_eq!(sink.pages, pages);
            }
        }
    }
}

#[test]
fn table_sample_rejects_rows_split_across_pages() {
    let html = sample_table_html()
        .replace("tr{break-inside:avoid}", "")
        .replacen("row &amp; 0", &"wrapped cell ".repeat(600), 1);
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new()
        .fonts(fonts)
        .stylesheet("body{font-family:Ahem;font-size:12px}");
    let document = crate::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(layout) = crate::layout(
        &document,
        Default::default(),
        Default::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout aborted");
    };
    assert!(layout.pages().map(body_rows).sum::<usize>() > 600);
    let mut sink = Record {
        inspect_sample: true,
        ..Default::default()
    };
    let mut stream = StreamingLayout::new(
        &resources,
        Default::default(),
        Default::default(),
        &mut sink,
    );
    for chunk in html.as_bytes().chunks(8192) {
        stream.feed(chunk).unwrap();
    }
    let summary = completed(stream.finish().unwrap());
    assert_eq!(sink.sample_rows, 0);
    let expected: Vec<_> = layout
        .pages()
        .map(|page| (page.index(), page.fragments().map(|f| f.node()).collect()))
        .collect();
    assert_eq!(sink.pages, expected);
    assert_eq!(summary.page_count as usize, sink.pages.len());
}

#[test]
fn cancelling_before_the_first_table_sample_reports_aborted() {
    let controller = AbortController::new();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut sink = Record {
        inspect_sample: true,
        abort_before_sample: Some(controller),
        ..Default::default()
    };
    assert!(matches!(
        stream(&sample_table_html(), 8192, &mut sink, config).unwrap(),
        StreamStatus::Aborted
    ));
    assert!(sink.pages.is_empty() && !sink.finished);
}

#[test]
fn ending_sample_needs_a_first_and_repeated_body_page() {
    assert!(ending_sample(&batch_layout("<p>short</p>"), 600).is_none());
    let layout = batch_layout(PAGED);
    assert!(layout.pages().len() >= 3);
    assert!(ending_sample(&layout, 600).is_none());
}

#[test]
fn a_sample_output_error_discards_the_tentative_page() {
    let mut sink = Record {
        inspect_sample: true,
        fail_on_page: Some(0),
        ..Default::default()
    };
    let result = stream(&sample_table_html(), 8192, &mut sink, Default::default());
    assert!(
        matches!(result, Err(RenderError::Io(error)) if error.to_string() == "sample output failed")
    );
    assert_eq!(sink.discarded, 1);
    assert!(sink.staged.is_empty() && sink.pages.is_empty() && !sink.finished);
}
