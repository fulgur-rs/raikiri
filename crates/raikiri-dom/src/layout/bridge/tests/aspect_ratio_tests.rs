use super::*;
use raikiri_traits::PageBox;

/// Lay out a 100px wide `<body>` with the blocks `build` appends to it, and
/// return the document and the nodes `build` returns.
fn lay_out_in_a_100px_body(
    build: impl FnOnce(&mut Document, usize) -> Vec<usize>,
) -> (Document, Vec<usize>) {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let nodes = build(&mut doc, body);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    (doc, nodes)
}

/// Append a block with `css` to `parent`.
fn append_block(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(
        Some(parent),
        "div",
        Some(format!("display:block;{css}").as_str()),
    )
}

/// Lay out a 100px wide `<body>` holding a block with `box_css` that holds a
/// block with `child_css`, and return the size of the outer block.
fn ratio_box_size(box_css: &str, child_css: &str) -> (f32, f32) {
    ratio_box_size_with(box_css, Some(child_css))
}

/// [`ratio_box_size`] with the inner block optional.
fn ratio_box_size_with(box_css: &str, child_css: Option<&str>) -> (f32, f32) {
    let (doc, nodes) = lay_out_in_a_100px_body(|doc, body| {
        let outer = append_block(doc, body, box_css);
        if let Some(child_css) = child_css {
            append_block(doc, outer, child_css);
        }
        vec![outer]
    });
    let size = doc.nodes[nodes[0]].layout.size;
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
fn a_ratio_without_auto_relates_the_border_box_of_a_border_box_block() {
    assert_eq!(
        ratio_box_size_with(
            "width:100px;padding:10px;box-sizing:border-box;aspect-ratio:2/1",
            None
        ),
        (100.0, 50.0)
    );
}

#[test]
fn an_auto_ratio_relates_the_content_box_of_a_border_box_block() {
    assert_eq!(
        ratio_box_size_with(
            "width:100px;padding:10px;box-sizing:border-box;aspect-ratio:auto 2/1",
            None
        ),
        (100.0, 60.0)
    );
}

#[test]
fn an_auto_ratio_gives_a_border_box_block_its_width_from_the_height() {
    assert_eq!(
        ratio_box_size_with(
            "height:60px;padding:10px;box-sizing:border-box;aspect-ratio:auto 2/1",
            None
        ),
        (100.0, 60.0)
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-flow:column wrap;width:max-content;height:100px"),
    );
    let item = doc.append_element(
        Some(flex),
        "div",
        Some("width:100%;aspect-ratio:1/1;min-height:0px"),
    );
    for _ in 0..2 {
        doc.append_element(Some(item), "div", Some("float:left;width:50px;height:50px"));
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    let size = |node: usize| {
        let size = doc.nodes[node].layout.size;
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:200px"),
    );
    let svg = doc.append_element(
        Some(body),
        "svg",
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
    let size = doc.nodes[svg].layout.size;
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

/// Border-box size of a paragraph block with `css` that holds `text` set in
/// Ahem 10px.
fn ratio_paragraph_size(css: &str, text: &str) -> (f32, f32) {
    use crate::layout::test_support::{
        ahem_paragraph_in, ifc_ahem_fonts, page_box_800x600, with_ahem,
    };
    let (mut doc, cascade, root) = ahem_paragraph_in("", css, text);
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::page_pipeline::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        page_box_800x600(),
    )
    .expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    let size = doc.nodes[root].layout.size;
    (size.width, size.height)
}

#[test]
fn a_paragraph_ratio_height_grows_to_fit_its_lines() {
    // The ratio gives 10px, but one 20px line needs more.
    assert_eq!(
        ratio_paragraph_size("width:100px;aspect-ratio:10/1;line-height:20px", "aaaa"),
        (100.0, 20.0)
    );
}

#[test]
fn a_paragraph_ratio_height_stays_when_the_lines_are_shorter() {
    assert_eq!(
        ratio_paragraph_size("width:100px;aspect-ratio:2/1;line-height:10px", "aaaa"),
        (100.0, 50.0)
    );
}

#[test]
fn a_zero_min_height_lets_the_paragraph_ratio_height_stand() {
    assert_eq!(
        ratio_paragraph_size(
            "width:100px;aspect-ratio:10/1;line-height:20px;min-height:0",
            "aaaa"
        ),
        (100.0, 10.0)
    );
}

#[test]
fn a_clipping_paragraph_keeps_the_ratio_height() {
    assert_eq!(
        ratio_paragraph_size(
            "width:100px;aspect-ratio:10/1;line-height:20px;overflow:hidden",
            "aaaa"
        ),
        (100.0, 10.0)
    );
}

#[test]
fn a_paragraph_maximum_height_caps_the_height_its_lines_ask_for() {
    assert_eq!(
        ratio_paragraph_size(
            "width:100px;aspect-ratio:10/1;line-height:20px;max-height:15px",
            "aaaa"
        ),
        (100.0, 15.0)
    );
}

#[test]
fn a_paragraph_ratio_width_grows_to_its_longest_word() {
    // The ratio gives 10px from the 10px height; the word is 40px wide.
    assert_eq!(
        ratio_paragraph_size("height:10px;aspect-ratio:1/1", "aaaa"),
        (40.0, 10.0)
    );
}

#[test]
fn a_paragraph_ratio_height_and_its_lines_both_count_the_padding() {
    // The content box is 80px wide, so 40px tall by 2:1; the 60px line is
    // taller, and the padding adds 20px to each axis.
    assert_eq!(
        ratio_paragraph_size(
            "width:80px;aspect-ratio:2/1;padding:10px;line-height:60px",
            "aaaa"
        ),
        (100.0, 80.0)
    );
}

#[test]
fn a_floated_paragraph_takes_its_height_through_the_ratio_from_the_width_its_text_gives() {
    // The text is 40px wide, so a square is 40px tall and the 10px line fits.
    assert_eq!(
        ratio_paragraph_size("float:left;aspect-ratio:1/1", "aaaa"),
        (40.0, 40.0)
    );
}

#[test]
fn a_floated_paragraph_ratio_height_grows_to_fit_its_lines() {
    assert_eq!(
        ratio_paragraph_size("float:left;aspect-ratio:4/1;line-height:30px", "aaaa"),
        (40.0, 30.0)
    );
}

#[test]
fn a_clipping_paragraph_keeps_the_ratio_width() {
    assert_eq!(
        ratio_paragraph_size("height:10px;aspect-ratio:2/1;overflow:hidden", "aaaa"),
        (20.0, 10.0)
    );
}

#[test]
fn a_paragraph_with_a_minimum_width_keeps_the_ratio_width() {
    assert_eq!(
        ratio_paragraph_size("height:20px;aspect-ratio:1/1;min-width:5px", "aaaa"),
        (20.0, 20.0)
    );
}

#[test]
fn a_paragraph_with_both_sizes_ignores_the_ratio() {
    assert_eq!(
        ratio_paragraph_size("width:100px;height:10px;aspect-ratio:1/1", "aaaa"),
        (100.0, 10.0)
    );
}

#[test]
fn a_size_contained_paragraph_ignores_its_lines() {
    assert_eq!(
        ratio_paragraph_size(
            "width:100px;aspect-ratio:1/1;contain:size;line-height:200px",
            "aaaa"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn an_inline_block_paragraph_ratio_height_grows_to_fit_its_lines() {
    assert_eq!(
        ratio_paragraph_size(
            "display:inline-block;width:100px;aspect-ratio:10/1;line-height:20px",
            "aaaa"
        ),
        (100.0, 20.0)
    );
}

#[test]
fn a_clipping_inline_block_paragraph_keeps_the_ratio_height() {
    assert_eq!(
        ratio_paragraph_size(
            "display:inline-block;width:100px;aspect-ratio:10/1;line-height:20px;overflow:hidden",
            "aaaa"
        ),
        (100.0, 10.0)
    );
}

#[test]
fn an_inline_block_paragraph_ratio_width_grows_to_its_longest_word() {
    assert_eq!(
        ratio_paragraph_size("display:inline-block;height:10px;aspect-ratio:1/1", "aaaa"),
        (40.0, 10.0)
    );
}

#[test]
fn a_clipping_inline_block_paragraph_keeps_the_ratio_width() {
    assert_eq!(
        ratio_paragraph_size(
            "display:inline-block;height:10px;aspect-ratio:2/1;overflow:hidden",
            "aaaa"
        ),
        (20.0, 10.0)
    );
}

#[test]
fn a_content_keyword_width_takes_its_value_from_the_height_through_the_ratio() {
    for width in ["min-content", "max-content", "fit-content"] {
        assert_eq!(
            ratio_box_size_with(&format!("width:{width};height:25px;aspect-ratio:4/1"), None),
            (100.0, 25.0),
            "width: {width}"
        );
    }
}

#[test]
fn a_minimum_height_reaches_a_content_keyword_width_through_the_ratio() {
    assert_eq!(
        ratio_box_size_with(
            "width:min-content;height:10px;min-height:25px;aspect-ratio:4/1",
            None
        ),
        (100.0, 25.0)
    );
}

#[test]
fn a_maximum_height_reaches_a_content_keyword_width_through_the_ratio() {
    assert_eq!(
        ratio_box_size_with(
            "width:min-content;height:300px;max-height:25px;aspect-ratio:4/1",
            None
        ),
        (100.0, 25.0)
    );
}

#[test]
fn a_content_keyword_width_gives_the_height_through_the_ratio_when_the_height_is_auto() {
    assert_eq!(
        ratio_box_size(
            "width:min-content;aspect-ratio:2/1",
            "width:40px;height:10px"
        ),
        (40.0, 20.0)
    );
}

#[test]
fn a_floated_block_with_a_content_keyword_width_takes_it_from_the_height() {
    assert_eq!(
        ratio_box_size_with(
            "float:left;width:max-content;height:25px;aspect-ratio:4/1",
            None
        ),
        (100.0, 25.0)
    );
}

#[test]
fn a_definite_width_and_height_still_ignore_the_ratio() {
    assert_eq!(
        ratio_box_size_with("width:40px;height:25px;aspect-ratio:4/1", None),
        (40.0, 25.0)
    );
}

/// Lay out `before`, `middle` and `after` blocks in a 100px wide `<body>`, with
/// a block with `inner_css` inside `middle`, and return the top and the height
/// of the three of them.
fn margin_stack(before: &str, middle: &str, inner_css: &str, after: &str) -> [(f32, f32); 3] {
    let (doc, nodes) = lay_out_in_a_100px_body(|doc, body| {
        let before = append_block(doc, body, &format!("width:100px;{before}"));
        let middle = append_block(doc, body, &format!("width:100px;{middle}"));
        append_block(doc, middle, &format!("width:100px;{inner_css}"));
        let after = append_block(doc, body, &format!("width:100px;{after}"));
        vec![before, middle, after]
    });
    [nodes[0], nodes[1], nodes[2]].map(|node| {
        let layout = doc.nodes[node].layout;
        (layout.location.y, layout.size.height)
    })
}

#[test]
fn margins_that_collapse_through_the_child_of_a_ratio_block_leave_its_height_alone() {
    assert_eq!(
        margin_stack(
            "height:25px;margin-bottom:-200px",
            "aspect-ratio:2/1",
            "margin-top:50px;margin-bottom:200px",
            "height:25px"
        ),
        [(0.0, 25.0), (25.0, 50.0), (75.0, 25.0)]
    );
}

#[test]
fn a_margin_below_a_child_of_a_ratio_block_stays_inside_and_raises_its_height() {
    assert_eq!(
        ratio_box_size(
            "width:100px;aspect-ratio:2/1",
            "height:40px;margin-bottom:20px"
        ),
        (100.0, 60.0)
    );
}

#[test]
fn margins_that_collapse_through_the_child_of_a_ratio_block_do_not_raise_its_height() {
    assert_eq!(
        ratio_box_size(
            "width:100px;aspect-ratio:2/1",
            "margin-top:30px;margin-bottom:200px"
        ),
        (100.0, 50.0)
    );
}

#[test]
fn an_inline_block_with_a_content_keyword_width_takes_it_from_the_height() {
    assert_eq!(
        ratio_box_size_with(
            "display:inline-block;width:min-content;height:25px;aspect-ratio:4/1",
            None
        ),
        (100.0, 25.0)
    );
}

#[test]
fn a_table_cell_with_a_content_keyword_width_takes_it_from_the_height() {
    assert_eq!(
        ratio_box_size_with(
            "display:table-cell;width:min-content;height:25px;aspect-ratio:4/1",
            None
        ),
        (100.0, 25.0)
    );
}

#[test]
fn a_maximum_width_caps_a_content_keyword_width_taken_from_the_height() {
    assert_eq!(
        ratio_box_size_with(
            "width:min-content;max-width:60px;height:25px;aspect-ratio:4/1",
            None
        ),
        (60.0, 25.0)
    );
}

#[test]
fn a_ratio_relates_the_content_box_of_a_content_keyword_width() {
    assert_eq!(
        ratio_box_size_with(
            "width:min-content;height:25px;padding:10px;aspect-ratio:4/1",
            None
        ),
        (120.0, 45.0)
    );
}

#[test]
fn a_paragraph_with_a_content_keyword_width_grows_to_its_longest_word() {
    assert_eq!(
        ratio_paragraph_size(
            "display:block;width:min-content;height:10px;aspect-ratio:1/1",
            "aaaa"
        ),
        (40.0, 10.0)
    );
}

#[test]
fn a_paragraph_with_a_content_keyword_width_takes_its_height_through_the_ratio() {
    assert_eq!(
        ratio_paragraph_size("display:block;width:min-content;aspect-ratio:1/1", "aaaa"),
        (40.0, 40.0)
    );
}

#[test]
fn a_size_contained_box_keeps_the_width_of_an_empty_box_for_a_content_keyword() {
    assert_eq!(
        ratio_box_size_with(
            "contain:size;width:min-content;height:25px;aspect-ratio:4/1",
            Some("width:30px;height:10px")
        ),
        (0.0, 25.0)
    );
}

#[test]
fn inline_size_containment_of_a_horizontal_box_keeps_the_width_of_an_empty_box() {
    assert_eq!(
        ratio_box_size_with(
            "contain:inline-size;width:min-content;height:25px;aspect-ratio:4/1",
            Some("width:30px;height:10px")
        ),
        (0.0, 25.0)
    );
}

#[test]
fn inline_size_containment_of_a_vertical_box_leaves_the_width_to_the_ratio() {
    assert_eq!(
        ratio_box_size_with(
            "writing-mode:vertical-rl;contain:inline-size;width:min-content;height:25px;aspect-ratio:4/1",
            Some("width:30px;height:10px")
        ),
        (100.0, 25.0)
    );
}

#[test]
fn a_percentage_height_that_cannot_resolve_sizes_a_content_keyword_width_by_its_content() {
    assert_eq!(
        ratio_box_size(
            "width:max-content;height:50%;aspect-ratio:2/1",
            "width:40px;height:10px"
        ),
        (40.0, 20.0)
    );
}

/// Lay out a 100px wide `<body>` holding a block with `parent_css` that holds
/// a block with `box_css` that holds a block with `child_css`, and return the
/// size of the middle block.
fn ratio_box_size_in(parent_css: &str, box_css: &str, child_css: &str) -> (f32, f32) {
    let (doc, nodes) = lay_out_in_a_100px_body(|doc, body| {
        let parent = append_block(doc, body, parent_css);
        let outer = append_block(doc, parent, box_css);
        append_block(doc, outer, child_css);
        vec![outer]
    });
    let size = doc.nodes[nodes[0]].layout.size;
    (size.width, size.height)
}

/// Like [`ratio_box_size_in`], with a line of text in the box instead of a
/// block.
fn ratio_box_size_holding_text_in(parent_css: &str, box_css: &str) -> (f32, f32) {
    let (doc, nodes) = lay_out_in_a_100px_body(|doc, body| {
        let parent = append_block(doc, body, parent_css);
        let outer = append_block(doc, parent, box_css);
        doc.append_text(outer, "x");
        vec![outer]
    });
    let size = doc.nodes[nodes[0]].layout.size;
    (size.width, size.height)
}

#[test]
fn a_percentage_height_that_cannot_resolve_takes_its_height_from_the_ratio() {
    assert_eq!(
        ratio_box_size(
            "width:40px;height:50%;aspect-ratio:2/1",
            "width:40px;height:10px"
        ),
        (40.0, 20.0)
    );
}

#[test]
fn a_percentage_height_that_cannot_resolve_is_not_raised_by_the_content() {
    // The height is no `auto` size, so the automatic minimum is not the height
    // of the content, which overflows.
    for box_css in [
        "width:40px;height:50%;aspect-ratio:2/1",
        "width:40px;height:calc(50% + 1px);aspect-ratio:2/1",
        "width:40px;height:50%;aspect-ratio:2/1;min-height:auto",
        "width:40px;height:50%;aspect-ratio:2/1;overflow:hidden",
        "float:left;width:40px;height:50%;aspect-ratio:2/1",
        "display:inline-block;width:40px;height:50%;aspect-ratio:2/1",
    ] {
        assert_eq!(
            ratio_box_size(box_css, "width:40px;height:30px"),
            (40.0, 20.0),
            "{box_css}"
        );
    }
}

#[test]
fn a_percentage_height_that_cannot_resolve_takes_the_width_of_the_box_for_its_ratio_height() {
    assert_eq!(
        ratio_box_size("height:50%;aspect-ratio:2/1", "height:80px"),
        (100.0, 50.0)
    );
}

#[test]
fn a_percentage_height_that_cannot_resolve_counts_the_edges_it_is_sized_by() {
    assert_eq!(
        ratio_box_size(
            "width:40px;height:50%;aspect-ratio:2/1;padding:3px;border:2px solid",
            "width:34px;height:30px"
        ),
        (50.0, 30.0)
    );
    assert_eq!(
        ratio_box_size(
            "width:40px;height:50%;aspect-ratio:2/1;box-sizing:border-box;padding:3px;border:2px solid",
            "width:30px;height:30px"
        ),
        (40.0, 20.0)
    );
}

#[test]
fn a_maximum_height_caps_the_ratio_height_of_a_percentage_height_that_cannot_resolve() {
    for box_css in [
        "width:40px;height:50%;aspect-ratio:2/1;max-height:15px",
        "width:40px;height:calc(50% + 1px);aspect-ratio:2/1;max-height:15px",
    ] {
        assert_eq!(
            ratio_box_size(box_css, "width:40px;height:10px"),
            (40.0, 15.0),
            "{box_css}"
        );
    }
    assert_eq!(
        ratio_box_size("height:50%;aspect-ratio:2/1;max-height:30px", "height:80px"),
        (60.0, 30.0)
    );
}

#[test]
fn a_maximum_width_reaches_a_percentage_height_that_cannot_resolve_through_the_ratio() {
    for box_css in [
        "width:40px;height:50%;aspect-ratio:2/1;max-width:30px",
        "width:50%;height:50%;aspect-ratio:2/1;max-width:30px",
    ] {
        assert_eq!(
            ratio_box_size(box_css, "width:10px;height:10px"),
            (30.0, 15.0),
            "{box_css}"
        );
    }
    assert_eq!(
        ratio_box_size("height:50%;aspect-ratio:2/1;max-width:60px", "height:10px"),
        (60.0, 30.0)
    );
}

#[test]
fn a_minimum_width_reaches_a_percentage_height_that_cannot_resolve_through_the_ratio() {
    assert_eq!(
        ratio_box_size(
            "width:50%;height:50%;aspect-ratio:2/1;min-width:80px",
            "height:10px"
        ),
        (80.0, 40.0)
    );
    assert_eq!(
        ratio_box_size("height:50%;aspect-ratio:2/1;min-width:120px", "height:10px"),
        (120.0, 60.0)
    );
}

#[test]
fn a_percentage_height_that_resolves_leaves_the_ratio_nothing_to_do() {
    for (box_css, child_css, expected) in [
        (
            "width:40px;height:10%;aspect-ratio:2/1",
            "width:40px;height:10px",
            (40.0, 10.0),
        ),
        (
            "width:40px;height:50%;aspect-ratio:2/1",
            "width:40px;height:30px",
            (40.0, 50.0),
        ),
        (
            "width:40px;height:50%;aspect-ratio:2/1;max-height:15px",
            "height:10px",
            (40.0, 15.0),
        ),
        (
            "width:50%;height:50%;aspect-ratio:2/1;max-width:30px",
            "height:10px",
            (30.0, 50.0),
        ),
        (
            "width:60%;height:50%;aspect-ratio:2/1;min-width:80px",
            "height:10px",
            (80.0, 50.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in("height:100px", box_css, child_css),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_bound_on_a_percentage_height_that_resolves_gives_the_other_axis_its_ratio_size() {
    for (box_css, expected) in [
        ("height:50%;aspect-ratio:2/1;max-height:15px", (30.0, 15.0)),
        ("height:50%;aspect-ratio:2/1;min-height:60px", (120.0, 60.0)),
        ("width:50%;aspect-ratio:2/1;max-height:15px", (50.0, 15.0)),
    ] {
        assert_eq!(
            ratio_box_size_in("height:100px", box_css, "height:10px"),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_flex_item_with_a_percentage_height_that_cannot_resolve_follows_the_ratio_and_its_bounds() {
    for (box_css, child_css, expected) in [
        (
            "width:40px;height:50%;aspect-ratio:2/1",
            "width:40px;height:30px",
            (40.0, 20.0),
        ),
        (
            "width:40px;height:50%;aspect-ratio:2/1;max-height:15px",
            "height:10px",
            (40.0, 15.0),
        ),
        (
            "width:50%;height:50%;aspect-ratio:2/1;max-width:30px",
            "height:10px",
            (30.0, 15.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in("display:flex", box_css, child_css),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_flex_item_stating_only_its_main_size_takes_the_cross_size_from_the_flexed_main_size() {
    for (parent_css, box_css, expected) in [
        // Growing and shrinking move the main size away from the stated one.
        (
            "display:flex;width:100px",
            "width:20px;aspect-ratio:1/1;flex-grow:1",
            (100.0, 100.0),
        ),
        (
            "display:flex;width:100px",
            "width:200px;aspect-ratio:2/1;flex-shrink:1",
            (100.0, 50.0),
        ),
        (
            "display:flex;width:100px",
            "width:50px;aspect-ratio:1/1;flex:1",
            (100.0, 100.0),
        ),
        (
            "display:flex;width:100px",
            "width:50px;aspect-ratio:1/1;min-width:0;flex:1",
            (100.0, 100.0),
        ),
        (
            "display:flex",
            "width:200px;aspect-ratio:8/1;min-width:0",
            (100.0, 12.5),
        ),
        (
            "display:flex;width:100px",
            "width:50%;aspect-ratio:1/1;flex-grow:1",
            (100.0, 100.0),
        ),
        // An item that keeps the main size it states keeps the ratio size.
        (
            "display:flex;width:100px",
            "width:50px;aspect-ratio:1/1",
            (50.0, 50.0),
        ),
        // A stretching item does not let the ratio override the container.
        (
            "display:flex;flex-direction:column;height:100px",
            "height:20px;aspect-ratio:2/1;flex-grow:1",
            (100.0, 100.0),
        ),
        (
            "display:flex;flex-direction:column;height:100px;align-items:flex-start",
            "height:20px;aspect-ratio:2/1;flex-grow:1",
            (200.0, 100.0),
        ),
        (
            "display:flex;width:100px;align-items:flex-start",
            "width:20px;aspect-ratio:1/2;flex-grow:1",
            (100.0, 200.0),
        ),
        // The ratio relates the content boxes of a box sized by its content box.
        (
            "display:flex;width:100px",
            "width:20px;aspect-ratio:1/1;flex-grow:1;padding:5px",
            (100.0, 100.0),
        ),
        (
            "display:flex;width:100px",
            "width:20px;aspect-ratio:1/1;flex-grow:1;padding:5px;box-sizing:border-box",
            (100.0, 100.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(parent_css, box_css, ""),
            expected,
            "{parent_css} | {box_css}"
        );
    }
}

#[test]
fn a_bound_on_the_flexed_main_size_of_a_ratio_item_limits_the_cross_size_it_gives() {
    for (box_css, child_css, expected) in [
        (
            "width:20px;aspect-ratio:1/2;flex-grow:1;max-width:60px",
            "",
            (60.0, 120.0),
        ),
        (
            "width:20px;aspect-ratio:1/2;flex-grow:1;min-width:30px",
            "",
            (100.0, 200.0),
        ),
        // The content raises the height the ratio gives, unless the item scrolls.
        (
            "width:20px;aspect-ratio:1/1;flex-grow:1",
            "height:200px",
            (100.0, 200.0),
        ),
        (
            "width:20px;aspect-ratio:1/1;flex-grow:1;overflow:hidden",
            "height:200px",
            (100.0, 100.0),
        ),
        (
            "width:20px;aspect-ratio:1/1;flex-grow:1;max-height:60px",
            "",
            (100.0, 60.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in("display:flex;width:100px", box_css, child_css),
            expected,
            "{box_css} | {child_css}"
        );
    }
}

#[test]
fn a_flex_basis_of_content_takes_the_main_size_from_the_cross_size_through_the_ratio() {
    for (parent_css, box_css, child_css, expected) in [
        // The main size the item states is ignored for the flex base size.
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px;min-width:0",
            "",
            (100.0, 100.0),
        ),
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px",
            "",
            (100.0, 100.0),
        ),
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:2/1;width:20px;min-width:0",
            "",
            (200.0, 100.0),
        ),
        (
            "display:flex;flex-direction:column;width:300px;height:300px",
            "flex-basis:content;width:100px;aspect-ratio:1/1;height:20px;min-height:0",
            "",
            (100.0, 100.0),
        ),
        // The cross size may be a percentage of a definite container size.
        (
            "display:flex;width:300px;height:200px",
            "flex-basis:content;height:50%;aspect-ratio:1/1;width:20px;min-width:0",
            "",
            (100.0, 100.0),
        ),
        // Padding is outside the content box the ratio relates.
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px;min-width:0;padding:5px",
            "",
            (110.0, 110.0),
        ),
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px;min-width:0;padding:5px;box-sizing:border-box",
            "",
            (100.0, 100.0),
        ),
        // The base size flexes like any other.
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:2/1;width:20px;min-width:0;flex-grow:1",
            "",
            (300.0, 100.0),
        ),
        (
            "display:flex;width:80px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px;min-width:0",
            "",
            (80.0, 100.0),
        ),
        (
            "display:flex;width:300px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px;min-width:0;flex-shrink:0",
            "",
            (100.0, 100.0),
        ),
        // The automatic minimum is the smaller of the content and the stated size.
        (
            "display:flex;width:60px",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px",
            "width:50px;height:10px",
            (60.0, 100.0),
        ),
        (
            "display:flex;flex-direction:column;width:300px;height:30px",
            "flex-basis:content;width:100px;aspect-ratio:1/1;height:20px;min-height:0",
            "",
            (100.0, 30.0),
        ),
        (
            "display:flex;width:300px;align-items:flex-start",
            "flex-basis:content;height:100px;aspect-ratio:1/1;width:20px;min-width:0",
            "",
            (100.0, 100.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(parent_css, box_css, child_css),
            expected,
            "{parent_css} | {box_css}"
        );
    }
}

#[test]
fn an_item_that_does_not_ask_for_a_flex_basis_of_content_keeps_the_sizes_it_states() {
    for (box_css, expected) in [
        (
            "width:20px;height:100px;aspect-ratio:1/1;min-width:0",
            (20.0, 100.0),
        ),
        (
            "flex-basis:auto;width:20px;height:100px;aspect-ratio:1/1;min-width:0;flex-grow:1",
            (300.0, 100.0),
        ),
        // Without a cross size to transfer, a basis of content is content.
        (
            "flex-basis:content;width:20px;aspect-ratio:1/1;min-width:0",
            (0.0, 0.0),
        ),
        (
            "flex-basis:content;height:100px;aspect-ratio:1/1",
            (100.0, 100.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in("display:flex;width:300px", box_css, ""),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_flex_item_that_measures_its_own_content_keeps_the_ratio_size_it_states() {
    for (box_css, expected) in [
        ("width:50px;aspect-ratio:1/1", (50.0, 50.0)),
        ("width:50px;aspect-ratio:1/2", (50.0, 100.0)),
        ("width:50px;aspect-ratio:1/1;overflow:hidden", (50.0, 50.0)),
        ("width:50px;aspect-ratio:1/1;padding:5px", (60.0, 60.0)),
    ] {
        assert_eq!(
            ratio_box_size_holding_text_in("display:flex;width:100px", box_css),
            expected,
            "text | {box_css}"
        );
    }
    for (box_css, child_css, expected) in [
        ("display:flex;width:50px;aspect-ratio:1/1", "", (50.0, 50.0)),
        (
            "display:flex;width:50px;aspect-ratio:1/1",
            "height:10px",
            (50.0, 50.0),
        ),
        (
            "display:grid;width:50px;aspect-ratio:1/1",
            "height:10px",
            (50.0, 50.0),
        ),
        (
            "display:flow-root;width:50px;aspect-ratio:1/1",
            "height:10px",
            (50.0, 50.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in("display:flex;width:100px", box_css, child_css),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_percentage_cross_size_that_cannot_resolve_gives_no_flex_base_size() {
    for cross in ["height:50%", "height:calc(50% + 10px)"] {
        let box_css = format!("flex-basis:content;{cross};aspect-ratio:1/1;width:20px;min-width:0");
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:300px",
                &box_css,
                "width:80px;height:80px"
            ),
            (80.0, 80.0),
            "{box_css}"
        );
    }
}

#[test]
fn a_flex_item_in_a_vertical_container_keeps_the_ratio_it_states() {
    for (parent_css, box_css, expected) in [
        (
            "display:flex;writing-mode:vertical-rl;height:100px",
            "width:20px;aspect-ratio:1/1;flex-grow:1",
            (20.0, 100.0),
        ),
        (
            "display:flex;writing-mode:vertical-rl;height:100px;width:200px",
            "height:20px;aspect-ratio:1/1;flex-grow:1",
            (200.0, 100.0),
        ),
        (
            "display:flex;flex-direction:column;writing-mode:vertical-rl;height:100px;width:300px",
            "width:20px;aspect-ratio:1/1;flex-grow:1",
            (300.0, 100.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(parent_css, box_css, ""),
            expected,
            "{parent_css} | {box_css}"
        );
    }
}

/// Lay out a 100px wide `<body>` holding a block with `parent_css` that holds
/// a block with `box_css` that holds a block with `child_css`, followed by the
/// blocks `siblings` hold, and return the sizes of the middle block and the
/// siblings.
fn ratio_item_sizes_among(
    parent_css: &str,
    box_css: &str,
    child_css: &str,
    siblings: &[&str],
) -> Vec<(f32, f32)> {
    let (doc, nodes) = lay_out_in_a_100px_body(|doc, body| {
        let parent = append_block(doc, body, parent_css);
        let outer = append_block(doc, parent, box_css);
        append_block(doc, outer, child_css);
        let mut nodes = vec![outer];
        nodes.extend(siblings.iter().map(|css| append_block(doc, parent, css)));
        nodes
    });
    nodes
        .into_iter()
        .map(|node| {
            let size = doc.nodes[node].layout.size;
            (size.width, size.height)
        })
        .collect()
}

#[test]
fn the_automatic_minimum_of_a_ratio_flex_item_is_the_width_of_its_content() {
    // The item states its height and takes 50px of width from the ratio; a
    // box in it wider than that keeps the item at the width of the box.
    for (parent_css, box_css, child_width, expected) in [
        ("width:0", "", "0", (50.0, 100.0)),
        ("width:0", "", "50px", (50.0, 100.0)),
        ("width:0", "", "100px", (100.0, 100.0)),
        ("width:0", "", "200px", (200.0, 100.0)),
        ("width:30px", "", "100px", (100.0, 100.0)),
        ("width:300px", "", "200px", (200.0, 100.0)),
        ("width:0", "flex-basis:0", "100px", (100.0, 100.0)),
        ("width:0", "flex-basis:content", "100px", (100.0, 100.0)),
        ("width:0", "flex-basis:20px", "100px", (100.0, 100.0)),
        ("width:0", "flex-shrink:0", "200px", (200.0, 100.0)),
        (
            "width:0;flex-direction:row-reverse",
            "",
            "100px",
            (100.0, 100.0),
        ),
        ("width:0;flex-wrap:wrap", "", "100px", (100.0, 100.0)),
        ("width:0;align-items:center", "", "100px", (100.0, 100.0)),
        (
            "width:0",
            "height:calc(50px + 50px)",
            "100px",
            (100.0, 100.0),
        ),
        ("width:0", "display:flex", "100px", (100.0, 100.0)),
        ("width:0", "display:grid", "100px", (100.0, 100.0)),
        ("width:0", "display:inline-block", "100px", (100.0, 100.0)),
        ("width:0", "margin:0 7px", "100px", (100.0, 100.0)),
        ("width:0", "overflow:clip", "100px", (100.0, 100.0)),
        ("width:0", "min-width:auto", "100px", (100.0, 100.0)),
    ] {
        let box_css = format!("height:100px;aspect-ratio:1/2;{box_css}");
        assert_eq!(
            ratio_box_size_in(
                &format!("display:flex;height:100px;{parent_css}"),
                &box_css,
                &format!("width:{child_width};height:10px")
            ),
            expected,
            "{parent_css} | {box_css} | {child_width}"
        );
    }
}

#[test]
fn the_automatic_minimum_of_a_ratio_flex_item_is_the_height_of_its_content_in_a_column() {
    for (parent_css, child_height, expected) in [
        ("height:0", "0", (100.0, 50.0)),
        ("height:0", "50px", (100.0, 50.0)),
        ("height:0", "100px", (100.0, 100.0)),
        ("height:0", "200px", (100.0, 200.0)),
        ("height:30px", "100px", (100.0, 100.0)),
        ("height:300px", "200px", (100.0, 200.0)),
        (
            "height:0;flex-direction:column-reverse",
            "100px",
            (100.0, 100.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(
                &format!("display:flex;flex-direction:column;width:100px;{parent_css}"),
                "width:100px;aspect-ratio:2/1",
                &format!("height:{child_height};width:10px")
            ),
            expected,
            "{parent_css} | {child_height}"
        );
    }
}

#[test]
fn a_percentage_cross_size_that_resolves_leaves_the_content_of_a_ratio_flex_item_its_room() {
    assert_eq!(
        ratio_box_size_in(
            "display:flex;width:0;height:100px",
            "aspect-ratio:1/2;height:100%",
            "width:100px;height:10px"
        ),
        (100.0, 100.0)
    );
    assert_eq!(
        ratio_box_size_in(
            "display:flex;flex-direction:column;width:100px;height:0",
            "aspect-ratio:2/1;width:100%",
            "height:100px;width:10px"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_ratio_flex_item_stretched_across_the_container_keeps_the_width_of_its_content() {
    // The item states no height; the container's height, which it is
    // stretched to, gives the ratio its cross size.
    assert_eq!(
        ratio_box_size_in(
            "display:flex;width:0;height:100px",
            "aspect-ratio:1/2",
            "width:100px;height:10px"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn the_automatic_minimum_of_a_ratio_flex_item_counts_its_padding_and_border() {
    for (box_css, expected) in [
        ("padding:10px", (120.0, 120.0)),
        ("padding:10px;box-sizing:border-box", (120.0, 100.0)),
        ("border:5px solid red", (110.0, 110.0)),
        ("border:5px solid red;box-sizing:border-box", (110.0, 100.0)),
        ("padding:10%", (140.0, 140.0)),
    ] {
        let box_css = format!("height:100px;aspect-ratio:1/2;{box_css}");
        let width = if box_css.contains("10%") {
            "200px"
        } else {
            "0"
        };
        assert_eq!(
            ratio_box_size_in(
                &format!("display:flex;width:{width};height:100px"),
                &box_css,
                "width:100px;height:10px"
            ),
            expected,
            "{box_css}"
        );
    }
    for (box_css, expected) in [
        ("padding:10px", (120.0, 120.0)),
        ("padding:10px;box-sizing:border-box", (100.0, 120.0)),
    ] {
        let box_css = format!("width:100px;aspect-ratio:2/1;{box_css}");
        assert_eq!(
            ratio_box_size_in(
                "display:flex;flex-direction:column;width:100px;height:0",
                &box_css,
                "height:100px;width:10px"
            ),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn the_automatic_minimum_of_a_ratio_flex_item_is_capped_by_its_maximum_width() {
    for (max_width, expected) in [("70px", (70.0, 100.0)), ("300px", (100.0, 100.0))] {
        let box_css = format!("height:100px;aspect-ratio:1/2;max-width:{max_width}");
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:0;height:100px",
                &box_css,
                "width:100px;height:10px"
            ),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_ratio_flex_item_that_does_not_use_the_automatic_minimum_shrinks_below_its_content() {
    for (box_css, expected) in [
        ("min-width:0", (0.0, 100.0)),
        ("min-width:20px", (20.0, 100.0)),
        ("overflow:hidden", (0.0, 100.0)),
        ("overflow:scroll", (0.0, 100.0)),
        ("overflow:auto", (0.0, 100.0)),
        ("overflow-x:hidden", (0.0, 100.0)),
        ("overflow-y:hidden", (0.0, 100.0)),
    ] {
        let box_css = format!("height:100px;aspect-ratio:1/2;{box_css}");
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:0;height:100px",
                &box_css,
                "width:100px;height:10px"
            ),
            expected,
            "{box_css}"
        );
    }
    assert_eq!(
        ratio_box_size_in(
            "display:flex;flex-direction:column;width:100px;height:0",
            "width:100px;aspect-ratio:2/1;overflow:hidden",
            "height:100px;width:10px"
        ),
        (100.0, 0.0)
    );
}

#[test]
fn a_ratio_flex_item_stating_both_sizes_is_not_shrunk_below_the_size_its_ratio_gives() {
    // The ratio gives the item no size of its own, but what its content is
    // at least: the size across the item, carried over, and no more than the
    // size the item states.
    for (box_css, expected) in [
        ("width:100px;height:100px;aspect-ratio:1", (100.0, 100.0)),
        ("width:100px;height:50px;aspect-ratio:1", (50.0, 50.0)),
        ("width:50px;height:100px;aspect-ratio:1", (50.0, 100.0)),
        ("width:100px;height:100px;aspect-ratio:2/1", (100.0, 100.0)),
        ("width:300px;height:100px;aspect-ratio:1/2", (50.0, 100.0)),
        (
            "width:100px;height:100px;aspect-ratio:1;padding:10px",
            (120.0, 120.0),
        ),
        (
            "width:100px;height:100px;aspect-ratio:1;padding:10px;box-sizing:border-box",
            (100.0, 100.0),
        ),
        // The size across the item is within its bounds before the ratio
        // carries it over.
        (
            "width:100px;height:100px;max-height:50px;aspect-ratio:1",
            (50.0, 50.0),
        ),
        (
            "width:100px;height:100px;max-height:50px;aspect-ratio:1/2",
            (25.0, 50.0),
        ),
        (
            "width:300px;height:100px;min-height:150px;aspect-ratio:1/2",
            (75.0, 150.0),
        ),
        (
            "width:30px;height:100px;min-height:150px;aspect-ratio:1/2",
            (30.0, 150.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:0;height:100px",
                box_css,
                "width:0;height:0"
            ),
            expected,
            "{box_css}"
        );
    }
    // The content, wider than the ratio gives, is held to the size the item
    // states.
    for (box_css, child_width, expected) in [
        ("width:100px", "80px", (80.0, 100.0)),
        ("width:300px", "100px", (100.0, 100.0)),
        ("width:300px", "400px", (300.0, 100.0)),
    ] {
        let box_css = format!("{box_css};height:100px;aspect-ratio:1/2");
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:0;height:100px",
                &box_css,
                &format!("width:{child_width};height:10px")
            ),
            expected,
            "{box_css} | {child_width}"
        );
    }
    for (box_css, child_css, expected) in [
        (
            "width:100px;height:100px;aspect-ratio:2",
            "height:10px;width:10px",
            (100.0, 50.0),
        ),
        (
            "width:100px;height:80px;aspect-ratio:2",
            "height:90px;width:10px",
            (100.0, 80.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(
                "display:flex;flex-direction:column;width:100px;height:0",
                box_css,
                child_css
            ),
            expected,
            "{box_css}"
        );
    }
}

#[test]
fn a_ratio_flex_item_wider_than_its_ratio_gives_takes_the_room_it_needs_from_its_siblings() {
    assert_eq!(
        ratio_item_sizes_among(
            "display:flex;width:200px;height:100px",
            "height:100px;aspect-ratio:1/2",
            "width:120px;height:10px",
            &["height:100px;width:200px"]
        ),
        [(120.0, 100.0), (80.0, 100.0)]
    );
    assert_eq!(
        ratio_item_sizes_among(
            "display:flex;width:200px;height:100px",
            "height:100px;aspect-ratio:1/2",
            "width:100px;height:10px",
            &["height:100px;width:200px"]
        ),
        [(100.0, 100.0), (100.0, 100.0)]
    );
    assert_eq!(
        ratio_item_sizes_among(
            "display:flex;width:300px;height:100px",
            "height:100px;aspect-ratio:1/2;flex-grow:1",
            "width:200px;height:10px",
            &["height:100px;width:100px;flex-grow:1"]
        ),
        [(200.0, 100.0), (100.0, 100.0)]
    );
}

#[test]
fn boxes_that_take_no_part_in_the_flex_layout_leave_a_ratio_flex_item_its_minimum() {
    for sibling in [
        "display:none;height:100px;aspect-ratio:1/2",
        "position:absolute",
    ] {
        assert_eq!(
            ratio_item_sizes_among(
                "display:flex;position:relative;width:0;height:100px",
                "height:100px;aspect-ratio:1/2",
                "width:100px;height:10px",
                &[sibling]
            )[0],
            (100.0, 100.0),
            "{sibling}"
        );
    }
}

#[test]
fn a_flex_item_that_states_a_keyword_width_keeps_the_width_its_ratio_gives() {
    for width in ["max-content", "min-content", "fit-content"] {
        let box_css = format!("width:{width};height:100px;aspect-ratio:1/2");
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:0;height:100px",
                &box_css,
                "width:100px;height:10px"
            ),
            (50.0, 100.0),
            "{box_css}"
        );
    }
}

#[test]
fn the_height_of_a_column_ratio_item_is_bounded_by_the_width_limits_carried_over_the_ratio() {
    let edges = "padding-left:15px;padding-top:10px;box-sizing:border-box";
    for (box_css, child_height, expected) in [
        (
            "width:100px;max-width:25px;aspect-ratio:1/8",
            "500px",
            (25.0, 200.0),
        ),
        (
            "width:100px;max-width:25px;aspect-ratio:1/1",
            "500px",
            (25.0, 25.0),
        ),
        (
            "width:10px;min-width:25px;aspect-ratio:1/8",
            "500px",
            (25.0, 500.0),
        ),
        (
            "width:10px;min-width:25px;aspect-ratio:1/8",
            "100px",
            (25.0, 200.0),
        ),
        (
            &format!("width:25px;aspect-ratio:1/1;{edges}"),
            "190px",
            (25.0, 200.0),
        ),
        (
            &format!("width:25px;aspect-ratio:auto 1/1;{edges}"),
            "190px",
            (25.0, 200.0),
        ),
        (
            &format!("width:100px;max-width:25px;aspect-ratio:1/8;{edges}"),
            "500px",
            (25.0, 200.0),
        ),
        (
            &format!("width:100px;max-width:25px;aspect-ratio:auto 1/19;{edges}"),
            "500px",
            (25.0, 200.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(
                "display:flex;flex-direction:column;width:100px;height:1px",
                box_css,
                &format!("height:{child_height}")
            ),
            expected,
            "{box_css} | {child_height}"
        );
    }
}

#[test]
fn the_width_of_a_row_ratio_item_is_not_bounded_by_the_height_limits_carried_over_the_ratio() {
    for (box_css, child_width, expected) in [
        (
            "height:100px;max-height:25px;aspect-ratio:8/1",
            "500px",
            (500.0, 25.0),
        ),
        (
            "height:100px;max-height:80px;aspect-ratio:1/2",
            "100px",
            (100.0, 80.0),
        ),
        (
            "height:100px;max-height:20px;aspect-ratio:2/1",
            "100px",
            (100.0, 20.0),
        ),
        (
            "height:100px;max-height:20px;aspect-ratio:2/1",
            "10px",
            (40.0, 20.0),
        ),
        (
            "height:10px;min-height:25px;aspect-ratio:8/1",
            "100px",
            (200.0, 25.0),
        ),
    ] {
        assert_eq!(
            ratio_box_size_in(
                "display:flex;width:1px;height:100px",
                box_css,
                &format!("width:{child_width};height:10px")
            ),
            expected,
            "{box_css} | {child_width}"
        );
    }
}

#[test]
fn a_flex_item_with_a_keyword_maximum_keeps_the_width_its_ratio_gives() {
    let box_css = "height:100px;aspect-ratio:1/2;max-width:max-content";
    assert_eq!(
        ratio_box_size_in(
            "display:flex;width:0;height:100px",
            box_css,
            "width:100px;height:10px"
        ),
        (50.0, 100.0)
    );
}
