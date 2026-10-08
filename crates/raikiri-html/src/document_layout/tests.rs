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
fn inline_svg_presentation_dimensions_yield_to_layered_author_css() {
    let document = dom(
        "<style>@page{size:200px 100px;margin:0}body{margin:0}svg{display:block}@layer x{svg{width:30px;height:15px}}</style><svg width='10' height='10'><rect width='10' height='10'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("svg"))
        .unwrap();
    let viewport = page.inline_svg(&fragment).unwrap().unwrap().viewport;
    assert_eq!((viewport.width, viewport.height), (30.0, 15.0));
}

#[test]
fn inline_svg_root_color_is_resolved_before_standalone_rendering() {
    for (css, attrs, expected) in [
        ("body{color:blue}", "color='inherit'", [0, 0, 255, 255]),
        ("svg{color:blue}", "color='red'", [0, 0, 255, 255]),
        ("", "color='red'", [255, 0, 0, 255]),
        (
            "body{color:blue}svg{color:revert}",
            "color='red'",
            [0, 0, 255, 255],
        ),
        (
            "@layer x{svg{color:blue;all:revert-layer}}",
            "color='red'",
            [255, 0, 0, 255],
        ),
        (
            "body{color:blue}",
            "style='color:inherit'",
            [0, 0, 255, 255],
        ),
    ] {
        let document = dom(&format!(
            "<style>@page{{size:200px 100px;margin:0}}body{{margin:0}}svg{{display:block}}{css}</style><svg width='10' height='10' {attrs}><rect width='10' height='10' fill='currentColor'/></svg>"
        ));
        let layout = completed(
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new(),
            )
            .unwrap(),
        );
        let page = layout.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|f| page.dom().local_name(f.node()) == Some("svg"))
            .unwrap();
        let svg = page.inline_svg(&fragment).unwrap().unwrap();
        let prepared = raikiri_svg::SvgDocument::parse(svg.source.as_bytes()).unwrap();
        let image = prepared
            .rasterize(
                raikiri_svg::SvgViewport {
                    width: svg.viewport.width,
                    height: svg.viewport.height,
                },
                raikiri_svg::SvgRootStyle::default(),
                None,
            )
            .unwrap();
        assert_eq!(&image.rgba[..4], &expected, "{css} / {attrs}");
        let computed = page.computed(fragment.node()).unwrap().color;
        assert_eq!([computed.r, computed.g, computed.b, computed.a], expected);
    }
}

#[test]
fn inline_svg_resolved_root_color_keeps_original_selector_matches() {
    let document = dom(
        "<style>@page{size:200px 100px;margin:0}body{margin:0}svg{display:block;color:blue}</style><svg width='10' height='10' color='red'><style>svg[color=red] rect {fill:green}</style><rect width='10' height='10' fill='currentColor'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("svg"))
        .unwrap();
    let svg = page.inline_svg(&fragment).unwrap().unwrap();
    let prepared = raikiri_svg::SvgDocument::parse(svg.source.as_bytes()).unwrap();
    let image = prepared
        .rasterize(
            raikiri_svg::SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            raikiri_svg::SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!(&image.rgba[..4], &[0, 128, 0, 255]);
}

#[test]
fn inline_svg_attribute_dimensions_define_the_viewport_before_css_overrides() {
    for (css, width, height) in [
        ("", 60.0, 40.0),
        ("width:90px", 90.0, 40.0),
        ("height:20px", 60.0, 20.0),
        ("width:20px;height:10px", 20.0, 10.0),
    ] {
        let document = dom(&format!(
            "<style>@page {{size:200px 150px;margin:0}} body {{margin:0}} svg {{display:block;{css}}}</style><svg width='60' height='40'><rect width='60' height='40'/></svg>"
        ));
        let layout = completed(
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new(),
            )
            .unwrap(),
        );
        assert_eq!(layout.pages().count(), 1);
        let page = layout.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|f| page.dom().local_name(f.node()) == Some("svg"))
            .unwrap();
        let payload = page.inline_svg(&fragment).unwrap().unwrap();
        assert_eq!(
            (payload.viewport.width, payload.viewport.height),
            (width, height)
        );
    }
}

#[test]
fn inline_svg_attribute_height_paginates_with_one_original_viewport() {
    let document = dom(
        "<style>@page {size:200px 100px;margin:10px} body {margin:0} svg {display:block}</style><svg width='40' height='180'><rect width='40' height='180'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    assert_eq!(layout.pages().count(), 3);
    for (index, page) in layout.pages().enumerate() {
        let fragment = page
            .fragments()
            .find(|f| page.dom().local_name(f.node()) == Some("svg"))
            .unwrap();
        let viewport = page.inline_svg(&fragment).unwrap().unwrap().viewport;
        assert_eq!(
            (viewport.x, viewport.y, viewport.width, viewport.height),
            (10.0, 10.0 - 80.0 * index as f32, 40.0, 180.0)
        );
    }
}

#[test]
fn inline_svg_payload_uses_the_resolved_content_box_and_host_style() {
    let document = dom(
        "<style>@page {size:300px 200px;margin:10px} body {margin:0} svg {box-sizing:border-box;width:200px;height:120px;border:2px solid black;padding:10%;color:blue;opacity:.5}</style><svg xmlns='http://www.w3.org/2000/svg' width='200' height='120' style='display:block'><rect width='10' height='10' fill='currentColor' opacity='inherit'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("svg"))
        .unwrap();
    let content = fragment.content_rect().unwrap();
    assert_eq!(
        (content.x, content.y, content.width, content.height),
        (40.0, 30.0, 140.0, 80.0)
    );
    let svg = page.inline_svg(&fragment).unwrap().unwrap();
    assert_eq!(svg.viewport, content);
    assert_eq!(svg.host_opacity, Some(0.5));
    let prepared = raikiri_svg::SvgDocument::parse(svg.source.as_bytes()).unwrap();
    assert_eq!(prepared.intrinsic_size().width, Some(140.0));
    assert_eq!(prepared.intrinsic_size().height, Some(80.0));
    let image = prepared
        .rasterize(
            raikiri_svg::SvgViewport {
                width: content.width,
                height: content.height,
            },
            raikiri_svg::SvgRootStyle::default(),
            None,
        )
        .unwrap();
    let offset = (2 * image.width as usize + 2) * 4;
    assert_eq!(&image.rgba[offset..offset + 4], &[0, 0, 255, 64]);
}

#[test]
fn inline_svg_payload_does_not_fold_parent_opacity_into_the_svg_source() {
    let document = dom(
        "<style>@page {size:200px 100px;margin:0} body {margin:0} div {opacity:.4} svg {display:block;opacity:.5}</style><div><svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'><rect width='20' height='10' fill='red'/></svg></div>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("svg"))
        .unwrap();
    let svg = page.inline_svg(&fragment).unwrap().unwrap();
    assert_eq!(svg.host_opacity, Some(0.5));
    let alphas: Vec<_> = page
        .paint_order()
        .into_iter()
        .filter_map(|event| match event {
            PaintEvent::PushOpacity(alpha) => Some(alpha),
            _ => None,
        })
        .collect();
    assert_eq!(alphas, [0.4, 0.5]);
}

#[test]
fn ordinary_boxes_have_content_geometry_and_no_inline_svg_payload() {
    let document = dom(
        "<style>@page {size:200px 100px;margin:0} body {margin:0} div {width:100px;height:50px;padding:5px;border:2px solid black}</style><div></div>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("div"))
        .unwrap();
    let content = fragment.content_rect().unwrap();
    assert_eq!(
        (content.x, content.y, content.width, content.height),
        (7.0, 7.0, 100.0, 50.0)
    );
    assert!(page.inline_svg(&fragment).unwrap().is_none());
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

// An abort fired by the observer does not stop the current batch: the driver
// still delivers the remaining events before reporting Aborted with no partial
// result. The consumer discards the per-call collection in that case.
#[test]
fn layout_abort_on_first_event_still_delivers_remaining_batch() {
    let doc = dom("<h1 style='bookmark-level: 1'>a</h1>\
         <h2 style='bookmark-level: 2'>b</h2><p style='bookmark-level: 3'>c</p>");
    let registrations = [crate::ConsumerPropertyRegistration::integer(
        "bookmark-level",
    )];
    let controller = AbortController::new();
    let mut values = Vec::new();
    let mut orders = Vec::new();
    let mut observer = |event: ConsumerPropertyEvent| {
        if let raikiri_traits::ConsumerPropertyValue::Integer(value) = event.value {
            values.push(value);
        }
        orders.push(event.source_order);
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
    .expect("layout returns a status, not an observer error");
    assert!(
        matches!(status, LayoutStatus::Aborted),
        "abort during delivery reports Aborted"
    );
    assert_eq!(
        values,
        vec![1, 2, 3],
        "full batch is delivered in document order"
    );
    assert_eq!(orders.len(), 3);
    assert!(
        orders.windows(2).all(|pair| pair[0] < pair[1]),
        "source order is strictly increasing, got {orders:?}"
    );
}

// Aborting on a middle event delivers the same full batch, not a prefix.
#[test]
fn layout_abort_on_middle_event_delivers_full_batch_without_partial_result() {
    let doc = dom("<h1 style='bookmark-level: 1'>a</h1>\
         <h2 style='bookmark-level: 2'>b</h2><p style='bookmark-level: 3'>c</p>");
    let registrations = [crate::ConsumerPropertyRegistration::integer(
        "bookmark-level",
    )];
    let controller = AbortController::new();
    let mut values = Vec::new();
    let mut seen = 0;
    let mut observer = |event: ConsumerPropertyEvent| {
        seen += 1;
        if let raikiri_traits::ConsumerPropertyValue::Integer(value) = event.value {
            values.push(value);
        }
        if seen == 2 {
            controller.abort();
        }
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
    .expect("layout returns a status, not an observer error");
    assert!(matches!(status, LayoutStatus::Aborted));
    assert_eq!(
        values,
        vec![1, 2, 3],
        "events after the aborting event are still delivered"
    );
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

// HTML LS §15.3.8 gives tables a 2px border-spacing, which separates
// adjoining cells and also the outer cells from the table's edge
// (CSS 2.2 §17.6.1).
#[test]
fn table_cells_are_separated_by_the_ua_border_spacing() {
    let cell_x = |css: &str| {
        let result = laid_out(&format!(
            "<style>@page{{size:200px 100px;margin:0}} body{{margin:0}} \
             td{{padding:0;width:10px;height:10px}} {css}</style>\
             <table><tr><td>a</td><td>b</td></tr></table>"
        ));
        let page = result.page(0).unwrap();
        let cells: Vec<_> = page
            .fragments()
            .filter(|f| page.dom().local_name(f.node()) == Some("td"))
            .map(|f| (f.rect().x, f.rect().y))
            .collect();
        cells
    };
    assert_eq!(cell_x(""), [(2.0, 2.0), (14.0, 2.0)]);
    assert_eq!(cell_x("table{border-spacing:0}"), [(0.0, 0.0), (10.0, 0.0)]);
}

#[test]
fn inline_svg_payload_retains_the_whole_viewport_across_page_cuts() {
    let document = dom(
        "<style>@page{size:100px 100px;margin:10px}body{margin:0}svg{display:block;width:20px;height:180px;padding:5px;border:2px solid black}</style><svg xmlns='http://www.w3.org/2000/svg' width='20' height='180'><rect width='20' height='180' fill='red'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    assert_eq!(layout.pages().count(), 3);
    for (index, page) in layout.pages().enumerate() {
        let fragment = page
            .fragments()
            .find(|f| page.dom().local_name(f.node()) == Some("svg"))
            .unwrap();
        let payload = page.inline_svg(&fragment).unwrap().unwrap();
        assert_eq!(
            payload.viewport,
            raikiri_traits::PaintRect::new(17.0, 17.0 - index as f32 * 80.0, 20.0, 180.0)
        );
        assert_eq!(payload.host_opacity, None);
        assert!(fragment.paint_rect().height < payload.viewport.height);
    }
}

#[test]
fn inline_svg_payload_preserves_negative_fixed_offsets_on_each_page() {
    let document = dom(
        "<style>@page{size:100px 100px;margin:10px}body{margin:0}div{height:180px}svg{display:block;position:fixed;top:-5px;left:3px;width:20px;height:10px;padding:4px;border:2px solid black}</style><div></div><svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'><rect width='20' height='10' fill='red'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    assert!(layout.pages().count() > 1);
    for page in layout.pages() {
        let fragment = page
            .fragments()
            .find(|f| page.dom().local_name(f.node()) == Some("svg"))
            .unwrap();
        assert_eq!(fragment.repeat(), Some(RepeatKind::EveryPage));
        assert_eq!(fragment.paint_rect().y, 5.0);
        assert_eq!(
            page.inline_svg(&fragment).unwrap().unwrap().viewport,
            raikiri_traits::PaintRect::new(19.0, 11.0, 20.0, 10.0)
        );
    }
}

#[test]
fn text_fragments_have_no_element_content_box() {
    let document = dom("<p>content</p>");
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let text = page
        .fragments()
        .find(|f| f.kind() == FragmentKind::Text)
        .unwrap();
    assert_eq!(text.content_rect(), None);
    assert!(page.inline_svg(&text).unwrap().is_none());
}

#[test]
fn inline_svg_payload_skips_empty_and_hidden_content_boxes() {
    for style in [
        "width:0;height:10px",
        "width:10px;height:0",
        "width:10px;height:10px;visibility:hidden",
    ] {
        let document = dom(&format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10' style='display:block;{style}'><rect width='10' height='10'/></svg>"
        ));
        let layout = completed(
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new(),
            )
            .unwrap(),
        );
        let page = layout.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|f| page.dom().local_name(f.node()) == Some("svg"))
            .unwrap();
        assert!(page.inline_svg(&fragment).unwrap().is_none());
    }
}

#[test]
fn inline_svg_payload_reports_unsupported_resources() {
    let document = dom(
        "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10' style='display:block'><image href='https://example.invalid/image.png' width='10' height='10'/></svg>",
    );
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("svg"))
        .unwrap();
    assert!(matches!(
        page.inline_svg(&fragment),
        Err(raikiri_svg::SvgError::ExternalReference)
    ));
}

#[test]
fn inline_svg_payload_reports_source_preparation_limits() {
    let source = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><style>{}</style>{}</svg>",
        "rect {fill:red}".repeat(257),
        "<rect width='1' height='1'/>".repeat(256),
    );
    raikiri_svg::SvgDocument::parse(source.as_bytes()).expect("initial source is admitted");
    let document = dom(&source);
    let layout = completed(
        layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap(),
    );
    let page = layout.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().local_name(f.node()) == Some("svg"))
        .unwrap();
    assert!(matches!(
        page.inline_svg(&fragment),
        Err(raikiri_svg::SvgError::InvalidDocument(message))
            if message.contains("selector freezing resource limit")
    ));
}
