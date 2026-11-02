use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

fn fixture(mode: &str, cell_css: &str, absolute_css: &str) -> (Document, usize) {
    let (doc, absolute, _) = fixture_with_candidates(mode, cell_css, absolute_css, false, "");
    (doc, absolute)
}

fn fixture_with_candidates(
    mode: &str,
    cell_css: &str,
    absolute_css: &str,
    multiple: bool,
    flow_css: &str,
) -> (Document, usize, Option<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:8px;font:10px/10px Ahem"),
    );
    let table = doc.append_element(
        Some(body),
        "div",
        Some("display:table;position:relative;writing-mode:vertical-rl"),
    );
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let first = doc.append_element(
        Some(row),
        "div",
        Some(if multiple {
            "display:table-cell;width:60px;height:50px;vertical-align:top"
        } else {
            "display:table-cell;width:60px;height:50px"
        }),
    );
    let paragraph = doc.append_element(
        Some(first),
        "div",
        Some(if multiple {
            "display:block;width:10px;height:10px"
        } else {
            "display:block"
        }),
    );
    doc.append_text(paragraph, "x");
    let first_absolute = multiple.then(|| {
        doc.append_element(
            Some(first),
            "div",
            Some("display:block;position:absolute;width:5px;height:5px"),
        )
    });
    let second = doc.append_element(
        Some(row),
        "div",
        Some(&format!(
            "display:table-cell;width:60px;height:30px;writing-mode:{mode};{cell_css}"
        )),
    );
    doc.append_element(
        Some(second),
        "div",
        Some(&format!("display:block;width:20px;height:10px;{flow_css}")),
    );
    let holder = if multiple {
        doc.append_element(Some(second), "div", Some("display:block;width:0;height:0"))
    } else {
        second
    };
    let absolute = doc.append_element(
        Some(holder),
        "div",
        Some(&format!(
            "display:block;position:absolute;width:5px;height:5px;{absolute_css}"
        )),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
    (doc, absolute, first_absolute)
}

#[test]
fn horizontal_cell_static_position_uses_its_final_vertical_table_position() {
    let (doc, absolute) = fixture("horizontal-tb", "", "");
    assert_eq!(absolute_rect(&doc, absolute), (8.0, 68.0, 5.0, 5.0));
}

#[test]
fn vertical_cell_static_position_comes_from_its_final_layout() {
    let (doc, absolute) = fixture("vertical-rl", "", "");
    assert_eq!(absolute_rect(&doc, absolute), (43.0, 58.0, 5.0, 5.0));
}

#[test]
fn explicit_table_insets_ignore_the_cell_static_position() {
    for mode in ["horizontal-tb", "vertical-rl"] {
        let (doc, absolute) = fixture(mode, "", "left:7px;top:9px");
        assert_eq!(absolute_rect(&doc, absolute), (15.0, 17.0, 5.0, 5.0));
    }
}

#[test]
fn a_positioned_cell_keeps_its_own_containing_block() {
    for mode in ["horizontal-tb", "vertical-rl"] {
        let (doc, absolute) = fixture(mode, "position:relative", "left:7px;top:9px");
        assert_eq!(absolute_rect(&doc, absolute), (15.0, 67.0, 5.0, 5.0));
    }
}

#[test]
fn candidates_in_multiple_cells_and_nested_blocks_use_final_positions() {
    let (doc, nested, first) = fixture_with_candidates("vertical-rl", "", "", true, "");
    assert_eq!(absolute_rect(&doc, first.unwrap()), (53.0, 8.0, 5.0, 5.0));
    assert_eq!(absolute_rect(&doc, nested), (43.0, 58.0, 5.0, 5.0));
}

#[test]
fn final_cell_axes_are_scoped_and_restored_after_unwinding() {
    let (mut doc, absolute) = fixture("vertical-rl", "", "");
    let cell = doc.nodes[absolute].parent.unwrap();
    super::with_layout_session(&mut doc, |session| {
        use crate::layout::dispatch::{block_right_to_left, with_final_vertical_table_cell};
        assert_eq!(block_right_to_left(session, cell), None);
        with_final_vertical_table_cell(session, cell, |session| {
            assert_eq!(block_right_to_left(session, cell), Some(true));
            with_final_vertical_table_cell(session, cell, |_| ());
            assert_eq!(block_right_to_left(session, cell), Some(true));
        });
        assert_eq!(block_right_to_left(session, cell), None);
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_final_vertical_table_cell(session, cell, |_| panic!("layout failed"));
        }));
        assert!(failure.is_err());
        assert_eq!(block_right_to_left(session, cell), None);
    });
}

#[test]
fn an_invalid_vertical_track_keeps_pending_candidates() {
    use crate::layout::protocol::{
        AxisStaticEdge, AxisStaticPosition, NodeId, OofCandidate, OofCandidates, OofPosition,
    };
    use raikiri_layout_geometry::Point;
    let (mut doc, absolute) = fixture("vertical-rl", "", "");
    let candidate = OofCandidate {
        node: NodeId::from(absolute),
        order: 0,
        position: OofPosition::Absolute,
        static_position: Point {
            x: AxisStaticPosition::from_edge(3.0, AxisStaticEdge::Start),
            y: AxisStaticPosition::from_edge(7.0, AxisStaticEdge::Start),
        },
    };
    super::with_layout_session(&mut doc, |session| {
        for height in [f32::INFINITY, f32::NAN] {
            let mut pending = OofCandidates::new();
            pending.push(candidate);
            let mut grid = super::super::TableGrid {
                n_cols: 0,
                rows: vec![],
                cells: vec![],
                col_widths: vec![],
            };
            let result = super::super::reposition_cells_for_vertical_writing(
                session,
                &mut grid,
                &[height],
                10.0,
                Point::ZERO,
                0.0,
                vec![(absolute, pending)],
            );
            assert_eq!(result.to_vec(), vec![candidate]);
        }
    });
}

#[test]
fn vertical_left_to_right_cell_emits_physical_static_positions() {
    let (doc, absolute) = fixture("vertical-lr", "vertical-align:top", "");
    assert_eq!(absolute_rect(&doc, absolute), (28.0, 58.0, 5.0, 5.0));
}

#[test]
fn vertical_cell_self_alignment_survives_hoisting_to_the_table() {
    let (doc, absolute) = fixture(
        "vertical-rl",
        "height:50px;vertical-align:top",
        "justify-self:end;align-self:start",
    );
    assert_eq!(absolute_rect(&doc, absolute), (43.0, 103.0, 5.0, 5.0));
}

#[test]
fn a_positioned_cell_aligns_its_auto_inset_child_with_its_flow() {
    let (doc, absolute) = fixture("vertical-rl", "position:relative;vertical-align:middle", "");
    assert_eq!(absolute_rect(&doc, absolute), (23.0, 58.0, 5.0, 5.0));
}

#[test]
fn a_positioned_cell_aligns_its_auto_inset_child_at_the_block_end() {
    let (doc, absolute) = fixture("vertical-rl", "position:relative;vertical-align:bottom", "");
    assert_eq!(absolute_rect(&doc, absolute), (3.0, 58.0, 5.0, 5.0));
}

#[test]
fn a_positioned_cell_keeps_self_alignment_after_flow_alignment() {
    let (doc, absolute) = fixture(
        "vertical-rl",
        "position:relative;vertical-align:middle;height:50px",
        "justify-self:end;align-self:start",
    );
    assert_eq!(absolute_rect(&doc, absolute), (23.0, 103.0, 5.0, 5.0));
}

#[test]
fn explicit_insets_with_auto_margins_keep_the_cell_positioning_area() {
    let (doc, absolute) = fixture(
        "vertical-rl",
        "position:relative;vertical-align:middle;height:50px",
        "left:0;right:0;top:0;bottom:0;margin:auto",
    );
    assert_eq!(absolute_rect(&doc, absolute), (35.5, 80.5, 5.0, 5.0));
}

#[test]
fn a_retained_horizontal_cell_candidate_follows_the_full_alignment_shift() {
    for (align, y) in [("middle", 88.0), ("bottom", 108.0)] {
        let (doc, absolute) = fixture(
            "horizontal-tb",
            &format!("height:50px;vertical-align:{align}"),
            "",
        );
        assert_eq!(absolute_rect(&doc, absolute), (8.0, y, 5.0, 5.0));
    }
}

#[test]
fn a_relaid_horizontal_cell_uses_new_percentage_height_static_positions() {
    let (doc, absolute, _) = fixture_with_candidates(
        "horizontal-tb",
        "vertical-align:top",
        "",
        false,
        "height:50%",
    );
    assert_eq!(absolute_rect(&doc, absolute), (8.0, 73.0, 5.0, 5.0));
}
