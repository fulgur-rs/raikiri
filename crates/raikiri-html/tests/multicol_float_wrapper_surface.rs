//! A block with no box of its own to break (no margin, padding, border or
//! background) that holds a float lets its children break between the
//! columns of a multicol container, as the container's own children do: the
//! float stays in the column it is placed in, and the lines after it flow
//! beside it. The positions are the ones Chromium prints for the same
//! documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// `body` laid out on 100x100 pages with no margin and 10px Ahem lines 10px
/// apart, where `.m` is a two-column container 100px wide.
fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}.m{{columns:2;column-gap:0;width:100px}}</style>\
         <body>{body}</body>"
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
    result
}

/// Every word of `body` that the clips in effect leave visible, as
/// `word@page:x,y`, the left and top of its glyphs on its page, sorted.
fn words(body: &str) -> String {
    let result = lay_out(body);
    let mut words = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("a page");
        let runs = page.text_runs();
        let mut clips = Vec::new();
        for event in page.paint_order_for_text_runs(&runs) {
            let line = match event {
                PaintEvent::PushClip(clip, _) => {
                    clips.push(clip);
                    continue;
                }
                PaintEvent::PopClip => {
                    clips.pop();
                    continue;
                }
                PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => line,
                _ => continue,
            };
            for run in runs.iter().filter(|run| run.line == line) {
                let mut x = run.origin.0;
                let y = (run.origin.1 - ASCENT).round();
                for word in run.text.split(' ') {
                    // A clip leaves the axes it does not clip open.
                    let visible = clips.iter().all(|clip| {
                        let rect = clip.rect;
                        (!clip.clip_x || (rect.x..rect.x + rect.width).contains(&x))
                            && (!clip.clip_y || (rect.y..rect.y + rect.height).contains(&y))
                    });
                    if visible && !word.trim().is_empty() {
                        words.push(format!("{}@{index}:{},{}", word.trim(), x.round(), y));
                    }
                    x += ADVANCE * (word.chars().count() + 1) as f32;
                }
            }
        }
    }
    words.sort();
    words.join(" ")
}

/// The words of `expected`, sorted as `words` gives them.
fn sorted(expected: &str) -> String {
    let mut words: Vec<_> = expected.split(' ').collect();
    words.sort();
    words.join(" ")
}

/// The children of a wrapper that holds a float break between the columns.
#[test]
fn the_children_of_a_wrapper_with_a_float_break_between_columns() {
    for (body, expected) in [
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:right'>F</div><div>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:0,10 C@0:50,0 F@0:40,10",
        ),
        (
            "<div class=m><div><div style='float:left'>F</div><div>A</div><div>B</div><div>C</div></div></div>",
            "A@0:10,0 B@0:0,10 C@0:50,0 F@0:0,0",
        ),
        (
            "<div class=m><div><div>A</div><div>B</div><div>C</div><div style='float:left'>F</div></div></div>",
            "A@0:0,0 B@0:0,10 C@0:50,0 F@0:50,10",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div style='clear:both'>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:50,0 C@0:50,10 F@0:0,10",
        ),
        (
            "<div class=m><div><div><div>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div>X</div><div><div>A</div><div style='float:left'>F</div><div>B</div></div><div>C</div></div>",
            "A@0:0,10 B@0:60,0 C@0:50,10 F@0:50,0 X@0:0,0",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div>B</div><div>C</div><div>D</div><div>E</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:0,20 D@0:50,0 E@0:50,10 F@0:0,10",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div style='float:left'>G</div><div>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:20,10 C@0:50,0 F@0:0,10 G@0:10,10",
        ),
        (
            "<div class=m><div><div style='display:list-item;list-style:none'>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m style='direction:rtl'><div><div>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div>",
            "A@0:90,0 B@0:90,10 C@0:40,0 F@0:50,10",
        ),
        (
            "<div class=m style='padding:10px;width:80px'><div><div>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div>",
            "A@0:10,10 B@0:20,20 C@0:50,10 F@0:10,20",
        ),
        (
            "<div class=m><div><div style='float:left'>F</div><div>A B C D</div></div></div>",
            "A@0:10,0 B@0:30,0 C@0:0,10 D@0:20,10 F@0:0,0",
        ),
        (
            "<div class=m><div><div>A</div><span style='float:left'>F</span><div>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div style='overflow:hidden'>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div>X</div><div><div style='float:left'>F</div></div><div>A</div><div>B</div></div>",
            "A@0:10,10 B@0:50,0 F@0:0,10 X@0:0,0",
        ),
        (
            "<div class=m><div><div><div><div>A</div><div style='float:left'>F</div></div><div>B</div></div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div>B</div><div style='height:20px'></div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,15 F@0:0,10",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div>B B B B B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:0,20 B@0:10,10 B@0:20,20 B@0:30,10 B@0:40,20 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div><div style='float:right'>F</div><div style='float:left'>G</div><div>A</div><div>B</div><div>C</div><div>D</div></div></div>",
            "A@0:10,0 B@0:0,10 C@0:50,0 D@0:50,10 F@0:40,0 G@0:0,0",
        ),
        (
            "<div class=m><div><div>A</div><div style='float:left'>F</div><div style='position:relative;top:5px'>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,15 C@0:50,0 F@0:0,10",
        ),
        (
            "<div class=m><div style='display:contents'><div>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div>",
            "A@0:0,0 B@0:10,10 C@0:50,0 F@0:0,10",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

#[test]
fn a_wrapper_of_another_direction_keeps_its_children_on_its_own_side() {
    // An ltr wrapper in an rtl multicol aligns a fixed-width child to its own
    // left edge, as in Chromium (A@50,0 in the first column on the right).
    let body = "<div class=m style='direction:rtl'><div style='direction:ltr'><div style='width:10px'>A</div><div style='float:left'>F</div><div>B</div><div>C</div></div></div>";
    let words = words(body);
    assert!(words.split(' ').any(|word| word == "A@0:50,0"), "{words}");
    assert!(words.split(' ').any(|word| word == "F@0:50,10"), "{words}");
    assert!(words.split(' ').any(|word| word == "B@0:60,10"), "{words}");
}

#[test]
fn a_wrapper_with_an_aspect_ratio_keeps_its_height() {
    let body = "<div class=m style='column-fill:auto;height:100px'><div style='aspect-ratio:1'><div style='float:left'>F</div><div>A</div></div><div>B</div></div>";
    // Chromium: A@0:10,0 B@0:0,50 F@0:0,0. The wrapper keeps its 50px
    // height, so B does not move up beside its short children. Where B goes
    // after the wrapper is left to the path a wrapper with a float takes
    // without opening it.
    let words = words(body);
    let has = |word: &str| words.split(' ').any(|w| w == word);
    assert!(has("A@0:10,0") && has("F@0:0,0"), "{words}");
    assert!(!has("B@0:0,10"), "{words}");
}
