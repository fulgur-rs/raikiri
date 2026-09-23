//! External-consumer test for the single render entry point.
//!
//! Uses only `raikiri-html` and `raikiri-traits`, the crates an external
//! consumer depends on, and checks that page-event observation, consumer
//! properties, and the resource handoff work together in one render with the
//! documented delivery order.

use std::sync::{Arc, Mutex};

use raikiri_html::{
    ConsumerPropertyRegistration, RenderOptions, RenderResources, parse_html_with_resources,
    render_streaming,
};
use raikiri_traits::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyValue, PageDefaults,
    PageEventObserver, PageFragment, PageFragmentEvent, PageFragmentKind, RenderSink, RenderStatus,
    RenderSummary, ResourceKind, StreamingConfig, WarningKind,
};

#[derive(Debug, Clone, PartialEq)]
enum Delivery {
    Property(String, ConsumerPropertyValue),
    Page(u32),
    Link { page_index: u32, href: String },
    Finish,
}

type Log = Arc<Mutex<Vec<Delivery>>>;

struct Sink(Log);

impl RenderSink for Sink {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        self.0.lock().unwrap().push(Delivery::Page(page.page_index));
        Ok(())
    }

    fn finish_render(&mut self, _summary: RenderSummary) -> std::io::Result<()> {
        self.0.lock().unwrap().push(Delivery::Finish);
        Ok(())
    }
}

type PageLog = Arc<Mutex<Vec<PageFragment>>>;

struct GeometrySink(PageLog);

impl RenderSink for GeometrySink {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        self.0.lock().unwrap().push(page);
        Ok(())
    }

    fn finish_render(&mut self, _summary: RenderSummary) -> std::io::Result<()> {
        Ok(())
    }
}

struct Links(Log);

impl PageEventObserver for Links {
    fn observe_event(&mut self, event: PageFragmentEvent) -> std::io::Result<()> {
        if let PageFragmentEvent::Link(link) = event {
            self.0.lock().unwrap().push(Delivery::Link {
                page_index: link.page_index,
                href: link.link.href,
            });
        }
        Ok(())
    }
}

struct Properties(Log);

impl ConsumerPropertyObserver for Properties {
    fn observe_event(&mut self, event: ConsumerPropertyEvent) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push(Delivery::Property(event.property_name, event.value));
        Ok(())
    }
}

fn defaults(width: f32, height: f32) -> PageDefaults {
    let mut defaults = PageDefaults::default();
    let mut page_box = raikiri_traits::PageBox::new();
    page_box.width = width;
    page_box.height = height;
    defaults.page_box = page_box;
    defaults
}

#[test]
fn one_render_combines_links_consumer_properties_and_resource_warnings() {
    let html = br#"<html><body>
        <h1 style="bookmark-level: 1; margin: 0; height: 20px">
          <a href="https://example.com/first">first</a>
        </h1>
        <img src="https://example.com/missing.png">
        <div style="break-before: page; height: 10px">
          <a href="https://example.com/second">second</a>
        </div>
    </body></html>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");

    let log: Log = Arc::default();
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut links = Links(Arc::clone(&log));
    let mut properties = Properties(Arc::clone(&log));
    let mut sink = Sink(Arc::clone(&log));
    let status = render_streaming(
        &doc,
        defaults(200.0, 100.0),
        StreamingConfig::default(),
        RenderOptions::new()
            .resources(&resources)
            .page_observer(&mut links)
            .consumer_properties(&registrations, &mut properties),
        &mut sink,
    )
    .expect("render");

    let RenderStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };
    assert_eq!(summary.total_pages, 2);
    assert!(
        summary.warnings.iter().any(|warning| matches!(
            &warning.kind,
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: Some(url),
            } if url.as_str() == "https://example.com/missing.png"
        )),
        "the unresolved image is reported in the same render: {:?}",
        summary.warnings
    );

    let log = log.lock().unwrap().clone();
    assert_eq!(
        log,
        vec![
            Delivery::Property(
                "bookmark-level".to_owned(),
                ConsumerPropertyValue::Integer(1)
            ),
            Delivery::Page(0),
            Delivery::Link {
                page_index: 0,
                href: "https://example.com/first".to_owned(),
            },
            Delivery::Page(1),
            Delivery::Link {
                page_index: 1,
                href: "https://example.com/second".to_owned(),
            },
            Delivery::Finish,
        ],
        "consumer properties precede the first page; each page's links follow that page"
    );
}

#[test]
fn first_ordered_grid_item_selects_initial_named_page_geometry() {
    let html = br#"<!doctype html>
        <style>
          @page wide { size: 200px 300px; margin: 5px; }
          @page narrow { size: 120px 180px; margin: 12px; }
        </style>
        <body style="display:grid;grid-template-columns:100px">
          <div style="grid-column:1;order:1;page:wide;height:10px">wide</div>
          <div style="grid-column:1;order:0;page:narrow;height:10px">narrow</div>
        </body>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let pages: PageLog = Arc::default();
    let mut sink = GeometrySink(Arc::clone(&pages));

    let status = render_streaming(
        &doc,
        defaults(400.0, 400.0),
        StreamingConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");
    let RenderStatus::Completed(_) = status else {
        panic!("expected a complete render");
    };

    let pages = pages.lock().unwrap();
    assert_eq!(pages.len(), 2);
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.page_name.as_deref(), Some("narrow"));
    assert_eq!(first.page_box.width, 120.0);
    assert_eq!(first.page_box.height, 180.0);
    assert_eq!(first.margins.left, 12.0);
    assert_eq!(first.margins.right, 12.0);
    assert_eq!(first.margins.top, 12.0);
    assert_eq!(first.margins.bottom, 12.0);

    let second = &pages[1];
    assert_eq!(second.page_name.as_deref(), Some("wide"));
    assert_eq!(second.page_box.width, 200.0);
    assert_eq!(second.page_box.height, 300.0);
}

#[test]
fn explicit_grid_row_ends_select_initial_page_from_resolved_grid_placement() {
    let html = br#"<!doctype html>
        <style>
          @page wide { size: 200px 300px; margin: 5px; }
          @page narrow { size: 120px 180px; margin: 12px; }
        </style>
        <body style="display:grid;grid-template-columns:100px">
          <div style="display:block;grid-row:auto / 2;order:1;page:wide;height:10px">wide</div>
          <div style="display:block;grid-row:auto / 3;order:0;page:narrow;height:10px">narrow</div>
        </body>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let pages: PageLog = Arc::default();
    let mut sink = GeometrySink(Arc::clone(&pages));

    let status = render_streaming(
        &doc,
        defaults(400.0, 400.0),
        StreamingConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");
    let RenderStatus::Completed(_) = status else {
        panic!("expected a complete render");
    };

    let pages = pages.lock().unwrap();
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.page_name.as_deref(), Some("wide"));
    assert_eq!(first.page_box.width, 200.0);
    assert_eq!(first.page_box.height, 300.0);
    assert_eq!(first.margins.left, 5.0);
    assert_eq!(first.margins.right, 5.0);
    assert_eq!(first.margins.top, 5.0);
    assert_eq!(first.margins.bottom, 5.0);
}

#[test]
fn explicit_grid_rows_resolve_first_page_before_sizing_and_wrapping() {
    let html = br#"<!doctype html>
        <style>
          @page wide { size: 200px 300px; margin: 5px; }
          @page narrow { size: 120px 180px; margin: 12px; }
        </style>
        <body style="display:grid;grid-template-columns:100%">
          <div style="display:block;grid-row:2;order:1;page:wide;height:10px">wide</div>
          <div style="display:block;grid-row:1;order:0;page:narrow;font-family:monospace;font-size:16px">alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau</div>
        </body>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let pages: PageLog = Arc::default();
    let mut sink = GeometrySink(Arc::clone(&pages));

    let status = render_streaming(
        &doc,
        defaults(400.0, 400.0),
        StreamingConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");
    let RenderStatus::Completed(_) = status else {
        panic!("expected a complete render");
    };

    let pages = pages.lock().unwrap();
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.page_name.as_deref(), Some("narrow"));
    assert_eq!(first.page_box.width, 120.0);
    assert_eq!(first.page_box.height, 180.0);
    assert_eq!(first.margins.left, 12.0);
    assert!((first.content_box.width - 96.0).abs() < 0.5);
    let text_lines = first
        .items
        .iter()
        .filter(|item| item.kind == PageFragmentKind::Text)
        .filter_map(|item| item.line_range)
        .map(|range| range.end - range.start)
        .max()
        .expect("the named grid item emits text lines");
    assert!(text_lines >= 4);
}

#[test]
fn multi_column_grid_order_selects_initial_named_page_geometry() {
    let html = br#"<!doctype html>
        <style>
          @page wide { size: 200px 300px; margin: 5px; }
          @page narrow { size: 120px 180px; margin: 12px; }
        </style>
        <body style="display:grid;grid-template-columns:50% 50%">
          <div style="display:block;order:1;page:wide;height:10px">wide</div>
          <div style="display:block;order:0;page:narrow;height:10px">narrow</div>
        </body>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let pages: PageLog = Arc::default();
    let mut sink = GeometrySink(Arc::clone(&pages));

    let status = render_streaming(
        &doc,
        defaults(400.0, 400.0),
        StreamingConfig::default(),
        RenderOptions::new().resources(&resources),
        &mut sink,
    )
    .expect("render");
    let RenderStatus::Completed(_) = status else {
        panic!("expected a complete render");
    };

    let pages = pages.lock().unwrap();
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.page_name.as_deref(), Some("narrow"));
    assert_eq!(first.page_box.width, 120.0);
    assert_eq!(first.page_box.height, 180.0);
    assert_eq!(first.margins.left, 12.0);
    assert_eq!(first.margins.right, 12.0);
}
