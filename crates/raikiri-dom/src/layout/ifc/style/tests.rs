use super::*;
use crate::layout::ifc::test_support::{block_fixture, span};
use shodo::geometry::{Direction, WritingMode};
use shodo::style::{
    FontFamily, GenericFamily, LineHeight, TextAlign, TextAlignLast, TextCombineUpright,
    TextOrientation, TextWrapMode, WhiteSpaceCollapse,
};

fn root_style(extra: &str) -> Result<InlineStyle, IfcError> {
    let fixture = block_fixture(extra, |doc, root| {
        doc.append_text(root, "x");
    });
    inline_style(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
}

fn fonts() -> shodo::font::FontCollection {
    crate::layout::ifc::test_support::ahem_fonts()
}

#[test]
fn font_size_weight_and_family_map() {
    let style = root_style("font-size:12px;font-weight:700;font-family:Ahem, serif").expect("map");
    assert_eq!(style.font_size, 12.0);
    assert_eq!(style.font_weight, 700.0);
    assert_eq!(
        style.font_families,
        vec![
            FontFamily::Named("Ahem".to_owned()),
            FontFamily::Generic(GenericFamily::Serif)
        ]
    );
}

#[test]
fn absolute_letter_and_word_spacing_map() {
    let style = root_style("letter-spacing:2px;word-spacing:3px").expect("map");
    assert_eq!(style.letter_spacing, 2.0);
    assert_eq!(style.word_spacing, 3.0);
}

#[test]
fn line_height_maps_number_and_length() {
    assert_eq!(
        root_style("line-height:1.5").expect("number").line_height,
        LineHeight::Number(1.5)
    );
    assert_eq!(
        root_style("line-height:20px").expect("length").line_height,
        LineHeight::Px(20.0)
    );
}

#[test]
fn ch_spacing_is_measured_with_the_declaring_font() {
    // Ahem at 10px: one ch is 10px.
    let style = root_style("letter-spacing:2ch;word-spacing:1ch").expect("map");
    assert_eq!(style.letter_spacing, 20.0);
    assert_eq!(style.word_spacing, 10.0);
}

#[test]
fn an_inherited_ch_value_keeps_the_declaring_font() {
    // The root declares `2ch` at 10px; the span inherits the computed length
    // and must not re-measure it with its own 20px font.
    let fixture = block_fixture("letter-spacing:2ch;word-spacing:1ch", |doc, root| {
        doc.append_text(root, "x");
        let inner = span(doc, root, "display:inline;font-size:20px");
        doc.append_text(inner, "y");
    });
    let inner = fixture.doc.nodes[fixture.root].children[1];
    let style = inline_style(&fixture.cascade.computed[inner], inner, &fonts()).expect("map");
    assert_eq!(style.letter_spacing, 20.0);
    assert_eq!(style.word_spacing, 10.0);
}

#[test]
fn a_ch_calc_adds_its_length_term() {
    let style =
        root_style("letter-spacing:calc(1ch + 3px);word-spacing:calc(2ch - 1px)").expect("map");
    assert_eq!(style.letter_spacing, 13.0);
    assert_eq!(style.word_spacing, 19.0);
}

#[test]
fn text_indent_in_ch_is_measured() {
    let fixture = block_fixture("text-indent:3ch", |doc, root| {
        doc.append_text(root, "x");
    });
    let (_, indent) = line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("options");
    assert_eq!(indent, ComputedTextIndent::Px(30.0));
}

#[test]
fn an_inherited_ch_text_indent_keeps_the_declaring_font() {
    let fixture = block_fixture("text-indent:calc(3ch + 1px)", |doc, root| {
        let inner = span(doc, root, "display:block;font-size:20px");
        doc.append_text(inner, "y");
    });
    let inner = fixture.doc.nodes[fixture.root].children[0];
    let (_, indent) =
        line_options(&fixture.cascade.computed[inner], inner, &fonts()).expect("options");
    assert_eq!(indent, ComputedTextIndent::Px(31.0));
}

#[test]
fn a_ch_text_indent_keeps_its_percentage_term() {
    let fixture = block_fixture("text-indent:calc(2ch + 10%)", |doc, root| {
        doc.append_text(root, "x");
    });
    let (_, indent) = line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("options");
    match indent {
        ComputedTextIndent::Calc(calc) => {
            assert_eq!(calc.px, 20.0);
            assert_eq!(calc.percent, 10.0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn inline_edges_in_ch_are_measured() {
    let fixture = block_fixture("padding-left:2ch;margin-right:1ch", |doc, root| {
        doc.append_text(root, "x");
    });
    let edges = inline_edges(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("edges");
    assert_eq!(edges.padding.inline_start, 20.0);
    assert_eq!(edges.margin.inline_end, 10.0);
}

#[test]
fn inline_edges_in_ch_use_the_declaring_element_font() {
    let fixture = block_fixture("", |doc, root| {
        let inner = span(
            doc,
            root,
            "display:inline;font-size:20px;padding-right:1ch;margin-left:2ch",
        );
        doc.append_text(inner, "y");
    });
    let inner = fixture.doc.nodes[fixture.root].children[0];
    let edges = inline_edges(&fixture.cascade.computed[inner], inner, &fonts()).expect("edges");
    assert_eq!(edges.padding.inline_end, 20.0);
    assert_eq!(edges.margin.inline_start, 40.0);
}

#[test]
fn white_space_pre_maps_to_preserve_and_no_wrap() {
    let style = root_style("white-space:pre").expect("map");
    assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Preserve);
    assert_eq!(style.text_wrap_mode, TextWrapMode::NoWrap);
}

#[test]
fn a_later_longhand_wins_over_the_legacy_white_space_keyword() {
    let style = root_style("white-space:pre;white-space-collapse:collapse").expect("map");
    assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Collapse);
    assert_eq!(style.text_wrap_mode, TextWrapMode::NoWrap);
}

#[test]
fn a_later_legacy_keyword_wins_over_an_earlier_longhand() {
    let style = root_style("white-space-collapse:preserve;white-space:nowrap").expect("map");
    assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Collapse);
    assert_eq!(style.text_wrap_mode, TextWrapMode::NoWrap);
}

#[test]
fn white_space_collapse_discard_collapses_white_space() {
    // shodo has no `discard`; its white space is collapsed.
    let style = root_style("white-space-collapse:discard").expect("discard");
    assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Collapse);
}

#[test]
fn text_align_and_last_map() {
    let fixture = block_fixture("text-align:center;text-align-last:justify", |doc, root| {
        doc.append_text(root, "x");
    });
    let (options, _) = line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("options");
    assert_eq!(options.text_align, TextAlign::Center);
    assert_eq!(options.text_align_last, TextAlignLast::Justify);
}

#[test]
fn rtl_and_plaintext_bidi_set_the_paragraph_style() {
    let fixture = block_fixture("direction:rtl;unicode-bidi:plaintext", |doc, root| {
        doc.append_text(root, "x");
    });
    let cv = &fixture.cascade.computed[fixture.root];
    let root = inline_style(cv, fixture.root, &fonts()).expect("root style");
    let paragraph = paragraph_style(cv, fixture.root, root).expect("paragraph");
    assert_eq!(paragraph.direction, Direction::Rtl);
    assert!(paragraph.unicode_bidi_plaintext);
}

#[test]
fn a_vertical_writing_mode_is_laid_out_as_horizontal() {
    // The parley path lays vertical text out horizontally; so does the
    // projection, whatever the authored writing mode.
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        let fixture = block_fixture(&format!("writing-mode:{mode}"), |doc, root| {
            doc.append_text(root, "x");
        });
        let cv = &fixture.cascade.computed[fixture.root];
        let root = inline_style(cv, fixture.root, &fonts()).expect("root style");
        let paragraph = paragraph_style(cv, fixture.root, root).expect("paragraph");
        assert_eq!(paragraph.writing_mode, WritingMode::HorizontalTb, "{mode}");
    }
}

#[test]
fn text_orientation_and_combine_upright_map() {
    let style = root_style("text-orientation:upright;text-combine-upright:all").expect("map");
    assert_eq!(style.text_orientation, TextOrientation::Upright);
    assert_eq!(style.text_combine_upright, TextCombineUpright::All);
}

#[test]
fn autospace_auto_and_custom_sets_map_to_normal_or_none() {
    use shodo::style::TextAutospace;
    for (css, expected) in [
        ("auto", TextAutospace::Normal),
        ("normal", TextAutospace::Normal),
        ("no-autospace", TextAutospace::NoAutospace),
        ("ideograph-alpha", TextAutospace::Normal),
        ("ideograph-numeric", TextAutospace::Normal),
        ("punctuation", TextAutospace::NoAutospace),
    ] {
        let style = root_style(&format!("text-autospace:{css}")).expect(css);
        assert_eq!(style.text_autospace, expected, "{css}");
    }
}

#[test]
fn inline_edges_swap_sides_for_rtl() {
    let edges = |direction: &str| {
        let fixture = block_fixture(
            &format!("direction:{direction};margin-left:1px;margin-right:2px;padding-top:3px"),
            |doc, root| {
                doc.append_text(root, "x");
            },
        );
        inline_edges(
            &fixture.cascade.computed[fixture.root],
            fixture.root,
            &fonts(),
        )
        .expect("edges")
    };
    let ltr = edges("ltr");
    assert_eq!((ltr.margin.inline_start, ltr.margin.inline_end), (1.0, 2.0));
    assert_eq!(ltr.padding.block_start, 3.0);
    let rtl = edges("rtl");
    assert_eq!((rtl.margin.inline_start, rtl.margin.inline_end), (2.0, 1.0));
}

#[test]
fn vertical_mappers_are_one_to_one_and_fail_closed() {
    use raikiri_style::property as p;
    assert_eq!(
        map_text_orientation(p::TextOrientation::Sideways),
        Ok(TextOrientation::Sideways)
    );
    assert_eq!(
        map_text_combine_upright(p::TextCombineUpright::None),
        Ok(TextCombineUpright::None)
    );
}

#[test]
fn word_break_manual_and_every_spacing_trim_value_map() {
    use shodo::style::{TextSpacingTrim, WordBreak};
    assert_eq!(
        root_style("word-break:manual").expect("manual").word_break,
        WordBreak::Manual
    );
    for (css, expected) in [
        ("normal", TextSpacingTrim::Normal),
        ("space-all", TextSpacingTrim::SpaceAll),
        ("trim-start", TextSpacingTrim::TrimStart),
        ("space-first", TextSpacingTrim::SpaceFirst),
        ("trim-both", TextSpacingTrim::TrimBoth),
        ("trim-all", TextSpacingTrim::TrimAll),
        ("auto", TextSpacingTrim::Auto),
    ] {
        let style = root_style(&format!("text-spacing-trim:{css}")).expect(css);
        assert_eq!(style.text_spacing_trim, expected, "{css}");
    }
}

#[test]
fn inline_edges_of_a_vertical_element_are_mapped_as_horizontal() {
    // Vertical text is laid out horizontally, so its edges keep the
    // horizontal-tb mapping: left is inline-start, top is block-start.
    let fixture = block_fixture(
        "writing-mode:vertical-rl;padding-top:3px;margin-left:1px",
        |doc, root| {
            doc.append_text(root, "x");
        },
    );
    let edges = inline_edges(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("vertical");
    assert_eq!(
        (edges.padding.block_start, edges.margin.inline_start),
        (3.0, 1.0)
    );
}

#[test]
fn percentage_and_calc_edges_and_spacing_are_taken_as_their_absolute_part() {
    let fixture = block_fixture(
        "padding-left:10%;margin-right:calc(5% + 2px);letter-spacing:10%",
        |doc, root| {
            doc.append_text(root, "x");
        },
    );
    let cv = &fixture.cascade.computed[fixture.root];
    let edges = inline_edges(cv, fixture.root, &fonts()).expect("edges");
    assert_eq!(
        (edges.padding.inline_start, edges.margin.inline_end),
        (0.0, 0.0)
    );
    let style = inline_style(cv, fixture.root, &fonts()).expect("style");
    assert_eq!(style.letter_spacing, cv.letter_spacing.px());
}

#[test]
fn a_vertical_align_percentage_resolves_against_the_line_height() {
    // CSS 2.1 10.8.1: 50% of a 20px line height raises the box by 10px.
    let style = root_style("line-height:20px;vertical-align:50%").expect("map");
    assert_eq!(
        style.vertical_align,
        shodo::style::VerticalAlign::Length(10.0)
    );
    let style = root_style("line-height:20px;vertical-align:calc(50% + 1px)").expect("map");
    assert_eq!(
        style.vertical_align,
        shodo::style::VerticalAlign::Length(11.0)
    );
}

#[test]
fn generic_families_shodo_does_not_name_take_the_nearest_one() {
    use shodo::style::{FontFamily, GenericFamily};
    for (css, expected) in [
        ("ui-serif", GenericFamily::Serif),
        ("ui-sans-serif", GenericFamily::SansSerif),
        ("ui-monospace", GenericFamily::Monospace),
        ("ui-rounded", GenericFamily::SansSerif),
        ("math", GenericFamily::Serif),
        ("emoji", GenericFamily::SansSerif),
        ("fangsong", GenericFamily::Serif),
    ] {
        let style = root_style(&format!("font-family:{css}")).expect(css);
        assert_eq!(
            style.font_families,
            [FontFamily::Generic(expected)],
            "{css}"
        );
    }
}

#[test]
fn the_wrap_longhand_after_a_legacy_keyword_changes_only_the_wrap() {
    let style = root_style("white-space:pre-wrap;text-wrap-mode:nowrap").expect("map");
    assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Preserve);
    assert_eq!(style.text_wrap_mode, TextWrapMode::NoWrap);
}

#[test]
fn word_break_break_word_is_normal_with_break_word_wrapping() {
    // The parley path turns `word-break: break-word` into normal breaking plus
    // `overflow-wrap: break-word` whatever `overflow-wrap` says
    // (`parley_overflow_wrap` in `inline_text.rs`), so that is what is mapped:
    // off and on then break and size the same way.
    for extra in ["", ";overflow-wrap:anywhere"] {
        let css = format!("word-break:break-word;overflow-wrap:normal{extra}");
        let style = root_style(&css).expect("map");
        assert_eq!(style.word_break, s::WordBreak::Normal, "{css}");
        assert_eq!(style.overflow_wrap, s::OverflowWrap::BreakWord, "{css}");
    }
}

#[test]
fn hanging_punctuation_first_maps_to_the_first_flag() {
    let fixture = block_fixture("hanging-punctuation:first", |doc, root| {
        doc.append_text(root, "x");
    });
    let (options, _) = line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("options");
    assert_eq!(
        options.hanging_punctuation,
        s::HangingPunctuation {
            first: true,
            ..s::HangingPunctuation::default()
        }
    );
}

#[test]
fn no_hanging_punctuation_leaves_the_flags_clear() {
    let fixture = block_fixture("", |doc, root| {
        doc.append_text(root, "x");
    });
    let (options, _) = line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("options");
    assert_eq!(
        options.hanging_punctuation,
        s::HangingPunctuation::default()
    );
}

#[test]
fn a_relative_offset_in_lines_keeps_lengths_and_degrades_the_rest() {
    let offset = |css: &str| {
        let fixture = block_fixture(&format!("position:relative;{css}"), |doc, root| {
            doc.append_text(root, "x");
        });
        relative_offset_in_lines(&fixture.cascade.computed[fixture.root])
    };
    assert_eq!(offset("left:2px;top:3px"), (2.0, 3.0));
    assert_eq!(offset("right:2px;bottom:3px"), (-2.0, -3.0));
    // A z-index does not drop the offset; the stacking context is not drawn.
    assert_eq!(offset("left:2px;z-index:1"), (2.0, 0.0));
    // An inset that needs the containing block is taken as zero.
    assert_eq!(offset("left:calc(1% + 1px);top:4px"), (0.0, 4.0));
}

#[test]
fn values_shodo_does_not_name_map_to_their_equivalents() {
    use shodo::style::{TextAlignLast, TextJustify, TextTransform};
    assert_eq!(
        root_style("text-transform:math-auto")
            .expect("math-auto")
            .text_transform,
        TextTransform::None
    );
    let fixture = block_fixture(
        "text-align-last:match-parent;text-justify:distribute",
        |doc, root| {
            doc.append_text(root, "x");
        },
    );
    let (options, _) = line_options(
        &fixture.cascade.computed[fixture.root],
        fixture.root,
        &fonts(),
    )
    .expect("options");
    assert_eq!(options.text_align_last, TextAlignLast::Auto);
    assert_eq!(options.text_justify, TextJustify::InterCharacter);
}
