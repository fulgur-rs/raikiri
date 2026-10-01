use super::*;
use crate::Document;
use crate::layout::test_support::ifc_ahem_fonts;
use shodo::limits::Limits;

fn enabled() -> Document {
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    doc
}

fn ahem(size: f32) -> StandaloneStyle {
    StandaloneStyle {
        families: vec!["Ahem".to_owned()],
        font_size: size,
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
        families: vec!["No Such Family".to_owned(), "Ahem".to_owned()],
        font_size: 10.0,
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
        families: vec!["SERIF".to_owned()],
        font_size: 10.0,
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
    let font_of = |family: &str| {
        let style = StandaloneStyle {
            families: vec![family.to_owned()],
            font_size: 10.0,
        };
        let text = doc
            .shape_standalone_text("A", &style, None, StandaloneAlign::Start)
            .expect("shaped");
        run_font_sizes(&text)
    };
    assert_eq!(font_of("SERIF"), [CANVAS_TEST.len()], "generic, any case");
    assert_eq!(font_of("serif"), [CANVAS_TEST.len()], "generic");
    assert_eq!(
        font_of("'serif'"),
        [AHEM.len()],
        "a quoted keyword is a name"
    );
    assert_eq!(font_of("\"serif\""), [AHEM.len()], "double quotes too");
}
