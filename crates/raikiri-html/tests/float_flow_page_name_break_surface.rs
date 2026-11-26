//! A float is no part of the page flow, but a change of page type between
//! two boxes in its own flow, or between a box and the text after it,
//! breaks the page there as a forced break would: the float splits, and what
//! follows the change inside it starts the next page, while the flow beside
//! and after the float stays where it is. The pages the float goes on to
//! keep the page type of the flow around it, and the first box of a box in
//! the float's flow has nothing before it there to differ from.
//!
//! The widths and positions are the ones Chromium prints for the same
//! documents (`page.pdf` with the page sizes taken from `@page`): `foo`
//! pages are 200px wide, `bar` pages 300px and other pages 100px, all 100px
//! high. Each page lists the 20px Ahem glyphs on it as `glyph@x,top`, with
//! 30px lines. The floats are 40px wide so that the glyphs beside them stand
//! apart.

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

const FLOAT: &str = "<div style='float:left;width:40px";

#[test]
fn a_named_box_after_text_in_a_float_splits_the_float() {
    // The named box starts the next page, which keeps the type of the flow
    // around the float, and the text beside the float stays on the first.
    expect(
        &format!("{FLOAT}'>F<div style='page:foo'>H</div></div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0"],
    );
    // The float's own page type is what its text compares with.
    expect(
        &format!("{FLOAT};page:foo'>F<div style='page:bar'>H</div></div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0"],
    );
    // The same page type on both sides breaks nothing.
    expect(
        &format!("{FLOAT};page:foo'>F<div style='page:foo'>H</div>K</div>Z"),
        &["100: F@0,0 H@0,30 K@0,60 Z@40,0"],
    );
    // The flow after the float goes on as without the break.
    expect(
        &format!("{FLOAT}'>F<div style='page:foo'>H</div></div>Z<div style='height:200px'></div>Y"),
        &["100: F@0,0 Z@40,0", "100: H@0,0", "100: Y@0,30"],
    );
}

#[test]
fn text_or_a_box_after_a_named_box_in_a_float_starts_another_page() {
    expect(
        &format!("{FLOAT}'>F<div style='page:foo'>H</div>K</div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0", "100: K@0,0"],
    );
    // The first box of the float has nothing before it to differ from.
    expect(
        &format!("{FLOAT}'><div style='page:foo'>H</div>K</div>Z"),
        &["100: H@0,0 Z@40,0", "100: K@0,0"],
    );
    // An avoided break does not hold the boxes on either side together.
    expect(
        &format!(
            "{FLOAT}'><div style='break-after:avoid'>A</div><div style='page:foo'>H</div></div>Z"
        ),
        &["100: A@0,0 Z@40,0", "100: H@0,0"],
    );
    expect(
        &format!("{FLOAT}'><div style='page:foo'>H</div><div style='page:bar'>K</div></div>Z"),
        &["100: H@0,0 Z@40,0", "100: K@0,0"],
    );
}

#[test]
fn a_named_box_deeper_in_the_float_flow_splits_the_float() {
    // The change at the nested box, and the one after the box around it.
    expect(
        &format!("{FLOAT}'><div>F<div style='page:foo'>H</div></div>K</div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0", "100: K@0,0"],
    );
    // A box takes the page type of its first box, which then breaks before
    // that box.
    expect(
        &format!("{FLOAT}'><div>F</div><div><div style='page:foo'>H</div></div></div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0"],
    );
    // A flex item stacks the same way.
    expect(
        &format!("{FLOAT}'>F<div style='display:flex'><div style='page:foo'>H</div></div></div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0"],
    );
}

#[test]
fn generated_content_in_a_float_takes_part_like_text() {
    let style = "<style>.b::before{content:'G'} .a::after{content:'K'}</style>";
    expect(
        &format!(
            "{style}<div class='b' style='float:left;width:40px'><div style='page:foo'>H</div></div>Z"
        ),
        &["100: G@0,0 Z@40,0", "100: H@0,0"],
    );
    expect(
        &format!(
            "{style}<div class='a' style='float:left;width:40px'>F<div style='page:foo'>H</div></div>Z"
        ),
        &["100: F@0,0 Z@40,0", "100: H@0,0", "100: K@0,0"],
    );
    expect(
        &format!(
            "{style}<div class='a' style='float:left;width:40px'><div style='page:foo'>H</div></div>Z"
        ),
        &["100: H@0,0 Z@40,0", "100: K@0,0"],
    );
    // Generated content after a box ends it on the page type around the
    // box, so the box after it goes on on the same page, whether the named
    // box is its child or deeper.
    for named in [
        "<div style='page:foo'>H</div>",
        "<div><div style='page:foo'>H</div></div>",
    ] {
        for after in ["<div>G</div>", "G"] {
            expect(
                &format!("{style}{FLOAT}'><div class='a'>{named}</div>{after}</div>Z"),
                &["100: H@0,0 Z@40,0", "100: G@0,30 K@0,0"],
            );
        }
    }
}

#[test]
fn a_float_that_holds_its_flow_together_is_not_split() {
    // A float that clips its content to a height of its own is monolithic.
    expect(
        &format!("{FLOAT};overflow:hidden;height:80px'>F<div style='page:foo'>H</div></div>Z"),
        &["100: F@0,0 H@0,30 Z@40,0"],
    );
    // A multicol container takes a change of page type in its columns.
    expect(
        &format!("{FLOAT};columns:1'>F<div style='page:foo'>H</div></div>Z"),
        &["100: F@0,0 H@0,30 Z@40,0"],
    );
    expect(
        &format!("{FLOAT}'><div style='columns:1'>F<div style='page:foo'>H</div></div></div>Z"),
        &["100: F@0,0 H@0,30 Z@40,0"],
    );
    // A float inside the float, and an absolutely positioned box, are no
    // part of its flow.
    expect(
        &format!("{FLOAT}'>F<div style='float:left;width:30px;page:foo'>G</div>H</div>Z"),
        &["100: F@0,0 G@0,30 H@20,0 Z@40,0"],
    );
    expect(
        &format!("{FLOAT}'>F<div style='position:absolute;page:foo'>G</div>H</div>Z"),
        &["100: F@0,0 G@0,30 H@20,0 Z@40,0"],
    );
}

#[test]
fn the_text_after_a_float_that_opens_a_page_stays_beside_it_when_it_splits() {
    // The text after the float changes the page type, so the float opens
    // the next page with it; a split inside the float leaves that text on
    // the page the float opened.
    for inside in [
        "<div style='page:bar'>H</div>",
        "<div style='page:foo'>H</div>",
        "<div style='break-before:page'>H</div>",
    ] {
        expect(
            &format!("<div style='page:foo'>A</div>{FLOAT}'>F{inside}</div>Z"),
            &["200: A@0,0", "100: F@0,0 Z@40,0", "100: H@0,0"],
        );
    }
    // The lines that go on past the page run beside the rest of the float.
    expect(
        "<div style='page:foo'>A</div>\
         <div style='float:left;width:40px'>F<div style='break-before:page'>H</div></div>\
         Z<br>Y<br>X<br>W",
        &[
            "200: A@0,0",
            "100: F@0,0 Y@40,30 Z@40,0",
            "100: H@0,0 W@0,30 X@40,0",
        ],
    );
}

#[test]
fn a_multicol_container_in_a_float_passes_only_its_own_page_value() {
    // A page value inside the multicol container falls in its columns,
    // which take no change of page type, so the float stays whole.
    let columns = "<div style='columns:1'><div style='page:foo'>H</div></div>";
    expect(
        &format!("{FLOAT}'>F{columns}</div>Z"),
        &["100: F@0,0 H@0,30 Z@40,0"],
    );
    expect(
        &format!("{FLOAT}'>{columns}K</div>Z"),
        &["100: H@0,0 K@0,30 Z@40,0"],
    );
    // The container's own value still changes the page type.
    let columns = "<div style='columns:1;page:foo'>H</div>";
    expect(
        &format!("{FLOAT}'>F{columns}</div>Z"),
        &["100: F@0,0 Z@40,0", "100: H@0,0"],
    );
    expect(
        &format!("{FLOAT}'>{columns}K</div>Z"),
        &["100: H@0,0 Z@40,0", "100: K@0,0"],
    );
}
