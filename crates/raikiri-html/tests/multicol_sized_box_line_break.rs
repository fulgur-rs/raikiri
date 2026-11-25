//! A box with a height of its own whose lines a column end cuts breaks
//! between two of its lines: the lines after the break go to the top of the
//! next column with the next piece of the box, where they may overflow the
//! rest of its height and overlap what follows it (CSS Fragmentation 3, 4.4).
//! The positions are the ones Chromium gives the same documents.

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

/// `body` laid out on 800x600 pages with no margin and 10px Ahem lines 10px
/// apart, where `.m` is a two-column container 200px wide.
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
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}.m{{columns:2;column-gap:0;width:200px}}</style>\
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

#[test]
fn the_lines_of_a_sized_box_break_between_columns() {
    for (body, expected) in [
        (
            r#"<div class=m><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 kl@0:100,10 mn@0:100,20",
        ),
        (
            r#"<div class=m><div style="height:45px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn op</div></div><div>Z</div>"#,
            "Z@0:0,38 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 kl@0:100,8 mn@0:100,18 op@0:100,28",
        ),
        (
            r#"<div class=m><div style="height:40px;width:20px;widows:1;orphans:1">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:100,0 kl@0:100,10 mn@0:100,20",
        ),
        (
            r#"<div class=m><div style="height:60px;width:20px;widows:3">ab cd ef gh ij kl</div><div style="width:20px">mn op</div></div><div>Z</div>"#,
            "Z@0:0,40 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:100,0 ij@0:100,10 kl@0:100,20 mn@0:100,20 op@0:100,30",
        ),
        (
            r#"<div class=m><div style="width:20px">xx</div><div style="height:40px;width:20px;orphans:3">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,40 ab@0:0,10 cd@0:0,20 ef@0:0,30 gh@0:100,0 kl@0:100,10 mn@0:100,20 xx@0:0,0",
        ),
        (
            r#"<div class=m><div style="width:20px">xx yy</div><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,40 ab@0:0,20 cd@0:0,30 ef@0:100,0 gh@0:100,10 kl@0:100,20 mn@0:100,30 xx@0:0,0 yy@0:0,10",
        ),
        (
            r#"<div class=m><div><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 kl@0:100,10 mn@0:100,20",
        ),
        (
            r#"<div class=m style="columns:3;width:300px"><div style="height:60px;width:20px">ab cd ef gh ij kl</div><div style="width:20px">mn</div></div><div>Z</div>"#,
            "Z@0:0,23 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 ij@0:200,0 kl@0:200,10 mn@0:200,13",
        ),
        (
            r#"<div class=m><div style="height:100px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,60 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:0,30 kl@0:100,40 mn@0:100,50",
        ),
        (
            r#"<div class=m style="height:30px"><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 kl@0:100,10 mn@0:100,20",
        ),
        (
            r#"<div class=m style="height:50px;column-fill:auto"><div style="height:80px;width:20px">ab cd ef gh ij kl</div><div style="width:20px">mn</div></div><div>Z</div>"#,
            "Z@0:0,50 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:0,30 ij@0:100,0 kl@0:100,10 mn@0:100,30",
        ),
        (
            r#"<div class=m><div style="height:40px;width:20px;background:#ccc">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 kl@0:100,10 mn@0:100,20",
        ),
        (
            r#"<div class=m><div style="height:40px;width:20px">ab cd ef gh</div><div style="height:40px;width:20px">kl mn op qr</div></div><div>Z</div>"#,
            "Z@0:0,40 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:0,30 kl@0:100,0 mn@0:100,10 op@0:100,20 qr@0:100,30",
        ),
        (
            r#"<div class=m><div style="width:20px">xx</div><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div><div>Z</div>"#,
            "Z@0:0,35 ab@0:0,10 cd@0:0,20 ef@0:100,0 gh@0:100,10 kl@0:100,15 mn@0:100,25 xx@0:0,0",
        ),
        (
            r#"<div class=m><div style="height:50px;width:20px">ab cd ef gh ij</div><div style="width:20px">kl</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:100,0 ij@0:100,10 kl@0:100,20",
        ),
        (
            r#"<div class=m style="columns:3;width:300px"><div style="height:80px;width:20px">ab cd ef gh ij kl mn op</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:100,0 ij@0:100,10 kl@0:100,20 mn@0:200,0 op@0:200,10",
        ),
        (
            r#"<div class=m><div style="height:40px;width:20px">ab cd ef gh</div></div><div>Z</div>"#,
            "Z@0:0,20 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10",
        ),
        (
            r#"<div class=m><div style="height:60px;width:20px">ab cd</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10",
        ),
        (
            r#"<div class=m><div style="width:20px">xx yy zz</div><div style="height:40px;width:20px">ab cd ef gh</div></div><div>Z</div>"#,
            "Z@0:0,40 ab@0:100,0 cd@0:100,10 ef@0:100,20 gh@0:100,30 xx@0:0,0 yy@0:0,10 zz@0:0,20",
        ),
        (
            r#"<div class=m><div style="width:20px">xx yy zz</div><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl</div></div><div>Z</div>"#,
            "Z@0:0,50 ab@0:0,30 cd@0:0,40 ef@0:100,0 gh@0:100,10 kl@0:100,20 xx@0:0,0 yy@0:0,10 zz@0:0,20",
        ),
        (
            r#"<div class=m><div style="height:40px;width:20px;orphans:3;widows:3">ab cd ef gh</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:100,0",
        ),
        (
            r#"<div class=m style="height:20px"><div style="height:40px;width:20px;orphans:3">ab cd ef gh</div></div><div>Z</div>"#,
            "Z@0:0,20 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10",
        ),
        (
            r#"<div class=m><div style="width:20px;margin-bottom:25px">xx</div><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl</div></div><div>Z</div>"#,
            "Z@0:0,50 ab@0:100,0 cd@0:100,10 ef@0:100,20 gh@0:100,30 kl@0:100,40 xx@0:0,0",
        ),
        (
            r#"<div class=m style="columns:3;width:300px;height:30px;column-fill:auto"><div style="height:80px;width:20px">ab cd ef gh ij kl mn op</div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:0,20 gh@0:100,0 ij@0:100,10 kl@0:100,20 mn@0:200,0 op@0:200,10",
        ),
        (
            r#"<div class=m><div style="padding-top:5px"><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl</div></div></div><div>Z</div>"#,
            "Z@0:0,28 ab@0:0,5 cd@0:0,15 ef@0:100,0 gh@0:100,10 kl@0:100,18",
        ),
        (
            r#"<div class=m><div style="width:20px">xx</div><div style="padding-top:5px"><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl</div></div></div><div>Z</div>"#,
            "Z@0:0,35 ab@0:0,15 cd@0:0,25 ef@0:100,0 gh@0:100,10 kl@0:100,20 xx@0:0,0",
        ),
        (
            r#"<div class=m><div><div style="height:40px;width:20px">ab cd ef gh</div><div style="width:20px">kl mn</div></div></div><div>Z</div>"#,
            "Z@0:0,30 ab@0:0,0 cd@0:0,10 ef@0:100,0 gh@0:100,10 kl@0:100,10 mn@0:100,20",
        ),
    ] {
        assert_eq!(words(body), sorted(expected), "{body}");
    }
}
