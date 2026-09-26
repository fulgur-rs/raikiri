//! External-consumer coverage for layout, links, properties, and shared resources.

use std::sync::{Arc, Mutex};

use raikiri_html::{
    ConsumerPropertyRegistration, FragmentKind, LayoutOptions, LayoutStatus, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_traits::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyValue, LayoutConfig,
    PageDefaults, ResourceKind, WarningKind,
};
type Log = Arc<Mutex<Vec<(String, ConsumerPropertyValue)>>>;
struct Properties(Log);
impl ConsumerPropertyObserver for Properties {
    fn observe_event(&mut self, event: ConsumerPropertyEvent) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((event.property_name, event.value));
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
    let mut properties = Properties(Arc::clone(&log));
    let status = layout(
        &doc,
        defaults(200.0, 100.0),
        LayoutConfig::default(),
        LayoutOptions::new()
            .resources(&resources)
            .consumer_properties(&registrations, &mut properties),
    )
    .expect("render");

    let LayoutStatus::Completed(summary) = status else {
        panic!("expected a complete render");
    };
    assert_eq!(summary.page_count(), 2);
    assert!(
        summary.warnings().iter().any(|warning| matches!(
            &warning.kind,
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: Some(url),
            } if url.as_str() == "https://example.com/missing.png"
        )),
        "the unresolved image is reported in the same render: {:?}",
        summary.warnings()
    );

    assert_eq!(
        *log.lock().unwrap(),
        vec![(
            "bookmark-level".to_owned(),
            ConsumerPropertyValue::Integer(1)
        )]
    );
    let links = summary
        .pages()
        .map(|page| {
            (
                page.index(),
                page.links()
                    .map(|link| link.target.to_owned())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        links,
        vec![
            (0, vec!["https://example.com/first".to_owned()]),
            (1, vec!["https://example.com/second".to_owned()])
        ]
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

    let status = layout(
        &doc,
        defaults(400.0, 400.0),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("render");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete render");
    };

    let pages: Vec<_> = result.pages().collect();
    assert_eq!(pages.len(), 2);
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.name(), Some("narrow"));
    assert_eq!(first.geometry().page_box.width, 120.0);
    assert_eq!(first.geometry().page_box.height, 180.0);
    assert_eq!(first.geometry().margins.left, 12.0);
    assert_eq!(first.geometry().margins.right, 12.0);
    assert_eq!(first.geometry().margins.top, 12.0);
    assert_eq!(first.geometry().margins.bottom, 12.0);

    let second = &pages[1];
    assert_eq!(second.name(), Some("wide"));
    assert_eq!(second.geometry().page_box.width, 200.0);
    assert_eq!(second.geometry().page_box.height, 300.0);
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

    let status = layout(
        &doc,
        defaults(400.0, 400.0),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("render");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete render");
    };

    let pages: Vec<_> = result.pages().collect();
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.name(), Some("wide"));
    assert_eq!(first.geometry().page_box.width, 200.0);
    assert_eq!(first.geometry().page_box.height, 300.0);
    assert_eq!(first.geometry().margins.left, 5.0);
    assert_eq!(first.geometry().margins.right, 5.0);
    assert_eq!(first.geometry().margins.top, 5.0);
    assert_eq!(first.geometry().margins.bottom, 5.0);
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

    let status = layout(
        &doc,
        defaults(400.0, 400.0),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("render");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete render");
    };

    let pages: Vec<_> = result.pages().collect();
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.name(), Some("narrow"));
    assert_eq!(first.geometry().page_box.width, 120.0);
    assert_eq!(first.geometry().page_box.height, 180.0);
    assert_eq!(first.geometry().margins.left, 12.0);
    assert!((first.geometry().content_box.width - 96.0).abs() < 0.5);
    let text_lines = first
        .fragments()
        .filter(|item| item.kind() == FragmentKind::Text)
        .filter_map(|item| item.line_range())
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

    let status = layout(
        &doc,
        defaults(400.0, 400.0),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("render");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete render");
    };

    let pages: Vec<_> = result.pages().collect();
    let first = pages.first().expect("the render emits a first page");
    assert_eq!(first.name(), Some("narrow"));
    assert_eq!(first.geometry().page_box.width, 120.0);
    assert_eq!(first.geometry().page_box.height, 180.0);
    assert_eq!(first.geometry().margins.left, 12.0);
    assert_eq!(first.geometry().margins.right, 12.0);
}
