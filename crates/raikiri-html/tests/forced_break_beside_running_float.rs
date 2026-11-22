//! Content after a forced page break starts the page the break opens. An
//! earlier float that runs on onto that page keeps its place there, and the
//! content after the break lines up beside it rather than below its end,
//! while the floats that end on the page the break leaves are left behind.
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
fn a_float_after_the_break_starts_the_page_beside_an_earlier_float() {
    // The float after the break and the text after it start the next page
    // beside the part of the earlier float that runs on onto it.
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px'>F</div>Z",
        &["E@60,0 T@0,0", "F@0,0 Z@40,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:left;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px'>F</div>Z",
        &["E@0,0 T@40,0", "F@40,0 Z@80,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:right;width:40px'>F</div>Z",
        &["E@60,0 T@0,0", "F@20,0 Z@0,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:150px'>E</div>T</div><div style='float:left;width:40px'>F</div>Z",
        &["E@60,0 T@0,0", "F@0,0 Z@40,0"],
    );
    expect(
        "<div style='float:right;width:40px;height:250px'>E</div><div style='break-after:page'>T</div><div style='float:left;width:40px'>F</div>Z",
        &["E@60,0 T@0,0", "F@0,0 Z@40,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px;margin-top:10px'>F</div>Z",
        &["E@60,0 T@0,0", "F@0,10 Z@40,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px;height:150px'>F</div>Z",
        &["E@60,0 T@0,0", "F@0,0 Z@40,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px'>F</div>Z Y X W",
        &["E@60,0 T@0,0", "F@0,0 W@0,60 X@40,30 Y@0,30 Z@40,0", ""],
    );
}

#[test]
fn a_float_that_does_not_fit_beside_the_earlier_float_goes_below_it() {
    // A float too wide to stand beside the earlier float, or one that clears
    // it, goes below that float, and the text after it starts the page.
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:70px'>F</div>Z",
        &["E@60,0 T@0,0", "Z@0,0", "F@0,50"],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px;clear:right'>F</div>Z",
        &["E@60,0 T@0,0", "Z@0,0", "F@0,50"],
    );
}

#[test]
fn a_block_after_the_break_starts_the_page_beside_an_earlier_float() {
    // A block after the break starts the next page, and its lines wrap beside
    // the earlier float there.
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div>Z</div>",
        &["E@60,0 T@0,0", "Z@0,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='margin-top:20px'>Z</div>",
        &["E@60,0 T@0,0", "Z@0,20", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div>Z Y X W V</div>",
        &["E@60,0 T@0,0", "V@0,60 W@40,30 X@0,30 Y@40,0 Z@0,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:200px'>E</div>T</div><div>Z</div>",
        &["E@60,0 T@0,0", "Z@0,0"],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:90px'>E</div>T</div><div>Z</div>",
        &["E@60,0 T@0,0", "Z@0,0"],
    );
    expect(
        "<div><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='break-before:page'>Z</div>",
        &["E@60,0 T@0,0", "Z@0,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div><div>Z</div></div>",
        &["E@60,0 T@0,0", "Z@0,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:left;width:40px;height:250px'>E</div><div style='float:right;width:40px;height:150px'>D</div>T</div><div>Z Y</div>",
        &["D@60,0 E@0,0 T@40,0", "Y@40,30 Z@40,0", ""],
    );
    expect(
        "<div dir='rtl' style='break-after:page'><div style='float:left;width:40px;height:250px'>E</div>T</div><div dir='rtl'>Z</div>",
        &["E@20,0 T@80,0", "Z@80,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:150px'>E</div>T</div><div style='break-after:page'>Z</div><div>Y</div>",
        &["E@60,0 T@0,0", "Z@0,0", "Y@0,0"],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='position:relative;top:10px'>Z</div>",
        &["E@60,0 T@0,0", "Z@0,10", ""],
    );
}

#[test]
fn clearance_and_new_formatting_contexts_still_avoid_the_earlier_float() {
    // A block that clears the earlier float, or one that starts a formatting
    // context of its own, is still placed against that float.
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='clear:right'>Z</div>",
        &["E@60,0 T@0,0", "", "", "Z@0,0"],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='overflow:hidden'>Z</div>",
        &["E@60,0 T@0,0", "Z@0,0", ""],
    );
    expect(
        "<div style='break-after:page'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px'>F</div><div style='clear:both'>C</div>",
        &["E@60,0 T@0,0", "", "", "C@0,30 F@0,0"],
    );
}

#[test]
fn a_change_of_named_page_starts_the_page_beside_an_earlier_float() {
    // A change of named page breaks the same way.
    expect(
        "<div style='page:foo'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='page:bar'>Z</div>",
        &["E@60,0 T@0,0", "Z@0,0", ""],
    );
    expect(
        "<div style='page:foo'><div style='float:right;width:40px;height:250px'>E</div>T</div><div style='float:left;width:40px'>F</div><div style='page:bar'>Z</div>",
        &["E@60,0 F@0,30 T@0,0", "Z@0,0", ""],
    );
}
