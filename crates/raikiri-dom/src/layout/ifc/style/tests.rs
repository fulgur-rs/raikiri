use super::*;
use crate::layout::ifc::test_support::block_fixture;
use shodo::geometry::{Direction, WritingMode};
use shodo::style::{
    FontFamily, GenericFamily, LineHeight, TextAlign, TextAlignLast, TextCombineUpright,
    TextOrientation, TextWrapMode, WhiteSpaceCollapse,
};

fn root_style(extra: &str) -> Result<InlineStyle, IfcError> {
    let fixture = block_fixture(extra, |doc, root| {
        doc.append_text(root, "x");
    });
    inline_style(&fixture.cascade.computed[fixture.root], fixture.root)
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
fn ch_spacing_is_rejected_until_it_can_be_measured() {
    let error = root_style("letter-spacing:1ch").expect_err("ch");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
}

#[test]
fn white_space_pre_maps_to_preserve_and_no_wrap() {
    let style = root_style("white-space:pre").expect("map");
    assert_eq!(style.white_space_collapse, WhiteSpaceCollapse::Preserve);
    assert_eq!(style.text_wrap_mode, TextWrapMode::NoWrap);
}

#[test]
fn legacy_white_space_with_a_conflicting_longhand_is_rejected() {
    let error = root_style("white-space:nowrap;white-space-collapse:preserve").expect_err("mix");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
}

#[test]
fn text_align_and_last_map() {
    let fixture = block_fixture("text-align:center;text-align-last:justify", |doc, root| {
        doc.append_text(root, "x");
    });
    let (options, _) =
        line_options(&fixture.cascade.computed[fixture.root], fixture.root).expect("options");
    assert_eq!(options.text_align, TextAlign::Center);
    assert_eq!(options.text_align_last, TextAlignLast::Justify);
}

#[test]
fn rtl_and_plaintext_bidi_set_the_paragraph_style() {
    let fixture = block_fixture("direction:rtl;unicode-bidi:plaintext", |doc, root| {
        doc.append_text(root, "x");
    });
    let cv = &fixture.cascade.computed[fixture.root];
    let root = inline_style(cv, fixture.root).expect("root style");
    let paragraph = paragraph_style(cv, fixture.root, root).expect("paragraph");
    assert_eq!(paragraph.direction, Direction::Rtl);
    assert!(paragraph.unicode_bidi_plaintext);
}

#[test]
fn vertical_writing_mode_is_read_from_the_cssom_value() {
    let fixture = block_fixture("writing-mode:vertical-rl", |doc, root| {
        doc.append_text(root, "x");
    });
    let cv = &fixture.cascade.computed[fixture.root];
    let root = inline_style(cv, fixture.root).expect("root style");
    let paragraph = paragraph_style(cv, fixture.root, root).expect("paragraph");
    assert_eq!(paragraph.writing_mode, WritingMode::VerticalRl);
}

#[test]
fn text_orientation_and_combine_upright_map() {
    let style = root_style("text-orientation:upright;text-combine-upright:all").expect("map");
    assert_eq!(style.text_orientation, TextOrientation::Upright);
    assert_eq!(style.text_combine_upright, TextCombineUpright::All);
}

#[test]
fn autospace_auto_is_rejected_instead_of_treated_as_normal() {
    let error = root_style("text-autospace:auto").expect_err("auto");
    assert!(matches!(error, IfcError::Unsupported { .. }), "{error}");
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
        inline_edges(&fixture.cascade.computed[fixture.root], fixture.root).expect("edges")
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
    for (from, to) in [
        (p::WritingMode::HorizontalTb, WritingMode::HorizontalTb),
        (p::WritingMode::VerticalRl, WritingMode::VerticalRl),
        (p::WritingMode::VerticalLr, WritingMode::VerticalLr),
        (p::WritingMode::SidewaysRl, WritingMode::SidewaysRl),
        (p::WritingMode::SidewaysLr, WritingMode::SidewaysLr),
    ] {
        assert_eq!(map_writing_mode(from), Ok(to));
    }
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
