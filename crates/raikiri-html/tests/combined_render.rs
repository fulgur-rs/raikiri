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
    PageEventObserver, PageFragment, PageFragmentEvent, RenderSink, RenderStatus, RenderSummary,
    ResourceKind, StreamingConfig, WarningKind,
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
