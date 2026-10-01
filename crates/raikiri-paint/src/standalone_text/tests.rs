use crate::text::{measure_margin_text, measure_margin_text_advance, measure_margin_text_height};
use raikiri_dom::Document;
use shodo::limits::Limits;

const FONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace"
);

fn engine_document() -> Document {
    let mut doc = Document::new();
    let collection = raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR))
        .expect("the Ahem layer");
    doc.enable_inline_formatting(collection, Limits::default());
    doc
}

#[test]
fn measurements_use_the_document_font_when_the_engine_is_on() {
    let doc = engine_document();
    assert_eq!(measure_margin_text(Some(&doc), "ab ", 10.0, "Ahem"), 20.0);
    assert_eq!(
        measure_margin_text_advance(Some(&doc), "ab ", 10.0, "Ahem"),
        30.0
    );
    assert_eq!(
        measure_margin_text_height(Some(&doc), "abc", 10.0, "Ahem"),
        10.0
    );
    assert_eq!(doc.standalone_text_calls(), 3);
}

#[test]
fn a_document_without_the_engine_measures_like_no_document() {
    // A pin: `shape` returns `None` for a document without the engine, so the
    // old path measures. It does not show that the old path ignores `document`.
    let off = Document::new();
    for content in ["abc", "ab ", ""] {
        assert_eq!(
            measure_margin_text(Some(&off), content, 16.0, "serif"),
            measure_margin_text(None, content, 16.0, "serif")
        );
        assert_eq!(
            measure_margin_text_advance(Some(&off), content, 16.0, "serif"),
            measure_margin_text_advance(None, content, 16.0, "serif")
        );
        assert_eq!(
            measure_margin_text_height(Some(&off), content, 16.0, "serif"),
            measure_margin_text_height(None, content, 16.0, "serif")
        );
    }
    assert_eq!(off.standalone_text_calls(), 0);
}

#[test]
fn what_the_engine_declines_is_measured_by_the_old_path() {
    let doc = engine_document();
    let hebrew = "ab \u{5d0}";
    assert_eq!(
        measure_margin_text(Some(&doc), hebrew, 16.0, "serif"),
        measure_margin_text(None, hebrew, 16.0, "serif")
    );
    assert_eq!(measure_margin_text(Some(&doc), "", 10.0, "Ahem"), 0.0);
    assert_eq!(doc.standalone_text_calls(), 0);
}

#[test]
fn a_family_list_string_is_split_and_unquoted() {
    let doc = engine_document();
    // Callers pass one family name, so the split is defensive. `Ahem` is the
    // second family here, in quotes.
    assert_eq!(
        measure_margin_text(Some(&doc), "abc", 10.0, "No Such, 'Ahem'"),
        30.0
    );
}

#[test]
fn an_unusable_font_size_falls_back_to_sixteen_like_before() {
    let doc = engine_document();
    // Ahem is one em per glyph, so the width follows the size actually used.
    assert_eq!(
        measure_margin_text(Some(&doc), "abc", f32::NAN, "Ahem"),
        48.0
    );
    assert_eq!(measure_margin_text(Some(&doc), "abc", 0.0, "Ahem"), 48.0);
    assert_eq!(
        measure_margin_text(Some(&doc), "abc", f32::INFINITY, "Ahem"),
        48.0
    );
}

use anyrender::Scene;
use anyrender::recording::RenderCommand;
use peniko::Color;

/// `(x, y)` of every recorded glyph, in the page's coordinates.
fn glyph_positions(scene: &Scene) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            let origin = run.transform.translation();
            for glyph in &run.glyphs {
                out.push((origin.x + f64::from(glyph.x), origin.y + f64::from(glyph.y)));
            }
        }
    }
    out
}

fn draw(
    doc: &Document,
    align: parley::Alignment,
    vertical: crate::text::MarginTextVerticalAlign,
) -> Vec<(f64, f64)> {
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        Some(doc),
        &mut scene,
        "abc",
        5.0,
        7.0,
        100.0,
        40.0,
        Color::from_rgba8(0, 0, 0, 255),
        10.0,
        "Ahem",
        align,
        vertical,
    );
    glyph_positions(&scene)
}

#[test]
fn the_text_sits_at_the_baseline_of_its_vertical_alignment() {
    use crate::text::MarginTextVerticalAlign::{Bottom, Middle, Top};
    let doc = engine_document();
    let y = |vertical| draw(&doc, parley::Alignment::Start, vertical)[0].1;
    assert_eq!(y(Top), 15.0);
    assert_eq!(y(Middle), 30.0);
    assert_eq!(y(Bottom), 45.0);
}

#[test]
fn the_text_is_aligned_inside_the_box_width() {
    use crate::text::MarginTextVerticalAlign::Top;
    let doc = engine_document();
    let xs = |align| -> Vec<f64> { draw(&doc, align, Top).into_iter().map(|(x, _)| x).collect() };
    assert_eq!(xs(parley::Alignment::Start), [5.0, 15.0, 25.0]);
    assert_eq!(xs(parley::Alignment::Center), [40.0, 50.0, 60.0]);
    assert_eq!(xs(parley::Alignment::End), [75.0, 85.0, 95.0]);
    assert_eq!(xs(parley::Alignment::Right), [75.0, 85.0, 95.0]);
}

#[test]
fn the_colour_is_the_colour_the_caller_gave() {
    let doc = engine_document();
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        Some(&doc),
        &mut scene,
        "a",
        0.0,
        0.0,
        100.0,
        20.0,
        Color::from_rgba8(255, 0, 0, 255),
        10.0,
        "Ahem",
        parley::Alignment::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    let red = anyrender::Paint::Solid(Color::from_rgba8(255, 0, 0, 255));
    let brushes: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::GlyphRun(run) => Some(run.brush.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(brushes, [red]);
    assert_eq!(doc.standalone_text_calls(), 1);
}

#[test]
fn text_the_engine_declines_is_drawn_by_the_old_path() {
    let doc = engine_document();
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        Some(&doc),
        &mut scene,
        "ab \u{5d0}",
        0.0,
        0.0,
        100.0,
        20.0,
        Color::from_rgba8(0, 0, 0, 255),
        10.0,
        "Ahem",
        parley::Alignment::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    assert_eq!(doc.standalone_text_calls(), 0);
    assert!(!glyph_positions(&scene).is_empty(), "parley still draws it");
}

#[test]
fn without_a_document_the_old_path_draws() {
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        None,
        &mut scene,
        "abc",
        0.0,
        0.0,
        100.0,
        20.0,
        Color::from_rgba8(0, 0, 0, 255),
        16.0,
        "serif",
        parley::Alignment::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    assert!(!glyph_positions(&scene).is_empty());
}

#[test]
fn a_second_line_is_drawn_one_line_below_the_first() {
    let doc = engine_document();
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        Some(&doc),
        &mut scene,
        "a\nb",
        0.0,
        0.0,
        100.0,
        40.0,
        Color::from_rgba8(0, 0, 0, 255),
        10.0,
        "Ahem",
        parley::Alignment::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    // Baselines at the ascent (8) of each 10px line.
    assert_eq!(glyph_positions(&scene), [(0.0, 8.0), (0.0, 18.0)]);
}

/// Glyph x positions of `content` drawn in a 100-wide box at x = 5.
fn aligned_xs(doc: &Document, content: &str, align: parley::Alignment) -> Vec<f64> {
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        Some(doc),
        &mut scene,
        content,
        5.0,
        0.0,
        100.0,
        40.0,
        Color::from_rgba8(0, 0, 0, 255),
        10.0,
        "Ahem",
        align,
        crate::text::MarginTextVerticalAlign::Top,
    );
    glyph_positions(&scene)
        .into_iter()
        .map(|(x, _)| x)
        .collect()
}

#[test]
fn trailing_blanks_hang_when_the_text_is_aligned() {
    // parley hangs trailing blanks outside the aligned content, so "ab " is
    // placed like "ab" (right-aligned at 5 + 80, centred at 5 + 40) and its
    // space glyph follows past the end.
    let doc = engine_document();
    assert_eq!(
        aligned_xs(&doc, "ab ", parley::Alignment::Right),
        [85.0, 95.0, 105.0]
    );
    assert_eq!(
        aligned_xs(&doc, "ab ", parley::Alignment::Center),
        [45.0, 55.0, 65.0]
    );
    // A line before a forced break hangs its blanks too; "b" is not moved.
    assert_eq!(
        aligned_xs(&doc, "a  \nb", parley::Alignment::End),
        [95.0, 105.0, 115.0, 95.0]
    );
    assert_eq!(
        aligned_xs(&doc, "ab ", parley::Alignment::Start),
        [5.0, 15.0, 25.0]
    );
}
