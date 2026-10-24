mod frame_tests;
mod scoped_override_tests;
mod view_tests;

use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use taffy::LengthPercentage;

/// A document whose body (8px margin) holds a container styled
/// `container_css` with one child per entry of `children`, each a
/// `display:block` div styled with that entry. The part of an entry after
/// a `|` is the Ahem text of the child. Returns the rects of the container
/// and of its children.
fn layout(container_css: &str, children: &[&str]) -> Vec<(f32, f32, f32, f32)> {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
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
fn vertical_sibling_block_margins_collapse_to_the_larger_margin() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        for (end_margin, start_margin, collapsed_margin) in
            [("10px", "20px", 20.0), ("10px", "-5px", 5.0)]
        {
            let first = format!("width:10px;height:10px;margin-block-end:{end_margin}");
            let second = format!("width:10px;height:10px;margin-block-start:{start_margin}");
            let rects = layout(
                &format!("writing-mode:{writing_mode};width:100px;height:100px"),
                &[&first, &second],
            );
            let distance = if writing_mode == "vertical-rl" {
                rects[1].0 - rects[2].0
            } else {
                rects[2].0 - rects[1].0
            };
            assert_eq!(
                distance,
                10.0 + collapsed_margin,
                "{writing_mode} end={end_margin} start={start_margin}: {rects:?}"
            );
        }
    }
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;writing-mode:vertical-rl;width:300px;height:200px"),
    );
    let child = doc.append_element(
        Some(container),
        "div",
        Some("display:block;writing-mode:horizontal-tb;width:40px"),
    );
    doc.append_element(Some(child), "div", Some("display:block;height:10px"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child), (268.0, 8.0, 40.0, 10.0));
}

/// A vertical-rl body of two paragraphs under a root styled `html_css`:
/// the rects of the body and of the paragraphs.
fn stacked_paragraphs(html_css: &str) -> [(f32, f32, f32, f32); 3] {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some(html_css));
    let body = doc.append_element(
        Some(html),
        "body",
        Some(
            "display:block;margin:8px;writing-mode:vertical-rl;font-family:Ahem;\
             font-size:10px;line-height:10px",
        ),
    );
    let paragraphs: Vec<usize> = ["aa bb", "cc"]
        .into_iter()
        .map(|text| {
            let p = doc.append_element(Some(body), "p", Some("display:block;margin:0 0 0 10px"));
            doc.append_text(p, text);
            p
        })
        .collect();
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    [
        absolute_rect(&doc, body),
        absolute_rect(&doc, paragraphs[0]),
        absolute_rect(&doc, paragraphs[1]),
    ]
}

#[test]
fn vertical_rl_body_stacks_its_paragraphs_from_the_right() {
    // The body's vertical writing mode is the root's, so the root and the
    // body run on the same axes: the body fills the page height and sits at
    // the right edge, its two 10px columns right to left.
    let [body, first, second] = stacked_paragraphs("display:block");
    assert_eq!(body, (762.0, 8.0, 30.0, 584.0));
    assert_eq!((first.0, first.1, first.2), (782.0, 8.0, 10.0));
    assert_eq!((second.0, second.1, second.2), (762.0, 8.0, 10.0));
}

#[test]
fn orthogonal_vertical_rl_body_stacks_its_paragraphs_from_the_right() {
    // Containment on the root keeps the body's writing mode from it, which
    // leaves the body an orthogonal flow.
    let [body, first, second] = stacked_paragraphs("display:block;contain:layout");
    // The orthogonal auto-sized body uses its max-content inline size (50px).
    // Its two 10px columns and their left margins occupy 40px across the page.
    assert_eq!(body, (8.0, 8.0, 40.0, 50.0));
    assert_eq!((first.0, first.1, first.2), (38.0, 8.0, 10.0));
    assert_eq!((second.0, second.1, second.2), (18.0, 8.0, 10.0));
}

/// A root styled `html_css` that holds a `body` styled `body_css` with one
/// 10px line, and after it a `display:block` child of its own with one line
/// of 110px, as script can add one. Returns the rects of the body and of that
/// child.
fn body_and_a_second_child_of_the_root(
    html_css: &str,
    body_css: &str,
) -> [(f32, f32, f32, f32); 2] {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some(html_css));
    let font = "font-family:Ahem;font-size:10px;line-height:10px";
    let body = doc.append_element(
        Some(html),
        "body",
        Some(format!("display:block;margin:8px;{font};{body_css}").as_str()),
    );
    let line = doc.append_element(Some(body), "div", Some("display:block"));
    doc.append_text(line, "x");
    let sibling = doc.append_element(
        Some(html),
        "div",
        Some(format!("display:block;margin:0;{font}").as_str()),
    );
    doc.append_text(sibling, "abcde fghij");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    [absolute_rect(&doc, body), absolute_rect(&doc, sibling)]
}

#[test]
fn another_child_of_the_root_inherits_the_writing_mode_the_root_computed_to() {
    // The body's vertical mode is the used one of the root, which its other
    // children do not inherit: this one stays horizontal, an orthogonal flow
    // beside the body in the block direction of the page.
    let [body, sibling] =
        body_and_a_second_child_of_the_root("display:block", "writing-mode:vertical-rl");
    assert_eq!(body, (782.0, 8.0, 10.0, 584.0));
    assert_eq!(sibling, (664.0, 0.0, 110.0, 10.0));
    let [body, sibling] =
        body_and_a_second_child_of_the_root("display:block", "writing-mode:vertical-lr");
    assert_eq!(body, (8.0, 8.0, 10.0, 584.0));
    assert_eq!(sibling, (26.0, 0.0, 110.0, 10.0));
}

#[test]
fn another_child_of_a_vertical_root_stays_vertical_under_a_horizontal_body() {
    // The body's horizontal mode is the used one of the root, which its other
    // children do not inherit: this one is vertical, as the root computed to.
    let [body, sibling] = body_and_a_second_child_of_the_root(
        "display:block;writing-mode:vertical-lr",
        "writing-mode:horizontal-tb",
    );
    assert_eq!(body, (8.0, 8.0, 784.0, 10.0));
    assert_eq!(sibling, (0.0, 26.0, 10.0, 110.0));
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
    layout_body_in("display:block", body_css, children)
}

/// [`layout_body`] with the root element styled `html_css`.
fn layout_body_in(
    html_css: &str,
    body_css: &str,
    children: &[(&str, &str, &str)],
) -> (Document, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some(html_css));
    let body = doc.append_element(
        Some(html),
        "body",
        Some(format!("font-family:Ahem;font-size:10px;line-height:10px;{body_css}").as_str()),
    );
    let mut ids = vec![body];
    for &(tag, css, text) in children {
        if tag == "text" {
            doc.append_text(body, text);
            continue;
        }
        let child = doc.append_element(Some(body), tag, Some(css));
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

fn vertical_document(writing_mode: &str, width: &str, height: &str) -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some(
            format!(
                "display:block;margin:0;writing-mode:{writing_mode};width:{width};height:{height};\
                 font-family:Ahem;font-size:10px;line-height:10px"
            )
            .as_str(),
        ),
    );
    (doc, body)
}

fn layout_vertical_document(doc: &mut Document) {
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(doc);
    let cascade = raikiri_style::cascade(doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(doc), &cascade, page_box_800x600()).expect("layout");
}

#[test]
fn same_mode_vertical_block_margins_collapse_through_a_child() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, body) = vertical_document(writing_mode, "200px", "40px");
        let root = doc.append_element(
            Some(body),
            "div",
            Some("display:flow-root;width:200px;height:40px"),
        );
        let outer = doc.append_element(
            Some(root),
            "div",
            Some("display:block;height:40px;margin-block-start:10px"),
        );
        let inner = doc.append_element(
            Some(outer),
            "p",
            Some("display:block;width:20px;margin-block:20px"),
        );
        doc.append_text(inner, "a");

        layout_vertical_document(&mut doc);

        let outer = absolute_rect(&doc, outer);
        let inner = absolute_rect(&doc, inner);
        if writing_mode == "vertical-rl" {
            assert_eq!(
                inner.0 + inner.2,
                outer.0 + outer.2,
                "{writing_mode}: {outer:?} {inner:?}"
            );
            assert_eq!(inner.0, outer.0, "{writing_mode}: {outer:?} {inner:?}");
        } else {
            assert_eq!(inner.0, outer.0, "{writing_mode}: {outer:?} {inner:?}");
            assert_eq!(
                inner.0 + inner.2,
                outer.0 + outer.2,
                "{writing_mode}: {outer:?} {inner:?}"
            );
        }
    }
}

#[test]
fn vertical_content_min_width_prevents_the_last_child_margin_from_escaping() {
    for (box_sizing, expected_width) in [
        ("min-width:50px;padding-right:10px", 60.0),
        (
            "box-sizing:border-box;min-width:60px;padding-right:10px",
            60.0,
        ),
    ] {
        let (mut doc, body) = vertical_document("vertical-rl", "200px", "40px");
        let root = doc.append_element(
            Some(body),
            "div",
            Some("display:flow-root;width:200px;height:40px"),
        );
        let wrapper = doc.append_element(
            Some(root),
            "div",
            Some(format!("display:block;width:auto;height:40px;{box_sizing}").as_str()),
        );
        doc.append_text(wrapper, "a");
        doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;width:20px;height:10px;margin-block-end:20px"),
        );
        let sibling = doc.append_element(
            Some(root),
            "div",
            Some("display:block;width:10px;height:10px"),
        );

        layout_vertical_document(&mut doc);

        let wrapper = absolute_rect(&doc, wrapper);
        let sibling = absolute_rect(&doc, sibling);
        assert_eq!(wrapper.2, expected_width, "{box_sizing}");
        assert_eq!(
            wrapper.0 - (sibling.0 + sibling.2),
            0.0,
            "{box_sizing}: {wrapper:?} {sibling:?}"
        );
    }
}

#[test]
fn same_mode_vertical_descendant_clear_sees_an_ancestor_float() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, body) = vertical_document(writing_mode, "100px", "40px");
        let root = doc.append_element(
            Some(body),
            "div",
            Some("display:flow-root;width:100px;height:40px"),
        );
        let float = doc.append_element(
            Some(root),
            "div",
            Some("display:block;float:left;width:10px;height:20px"),
        );
        let wrapper = doc.append_element(
            Some(root),
            "div",
            Some("display:block;width:100px;height:40px"),
        );
        let cleared = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;clear:both;width:20px;height:20px"),
        );

        layout_vertical_document(&mut doc);

        let float = absolute_rect(&doc, float);
        let cleared = absolute_rect(&doc, cleared);
        if writing_mode == "vertical-rl" {
            assert!(
                cleared.0 + cleared.2 <= float.0,
                "{writing_mode}: {float:?} {cleared:?}"
            );
        } else {
            assert!(
                float.0 + float.2 <= cleared.0,
                "{writing_mode}: {float:?} {cleared:?}"
            );
        }
    }
}

#[test]
fn matching_vertical_and_sideways_block_axes_share_floats() {
    for (parent_mode, child_mode) in [
        ("vertical-rl", "sideways-rl"),
        ("vertical-lr", "sideways-lr"),
    ] {
        let (mut doc, body) = vertical_document(parent_mode, "100px", "40px");
        let root = doc.append_element(
            Some(body),
            "div",
            Some("display:flow-root;width:100px;height:40px"),
        );
        let float = doc.append_element(
            Some(root),
            "div",
            Some("display:block;float:left;width:10px;height:20px"),
        );
        let wrapper = doc.append_element(
            Some(root),
            "div",
            Some(
                format!("display:block;writing-mode:{child_mode};width:100px;height:40px").as_str(),
            ),
        );
        let cleared = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;clear:both;width:20px;height:20px"),
        );

        layout_vertical_document(&mut doc);

        let float = absolute_rect(&doc, float);
        let cleared = absolute_rect(&doc, cleared);
        if parent_mode == "vertical-rl" {
            assert!(
                cleared.0 + cleared.2 <= float.0,
                "parent={parent_mode}, child={child_mode}: {float:?} {cleared:?}"
            );
        } else {
            assert!(
                float.0 + float.2 <= cleared.0,
                "parent={parent_mode}, child={child_mode}: {float:?} {cleared:?}"
            );
        }
    }
}

#[test]
fn vertical_inline_content_shares_ancestor_floats() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, body) = vertical_document(writing_mode, "100px", "40px");
        let root = doc.append_element(
            Some(body),
            "div",
            Some("display:flow-root;width:100px;height:40px"),
        );
        let float = doc.append_element(
            Some(root),
            "div",
            Some("display:block;float:right;width:10px;height:20px"),
        );
        let paragraph = doc.append_element(Some(root), "p", Some("display:block;margin:0"));
        doc.append_text(paragraph, "a a a a");

        layout_vertical_document(&mut doc);

        let lines = doc.nodes[paragraph]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .expect("paragraph lines");
        assert!(
            lines.beside_floats,
            "{writing_mode} lines should use the shared float slots: float={:?}, paragraph={:?}, line_offsets={:?}, line_sizes={:?}",
            absolute_rect(&doc, float),
            absolute_rect(&doc, paragraph),
            lines
                .lines
                .iter()
                .map(|line| line.block_offset())
                .collect::<Vec<_>>(),
            lines
                .lines
                .iter()
                .map(|line| line.block_size())
                .collect::<Vec<_>>(),
        );
    }
}

#[test]
fn vertical_body_of_text_keeps_its_bottom_margin() {
    // The body's inline margins remain margins outside its own border box.
    let (doc, ids) = layout_body(
        "display:block;margin:8px;writing-mode:vertical-rl",
        &[("text", "", "aa bb")],
    );
    let layout = doc.nodes[ids[0]].layout;
    assert_eq!((layout.margin.top, layout.margin.bottom), (8.0, 8.0));
    assert_eq!((layout.padding.top, layout.padding.bottom), (0.0, 0.0));
}

/// The item of a vertical-rl flex body under a root styled `html_css`.
fn flex_body_item(html_css: &str) -> (f32, f32, f32, f32) {
    let (mut doc, ids) = layout_body_in(
        html_css,
        "display:flex;margin:8px;writing-mode:vertical-rl",
        &[("div", "width:20px;height:10px", "")],
    );
    let cascade = crate::layout::test_support::cascaded(&doc);
    let padding = crate::layout::test_support::bridged_style(&mut doc, &cascade, ids[0]).padding;
    let zero = LengthPercentage::length(0.0);
    assert_eq!((padding.top, padding.bottom), (zero, zero));
    absolute_rect(&doc, ids[1])
}

#[test]
fn vertical_flex_body_keeps_its_vertical_margins() {
    // A flex body lays its items out below its top margin rather than below
    // padding in place of it. It sits at the right edge, the block start of
    // the vertical root it makes of the page.
    assert_eq!(flex_body_item("display:block"), (772.0, 8.0, 20.0, 10.0));
}

#[test]
fn orthogonal_vertical_flex_body_keeps_its_vertical_margins() {
    // Without the propagation the flex body lays its items out on horizontal
    // axes, below its top margin.
    assert_eq!(
        flex_body_item("display:block;contain:layout"),
        (8.0, 8.0, 20.0, 10.0)
    );
}

#[test]
fn contents_element_with_columns_keeps_its_children_vertical() {
    // A `display: contents` element is no multicol container, so its child
    // keeps the vertical mapping of `margin-block-start`.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;writing-mode:vertical-rl;width:300px;height:200px"),
    );
    let contents = doc.append_element(
        Some(container),
        "div",
        Some("display:contents;column-count:2"),
    );
    let child = doc.append_element(
        Some(contents),
        "div",
        Some("display:block;width:30px;margin-block-start:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child), (268.0, 8.0, 30.0, 200.0));
}

#[test]
fn contents_child_of_a_vertical_flex_container_follows_the_container_mapping() {
    // The flex container places the child of its `display: contents` child
    // on the axes of its writing mode, so `margin-block-start` maps to the
    // right margin as it does for a direct flex item.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;writing-mode:vertical-rl;width:300px;height:200px"),
    );
    let contents = doc.append_element(Some(container), "div", Some("display:contents"));
    let child = doc.append_element(
        Some(contents),
        "div",
        Some("display:block;width:30px;height:20px;margin-block-start:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child), (268.0, 8.0, 30.0, 20.0));
}

#[test]
fn vertical_body_percentages_follow_the_root_inline_size() {
    // Percentages of the body's padding resolve against the inline size of
    // its containing block, the root: the page height for a vertical root,
    // the page width for a horizontal one.
    let body = "display:block;margin:0;padding-top:10%;writing-mode:vertical-rl";
    let children = [("text", "", "aa bb")];
    let (doc, ids) = layout_body_in("display:block;writing-mode:vertical-rl", body, &children);
    assert_eq!(doc.nodes[ids[0]].layout.padding.top, 60.0);
    // The body's writing mode is the root's even when the root declares none.
    let (doc, ids) = layout_body_in("display:block", body, &children);
    assert_eq!(doc.nodes[ids[0]].layout.padding.top, 60.0);
    // Containment on the root leaves the body an orthogonal flow of a
    // horizontal root: the page width.
    let (doc, ids) = layout_body_in("display:block;contain:layout", body, &children);
    assert_eq!(doc.nodes[ids[0]].layout.padding.top, 80.0);
}

#[test]
fn horizontal_child_padding_percentages_follow_the_container_inline_size() {
    // The containing block of a horizontal child is the vertical container,
    // whose inline size is its physical height: `padding-left: 10%` of a
    // 400px tall container is 40px.
    let rects = layout(
        "writing-mode:vertical-rl;width:200px;height:400px",
        &["writing-mode:horizontal-tb;width:20px;height:10px;padding-left:10%"],
    );
    assert_eq!(rects[1].2, 60.0);
}

#[test]
fn vertical_child_padding_percentages_follow_the_container_inline_size() {
    // The block-axis padding of a vertical child is a percentage of the
    // container's inline size too, its physical height.
    let rects = layout(
        "writing-mode:vertical-rl;width:200px;height:400px",
        &["width:20px;padding-left:10%"],
    );
    assert_eq!(rects[1].2, 60.0);
}

#[test]
fn an_empty_vertical_child_resolves_percentage_padding_against_the_container_inline_size() {
    // A box without children is measured as a leaf, which takes the
    // percentages of its padding from the width of its parent unless it is
    // given the inline size of the container: 10% of 400px is 40px on every
    // side.
    let rects = layout(
        "writing-mode:vertical-rl;width:200px;height:400px",
        &["width:20px;height:10px;padding:10%"],
    );
    assert_eq!((rects[1].2, rects[1].3), (100.0, 90.0));
}

#[test]
fn vertical_container_padding_percentages_follow_a_horizontal_parent() {
    // The containing block of the vertical container is the horizontal
    // body, 784px wide: `padding-right: 25%` is 196px, and the first child
    // stacks against it.
    let rects = layout(
        "writing-mode:vertical-rl;width:200px;height:100px;padding-right:25%",
        &["width:20px"],
    );
    assert_eq!(rects[0].2, 396.0);
    assert_eq!(rects[1].0, 188.0);
}

/// The container and its child under a vertical-rl body and a horizontal
/// `display: contents` parent, below a root styled `html_css`.
fn container_below_a_contents_parent(html_css: &str) -> [(f32, f32, f32, f32); 2] {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some(html_css));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;writing-mode:vertical-rl"),
    );
    let contents = doc.append_element(
        Some(body),
        "div",
        Some("display:contents;writing-mode:horizontal-tb"),
    );
    let container = doc.append_element(
        Some(contents),
        "div",
        Some("display:block;writing-mode:vertical-rl;width:100px;height:200px;padding-right:10%"),
    );
    let child = doc.append_element(Some(container), "div", Some("display:block;width:20px"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    [absolute_rect(&doc, container), absolute_rect(&doc, child)]
}

#[test]
fn vertical_container_padding_looks_through_a_contents_parent() {
    // A horizontal `display: contents` parent has no box: the containing
    // block is the vertical body around it. The body is as tall as the page,
    // the inline size of the vertical root it makes of it, so
    // `padding-right: 10%` is 60px.
    let [container, child] = container_below_a_contents_parent("display:block");
    assert_eq!(container.2, 160.0);
    assert_eq!(child.0, 720.0);
}

#[test]
fn orthogonal_vertical_container_padding_looks_through_a_contents_parent() {
    // Containment on the root leaves the body an orthogonal flow, whose auto
    // inline size fits the 200px-tall child: `padding-right: 10%` is 20px.
    let [container, child] = container_below_a_contents_parent("display:block;contain:layout");
    assert_eq!(container.2, 120.0);
    assert_eq!(child.0, 80.0);
}

#[test]
fn vertical_flex_item_keeps_its_padding_percentages_while_measured() {
    // A row flex container measures its items' flex base sizes without a
    // main-axis parent size; `padding-right: 10%` of the 300px wide
    // container is still 30px.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;width:300px;height:100px"),
    );
    let item = doc.append_element(
        Some(flex),
        "div",
        Some("display:block;writing-mode:vertical-rl;padding-right:10%"),
    );
    doc.append_element(Some(item), "div", Some("display:block;width:20px"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, item).2, 50.0);
}
