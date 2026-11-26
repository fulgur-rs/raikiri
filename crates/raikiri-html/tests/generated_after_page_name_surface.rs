//! In-flow `::after` text takes the page type of the box it ends, not that
//! of the box's last child: when that child ends with another page type,
//! the page breaks before the generated text, as it would before text
//! after the child, and the box after the box compares its own type with
//! the generated text's.
//!
//! The widths and positions are the ones Chromium prints for the same
//! documents (`page.pdf` with the page sizes taken from `@page`): `foo`
//! pages are 200px wide, `bar` pages 300px and other pages 100px, all 100px
//! high. Each page lists the 20px Ahem glyphs on it as `glyph@x,top`, with
//! 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page as its width, then each 20px Ahem glyph on it as
/// `glyph@x,top`, sorted.
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
         @page foo{{size:200px 100px}} @page bar{{size:300px 100px}}\
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
            let width = page.geometry().page_box.width.round() as i32;
            format!("{width}: {}", glyphs.join(" "))
        })
        .collect()
}

fn expect(body: &str, expected: &[&str]) {
    assert_eq!(pages(body), expected, "{body}");
}

const STYLE: &str = "<style>.a::after{content:'G'} .c::after{content:'G';display:block}</style>";

fn expect_styled(body: &str, expected: &[&str]) {
    expect(&format!("{STYLE}{body}"), expected);
}

#[test]
fn generated_text_after_a_named_last_child_starts_a_page_of_its_box() {
    expect_styled(
        "<div class=a><div style='page:foo'>H</div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
    expect_styled(
        "<div class=a><div style='page:foo'>H</div></div>",
        &["200: H@0,0", "100: G@0,0"],
    );
    // A block `::after` box breaks the same way.
    expect_styled(
        "<div class=c><div style='page:foo'>H</div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
    // So does one after a deeper named box, or after a named box that its
    // own lines take over several lines of its page.
    expect_styled(
        "<div class=a><div><div style='page:foo'>H</div></div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
    expect_styled(
        "<div class=a><div style='page:foo'>H<br>H<br>H</div></div><div>K</div>",
        &["200: H@0,0 H@0,30 H@0,60", "100: G@0,0 K@0,30"],
    );
    // The box's own padding follows the text.
    expect_styled(
        "<div class=a style='padding-bottom:10px'><div style='page:foo'>H</div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,40"],
    );
    // A named page before the box changes nothing.
    expect_styled(
        "<div style='page:foo'>X</div><div class=a><div style='page:foo'>H</div></div><div>K</div>",
        &["200: H@0,30 X@0,0", "100: G@0,0 K@0,30"],
    );
}

#[test]
fn the_page_of_the_generated_text_has_the_type_of_its_box() {
    // The box's own page value names that page, and the box after it
    // compares its type with it.
    expect_styled(
        "<div class=a style='page:bar'><div style='page:foo'>H</div></div><div>K</div>",
        &["200: H@0,0", "300: G@0,0", "100: K@0,0"],
    );
    expect_styled(
        "<div style='page:foo'><div class=a><div style='page:bar'>H</div></div></div><div>K</div>",
        &["300: H@0,0", "200: G@0,0", "100: K@0,0"],
    );
    expect_styled(
        "<div class=a><div style='page:foo'>H</div></div><div style='page:foo'>K</div>",
        &["200: H@0,0", "100: G@0,0", "200: K@0,0"],
    );
    // A forced break after the named box is the same break.
    expect_styled(
        "<div class=a><div style='page:foo;break-after:page'>H</div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
    // Text after the box joins the generated text.
    expect_styled(
        "<div class=a><div style='page:foo'>H</div></div>T",
        &["200: H@0,0", "100: G@0,0 T@0,30"],
    );
}

#[test]
fn floats_positioned_boxes_and_avoided_breaks_do_not_hold_the_text_back() {
    expect_styled(
        "<div class=a><div style='page:foo'>H</div><div style='float:left'>F</div></div><div>K</div>",
        &["200: H@0,0", "100: F@0,0 G@20,0 K@0,30"],
    );
    expect_styled(
        "<div class=a><div style='page:foo'>H</div><div style='position:absolute'>P</div></div>\
         <div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30 P@0,0"],
    );
    expect_styled(
        "<div class=a style='break-inside:avoid'><div style='page:foo'>H</div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
    expect_styled(
        "<div class=a><div style='display:flex;page:foo'>H</div></div><div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
}

#[test]
fn flex_items_and_table_cells_break_before_their_generated_text_too() {
    // The row flex container that takes the item's named page breaks no
    // page after itself once the text has moved on.
    expect_styled(
        "<div style='display:flex'><div class=a><div style='page:foo'>H</div></div></div>\
         <div>K</div>",
        &["200: H@0,0", "100: G@0,0 K@0,30"],
    );
    expect_styled(
        "<div style='display:flex'><div class=a><div style='page:foo'>H</div></div></div>",
        &["200: H@0,0", "100: G@0,0"],
    );
    expect_styled(
        "<table><tr><td class=a><div style='page:foo'>H</div></td></tr></table><div>K</div>",
        &["200: H@3,3", "100: G@3,0 K@0,33"],
    );
}

#[test]
fn a_named_box_inside_an_inline_box_keeps_the_text_on_its_page() {
    // The generated text shares the line that closes the inline box.
    expect_styled(
        "<div class=a><span><div style='page:foo'>H</div></span></div><div>K</div>",
        &["200: G@0,30 H@0,0", "100: K@0,0"],
    );
}
