//! A float that a forced break moves to the next page keeps the relative
//! offset that a positioned ancestor gives it, as the content beside it on
//! that page does. Chromium paints the float, and the in-flow content that
//! follows inside the offset ancestor, shifted by the ancestor's `top`.
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

#[test]
fn a_float_after_a_forced_break_keeps_its_ancestors_relative_offset() {
    let cases: &[(&str, &[&str])] = &[
        (
            r#"<div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10 Z@40,0"],
        ),
        (
            r#"<div style="position:relative;top:10px;padding-bottom:30px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10 Z@40,0"],
        ),
        (
            r#"<div><div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10 Z@40,0"],
        ),
        (
            r#"<div style="position:relative;top:10px"><div><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10 Z@40,0"],
        ),
        (
            r#"<div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div>Y</div><div>Z</div>"#,
            &["T@0,10", "F@0,10 Y@40,10 Z@0,30"],
        ),
        (
            r#"<div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px;height:50px">F</div></div><div>Z</div><div>Y</div>"#,
            &["T@0,10", "F@0,10 Y@40,30 Z@40,0"],
        ),
        (
            r#"<div style="padding-bottom:30px"><div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10 Z@40,0"],
        ),
        (
            r#"<div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div><div>Z Y X W</div>"#,
            &["T@0,10", "F@0,10 W@40,30 X@0,30 Y@80,0 Z@40,0"],
        ),
        (
            r#"<div style="position:relative;top:10px;padding-bottom:30px"><div style="break-after:page">T</div><div style="float:left;width:40px;height:50px">F</div></div><div>Z</div><div>Y</div>"#,
            &["T@0,10", "F@0,10 Y@40,30 Z@40,0"],
        ),
        (
            r#"<div style="position:relative;top:10px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div><div style="float:left;width:40px">G</div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10 G@40,10 Z@80,0"],
        ),
        (
            r#"<div style="position:relative;top:10px;padding-bottom:80px"><div style="break-after:page">T</div><div style="float:left;width:40px">F</div></div><div>Z</div>"#,
            &["T@0,10", "F@0,10", "Z@0,0"],
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(pages(body), *expected, "{body}");
    }
}
