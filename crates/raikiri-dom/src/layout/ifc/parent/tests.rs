use crate::layout::ifc::test_support::{block_fixture, span};
use crate::layout::layout_single_page;
use crate::layout::test_support::{LAYOUT_ORIGIN_STEPS, cascaded, page_box_800x600};
use crate::node::NodeFlags;
use crate::{Document, document::OofCoordinate};
use raikiri_layout_geometry::Point;

#[test]
fn a_box_under_inline_elements_is_located_from_the_root() {
    let mut fixture = block_fixture("", |doc, root| {
        let outer = span(doc, root, "");
        let inner = span(doc, outer, "");
        span(doc, inner, "display:inline-block");
    });
    let root = fixture.root;
    let outer = fixture.doc.nodes[root].children[0];
    let inner = fixture.doc.nodes[outer].children[0];
    let atomic = fixture.doc.nodes[inner].children[0];
    // Mark the paragraph as the assignment would: the inline elements are
    // its content, the inline-block one of its boxes.
    fixture.doc.nodes[root].flags.insert(NodeFlags::IS_IFC_ROOT);
    for id in [outer, inner] {
        fixture.doc.nodes[id]
            .flags
            .insert(NodeFlags::IN_IFC_SUBTREE);
    }
    let doc = &fixture.doc;
    assert_eq!(doc.layout_parent_of(atomic), Some(root));
    assert_eq!(doc.layout_parent_of(inner), Some(outer));
    assert_eq!(doc.layout_parent_of(outer), Some(root));
    assert!(!doc.contributes_layout_offset(inner));
    assert!(doc.contributes_layout_offset(root));
    assert!(doc.contributes_layout_offset(atomic));
}

#[test]
fn a_cyclic_oof_owner_chain_falls_back_to_its_initial_coordinate() {
    let mut doc = Document::new();
    let node = doc.append_element(Some(0), "div", None::<&str>);
    let local = Point { x: 7.0, y: 11.0 };
    doc.nodes[node].layout.location = local;
    doc.oof_coordinates.insert(
        node,
        OofCoordinate {
            owner: node,
            static_owner: node,
            follow_static_x: true,
            follow_static_y: true,
            initial_owner_origin: Some(Point::ZERO),
            initial_static_origin: Some(Point::ZERO),
        },
    );

    assert_eq!(doc.layout_offset_of(node), Some(local));
}

#[test]
fn the_origin_of_a_chain_of_positioned_boxes_takes_steps_that_follow_its_length() {
    // A positioned box takes its origin from the box it is positioned against,
    // from the one it would sit in without being positioned, and from its
    // coordinate parent. In a chain of positioned boxes those are the box
    // above, three times over, so a chain of `n` boxes used to take `3^n` steps.
    const DEPTH: usize = 12;
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let mut parent = doc.append_element(Some(html), "body", Some("display:block"));
    for _ in 0..DEPTH {
        parent = doc.append_element(
            Some(parent),
            "div",
            Some("position:absolute;left:1px;top:2px;width:50px;height:50px"),
        );
    }
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");

    LAYOUT_ORIGIN_STEPS.with(|steps| steps.set(0));
    let origin = doc.layout_origin_of(parent).expect("an origin");
    let steps = LAYOUT_ORIGIN_STEPS.with(std::cell::Cell::get);

    // Every box sits 1px right of and 2px below the one it is positioned
    // against.
    assert_eq!(origin, Point { x: 12.0, y: 24.0 });
    assert!(
        steps <= 4 * DEPTH,
        "the origin of a chain of {DEPTH} positioned boxes took {steps} steps"
    );
}
