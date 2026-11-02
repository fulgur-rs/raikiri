use super::*;
use crate::layout::test_support::{absolute_rect, with_ahem};

struct Fixture {
    doc: Document,
    table: usize,
    row: usize,
    paragraph: usize,
    horizontal: usize,
    orthogonal: usize,
}

fn fixture(mode: &str, reversed: bool) -> Fixture {
    configured_fixture(mode, reversed, |_| {})
}

fn configured_fixture(mode: &str, reversed: bool, configure: impl FnOnce(&mut Fixture)) -> Fixture {
    let mut doc = Document::new();
    with_ahem(&mut doc);
    let html = doc.append_element(
        Some(0),
        "html",
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let table = doc.append_element(Some(body), "div", Some("display:table;border-spacing:0"));
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let mut horizontal = None;
    let mut orthogonal = None;
    let mut paragraph = None;
    for vertical in [reversed, !reversed] {
        if vertical {
            let cell = doc.append_element(Some(row), "div", Some(&format!("display:table-cell;vertical-align:baseline;writing-mode:{mode};width:60px;height:40px")));
            doc.append_element(
                Some(cell),
                "div",
                Some("display:block;width:30px;height:20px"),
            );
            orthogonal = Some(cell);
        } else {
            let cell = doc.append_element(
                Some(row),
                "div",
                Some("display:table-cell;vertical-align:baseline;font-size:20px;line-height:20px"),
            );
            let p = doc.append_element(Some(cell), "div", Some("display:block"));
            doc.append_text(p, "xx");
            horizontal = Some(cell);
            paragraph = Some(p);
        }
    }
    let mut f = Fixture {
        doc,
        table,
        row,
        paragraph: paragraph.unwrap(),
        horizontal: horizontal.unwrap(),
        orthogonal: orthogonal.unwrap(),
    };
    configure(&mut f);
    f.doc.mark_in_document_flags();
    let computed = cascade(&f.doc, &build_rule_tree(&f.doc)).unwrap();
    crate::layout::layout_single_page(&mut f.doc, &computed, PageBox::A4).unwrap();
    f
}

fn spanning_fixture(mode: &str, reversed: bool) -> Fixture {
    configured_fixture(mode, reversed, |f| {
        f.doc
            .set_element_attribute(
                f.horizontal,
                "style",
                "display:table-cell;vertical-align:baseline;font-size:10px;line-height:10px",
            )
            .unwrap();
        f.doc
            .set_element_attribute(f.orthogonal, "rowspan", "2")
            .unwrap();
        let second =
            f.doc
                .append_element(Some(f.table), "div", Some("display:table-row;height:30px"));
        f.doc.append_element(
            Some(second),
            "div",
            Some("display:table-cell;height:30px;vertical-align:top"),
        );
    })
}

#[test]
fn orthogonal_cells_do_not_raise_the_horizontal_row_baseline() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for reversed in [false, true] {
            let f = fixture(mode, reversed);
            let (_, y, _, _) = absolute_rect(&f.doc, f.paragraph);
            assert_eq!(y, 8.0, "{mode}, reversed={reversed}");
            assert_eq!(
                f.doc.nodes[f.table].table_first_baseline,
                Some(16.0),
                "{mode}, reversed={reversed}"
            );
            assert_eq!(f.doc.nodes[f.horizontal].layout.size.height, 40.0);
        }
    }
}

#[test]
fn parallel_box_only_cells_keep_their_bottom_content_baseline() {
    for reversed in [false, true] {
        let f = fixture("horizontal-tb", reversed);
        let (_, y, _, _) = absolute_rect(&f.doc, f.paragraph);
        assert_eq!(y, 12.0);
        assert_eq!(f.doc.nodes[f.table].table_first_baseline, Some(20.0));
        assert_eq!(f.doc.nodes[f.horizontal].layout.size.height, 40.0);
        assert_eq!(f.doc.nodes[f.orthogonal].layout.size.height, 40.0);
    }
}

#[test]
fn an_orthogonal_rowspan_does_not_floor_its_first_row_at_an_unrelated_baseline() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for reversed in [false, true] {
            let f = spanning_fixture(mode, reversed);
            assert_eq!(
                absolute_rect(&f.doc, f.paragraph).1,
                8.0,
                "{mode}, {reversed}"
            );
            assert_eq!(
                f.doc.nodes[f.horizontal].layout.size.height, 10.0,
                "{mode}, {reversed}"
            );
            assert_eq!(
                f.doc.nodes[f.table].layout.size.height, 40.0,
                "{mode}, {reversed}"
            );
            assert_eq!(f.doc.nodes[f.table].table_first_baseline, Some(8.0));
        }
    }
}

#[test]
fn a_parallel_rowspan_still_floors_and_aligns_the_first_row() {
    for reversed in [false, true] {
        let f = spanning_fixture("horizontal-tb", reversed);
        assert_eq!(absolute_rect(&f.doc, f.paragraph).1, 20.0);
        assert_eq!(f.doc.nodes[f.horizontal].layout.size.height, 22.0);
        assert_eq!(f.doc.nodes[f.table].layout.size.height, 52.0);
        assert_eq!(f.doc.nodes[f.table].table_first_baseline, Some(20.0));
    }
}

#[test]
fn a_row_of_only_orthogonal_cells_keeps_the_content_edge_baseline_fallback() {
    let f = configured_fixture("vertical-rl", false, |f| {
        f.doc.set_element_attribute(f.horizontal, "style", "display:table-cell;writing-mode:vertical-rl;width:30px;height:20px;vertical-align:baseline;font-size:20px;line-height:20px").unwrap();
    });
    assert_eq!(f.doc.nodes[f.horizontal].layout.size.height, 40.0);
    assert_eq!(f.doc.nodes[f.table].table_first_baseline, Some(40.0));
}

#[test]
fn a_rows_writing_mode_does_not_remove_its_anonymous_cells_content_baseline() {
    let mut f = configured_fixture("vertical-rl", false, |f| {
        f.doc
            .set_element_attribute(
                f.row,
                "style",
                "display:table-row;writing-mode:vertical-rl;font-size:40px;line-height:40px",
            )
            .unwrap();
        f.doc.set_element_attribute(f.horizontal, "style", "display:table-cell;writing-mode:horizontal-tb;vertical-align:baseline;font-size:20px;line-height:20px").unwrap();
        f.doc
            .set_element_attribute(
                f.orthogonal,
                "style",
                "display:block;writing-mode:vertical-rl;width:60px;height:40px",
            )
            .unwrap();
    });
    assert_eq!(absolute_rect(&f.doc, f.paragraph).1, 32.0);
    assert_eq!(f.doc.nodes[f.table].table_first_baseline, Some(40.0));
    assert_eq!(f.doc.nodes[f.table].layout.size.height, 44.0);
    let grid = build_table_grid_in_document(&mut f.doc, f.table).unwrap();
    assert!(
        grid.cells
            .iter()
            .any(|cell| f.doc.ifc_source_owner(cell.node_id) == f.row)
    );
}

#[test]
fn native_vertical_tables_keep_their_physical_cell_positions() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let f = configured_fixture(mode, false, |f| {
            f.doc
                .set_element_attribute(
                    f.table,
                    "style",
                    format!("display:table;writing-mode:{mode};border-spacing:0"),
                )
                .unwrap();
            f.doc.set_element_attribute(f.horizontal, "style", "display:table-cell;writing-mode:horizontal-tb;vertical-align:baseline;font-size:20px;line-height:20px").unwrap();
        });
        assert_eq!(
            f.doc.nodes[f.table].layout.size,
            Size {
                width: 60.0,
                height: 60.0
            },
            "{mode}"
        );
        assert_eq!(
            absolute_rect(&f.doc, f.paragraph),
            (8.0, 8.0, 60.0, 20.0),
            "{mode}"
        );
        assert_eq!(
            absolute_rect(&f.doc, f.orthogonal),
            (8.0, 28.0, 60.0, 40.0),
            "{mode}"
        );
    }
}

#[test]
fn nonbaseline_alignments_do_not_join_the_row_group() {
    for align in ["top", "middle", "bottom"] {
        let f = configured_fixture("horizontal-tb", false, |f| {
            f.doc
                .set_element_attribute(
                    f.orthogonal,
                    "style",
                    format!("display:table-cell;vertical-align:{align};width:60px;height:40px"),
                )
                .unwrap();
        });
        assert_eq!(absolute_rect(&f.doc, f.paragraph).1, 8.0, "{align}");
        assert_eq!(f.doc.nodes[f.table].table_first_baseline, Some(16.0));
    }
}
