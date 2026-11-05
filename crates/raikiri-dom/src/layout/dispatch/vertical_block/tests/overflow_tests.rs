use super::super::*;
use crate::layout::test_support::{page_box_800x600, with_ahem};

fn overflow_rects(
    container_css: &str,
    build: impl FnOnce(&mut Document, usize) -> Vec<usize>,
) -> Vec<Edges<f32>> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:400px;height:300px"),
    );
    let container = doc.append_element(
        Some(parent),
        "div",
        Some(&format!(
            "display:block;width:100px;height:100px;margin-left:150px;{container_css}"
        )),
    );
    let mut nodes = vec![container, parent];
    nodes.extend(build(&mut doc, container));
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    with_ahem(&mut doc);
    let mut previous = None;
    for _ in 0..2 {
        crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
        let rects: Vec<_> = nodes
            .iter()
            .map(|&id| doc.nodes[id].layout.scrollable_overflow_rect)
            .collect();
        if let Some(previous) = &previous {
            assert_eq!(
                &rects, previous,
                "repeated layout must keep physical overflow"
            );
        }
        previous = Some(rects);
    }
    previous.unwrap()
}

fn two_children(doc: &mut Document, parent: usize, css: &str) -> Vec<usize> {
    (0..2)
        .map(|_| doc.append_element(Some(parent), "div", Some(css)))
        .collect()
}

#[test]
fn vertical_block_overflow_is_physical_in_the_child_and_parent() {
    for (mode, left, right, parent_right) in [
        ("vertical-rl", -100.0, 100.0, 250.0),
        ("vertical-lr", 0.0, 200.0, 350.0),
    ] {
        let rects = overflow_rects(&format!("writing-mode:{mode}"), |doc, container| {
            two_children(doc, container, "display:block;width:100px;height:100px")
        });
        assert_eq!((rects[0].left, rects[0].right), (left, right), "{mode}");
        assert_eq!(rects[1].right, parent_right, "{mode}");
    }
}

#[test]
fn block_overflow_mirrors_inline_reversal_on_the_frame_axes() {
    for (mode, left, right, naturally_reversed) in [
        ("vertical-rl", -100.0, 100.0, false),
        ("vertical-lr", 0.0, 200.0, false),
        ("sideways-rl", -100.0, 100.0, false),
        ("sideways-lr", 0.0, 200.0, true),
    ] {
        for rtl in [false, true] {
            let direction = if rtl { "rtl" } else { "ltr" };
            let rects = overflow_rects(
                &format!("writing-mode:{mode};direction:{direction}"),
                |doc, container| {
                    two_children(doc, container, "display:block;width:100px;height:150px")
                },
            );
            let (top, bottom) = if naturally_reversed != rtl {
                (-50.0, 100.0)
            } else {
                (0.0, 150.0)
            };
            assert_eq!(
                rects[0],
                Edges {
                    left,
                    right,
                    top,
                    bottom
                },
                "{mode}/{direction}"
            );
        }
    }
}

#[test]
fn container_borders_do_not_expand_the_overflow_mirror_extent() {
    let rects = overflow_rects(
        "writing-mode:vertical-rl;border-style:solid;border-width:2px 3px 5px 7px",
        |doc, container| two_children(doc, container, "display:block;width:100px;height:100px"),
    );
    // Block flow includes the leading border before the padding-box origin.
    // The 3px block-start border and 2px inline-start border remain in the
    // rectangle; they do not increase its 100px mirror extent.
    assert_eq!(
        rects[0],
        Edges {
            left: -100.0,
            right: 103.0,
            top: -2.0,
            bottom: 100.0
        }
    );
    assert_eq!(rects[1].right, 260.0);
}

#[test]
fn a_nested_child_keeps_its_own_padding_box_through_both_conversions() {
    let rects = overflow_rects("writing-mode:vertical-rl", |doc, container| {
        let child = doc.append_element(
            Some(container),
            "div",
            Some("display:block;width:100px;height:100px;border-style:solid;border-width:2px 3px 5px 7px"),
        );
        two_children(doc, child, "display:block;width:100px;height:100px");
        vec![child]
    });
    assert_eq!(
        rects[2],
        Edges {
            left: -100.0,
            right: 103.0,
            top: -2.0,
            bottom: 100.0
        }
    );
    assert_eq!(
        rects[0],
        Edges {
            left: -103.0,
            right: 100.0,
            top: 0.0,
            bottom: 107.0
        }
    );
    assert_eq!(rects[1].right, 250.0);
}

#[test]
fn an_empty_child_retains_zero_overflow() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        let rects = overflow_rects(&format!("writing-mode:{mode}"), |doc, container| {
            vec![doc.append_element(
                Some(container),
                "div",
                Some("display:block;width:20px;height:30px"),
            )]
        });
        assert_eq!(rects[2], Edges::ZERO, "{mode}");
    }
}

#[test]
fn container_padding_belongs_to_the_mirror_extent() {
    let rects = overflow_rects(
        "writing-mode:vertical-rl;border-style:solid;border-width:2px 3px 5px 7px;padding:5px 11px 13px 7px",
        |doc, container| two_children(doc, container, "display:block;width:100px;height:100px"),
    );
    assert_eq!(
        rects[0],
        Edges {
            left: -93.0,
            right: 121.0,
            top: -2.0,
            bottom: 105.0
        }
    );
    assert_eq!(rects[1].right, 278.0);
}

#[test]
fn flex_and_grid_children_are_not_mirrored_twice() {
    for css in [
        "display:flex;flex-direction:column;width:100px;height:100px",
        "display:grid;grid-template-rows:100px 100px;width:100px;height:100px",
    ] {
        let rects = overflow_rects("writing-mode:vertical-rl", |doc, container| {
            let child = doc.append_element(Some(container), "div", Some(css));
            two_children(
                doc,
                child,
                "display:block;width:100px;height:100px;flex-shrink:0",
            );
            vec![child]
        });
        let expected = Edges {
            left: -100.0,
            right: 100.0,
            top: 0.0,
            bottom: 100.0,
        };
        assert_eq!(rects[0], expected, "{css}");
        assert_eq!(rects[2], expected, "{css}");
        assert_eq!(rects[1].right, 250.0, "{css}");
    }
}

#[test]
fn an_opposite_vertical_child_keeps_the_physical_side_of_its_overflow() {
    let rects = overflow_rects("writing-mode:vertical-rl", |doc, container| {
        let child = doc.append_element(
            Some(container),
            "div",
            Some("display:block;writing-mode:vertical-lr;width:100px;height:100px"),
        );
        two_children(doc, child, "display:block;width:100px;height:100px");
        vec![child]
    });
    let expected = Edges {
        left: 0.0,
        right: 200.0,
        top: 0.0,
        bottom: 100.0,
    };
    assert_eq!(rects[2], expected);
    assert_eq!(rects[0], expected);
    assert_eq!(rects[1].right, 350.0);
}

#[test]
fn block_children_encode_overflow_with_their_own_inline_direction() {
    for (mode, naturally_reversed) in [
        ("vertical-rl", false),
        ("vertical-lr", false),
        ("sideways-rl", false),
        ("sideways-lr", true),
    ] {
        for parent_rtl in [false, true] {
            let parent_direction = if parent_rtl { "rtl" } else { "ltr" };
            let child_rtl = !parent_rtl;
            let child_direction = if child_rtl { "rtl" } else { "ltr" };
            let rects = overflow_rects(
                &format!("writing-mode:{mode};direction:{parent_direction}"),
                |doc, container| {
                    let child = doc.append_element(
                        Some(container),
                        "div",
                        Some(&format!(
                            "display:block;width:100px;height:100px;direction:{child_direction}"
                        )),
                    );
                    doc.append_element(
                        Some(child),
                        "div",
                        Some("display:block;width:100px;height:150px"),
                    );
                    vec![child]
                },
            );
            let reversed = naturally_reversed != child_rtl;
            let (top, bottom) = if reversed {
                (-50.0, 100.0)
            } else {
                (0.0, 150.0)
            };
            let expected = Edges {
                left: 0.0,
                right: 100.0,
                top,
                bottom,
            };
            assert_eq!(
                rects[2], expected,
                "child {mode}/{parent_direction}/{child_direction}"
            );
            assert_eq!(
                rects[0], expected,
                "parent {mode}/{parent_direction}/{child_direction}"
            );
            assert_eq!(
                (rects[1].top, rects[1].bottom),
                (top, bottom),
                "ancestor {mode}/{parent_direction}/{child_direction}"
            );
        }
    }
}

#[test]
fn flex_and_grid_keep_their_parent_direction_overflow_contract() {
    for (mode, naturally_reversed) in [
        ("vertical-rl", false),
        ("vertical-lr", false),
        ("sideways-rl", false),
        ("sideways-lr", true),
    ] {
        for parent_rtl in [false, true] {
            for display in [
                "display:flex;flex-direction:column",
                "display:grid;grid-template-rows:100px",
            ] {
                let parent_direction = if parent_rtl { "rtl" } else { "ltr" };
                let child_rtl = !parent_rtl;
                let child_direction = if child_rtl { "rtl" } else { "ltr" };
                let rects = overflow_rects(
                    &format!("{display};writing-mode:{mode};direction:{parent_direction}"),
                    |doc, container| {
                        let child = doc.append_element(Some(container), "div", Some(&format!(
                            "display:block;width:100px;height:100px;flex-shrink:0;direction:{child_direction}"
                        )));
                        doc.append_element(
                            Some(child),
                            "div",
                            Some("display:block;width:100px;height:150px"),
                        );
                        vec![child]
                    },
                );
                let reversed = naturally_reversed != child_rtl;
                let (top, bottom) = if reversed {
                    (-50.0, 100.0)
                } else {
                    (0.0, 150.0)
                };
                let expected = Edges {
                    left: 0.0,
                    right: 100.0,
                    top,
                    bottom,
                };
                assert_eq!(
                    rects[2], expected,
                    "child {display}/{mode}/{parent_direction}/{child_direction}"
                );
                assert_eq!(
                    rects[0], expected,
                    "parent {display}/{mode}/{parent_direction}/{child_direction}"
                );
                assert_eq!(
                    (rects[1].top, rects[1].bottom),
                    (top, bottom),
                    "ancestor {display}/{mode}/{parent_direction}/{child_direction}"
                );
            }
        }
    }
}

#[test]
fn horizontal_rtl_block_child_overflow_is_decoded_before_transposing() {
    let rects = overflow_rects("writing-mode:vertical-rl", |doc, container| {
        let child = doc.append_element(
            Some(container),
            "div",
            Some("display:block;writing-mode:horizontal-tb;direction:rtl;width:100px;height:100px"),
        );
        doc.append_element(
            Some(child),
            "div",
            Some("display:block;width:150px;height:100px"),
        );
        vec![child]
    });
    assert_eq!((rects[2].left, rects[2].right), (-50.0, 100.0));
    assert_eq!((rects[0].left, rects[0].right), (-50.0, 100.0));
    assert_eq!(rects[1].right, 250.0);
}

#[test]
fn horizontal_block_parent_keeps_vertical_rtl_child_overflow_physical() {
    let rects = overflow_rects(
        "writing-mode:vertical-rl;direction:rtl",
        |doc, container| two_children(doc, container, "display:block;width:100px;height:100px"),
    );
    assert_eq!((rects[0].left, rects[0].right), (-100.0, 100.0));
    assert_eq!(rects[1].right, 250.0);
}

#[test]
fn horizontal_rtl_producers_share_the_block_boundary_decode() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for kind in ["block", "flex", "grid"] {
            let rects = overflow_rects(&format!("writing-mode:{mode}"), |doc, container| {
                let display = kind;
                let child = doc.append_element(Some(container), "div", Some(&format!(
                    "display:{display};writing-mode:horizontal-tb;direction:rtl;width:100px;height:100px;grid-template-columns:150px;font-family:Ahem;font-size:10px;line-height:100px;white-space:nowrap"
                )));
                doc.append_element(
                    Some(child),
                    "div",
                    Some("display:block;width:150px;height:100px;flex-shrink:0"),
                );
                vec![child]
            });
            assert_eq!(
                (rects[2].left, rects[2].right),
                (-50.0, 100.0),
                "{mode}/{kind}/child"
            );
            assert_eq!(
                (rects[0].left, rects[0].right),
                (-50.0, 100.0),
                "{mode}/{kind}/container"
            );
            assert_eq!(rects[1].right, 250.0, "{mode}/{kind}/ancestor");
        }
    }
}

#[test]
fn native_horizontal_parents_keep_a_vertical_childs_stored_overflow_physical() {
    for display in ["flex", "grid"] {
        for direction in ["ltr", "rtl"] {
            let rects = overflow_rects(
                &format!("display:{display};direction:{direction};grid-template-columns:100px"),
                |doc, parent| {
                    let child = doc.append_element(Some(parent), "div", Some(
                        "display:block;writing-mode:vertical-rl;direction:rtl;width:100px;height:100px;flex-shrink:0"
                    ));
                    two_children(doc, child, "display:block;width:100px;height:100px");
                    vec![child]
                },
            );
            assert_eq!(
                (rects[2].left, rects[2].right),
                (-100.0, 100.0),
                "{display}/{direction}/child"
            );
            assert_eq!(rects[1].right, 250.0, "{display}/{direction}/ancestor");
        }
    }
}

#[test]
fn rtl_initial_containing_block_keeps_positioned_vertical_overflow_physical() {
    for position in ["absolute", "fixed"] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("direction:rtl"));
        let body = doc.append_element(Some(html), "body", Some("margin:0"));
        let child = doc.append_element(Some(body), "div", Some(&format!(
            "display:block;position:{position};left:150px;top:0;width:100px;height:100px;writing-mode:vertical-rl;direction:rtl"
        )));
        two_children(&mut doc, child, "display:block;width:100px;height:100px");
        doc.mark_in_document_flags();
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        with_ahem(&mut doc);
        for _ in 0..2 {
            crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
            let rect = doc.nodes[child].layout.scrollable_overflow_rect;
            assert_eq!((rect.left, rect.right), (-100.0, 100.0), "{position}");
        }
    }
}
