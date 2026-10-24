use super::*;
use crate::layout::test_support::page_box_800x600;
use raikiri_layout_geometry::Point;
use raikiri_style::{build_rule_tree, cascade};

#[test]
fn percentage_width_under_page_positioned_box_uses_its_owner_page() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    document.append_element(Some(html), "div", Some("display:block;height:20px"));
    let body = document.append_element(Some(html), "body", Some("display:block"));
    let positioned = document.append_element(
        Some(body),
        "div",
        Some("position:absolute;top:90px;left:0;right:0;height:20px"),
    );
    let child = document.append_element(
        Some(positioned),
        "div",
        Some("display:block;width:50%;height:10px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let mut page_box = page_box_800x600();
    page_box.height = 100.0;

    let pages = layout_pages_with_options(
        &mut document,
        &cascade,
        page_box,
        crate::PageLayoutOptions {
            page_steps: &[],
            page_widths: &[800.0, 400.0],
            control: Some(&PageLayoutControl::default()),
            ..Default::default()
        },
    )
    .expect("paged layout");

    assert_eq!(pages.len(), 2);
    assert_eq!(document.layout_origin_of(child).unwrap().y, 90.0);
    assert_eq!(document.nodes[child].layout.size.width, 400.0);
    assert_eq!(
        document.layout_offset_of(child),
        Some(document.nodes[child].layout.location)
    );
    assert_eq!(current_abs_y(&document, child), 90.0);

    let local_offset = document.nodes[child].layout.location;
    let coordinate = document.oof_coordinates.get(&positioned).unwrap();
    assert!(coordinate.initial_owner_origin.is_some());
    assert!(coordinate.initial_static_origin.is_some());
    document.begin_oof_layout();
    let coordinate = document.oof_coordinates.get(&positioned).unwrap();
    assert_eq!(coordinate.initial_owner_origin, None);
    assert_eq!(coordinate.initial_static_origin, None);
    assert_eq!(document.layout_offset_of(child), Some(local_offset));

    document.finalize_oof_layout();
    assert_eq!(document.layout_origin_of(child).unwrap().y, 90.0);
}

#[test]
fn vertical_pagination_keeps_an_absolute_box_at_the_page_origin() {
    for mode in ["vertical-rl", "sideways-rl"] {
        let mut document = Document::new();
        let html = document.append_element(
            Some(0),
            "html",
            Some(&format!("display:block;writing-mode:{mode}")),
        );
        let body = document.append_element(Some(html), "body", Some("display:block;margin:20px"));
        let positioned = document.append_element(
            Some(body),
            "div",
            Some("position:absolute;left:0;top:0;width:200px;height:50px"),
        );
        // Two blocks that cannot share a page make the document paginate.
        for width in [600, 800] {
            document.append_element(
                Some(body),
                "div",
                Some(&format!(
                    "display:block;width:{width}px;height:100px;break-inside:avoid"
                )),
            );
        }
        let rules = build_rule_tree(&document);
        let cascade = cascade(&document, &rules).expect("cascade");

        let pages = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box_800x600(),
            crate::PageLayoutOptions {
                page_steps: &[],
                page_widths: &[],
                control: Some(&PageLayoutControl::default()),
                ..Default::default()
            },
        )
        .expect("paged layout");

        assert!(pages.len() > 1, "{mode}");
        // The containing block is the initial one, so the page area holds the
        // box whatever the writing mode of the flow it was found in.
        assert_eq!(
            document.nodes[positioned].layout.location,
            Point { x: 0.0, y: 0.0 },
            "{mode}"
        );
        let origin = document.layout_origin_of(positioned).unwrap();
        assert_eq!((origin.x, origin.y), (0.0, 0.0), "{mode}");
    }
}

#[test]
fn a_box_in_a_filtered_inline_on_a_later_page_is_placed_like_one_in_a_relative_inline() {
    // The paragraph starts the second 100px page. The padding box of "bb" is
    // x 50 to 70 and y 100 to 110, so 50% of it is (60, 105).
    for (outer_css, child_css) in [
        ("display:inline;position:relative", "position:absolute"),
        ("display:inline;filter:blur(1px)", "position:absolute"),
        ("display:inline;filter:blur(1px)", "position:fixed"),
    ] {
        let mut document = Document::new();
        let html = document.append_element(Some(0), "html", Some("display:block"));
        let body = document.append_element(
            Some(html),
            "body",
            Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
        );
        document.append_element(Some(body), "div", Some("display:block;height:100px"));
        let paragraph =
            document.append_element(Some(body), "div", Some("display:block;width:200px"));
        document.append_text(paragraph, "aaaa ");
        let outer = document.append_element(Some(paragraph), "span", Some(outer_css));
        document.append_text(outer, "bb");
        let child = document.append_element(
            Some(outer),
            "span",
            Some(&format!(
                "{child_css};left:50%;top:50%;width:10px;height:10px"
            )),
        );
        let rules = build_rule_tree(&document);
        let cascade = cascade(&document, &rules).expect("cascade");
        crate::layout::test_support::with_ahem(&mut document);
        let mut page_box = page_box_800x600();
        page_box.height = 100.0;

        let pages = layout_pages_with_options(
            &mut document,
            &cascade,
            page_box,
            crate::PageLayoutOptions {
                page_steps: &[],
                page_widths: &[],
                control: Some(&PageLayoutControl::default()),
                ..Default::default()
            },
        )
        .expect("paged layout");

        assert_eq!(pages.len(), 2, "{outer_css} {child_css}");
        let origin = document.layout_origin_of(child).unwrap();
        assert_eq!(
            (origin.x, origin.y),
            (60.0, 105.0),
            "{outer_css} {child_css}"
        );
    }
}
