use super::*;
use raikiri_traits::PageBox;

/// Lay out a 100px wide `<body>` holding a block with `box_css` that holds a
/// block with `child_css`, and return the size of the outer block.
fn ratio_box_size(box_css: &str, child_css: &str) -> (f32, f32) {
    ratio_box_size_with(box_css, Some(child_css))
}

/// [`ratio_box_size`] with the inner block optional.
fn ratio_box_size_with(box_css: &str, child_css: Option<&str>) -> (f32, f32) {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:100px"),
    );
    let outer = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(format!("display:block;{box_css}").as_str()),
    );
    if let Some(child_css) = child_css {
        doc.append_element(
            Some(outer),
            "div",
            Style::default(),
            Some(format!("display:block;{child_css}").as_str()),
        );
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    let size = doc.nodes[outer].unrounded_layout.size;
    (size.width, size.height)
}

#[test]
fn a_block_height_from_the_ratio_grows_to_fit_its_content() {
    assert_eq!(
        ratio_box_size("width:100px;aspect-ratio:2/1", "height:100px"),
        (100.0, 100.0)
    );
}

#[test]
fn a_block_ratio_height_stays_when_the_content_is_shorter() {
    assert_eq!(
        ratio_box_size("aspect-ratio:2/1", "height:30px"),
        (100.0, 50.0)
    );
}

#[test]
fn a_zero_min_height_lets_the_ratio_height_stand() {
    assert_eq!(
        ratio_box_size("width:100px;aspect-ratio:2/1;min-height:0", "height:100px"),
        (100.0, 50.0)
    );
}

#[test]
fn a_clipping_block_keeps_the_ratio_height() {
    assert_eq!(
        ratio_box_size(
            "width:100px;aspect-ratio:2/1;overflow:hidden",
            "height:100px"
        ),
        (100.0, 50.0)
    );
}

#[test]
fn a_block_width_from_the_ratio_grows_to_its_min_content_width() {
    assert_eq!(
        ratio_box_size("height:50px;aspect-ratio:2/1", "width:300px;height:10px"),
        (300.0, 50.0)
    );
}

#[test]
fn a_maximum_carried_over_from_the_height_does_not_cap_a_width_the_content_raised() {
    assert_eq!(
        ratio_box_size(
            "height:200px;max-height:100px;aspect-ratio:1/1",
            "width:300px;height:10px"
        ),
        (300.0, 100.0)
    );
}

#[test]
fn the_maximum_width_the_block_states_caps_a_width_the_content_raised() {
    assert_eq!(
        ratio_box_size(
            "height:50px;max-width:80px;aspect-ratio:2/1",
            "width:300px;height:10px"
        ),
        (80.0, 50.0)
    );
}

#[test]
fn a_size_contained_empty_block_keeps_the_ratio_it_states() {
    assert_eq!(
        ratio_box_size_with("contain:size;width:100px;aspect-ratio:1/1", None),
        (100.0, 100.0)
    );
}

#[test]
fn a_size_contained_block_ignores_its_content_but_not_its_ratio() {
    assert_eq!(
        ratio_box_size(
            "contain:size;width:100px;aspect-ratio:2/1",
            "width:300px;height:300px"
        ),
        (100.0, 50.0)
    );
}

#[test]
fn a_floated_block_width_from_the_ratio_grows_to_its_min_content_width() {
    assert_eq!(
        ratio_box_size(
            "float:left;height:50px;aspect-ratio:2/1",
            "width:300px;height:10px"
        ),
        (300.0, 50.0)
    );
}

#[test]
fn a_floated_block_height_from_the_ratio_grows_to_fit_its_content() {
    assert_eq!(
        ratio_box_size("float:left;width:100px;aspect-ratio:2/1", "height:100px"),
        (100.0, 100.0)
    );
}

#[test]
fn a_block_takes_its_width_from_a_definite_height_through_the_ratio() {
    assert_eq!(
        ratio_box_size("height:30px;aspect-ratio:2/1", "height:10px"),
        (60.0, 30.0)
    );
}

#[test]
fn a_floated_block_takes_its_width_from_a_definite_height_through_the_ratio() {
    assert_eq!(
        ratio_box_size("float:left;height:30px;aspect-ratio:2/1", "height:10px"),
        (60.0, 30.0)
    );
}

#[test]
fn a_minimum_height_is_not_carried_into_a_definite_width() {
    assert_eq!(
        ratio_box_size(
            "aspect-ratio:2/1;min-height:100px;width:100px",
            "height:10px"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_maximum_width_is_not_carried_into_a_definite_height() {
    assert_eq!(
        ratio_box_size(
            "height:100px;aspect-ratio:2/1;max-width:100px",
            "width:200px;height:10px"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_maximum_height_is_not_carried_into_a_definite_width() {
    assert_eq!(
        ratio_box_size(
            "width:100px;aspect-ratio:1/2;max-height:100px",
            "height:200px"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_carried_maximum_does_not_cap_the_height_the_content_asks_for() {
    assert_eq!(
        ratio_box_size(
            "width:200px;aspect-ratio:2/1;max-width:100px",
            "height:100px"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_maximum_the_box_states_caps_the_height_the_content_asks_for() {
    assert_eq!(
        ratio_box_size(
            "width:100px;aspect-ratio:2/1;max-height:80px",
            "height:200px"
        ),
        (100.0, 80.0)
    );
}

#[test]
fn an_empty_block_with_a_minimum_height_keeps_a_definite_width() {
    assert_eq!(
        ratio_box_size_with("aspect-ratio:2/1;min-height:100px;width:100px", None),
        (100.0, 100.0)
    );
}

#[test]
fn a_maximum_width_wins_over_the_minimum_width_carried_from_the_minimum_height() {
    assert_eq!(
        ratio_box_size_with(
            "height:50px;aspect-ratio:2/1;min-height:100px;max-width:100px",
            None
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_column_flex_item_with_a_ratio_gets_its_flex_base_from_the_max_content_width() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-flow:column wrap;width:max-content;height:100px"),
    );
    let item = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("width:100%;aspect-ratio:1/1;min-height:0px"),
    );
    for _ in 0..2 {
        doc.append_element(
            Some(item),
            "div",
            Style::default(),
            Some("float:left;width:50px;height:50px"),
        );
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    let size = |node: usize| {
        let size = doc.nodes[node].unrounded_layout.size;
        (size.width, size.height)
    };
    assert_eq!((size(flex), size(item)), ((100.0, 100.0), (100.0, 100.0)));
}

#[test]
fn a_floated_block_takes_its_height_through_the_ratio_from_the_width_its_content_gives() {
    assert_eq!(
        ratio_box_size("float:left;aspect-ratio:1/1", "width:60px;height:10px"),
        (60.0, 60.0)
    );
}

#[test]
fn a_floated_block_shrinks_to_fit_its_content() {
    assert_eq!(
        ratio_box_size("float:left", "width:60px;height:10px"),
        (60.0, 10.0)
    );
}

#[test]
fn a_content_box_ratio_applies_to_the_content_box() {
    // The content box is 80px wide, so 40px tall by 2:1, and the padding adds
    // 20px to each axis.
    assert_eq!(
        ratio_box_size("width:80px;aspect-ratio:2/1;padding:10px", "height:1px"),
        (100.0, 60.0)
    );
}

#[test]
fn a_border_box_ratio_applies_to_the_border_box() {
    assert_eq!(
        ratio_box_size(
            "width:100px;aspect-ratio:2/1;padding:10px;box-sizing:border-box",
            "height:1px"
        ),
        (100.0, 50.0)
    );
}

#[test]
fn a_multicol_block_width_from_the_ratio_grows_to_the_widths_of_its_columns() {
    // The ratio gives 50px from the 50px height, but two columns of 100px
    // content need 200px.
    assert_eq!(
        ratio_box_size(
            "columns:2;column-gap:0;height:50px;aspect-ratio:1/1",
            "width:100px;height:10px"
        ),
        (200.0, 50.0)
    );
}

/// Lay out a 200px wide `<body>` holding an inline `<svg>` with a 2:1
/// `viewBox` and no `width` or `height` attribute, and return its size.
fn ratio_only_svg_size(css: &str) -> (f32, f32) {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:200px"),
    );
    let svg = doc.append_element(
        Some(body),
        "svg",
        Style::default(),
        Some(format!("display:block;{css}").as_str()),
    );
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(svg, vec![("viewBox".into(), "0 0 2 1".into())]);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 200.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    let size = doc.nodes[svg].unrounded_layout.size;
    (size.width, size.height)
}

#[test]
fn the_viewbox_ratio_sizes_a_ratio_only_svg_without_a_css_ratio() {
    assert_eq!(ratio_only_svg_size("width:100px"), (100.0, 50.0));
}

#[test]
fn an_authored_ratio_replaces_the_viewbox_ratio_of_an_svg_with_a_width() {
    assert_eq!(
        ratio_only_svg_size("width:100px;aspect-ratio:1/1"),
        (100.0, 100.0)
    );
}

#[test]
fn an_authored_ratio_replaces_the_viewbox_ratio_of_an_svg_with_a_height() {
    assert_eq!(
        ratio_only_svg_size("height:100px;aspect-ratio:1/1"),
        (100.0, 100.0)
    );
}

#[test]
fn an_authored_ratio_replaces_the_viewbox_ratio_of_an_svg_with_neither_size() {
    assert_eq!(ratio_only_svg_size("aspect-ratio:1/1"), (200.0, 200.0));
}

#[test]
fn an_auto_ratio_keeps_the_viewbox_ratio_of_an_svg() {
    assert_eq!(
        ratio_only_svg_size("width:100px;aspect-ratio:auto 1/1"),
        (100.0, 50.0)
    );
}
