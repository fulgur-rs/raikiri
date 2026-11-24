//! A paragraph whose block that forces a page break after it is followed by
//! an empty block (no content, or floats only) and then by a line keeps the
//! flow after it below that line on the next page. The line box keeps the
//! block-end margins of the empty block apart from those of the paragraph,
//! so the flow after the paragraph does not join the margins that the empty
//! block keeps at the top of the page.
//!
//! The positions are the ones Chromium prints for the same documents
//! (`page.pdf` with 100px square pages). Each 20px Ahem glyph is shown as
//! `text@x,top` on its page, sorted, with 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

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

fn assert_pages(cases: &[(&str, &[&str])]) {
    for (body, expected) in cases {
        assert_eq!(&pages(body), expected, "{body}");
    }
}

const AFTER: &str = r#"<style>.g::after{content:"x"}</style>"#;

#[test]
fn the_flow_after_a_generated_line_below_an_empty_box_follows_the_line() {
    assert_pages(&[
        (
            &format!(
                r#"{AFTER}<div class=g style="width:20px"><div style="break-after:page">T</div><div style="height:0"></div></div><div>Z</div>"#
            ),
            &["T@0,0", "Z@0,30 x@0,0"],
        ),
        (
            &format!(
                r#"{AFTER}<div style="width:20px"><div class=g><div style="break-after:page">T</div><div></div></div></div><div>Z</div>"#
            ),
            &["T@0,0", "Z@0,30 x@0,0"],
        ),
        (
            r#"<style>.g::after{content:"x";display:block}</style><div class=g style="width:20px"><div style="break-after:page">T</div><div style="height:0"></div></div><div>Z</div>"#,
            &["T@0,0", "Z@0,30 x@0,0"],
        ),
        (
            r#"<div style="width:20px"><div><div style="break-after:page">T</div><div></div>x</div></div><div>Z</div>"#,
            &["T@0,0", "Z@0,30 x@0,0"],
        ),
    ]);
}

#[test]
fn the_margins_of_the_empty_box_stay_above_the_line() {
    assert_pages(&[
        (
            &format!(
                r#"{AFTER}<div class=g style="width:20px"><div style="break-after:page">T</div><div style="height:0;margin-bottom:10px"></div></div><div>Z</div>"#
            ),
            &["T@0,0", "Z@0,40 x@0,10"],
        ),
        (
            &format!(
                r#"{AFTER}<div class=g style="width:20px"><div style="break-after:page">T</div><div style="height:0;margin:10px 0"></div></div><div style="margin-top:5px">Z</div>"#
            ),
            &["T@0,0", "Z@0,45 x@0,10"],
        ),
        (
            &format!(
                r#"{AFTER}<div class=g style="width:20px"><div style="break-after:page">T</div><div style="height:0"></div></div><div style="margin-top:20px">Z</div>"#
            ),
            &["T@0,0", "Z@0,50 x@0,0"],
        ),
        (
            &format!(
                r#"{AFTER}<div class=g style="width:20px;margin-bottom:10px"><div style="break-after:page">T</div><div style="height:0;margin-bottom:20px"></div></div><div>Z</div>"#
            ),
            &["T@0,0", "Z@0,60 x@0,20"],
        ),
    ]);
}

#[test]
fn the_flow_after_a_line_below_a_box_of_floats_follows_the_line() {
    assert_pages(&[
        (
            &format!(
                r#"{AFTER}<div class=g style="width:20px"><div style="break-after:page">T</div><div><div style="float:left;width:20px;height:20px">F</div></div></div><div>Z</div>"#
            ),
            &["T@0,0", "F@0,0 Z@0,50 x@0,20"],
        ),
        (
            r#"<div style="width:20px"><div style="break-after:page">T</div><div><div style="float:left;width:20px;height:20px">F</div></div>x</div><div>Z</div>"#,
            &["T@0,0", "F@0,0 Z@0,50 x@0,20"],
        ),
    ]);
}
