use crate::text::{measure_margin_text, measure_margin_text_advance, measure_margin_text_height};
use raikiri_dom::{Document, StandaloneAlign};
use shodo::limits::Limits;

const FONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace"
);

fn engine_document() -> Document {
    let mut doc = Document::new();
    let collection = raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR))
        .expect("the Ahem layer");
    doc.set_font_collection_with_limits(collection, Limits::default());
    doc
}

#[test]
fn measurements_use_the_document_font_when_the_engine_is_on() {
    let doc = engine_document();
    assert_eq!(measure_margin_text(&doc, "ab ", 10.0, "Ahem"), 20.0);
    assert_eq!(measure_margin_text_advance(&doc, "ab ", 10.0, "Ahem"), 30.0);
    assert_eq!(measure_margin_text_height(&doc, "abc", 10.0, "Ahem"), 10.0);
    assert_eq!(doc.standalone_text_calls(), 3);
}

#[test]
fn a_family_list_string_is_split_and_unquoted() {
    let doc = engine_document();
    // Callers pass one family name, so the split is defensive. `Ahem` is the
    // second family here, in quotes.
    assert_eq!(
        measure_margin_text(&doc, "abc", 10.0, "No Such, 'Ahem'"),
        30.0
    );
}

#[test]
fn an_unusable_font_size_falls_back_to_sixteen_like_before() {
    let doc = engine_document();
    // Ahem is one em per glyph, so the width follows the size actually used.
    assert_eq!(measure_margin_text(&doc, "abc", f32::NAN, "Ahem"), 48.0);
    assert_eq!(measure_margin_text(&doc, "abc", 0.0, "Ahem"), 48.0);
    assert_eq!(
        measure_margin_text(&doc, "abc", f32::INFINITY, "Ahem"),
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
    align: StandaloneAlign,
    vertical: crate::text::MarginTextVerticalAlign,
) -> Vec<(f64, f64)> {
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        doc,
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
    let y = |vertical| draw(&doc, StandaloneAlign::Start, vertical)[0].1;
    assert_eq!(y(Top), 15.0);
    assert_eq!(y(Middle), 30.0);
    assert_eq!(y(Bottom), 45.0);
}

#[test]
fn the_text_is_aligned_inside_the_box_width() {
    use crate::text::MarginTextVerticalAlign::Top;
    let doc = engine_document();
    let xs = |align| -> Vec<f64> { draw(&doc, align, Top).into_iter().map(|(x, _)| x).collect() };
    assert_eq!(xs(StandaloneAlign::Start), [5.0, 15.0, 25.0]);
    assert_eq!(xs(StandaloneAlign::Center), [40.0, 50.0, 60.0]);
    assert_eq!(xs(StandaloneAlign::End), [75.0, 85.0, 95.0]);
    assert_eq!(xs(StandaloneAlign::Right), [75.0, 85.0, 95.0]);
}

#[test]
fn the_colour_is_the_colour_the_caller_gave() {
    let doc = engine_document();
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        &doc,
        &mut scene,
        "a",
        0.0,
        0.0,
        100.0,
        20.0,
        Color::from_rgba8(255, 0, 0, 255),
        10.0,
        "Ahem",
        StandaloneAlign::Start,
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
fn a_second_line_is_drawn_one_line_below_the_first() {
    let doc = engine_document();
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        &doc,
        &mut scene,
        "a\nb",
        0.0,
        0.0,
        100.0,
        40.0,
        Color::from_rgba8(0, 0, 0, 255),
        10.0,
        "Ahem",
        StandaloneAlign::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    // Baselines at the ascent (8) of each 10px line.
    assert_eq!(glyph_positions(&scene), [(0.0, 8.0), (0.0, 18.0)]);
}

/// Glyph x positions of `content` drawn in a 100-wide box at x = 5.
fn aligned_xs(doc: &Document, content: &str, align: StandaloneAlign) -> Vec<f64> {
    let mut scene = Scene::new();
    crate::text::draw_margin_text(
        doc,
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
    // Trailing blanks hang outside the aligned content, so "ab " is
    // placed like "ab" (right-aligned at 5 + 80, centred at 5 + 40) and its
    // space glyph follows past the end.
    let doc = engine_document();
    assert_eq!(
        aligned_xs(&doc, "ab ", StandaloneAlign::Right),
        [85.0, 95.0, 105.0]
    );
    assert_eq!(
        aligned_xs(&doc, "ab ", StandaloneAlign::Center),
        [45.0, 55.0, 65.0]
    );
    // A line before a forced break hangs its blanks too; "b" is not moved.
    assert_eq!(
        aligned_xs(&doc, "a  \nb", StandaloneAlign::End),
        [95.0, 105.0, 115.0, 95.0]
    );
    assert_eq!(
        aligned_xs(&doc, "ab ", StandaloneAlign::Start),
        [5.0, 15.0, 25.0]
    );
}

#[test]
fn standalone_text_is_laid_out_by_the_engine_only() {
    // A document that was never laid out has no fonts of its own: the run is
    // still shaped by the engine, over the installed fonts. There is no other
    // path to fall back to.
    let doc = Document::new();
    let run = super::shape(&doc, "abc", 16.0, "serif", None, StandaloneAlign::Start)
        .expect("the engine shapes the run");
    let glyphs: usize = run.lines()[0]
        .fragments()
        .map(|fragment| match fragment {
            shodo::Fragment::GlyphRun(run) => run.glyphs().count(),
            _ => 0,
        })
        .sum();
    assert!(glyphs >= 3, "{glyphs}");
}

#[test]
fn a_right_to_left_run_is_drawn_by_the_engine_inside_its_box() {
    // Ahem draws the two Hebrew letters with its 1em notdef: five 10px glyphs
    // starting at the box's left edge (x = 5), whatever their visual order.
    let doc = engine_document();
    let mut xs = aligned_xs(&doc, "ab \u{5d0}\u{5d1}", StandaloneAlign::Start);
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, [5.0, 15.0, 25.0, 35.0, 45.0]);
    assert_eq!(doc.standalone_text_calls(), 1);
}

fn styled_scene(
    content: &str,
    mode: shodo::geometry::WritingMode,
    orientation: shodo::style::TextOrientation,
    direction: shodo::geometry::Direction,
    align: StandaloneAlign,
    cross_align: crate::text::MarginTextVerticalAlign,
) -> Scene {
    let doc = engine_document();
    let mut style = super::style(10.0, "Ahem");
    style.writing_mode = mode;
    style.text_orientation = orientation;
    style.direction = direction;
    let mut scene = Scene::new();
    crate::text::draw_margin_text_styled(
        &doc,
        &mut scene,
        content,
        5.0,
        7.0,
        100.0,
        40.0,
        Color::from_rgba8(0, 0, 0, 255),
        &style,
        align,
        cross_align,
    );
    scene
}

#[test]
fn vertical_columns_preserve_newlines_and_rotate_outlines() {
    use crate::text::MarginTextVerticalAlign::Top;
    use shodo::geometry::{Direction::Ltr, WritingMode::*};
    use shodo::style::TextOrientation::Mixed;
    for (mode, dx, dy, matrix) in [
        (VerticalRl, -10.0, 10.0, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
        (VerticalLr, 10.0, 10.0, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
        (SidewaysRl, -10.0, 10.0, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
        (SidewaysLr, 10.0, -10.0, [0.0, -1.0, 1.0, 0.0, 0.0, 0.0]),
    ] {
        let scene = styled_scene("ab\ncd", mode, Mixed, Ltr, StandaloneAlign::Start, Top);
        let positions = glyph_positions(&scene);
        assert_eq!(positions.len(), 4, "{mode:?}");
        assert_eq!(positions[1].0, positions[0].0);
        assert_eq!(positions[1].1 - positions[0].1, dy, "{mode:?}");
        assert_eq!(positions[2].0 - positions[0].0, dx, "{mode:?}");
        assert_eq!(positions[2].1, positions[0].1);
        for command in &scene.commands {
            if let RenderCommand::GlyphRun(run) = command {
                assert_eq!(run.glyph_transform.unwrap().as_coeffs(), matrix, "{mode:?}");
            }
        }
    }
}

#[test]
fn upright_vertical_outlines_are_not_rotated_or_mirrored() {
    use shodo::geometry::{Direction::Rtl, WritingMode::VerticalRl};
    use shodo::style::TextOrientation::Upright;
    let scene = styled_scene(
        "ab",
        VerticalRl,
        Upright,
        Rtl,
        StandaloneAlign::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    let positions = glyph_positions(&scene);
    assert_eq!(positions[1].1 - positions[0].1, 10.0);
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            assert!(run.glyph_transform.is_none());
        }
    }
}

#[test]
fn trailing_blanks_do_not_move_aligned_content_in_any_flow() {
    use crate::text::MarginTextVerticalAlign::Top;
    use shodo::geometry::{Direction::*, WritingMode::*};
    use shodo::style::TextOrientation::Mixed;
    for mode in [HorizontalTb, VerticalRl, VerticalLr, SidewaysRl, SidewaysLr] {
        for direction in [Ltr, Rtl] {
            for align in [
                StandaloneAlign::Start,
                StandaloneAlign::End,
                StandaloneAlign::Left,
                StandaloneAlign::Right,
                StandaloneAlign::Center,
            ] {
                let bare = glyph_positions(&styled_scene("ab", mode, Mixed, direction, align, Top));
                for suffix in [" ", "\u{a0}", "\t"] {
                    let padded = glyph_positions(&styled_scene(
                        &format!("ab{suffix}"),
                        mode,
                        Mixed,
                        direction,
                        align,
                        Top,
                    ));
                    // Ahem ASCII remains a single LTR run inside an RTL paragraph.
                    assert_eq!(
                        &padded[..2],
                        bare.as_slice(),
                        "{mode:?} {direction:?} {align:?} {suffix:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn vertical_cross_alignment_uses_the_available_width() {
    use crate::text::MarginTextVerticalAlign::{Bottom, Middle, Top};
    use shodo::geometry::{Direction::Ltr, WritingMode::*};
    use shodo::style::TextOrientation::Mixed;
    for (mode, sign) in [
        (VerticalRl, -1.0),
        (VerticalLr, 1.0),
        (SidewaysRl, -1.0),
        (SidewaysLr, 1.0),
    ] {
        let at = |cross| {
            glyph_positions(&styled_scene(
                "ab",
                mode,
                Mixed,
                Ltr,
                StandaloneAlign::Start,
                cross,
            ))[0]
                .0
        };
        assert_eq!(at(Middle) - at(Top), sign * 45.0, "{mode:?}");
        assert_eq!(at(Bottom) - at(Top), sign * 90.0, "{mode:?}");
    }
}

#[test]
fn rtl_start_and_end_align_to_opposite_physical_edges_without_mirroring() {
    use shodo::geometry::{Direction::Rtl, WritingMode::HorizontalTb};
    use shodo::style::TextOrientation::Mixed;
    for (align, x) in [
        (StandaloneAlign::Start, 85.0),
        (StandaloneAlign::End, 5.0),
        (StandaloneAlign::Left, 5.0),
        (StandaloneAlign::Right, 85.0),
        (StandaloneAlign::Center, 45.0),
    ] {
        let scene = styled_scene(
            "ab",
            HorizontalTb,
            Mixed,
            Rtl,
            align,
            crate::text::MarginTextVerticalAlign::Top,
        );
        assert_eq!(glyph_positions(&scene), [(x, 15.0), (x + 10.0, 15.0)]);
        for command in &scene.commands {
            if let RenderCommand::GlyphRun(run) = command {
                assert!(run.glyph_transform.is_none());
            }
        }
    }
}

#[test]
fn vertical_text_wraps_to_the_box_height_not_its_width() {
    use shodo::geometry::{Direction::Ltr, WritingMode::VerticalRl};
    use shodo::style::TextOrientation::Mixed;
    let scene = styled_scene(
        "abc def",
        VerticalRl,
        Mixed,
        Ltr,
        StandaloneAlign::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    let positions = glyph_positions(&scene);
    let mut xs: Vec<_> = positions.iter().map(|p| p.0).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    assert_eq!(xs.len(), 2);
    assert_eq!(xs[1] - xs[0], 10.0);
}

#[test]
fn overflowing_rl_columns_keep_the_first_column_at_the_right_edge() {
    use crate::text::MarginTextVerticalAlign::Top;
    use shodo::geometry::{
        Direction::Ltr,
        WritingMode::{SidewaysRl, VerticalRl},
    };
    use shodo::style::TextOrientation::Mixed;
    for mode in [VerticalRl, SidewaysRl] {
        let one = glyph_positions(&styled_scene(
            "a",
            mode,
            Mixed,
            Ltr,
            StandaloneAlign::Start,
            Top,
        ));
        let many = glyph_positions(&styled_scene(
            &["a"; 11].join("\n"),
            mode,
            Mixed,
            Ltr,
            StandaloneAlign::Start,
            Top,
        ));
        assert_eq!(one[0], many[0], "{mode:?}");
        assert_eq!(many[10].0, one[0].0 - 100.0);
    }
}

#[test]
fn hebrew_run_stays_at_the_rtl_start_edge_without_mirroring() {
    use shodo::geometry::{Direction::Rtl, WritingMode::HorizontalTb};
    use shodo::style::TextOrientation::Mixed;
    let scene = styled_scene(
        "\u{5d0}\u{5d1}",
        HorizontalTb,
        Mixed,
        Rtl,
        StandaloneAlign::Start,
        crate::text::MarginTextVerticalAlign::Top,
    );
    let mut positions = glyph_positions(&scene);
    positions.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(positions, [(85.0, 15.0), (95.0, 15.0)]);
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            assert!(run.glyph_transform.is_none());
        }
    }
}
