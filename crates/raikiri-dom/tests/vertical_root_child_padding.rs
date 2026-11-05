//! A box that flows horizontally, in a vertical root, resolves the percentages
//! of its padding against the inline size of the root: its physical height
//! (CSS Writing Modes 4 §7.1). A block child does so whatever its width is
//! made of, whether it holds text, a block or a block in a block, and so does
//! an absolutely positioned one against the height of its containing block.
//! The root is 100px wide and 200px tall, so `10%` is 20px of padding. The
//! expected border boxes are the ones Chromium lays out for the same markup,
//! in an 800px by 600px viewport without margins and in Ahem at 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A block `#g` of twelve words, `aa bb ... ll`.
const G: &str = "<div id=g>aa bb cc dd ee ff gg hh ii jj kk ll</div>";

/// A row of a table: the style of the root beyond its writing mode and size,
/// the style of the child `#c`, what it holds, and the boxes of the ids that
/// Chromium lays out for it in a `vertical-rl` and in a `vertical-lr` root.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

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
/// root of each of the two vertical writing modes that flow `block`.
fn assert_rows(rows: &[Row]) {
    for &(root, child, inner, rl, lr) in rows {
        for (writing_mode, expected) in [("vertical-rl", rl), ("vertical-lr", lr)] {
            let body = format!(
                "<div id=r style=\"writing-mode:{writing_mode};width:100px;height:200px;{root}\">\
                 <div id=c style=\"{child}\">{inner}</div></div>"
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

const PADDING_ROWS: &[Row] = &[
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:auto",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:fit-content",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:min-content",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:max-content",
        G,
        "c:-290,0,390,50 g:-270,20,350,10",
        "c:0,0,390,50 g:20,20,350,10",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:0 10%;width:auto",
        G,
        "c:0,0,100,60 g:20,0,60,60",
        "c:0,0,100,60 g:20,0,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:0 10%;width:60px",
        G,
        "c:40,0,60,120 g:60,0,20,120",
        "c:0,0,60,120 g:20,0,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:0 10%;width:fit-content",
        G,
        "c:0,0,100,60 g:20,0,60,60",
        "c:0,0,100,60 g:20,0,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:0 10%;width:min-content",
        G,
        "c:40,0,60,120 g:60,0,20,120",
        "c:0,0,60,120 g:20,0,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:0 10%;width:max-content",
        G,
        "c:-290,0,390,10 g:-270,0,350,10",
        "c:0,0,390,10 g:20,0,350,10",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10% 0;width:auto",
        G,
        "c:0,0,100,80 g:0,20,100,40",
        "c:0,0,100,80 g:0,20,100,40",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10% 0;width:60px",
        G,
        "c:40,0,60,100 g:40,20,60,60",
        "c:0,0,60,100 g:0,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10% 0;width:fit-content",
        G,
        "c:0,0,100,80 g:0,20,100,40",
        "c:0,0,100,80 g:0,20,100,40",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10% 0;width:min-content",
        G,
        "c:80,0,20,160 g:80,20,20,120",
        "c:0,0,20,160 g:0,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10% 0;width:max-content",
        G,
        "c:-250,0,350,50 g:-250,20,350,10",
        "c:0,0,350,50 g:0,20,350,10",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:5% 10% 15% 20%;width:auto",
        G,
        "c:0,0,100,160 g:40,10,40,120",
        "c:0,0,100,160 g:40,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:5% 10% 15% 20%;width:60px",
        G,
        "c:40,0,60,160 g:80,10,0,120",
        "c:0,0,60,160 g:40,10,0,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:5% 10% 15% 20%;width:fit-content",
        G,
        "c:0,0,100,160 g:40,10,40,120",
        "c:0,0,100,160 g:40,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:5% 10% 15% 20%;width:min-content",
        G,
        "c:20,0,80,160 g:60,10,20,120",
        "c:0,0,80,160 g:40,10,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:5% 10% 15% 20%;width:max-content",
        G,
        "c:-310,0,410,50 g:-270,10,350,10",
        "c:0,0,410,50 g:40,10,350,10",
    ),
];
const NESTED_ROWS: &[Row] = &[
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:auto",
        "<div><div id=g>aa bb cc dd ee ff gg hh ii jj kk ll</div></div>",
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:60px",
        "<div><div id=g>aa bb cc dd ee ff gg hh ii jj kk ll</div></div>",
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;margin:10%;width:auto",
        G,
        "c:20,20,60,60 g:20,20,60,60",
        "c:20,20,60,60 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;margin:0 10%;width:60px",
        G,
        "c:20,0,60,60 g:20,0,60,60",
        "c:20,0,60,60 g:20,0,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;box-sizing:border-box;width:80px",
        G,
        "c:20,0,80,160 g:40,20,40,120",
        "c:0,0,80,160 g:20,20,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:auto",
        "<div style=\"padding:10%\"><div id=g>aa bb cc dd ee ff gg hh ii jj kk ll</div></div>",
        "c:0,0,100,172 g:26,26,48,120",
        "c:0,0,100,172 g:26,26,48,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;width:auto",
        "<div id=g>aa bb cc dd ee ff gg hh ii jj kk ll</div><div id=h>aa bb</div>",
        "c:0,0,100,110 g:20,20,60,60 h:20,80,60,10",
        "c:0,0,100,110 g:20,20,60,60 h:20,80,60,10",
    ),
];
const DISPLAY_ROWS: &[Row] = &[
    (
        "",
        "writing-mode:horizontal-tb;display:block;padding:10%",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:block;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:block;padding:5% 15%",
        G,
        "c:0,0,100,140 g:30,10,40,120",
        "c:0,0,100,140 g:30,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:flow-root;padding:10%",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:flow-root;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:flow-root;padding:5% 15%",
        G,
        "c:0,0,100,140 g:30,10,40,120",
        "c:0,0,100,140 g:30,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:grid;padding:10%",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:grid;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:grid;padding:5% 15%",
        G,
        "c:0,0,100,140 g:30,10,40,120",
        "c:0,0,100,140 g:30,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:inline-block;padding:10%",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:inline-block;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:inline-block;padding:5% 15%",
        G,
        "c:0,0,100,140 g:30,10,40,120",
        "c:0,0,100,140 g:30,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:list-item;padding:10%",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:list-item;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;display:list-item;padding:5% 15%",
        G,
        "c:0,0,100,140 g:30,10,40,120",
        "c:0,0,100,140 g:30,10,40,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;float:left;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;position:relative;padding:10%;width:60px",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "",
        "writing-mode:horizontal-tb;padding:10%;margin:5%;width:60px",
        G,
        "c:30,10,60,160 g:50,30,20,120",
        "c:10,10,60,160 g:30,30,20,120",
    ),
];
const ABSOLUTE_ROWS: &[Row] = &[
    (
        "position:relative",
        "writing-mode:vertical-rl;position:absolute;padding:10%;width:60px;",
        G,
        "c:40,0,60,200 g:50,20,30,160",
        "c:0,0,60,200 g:10,20,30,160",
    ),
    (
        "position:relative",
        "writing-mode:vertical-rl;position:absolute;padding:10%;height:100px;",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "position:relative",
        "writing-mode:vertical-rl;position:absolute;padding:10%;",
        G,
        "c:30,0,70,200 g:50,20,30,160",
        "c:0,0,70,200 g:20,20,30,160",
    ),
    (
        "position:relative",
        "writing-mode:vertical-lr;position:absolute;padding:10%;width:60px;",
        G,
        "c:40,0,60,200 g:60,20,30,160",
        "c:0,0,60,200 g:20,20,30,160",
    ),
    (
        "position:relative",
        "writing-mode:vertical-lr;position:absolute;padding:10%;height:100px;",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "position:relative",
        "writing-mode:vertical-lr;position:absolute;padding:10%;",
        G,
        "c:30,0,70,200 g:50,20,30,160",
        "c:0,0,70,200 g:20,20,30,160",
    ),
    (
        "position:relative",
        "writing-mode:horizontal-tb;position:absolute;padding:10%;width:60px;",
        G,
        "c:40,0,60,160 g:60,20,20,120",
        "c:0,0,60,160 g:20,20,20,120",
    ),
    (
        "position:relative",
        "writing-mode:horizontal-tb;position:absolute;padding:10%;height:100px;",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "position:relative",
        "writing-mode:horizontal-tb;position:absolute;padding:10%;",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "position:relative",
        "writing-mode:sideways-rl;position:absolute;padding:10%;width:60px;",
        G,
        "c:40,0,60,200 g:50,20,30,160",
        "c:0,0,60,200 g:10,20,30,160",
    ),
    (
        "position:relative",
        "writing-mode:sideways-rl;position:absolute;padding:10%;height:100px;",
        G,
        "c:0,0,100,100 g:20,20,60,60",
        "c:0,0,100,100 g:20,20,60,60",
    ),
    (
        "position:relative",
        "writing-mode:sideways-rl;position:absolute;padding:10%;",
        G,
        "c:30,0,70,200 g:50,20,30,160",
        "c:0,0,70,200 g:20,20,30,160",
    ),
];

#[test]
fn a_block_child_resolves_its_padding_against_the_height_of_the_root() {
    assert_rows(PADDING_ROWS);
}

#[test]
fn a_block_inside_the_child_and_other_percentages_follow() {
    assert_rows(NESTED_ROWS);
}

#[test]
fn every_display_that_flows_horizontally_pads_the_same() {
    assert_rows(DISPLAY_ROWS);
}

#[test]
fn an_absolutely_positioned_box_pads_against_the_height_of_its_containing_block() {
    assert_rows(ABSOLUTE_ROWS);
}

#[test]
fn orthogonal_tables_keep_their_measured_outer_height() {
    // Chromium 152, Ahem 10px, an 800x600 viewport, after fonts are ready.
    // A table fragment describes its grid. Caption cases subtract the
    // measured caption from Chromium's wrapper rectangle and check both.
    // Collapsed-cell inline geometry is separate from this height contract;
    // those rows compare the table box, leaving the cell width out.
    let cases = [
        (
            "vertical-rl-table-padding",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-table-border",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;border:20px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-table-padding-y",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-rl-table-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-table-percent-y",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10% 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-rl-table-padding-border",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10px;border:10px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-table-width60",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;width:60px"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:40,0,60,160 g:60,20,20,120",
        ),
        (
            "vertical-rl-inline-table-padding",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-inline-table-border",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;border:20px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-inline-table-padding-y",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-rl-inline-table-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-inline-table-percent-y",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10% 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-rl-inline-table-padding-border",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10px;border:10px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-inline-table-width60",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;width:60px"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:40,0,60,160 g:60,20,20,120",
        ),
        (
            "vertical-rl-native-padding",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-rl-native-border",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;border:20px solid;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-rl-native-padding-y",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px 0;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,86 g:2,22,96,42",
        ),
        (
            "vertical-rl-native-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-rl-native-percent-y",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10% 0;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,86 g:2,22,96,42",
        ),
        (
            "vertical-rl-native-padding-border",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10px;border:10px solid;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-rl-native-width60",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;width:60px"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:34,0,66,166 g:56,22,22,122",
        ),
        (
            "vertical-lr-table-padding",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-table-border",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;border:20px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-table-padding-y",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-lr-table-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-table-percent-y",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10% 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-lr-table-padding-border",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10px;border:10px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-table-width60",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;width:60px"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,60,160 g:20,20,20,120",
        ),
        (
            "vertical-lr-inline-table-padding",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-inline-table-border",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;border:20px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-inline-table-padding-y",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-lr-inline-table-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-inline-table-percent-y",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10% 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "vertical-lr-inline-table-padding-border",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10px;border:10px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-inline-table-width60",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;width:60px"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,60,160 g:20,20,20,120",
        ),
        (
            "vertical-lr-native-padding",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-native-border",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;border:20px solid;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-native-padding-y",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px 0;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,86 g:2,22,96,42",
        ),
        (
            "vertical-lr-native-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-native-percent-y",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10% 0;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,86 g:2,22,96,42",
        ),
        (
            "vertical-lr-native-padding-border",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10px;border:10px solid;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-native-width60",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;width:60px"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,66,166 g:22,22,22,122",
        ),
        (
            "horizontal-tb-table-padding",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "horizontal-tb-table-border",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;border:20px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "horizontal-tb-table-padding-y",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "horizontal-tb-table-percent",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,60 g:10,10,80,40",
        ),
        (
            "horizontal-tb-table-percent-y",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10% 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,60 g:0,10,100,40",
        ),
        (
            "horizontal-tb-table-padding-border",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10px;border:10px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "horizontal-tb-table-width60",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;width:60px"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,60,160 g:20,20,20,120",
        ),
        (
            "horizontal-tb-inline-table-padding",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "horizontal-tb-inline-table-border",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;border:20px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "horizontal-tb-inline-table-padding-y",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,80 g:0,20,100,40",
        ),
        (
            "horizontal-tb-inline-table-percent",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,60 g:10,10,80,40",
        ),
        (
            "horizontal-tb-inline-table-percent-y",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10% 0;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,60 g:0,10,100,40",
        ),
        (
            "horizontal-tb-inline-table-padding-border",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:10px;border:10px solid;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "horizontal-tb-inline-table-width60",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;width:60px"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,60,160 g:20,20,20,120",
        ),
        (
            "horizontal-tb-native-padding",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "horizontal-tb-native-border",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;border:20px solid;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "horizontal-tb-native-padding-y",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px 0;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,86 g:2,22,96,42",
        ),
        (
            "horizontal-tb-native-percent",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,86 g:12,12,76,62",
        ),
        (
            "horizontal-tb-native-percent-y",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10% 0;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,66 g:2,12,96,42",
        ),
        (
            "horizontal-tb-native-padding-border",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:10px;border:10px solid;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "horizontal-tb-native-width60",
            r#"<div id="r" style="writing-mode:horizontal-tb;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;width:60px"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,66,166 g:22,22,22,122",
        ),
        (
            "vertical-rl-flex-table",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;display:flex"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-flex-inline-table",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;display:flex"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-flex-native",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;display:flex"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-rl-grid-table",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;display:grid"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,200 g:20,20,60,60",
        ),
        (
            "vertical-rl-grid-inline-table",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;display:grid"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,200 g:20,20,60,60",
        ),
        (
            "vertical-rl-grid-native",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;display:grid"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,200 g:22,22,56,156",
        ),
        (
            "vertical-rl-caption-top",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><caption id="cap" style="caption-side:top">caption</caption><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,10,100,106 g:22,32,56,62 cap:0,0,100,10",
        ),
        (
            "vertical-rl-caption-bottom",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><caption id="cap" style="caption-side:bottom">caption</caption><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62 cap:0,106,100,10",
        ),
        (
            "vertical-rl-table-empty",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"></div></div>"#,
            "c:60,0,40,40",
        ),
        (
            "vertical-rl-table-height-border-box",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:border-box"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,160 g:20,20,60,60",
        ),
        (
            "vertical-rl-table-height-content-box",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:content-box"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,200 g:20,20,60,60",
        ),
        (
            "vertical-rl-native-empty",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"></table></div>"#,
            "c:60,0,40,40",
        ),
        (
            "vertical-rl-native-height-border-box",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:border-box"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,160 g:22,22,56,116",
        ),
        (
            "vertical-rl-native-height-content-box",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:content-box"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,200 g:22,22,56,156",
        ),
        (
            "vertical-rl-collapsed",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;border:20px solid;border-collapse:collapse"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,102",
        ),
        (
            "vertical-lr-flex-table",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;display:flex"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-flex-inline-table",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;display:flex"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-flex-native",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;display:flex"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-grid-table",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;display:grid"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,200 g:20,20,60,60",
        ),
        (
            "vertical-lr-grid-inline-table",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;display:grid"><div id="c" style="display:inline-table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,200 g:20,20,60,60",
        ),
        (
            "vertical-lr-grid-native",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;display:grid"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,200 g:22,22,56,156",
        ),
        (
            "vertical-lr-caption-top",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><caption id="cap" style="caption-side:top">caption</caption><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,10,100,106 g:22,32,56,62 cap:0,0,100,10",
        ),
        (
            "vertical-lr-caption-bottom",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><caption id="cap" style="caption-side:bottom">caption</caption><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62 cap:0,106,100,10",
        ),
        (
            "vertical-lr-table-empty",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"></div></div>"#,
            "c:0,0,40,40",
        ),
        (
            "vertical-lr-table-height-border-box",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:border-box"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,160 g:20,20,60,60",
        ),
        (
            "vertical-lr-table-height-content-box",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:content-box"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,200 g:20,20,60,60",
        ),
        (
            "vertical-lr-native-empty",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;"></table></div>"#,
            "c:0,0,40,40",
        ),
        (
            "vertical-lr-native-height-border-box",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:border-box"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,160 g:22,22,56,116",
        ),
        (
            "vertical-lr-native-height-content-box",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;padding:20px;height:160px;box-sizing:content-box"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,200 g:22,22,56,156",
        ),
        (
            "vertical-lr-collapsed",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><table id="c" style="writing-mode:horizontal-tb;border:20px solid;border-collapse:collapse"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,102",
        ),
        (
            "vertical-rl-float-left-table-px",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,70,100,100 g:20,90,60,60",
        ),
        (
            "vertical-rl-float-left-table-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,70,100,100 g:20,90,60,60",
        ),
        (
            "vertical-rl-float-left-native-px",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,70,100,106 g:22,92,56,62",
        ),
        (
            "vertical-rl-float-left-native-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,70,100,106 g:22,92,56,62",
        ),
        (
            "vertical-rl-float-right-table-px",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-float-right-table-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-rl-float-right-native-px",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-rl-float-right-native-percent",
            r#"<div id="r" style="writing-mode:vertical-rl;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-float-left-table-px",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,70,100,100 g:20,90,60,60",
        ),
        (
            "vertical-lr-float-left-table-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,70,100,100 g:20,90,60,60",
        ),
        (
            "vertical-lr-float-left-native-px",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,70,100,106 g:22,92,56,62",
        ),
        (
            "vertical-lr-float-left-native-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:left;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,70,100,106 g:22,92,56,62",
        ),
        (
            "vertical-lr-float-right-table-px",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:20px;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-float-right-table-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><div id="c" style="display:table;writing-mode:horizontal-tb;padding:10%;"><div id="g">aa bb cc dd ee ff gg hh ii jj kk ll</div></div></div>"#,
            "c:0,0,100,100 g:20,20,60,60",
        ),
        (
            "vertical-lr-float-right-native-px",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:20px;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
        (
            "vertical-lr-float-right-native-percent",
            r#"<div id="r" style="writing-mode:vertical-lr;width:100px;height:200px;"><div style="float:right;width:30px;height:70px"></div><table id="c" style="writing-mode:horizontal-tb;padding:10%;"><tr><td id="g">aa bb cc dd ee ff gg hh ii jj kk ll</td></tr></table></div>"#,
            "c:0,0,100,106 g:22,22,56,62",
        ),
    ];
    let mut failures = Vec::new();
    for (name, body, expected) in cases {
        let actual = boxes(body);
        for entry in expected.split(' ') {
            let (id, rect) = entry.split_once(':').unwrap();
            let wanted: Vec<f32> = rect.split(',').map(|n| n.parse().unwrap()).collect();
            let found = actual
                .iter()
                .find(|(name, _)| name == id)
                .map(|(_, rect)| rect);
            if found.is_none_or(|rect| rect.iter().zip(&wanted).any(|(a, b)| (a - b).abs() > 0.01))
            {
                failures.push(format!("{name} #{id}: {found:?} != {wanted:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
