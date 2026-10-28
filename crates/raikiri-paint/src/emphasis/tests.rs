use super::centered_offset;

#[test]
fn a_mark_is_centered_in_whole_layout_units() {
    // 16px - 7.99px leaves 513/64 px; half of it truncates to 256/64 px,
    // where an exact half would be 4.005px.
    assert_eq!(centered_offset(16.0, 7.99), 4.0);
    // A single unit of free space is not split.
    assert_eq!(centered_offset(10.0, 9.98), 0.0);
    // A mark wider than its character overhangs both sides evenly.
    assert_eq!(centered_offset(4.0, 5.0), -0.5);
}

mod scaling_tests;

thread_local! {
    pub(super) static GLYPH_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_glyph_work(work: usize) {
    GLYPH_WORK.with(|count| count.set(count.get() + work));
}

#[test]
fn long_vertical_runs_have_subquadratic_glyph_work() {
    use super::*;
    let mut document = Document::new();
    document.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    for n in [32_usize, 64, 128, 256] {
        let style = raikiri_dom::StandaloneStyle {
            writing_mode: WritingMode::VerticalRl,
            ..Default::default()
        };
        let shaped = document
            .shape_standalone_text(&"a".repeat(n), &style, None, StandaloneAlign::Start)
            .unwrap();
        let line = &shaped.lines()[0];
        let run = line
            .fragments()
            .find_map(|f| {
                if let Fragment::GlyphRun(run) = f {
                    Some(run)
                } else {
                    None
                }
            })
            .unwrap();
        let converter = PhysicalConverter::new(
            line.writing_mode(),
            line.used_direction(),
            shaped.container(),
        );
        let mark = shodo::EmphasisMark {
            character: '•',
            font_size: 8.0,
            line_over: true,
            offset: 8.0,
        };
        GLYPH_WORK.with(|count| count.set(0));
        let points =
            vertical_mark_centers(&mark, (line, &run), (WritingMode::VerticalRl, &converter));
        assert_eq!(points.len(), n);
        let work = GLYPH_WORK.with(std::cell::Cell::get);
        assert!(
            work <= 4 * n * (n.ilog2() as usize + 1),
            "n={n}, work={work}"
        );
    }
}

#[test]
fn vertical_centers_match_the_quadratic_reference_for_real_runs() {
    use super::*;
    use shodo::style::TextOrientation;
    let mut document = Document::new();
    document.set_font_collection(shodo::font::FontCollection::new(
        &shodo::limits::Limits::default(),
    ));
    for writing_mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        for direction in [Direction::Ltr, Direction::Rtl] {
            for orientation in [
                TextOrientation::Mixed,
                TextOrientation::Upright,
                TextOrientation::Sideways,
            ] {
                for text in [
                    "abc",
                    "ffi",
                    "a\u{301}文。\t a",
                    "abc אבג def",
                    "\u{200b}\u{200d}\u{00ad}",
                    "👩\u{200d}💻🇯🇵",
                ] {
                    let style = raikiri_dom::StandaloneStyle {
                        writing_mode,
                        direction,
                        text_orientation: orientation,
                        ..Default::default()
                    };
                    let shaped = document
                        .shape_standalone_text(text, &style, None, StandaloneAlign::Start)
                        .unwrap();
                    for line in shaped.lines() {
                        let converter = PhysicalConverter::new(
                            line.writing_mode(),
                            line.used_direction(),
                            shaped.container(),
                        );
                        for fragment in line.fragments() {
                            let Fragment::GlyphRun(run) = fragment else {
                                continue;
                            };
                            assert!(run.glyphs().all(|glyph| glyph.inline_position.is_finite()));
                            for over in [false, true] {
                                let mark = shodo::EmphasisMark {
                                    character: '•',
                                    font_size: 8.0,
                                    line_over: over,
                                    offset: 8.0,
                                };
                                assert_eq!(
                                    vertical_mark_centers(
                                        &mark,
                                        (line, &run),
                                        (writing_mode, &converter)
                                    ),
                                    oracle::vertical_mark_centers(
                                        &mark,
                                        (line, &run),
                                        (writing_mode, &converter)
                                    ),
                                    "{writing_mode:?}/{direction:?}/{orientation:?}/{text:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
