//! Rows and row groups as the containing block of absolutely positioned and
//! fixed boxes. A table lays its cells out itself, so it hands the boxes a row
//! or row group contains over to that part.

use crate::document::Document;
use crate::layout::page_pipeline::{
    PageLayoutControl, layout_pages_with_page_geometry_and_control,
};
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

const FIXED_CSS: &str = "position:fixed;left:10px;top:15px;width:30px;height:10px";
const ABSOLUTE_CSS: &str = "position:absolute;left:10px;top:15px;width:30px;height:10px";
const PARTS: [&str; 4] = ["tr", "tbody", "thead", "tfoot"];

fn element(doc: &mut Document, parent: usize, tag: &str, css: &str) -> usize {
    doc.append_element(Some(parent), tag, Style::default(), Some(css))
}

/// `html > body(margin:0) > [div(height:50px), table(border-spacing:0)]`,
/// and the table.
fn page_with_table(doc: &mut Document) -> usize {
    let html = element(doc, 0, "html", "display:block");
    let body = element(doc, html, "body", "display:block;margin:0");
    element(doc, body, "div", "display:block;height:50px");
    element(doc, body, "table", "display:table;border-spacing:0")
}

fn cell(doc: &mut Document, row: usize) -> usize {
    element(
        doc,
        row,
        "td",
        "display:table-cell;width:80px;height:20px;padding:0",
    )
}

fn lay_out(doc: &mut Document, paged: bool) {
    let rules = build_rule_tree(doc);
    let cascade = cascade(doc, &rules).expect("cascade");
    with_ahem(doc);
    if paged {
        layout_pages_with_page_geometry_and_control(
            doc,
            &cascade,
            page_box_800x600(),
            &[],
            &[],
            &PageLayoutControl::default(),
        )
        .expect("paged layout");
    } else {
        crate::layout::layout_single_page(doc, &cascade, page_box_800x600()).expect("layout");
    }
}

/// A table of a spacer row and the row or row group `tag` styled `part_css`,
/// laid out on one page or paginated. A cell of the row holds a block styled
/// `child_css`. Returns the document, the block and the top of the part on
/// the page: the header group comes before the spacer, every other part
/// after it.
fn table_with_part(
    tag: &str,
    part_css: &str,
    child_css: &str,
    paged: bool,
) -> (Document, usize, f32) {
    let mut doc = Document::new();
    let table = page_with_table(&mut doc);
    let (row, top) = if tag == "tr" {
        let spacer = element(&mut doc, table, "tr", "display:table-row");
        cell(&mut doc, spacer);
        let row = element(
            &mut doc,
            table,
            "tr",
            &format!("display:table-row;{part_css}"),
        );
        (row, 70.0)
    } else {
        let spacer_group = element(&mut doc, table, "tbody", "display:table-row-group");
        let spacer = element(&mut doc, spacer_group, "tr", "display:table-row");
        cell(&mut doc, spacer);
        let display = match tag {
            "tbody" => "table-row-group",
            "thead" => "table-header-group",
            _ => "table-footer-group",
        };
        let group = element(
            &mut doc,
            table,
            tag,
            &format!("display:{display};{part_css}"),
        );
        let row = element(&mut doc, group, "tr", "display:table-row");
        (row, if tag == "thead" { 50.0 } else { 70.0 })
    };
    let cell = cell(&mut doc, row);
    let child = element(&mut doc, cell, "div", &format!("display:block;{child_css}"));
    lay_out(&mut doc, paged);
    (doc, child, top)
}

#[test]
fn transformed_and_filtered_rows_and_row_groups_contain_fixed_boxes() {
    for paged in [false, true] {
        for tag in PARTS {
            for part_css in ["transform:translateX(20px)", "filter:blur(1px)"] {
                let (doc, fixed, top) = table_with_part(tag, part_css, FIXED_CSS, paged);
                assert_eq!(
                    absolute_rect(&doc, fixed),
                    (10.0, top + 15.0, 30.0, 10.0),
                    "{tag} {part_css} paged={paged}"
                );
            }
        }
    }
}

#[test]
fn positioned_rows_and_row_groups_contain_absolute_boxes_but_not_fixed_boxes() {
    for paged in [false, true] {
        for tag in PARTS {
            let (doc, absolute, top) =
                table_with_part(tag, "position:relative", ABSOLUTE_CSS, paged);
            assert_eq!(
                absolute_rect(&doc, absolute),
                (10.0, top + 15.0, 30.0, 10.0),
                "{tag} absolute paged={paged}"
            );
            // The page area, not the row, contains the fixed box.
            let (doc, fixed, _) = table_with_part(tag, "position:relative", FIXED_CSS, paged);
            assert_eq!(
                absolute_rect(&doc, fixed),
                (10.0, 15.0, 30.0, 10.0),
                "{tag} fixed paged={paged}"
            );
        }
    }
}

#[test]
fn a_transformed_row_is_the_percentage_basis_of_its_boxes() {
    for paged in [false, true] {
        let mut doc = Document::new();
        let table = page_with_table(&mut doc);
        let first = element(&mut doc, table, "tr", "display:table-row");
        let second = element(
            &mut doc,
            table,
            "tr",
            "display:table-row;transform:translateX(10px)",
        );
        for row in [first, second] {
            element(
                &mut doc,
                row,
                "td",
                "display:table-cell;width:50px;height:50px;padding:0",
            );
        }
        let holder = element(
            &mut doc,
            second,
            "td",
            "display:table-cell;width:50px;height:50px;padding:0",
        );
        let absolute = element(
            &mut doc,
            holder,
            "div",
            "display:block;position:absolute;left:20px;top:20px;width:100%;height:100%",
        );
        lay_out(&mut doc, paged);
        // The second row is the 100px by 50px box at (0, 100) below the
        // block and the first row.
        assert_eq!(
            absolute_rect(&doc, absolute),
            (20.0, 120.0, 100.0, 50.0),
            "paged={paged}"
        );
    }
}

#[test]
fn rows_of_a_nested_table_leave_boxes_they_do_not_contain_to_the_outer_row() {
    for paged in [false, true] {
        let mut doc = Document::new();
        let table = page_with_table(&mut doc);
        let outer_row = element(
            &mut doc,
            table,
            "tr",
            "display:table-row;transform:translateX(20px)",
        );
        let outer_cell = cell(&mut doc, outer_row);
        let inner_table = element(
            &mut doc,
            outer_cell,
            "table",
            "display:table;border-spacing:0",
        );
        let inner_row = element(
            &mut doc,
            inner_table,
            "tr",
            "display:table-row;position:relative",
        );
        let inner_cell = cell(&mut doc, inner_row);
        let fixed = element(
            &mut doc,
            inner_cell,
            "div",
            &format!("display:block;{FIXED_CSS}"),
        );
        lay_out(&mut doc, paged);
        // The relative inner row does not contain the fixed box, so it
        // reaches the transformed outer row at y=50.
        assert_eq!(
            absolute_rect(&doc, fixed),
            (10.0, 65.0, 30.0, 10.0),
            "paged={paged}"
        );
    }
}

#[test]
fn the_innermost_part_that_contains_a_box_claims_it() {
    for paged in [false, true] {
        let mut doc = Document::new();
        let table = page_with_table(&mut doc);
        let group = element(
            &mut doc,
            table,
            "tbody",
            "display:table-row-group;transform:translateX(20px)",
        );
        let first = element(&mut doc, group, "tr", "display:table-row");
        let first_cell = cell(&mut doc, first);
        let second = element(
            &mut doc,
            group,
            "tr",
            "display:table-row;transform:translateX(10px)",
        );
        let second_cell = cell(&mut doc, second);
        let in_group = element(
            &mut doc,
            first_cell,
            "div",
            &format!("display:block;{FIXED_CSS}"),
        );
        let in_row = element(
            &mut doc,
            second_cell,
            "div",
            &format!("display:block;{FIXED_CSS}"),
        );
        lay_out(&mut doc, paged);
        // The group is at y=50 and the second row below the first at y=70.
        assert_eq!(
            absolute_rect(&doc, in_group),
            (10.0, 65.0, 30.0, 10.0),
            "paged={paged}"
        );
        assert_eq!(
            absolute_rect(&doc, in_row),
            (10.0, 85.0, 30.0, 10.0),
            "paged={paged}"
        );
    }
}
