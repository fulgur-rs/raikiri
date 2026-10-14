use super::*;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

fn fixture(page_css: &str, html_style: &str) -> (Document, CascadeResult) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some(html_style));
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(style, page_css);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    document.append_text(body, "hi");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    (document, cascade)
}

fn page(width: f32, height: f32) -> PageBox {
    let mut page_box = PageBox::A4;
    page_box.width = width;
    page_box.height = height;
    page_box
}

#[test]
fn definite_root_sits_in_the_page_area_inside_its_margins() {
    let (document, cascade) = fixture(
        "@page { size: 400px 300px; margin: 10px 20px; padding: 5px; border: 1px solid }",
        "display: block; margin: 50px 30px; height: 100px; width: 200px; \
         padding: 4px 10%; border: 2px solid",
    );
    let root = root_box(&document, &cascade, page(400.0, 300.0), 0.0, 1).expect("root box");
    assert_eq!(root.node, NodeId(1));
    // The page area starts after the 20px / 10px margins and the 6px of
    // page border and padding; it is 400 - 40 - 12 = 348px wide.
    assert_eq!(root.border, PaintInsets::new(2.0, 2.0, 2.0, 2.0));
    assert!((root.padding.left - 34.8).abs() < 1e-3);
    assert_eq!(root.padding.top, 4.0);
    let rect = root.border_box;
    assert_eq!((rect.x, rect.y), (20.0 + 6.0 + 30.0, 10.0 + 6.0 + 50.0));
    // Content-box sizes grow by the padding and border.
    assert!((rect.width - (200.0 + 2.0 * 34.8 + 4.0)).abs() < 1e-3);
    assert_eq!(rect.height, 100.0 + 8.0 + 4.0);
    assert_eq!(root.transform, None);

    // A later page sees the same box moved up by the flow before it.
    let later = root_box(&document, &cascade, page(400.0, 300.0), 268.0, 2).expect("root box");
    assert_eq!(later.border_box.y, rect.y - 268.0);
    assert_eq!(later.border_box.height, rect.height);
}

#[test]
fn auto_root_fills_the_flow_of_every_page() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; margin: 10px; padding: 5px",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 3).expect("root box");
    let rect = root.border_box;
    assert_eq!((rect.x, rect.y), (10.0, 10.0));
    assert_eq!(rect.width, 180.0);
    assert_eq!(rect.height, 3.0 * 100.0 - 20.0);
}

#[test]
fn root_limits_clamp_the_size_as_box_sizing_reads_them() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; box-sizing: border-box; padding: 5px; max-height: 50px; \
         min-width: 300px; width: 100px",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 4).expect("root box");
    // Border-box limits include the padding.
    assert_eq!(root.border_box.height, 50.0);
    assert_eq!(root.border_box.width, 300.0);

    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; height: 10px; min-height: 40px; padding: 5px",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.height, 50.0);
}

#[test]
fn root_transform_turns_about_its_origin_on_the_box() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; height: 40px; width: 100px; margin: 10px; \
         transform: scale(2); transform-origin: 0 0",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    // Scaling about the box's top-left corner (10, 10) keeps that corner.
    assert_eq!(
        root.transform,
        Some(PaintTransform::new(2.0, 0.0, 0.0, 2.0, -10.0, -10.0))
    );

    // The default origin is the box's center, and percentages of
    // `translate` resolve against the box.
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; height: 40px; width: 100px; transform: translateX(10%)",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(
        root.transform,
        Some(PaintTransform::new(1.0, 0.0, 0.0, 1.0, 10.0, 0.0))
    );
}

#[test]
fn no_root_element_has_no_root_box() {
    let mut document = Document::new();
    let div = document.append_element(Some(0), "div", Style::default(), None::<&str>);
    document.append_text(div, "hi");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    assert_eq!(root_box(&document, &cascade, PageBox::A4, 0.0, 1), None);
}

#[test]
fn auto_margins_center_a_root_of_definite_width() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; width: 100px; margin: 0 auto",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, 50.0);

    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; width: 100px; margin: 0 30px 0 auto",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, 70.0);

    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; width: 100px; margin: 0 auto 0 30px",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, 30.0);
}

#[test]
fn auto_width_root_keeps_its_width_limits() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; max-width: 80px; padding: 0 10px",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.width, 100.0);
}

#[test]
fn auto_height_root_reaches_at_least_the_page_painted() {
    // A caller that does not know the number of pages still sees the root
    // run down through the page it paints.
    let (document, cascade) = fixture("@page { size: 200px 100px; margin: 0 }", "display: block");
    let root = root_box(&document, &cascade, page(200.0, 100.0), 200.0, 1).expect("root box");
    assert_eq!(root.border_box.y, -200.0);
    assert_eq!(root.border_box.height, 300.0);
}

#[test]
fn a_root_without_a_box_has_no_root_box() {
    let (document, cascade) = fixture("", "display: none");
    assert_eq!(root_box(&document, &cascade, PageBox::A4, 0.0, 1), None);
}

#[test]
fn auto_margins_center_a_clamped_auto_width() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; max-width: 100px; margin: 0 auto",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!((root.border_box.x, root.border_box.width), (50.0, 100.0));
}

#[test]
fn auto_margins_are_zero_on_an_overflowing_root() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; width: 300px; margin: 0 auto",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, 0.0);

    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; width: 300px; margin: 0 10px 0 auto",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, 0.0);
}

#[test]
fn an_over_constrained_rtl_root_keeps_its_right_margin() {
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; direction: rtl; width: 100px; margin: 0 20px 0 10px",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, 80.0);

    // An overflowing RTL root with an auto margin also ends at its right
    // margin.
    let (document, cascade) = fixture(
        "@page { size: 200px 100px; margin: 0 }",
        "display: block; direction: rtl; width: 300px; margin: 0 10px 0 auto",
    );
    let root = root_box(&document, &cascade, page(200.0, 100.0), 0.0, 1).expect("root box");
    assert_eq!(root.border_box.x, -110.0);
}
