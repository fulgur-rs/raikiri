use super::*;
use crate::layout::ifc::test_support::{block_fixture, span};
use shodo::geometry::{Direction, WritingMode};
use shodo::style::{
    FontFamily, GenericFamily, LineHeight, TextAlign, TextAlignLast, TextCombineUpright,
    TextOrientation, TextWrapMode, WhiteSpaceCollapse, WordSpaceTransform,
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
fn font_feature_settings_map_to_shodo_and_keep_computed_order() {
    let style = root_style(r#"font-feature-settings: "kern" 1, "liga" 0, "kern" 0, "KERN" 2"#)
        .expect("map");
    assert_eq!(
        style.font_features,
        vec![
            shodo::style::FontFeature {
                tag: *b"KERN",
                value: 2,
            },
            shodo::style::FontFeature {
                tag: *b"kern",
                value: 0,
            },
            shodo::style::FontFeature {
                tag: *b"liga",
                value: 0,
            },
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
fn word_space_transform_maps_every_computed_value() {
    for (value, expected) in [
        ("none", WordSpaceTransform::None),
        ("space", WordSpaceTransform::Space),
        ("ideographic-space", WordSpaceTransform::IdeographicSpace),
        ("space auto-phrase", WordSpaceTransform::SpaceAutoPhrase),
        (
            "ideographic-space auto-phrase",
            WordSpaceTransform::IdeographicSpaceAutoPhrase,
        ),
    ] {
        let style = root_style(&format!("word-space-transform:{value}")).expect("map");
        assert_eq!(style.word_space_transform, expected, "{value}");
    }
}

#[test]
fn word_space_transform_is_inherited_and_child_values_override_it() {
    let fixture = block_fixture("word-space-transform:space", |doc, root| {
        let inherited = span(doc, root, "display:inline");
        doc.append_text(inherited, "a");
        let none = span(doc, root, "display:inline;word-space-transform:none");
        doc.append_text(none, "b");
        let ideographic = span(
            doc,
            root,
            "display:inline;word-space-transform:ideographic-space auto-phrase",
        );
        doc.append_text(ideographic, "c");
    });
    let children = &fixture.doc.nodes[fixture.root].children;

    let inherited = inline_style(
        &fixture.cascade.computed[children[0]],
        children[0],
        &fonts(),
    )
    .expect("map inherited value");
    assert_eq!(inherited.word_space_transform, WordSpaceTransform::Space);

    let none = inline_style(
        &fixture.cascade.computed[children[1]],
        children[1],
        &fonts(),
    )
    .expect("map child override");
    assert_eq!(none.word_space_transform, WordSpaceTransform::None);

    let ideographic = inline_style(
        &fixture.cascade.computed[children[2]],
        children[2],
        &fonts(),
    )
    .expect("map second child override");
    assert_eq!(
        ideographic.word_space_transform,
        WordSpaceTransform::IdeographicSpaceAutoPhrase
    );
}

#[test]
fn word_spacing_percent_and_calc_terms_map_to_shodo() {
    let percent = root_style("font-size:20px;word-spacing:25%").expect("map percentage");
    assert_eq!(percent.word_spacing, 5.0);
    assert_eq!(percent.word_spacing_percent, 0.0);

    let calc = root_style("font-size:20px;word-spacing:calc(6px + 25%)").expect("map calc");
    assert_eq!(calc.word_spacing, 11.0);
    assert_eq!(calc.word_spacing_percent, 0.0);

    let hundred = root_style("font-size:20px;word-spacing:100%").expect("map 100 percent");
    assert_eq!(hundred.word_spacing, 20.0);
    assert_eq!(hundred.word_spacing_percent, 0.0);
}

#[test]
fn word_spacing_ch_calc_percent_maps_to_absolute_shodo_length() {
    let ch_calc = root_style("font-size:20px;word-spacing:calc(2ch - 1px + 25%)")
        .expect("map calc with ch and percentage");
    assert_eq!(ch_calc.word_spacing, 44.0);
    assert_eq!(ch_calc.word_spacing_percent, 0.0);
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
fn supported_vertical_writing_modes_reach_the_paragraph() {
    for (mode, expected) in [
        ("horizontal-tb", WritingMode::HorizontalTb),
        ("vertical-rl", WritingMode::VerticalRl),
        ("vertical-lr", WritingMode::VerticalLr),
    ] {
        let fixture = block_fixture(&format!("writing-mode:{mode}"), |doc, root| {
            doc.append_text(root, "x");
        });
        let cv = &fixture.cascade.computed[fixture.root];
        let root = inline_style(cv, fixture.root, &fonts()).expect("root style");
        let paragraph = paragraph_style(cv, fixture.root, root).expect("paragraph");
        assert_eq!(paragraph.writing_mode, expected, "{mode}");
    }
}

#[test]
fn sideways_writing_modes_keep_cssom_values_with_horizontal_layout_fallback() {
    use raikiri_style::property as p;

    for (mode, computed) in [
        ("sideways-rl", p::WritingMode::SidewaysRl),
        ("sideways-lr", p::WritingMode::SidewaysLr),
    ] {
        let fixture = block_fixture(
            &format!("writing-mode:{mode};padding-left:2px;padding-right:3px"),
            |doc, root| {
                doc.append_text(root, "x");
            },
        );
        let cv = &fixture.cascade.computed[fixture.root];
        assert_eq!(cv.cssom_writing_mode, computed, "{mode}");
        let root = inline_style(cv, fixture.root, &fonts()).expect("root style");
        let paragraph = paragraph_style(cv, fixture.root, root).expect("layout fallback");
        assert_eq!(paragraph.writing_mode, WritingMode::HorizontalTb, "{mode}");
        let edges = inline_edges(cv, fixture.root, &fonts()).expect("edge fallback");
        assert_eq!(
            (edges.padding.inline_start, edges.padding.inline_end),
            (2.0, 3.0)
        );
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
fn inline_edges_follow_vertical_writing_mode_and_direction() {
    use shodo::node::Sides;

    for (css, expected_margin, expected_padding, expected_border) in [
        (
            "writing-mode:vertical-rl;direction:ltr",
            Sides {
                inline_start: 1.0,
                inline_end: 3.0,
                block_start: 2.0,
                block_end: 4.0,
            },
            Sides {
                inline_start: 5.0,
                inline_end: 7.0,
                block_start: 6.0,
                block_end: 8.0,
            },
            Sides {
                inline_start: 9.0,
                inline_end: 11.0,
                block_start: 10.0,
                block_end: 12.0,
            },
        ),
        (
            "writing-mode:vertical-lr;direction:rtl",
            Sides {
                inline_start: 3.0,
                inline_end: 1.0,
                block_start: 4.0,
                block_end: 2.0,
            },
            Sides {
                inline_start: 7.0,
                inline_end: 5.0,
                block_start: 8.0,
                block_end: 6.0,
            },
            Sides {
                inline_start: 11.0,
                inline_end: 9.0,
                block_start: 12.0,
                block_end: 10.0,
            },
        ),
    ] {
        let fixture = block_fixture(
            &format!(
                "{css};margin:1px 2px 3px 4px;padding:5px 6px 7px 8px;border-style:solid;border-width:9px 10px 11px 12px"
            ),
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
        assert_eq!(edges.margin, expected_margin, "{css}");
        assert_eq!(edges.padding, expected_padding, "{css}");
        assert_eq!(edges.border, expected_border, "{css}");
    }
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
fn word_break_break_word_preserves_its_legacy_keyword_semantics() {
    for (css, expected_wrap) in [
        (
            "word-break:break-word;overflow-wrap:normal",
            s::OverflowWrap::Normal,
        ),
        (
            "word-break:break-word;overflow-wrap:anywhere",
            s::OverflowWrap::Anywhere,
        ),
    ] {
        let style = root_style(css).expect("map");
        assert_eq!(style.word_break, s::WordBreak::BreakWord, "{css}");
        assert_eq!(style.overflow_wrap, expected_wrap, "{css}");
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
fn hanging_punctuation_last_maps_to_the_last_flag() {
    let fixture = block_fixture("hanging-punctuation:last", |doc, root| {
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
            last: true,
            ..s::HangingPunctuation::default()
        }
    );
}

#[test]
fn every_inline_style_carries_its_own_hanging_punctuation() {
    // An inline box's value replaces the paragraph's, so `none` is explicit.
    assert_eq!(
        root_style("").expect("map").hanging_punctuation,
        Some(s::HangingPunctuation::default())
    );
    assert_eq!(
        root_style("hanging-punctuation:first")
            .expect("map")
            .hanging_punctuation,
        Some(s::HangingPunctuation {
            first: true,
            ..s::HangingPunctuation::default()
        })
    );
    assert_eq!(
        root_style("hanging-punctuation:last")
            .expect("map")
            .hanging_punctuation,
        Some(s::HangingPunctuation {
            last: true,
            ..s::HangingPunctuation::default()
        })
    );
}

#[test]
fn hanging_punctuation_full_grammar_maps_all_root_and_inline_flags() {
    for (css, (first, last, force_end, allow_end)) in [
        ("none", (false, false, false, false)),
        ("first", (true, false, false, false)),
        ("last", (false, true, false, false)),
        ("force-end", (false, false, true, false)),
        ("allow-end", (false, false, false, true)),
        ("first last", (true, true, false, false)),
        ("first force-end", (true, false, true, false)),
        ("first allow-end", (true, false, false, true)),
        ("force-end last", (false, true, true, false)),
        ("allow-end last", (false, true, false, true)),
        ("first force-end last", (true, true, true, false)),
        ("first allow-end last", (true, true, false, true)),
    ] {
        let expected = s::HangingPunctuation {
            first,
            last,
            force_end,
            allow_end,
        };
        let fixture = block_fixture(&format!("hanging-punctuation:{css}"), |doc, root| {
            doc.append_text(root, "x");
        });
        let computed = &fixture.cascade.computed[fixture.root];
        let options = line_options(computed, fixture.root, &fonts())
            .expect("root options")
            .0;
        let inline = inline_style(computed, fixture.root, &fonts()).expect("inline style");
        assert_eq!(options.hanging_punctuation, expected, "root {css}");
        assert_eq!(inline.hanging_punctuation, Some(expected), "inline {css}");
    }
}

#[test]
fn hanging_punctuation_combinations_inherit_and_inline_none_clears_root_flags() {
    let fixture = block_fixture("hanging-punctuation:first force-end last", |doc, root| {
        for css in [
            "display:inline",
            "display:inline;hanging-punctuation:none",
            "display:inline;hanging-punctuation:first allow-end",
        ] {
            let child = span(doc, root, css);
            doc.append_text(child, "x");
        }
    });
    let children = &fixture.doc.nodes[fixture.root].children;
    for (&node, expected) in children.iter().zip([
        s::HangingPunctuation {
            first: true,
            last: true,
            force_end: true,
            allow_end: false,
        },
        s::HangingPunctuation::default(),
        s::HangingPunctuation {
            first: true,
            last: false,
            force_end: false,
            allow_end: true,
        },
    ]) {
        let mapped = inline_style(&fixture.cascade.computed[node], node, &fonts()).expect("map");
        assert_eq!(mapped.hanging_punctuation, Some(expected));
    }
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

#[test]
fn text_emphasis_maps_shapes_fill_and_position() {
    use shodo::style::{TextEmphasisPosition as P, TextEmphasisShape as S};
    let emphasis = |css: &str| root_style(css).expect("map").text_emphasis;
    assert_eq!(emphasis(""), None);
    assert_eq!(emphasis("text-emphasis-style:none"), None);
    let dot = emphasis("text-emphasis-style:dot").expect("dot");
    assert_eq!(
        (dot.shape, dot.filled, dot.position),
        (S::Dot, true, P::OverRight)
    );
    let open = emphasis("text-emphasis-style:open triangle;text-emphasis-position:under left")
        .expect("triangle");
    assert_eq!(
        (open.shape, open.filled, open.position),
        (S::Triangle, false, P::UnderLeft)
    );
    for (css, shape) in [
        ("circle", S::Circle),
        ("double-circle", S::DoubleCircle),
        ("sesame", S::Sesame),
    ] {
        let mapped = emphasis(&format!("text-emphasis-style:{css}")).expect("shape");
        assert_eq!(mapped.shape, shape);
    }
    let over_left =
        emphasis("text-emphasis-style:dot;text-emphasis-position:over left").expect("over left");
    assert_eq!(over_left.position, P::OverLeft);
    let under =
        emphasis("text-emphasis-style:dot;text-emphasis-position:under right").expect("under");
    assert_eq!(under.position, P::UnderRight);
    // A fill alone is a circle in horizontal and a sesame in vertical text.
    let open_default = emphasis("text-emphasis-style:open").expect("open");
    assert_eq!(
        (open_default.shape, open_default.filled),
        (S::Circle, false)
    );
    let vertical =
        emphasis("text-emphasis-style:filled;writing-mode:vertical-rl").expect("vertical");
    assert_eq!(vertical.shape, S::Sesame);
    // A string mark is its first character.
    let string = emphasis("text-emphasis-style:'xy'").expect("string");
    assert_eq!((string.shape, string.filled), (S::Custom('x'), true));
    // `auto` stays `over right` without a content language.
    let auto = emphasis("text-emphasis-style:dot;text-emphasis-position:auto").expect("auto");
    assert_eq!(auto.position, P::OverRight);
}

#[test]
fn auto_emphasis_position_is_under_for_horizontal_chinese_only() {
    use shodo::style::TextEmphasisPosition as P;
    let resolve = |css: &str, lang: Option<&str>| {
        let fixture = block_fixture(css, |doc, root| {
            doc.append_text(root, "x");
        });
        let cv = &fixture.cascade.computed[fixture.root];
        let mut style = inline_style(cv, fixture.root, &fonts()).expect("map");
        style.lang = lang.map(str::to_owned);
        resolve_auto_emphasis_position(cv, &mut style);
        style.text_emphasis.map(|emphasis| emphasis.position)
    };
    let auto = "text-emphasis-style:dot;text-emphasis-position:auto";
    assert_eq!(resolve(auto, Some("zh")), Some(P::UnderRight));
    assert_eq!(resolve(auto, Some("zh-hant")), Some(P::UnderRight));
    assert_eq!(resolve(auto, Some("ja")), Some(P::OverRight));
    assert_eq!(resolve(auto, Some("zhx")), Some(P::OverRight));
    assert_eq!(resolve(auto, None), Some(P::OverRight));
    let vertical = format!("{auto};writing-mode:vertical-rl");
    assert_eq!(resolve(&vertical, Some("zh")), Some(P::OverRight));
    // An explicit position is kept.
    let explicit = "text-emphasis-style:dot;text-emphasis-position:over right";
    assert_eq!(resolve(explicit, Some("zh")), Some(P::OverRight));
    // No marks, nothing to resolve.
    assert_eq!(resolve("text-emphasis-position:auto", Some("zh")), None);
}

#[test]
fn ellipsis_needs_a_non_visible_inline_overflow() {
    let ends = |css: &str| {
        let fixture = block_fixture(css, |doc, root| {
            doc.append_text(root, "x");
        });
        ends_in_ellipsis(&fixture.cascade.computed[fixture.root])
    };
    assert!(ends("text-overflow:ellipsis;overflow:hidden"));
    assert!(ends("text-overflow:ellipsis;overflow-x:clip"));
    assert!(!ends("text-overflow:ellipsis"));
    assert!(!ends("text-overflow:clip;overflow:hidden"));
    assert!(!ends("overflow:hidden"));
}
