use super::*;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

/// `html(html_css) > body(body_css) > div(child_css)`, cascaded. Returns the
/// document, its cascade, the body and the child.
fn body_doc(
    html_css: &str,
    body_css: &str,
    child_css: &str,
) -> (Document, CascadeResult, usize, usize) {
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
        Some(format!("display:block;{body_css}").as_str()),
    );
    let child = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(format!("display:block;height:10px;{child_css}").as_str()),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    (doc, cascade, body, child)
}

fn laid_out(html_css: &str, body_css: &str, child_css: &str) -> (Document, usize, usize) {
    let (mut doc, cascade, body, child) = body_doc(html_css, body_css, child_css);
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    (doc, body, child)
}

#[test]
fn the_body_margin_collapses_with_the_first_child_margin() {
    let (doc, body, child) = laid_out("", "margin:20px", "margin-top:30px");
    assert_eq!(doc.body_block_start_margin(), 30.0);
    assert_eq!(doc.nodes[child].unrounded_layout.location.y, 30.0);
    assert_eq!(doc.nodes[body].unrounded_layout.location.y, 0.0);
}

#[test]
fn boxes_that_keep_the_margins_apart_add_them() {
    for body_css in [
        "margin:20px;display:flex",
        "margin:20px;display:flow-root",
        "margin:20px;float:left",
        "margin:20px;position:absolute",
    ] {
        let (doc, _, child) = laid_out("", body_css, "margin-top:30px");
        assert_eq!(doc.body_block_start_margin(), 20.0, "{body_css}");
        assert_eq!(
            doc.nodes[child].unrounded_layout.location.y, 50.0,
            "{body_css}"
        );
    }
}

#[test]
fn body_overflow_is_visible_only_when_it_propagates_to_the_viewport() {
    let (doc, body, _) = laid_out("", "margin:8px;overflow:hidden", "margin-top:30px");
    assert_eq!(doc.nodes[body].style.overflow.y, TaffyOverflow::Visible);
    assert_eq!(doc.body_block_start_margin(), 30.0);
    let (doc, body, _) = laid_out(
        "overflow:hidden",
        "margin:8px;overflow:hidden",
        "margin-top:30px",
    );
    assert_ne!(doc.nodes[body].style.overflow.y, TaffyOverflow::Visible);
    assert_eq!(doc.body_block_start_margin(), 8.0);
}

#[test]
fn a_body_outside_html_keeps_its_overflow() {
    let mut doc = Document::new();
    let body = doc.append_element(
        Some(0),
        "body",
        Style::default(),
        Some("display:block;overflow:hidden"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    let before = doc.nodes[body].style.overflow;
    propagate_body_overflow_to_viewport(&mut doc, &cascade, body);
    assert_eq!(doc.nodes[body].style.overflow, before);
}

#[test]
fn top_padding_restores_the_height_from_the_captured_one() {
    let (mut doc, cascade, body, _) = body_doc("", "padding-top:5px", "");
    crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
    doc.nodes[body].style.size.height = Dimension::length(100.0);
    let captured = BodyTopPadding::capture(&doc, body, 800.0);
    assert_eq!(captured.apply(&mut doc, body, 8.0), 8.0);
    assert_eq!(
        doc.nodes[body].style.padding.top,
        LengthPercentage::length(13.0)
    );
    assert_eq!(doc.nodes[body].style.size.height, Dimension::length(92.0));
    // A second offset replaces the first instead of adding to it, and a
    // negative one is clamped.
    assert_eq!(captured.apply(&mut doc, body, -3.0), 0.0);
    assert_eq!(
        doc.nodes[body].style.padding.top,
        LengthPercentage::length(5.0)
    );
    assert_eq!(doc.nodes[body].style.size.height, Dimension::length(100.0));
    assert_eq!(captured.apply(&mut doc, body, 500.0), 500.0);
    assert_eq!(doc.nodes[body].style.size.height, Dimension::length(0.0));
    // With `border-box` the height already holds the padding.
    doc.nodes[body].style.box_sizing = TaffyBoxSizing::BorderBox;
    doc.nodes[body].style.size.height = Dimension::length(100.0);
    let captured = BodyTopPadding::capture(&doc, body, 800.0);
    captured.apply(&mut doc, body, 8.0);
    assert_eq!(doc.nodes[body].style.size.height, Dimension::length(100.0));
}

#[test]
fn shifting_the_body_content_leaves_inset_positioned_boxes() {
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", Style::default(), None::<&str>);
    let flow = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let inset = doc.append_element(
        Some(body),
        "div",
        Style {
            position: TaffyPosition::Absolute,
            inset: Rect {
                top: LengthPercentageAuto::length(0.0),
                ..Rect::auto()
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let bottom = doc.append_element(
        Some(body),
        "div",
        Style {
            position: TaffyPosition::Absolute,
            inset: Rect {
                bottom: LengthPercentageAuto::length(0.0),
                ..Rect::auto()
            },
            ..Style::default()
        },
        None::<&str>,
    );
    let static_position = doc.append_element(
        Some(body),
        "div",
        Style {
            position: TaffyPosition::Absolute,
            ..Style::default()
        },
        None::<&str>,
    );
    doc.mark_in_document_flags();
    shift_body_content(&mut doc, body, 0.0);
    shift_body_content(&mut doc, body, f32::NAN);
    assert_eq!(doc.nodes[flow].unrounded_layout.location.y, 0.0);
    shift_body_content(&mut doc, body, 8.0);
    assert_eq!(doc.nodes[flow].unrounded_layout.location.y, 8.0);
    assert_eq!(doc.nodes[inset].unrounded_layout.location.y, 0.0);
    assert_eq!(doc.nodes[bottom].unrounded_layout.location.y, 0.0);
    assert_eq!(doc.nodes[static_position].unrounded_layout.location.y, 8.0);
}

#[test]
fn a_right_to_left_body_root_is_placed_at_the_start_of_the_inline_axis() {
    let (doc, body, _) = laid_out("", "direction:rtl;max-width:300px", "");
    let layout = doc.nodes[body].unrounded_layout;
    assert_eq!(layout.location.x + layout.size.width, 800.0);
}

#[test]
fn an_auto_body_margin_is_zero() {
    let (doc, _, child) = laid_out("", "margin-top:auto", "margin-top:30px");
    assert_eq!(doc.body_block_start_margin(), 30.0);
    assert_eq!(doc.nodes[child].unrounded_layout.location.y, 30.0);
}

#[test]
fn the_body_root_honours_crossed_min_and_max_heights_and_scrollbars() {
    let (doc, body, _) = laid_out(
        "overflow:hidden",
        "min-height:200px;max-height:100px;overflow:scroll",
        "",
    );
    let layout = doc.nodes[body].unrounded_layout;
    assert_eq!(layout.size.height, 200.0);
    assert!(layout.scrollbar_size.width >= 0.0 && layout.scrollbar_size.height >= 0.0);
}
