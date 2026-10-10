use std::io::Write;

use raikiri_traits::{AbortController, LimitKind, ParseError, RenderLimits, WarningKind};

use super::*;
use crate::StringFetchMode;

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

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        self.finished = true;
        Ok(summary)
    }
}

fn batch_pages(html: &str) -> Vec<(u32, Vec<NodeId>)> {
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
    let firsts: Vec<NodeId> = record.pages.iter().map(|(_, nodes)| nodes[0]).collect();
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
    let rebuilt = page_count_margin_boxes(&laid_out, pages, None).expect("not aborted");
    assert_eq!(rebuilt.len(), pages as usize);
    let controller = AbortController::new();
    controller.abort();
    assert!(page_count_margin_boxes(&laid_out, pages, Some(&controller.signal)).is_none());
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
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: bytes::Bytes::from(css),
            content_type: Some("text/css".to_owned()),
            final_url: request.url,
            encoding: None,
        }))
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
