//! Table parts outside any table (rows, row groups, cells and captions), which
//! an anonymous table wraps (CSS 2.1 §17.2.1 rule 3, CSS Tables 3 §3.9).

use crate::document::Document;
use crate::layout::test_support::absolute_rect;
use raikiri_style::{build_rule_tree, cascade};

pub(super) fn div(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(css))
}

/// `html > body(display:block; body_css)`, with `build` filling the body,
/// laid out on one 200px by 200px page.
pub(super) fn laid_out(body_css: &str, build: impl FnOnce(&mut Document, usize)) -> Document {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some(&format!("display:block;margin:0;width:200px;{body_css}")),
    );
    build(&mut doc, body);
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 200.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    doc
}

#[test]
fn sibling_rows_share_the_columns_of_one_shrink_to_fit_table() {
    let mut ids = Vec::new();
    let doc = laid_out("", |doc, body| {
        ids.push(div(doc, body, "display:block;height:5px"));
        let first = div(doc, body, "display:table-row");
        ids.push(first);
        ids.push(div(doc, first, "display:table-cell;width:20px;height:10px"));
        ids.push(div(doc, first, "display:table-cell;width:30px;height:10px"));
        // White space between the rows does not end the run.
        doc.append_text(body, "\n  ");
        let second = div(doc, body, "display:table-row");
        ids.push(second);
        ids.push(div(
            doc,
            second,
            "display:table-cell;width:40px;height:10px",
        ));
        ids.push(div(
            doc,
            second,
            "display:table-cell;width:10px;height:15px",
        ));
        ids.push(div(doc, body, "display:block;height:5px"));
    });
    let rects: Vec<_> = ids.iter().map(|&id| absolute_rect(&doc, id)).collect();
    assert_eq!(
        rects,
        vec![
            (0.0, 0.0, 200.0, 5.0),
            // The rows are as wide as their columns, not as their parent.
            (0.0, 5.0, 70.0, 10.0),
            (0.0, 5.0, 40.0, 10.0),
            (40.0, 5.0, 30.0, 10.0),
            (0.0, 15.0, 70.0, 15.0),
            (0.0, 15.0, 40.0, 15.0),
            (40.0, 15.0, 30.0, 15.0),
            (0.0, 30.0, 200.0, 5.0),
        ]
    );
}

#[test]
fn text_in_a_row_takes_an_anonymous_cell_beside_its_cells() {
    let mut cells = [0; 2];
    let doc = laid_out(
        "font-family:Ahem;font-size:10px;line-height:10px",
        |doc, body| {
            let row = div(doc, body, "display:table-row");
            doc.append_text(row, "AB");
            cells[0] = div(doc, row, "display:table-cell");
            doc.append_text(cells[0], "C");
            cells[1] = row;
        },
    );
    // `AB` fills a 20px anonymous cell, so the cell follows it on the same
    // line instead of below it.
    assert_eq!(absolute_rect(&doc, cells[0]), (20.0, 0.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, cells[1]), (0.0, 0.0, 30.0, 10.0));
}

#[test]
fn a_row_group_and_its_sibling_rows_form_one_table() {
    let mut cells = [0; 2];
    let doc = laid_out("", |doc, body| {
        let group = div(doc, body, "display:table-row-group");
        let row = div(doc, group, "display:table-row");
        cells[0] = div(doc, row, "display:table-cell;width:50px;height:10px");
        let row = div(doc, body, "display:table-row");
        cells[1] = div(doc, row, "display:table-cell;width:10px;height:10px");
    });
    assert_eq!(absolute_rect(&doc, cells[0]), (0.0, 0.0, 50.0, 10.0));
    assert_eq!(absolute_rect(&doc, cells[1]), (0.0, 10.0, 50.0, 10.0));
}

#[test]
fn a_block_between_rows_starts_another_table() {
    let mut cells = [0; 2];
    let doc = laid_out("", |doc, body| {
        let row = div(doc, body, "display:table-row");
        cells[0] = div(doc, row, "display:table-cell;width:50px;height:10px");
        div(doc, body, "display:block;height:5px");
        let row = div(doc, body, "display:table-row");
        cells[1] = div(doc, row, "display:table-cell;width:10px;height:10px");
    });
    assert_eq!(doc.table_objects.tables.len(), 2);
    assert_eq!(absolute_rect(&doc, cells[0]), (0.0, 0.0, 50.0, 10.0));
    assert_eq!(absolute_rect(&doc, cells[1]), (0.0, 15.0, 10.0, 10.0));
}

/// A `div` in the Ahem body (10px font, 10px line height) that `build` fills,
/// laid out. Returns the document and the `div`.
fn in_paragraph(
    paragraph_css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, usize) {
    let mut paragraph = 0;
    let doc = laid_out(
        "font-family:Ahem;font-size:10px;line-height:10px",
        |doc, body| {
            paragraph = div(doc, body, &format!("display:block;{paragraph_css}"));
            build(doc, paragraph);
        },
    );
    (doc, paragraph)
}

fn cell(doc: &mut Document, parent: usize, width: u32) -> usize {
    div(
        doc,
        parent,
        &format!("display:table-cell;width:{width}px;height:10px"),
    )
}

#[test]
fn a_row_between_lines_is_a_block_level_table_between_them() {
    let mut ids = [0; 2];
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        doc.append_text(paragraph, "A");
        ids[0] = div(doc, paragraph, "display:table-row");
        ids[1] = cell(doc, ids[0], 10);
        doc.append_text(paragraph, "B");
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    // The table is as wide as its cell, between the lines of "A" and "B".
    assert_eq!(absolute_rect(&doc, ids[0]), (0.0, 10.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 10.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph), (0.0, 0.0, 200.0, 30.0));
}

#[test]
fn rows_between_lines_share_the_columns_of_one_table() {
    let mut rows = [0; 2];
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        doc.append_text(paragraph, "A");
        rows[0] = div(doc, paragraph, "display:table-row");
        cell(doc, rows[0], 20);
        rows[1] = div(doc, paragraph, "display:table-row");
        cell(doc, rows[1], 10);
        doc.append_text(paragraph, "B");
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    assert_eq!(absolute_rect(&doc, rows[0]), (0.0, 10.0, 20.0, 10.0));
    assert_eq!(absolute_rect(&doc, rows[1]), (0.0, 20.0, 20.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 40.0);
}

#[test]
fn a_line_between_rows_starts_another_table() {
    let mut rows = [0; 2];
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        rows[0] = div(doc, paragraph, "display:table-row");
        cell(doc, rows[0], 20);
        doc.append_text(paragraph, "A");
        rows[1] = div(doc, paragraph, "display:table-row");
        cell(doc, rows[1], 10);
    });
    assert_eq!(doc.table_objects.tables.len(), 2);
    assert_eq!(absolute_rect(&doc, rows[0]), (0.0, 0.0, 20.0, 10.0));
    assert_eq!(absolute_rect(&doc, rows[1]), (0.0, 20.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 30.0);
}

#[test]
fn collapsible_white_space_around_a_row_makes_no_line() {
    let mut row = 0;
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        doc.append_text(paragraph, "   ");
        row = div(doc, paragraph, "display:table-row");
        cell(doc, row, 10);
        doc.append_text(paragraph, "   ");
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    assert_eq!(absolute_rect(&doc, row), (0.0, 0.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 10.0);
}

#[test]
fn preserved_white_space_before_a_row_is_a_line_above_the_table() {
    let mut row = 0;
    let (doc, paragraph) = in_paragraph("white-space:pre", |doc, paragraph| {
        doc.append_text(paragraph, "\n");
        row = div(doc, paragraph, "display:table-row");
        cell(doc, row, 10);
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    assert_eq!(absolute_rect(&doc, row), (0.0, 10.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 20.0);
}

#[test]
fn a_row_after_a_line_and_before_another_keeps_the_lines_either_side() {
    let (mut inline, mut row, mut after) = (0, 0, 0);
    let (doc, _) = in_paragraph("", |doc, paragraph| {
        inline = doc.append_element(Some(paragraph), "b", Some("display:inline"));
        doc.append_text(inline, "A");
        row = div(doc, paragraph, "display:table-row");
        cell(doc, row, 10);
        after = doc.append_element(Some(paragraph), "i", Some("display:inline"));
        doc.append_text(after, "B");
    });
    assert_eq!(absolute_rect(&doc, inline), (0.0, 0.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, row), (0.0, 10.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, after), (0.0, 20.0, 10.0, 10.0));
}

#[test]
fn a_row_group_and_bare_cells_between_lines_are_wrapped_too() {
    let (mut group, mut cells) = (0, [0; 2]);
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        doc.append_text(paragraph, "A");
        group = div(doc, paragraph, "display:table-row-group");
        let row = div(doc, group, "display:table-row");
        cell(doc, row, 10);
        doc.append_text(paragraph, "B");
        doc.append_text(paragraph, "C");
        cells[0] = cell(doc, paragraph, 10);
        cells[1] = cell(doc, paragraph, 20);
    });
    assert_eq!(doc.table_objects.tables.len(), 2);
    assert_eq!(absolute_rect(&doc, group), (0.0, 10.0, 10.0, 10.0));
    // "BC" is one line between the tables; the cells fall in a row of their own.
    assert_eq!(absolute_rect(&doc, cells[0]), (0.0, 30.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, cells[1]), (10.0, 30.0, 20.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 40.0);
}

#[test]
fn a_column_between_lines_is_an_empty_table_between_them() {
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        doc.append_text(paragraph, "aa");
        div(doc, paragraph, "display:table-column;width:30px");
        doc.append_text(paragraph, "bb");
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    // The table has no height, but it is a block-level box: it ends the line.
    assert_eq!(absolute_rect(&doc, paragraph).3, 20.0);
}

#[test]
fn comments_and_boxes_that_generate_nothing_between_rows_leave_one_table() {
    let mut rows = [0; 2];
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        doc.append_text(paragraph, "A");
        rows[0] = div(doc, paragraph, "display:table-row");
        cell(doc, rows[0], 20);
        doc.append_comment(Some(paragraph), "gap");
        doc.append_text(paragraph, "  ");
        div(doc, paragraph, "display:none");
        rows[1] = div(doc, paragraph, "display:table-row");
        cell(doc, rows[1], 10);
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    assert_eq!(absolute_rect(&doc, rows[1]), (0.0, 20.0, 20.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 30.0);
}

#[test]
fn a_row_before_the_only_line_leaves_the_line_below_it() {
    let mut row = 0;
    let (doc, paragraph) = in_paragraph("", |doc, paragraph| {
        row = div(doc, paragraph, "display:table-row");
        cell(doc, row, 10);
        doc.append_text(paragraph, "A");
    });
    assert_eq!(absolute_rect(&doc, row), (0.0, 0.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, paragraph).3, 20.0);
}

#[test]
fn rows_in_an_inline_block_beside_text_make_a_table_in_it() {
    let (mut inline_block, mut row) = (0, 0);
    let (doc, _) = in_paragraph("", |doc, paragraph| {
        inline_block = div(doc, paragraph, "display:inline-block");
        doc.append_text(inline_block, "A");
        row = div(doc, inline_block, "display:table-row");
        cell(doc, row, 20);
    });
    assert_eq!(doc.table_objects.tables.len(), 1);
    assert_eq!(absolute_rect(&doc, inline_block), (0.0, 0.0, 20.0, 20.0));
    assert_eq!(absolute_rect(&doc, row), (0.0, 10.0, 20.0, 10.0));
}

#[test]
fn the_anonymous_table_inherits_border_spacing_from_its_parent() {
    let mut cells = [0; 2];
    let doc = laid_out("border-spacing:4px 2px", |doc, body| {
        let row = div(doc, body, "display:table-row");
        cells[0] = div(doc, row, "display:table-cell;width:10px;height:10px");
        cells[1] = div(doc, row, "display:table-cell;width:10px;height:10px");
    });
    assert_eq!(absolute_rect(&doc, cells[0]), (4.0, 2.0, 10.0, 10.0));
    assert_eq!(absolute_rect(&doc, cells[1]), (18.0, 2.0, 10.0, 10.0));
}

#[test]
fn an_empty_row_group_keeps_its_place_across_layout_passes() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    div(&mut doc, body, "display:block;height:50px");
    let group = div(&mut doc, body, "display:table-row-group");
    let row = div(&mut doc, body, "display:table-row");
    div(&mut doc, row, "display:table-cell;width:10px;height:10px");
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 200.0;
    let mut tops = Vec::new();
    for _ in 0..3 {
        crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
        tops.push(absolute_rect(&doc, group).1);
    }
    assert_eq!(tops, vec![50.0; 3]);
}

#[test]
fn bare_cells_captions_and_columns_keep_their_places_across_layout_passes() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    div(&mut doc, body, "display:block;height:50px");
    let wrapper = div(&mut doc, body, "display:contents");
    let parts = [
        div(&mut doc, wrapper, "display:table-column;width:20px"),
        div(
            &mut doc,
            wrapper,
            "display:table-caption;width:30px;height:10px",
        ),
        div(
            &mut doc,
            body,
            "display:table-caption;width:30px;height:10px",
        ),
        div(&mut doc, body, "display:table-cell;width:10px;height:10px"),
        div(&mut doc, body, "display:table-row-group"),
    ];
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 200.0;
    page.height = 200.0;
    let mut passes = Vec::new();
    for _ in 0..3 {
        crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
        passes.push(parts.map(|part| absolute_rect(&doc, part)));
    }
    assert_eq!(passes[0], passes[1]);
    assert_eq!(passes[1], passes[2]);
    // The caption sits below the block above the table, and so does the cell.
    assert_eq!(passes[0][1].1, 50.0);
}

#[test]
fn a_contents_wrapper_with_generated_content_keeps_the_cells_around_it_blocks() {
    let mut doc = Document::new();
    let style = doc.append_element(Some(0), "style", Some("display:none"));
    doc.append_text(style, "main::before{content:'X'}");
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let wrapper = doc.append_element(Some(body), "main", Some("display:contents"));
    div(&mut doc, wrapper, "display:table-cell");
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(doc.table_objects.tables.is_empty());
}

/// The number of anonymous tables built for a row in a `body` with `body_css`
/// and the style sheet `sheet`.
fn anonymous_tables_around_a_row(sheet: &str, body_css: &str) -> usize {
    let mut doc = Document::new();
    let style = doc.append_element(Some(0), "style", Some("display:none"));
    doc.append_text(style, sheet);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some(&format!("display:block;{body_css}")),
    );
    let row = div(&mut doc, body, "display:table-row");
    div(&mut doc, row, "display:table-cell");
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.table_objects.tables.len()
}

#[test]
fn multicol_and_inside_marker_parents_keep_rows_blocks() {
    for (sheet, body_css) in [
        ("", "column-count:2"),
        ("", "column-width:100px"),
        // The marker sits on the first line of the item.
        ("", "display:list-item;list-style:inside disc"),
    ] {
        assert_eq!(
            anonymous_tables_around_a_row(sheet, body_css),
            0,
            "{sheet}{body_css}"
        );
    }
}

#[test]
fn a_parent_with_generated_content_wraps_its_rows() {
    for sheet in [
        "body::before{content:'X';display:block}",
        "body::after{content:'X';display:block}",
        "body::before{content:'X'}body::after{content:'Y'}",
    ] {
        assert_eq!(anonymous_tables_around_a_row(sheet, ""), 1, "{sheet}");
    }
}

/// The number of anonymous tables built in a vertical `body` that holds a row
/// and the block `sibling` builds, with a block of its own below `sibling`.
fn vertical_tables_beside(sibling: impl FnOnce(&mut Document, usize)) -> usize {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;writing-mode:vertical-rl"),
    );
    let block = div(&mut doc, body, "display:block");
    sibling(&mut doc, block);
    let row = div(&mut doc, body, "display:table-row");
    div(&mut doc, row, "display:table-cell");
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
    doc.table_objects.tables.len()
}

#[test]
fn a_vertical_parent_whose_blocks_hold_lines_keeps_rows_blocks() {
    // The text makes the vertical parent lay its children out as lines (see
    // `inline_content`), which places no anonymous table.
    assert_eq!(
        vertical_tables_beside(|doc, block| {
            doc.append_text(block, "x");
        }),
        0
    );
    assert_eq!(
        vertical_tables_beside(|doc, block| {
            let inner = div(doc, block, "display:block");
            doc.append_text(inner, "x");
        }),
        0
    );
    assert_eq!(
        vertical_tables_beside(|doc, block| {
            doc.append_element(Some(block), "span", Some("display:inline"));
        }),
        0
    );
    // Empty blocks, white space and hidden boxes leave no line.
    assert_eq!(
        vertical_tables_beside(|doc, block| {
            doc.append_text(block, " \n ");
            div(doc, block, "display:block");
            let hidden = div(doc, block, "display:none");
            doc.append_text(hidden, "x");
            // A script generates no box, whatever its text or display.
            let script = doc.append_element(Some(block), "script", Some("display:block"));
            doc.append_text(script, "x");
        }),
        1
    );
}

#[test]
fn vertical_and_outside_marker_list_item_parents_wrap_their_rows() {
    for body_css in [
        "writing-mode:vertical-rl",
        "writing-mode:vertical-lr",
        "display:list-item;list-style:outside disc",
        "display:list-item;list-style:none",
        // No marker is drawn, so nothing sits on the first line.
        "display:list-item;list-style:inside none",
    ] {
        assert_eq!(anonymous_tables_around_a_row("", body_css), 1, "{body_css}");
    }
}

#[test]
fn a_row_belongs_to_the_anonymous_table_around_it_not_to_a_table_further_out() {
    let (mut loose, mut grouped, mut nested, mut in_table, mut inner, mut outer) =
        (0, 0, 0, 0, 0, 0);
    let (mut group, mut inline_row) = (0, 0);
    let doc = laid_out("", |doc, body| {
        // A row beside rows, and a row inside a row group, outside any table.
        loose = div(doc, body, "display:table-row");
        div(doc, loose, "display:table-cell;height:10px");
        div(doc, body, "display:block;height:5px");
        group = div(doc, body, "display:table-row-group");
        grouped = div(doc, group, "display:table-row");
        div(doc, grouped, "display:table-cell;height:10px");
        div(doc, body, "display:block;height:5px");
        // A table whose cell holds rows outside any table.
        outer = div(doc, body, "display:table");
        let outer_row = div(doc, outer, "display:table-row");
        in_table = div(doc, outer, "display:table-row");
        div(doc, in_table, "display:table-cell;height:10px");
        let cell = div(doc, outer_row, "display:table-cell");
        nested = div(doc, cell, "display:table-row");
        div(doc, nested, "display:table-cell;height:10px");
        // A table in another cell keeps its own rows.
        let other = div(doc, outer_row, "display:table-cell");
        inner = div(doc, other, "display:inline-table");
        inline_row = div(doc, inner, "display:table-row");
        div(doc, inline_row, "display:table-cell;height:10px");
    });
    let anonymous = |row: usize| doc.table_of_row(row).filter(|&t| doc.is_anonymous_table(t));
    // The anonymous tables are not DOM ancestors of the rows they wrap.
    assert_eq!(
        doc.anonymous_table_parts(anonymous(loose).unwrap()),
        Some(&[loose][..])
    );
    assert_eq!(anonymous(grouped), doc.anonymous_table_of(group));
    assert!(anonymous(grouped).is_some());
    assert_ne!(anonymous(loose), anonymous(grouped));
    // The rows in the cell are wrapped apart from the table around the cell.
    assert!(anonymous(nested).is_some());
    assert_eq!(
        doc.table_parts_boundary(anonymous(nested).unwrap()),
        doc.parent_of(nested).unwrap()
    );
    // Rows of source tables belong to those tables.
    assert_eq!(doc.table_of_row(in_table), Some(outer));
    assert_eq!(doc.table_of_row(inline_row), Some(inner));
}
