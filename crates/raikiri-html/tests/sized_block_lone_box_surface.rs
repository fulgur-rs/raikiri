//! A block that holds one box, which the bottom padding or border of the
//! block around it follows, moves to the next page whole when those edges do
//! not fit, even when a block of set `height`, `min-height` or `max-height`
//! holds that block and leaves room at its own end: the room does not follow
//! the box, and the page breaks before the outermost block. Edges that run
//! past the end of a block too short for its content follow the box as well.
//! Where `K` falls is what Chromium prints for the same documents on pages
//! 180px by 120px, with Ahem glyphs of 20px on lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, left and top of the glyph `K` in `body`, and the number of
/// pages.
fn marker(body: &str) -> (u32, i32, i32, u32) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:180px 120px;margin:0}} html,body{{margin:0}} body{{font:20px/20px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            if run.text == "K" {
                let top = (run.origin.1 - run.ascent).round() as i32;
                return (index, run.origin.0.round() as i32, top, result.page_count());
            }
        }
    }
    panic!("no K in {body}")
}

/// `outer` around a block of 51px with `inner` edges, around a box of
/// `child` px, after a spacer of `spacer` px and before the marker.
fn chain(spacer: u32, outer: &str, inner: &str, child: u32) -> String {
    format!(
        "<div style='height:{spacer}px'></div><div style='{outer}'><div style='height:51px;{inner}'><div style='height:{child}px'></div></div></div><p style='margin:0'>K</p>"
    )
}

const BORDER: &str = "border-bottom:40px solid";
const PADDING: &str = "padding-bottom:30px";

#[test]
fn a_block_of_set_size_moves_whole_for_the_edges_inside_it() {
    for (spacer, outer, inner, child, expected) in [
        (40, "height:100px", BORDER, 60, (1, 0, 100, 2)),
        (40, "height:100px", PADDING, 60, (1, 0, 100, 2)),
        (40, "min-height:100px", BORDER, 60, (1, 0, 100, 2)),
        (
            40,
            "max-height:100px;height:120px",
            PADDING,
            60,
            (1, 0, 100, 2),
        ),
        (
            40,
            "height:100px;padding-bottom:10px",
            BORDER,
            60,
            (2, 0, 0, 3),
        ),
        (40, "height:150px", BORDER, 60, (2, 0, 30, 3)),
        (60, "height:100px", BORDER, 60, (1, 0, 100, 2)),
        (60, "height:100px", PADDING, 60, (1, 0, 100, 2)),
    ] {
        let body = chain(spacer, outer, inner, child);
        assert_eq!(marker(&body), expected, "{body}");
    }
}

#[test]
fn room_right_after_the_box_keeps_the_block_in_place() {
    for (spacer, outer, inner, child, expected) in [
        (40, "height:100px", BORDER, 40, (1, 0, 20, 2)),
        (40, "min-height:100px", BORDER, 40, (1, 0, 40, 2)),
        (40, "min-height:100px", PADDING, 40, (1, 0, 30, 2)),
        (
            40,
            "height:100px;padding-bottom:10px",
            PADDING,
            40,
            (1, 0, 30, 2),
        ),
        (40, "height:150px", PADDING, 40, (1, 0, 70, 2)),
        (40, "height:100px", BORDER, 85, (1, 0, 20, 2)),
        (60, "height:100px", BORDER, 40, (1, 0, 40, 2)),
    ] {
        let body = chain(spacer, outer, inner, child);
        assert_eq!(marker(&body), expected, "{body}");
    }
}

#[test]
fn edges_past_the_end_of_a_short_block_follow_the_box() {
    for (spacer, outer, inner, child, expected) in [
        (40, "height:60px", BORDER, 60, (1, 0, 60, 2)),
        (40, "height:60px", PADDING, 60, (1, 0, 60, 2)),
        (60, "height:60px", BORDER, 60, (1, 0, 60, 2)),
        (
            40,
            "height:60px;border-bottom:5px solid",
            BORDER,
            60,
            (1, 0, 65, 2),
        ),
        (10, "height:60px", BORDER, 60, (0, 0, 70, 1)),
        // The box itself runs past the end of the block: it breaks.
        (40, "height:60px", BORDER, 40, (0, 0, 100, 2)),
        (40, "height:60px", BORDER, 85, (0, 0, 100, 2)),
        (60, "height:60px", BORDER, 85, (1, 0, 0, 2)),
    ] {
        let body = chain(spacer, outer, inner, child);
        assert_eq!(marker(&body), expected, "{body}");
    }
}

#[test]
fn the_outermost_block_of_the_chain_moves() {
    let inner =
        "<div style='height:51px;border-bottom:40px solid'><div style='height:60px'></div></div>";
    let spacer = "<div style='height:40px'></div>";
    let k = "<p style='margin:0'>K</p>";
    for (body, expected) in [
        (
            format!("{spacer}<div><div style='height:100px'>{inner}</div></div>{k}"),
            (1, 0, 100, 2),
        ),
        (
            format!("{spacer}<div><div><div style='height:100px'>{inner}</div></div></div>{k}"),
            (1, 0, 100, 2),
        ),
        (
            format!(
                "{spacer}<div style='height:110px'><div style='height:100px'>{inner}</div></div>{k}"
            ),
            (2, 0, 0, 3),
        ),
        (
            format!(
                "{spacer}<div style='padding-bottom:5px'><div style='height:100px'>{inner}</div></div>{k}"
            ),
            (2, 0, 0, 3),
        ),
        (
            format!(
                "{spacer}<div style='border-bottom:10px solid'><div style='height:100px'>{inner}</div></div>{k}"
            ),
            (2, 0, 0, 3),
        ),
        (
            format!("{spacer}<div><div style='height:60px'>{inner}</div></div>{k}"),
            (1, 0, 60, 2),
        ),
        (
            format!(
                "{spacer}<div style='height:100px'><div style='float:left;width:20px;height:10px'></div>{inner}</div>{k}"
            ),
            (1, 0, 100, 2),
        ),
        (
            format!(
                "<div style='height:100px'><div style='height:51px;border-bottom:80px solid'><div style='height:60px'></div></div></div>{k}"
            ),
            (0, 0, 100, 2),
        ),
    ] {
        assert_eq!(marker(&body), expected, "{body}");
    }
}

#[test]
fn neighbours_of_the_chain_keep_their_breaks() {
    let inner =
        "<div style='height:51px;border-bottom:40px solid'><div style='height:60px'></div></div>";
    let k = "<p style='margin:0'>K</p>";
    for (body, expected) in [
        // A short block around one with room: the box ends within the short
        // block's content, so the edges past its end follow the box.
        (
            format!("<div style='height:40px'></div><div style='height:40px'><div style='height:100px'>{inner}</div></div>{k}"),
            (1, 0, 40, 2),
        ),
        // Room under half a pixel is no room: the padding around it follows.
        (
            format!("<div style='height:30px'></div><div style='padding-bottom:30px'><div style='height:91.3px'>{inner}</div></div>{k}"),
            (2, 0, 30, 3),
        ),
        // The last box of a run tied by an avoided break.
        (
            format!("<div style='height:20px'></div><div style='break-after:avoid;height:10px'></div><div style='height:100px'>{inner}</div>{k}"),
            (2, 0, 0, 3),
        ),
        // Two lines in a block of set size with room keep their own break.
        (
            "<div style='height:40px'></div><div style='padding-bottom:20px'><div style='height:200px'><div style='padding-bottom:10px'><p style='margin:0;orphans:2;widows:2'>A<br>B</p></div></div></div><p style='margin:0'>K</p>".to_string(),
            (2, 0, 20, 3),
        ),
    ] {
        assert_eq!(marker(&body), expected, "{body}");
    }
}
