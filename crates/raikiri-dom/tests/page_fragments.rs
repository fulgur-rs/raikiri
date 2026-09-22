//! Public page-fragment API reachability tests.

use parley::FontContext;
use raikiri_dom::{
    Document, PageFragmentItem, PageFragmentKind, PageFragmentRect, layout_page_fragments,
    page_fragment_geometry_table,
};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{NodeId, PageBox};
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
}
