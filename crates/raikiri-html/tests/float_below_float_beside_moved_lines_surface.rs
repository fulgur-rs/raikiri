//! Lines beside a float that stands below another, where clearance or the
//! floats before it put it rather than the line it comes in. A page end that
//! moves the lines down leaves such a float where it is: the float does not
//! take the flow to its page, nor move with the lines, and the lines are laid
//! out again beside it where they land. Where each word falls is what
//! Chromium prints for the same documents on pages 100px square, with Ahem
//! glyphs of 20px on lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The distance from the top of a 30px line to the top of its 20px Ahem
/// glyphs.
const HALF_LEADING: f32 = 5.0;

/// Every word of `body` as `word@page:x,y`, the left of its first glyph and
/// the top of its line on its page, sorted.
fn words(body: &str) -> String {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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
    let mut words = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let top = (run.origin.1 - run.ascent - HALF_LEADING).round();
            let mut x = run.origin.0;
            for word in run.text.split(' ') {
                if !word.is_empty() {
                    words.push(format!("{word}@{index}:{},{top}", x.round()));
                }
                x += 20.0 * (word.chars().count() + 1) as f32;
            }
        }
    }
    words.sort();
    format!("{}p {}", result.page_count(), words.join(" "))
}

/// Twelve words after a float `first`px tall and a float below it that
/// clears it, on the side and as tall as `(side, height)`, then `after`.
fn stacked(first: u32, (side, height): (&str, u32), after: &str) -> String {
    format!(
        "<div style='float:left;width:20px;height:{first}px'></div>\
         <div style='float:{side};clear:left;width:40px;height:{height}px'></div>\
         XX XX XX XX XX XX XX XX XX XX XX XX{after}"
    )
}

#[test]
fn a_float_below_another_stays_where_the_lines_beside_it_move() {
    for (body, expected) in [
        // The second float starts the page after the one the first ends on.
        (
            stacked(195, ("left", 97), ""),
            "4p XX@0:20,0 XX@0:20,30 XX@0:20,60 XX@1:20,0 XX@1:20,30 XX@1:20,60 XX@2:40,0 XX@2:40,30 XX@2:40,60 XX@3:0,0 XX@3:0,30 XX@3:60,0",
        ),
        (
            stacked(95, ("left", 97), ""),
            "3p XX@0:20,0 XX@0:20,30 XX@0:20,60 XX@1:40,0 XX@1:40,30 XX@1:40,60 XX@2:0,0 XX@2:0,30 XX@2:0,60 XX@2:60,0 XX@2:60,30 XX@2:60,60",
        ),
        // The lines move past the page end, and the line that lands beside the
        // second float is the one it narrows.
        (
            stacked(150, ("left", 3), ""),
            "3p XX@0:20,0 XX@0:20,30 XX@0:20,60 XX@1:0,60 XX@1:20,0 XX@1:40,30 XX@1:60,60 XX@2:0,0 XX@2:0,30 XX@2:0,60 XX@2:60,0 XX@2:60,30",
        ),
        (
            stacked(150, ("right", 3), ""),
            "3p XX@0:20,0 XX@0:20,30 XX@0:20,60 XX@1:0,60 XX@1:20,0 XX@1:20,30 XX@1:60,60 XX@2:0,0 XX@2:0,30 XX@2:0,60 XX@2:60,0 XX@2:60,30",
        ),
        // The lines before a block child of the paragraph.
        (
            stacked(150, ("left", 3), "<div>Y</div>"),
            "4p XX@0:20,0 XX@0:20,30 XX@0:20,60 XX@1:0,60 XX@1:20,0 XX@1:40,30 XX@1:60,60 XX@2:0,0 XX@2:0,30 XX@2:0,60 XX@2:60,0 XX@2:60,30 Y@3:0,0",
        ),
    ] {
        assert_eq!(words(&body), expected, "{body}");
    }
}
