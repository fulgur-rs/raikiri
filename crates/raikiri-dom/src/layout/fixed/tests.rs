use super::*;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

/// `html > body(margin:0) > div(parent_css) > div(wrapper_css) > div(fixed_css)`
/// laid out on an 800x600 page with no margins. Returns the document and the
/// innermost div.
fn laid_out(parent_css: &str, wrapper_css: &str, fixed_css: &str) -> (Document, usize) {
    laid_out_in("", parent_css, wrapper_css, fixed_css)
}

/// [`laid_out`] with `body_css` added to the body.
fn laid_out_in(
    body_css: &str,
    parent_css: &str,
    wrapper_css: &str,
    fixed_css: &str,
) -> (Document, usize) {
    laid_out_with("", body_css, parent_css, wrapper_css, fixed_css)
}

/// [`laid_out_in`] with `html_css` added to the root element.
fn laid_out_with(
    html_css: &str,
    body_css: &str,
    parent_css: &str,
    wrapper_css: &str,
    fixed_css: &str,
) -> (Document, usize) {
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
        Some(format!("display:block;margin:0;{body_css}").as_str()),
    );
    let add = |doc: &mut Document, parent: usize, css: &str| {
        doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some(format!("display:block;{css}").as_str()),
        )
    };
    add(&mut doc, body, "height:50px");
    let parent = add(&mut doc, body, parent_css);
    let wrapper = add(&mut doc, parent, wrapper_css);
    let fixed = add(&mut doc, wrapper, fixed_css);
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    (doc, fixed)
}

#[test]
fn nested_fixed_boxes_are_placed_against_the_page_area() {
    let (doc, fixed) = laid_out(
        "margin-left:30px;padding:7px",
        "",
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
}

#[test]
fn nested_fixed_boxes_resolve_sizes_and_end_insets_against_the_page_area() {
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px;height:40px",
        "",
        "position:fixed;right:5px;bottom:6px;width:50%;height:10%",
    );
    assert_eq!(absolute_rect(&doc, fixed), (395.0, 534.0, 400.0, 60.0));
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "",
        "position:fixed;left:10px;right:20px;top:0;bottom:0",
    );
    assert_eq!(absolute_rect(&doc, fixed), (10.0, 0.0, 770.0, 600.0));
}

#[test]
fn auto_margins_center_nested_fixed_boxes_in_the_page_area() {
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "",
        "position:fixed;left:0;right:0;width:100px;height:10px;margin:0 auto;top:0",
    );
    assert_eq!(absolute_rect(&doc, fixed), (350.0, 0.0, 100.0, 10.0));
}

#[test]
fn an_axis_with_auto_insets_keeps_the_static_position() {
    let (doc, fixed) = laid_out(
        "margin-left:30px",
        "padding-top:4px",
        "position:fixed;left:5px;width:20px;height:10px",
    );
    // The static position is the wrapper's content box: x is the page area
    // inset, y stays below the first block and the wrapper's padding.
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 54.0, 20.0, 10.0));
}

#[test]
fn transformed_and_filtered_ancestors_contain_fixed_boxes() {
    for css in ["transform:translateX(1px)", "filter:blur(1px)"] {
        let (doc, fixed) = laid_out(
            &format!("margin-left:30px;{css}"),
            "",
            "position:fixed;left:5px;top:6px;width:20px;height:10px",
        );
        assert_eq!(
            absolute_rect(&doc, fixed),
            (35.0, 56.0, 20.0, 10.0),
            "{css}"
        );
    }
}

#[test]
fn fixed_boxes_inside_nested_fixed_boxes_are_placed_against_the_page_area() {
    let (doc, fixed) = laid_out(
        "margin-left:30px",
        "position:fixed;left:40px;top:50px;width:100px;height:100px",
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
}

#[test]
fn relaying_out_keeps_the_placement() {
    let (mut doc, fixed) = laid_out(
        "margin-left:30px",
        "",
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
}

#[test]
fn sized_by_keyword_nested_fixed_boxes_keep_that_size_and_move() {
    let (doc, fixed) = laid_out(
        "margin-left:30px;width:200px;height:40px",
        "",
        "position:fixed;left:5px;top:6px;width:max-content;height:50%",
    );
    // The keyword width keeps taffy's size; the percentage height resolves
    // against the page area.
    let (x, y, _, height) = absolute_rect(&doc, fixed);
    assert_eq!((x, y, height), (5.0, 6.0, 300.0));
}

#[test]
fn nested_fixed_boxes_with_auto_insets_are_sized_against_the_page_area() {
    let (doc, fixed) = laid_out(
        "margin-left:30px;width:200px",
        "",
        "position:fixed;width:50%;height:10px",
    );
    // Both axes keep the static position: the parent's content box below
    // the first block.
    assert_eq!(absolute_rect(&doc, fixed), (30.0, 50.0, 400.0, 10.0));
}

#[test]
fn fixed_boxes_inside_paragraphs_are_placed_against_the_page_area() {
    for (fixed_css, expected) in [
        ("position:fixed;left:5px;top:6px", (5.0, 6.0, 20.0, 10.0)),
        (
            "position:fixed;right:5px;bottom:6px",
            (775.0, 584.0, 20.0, 10.0),
        ),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        let div = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;margin:40px 0 0 30px"),
        );
        let p = doc.append_element(Some(div), "p", Style::default(), Some("display:block"));
        doc.append_text(p, "text ");
        let span = doc.append_element(Some(p), "span", Style::default(), Some(fixed_css));
        doc.append_text(span, "fx");
        doc.append_text(p, " more");
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        with_ahem(&mut doc);
        layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
        assert_eq!(absolute_rect(&doc, span), expected, "{fixed_css}");
    }
}

#[test]
fn over_constrained_nested_fixed_boxes_follow_the_page_direction() {
    let fixed_css = "position:fixed;left:10px;right:20px;top:0;width:100px;height:10px";
    let (doc, fixed) = laid_out_with("direction:rtl", "", "margin-right:30px", "", fixed_css);
    assert_eq!(absolute_rect(&doc, fixed), (680.0, 0.0, 100.0, 10.0));
    // The page area takes the root element's direction, not the body's.
    let (doc, fixed) = laid_out_with(
        "direction:rtl",
        "direction:ltr",
        "margin-right:30px",
        "",
        fixed_css,
    );
    assert_eq!(absolute_rect(&doc, fixed), (680.0, 0.0, 100.0, 10.0));
    let (doc, fixed) = laid_out_in("direction:rtl", "margin-right:30px", "", fixed_css);
    assert_eq!(absolute_rect(&doc, fixed), (10.0, 0.0, 100.0, 10.0));
}

#[test]
fn auto_margins_that_would_be_negative_are_zero() {
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "",
        "position:fixed;left:0;right:0;width:900px;height:10px;margin:0 auto;top:0",
    );
    assert_eq!(absolute_rect(&doc, fixed), (0.0, 0.0, 900.0, 10.0));
}

#[test]
fn fixed_boxes_in_hidden_subtrees_are_left_alone() {
    let (doc, fixed) = laid_out(
        "margin-left:30px",
        "display:none",
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    assert_eq!(doc.nodes[fixed].unrounded_layout.size.width, 0.0);
}

#[test]
fn a_fixed_box_with_its_own_transform_is_placed_against_the_page_area() {
    for css in ["transform:translateX(1px)", "filter:blur(1px)"] {
        let (doc, fixed) = laid_out(
            "margin-left:30px",
            "",
            &format!("position:fixed;left:5px;top:6px;width:20px;height:10px;{css}"),
        );
        assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0), "{css}");
    }
}

#[test]
fn the_body_border_does_not_move_fixed_boxes() {
    let (doc, fixed) = laid_out_in(
        "border:3px solid",
        "position:fixed;left:40px;top:50px;width:100px;height:100px",
        "",
        "position:fixed;left:0;top:0;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (0.0, 0.0, 20.0, 10.0));
    let body_child = doc.parent_of(doc.parent_of(fixed).unwrap()).unwrap();
    assert_eq!(absolute_rect(&doc, body_child), (40.0, 50.0, 100.0, 100.0));
}

#[test]
fn auto_margins_are_zero_when_an_inset_is_auto() {
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "",
        "position:fixed;right:0;top:0;width:100px;height:10px;margin-right:auto",
    );
    assert_eq!(absolute_rect(&doc, fixed), (700.0, 0.0, 100.0, 10.0));
}

#[test]
fn a_right_to_left_static_position_keeps_the_right_edge() {
    let (doc, fixed) = laid_out_in(
        "direction:rtl",
        "margin-right:30px;width:200px",
        "",
        "position:fixed;width:50%;height:10px",
    );
    // The parent's right content edge is at 770; the box grows to 400px
    // leftwards from it.
    assert_eq!(absolute_rect(&doc, fixed), (370.0, 50.0, 400.0, 10.0));
}

#[test]
fn a_transformed_body_contains_fixed_boxes() {
    let (doc, fixed) = laid_out_in(
        "transform:translateX(1px)",
        "margin-left:30px",
        "",
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (35.0, 56.0, 20.0, 10.0));
}

#[test]
fn static_positions_follow_margins_resolved_against_the_page_area() {
    let (doc, fixed) = laid_out(
        "margin-left:30px;width:200px",
        "",
        "position:fixed;width:20px;height:10px;margin-left:10%",
    );
    // The margin is 10% of the 800px page area, not of the 200px parent.
    assert_eq!(absolute_rect(&doc, fixed), (110.0, 50.0, 20.0, 10.0));
}

#[test]
fn grid_abspos_static_positions_survive_the_fixed_relayout() {
    let (mut doc, grid) = laid_out(
        "margin-left:30px;width:200px",
        "position:fixed;left:0;top:0;width:50%",
        "display:grid;width:50%;padding:10px",
    );
    let item = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("display:block;position:absolute;width:5px;height:5px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    // The grid is 50% of the 400px fixed box; the item starts at its
    // padding edge.
    let (gx, gy, gw, _) = absolute_rect(&doc, grid);
    assert_eq!(gw, 220.0);
    let (x, y, _, _) = absolute_rect(&doc, item);
    assert_eq!((x - gx, y - gy), (10.0, 10.0));
}

#[test]
fn paginated_margin_truncation_preserves_fixed_boxes_and_their_grid_children() {
    for (wrapper_css, fixed_css, expected_y) in [
        ("", "top:5px", 5.0),
        ("", "bottom:5px", 75.0),
        ("", "", 100.0),
        ("transform:translateY(1px)", "top:5px", 105.0),
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0"),
        );
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:100px"),
        );
        let wrapper = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(&format!(
                "display:block;margin-top:30px;height:20px;{wrapper_css}"
            )),
        );
        let fixed = doc.append_element(
            Some(wrapper),
            "div",
            Style::default(),
            Some(&format!(
                "display:block;position:fixed;left:5px;width:50px;height:20px;{fixed_css}"
            )),
        );
        let grid = doc.append_element(
            Some(fixed),
            "div",
            Style::default(),
            Some("display:grid;width:50%;padding:10px"),
        );
        let item = doc.append_element(
            Some(grid),
            "div",
            Style::default(),
            Some("display:block;position:absolute;width:5px;height:5px"),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        let mut page = page_box_800x600();
        page.height = 100.0;
        layout_pages(with_ahem(&mut doc), &cascade, page).expect("layout");
        assert_eq!(absolute_rect(&doc, wrapper).1, 100.0);
        let (_, y, width, height) = absolute_rect(&doc, fixed);
        assert_eq!(
            (y, width, height),
            (expected_y, 50.0, 20.0),
            "{wrapper_css};{fixed_css}"
        );
        let (gx, gy, gw, _) = absolute_rect(&doc, grid);
        assert_eq!(gw, 45.0);
        let (x, y, _, _) = absolute_rect(&doc, item);
        assert_eq!((x - gx, y - gy), (10.0, 10.0));
    }
}

#[test]
fn keyword_sized_fixed_boxes_take_padding_resolved_against_the_page_area() {
    let (mut doc, fixed) = laid_out(
        "margin-left:30px;width:200px",
        "",
        "position:fixed;left:5px;top:6px;width:min-content;height:10px;padding-left:10%",
    );
    doc.append_element(
        Some(fixed),
        "div",
        Style::default(),
        Some("display:block;width:30px;height:5px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    // The 30px content plus padding of 10% of the 800px page area.
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 110.0, 10.0));
}

#[test]
fn the_static_direction_comes_from_the_box_that_lays_out_the_fixed_box() {
    let (doc, fixed) = laid_out(
        "margin-left:30px;width:200px",
        "display:contents;direction:rtl",
        "position:fixed;width:50%;height:10px",
    );
    // The right-to-left wrapper has no box; the left-to-right parent keeps
    // the left edge.
    assert_eq!(absolute_rect(&doc, fixed), (30.0, 50.0, 400.0, 10.0));
}

#[test]
fn fixed_boxes_use_the_page_area_when_a_content_box_body_outgrows_it() {
    let (doc, fixed) = laid_out_in(
        "width:100px;border:3px solid",
        "margin-left:30px",
        "",
        "position:fixed;right:0;bottom:0;width:50%;height:10%",
    );
    assert_eq!(absolute_rect(&doc, fixed), (400.0, 540.0, 400.0, 60.0));
}
