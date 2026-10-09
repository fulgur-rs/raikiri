use super::*;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

fn fixture(depth: usize) -> (Document, usize, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0;font:20px/20px Ahem"),
    );
    let mc = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;width:100px;height:40px;column-count:2;column-gap:20px;orphans:1;widows:1"),
    );
    let mut chain = Vec::new();
    let mut parent = mc;
    for _ in 0..depth {
        parent = doc.append_element(
            Some(parent),
            "div",
            taffy::Style::default(),
            Some("display:block;width:80px;margin:0"),
        );
        chain.push(parent);
    }
    doc.append_text(parent, "A");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    (doc, mc, chain)
}

#[test]
fn cyclic_chains_are_rejected_without_returning_an_incomplete_path() {
    let (mut doc, mc, chain) = fixture(2);
    let leaf = *chain.last().unwrap();
    doc.nodes[leaf].ifc = None;
    doc.nodes[leaf].children = vec![chain[0]];
    assert!(supported(&doc, mc).is_none());
}

#[test]
fn taffy_calc_margins_resolve_against_the_fragmentainer_width() {
    let (mut doc, mc, chain) = fixture(1);
    let value = std::sync::Arc::new(raikiri_style::property::CalcLengthPercentage {
        px: 1.0,
        percent: 10.0,
    });
    let pointer = std::sync::Arc::as_ptr(&value).cast();
    doc.calc_values.push(value);
    doc.nodes[chain[0]].style.margin.left = LengthPercentageAuto::calc(pointer);
    doc.fragment_tree.fragments.clear();
    let context = FragmentationContext {
        available_width: 100.0,
        available_height: Some(40.0),
        column_fill: raikiri_style::property::ColumnFillValue::Auto,
        column_width: 40.0,
        column_count: 2,
        column_gap: 20.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    assert_eq!(
        layout(
            &mut doc,
            mc,
            &chain,
            context,
            Size {
                width: 100.0,
                height: 40.0,
            },
            Point::ZERO,
        ),
        Some(40.0)
    );
    let fragment = doc
        .fragment_tree
        .fragments
        .iter()
        .find(|fragment| fragment.node_id == chain[0])
        .unwrap();
    assert_eq!(fragment.rect.x, 5.0);
    assert_eq!(fragment.rect.width, 80.0);
}
