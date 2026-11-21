//! Where the content of a spanner and the columns after it land when the
//! spanner crosses a page end, in a multicol container whose children are
//! boxes. A page breaks inside a spanner as it does inside any other block
//! (CSS Fragmentation 3, 4.4), and the columns after the spanner start below
//! the part of it on their page (CSS Multi-column Layout 1, 6.1). The
//! positions are the ones Chromium prints for the same documents.

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

/// The words of `expected`, sorted as [`words`] gives them.
fn sorted(expected: &str) -> String {
    let mut words: Vec<_> = expected.split(' ').collect();
    words.sort();
    words.join(" ")
}

/// The lines of a spanner fill the page and the rest continue at the top of
/// the next one, with the columns after the spanner below them.
#[test]
fn a_spanner_that_crosses_a_page_end_breaks_between_its_lines() {
    for (body, expected) in [
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='width:20px'>aa bb cc dd ee ff gg hh ii</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,15 bb@0:0,25 cc@0:0,35 dd@0:0,45 ee@0:0,55 ff@0:0,65 gg@0:0,75 hh@1:0,0 ii@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='width:20px'>aa bb cc dd ee ff gg hh ii jj</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,15 bb@0:0,25 cc@0:0,35 dd@0:0,45 ee@0:0,55 ff@0:0,65 gg@0:0,75 hh@0:0,85 ii@1:0,0 jj@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:75px'>S</div><div style='width:20px'>aa bb cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@1:0,0 bb@1:0,10 cc@1:0,20 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:65px'>S</div><div style='width:20px;orphans:2;widows:2'>aa bb cc dd</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,80 bb@0:0,90 cc@1:0,0 dd@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:75px'>S</div><div style='width:20px'>aa bb cc dd</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@1:0,0 bb@1:0,10 cc@1:0,20 dd@1:0,30 C@1:0,40 X@1:0,50",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all;background:yellow'><div style='width:20px'>aa bb cc dd ee ff gg hh ii jj</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,15 bb@0:0,25 cc@0:0,35 dd@0:0,45 ee@0:0,55 ff@0:0,65 gg@0:0,75 hh@0:0,85 ii@1:0,0 jj@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='width:20px'>aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,15 bb@0:0,25 cc@0:0,35 dd@0:0,45 ee@0:0,55 ff@0:0,65 gg@0:0,75 hh@0:0,85 ii@1:0,0 jj@1:0,10 kk@1:0,20 ll@1:0,30 mm@1:0,40 nn@1:0,50 oo@1:0,60 pp@1:0,70 qq@1:0,80 rr@1:0,90 C@2:0,0 X@2:0,10",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='width:20px'>aa bb cc dd ee ff gg hh ii</div></div><div style='width:20px'>c1 c2 c3 c4</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,15 bb@0:0,25 cc@0:0,35 dd@0:0,45 ee@0:0,55 ff@0:0,65 gg@0:0,75 hh@1:0,0 ii@1:0,10 c1@1:0,20 c2@1:0,30 X@1:0,40 c3@1:50,20 c4@1:50,30",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

/// The boxes of a spanner that cross the page end move to the next page, or
/// break between their own lines, as they do outside a spanner.
#[test]
fn a_spanner_that_crosses_a_page_end_breaks_between_its_boxes() {
    for (body, expected) in [
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><span><div style='width:20px'>aa bb cc dd</div>t</span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,80 bb@0:0,90 cc@1:0,0 dd@1:0,10 t@1:0,20 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:15px solid'><span><div style='width:20px'>aa bb cc dd</div>t</span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@1:0,15 bb@1:0,25 cc@1:0,35 dd@1:0,45 t@1:0,55 C@1:0,65 X@1:0,75",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div style='display:flex;height:40px'>F</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 F@0:0,75 C@1:0,15 X@1:0,25",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div style='display:grid'><div style='width:20px'>aa bb cc</div></div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all;display:flex;flex-direction:column'><div style='height:60px'>S</div><div>aa</div><div>bb</div><div>cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><span><div style='height:60px'>S</div><div style='width:20px'>aa bb cc dd</div></span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 dd@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><span><div style='width:20px'>aa bb cc dd</div></span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,80 bb@0:0,90 cc@1:0,0 dd@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:15px solid'><span><div style='width:20px'>aa bb cc dd</div></span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@1:0,15 bb@1:0,25 cc@1:0,35 dd@1:0,45 C@1:0,55 X@1:0,65",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><span>t<div style='width:20px'>aa bb cc dd</div></span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 t@1:0,5 aa@1:0,15 bb@1:0,25 cc@1:0,35 dd@1:0,45 C@1:0,55 X@1:0,65",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><span>t<br>u<div style='width:20px'>aa bb cc dd</div></span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 t@0:0,80 u@0:0,90 aa@1:0,0 bb@1:0,10 cc@1:0,20 dd@1:0,30 C@1:0,40 X@1:0,50",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid;orphans:1'><span>t<div style='width:20px'>aa bb cc dd</div></span></div><div>C</div></div><div>X</div>",
            "A@0:0,0 t@0:0,80 aa@0:0,90 bb@1:0,0 cc@1:0,10 dd@1:0,20 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:45px'></div><div style='column-span:all;border-top:5px solid'><span>t<div style='width:20px'>aa bb</div></span><div style='height:40px'>Q</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 t@0:0,60 aa@0:0,70 bb@0:0,80 Q@0:0,90 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><div style='float:right;height:60px;width:10px'></div><div style='width:20px'>aa bb cc dd</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,80 bb@0:0,90 cc@1:0,0 dd@1:0,10 C@1:0,40 X@1:0,50",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid;position:relative'><div style='position:absolute;height:60px;width:10px;left:80px'></div><div style='width:20px'>aa bb cc dd</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,80 bb@0:0,90 cc@1:0,0 dd@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><div style='width:20px'>aa bb cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,80 bb@0:0,90 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:65px'></div><div style='column-span:all;border-top:5px solid'><div style='width:20px;orphans:1;widows:3'>aa bb cc dd</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 aa@0:0,80 bb@1:0,0 cc@1:0,10 dd@1:0,20 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all'><div style='height:80px'>S</div><div style='height:20px'>R</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,10 R@0:0,90 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all'><div style='height:80px'>S</div><div style='height:25px'>R</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,10 R@0:0,90 C@1:0,15 X@1:0,25",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all'><div style='height:85px'>S</div><div style='height:20px'>R</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:30px'>S</div><div style='height:60px'>R</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 R@0:0,45 C@1:0,5 X@1:0,15",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:50px'>S</div><div style='height:40px'>R</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 R@0:0,65 C@1:0,5 X@1:0,15",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:70px'>S</div><div style='padding:5px 0;width:20px'>aa bb</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@1:0,5 bb@1:0,15 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all;padding:5px 0'><div style='height:70px'>S</div><div style='width:20px'>aa bb</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,20 aa@1:0,0 bb@1:0,10 C@1:0,25 X@1:0,35",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div style='width:20px;break-inside:avoid'>aa bb cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@1:0,0 bb@1:0,10 cc@1:0,20 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all'><div><div style='height:80px'>S</div><div style='height:20px'>R</div></div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,10 R@0:0,90 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div style='margin-top:10px;width:20px'>aa bb cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@1:0,0 bb@1:0,10 cc@1:0,20 C@1:0,30 X@1:0,40",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div><div style='height:40px;width:20px;display:inline-block;vertical-align:top'>I</div></div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 I@1:0,0 C@1:0,40 X@1:0,50",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:50px'>S</div><div style='height:40px'>R</div></div><div>C</div><div style='column-span:all;height:20px'>T</div><div>D</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 R@0:0,65 C@1:0,5 T@1:0,15 D@1:0,35 X@1:0,45",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

/// Table rows, floats, list items and relatively positioned boxes that move
/// to the next page take the columns after the spanner with them.
#[test]
fn the_columns_after_a_spanner_follow_the_content_that_moves_inside_it() {
    for (body, expected) in [
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><table style='border-spacing:0'><tr><td style='padding:0'>aa</td></tr><tr><td style='padding:0'>bb</td></tr><tr><td style='padding:0'>cc</td></tr><tr><td style='padding:0'>dd</td></tr></table></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 dd@1:0,10 C@1:0,20 X@1:0,30",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><table style='column-span:all;border-spacing:0'><tr><td style='padding:0;height:60px'>S</td></tr><tr><td style='padding:0'>aa</td></tr><tr><td style='padding:0'>bb</td></tr><tr><td style='padding:0'>cc</td></tr></table><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,40 aa@0:0,75 bb@0:0,85 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><table style='border-spacing:0'><tr><td style='padding:0;width:20px'>aa bb cc</td></tr></table></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div style='float:left;width:20px'>aa bb cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><ul style='margin:0;padding:0;list-style:none'><li>aa</li><li>bb</li><li>cc</li></ul></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,75 bb@0:0,85 cc@1:0,0 C@1:0,10 X@1:0,20",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all;height:5px'></div><div style='column-span:all'><div style='height:60px'>S</div><div style='position:relative;top:3px;width:20px'>aa bb cc</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,15 aa@0:0,78 bb@0:0,88 cc@1:0,3 C@1:0,10 X@1:0,20",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}

/// A spanner that crosses a page end and also holds a forced break continues
/// across both.
#[test]
fn a_spanner_breaks_at_a_page_end_and_at_a_forced_break() {
    for (body, expected) in [
        (
            "<div class='m'><div>A</div><div style='column-span:all'><div>S</div><div style='break-before:page;height:85px'>R</div><div style='height:20px'>Q</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 Q@1:0,85 C@2:0,5 X@2:0,15",
        ),
        (
            "<div class='m'><div>A</div><div style='column-span:all'><div style='height:85px'>S</div><div style='height:20px'>R</div><div style='break-before:page'>Q</div></div><div>C</div></div><div>X</div>",
            "A@0:0,0 S@0:0,10 R@1:0,0 Q@2:0,0 C@2:0,10 X@2:0,20",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}
