//! A generated `::after` line that follows a block with `break-after: page`
//! goes to the next page, as an anonymous line after a forced break does,
//! when the block sits in a table cell or in a float. Chromium starts the
//! line at the top of the next page, and the content after the cell or the
//! float follows it there.
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

const GENERATED: &str = r#"<style>.g::after{content:"x"}</style><div style="height:40px"></div>"#;

fn assert_pages(cases: &[(&str, &[&str])]) {
    for (body, expected) in cases {
        assert_eq!(&pages(body), expected, "{body}");
    }
}

#[test]
fn a_generated_line_after_a_forced_break_in_a_table_cell_starts_the_next_page() {
    assert_pages(&[
        (
            &format!(
                r#"{GENERATED}<div style="width:20px"><table style="border-spacing:0"><tr><td class=g style="padding:0"><div style="height:20px;break-after:page">T</div></td></tr></table>Z</div>"#
            ),
            &["T@0,40", "Z@0,30 x@0,0"],
        ),
        (
            &format!(
                r#"{GENERATED}<div style="width:20px"><table><tr><td class=g><div style="height:20px;break-after:page">T</div></td></tr></table>Z</div>"#
            ),
            &["T@3,43", "Z@0,33 x@3,0"],
        ),
        (
            &format!(
                r#"{GENERATED}<div style="width:20px"><table style="border-spacing:0"><tr><td style="padding:0"><div class=g><div style="height:20px;break-after:page">T</div></div></td></tr></table>Z</div>"#
            ),
            &["T@0,40", "Z@0,30 x@0,0"],
        ),
    ]);
}

#[test]
fn a_generated_line_after_a_forced_break_in_a_float_starts_the_next_page() {
    assert_pages(&[(
        &format!(
            r#"{GENERATED}<div style="width:20px"><div class=g style="float:left"><div style="height:20px;break-after:page">T</div></div><div style="clear:both">Z</div></div>"#
        ),
        &["T@0,40", "Z@0,30 x@0,0"],
    )]);
}

#[test]
fn a_generated_line_after_a_forced_break_in_a_block_of_a_float_leads_the_rest_of_it() {
    assert_pages(&[(
        &format!(
            r#"{GENERATED}<div style="width:20px"><div style="float:left"><div class=g><div style="height:20px;break-after:page">T</div></div><div>W</div></div><div style="clear:both">Z</div></div>"#
        ),
        &["T@0,40", "W@0,30 Z@0,60 x@0,0"],
    )]);
}

#[test]
fn a_generated_line_of_a_float_past_the_page_end_pushes_the_clearing_content() {
    assert_pages(&[(
        &format!(
            r#"{GENERATED}<div style="width:20px"><div class=g style="float:left"><div style="height:45px">T</div></div><div style="clear:both">Z</div></div>"#
        ),
        &["T@0,40", "Z@0,30 x@0,0"],
    )]);
}

#[test]
fn a_line_after_a_forced_break_in_a_float_pushes_the_clearing_content_below_it() {
    assert_pages(&[(
        r#"<div style="height:40px"></div><div style="width:20px"><div style="float:left"><div style="height:20px;break-after:page">T</div>x</div><div style="clear:both">Z</div></div>"#,
        &["T@0,40", "Z@0,30 x@0,0"],
    )]);
}

#[test]
fn a_generated_line_after_a_forced_break_in_a_block_starts_the_next_page() {
    assert_pages(&[(
        &format!(
            r#"{GENERATED}<div style="width:20px"><div class=g><div style="height:20px;break-after:page">T</div></div>Z</div>"#
        ),
        &["T@0,40", "Z@0,30 x@0,0"],
    )]);
}

const SHORT: &str = r#"<style>.g::after{content:"x"}</style><div style="height:10px"></div>"#;

#[test]
fn the_generated_line_after_a_forced_break_moves_when_lines_come_before_the_block() {
    let block = r#"<div style="height:20px;break-after:page">T</div>"#;
    let inline_block = r#"<span style="display:inline-block;width:20px;height:20px"></span>"#;
    assert_pages(&[
        (
            &format!(
                r#"{GENERATED}<div style="width:20px"><div class=g>{inline_block}{block}</div>Z</div>"#
            ),
            &["T@0,70", "Z@0,30 x@0,0"],
        ),
        (
            &format!(r#"{SHORT}<div style="width:20px"><div class=g>A{block}</div>Z</div>"#),
            &["A@0,10 T@0,40", "Z@0,30 x@0,0"],
        ),
        (
            &format!(r#"{SHORT}<div style="width:20px"><div class=g>A B{block}</div>Z</div>"#),
            &["A@0,10 B@0,40 T@0,70", "Z@0,30 x@0,0"],
        ),
        (
            &format!(r#"{SHORT}<div style="width:20px"><div class=g>A{block}B</div>Z</div>"#),
            &["A@0,10 T@0,40", "B@0,0 Z@0,30 x@20,0"],
        ),
        (
            &format!(
                r#"{SHORT}<div style="width:20px"><div class=g>A{block}{inline_block}</div>Z</div>"#
            ),
            &["A@0,10 T@0,40", "Z@0,60 x@0,30"],
        ),
        (
            &format!(
                r#"{SHORT}<div style="width:20px"><table style="border-spacing:0"><tr><td class=g style="padding:0">A{block}</td></tr></table>Z</div>"#
            ),
            &["A@0,10 T@0,40", "Z@0,30 x@0,0"],
        ),
    ]);
}

#[test]
fn the_generated_lines_after_a_block_of_more_lines_move_past_the_page_end() {
    assert_pages(&[
        (
            &format!(
                r#"{SHORT}<div style="width:20px"><div class=g>A<div style="height:45px">T</div></div>Z</div>"#
            ),
            &["A@0,10 T@0,40", "Z@0,30 x@0,0"],
        ),
        (
            &format!(
                r#"{SHORT}<div style="width:20px"><div class=g>A<div style="height:20px;break-after:avoid">T</div></div><div style="height:30px">Z</div></div>"#
            ),
            &["A@0,10 T@0,40 x@0,60", "Z@0,0"],
        ),
    ]);
}

#[test]
fn a_generated_line_beside_a_float_after_a_forced_break_moves_with_the_float() {
    assert_pages(&[(
        &format!(
            r#"{GENERATED}<div style="width:40px"><div class=g><div style="height:20px;break-after:page">T</div><div style="float:left;width:20px;height:20px">F</div></div>Z</div>"#
        ),
        &["T@0,40", "F@0,0 Z@0,30 x@20,0"],
    )]);
}
