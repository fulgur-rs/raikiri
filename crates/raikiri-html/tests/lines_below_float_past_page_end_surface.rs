//! The lines of a block that floats put down past the end of the page the
//! block starts on. The block's top stays on that page, with the room the
//! floats leave above its first line, and the lines break from the page
//! their first one is on: each line that crosses a page end starts the next
//! page, as in Chromium. Where each word falls is what Chromium prints for
//! the same documents on pages 100px square, with Ahem glyphs of 20px on
//! lines of 30px.

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
    words.join(" ")
}

/// A left float 40px wide and 160px tall: the 80px word `WWWW` does not fit
/// beside it, so the first line of the block after `ZZ YY XX` goes below it,
/// on the page after the one the block starts on.
const FLOAT: &str = "<div style='float:left;width:40px;height:160px'></div>";

#[test]
fn the_lines_after_a_first_line_that_floats_put_on_a_later_page_start_the_next_page() {
    for (body, expected) in [
        // `VV UU` crosses the end of the page `WWWW` is on and starts the
        // next one, as one line of the block is all that page holds.
        (
            format!("{FLOAT}ZZ YY XX<div>WWWW VV UU TT SS</div>"),
            "SS@2:60,30 TT@2:0,30 UU@2:60,0 VV@2:0,0 WWWW@1:0,60 XX@0:40,60 \
             YY@0:40,30 ZZ@0:40,0",
        ),
        (
            "<div style='float:right;width:40px;height:160px'></div>ZZ YY XX\
             <div>WWWW VV UU TT SS</div>"
                .to_string(),
            "SS@2:60,30 TT@2:0,30 UU@2:60,0 VV@2:0,0 WWWW@1:0,60 XX@0:0,60 \
             YY@0:0,30 ZZ@0:0,0",
        ),
        // The block of the lines inside the one the floats put down.
        (
            format!(
                "{FLOAT}ZZ YY XX<div><div style='padding-bottom:10px'>WWWW VV UU TT SS</div></div>"
            ),
            "SS@2:60,30 TT@2:0,30 UU@2:60,0 VV@2:0,0 WWWW@1:0,60 XX@0:40,60 \
             YY@0:40,30 ZZ@0:40,0",
        ),
        // `break-inside: avoid` does not take the lines to the page after.
        (
            format!("{FLOAT}ZZ YY XX<div style='break-inside:avoid'>WWWW VV UU TT SS</div>"),
            "SS@2:60,30 TT@2:0,30 UU@2:60,0 VV@2:0,0 WWWW@1:0,60 XX@0:40,60 \
             YY@0:40,30 ZZ@0:40,0",
        ),
        // A line that crosses the page end lower down.
        (
            "<div style='float:left;width:40px;height:150px'></div>ZZ YY XX\
             <div>WWWW VV UU TT SS RR QQ</div>"
                .to_string(),
            "QQ@2:60,60 RR@2:0,60 SS@2:60,30 TT@2:0,30 UU@2:60,0 VV@2:0,0 \
             WWWW@1:0,50 XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        // Floats that end two pages down.
        (
            "<div style='float:left;width:40px;height:250px'></div>ZZ YY XX\
             <div>WWWW VV UU TT SS RR QQ</div>"
                .to_string(),
            "QQ@3:60,60 RR@3:0,60 SS@3:60,30 TT@3:0,30 UU@3:60,0 VV@3:0,0 \
             WWWW@2:0,50 XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
    ] {
        assert_eq!(words(&body), expected, "{body}");
    }
}

#[test]
fn the_page_the_first_line_opens_gives_its_widows_to_the_next() {
    // Three of the four lines fit the page the first line starts, at its
    // top, but the last would be alone on the next page: `TT SS` goes with it.
    let body = "<div style='float:left;width:40px;height:100px'></div>ZZ YY XX\
                <div>WWWW VV UU TT SS RR QQ</div>";
    assert_eq!(
        words(body),
        "QQ@2:60,30 RR@2:0,30 SS@2:60,0 TT@2:0,0 UU@1:60,30 VV@1:0,30 WWWW@1:0,0 \
         XX@0:40,60 YY@0:40,30 ZZ@0:40,0"
    );
}
