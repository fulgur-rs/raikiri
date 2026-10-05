//! A page's body content listed in paint order, after a full layout.

use raikiri_dom::PaintEvent;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, NodeId, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// One label per event of page `page`: `Box(id)`, `Text(parent-id)`,
/// `Replaced(id)`, `PushClip`, `PopClip`, `PushOpacity` or `PopOpacity`.
/// Elements without an id are labelled with their tag name.
fn order(html: &str, page: u32) -> Vec<String> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!("<!doctype html><style>body{{font:10px/10px Ahem}}</style>{html}");
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result
        .pages()
        .find(|p| p.index() == page)
        .expect("page in range");
    let dom = page.dom();
    let label = |node: NodeId| -> String {
        dom.attr(node, "id")
            .or_else(|| dom.local_name(node))
            .unwrap_or("?")
            .to_owned()
    };
    let (document, cascade, _, _) = page.paint_inputs();
    document
        .page_paint_order(cascade, page.index(), page.name())
        .into_iter()
        .map(|event| match event {
            PaintEvent::PushClip(..) => "PushClip".to_owned(),
            PaintEvent::PopClip => "PopClip".to_owned(),
            PaintEvent::PushOpacity(_) => "PushOpacity".to_owned(),
            PaintEvent::PopOpacity => "PopOpacity".to_owned(),
            PaintEvent::Box(f) => format!("Box({})", label(f.node())),
            PaintEvent::Text(f) => {
                let parent = dom.parent(f.node()).expect("text has a parent");
                format!("Text({})", label(parent))
            }
            PaintEvent::Replaced(f) => format!("Replaced({})", label(f.node())),
            _ => "Other".to_owned(),
        })
        .collect()
}

fn position(events: &[String], label: &str) -> usize {
    events
        .iter()
        .position(|e| e == label)
        .unwrap_or_else(|| panic!("{label} missing from {events:?}"))
}

#[test]
fn absolute_box_comes_after_preceding_paragraph_text() {
    let events = order(
        "<body><p id=p>text</p><div id=a style='position:absolute;top:0;left:0;\
         width:50px;height:50px;background:red'></div></body>",
        0,
    );
    let text = position(&events, "Text(p)");
    let abs = position(&events, "Box(a)");
    assert!(text < abs, "{events:?}");
}

#[test]
fn negative_z_index_paints_before_in_flow_siblings() {
    let events = order(
        "<body><p id=p>text</p><div id=n style='position:relative;z-index:-1;height:10px'></div></body>",
        0,
    );
    let neg = position(&events, "Box(n)");
    let para = position(&events, "Box(p)");
    assert!(neg < para, "{events:?}");
}

#[test]
fn opacity_and_overflow_nest_around_children() {
    let events = order(
        "<body><div id=o style='opacity:.5;overflow:hidden;height:40px'><p id=c>x</p></div></body>",
        0,
    );
    let start = position(&events, "PushOpacity");
    assert_eq!(
        &events[start..start + 7],
        [
            "PushOpacity",
            "Box(o)",
            "PushClip",
            "Box(c)",
            "Text(c)",
            "PopClip",
            "PopOpacity"
        ],
        "{events:?}"
    );
}

#[test]
fn events_are_balanced_on_every_page() {
    // Three pages; the opacity element spans the first two.
    let html = "<style>@page{size:300px 100px;margin:0}</style><body>\
        <div style='opacity:.5;overflow:hidden'><p style='height:150px'>a</p></div>\
        <p style='height:80px'>b</p></body>";
    for page in 0..3 {
        let mut depth = 0i32;
        for e in order(html, page) {
            match e.as_str() {
                "PushClip" | "PushOpacity" => depth += 1,
                "PopClip" | "PopOpacity" => {
                    depth -= 1;
                    assert!(depth >= 0, "page {page}");
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "page {page}");
    }
}

#[test]
fn replaced_follows_its_box() {
    let events = order("<body><img id=i style='width:10px;height:10px'></body>", 0);
    let b = position(&events, "Box(i)");
    assert_eq!(events[b + 1], "Replaced(i)", "{events:?}");
}

#[test]
fn inline_element_boxes_come_before_paragraph_text() {
    let events = order(
        "<body><p id=p>a <span id=s style='background:red'>b</span> c</p></body>",
        0,
    );
    let span = position(&events, "Box(s)");
    let text = position(&events, "Text(p)");
    assert!(span < text, "{events:?}");
    assert!(events.contains(&"Text(s)".to_owned()), "{events:?}");
}

#[test]
fn every_event_fragment_is_a_page_fragment() {
    let html = "<body><p id=p>a <span>b</span></p><img style='width:4px;height:4px'>\
        <div style='opacity:.5;overflow:hidden'><p>x</p></div></body>";
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    let fragments: Vec<_> = document
        .page_fragments(page.index())
        .map(|f| (f.node(), f.kind(), f.fragment_index()))
        .collect();
    for event in document.page_paint_order(cascade, page.index(), page.name()) {
        let f = match event {
            PaintEvent::Box(f) | PaintEvent::Text(f) | PaintEvent::Replaced(f) => f,
            _ => continue,
        };
        assert!(
            fragments.contains(&(f.node(), f.kind(), f.fragment_index())),
            "{:?} is not a page fragment",
            f.node()
        );
    }
}

#[test]
fn multicol_containers_are_reported_as_approximations() {
    let html = "<!doctype html><body><div id=m style='column-count:2'><p>a</p><p>b</p></div>\
        <div style='column-count:1'><p>c</p></div></body>";
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    let found: Vec<_> = document
        .paint_order_approximations(cascade)
        .into_iter()
        .map(|(node, reason)| (page.dom().attr(node, "id"), reason))
        .collect();
    assert_eq!(
        found,
        [(Some("m"), "columns are listed without column clips")]
    );
}

#[test]
fn nested_multicol_reports_only_the_outermost() {
    let html = "<!doctype html><body><div id=outer style='columns:2'>\
        <div id=inner style='columns:2'><p>a</p><p>b</p></div><p>c</p></div></body>";
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    let found: Vec<_> = document
        .paint_order_approximations(cascade)
        .into_iter()
        .map(|(node, _)| page.dom().attr(node, "id"))
        .collect();
    assert_eq!(found, [Some("outer")]);
}

#[test]
fn multicol_inside_display_none_is_not_reported() {
    let html = "<!doctype html><body><div style='display:none'>\
        <div style='columns:2'><p>a</p><p>b</p></div></div><p>c</p></body>";
    let doc = parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("one page");
    let (document, cascade, _, _) = page.paint_inputs();
    assert!(document.paint_order_approximations(cascade).is_empty());
}
