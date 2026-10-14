use super::*;

/// A body holding a container of style `container` with one child of style
/// `child`, laid out on an A4 page.
fn child_in(container: &str, child: &str) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let container =
        document.append_element(Some(body), "div", taffy::Style::default(), Some(container));
    let child =
        document.append_element(Some(container), "div", taffy::Style::default(), Some(child));
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    (document, cascade, child)
}

fn offset(container: &str, child: &str) -> (f32, f32) {
    let (document, cascade, child) = child_in(container, child);
    used_relative_offset(&document, &cascade.computed[child], child)
}

#[test]
fn percentages_resolve_against_the_containing_block_content_box() {
    // Hand-computed: the content box is 200px by 100px inside 10px of
    // padding and 5px of border.
    let container = "display:block;width:200px;height:100px;padding:10px;border:5px solid";
    assert_eq!(
        offset(container, "float:left;position:relative;left:10%;top:25%"),
        (20.0, 25.0)
    );
    assert_eq!(
        offset(
            container,
            "float:left;position:relative;right:calc(50% - 4px);bottom:calc(10% + 1px)"
        ),
        (-96.0, -11.0)
    );
}

#[test]
fn a_percentage_of_an_indefinite_height_counts_as_auto() {
    // CSS 2.1 §9.3.2: `top` falls back to `auto`, so `bottom` applies, and
    // with both `auto` there is no vertical offset.
    let container = "display:block;width:200px";
    assert_eq!(
        offset(container, "float:left;position:relative;top:50%;bottom:3px"),
        (0.0, -3.0)
    );
    assert_eq!(
        offset(
            container,
            "float:left;position:relative;left:5%;top:50%;bottom:10%"
        ),
        (10.0, 0.0)
    );
}

#[test]
fn a_display_contents_ancestor_is_skipped() {
    // The containing block is the 100px-high body child around the
    // `display: contents` element.
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let block = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;width:100px;height:100px"),
    );
    let contents = document.append_element(
        Some(block),
        "div",
        taffy::Style::default(),
        Some("display:contents"),
    );
    let child = document.append_element(
        Some(contents),
        "div",
        taffy::Style::default(),
        Some("float:left;position:relative;left:10%;top:20%"),
    );
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[child], child),
        (10.0, 20.0)
    );
}

#[test]
fn only_relative_boxes_are_offset() {
    let container = "display:block;width:200px;height:100px";
    assert_eq!(
        offset(container, "position:static;left:10px;top:10px"),
        (0.0, 0.0)
    );
    assert_eq!(
        offset(container, "position:sticky;left:10px;top:10px"),
        (0.0, 0.0)
    );
    let (document, cascade, child) = child_in(container, "position:sticky;top:10%");
    assert_eq!(
        used_relative_insets(
            &document,
            &cascade.computed[child],
            child,
            Precedence::WritingMode
        ),
        (None, Some(10.0))
    );
}

#[test]
fn a_right_to_left_containing_block_takes_right_over_left() {
    let container = "display:block;direction:rtl;width:200px;height:100px";
    assert_eq!(
        offset(
            container,
            "float:left;position:relative;left:10px;right:10%"
        ),
        (-20.0, 0.0)
    );
    assert_eq!(
        offset(container, "float:left;position:relative;left:10%"),
        (20.0, 0.0)
    );
}

#[test]
fn a_column_box_is_the_containing_block_inside_columns() {
    // Hand-computed: two 90px columns with a 20px gap.
    let container = "display:block;width:200px;column-count:2;column-gap:20px";
    assert_eq!(offset(container, "position:relative;left:10%"), (9.0, 0.0));
}

#[test]
fn an_inline_ancestor_is_not_the_containing_block() {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let block = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;width:300px;height:100px"),
    );
    let span = document.append_element(
        Some(block),
        "span",
        taffy::Style::default(),
        Some("display:inline"),
    );
    let child = document.append_element(
        Some(span),
        "div",
        taffy::Style::default(),
        Some("float:left;position:relative;left:10%;top:10%;width:10px;height:10px"),
    );
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[child], child),
        (30.0, 10.0)
    );
}

#[test]
fn a_column_spanner_takes_the_full_multicol_width() {
    // Hand-computed: the spanner's containing block is the whole 200px
    // content box, not one 90px column.
    let container = "display:block;width:200px;column-count:2;column-gap:20px";
    assert_eq!(
        offset(
            container,
            "display:block;column-span:all;position:relative;left:50%"
        )
        .0,
        100.0
    );
    // A float does not span the columns, whatever its `column-span`, so it
    // stays inside a 90px column.
    assert_eq!(
        offset(
            container,
            "column-span:all;float:left;position:relative;left:50%"
        )
        .0,
        45.0
    );
}

#[test]
fn a_vertical_containing_block_takes_its_start_side_inset() {
    // CSS Position 3 §3.3: in `vertical-rl` the right side is the block
    // start, so `right` wins over `left`.
    let container = "display:block;width:200px;height:100px;writing-mode:vertical-rl";
    assert_eq!(
        offset(container, "float:left;position:relative;left:10%;right:5px").0,
        -5.0
    );
}

#[test]
fn a_calc_with_a_zero_percentage_needs_no_definite_height() {
    let container = "display:block;width:200px";
    assert_eq!(
        offset(
            container,
            "float:left;position:relative;top:calc(5px + 0%);bottom:3px"
        )
        .1,
        5.0
    );
}

#[test]
fn body_percentages_resolve_against_the_page_area() {
    let (document, cascade, body) = relative_body(
        None,
        "display:block;margin:0;position:relative;left:10%;top:10%",
    );
    let (width, height) = document.fixed_page_area;
    let (dx, dy) = used_relative_offset(&document, &cascade.computed[body], body);
    assert!((dx - width * 0.1).abs() < 1e-3 && (dy - height * 0.1).abs() < 1e-3);
}

/// A relatively positioned body of style `body` under an `html` root of
/// style `html`, laid out on an A4 page.
fn relative_body(
    html: Option<&str>,
    body: &str,
) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", taffy::Style::default(), html);
    let body = document.append_element(Some(html), "body", taffy::Style::default(), Some(body));
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    (document, cascade, body)
}

#[test]
fn the_body_takes_the_direction_of_the_root_element() {
    // CSS Writing Modes 4 §8: the initial containing block takes the root
    // element's direction, so `right` wins.
    let (document, cascade, body) = relative_body(
        Some("direction:rtl"),
        "display:block;margin:0;position:relative;left:10px;right:20px",
    );
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[body], body).0,
        -20.0
    );
}

#[test]
fn a_sideways_containing_block_takes_its_start_side_inset() {
    // CSS Position 3 §3.3: `sideways-rl` starts its block axis on the right
    // and `sideways-lr` starts its inline axis at the bottom.
    let container = "display:block;width:200px;height:100px;writing-mode:sideways-rl";
    assert_eq!(
        offset(container, "float:left;position:relative;left:10%;right:5px").0,
        -5.0
    );
    let container = "display:block;width:200px;height:100px;writing-mode:sideways-lr";
    assert_eq!(
        offset(container, "float:left;position:relative;top:5px;bottom:10%").1,
        -10.0
    );
}

#[test]
fn a_stretched_flex_item_has_a_definite_height() {
    // CSS Flexbox 1 §9.8: the item is stretched to the container's definite
    // 100px, so layout resolves `top: 10%` of an in-flow child to 10px.
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let flex = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:flex;height:100px"),
    );
    let item = document.append_element(
        Some(flex),
        "div",
        taffy::Style::default(),
        Some("display:block;width:50px"),
    );
    let child = document.append_element(
        Some(item),
        "div",
        taffy::Style::default(),
        Some("float:left;position:relative;top:10%;width:5px;height:5px"),
    );
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[child], child),
        (0.0, 10.0)
    );
}

/// Lays out a body holding a chain of nested elements, the first styled
/// `outer` and the second `inner`, with a child of style `child` in the
/// innermost one; returns the painter's offset for the child.
fn nested_offset(outer: &str, inner: &str, child: &str) -> (f32, f32) {
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let outer = document.append_element(Some(body), "div", taffy::Style::default(), Some(outer));
    let inner = document.append_element(Some(outer), "span", taffy::Style::default(), Some(inner));
    let child = document.append_element(Some(inner), "div", taffy::Style::default(), Some(child));
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    used_relative_offset(&document, &cascade.computed[child], child)
}

#[test]
fn an_unstretched_flex_item_keeps_an_indefinite_height() {
    // CSS Flexbox 1 §9.8: only a stretched item's cross size is definite, so
    // `top: 50%` counts as `auto`, as it does for an in-flow child.
    assert_eq!(
        nested_offset(
            "display:flex;align-items:flex-start;height:100px",
            "display:block;width:50px",
            "float:left;position:relative;width:5px;height:5px;top:50%"
        ),
        (0.0, 0.0)
    );
}

#[test]
fn insets_give_an_absolutely_positioned_block_a_definite_height() {
    // CSS Position 3 §4.1: `top: 0; bottom: 0` fixes the height at 300px.
    assert_eq!(
        nested_offset(
            "display:block;position:relative;height:300px",
            "display:block;position:absolute;top:0;bottom:0;width:50px",
            "float:left;position:relative;width:5px;height:5px;top:10%"
        ),
        (0.0, 30.0)
    );
}

#[test]
fn a_blockified_inline_flex_item_is_a_containing_block() {
    // CSS Display 3 §2.7: the `inline` item is blockified to 100px by 50px.
    assert_eq!(
        nested_offset(
            "display:flex;width:200px;height:100px",
            "display:inline;width:100px;height:50px",
            "float:left;position:relative;width:5px;height:5px;left:50%;top:50%"
        ),
        (50.0, 25.0)
    );
}

#[test]
fn a_spanner_keeps_its_own_definite_height() {
    // Multicol lays a spanner out with no height of its own to impose, so
    // its `height: 100px` is the percentage basis.
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let multicol = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;width:200px;column-count:2;column-gap:20px"),
    );
    let spanner = document.append_element(
        Some(multicol),
        "div",
        taffy::Style::default(),
        Some("display:block;column-span:all;height:100px"),
    );
    let child = document.append_element(
        Some(spanner),
        "div",
        taffy::Style::default(),
        Some("float:left;position:relative;width:5px;height:5px;top:50%"),
    );
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[child], child),
        (0.0, 50.0)
    );
}

#[test]
fn an_anonymous_table_cell_is_a_containing_block() {
    // CSS Tables 3 §3.9: the floated row child is wrapped in a 50px
    // anonymous cell beside a 150px cell, so `left: 50%` is 25px.
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let table = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:table;width:200px;border-spacing:0"),
    );
    let row = document.append_element(
        Some(table),
        "div",
        taffy::Style::default(),
        Some("display:table-row"),
    );
    let child = document.append_element(
        Some(row),
        "div",
        taffy::Style::default(),
        Some("float:left;position:relative;width:50px;height:5px;left:50%"),
    );
    document.append_element(
        Some(row),
        "div",
        taffy::Style::default(),
        Some("display:table-cell;width:150px"),
    );
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[child], child),
        (25.0, 0.0)
    );
}

#[test]
fn a_table_cell_resolves_against_its_table() {
    // CSS Tables 3 §3.5: the cell sits in the 200px by 100px table's grid,
    // not in its row.
    let mut document = Document::new();
    let body = document.append_element(
        Some(0),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let table = document.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:table;width:200px;height:100px;border-spacing:0"),
    );
    let row = document.append_element(
        Some(table),
        "div",
        taffy::Style::default(),
        Some("display:table-row"),
    );
    let cell = document.append_element(
        Some(row),
        "div",
        taffy::Style::default(),
        Some("display:table-cell;position:relative;left:50%;top:50%"),
    );
    document.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    crate::layout_single_page(&mut document, &cascade, raikiri_traits::PageBox::A4).unwrap();
    assert_eq!(
        used_relative_offset(&document, &cascade.computed[cell], cell),
        (100.0, 50.0)
    );
}

#[test]
fn layout_precedence_takes_top_in_every_writing_mode() {
    // The painter takes the block-start `bottom` of an RTL `vertical-rl`
    // block; taffy's block layout applies `top` whatever the writing mode.
    let (document, cascade, child) = child_in(
        "display:block;width:200px;height:100px;writing-mode:vertical-rl;direction:rtl",
        "float:left;position:relative;top:7px;bottom:3px",
    );
    let cv = &cascade.computed[child];
    assert_eq!(
        used_relative_insets(&document, cv, child, Precedence::WritingMode).1,
        Some(-3.0)
    );
    assert_eq!(
        used_relative_insets(&document, cv, child, Precedence::LeftTop).1,
        Some(7.0)
    );
}
