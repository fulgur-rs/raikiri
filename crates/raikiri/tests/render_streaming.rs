//! Focused tests for neutral page emission and completion semantics.

use raikiri::{
    AbortController, IntrinsicBox, LayoutConfig, PageBox, PageDefaults, PageEventObserver,
    PageFragment, PageFragmentEvent, RenderOptions, RenderResources, RenderSink, RenderStatus,
    RenderSummary, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest, parse_html, render_streaming,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoopResolver;

impl ReplacedResolver for NoopResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        unreachable!("test documents contain no replaced elements")
    }
}

struct FixedIntrinsicResolver {
    width: f32,
    calls: AtomicUsize,
}

impl ReplacedResolver for FixedIntrinsicResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(self.width, 40.0),
            disposition: ResolveDisposition::Ok,
        })
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
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let status = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&resources),
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
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let err = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect_err("sink failure should be terminal");
    assert!(matches!(err, raikiri::RenderError::Observer(_)));
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
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let mut sink = RecordingSink {
        abort_after_page: Some(controller),
        ..RecordingSink::default()
    };
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let status = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        config,
        RenderOptions::new().resources(&resources),
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
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let status = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        LayoutConfig::default(),
        RenderOptions::new()
            .resources(&resources)
            .page_observer(&mut observer),
        &mut sink,
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
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let err = render_streaming(
        &doc,
        defaults(100.0, 50.0),
        LayoutConfig::default(),
        RenderOptions::new()
            .resources(&resources)
            .page_observer(&mut observer),
        &mut sink,
    )
    .expect_err("observer failure should be terminal");

    assert!(matches!(err, raikiri::RenderError::Observer(_)));
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
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&resources),
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
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");

    let first = sink.pages.first().expect("first page");
    assert_eq!(first.page_name.as_deref(), Some("cover"));
    assert_eq!(first.page_box.width, 130.0);
    assert_eq!(first.page_box.height, 140.0);
    assert_eq!(first.margins.left, 11.0);
}

#[test]
fn render_streaming_resolves_nested_grid_page_before_layout() {
    let stylesheet = r#"
        @page wide { size: 200px 300px; margin: 5px; }
        @page narrow { size: 120px 180px; margin: 12px; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body style="margin:0"><div style="display:grid;grid-template-columns:100%;grid-template-rows:auto auto"><div style="grid-row:2;order:0;page:wide;height:10px">wide</div><div style="grid-row:1;order:1;page:narrow;font-size:10px;line-height:12px">narrow page text</div></div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");

    let first = sink.pages.first().expect("first page");
    assert_eq!(first.page_name.as_deref(), Some("narrow"));
    assert_eq!(
        (first.page_box.width, first.page_box.height),
        (120.0, 180.0)
    );
    assert_eq!(first.margins.left, 12.0);
}

#[test]
fn render_streaming_preflight_uses_resolved_image_size_for_grid_page_selection() {
    let html = &br#"<!doctype html><style>
        @page wide { size:200px 300px; margin:5px }
        @page narrow { size:120px 180px; margin:12px }
        body { margin:0 }
        .flex { display:flex; width:300px }
        .grid { display:grid; order:0; flex:1 1 0; min-width:0;
                grid-template-columns:repeat(auto-fit,minmax(100px,1fr));
                grid-template-rows:auto auto }
        img { order:1; flex:0 0 auto }
    </style><body><div class="flex"><div class="grid">
        <div style="display:block;grid-row:2;order:0;page:wide;height:10px">wide</div>
        <div style="display:block;grid-row:1;order:1;page:narrow;font-size:10px;line-height:12px">narrow page text</div>
    </div><img src="image.png"></div></body>"#[..];
    let base_url = raikiri::Url::parse("https://example.test/assets/").expect("base URL");
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(html, &options).expect("parse");
    assert!((0..doc.dom().node_count()).any(|node_id| {
        doc.dom()
            .get_node(node_id)
            .is_some_and(|node| node.tag_name() == Some("img") && node.attribute("src").is_some())
    }));

    let small_resolver = FixedIntrinsicResolver {
        width: 50.0,
        calls: AtomicUsize::new(0),
    };
    let small_resources = RenderResources::new()
        .base_url(base_url.clone())
        .replaced_resolver(&small_resolver);
    let mut small_sink = RecordingSink::default();
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&small_resources),
        &mut small_sink,
    )
    .expect("small-image render");

    let large_resolver = FixedIntrinsicResolver {
        width: 250.0,
        calls: AtomicUsize::new(0),
    };
    let large_resources = RenderResources::new()
        .base_url(base_url)
        .replaced_resolver(&large_resolver);
    let mut large_sink = RecordingSink::default();
    render_streaming(
        &doc,
        defaults(100.0, 100.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&large_resources),
        &mut large_sink,
    )
    .expect("large-image render");

    assert!(small_resolver.calls.load(Ordering::Relaxed) >= 2);
    assert!(large_resolver.calls.load(Ordering::Relaxed) >= 3);
    let small_first = small_sink.pages.first().expect("small first page");
    let large_first = large_sink.pages.first().expect("large first page");
    assert_eq!(small_first.page_name.as_deref(), Some("wide"));
    assert_eq!(large_first.page_name.as_deref(), Some("narrow"));
    assert_eq!(
        (small_first.page_box.width, small_first.page_box.height),
        (200.0, 300.0)
    );
    assert_eq!(
        (large_first.page_box.width, large_first.page_box.height),
        (120.0, 180.0)
    );
}

#[test]
fn render_streaming_relayouts_until_page_geometry_converges() {
    let stylesheet = r#"
        @page { size: 100px 100px; margin: 0; }
        @page :left { size: 100px 240px; margin: 0; }
        @page wide { size: 100px 180px; margin: 0; }
    "#;
    let options = raikiri::ParseOptions {
        extra_stylesheets: &[stylesheet],
        network: None,
        base_url: None,
    };
    let doc = parse_html(
        &br#"<html><body><div style="height:200px">first</div><div style="page:wide;break-before:page;height:10px">wide</div><div style="height:300px">tail</div></body></html>"#[..],
        &options,
    )
    .expect("parse");
    let mut sink = RecordingSink::default();
    let resources = RenderResources::new().replaced_resolver(&NoopResolver);
    let status = render_streaming(
        &doc,
        defaults(100.0, 100.0),
        LayoutConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");

    let summary = match status {
        RenderStatus::Completed(summary) => summary,
        _ => panic!("geometry convergence case should complete"),
    };
    assert_eq!(summary.total_pages, 4);
    assert_eq!(sink.pages.len(), 4);
    assert_eq!(sink.pages[0].content_origin_y, 0.0);
    assert_eq!(sink.pages[1].content_origin_y, 100.0);
    assert_eq!(sink.pages[2].content_origin_y, 280.0);
    assert_eq!(sink.pages[3].content_origin_y, 380.0);
    assert_eq!(sink.pages[0].page_box.height, 100.0);
    assert_eq!(sink.pages[1].page_box.height, 180.0);
    assert_eq!(sink.pages[2].page_box.height, 100.0);
    assert_eq!(sink.pages[3].page_box.height, 240.0);
    assert_eq!(sink.pages[1].page_name.as_deref(), Some("wide"));

    // Every emitted item must fit the resolved page-local geometry. Before the
    // schedule refresh, page 1 used the 240px item projection with a 180px
    // `wide` page metadata record.
    for page in &sink.pages {
        assert!(page.items.iter().all(|item| {
            item.rect.y >= -0.001 && item.rect.y + item.rect.height <= page.page_box.height + 0.001
        }));
    }
}
