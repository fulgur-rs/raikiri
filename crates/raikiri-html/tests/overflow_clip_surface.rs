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
fn an_overflow_clip_margin_keeps_the_pages_of_the_content_it_reveals() {
    // The 100px outset puts the clip edge 100px below the box, so the lines in
    // that band are visible and take the second page.
    let result = spilling_lines("overflow:clip;overflow-clip-margin:100px");
    assert_eq!(result.page_count(), 2);
    // A scroll container never clips beyond its padding box.
    let result = spilling_lines("overflow:hidden;overflow-clip-margin:100px");
    assert_eq!(result.page_count(), 1);
}

#[test]
fn an_overflow_clip_margin_has_no_effect_while_the_other_axis_is_visible() {
    // Chromium puts the overflow clip edge at the padding box when only one
    // axis clips, whatever `overflow-clip-margin` says: 1 page for a 60px box
    // with a 60px margin around 30 lines, 2 for a 150px one.
    for overflow in [
        "overflow-y:clip",
        "overflow-y:clip;overflow-x:visible",
        "overflow:visible clip",
    ] {
        for margin in [
            "overflow-clip-margin:100px",
            "overflow-clip-margin:border-box 100px",
            "overflow-clip-margin:content-box 20px",
        ] {
            let result = spilling_lines(&format!(
                "{overflow};{margin};height:80px;border-bottom:5px solid"
            ));
            let edge = clip(&result, 0, "clip").unwrap();
            assert_eq!(result.page_count(), 1, "{overflow};{margin}");
            assert!(edge.clip_y && !edge.clip_x, "{overflow};{margin}");
            assert_eq!(
                (edge.rect.y, edge.rect.height),
                (0.0, 80.0),
                "{overflow};{margin}"
            );
        }
    }
    // The margin reaches the clip once both axes clip.
    let result = spilling_lines("overflow:clip;overflow-clip-margin:content-box 100px");
    assert_eq!(result.page_count(), 2);
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
    // it move down by a page break; the clip then crosses the next one, and
    // the line that would cross it starts the next page, where the clip shows
    // only its top. Chromium prints the same lines.
    let result = laid_out(
        &format!(
            "<div style='height:60px'></div><div style='height:60px;break-inside:avoid'></div><div id='clip'>{lines}</div>"
        ),
        "#clip{height:50px;overflow:hidden}",
    );
    assert_eq!(result.page_count(), 3);
    assert_eq!(visible_line_tops(&result, 1, "clip"), [60, 72, 84]);
    assert_eq!(visible_line_tops(&result, 2, "clip"), [0]);
}

/// A spacer, then a fixed-height paragraph of 20px `X` words in 30px lines
/// that clips them, then a 30px block holding `Z`. Returns the page count and
/// the painted glyph tops on each page, as `(page, glyph, top)`: an `X` line
/// is listed only where the paragraph's clip shows part of it.
fn clipped_paragraph_glyph_tops(
    spacer: u32,
    height: u32,
    words: usize,
    style: &str,
) -> (u32, Vec<(u32, char, i32)>) {
    let words = "X ".repeat(words);
    let result = laid_out(
        &format!(
            "<div style='height:{spacer}px'></div><div id='c' style='width:20px;height:{height}px;overflow:hidden;{style}'>{words}</div><div style='height:30px'>Z</div>"
        ),
        "body{font:20px/30px Ahem}",
    );
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        let window = page
            .fragments()
            .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("c"))
            .and_then(|fragment| fragment.overflow_clip())
            .map(|clip| clip.rect);
        for run in page.text_runs() {
            let top = run.origin.1 - run.ascent;
            let Some(glyph) = run.text.trim().chars().next() else {
                continue;
            };
            if glyph == 'X'
                && !window.is_some_and(|window| {
                    top < window.y + window.height - 0.01 && top + 20.0 > window.y + 0.01
                })
            {
                continue;
            }
            tops.push((index, glyph, top.round() as i32));
        }
    }
    tops.sort_unstable();
    (result.page_count(), tops)
}

#[test]
fn lines_past_the_clip_of_a_fixed_height_paragraph_add_no_page() {
    // The fourth line runs over the clip edge at the bottom of a box that ends
    // on the first page, so it is no break point and the lines past it stay
    // hidden: only the block after the box moves to the next page, as in
    // Chromium.
    for height in [90, 100] {
        for style in ["", "orphans:2;widows:2", "orphans:1;widows:1"] {
            assert_eq!(
                clipped_paragraph_glyph_tops(0, height, 6, style),
                (
                    2,
                    vec![(0, 'X', 5), (0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)]
                ),
                "{height} {style}"
            );
        }
    }
}

#[test]
fn a_clipped_paragraph_crossing_the_page_keeps_no_widows() {
    // The box crosses the page and clips its fifth line, so the fourth line
    // moves to the next page alone: Chromium keeps no widows once lines run
    // past the clip.
    assert_eq!(
        clipped_paragraph_glyph_tops(0, 130, 6, "orphans:2;widows:2"),
        (
            2,
            vec![
                (0, 'X', 5),
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Z', 35)
            ]
        )
    );
    // A box ending past the page moves the line that crosses it, as when
    // nothing is clipped.
    assert_eq!(
        clipped_paragraph_glyph_tops(30, 90, 6, "orphans:2;widows:2").1,
        [(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 25)]
    );
}

#[test]
fn a_clip_above_the_first_line_bottom_leaves_the_line_in_place() {
    // The clip shows only the top of the first line and the box ends on the
    // first page: no line moves, and the block after the box starts the next
    // page in Chromium.
    for style in ["", "orphans:2;widows:2"] {
        let (pages, tops) = clipped_paragraph_glyph_tops(80, 10, 3, style);
        assert_eq!(pages, 2, "{style}");
        assert_eq!(tops, [(0, 'X', 85), (1, 'Z', 5)], "{style}");
    }
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

#[test]
fn inline_svg_defaults_clip_the_viewport_and_author_styles_can_open_it() {
    let body = "<svg id=box width=20 height=10><rect width=40 height=20 fill=red /></svg>";
    let default = laid_out(body, "svg{display:block}");
    let page = default.page(0).unwrap();
    let fragment = page
        .fragments()
        .find(|f| page.dom().attr(f.node(), "id") == Some("box"))
        .unwrap();
    let values = page.computed(fragment.node()).unwrap();
    assert_eq!(
        values.overflow.x,
        raikiri_style::property::OverflowValue::Hidden
    );
    assert_eq!(
        values.overflow.y,
        raikiri_style::property::OverflowValue::Hidden
    );
    let expected = PaintClip::new(PaintRect::new(0.0, 0.0, 20.0, 10.0));
    assert_eq!(clip(&default, 0, "box"), Some(expected));
    assert!(page.paint_order().into_iter().any(|event| matches!(event, PaintEvent::PushClip(value, ClipKind::Overflow) if value == expected)));

    let attribute = body.replace("id=box", "id=box overflow=visible");
    for visible in [
        laid_out(body, "svg{display:block;overflow:visible}"),
        laid_out(&attribute, "svg{display:block}"),
    ] {
        let page = visible.page(0).unwrap();
        let fragment = page
            .fragments()
            .find(|f| page.dom().attr(f.node(), "id") == Some("box"))
            .unwrap();
        let values = page.computed(fragment.node()).unwrap();
        assert_eq!(
            values.overflow.x,
            raikiri_style::property::OverflowValue::Visible
        );
        assert_eq!(
            values.overflow.y,
            raikiri_style::property::OverflowValue::Visible
        );
        assert_eq!(clip(&visible, 0, "box"), None);
        assert!(
            !page
                .paint_order()
                .into_iter()
                .any(|event| matches!(event, PaintEvent::PushClip(_, ClipKind::Overflow)))
        );
    }
}

/// Thirty lines of 12px in an absolutely positioned box placed by `style`.
fn absolute_lines(style: &str) -> String {
    format!(
        "<div style='position:absolute;left:0;{style}'>{}</div>",
        "x<br>".repeat(30)
    )
}

#[test]
fn lines_of_an_absolute_box_past_the_clip_that_contains_it_add_no_page() {
    // The clip is its containing block, so it hides what the box paints past
    // the edge. Chromium prints these on one page.
    for overflow in ["hidden", "auto", "scroll", "clip"] {
        let result = laid_out(
            &format!("<div id='clip'>{}</div>", absolute_lines("top:0")),
            &format!("#clip{{position:relative;overflow:{overflow};height:50px;width:100px}}"),
        );
        assert_eq!(result.page_count(), 1, "{overflow}");
    }
    // Placed by its bottom edge, or under a transform, which contains it too.
    for (style, css) in [
        (
            "bottom:-250px",
            "#clip{position:relative;overflow:hidden;height:50px;width:100px}",
        ),
        (
            "top:0",
            "#clip{transform:translateX(0);overflow:hidden;height:50px;width:100px}",
        ),
    ] {
        let result = laid_out(
            &format!("<div id='clip'>{}</div>", absolute_lines(style)),
            css,
        );
        assert_eq!(result.page_count(), 1, "{style} {css}");
    }
}

#[test]
fn lines_of_an_absolute_box_are_clipped_by_the_clips_above_its_containing_block() {
    // The containing block does not clip, but the box around it does.
    let result = laid_out(
        &format!(
            "<div id='clip'><div style='position:relative;height:20px'>{}</div></div>",
            absolute_lines("top:0")
        ),
        "#clip{overflow:hidden;height:50px;width:100px}",
    );
    assert_eq!(result.page_count(), 1);
    // A box inside an absolute box is held by that box, and so by its clips.
    let result = laid_out(
        &format!(
            "<div id='clip'><div style='position:absolute;top:0;left:0;width:60px;height:30px'>{}</div></div>",
            absolute_lines("top:0")
        ),
        "#clip{position:relative;overflow:hidden;height:50px;width:100px}",
    );
    assert_eq!(result.page_count(), 1);
}

#[test]
fn lines_of_an_absolute_box_are_not_clipped_by_boxes_that_do_not_contain_it() {
    // Its containing block is the page, or lies outside the clip, so the clip
    // hides nothing of it and the lines take their pages: 30 lines of 12px on
    // pages 100px tall.
    for body in [
        format!(
            "<div style='overflow:hidden;height:50px;width:100px'>{}</div>",
            absolute_lines("top:0")
        ),
        format!(
            "<div style='position:relative;height:50px;width:100px'>\
             <div style='overflow:hidden;height:50px'>{}</div></div>",
            absolute_lines("top:0")
        ),
        format!(
            "<div style='position:relative;height:50px;width:100px'>{}</div>",
            absolute_lines("top:0")
        ),
    ] {
        assert_eq!(laid_out(&body, "").page_count(), 4, "{body}");
    }
}

#[test]
fn an_absolute_box_clipped_past_the_page_keeps_the_pages_it_takes() {
    // A clip taller than a page still breaks across pages.
    let result = laid_out(
        &format!("<div id='clip'>{}</div>", absolute_lines("top:0")),
        "#clip{position:relative;overflow:hidden;height:150px;width:100px}",
    );
    assert_eq!(result.page_count(), 2);
}

#[test]
fn an_absolute_box_that_starts_past_the_clip_takes_only_the_page_it_starts_on() {
    // The clip is 50px tall and hides all of the box from its edge on, yet
    // Chromium gives the page that the top of the box falls on, and no page
    // after it, wherever the clip ends.
    for (top, pages) in [
        (0, 1),
        (40, 1),
        (49, 1),
        (50, 1),
        (60, 1),
        (120, 2),
        (300, 4),
    ] {
        let result = laid_out(
            &format!(
                "<div id='clip'>{}</div>",
                absolute_lines(&format!("top:{top}px"))
            ),
            "#clip{position:relative;overflow:hidden;height:50px;width:100px}",
        );
        assert_eq!(result.page_count(), pages, "top:{top}px");
    }
}

#[test]
fn what_is_inside_or_around_a_box_past_the_clip_does_not_add_pages() {
    let clip = "#clip{position:relative;overflow:hidden;height:50px;width:100px}";
    // A box inside it is held by it, and so by the clip above.
    for (top, pages) in [(120, 2), (300, 4)] {
        let result = laid_out(
            &format!(
                "<div id='clip'><div style='position:absolute;left:0;top:{top}px'>{}</div></div>",
                absolute_lines("top:0")
            ),
            clip,
        );
        assert_eq!(result.page_count(), pages, "inside top:{top}px");
    }
    // Padding above the lines keeps them on the page of the box.
    for (top, pages) in [(120, 2), (300, 4)] {
        let result = laid_out(
            &format!(
                "<div id='clip'>{}</div>",
                absolute_lines(&format!("top:{top}px;padding-top:20px"))
            ),
            clip,
        );
        assert_eq!(result.page_count(), pages, "padded top:{top}px");
    }
    // Two boxes past the edge take the pages that the later one starts on.
    let result = laid_out(
        &format!(
            "<div id='clip'>{}{}</div>",
            absolute_lines("top:120px"),
            absolute_lines("top:300px")
        ),
        clip,
    );
    assert_eq!(result.page_count(), 4);
}

#[test]
fn the_clip_of_a_box_does_not_reach_the_absolute_boxes_of_the_next_one() {
    let result = laid_out(
        &format!(
            "<div id='clip'>{}</div><div style='position:relative;height:20px'>{}</div>",
            absolute_lines("top:0"),
            absolute_lines("top:0")
        ),
        "#clip{position:relative;overflow:hidden;height:50px;width:100px}",
    );
    assert_eq!(result.page_count(), 5);
}

#[test]
fn an_empty_absolute_box_escapes_a_clip_below_the_initial_containing_block() {
    let result = laid_out(
        "<div style='overflow:hidden;height:50px;width:100px'><div style='position:absolute;top:0;left:0;width:50px;height:300px;background:red'></div></div>",
        "",
    );
    assert_eq!(result.page_count(), 3);
}

#[test]
fn an_empty_absolute_box_escapes_a_clip_below_its_positioned_containing_block() {
    let result = laid_out(
        "<div style='position:relative;height:50px;width:100px'><div style='overflow:hidden;height:50px'><div style='position:absolute;top:0;left:0;width:50px;height:300px;background:red'></div></div></div>",
        "",
    );
    assert_eq!(result.page_count(), 3);
}

#[test]
fn an_empty_absolute_box_past_its_clip_keeps_the_page_it_starts_on() {
    let result = laid_out(
        "<div style='position:relative;overflow:hidden;height:50px;width:100px'><div style='position:absolute;top:120px;left:0;width:50px;height:30px;background:red'></div></div>",
        "",
    );
    assert_eq!(result.page_count(), 2);
}

#[test]
fn empty_absolute_boxes_keep_the_clips_of_their_containing_blocks() {
    const BOX: &str =
        "<div style='position:absolute;top:0;left:0;width:50px;height:300px;background:red'></div>";
    for overflow in ["hidden", "auto", "scroll", "clip"] {
        let body = format!(
            "<div style='position:relative;overflow:{overflow};height:50px;width:100px'>{BOX}</div>"
        );
        assert_eq!(laid_out(&body, "").page_count(), 1, "{overflow}");
    }
    for (body, expected) in [
        (BOX.to_string(), 3),
        (format!("<div style='transform:translateX(0);overflow:hidden;height:50px'>{BOX}</div>"), 1),
        (format!("<div style='overflow:hidden;height:50px'><div style='position:relative;height:30px'>{BOX}</div></div>"), 1),
        ("<div style='overflow:hidden;height:50px'><div style='height:300px;background:red'></div></div>".to_string(), 1),
    ] {
        assert_eq!(laid_out(&body, "").page_count(), expected, "{body}");
    }
}

#[test]
fn hidden_empty_absolute_boxes_count_the_box_start_before_padding_and_border() {
    for (top, extra, pages) in [
        (40, "", 1),
        (50, "", 1),
        (60, "", 1),
        (100, "", 2),
        (120, "", 2),
        (300, "", 4),
        (120, "padding-top:200px", 2),
        (120, "border-top:200px solid blue", 2),
        (120, "height:0", 2),
    ] {
        let body = format!(
            "<div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:{top}px;left:0;width:50px;height:30px;{extra};background:red'></div></div>"
        );
        assert_eq!(laid_out(&body, "").page_count(), pages, "{body}");
    }
}

#[test]
fn a_clipped_positioned_subtree_does_not_reopen_a_hidden_descendants_page() {
    for height in [5, 300] {
        let body = format!(
            "<div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:40px;height:{height}px'><div style='position:absolute;top:120px;left:0;width:50px;height:300px;background:red'></div></div></div>"
        );
        assert_eq!(laid_out(&body, "").page_count(), 1, "height:{height}");
    }
    let child = "<div style='position:absolute;top:120px;left:0;width:50px;height:30px;background:red'></div>";
    for (body, expected) in [
        ("<div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:120px;height:30px'><div style='position:absolute;top:300px;left:0;width:50px;height:300px;background:red'></div></div></div>".to_string(), 2),
        (format!("<div style='position:absolute;overflow:hidden;top:0;left:0;height:50px;width:100px'>{child}</div>"), 2),
        (format!("<div style='position:absolute;top:0;height:50px'><div style='position:relative;overflow:hidden;height:50px'>{child}</div></div>"), 2),
        (format!("<div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:0;height:300px'><div style='position:relative;height:30px'>{child}</div></div></div>"), 1),
    ] {
        assert_eq!(laid_out(&body, "").page_count(), expected, "{body}");
    }
}

#[test]
fn empty_positioned_boxes_use_inline_containing_blocks_without_changing_coordinates() {
    let child =
        "<div style='position:absolute;top:0;left:0;width:50px;height:300px;background:red'></div>";
    for (body, expected) in [
        (
            format!(
                "<div style='overflow:hidden;height:50px'><span style='position:relative'>{child}</span></div>"
            ),
            1,
        ),
        (
            format!(
                "<span style='position:relative'><div style='overflow:hidden;height:50px'>{child}</div></span>"
            ),
            3,
        ),
    ] {
        assert_eq!(laid_out(&body, "").page_count(), expected, "{body}");
    }
}

#[test]
fn empty_fixed_boxes_use_the_fixed_containing_block_for_clipping() {
    let child =
        "<div style='position:fixed;top:0;left:0;width:50px;height:300px;background:red'></div>";
    for (body, expected) in [
        (format!("<div style='overflow:hidden;height:50px'>{child}</div>"), 1),
        (format!("<div style='transform:translateX(0);overflow:hidden;height:50px'>{child}</div>"), 1),
        (format!("<div style='transform:translateX(0);height:50px'><div style='overflow:hidden;height:50px'>{child}</div></div>"), 3),
        (format!("<div style='transform:translateX(0);height:50px'><div style='position:relative;overflow:hidden;height:50px'>{child}</div></div>"), 3),
        (format!("<div style='transform:translateX(0);height:50px'><div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:120px;height:30px'>{child}</div></div></div>"), 3),
        ("<div style='transform:translateX(0);overflow:hidden;height:50px'><div style='position:fixed;top:120px;left:0;width:50px;height:30px;background:red'></div></div>".to_string(), 2),
        ("<div style='transform:translateX(0);height:50px'><div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:40px;height:300px'><div style='position:fixed;top:120px;left:0;width:50px;height:30px;background:red'></div></div></div></div>".to_string(), 2),
    ] {
        assert_eq!(laid_out(&body, "").page_count(), expected, "{body}");
    }
}

#[test]
fn positioned_clip_outsets_paint_without_adding_fragmentation_pages() {
    const CASES: &[(&str, u32)] = &[
        (
            r#"<div style="position:relative;overflow:clip;overflow-clip-margin:100px;height:50px;width:100px"><div style="width:50px;height:300px;background:red"></div></div>"#,
            2,
        ),
        (
            r#"<div style="position:relative;overflow:clip;overflow-clip-margin:100px;height:50px;width:100px"><div style="position:absolute;top:0;left:0;width:50px;height:300px;background:red"></div></div>"#,
            1,
        ),
        (
            r#"<div style="position:absolute;top:0;left:0;overflow:clip;overflow-clip-margin:100px;height:50px;width:100px"><div style="height:300px;background:red"></div></div>"#,
            1,
        ),
        (
            r#"<div style="position:absolute;top:0;left:0;overflow:clip;overflow-clip-margin:100px;height:50px;width:100px"><div style="position:absolute;top:0;left:0;height:300px;width:50px;background:red"></div></div>"#,
            1,
        ),
        (
            r#"<div style="position:relative;overflow:clip;overflow-clip-margin:100px;height:50px;width:100px"><div style="position:absolute;top:40px;height:5px"><div style="position:absolute;top:200px;height:5px"><div style="position:absolute;top:-240px;left:0;width:50px;height:300px;background:red"></div></div></div></div>"#,
            1,
        ),
        (
            r#"<div style="position:relative;overflow:clip;overflow-clip-margin:100px;height:50px;width:100px"><div style="position:absolute;top:40px;height:5px"><div style="position:absolute;top:200px;height:5px"></div></div></div>"#,
            1,
        ),
    ];
    for &(body, pages) in CASES {
        assert_eq!(laid_out(body, "").page_count(), pages, "{body}");
    }
}

#[test]
fn fixed_boxes_in_columns_follow_their_fixed_containing_blocks() {
    const CASES: &[(&str, u32)] = &[
        (
            r#"<div style="transform:translateX(0)"><div style="columns:2;column-fill:auto;height:50px"><div style="position:fixed;top:0;left:0;width:50px;height:300px;background:red"></div></div></div>"#,
            3,
        ),
        (
            r#"<div style="transform:translateX(0)"><div style="columns:2;column-fill:auto;height:50px"><div style="position:relative;height:30px"><div style="position:fixed;top:0;left:0;width:50px;height:300px;background:red"></div></div></div></div>"#,
            3,
        ),
        (
            r#"<div style="columns:2;column-fill:auto;height:50px"><div style="transform:translateX(0);height:30px"><div style="position:fixed;top:0;left:0;width:50px;height:300px;background:red"></div></div></div>"#,
            1,
        ),
    ];
    for &(body, pages) in CASES {
        assert_eq!(laid_out(body, "").page_count(), pages, "{body}");
    }
}

#[test]
fn inline_filters_establish_the_fixed_containing_block_at_the_clip_boundary() {
    const CASES: &[(&str, u32)] = &[
        (
            r#"<div style="overflow:hidden;height:50px"><span style="filter:blur(0px)"><div style="position:fixed;top:120px;left:0;width:50px;height:30px;background:red"></div></span></div>"#,
            2,
        ),
        (
            r#"<span style="filter:blur(0px)"><div style="overflow:hidden;height:50px"><div style="position:fixed;top:0;left:0;width:50px;height:300px;background:red"></div></div></span>"#,
            3,
        ),
    ];
    for &(body, pages) in CASES {
        assert_eq!(laid_out(body, "").page_count(), pages, "{body}");
    }
}

#[test]
fn fixed_can_escape_columns_from_an_absolute_box_in_the_column_flow() {
    let body = r#"<div style="transform:translateX(0)"><div style="columns:2;column-fill:auto;height:50px"><div style="position:relative;height:30px"><div style="position:absolute;top:0;left:0"><div style="position:fixed;top:0;left:0;width:50px;height:300px;background:red"></div></div></div></div></div>"#;
    assert_eq!(laid_out(body, "").page_count(), 3);
}

#[test]
fn a_hidden_positioned_start_has_no_artificial_extent_at_a_page_edge() {
    for (position, owner) in [
        ("absolute", "position:relative"),
        ("fixed", "transform:translateX(0)"),
    ] {
        for (top, pages) in [
            ("99.5", 1),
            ("99.75", 1),
            ("100", 2),
            ("199.5", 2),
            ("200", 3),
        ] {
            let body = format!(
                "<div style='{owner};overflow:hidden;height:50px'><div style='position:{position};top:{top}px;left:0;width:50px;height:30px;background:red'></div></div>"
            );
            assert_eq!(
                laid_out(&body, "").page_count(),
                pages,
                "{position} top:{top}"
            );
        }
    }
    for (top, pages) in [("149.5", 1), ("150", 2)] {
        let body = format!(
            "<div style='position:relative;overflow:hidden;height:50px'><div style='position:absolute;top:{top}px;left:0;width:50px;height:30px;background:red'></div></div>"
        );
        assert_eq!(
            laid_out(&body, "@page:first{size:200px 150px}").page_count(),
            pages,
            "first150 top:{top}"
        );
    }
}

/// Page counts for a clip box in a vertical writing mode, where the clipped
/// (block) axis is the physical x axis. `overflow-x` is the block-axis
/// overflow there.
fn vertical_clip_pages(writing_mode: &str, overflow: &str, shape: &str) -> usize {
    let words = "x ".repeat(60);
    let lines = "x<br>".repeat(30);
    let (body, extra) = match shape {
        "lines" => (
            format!(
                "<div id='clip'><div style='width:20px'></div><div style='width:30px'>{lines}</div></div>"
            ),
            "",
        ),
        "paragraph" => (
            format!("<div id='clip'><p>{words}</p></div>"),
            "p{height:12px;margin:0}",
        ),
        "box" => (
            "<div id='clip'><div style='width:400px;height:20px'></div></div><div id='after' style='width:10px;height:10px'>D</div>".to_owned(),
            "",
        ),
        other => panic!("unknown shape {other}"),
    };
    let css = format!("html{{writing-mode:{writing_mode}}}#clip{{width:200px;{overflow}}}{extra}");
    laid_out(&body, &css).page_count() as usize
}

#[test]
fn vertical_clip_boxes_add_no_page_for_lines_past_the_block_end() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        for overflow in [
            "overflow:clip",
            "overflow:hidden",
            "overflow:auto",
            "overflow:scroll",
            "overflow-x:clip;overflow-y:visible",
        ] {
            for shape in ["lines", "paragraph"] {
                assert_eq!(
                    vertical_clip_pages(writing_mode, overflow, shape),
                    1,
                    "{writing_mode} {overflow} {shape}"
                );
            }
        }
    }
}

#[test]
fn vertical_boxes_without_a_block_axis_clip_still_add_pages() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        for overflow in ["overflow:visible", "overflow-y:clip;overflow-x:visible"] {
            assert_eq!(
                vertical_clip_pages(writing_mode, overflow, "lines"),
                2,
                "{writing_mode} {overflow} lines"
            );
            assert_eq!(
                vertical_clip_pages(writing_mode, overflow, "paragraph"),
                4,
                "{writing_mode} {overflow} paragraph"
            );
        }
    }
}

#[test]
fn vertical_boxes_take_an_overflow_clip_margin_only_when_both_axes_clip() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        assert_eq!(
            vertical_clip_pages(
                writing_mode,
                "overflow:clip;overflow-clip-margin:100px",
                "lines"
            ),
            2,
            "{writing_mode} both axes clip"
        );
        for overflow in [
            "overflow-x:clip;overflow-y:visible",
            "overflow:clip visible",
        ] {
            assert_eq!(
                vertical_clip_pages(
                    writing_mode,
                    &format!("{overflow};overflow-clip-margin:100px"),
                    "lines"
                ),
                1,
                "{writing_mode} {overflow}"
            );
        }
    }
}

#[test]
fn the_flow_after_a_vertical_clip_starts_where_the_clip_ends() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        for overflow in [
            "overflow:clip",
            "overflow:hidden",
            "overflow:auto",
            "overflow:visible",
        ] {
            assert_eq!(
                vertical_clip_pages(writing_mode, overflow, "box"),
                2,
                "{writing_mode} {overflow} box"
            );
        }
        let css = format!(
            "html{{writing-mode:{writing_mode}}}#clip{{width:300px;overflow:hidden}}#after{{width:10px;height:10px;font:10px/10px Ahem}}"
        );
        let lines = "x<br>".repeat(40);
        for body in [
            "<div id='clip'><div style='width:600px;height:20px'></div></div><div id='after'>D</div>"
                .to_owned(),
            format!("<div id='clip'>{lines}</div><div id='after'>D</div>"),
        ] {
            assert_eq!(laid_out(&body, &css).page_count(), 2, "{writing_mode} {body}");
        }
    }
}
