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

#[test]
fn the_lines_a_float_runs_beside_onto_a_later_page_take_the_full_width_below_it() {
    // A float 195px tall runs beside the lines of two pages; the page edge
    // below each moves a narrowed line down, and the lines below the float
    // on the third page take the full width. Chromium prints both floats,
    // 195px and 185px tall, the same way.
    for height in [195, 185] {
        let text = vec!["XX"; 16].join(" ");
        let mut expected = Vec::new();
        for page in 0..2 {
            expected.extend((0..3).map(|line| format!("XX@{page}:40,{}", line * 30)));
        }
        for (page, lines) in [(2, 3), (3, 2)] {
            for line in 0..lines {
                for x in [0, 60] {
                    expected.push(format!("XX@{page}:{x},{}", line * 30));
                }
            }
        }
        expected.sort();
        assert_eq!(
            words(&format!(
                "<div style='float:left;width:40px;height:{height}px'></div>{text}"
            )),
            expected.join(" "),
            "{height}"
        );
    }
}

#[test]
fn a_hanging_indent_is_no_float_narrowing_a_line() {
    // Every line but the first is indented 10px, so a full-width line still
    // leaves less room than the content box: only the lines beside the
    // float are narrowed by it, and the page edges after them move full-width
    // lines. Chromium puts one word on each line, three lines on each page,
    // and keeps two lines for each of the last two pages.
    let text = vec!["XX"; 100].join(" ");
    let mut expected = vec![
        "XX@0:40,0".to_string(),
        "XX@0:50,30".to_string(),
        "XX@0:50,60".to_string(),
    ];
    for page in 1..=33 {
        let lines = if page >= 32 { 2 } else { 3 };
        expected.extend((0..lines).map(|line| format!("XX@{page}:10,{}", line * 30)));
    }
    expected.sort();
    assert_eq!(
        words(&format!(
            "<div style='text-indent:10px hanging'>\
             <div style='float:left;width:40px;height:95px'></div>{text}</div>"
        )),
        expected.join(" ")
    );
}

/// Where Chromium puts 100 words of `XX` after a float 40px wide and
/// `height` tall: each line beside the float holds one word, each line
/// below it two, three lines to a page, and the last page keeps two lines.
fn beside_a_float(height: u32) -> String {
    let mut lines = Vec::new();
    let mut left = 100;
    while left > 0 {
        let (page, slot) = (lines.len() as u32 / 3, lines.len() as u32 % 3);
        let beside = page * 100 + slot * 30 < height;
        let count = if beside { 1 } else { 2.min(left) };
        left -= count;
        lines.push((page, slot, beside, count));
    }
    if lines.len() % 3 == 1 {
        let page = lines[lines.len() - 1].0;
        let moved = lines.len() - 2;
        lines[moved].0 = page;
        lines[moved].1 = 0;
        lines[moved + 1].1 = 1;
    }
    let mut words = Vec::new();
    for (page, slot, beside, count) in lines {
        let xs: &[u32] = if beside { &[40] } else { &[0, 60] };
        for x in &xs[..count as usize] {
            words.push(format!("XX@{page}:{x},{}", slot * 30));
        }
    }
    words.sort();
    words.join(" ")
}

#[test]
fn the_lines_below_a_float_many_pages_tall_take_the_full_width() {
    // A float 1095px tall narrows the lines of eleven pages, and each page
    // edge moves a narrowed line down; the lines on page 11, below the
    // float, take the full width. So do those below floats ending on other
    // pages, and those of a paragraph after the float.
    let text = vec!["XX"; 100].join(" ");
    for height in [595, 1095, 1495, 2095] {
        let float = format!("<div style='float:left;width:40px;height:{height}px'></div>");
        assert_eq!(
            words(&format!("{float}{text}")),
            beside_a_float(height),
            "{height}"
        );
        assert_eq!(
            words(&format!("{float}<p style='margin:0'>{text}</p>")),
            beside_a_float(height),
            "{height} paragraph"
        );
    }
}
