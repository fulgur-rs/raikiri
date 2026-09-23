//! Paint coverage for lines that carry text-autospace boundaries.

use anyrender::{Scene, recording::RenderCommand};
use raikiri_dom::{Document, layout_single_page};
use raikiri_paint::paint_single_page;
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

/// Glyph runs in the same font on one autospace line share a baseline, even
/// when only some of them sit next to an autospace boundary.
#[test]
fn same_font_runs_share_a_baseline_on_autospace_lines() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let div = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;font-size:40px;text-autospace:normal"),
    );
    // `Hello ` ends in a space, so only `国` / `abc` meet at a boundary.
    document.append_text(div, "Hello 国abc");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    layout_single_page(
        &mut document,
        &cascade,
        PageBox::A4,
        parley::FontContext::new(),
    )
    .expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    let runs: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => {
                let first = run.glyphs.first()?;
                Some((
                    run.font_data.clone(),
                    run.transform.translation().y + first.y as f64,
                ))
            }
            _ => None,
        })
        .collect();
    assert!(runs.len() >= 2, "expected the line to paint glyph runs");
    for (index, (font, baseline)) in runs.iter().enumerate() {
        for (other_font, other_baseline) in &runs[index + 1..] {
            if font == other_font {
                assert_eq!(
                    baseline, other_baseline,
                    "same-font runs must share a baseline"
                );
            }
        }
    }
}
