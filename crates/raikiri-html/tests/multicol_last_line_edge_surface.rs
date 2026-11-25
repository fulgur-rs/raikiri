//! Where the columns of a multicol container break a paragraph whose last
//! line and the padding and border below it do not fit one column together.
//! With no other break left, the columns break between the line and that
//! edge: the line stays in its column and the edge goes on alone at the top
//! of the next one, overflowing it when it is taller. A break before the
//! paragraph, or between its lines, comes first when there is one. The
//! positions and pieces are the ones Chromium gives the same documents.

use raikiri_html::{
    FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus, PaintEvent, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// Every word of `body`, laid out on a 100x100 page with 10px Ahem lines
/// 50px apart, as `word@x,y`, the left and top of its glyphs, in source
/// order, then the pieces of each box that has more than one, as
/// `(x,y,width,height)` cut by the clips around them, in paint order; on a
/// later page, every box piece. `.m`
/// is a container of `columns:2; column-fill:auto` 100px wide and tall, and
/// `.z` a line 10px tall. The content fits one page.
fn layout_of(body: &str) -> String {
    let (layout, pages) = page_of(body, 0);
    assert_eq!(pages, 1, "the content fits one page");
    layout
}

/// The `index`th page of `body` as [`layout_of`] lists it, with the number
/// of pages.
fn page_of(body: &str, index: u32) -> (String, u32) {
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
         body{{margin:0;font:10px/50px Ahem}}\
         .m{{columns:2;column-fill:auto;column-gap:0;width:100px;height:100px}}\
         .z{{line-height:10px}}</style><body>{body}</body>"
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
    let page = result.page(index).expect("the page");
    let mut words = Vec::new();
    for run in page.text_runs() {
        let mut x = run.origin.0;
        let y = (run.origin.1 - ASCENT).round();
        for word in run.text.split(' ') {
            if !word.trim().is_empty() {
                words.push(format!("{}@{},{}", word.trim(), x.round(), y));
            }
            x += ADVANCE * (word.chars().count() + 1) as f32;
        }
    }
    // The box events with the block-axis clips around them applied.
    let mut clips: Vec<(f32, f32)> = Vec::new();
    let mut boxes: Vec<(raikiri_html::NodeId, Vec<String>)> = Vec::new();
    for event in page.paint_order().iter() {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(if clip.clip_y {
                (clip.rect.y, clip.rect.y + clip.rect.height)
            } else {
                (f32::NEG_INFINITY, f32::INFINITY)
            }),
            PaintEvent::PopClip => {
                clips.pop();
            }
            PaintEvent::Box(fragment) if fragment.kind() == FragmentKind::Box => {
                let rect = fragment.rect();
                let (top, bottom) = clips.iter().fold(
                    (rect.y, rect.y + rect.height),
                    |(top, bottom), &(from, to)| (top.max(from), bottom.min(to)),
                );
                // A piece that the clips hide entirely paints nothing.
                if bottom <= top {
                    continue;
                }
                let piece = format!("({},{},{},{})", rect.x, top, rect.width, bottom - top);
                match boxes.iter_mut().find(|(node, _)| *node == fragment.node()) {
                    Some((_, pieces)) => pieces.push(piece),
                    None => boxes.push((fragment.node(), vec![piece])),
                }
            }
            _ => {}
        }
    }
    let pieces: Vec<String> = boxes
        .into_iter()
        .filter(|(_, pieces)| pieces.len() > 1 || index > 0)
        .map(|(_, pieces)| pieces.concat())
        .collect();
    (
        format!("{} | {}", words.join(" "), pieces.join(" ")),
        result.page_count(),
    )
}

#[test]
fn the_edge_below_a_last_line_goes_on_alone_in_the_next_column() {
    let cases = [
        (
            "<div class=m><div style='padding-bottom:80px'>xxxxx</div><div class=z>Z</div></div>",
            "xxxxx@0,20 Z@50,80 | (0,0,50,50)(50,0,50,80)",
        ),
        (
            "<div class=m><div style='padding-bottom:80px;border-bottom:5px solid'>xxxxx</div>\
             <div class=z>Z</div></div>",
            "xxxxx@0,20 Z@50,85 | (0,0,50,50)(50,0,50,85)",
        ),
        (
            "<div class=m><div style='padding-bottom:60px'>xxxxx</div><div class=z>Z</div></div>",
            "xxxxx@0,20 Z@50,60 | (0,0,50,50)(50,0,50,60)",
        ),
        (
            "<div class=m><div style='padding:20px 0 40px;border-bottom:5px solid'>xxxxx</div>\
             <div class=z>Z</div></div>",
            "xxxxx@0,40 Z@50,45 | (0,0,50,70)(50,0,50,45)",
        ),
        // Without another box after it.
        (
            "<div class=m><div style='padding-bottom:80px'>xxxxx</div></div>",
            "xxxxx@0,20 | (0,0,50,50)(50,0,50,80)",
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(layout_of(body), expected, "{body}");
    }
}

#[test]
fn an_edge_taller_than_a_column_overflows_the_next_one() {
    // The edge, 120px tall, takes all of the second column and runs past the
    // container's end: the box after it starts the third column, and the
    // next page holds the rest of the edge.
    let body = "<div class=m style='columns:3;width:99px'>\
                <div style='padding-bottom:120px;background:green'>xxx</div>\
                <div class=z>Z</div></div>";
    assert_eq!(
        page_of(body, 0),
        ("xxx@0,20 Z@66,0 | (0,0,33,50)(33,0,33,100)".to_owned(), 2)
    );
    assert_eq!(page_of(body, 1).0, " | (33,0,33,20)");
}

#[test]
fn a_break_before_the_paragraph_or_between_its_lines_comes_first() {
    let cases = [
        // The paragraph goes to the next column whole, and breaks there.
        (
            "<div class=m><div>Q</div><div style='padding-bottom:80px'>xxxxx</div>\
             <div class=z>Z</div></div>",
            "Q@0,20 xxxxx@50,20 Z@100,80 | (50,0,50,50)(100,0,50,80)",
        ),
        // Three columns: the break between the lines, though orphans and
        // widows forbid it, comes before the one above the edge.
        (
            "<div class=m style='columns:3;width:99px'><div style='padding-bottom:80px'>xxx yyy</div>\
             <div class=z>Z</div></div>",
            "xxx@0,20 yyy@33,20 Z@66,80 | (0,0,33,100)(33,0,33,50)(66,0,33,80)",
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(layout_of(body), expected, "{body}");
    }
}

#[test]
fn balanced_columns_break_between_the_last_line_and_the_edge_too() {
    assert_eq!(
        layout_of(
            "<div class=m style='column-fill:balance'><div style='padding-bottom:80px'>xxxxx</div>\
             <div class=z>Z</div></div>"
        ),
        "xxxxx@0,20 Z@50,80 | (0,0,50,50)(50,0,50,80)"
    );
}

#[test]
fn the_padding_below_a_wrapper_goes_on_alone_in_the_next_column() {
    let cases = [(
        "<div class=m><div style='padding-bottom:60px;background:green'><div>xxxxx</div></div>\
             <div class=z>Z</div></div>",
        "xxxxx@0,20 Z@50,60 | (0,0,50,50)(50,0,50,60)",
    )];
    for (body, expected) in cases {
        assert_eq!(layout_of(body), expected, "{body}");
    }
}

#[test]
fn padding_below_a_wrapper_taller_than_a_column_stays_whole() {
    // The padding goes whole to the second column and runs past its end, so
    // the box after it starts the third.
    let body = "<div class=m style='columns:3;width:99px'>\
                <div style='padding-bottom:150px;background:green'><div>Q</div></div>\
                <div class=z>Z</div></div>";
    assert_eq!(
        page_of(body, 0).0,
        "Q@0,20 Z@66,0 | (0,0,33,50)(33,0,33,100)"
    );
}
