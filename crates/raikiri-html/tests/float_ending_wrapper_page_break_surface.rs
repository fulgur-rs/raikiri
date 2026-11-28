//! A forced break after the last box of a wrapper that only floats follow
//! sends the first of those floats to the next page, where it starts the
//! page. The content after the wrapper lines up beside that float, as it
//! does beside a float right after a forced break among siblings. The
//! positions are the ones Chromium prints for the same documents
//! (`page.pdf` with the page size taken from `@page`).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each 20px Ahem glyph as `text@page:x,top`, sorted, on pages 100px
/// square with 30px lines.
fn glyphs(body: &str) -> Vec<String> {
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
    let mut glyphs = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for run in page.text_runs() {
            // The 20px Ahem baseline sits 21px below the top of its 30px line.
            let top = (run.origin.1 - 21.0).round() as i32;
            let mut x = run.origin.0.round() as i32;
            for glyph in run.text.chars() {
                if glyph != ' ' {
                    glyphs.push(format!("{glyph}@{index}:{x},{top}"));
                }
                x += 20;
            }
        }
    }
    glyphs.sort();
    glyphs
}

fn expect(body: &str, glyphs_expected: &[&str]) {
    let mut expected: Vec<String> = glyphs_expected.iter().map(|g| g.to_string()).collect();
    expected.sort();
    assert_eq!(glyphs(body), expected, "{body}");
}

/// A wrapper whose last box forces a break, then the float `float`.
fn wrapper(float: &str) -> String {
    format!("<div><div style='break-after:page'>T</div>{float}</div>")
}

const F: &str = "<div style='float:left'>F</div>";

#[test]
fn the_content_after_the_wrapper_sits_beside_the_float() {
    // Text, a block, a block inside another, a new formatting context, and
    // a wrapper inside the wrapper.
    for after in [
        "Z",
        "<div>Z</div>",
        "<div><div>Z</div></div>",
        "<div style='display:flow-root'>Z</div>",
    ] {
        expect(
            &format!("{}{after}", wrapper(F)),
            &["F@1:0,0", "T@0:0,0", "Z@1:20,0"],
        );
    }
    expect(
        &format!("<div>{}</div>Z", wrapper(F)),
        &["F@1:0,0", "T@0:0,0", "Z@1:20,0"],
    );
    // A box that clears the float goes below it.
    expect(
        &format!("{}<div style='clear:left'>Z</div>", wrapper(F)),
        &["F@1:0,0", "T@0:0,0", "Z@1:0,30"],
    );
    // A float on the right, and two floats.
    expect(
        &format!("{}Z", wrapper("<div style='float:right'>F</div>")),
        &["F@1:80,0", "T@0:0,0", "Z@1:0,0"],
    );
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper("<div style='float:left'>F</div><div style='float:right'>G</div>")
        ),
        &["F@1:0,0", "G@1:80,0", "T@0:0,0", "Z@1:20,0"],
    );
    // The lines below a tall float, and a float as wide as the page.
    expect(
        &format!(
            "{}<div>Z</div><div>W</div><div>V</div>",
            wrapper("<div style='float:left;height:50px'>F</div>")
        ),
        &["F@1:0,0", "T@0:0,0", "V@1:0,60", "W@1:20,30", "Z@1:20,0"],
    );
    expect(
        &format!(
            "{}<div>Z</div>",
            wrapper("<div style='float:left;width:100px'>F</div>")
        ),
        &["F@1:0,0", "T@0:0,0", "Z@1:0,30"],
    );
    expect(
        &format!(
            "{}<div>Z Y</div>",
            wrapper("<div style='float:left;width:60px;height:40px'></div>")
        ),
        &["T@0:0,0", "Y@1:60,30", "Z@1:60,0"],
    );
    // A float that a break inside a wrapper in the wrapper sent to the
    // next page starts that page, and the float after that wrapper lines up
    // beside it.
    for after in ["<div>Z</div>", "Z"] {
        expect(
            &format!(
                "<div><div><div style='break-after:page'>T</div><div style='float:left'>G</div></div>{F}</div>{after}"
            ),
            &["F@1:20,0", "G@1:0,0", "T@0:0,0", "Z@1:40,0"],
        );
    }
    // A forced break after the wrapper too.
    expect(
        &format!(
            "<div style='break-after:page'><div style='break-after:page'>T</div>{F}</div><div>Z</div>"
        ),
        &["F@1:0,0", "T@0:0,0", "Z@1:20,0"],
    );
}

#[test]
fn the_floats_before_the_break_do_not_hold_the_content_after_the_wrapper() {
    // A float beside the wrapper that ends on the page before, after a
    // spacer or not.
    for (lead, tail) in [
        (
            "<div style='float:left;width:40px;height:60px'></div>",
            "<div>Z</div>",
        ),
        ("<div style='float:left;width:40px;height:60px'></div>", "Z"),
        (
            "<div style='float:left;width:40px;height:20px'></div>",
            "<div>Z</div>",
        ),
        ("<div style='height:60px'></div>", "Z"),
    ] {
        expect(
            &format!("{lead}<div><div style='height:20px;break-after:page'>T</div>{F}</div>{tail}"),
            &[
                "F@1:0,0",
                if lead.contains("float") {
                    "T@0:40,0"
                } else {
                    "T@0:0,60"
                },
                "Z@1:20,0",
            ],
        );
    }
}

#[test]
fn a_float_before_the_break_after_the_wrapper_stays_on_its_page() {
    // The wrapper's own break falls after the float, so the float is no
    // page-start float.
    expect(
        &format!("<div style='break-after:page'><div>T</div>{F}</div><div>Z</div>"),
        &["F@0:0,30", "T@0:0,0", "Z@1:0,0"],
    );
}

#[test]
fn generated_content_after_the_wrapper_lines_up_beside_the_float() {
    // Only the inline position is checked: which page the generated line
    // lands on is decided apart from this.
    let glyphs = glyphs(&format!(
        "<style>.p::after{{content:'A'}}</style><div class=p>{}</div><div>Z</div>",
        wrapper(F)
    ));
    let generated: Vec<_> = glyphs.iter().filter(|g| g.starts_with('A')).collect();
    assert_eq!(generated.len(), 1, "{glyphs:?}");
    assert!(generated[0].contains(":20,"), "{glyphs:?}");
}
