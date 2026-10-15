use super::*;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use taffy::Style;

/// Paginate an 800x600 page holding a root styled `html_css` and a body
/// styled `body_css` (Ahem text, 10px lines) with one block child per entry
/// of `children`: `(tag, css, text)`. Returns the document, the body and its
/// children, and the pages.
fn paginate(
    html_css: &str,
    body_css: &str,
    children: &[(&str, &str, &str)],
) -> (Document, Vec<usize>, Vec<PageSlice>) {
    let (doc, _, ids, pages) = paginate_with_cascade(html_css, body_css, children);
    (doc, ids, pages)
}

/// [`paginate`], also returning the cascade.
fn paginate_with_cascade(
    html_css: &str,
    body_css: &str,
    children: &[(&str, &str, &str)],
) -> (
    Document,
    raikiri_style::CascadeResult,
    Vec<usize>,
    Vec<PageSlice>,
) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some(format!("display:block;{html_css}").as_str()),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some(
            format!("display:block;font-family:Ahem;font-size:10px;line-height:10px;{body_css}")
                .as_str(),
        ),
    );
    let mut ids = vec![body];
    for &(tag, css, text) in children {
        let child = doc.append_element(
            Some(body),
            tag,
            Style::default(),
            Some(format!("display:block;{css}").as_str()),
        );
        if !text.is_empty() {
            doc.append_text(child, text);
        }
        ids.push(child);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let pages = layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("pages");
    (doc, cascade, ids, pages)
}

fn origins(pages: &[PageSlice]) -> Vec<f32> {
    pages.iter().map(|page| page.content_origin_y).collect()
}

#[test]
fn vertical_rl_pages_continue_to_the_left() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            ("div", "width:800px", ""),
            ("div", "width:800px", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0, 1600.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (-800.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[3]), (-1600.0, 0.0, 800.0, 600.0));
}

#[test]
fn vertical_lr_pages_continue_to_the_right() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-lr",
        "margin:0",
        &[("div", "width:800px", ""), ("div", "width:800px", "")],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (800.0, 0.0, 800.0, 600.0));
}

#[test]
fn a_block_that_avoids_breaks_moves_to_the_next_page() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:600px", ""),
            ("div", "width:700px;break-inside:avoid", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (200.0, 0.0, 600.0, 600.0));
    // The second block starts the second page, whose block-start edge is
    // 800px left of the first page's.
    assert_eq!(absolute_rect(&doc, ids[2]), (-700.0, 0.0, 700.0, 600.0));
}

#[test]
fn a_forced_break_starts_a_page_along_the_block_axis() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:100px", ""),
            ("div", "width:100px;break-before:page", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (700.0, 0.0, 100.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (-100.0, 0.0, 100.0, 600.0));
}

#[test]
fn lines_of_a_vertical_paragraph_continue_on_the_next_page() {
    // 100 lines of 10px: the first 80 fill the first page.
    let text = vec!["X"; 100].join(" ");
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("p", "margin:0;height:10px", text.as_str())],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (-200.0, 0.0, 1000.0, 10.0));
}

#[test]
fn a_horizontal_root_is_paginated_along_y() {
    let (_, _, pages) = paginate(
        "",
        "margin:0",
        &[("div", "height:600px", ""), ("div", "height:600px", "")],
    );
    assert_eq!(origins(&pages), [0.0, 600.0]);
}

#[test]
fn a_moved_block_of_a_vertical_lr_root_lands_on_the_next_page() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-lr",
        "margin:0",
        &[
            ("div", "width:600px", ""),
            ("div", "width:700px;break-inside:avoid", ""),
        ],
    );
    assert_eq!(origins(&pages), [0.0, 800.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (0.0, 0.0, 600.0, 600.0));
    assert_eq!(absolute_rect(&doc, ids[2]), (800.0, 0.0, 700.0, 600.0));
}

#[test]
fn logical_sizes_of_a_vertical_box_keep_their_axes() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("div", "block-size:100px;inline-size:300px", "")],
    );
    assert_eq!(origins(&pages), [0.0]);
    assert_eq!(absolute_rect(&doc, ids[1]), (700.0, 0.0, 100.0, 300.0));
}

#[test]
fn a_table_of_bare_text_stays_at_the_block_start_edge() {
    let (doc, ids, pages) = paginate(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("div", "display:table", "X")],
    );
    assert_eq!(origins(&pages), [0.0]);
    assert_eq!(doc.anonymous_table_cells(ids[1]).count(), 1);
    let (x, y, width, _) = absolute_rect(&doc, ids[1]);
    assert_eq!((x + width, y), (800.0, 0.0));
}

/// The border boxes `node` has on each page after projection, page-local.
fn projected_rects(
    doc: &mut Document,
    cascade: &raikiri_style::CascadeResult,
    pages: &[PageSlice],
    node: usize,
) -> Vec<Vec<(f32, f32, f32, f32)>> {
    let page = page_box_800x600();
    let geometry = (
        page,
        crate::layout::page_margins(cascade, page),
        crate::layout::page_content_insets(cascade, page),
    );
    doc.project_pages(cascade, page, pages, &vec![geometry; pages.len()])
        .expect("projection");
    pages
        .iter()
        .map(|slice| {
            doc.page_fragments(slice.page_index)
                .filter(|fragment| fragment.node() == raikiri_traits::NodeId::new(node as u64))
                .map(|fragment| {
                    let rect = fragment.rect();
                    (rect.x, rect.y, rect.width, rect.height)
                })
                .collect()
        })
        .collect()
}

#[test]
fn projection_places_blocks_on_their_pages_along_x() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("div", "width:800px", ""), ("div", "width:800px", "")],
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[1]),
        [vec![(0.0, 0.0, 800.0, 600.0)], vec![]]
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[2]),
        [vec![], vec![(0.0, 0.0, 800.0, 600.0)]]
    );
}

#[test]
fn projection_splits_a_vertical_paragraph_by_its_lines() {
    // 100 lines of 10px: the first 80 fill the first page.
    let text = vec!["X"; 100].join(" ");
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[("p", "margin:0;height:10px", text.as_str())],
    );
    let text_node = doc.nodes[ids[1]].children[0];
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, text_node),
        [
            vec![(0.0, 0.0, 800.0, 10.0)],
            vec![(600.0, 0.0, 200.0, 10.0)]
        ]
    );
    let line_ranges: Vec<Vec<_>> = pages
        .iter()
        .map(|slice| {
            doc.page_fragments(slice.page_index)
                .filter(|fragment| fragment.node() == raikiri_traits::NodeId::new(text_node as u64))
                .filter_map(|fragment| fragment.line_range())
                .collect()
        })
        .collect();
    assert_eq!(line_ranges, [0..80, 80..100].map(|range| vec![range]));
}

#[test]
fn projection_places_the_text_of_a_horizontal_box_on_its_page() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-rl",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            (
                "div",
                "writing-mode:horizontal-tb;width:100px;height:600px",
                "X",
            ),
        ],
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[2]),
        [vec![], vec![(700.0, 0.0, 100.0, 600.0)]]
    );
    assert!(doc.page_text_runs(&cascade, 0).is_empty());
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(runs.len(), 1);
    let (x, y) = runs[0].origin;
    assert!((700.0..800.0).contains(&x), "{x}");
    assert!((0.0..20.0).contains(&y), "{y}");
}

#[test]
fn projection_places_blocks_of_a_vertical_lr_root_on_their_pages() {
    let (mut doc, cascade, ids, pages) = paginate_with_cascade(
        "writing-mode:vertical-lr",
        "margin:0",
        &[
            ("div", "width:800px", ""),
            (
                "div",
                "writing-mode:horizontal-tb;width:780px;height:590px;padding:10px 0 0 20px",
                "X",
            ),
        ],
    );
    assert_eq!(
        projected_rects(&mut doc, &cascade, &pages, ids[2]),
        [vec![], vec![(0.0, 0.0, 800.0, 600.0)]]
    );
    let content = doc
        .page_fragments(pages[1].page_index)
        .find(|fragment| fragment.node() == raikiri_traits::NodeId::new(ids[2] as u64))
        .and_then(|fragment| fragment.content_rect())
        .map(|rect| (rect.x, rect.y, rect.width, rect.height));
    assert_eq!(content, Some((20.0, 10.0, 780.0, 590.0)));
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(runs.len(), 1);
    assert!(
        (20.0..40.0).contains(&runs[0].origin.0),
        "{:?}",
        runs[0].origin
    );
}
