use super::*;
use crate::Document;
use crate::layout::test_support::ifc_ahem_fonts;
use raikiri_style::FontFamilyName;
use shodo::limits::Limits;

fn enabled() -> Document {
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    doc
}

fn ahem(size: f32) -> StandaloneStyle {
    StandaloneStyle {
        families: vec![FontFamilyName::named("Ahem")],
        font_size: size,
        ..StandaloneStyle::default()
    }
}

fn shape(doc: &Document, text: &str, width: Option<f32>) -> StandaloneText {
    doc.shape_standalone_text(text, &ahem(10.0), width, StandaloneAlign::Start)
        .expect("shaped")
}

#[test]
fn a_run_is_measured_with_the_document_font() {
    let doc = enabled();
    let text = shape(&doc, "abc", None);
    assert_eq!(text.width(), 30.0);
    assert_eq!(text.advance(), 30.0);
    assert_eq!(text.height(), 10.0);
    assert_eq!(text.lines().len(), 1);
}

#[test]
fn a_trailing_space_counts_in_the_advance_but_not_in_the_width() {
    let doc = enabled();
    let text = shape(&doc, "ab ", None);
    assert_eq!(text.width(), 20.0);
    assert_eq!(text.advance(), 30.0);
}

#[test]
fn spaces_are_preserved_and_a_newline_breaks_the_line() {
    let doc = enabled();
    assert_eq!(shape(&doc, "a  b", None).width(), 40.0);
    let two = shape(&doc, "a\nb", None);
    assert_eq!(
        (two.lines().len(), two.width(), two.height()),
        (2, 10.0, 20.0)
    );
}

#[test]
fn a_run_wraps_at_the_given_width() {
    let doc = enabled();
    let text = shape(&doc, "ab cd", Some(25.0));
    assert_eq!(
        (text.lines().len(), text.width(), text.height()),
        (2, 20.0, 20.0)
    );
}

fn first_glyph_x(text: &StandaloneText) -> f32 {
    use shodo::Fragment;
    let line = &text.lines()[0];
    line.fragments()
        .find_map(|fragment| match fragment {
            Fragment::GlyphRun(run) => run.glyph_origin(0).map(|(x, _)| x),
            _ => None,
        })
        .expect("a glyph run")
}

#[test]
fn alignment_moves_the_line_inside_the_width() {
    let doc = enabled();
    let at = |align| {
        let text = doc
            .shape_standalone_text("abc", &ahem(10.0), Some(100.0), align)
            .expect("shaped");
        first_glyph_x(&text)
    };
    assert_eq!(at(StandaloneAlign::Start), 0.0);
    assert_eq!(at(StandaloneAlign::Left), 0.0);
    assert_eq!(at(StandaloneAlign::Center), 35.0);
    assert_eq!(at(StandaloneAlign::End), 70.0);
    assert_eq!(at(StandaloneAlign::Right), 70.0);
}

#[test]
fn a_family_list_uses_the_first_family_the_collection_has() {
    let doc = enabled();
    let style = StandaloneStyle {
        families: vec![
            FontFamilyName::named("No Such Family"),
            FontFamilyName::named("Ahem"),
        ],
        font_size: 10.0,
        ..StandaloneStyle::default()
    };
    let text = doc
        .shape_standalone_text("abc", &style, None, StandaloneAlign::Start)
        .expect("shaped");
    assert_eq!(text.width(), 30.0);
}

#[test]
fn a_generic_keyword_resolves_in_the_document_layer() {
    let doc = enabled();
    let style = StandaloneStyle {
        families: vec![FontFamilyName::generic("SERIF")],
        font_size: 10.0,
        ..StandaloneStyle::default()
    };
    let text = doc
        .shape_standalone_text("abc", &style, None, StandaloneAlign::Start)
        .expect("shaped");
    // The test layer holds only Ahem, so every generic family is Ahem.
    assert_eq!(text.width(), 30.0);
}

#[test]
fn the_engine_declines_only_empty_text_and_unusable_sizes() {
    let doc = enabled();
    let declined = |text: &str, size: f32| {
        doc.shape_standalone_text(text, &ahem(size), None, StandaloneAlign::Start)
            .is_none()
    };
    assert!(declined("", 10.0), "empty text");
    assert!(declined("ab", 0.0), "no size");
    assert!(declined("ab", f32::NAN), "a non-finite size");
    assert!(!declined("ab \u{5d0}", 10.0), "a right-to-left character");
}

#[test]
fn a_right_to_left_run_keeps_its_glyphs_inside_its_line() {
    // Ahem has no Hebrew glyphs: the two letters take its 1em notdef, so the
    // line is five 10px glyphs wide whatever order the letters are drawn in.
    let doc = enabled();
    let text = shape(&doc, "ab \u{5d0}\u{5d1}", None);
    assert_eq!(text.width(), 50.0);
    let mut xs: Vec<f32> = text.lines()[0]
        .fragments()
        .filter_map(|fragment| match fragment {
            shodo::Fragment::GlyphRun(run) => Some(
                (0..run.glyphs().count())
                    .filter_map(|index| run.glyph_origin(index).map(|(x, _)| x))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs, vec![0.0, 10.0, 20.0, 30.0, 40.0]);
}

#[test]
fn a_document_that_was_never_laid_out_shapes_with_the_installed_fonts() {
    let doc = Document::new();
    let text = doc
        .shape_standalone_text("abc", &ahem(10.0), None, StandaloneAlign::Start)
        .expect("the installed fonts lay the run out");
    assert_eq!(text.lines().len(), 1);
    assert!(text.width() > 0.0);
}

#[test]
fn only_a_produced_result_counts_as_a_call() {
    let doc = enabled();
    assert_eq!(doc.standalone_text_calls(), 0);
    let _ = shape(&doc, "abc", None);
    let _ = doc.shape_standalone_text("", &ahem(10.0), None, StandaloneAlign::Start);
    assert_eq!(doc.standalone_text_calls(), 1);
    assert_eq!(Document::new().standalone_text_calls(), 0);
}

fn shared(
    doc: &Document,
    text: &str,
    style: &StandaloneStyle,
    width: Option<f32>,
    align: StandaloneAlign,
) -> Arc<StandaloneText> {
    doc.shape_standalone_text_shared(text, style, width, align)
        .expect("shaped")
}

#[test]
fn a_shared_run_is_shaped_once_for_the_same_text_style_width_and_alignment() {
    let doc = enabled();
    let style = ahem(10.0);
    let start = StandaloneAlign::Start;
    let first = shared(&doc, "ab cd", &style, Some(25.0), start);
    assert_eq!(doc.standalone_text_calls(), 1);
    let again = shared(&doc, "ab cd", &style, Some(25.0), start);
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(doc.standalone_text_calls(), 1);
    // The run is the one that shaping it directly gives.
    let direct = shape(&doc, "ab cd", Some(25.0));
    assert_eq!(
        (first.lines().len(), first.width(), first.height()),
        (direct.lines().len(), direct.width(), direct.height())
    );
    let calls = doc.standalone_text_calls();
    // Each of the inputs makes another run.
    let differing = [
        shared(&doc, "ab ce", &style, Some(25.0), start),
        shared(&doc, "ab cd", &ahem(12.0), Some(25.0), start),
        shared(&doc, "ab cd", &style, Some(26.0), start),
        shared(&doc, "ab cd", &style, None, start),
        shared(&doc, "ab cd", &style, Some(25.0), StandaloneAlign::End),
    ];
    assert_eq!(doc.standalone_text_calls(), calls + differing.len());
    for run in &differing {
        assert!(!Arc::ptr_eq(&first, run));
    }
}

#[test]
fn a_shared_run_with_nothing_to_shape_is_not_remembered() {
    let doc = enabled();
    assert!(
        doc.shape_standalone_text_shared("", &ahem(10.0), None, StandaloneAlign::Start)
            .is_none()
    );
    assert_eq!(doc.standalone_text_calls(), 0);
}

#[test]
fn shared_runs_nobody_asks_for_again_are_evicted_first() {
    let doc = enabled();
    let style = ahem(10.0);
    let start = StandaloneAlign::Start;
    let kept = shared(&doc, "kept", &style, None, start);
    let fresh_runs = MEMO_CAPACITY + 8;
    for index in 0..fresh_runs {
        shared(&doc, &format!("run {index}"), &style, None, start);
        // Asked for on every round, as a running header is on every page.
        assert!(Arc::ptr_eq(
            &kept,
            &shared(&doc, "kept", &style, None, start)
        ));
    }
    assert_eq!(doc.standalone_text_calls(), 1 + fresh_runs);
    // The newest run is still there, the oldest one is shaped again.
    shared(
        &doc,
        &format!("run {}", fresh_runs - 1),
        &style,
        None,
        start,
    );
    assert_eq!(doc.standalone_text_calls(), 1 + fresh_runs);
    shared(&doc, "run 0", &style, None, start);
    assert_eq!(doc.standalone_text_calls(), 2 + fresh_runs);
}

#[test]
fn shared_runs_are_not_kept_across_font_collections() {
    let mut doc = enabled();
    let style = ahem(10.0);
    let before = shared(&doc, "abc", &style, None, StandaloneAlign::Start);
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    assert_eq!(doc.standalone_text_calls(), 0);
    let after = shared(&doc, "abc", &style, None, StandaloneAlign::Start);
    assert_eq!(doc.standalone_text_calls(), 1);
    assert!(!Arc::ptr_eq(&before, &after));
}

#[test]
fn shared_runs_are_shaped_again_when_the_fonts_change_under_the_document() {
    // The caller keeps a handle on the collection the document got, and
    // changes a generic family list through it.
    let fonts = ifc_ahem_fonts();
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(fonts.clone(), Limits::default());
    let style = ahem(10.0);
    let start = StandaloneAlign::Start;
    let before = shared(&doc, "abc", &style, None, start);
    assert!(Arc::ptr_eq(
        &before,
        &shared(&doc, "abc", &style, None, start)
    ));
    assert_eq!(doc.standalone_text_calls(), 1);
    fonts.set_generic_families(shodo::style::GenericFamily::Serif, vec!["Ahem".to_owned()]);
    let after = shared(&doc, "abc", &style, None, start);
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(doc.standalone_text_calls(), 2);
    // The run shaped under the new fonts is the one handed out from now on.
    assert!(Arc::ptr_eq(
        &after,
        &shared(&doc, "abc", &style, None, start)
    ));
    assert_eq!(doc.standalone_text_calls(), 2);
}

#[test]
fn a_shared_run_of_a_long_text_is_not_remembered() {
    // A text that is new on every page would otherwise be pinned once per
    // remembered run.
    let doc = enabled();
    let style = ahem(10.0);
    let start = StandaloneAlign::Start;
    let longest = "a".repeat(MEMO_MAX_TEXT_BYTES);
    let kept = shared(&doc, &longest, &style, None, start);
    assert!(Arc::ptr_eq(
        &kept,
        &shared(&doc, &longest, &style, None, start)
    ));
    assert_eq!(doc.standalone_text_calls(), 1);
    let too_long = "a".repeat(MEMO_MAX_TEXT_BYTES + 1);
    let first = shared(&doc, &too_long, &style, None, start);
    let second = shared(&doc, &too_long, &style, None, start);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(first.width(), second.width());
    assert_eq!(doc.standalone_text_calls(), 3);
}

#[test]
fn a_shared_run_with_a_long_font_family_list_is_not_remembered() {
    // Each remembered run holds a copy of the list.
    let doc = enabled();
    let start = StandaloneAlign::Start;
    let with_families = |count: usize| {
        let mut style = ahem(10.0);
        style
            .families
            .extend((1..count).map(|index| FontFamilyName::named(format!("Missing {index}"))));
        assert_eq!(style.families.len(), count);
        style
    };
    let longest = with_families(MEMO_MAX_FAMILIES);
    let kept = shared(&doc, "abc", &longest, None, start);
    assert!(Arc::ptr_eq(
        &kept,
        &shared(&doc, "abc", &longest, None, start)
    ));
    assert_eq!(doc.standalone_text_calls(), 1);
    let too_long = with_families(MEMO_MAX_FAMILIES + 1);
    let first = shared(&doc, "abc", &too_long, None, start);
    let second = shared(&doc, "abc", &too_long, None, start);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(first.width(), second.width());
    assert_eq!(doc.standalone_text_calls(), 3);
}

#[test]
fn a_shared_run_of_many_lines_is_not_remembered() {
    // A narrow box breaks a short text into a line per word; the lines are
    // what a remembered run holds on to.
    let doc = enabled();
    let style = ahem(10.0);
    let start = StandaloneAlign::Start;
    let lines = |count: usize| vec!["a"; count].join("\n");
    let longest = lines(MEMO_MAX_LINES);
    let kept = shared(&doc, &longest, &style, None, start);
    assert_eq!(kept.lines().len(), MEMO_MAX_LINES);
    assert!(Arc::ptr_eq(
        &kept,
        &shared(&doc, &longest, &style, None, start)
    ));
    assert_eq!(doc.standalone_text_calls(), 1);
    let too_long = lines(MEMO_MAX_LINES + 1);
    let first = shared(&doc, &too_long, &style, None, start);
    let second = shared(&doc, &too_long, &style, None, start);
    assert_eq!(first.lines().len(), MEMO_MAX_LINES + 1);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(doc.standalone_text_calls(), 3);
}

#[test]
fn a_document_that_was_never_laid_out_shares_no_runs() {
    let doc = Document::new();
    let style = ahem(10.0);
    let first = shared(&doc, "abc", &style, None, StandaloneAlign::Start);
    let second = shared(&doc, "abc", &style, None, StandaloneAlign::Start);
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(first.width(), second.width());
}

#[test]
fn a_wrapped_first_line_counts_its_hanging_space_in_the_advance() {
    // "ab " ends in a soft break: shodo keeps its space in `hang_end` (10), so
    // `inline_size + hang_end` is 30 while the width stays 20.
    let doc = enabled();
    let text = shape(&doc, "ab cd", Some(25.0));
    assert_eq!(text.advance(), 30.0);
    assert_eq!(text.width(), 20.0);
}

#[test]
fn trailing_blanks_before_a_newline_do_not_count_in_the_width() {
    // The first line is "ab  " (40 with its two spaces); the width is the
    // widest line without trailing blanks: "ab" and "cd" are 20 each.
    let doc = enabled();
    let text = shape(&doc, "ab  \ncd", None);
    assert_eq!(
        (text.lines().len(), text.width(), text.advance()),
        (2, 20.0, 40.0)
    );
}

#[test]
fn a_trailing_no_break_space_is_not_counted_in_the_width() {
    // U+00A0 counts as a trailing blank at a line end.
    let doc = enabled();
    let text = shape(&doc, "ab\u{a0}", None);
    assert_eq!((text.width(), text.advance()), (20.0, 30.0));
}

#[test]
fn spaces_inside_a_line_still_count() {
    let doc = enabled();
    assert_eq!(shape(&doc, "a  b  ", None).width(), 40.0);
}

#[test]
fn a_run_of_only_spaces_has_no_width() {
    let doc = enabled();
    let text = shape(&doc, "  ", None);
    assert_eq!(
        (text.width(), text.advance(), text.height()),
        (0.0, 20.0, 10.0)
    );
}

#[test]
fn eligibility_matches_what_shaping_accepts() {
    let doc = enabled();
    for (text, size, expected) in [
        ("abc", 10.0, true),
        ("", 10.0, false),
        ("ab \u{5d0}", 10.0, true),
        ("abc", 0.0, false),
        ("abc", -1.0, false),
        ("abc", f32::NAN, false),
        ("abc", f32::INFINITY, false),
    ] {
        assert_eq!(
            doc.standalone_text_eligible(text, size),
            expected,
            "{text:?} {size}"
        );
        assert_eq!(
            doc.shape_standalone_text(text, &ahem(size), None, StandaloneAlign::Start)
                .is_some(),
            expected,
            "{text:?} {size}"
        );
    }
    assert!(Document::new().standalone_text_eligible("abc", 10.0));
}

/// Byte length of the font of each glyph run on the first line: tells the
/// faces of a two-face layer apart.
fn run_font_sizes(text: &StandaloneText) -> Vec<usize> {
    text.lines()[0]
        .fragments()
        .filter_map(|fragment| match fragment {
            shodo::Fragment::GlyphRun(run) => run.font_data().map(|font| font.data.len()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_generic_keyword_matches_in_any_case_and_a_quoted_one_names_a_family() {
    use crate::layout::ifc::font::{BundledFace, bundled_collection};
    use crate::layout::ifc::test_support::AHEM;
    const CANVAS_TEST: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/text-autospace/CanvasTest-nospace.ttf"
    ));
    // Generic families map to the first face (CanvasTest, which has "A"); an
    // unknown named family falls back to Ahem.
    let fonts = bundled_collection(
        &Limits::default(),
        vec![
            BundledFace {
                family: "CanvasTest".to_owned(),
                bytes: CANVAS_TEST.to_vec(),
            },
            BundledFace {
                family: "Ahem".to_owned(),
                bytes: AHEM.to_vec(),
            },
        ],
        false,
    )
    .expect("two faces");
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(fonts, Limits::default());
    let font_of = |family: FontFamilyName| {
        let style = StandaloneStyle {
            families: vec![family],
            font_size: 10.0,
            ..StandaloneStyle::default()
        };
        let text = doc
            .shape_standalone_text("A", &style, None, StandaloneAlign::Start)
            .expect("shaped");
        run_font_sizes(&text)
    };
    assert_eq!(
        font_of(FontFamilyName::generic("SERIF")),
        [CANVAS_TEST.len()],
        "generic, any case"
    );
    assert_eq!(
        font_of(FontFamilyName::generic("serif")),
        [CANVAS_TEST.len()],
        "generic"
    );
    assert_eq!(
        font_of(FontFamilyName::named("serif")),
        [AHEM.len()],
        "a quoted keyword is a name"
    );
    // A parsed name is one family even when it contains a comma: split, its
    // first part would select CanvasTest.
    assert_eq!(
        font_of(FontFamilyName::named("CanvasTest, Ahem")),
        [AHEM.len()],
        "a comma inside a quoted name does not split it"
    );
}

#[test]
fn vertical_standalone_uses_logical_constraints_and_physical_dimensions() {
    for writing_mode in [
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        let doc = enabled();
        let style = StandaloneStyle {
            writing_mode,
            ..ahem(10.0)
        };
        let text = doc
            .shape_standalone_text("ab cd", &style, Some(25.0), StandaloneAlign::Start)
            .unwrap();
        assert_eq!(text.lines().len(), 2, "{writing_mode:?}");
        assert_eq!(text.inline_size(), 20.0);
        assert_eq!(text.block_size(), 20.0);
        assert_eq!((text.width(), text.height()), (20.0, 20.0));
        assert_eq!(
            text.container(),
            PhysicalSize {
                width: 20.0,
                height: 25.0
            }
        );
        assert_eq!(text.advance(), 30.0);
        let text = doc
            .shape_standalone_text("abc ", &style, None, StandaloneAlign::Start)
            .unwrap();
        assert_eq!((text.width(), text.height()), (10.0, 30.0));
        assert_eq!(text.advance(), 40.0);
    }
}

#[test]
fn standalone_propagates_direction_and_upright_orientation() {
    let doc = enabled();
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let style = StandaloneStyle {
            writing_mode: mode,
            direction: Direction::Rtl,
            text_orientation: TextOrientation::Upright,
            ..ahem(10.0)
        };
        let text = doc
            .shape_standalone_text("ab", &style, Some(100.0), StandaloneAlign::Start)
            .unwrap();
        assert_eq!(text.lines()[0].writing_mode(), mode);
        assert_eq!(
            text.lines()[0].used_direction(),
            if mode.is_vertical() {
                Direction::Ltr
            } else {
                Direction::Rtl
            }
        );
        for fragment in text.lines()[0].fragments() {
            if let shodo::Fragment::GlyphRun(run) = fragment {
                assert_eq!(
                    run.orientation(),
                    if mode.is_vertical() {
                        shodo::GlyphOrientation::Upright
                    } else {
                        shodo::GlyphOrientation::Horizontal
                    }
                );
            }
        }
    }
}

#[test]
fn a_fitted_run_matches_one_shaped_inside_its_measured_width() {
    let doc = enabled();
    let vertical = StandaloneStyle {
        writing_mode: WritingMode::VerticalRl,
        ..ahem(10.0)
    };
    for style in [ahem(10.0), vertical] {
        for text in ["1. ", "42.", "• ", "a\nbc ", " ", "ab  \n"] {
            let align = StandaloneAlign::Start;
            let fitted = doc.shape_standalone_text_fitted(text, &style, align);
            let reshaped = doc
                .shape_standalone_text(text, &style, None, align)
                .and_then(|measured| {
                    doc.shape_standalone_text(text, &style, Some(measured.width()), align)
                });
            assert_eq!(format!("{fitted:?}"), format!("{reshaped:?}"), "{text:?}");
        }
    }
}

fn image(offset: usize, width: f32, height: f32) -> StandaloneImage {
    StandaloneImage {
        offset,
        width,
        height,
    }
}

fn shape_with_images(
    doc: &Document,
    text: &str,
    images: &[StandaloneImage],
    width: Option<f32>,
) -> StandaloneText {
    doc.shape_standalone_text_with_images(text, images, &ahem(10.0), width, StandaloneAlign::Start)
        .expect("shaped")
}

#[test]
fn an_image_sits_on_the_baseline_after_the_text_before_it() {
    let doc = enabled();
    let text = shape_with_images(&doc, "ab", &[image(2, 30.0, 20.0)], None);
    // The image is 20px over the baseline, the strut descends 2px under it.
    assert_eq!((text.width(), text.height()), (50.0, 22.0));
    let placed = text.images();
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].0, 0);
    let rect = placed[0].1;
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (20.0, 0.0, 30.0, 20.0)
    );
}

#[test]
fn images_are_placed_in_the_order_of_the_list_between_the_pieces_of_text() {
    let doc = enabled();
    let text = shape_with_images(
        &doc,
        "xy",
        &[
            image(0, 10.0, 10.0),
            image(1, 20.0, 30.0),
            image(2, 5.0, 5.0),
        ],
        None,
    );
    let placed: Vec<_> = text
        .images()
        .into_iter()
        .map(|(index, rect)| (index, rect.x, rect.width))
        .collect();
    assert_eq!(placed, [(0, 0.0, 10.0), (1, 20.0, 20.0), (2, 50.0, 5.0)]);
}

#[test]
fn a_run_of_images_alone_has_no_text_to_decline() {
    let doc = enabled();
    let text = shape_with_images(&doc, "", &[image(0, 30.0, 20.0)], None);
    assert_eq!((text.width(), text.height()), (30.0, 22.0));
    // Without an image, empty text is still declined.
    assert!(
        doc.shape_standalone_text_with_images("", &[], &ahem(10.0), None, StandaloneAlign::Start)
            .is_none()
    );
    // And so is a size that cannot be shaped.
    assert!(
        doc.shape_standalone_text_with_images(
            "",
            &[image(0, 30.0, 20.0)],
            &ahem(0.0),
            None,
            StandaloneAlign::Start,
        )
        .is_none()
    );
}

#[test]
fn an_image_that_does_not_fit_beside_the_text_takes_the_next_line() {
    let doc = enabled();
    let text = shape_with_images(&doc, "ab", &[image(2, 30.0, 20.0)], Some(40.0));
    assert_eq!(text.lines().len(), 2);
    let rect = text.images()[0].1;
    assert_eq!((rect.x, rect.y), (0.0, 10.0));
    assert_eq!(text.height(), 32.0);
}

#[test]
fn an_offset_inside_a_character_or_past_the_text_is_moved_to_a_place_in_it() {
    let doc = enabled();
    // 'é' is two bytes; an image in the middle of it goes before it, and an
    // offset after the end goes at the end.
    let text = shape_with_images(
        &doc,
        "éa",
        &[image(1, 10.0, 10.0), image(99, 10.0, 10.0)],
        None,
    );
    let xs: Vec<_> = text.images().into_iter().map(|(_, rect)| rect.x).collect();
    assert_eq!(xs, [0.0, 30.0]);
}

#[test]
fn alignment_moves_an_image_with_its_line() {
    let doc = enabled();
    let text = doc
        .shape_standalone_text_with_images(
            "ab",
            &[image(2, 30.0, 20.0)],
            &ahem(10.0),
            Some(100.0),
            StandaloneAlign::Right,
        )
        .expect("shaped");
    let rect = text.images()[0].1;
    assert_eq!(rect.x, 70.0);
}

#[test]
fn images_stand_across_the_lines_of_vertical_text() {
    let doc = enabled();
    let mut style = ahem(10.0);
    style.writing_mode = shodo::geometry::WritingMode::VerticalRl;
    let text = doc
        .shape_standalone_text_with_images(
            "ab",
            &[image(2, 30.0, 20.0)],
            &style,
            None,
            StandaloneAlign::Start,
        )
        .expect("shaped");
    let rect = text.images()[0].1;
    // The image keeps its physical size, and follows the text down the line.
    assert_eq!((rect.width, rect.height), (30.0, 20.0));
    assert_eq!(rect.y, 20.0);
}

#[test]
fn a_run_with_images_is_not_remembered() {
    let doc = enabled();
    let style = ahem(10.0);
    let images = [image(2, 30.0, 20.0)];
    let first = doc
        .shape_standalone_text_shared_with_images(
            "ab",
            &images,
            &style,
            None,
            StandaloneAlign::Start,
        )
        .expect("shaped");
    let second = doc
        .shape_standalone_text_shared_with_images(
            "ab",
            &images,
            &style,
            None,
            StandaloneAlign::Start,
        )
        .expect("shaped");
    assert!(!std::sync::Arc::ptr_eq(&first, &second));
    // The same text without images is a different run.
    let plain = doc
        .shape_standalone_text_shared("ab", &style, None, StandaloneAlign::Start)
        .expect("shaped");
    assert_eq!(plain.images(), []);
    assert_eq!(plain.width(), 20.0);
}

#[test]
fn an_image_size_that_is_not_a_length_counts_as_zero() {
    let doc = enabled();
    let text = shape_with_images(
        &doc,
        "ab",
        &[image(1, f32::NAN, f32::INFINITY), image(2, -5.0, 3.0)],
        None,
    );
    assert_eq!(text.width(), 20.0);
    assert_eq!(text.height(), 10.0);
}

#[test]
fn blanks_after_the_last_image_of_a_line_do_not_count_in_the_width() {
    let doc = enabled();
    // "ab", the image and a blank: the blank ends the line.
    let text = shape_with_images(&doc, "ab ", &[image(2, 30.0, 20.0)], None);
    assert_eq!(text.width(), 50.0);
    assert_eq!(text.advance(), 60.0);
    // A blank in front of the image is inside the line.
    let text = shape_with_images(&doc, "a ", &[image(2, 30.0, 20.0)], None);
    assert_eq!(text.width(), 50.0);
    // And so is the blank between two images.
    let text = shape_with_images(
        &doc,
        " ",
        &[image(0, 10.0, 10.0), image(1, 10.0, 10.0)],
        None,
    );
    assert_eq!(text.width(), 30.0);
}

#[test]
fn trailing_blanks_hang_outside_a_right_aligned_line_with_an_image() {
    let doc = enabled();
    let text = doc
        .shape_standalone_text_with_images(
            "ab ",
            &[image(2, 30.0, 20.0)],
            &ahem(10.0),
            Some(100.0),
            StandaloneAlign::Right,
        )
        .expect("shaped");
    // The image ends at the right edge, and the blank hangs beyond it.
    let rect = text.images()[0].1;
    assert_eq!(rect.x + rect.width, 100.0);
}

#[test]
fn each_line_of_a_run_with_images_keeps_its_images_where_they_were_inserted() {
    let doc = enabled();
    // "a" [30] " " newline "b" [20] " ": the blanks end both lines.
    let text = doc
        .shape_standalone_text_with_images(
            "a \nb ",
            &[image(1, 30.0, 20.0), image(4, 20.0, 20.0)],
            &ahem(10.0),
            Some(100.0),
            StandaloneAlign::Right,
        )
        .expect("shaped");
    assert_eq!(text.width(), 40.0);
    let placed: Vec<_> = text
        .images()
        .into_iter()
        .map(|(index, rect)| (index, rect.x + rect.width))
        .collect();
    assert_eq!(placed, [(0, 100.0), (1, 100.0)]);
}
