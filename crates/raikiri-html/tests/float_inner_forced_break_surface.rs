//! A break forced inside a float, before or after a box in its flow, splits
//! the float: what follows the break inside the float starts the next page,
//! while the flow beside and after the float stays where it is, as an
//! absolutely positioned box keeps the breaks forced inside it to itself.
//! A box that clears the float starts below its part on the next page.
//!
//! The positions are the ones Chromium prints for the same documents
//! (`page.pdf` with the page size taken from `@page`): each page lists the
//! 20px Ahem glyphs on it as `glyph@x,top`, with 30px lines. The floats are
//! 40px wide so that the glyphs beside them stand apart.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page as the 20px Ahem glyphs on it, `glyph@x,top`, sorted.
fn pages(body: &str) -> Vec<String> {
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
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
        .pages()
        .map(|page| {
            let mut glyphs = Vec::new();
            for run in page.text_runs() {
                // The 20px Ahem baseline sits 21px below the top of its 30px line.
                let top = (run.origin.1 - 21.0).round() as i32;
                let mut x = run.origin.0.round() as i32;
                for glyph in run.text.chars() {
                    if glyph != ' ' {
                        glyphs.push(format!("{glyph}@{x},{top}"));
                    }
                    x += 20;
                }
            }
            glyphs.sort();
            glyphs.join(" ")
        })
        .collect()
}

fn expect(body: &str, expected: &[&str]) {
    assert_eq!(pages(body), expected, "{body}");
}

#[test]
fn a_break_before_a_box_inside_a_float_splits_the_float() {
    // The box after the break starts the next page, and the flow beside the
    // float stays on the page the break leaves.
    expect(
        "<div>T</div><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,30 T@0,0 Z@40,30", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:right;width:40px'>F<div style='break-before:page'>H</div></div>Z",
        &["F@60,0 Z@0,0", "H@60,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='page-break-before:always'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:left'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'><div>F<div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='display:contents'><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='overflow:hidden'><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='margin-top:20px;break-before:page'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,20"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page;margin-top:10px'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,10"],
    );
    expect(
        "<div style='float:left;width:40px;border-top:10px solid'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,10 Z@40,0", "H@0,0"],
    );
}

#[test]
fn a_break_after_a_box_inside_a_float_splits_the_float() {
    // A break after the last box of the float falls nowhere.
    expect(
        "<div style='float:left;width:40px;background:red'>F<div style='break-after:page'>H</div>G</div>Z",
        &["F@0,0 H@0,30 Z@40,0", "G@0,0"],
    );
    expect(
        "<div>T</div><div style='float:left;width:40px'><div style='break-after:page'>F</div>H</div>Z",
        &["F@0,30 T@0,0 Z@40,30", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-after:page'>H</div>G</div>Z",
        &["F@0,0 H@0,30 Z@40,0", "G@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-after:page'>H</div><div>G</div></div>Z",
        &["F@0,0 H@0,30 Z@40,0", "G@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page;break-after:page'>H</div>G</div>Z",
        &["F@0,0 Z@40,0", "H@0,0", "G@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-after:page'>H</div></div>Z",
        &["F@0,0 H@0,30 Z@40,0"],
    );
}

#[test]
fn the_rest_of_the_float_goes_on_after_the_break() {
    // Later boxes and lines of the float follow the box after the break, and
    // a float of set height or with padding and a border splits the same way.
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div><div style='break-before:page'>G</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0", "G@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div>G G G G</div>Z",
        &["F@0,0 Z@40,0", "G@0,30 G@0,60 H@0,0", "G@0,0 G@0,30"],
    );
    expect(
        "<div style='float:left;width:40px'><div>F</div><div style='break-before:page'>H</div><div>G</div></div><div>Z</div>",
        &["F@0,0 Z@40,0", "G@0,30 H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div><div>G</div><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 G@0,30 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<ul style='margin:0;padding:0;list-style:none'><li>G</li><li style='break-before:page'>H</li></ul></div>Z",
        &["F@0,0 G@0,30 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px;padding:5px 0;border-bottom:5px solid'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,5 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px;padding-bottom:10px;background:red'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px;height:50px'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
}

#[test]
fn the_flow_after_the_float_stays_and_boxes_that_clear_it_follow_it() {
    // In-flow content after the float stays beside it; a box that clears it
    // starts below its part on the next page.
    expect(
        "<div style='float:left;width:40px'>F<div style='break-after:page'>H</div>G</div><div style='clear:both'>C</div>",
        &["F@0,0 H@0,30", "C@0,30 G@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-after:page'>H</div>G G</div>Z<div style='clear:both'>C</div>",
        &["F@0,0 H@0,30 Z@40,0", "C@0,60 G@0,0 G@0,30"],
    );
    expect(
        "<div>T</div><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div><div>Z</div><div>Y</div>",
        &["F@0,30 T@0,0 Y@40,60 Z@40,30", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div><div style='clear:both'>C</div>",
        &["F@0,0", "C@0,30 H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div><div style='float:left;width:40px'>G</div>Z",
        &["F@0,0 G@40,0 Z@80,0", "H@0,0"],
    );
    expect(
        "<div><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z</div><div>Y</div>",
        &["F@0,0 Y@40,30 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div><div style='float:left;width:40px'>G<div style='break-before:page'>K</div></div>Z",
        &["F@0,0 G@40,0 Z@80,0", "H@0,0 K@40,0"],
    );
    expect(
        "<div style='display:flow-root'><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z</div><div>Y</div>",
        &["F@0,0 Z@40,0", "H@0,0 Y@0,30"],
    );
    expect(
        "<div>T</div><div>T</div><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,60 T@0,0 T@0,30 Z@40,60", "H@0,0"],
    );
    expect(
        "<p style='margin:0'>A<span><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div></span>Z</p>",
        &["A@0,0 F@0,30 Z@40,30", "H@0,0"],
    );
}

#[test]
fn a_break_inside_a_flex_grid_or_nested_box_of_a_float_splits_it() {
    // A flex or grid container, a nested float, a table cell around the float
    // and an absolutely positioned box inside or around it. An absolutely
    // positioned box splits too inside a monolithic box, which does not hold
    // it, and its parent and the flow after it stay.
    expect(
        "<div><div style='position:absolute;width:40px'><div style='float:left'>A<div style='break-before:page'>H</div></div></div><div>Z</div></div><div>Y</div>",
        &["A@0,0 Y@0,30 Z@0,0", "H@0,0"],
    );
    expect(
        "<div style='overflow:hidden;height:80px'><div style='position:absolute;width:40px'>F<div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0", "H@0,0 Z@0,0"],
    );
    expect(
        "<span style='display:inline-block;width:60px'><div style='position:absolute;width:40px'>F<div style='break-before:page'>H</div></div></span>Z",
        &["F@0,21 Z@60,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px;display:flex;flex-direction:column'><div>F</div><div style='break-before:page'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='display:flex'><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='display:flex;flex-direction:column'><div>G</div><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 G@0,30 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='display:grid'><div>G</div><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 G@0,30 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='display:flex'><div>G</div><div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 Z@40,0", "G@0,0 H@20,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='display:flex;flex-direction:column'><div style='break-after:page'>G</div><div>H</div></div></div>Z",
        &["F@0,0 G@0,30 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='float:left;width:20px'>E<div style='break-before:page'>H</div></div></div>Z",
        &["E@0,0 F@20,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='float:left;width:40px;break-inside:avoid'>F<div style='break-before:page'>H</div></div>Z",
        &["F@0,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<table><tr><td><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z</td></tr></table>",
        &["F@3,3 Z@43,3", "H@3,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='position:absolute;width:40px'>A<div style='break-before:page'>H</div></div>G</div>Z",
        &["A@0,30 F@0,0 G@20,0 Z@40,0", "H@0,0"],
    );
    expect(
        "<div style='position:absolute;width:40px'>F<div style='float:left;width:40px'>A<div style='break-before:page'>H</div></div>G</div>Z",
        &["A@0,30 F@0,0 G@20,0 Z@0,0", "H@0,0"],
    );
}

#[test]
fn breaks_that_do_not_split_the_float() {
    // A column break outside columns, an avoid value, an inline-level box and
    // a monolithic box inside the float or around it.
    expect(
        "<span style='display:inline-block;width:60px'><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div></span>",
        &["F@0,0 H@0,30"],
    );
    expect(
        "<div style='overflow:hidden;height:80px'><div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>Z</div>",
        &["F@0,0 H@0,30 Z@40,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='overflow:auto;height:50px'>G<div style='break-before:page'>H</div></div></div>Z",
        &["F@0,0 G@0,30 H@0,60 Z@40,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:column'>H</div></div>Z",
        &["F@0,0 H@0,30 Z@40,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<div style='break-before:avoid-page'>H</div></div>Z",
        &["F@0,0 H@0,30 Z@40,0"],
    );
    expect(
        "<div style='float:left;width:40px'>F<span style='display:inline-block;break-before:page'>H</span></div>Z",
        &["F@0,0 H@20,0 Z@40,0"],
    );
}
