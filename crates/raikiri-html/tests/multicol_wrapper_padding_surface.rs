//! Where a wrapper's padding and its first and last boxes land in a balanced
//! multicol container. A break between the padding at the top of a wrapper
//! and its first box is a last resort, so balancing keeps the two together
//! and moves the wrapper to the next column as a whole when the column
//! already holds content (CSS Fragmentation 3, 4.4). Balanced columns grow
//! rather than leave the padding at the bottom of a wrapper alone at the top
//! of the next column. The positions are the ones Chromium prints for the
//! same documents.

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

/// Every word of `body` on the first page that the clips in effect leave
/// visible, as `word@x,y`, the left and top of its glyphs, sorted.
fn words(body: &str) -> String {
    let result = lay_out(body);
    assert_eq!(result.page_count(), 1, "the document fits on one page");
    let page = result.page(0).expect("a page");
    let runs = page.text_runs();
    let mut clips = Vec::new();
    let mut words = Vec::new();
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
                let visible = clips.iter().all(|clip| {
                    let rect = clip.rect;
                    (!clip.clip_x || (rect.x..rect.x + rect.width).contains(&x))
                        && (!clip.clip_y || (rect.y..rect.y + rect.height).contains(&y))
                });
                if visible && !word.trim().is_empty() {
                    words.push(format!("{}@{},{}", word.trim(), x.round(), y));
                }
                x += ADVANCE * (word.chars().count() + 1) as f32;
            }
        }
    }
    words.sort();
    words.join(" ")
}

#[test]
fn top_padding_stays_with_the_first_box() {
    let body = "<div class=m><div style='padding-top:1px'><div>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,1 X@0,11");
}

#[test]
fn top_padding_stays_with_the_first_box_below_a_spanner() {
    let body = "<div class=m><div style='column-span:all'>S</div>\
                <div style='padding-top:1px'><div>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,11 S@0,0 X@0,21");
}

#[test]
fn thick_top_padding_stays_with_the_first_box() {
    let body = "<div class=m><div style='column-span:all'>S</div>\
                <div style='padding-top:5px'><div>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,15 S@0,0 X@0,25");
    let body = "<div class=m><div style='padding-top:15px'><div>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,15 X@0,25");
}

#[test]
fn top_padding_after_a_margin_stays_with_the_first_box() {
    let body = "<div class=m><div style='column-span:all'>S</div>\
                <div style='margin-top:1px;padding-top:1px'><div>C</div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "C@0,12 S@0,0 X@0,22");
}

#[test]
fn nested_top_paddings_stay_with_the_first_box() {
    let body = "<div class=m><div style='padding-top:1px'><div style='padding-top:1px'>\
                <div>C</div></div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,2 X@0,12");
}

#[test]
fn bottom_padding_stays_with_the_last_box() {
    let body = "<div class=m><div style='column-span:all'>S</div>\
                <div style='padding-bottom:1px'><div>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,10 S@0,0 X@0,21");
    let body = "<div class=m><div>A</div><div style='padding-bottom:4px'><div>C</div></div>\
                <div>D</div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@0,10 D@50,0 X@0,24");
    let body = "<div class=m><div style='padding-top:2px'><div style='padding-bottom:2px'>\
                <div>C</div></div></div><div>D</div></div><div>X</div>";
    assert_eq!(words(body), "C@0,2 D@50,0 X@0,14");
}

#[test]
fn padded_wrapper_between_spanners_stays_whole() {
    let body = "<div class=m><div style='column-span:all'>S</div>\
                <div style='padding-top:1px'><div>C</div></div>\
                <div style='column-span:all'>T</div></div><div>X</div>";
    assert_eq!(words(body), "C@0,11 S@0,0 T@0,21 X@0,31");
}

#[test]
fn padded_wrapper_moves_whole_to_the_next_column() {
    let body = "<div class=m style='height:10px'><div style='padding-top:1px'><div>C</div>\
                </div></div><div>X</div>";
    assert_eq!(words(body), "C@50,0 X@0,10");
    let body = "<div class=m style='height:20px;column-fill:auto'><div>A</div>\
                <div style='padding-top:5px'><div>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@50,5 X@0,20");
    let body = "<div class=m><div>A</div><div>B</div><div style='padding-top:3px'>\
                <div>C</div><div>D</div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,3 D@50,13 X@0,23");
}

#[test]
fn padded_wrapper_splits_between_its_boxes() {
    let body = "<div class=m><div style='padding-top:5px'><div>C</div><div>D</div>\
                <div>E</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,5 D@50,0 E@50,10 X@0,20");
    let body = "<div class=m><div style='padding:3px 0'><div>C</div><div>D</div><div>E</div>\
                <div>F</div></div></div><div>X</div>";
    assert_eq!(words(body), "C@0,3 D@0,13 E@50,0 F@50,10 X@0,23");
    let body = "<div class=m><div>A</div><div style='padding-top:3px'><div>C<br>D<br>E</div>\
                </div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@0,13 D@0,23 E@50,0 X@0,33");
}

#[test]
fn empty_padded_wrapper_takes_only_its_padding() {
    let body = "<div class=m><div>A</div><div style='padding-top:3px'><div></div></div>\
                <div>B</div><div>C</div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@50,0 C@50,10 X@0,20");
}

#[test]
fn margin_below_the_top_padding_moves_with_the_wrapper() {
    let body = "<div class=m><div>A</div><div style='padding-top:2px'>\
                <div style='margin-top:5px'><div>C</div></div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@50,7 X@0,17");
    let body = "<div class=m><div>A</div><div>B</div><div style='padding-top:2px'>\
                <div style='margin-top:5px'><div>C</div></div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,7 X@0,20");
    let body = "<div class=m style='height:20px;column-fill:auto'><div>A</div>\
                <div style='padding-top:3px'><div style='margin-top:4px'><div>C</div></div>\
                </div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@50,7 X@0,20");
}

#[test]
fn negative_margin_below_the_top_padding_pulls_the_box_up() {
    let body = "<div class=m><div>A</div><div style='padding-top:5px'>\
                <div style='margin-top:-5px'>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@50,0 X@0,10");
    let body = "<div class=m style='height:20px;column-fill:auto'><div>A</div>\
                <div style='padding-top:5px'><div style='margin-top:-5px'>C</div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "A@0,0 C@0,10 X@0,20");
}

#[test]
fn last_line_goes_with_the_bottom_padding_and_the_columns_keep_their_height() {
    let body = "<div class=m><div style='padding:5px'><div>a<br>b</div></div></div><div>X</div>";
    assert_eq!(words(body), "X@0,25 a@5,5 b@55,0");
    let body = "<div class=m><div style='padding-bottom:5px'><div>a<br>b</div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "X@0,20 a@0,0 b@50,0");
    let body = "<div class=m style='height:60px;align-content:end'><div style='padding:5px'>\
                <div>a<br>b</div></div></div><div>X</div>";
    assert_eq!(words(body), "X@0,60 a@5,40 b@55,35");
}

#[test]
fn last_line_goes_with_the_bottom_padding_past_white_space_and_margins() {
    let body = "<div class=m><div style='padding-bottom:5px'> <div>a<br>b</div> </div></div>\
                <div>X</div>";
    assert_eq!(words(body), "X@0,20 a@0,0 b@50,0");
    let body = "<div class=m><div style='padding-bottom:5px'>\
                <div style='margin-bottom:10px'>a<br>b</div></div></div><div>X</div>";
    assert_eq!(words(body), "X@0,25 a@0,0 b@50,0");
}

#[test]
fn last_line_goes_with_the_bottom_padding_in_columns_of_a_definite_height() {
    let body = "<div class=m style='height:30px;column-fill:auto'><div>A</div>\
                <div style='padding-bottom:5px'><div>B<br>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,0 X@0,30");
    let body = "<div class=m style='height:20px;column-fill:auto'>\
                <div style='padding-bottom:5px'><div>a<br>b</div></div></div><div>X</div>";
    assert_eq!(words(body), "X@0,20 a@0,0 b@50,0");
}

#[test]
fn white_space_and_empty_boxes_below_the_top_padding_take_no_room() {
    let body = "<div class=m><div>A</div><div style='padding-top:1px'> <div>C</div> </div></div>\
                <div>X</div>";
    assert_eq!(words(body), "A@0,0 C@50,1 X@0,11");
    let body = "<div class=m><div>A</div><div style='padding:3px 0'><div></div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "A@0,0 X@0,10");
    let body = "<div class=m><div>A</div><div style='padding-top:3px'><div></div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "A@0,0 X@0,10");
    let body = "<div class=m><div>A</div><div>B</div><div style='padding-top:3px'><div></div>\
                </div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@50,0 X@0,13");
    let body = "<div class=m><div>A</div><div>B</div><div style='padding:3px 0'><div></div>\
                </div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@50,0 X@0,16");
}

#[test]
fn bottom_padding_stays_with_a_sized_last_box() {
    let body = "<div class=m><div>A</div><div>B</div><div style='padding-bottom:5px'>\
                <div style='height:10px'></div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 X@0,20");
    let body = "<div class=m><div>A</div><div>B</div><div style='padding-bottom:5px'><div>C</div>\
                <div style='height:10px'></div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,0 X@0,25");
    let body = "<div class=m><div>A</div><div>B</div><div>C</div>\
                <div style='padding-bottom:5px'><div style='height:10px'></div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,0 X@0,25");
    let body = "<div class=m><div>A</div><div>B</div><div>C</div>\
                <div style='padding-bottom:5px'><div style='height:10px'>D</div></div></div>\
                <div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,0 D@50,10 X@0,25");
}

#[test]
fn top_padding_stays_with_a_sized_first_box() {
    let body = "<div class=m><div>A</div><div style='padding-top:3px'>\
                <div style='height:10px'>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 C@50,3 X@0,13");
    let body = "<div class=m><div>A</div><div>B</div><div style='padding-top:3px'>\
                <div style='height:10px'>C</div></div></div><div>X</div>";
    assert_eq!(words(body), "A@0,0 B@0,10 C@50,3 X@0,20");
    let body = "<div class=m><div style='padding-top:3px'><div style='height:20px'></div></div>\
                </div><div>X</div>";
    assert_eq!(words(body), "X@0,12");
}
