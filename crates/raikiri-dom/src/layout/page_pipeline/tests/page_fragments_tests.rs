//! Page projection contracts.

use super::super::super::*;
use crate::layout::test_support::with_ahem;
use raikiri_style::{Origin, build_rule_tree, cascade};
use raikiri_traits::{NodeId, PageBox};
use smol_str::SmolStr;
use taffy::Style;

#[test]
fn public_page_fragment_snapshot_is_node_ordered() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let first = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:10px"),
    );
    document.append_text(first, "first");
    let second = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:10px"),
    );
    document.append_text(second, "second");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4)
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
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let flow = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:70px;width:20px"),
    );
    document.append_text(flow, "flow");
    let fixed = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;position:fixed;top:0;left:0;width:20px;height:5px"),
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
        Some("display:block;break-before:page;height:10px"),
    );
    document.append_text(forced, "forced");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let pages = layout_page_fragments(&mut document, &cascade, page)
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
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4)
        .expect("page fragment layout should succeed");
    let events = page_fragment_events_from_pages(&document, &pages);

    assert_eq!(events.len(), 1, "one leaf placement should yield one event");
    let PageFragmentEvent::Link(event) = &events[0];
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
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4)
        .expect("page fragment layout should succeed");
    let events = page_fragment_events_from_pages(&document, &pages);

    assert_eq!(events.len(), 1);
    let PageFragmentEvent::Link(event) = &events[0];
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
    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4)
        .expect("page fragment layout should succeed");
    let events = page_fragment_events_from_pages(&document, &pages);
    let events_again = page_fragment_events_from_pages(&document, &pages);

    assert_eq!(events, events_again);
    let anchors: Vec<_> = events
        .iter()
        .map(|event| match event {
            PageFragmentEvent::Link(event) => event.anchor_node_id,
        })
        .collect();
    assert!(anchors.windows(2).all(|ids| ids[0] <= ids[1]));
}

#[test]
fn page_fragment_link_events_skip_documents_without_a_body() {
    let document = Document::new();
    let pages = [PageFragment::default()];
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
    let page = PageFragment {
        items: vec![
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
        ],
        ..Default::default()
    };

    let events = page_fragment_events_from_pages(&document, &[page]);
    let placements: Vec<_> = events
        .iter()
        .map(|event| match event {
            PageFragmentEvent::Link(event) => event.placement_node_id,
        })
        .collect();
    assert!(!placements.contains(&NodeId::new(anchor as u64)));
    assert!(placements.contains(&NodeId::new(span as u64)));
    assert!(placements.contains(&NodeId::new(direct_text as u64)));
}

#[test]
fn page_fragment_margin_padding_coordinates_are_applied_once() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let box_id = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:10px;height:10px"),
    );
    document.append_text(box_id, "box");

    let mut rules = build_rule_tree(&document);
    rules.add_stylesheet("@page { margin:20px; padding:5px; }", Origin::Author);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let mut page_box = PageBox::new();
    page_box.width = 100.0;
    page_box.height = 100.0;
    let pages = layout_page_fragments(&mut document, &cascade, page_box)
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
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let named = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;page:named"),
    );
    let out = document.append_element(
        Some(named),
        "div",
        Style::default(),
        Some("display:block;position:absolute;top:0"),
    );
    document.append_text(out, "out of flow");
    let flow = document.append_element(Some(named), "div", Style::default(), Some("display:block"));
    document.append_text(flow, "in flow");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade named-page fixture");

    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4)
        .expect("layout named-page fixture");
    assert!(!pages.is_empty());
}

fn text_fragments(
    doc: &Document,
    cascade: &raikiri_style::CascadeResult,
    page: PageBox,
    slices: &[PageSlice],
    text: usize,
) -> Vec<(u32, Option<PageFragmentLineRange>)> {
    page_fragments_from_slices(doc, cascade, page, slices)
        .iter()
        .flat_map(|p| p.items.iter())
        .filter(|item| item.node_id == NodeId::new(text as u64))
        .map(|item| (item.page_index, item.line_range))
        .collect()
}

#[test]
fn an_ifc_paragraph_across_a_page_reports_the_same_line_ranges_as_parley() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    let mut ranges = Vec::new();
    for ifc in [false, true] {
        // Ahem at 10px, width 40: five lines of 10px; a 30px page holds three.
        let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc dddd eeee", "width:40px");
        if ifc {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 30.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        let text = doc.nodes[root].children[0];
        ranges.push(text_fragments(&doc, &cascade, page, &slices, text));
    }
    assert_eq!(
        ranges[0],
        [
            (0, Some(PageFragmentLineRange::new(0, 3))),
            (1, Some(PageFragmentLineRange::new(3, 5))),
        ],
        "the parley path is the oracle"
    );
    assert_eq!(ranges[1], ranges[0]);
}

#[test]
fn a_text_inside_a_span_starts_at_the_root_content_origin() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    let (mut doc, _cascade, root) =
        ahem_paragraph("", "width:40px;padding:5px;box-sizing:content-box");
    doc.append_text(root, "aa ");
    let span = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline;padding-left:3px"),
    );
    let text = doc.append_text(span, "bbbb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(
        doc.nodes[root].is_ifc_root(),
        "the paragraph is laid out by the engine"
    );
    let item = page_fragments_from_slices(&doc, &cascade, page, &slices)
        .iter()
        .flat_map(|p| p.items.iter().cloned())
        .find(|item| item.node_id == NodeId::new(text as u64))
        .expect("the text has a fragment");
    // The root is the first child of html > body (no margins): its content
    // box starts at (5, 5). "bbbb" wraps to the second line (top 10), and the
    // span's own box on that line must not be added a second time.
    assert_eq!((item.rect.x, item.rect.y), (5.0, 15.0));
    assert_eq!(item.line_range, Some(PageFragmentLineRange::new(0, 1)));
}

#[test]
fn a_text_that_starts_below_the_first_line_splits_at_its_own_lines() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    // Lines (Ahem 10px, width 40): "aa", "bbbb", "cccc", "dddd". The span's
    // text owns lines 1-3 (tops 10, 20, 30); a 30px page holds the first
    // three lines, so the span's text has two lines on page 0 and one on
    // page 1, counted from its own first line.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:40px");
    doc.append_text(root, "aa ");
    let span = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline"),
    );
    let text = doc.append_text(span, "bbbb cccc dddd");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 30.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(
        doc.nodes[root].is_ifc_root(),
        "the paragraph is laid out by the engine"
    );
    assert_eq!(
        text_fragments(&doc, &cascade, page, &slices, text),
        [
            (0, Some(PageFragmentLineRange::new(0, 2))),
            (1, Some(PageFragmentLineRange::new(2, 3))),
        ]
    );
}

#[test]
fn an_ifc_paragraph_that_would_leave_one_line_behind_moves_like_the_parley_one() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    let mut moved = Vec::new();
    for ifc in [false, true] {
        let (mut doc, _cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:40px");
        let body = doc.parent_of(root).expect("body");
        let spacer = doc.append_element(
            Some(body),
            "div",
            taffy::Style::default(),
            Some("display:block;height:25px"),
        );
        doc.nodes[body].children.retain(|&c| c != spacer);
        doc.nodes[body].children.insert(0, spacer);
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 40.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[root].is_ifc_root());
        let text = doc.nodes[root].children[0];
        let lines_per_page: Vec<(u32, usize)> = text_fragments(&doc, &cascade, page, &slices, text)
            .iter()
            .filter_map(|(page_index, range)| {
                range.map(|r| (*page_index, (r.end - r.start) as usize))
            })
            .collect();
        moved.push(lines_per_page);
    }
    assert_eq!(
        moved[0],
        [(1, 3)],
        "orphans:2 moves the whole paragraph to page 2 (parley oracle)"
    );
    assert_eq!(moved[1], moved[0]);
}

/// Lay out `html > body > (spacer, root)` with a fixed-height root of Ahem
/// text on 100px-wide pages, OFF and then ON, and return for each run the
/// page count and the pages the root's first text has a fragment on.
fn pages_of_a_fixed_height_paragraph(
    text: &str,
    root_css: &str,
    spacer_height: f32,
    page_height: f32,
) -> Vec<(usize, Vec<u32>)> {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    let mut results = Vec::new();
    for ifc in [false, true] {
        let (mut doc, _cascade, root) = ahem_paragraph(text, root_css);
        let body = doc.parent_of(root).expect("body");
        let spacer = doc.append_element(
            Some(body),
            "div",
            taffy::Style::default(),
            Some(format!("display:block;height:{spacer_height}px").as_str()),
        );
        doc.nodes[body].children.retain(|&c| c != spacer);
        doc.nodes[body].children.insert(0, spacer);
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = page_height;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[root].is_ifc_root());
        let text = doc.nodes[root].children[0];
        let on_page: Vec<u32> = text_fragments(&doc, &cascade, page, &slices, text)
            .iter()
            .map(|(page_index, _)| *page_index)
            .collect();
        results.push((slices.len(), on_page));
    }
    results
}

#[test]
fn a_paragraph_whose_first_line_overflows_the_page_moves_like_the_parley_one() {
    // The 20px paragraph at y 10 ends at 30 > 25: it moves to page 1 as a unit.
    let results = pages_of_a_fixed_height_paragraph(
        "aaaa bbbb",
        "width:40px;height:20px;orphans:1;widows:1",
        10.0,
        25.0,
    );
    assert_eq!(results[0], (2, vec![1]), "the parley path is the oracle");
    assert_eq!(results[1], results[0]);
}

#[test]
fn a_padded_paragraph_whose_first_line_overflows_moves_like_the_parley_one() {
    // The root's border box (22px tall, it fits a 26px page) starts at 5 and
    // its content box at 7: the lines end at 27 > 26 and the paragraph moves.
    // Measured from the border box they would end at 25 and stay.
    let results = pages_of_a_fixed_height_paragraph(
        "aaaa bbbb",
        "width:40px;height:20px;padding-top:2px;box-sizing:content-box;orphans:1;widows:1",
        5.0,
        26.0,
    );
    assert_eq!(results[0], (2, vec![1]), "the parley path is the oracle");
    assert_eq!(results[1], results[0]);
}

/// `html > body > (15px spacer, root)`: the root holds `<span>aa </span>`
/// on line 0 and a direct text "bbbb cccc" on lines 1 and 2 (Ahem 10px,
/// width 40), so the direct text starts 10px below the content box. Returns,
/// OFF and then ON, the text's line ranges per page.
fn ranges_of_a_text_below_a_span(
    page_height: f32,
) -> Vec<Vec<(u32, Option<PageFragmentLineRange>)>> {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    let mut results = Vec::new();
    for ifc in [false, true] {
        let (mut doc, _cascade, root) = ahem_paragraph("", "width:40px");
        let span = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline"),
        );
        doc.append_text(span, "aa ");
        let text = doc.append_text(root, "bbbb cccc");
        let body = doc.parent_of(root).expect("body");
        let spacer = doc.append_element(
            Some(body),
            "div",
            taffy::Style::default(),
            Some("display:block;height:15px"),
        );
        doc.nodes[body].children.retain(|&c| c != spacer);
        doc.nodes[body].children.insert(0, spacer);
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = page_height;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[root].is_ifc_root());
        results.push(text_fragments(&doc, &cascade, page, &slices, text));
    }
    results
}

#[test]
fn a_text_that_starts_on_a_later_line_is_checked_for_orphans_from_that_line() {
    // The text's lines end at 35 and 45 on a 40px page: one line would be
    // left behind, so orphans:2 moves the paragraph to page 1.
    let results = ranges_of_a_text_below_a_span(40.0);
    assert_eq!(
        results[1],
        [(1, Some(PageFragmentLineRange::new(0, 2)))],
        "the ifc text moves with its paragraph"
    );
}

#[test]
fn a_text_that_starts_on_a_later_line_stays_when_its_lines_fit() {
    // The text's lines end at 35 and 45 on a 46px page: both fit.
    let results = ranges_of_a_text_below_a_span(46.0);
    assert_eq!(
        results[1],
        [(0, Some(PageFragmentLineRange::new(0, 2)))],
        "the ifc text stays on page 0"
    );
}

/// `body > section(page:a) > (div(page:b) "x", root "aaaa", next "bbbb")`,
/// OFF and then ON: the roots are not page candidates of their own, so the
/// page change back to `a` is found at the first root's text. Returns the
/// slices' page names and the pages the two roots' texts have fragments on.
#[allow(clippy::type_complexity)]
fn pages_of_a_paragraph_after_a_named_box() -> Vec<(Vec<Option<String>>, Vec<u32>, Vec<u32>)> {
    use crate::layout::ifc::test_support::{Fixture, block_fixture};
    use crate::layout::test_support::ifc_ahem_fonts;
    let mut results = Vec::new();
    for ifc in [false, true] {
        let Fixture { mut doc, root, .. } =
            block_fixture("line-height:10px;width:40px", |doc, root| {
                doc.append_text(root, "aaaa");
            });
        let body = doc.parent_of(root).expect("body");
        let section = doc.append_element(
            Some(body),
            "section",
            taffy::Style::default(),
            Some("display:block;page:a"),
        );
        let named = doc.append_element(
            Some(section),
            "div",
            taffy::Style::default(),
            Some("display:block;page:b;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(named, "x");
        doc.detach_from_parent(root);
        doc.append_child(section, root).expect("move the root");
        let next = doc.append_element(
            Some(section),
            "div",
            taffy::Style::default(),
            Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:40px"),
        );
        doc.append_text(next, "bbbb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[root].is_ifc_root());
        assert!(doc.nodes[next].is_ifc_root());
        let on_page = |id: usize| -> Vec<u32> {
            text_fragments(&doc, &cascade, page, &slices, doc.nodes[id].children[0])
                .iter()
                .map(|(page_index, _)| *page_index)
                .collect()
        };
        results.push((
            slices.iter().map(|s| s.page_name.clone()).collect(),
            on_page(root),
            on_page(next),
        ));
    }
    results
}

#[test]
fn a_paragraph_whose_text_changes_the_page_name_moves_like_the_parley_one() {
    let results = pages_of_a_paragraph_after_a_named_box();
    assert_eq!(
        results[0],
        (
            vec![Some("b".to_owned()), Some("a".to_owned())],
            vec![1],
            vec![1]
        ),
        "the parley path is the oracle"
    );
    assert_eq!(results[1], results[0]);
}

#[test]
fn paginating_an_ifc_paragraph_twice_gives_the_same_fragments() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts};
    // The paragraph is moved to page 1 (orphans) on every pass; a second pass
    // must start again from the laid-out positions, not add a second move.
    let (mut doc, _cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:40px");
    let body = doc.parent_of(root).expect("body");
    let spacer = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;height:25px"),
    );
    doc.nodes[body].children.retain(|&c| c != spacer);
    doc.nodes[body].children.insert(0, spacer);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 40.0;
    let text = doc.nodes[root].children[0];
    let mut passes = Vec::new();
    for _ in 0..2 {
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        let items: Vec<_> = page_fragments_from_slices(&doc, &cascade, page, &slices)
            .iter()
            .flat_map(|p| p.items.iter())
            .filter(|item| item.node_id == NodeId::new(text as u64))
            .map(|item| (item.page_index, item.line_range, item.rect))
            .collect();
        passes.push(items);
    }
    assert_eq!(passes[0].len(), 1, "{:?}", passes[0]);
    assert_eq!(passes[0][0].0, 1, "the paragraph is on page 1");
    assert_eq!(passes[1], passes[0]);
}

#[test]
fn a_box_moved_inside_a_body_paragraph_is_not_moved_again_by_the_text_after_it() {
    use crate::layout::test_support::ifc_ahem_fonts;
    // body: "aa " + a 20px block of two lines + " dd". The block's first line
    // ends at 30 > 25, so the block moves to page 1 as a unit; the text after
    // it comes later in the same paragraph.
    let mut results = Vec::new();
    for ifc in [false, true] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:40px"),
        );
        doc.append_text(body, "aa ");
        let block = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:20px;orphans:1;widows:1"),
        );
        let inner = doc.append_text(block, "bbbb cccc");
        doc.append_text(body, " dd");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 25.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[body].is_ifc_root());
        let rects: Vec<_> = page_fragments_from_slices(&doc, &cascade, page, &slices)
            .iter()
            .flat_map(|p| p.items.iter())
            .filter(|item| item.node_id == NodeId::new(inner as u64))
            .map(|item| (item.page_index, item.rect.y))
            .collect();
        results.push(rects);
    }
    assert_eq!(results[0], [(1, 0.0)], "the parley path is the oracle");
    assert_eq!(results[1], results[0]);
}

#[test]
fn pagination_keeps_an_atomic_inside_a_span_on_its_line() {
    // `aa <span>bb<b></b></span>` in a body paragraph: the inline-block's
    // page fragment is where the line put it, not offset again by the span.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:100px"),
    );
    doc.append_text(body, "aa ");
    let span = doc.append_element(Some(body), "span", Style::default(), None::<&str>);
    doc.append_text(span, "bb");
    let atomic = doc.append_element(
        Some(span),
        "b",
        Style::default(),
        Some("display:inline-block;width:20px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(
        crate::layout::test_support::ifc_ahem_fonts(),
        shodo::limits::Limits::default(),
    );
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(doc.nodes[body].is_ifc_root());
    let rects: Vec<_> = page_fragments_from_slices(&doc, &cascade, page, &slices)
        .iter()
        .flat_map(|p| p.items.iter())
        .filter(|item| item.node_id == NodeId::new(atomic as u64))
        .map(|item| (item.page_index, item.rect.x, item.rect.y))
        .collect();
    // Hand-computed: "aa bb" is 50px, the atomic follows on the first line.
    assert_eq!(rects, [(0, 50.0, 0.0)]);
}

#[test]
fn a_block_inside_a_span_of_a_body_paragraph_moves_to_the_next_page_like_a_direct_child() {
    // As a_box_moved_inside_a_body_paragraph_is_not_moved_again_by_the_text_after_it,
    // with the block inside a span: pagination finds it through the span.
    let mut results = Vec::new();
    for nested in [false, true] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:40px"),
        );
        doc.append_text(body, "aa ");
        let parent = if nested {
            doc.append_element(Some(body), "span", Style::default(), None::<&str>)
        } else {
            body
        };
        doc.append_text(parent, "bb");
        let block = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;height:20px;orphans:1;widows:1"),
        );
        let inner = doc.append_text(block, "bbbb cccc");
        doc.append_text(body, " dd");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        doc.enable_inline_formatting(
            crate::layout::test_support::ifc_ahem_fonts(),
            shodo::limits::Limits::default(),
        );
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 25.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[body].is_ifc_root());
        let rects: Vec<_> = page_fragments_from_slices(&doc, &cascade, page, &slices)
            .iter()
            .flat_map(|p| p.items.iter())
            .filter(|item| item.node_id == NodeId::new(inner as u64))
            .map(|item| (item.page_index, item.rect.y))
            .collect();
        results.push(rects);
    }
    assert_eq!(results[0], [(1, 0.0)]);
    assert_eq!(results[1], results[0]);
}

#[test]
fn a_block_inside_a_span_that_fits_its_page_stays_there() {
    // "aa", a `<br>` and the span's "bb" make the first two 10px lines; the
    // 20px block follows at y = 20 and ends at 40, inside the 45px page. The
    // span's own location (10px down) is not added to the block's when
    // pagination checks whether it fits.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:40px"),
    );
    doc.append_text(body, "aa");
    doc.append_element(Some(body), "br", Style::default(), Some("display:inline"));
    let span = doc.append_element(Some(body), "span", Style::default(), None::<&str>);
    doc.append_text(span, "bb");
    let block = doc.append_element(
        Some(span),
        "div",
        Style::default(),
        Some("display:block;height:20px;orphans:1;widows:1"),
    );
    let inner = doc.append_text(block, "cccc");
    doc.append_text(body, " dd");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(
        crate::layout::test_support::ifc_ahem_fonts(),
        shodo::limits::Limits::default(),
    );
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 45.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(doc.nodes[body].is_ifc_root());
    let rects: Vec<_> = page_fragments_from_slices(&doc, &cascade, page, &slices)
        .iter()
        .flat_map(|p| p.items.iter())
        .filter(|item| item.node_id == NodeId::new(inner as u64))
        .map(|item| (item.page_index, item.rect.y))
        .collect();
    assert_eq!(rects, [(0, 20.0)]);
}

/// `body > ["aa", div(css) > "bb", " cc"]` in a 100x50 page; returns the page
/// index and y of the div's text, the page index and y of " cc", and the page
/// count.
fn forced_break_in_a_body_paragraph(css: &str, ifc: bool) -> ((u32, f32), (u32, f32), usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(body, "aa");
    let block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(&format!("display:block;{css}")),
    );
    let inner = doc.append_text(block, "bb");
    let tail = doc.append_text(body, " cc");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    if ifc {
        doc.enable_inline_formatting(
            crate::layout::test_support::ifc_ahem_fonts(),
            shodo::limits::Limits::default(),
        );
    }
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(doc.nodes[body].is_ifc_root(), "{css}");
    let fragments = page_fragments_from_slices(&doc, &cascade, page, &slices);
    let first = |node: usize| {
        fragments
            .iter()
            .flat_map(|p| p.items.iter())
            .find(|item| item.node_id == NodeId::new(node as u64))
            .map(|item| (item.page_index, item.rect.y))
            .expect("fragment")
    };
    (first(inner), first(tail), slices.len())
}

#[test]
fn a_forced_break_before_a_block_of_a_body_paragraph_starts_a_page() {
    let mut expected_1167 = [
        ((1, 0.0), (1, 10.0), 2),
        ((0, 10.0), (1, 0.0), 2),
        ((1, 0.0), (2, 0.0), 3),
    ]
    .into_iter();
    for css in ["break-before:page", "break-after:page", "page:chapter"] {
        assert_eq!(
            forced_break_in_a_body_paragraph(css, true),
            expected_1167.next().expect("a value per case"),
            "{css}"
        );
    }
    // Hand-computed: "aa" on the first page; the block's "bb" starts the
    // second page and " cc" follows it there.
    assert_eq!(
        forced_break_in_a_body_paragraph("break-before:page", true),
        ((1, 0.0), (1, 10.0), 2)
    );
    // After the block: "aa" and "bb" on the first page, " cc" on the second.
    assert_eq!(
        forced_break_in_a_body_paragraph("break-after:page", true),
        ((0, 10.0), (1, 0.0), 2)
    );
}

#[test]
fn a_forced_break_after_text_inside_a_span_starts_a_page() {
    // The body paragraph's only text is inside a link: the break before the
    // block is still not the first thing on the page.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let link = doc.append_element(Some(body), "a", Style::default(), None::<&str>);
    doc.append_text(link, "aa");
    let block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;break-before:page"),
    );
    let inner = doc.append_text(block, "bb");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(
        crate::layout::test_support::ifc_ahem_fonts(),
        shodo::limits::Limits::default(),
    );
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(doc.nodes[body].is_ifc_root());
    assert_eq!(slices.len(), 2);
    let rects: Vec<_> = page_fragments_from_slices(&doc, &cascade, page, &slices)
        .iter()
        .flat_map(|p| p.items.iter())
        .filter(|item| item.node_id == NodeId::new(inner as u64))
        .map(|item| (item.page_index, item.rect.y))
        .collect();
    assert_eq!(rects, [(1, 0.0)]);
}

#[test]
fn two_forced_breaks_in_one_body_paragraph_start_two_pages() {
    let run = |ifc: bool| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        doc.append_text(body, "aa");
        let mut texts = Vec::new();
        for (inner, tail) in [("bb", " cc"), ("dd", " ee")] {
            let block = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:block;break-before:page"),
            );
            texts.push(doc.append_text(block, inner));
            texts.push(doc.append_text(body, tail));
        }
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(
                crate::layout::test_support::ifc_ahem_fonts(),
                shodo::limits::Limits::default(),
            );
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 50.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[body].is_ifc_root());
        let fragments = page_fragments_from_slices(&doc, &cascade, page, &slices);
        texts
            .iter()
            .map(|&text| {
                fragments
                    .iter()
                    .flat_map(|p| p.items.iter())
                    .find(|item| item.node_id == NodeId::new(text as u64))
                    .map(|item| (item.page_index, item.rect.y))
                    .expect("fragment")
            })
            .collect::<Vec<_>>()
    };
    // Hand-computed: "bb" and " cc" on the second page, "dd" and " ee" on
    // the third.
    assert_eq!(run(true), [(1, 0.0), (1, 10.0), (2, 0.0), (2, 10.0)]);
    assert_eq!(run(true), [(1, 0.0), (1, 10.0), (2, 0.0), (2, 10.0)]);
}

#[test]
fn a_fixed_height_block_pushed_to_the_next_page_takes_the_lines_after_it() {
    // "aa", a 20px block holding "bb" that would straddle the 25px page,
    // then " cc": the block moves to the second page as a unit and " cc"
    // follows it there (on both paths).
    let run = |ifc: bool| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;width:40px"),
        );
        doc.append_text(body, "aa");
        let block = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:20px;orphans:1;widows:1"),
        );
        let inner = doc.append_text(block, "bbbb cccc");
        let tail = doc.append_text(body, " dd");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(
                crate::layout::test_support::ifc_ahem_fonts(),
                shodo::limits::Limits::default(),
            );
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 25.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        assert!(doc.nodes[body].is_ifc_root());
        let fragments = page_fragments_from_slices(&doc, &cascade, page, &slices);
        [inner, tail].map(|text| {
            fragments
                .iter()
                .flat_map(|p| p.items.iter())
                .find(|item| item.node_id == NodeId::new(text as u64))
                .map(|item| (item.page_index, item.rect.y))
                .expect("fragment")
        })
    };
    // Hand-computed: the block's two lines start the second page; " dd"
    // follows the 20px block.
    assert_eq!(run(true), [(1, 0.0), (1, 20.0)]);
    assert_eq!(run(true), [(1, 0.0), (1, 20.0)]);
}

#[test]
fn nested_named_pages_inside_paragraphs_start_their_pages() {
    // The page-name-002 shape: named boxes inside paragraphs, and text after
    // them that returns to the outer page name.
    let run = |ifc: bool| {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        let div = |doc: &mut Document, parent: usize, css: &str| {
            doc.append_element(
                Some(parent),
                "div",
                Style::default(),
                Some(&format!("display:block;{css}")),
            )
        };
        let mut texts = Vec::new();
        let d1 = div(&mut doc, body, "page:a");
        texts.push(doc.append_text(d1, "p1"));
        let d2 = div(&mut doc, body, "page:a");
        let d2b = div(&mut doc, d2, "page:b");
        texts.push(doc.append_text(d2b, "p2"));
        texts.push(doc.append_text(d2, "p3"));
        let d3 = div(&mut doc, body, "page:a");
        texts.push(doc.append_text(d3, "p3b"));
        texts.push(doc.append_text(body, "p4"));
        let d4 = div(&mut doc, body, "page:a");
        texts.push(doc.append_text(d4, "p5"));
        let d5 = div(&mut doc, body, "page:a");
        let d5a = div(&mut doc, d5, "");
        let d5b = div(&mut doc, d5a, "page:b");
        texts.push(doc.append_text(d5b, "p6"));
        texts.push(doc.append_text(d5a, "p7"));
        texts.push(doc.append_text(d5, "p7b"));
        texts.push(doc.append_text(body, "p8"));
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        if ifc {
            doc.enable_inline_formatting(
                crate::layout::test_support::ifc_ahem_fonts(),
                shodo::limits::Limits::default(),
            );
        }
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 50.0;
        let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
        let fragments = page_fragments_from_slices(&doc, &cascade, page, &slices);
        let first_fragments: Vec<_> = texts
            .iter()
            .map(|&text| {
                fragments
                    .iter()
                    .flat_map(|p| p.items.iter())
                    .find(|item| item.node_id == NodeId::new(text as u64))
                    .map(|item| (item.page_index, item.rect.y))
                    .expect("fragment")
            })
            .collect::<Vec<_>>();
        (slices.len(), first_fragments)
    };
    // Hand-computed: a page per change of page name, the text that returns
    // to `a` after a `b` box on a page of its own below it.
    assert_eq!(
        run(true),
        (
            8,
            vec![
                (0, 0.0),
                (1, 0.0),
                (2, 0.0),
                (2, 10.0),
                (3, 0.0),
                (4, 0.0),
                (5, 0.0),
                (6, 0.0),
                (6, 10.0),
                (7, 0.0)
            ]
        )
    );
    assert_eq!(
        run(true),
        (
            8,
            vec![
                (0, 0.0),
                (1, 0.0),
                (2, 0.0),
                (2, 10.0),
                (3, 0.0),
                (4, 0.0),
                (5, 0.0),
                (6, 0.0),
                (6, 10.0),
                (7, 0.0)
            ]
        )
    );
}

/// The page fragments of `ids` in a body paragraph built by `build`, laid
/// out by the engine on 100x50 pages: `(page, x, y)` of each first fragment.
fn engine_fragments_of(
    build: impl FnOnce(&mut Document, usize) -> Vec<usize>,
) -> Vec<(u32, f32, f32)> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let ids = build(&mut doc, body);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    doc.enable_inline_formatting(
        crate::layout::test_support::ifc_ahem_fonts(),
        shodo::limits::Limits::default(),
    );
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(doc.nodes[body].is_ifc_root());
    let fragments = page_fragments_from_slices(&doc, &cascade, page, &slices);
    ids.iter()
        .map(|&id| {
            fragments
                .iter()
                .flat_map(|p| p.items.iter())
                .find(|item| item.node_id == NodeId::new(id as u64))
                .map(|item| (item.page_index, item.rect.x, item.rect.y))
                .expect("fragment")
        })
        .collect()
}

#[test]
fn a_nested_inline_element_is_located_on_its_line() {
    // "aaaa " (50px) then <b>"bb " (30px) <a>"cc"</a></b>: the <a> starts at
    // x = 80 on the first line, not at its offset inside <b>.
    let fragments = engine_fragments_of(|doc, body| {
        doc.append_text(body, "aaaa ");
        let b = doc.append_element(Some(body), "b", Style::default(), None::<&str>);
        doc.append_text(b, "bb ");
        let a = doc.append_element(Some(b), "a", Style::default(), None::<&str>);
        doc.append_text(a, "cc");
        vec![b, a]
    });
    assert_eq!(fragments, [(0, 50.0, 0.0), (0, 80.0, 0.0)]);
}

#[test]
fn an_inline_element_after_a_block_moved_to_the_next_page_follows_its_line() {
    // "aa", a block with `break-before: page`, then <span>"cc"</span>: the
    // span is on the second page, on the line after the block.
    let fragments = engine_fragments_of(|doc, body| {
        doc.append_text(body, "aa");
        let block = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;break-before:page"),
        );
        doc.append_text(block, "bb");
        let span = doc.append_element(Some(body), "span", Style::default(), None::<&str>);
        doc.append_text(span, "cc");
        vec![span]
    });
    assert_eq!(fragments, [(1, 0.0, 10.0)]);
}
