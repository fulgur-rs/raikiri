//! The margins of an empty child adjoin each other and, when nothing separates
//! them, the top margin of its parent (CSS 2.1, 8.3.1), so they lie outside the
//! parent and add nothing to its height. The boxes are the ones Chromium
//! computes for the same markup (body margin 0, no text).

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Border box `(y, height)` of the element `id` in the standards-mode
/// document made of `body`.
fn boxes(body: &str, id: &str) -> (f32, f32) {
    let html = format!("<!DOCTYPE html><style>body{{margin:0}}</style>{body}");
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    for index in 0..result.page_count() {
        let page = result.page(index).expect("page");
        let dom = page.dom();
        for fragment in page.fragments() {
            if dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                return (rect.y, rect.height);
            }
        }
    }
    panic!("no fragment for {id} in {body}");
}

/// Lays out every `(markup, id, (y, height))` case and reports all that differ.
fn assert_boxes(cases: &[(&str, &str, (f32, f32))]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|&(body, id, expected)| {
            let (y, height) = boxes(body, id);
            ((y - expected.0).abs() >= 0.01 || (height - expected.1).abs() >= 0.01).then(|| {
                format!(
                    "{body}: #{id} expected (y {}, height {}), got (y {y}, height {height})",
                    expected.0, expected.1
                )
            })
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_margins_of_an_empty_child_join_the_top_margin_of_a_padded_parent() {
    assert_boxes(&[
        (
            r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (30.0, 10.0),
        ),
        // A larger bottom margin raises the margin the parent shares, not its
        // height.
        (
            r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:200px"></div></div>"#,
            "p",
            (200.0, 10.0),
        ),
        (
            r#"<div id=p style="border-bottom:5px solid"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (30.0, 5.0),
        ),
        // Every margin of several empty children adjoins.
        (
            r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div><div style="margin-top:10px;margin-bottom:50px"></div></div>"#,
            "p",
            (50.0, 10.0),
        ),
        // So do the margins of an empty grandchild.
        (
            r#"<div id=p style="padding-bottom:10px"><div><div style="margin-top:30px;margin-bottom:20px"></div></div></div>"#,
            "p",
            (30.0, 10.0),
        ),
    ]);
}

#[test]
fn the_shared_margin_collapses_with_what_precedes_the_parent() {
    assert_boxes(&[
        (
            r#"<div style="height:10px"></div><div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (40.0, 10.0),
        ),
        (
            r#"<div id=p style="margin-top:15px;padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (30.0, 10.0),
        ),
    ]);
}

#[test]
fn a_minimum_height_is_not_raised_by_the_margins_that_joined_the_top_margin() {
    assert_boxes(&[
        (
            r#"<div id=p style="box-sizing:border-box;padding-bottom:10px;min-height:50px"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (30.0, 50.0),
        ),
        (
            r#"<div id=p style="padding-bottom:10px;min-height:50px"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (30.0, 60.0),
        ),
    ]);
}

#[test]
fn the_bottom_margin_of_a_child_with_content_stays_inside_the_padding() {
    assert_boxes(&[
        (
            r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px;height:40px"></div></div>"#,
            "p",
            (30.0, 70.0),
        ),
        // An empty child before a child with content adds nothing either.
        (
            r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div><div style="height:40px"></div></div>"#,
            "p",
            (30.0, 50.0),
        ),
    ]);
}

#[test]
fn padding_at_the_top_keeps_the_margins_of_an_empty_child_inside() {
    assert_boxes(&[(
        r#"<div id=p style="padding:10px 0"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
        "p",
        (0.0, 50.0),
    )]);
}

#[test]
fn empty_children_after_content_collapse_their_margins_with_its_bottom_margin() {
    assert_boxes(&[
        (
            r#"<div id=p style="padding-bottom:10px"><div style="height:40px;margin-bottom:20px"></div><div style="margin-top:30px;margin-bottom:5px"></div></div>"#,
            "p",
            (0.0, 80.0),
        ),
        (
            r#"<div id=p style="padding-bottom:10px"><div style="height:40px"></div><div style="margin-top:30px"></div><div style="margin-bottom:50px"></div></div>"#,
            "p",
            (0.0, 100.0),
        ),
        // Negative margins collapse with the positive ones.
        (
            r#"<div id=p style="padding-bottom:10px"><div style="height:40px;margin-bottom:20px"></div><div style="margin-top:-15px;margin-bottom:-5px"></div></div>"#,
            "p",
            (0.0, 55.0),
        ),
        (
            r#"<div id=p style="padding-top:10px;padding-bottom:10px"><div style="height:40px;margin-bottom:20px"></div><div style="margin-bottom:30px"></div></div>"#,
            "p",
            (0.0, 90.0),
        ),
        // The empty child in front joins the top margin of the parent, the
        // ones after the content stay inside.
        (
            r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div><div style="height:40px;margin-bottom:5px"></div><div style="margin-bottom:60px"></div></div>"#,
            "p",
            (30.0, 110.0),
        ),
    ]);
}

#[test]
fn negative_margins_of_an_empty_child_join_the_top_margin_of_the_parent() {
    assert_boxes(&[(
        r#"<div id=p style="padding-bottom:10px"><div style="margin-top:30px;margin-bottom:-10px"></div></div>"#,
        "p",
        (20.0, 10.0),
    )]);
}

#[test]
fn the_margins_of_an_empty_child_stay_inside_a_block_formatting_context() {
    assert_boxes(&[
        (
            r#"<div id=p style="display:flow-root;padding-bottom:10px"><div style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "p",
            (0.0, 40.0),
        ),
        // So do those of an empty grandchild below padding.
        (
            r#"<div id=p style="padding:10px 0"><div><div style="margin-top:30px;margin-bottom:20px"></div></div></div>"#,
            "p",
            (0.0, 50.0),
        ),
    ]);
}

#[test]
fn the_end_margin_of_a_parent_without_padding_collapses_out_of_it() {
    assert_boxes(&[
        (
            r#"<div id=p><div style="height:40px"></div><div style="margin-bottom:30px"></div></div><div id=q style="height:5px"></div>"#,
            "p",
            (0.0, 40.0),
        ),
        (
            r#"<div id=p><div style="height:40px"></div><div style="margin-bottom:30px"></div></div><div id=q style="height:5px"></div>"#,
            "q",
            (70.0, 5.0),
        ),
    ]);
}

#[test]
fn a_minimum_height_keeps_margins_out_of_the_height_unless_it_is_the_content() {
    // The border box is 20px tall, as the minimum says, and the margins of
    // the empty child neither escape nor add to it.
    let a = r#"<div id=p style="box-sizing:border-box;padding-top:10px;min-height:20px"><div style="margin:30px 0 20px"></div></div><div id=q style="height:5px"></div>"#;
    // The content is taller than the minimum: the bottom margin escapes.
    let b = r#"<div id=p style="box-sizing:border-box;min-height:20px"><div style="height:40px;margin-bottom:15px"></div></div><div id=q style="height:5px"></div>"#;
    let c = r#"<div id=p style="box-sizing:border-box;padding-top:10px;min-height:20px"><div style="height:40px;margin-bottom:15px"></div><div style="margin:30px 0 20px"></div></div><div id=q style="height:5px"></div>"#;
    assert_boxes(&[
        (a, "p", (0.0, 20.0)),
        (a, "q", (20.0, 5.0)),
        (b, "p", (0.0, 40.0)),
        (b, "q", (55.0, 5.0)),
        (c, "p", (0.0, 50.0)),
        (c, "q", (80.0, 5.0)),
    ]);
}

#[test]
fn an_empty_child_with_a_zero_height_collapses_through_like_an_auto_one() {
    let padded = r#"<div id=p style="padding-bottom:10px"><div style="height:40px;margin-bottom:15px"></div><div style="height:0;margin:30px 0 20px"></div></div><div id=q style="height:5px"></div>"#;
    let plain = r#"<div id=p><div style="height:40px;margin-bottom:15px"></div><div style="height:0;margin:30px 0 20px"></div></div><div id=q style="height:5px"></div>"#;
    assert_boxes(&[
        (padded, "p", (0.0, 80.0)),
        (padded, "q", (80.0, 5.0)),
        (plain, "p", (0.0, 40.0)),
        (plain, "q", (70.0, 5.0)),
    ]);
}

#[test]
fn margins_that_joined_the_top_margin_do_not_count_as_content_to_align() {
    assert_boxes(&[
        (
            r#"<div id=p style="height:100px;align-content:center"><div id=c style="margin-top:30px;margin-bottom:20px"></div></div>"#,
            "c",
            (65.0, 0.0),
        ),
        (
            r#"<div id=p style="height:100px;align-content:center"><div id=c style="margin-top:30px;margin-bottom:20px"></div><div id=d style="height:10px"></div></div>"#,
            "d",
            (60.0, 10.0),
        ),
    ]);
}
