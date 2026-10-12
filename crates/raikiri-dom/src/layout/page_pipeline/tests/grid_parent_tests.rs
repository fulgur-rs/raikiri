use super::*;
use crate::layout::test_support::GRID_PARENT_LOOKUP_STEPS;
use raikiri_style::{build_rule_tree, cascade};

fn fixture(display: &str, children: &[&str]) -> (Document, CascadeResult, usize, Vec<usize>) {
    let mut doc = Document::new();
    let parent = doc.append_element(
        Some(0),
        "div",
        Style::default(),
        Some(format!(
            "display:{display};border:2px solid;padding:5px;align-items:center"
        )),
    );
    let children: Vec<_> = children
        .iter()
        .map(|css| doc.append_element(Some(parent), "div", Style::default(), Some(*css)))
        .collect();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    doc.nodes[parent].style.padding = taffy::Rect {
        left: taffy::LengthPercentage::length(5.0),
        right: taffy::LengthPercentage::length(5.0),
        top: taffy::LengthPercentage::length(5.0),
        bottom: taffy::LengthPercentage::length(5.0),
    };
    doc.nodes[parent].unrounded_layout.size = taffy::Size {
        width: 140.0,
        height: 100.0,
    };
    for &child in &children {
        doc.nodes[child].unrounded_layout.size = taffy::Size {
            width: 30.0,
            height: 20.0,
        };
        doc.nodes[child].unrounded_layout.location = taffy::Point { x: 71.0, y: 73.0 };
    }
    (doc, cascade, parent, children)
}

#[test]
fn grid_parent_lookup_work_is_linear_for_many_positioned_siblings() {
    for display in ["grid", "inline-grid", "block"] {
        for count in [0, 32, 128, 512] {
            let styles: Vec<_> = (0..count)
                .map(|id| {
                    if id % 2 == 0 {
                        "position:absolute"
                    } else {
                        "position:fixed"
                    }
                })
                .collect();
            let (mut doc, cascade, _, children) = fixture(display, &styles);
            GRID_PARENT_LOOKUP_STEPS.with(|steps| steps.set(0));
            realign_grid_abspos_static_positions(&mut doc, &cascade);
            let steps = GRID_PARENT_LOOKUP_STEPS.with(Cell::get);
            assert!(
                steps <= doc.nodes.len(),
                "{display}, {count}: {steps} parent lookup steps"
            );
            for child in children {
                let layout = doc.nodes[child].unrounded_layout;
                let expected = if display == "block" {
                    (71.0, 73.0)
                } else {
                    (7.0, 40.0)
                };
                assert_eq!((layout.location.x, layout.location.y), expected);
                assert_eq!((layout.size.width, layout.size.height), (30.0, 20.0));
            }
        }
    }
}

#[test]
fn grid_parent_lookup_preserves_explicit_placement_and_partial_insets() {
    let (mut doc, cascade, _, children) = fixture(
        "grid",
        &[
            "position:absolute;left:3px",
            "position:fixed;top:4px",
            "position:absolute;right:5px",
            "position:absolute;bottom:6px",
            "position:absolute;grid-row-start:1",
            "position:absolute;grid-row-end:2",
            "position:absolute;grid-column-start:1",
            "position:absolute;grid-column-end:2",
            "position:static",
        ],
    );
    realign_grid_abspos_static_positions(&mut doc, &cascade);
    for child in children {
        let layout = doc.nodes[child].unrounded_layout;
        assert_eq!((layout.location.x, layout.location.y), (71.0, 73.0));
        assert_eq!((layout.size.width, layout.size.height), (30.0, 20.0));
    }
}

#[test]
fn grid_parent_lookup_follows_reparenting_and_skips_detached_nodes() {
    let (mut doc, mut cascade, parent, children) =
        fixture("grid", &["position:absolute", "position:fixed"]);
    let new_parent =
        doc.append_element(Some(0), "section", Style::default(), Some("display:block"));
    cascade.computed.push(cascade.computed[0].clone());
    doc.detach_from_parent(children[0]);
    doc.append_child(new_parent, children[1]).unwrap();
    assert!(doc.nodes[parent].children.is_empty());
    realign_grid_abspos_static_positions(&mut doc, &cascade);
    for child in children {
        let layout = doc.nodes[child].unrounded_layout;
        assert_eq!((layout.location.x, layout.location.y), (71.0, 73.0));
    }
}
