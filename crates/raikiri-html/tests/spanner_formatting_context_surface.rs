//! Floats inside a box that spans all columns stay inside it. A spanner
//! establishes a new block formatting context (CSS Multi-column Layout 1, 6.1),
//! so the lines after it do not move aside for the floats it holds, and its
//! margins do not collapse with those of its children. The positions are the
//! ones Chromium prints for the same documents.

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

/// The columns after a spanner inside the spanner of an outer multicol
/// container start at the left edge, below the float the inner spanner holds.
#[test]
fn the_lines_after_a_nested_spanner_do_not_move_aside_for_its_floats() {
    for (body, expected) in [
        (
            "<div class=m><div style='column-span:all'><div class=m><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div></div></div></div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div class=m><div style='column-span:all'><div class=m><div style='column-span:all'><div style='float:left;width:20px'>aa</div>S</div><div>C</div></div></div></div>",
            "aa@0:0,0 S@0:20,0 C@0:0,10",
        ),
        (
            "<div class=m><div style='column-span:all'><div class=m><div>B</div><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div></div></div></div>",
            "B@0:0,0 aa@0:0,10 C@0:0,20",
        ),
        (
            "<div class=m><div style='column-span:all'><div><div class=m><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div></div></div></div></div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div class=m><div>A</div><div style='column-span:all'><div class=m><div>B</div><div style='column-span:all'><div style='height:30px'>S</div><div style='float:left;width:20px'>aa bb cc</div></div><div>C</div></div></div><div>D</div></div><div>X</div>",
            "A@0:0,0 B@0:0,10 S@0:0,20 aa@0:0,50 bb@0:0,60 cc@0:0,70 C@0:0,80 D@0:0,90 X@1:0,0",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

/// A spanner in the columns of one container holds its floats too, as a
/// list item that spans the columns does.
#[test]
fn the_lines_after_a_spanner_do_not_move_aside_for_its_floats() {
    for (body, expected) in [
        (
            "<div class=m><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div></div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div class=m><div>B</div><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div><div>D</div></div>",
            "B@0:0,0 aa@0:0,10 C@0:0,20 D@0:50,20",
        ),
        (
            "<div class=m><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div>C</div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div class=m><div><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div></div></div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div class=m><div style='display:list-item;column-span:all;list-style:none'><div style='float:left;width:20px'>aa</div></div><div>C</div></div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div class=m><div style='float:left;width:20px'>aa</div><div style='column-span:all'>S</div><div>C</div></div>",
            "aa@0:0,0 S@0:0,10 C@0:0,20",
        ),
        (
            "<div class=m><div style='column-span:all'><div style='float:left;width:20px;height:20px'>aa</div>S</div><div>C</div></div>",
            "aa@0:0,0 S@0:20,0 C@0:0,20",
        ),
        (
            "<div class=m><div>A</div><div style='column-span:all'><div class=m><div>B</div><div style='column-span:all'><div style='float:left;width:20px;height:30px'>aa</div></div><div>C</div><div>D</div></div></div><div>E</div></div>",
            "A@0:0,0 B@0:0,10 aa@0:0,20 C@0:0,50 E@0:0,60 D@0:50,50",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

/// The margins of the children of a spanner stay inside it.
#[test]
fn the_margins_of_the_children_of_a_spanner_stay_inside_it() {
    for (body, expected) in [
        (
            "<div class=m><div>A</div><div style='column-span:all'><div style='margin-top:10px'>S</div></div><div>C</div></div>",
            "A@0:0,0 S@0:0,20 C@0:0,30",
        ),
        (
            "<div class=m><div>A</div><div style='column-span:all;margin-top:5px'><div style='margin-top:10px'>S</div></div><div>C</div></div>",
            "A@0:0,0 S@0:0,25 C@0:0,35",
        ),
        (
            "<div class=m><div>A</div><div style='column-span:all'><div style='margin-bottom:10px'>S</div></div><div>C</div></div>",
            "A@0:0,0 S@0:0,10 C@0:0,30",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

/// A box with `column-span: all` outside any multicol container holds its
/// floats and the margins of its children as well, as browsers lay it out.
#[test]
fn a_box_that_spans_all_columns_outside_a_multicol_container_holds_its_floats() {
    for (body, expected) in [
        (
            "<div><div style='column-span:all'><div style='float:left;width:20px'>aa</div></div><div>C</div></div>",
            "aa@0:0,0 C@0:0,10",
        ),
        (
            "<div><div>A</div><div style='column-span:all'><div style='margin-top:10px'>S</div></div><div>C</div></div>",
            "A@0:0,0 S@0:0,20 C@0:0,30",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}
