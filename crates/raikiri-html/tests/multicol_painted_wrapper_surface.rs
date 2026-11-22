//! How a block that holds only blocks and paints a background or borders
//! breaks between the columns of a multicol container. Its boxes join the
//! flow of the columns, as those of a block that paints nothing do, and the
//! block has a piece in each column it reaches, cut where its boxes break: a
//! border above or below them shows only where the block begins or ends. The
//! positions and pieces are the ones Chromium gives the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// `body` laid out on a 100x100 page with 10px Ahem lines 10px apart, where
/// `.m` is a container of two columns 50px wide.
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
         body{{margin:0;font:10px/10px Ahem}}.m{{columns:2;column-gap:0;width:100px}}\
         </style><body>{body}</body>"
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

/// Every word of `body` as `word@x,y`, the left and top of its glyphs, in
/// source order, then the pieces of each box that has more than one, as
/// `(x,y,width,height)` cut by the clips around them, in paint order.
fn layout_of(body: &str) -> String {
    let result = lay_out(body);
    assert!(result.page(1).is_none(), "the content fits one page");
    page_of(&result, 0)
}

/// The words and the pieces of the boxes on the `index`th page of `result`,
/// as [`layout_of`] lists them.
fn page_of(result: &DocumentLayout, index: u32) -> String {
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
        .filter(|(_, pieces)| pieces.len() > 1)
        .map(|(_, pieces)| pieces.concat())
        .collect();
    format!("{} | {}", words.join(" "), pieces.join(" "))
}

#[test]
fn a_block_with_a_background_breaks_between_the_columns() {
    assert_eq!(
        layout_of(
            "<div class=m><div style=background:red>\
             <div>A</div><div>B</div><div>C</div><div>D</div></div></div>"
        ),
        "A@0,0 B@0,10 C@50,0 D@50,10 | (0,0,50,20)(50,0,50,20)"
    );
    // A `flow-root` box breaks the same way.
    assert_eq!(
        layout_of(
            "<div class=m><div style=background:red;display:flow-root>\
             <div>A</div><div>B</div><div>C</div><div>D</div></div></div>"
        ),
        "A@0,0 B@0,10 C@50,0 D@50,10 | (0,0,50,20)(50,0,50,20)"
    );
    // A block between others starts where they end.
    assert_eq!(
        layout_of(
            "<div class=m><div>X</div><div style=background:red>\
             <div>A</div><div>B</div></div><div>Y</div></div>"
        ),
        "X@0,0 A@0,10 B@50,0 Y@50,10 | (0,10,50,10)(50,0,50,10)"
    );
    // A border at the side narrows the boxes in every column.
    assert_eq!(
        layout_of(
            "<div class=m><div style='border-left:5px solid;background:red'>\
             <div>A</div><div>B</div><div>C</div><div>D</div></div></div>"
        ),
        "A@5,0 B@5,10 C@55,0 D@55,10 | (0,0,50,20)(50,0,50,20)"
    );
}

#[test]
fn a_border_above_or_below_shows_where_the_block_begins_or_ends() {
    assert_eq!(
        layout_of(
            "<div class=m><div style='border-top:10px solid'>\
             <div>A</div><div>B</div><div>C</div></div></div>"
        ),
        "A@0,10 B@50,0 C@50,10 | (0,0,50,20)(50,0,50,20)"
    );
    assert_eq!(
        layout_of(
            "<div class=m><div style='border:2px solid;padding:3px'>\
             <div>A</div><div>B</div><div>C</div></div></div>"
        ),
        "A@5,5 B@5,15 C@55,0 | (0,0,50,25)(50,0,50,15)"
    );
    // A block over three columns has a middle piece with neither.
    assert_eq!(
        layout_of(
            "<div class=m style=columns:3;width:150px>\
             <div style='background:red;border-bottom:4px solid'>\
             <div>A</div><div>B</div><div>C</div><div>D</div><div>E</div></div></div>"
        ),
        "A@0,0 B@0,10 C@50,0 D@50,10 E@100,0 | (0,0,50,20)(50,0,50,20)(100,0,50,14)"
    );
}

#[test]
fn a_painted_block_inside_another_has_pieces_of_its_own() {
    assert_eq!(
        layout_of(
            "<div class=m><div style=background:red><div style=background:blue>\
             <div>A</div><div>B</div><div>C</div></div><div>D</div></div></div>"
        ),
        "A@0,0 B@0,10 C@50,0 D@50,10 | (0,0,50,20)(50,0,50,20) (0,0,50,20)(50,0,50,10)"
    );
}

#[test]
fn a_painted_block_breaks_between_columns_of_a_set_height() {
    assert_eq!(
        layout_of(
            "<div class=m style=height:20px;column-fill:auto><div style=background:red>\
             <div>A</div><div>B</div><div>C</div></div></div>"
        ),
        "A@0,0 B@0,10 C@50,0 | (0,0,50,20)(50,0,50,10)"
    );
}

#[test]
fn a_painted_block_breaks_between_the_columns_of_each_page() {
    let lines = "<div>L</div>".repeat(24);
    let result = lay_out(&format!(
        "<div class=m><div style=background:red>{lines}</div>\
         <div style=column-span:all>S</div></div>"
    ));
    let full: Vec<String> = (0..10).map(|row| format!("L@0,{}", row * 10)).collect();
    let second: Vec<String> = (0..10).map(|row| format!("L@50,{}", row * 10)).collect();
    assert_eq!(
        page_of(&result, 0),
        format!(
            "{} {} | (0,0,50,100)(50,0,50,100)",
            full.join(" "),
            second.join(" ")
        )
    );
    assert_eq!(
        page_of(&result, 1),
        "L@0,0 L@0,10 L@50,0 L@50,10 S@0,20 | (0,0,50,20)(50,0,50,20)"
    );
}
