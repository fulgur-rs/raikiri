use super::*;
use raikiri_traits::{
    NodeId, NodeKind, PageFragmentItem, PageFragmentKind, PageFragmentLineRange, PageFragmentRect,
};

fn rect(x: f32, y: f32, w: f32, h: f32) -> PageFragmentRect {
    let mut r = PageFragmentRect::default();
    r.x = x;
    r.y = y;
    r.width = w;
    r.height = h;
    r
}

#[test]
fn fragment_rect_is_moved_to_the_page_box_origin() {
    let item = PageFragmentItem::new(
        NodeId(7),
        rect(5.0, 8.0, 20.0, 10.0),
        PageFragmentKind::Text,
        0,
        2,
        false,
    )
    .with_page_index(0)
    .with_line_range(PageFragmentLineRange::new(0, 3));
    let f = Fragment::new(&item, rect(30.0, 40.0, 100.0, 100.0));
    let r = f.rect();
    assert_eq!((r.x, r.y, r.width, r.height), (35.0, 48.0, 20.0, 10.0));
    assert_eq!(f.node(), NodeId(7));
    assert_eq!(f.kind(), FragmentKind::Text);
    assert_eq!(f.line_range(), Some(0..3));
    assert_eq!(f.is_first_fragment(), Some(true));
    assert_eq!(f.is_last_fragment(), Some(false));
    assert_eq!(f.repeat(), None);
    assert!(!f.continuation());
}

#[test]
fn repeated_fragment_reports_every_page() {
    let item = PageFragmentItem::new(
        NodeId(3),
        rect(0.0, 0.0, 1.0, 1.0),
        PageFragmentKind::Box,
        1,
        2,
        true,
    );
    let item = item.with_page_index(1);
    let f = Fragment::new(&item, rect(0.0, 0.0, 10.0, 10.0));
    assert_eq!(f.repeat(), Some(RepeatKind::EveryPage));
    assert_eq!(f.is_last_fragment(), Some(true));
}

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

use crate::{RenderOptions, render_streaming};
use raikiri_traits::{
    AbortController, ConsumerPropertyEvent, ConsumerPropertyObserver, PageDefaults, PageFragment,
    RenderError, RenderSink, RenderStatus, RenderSummary, StreamingConfig,
};

const PAGED: &str = "<style>\
    @page { size: 300px 200px; margin: 20px; border: 3px solid black; padding: 4px }\
    @page :first { size: 260px 200px }\
    p { margin: 0; height: 90px }\
    </style><p>aaa bbb ccc</p><p>ddd</p><p>eee fff</p>";

#[derive(Default)]
struct Collect(Vec<PageFragment>);
impl RenderSink for Collect {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        self.0.push(page);
        Ok(())
    }
    fn finish_render(&mut self, _: RenderSummary) -> std::io::Result<()> {
        Ok(())
    }
}

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
fn layout_matches_render_streaming_pages_and_items() {
    let doc = dom(PAGED);
    let mut sink = Collect::default();
    let status = render_streaming(
        &doc,
        PageDefaults::default(),
        StreamingConfig::default(),
        RenderOptions::new(),
        &mut sink,
    )
    .expect("render");
    assert!(matches!(status, RenderStatus::Completed(_)));
    let layout = completed(
        layout(
            &doc,
            PageDefaults::default(),
            StreamingConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout"),
    );

    assert!(sink.0.len() >= 2, "fixture must paginate");
    assert_eq!(layout.page_count() as usize, sink.0.len());
    for (page, neutral) in layout.pages().zip(sink.0.iter()) {
        assert_eq!(page.index(), neutral.page_index);
        assert_eq!(page.name(), neutral.page_name.as_deref());
        let g = page.geometry();
        assert_eq!(g.content_box.x, neutral.content_box.x);
        assert_eq!(g.content_box.y, neutral.content_box.y);
        assert_eq!(g.page_box.width, neutral.page_box.width);
        let fragments: Vec<_> = page.fragments().collect();
        assert_eq!(fragments.len(), neutral.items.len());
        for (f, item) in fragments.iter().zip(neutral.items.iter()) {
            assert_eq!(f.node(), item.node_id);
            let r = f.rect();
            assert_eq!(r.x - neutral.content_box.x, item.rect.x);
            assert_eq!(r.y - neutral.content_box.y, item.rect.y);
            assert_eq!(f.line_range(), item.line_range.map(|l| l.start..l.end));
        }
    }
}

#[test]
fn layout_exposes_the_cascade_used_for_layout_and_page_styles() {
    let doc = dom(PAGED);
    let layout = completed(
        layout(
            &doc,
            PageDefaults::default(),
            StreamingConfig::default(),
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
    let config = StreamingConfig::builder()
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
        StreamingConfig::default(),
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
        StreamingConfig::default(),
        LayoutOptions::new().consumer_properties(&registrations, &mut observer),
    )
    .err()
    .expect("observer error");
    assert!(matches!(err, RenderError::Sink(_)));
}

#[test]
fn layout_keeps_the_input_document_usable() {
    let doc = dom(PAGED);
    let first = completed(
        layout(
            &doc,
            PageDefaults::default(),
            StreamingConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout"),
    );
    let second = completed(
        layout(
            &doc,
            PageDefaults::default(),
            StreamingConfig::default(),
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
            StreamingConfig::default(),
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
