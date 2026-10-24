//! Resolved overflow geometry consumed through the public page views.

use raikiri_html::{
    ClipKind, DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintClip,
    PaintEvent, PaintRect, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn laid_out(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}}body{{margin:0;font:12px/12px Ahem}}{css}</style><body id='body'>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let status = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap();
    let LayoutStatus::Completed(result) = status else {
        panic!("layout must complete");
    };
    result
}

fn clip(result: &DocumentLayout, page_index: u32, id: &str) -> Option<PaintClip> {
    let page = result.page(page_index).unwrap();
    page.fragments()
        .find(|fragment| page.dom().attr(fragment.node(), "id") == Some(id))
        .expect("fixture has the requested fragment")
        .overflow_clip()
}

#[test]
fn rectangular_clip_constructor_keeps_both_axes_and_square_corners() {
    let rect = PaintRect::new(1.0, 2.0, 30.0, 40.0);
    let clip = PaintClip::new(rect);
    assert_eq!(clip.rect, rect);
    assert!(clip.clip_x && clip.clip_y);
    assert_eq!(clip.corner_radii, None);
}

#[test]
fn overflow_clip_resolves_asymmetric_padding_edge_ellipses() {
    let result = laid_out(
        "<div id='box'></div>",
        "#box{box-sizing:border-box;width:100px;height:60px;overflow:hidden;border-style:solid;border-width:6px 12px 10px 8px;border-radius:40px 30px 20px 10px / 20px 18px 16px 14px}",
    );
    let clip = clip(&result, 0, "box").unwrap();
    assert_eq!(clip.rect, PaintRect::new(8.0, 6.0, 80.0, 44.0));
    assert!(clip.clip_x && clip.clip_y);
    assert_eq!(
        clip.corner_radii,
        Some([[32.0, 14.0], [18.0, 12.0], [8.0, 6.0], [2.0, 4.0]])
    );
    let page = result.page(0).unwrap();
    let event_clip = page
        .paint_order()
        .into_iter()
        .find_map(|event| match event {
            PaintEvent::PushClip(clip, ClipKind::Overflow) => Some(clip),
            _ => None,
        });
    assert_eq!(event_clip, Some(clip));
}

#[test]
fn overflow_percentages_use_the_outer_box_before_insetting() {
    let result = laid_out(
        "<div id='box'></div>",
        "#box{box-sizing:border-box;width:100px;height:60px;overflow:hidden;border-style:solid;border-width:6px 12px 10px 8px;border-radius:80% / 70%}",
    );
    assert_eq!(
        clip(&result, 0, "box").unwrap().corner_radii,
        Some([[42.0, 20.25], [38.0, 20.25], [38.0, 16.25], [42.0, 16.25]])
    );
}

#[test]
fn cropped_padding_curves_keep_radii_larger_than_the_inner_box() {
    let result = laid_out(
        "<div id='box'></div>",
        "#box{box-sizing:border-box;width:100px;height:100px;overflow:hidden;border:40px solid blue;border-radius:100px 0 0 0}",
    );
    let clip = clip(&result, 0, "box").unwrap();
    assert_eq!(clip.rect, PaintRect::new(40.0, 40.0, 20.0, 20.0));
    assert_eq!(
        clip.corner_radii,
        Some([[60.0, 60.0], [0.0; 2], [0.0; 2], [0.0; 2]])
    );
}

#[test]
fn page_cuts_preserve_the_whole_percentage_clip_shape() {
    let result = laid_out(
        "<div id='box'></div>",
        "#box{box-sizing:border-box;width:100px;height:150px;overflow:hidden;border:4px solid blue;border-radius:50%}",
    );
    assert_eq!(result.pages().count(), 2);
    for (index, y) in [(0, 4.0), (1, -96.0)] {
        let clip = clip(&result, index, "box").unwrap();
        assert_eq!(clip.rect, PaintRect::new(4.0, y, 92.0, 142.0));
        assert_eq!(clip.corner_radii, Some([[46.0, 71.0]; 4]));
    }
}

#[test]
fn an_open_axis_has_no_corner_curve() {
    for (overflow, axes) in [
        ("overflow-x:visible;overflow-y:clip", (false, true)),
        ("overflow-x:clip;overflow-y:visible", (true, false)),
    ] {
        let result = laid_out(
            "<div id='box'></div>",
            &format!("#box{{width:100px;height:60px;border-radius:40px;{overflow}}}"),
        );
        let clip = clip(&result, 0, "box").unwrap();
        assert_eq!((clip.clip_x, clip.clip_y), axes);
        assert_eq!(clip.corner_radii, None);
    }
}

#[test]
fn viewport_propagation_does_not_clip_the_body_box() {
    let propagated = laid_out("<div style='height:20px'></div>", "body{overflow:hidden}");
    assert_eq!(clip(&propagated, 0, "body"), None);
    let local = laid_out(
        "<div style='height:20px'></div>",
        "html{overflow:hidden}body{overflow:hidden}",
    );
    assert!(clip(&local, 0, "body").is_some());
}

#[test]
fn inline_boxes_do_not_clip_their_descendants() {
    let result = laid_out(
        "<span id='box'>ABC</span>",
        "#box{overflow:hidden;border-radius:20px}",
    );
    assert_eq!(clip(&result, 0, "box"), None);
}

#[test]
fn a_fixed_clip_repeats_at_the_same_page_coordinates() {
    let result = laid_out(
        "<div style='height:150px'></div><div id='box'></div>",
        "#box{position:fixed;left:5px;top:7px;box-sizing:border-box;width:20px;height:20px;overflow:hidden;border:2px solid blue;border-radius:8px}",
    );
    assert_eq!(result.pages().count(), 2);
    for index in 0..2 {
        let clip = clip(&result, index, "box").unwrap();
        assert_eq!(clip.rect, PaintRect::new(7.0, 9.0, 16.0, 16.0));
        assert_eq!(clip.corner_radii, Some([[6.0; 2]; 4]));
    }
}

#[test]
fn an_off_page_parent_clip_keeps_its_child_in_paint_order() {
    for height in [0, 20] {
        let result = laid_out(
            "<div id='parent'><div id='child'></div></div><div style='height:150px'></div>",
            &format!(
                "#parent{{width:40px;height:{height}px;overflow-x:clip;overflow-y:visible}}#child{{width:80px;height:150px;background:green}}"
            ),
        );
        let page = result.page(1).unwrap();
        assert!(
            page.fragments()
                .all(|fragment| page.dom().attr(fragment.node(), "id") != Some("parent"))
        );
        let parent_clip = page
            .overflow_clips()
            .find(|clip| page.dom().attr(clip.node, "id") == Some("parent"))
            .expect("an off-page ancestor still clips its child's closed axis");
        assert_eq!(
            parent_clip.border_box,
            PaintRect::new(0.0, -100.0, 40.0, height as f32)
        );
        assert_eq!(parent_clip.clip.rect, parent_clip.border_box);
        assert!(parent_clip.clip.clip_x && !parent_clip.clip.clip_y);
        let events = page.paint_order();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, PaintEvent::Box(fragment)
            if page.dom().attr(fragment.node(), "id") == Some("child")))
        );
        assert!(events.iter().any(
            |event| matches!(event, PaintEvent::PushClip(clip, ClipKind::Overflow)
            if *clip == parent_clip.clip)
        ));
    }
}

/// A clipping box as tall as the page, holding a box whose lines run far below
/// the page: the part past the clip is never visible.
fn spilling_lines(clip_css: &str) -> DocumentLayout {
    let lines = "x<br>".repeat(30);
    laid_out(
        &format!(
            "<div id='clip'><div style='height:20px'></div><div style='height:30px'>{lines}</div></div>"
        ),
        &format!("#clip{{height:100px;{clip_css}}}"),
    )
}

/// The tops of the text lines painted inside the clip of `id` on page
/// `page_index`, rounded to whole pixels.
fn visible_line_tops(result: &DocumentLayout, page_index: u32, id: &str) -> Vec<i32> {
    let window = clip(result, page_index, id).unwrap().rect;
    let mut tops: Vec<i32> = result
        .page(page_index)
        .unwrap()
        .text_runs()
        .iter()
        .map(|run| (run.origin.1 - run.ascent).round() as i32)
        .filter(|&top| top as f32 + 1.0 > window.y && (top as f32) < window.y + window.height)
        .collect();
    tops.sort_unstable();
    tops
}

#[test]
fn content_past_a_clip_adds_no_page() {
    for overflow in [
        "overflow:clip",
        "overflow:hidden",
        "overflow:auto",
        "overflow:scroll",
        "overflow:clip hidden",
    ] {
        let result = spilling_lines(overflow);
        assert_eq!(result.page_count(), 1, "{overflow}");
    }
}

#[test]
fn content_past_a_box_that_does_not_clip_the_block_axis_adds_pages() {
    for overflow in ["overflow:visible", "overflow-x:clip;overflow-y:visible"] {
        let result = spilling_lines(overflow);
        assert!(result.page_count() > 1, "{overflow}");
    }
}

#[test]
fn a_paragraph_running_over_a_clip_edge_adds_no_page() {
    let words = "x ".repeat(60);
    let result = laid_out(
        &format!("<div id='clip'><p>{words}</p></div>"),
        "#clip{height:100px;overflow:hidden}p{width:12px;margin:0}",
    );
    assert_eq!(result.page_count(), 1);
}

#[test]
fn boxes_past_a_clip_edge_add_no_page() {
    let rows = "<tr><td>x</td></tr>".repeat(30);
    let items = "<div>x</div>".repeat(30);
    for (body, css) in [
        (
            format!("<table>{rows}</table>"),
            "table{border-collapse:collapse}",
        ),
        (
            format!("<div id='column'>{items}</div>"),
            "#column{display:flex;flex-direction:column}",
        ),
        (
            "<div style='height:400px;background:green'></div>".to_owned(),
            "",
        ),
    ] {
        let result = laid_out(
            &format!("<div id='clip'>{body}</div>"),
            &format!("#clip{{height:100px;overflow:hidden}}{css}"),
        );
        assert_eq!(result.page_count(), 1, "{css}");
    }
}

#[test]
fn a_clip_taller_than_a_page_still_breaks_across_pages() {
    let lines = "x<br>".repeat(30);
    let result = laid_out(
        &format!("<div id='clip'>{lines}</div>"),
        "#clip{height:150px;overflow:hidden}",
    );
    assert_eq!(result.page_count(), 2);
    assert_eq!(clip(&result, 0, "clip").unwrap().rect.height, 150.0);
}

#[test]
fn the_flow_after_a_clip_starts_where_the_clip_ends() {
    let lines = "x<br>".repeat(30);
    let result = laid_out(
        &format!("<div id='clip'>{lines}</div><p>AFTER</p>"),
        "#clip{height:100px;overflow:hidden}p{margin:0}",
    );
    assert_eq!(result.page_count(), 2);
    let runs = result.page(1).unwrap().text_runs();
    let after = runs
        .iter()
        .find(|run| run.text == "AFTER")
        .expect("the paragraph after the clip is on the second page");
    assert!((after.origin.1 - after.ascent).abs() < 0.01);
}

#[test]
fn a_clip_pushed_down_by_the_flow_keeps_its_visible_lines() {
    let lines = "x<br>".repeat(30);
    // The avoiding box does not fit below the spacer, so it and the clip after
    // it move down by a page break; the clip then crosses the next one.
    let result = laid_out(
        &format!(
            "<div style='height:60px'></div><div style='height:60px;break-inside:avoid'></div><div id='clip'>{lines}</div>"
        ),
        "#clip{height:50px;overflow:hidden}",
    );
    assert_eq!(result.page_count(), 3);
    assert_eq!(visible_line_tops(&result, 1, "clip"), [60, 72, 84]);
    assert_eq!(visible_line_tops(&result, 2, "clip"), [-4, 8]);
}

#[test]
fn columns_in_a_clip_keep_their_pages() {
    let lines = "x<br>".repeat(30);
    let more_lines = "x<br>".repeat(60);
    for (body, css, pages) in [
        // The columns are taller than a page and are all visible.
        (
            lines.clone(),
            "#clip{height:300px;overflow:hidden;column-count:2}",
            3,
        ),
        // Columns inside a clip show up to its edge and no further.
        (
            format!("<div style='column-count:2'>{more_lines}</div>"),
            "#clip{height:300px;overflow:hidden}",
            3,
        ),
        (
            format!("<div style='column-count:2'>{more_lines}</div>"),
            "#clip{height:150px;overflow:hidden}",
            2,
        ),
        (
            format!("<div style='column-count:3'>{lines}</div>"),
            "#clip{height:100px;overflow:hidden}",
            1,
        ),
    ] {
        let result = laid_out(&format!("<div id='clip'>{body}</div>"), css);
        assert_eq!(result.page_count(), pages, "{css}");
    }
}

#[test]
fn a_forced_break_past_a_clip_edge_still_breaks() {
    let result = laid_out(
        "<div id='clip'><div style='height:120px'></div><div style='break-before:page'>X</div></div>",
        "#clip{height:100px;overflow:hidden}",
    );
    assert_eq!(result.page_count(), 2);
}

#[test]
fn a_positioned_clip_clips_its_own_content() {
    for position in ["absolute", "relative"] {
        let result = spilling_lines(&format!("position:{position};overflow:hidden"));
        assert_eq!(result.page_count(), 1, "{position}");
    }
}

#[test]
fn an_inline_block_clips_its_own_content() {
    let lines = "x<br>".repeat(30);
    let result = laid_out(
        &format!("<p>text <span id='clip'>{lines}</span></p>"),
        "p{margin:0}#clip{display:inline-block;height:20px;overflow:hidden;vertical-align:top}",
    );
    assert_eq!(result.page_count(), 1);
}
