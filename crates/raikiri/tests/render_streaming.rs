//! Focused tests for neutral page emission and completion semantics.

use raikiri::{
    AbortController, PageBox, PageDefaults, PageEventObserver, PageFragment, PageFragmentEvent,
    RenderSink, RenderStatus, RenderSummary, ReplacedResolver, ResolvedIntrinsic, ResolverError,
    ResolverRequest, StreamingConfig, parse_html, render_streaming, render_streaming_with_observer,
};

struct NoopResolver;

impl ReplacedResolver for NoopResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        unreachable!("test documents contain no replaced elements")
    }
}

#[derive(Default)]
struct RecordingSink {
    pages: Vec<PageFragment>,
    summary: Option<RenderSummary>,
    fail_on_page: Option<usize>,
    abort_after_page: Option<AbortController>,
}

impl RenderSink for RecordingSink {
    fn accept_page(&mut self, page: PageFragment) -> Result<(), std::io::Error> {
        if self.fail_on_page == Some(self.pages.len()) {
            return Err(std::io::Error::other("test sink failure"));
        }
        self.pages.push(page);
        if self.pages.len() == 1
            && let Some(controller) = &self.abort_after_page
        {
            controller.abort();
        }
        Ok(())
    }

    fn finish_render(&mut self, summary: RenderSummary) -> Result<(), std::io::Error> {
        self.summary = Some(summary);
        Ok(())
    }
}

fn defaults(width: f32, height: f32) -> PageDefaults {
    let mut defaults = PageDefaults::default();
    let mut page_box = PageBox::new();
    page_box.width = width;
    page_box.height = height;
    defaults.page_box = page_box;
    defaults
}

#[test]
fn forced_break_pages_are_emitted_in_order_and_finished() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:40px">first</div><div style="break-before:page;height:10px">second</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    let status = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("render");

    let summary = match status {
        RenderStatus::Completed(summary) => summary,
        _ => panic!("forced break render should complete"),
    };
    assert_eq!(sink.pages.len(), 2);
    assert_eq!(
        sink.pages
            .iter()
            .map(|page| page.page_index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(summary.total_pages, 2);
    assert_eq!(sink.summary.expect("finish_render").total_pages, 2);
}

#[test]
fn sink_failure_stops_before_completion() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(&b"<html><body><p>hello</p></body></html>"[..], &options).expect("parse");
    let mut sink = RecordingSink {
        fail_on_page: Some(0),
        ..RecordingSink::default()
    };
    let err = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect_err("sink failure should be terminal");
    assert!(matches!(err, raikiri::RenderError::Sink(_)));
    assert!(sink.summary.is_none());
}

#[test]
fn midstream_abort_reports_partial_pages_without_completion() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:40px">first</div><div style="break-before:page;height:10px">second</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let controller = AbortController::new();
    let config = StreamingConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut sink = RecordingSink {
        abort_after_page: Some(controller),
        ..RecordingSink::default()
    };
    let status = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        &NoopResolver,
        config,
        &mut sink,
    )
    .expect("abort is a status");
    assert!(matches!(status, RenderStatus::Aborted { partial_pages: 1 }));
    assert_eq!(sink.pages.len(), 1);
    assert!(sink.summary.is_none());
}

#[derive(Default)]
struct RecordingObserver {
    events: Vec<PageFragmentEvent>,
    fail: bool,
}

impl PageEventObserver for RecordingObserver {
    fn observe_event(&mut self, event: PageFragmentEvent) -> Result<(), std::io::Error> {
        if self.fail {
            return Err(std::io::Error::other("test observer failure"));
        }
        self.events.push(event);
        Ok(())
    }
}

#[test]
fn observer_receives_page_local_link_events_after_page_emission() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br##"<html><body><a href="#target">link</a><div style="break-before:page">second</div></body></html>"##[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    let mut observer = RecordingObserver::default();
    let status = render_streaming_with_observer(
        &doc,
        defaults(100.0, 50.0),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
        &mut observer,
    )
    .expect("render");

    assert!(matches!(status, RenderStatus::Completed(_)));
    assert_eq!(sink.pages.len(), 2);
    assert_eq!(observer.events.len(), 1);
    let PageFragmentEvent::Link(event) = &observer.events[0] else {
        panic!("expected link event");
    };
    assert_eq!(event.page_index, 0);
    assert_eq!(event.link.href, "#target");
    assert!(
        sink.pages[0]
            .items
            .iter()
            .any(|item| item.node_id == event.placement_node_id && item.rect == event.rect)
    );
    assert!(sink.summary.is_some());
}

#[test]
fn observer_failure_is_a_structural_sink_error_and_skips_completion() {
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><a href="https://example.com">link</a></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    let mut observer = RecordingObserver {
        fail: true,
        ..RecordingObserver::default()
    };
    let err = render_streaming_with_observer(
        &doc,
        defaults(100.0, 50.0),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
        &mut observer,
    )
    .expect_err("observer failure should be terminal");

    assert!(matches!(err, raikiri::RenderError::Sink(_)));
    assert_eq!(sink.pages.len(), 1, "page is accepted before its events");
    assert!(sink.summary.is_none());
}

#[test]
fn render_streaming_resolves_page_geometry_from_page_contexts() {
    let stylesheet = r#"
        @page { size: 100px 100px; margin: 5px; }
        @page :first { size: 100px 120px; margin: 10px; }
        @page :right { padding: 3px; }
        @page :left { size: 120px 100px; margin: 7px; }
        @page wide { size: 200px 200px; margin: 20px; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:90px">first</div><div style="page:wide;break-before:page;height:10px">wide</div><div style="height:180px">tail</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("render");
    assert!(sink.pages.len() >= 2);
    let first = &sink.pages[0];
    assert_eq!(first.page_box.height, 120.0);
    assert_eq!(first.margins.top, 10.0);
    assert_eq!(first.content_insets.left, 3.0);

    let wide = sink
        .pages
        .iter()
        .find(|page| page.page_name.as_deref() == Some("wide"))
        .expect("named page geometry");
    assert_eq!(wide.page_box.width, 200.0);
    assert_eq!(wide.page_box.height, 200.0);
    assert_eq!(wide.margins.left, 20.0);
    assert_eq!(wide.content_insets.left, 0.0);

    let left = sink
        .pages
        .iter()
        .find(|page| page.page_box.width == 120.0)
        .expect("left-page geometry");
    assert_eq!(left.margins.left, 7.0);
    assert_eq!(left.content_box.x, 7.0);
    assert!(
        sink.pages
            .windows(2)
            .any(|pages| pages[0].page_box != pages[1].page_box)
    );
}

#[test]
fn render_streaming_resolves_named_first_page_before_layout() {
    let stylesheet = r#"
        @page { size: 100px 100px; margin: 5px; }
        @page cover { size: 130px 140px; margin: 11px; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="page:cover;height:10px">cover</div><div style="break-before:page;height:10px">next</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        &NoopResolver,
        StreamingConfig::default(),
        &mut sink,
    )
    .expect("render");

    let first = sink.pages.first().expect("first page");
    assert_eq!(first.page_name.as_deref(), Some("cover"));
    assert_eq!(first.page_box.width, 130.0);
    assert_eq!(first.page_box.height, 140.0);
    assert_eq!(first.margins.left, 11.0);
}
