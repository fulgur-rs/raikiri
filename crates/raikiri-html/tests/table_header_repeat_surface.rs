//! Table headers reserve space and repeat only on their table's pages.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, NodeId, PaintRect,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str) -> DocumentLayout {
    lay_out_with_css(body, "")
}

fn lay_out_with_css(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}body{{margin:0;font:10px/10px Ahem}}table{{border-spacing:0}}td,th{{padding:0;width:20px}}td{{height:20px}}th{{height:10px;font-weight:400}}{css}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

fn table(rows: usize) -> String {
    let rows: String = (0..rows)
        .map(|row| format!("<tr><td id='r{row}'>X</td></tr>"))
        .collect();
    format!("<table><thead><tr><th id='h'>H</th></tr></thead><tbody>{rows}</tbody></table>")
}

fn find_by_id(node: NodeId, dom: &raikiri_html::DomView<'_>, id: &str) -> Option<NodeId> {
    if dom.attr(node, "id") == Some(id) {
        return Some(node);
    }
    dom.children(node)
        .find_map(|child| find_by_id(child, dom, id))
}

fn rect(document: &DocumentLayout, id: &str, page_index: u32) -> PaintRect {
    let page = document.page(page_index).unwrap();
    let dom = page.dom();
    let node = find_by_id(dom.root(), &dom, id).unwrap();
    page.fragments()
        .find(|fragment| fragment.node() == node)
        .unwrap_or_else(|| panic!("{id} must occur on page {page_index}"))
        .rect()
}

fn header_count(document: &DocumentLayout, page_index: u32) -> usize {
    let page = document.page(page_index).unwrap();
    let dom = page.dom();
    let node = find_by_id(dom.root(), &dom, "h").unwrap();
    page.fragments()
        .filter(|fragment| fragment.node() == node)
        .count()
}

#[test]
fn repeated_headers_reserve_room_for_whole_body_rows() {
    let document = lay_out(&table(9));
    assert_eq!(document.page_count(), 3);
    for page in 0..3 {
        assert_eq!(header_count(&document, page), 1);
        assert_eq!(rect(&document, "h", page).y, 0.0);
        assert_eq!(rect(&document, "h", page).height, 10.0);
    }
    for (row, page) in [(0, 0), (4, 1), (8, 2)] {
        assert_eq!(rect(&document, &format!("r{row}"), page).y, 10.0);
        assert_eq!(rect(&document, &format!("r{row}"), page).height, 20.0);
    }
}

#[test]
fn a_single_page_header_is_not_duplicated() {
    let document = lay_out(&table(2));
    assert_eq!(document.page_count(), 1);
    assert_eq!(header_count(&document, 0), 1);
    assert_eq!(rect(&document, "r0", 0).y, 10.0);
    assert_eq!(rect(&document, "r1", 0).y, 30.0);
}

#[test]
fn repeated_headers_preserve_complete_text_and_source_identity() {
    let document = lay_out(&table(5));
    let first_runs = document.page(0).unwrap().text_runs();
    let first = first_runs.iter().find(|run| run.text == "H").unwrap();
    for page in 0..2 {
        let runs = document.page(page).unwrap().text_runs();
        let headers: Vec<_> = runs.iter().filter(|run| run.text == "H").collect();
        assert_eq!(headers.len(), 1, "header text on page {page}");
        assert_eq!(headers[0].source, first.source);
        assert_eq!(headers[0].glyphs.len(), first.glyphs.len());
    }
}

#[test]
fn a_table_starting_after_content_keeps_its_header_on_table_pages() {
    let document = lay_out(&format!(
        "<div style='height:30px'></div>{}<div id='end' style='height:60px'></div>",
        table(5)
    ));
    assert_eq!(rect(&document, "h", 0).y, 30.0);
    assert_eq!(rect(&document, "h", 1).y, 0.0);
    assert_eq!(rect(&document, "r3", 1).y, 10.0);
    assert_eq!(rect(&document, "end", 1).y, 50.0);
    assert!(document.page(2).is_some());
    assert_eq!(header_count(&document, 2), 0);
}

#[test]
fn header_fragment_ordinals_start_at_the_table_not_the_document() {
    let document = lay_out(&format!("<div style='height:100px'></div>{}", table(5)));
    assert_eq!(header_count(&document, 0), 0);
    for (ordinal, page_index) in [1, 2].into_iter().enumerate() {
        let page = document.page(page_index).unwrap();
        let dom = page.dom();
        let h = find_by_id(dom.root(), &dom, "h").unwrap();
        let header = page
            .fragments()
            .find(|fragment| fragment.node() == h)
            .unwrap();
        assert_eq!(header.fragment_index(), ordinal as u32);
        assert_eq!(header.is_first_fragment(), Some(ordinal == 0));
        assert_eq!(header.is_last_fragment(), Some(ordinal == 1));
        assert_eq!(header.repeat(), Some(raikiri_html::RepeatKind::TableHeader));
        assert_eq!(header.rect().y, 0.0);
    }
}

#[test]
fn row_group_display_does_not_acquire_header_repetition() {
    let document = lay_out_with_css(&table(5), "thead{display:table-row-group}");
    assert_eq!(header_count(&document, 0), 1);
    assert_eq!(header_count(&document, 1), 0);
    assert_eq!(rect(&document, "r4", 1).y, 0.0);
}

#[test]
fn only_the_first_header_group_repeats() {
    let body = table(5).replace(
        "</thead>",
        "</thead><thead><tr><th id='second'>S</th></tr></thead>",
    );
    let document = lay_out(&body);
    assert_eq!(header_count(&document, 0), 1);
    assert_eq!(header_count(&document, 1), 1);
    assert_eq!(rect(&document, "second", 0).y, 10.0);
    let page = document.page(1).unwrap();
    let dom = page.dom();
    let second = find_by_id(dom.root(), &dom, "second").unwrap();
    assert!(!page.fragments().any(|fragment| fragment.node() == second));
}

#[test]
fn generated_header_text_reuses_its_counter_value_on_each_copy() {
    let document = lay_out_with_css(
        &table(5),
        "table{counter-reset:part 6}th::before{counter-increment:part;content:counter(part)}th::after{content:'Z'}",
    );
    for page in 0..2 {
        let runs = document.page(page).unwrap().text_runs();
        let text: String = runs
            .iter()
            .filter(|run| run.text != "X")
            .map(|run| run.text)
            .collect();
        assert_eq!(text, "7HZ");
    }
}

#[test]
fn the_initial_header_stays_with_the_first_body_row() {
    for spacer in [85, 95] {
        let document = lay_out(&format!(
            "<div style='height:{spacer}px'></div>{}",
            table(5)
        ));
        assert_eq!(header_count(&document, 0), 0, "spacer {spacer}");
        assert_eq!(rect(&document, "h", 1).y, 0.0);
        assert_eq!(rect(&document, "r0", 1).y, 10.0);
        assert_eq!(rect(&document, "h", 2).y, 0.0);
    }
}

#[test]
fn fixed_header_descendants_keep_every_page_placements() {
    let body = table(5).replace("H</th>", "H<span id='fixed' style='position:fixed;top:2px;left:30px;width:10px;height:10px'>F</span></th>");
    let document = lay_out(&format!("{body}<div style='height:120px'></div>"));
    assert_eq!(document.page_count(), 3);
    for index in 0..3 {
        let page = document.page(index).unwrap();
        let dom = page.dom();
        let fixed = find_by_id(dom.root(), &dom, "fixed").unwrap();
        let fragment = page
            .fragments()
            .find(|fragment| fragment.node() == fixed)
            .unwrap();
        assert_eq!(fragment.repeat(), Some(raikiri_html::RepeatKind::EveryPage));
        assert_eq!(fragment.rect().y, 2.0);
        assert_eq!(
            page.text_runs()
                .iter()
                .filter(|run| run.text == "F")
                .count(),
            1
        );
    }
}

#[test]
fn explicit_header_breaks_keep_the_existing_pagination_boundary() {
    let document = lay_out_with_css(
        &format!("<div style='height:30px'></div>{}", table(5)),
        "thead{break-before:page}",
    );
    assert_eq!(header_count(&document, 0), 0);
    assert_eq!(rect(&document, "h", 1).y, 0.0);
}

#[test]
fn multiple_header_rows_reserve_the_complete_group_height() {
    let body = table(9).replace("</thead>", "<tr><th id='h2'>J</th></tr></thead>");
    let document = lay_out(&body);
    assert_eq!(document.page_count(), 3);
    for page in 0..3 {
        assert_eq!(rect(&document, "h", page).y, 0.0);
        assert_eq!(rect(&document, "h2", page).y, 10.0);
    }
    for (row, page) in [(0, 0), (4, 1), (8, 2)] {
        assert_eq!(rect(&document, &format!("r{row}"), page).y, 20.0);
    }
}

#[test]
fn separate_border_spacing_is_reserved_between_the_header_and_rows() {
    let document = lay_out_with_css(&table(5), "table{border-spacing:2px}");
    assert_eq!(rect(&document, "h", 0).y, 2.0);
    assert_eq!(rect(&document, "r0", 0).y, 14.0);
    assert_eq!(rect(&document, "h", 1).y, 0.0);
    assert_eq!(rect(&document, "r4", 1).y, 12.0);
}

#[test]
fn header_only_and_oversized_header_tables_keep_ordinary_placements() {
    let only = lay_out("<table><thead><tr><th id='h'>H</th></tr></thead></table>");
    assert_eq!(only.page_count(), 1);
    assert_eq!(rect(&only, "h", 0).height, 10.0);
    let oversized = lay_out_with_css(&table(5), "th{height:30px}");
    assert_eq!(header_count(&oversized, 0), 1);
    assert_eq!(header_count(&oversized, 1), 0);
    assert_eq!(rect(&oversized, "r3", 1).y, 0.0);
}

#[test]
fn spanning_rows_and_bottom_captions_keep_the_existing_geometry() {
    let caption = table(5).replace(
        "<table>",
        "<table><caption id='cap' style='caption-side:bottom;height:10px'>C</caption>",
    );
    let span = table(5).replace("id='r0'", "id='r0' rowspan='2'");
    for (body, ids) in [
        (caption, vec!["h", "r0", "r4", "cap"]),
        (span, vec!["h", "r0", "r1", "r4"]),
    ] {
        let actual = lay_out(&body);
        let reference = lay_out_with_css(&body, "thead{display:table-row-group}");
        assert_eq!(actual.page_count(), reference.page_count());
        for index in 0..actual.page_count() {
            for id in &ids {
                let placements = |document: &DocumentLayout| {
                    let page = document.page(index).unwrap();
                    let dom = page.dom();
                    let node = find_by_id(dom.root(), &dom, id).unwrap();
                    page.fragments()
                        .filter(|fragment| fragment.node() == node)
                        .map(|fragment| fragment.rect())
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    placements(&actual),
                    placements(&reference),
                    "{id} page {index}"
                );
            }
        }
    }
}

#[test]
fn contents_wrapped_cells_keep_the_existing_table_projection() {
    for span in [1, 2] {
        let body = table(5).replace("<td id='r0'>X</td>", &format!("<td style='display:contents'><div id='r0' rowspan='{span}' style='display:table-cell;width:20px;height:20px'>X</div></td>"));
        let document = lay_out(&body);
        assert_eq!(header_count(&document, 0), 1);
        assert_eq!(header_count(&document, 1), 0);
        let page = document.page(0).unwrap();
        let dom = page.dom();
        let h = find_by_id(dom.root(), &dom, "h").unwrap();
        let header = page
            .fragments()
            .find(|fragment| fragment.node() == h)
            .unwrap();
        assert_eq!(header.repeat(), None);
    }
}

#[test]
fn multicolumn_tables_keep_the_existing_header_projection() {
    let document = lay_out_with_css(&table(5), "body{column-count:2}");
    for page in document.pages() {
        let dom = page.dom();
        let header = find_by_id(dom.root(), &dom, "h").unwrap();
        assert!(
            page.fragments()
                .filter(|fragment| fragment.node() == header)
                .all(|fragment| fragment.repeat().is_none())
        );
    }
}

#[test]
fn explicit_css_rows_and_hidden_cells_keep_their_header_spacing() {
    let rows: String = (0..9).map(|row| format!("<div class='row'><div class='cell' id='r{row}'>X</div><div class='cell' style='display:none'>hidden</div></div>")).collect();
    let document = lay_out_with_css(
        &format!(
            "<div class='table'><div class='head'><div class='row'><div class='cell' id='h'>H</div></div></div>{rows}</div>"
        ),
        ".table{display:table;border-spacing:0}.head{display:table-header-group}.row{display:table-row}.cell{display:table-cell;width:20px;height:20px;padding:0}.head .cell{height:10px}",
    );
    assert_eq!(document.page_count(), 3);
    for (row, page) in [(0, 0), (4, 1), (8, 2)] {
        assert_eq!(rect(&document, "h", page).y, 0.0);
        assert_eq!(rect(&document, &format!("r{row}"), page).y, 10.0);
        assert!(
            document
                .page(page)
                .unwrap()
                .text_runs()
                .iter()
                .all(|run| !run.text.contains("hidden"))
        );
    }
}

#[test]
fn collapsed_table_headers_with_a_top_caption_repeat_after_the_caption() {
    let body = table(5).replace(
        "<table>",
        "<table><caption id='cap' style='height:10px'>C</caption>",
    );
    let document = lay_out_with_css(&body, "table{border-collapse:collapse}");
    assert_eq!(document.page_count(), 2);
    assert_eq!(rect(&document, "cap", 0).y, 0.0);
    assert_eq!(rect(&document, "h", 0).y, 10.0);
    assert_eq!(rect(&document, "r0", 0).y, 20.0);
    assert_eq!(rect(&document, "h", 1).y, 0.0);
    assert_eq!(rect(&document, "r4", 1).y, 10.0);
}

#[test]
fn header_text_is_absent_from_content_after_its_table() {
    let document = lay_out(&format!(
        "{}<p style='break-before:page;margin:0'>END</p>",
        table(5)
    ));
    assert_eq!(document.page_count(), 3);
    assert_eq!(header_count(&document, 2), 0);
    let page = document.page(2).unwrap();
    let text: String = page.text_runs().iter().map(|run| run.text).collect();
    assert_eq!(text, "END");
}
