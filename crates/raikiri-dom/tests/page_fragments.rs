//! Public page-fragment API reachability tests.

use parley::FontContext;
use raikiri_dom::{
    Document, PageFragmentEvent, PageFragmentItem, PageFragmentKind, PageFragmentRect, PageSlice,
    layout_page_fragments, page_fragment_events_from_pages, page_fragment_geometry_table,
    page_fragments_from_slices_with_page_geometry,
};
use raikiri_html::{ParseOptions, parse};
use raikiri_style::{Origin, build_rule_tree, cascade};
use raikiri_traits::{
    NodeId, PageBox, PageFragmentInsets, PageFragmentOrientation, PageFragmentPageGeometry,
};
use smol_str::SmolStr;
use taffy::Style;

#[test]
fn public_page_fragment_snapshot_is_node_ordered() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let first = document.append_element(Some(body), "div", Style::default(), Some("height:10px"));
    document.append_text(first, "first");
    let second = document.append_element(Some(body), "div", Style::default(), Some("height:10px"));
    document.append_text(second, "second");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4, FontContext::new())
        .expect("page fragment layout should succeed");

    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_index, 0);
    assert!(pages[0].items.iter().all(|item| item.page_index == 0));
    assert!(
        pages[0]
            .items
            .windows(2)
            .all(|items| items[0].node_id <= items[1].node_id)
    );
    assert!(
        pages[0]
            .items
            .iter()
            .any(|item| item.kind == PageFragmentKind::Text)
    );

    let table = page_fragment_geometry_table(&pages);
    let node_ids: Vec<_> = table.keys().copied().collect();
    assert!(node_ids.windows(2).all(|ids| ids[0] <= ids[1]));
    assert_eq!(
        table
            .get(&NodeId::new(first as u64))
            .expect("first geometry")
            .node_id,
        NodeId::new(first as u64)
    );
}

#[test]
fn shared_inline_paragraph_projects_line_metrics_to_text_fragments() {
    let html = br#"<!doctype html><html><head><style>html,body{margin:0}.sample{font:20px/1 monospace;width:1ch;line-break:anywhere}</style></head><body><div class="sample"><span>A</span><span>B</span><span>C</span></div></body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut parsed = parse(&html[..], &options).expect("parse shared-inline fixture");
    let rules = build_rule_tree(&parsed.dom);
    let cascade = cascade(&parsed.dom, &rules).expect("cascade shared-inline fixture");
    let pages = layout_page_fragments(&mut parsed.dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout shared-inline fixture");
    assert_eq!(pages.len(), 1);
    let text_items: Vec<_> = pages[0]
        .items
        .iter()
        .filter(|item| item.kind == PageFragmentKind::Text)
        .collect();
    assert_eq!(text_items.len(), 3);
    assert!(text_items.iter().all(|item| {
        item.line_range
            .is_some_and(|range| range.start == 0 && range.end == 1)
            && item.rect.width > 0.0
            && item.rect.height > 0.0
    }));
}

#[test]
fn public_page_fragment_item_keeps_repeat_distinct_from_split() {
    let item = PageFragmentItem::new(
        raikiri_traits::NodeId::new(7),
        PageFragmentRect::new(0.0, 0.0, 10.0, 5.0),
        PageFragmentKind::Box,
        0,
        2,
        true,
    );
    assert!(!item.is_split());
}

#[test]
fn fixed_position_subtrees_repeat_on_every_page() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flow = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("height:70px;width:20px"),
    );
    document.append_text(flow, "flow");
    let fixed = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:fixed;top:0;left:0;width:20px;height:5px"),
    );
    let fixed_text = document.append_text(fixed, "fixed");
    let fixed_link = document.append_element(
        Some(body),
        "a",
        Style::default(),
        Some("position:fixed;top:10px;left:0;width:20px;height:5px"),
    );
    document.set_element_attributes(
        fixed_link,
        vec![(SmolStr::new("href"), SmolStr::new("#fixed"))],
    );
    let fixed_link_text = document.append_text(fixed_link, "fixed link");
    let forced = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("break-before:page;height:10px"),
    );
    document.append_text(forced, "forced");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let pages = layout_page_fragments(&mut document, &cascade, page, FontContext::new())
        .expect("page fragment layout should succeed");
    assert!(pages.len() >= 2);

    let fixed_items: Vec<_> = pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| item.node_id == NodeId::new(fixed as u64))
        .collect();
    assert_eq!(fixed_items.len(), pages.len());
    assert!(fixed_items.iter().all(|item| item.is_repeat));
    assert!(fixed_items.iter().all(|item| !item.is_split()));
    assert!(
        fixed_items
            .iter()
            .all(|item| item.fragment_count as usize == pages.len())
    );
    assert_eq!(
        fixed_items
            .iter()
            .map(|item| item.page_index)
            .collect::<Vec<_>>(),
        (0..pages.len() as u32).collect::<Vec<_>>()
    );
    let fixed_text_items: Vec<_> = pages
        .iter()
        .flat_map(|page| page.items.iter())
        .filter(|item| item.node_id == NodeId::new(fixed_text as u64))
        .collect();
    assert_eq!(fixed_text_items.len(), pages.len());
    assert!(fixed_text_items.iter().all(|item| item.is_repeat));
    assert!(
        fixed_text_items
            .iter()
            .all(|item| item.line_range.is_some())
    );

    let table = page_fragment_geometry_table(&pages);
    let geometry = table
        .get(&NodeId::new(fixed as u64))
        .expect("fixed geometry");
    assert!(geometry.is_repeat);
    assert_eq!(geometry.fragments.len(), pages.len());

    let events = page_fragment_events_from_pages(&document, &pages);
    let fixed_link_events: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            PageFragmentEvent::Link(event)
                if event.anchor_node_id == NodeId::new(fixed_link as u64) =>
            {
                Some(event)
            }
            _ => None,
        })
        .collect();
    assert_eq!(fixed_link_events.len(), pages.len());
    assert!(
        fixed_link_events
            .iter()
            .all(|event| event.placement_node_id == NodeId::new(fixed_link_text as u64))
    );
    assert!(fixed_link_events.iter().all(|event| event.is_repeat));
    assert_eq!(
        fixed_link_events
            .iter()
            .map(|event| event.page_index)
            .collect::<Vec<_>>(),
        (0..pages.len() as u32).collect::<Vec<_>>()
    );
}

#[test]
fn page_fragment_link_events_preserve_anchor_and_placement_geometry() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let anchor = document.append_element(
        Some(body),
        "a",
        Style::default(),
        Some("display:block;width:80px;height:10px"),
    );
    document.set_element_attributes(
        anchor,
        vec![(SmolStr::new("href"), SmolStr::new("  #chapter  "))],
    );
    let text = document.append_text(anchor, "chapter");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4, FontContext::new())
        .expect("page fragment layout should succeed");
    let events = page_fragment_events_from_pages(&document, &pages);

    assert_eq!(events.len(), 1, "one leaf placement should yield one event");
    let PageFragmentEvent::Link(event) = &events[0] else {
        panic!("expected link event");
    };
    assert_eq!(event.anchor_node_id, NodeId::new(anchor as u64));
    assert_eq!(event.placement_node_id, NodeId::new(text as u64));
    assert_eq!(event.page_index, 0);
    assert_eq!(event.link.href, "#chapter");
    assert!(event.rect.width > 0.0 && event.rect.height > 0.0);
    assert_eq!(event.fragment_count, 1);
    assert!(!event.is_repeat);
    assert!(event.line_range.is_some());

    let placement = pages[0]
        .items
        .iter()
        .find(|item| item.node_id == NodeId::new(text as u64))
        .expect("text placement");
    assert_eq!(event.rect, placement.rect);
    assert_eq!(event.fragment_index, placement.fragment_index);
}

#[test]
fn page_fragment_link_events_keep_empty_href_and_skip_non_links_and_hidden_nodes() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let empty = document.append_element(
        Some(body),
        "a",
        Style::default(),
        Some("display:block;width:30px;height:10px"),
    );
    document.set_element_attributes(empty, vec![(SmolStr::new("href"), SmolStr::new(""))]);
    document.append_text(empty, "empty");
    let no_href = document.append_element(
        Some(body),
        "a",
        Style::default(),
        Some("display:block;width:30px;height:10px"),
    );
    document.append_text(no_href, "not a link");
    let hidden = document.append_element(Some(body), "a", Style::default(), Some("display:none"));
    document.set_element_attributes(
        hidden,
        vec![(SmolStr::new("href"), SmolStr::new("#hidden"))],
    );
    document.append_text(hidden, "hidden");
    let link_element = document.append_element(
        Some(body),
        "link",
        Style::default(),
        Some("display:block;width:30px;height:10px"),
    );
    document.set_element_attributes(
        link_element,
        vec![(SmolStr::new("href"), SmolStr::new("#stylesheet"))],
    );

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4, FontContext::new())
        .expect("page fragment layout should succeed");
    let events = page_fragment_events_from_pages(&document, &pages);

    assert_eq!(events.len(), 1);
    let PageFragmentEvent::Link(event) = &events[0] else {
        panic!("expected link event");
    };
    assert_eq!(event.anchor_node_id, NodeId::new(empty as u64));
    assert_eq!(event.link.href, "");
}

#[test]
fn page_fragment_link_event_order_is_page_and_node_deterministic() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    for (href, text) in [("#second", "second"), ("#first", "first")] {
        let anchor = document.append_element(
            Some(body),
            "a",
            Style::default(),
            Some("display:block;width:40px;height:10px"),
        );
        document.set_element_attributes(anchor, vec![(SmolStr::new("href"), SmolStr::new(href))]);
        document.append_text(anchor, text);
    }
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4, FontContext::new())
        .expect("page fragment layout should succeed");
    let events = page_fragment_events_from_pages(&document, &pages);
    let events_again = page_fragment_events_from_pages(&document, &pages);

    assert_eq!(events, events_again);
    let anchors: Vec<_> = events
        .iter()
        .map(|event| match event {
            PageFragmentEvent::Link(event) => event.anchor_node_id,
            _ => unreachable!("only link events are currently emitted"),
        })
        .collect();
    assert!(anchors.windows(2).all(|ids| ids[0] <= ids[1]));
}

#[test]
fn page_fragment_link_events_skip_documents_without_a_body() {
    let document = Document::new();
    let pages = [raikiri_traits::PageFragment::default()];
    assert!(page_fragment_events_from_pages(&document, &pages).is_empty());
}

#[test]
fn page_fragment_link_events_prefer_leaf_placements_and_keep_box_fallbacks() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let anchor = document.append_element(Some(body), "a", Style::default(), None::<&str>);
    document.set_element_attributes(
        anchor,
        vec![(SmolStr::new("href"), SmolStr::new("#target"))],
    );
    let span = document.append_element(Some(anchor), "span", Style::default(), None::<&str>);
    let direct_text = document.append_text(anchor, "direct");
    document.append_text(span, "nested");

    let rect = PageFragmentRect::new(0.0, 0.0, 10.0, 5.0);
    let mut page = raikiri_traits::PageFragment::default();
    page.items = vec![
        PageFragmentItem::new(
            NodeId::new(anchor as u64),
            rect,
            PageFragmentKind::Box,
            0,
            1,
            false,
        ),
        PageFragmentItem::new(
            NodeId::new(span as u64),
            rect,
            PageFragmentKind::Box,
            0,
            1,
            false,
        ),
        PageFragmentItem::new(
            NodeId::new(direct_text as u64),
            rect,
            PageFragmentKind::Text,
            0,
            1,
            false,
        ),
    ];

    let events = page_fragment_events_from_pages(&document, &[page]);
    let placements: Vec<_> = events
        .iter()
        .map(|event| match event {
            PageFragmentEvent::Link(event) => event.placement_node_id,
            _ => unreachable!("only link events are currently emitted"),
        })
        .collect();
    assert!(!placements.contains(&NodeId::new(anchor as u64)));
    assert!(placements.contains(&NodeId::new(span as u64)));
    assert!(placements.contains(&NodeId::new(direct_text as u64)));
}

#[test]
fn page_fragment_margin_padding_coordinates_are_applied_once() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let box_id = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    document.append_text(box_id, "box");

    let mut rules = build_rule_tree(&document);
    rules.add_stylesheet("@page { margin:20px; padding:5px; }", Origin::Author);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut page_box = PageBox::new();
    page_box.width = 100.0;
    page_box.height = 100.0;
    let pages = layout_page_fragments(&mut document, &cascade, page_box, FontContext::new())
        .expect("page fragment layout should succeed");

    assert_eq!(pages.len(), 1);
    let page = &pages[0];
    assert_eq!(page.page_box, page_box);
    assert_eq!(
        page.margins,
        PageFragmentInsets::new(20.0, 20.0, 20.0, 20.0)
    );
    assert_eq!(
        page.content_insets,
        PageFragmentInsets::new(5.0, 5.0, 5.0, 5.0)
    );
    assert_eq!(
        page.content_box,
        PageFragmentRect::new(25.0, 25.0, 60.0, 50.0)
    );

    let item = page
        .items
        .iter()
        .find(|item| item.node_id == NodeId::new(box_id as u64))
        .expect("box placement");
    assert_eq!(item.rect.x, 0.0);
    assert_eq!(item.rect.y, 0.0);
    assert_eq!(
        (
            page.content_box.x + item.rect.x,
            page.content_box.y + item.rect.y
        ),
        (25.0, 25.0),
        "consumer adds the content-box origin exactly once"
    );
}

#[test]
fn page_fragment_projection_keeps_distinct_resolved_geometry_per_page() {
    let document = Document::new();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut fallback_page = PageBox::new();
    fallback_page.width = 100.0;
    fallback_page.height = 100.0;
    let slices = [
        PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: Some("first".to_string()),
        },
        PageSlice {
            page_index: 1,
            content_origin_y: 80.0,
            page_name: Some("wide".to_string()),
        },
    ];
    let first = PageFragmentPageGeometry::new(
        0,
        fallback_page,
        PageFragmentInsets::new(10.0, 10.0, 10.0, 10.0),
        PageFragmentInsets::new(2.0, 2.0, 2.0, 2.0),
        PageFragmentRect::new(12.0, 12.0, 76.0, 76.0),
        PageFragmentOrientation::Portrait,
    );
    let second = PageFragmentPageGeometry::new(
        1,
        {
            let mut page = PageBox::new();
            page.width = 200.0;
            page.height = 100.0;
            page
        },
        PageFragmentInsets::new(20.0, 15.0, 20.0, 15.0),
        PageFragmentInsets::new(4.0, 3.0, 4.0, 3.0),
        PageFragmentRect::new(18.0, 24.0, 164.0, 52.0),
        PageFragmentOrientation::Landscape,
    );

    let pages = page_fragments_from_slices_with_page_geometry(
        &document,
        &cascade,
        fallback_page,
        &slices,
        &[first, second],
    );

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].page_box, fallback_page);
    assert_eq!(pages[0].content_box, first.content_box);
    assert_eq!(pages[0].page_name.as_deref(), Some("first"));
    assert_eq!(pages[1].page_box.width, 200.0);
    assert_eq!(pages[1].margins, second.margins);
    assert_eq!(pages[1].content_insets, second.content_insets);
    assert_eq!(pages[1].content_box, second.content_box);
    assert_eq!(pages[1].orientation, PageFragmentOrientation::Landscape);
    assert_eq!(pages[1].page_name.as_deref(), Some("wide"));
}
#[test]
fn named_page_propagation_skips_out_of_flow_children() {
    let html = br#"<!doctype html>
        <html><body>
          <div style="page: named">
            <div style="position:absolute;top:0">out of flow</div>
            <div>in flow</div>
          </div>
        </body></html>"#;
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let mut parsed = parse(&html[..], &options).expect("parse named-page fixture");
    let rules = build_rule_tree(&parsed.dom);
    let cascade = cascade(&parsed.dom, &rules).expect("cascade named-page fixture");

    let pages = raikiri_dom::layout_page_fragments(
        &mut parsed.dom,
        &cascade,
        PageBox::A4,
        FontContext::new(),
    )
    .expect("layout named-page fixture");
    assert!(!pages.is_empty());
}
