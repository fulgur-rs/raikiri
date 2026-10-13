use super::*;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use taffy::{AlignItems, Style};

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
    // The keyword width keeps the measured size; the percentage height
    // resolves against the page area.
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
    // The insets are measured from the body's padding box, not from the
    // parent the box sits in.
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
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
fn absolute_boxes_are_placed_against_a_positioned_ancestor_other_than_the_parent() {
    // The body contains the box, not the 0px tall wrapper it sits in.
    let (doc, abs) = laid_out_in(
        "position:relative",
        "margin-left:30px;height:300px",
        "",
        "position:absolute;bottom:0;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, abs), (30.0, 590.0, 20.0, 10.0));
    // The grandparent's padding box: 214px wide at x=30, y=50.
    for (abs_css, expected) in [
        (
            "right:0;top:0;width:20px;height:10px",
            (224.0, 50.0, 20.0, 10.0),
        ),
        (
            "left:0;bottom:0;width:50%;height:10px",
            (30.0, 154.0, 107.0, 10.0),
        ),
        // Auto insets keep the static position.
        ("width:20px;height:10px", (47.0, 57.0, 20.0, 10.0)),
    ] {
        let (doc, abs) = laid_out(
            "position:relative;margin-left:30px;padding:7px;width:200px;height:100px",
            "margin-left:10px",
            &format!("position:absolute;{abs_css}"),
        );
        assert_eq!(absolute_rect(&doc, abs), expected, "{abs_css}");
    }
}

#[test]
fn fixed_boxes_are_placed_against_a_transformed_ancestor_other_than_the_parent() {
    let (doc, fixed) = laid_out(
        "transform:translateX(0px);margin-left:30px;width:200px;height:100px;border:2px solid",
        "margin-left:10px",
        "position:fixed;right:0;bottom:0;width:10px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (222.0, 142.0, 10.0, 10.0));
}

#[test]
fn grid_static_positions_follow_justify_self_after_the_fixed_relayout() {
    // Centered in the 200px grid starting at x=100: 400px wide against the
    // page area, the margin box starts at x=0.
    for (grid_css, item_css, expected_x) in [
        ("justify-items:center", "", 0.0),
        // `justify-self` overrides `justify-items`; the right edge stays at
        // x=300.
        ("justify-items:center", "justify-self:end", -100.0),
        ("justify-items:end", "justify-self:start", 100.0),
        ("justify-items:end", "justify-self:center", 0.0),
        // A right-to-left grid starts at its right edge.
        ("direction:rtl", "", -100.0),
        ("direction:rtl;justify-items:end", "", 100.0),
        // The margin box is aligned.
        ("justify-items:end", "margin-right:20px", -120.0),
    ] {
        let (doc, fixed) = laid_out(
            "margin-left:100px;width:200px",
            &format!("display:grid;{grid_css}"),
            &format!("position:fixed;width:50%;height:10px;{item_css}"),
        );
        assert_eq!(
            absolute_rect(&doc, fixed),
            (expected_x, 50.0, 400.0, 10.0),
            "{grid_css};{item_css}"
        );
    }
}

#[test]
fn grid_static_positions_follow_justify_self_without_a_relayout() {
    // An absolutely positioned item contained by the grid keeps its size.
    for (grid_css, item_css, expected_x) in [
        ("justify-items:center", "", 175.0),
        ("justify-items:center", "justify-self:end", 250.0),
        ("", "justify-self:center", 175.0),
        ("justify-items:end", "justify-self:auto", 250.0),
        ("justify-items:stretch", "", 100.0),
        ("justify-items:center", "margin-left:10px", 180.0),
    ] {
        let (doc, item) = laid_out(
            "margin-left:100px;width:200px",
            &format!("display:grid;position:relative;{grid_css}"),
            &format!("position:absolute;width:50px;height:10px;{item_css}"),
        );
        assert_eq!(
            absolute_rect(&doc, item),
            (expected_x, 50.0, 50.0, 10.0),
            "{grid_css};{item_css}"
        );
    }
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

#[test]
fn keyword_sized_fixed_boxes_are_clamped_against_the_page_area() {
    let (mut doc, fixed) = laid_out(
        "margin-left:30px;width:200px",
        "",
        "position:fixed;left:5px;top:6px;width:min-content;max-width:50%;height:10px",
    );
    doc.append_element(
        Some(fixed),
        "div",
        Style::default(),
        Some("display:block;width:500px;height:5px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    // Against the 200px parent the 500px content was clamped to 100px; 50%
    // of the 800px page area lets it grow to 400px.
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 400.0, 10.0));
}

#[test]
fn static_positions_keep_the_flex_alignment_of_the_parent() {
    // Centered in the 200px row starting at x=100: 100px wide at x=150
    // against the parent, 400px wide at x=0 against the page area.
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "display:flex;justify-content:center",
        "position:fixed;width:50%;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (0.0, 50.0, 400.0, 10.0));
    // Aligned to the end of a column's cross axis, the right edge stays at
    // x=300.
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "display:flex;flex-direction:column;align-items:flex-end",
        "position:fixed;width:50%;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (-100.0, 50.0, 400.0, 10.0));
    // A distributed alignment falls back to start once the box overflows.
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "display:flex;justify-content:space-around",
        "position:fixed;width:50%;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (100.0, 50.0, 400.0, 10.0));
    // So does a safe alignment.
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "display:flex;justify-content:safe center",
        "position:fixed;width:50%;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (100.0, 50.0, 400.0, 10.0));
    // The default start alignment keeps the left edge.
    let (doc, fixed) = laid_out(
        "margin-left:100px;width:200px",
        "display:flex",
        "position:fixed;width:50%;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (100.0, 50.0, 400.0, 10.0));
}

#[test]
fn uncontained_absolute_boxes_are_placed_against_the_initial_containing_block() {
    let (doc, abspos) = laid_out(
        "margin-left:30px;height:1000px",
        "",
        "position:absolute;left:5px;bottom:0;width:50%;height:10px",
    );
    assert_eq!(absolute_rect(&doc, abspos), (5.0, 590.0, 400.0, 10.0));
}

#[test]
fn positioned_ancestors_contain_absolute_boxes() {
    let (doc, abspos) = laid_out(
        "margin-left:30px",
        "position:relative;height:40px",
        "position:absolute;left:5px;bottom:0;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, abspos), (35.0, 80.0, 20.0, 10.0));
}

#[test]
fn contained_and_will_change_ancestors_contain_absolute_boxes() {
    for css in ["contain:layout", "contain:paint", "will-change:position"] {
        let (doc, abspos) = laid_out(
            &format!("margin-left:30px;{css}"),
            "",
            "position:absolute;left:5px;top:6px;width:20px;height:10px",
        );
        // Taffy's placement against the parent, at its content box below
        // the first block, is the right one.
        assert_eq!(
            absolute_rect(&doc, abspos),
            (35.0, 56.0, 20.0, 10.0),
            "{css}"
        );
    }
    // `will-change: position` does not contain fixed boxes.
    let (doc, fixed) = laid_out(
        "margin-left:30px;will-change:position",
        "",
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
}

/// [`laid_out`] with a fixed box holding ten 100px Ahem words, so that its
/// min-content width is 100px and its max-content width 1090px.
fn laid_out_with_words(fixed_css: &str) -> (Document, usize) {
    let (mut doc, fixed) = laid_out("margin-left:30px;width:200px", "", fixed_css);
    doc.append_text(fixed, ["XXXXXXXXXX"; 10].join(" "));
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    (doc, fixed)
}

#[test]
fn sizing_keywords_resolve_against_the_containing_block() {
    let width = |style: Dimension| match resolve_sizing_keyword(style, 300.0, 800.0) {
        Some(KeywordSize::Measure(AvailableSpace::Definite(width))) => Some(("measure", width)),
        Some(KeywordSize::Measure(AvailableSpace::MinContent)) => Some(("min", 0.0)),
        Some(KeywordSize::Measure(AvailableSpace::MaxContent)) => Some(("max", 0.0)),
        Some(KeywordSize::Exact(width)) => Some(("exact", width)),
        None => None,
    };
    assert_eq!(width(Dimension::min_content()), Some(("min", 0.0)));
    assert_eq!(width(Dimension::max_content()), Some(("max", 0.0)));
    assert_eq!(
        width(Dimension::fit_content_px(120.0)),
        Some(("measure", 120.0))
    );
    assert_eq!(
        width(Dimension::fit_content_percent(0.25)),
        Some(("measure", 200.0))
    );
    assert_eq!(width(Dimension::fit_content()), Some(("measure", 300.0)));
    assert_eq!(width(Dimension::stretch()), Some(("exact", 300.0)));
    assert_eq!(width(Dimension::length(10.0)), None);
    assert_eq!(width(Dimension::auto()), None);
}

#[test]
fn sizing_keywords_measure_the_box_in_each_axis() {
    let (mut doc, fixed) = laid_out_with_words("position:absolute;left:5px;top:6px;font:10px Ahem");
    let area = Size {
        width: 800.0,
        height: 600.0,
    };
    let inset = Rect {
        left: Some(5.0),
        right: None,
        top: Some(6.0),
        bottom: None,
    };
    let margin = Rect {
        left: Some(0.0),
        right: Some(0.0),
        top: Some(0.0),
        bottom: Some(0.0),
    };
    let mut resolve = |width: Dimension, height: Dimension, known: Size<Option<f32>>| {
        let mut known = known;
        doc.nodes[fixed].style.size = Size { width, height };
        resolve_sizing_keywords(
            &mut doc,
            fixed,
            &mut known,
            Size { width, height },
            area,
            inset,
            margin,
        );
        known
    };
    // Both axes measured together: the min-content width of one word and
    // one 10px line per word.
    assert_eq!(
        resolve(
            Dimension::min_content(),
            Dimension::max_content(),
            Size::NONE
        ),
        Size {
            width: Some(100.0),
            height: Some(100.0),
        }
    );
    // `fit-content` measures the box in the 795px stretch size.
    assert_eq!(
        resolve(
            Dimension::fit_content(),
            Dimension::length(10.0),
            Size::NONE
        )
        .width,
        Some(795.0)
    );
    // A stretched width and a measured height wrap at the stretch size.
    assert_eq!(
        resolve(Dimension::stretch(), Dimension::max_content(), Size::NONE),
        Size {
            width: Some(795.0),
            height: Some(20.0),
        }
    );
    assert_eq!(
        resolve(Dimension::auto(), Dimension::stretch(), Size::NONE),
        Size {
            width: None,
            height: Some(594.0),
        }
    );
    // Known sizes are kept.
    let known = Size {
        width: Some(300.0),
        height: Some(40.0),
    };
    assert_eq!(
        resolve(Dimension::min_content(), Dimension::min_content(), known),
        known
    );
}

#[test]
fn static_position_alignment_follows_the_flex_container() {
    let alignment = |wrapper_css: &str, fixed_css: &str| {
        let (doc, fixed) = laid_out(
            "width:200px;height:100px",
            &format!("display:flex;height:100px;{wrapper_css}"),
            &format!("position:fixed;width:20px;height:10px;{fixed_css}"),
        );
        let (alignment, _) = static_position_alignment(
            &doc,
            fixed,
            Size {
                width: 100.0,
                height: 50.0,
            },
        );
        (alignment.width, alignment.height)
    };
    assert_eq!(alignment("", ""), (0.0, 0.0));
    assert_eq!(alignment("flex-direction:row-reverse", ""), (1.0, 0.0));
    assert_eq!(alignment("flex-direction:column-reverse", ""), (0.0, 1.0));
    assert_eq!(alignment("justify-content:end", ""), (1.0, 0.0));
    assert_eq!(
        alignment("justify-content:start;direction:rtl", ""),
        (1.0, 0.0)
    );
    assert_eq!(alignment("justify-content:flex-end", ""), (1.0, 0.0));
    assert_eq!(alignment("justify-content:space-around", ""), (0.5, 0.0));
    assert_eq!(alignment("justify-content:space-between", ""), (0.0, 0.0));
    assert_eq!(alignment("align-items:center", ""), (0.0, 0.5));
    assert_eq!(alignment("", "align-self:end"), (0.0, 1.0));
    assert_eq!(alignment("", "align-self:flex-end"), (0.0, 1.0));
    assert_eq!(alignment("", "align-self:baseline"), (0.0, 0.0));
    assert_eq!(alignment("flex-wrap:wrap-reverse", ""), (0.0, 1.0));
    assert_eq!(
        alignment("flex-direction:column;direction:rtl", ""),
        (1.0, 0.0)
    );
    // Taffy styles the cascade does not produce: safe alignment and the
    // `self-*` keywords.
    let with_align_self = |flex_direction: &str, direction: &str, align: AlignItems| {
        let (mut doc, fixed) = laid_out(
            "width:200px;height:100px",
            &format!("display:flex;height:100px;width:100px;flex-direction:{flex_direction}"),
            &format!("position:fixed;width:20px;height:10px;direction:{direction}"),
        );
        doc.nodes[fixed].style.align_self = Some(align);
        let (alignment, _) = static_position_alignment(
            &doc,
            fixed,
            Size {
                width: 50.0,
                height: 50.0,
            },
        );
        (alignment.width, alignment.height)
    };
    let align = |keyword, safety| AlignItems { keyword, safety };
    // Safe alignment that would overflow the container aligns as start.
    assert_eq!(
        with_align_self(
            "row",
            "ltr",
            align(AlignItemsKeyword::End, AlignmentSafety::Safe)
        ),
        (0.0, 1.0)
    );
    assert_eq!(
        with_align_self(
            "column",
            "ltr",
            align(AlignItemsKeyword::End, AlignmentSafety::Safe)
        ),
        (1.0, 0.0)
    );
    let (doc, fixed) = laid_out(
        "width:200px;height:100px",
        "display:flex;height:100px;width:100px",
        "position:fixed;width:20px;height:10px",
    );
    let mut doc = doc;
    doc.nodes[fixed].style.align_self = Some(align(AlignItemsKeyword::End, AlignmentSafety::Safe));
    let (alignment, _) = static_position_alignment(
        &doc,
        fixed,
        Size {
            width: 20.0,
            height: 200.0,
        },
    );
    assert_eq!((alignment.width, alignment.height), (0.0, 0.0));
    // `self-start` and `self-end` follow the box's own direction where the
    // cross axis is its inline axis.
    for (flex_direction, direction, keyword, expected) in [
        ("row", "rtl", AlignItemsKeyword::SelfStart, (0.0, 0.0)),
        ("row", "rtl", AlignItemsKeyword::SelfEnd, (0.0, 1.0)),
        ("column", "ltr", AlignItemsKeyword::SelfEnd, (1.0, 0.0)),
        ("column", "rtl", AlignItemsKeyword::SelfStart, (1.0, 0.0)),
        ("column", "rtl", AlignItemsKeyword::SelfEnd, (0.0, 0.0)),
    ] {
        assert_eq!(
            with_align_self(
                flex_direction,
                direction,
                align(keyword, AlignmentSafety::Unsafe)
            ),
            expected,
            "{flex_direction} {direction} {keyword:?}"
        );
    }
    // Block containers keep the start edge in the inline direction.
    let (doc, fixed) = laid_out("", "direction:rtl", "position:fixed");
    let (alignment, _) = static_position_alignment(&doc, fixed, Size::ZERO);
    assert_eq!((alignment.width, alignment.height), (1.0, 0.0));
}

#[test]
fn boxes_that_containment_does_not_apply_to_do_not_contain() {
    let fixed_css = "position:fixed;left:5px;top:6px;width:20px;height:10px";
    for wrapper_css in [
        "display:inline;contain:layout",
        "display:inline;will-change:transform",
        "display:inline;will-change:contain",
        "display:table-row;contain:paint",
        "display:contents;contain:layout",
        "display:contents;will-change:filter",
    ] {
        let (doc, fixed) = laid_out("margin-left:30px", wrapper_css, fixed_css);
        assert_eq!(
            absolute_rect(&doc, fixed),
            (5.0, 6.0, 20.0, 10.0),
            "{wrapper_css}"
        );
    }
    // Flex items are blockified, so containment applies to them.
    for wrapper_css in [
        "display:inline;contain:layout",
        "display:inline;will-change:transform",
        "display:inline;will-change:filter",
    ] {
        let (doc, fixed) = laid_out("margin-left:30px;display:flex", wrapper_css, fixed_css);
        assert_eq!(
            absolute_rect(&doc, fixed),
            (35.0, 56.0, 20.0, 10.0),
            "{wrapper_css}"
        );
    }
}

#[test]
fn positioned_boxes_that_are_not_generated_do_not_contain() {
    let (doc, abspos) = laid_out(
        "margin-left:30px",
        "display:contents;position:relative",
        "position:absolute;left:5px;top:6px;width:20px;height:10px",
    );
    assert_eq!(absolute_rect(&doc, abspos), (5.0, 6.0, 20.0, 10.0));
}

#[test]
fn the_body_border_does_not_move_uncontained_absolute_children() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;border:30px solid"),
    );
    let abspos = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;position:absolute;left:5px;top:6px;width:50%;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, abspos), (5.0, 6.0, 400.0, 10.0));
}

#[test]
fn min_content_fixed_paragraphs_take_their_longest_word() {
    let (doc, fixed) =
        laid_out_with_words("position:fixed;left:5px;top:6px;font:10px Ahem;width:min-content");
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 100.0, 100.0));
}

#[test]
fn a_root_filter_does_not_contain_absolute_descendants() {
    for root_css in [
        "display:block;filter:blur(0)",
        "display:block;will-change:filter",
    ] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), Some(root_css));
        let body = doc.append_element(
            Some(html),
            "body",
            Style::default(),
            Some("display:block;margin:0"),
        );
        let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;margin-left:30px;margin-top:50px"),
        );
        let abspos = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;position:absolute;left:5px;top:6px;width:20px;height:10px"),
        );
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).expect("cascade");
        layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
        assert_eq!(
            absolute_rect(&doc, abspos),
            (5.0, 6.0, 20.0, 10.0),
            "{root_css}"
        );
    }
}

#[test]
fn inline_form_controls_contain_fixed_descendants() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let parent = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;margin-left:30px;margin-top:50px"),
    );
    let button = doc.append_element(
        Some(parent),
        "button",
        Style::default(),
        Some("display:inline;will-change:transform"),
    );
    let fixed = doc.append_element(
        Some(button),
        "div",
        Style::default(),
        Some("display:block;position:fixed;left:5px;top:6px;width:20px;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_ne!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
}
