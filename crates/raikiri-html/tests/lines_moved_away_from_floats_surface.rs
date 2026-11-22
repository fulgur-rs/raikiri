//! A paragraph whose lines were broken beside floats, and that moves whole
//! to the next page while the floats stay on the page it leaves, breaks its
//! lines again on the next page, where the floats take no room. Where `K`
//! falls is what Chromium prints for the same documents on pages 180px by
//! 120px, with Ahem glyphs of 20px on lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, left and top of the glyph `K` in `body`.
fn marker(body: &str) -> (u32, i32, i32) {
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
    // Each Ahem glyph and space is 20px wide, so the left of `K` follows
    // from its place in its run.
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            if let Some(offset) = run.text.chars().position(|glyph| glyph == 'K') {
                let top = (run.origin.1 - run.ascent).round() as i32;
                let left = run.origin.0 + 20.0 * offset as f32;
                return (index, left.round() as i32, top);
            }
        }
    }
    panic!("no K in {body}")
}

#[test]
fn lines_moved_away_from_the_floats_beside_them_take_the_whole_width() {
    for (body, expected) in [
        // A float that fits stays behind, and the line starts the next page at its start.
        (
            "<div style='height:110px'></div><div style='float:left;width:10px;height:5px'></div><p style='margin:0'>K</p>",
            (1, 0, 0),
        ),
        // So does one inside a block before the paragraph.
        (
            "<div style='height:110px'></div><div><div style='float:left;width:10px;height:5px'></div></div><p style='margin:0'>K</p>",
            (1, 0, 0),
        ),
        // One that ends right at the page end too.
        (
            "<div style='height:105px'></div><div style='float:left;width:10px;height:15px'></div><p style='margin:0'>K</p>",
            (1, 0, 0),
        ),
        // Only the first line was beside it.
        (
            "<div style='height:110px'></div><div style='float:left;width:10px;height:5px'></div><p style='margin:0'>K<br>L</p>",
            (1, 0, 0),
        ),
        // The words beside the float wrapped early; on the next page they fill the line.
        (
            "<div style='height:105px'></div><div style='float:left;width:40px;height:5px'></div><p style='margin:0'>A B C K</p>",
            (1, 120, 0),
        ),
        // The same beside a right float.
        (
            "<div style='height:110px'></div><div style='float:right;width:40px;height:5px'></div><p style='margin:0'>A B C K</p>",
            (1, 120, 0),
        ),
        // A block that the paragraph begins moves with it.
        (
            "<div style='height:100px'></div><div style='float:left;width:40px;height:5px'></div><div style='padding-top:3px'><p style='margin:0'>A K</p></div>",
            (1, 40, 3),
        ),
        // A paragraph that avoids breaks inside moves whole.
        (
            "<div style='height:100px'></div><div style='float:left;width:40px;height:5px'></div><p style='margin:0;break-inside:avoid'>A B<br>K</p>",
            (1, 0, 20),
        ),
        // A box moved to the second page earlier shifts the flow; the
        // paragraph still starts the third page at its start.
        (
            "<div style='height:100px'></div><div style='height:40px;break-inside:avoid'><div style='height:40px'></div></div><div style='height:70px'></div><div style='float:left;width:40px;height:5px'></div><p style='margin:0'>A B C K</p>",
            (2, 120, 0),
        ),
        // Only the first line was beside the float; the rest wrap anew.
        (
            "<div style='height:100px'></div><div style='float:left;width:40px;height:5px'></div><p style='margin:0'>A B C D E F G H I J K</p>",
            (1, 0, 40),
        ),
        // A list item moves the same way.
        (
            "<div style='height:110px'></div><div style='float:left;width:40px;height:5px'></div><ul style='margin:0;padding:0;list-style:none'><li>A B C K</li></ul>",
            (1, 120, 0),
        ),
        // The paragraph after it follows it.
        (
            "<div style='height:110px'></div><div style='float:left;width:40px;height:5px'></div><p style='margin:0'>A B C</p><p style='margin:0'>K</p>",
            (1, 0, 20),
        ),
    ] {
        assert_eq!(marker(body), expected, "{body}");
    }
}

#[test]
fn lines_that_move_with_their_floats_stay_beside_them() {
    for (body, expected) in [
        // A float that runs past the page end goes on with its rest, and the line stays beside it.
        (
            "<div style='height:110px'></div><div style='float:left;width:10px;height:11px'></div><p style='margin:0'>K</p>",
            (1, 10, 0),
        ),
        // A float that does not fit moves too.
        (
            "<div style='height:105px'></div><div style='float:left;width:40px;height:30px'></div><p style='margin:0;orphans:2;widows:2'>A B C D K</p>",
            (1, 0, 20),
        ),
    ] {
        assert_eq!(marker(body), expected, "{body}");
    }
}
