//! The lines beside a float whose content runs on past the end of a page.
//! The float grows with the lines the page break moves down inside it, and
//! the lines of the flow beside it on the next page are broken against the
//! part of the float that continues there, down to its new end. Where each
//! word falls is what Chromium prints for the same documents on pages 100px
//! square, with Ahem glyphs of 20px on lines of 30px.

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

/// A left float 40px wide with the four lines `F G G G`: the page break
/// moves its last two lines to the next page (its widows), so that it grows
/// by 40px. The lines beside it move down by 10px only.
const FLOAT: &str = "<div style='float:left;width:40px'>F G G G</div>";

#[test]
fn the_lines_on_the_next_page_wrap_beside_the_float_that_continues_there() {
    for (body, expected) in [
        (
            format!("{FLOAT}ZZ YY XX WW VV"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 VV@1:40,30 WW@1:40,0 \
             XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        // The lines below the float's new end take the full width again.
        (
            format!("{FLOAT}ZZ YY XX WW VV UU TT"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 TT@1:60,60 UU@1:0,60 VV@1:40,30 \
             WW@1:40,0 XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        // So do those of a paragraph that holds the float.
        (
            format!("<div style='display:flow-root'>{FLOAT}ZZ YY XX WW VV UU TT</div>"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 TT@1:60,60 UU@1:0,60 VV@1:40,30 \
             WW@1:40,0 XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        // The lines of a block after the paragraph wrap beside it too.
        (
            format!("{FLOAT}ZZ YY XX<div>WW VV</div>"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 VV@1:40,30 WW@1:40,0 \
             XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        (
            format!("<div>{FLOAT}ZZ YY XX</div><div>WW VV</div>"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 VV@1:40,30 WW@1:40,0 \
             XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        // A float on the right, and one with an earlier float beside it.
        (
            "<div style='float:right;width:40px'>F G G G</div>ZZ YY XX WW VV".to_string(),
            "F@0:60,0 G@0:60,30 G@1:60,0 G@1:60,30 VV@1:0,30 WW@1:0,0 \
             XX@0:0,60 YY@0:0,30 ZZ@0:0,0",
        ),
        (
            format!("{FLOAT}<div style='float:left;width:20px'>E</div>ZZ YY XX WW VV"),
            "E@0:40,0 F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 VV@1:40,30 WW@1:40,0 \
             XX@0:40,60 YY@0:40,30 ZZ@0:60,0",
        ),
    ] {
        assert_eq!(words(&body), expected, "{body}");
    }
}

#[test]
fn a_box_after_a_forced_break_starts_beside_a_float_that_a_break_inside_it_grew() {
    let body = "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>\
                Z<div style='break-before:page'>Q</div>";
    assert_eq!(words(body), "F@0:0,0 H@1:0,0 Q@1:40,0 Z@0:40,0");
}

#[test]
fn a_float_that_runs_over_several_pages_narrows_the_lines_on_each() {
    let body = "<div style='float:left;width:40px'>F G G G G G G</div>\
                ZZ YY XX WW VV UU TT SS RR QQ PP";
    assert_eq!(
        words(body),
        "F@0:0,0 G@0:0,30 G@0:0,60 G@1:0,0 G@1:0,30 G@2:0,0 G@2:0,30 PP@3:0,30 \
         QQ@3:60,0 RR@3:0,0 SS@2:40,30 TT@2:40,0 UU@1:40,60 VV@1:40,30 WW@1:40,0 \
         XX@0:40,60 YY@0:40,30 ZZ@0:40,0"
    );
}

#[test]
fn what_the_float_growth_already_moves_keeps_its_place() {
    for (body, expected) in [
        // A formatting context grows around the float; the box after it
        // starts below.
        (
            format!("<div style='display:flow-root'>{FLOAT}ZZ</div><div>Q</div>"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 Q@1:0,60 ZZ@0:40,0",
        ),
        // A box that clears the float starts below it.
        (
            format!("{FLOAT}ZZ YY XX WW<div style='clear:left'>Q</div>"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 Q@1:0,60 WW@1:40,30 XX@1:40,0 \
             YY@0:40,30 ZZ@0:40,0",
        ),
        // A paragraph that clears the float takes the full width below it.
        (
            format!("{FLOAT}<p style='clear:left;margin:0'>AA BB CC</p>"),
            "AA@2:0,0 BB@2:60,0 CC@2:0,30 F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30",
        ),
        (
            format!("{FLOAT}ZZ<p style='clear:left;margin:0'>AA BB CC</p>"),
            "AA@2:0,0 BB@2:60,0 CC@2:0,30 F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 ZZ@0:40,0",
        ),
        // A block beside the float that moves to the next page whole moves
        // the flow after it as far as the float grew: the lines after it
        // take the full width.
        (
            format!("{FLOAT}ZZ YY<div style='break-inside:avoid'>XX WW</div>VV UU TT"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 TT@2:0,30 UU@2:60,0 VV@2:0,0 \
             WW@1:40,30 XX@1:40,0 YY@0:40,30 ZZ@0:40,0",
        ),
        // One that moves less wraps beside the float's continuation, and the
        // lines below its new end take the full width again.
        (
            format!("{FLOAT}ZZ YY XX<div style='break-inside:avoid'>WW VV UU</div>TT"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 TT@2:0,0 UU@1:0,60 VV@1:40,30 \
             WW@1:40,0 XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        (
            format!("{FLOAT}ZZ YY XX<div style='orphans:1;widows:1'>WW VV UU TT SS</div>"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 SS@2:0,0 TT@1:60,60 UU@1:0,60 \
             VV@1:40,30 WW@1:40,0 XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
        // A float whose lines move down as far as the lines beside it keeps
        // the end it had.
        (
            "<div style='float:left;width:40px;orphans:1;widows:1'>F G G G</div>\
             ZZ YY XX WW VV"
                .to_string(),
            "F@0:0,0 G@0:0,30 G@0:0,60 G@1:0,0 VV@1:0,30 WW@1:40,0 XX@0:40,60 \
             YY@0:40,30 ZZ@0:40,0",
        ),
    ] {
        assert_eq!(words(&body), expected, "{body}");
    }
}

#[test]
fn the_lines_wrap_beside_each_float_that_continues_on_the_next_page() {
    for (body, expected) in [
        // A float before a block, on the right.
        (
            "<div style='float:right;width:40px'>F G G G</div><div>ZZ YY XX WW VV</div>",
            "F@0:60,0 G@0:60,30 G@1:60,0 G@1:60,30 VV@1:0,30 WW@1:0,0 XX@0:0,60 YY@0:0,30 ZZ@0:0,0",
        ),
        // Two floats on the same side that both run on.
        (
            "<div style='float:left;width:40px'>F G G G</div>\
             <div style='float:left;width:20px'>H I I I</div>ZZ YY XX WW VV",
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 H@0:40,0 I@0:40,30 I@1:40,0 I@1:40,30 VV@1:60,30 WW@1:60,0 XX@0:60,60 YY@0:60,30 ZZ@0:60,0",
        ),
        (
            "<div style='float:right;width:40px'>F G G G</div>\
             <div style='float:right;width:20px'>H I I I</div>ZZ YY XX WW VV",
            "F@0:60,0 G@0:60,30 G@1:60,0 G@1:60,30 H@0:40,0 I@0:40,30 I@1:40,0 I@1:40,30 VV@1:0,30 WW@1:0,0 XX@0:0,60 YY@0:0,30 ZZ@0:0,0",
        ),
    ] {
        assert_eq!(words(body), expected, "{body}");
    }
}

#[test]
fn a_word_too_wide_for_the_room_beside_a_continuing_float_moves_below_it() {
    for (body, expected) in [
        // `WWWW` fits the page but not the 60px beside the float's
        // continuation: its line moves below where the float ends.
        (
            format!("{FLOAT}ZZ YY XX WWWW VV"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 VV@2:0,0 WWWW@1:0,60 XX@0:40,60 \
             YY@0:40,30 ZZ@0:40,0",
        ),
        (
            "<div style='float:right;width:40px'>F G G G</div>ZZ YY XX WWWW VV".to_string(),
            "F@0:60,0 G@0:60,30 G@1:60,0 G@1:60,30 VV@2:0,0 WWWW@1:0,60 XX@0:0,60 \
             YY@0:0,30 ZZ@0:0,0",
        ),
        // The words that fit beside the continuation stay there.
        (
            format!("{FLOAT}ZZ YY XX WW VVVV UU"),
            "F@0:0,0 G@0:0,30 G@1:0,0 G@1:0,30 UU@2:0,0 VVVV@1:0,60 WW@1:40,0 \
             XX@0:40,60 YY@0:40,30 ZZ@0:40,0",
        ),
    ] {
        assert_eq!(words(&body), expected, "{body}");
    }
}
