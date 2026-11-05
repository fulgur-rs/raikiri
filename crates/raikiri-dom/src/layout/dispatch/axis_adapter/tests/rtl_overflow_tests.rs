use crate::Document;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use raikiri_layout_geometry::{BoxLayout, Edges, Size};

const MODES: [(&str, bool, bool); 4] = [
    ("vertical-rl", true, false),
    ("vertical-lr", false, false),
    ("sideways-rl", true, false),
    ("sideways-lr", false, true),
];

fn edges(left: f32, right: f32, top: f32, bottom: f32) -> Edges<f32> {
    Edges {
        left,
        right,
        top,
        bottom,
    }
}

fn layouts(
    container_css: &str,
    build: impl FnOnce(&mut Document, usize) -> Vec<usize>,
) -> Vec<BoxLayout> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let ancestor = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:400px;height:300px"),
    );
    let container = doc.append_element(
        Some(ancestor),
        "div",
        Some(&format!(
            "display:block;width:100px;height:100px;margin-left:150px;{container_css}"
        )),
    );
    let mut nodes = vec![container, ancestor];
    nodes.extend(build(&mut doc, container));
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    with_ahem(&mut doc);
    let mut first = None;
    for _ in 0..2 {
        crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
        let result: Vec<_> = nodes.iter().map(|&node| doc.nodes[node].layout).collect();
        if let Some(first) = &first {
            assert_eq!(
                &result, first,
                "repeated layout keeps the same physical geometry"
            );
        } else {
            first = Some(result);
        }
    }
    first.unwrap()
}

fn horizontal_child(doc: &mut Document, parent: usize, kind: &str) -> Vec<usize> {
    let child = doc.append_element(Some(parent), "div", Some(&format!(
        "display:{kind};writing-mode:horizontal-tb;direction:rtl;width:100px;height:100px;flex-shrink:0;grid-template-columns:150px"
    )));
    doc.append_element(
        Some(child),
        "div",
        Some("display:block;width:150px;height:100px;flex-shrink:0"),
    );
    vec![child]
}

#[test]
fn vertical_native_parents_decode_horizontal_rtl_child_overflow() {
    for (mode, _, _) in MODES {
        for direction in ["ltr", "rtl"] {
            for display in ["flex", "grid"] {
                for child_kind in ["block", "flex", "grid"] {
                    let boxes = layouts(
                        &format!(
                            "display:{display};writing-mode:{mode};direction:{direction};flex-direction:column;grid-template-rows:100px"
                        ),
                        |doc, parent| horizontal_child(doc, parent, child_kind),
                    );
                    let expected = edges(-50.0, 100.0, 0.0, 100.0);
                    assert_eq!(
                        boxes[2].scrollable_overflow_rect, expected,
                        "{mode}/{direction}/{display}/{child_kind}/child"
                    );
                    assert_eq!(
                        boxes[0].scrollable_overflow_rect, expected,
                        "{mode}/{direction}/{display}/{child_kind}/parent"
                    );
                    assert_eq!(
                        boxes[1].scrollable_overflow_rect,
                        edges(0.0, 250.0, 0.0, 100.0),
                        "{mode}/{direction}/{display}/{child_kind}/ancestor"
                    );
                }
            }
        }
    }
}

#[test]
fn horizontal_native_parents_encode_vertical_output_and_store_it_physically() {
    for (mode, block_reversed, _) in MODES {
        for direction in ["ltr", "rtl"] {
            for display in ["flex", "grid"] {
                for child_kind in ["block", "flex", "grid"] {
                    let boxes = layouts(
                        &format!(
                            "display:{display};direction:{direction};grid-template-columns:100px"
                        ),
                        |doc, parent| {
                            let child = doc.append_element(Some(parent), "div", Some(&format!(
                            "display:{child_kind};writing-mode:{mode};direction:rtl;width:100px;height:100px;flex-direction:column;flex-shrink:0;grid-template-rows:100px 100px"
                        )));
                            for _ in 0..2 {
                                doc.append_element(
                                    Some(child),
                                    "div",
                                    Some("display:block;width:100px;height:100px;flex-shrink:0"),
                                );
                            }
                            vec![child]
                        },
                    );
                    let (physical, legacy_rtl, ancestor_right) = if block_reversed {
                        (
                            edges(-100.0, 100.0, 0.0, 100.0),
                            edges(0.0, 200.0, 0.0, 100.0),
                            250.0,
                        )
                    } else {
                        (
                            edges(0.0, 200.0, 0.0, 100.0),
                            edges(-100.0, 100.0, 0.0, 100.0),
                            350.0,
                        )
                    };
                    // Horizontal native containers retain their own legacy origin;
                    // their physical vertical child must not inherit that encoding.
                    let parent = if direction == "rtl" {
                        legacy_rtl
                    } else {
                        physical
                    };
                    assert_eq!(
                        boxes[2].scrollable_overflow_rect, physical,
                        "{mode}/{direction}/{display}/{child_kind}/child"
                    );
                    assert_eq!(
                        boxes[0].scrollable_overflow_rect, parent,
                        "{mode}/{direction}/{display}/{child_kind}/parent"
                    );
                    assert_eq!(
                        boxes[1].scrollable_overflow_rect,
                        edges(0.0, ancestor_right, 0.0, 100.0),
                        "{mode}/{direction}/{display}/{child_kind}/ancestor"
                    );
                }
            }
        }
    }
}

#[test]
fn asymmetric_inline_edges_decode_a_horizontal_rtl_paragraph_before_transpose() {
    for (mode, block_reversed, naturally_reversed) in MODES {
        for rtl in [false, true] {
            for display in ["flex", "grid"] {
                let direction = if rtl { "rtl" } else { "ltr" };
                let boxes = layouts(
                    &format!(
                        "display:{display};writing-mode:{mode};direction:{direction};width:200px;height:160px;flex-direction:column;grid-template-rows:130px"
                    ),
                    |doc, parent| {
                        let child = doc.append_element(Some(parent), "div", Some(
                        "display:block;writing-mode:horizontal-tb;direction:rtl;width:100px;height:100px;flex-shrink:0;padding:3px 13px 9px 7px;border-style:solid;border-width:2px 3px 5px 7px;font:10px/100px Ahem"
                    ));
                        doc.append_text(child, "XX");
                        vec![child]
                    },
                );
                // The paragraph's measured line box fits its stated width.
                // Its 120px padding box excludes the 7px + 3px borders; RTL
                // start padding is 13px, and end padding is not overflow.
                assert_eq!(
                    boxes[2].scrollable_overflow_rect,
                    edges(7.0, 120.0, 0.0, 103.0),
                    "{mode}/{direction}/{display}/child"
                );
                assert_eq!(
                    boxes[2].size,
                    Size {
                        width: 130.0,
                        height: 119.0
                    }
                );
                // Container overflow also retains the physical zero origin.
                let right = if block_reversed { 200.0 } else { 130.0 };
                let bottom = if naturally_reversed != rtl {
                    160.0
                } else {
                    119.0
                };
                assert_eq!(
                    boxes[0].scrollable_overflow_rect,
                    edges(0.0, right, 0.0, bottom),
                    "{mode}/{direction}/{display}/parent"
                );
                assert_eq!(
                    boxes[1].scrollable_overflow_rect,
                    edges(0.0, 350.0, 0.0, 160.0),
                    "{mode}/{direction}/{display}/ancestor"
                );
            }
        }
    }
}

#[test]
fn a_bordered_padded_vertical_parent_reflects_inside_its_padding_box() {
    for (mode, block_reversed, naturally_reversed) in MODES {
        for rtl in [false, true] {
            for display in ["flex", "grid"] {
                let direction = if rtl { "rtl" } else { "ltr" };
                let boxes = layouts(
                    &format!(
                        "display:{display};writing-mode:{mode};direction:{direction};flex-direction:column;grid-template-rows:100px;padding:3px 13px 9px 7px;border-style:solid;border-width:2px 3px 5px 7px"
                    ),
                    |doc, parent| horizontal_child(doc, parent, "block"),
                );
                assert_eq!(
                    boxes[0].size,
                    Size {
                        width: 130.0,
                        height: 119.0
                    }
                );
                assert_eq!(
                    boxes[2].scrollable_overflow_rect,
                    edges(-50.0, 100.0, 0.0, 100.0),
                    "{mode}/{direction}/{display}/child"
                );
                // The child starts 7px inside the padding box and reaches
                // 50px past its own left edge: 7 - 50 = -43. The parent's
                // reflection extent is 120px, excluding both borders.
                let right = if block_reversed { 120.0 } else { 107.0 };
                let bottom = if naturally_reversed != rtl {
                    112.0
                } else {
                    103.0
                };
                assert_eq!(
                    boxes[0].scrollable_overflow_rect,
                    edges(-43.0, right, 0.0, bottom),
                    "{mode}/{direction}/{display}/parent"
                );
                assert_eq!(
                    boxes[1].scrollable_overflow_rect,
                    edges(0.0, 280.0, 0.0, 119.0),
                    "{mode}/{direction}/{display}/ancestor"
                );
            }
        }
    }
}

#[test]
fn nested_native_boundaries_restore_each_containers_origin() {
    for (mode, block_reversed, _) in MODES {
        for direction in ["ltr", "rtl"] {
            for outer_display in ["flex", "grid"] {
                for middle_display in ["flex", "grid"] {
                    let (inner_mode, expected, ancestor_right) = if block_reversed {
                        ("vertical-lr", edges(0.0, 200.0, 0.0, 100.0), 350.0)
                    } else {
                        ("vertical-rl", edges(-100.0, 100.0, 0.0, 100.0), 250.0)
                    };
                    let inner_direction = if direction == "ltr" { "rtl" } else { "ltr" };
                    let boxes = layouts(
                        &format!(
                            "display:{outer_display};writing-mode:{mode};direction:{direction};flex-direction:column;grid-template-rows:100px"
                        ),
                        |doc, outer| {
                            let middle = doc.append_element(Some(outer), "div", Some(&format!(
                            "display:{middle_display};writing-mode:horizontal-tb;direction:rtl;width:100px;height:100px;flex-shrink:0;grid-template-columns:100px"
                        )));
                            let inner = doc.append_element(Some(middle), "div", Some(&format!(
                            "display:block;writing-mode:{inner_mode};direction:{inner_direction};width:100px;height:100px;flex-shrink:0"
                        )));
                            for _ in 0..2 {
                                doc.append_element(
                                    Some(inner),
                                    "div",
                                    Some("display:block;width:100px;height:100px"),
                                );
                            }
                            vec![middle, inner]
                        },
                    );
                    for &index in &[0, 2, 3] {
                        assert_eq!(
                            boxes[index].scrollable_overflow_rect, expected,
                            "{mode}/{direction}/{outer_display}/{middle_display}/box{index}"
                        );
                    }
                    assert_eq!(
                        boxes[1].scrollable_overflow_rect,
                        edges(0.0, ancestor_right, 0.0, 100.0),
                        "{mode}/{direction}/{outer_display}/{middle_display}/ancestor"
                    );
                }
            }
        }
    }
}

#[test]
fn empty_child_rectangles_stay_zero_through_both_native_boundaries() {
    for (mode, _, _) in MODES {
        for direction in ["ltr", "rtl"] {
            for display in ["flex", "grid"] {
                for child_kind in ["block", "flex", "grid"] {
                    let vertical_parent = layouts(
                        &format!(
                            "display:{display};writing-mode:{mode};direction:{direction};flex-direction:column;grid-template-rows:100px"
                        ),
                        |doc, parent| {
                            vec![doc.append_element(Some(parent), "div", Some(&format!(
                            "display:{child_kind};writing-mode:horizontal-tb;direction:rtl;width:100px;height:100px;flex-shrink:0"
                        ))) ]
                        },
                    );
                    let horizontal_parent = layouts(
                        &format!(
                            "display:{display};direction:{direction};grid-template-columns:100px"
                        ),
                        |doc, parent| {
                            vec![doc.append_element(Some(parent), "div", Some(&format!(
                            "display:{child_kind};writing-mode:{mode};direction:rtl;width:100px;height:100px;flex-shrink:0"
                        ))) ]
                        },
                    );
                    for (boundary, boxes) in [
                        ("vertical parent", vertical_parent),
                        ("horizontal parent", horizontal_parent),
                    ] {
                        assert_eq!(
                            boxes[2].scrollable_overflow_rect,
                            Edges::ZERO,
                            "{boundary}/{mode}/{direction}/{display}/{child_kind}/child"
                        );
                        assert_eq!(
                            boxes[0].scrollable_overflow_rect,
                            edges(0.0, 100.0, 0.0, 100.0),
                            "{boundary}/{mode}/{direction}/{display}/{child_kind}/parent"
                        );
                        assert_eq!(
                            boxes[1].scrollable_overflow_rect,
                            edges(0.0, 250.0, 0.0, 100.0),
                            "{boundary}/{mode}/{direction}/{display}/{child_kind}/ancestor"
                        );
                    }
                }
            }
        }
    }
}

fn positioned_paragraph(
    doc: &mut Document,
    parent: usize,
    paragraph_css: &str,
    positioned_css: &str,
) -> Vec<usize> {
    let child = doc.append_element(Some(parent), "div", Some(&format!(
        "display:block;position:relative;width:100px;height:100px;flex-shrink:0;font:10px/10px Ahem;{paragraph_css}"
    )));
    doc.append_text(child, "X");
    let positioned = doc.append_element(
        Some(child),
        "div",
        Some(&format!(
            "display:block;position:absolute;width:150px;height:100px;top:0;{positioned_css}"
        )),
    );
    vec![child, positioned]
}

#[test]
fn vertical_paragraphs_preserve_negative_positioned_overflow_in_native_parents() {
    let mut failures = Vec::new();
    for (mode, _, _) in MODES {
        for parent_direction in ["ltr", "rtl"] {
            for child_direction in ["ltr", "rtl"] {
                for display in ["flex", "grid"] {
                    let boxes = layouts(
                        &format!(
                            "display:{display};direction:{parent_direction};grid-template-columns:100px"
                        ),
                        |doc, parent| {
                            positioned_paragraph(
                                doc,
                                parent,
                                &format!("writing-mode:{mode};direction:{child_direction}"),
                                "left:-50px",
                            )
                        },
                    );
                    let child = edges(-50.0, 100.0, 0.0, 100.0);
                    let parent = if parent_direction == "rtl" {
                        edges(0.0, 150.0, 0.0, 100.0)
                    } else {
                        child
                    };
                    let actual = [
                        boxes[2].scrollable_overflow_rect,
                        boxes[0].scrollable_overflow_rect,
                        boxes[1].scrollable_overflow_rect,
                    ];
                    let expected = [child, parent, edges(0.0, 250.0, 0.0, 100.0)];
                    if actual != expected {
                        failures.push(format!("{mode}/{parent_direction}/{child_direction}/{display}: {actual:?} != {expected:?}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn aligned_and_auto_inset_paragraph_overflow_uses_the_final_physical_position() {
    for (mode, block_reversed, _) in MODES {
        for parent_direction in ["ltr", "rtl"] {
            for child_direction in ["ltr", "rtl"] {
                for display in ["flex", "grid"] {
                    for both_insets in [false, true] {
                        let inset = if both_insets {
                            "left:0;right:0;align-self:end"
                        } else {
                            "left:0;align-self:end"
                        };
                        let boxes = layouts(
                            &format!(
                                "display:{display};direction:{parent_direction};grid-template-columns:100px"
                            ),
                            |doc, parent| {
                                positioned_paragraph(
                                    doc,
                                    parent,
                                    &format!("writing-mode:{mode};direction:{child_direction}"),
                                    inset,
                                )
                            },
                        );
                        let moved_left = both_insets && block_reversed;
                        let (x, child, parent_rtl, ancestor_right) = if moved_left {
                            (
                                -50.0,
                                edges(-50.0, 100.0, 0.0, 100.0),
                                edges(0.0, 150.0, 0.0, 100.0),
                                250.0,
                            )
                        } else {
                            (
                                0.0,
                                edges(0.0, 150.0, 0.0, 100.0),
                                edges(-50.0, 100.0, 0.0, 100.0),
                                300.0,
                            )
                        };
                        let parent = if parent_direction == "rtl" {
                            parent_rtl
                        } else {
                            child
                        };
                        assert_eq!(
                            boxes[3].location.x, x,
                            "{mode}/{parent_direction}/{child_direction}/{display}/{inset}/positioned"
                        );
                        assert_eq!(
                            boxes[2].scrollable_overflow_rect, child,
                            "{mode}/{parent_direction}/{child_direction}/{display}/{inset}/paragraph"
                        );
                        assert_eq!(
                            boxes[0].scrollable_overflow_rect, parent,
                            "{mode}/{parent_direction}/{child_direction}/{display}/{inset}/parent"
                        );
                        assert_eq!(
                            boxes[1].scrollable_overflow_rect,
                            edges(0.0, ancestor_right, 0.0, 100.0),
                            "{mode}/{parent_direction}/{child_direction}/{display}/{inset}/ancestor"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn vertical_paragraph_leaf_overflow_uses_its_padding_extent_without_changing_horizontal_ifcs() {
    for (mode, naturally_reversed) in [
        ("horizontal-tb", false),
        ("vertical-rl", false),
        ("vertical-lr", false),
        ("sideways-rl", false),
        ("sideways-lr", true),
    ] {
        for parent_direction in ["ltr", "rtl"] {
            for child_rtl in [false, true] {
                for display in ["flex", "grid"] {
                    let child_direction = if child_rtl { "rtl" } else { "ltr" };
                    let boxes = layouts(
                        &format!(
                            "display:{display};direction:{parent_direction};grid-template-columns:100px"
                        ),
                        |doc, parent| {
                            let child = doc.append_element(Some(parent), "div", Some(&format!(
                            "display:block;writing-mode:{mode};direction:{child_direction};width:100px;height:100px;flex-shrink:0;padding:3px 13px 9px 7px;border-style:solid;border-width:2px 3px 5px 7px;font:10px/100px Ahem"
                        )));
                            doc.append_text(child, "X");
                            vec![child]
                        },
                    );
                    let expected = if mode == "horizontal-tb" {
                        // Horizontal producers retain the existing legacy
                        // origin; only vertical paragraph output is normalized.
                        edges(0.0, if child_rtl { 113.0 } else { 107.0 }, 0.0, 103.0)
                    } else if naturally_reversed != child_rtl {
                        edges(7.0, 120.0, 0.0, 103.0)
                    } else {
                        edges(0.0, 107.0, 0.0, 103.0)
                    };
                    assert_eq!(
                        boxes[2].size,
                        Size {
                            width: 130.0,
                            height: 119.0
                        }
                    );
                    assert_eq!(
                        boxes[2].scrollable_overflow_rect, expected,
                        "{mode}/{parent_direction}/{child_direction}/{display}/paragraph"
                    );
                    assert_eq!(
                        boxes[0].scrollable_overflow_rect,
                        edges(0.0, 130.0, 0.0, 119.0),
                        "{mode}/{parent_direction}/{child_direction}/{display}/parent"
                    );
                    let right = if parent_direction == "rtl" {
                        250.0
                    } else {
                        280.0
                    };
                    assert_eq!(
                        boxes[1].scrollable_overflow_rect,
                        edges(0.0, right, 0.0, 119.0),
                        "{mode}/{parent_direction}/{child_direction}/{display}/ancestor"
                    );
                }
            }
        }
    }
}
