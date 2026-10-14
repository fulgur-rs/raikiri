use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use taffy::Style;

/// The rect of a box styled `css` under an 8px-margin body, holding either
/// the Ahem text "aa bb" or a 50px square block.
fn rect_of(css: &str, text: bool) -> (f32, f32, f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:8px"),
    );
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(format!("font-family:Ahem;font-size:10px;line-height:10px;{css}").as_str()),
    );
    if text {
        doc.append_text(container, "aa bb");
    } else {
        doc.append_element(
            Some(container),
            "div",
            Style::default(),
            Some("display:block;width:50px;height:50px"),
        );
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, container)
}

#[test]
fn vertical_block_under_a_horizontal_body_is_as_wide_as_its_lines() {
    // One 10px column of "aa bb", 50px tall, at the body's content edge.
    let rect = rect_of("display:block;writing-mode:vertical-rl", true);
    assert_eq!(rect, (8.0, 8.0, 10.0, 50.0));
    let rect = rect_of("display:block;writing-mode:vertical-lr", true);
    assert_eq!(rect, (8.0, 8.0, 10.0, 50.0));
}

#[test]
fn vertical_flex_container_under_a_horizontal_body_is_as_wide_as_its_items() {
    let rect = rect_of("display:flex;writing-mode:vertical-rl", true);
    assert_eq!(rect, (8.0, 8.0, 10.0, 50.0));
}

#[test]
fn vertical_block_of_blocks_is_as_wide_as_its_children() {
    let rect = rect_of("display:block;writing-mode:vertical-rl", false);
    assert_eq!(rect, (8.0, 8.0, 50.0, 50.0));
}

#[test]
fn vertical_block_keeps_an_authored_width() {
    let rect = rect_of("display:block;writing-mode:vertical-rl;width:100px", true);
    assert_eq!(rect, (8.0, 8.0, 100.0, 50.0));
}

#[test]
fn horizontal_block_still_fills_the_body() {
    let rect = rect_of("display:block", true);
    assert_eq!(rect, (8.0, 8.0, 784.0, 10.0));
}

#[test]
fn vertical_rl_block_keeps_its_absolute_child_on_the_block_start_edge() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:8px"),
    );
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(
            "display:block;writing-mode:vertical-rl;position:relative;\
             font-family:Ahem;font-size:10px;line-height:10px",
        ),
    );
    let absolute = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;position:absolute"),
    );
    doc.append_text(absolute, "dd ee");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let container = absolute_rect(&doc, container);
    let absolute = absolute_rect(&doc, absolute);
    // The empty container has no size, so the absolute box shrinks to its
    // min-content inline size: "dd" and "ee" on two 20px columns, ending
    // at the container's right (block-start) edge.
    assert_eq!(container, (8.0, 8.0, 0.0, 0.0));
    assert_eq!(absolute, (-12.0, 8.0, 20.0, 20.0));
}

#[test]
fn vertical_list_item_is_as_wide_as_its_lines() {
    let rect = rect_of(
        "display:list-item;list-style:none;writing-mode:vertical-rl",
        true,
    );
    assert_eq!(rect, (8.0, 8.0, 10.0, 50.0));
}

#[test]
fn block_inheriting_a_vertical_mode_through_display_contents_is_as_wide_as_its_lines() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:8px"),
    );
    let contents = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:contents;writing-mode:vertical-rl"),
    );
    let block = doc.append_element(
        Some(contents),
        "div",
        Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(block, "aa bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, block), (8.0, 8.0, 10.0, 50.0));
}

#[test]
fn right_float_stays_inside_a_vertical_block() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:8px"),
    );
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;writing-mode:vertical-rl"),
    );
    let float = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;float:right;width:20px;height:20px"),
    );
    doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;width:50px;height:50px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let container = absolute_rect(&doc, container);
    let float = absolute_rect(&doc, float);
    // The float meets the block-start edge of the 50px-wide container, its
    // right edge, not that of the body it was stretched across before. A
    // right float goes to the line-right side, the bottom in `vertical-rl`
    // (CSS Writing Modes 4 §6.2).
    assert_eq!(container, (8.0, 8.0, 50.0, 50.0));
    assert_eq!(float, (38.0, 38.0, 20.0, 20.0));
}
