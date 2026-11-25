//! The lines of a paragraph that a page end moves past the bottom of a
//! float. Moved lines are laid out again where they land: a float that
//! ends above them no longer narrows them, so each takes the full width,
//! as in Chromium. Where each word falls is what Chromium prints for the
//! same documents on pages 100px square, with Ahem glyphs of 20px on lines
//! of 30px.

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

#[test]
fn the_lines_a_page_end_moves_below_a_float_take_the_full_width() {
    // The float is 95px tall: the fourth line would start 5px above its
    // bottom, but the page end moves it to the next page, where it and the
    // lines after it are no longer beside the float.
    assert_eq!(
        words("<div style='float:left;width:40px;height:95px'></div>XX XX XX XX XX XX XX"),
        "XX@0:40,0 XX@0:40,30 XX@0:40,60 XX@1:0,0 XX@1:0,30 XX@1:60,0 XX@1:60,30"
    );
}

#[test]
fn the_lines_a_page_end_moves_past_a_float_on_a_later_page_take_the_full_width() {
    let lead = "<div style='break-after:page;height:30px;margin-bottom:{before}px'>a</div>";
    for (before, float, margins, top, expected) in [
        (
            0,
            "left",
            "margin-top:20px;margin-bottom:0px",
            0,
            "XX@1:40,0 XX@1:40,30 XX@2:0,0 XX@2:60,0 a@0:0,0",
        ),
        (
            0,
            "left",
            "margin-top:0px;margin-bottom:15px",
            0,
            "XX@1:40,0 XX@1:40,30 XX@2:0,0 XX@2:60,0 a@0:0,0",
        ),
        (
            0,
            "left",
            "margin-top:20px;margin-bottom:15px",
            10,
            "XX@1:40,10 XX@1:40,40 XX@2:0,0 XX@2:60,0 a@0:0,0",
        ),
        (
            30,
            "right",
            "margin-top:20px;margin-bottom:15px",
            0,
            "XX@1:0,0 XX@1:0,30 XX@2:0,0 XX@2:60,0 a@0:0,0",
        ),
    ] {
        let body = format!(
            "{}<div style='float:{float};width:40px;height:50px;{margins}'></div>\
             <div style='margin-top:{top}px'>XX XX XX XX</div>",
            lead.replace("{before}", &before.to_string())
        );
        assert_eq!(words(&body), expected, "{body}");
    }
}

#[test]
fn a_long_paragraph_below_a_short_float_breaks_across_many_pages() {
    // Only the line that the float narrowed takes a gap: the page edges the
    // full-width lines after it cross move them with the room they have, so
    // a paragraph many pages long still lays out. Chromium puts three lines
    // of two words on each page, keeps two of them for the last page, and
    // ends with three words.
    let text = vec!["XX"; 100].join(" ");
    let mut expected = vec![
        "XX@0:40,0".to_string(),
        "XX@0:40,30".to_string(),
        "XX@0:40,60".to_string(),
    ];
    for page in 1..=17 {
        let words: &[(u32, u32)] = match page {
            16 => &[(0, 0), (60, 0), (0, 30), (60, 30)],
            17 => &[(0, 0), (60, 0), (0, 30)],
            _ => &[(0, 0), (60, 0), (0, 30), (60, 30), (0, 60), (60, 60)],
        };
        expected.extend(words.iter().map(|(x, y)| format!("XX@{page}:{x},{y}")));
    }
    expected.sort();
    assert_eq!(
        words(&format!(
            "<div style='float:left;width:40px;height:95px'></div>{text}"
        )),
        expected.join(" ")
    );
}
