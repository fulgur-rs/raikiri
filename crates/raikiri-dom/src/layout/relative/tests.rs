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
        used_relative_insets(&document, &cascade.computed[child], child),
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
