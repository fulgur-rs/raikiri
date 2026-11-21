//! A block whose first box is closed by bottom padding or a border that does
//! not fit the page, while the box itself does, moves to the next page whole
//! when other boxes follow that first box: the page breaks before the block,
//! which is the break before its first box, rather than between the box and
//! its edges. Where `K` falls is what Chromium prints for the same documents
//! on pages 180px by 120px, with Ahem glyphs of 20px on lines of 20px.

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

#[test]
fn a_block_moves_whole_for_the_edges_of_its_first_box() {
    for (body, expected) in [
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:50px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 30, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:100px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 80, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><p style='margin:0'>A</p></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div style='padding-top:10px'><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div style='border-top:10px solid'><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='padding-bottom:40px'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='border-bottom:40px solid'><div style='height:60px'></div></div></div><p style='margin:0'>K</p>",
            (2, 0, 80, 3),
        ),
        (
            "<div style='height:40px'></div><div style='margin-top:10px'><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px;break-before:avoid'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:60px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:80px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 5, 3),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:30px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (1, 0, 95, 2),
        ),
        (
            "<div style='height:40px'></div><div><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='float:left;width:10px;height:5px'></div></div><p style='margin:0'>K</p>",
            (1, 10, 100, 2),
        ),
        (
            "<div style='height:40px'></div><div style='break-inside:avoid'><div style='border-bottom:40px solid'><div style='height:60px'></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>",
            (2, 0, 0, 3),
        ),
    ] {
        assert_eq!(marker(body), expected, "{body}");
    }
}

#[test]
fn a_block_of_set_size_as_the_first_box_moves_its_parent() {
    let body = "<div style='height:40px'></div><div><div style='height:100px'><div style='height:51px;border-bottom:40px solid'><div style='height:60px'></div></div></div><div style='height:5px'></div></div><p style='margin:0'>K</p>";
    assert_eq!(marker(body), (2, 0, 0, 3), "{body}");
}

#[test]
fn what_follows_the_first_box_stays_behind_it() {
    let s = "<div style='height:40px'></div>";
    let x = "<div style='border-bottom:40px solid'><div style='height:60px'></div></div>";
    let k = "<p style='margin:0'>K</p>";
    for (body, expected) in [
        // A float beside a later box.
        (
            format!(
                "{s}<div>{x}<div style='float:left;width:10px;height:50px'></div><div style='height:5px'></div></div>{k}"
            ),
            (2, 10, 0, 3),
        ),
        // A clip hides the border past the end of the block.
        (
            format!(
                "{s}<div style='height:80px;overflow:hidden'>{x}<div style='height:5px'></div></div>{k}"
            ),
            (1, 0, 0, 2),
        ),
        // The padding around the block does not follow the first box.
        (
            format!(
                "<div style='height:20px'></div><div style='padding-bottom:30px'><div style='height:80px'>{x}<div style='height:5px'></div></div></div>{k}"
            ),
            (1, 0, 30, 2),
        ),
        (
            format!(
                "{s}<div style='padding-bottom:30px'><div style='height:80px'>{x}<div style='height:5px'></div></div></div>{k}"
            ),
            (2, 0, 0, 3),
        ),
        // The border of the first box runs past the end of a short block.
        (
            format!("{s}<div style='height:80px'>{x}<div style='height:5px'></div></div>{k}"),
            (1, 0, 80, 2),
        ),
    ] {
        assert_eq!(marker(&body), expected, "{body}");
    }
}

#[test]
fn a_block_that_avoids_breaks_inside_keeps_the_edges_after_it() {
    for (body, expected) in [
        // The block moves with the padding of its parent, which does not fit
        // after it.
        (
            "<div style='height:50px'></div><div style='padding-bottom:40px'><div style='break-inside:avoid'><div style='height:10px;border-bottom:10px solid'></div><div style='height:20px'></div></div></div><p style='margin:0'>K</p>",
            (1, 0, 80, 2),
        ),
        (
            "<div style='height:50px'></div><div style='padding-bottom:40px'><div style='break-inside:avoid'><div style='height:10px;border-bottom:10px solid'></div><div style='height:20px'></div></div></div><div style='height:5px'></div><p style='margin:0'>K</p>",
            (1, 0, 85, 2),
        ),
        // Without the property, the page breaks after its first box.
        (
            "<div style='height:50px'></div><div style='padding-bottom:40px'><div><div style='height:10px;border-bottom:10px solid'></div><div style='height:20px'></div></div></div><p style='margin:0'>K</p>",
            (1, 0, 60, 2),
        ),
    ] {
        assert_eq!(marker(body), expected, "{body}");
    }
}
