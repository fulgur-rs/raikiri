use super::*;
use raikiri_traits::{NodeId, NodeKind};

fn dom(html: &str) -> crate::HtmlDocument {
    crate::parse_html_with_resources(html.as_bytes(), &crate::RenderResources::new())
        .expect("parse")
}

fn find(view: &DomView<'_>, node: NodeId, name: &str) -> Option<NodeId> {
    if view.local_name(node) == Some(name) {
        return Some(node);
    }
    view.children(node)
        .find_map(|child| find(view, child, name))
}

#[test]
fn dom_view_walks_structure_and_attributes() {
    let doc = dom(r#"<p id="x" class="c">He<b>ll</b>o</p><svg><g></g></svg>"#);
    let view = DomView::new(doc.dom());
    let p = find(&view, view.root(), "p").expect("p");
    assert_eq!(view.kind(p), Some(NodeKind::Element));
    assert_eq!(view.attr(p, "id"), Some("x"));
    assert_eq!(
        view.namespace(p),
        None,
        "HTML namespace uses the None fast path"
    );
    let b = find(&view, p, "b").expect("b");
    assert_eq!(view.parent(b), Some(p));
    assert_eq!(view.text_content(p), "Hello");
    let g = find(&view, view.root(), "g").expect("g");
    assert_eq!(view.namespace(g), Some("http://www.w3.org/2000/svg"));
}

#[test]
fn dom_view_text_content_inserts_spaces_at_block_boundaries() {
    let doc = dom("<a href=x><div>foo</div><div>bar</div></a>");
    let view = DomView::new(doc.dom());
    let a = find(&view, view.root(), "a").expect("a");
    assert_eq!(view.text_content(a), "foo bar");
}

#[test]
fn dom_view_is_total_on_out_of_range_ids() {
    let doc = dom("<p>x</p>");
    let view = DomView::new(doc.dom());
    let bad = NodeId(1_000_000);
    assert_eq!(view.kind(bad), None);
    assert_eq!(view.parent(bad), None);
    assert_eq!(view.children(bad).count(), 0);
    assert_eq!(view.local_name(bad), None);
    assert_eq!(view.attr(bad, "id"), None);
    assert_eq!(view.text(bad), None);
    assert_eq!(view.text_content(bad), "");
}

use raikiri_traits::{
    AbortController, ConsumerPropertyEvent, ConsumerPropertyObserver, LayoutConfig, PageDefaults,
    RenderError,
};

const PAGED: &str = "<style>\
    @page { size: 300px 200px; margin: 20px; border: 3px solid black; padding: 4px }\
    @page :first { size: 260px 200px }\
    p { margin: 0; height: 90px }\
    </style><p>aaa bbb ccc</p><p>ddd</p><p>eee fff</p>";

fn completed(status: LayoutStatus) -> DocumentLayout {
    match status {
        LayoutStatus::Completed(layout) => layout,
        other => panic!(
            "expected Completed, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

#[test]
fn layout_exposes_the_cascade_used_for_layout_and_page_styles() {
    let doc = dom(PAGED);
    let layout = completed(
        layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout"),
    );
    let page = layout.page(0).expect("page 0");
    let p = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("p"))
        .expect("a <p> fragment");
    assert!(page.computed(p.node()).is_some());
    assert!(page.computed(NodeId(1_000_000)).is_none());
    let _ = page.page_style().declarations();
    assert!(layout.page(layout.page_count()).is_none());
}

#[test]
fn layout_aborts_before_layout_without_partial_result() {
    let doc = dom(PAGED);
    let controller = AbortController::new();
    controller.abort();
    let config = LayoutConfig::builder()
        .signal(Some(controller.signal.clone()))
        .build();
    let status = layout(&doc, PageDefaults::default(), config, LayoutOptions::new()).expect("ok");
    assert!(matches!(status, LayoutStatus::Aborted));
}

#[derive(Default)]
struct CountProperties(usize);
impl ConsumerPropertyObserver for CountProperties {
    fn observe_event(&mut self, _: ConsumerPropertyEvent) -> std::io::Result<()> {
        self.0 += 1;
        Ok(())
    }
}

struct FailProperties;
impl ConsumerPropertyObserver for FailProperties {
    fn observe_event(&mut self, _: ConsumerPropertyEvent) -> std::io::Result<()> {
        Err(std::io::Error::other("consumer failed"))
    }
}

#[test]
fn layout_delivers_consumer_properties_before_returning() {
    let doc = dom("<h1 style='bookmark-level: 1'>x</h1>");
    let registrations = [crate::ConsumerPropertyRegistration::integer(
        "bookmark-level",
    )];
    let mut observer = CountProperties::default();
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().consumer_properties(&registrations, &mut observer),
    )
    .expect("layout");
    assert!(matches!(status, LayoutStatus::Completed(_)));
    assert_eq!(observer.0, 1);
}

#[test]
fn layout_returns_observer_errors() {
    let doc = dom("<h1 style='bookmark-level: 1'>x</h1>");
    let registrations = [crate::ConsumerPropertyRegistration::integer(
        "bookmark-level",
    )];
    let mut observer = FailProperties;
    let err = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().consumer_properties(&registrations, &mut observer),
    )
    .err()
    .expect("observer error");
    assert!(matches!(err, RenderError::Observer(_)));
}

#[test]
fn layout_keeps_the_input_document_usable() {
    let doc = dom(PAGED);
    let first = completed(
        layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout"),
    );
    let second = completed(
        layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout"),
    );
    assert_eq!(first.page_count(), second.page_count());
}

const NAV: &str = "<style>@page { size: 300px 200px; margin: 20px } p { margin: 0; height: 120px }</style>\
    <p id='first'>one <a href=' #second '>jump</a> <a href='#x'>a</a><a href='#x'>b</a></p>\
    <p id='second' style='break-before: page'>two</p><a name='named'>n</a><p id='first'>dup</p>\
    <div style='display:none' id='hidden'>h</div><a href='   '>empty</a>";

fn laid_out(html: &str) -> DocumentLayout {
    completed(
        layout(
            &dom(html),
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout"),
    )
}

#[test]
fn anchors_index_ids_and_a_names_first_in_document_order() {
    let layout = laid_out(NAV);
    let anchors = layout.anchors();
    let first = anchors.get("first").expect("first");
    assert_eq!(first.page_index, 0);
    let second = anchors.get("second").expect("second");
    assert!(
        second.page_index >= 1,
        "second paragraph is on a later page"
    );
    assert!(anchors.get("named").is_some());
    assert!(
        anchors.get("hidden").is_none(),
        "display:none has no fragment"
    );
}

#[test]
fn links_group_quads_by_owner_and_target_and_trim_href() {
    let layout = laid_out(NAV);
    let page0 = layout.page(0).expect("page 0");
    let links: Vec<_> = page0.links().collect();
    let jump = links
        .iter()
        .find(|l| l.target == "#second")
        .expect("trimmed href");
    assert!(!jump.quads.is_empty());
    let xs: Vec<_> = links.iter().filter(|l| l.target == "#x").collect();
    assert_eq!(
        xs.len(),
        2,
        "two <a> owners with the same target stay separate"
    );
    assert!(
        links.iter().all(|l| !l.target.is_empty()),
        "blank href makes no link"
    );
}

#[test]
fn is_rendered_reflects_fragments_and_is_total() {
    let layout = laid_out(NAV);
    let page = layout.page(0).expect("page 0");
    let p = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("p"))
        .expect("p");
    assert!(layout.is_rendered(p.node()));
    assert!(!layout.is_rendered(NodeId(1_000_000)));
}

#[test]
fn dom_and_computed_reject_large_node_ids() {
    let result = laid_out(PAGED);
    let page = result.page(0).expect("page");
    let view = page.dom();
    for bad in [NodeId(1_u64 << 32), NodeId(u64::MAX)] {
        assert_eq!(view.kind(bad), None);
        assert_eq!(view.parent(bad), None);
        assert_eq!(view.children(bad).count(), 0);
        assert_eq!(view.local_name(bad), None);
        assert_eq!(view.namespace(bad), None);
        assert_eq!(view.attr(bad, "id"), None);
        assert_eq!(view.text(bad), None);
        assert_eq!(view.text_content(bad), "");
        assert!(page.computed(bad).is_none());
        assert!(!result.is_rendered(bad));
    }
}

// Losing the abort check after consumer delivery would return a partial result.
#[test]
fn layout_aborts_when_property_observer_aborts() {
    let doc = dom("<h1 style='bookmark-level:1'>x</h1>");
    let registrations = [crate::ConsumerPropertyRegistration::integer(
        "bookmark-level",
    )];
    let controller = AbortController::new();
    let mut count = 0;
    let mut observer = |_: ConsumerPropertyEvent| {
        count += 1;
        controller.abort();
        Ok::<_, std::io::Error>(())
    };
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::builder()
            .signal(Some(controller.signal.clone()))
            .build(),
        LayoutOptions::new().consumer_properties(&registrations, &mut observer),
    )
    .unwrap();
    assert!(matches!(status, LayoutStatus::Aborted));
    assert_eq!(count, 1);
    assert!(matches!(
        layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new()
        )
        .unwrap(),
        LayoutStatus::Completed(_)
    ));
}

// A second origin offset would move placements and clickable areas off the page.
#[test]
fn layout_page_origin_is_applied_once_to_fragments_and_links() {
    let doc = dom(
        "<style>body{margin:0} @page{size:100px 100px;margin:20px;padding:5px} div{width:10px;height:10px}</style><div><a href=' /go '>x</a></div>",
    );
    let result = completed(
        layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = result.page(0).unwrap();
    let rect = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("div"))
        .unwrap()
        .rect();
    assert_eq!(rect, raikiri_traits::PaintRect::new(25.0, 25.0, 10.0, 10.0));
    assert_eq!(
        page.geometry().content_box,
        raikiri_traits::PaintRect::new(25.0, 25.0, 60.0, 50.0)
    );
    let text = page
        .fragments()
        .find(|f| f.kind() == FragmentKind::Text)
        .unwrap()
        .rect();
    let links: Vec<_> = page.links().collect();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "/go");
    assert_eq!(links[0].quads, &[text]);
}
