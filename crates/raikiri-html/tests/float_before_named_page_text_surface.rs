//! A float between a box with a named page and text with another page type.
//! The text opens a new page, and Chromium takes the last float before the
//! text along onto that page: the text lines up beside it there. Floats
//! before that one stay on the page before, and so do floats before a block.
//!
//! The positions are the ones Chromium prints for the same documents
//! (`page.pdf` with the page sizes taken from `@page`): `foo` pages are
//! 120px wide and other pages 100px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page as its width, then each 20px Ahem glyph on it as `text@x,top`,
/// sorted, with 30px lines.
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
         @page foo{{size:120px 100px}}\
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

const F: &str = "<div style='float:left;width:40px'>F</div>";
const G: &str = "<div style='float:left;width:40px'>G</div>";

/// The text after the float has another page type than the box before the
/// float, so it opens a new page; the float right before it goes on to that
/// page too, and the text lines up beside it.
#[test]
fn the_float_right_before_the_text_goes_on_to_the_new_page_with_it() {
    let cases: Vec<(String, Vec<&str>)> = vec![
        // A left float.
        (
            format!("<div style='page:foo'>T</div>{F} Z"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
        // A right float.
        (
            "<div style='page:foo'>T</div><div style='float:right;width:40px'>F</div>Z".to_string(),
            vec![
                "120: T@0,0",
                "100: F@60,0 Z@0,0",
            ],
        ),
        // A float taller than a line.
        (
            "<div style='page:foo'>T</div><div style='float:left;width:40px;height:80px'>F</div>Z".to_string(),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
        // A float with a top margin keeps it on the new page.
        (
            "<div style='page:foo'>T</div><div style='float:left;width:40px;margin-top:10px'>F</div>Z".to_string(),
            vec![
                "120: T@0,0",
                "100: F@0,10 Z@40,0",
            ],
        ),
        // More text after the float.
        (
            format!("<div style='page:foo'>T</div>{F}Z Y"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Y@80,0 Z@40,0",
            ],
        ),
        // Text in an inline box.
        (
            format!("<div style='page:foo'>T</div>{F}<span>Z</span>"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
        // A float inside an inline box.
        (
            format!("<div style='page:foo'>T</div><span>{F}</span>Z"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
        // A line break after the float ends the line beside it.
        (
            format!("<div style='page:foo'>T</div>{F}<br>Z"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@0,30",
            ],
        ),
        // The named box ends deeper in.
        (
            format!("<div style='page:foo'><div>T</div></div>{F}Z"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
        // Float and text inside a block of their own: the block opens the page.
        (
            format!("<div style='page:foo'>T</div><div>{F}Z</div>"),
            vec![
                "120: T@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
        // After a named page that opened a page of its own.
        (
            format!("<div>T</div><div style='page:foo'>U</div>{F}Z"),
            vec![
                "100: T@0,0",
                "120: U@0,0",
                "100: F@0,0 Z@40,0",
            ],
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(pages(&body), expected, "{body}");
    }
}

/// Only the last float before the text goes on with it: the floats before
/// that one stay on the page before.
#[test]
fn earlier_floats_stay_on_the_page_before() {
    let cases: Vec<(String, Vec<&str>)> = vec![
        // Two left floats.
        (
            format!("<div style='page:foo'>T</div>{F}{G} Z"),
            vec!["120: F@0,30 T@0,0", "100: G@0,0 Z@40,0"],
        ),
        // A left float, then a right one.
        (
            format!("<div style='page:foo'>T</div>{F}<div style='float:right;width:40px'>G</div>Z"),
            vec!["120: F@0,30 T@0,0", "100: G@60,0 Z@0,0"],
        ),
        // The last float clears the one before.
        (
            format!(
                "<div style='page:foo'>T</div>{F}<div style='float:left;width:40px;clear:left'>G</div>Z"
            ),
            vec!["120: F@0,30 T@0,0", "100: G@0,0 Z@40,0"],
        ),
        // More text, which wraps beside the float.
        (
            format!("<div style='page:foo'>T</div>{F}{G} Z Y X W"),
            vec![
                "120: F@0,30 T@0,0",
                "100: G@0,0 W@40,30 X@0,30 Y@80,0 Z@40,0",
            ],
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(pages(&body), expected, "{body}");
    }
}

/// A block after the float opens the new page alone.
#[test]
fn floats_before_a_block_stay_on_the_page_before() {
    let cases: Vec<(String, Vec<&str>)> = vec![
        // The block starts the new page at its start edge.
        (
            format!("<div style='page:foo'>T</div>{F}<div>Z</div>"),
            vec!["120: F@0,30 T@0,0", "100: Z@0,0"],
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(pages(&body), expected, "{body}");
    }
}
