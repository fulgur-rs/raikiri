//! Focused tests for neutral page emission and completion semantics.

use raikiri::{
    AbortController, PageBox, PageDefaults, PageFragment, RenderSink, RenderStatus, RenderSummary,
    ReplacedResolver, ResolvedIntrinsic, ResolverError, ResolverRequest, StreamingConfig,
    parse_html, render_streaming,
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
