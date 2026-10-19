use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

/// An absolute rect: x, y, width and height.
type Rect = (f32, f32, f32, f32);

/// `body > div(container_css) > [div(height:50px), div(float_css),
/// div(abs_css)]` with a 200px wide container, laid out on an 800x600 page.
/// Returns the absolute rect of the last div.
fn abs_rect(container_css: &str, float_css: &str, abs_css: &str) -> (f32, f32, f32, f32) {
    abs_rect_after(
        container_css,
        &[
            "height:50px",
            &format!("width:50px;height:50px;{float_css}"),
        ],
        abs_css,
    )
}

/// `body > div(container_css) > [div(before)..., div(abs_css)]` with a
/// 200px wide container, laid out on an 800x600 page. Returns the absolute
/// rect of the last div.
fn abs_rect_after(container_css: &str, before: &[&str], abs_css: &str) -> (f32, f32, f32, f32) {
    let mut doc = Document::new();
    let add = |doc: &mut Document, parent: usize, tag: &str, css: &str| {
        doc.append_element(Some(parent), tag, Style::default(), Some(css))
    };
    let html = add(&mut doc, 0, "html", "display:block");
    let body = add(&mut doc, html, "body", "display:block;margin:0");
    let container = add(
        &mut doc,
        body,
        "div",
        &format!("display:block;position:relative;width:200px;height:100px;{container_css}"),
    );
    for css in before {
        add(&mut doc, container, "div", &format!("display:block;{css}"));
    }
    let abs = add(
        &mut doc,
        container,
        "div",
        &format!("position:absolute;width:40px;height:10px;{abs_css}"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, abs)
}

fn abs_rect_after_nested_float(
    container_css: &str,
    owner_css: &str,
    float_css: &str,
    abs_css: &str,
) -> Rect {
    let mut doc = Document::new();
    let add = |doc: &mut Document, parent: usize, css: &str| {
        doc.append_element(Some(parent), "div", Style::default(), Some(css))
    };
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let container = add(
        &mut doc,
        body,
        &format!("display:block;position:relative;width:200px;height:100px;{container_css}"),
    );
    let owner = add(&mut doc, container, &format!("display:block;{owner_css}"));
    add(
        &mut doc,
        owner,
        &format!("display:block;width:50px;height:50px;{float_css}"),
    );
    let abs = add(
        &mut doc,
        container,
        &format!("position:absolute;width:40px;height:10px;{abs_css}"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, abs)
}

#[test]
fn an_inline_level_box_takes_the_aligned_point_of_its_line() {
    let inline = "display:inline";
    for (container, float, expected_x) in [
        // The float shortens the line to 0..150 or 50..200.
        ("text-align:center", "float:right", 75.0),
        ("text-align:center", "float:left", 125.0),
        ("text-align:left", "float:left", 50.0),
        ("text-align:right", "float:right", 150.0),
        ("text-align:end", "", 200.0),
        ("text-align:justify", "float:left", 50.0),
        // In a right-to-left container the point is the box's right edge.
        ("direction:rtl;text-align:center", "float:left", 85.0),
        ("direction:rtl;text-align:left", "float:left", 10.0),
        ("direction:rtl;text-align:end", "", -40.0),
        ("direction:rtl", "float:right", 110.0),
    ] {
        assert_eq!(
            abs_rect(container, float, inline),
            // Without a float the second block pushes the line down.
            (
                expected_x,
                if float.is_empty() { 100.0 } else { 50.0 },
                40.0,
                10.0
            ),
            "{container} {float}"
        );
    }
}

#[test]
fn margins_offset_the_box_from_the_point() {
    assert_eq!(
        abs_rect(
            "text-align:center",
            "",
            "display:inline;margin-left:5px;margin-right:auto"
        ),
        (105.0, 100.0, 40.0, 10.0)
    );
    assert_eq!(
        abs_rect(
            "direction:rtl;text-align:center",
            "",
            "display:inline;margin-right:5px;margin-left:auto"
        ),
        (55.0, 100.0, 40.0, 10.0)
    );
}

#[test]
fn a_float_beside_another_line_does_not_shorten_it() {
    // The float ends above the line.
    assert_eq!(
        abs_rect_after(
            "text-align:left",
            &["float:left;width:50px;height:20px", "height:50px"],
            "display:inline"
        ),
        (0.0, 50.0, 40.0, 10.0)
    );
    // A vertical inset moves the box off its line, which is still aligned.
    assert_eq!(
        abs_rect("text-align:center", "", "display:inline;top:60px"),
        (100.0, 60.0, 40.0, 10.0)
    );
}

#[test]
fn other_boxes_keep_the_block_static_position() {
    for (container, abs) in [
        // A horizontal inset fixes the position.
        ("text-align:center", "left:0"),
        // Vertical writing modes are left to taffy.
        ("text-align:center;writing-mode:vertical-lr", ""),
    ] {
        assert_eq!(
            abs_rect(container, "", &format!("display:inline;{abs}")),
            abs_rect(container, "", &format!("display:block;{abs}")),
            "{container} {abs}"
        );
    }
    // The children of a multicol container are placed in its columns.
    assert_eq!(
        abs_rect("text-align:center;column-count:2", "", "display:inline").0,
        0.0
    );
    // A block-level box is placed as a block.
    assert_eq!(abs_rect("text-align:center", "", "display:block").0, 0.0);
}

#[test]
fn the_box_keeps_its_place_when_its_containing_block_is_further_up() {
    // The containing block is the page, not the parent.
    assert_eq!(
        abs_rect(
            "text-align:center;position:static",
            "float:right",
            "display:inline"
        ),
        (75.0, 50.0, 40.0, 10.0)
    );
}

#[test]
fn fixed_descendants_stay_where_the_page_area_puts_them() {
    let mut doc = Document::new();
    let add = |doc: &mut Document, parent: usize, css: &str| {
        doc.append_element(Some(parent), "div", Style::default(), Some(css))
    };
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let container = add(
        &mut doc,
        body,
        "display:block;width:200px;text-align:center",
    );
    add(&mut doc, container, "display:block;height:50px");
    let abs = add(
        &mut doc,
        container,
        "display:inline;position:absolute;width:40px;height:10px",
    );
    let fixed = add(
        &mut doc,
        abs,
        "position:fixed;left:5px;top:6px;width:20px;height:10px",
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, abs), (100.0, 50.0, 40.0, 10.0));
    assert_eq!(absolute_rect(&doc, fixed), (5.0, 6.0, 20.0, 10.0));
}

#[test]
fn an_atomic_inline_is_aligned_as_its_margin_box() {
    assert_eq!(
        abs_rect(
            "text-align:center",
            "float:left",
            "display:inline-block;margin-left:10px"
        ),
        (110.0, 50.0, 40.0, 10.0)
    );
    assert_eq!(
        abs_rect(
            "direction:rtl;text-align:right",
            "float:right",
            "display:inline-block;margin-right:10px"
        ),
        (100.0, 50.0, 40.0, 10.0)
    );
    // A box wider than its float-shortened line moves below the float.
    assert_eq!(
        abs_rect(
            "text-align:center",
            "float:left",
            "display:inline-block;width:180px"
        ),
        (10.0, 100.0, 180.0, 10.0)
    );
}

#[test]
fn a_float_in_an_earlier_block_of_the_same_bfc_shortens_the_static_line() {
    assert_eq!(
        abs_rect_after_nested_float(
            "text-align:center",
            "height:20px",
            "float:left",
            "display:inline"
        ),
        (125.0, 20.0, 40.0, 10.0)
    );
}

#[test]
fn contents_children_participate_in_the_static_line() {
    let mut doc = Document::new();
    let add = |doc: &mut Document, parent: usize, css: &str| {
        doc.append_element(Some(parent), "div", Style::default(), Some(css))
    };
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let container = add(
        &mut doc,
        body,
        "display:block;position:relative;width:200px;height:100px;text-align:center",
    );
    let float_contents = add(&mut doc, container, "display:contents");
    add(
        &mut doc,
        float_contents,
        "display:block;width:50px;height:50px;float:left",
    );
    let abs_contents = add(&mut doc, container, "display:contents");
    let abs = add(
        &mut doc,
        abs_contents,
        "display:inline;position:absolute;width:40px;height:10px",
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, abs), (125.0, 0.0, 40.0, 10.0));
}

#[test]
fn vertical_insets_do_not_change_the_float_slot_of_the_static_line() {
    assert_eq!(
        abs_rect("text-align:center", "float:left", "display:inline;top:60px"),
        (125.0, 60.0, 40.0, 10.0)
    );
    assert_eq!(
        abs_rect(
            "text-align:center",
            "float:left",
            "display:inline;bottom:10px"
        ),
        (125.0, 80.0, 40.0, 10.0)
    );
}

#[test]
fn text_align_last_aligns_the_static_line() {
    assert_eq!(
        abs_rect_after(
            "text-align:left;text-align-last:right",
            &["height:50px"],
            "display:inline"
        ),
        (200.0, 50.0, 40.0, 10.0)
    );
    assert_eq!(
        abs_rect_after(
            "text-align:left;text-align-last:center",
            &["height:50px"],
            "display:inline"
        ),
        (100.0, 50.0, 40.0, 10.0)
    );
}

#[test]
fn logical_and_physical_text_align_last_values_set_the_static_line_edge() {
    for (text_align_last, expected_x) in [("start", 0.0), ("end", 200.0), ("left", 0.0)] {
        assert_eq!(
            abs_rect_after(
                &format!("text-align:left;text-align-last:{text_align_last}"),
                &[],
                "display:inline"
            ),
            (expected_x, 0.0, 40.0, 10.0),
            "text-align-last:{text_align_last}"
        );
    }
}

#[test]
fn unexpandable_justified_static_lines_use_center_alignment() {
    assert_eq!(
        abs_rect_after(
            "text-align:left;text-align-last:justify",
            &[],
            "display:inline-block;width:40px"
        ),
        (80.0, 0.0, 40.0, 10.0)
    );
    assert_eq!(
        abs_rect_after(
            "text-align:justify-all",
            &[],
            "display:inline-block;width:40px"
        ),
        (80.0, 0.0, 40.0, 10.0)
    );
}

#[test]
fn inline_static_line_still_avoids_floats_when_horizontal_inset_is_set() {
    assert_eq!(
        abs_rect_after(
            "text-align:center",
            &[
                "height:50px",
                "width:100px;height:50px;float:left",
                "width:100px;height:50px;float:right",
            ],
            "left:0;display:inline;top:auto;bottom:auto"
        ),
        (0.0, 100.0, 40.0, 10.0)
    );
}

#[test]
fn shrink_wrapped_inline_static_line_accounts_for_margins_when_fitting() {
    let mut doc = Document::new();
    let add = |doc: &mut Document, parent: usize, css: &str| {
        doc.append_element(Some(parent), "div", Style::default(), Some(css))
    };
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let container = add(
        &mut doc,
        body,
        "display:block;position:relative;width:200px;height:100px",
    );
    add(&mut doc, container, "display:block;height:50px");
    add(
        &mut doc,
        container,
        "display:block;width:50px;height:50px;float:left",
    );
    let abs = add(
        &mut doc,
        container,
        "position:absolute;display:inline-block;width:auto;height:10px;min-width:100px;max-width:220px;margin-left:10px;margin-right:10px;font-family:Ahem;font-size:10px",
    );
    doc.append_text(abs, "abcdefghij abcdefghij");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, abs).1, 50.0);
}

#[test]
fn a_static_line_with_no_float_slot_moves_below_the_floats() {
    assert_eq!(
        abs_rect_after(
            "text-align:center",
            &[
                "height:50px",
                "width:100px;height:50px;float:left",
                "width:100px;height:50px;float:right",
            ],
            "display:inline"
        ),
        (100.0, 100.0, 40.0, 10.0)
    );
}

#[test]
fn a_fixed_box_laid_out_again_keeps_its_children_aligned() {
    let mut doc = Document::new();
    let add = |doc: &mut Document, parent: usize, css: &str| {
        doc.append_element(Some(parent), "div", Style::default(), Some(css))
    };
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let narrow = add(&mut doc, body, "display:block;width:100px");
    // The fixed box is 50px wide against its parent, then 400px against the
    // page area.
    let fixed = add(
        &mut doc,
        narrow,
        "display:block;position:fixed;left:0;top:0;width:50%;height:100px;text-align:center",
    );
    add(&mut doc, fixed, "display:block;height:50px");
    let abs = add(
        &mut doc,
        fixed,
        "display:inline;position:absolute;width:40px;height:10px",
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, fixed).2, 400.0);
    assert_eq!(absolute_rect(&doc, abs), (200.0, 50.0, 40.0, 10.0));
}

#[test]
fn fixed_and_replaced_boxes_take_their_line_too() {
    // A fixed box keeps its static position in the page area.
    assert_eq!(
        abs_rect(
            "text-align:center",
            "float:right",
            "display:inline;position:fixed"
        ),
        (75.0, 50.0, 40.0, 10.0)
    );
}

/// `body > div(width:100px) > div(fixed, 50% wide, centered) >
/// [div(height:50px), tag(inner_css) > div(fixed, nested_css)]`. Returns
/// the absolute rects of the inner box and of the fixed box inside it.
fn inside_a_relaid_fixed_box(tag: &str, inner_css: &str, nested_css: &str) -> (Rect, Rect) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let add = |doc: &mut Document, parent: usize, tag: &str, css: &str| {
        doc.append_element(Some(parent), tag, Style::default(), Some(css))
    };
    let narrow = add(&mut doc, body, "div", "display:block;width:100px");
    let fixed = add(
        &mut doc,
        narrow,
        "div",
        "display:block;position:fixed;left:0;top:0;width:50%;height:100px;text-align:center",
    );
    add(&mut doc, fixed, "div", "display:block;height:50px");
    let inner = add(&mut doc, fixed, tag, inner_css);
    let nested = add(
        &mut doc,
        inner,
        "div",
        &format!("display:block;position:fixed;top:6px;width:20px;height:10px;{nested_css}"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    (absolute_rect(&doc, inner), absolute_rect(&doc, nested))
}

#[test]
fn a_moved_box_leaves_its_placed_fixed_boxes_in_place() {
    let (inner, nested) = inside_a_relaid_fixed_box(
        "div",
        "display:inline;position:absolute;width:40px;height:10px",
        "left:5px",
    );
    assert_eq!(inner, (200.0, 50.0, 40.0, 10.0));
    assert_eq!(nested, (5.0, 6.0, 20.0, 10.0));
    // A fixed box at its static position moves with the box.
    let (_, nested) = inside_a_relaid_fixed_box(
        "div",
        "display:inline;position:absolute;width:40px;height:10px",
        "",
    );
    assert_eq!(nested.0, 200.0);
    // A transformed box is the containing block of the fixed box.
    let (_, nested) = inside_a_relaid_fixed_box(
        "div",
        "display:inline;position:absolute;width:40px;height:10px;transform:translate(0)",
        "left:5px",
    );
    assert_eq!(nested.0, 205.0);
}

#[test]
fn a_replaced_inline_is_aligned_as_its_box() {
    let (inner, _) = inside_a_relaid_fixed_box(
        "img",
        "display:inline;position:absolute;width:40px;height:10px",
        "left:5px",
    );
    assert_eq!(inner, (180.0, 50.0, 40.0, 10.0));
}
