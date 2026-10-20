//! Rows and row groups outside any table, which an anonymous table wraps
//! (CSS 2.1 §17.2.1 rule 3, CSS Tables 3 §3.9).

use crate::LayoutStyle as Style;
use crate::document::Document;
use crate::layout::test_support::absolute_rect;
use raikiri_style::{build_rule_tree, cascade};

fn div(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Style::default(), Some(css))
}

/// `html > body(display:block; body_css)`, with `build` filling the body,
/// laid out on one 200px by 200px page.
fn laid_out(body_css: &str, build: impl FnOnce(&mut Document, usize)) -> Document {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
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

#[test]
fn rows_beside_inline_content_stay_blocks() {
    let mut row = 0;
    let doc = laid_out(
        "font-family:Ahem;font-size:10px;line-height:10px",
        |doc, body| {
            doc.append_text(body, "A");
            row = div(doc, body, "display:table-row");
            div(doc, row, "display:table-cell;width:10px;height:10px");
        },
    );
    // The inline engine lays out the body's children, rows included.
    assert!(doc.table_objects.tables.is_empty());
    assert_eq!(absolute_rect(&doc, row).2, 200.0);
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
fn preserved_white_space_beside_rows_keeps_them_blocks() {
    let doc = laid_out(
        "font-family:Ahem;font-size:10px;line-height:10px;white-space:pre",
        |doc, body| {
            doc.append_text(body, "\n");
            let row = div(doc, body, "display:table-row");
            div(doc, row, "display:table-cell;width:10px;height:10px");
        },
    );
    // The newline forms a line, so the inline engine lays the rows out.
    assert!(doc.table_objects.tables.is_empty());
}

#[test]
fn an_empty_row_group_keeps_its_place_across_layout_passes() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
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
fn multicol_and_generated_content_parents_keep_rows_blocks() {
    for (sheet, body_css) in [
        ("", "column-count:2"),
        ("", "writing-mode:vertical-rl"),
        ("body::before{content:'X';display:block}", ""),
    ] {
        let mut doc = Document::new();
        let style = doc.append_element(Some(0), "style", Style::default(), Some("display:none"));
        doc.append_text(style, sheet);
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some(&format!("display:block;{body_css}")),
        );
        let row = div(&mut doc, body, "display:table-row");
        div(&mut doc, row, "display:table-cell");
        doc.mark_in_document_flags();
        let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
        assert!(doc.table_objects.tables.is_empty(), "{sheet}{body_css}");
    }
}
