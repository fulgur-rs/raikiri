use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use taffy::{LengthPercentage, Style};

/// A document whose body (8px margin) holds a container styled
/// `container_css` with one child per entry of `children`, each a
/// `display:block` div styled with that entry. The part of an entry after
/// a `|` is the Ahem text of the child. Returns the rects of the container
/// and of its children.
fn layout(container_css: &str, children: &[&str]) -> Vec<(f32, f32, f32, f32)> {
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
            format!(
                "display:block;font-family:Ahem;font-size:10px;line-height:10px;{container_css}"
            )
            .as_str(),
        ),
    );
    let mut ids = vec![container];
    for css in children {
        let (css, text) = css.split_once('|').unwrap_or((css, ""));
        let child = doc.append_element(
            Some(container),
            "div",
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
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    ids.into_iter().map(|id| absolute_rect(&doc, id)).collect()
}

#[test]
fn vertical_rl_blocks_stack_from_right_to_left() {
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px",
        &["width:50px", "width:30px"],
    );
    assert_eq!(rects[0], (8.0, 8.0, 300.0, 200.0));
    // Each child fills the container's inline size, its physical height.
    assert_eq!(rects[1], (258.0, 8.0, 50.0, 200.0));
    assert_eq!(rects[2], (228.0, 8.0, 30.0, 200.0));
}

#[test]
fn vertical_lr_blocks_stack_from_left_to_right() {
    let rects = layout(
        "writing-mode:vertical-lr;width:300px;height:200px",
        &["width:50px", "width:30px"],
    );
    assert_eq!(rects[1], (8.0, 8.0, 50.0, 200.0));
    assert_eq!(rects[2], (58.0, 8.0, 30.0, 200.0));
}

#[test]
fn vertical_rl_block_margins_follow_the_physical_sides() {
    // The block-start margin is the right one in `vertical-rl`; adjacent
    // block margins collapse (10px and 20px give 20px); inline margins
    // shorten the physical height.
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px",
        &[
            "width:50px;margin:5px 10px 15px 0",
            "width:30px;margin-right:20px",
        ],
    );
    assert_eq!(rects[1], (248.0, 13.0, 50.0, 180.0));
    assert_eq!(rects[2], (198.0, 8.0, 30.0, 200.0));
}

#[test]
fn vertical_rl_container_padding_and_border_offset_its_children() {
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px;box-sizing:border-box;\
         padding:1px 2px 3px 4px;border:solid;border-width:5px 6px 7px 8px",
        &["width:50px"],
    );
    // Right edge: 8 + 300 - 6 - 2 = 300; top: 8 + 5 + 1 = 14; height:
    // 200 - 5 - 1 - 3 - 7 = 184.
    assert_eq!(rects[1], (250.0, 14.0, 50.0, 184.0));
}

#[test]
fn vertical_rl_auto_width_is_the_extent_of_its_blocks() {
    let rects = layout(
        "writing-mode:vertical-rl;height:200px",
        &["width:50px", "width:30px"],
    );
    assert_eq!(rects[0], (8.0, 8.0, 80.0, 200.0));
    assert_eq!(rects[1], (38.0, 8.0, 50.0, 200.0));
    assert_eq!(rects[2], (8.0, 8.0, 30.0, 200.0));
}

#[test]
fn vertical_auto_height_under_a_horizontal_body_fits_its_content() {
    // An orthogonal flow takes the fit-content inline size: the longest
    // child here, 50px, not the body's 584px.
    let rects = layout(
        "writing-mode:vertical-rl",
        &["width:20px;height:50px", "width:20px;height:30px"],
    );
    assert_eq!(rects[0], (8.0, 8.0, 40.0, 50.0));
    assert_eq!(rects[1], (28.0, 8.0, 20.0, 50.0));
    assert_eq!(rects[2], (8.0, 8.0, 20.0, 30.0));
}

#[test]
fn horizontal_child_of_a_vertical_container_is_as_tall_as_its_content() {
    // The horizontal child is an orthogonal flow: its physical height is
    // its block size, from its 10px-tall child, not the container's inline
    // size it would be stretched to.
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
        Some("display:block;writing-mode:vertical-rl;width:300px;height:200px"),
    );
    let child = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;writing-mode:horizontal-tb;width:40px"),
    );
    doc.append_element(
        Some(child),
        "div",
        Style::default(),
        Some("display:block;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child), (268.0, 8.0, 40.0, 10.0));
}

#[test]
fn vertical_rl_body_stacks_its_paragraphs_from_the_right() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some(
            "display:block;margin:8px;writing-mode:vertical-rl;font-family:Ahem;\
             font-size:10px;line-height:10px",
        ),
    );
    let paragraphs: Vec<usize> = ["aa bb", "cc"]
        .into_iter()
        .map(|text| {
            let p = doc.append_element(
                Some(body),
                "p",
                Style::default(),
                Some("display:block;margin:0 0 0 10px"),
            );
            doc.append_text(p, text);
            p
        })
        .collect();
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let body = absolute_rect(&doc, body);
    let first = absolute_rect(&doc, paragraphs[0]);
    let second = absolute_rect(&doc, paragraphs[1]);
    // One 10px column each, from the right edge of the body's content box
    // (inside its 8px margin) leftwards, 10px apart from the left margin of
    // the first.
    assert_eq!(body, (0.0, 0.0, 800.0, 600.0));
    assert_eq!((first.0, first.1, first.2), (782.0, 8.0, 10.0));
    assert_eq!((second.0, second.1, second.2), (762.0, 8.0, 10.0));
}

#[test]
fn vertical_auto_height_never_shrinks_below_its_content() {
    // Content taller than the space under the body keeps its height.
    let rects = layout("writing-mode:vertical-rl", &["width:20px;height:1000px"]);
    assert_eq!(rects[0], (8.0, 8.0, 20.0, 1000.0));
}

#[test]
fn horizontal_child_of_a_vertical_container_fills_its_width() {
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px;padding:0 5px",
        &["writing-mode:horizontal-tb;height:10px;margin:0 7px"],
    );
    // The 300px content box less the child's 7px margins.
    assert_eq!(rects[1], (20.0, 8.0, 286.0, 10.0));
}

#[test]
fn horizontal_child_of_a_vertical_container_keeps_its_max_width() {
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px",
        &["writing-mode:horizontal-tb;height:10px;max-width:100px"],
    );
    // Placed on the block-start (right) edge.
    assert_eq!(rects[1], (208.0, 8.0, 100.0, 10.0));
}

#[test]
fn horizontal_child_margins_resolve_against_the_inline_size() {
    // 10% of the container's 400px inline size, its physical height.
    let rects = layout(
        "writing-mode:vertical-rl;width:200px;height:400px",
        &["writing-mode:horizontal-tb;height:10px;margin:0 10%"],
    );
    assert_eq!(rects[1], (48.0, 8.0, 120.0, 10.0));
}

#[test]
fn horizontal_child_of_a_vertical_container_keeps_its_min_width() {
    // `min-width` and `max-width` are content-box sizes here: the 10px
    // padding adds to them.
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px",
        &["writing-mode:horizontal-tb;height:10px;padding:0 5px;max-width:20px;min-width:100px"],
    );
    assert_eq!(rects[1], (198.0, 8.0, 110.0, 10.0));
}

#[test]
fn border_box_child_max_width_includes_its_padding() {
    let rects = layout(
        "writing-mode:vertical-rl;width:300px;height:200px",
        &[
            "writing-mode:horizontal-tb;height:10px;padding:0 5px;box-sizing:border-box;max-width:100px",
        ],
    );
    assert_eq!(rects[1], (208.0, 8.0, 100.0, 10.0));
}

#[test]
fn horizontal_child_of_an_auto_width_vertical_container_keeps_its_width() {
    // Without a definite physical width there is no space to fill.
    let rects = layout(
        "writing-mode:vertical-rl;height:200px",
        &["writing-mode:horizontal-tb;width:40px;height:10px"],
    );
    assert_eq!(rects[0], (8.0, 8.0, 40.0, 200.0));
    assert_eq!(rects[1], (8.0, 8.0, 40.0, 10.0));
}

#[test]
fn vertical_flex_item_stretches_to_its_flex_line() {
    // The flex container measures the vertical item's physical height, its
    // transposed width, before stretching it.
    let rects = layout(
        "display:flex;height:100px",
        &["writing-mode:vertical-rl;width:30px"],
    );
    assert_eq!(rects[1], (8.0, 8.0, 30.0, 100.0));
}

/// Lays out a body styled `body_css` holding one element per entry of
/// `children` (`tag`, `css`, `text`) and returns the document and the ids
/// of the body and its children.
fn layout_body(body_css: &str, children: &[(&str, &str, &str)]) -> (Document, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some(format!("font-family:Ahem;font-size:10px;line-height:10px;{body_css}").as_str()),
    );
    let mut ids = vec![body];
    for &(tag, css, text) in children {
        if tag == "text" {
            doc.append_text(body, text);
            continue;
        }
        let child = doc.append_element(Some(body), tag, Style::default(), Some(css));
        if !text.is_empty() {
            doc.append_text(child, text);
        }
        ids.push(child);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (doc, ids)
}

#[test]
fn vertical_body_of_text_keeps_its_bottom_margin() {
    // The body's lines run down the page: its top and bottom margins are
    // inline margins, carried as padding like the left and right ones.
    let (doc, ids) = layout_body(
        "display:block;margin:8px;writing-mode:vertical-rl",
        &[("text", "", "aa bb")],
    );
    let eight = LengthPercentage::length(8.0);
    let padding = doc.nodes[ids[0]].style.padding;
    assert_eq!((padding.top, padding.bottom), (eight, eight));
}

#[test]
fn vertical_flex_body_keeps_its_vertical_margins() {
    let (doc, ids) = layout_body(
        "display:flex;margin:8px;writing-mode:vertical-rl",
        &[("div", "width:20px;height:10px", "")],
    );
    // A flex body lays its items out on horizontal axes, below its top
    // margin rather than below padding in place of it.
    let padding = doc.nodes[ids[0]].style.padding;
    let zero = LengthPercentage::length(0.0);
    assert_eq!((padding.top, padding.bottom), (zero, zero));
    assert_eq!(absolute_rect(&doc, ids[1]), (8.0, 8.0, 20.0, 10.0));
}

#[test]
fn contents_element_with_columns_keeps_its_children_vertical() {
    // A `display: contents` element is no multicol container, so its child
    // keeps the vertical mapping of `margin-block-start`.
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
        Some("display:block;writing-mode:vertical-rl;width:300px;height:200px"),
    );
    let contents = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:contents;column-count:2"),
    );
    let child = doc.append_element(
        Some(contents),
        "div",
        Style::default(),
        Some("display:block;width:30px;margin-block-start:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child), (268.0, 8.0, 30.0, 200.0));
}

#[test]
fn contents_child_of_a_vertical_flex_container_keeps_the_horizontal_mapping() {
    // The flex container places the child of its `display: contents` child
    // on horizontal axes, so `margin-block-start` maps to the top margin as
    // it does for a direct flex item.
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
        Some("display:flex;writing-mode:vertical-rl;width:300px;height:200px"),
    );
    let contents = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:contents"),
    );
    let child = doc.append_element(
        Some(contents),
        "div",
        Style::default(),
        Some("display:block;width:30px;height:20px;margin-block-start:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child), (8.0, 18.0, 30.0, 20.0));
}
