use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};

/// The rect of a box styled `css` under an 8px-margin body, holding either
/// the Ahem text "aa bb" or a 50px square block.
fn rect_of(css: &str, text: bool) -> (f32, f32, f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(format!("font-family:Ahem;font-size:10px;line-height:10px;{css}").as_str()),
    );
    if text {
        doc.append_text(container, "aa bb");
    } else {
        doc.append_element(
            Some(container),
            "div",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(
            "display:block;writing-mode:vertical-rl;position:relative;\
             font-family:Ahem;font-size:10px;line-height:10px",
        ),
    );
    let absolute = doc.append_element(
        Some(container),
        "div",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let contents = doc.append_element(
        Some(body),
        "div",
        Some("display:contents;writing-mode:vertical-rl"),
    );
    let block = doc.append_element(
        Some(contents),
        "div",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;writing-mode:vertical-rl"),
    );
    let float = doc.append_element(
        Some(container),
        "div",
        Some("display:block;float:right;width:20px;height:20px"),
    );
    doc.append_element(
        Some(container),
        "div",
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

#[test]
fn sideways_blocks_use_the_content_width_under_a_horizontal_parent() {
    // A 50px square block gives a 50px max-content block size in either
    // sideways mode; the horizontal parent must not stretch it to 784px.
    for mode in ["sideways-rl", "sideways-lr"] {
        assert_eq!(
            rect_of(&format!("display:block;writing-mode:{mode}"), false),
            (8.0, 8.0, 50.0, 50.0)
        );
    }
}

#[test]
fn sideways_percentage_padding_uses_the_containing_block_inline_extent() {
    // CSS Writing Modes 4 section 7.2: 10% of the 200px inline extent
    // is 20px, even while sideways glyph shaping retains its fallback.
    for mode in ["sideways-rl", "sideways-lr"] {
        let mut doc = Document::new();
        let body = doc.append_element(Some(0), "body", Some("display:block;margin:0"));
        let parent = doc.append_element(
            Some(body),
            "div",
            Some(format!(
                "display:block;position:relative;writing-mode:{mode};width:100px;height:200px;font:10px Ahem"
            )),
        );
        let child = doc.append_element(
            Some(parent),
            "div",
            Some("display:block;position:absolute;left:0;top:0;width:auto;height:10px;padding-left:10%"),
        );
        doc.append_text(child, "x");
        doc.mark_in_document_flags();
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        assert!(!doc.nodes[parent].is_ifc_root());
        assert!(doc.nodes[child].is_ifc_root());
        assert_eq!(doc.nodes[child].layout.size.width, 30.0);
    }
}

/// A hundred three-letter words, 299 Ahem glyphs: a 60px-tall column of 10px
/// glyphs takes 20 words, so the 600px page area holds 5 columns of it.
fn long_text() -> String {
    vec!["aa"; 100].join(" ")
}

const VERTICAL_TEXT: &str = "display:block;writing-mode:vertical-rl;font-family:Ahem;\
                             font-size:10px;line-height:10px";

/// Lay out `doc` on one 800x600 page.
fn lay_out(doc: &mut Document) {
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(doc);
    let cascade = raikiri_style::cascade(doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(doc), &cascade, page_box_800x600()).expect("layout");
}

/// A zero-margin body that holds a block `parent_css` and, in it, the vertical
/// block `css` with `text`. The node ids are those of the parent and of the
/// vertical block.
fn vertical_block_under(parent_css: &str, css: &str, text: &str) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let parent = doc.append_element(Some(body), "div", Some(parent_css));
    let container = doc.append_element(
        Some(parent),
        "div",
        Some(format!("{VERTICAL_TEXT};{css}").as_str()),
    );
    doc.append_text(container, text);
    (doc, parent, container)
}

/// The border-box rect of a vertical block holding [`long_text`] under the
/// block `parent_css`.
fn long_paragraph_rect(parent_css: &str, css: &str) -> (f32, f32, f32, f32) {
    let (mut doc, _, container) = vertical_block_under(parent_css, css, &long_text());
    lay_out(&mut doc);
    absolute_rect(&doc, container)
}

#[test]
fn a_long_vertical_paragraph_wraps_at_the_page_height() {
    // The lines fit the initial containing block when nothing else gives the
    // box a height (CSS Writing Modes 4 §7.3.1): 5 columns of 600px, not one
    // column of 2990px.
    for mode in ["vertical-rl", "vertical-lr"] {
        assert_eq!(
            long_paragraph_rect("display:block", &format!("writing-mode:{mode}")),
            (0.0, 0.0, 50.0, 600.0)
        );
    }
}

#[test]
fn the_page_height_is_the_room_whatever_edges_and_overflow_an_auto_height_parent_has() {
    for parent in [
        "display:block;padding:20px",
        "display:block;border:5px solid",
        "display:block;overflow:auto",
        "display:block;height:50%",
        "display:block;min-height:300px",
    ] {
        let (_, _, width, height) = long_paragraph_rect(parent, "");
        assert_eq!((width, height), (50.0, 600.0), "{parent}");
    }
}

#[test]
fn a_definite_height_of_the_parent_is_the_height_a_vertical_paragraph_fits() {
    // The 300px parent gives 10 words a column, 10 columns in all.
    assert_eq!(
        long_paragraph_rect("display:block;height:300px", ""),
        (0.0, 0.0, 100.0, 300.0)
    );
    assert_eq!(
        long_paragraph_rect("display:block;height:300px;overflow:hidden", ""),
        (0.0, 0.0, 100.0, 300.0)
    );
    // The height is the one inside the parent's padding and border.
    assert_eq!(
        long_paragraph_rect("display:block;height:300px;border:5px solid", ""),
        (5.0, 5.0, 100.0, 300.0)
    );
    assert_eq!(
        long_paragraph_rect(
            "display:block;box-sizing:border-box;height:300px;padding:20px",
            ""
        ),
        (20.0, 20.0, 120.0, 260.0)
    );
}

#[test]
fn the_margins_of_a_vertical_paragraph_come_off_the_height_it_fits() {
    // 570px: 19 words to a column, 6 columns.
    assert_eq!(
        long_paragraph_rect("display:block", "margin:15px"),
        (15.0, 15.0, 60.0, 570.0)
    );
    assert_eq!(
        long_paragraph_rect("display:block", "margin-top:40px"),
        (0.0, 40.0, 60.0, 560.0)
    );
    // A percentage refers to the 800px width of the parent: 5% is 40px.
    assert_eq!(
        long_paragraph_rect("display:block", "margin:5%"),
        (40.0, 40.0, 60.0, 520.0)
    );
    assert_eq!(
        long_paragraph_rect("display:block;height:300px", "margin:20px"),
        (20.0, 20.0, 120.0, 260.0)
    );
}

#[test]
fn margins_that_leave_no_room_put_every_word_on_a_column_of_its_own() {
    // 350px above and below leave nothing of the 600px: the min-content size,
    // 100 columns of the 20px words.
    assert_eq!(
        long_paragraph_rect("display:block", "margin:350px 0"),
        (0.0, 350.0, 1000.0, 20.0)
    );
}

#[test]
fn the_padding_and_border_of_a_vertical_paragraph_come_off_the_height_inside_them() {
    // The 600px is the border box: 570px of lines between 15px of padding.
    assert_eq!(
        long_paragraph_rect("display:block", "padding:15px"),
        (0.0, 0.0, 90.0, 600.0)
    );
    assert_eq!(
        long_paragraph_rect("display:block", "padding:15px;margin:15px"),
        (15.0, 15.0, 90.0, 570.0)
    );
    assert_eq!(
        long_paragraph_rect("display:block", "border:5px solid"),
        (0.0, 0.0, 60.0, 600.0)
    );
}

#[test]
fn a_short_vertical_paragraph_keeps_the_height_of_its_line() {
    let (mut doc, _, container) = vertical_block_under("display:block", "", "aa aa aa");
    lay_out(&mut doc);
    assert_eq!(absolute_rect(&doc, container), (0.0, 0.0, 10.0, 80.0));
}

#[test]
fn a_vertical_block_of_paragraphs_wraps_each_at_the_page_height() {
    let (mut doc, _, container) = vertical_block_under("display:block", "", "");
    let first = doc.append_element(Some(container), "p", Some("display:block;margin:0"));
    doc.append_text(first, long_text());
    let second = doc.append_element(Some(container), "p", Some("display:block;margin:0"));
    doc.append_text(second, long_text());
    lay_out(&mut doc);
    assert_eq!(absolute_rect(&doc, container), (0.0, 0.0, 100.0, 600.0));
    // The first paragraph is at the block start, the right.
    assert_eq!(absolute_rect(&doc, first), (50.0, 0.0, 50.0, 600.0));
    assert_eq!(absolute_rect(&doc, second), (0.0, 0.0, 50.0, 600.0));
}

#[test]
fn a_vertical_block_container_that_holds_no_text_takes_its_margins_off_the_height_once() {
    // 110 boxes of 10px, wrapped in a flex container, line up along the
    // height they are offered. The container is no paragraph root, so it is
    // laid out on transposed axes.
    let flex_boxes = |css: &str, parent_css: &str| {
        let (mut doc, _, container) = vertical_block_under(parent_css, css, "");
        let flex = doc.append_element(Some(container), "div", Some("display:flex;flex-wrap:wrap"));
        for _ in 0..110 {
            doc.append_element(
                Some(flex),
                "div",
                Some("display:block;width:10px;height:10px;flex:none"),
            );
        }
        lay_out(&mut doc);
        assert!(!doc.nodes[container].is_ifc_root());
        absolute_rect(&doc, container)
    };
    // 600px fit 60 of them in a line: 2 lines.
    assert_eq!(flex_boxes("", "display:block"), (0.0, 0.0, 20.0, 600.0));
    // 570px fit 57, and a third line would need 540px or less.
    assert_eq!(
        flex_boxes("margin:15px", "display:block"),
        (15.0, 15.0, 20.0, 570.0)
    );
    assert_eq!(
        flex_boxes("margin:20px", "display:block;height:300px"),
        (20.0, 20.0, 50.0, 260.0)
    );
}
