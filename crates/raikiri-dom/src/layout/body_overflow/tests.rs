use super::*;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
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
    assert_eq!(absolute_rect(&doc, child).1, 30.0);
    assert_eq!(doc.nodes[child].unrounded_layout.location.y, 0.0);
    assert_eq!(absolute_rect(&doc, body).1, 30.0);
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
        assert_eq!(absolute_rect(&doc, child).1, 50.0, "{body_css}");
    }
}

#[test]
fn body_overflow_is_visible_only_when_it_propagates_to_the_viewport() {
    let (doc, body, _) = laid_out("", "margin:8px;overflow:hidden", "margin-top:30px");
    assert_eq!(
        doc.layout_node_with_style(body).unwrap().style.overflow.y,
        TaffyOverflow::Visible
    );
    assert_eq!(absolute_rect(&doc, body).1, 30.0);
    let (doc, body, _) = laid_out(
        "overflow:hidden",
        "margin:8px;overflow:hidden",
        "margin-top:30px",
    );
    assert_ne!(
        doc.layout_node_with_style(body).unwrap().style.overflow.y,
        TaffyOverflow::Visible
    );
    assert_eq!(absolute_rect(&doc, body).1, 8.0);
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
    let before = doc.layout_node_with_style(body).unwrap().style.overflow;
    propagate_body_overflow_to_viewport(&mut doc, &cascade, body);
    assert_eq!(
        doc.layout_node_with_style(body).unwrap().style.overflow,
        before
    );
}

#[test]
fn repeated_layout_keeps_authored_body_padding_and_height() {
    let (mut doc, cascade, body, child) =
        body_doc("", "margin:8px;padding-top:5px;height:100px", "");
    with_ahem(&mut doc);
    for _ in 0..2 {
        layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
        assert_eq!(absolute_rect(&doc, body), (8.0, 8.0, 784.0, 105.0));
        assert_eq!(absolute_rect(&doc, child).1, 13.0);
        assert_eq!(doc.nodes[body].unrounded_layout.padding.top, 5.0);
    }
}

#[test]
fn a_body_uses_the_root_direction_for_its_over_constrained_margins() {
    let (doc, body, _) = laid_out("direction:rtl", "direction:ltr;max-width:300px", "");
    let layout = doc.nodes[body].unrounded_layout;
    assert_eq!(layout.location.x + layout.size.width, 800.0);
}

#[test]
fn an_auto_body_margin_is_zero() {
    let (doc, _, child) = laid_out("", "margin-top:auto", "margin-top:30px");
    assert_eq!(absolute_rect(&doc, child).1, 30.0);
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
