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
