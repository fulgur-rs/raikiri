//! A box that flows horizontally in a vertical root is never narrower than its
//! padding and border across, or shorter than they are down, whatever size it
//! states (CSS Box Sizing 3 section 5.1): `width: 60px` with 31px of padding on
//! both sides is a border box 62px wide. A flex container took the width it was
//! given as it was, so it came out narrower than its own padding and was
//! dropped from the page as an empty box. The root is 100px wide and 200px
//! tall. The expected border boxes are the ones Chromium lays out for the same
//! markup, in an 800px by 600px viewport without margins and in Ahem at 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A row of the table: the style of the child `#c`, which holds a block `#g`
/// of twelve words, and the boxes of the ids that Chromium lays out for it in
/// a `vertical-rl` and in a `vertical-lr` root.
type Row = (&'static str, &'static str, &'static str);

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Check that every row of `rows` is laid out as Chromium lays it out, in a
/// root of each of the two vertical writing modes.
fn assert_rows(rows: &[Row]) {
    for &(child, rl, lr) in rows {
        for (writing_mode, expected) in [("vertical-rl", rl), ("vertical-lr", lr)] {
            let body = format!(
                "<div id=r style=\"writing-mode:{writing_mode};width:100px;height:200px\">\
                 <div id=c style=\"{child}\"><div id=g>aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"
            );
            let actual = boxes(&body);
            for entry in expected.split(' ') {
                let (id, rect) = entry.split_once(':').expect("an id and a box");
                let want: Vec<f32> = rect
                    .split(',')
                    .map(|n| n.parse().expect("a number"))
                    .collect();
                let got = actual
                    .iter()
                    .find(|(name, _)| name == id)
                    .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
                    .1;
                assert!(
                    got.iter()
                        .zip(&want)
                        .all(|(got, want)| (got - want).abs() < 0.01),
                    "#{id} is {got:?}, expected {want:?} for {body}"
                );
            }
        }
    }
}

const FLEX_ROWS: &[Row] = &[
    (
        "writing-mode:horizontal-tb;display:flex;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flex;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,20,120",
        "c:0,0,62,182 g:31,31,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flex;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:flex;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,20,120",
        "c:0,0,62,120 g:31,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flex;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,0",
        "c:0,0,100,60 g:0,30,100,0",
    ),
    (
        "writing-mode:horizontal-tb;display:flex;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,20,120",
        "c:0,0,62,120 g:31,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flex;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,20,0",
        "c:0,0,62,40 g:31,20,20,0",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,20,120",
        "c:0,0,62,182 g:31,31,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,20,120",
        "c:0,0,62,120 g:31,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,0",
        "c:0,0,100,60 g:0,30,100,0",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,20,120",
        "c:0,0,62,120 g:31,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-flex;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,20,0",
        "c:0,0,62,40 g:31,20,20,0",
    ),
];

const OTHER_ROWS: &[Row] = &[
    (
        "writing-mode:horizontal-tb;display:block;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:block;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,0,120",
        "c:0,0,62,182 g:31,31,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:block;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:block;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:block;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,40",
        "c:0,0,100,60 g:0,30,100,40",
    ),
    (
        "writing-mode:horizontal-tb;display:block;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:block;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,0,120",
        "c:0,0,62,40 g:31,20,0,120",
    ),
    (
        "writing-mode:horizontal-tb;float:left;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,0,120",
        "c:0,0,62,182 g:31,31,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,40",
        "c:0,0,100,60 g:0,30,100,40",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:flow-root;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,0,120",
        "c:0,0,62,40 g:31,20,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,20,120",
        "c:0,0,62,182 g:31,31,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,20,120",
        "c:0,0,62,120 g:31,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,40",
        "c:0,0,100,60 g:0,30,100,40",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,20,120",
        "c:0,0,62,120 g:31,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:grid;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,20,120",
        "c:0,0,62,40 g:31,20,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,0,120",
        "c:0,0,62,182 g:31,31,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,40",
        "c:0,0,100,60 g:0,30,100,40",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:inline-block;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,0,120",
        "c:0,0,62,40 g:31,20,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;padding:0 60px",
        "c:-40,0,140,120 g:20,0,20,120",
        "c:0,0,140,120 g:60,0,20,120",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;border:31px solid;width:60px",
        "c:38,0,62,182 g:69,31,0,120",
        "c:0,0,62,182 g:31,31,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;padding:0 31px;width:60px;box-sizing:content-box",
        "c:-22,0,122,60 g:9,0,60,60",
        "c:0,0,122,60 g:31,0,60,60",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;padding:0 31px;max-width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;padding:30px 0;height:50px",
        "c:0,0,100,60 g:0,30,100,40",
        "c:0,0,100,60 g:0,30,100,40",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
    (
        "writing-mode:horizontal-tb;display:list-item;padding:20px 31px;width:60px;height:30px",
        "c:38,0,62,40 g:69,20,0,120",
        "c:0,0,62,40 g:31,20,0,120",
    ),
    (
        "writing-mode:horizontal-tb;position:relative;padding:0 31px;width:60px",
        "c:38,0,62,120 g:69,0,0,120",
        "c:0,0,62,120 g:31,0,0,120",
    ),
];

#[test]
fn a_flex_container_is_never_narrower_than_its_padding_and_border() {
    assert_rows(FLEX_ROWS);
}

#[test]
fn other_boxes_keep_their_padding_and_border_across_and_down() {
    assert_rows(OTHER_ROWS);
}
