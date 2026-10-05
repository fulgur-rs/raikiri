use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use taffy::Style;

/// Rects of a column of two Ahem paragraphs in a positioned container under
/// a root with `root_css`. `column_css` styles the column.
fn column_rects(root_css: &str, column_css: &str) -> Vec<(f32, f32, f32, f32)> {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some(format!("display:block;{root_css}").as_str()),
    );
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;position:relative"),
    );
    let column = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some(format!("display:block;{column_css}").as_str()),
    );
    let mut paragraphs = Vec::new();
    for text in ["aa bb", "cc dd"] {
        let paragraph =
            doc.append_element(Some(column), "div", Style::default(), Some("display:block"));
        doc.append_text(paragraph, text);
        paragraphs.push(paragraph);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    std::iter::once(column)
        .chain(paragraphs)
        .map(|node| absolute_rect(&doc, node))
        .collect()
}

#[test]
fn vertical_rl_absolute_box_shrinks_to_its_containing_block_inline_size() {
    let in_flow = column_rects("writing-mode:vertical-rl", "");
    let absolute = column_rects("writing-mode:vertical-rl", "position:absolute");
    // Each paragraph keeps "aa bb" on one 50px line instead of breaking at
    // min-content, and the box sits on the block-start (right) edge.
    assert_eq!(in_flow[0], (780.0, 0.0, 20.0, 50.0));
    assert_eq!(absolute, in_flow);
}

#[test]
fn vertical_lr_absolute_box_starts_at_the_left_edge() {
    let in_flow = column_rects("writing-mode:vertical-lr", "");
    let absolute = column_rects("writing-mode:vertical-lr", "position:absolute");
    assert_eq!(in_flow[0], (0.0, 0.0, 20.0, 50.0));
    assert_eq!(absolute, in_flow);
}

#[test]
fn vertical_absolute_box_keeps_authored_physical_offsets() {
    let rects = column_rects(
        "writing-mode:vertical-rl",
        "position:absolute;left:10px;top:30px",
    );
    assert_eq!(rects[0], (10.0, 30.0, 20.0, 50.0));
}

#[test]
fn horizontal_absolute_box_is_unchanged() {
    let rects = column_rects("", "position:absolute");
    assert_eq!(rects[0], (0.0, 0.0, 50.0, 20.0));
}
