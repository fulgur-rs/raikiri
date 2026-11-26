//! Where the columns of a multicol container break a box with a block size
//! of its own and padding or border above or below its content. A column
//! end cuts such a box like one without them, its pieces slicing the
//! padding and border; a cut that would fall inside the padding and border
//! below the content falls where the content ends, and they go on alone at
//! the top of the next column. Where that leaves the break right below the
//! box's last line, the box goes on whole instead, as it does for a
//! paragraph. The positions and pieces are the ones Chromium gives the same
//! documents.

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

/// Every word of `body`, laid out on a 400x400 page with 10px Ahem lines
/// 10px apart, as `word@x,y`, the left and top of its glyphs, in source
/// order, then the pieces of each box that has more than one, as
/// `(x,y,width,height)` cut by the clips around them, in paint order; on a
/// later page, every box piece. `.m`
/// is a container of two columns 100px wide and tall, and every `div` sizes
/// its border box. The content fits one page.
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
        "<!doctype html><style>@page{{size:400px 400px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}} div{{box-sizing:border-box}}\
         .m{{columns:2;column-gap:0;width:200px;height:100px}}\
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
fn a_column_end_cuts_a_box_with_padding_or_border() {
    let cases = [
        // Balanced columns 45px tall cut the second box, as they would one
        // without padding.
        (
            "<div class=m><div style='height:30px;padding:5px'></div>\
             <div style='height:30px;padding:5px'></div><div style='height:30px;padding:5px'></div></div>",
            " | (0,30,100,15)(100,0,100,15)",
        ),
        (
            "<div class=m><div style='height:30px;border:5px solid'></div>\
             <div style='height:30px;border:5px solid'></div><div style='height:30px;border:5px solid'></div></div>",
            " | (0,30,100,15)(100,0,100,15)",
        ),
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='height:60px;padding:5px'></div></div>",
            " | (0,60,100,40)(100,0,100,20)",
        ),
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='height:60px;padding:5px'>L L L</div></div>",
            "L@5,65 L@25,65 L@45,65 | (0,60,100,40)(100,0,100,20)",
        ),
        // The padding above the content does not fit: the box goes on whole.
        (
            "<div class=m style='column-fill:auto'><div style='height:90px'></div>\
             <div style='height:50px;padding-top:20px'></div><div style='height:10px'>Z</div></div>",
            "Z@100,50 | ",
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(layout_of(body), expected, "{body}");
    }
}

#[test]
fn a_cut_inside_the_edge_below_the_content_falls_where_the_content_ends() {
    let cases = [
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='height:50px;padding-bottom:20px'></div><div style='height:10px'>Z</div></div>",
            "Z@100,20 | (0,60,100,30)(100,0,100,20)",
        ),
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='height:50px;border-bottom:20px solid'></div></div>",
            " | (0,60,100,30)(100,0,100,20)",
        ),
        (
            "<div class=m style='column-fill:auto'><div style='height:85px'></div>\
             <div style='height:30px;padding-bottom:20px'></div></div>",
            " | (0,85,100,10)(100,0,100,20)",
        ),
        // With a line in the content, above where it ends.
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='height:50px;padding-bottom:20px'>L</div><div style='height:10px'>Z</div></div>",
            "L@0,60 Z@100,20 | (0,60,100,30)(100,0,100,20)",
        ),
        // Inside a wrapper with padding of its own below the box.
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='padding-bottom:20px'><div style='height:50px;padding-bottom:20px'></div></div>\
             <div style='height:10px'>Z</div></div>",
            "Z@100,40 | (0,60,100,30)(100,0,100,20)",
        ),
        // The wrapper's padding follows the box's own: both go on.
        (
            "<div class=m style='column-fill:auto'><div style='height:60px'></div>\
             <div style='padding-bottom:5px'><div style='height:50px;padding-bottom:20px'></div></div>\
             <div style='height:10px'>Z</div></div>",
            "Z@100,25 | (0,60,100,30)(100,0,100,20)",
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(layout_of(body), expected, "{body}");
    }
}

#[test]
fn a_box_whose_content_ends_with_its_last_line_goes_on_whole() {
    // A cut where the content of the second box ends would fall right below
    // its line: the balanced columns grow to 60px and the third box starts
    // the second column.
    assert_eq!(
        layout_of(
            "<div class=m><div style='height:30px;padding-bottom:20px'>L</div>\
             <div style='height:30px;padding-bottom:20px'>L</div>\
             <div style='height:30px;padding-bottom:20px'>L</div></div>"
        ),
        "L@0,0 L@0,30 L@100,0 | "
    );
}
