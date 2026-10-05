use super::*;

use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

fn cascaded(doc: &Document) -> CascadeResult {
    let rules = build_rule_tree(doc);
    cascade(doc, &rules).unwrap()
}

#[test]
fn negative_z_index_sorts_before_static_and_positive_after() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let a = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative;z-index:2"),
    );
    let b = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let c = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative;z-index:-1"),
    );
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    assert_eq!(find_paint_root(&doc), Some(body));
    let mut children = doc.get_node(body).unwrap().children.clone();
    sort_paint_children(&mut children, cr.computed[body].display, &cr);
    assert_eq!(children, [c, b, a]);
}

#[test]
fn overflow_and_opacity_predicates() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let a = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("overflow:hidden;opacity:0.5"),
    );
    let b = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    assert!(clips_overflow(&cr.computed[a]));
    assert_eq!(opacity_layer(&cr.computed[a]), Some(0.5));
    assert!(!clips_overflow(&cr.computed[b]));
    assert_eq!(opacity_layer(&cr.computed[b]), None);
    assert!(!is_visibility_hidden_table(&cr.computed[a]));
}

#[test]
fn named_page_matching_skips_check_when_no_active_page() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let cr = cascaded(&doc);
    assert!(named_page_matches(&doc, &cr, body, None));
    assert!(named_page_matches(&doc, &cr, body, Some(None)));
}

#[test]
fn flex_static_boxes_use_the_positioned_paint_bucket() {
    let mut document = Document::new();
    let flex = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    assert_eq!(paint_order_key(&cascade, flex), (2, 0));
}

#[test]
fn flex_grid_paint_order_uses_order_with_stable_source_order_ties() {
    fn parent_with_ordered_children(document: &mut Document, display: &str) -> (usize, [usize; 3]) {
        let parent = document.append_element(
            Some(document.root_index()),
            "div",
            Style::default(),
            Some(display),
        );
        let a = document.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;order:2"),
        );
        let b = document.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;order:-1"),
        );
        let c = document.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;order:2"),
        );
        (parent, [a, b, c])
    }

    let mut document = Document::new();
    let (flex, flex_items) = parent_with_ordered_children(&mut document, "display:flex");
    let absolute_first = document.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;order:10;position:absolute"),
    );
    let absolute_second = document.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;order:5;position:absolute"),
    );
    let (grid, grid_items) = parent_with_ordered_children(&mut document, "display:grid");
    let (block, block_items) = parent_with_ordered_children(&mut document, "display:block");

    // Flex/grid items with different inner display values still share one
    // order-modified paint sequence.
    let mixed_parent = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:grid"),
    );
    let nested_flex_item = document.append_element(
        Some(mixed_parent),
        "div",
        Style::default(),
        Some("display:flex;order:-1"),
    );
    let block_item = document.append_element(
        Some(mixed_parent),
        "div",
        Style::default(),
        Some("display:block;order:1"),
    );

    let z_index_parent = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let z_one_late = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("display:block;z-index:1;order:1"),
    );
    let z_two_early = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("display:block;z-index:2;order:-1"),
    );
    let z_one_early = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("display:block;z-index:1;order:0"),
    );
    let positioned_z_two = document.append_element(
        Some(z_index_parent),
        "div",
        Style::default(),
        Some("display:block;position:relative;z-index:2;order:-2"),
    );

    // Out-of-flow flex children are treated as order 0 for painting. Their
    // authored order is ignored, with DOM order breaking the 0-value tie.
    let positioned_parent = document.append_element(
        Some(document.root_index()),
        "div",
        Style::default(),
        Some("display:flex"),
    );
    let absolute_slot = document.append_element(
        Some(positioned_parent),
        "div",
        Style::default(),
        Some("display:block;position:absolute;order:100"),
    );
    let relative_late = document.append_element(
        Some(positioned_parent),
        "div",
        Style::default(),
        Some("display:block;position:relative;order:1"),
    );
    let relative_early = document.append_element(
        Some(positioned_parent),
        "div",
        Style::default(),
        Some("display:block;position:relative;order:-1"),
    );

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");

    let mut flex_children = document.get_node(flex).unwrap().children.clone();
    sort_paint_children(&mut flex_children, cascade.computed[flex].display, &cascade);
    assert_eq!(
        flex_children,
        [
            flex_items[1],
            flex_items[0],
            flex_items[2],
            absolute_first,
            absolute_second,
        ]
    );
    assert_eq!(
        document.get_node(flex).unwrap().children.as_slice(),
        &[
            flex_items[0],
            flex_items[1],
            flex_items[2],
            absolute_first,
            absolute_second,
        ]
    );

    let mut grid_children = document.get_node(grid).unwrap().children.clone();
    sort_paint_children(&mut grid_children, cascade.computed[grid].display, &cascade);
    assert_eq!(grid_children, [grid_items[1], grid_items[0], grid_items[2]]);
    assert_eq!(
        document.get_node(grid).unwrap().children.as_slice(),
        &grid_items
    );

    let mut mixed_children = document.get_node(mixed_parent).unwrap().children.clone();
    sort_paint_children(
        &mut mixed_children,
        cascade.computed[mixed_parent].display,
        &cascade,
    );
    assert_eq!(mixed_children, [nested_flex_item, block_item]);

    let mut z_index_children = document.get_node(z_index_parent).unwrap().children.clone();
    sort_paint_children(
        &mut z_index_children,
        cascade.computed[z_index_parent].display,
        &cascade,
    );
    assert_eq!(
        z_index_children,
        [z_one_early, z_one_late, positioned_z_two, z_two_early]
    );

    let mut positioned_children = document
        .get_node(positioned_parent)
        .unwrap()
        .children
        .clone();
    sort_paint_children(
        &mut positioned_children,
        cascade.computed[positioned_parent].display,
        &cascade,
    );
    assert_eq!(
        positioned_children,
        [relative_early, absolute_slot, relative_late]
    );

    let mut block_children = document.get_node(block).unwrap().children.clone();
    sort_paint_children(
        &mut block_children,
        cascade.computed[block].display,
        &cascade,
    );
    assert_eq!(block_children, block_items);
}
