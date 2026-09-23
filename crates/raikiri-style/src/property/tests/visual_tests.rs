//! Tests for the visual-effect property parsers in `parse/visual.rs`.

use super::*;

// ── visibility (CSS Display 3 §4) ──
//
// Value grammar (spec verbatim): `visible | hidden | collapse`. This
// crate implements all 3 keywords (`Visibility` doc's "Scope carving"
// section — `collapse`'s formatting-context-specific space-saving effect
// is unimplemented, but the keyword itself is fully accepted). Initial:
// visible / Inherited: yes / Computed value: as specified.

#[test]
fn visibility_parse_all_keywords() {
    assert_eq!(
        parse("visible", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Visible))
    );
    assert_eq!(
        parse("hidden", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Hidden))
    );
    assert_eq!(
        parse("collapse", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Collapse))
    );
}

#[test]
fn visibility_is_case_insensitive() {
    assert_eq!(
        parse("VISIBLE", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Visible))
    );
    assert_eq!(
        parse("Hidden", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Hidden))
    );
    assert_eq!(
        parse("Collapse", "visibility"),
        Some(PropertyValue::Visibility(Visibility::Collapse))
    );
}

#[test]
fn visibility_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "visibility"), None);
}

#[test]
fn visibility_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "visibility"), None);
    }
}

#[test]
fn visibility_rejects_non_ident() {
    assert_eq!(parse("16px", "visibility"), None);
    assert_eq!(parse(r#""hidden""#, "visibility"), None);
}

#[test]
fn visibility_key_maps_to_visibility_property_key() {
    let v = PropertyValue::Visibility(Visibility::Visible);
    assert_eq!(v.key(), PropertyKey::Visibility);
    let v = PropertyValue::Visibility(Visibility::Hidden);
    assert_eq!(v.key(), PropertyKey::Visibility);
    let v = PropertyValue::Visibility(Visibility::Collapse);
    assert_eq!(v.key(), PropertyKey::Visibility);
}

// ── parse_url_value helper ────────────────────
//
// CSS Values 4 §4.4 <url> value type
// (<https://www.w3.org/TR/css-values-4/#urls>) の共通 helper を property
// dispatcher (`parse_value`) を経由せず直接叩く — `background-image` の
// 呼び出し経路とは独立に helper 自体の grammar 境界 (unquoted/quoted
// form、`<url-modifier>` reject 等) を check する
// (`parse_length_value` helper 単体 test と同じ fixture pattern)。

fn parse_url(source: &str) -> Option<String> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_url_value(&mut parser)
}

#[test]
fn url_value_accepts_unquoted_form() {
    // unquoted `<url-token>` form — see `parse_url_value` doc.
    assert_eq!(parse_url("url(foo.png)"), Some("foo.png".to_string()));
}

#[test]
fn url_value_accepts_quoted_form() {
    // quoted `url( <string> )` function-token form — see
    // `parse_url_value` doc.
    assert_eq!(parse_url("url(\"foo.png\")"), Some("foo.png".to_string()));
}

#[test]
fn url_value_accepts_quoted_form_with_single_quotes() {
    // CSS Syntax 3 `<string-token>` は `"` `'` どちらの quote 文字も
    // 受理する — `url()` 内の `<string>` も同様。
    assert_eq!(parse_url("url('foo.png')"), Some("foo.png".to_string()));
}

#[test]
fn url_value_rejects_css_wide_keywords() {
    // CSS-wide keyword は `<url>` grammar のどの alternative にも
    // 一致しない — see `parse_url_value` doc.
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse_url(keyword), None, "{keyword}");
    }
}

#[test]
fn url_value_rejects_bare_string_without_url_wrapper() {
    // bare `<string>` (no `url()` wrapper) — @import-only legacy
    // allowance, not part of the general `<url>` value type. See
    // `parse_url_value` doc.
    assert_eq!(parse_url("\"foo.png\""), None);
}

#[test]
fn url_value_rejects_non_url_ident() {
    assert_eq!(parse_url("foo"), None);
}

#[test]
fn url_value_rejects_empty_input() {
    assert_eq!(parse_url(""), None);
}

#[test]
fn url_value_rejects_number() {
    assert_eq!(parse_url("42"), None);
}

#[test]
fn url_value_rejects_url_with_modifier() {
    // `<url-modifier>` (`crossorigin()` 等) 付き `url()` — unsupported,
    // see `parse_url_value` doc for the block-exhaustion mechanism.
    assert_eq!(parse_url("url(\"foo.png\" crossorigin)"), None);
}

// ── background-repeat (CSS Backgrounds 3 §2.4) ──

#[test]
fn background_repeat_parse_single_keyword_applies_to_both_axes() {
    for (source, keyword) in [
        ("repeat", BackgroundRepeatKeyword::Repeat),
        ("space", BackgroundRepeatKeyword::Space),
        ("round", BackgroundRepeatKeyword::Round),
        ("no-repeat", BackgroundRepeatKeyword::NoRepeat),
    ] {
        // cov:ignore: the failure-message branch of this `assert_eq!`
        // only executes when the assertion fails; it passes here, so
        // llvm-cov reports the macro's condition-false region as an
        // uncovered added line even though the assertion itself runs
        // and does its job.
        assert_eq!(
            parse(source, "background-repeat"),
            Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
                x: keyword,
                y: keyword,
            })),
            "{source}"
        );
    }
}

#[test]
fn background_repeat_parse_repeat_x_and_repeat_y() {
    assert_eq!(
        parse("repeat-x", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }))
    );
    assert_eq!(
        parse("repeat-y", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::Repeat,
        }))
    );
}

#[test]
fn background_repeat_parse_two_keyword_form() {
    assert_eq!(
        parse("space round", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Space,
            y: BackgroundRepeatKeyword::Round,
        }))
    );
}

#[test]
fn background_repeat_is_case_insensitive() {
    assert_eq!(
        parse("REPEAT-X", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }))
    );
    assert_eq!(
        parse("No-Repeat", "background-repeat"),
        Some(PropertyValue::BackgroundRepeat(BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }))
    );
}

#[test]
fn background_repeat_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-repeat"), None, "{keyword}");
    }
}

#[test]
fn background_repeat_rejects_unknown_keyword() {
    assert_eq!(parse("stretch", "background-repeat"), None);
    assert_eq!(parse("16px", "background-repeat"), None);
}

#[test]
fn background_repeat_key_maps_to_background_repeat_property_key() {
    let v = PropertyValue::BackgroundRepeat(BackgroundRepeat {
        x: BackgroundRepeatKeyword::Repeat,
        y: BackgroundRepeatKeyword::Repeat,
    });
    assert_eq!(v.key(), PropertyKey::BackgroundRepeat);
}

// ── background-attachment (CSS Backgrounds 3 §2.5) ──

#[test]
fn background_attachment_parse_all_three_keywords() {
    assert_eq!(
        parse("scroll", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Scroll
        ))
    );
    assert_eq!(
        parse("fixed", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Fixed
        ))
    );
    assert_eq!(
        parse("local", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Local
        ))
    );
}

#[test]
fn background_attachment_is_case_insensitive() {
    assert_eq!(
        parse("FIXED", "background-attachment"),
        Some(PropertyValue::BackgroundAttachment(
            BackgroundAttachment::Fixed
        ))
    );
}

#[test]
fn background_attachment_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-attachment"), None, "{keyword}");
    }
}

#[test]
fn background_attachment_rejects_unknown_keyword() {
    assert_eq!(parse("static", "background-attachment"), None);
}

#[test]
fn background_attachment_key_maps_to_background_attachment_property_key() {
    let v = PropertyValue::BackgroundAttachment(BackgroundAttachment::Scroll);
    assert_eq!(v.key(), PropertyKey::BackgroundAttachment);
}

// ── background-clip (CSS Backgrounds 3 §2.7) / background-origin (§2.8) ──

#[test]
fn background_clip_parse_all_three_keywords() {
    assert_eq!(
        parse("border-box", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::BorderBox))
    );
    assert_eq!(
        parse("padding-box", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::PaddingBox))
    );
    assert_eq!(
        parse("content-box", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::ContentBox))
    );
}

#[test]
fn background_origin_parse_all_three_keywords() {
    assert_eq!(
        parse("border-box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::BorderBox))
    );
    assert_eq!(
        parse("padding-box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::PaddingBox))
    );
    assert_eq!(
        parse("content-box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::ContentBox))
    );
}

#[test]
fn background_clip_and_origin_are_case_insensitive() {
    assert_eq!(
        parse("BORDER-BOX", "background-clip"),
        Some(PropertyValue::BackgroundClip(VisualBox::BorderBox))
    );
    assert_eq!(
        parse("Padding-Box", "background-origin"),
        Some(PropertyValue::BackgroundOrigin(VisualBox::PaddingBox))
    );
}

#[test]
fn background_clip_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-clip"), None, "{keyword}");
    }
}

#[test]
fn background_origin_rejects_unknown_keyword() {
    assert_eq!(parse("fill-box", "background-origin"), None);
    assert_eq!(parse("16px", "background-origin"), None);
}

#[test]
fn background_clip_key_maps_to_background_clip_property_key() {
    let v = PropertyValue::BackgroundClip(VisualBox::BorderBox);
    assert_eq!(v.key(), PropertyKey::BackgroundClip);
}

#[test]
fn background_origin_key_maps_to_background_origin_property_key() {
    let v = PropertyValue::BackgroundOrigin(VisualBox::PaddingBox);
    assert_eq!(v.key(), PropertyKey::BackgroundOrigin);
}

// ── background-size (CSS Backgrounds 3 §2.9) ──

#[test]
fn background_size_parse_cover_and_contain() {
    assert_eq!(
        parse("cover", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Cover))
    );
    assert_eq!(
        parse("contain", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Contain))
    );
}

#[test]
fn background_size_parse_single_value_fills_auto_for_second_axis() {
    // spec verbatim: "If only one value is given the second is assumed
    // to be auto." — NOT a duplicate of the first value (unlike
    // `border-radius`'s 1-4 value fill rule).
    assert_eq!(
        parse("50%", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Percent(50.0)),
            height: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn background_size_parse_two_values() {
    assert_eq!(
        parse("100px 50%", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Px(100.0)),
            height: LengthOrAuto::Length(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_size_parse_auto_auto() {
    assert_eq!(
        parse("auto auto", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        }))
    );
    // single bare `auto` also fills the second axis with `auto`.
    assert_eq!(
        parse("auto", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn background_size_rejects_negative_length() {
    // `<length-percentage [0,∞]>` — negative values are grammar-invalid.
    assert_eq!(parse("-10px", "background-size"), None);
    // A present-but-invalid 2nd axis is not the same as an *omitted*
    // 2nd axis: `parse_background_size_axis` rejects `-10px` and the
    // wrapping `try_parse` rewinds, so the token survives as leftover
    // for the caller's `expect_exhausted` (`rule.rs`) to drop the whole
    // declaration — `parse` alone (no exhaustion check) would otherwise
    // silently observe only the 1st axis and default the 2nd to `auto`.
    assert_eq!(parse_entire("10px -10px", "background-size"), None);
}

#[test]
fn background_size_rejects_negative_percentage() {
    // Sibling of `background_size_rejects_negative_length` — the
    // `[0,∞]` bound applies to the whole `<length-percentage>`, not
    // just its `<length>` alternative. `Length::payload` extracts the
    // numeric payload uniformly across `Length` variants including
    // `Percent`, so `parse_background_size_axis`'s `>= 0.0` gate
    // covers this case identically.
    assert_eq!(parse("-10%", "background-size"), None);
}

#[test]
fn background_size_accepts_percentage() {
    assert_eq!(
        parse("10%", "background-size"),
        Some(PropertyValue::BackgroundSize(BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Percent(10.0)),
            height: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn background_size_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-size"), None, "{keyword}");
    }
}

#[test]
fn background_size_rejects_unknown_unit() {
    assert_eq!(parse("10vw", "background-size"), None);
}

#[test]
fn background_size_key_maps_to_background_size_property_key() {
    let v = PropertyValue::BackgroundSize(BackgroundSize::Cover);
    assert_eq!(v.key(), PropertyKey::BackgroundSize);
}

// ── background-position / `<position>` (CSS Backgrounds 3 §2.6) ──
//
// `<position>` grammar has 3 overlapping alternatives (`CssPosition`
// doc) — the test names below reference which alternative/branch each
// input exercises so a future regression is easy to localize.

#[test]
fn background_position_parse_single_keyword() {
    // 1st alternative, bare keyword — the other axis defaults to
    // `center`.
    assert_eq!(
        parse("center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("left", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("right", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("top", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
    assert_eq!(
        parse("bottom", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        }))
    );
}

#[test]
fn background_position_parse_single_length_percentage() {
    // 1st alternative, bare `<length-percentage>` — always horizontal,
    // vertical defaults to `center`.
    assert_eq!(
        parse("25%", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(25.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_parse_two_bare_length_percentages() {
    // 2nd alternative: `[left|center|right|<LP>] [top|center|bottom|<LP>]`
    // — strict horizontal-then-vertical order, no reordering.
    assert_eq!(
        parse("10px 20px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Px(20.0)),
        }))
    );
}

#[test]
fn background_position_parse_two_keywords_reordered() {
    // 3rd alternative (`&&`, either order) — `top left` and `left top`
    // must produce the identical result.
    let expected = PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(0.0)),
    });
    assert_eq!(
        parse("left top", "background-position"),
        Some(expected.clone())
    );
    assert_eq!(parse("top left", "background-position"), Some(expected));
}

#[test]
fn background_position_parse_center_with_single_edge_keyword_either_order() {
    // `center` is ambiguous until the other token disambiguates it
    // (`parse_position_branch3` doc) — both orders must agree.
    let expected = PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(50.0)),
    });
    assert_eq!(
        parse("center left", "background-position"),
        Some(expected.clone())
    );
    assert_eq!(parse("left center", "background-position"), Some(expected));
}

#[test]
fn background_position_parse_keyword_then_bare_length_percentage() {
    // "left 10px" — `left` fills the horizontal slot (0%, no attached
    // offset — the 3rd alternative's `left <length-percentage>?`
    // greedily tries to consume `10px` as an offset first, but then
    // has nothing left for the mandatory vertical group and fails as a
    // whole; the 2nd alternative matches instead, treating `10px` as
    // the bare vertical value). See `parse_bg_position` doc.
    assert_eq!(
        parse("left 10px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_right_10px_is_not_an_edge_offset() {
    // The well-known 2-value gotcha: "right 10px" does NOT mean "10px
    // from the right edge" — with only 2 tokens the 3rd alternative
    // (edge-offset form) cannot satisfy its mandatory vertical group,
    // so the 2nd alternative wins: horizontal = `right` (100%),
    // vertical = the bare `10px`. The edge-offset reading requires a
    // 3rd token (see `background_position_parse_edge_offset_three_values`).
    assert_eq!(
        parse("right 10px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_three_values() {
    // 3rd alternative, 3 tokens: an offset attached to one edge, the
    // other axis a bare `center`.
    assert_eq!(
        parse("right 10px center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_four_values() {
    // 3rd alternative, 4 tokens, both axes carrying an explicit offset
    // — matches CSS Backgrounds 3 §2.6's own worked example verbatim
    // ("a 10px upward offset from the bottom and 20px leftward offset
    // from the right edge").
    assert_eq!(
        parse("bottom 10px right 20px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_missing_offset_defaults_to_zero() {
    // 3rd alternative, 3 tokens: the edge with no attached offset
    // defaults to `0` — `right` alone normalizes to `Start(100%)`
    // (`normalize_css_position_offset` collapses a percentage `End`
    // back to `Start`).
    assert_eq!(
        parse("bottom 10px right", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(100.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_accepts_negative_length_offset() {
    // Unlike `background-size`, `<position>`'s `<length-percentage>`
    // has no `[0,∞]` restriction — negative offsets are spec-valid
    // ("outward" offsets, per the propdef's offset-computation prose).
    assert_eq!(
        parse("bottom -10px right -20px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(-20.0)),
            vertical: CssPositionOffset::End(Length::Px(-10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_offset_percentage_normalizes_non_zero() {
    // `normalize_css_position_offset` folds `End(Percent(p))` back to
    // `Start(Percent(100.0 - p))` — the other tests above only exercise
    // this at `p = 0` (`right` alone, via
    // `background_position_parse_edge_offset_missing_offset_defaults_to_zero`).
    // A non-zero `p` proves the subtraction itself, not just the
    // identity case. `right 30%` measures 30% in from the right edge,
    // which is the same physical point as 70% in from the left edge —
    // the two are interchangeable because both `Start` and `End`
    // percentages share the same basis (CSS Backgrounds 3 §2.6:
    // "refer to size of background positioning area minus size of
    // background image"), whatever that basis resolves to, so
    // `100% - 30% = 70%` lands on the correct point without this
    // helper itself needing to know that basis.
    assert_eq!(
        parse("right 30% center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(70.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_parse_branch2_vertical_keywords_with_bare_horizontal_length() {
    // "<LP> top|center|bottom" only reaches the 2nd alternative's
    // vertical keyword arms (`parse_position_branch2_vertical`) when
    // the horizontal side is a bare `<length-percentage>` — every
    // keyword-pair input elsewhere in this file (`"left top"` etc.) is
    // claimed by the 3rd (edge-offset) alternative first, since that
    // one is tried before the 2nd (`parse_bg_position` doc).
    assert_eq!(
        parse("10px top", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
    assert_eq!(
        parse("10px center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
    assert_eq!(
        parse("10px bottom", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        }))
    );
}

#[test]
fn background_position_parse_branch2_horizontal_center_with_bare_vertical_length() {
    // Sibling of the test above, horizontal side of the 2nd
    // alternative: "center <LP>" reaches `parse_position_branch2_horizontal`'s
    // `center` arm — every other `center`-with-keyword input in this
    // file is claimed by the 3rd alternative first.
    assert_eq!(
        parse("center 10px", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
}

#[test]
fn background_position_parse_edge_keyword_then_center() {
    // "top center" — the 3rd alternative's vertical-edge-first branch
    // (`top` matches `parse_position_vertical_edge`) leaves `center`
    // for the mandatory horizontal group, reaching
    // `parse_position_horizontal_group`'s own `center` arm (distinct
    // from the ambiguous-`center`-first branch exercised by
    // `background_position_parse_center_with_single_edge_keyword_either_order`,
    // which starts from `center` rather than ending on it).
    assert_eq!(
        parse("top center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
}

#[test]
fn background_position_parse_center_then_vertical_edge() {
    // "center top" — `parse_position_branch3`'s ambiguous-`center`
    // branch resolves by trying the vertical edge first; distinct from
    // `background_position_parse_center_with_single_edge_keyword_either_order`,
    // which only ever pairs `center` with a *horizontal* edge (`left`).
    assert_eq!(
        parse("center top", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
}

#[test]
fn background_position_parse_center_center() {
    // "center center" — `parse_position_branch3`'s ambiguous-`center`
    // branch falls through both the vertical-edge and horizontal-edge
    // attempts before matching the explicit trailing `center` ident.
    assert_eq!(
        parse("center center", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

#[test]
fn background_position_rejects_two_horizontal_keywords() {
    // "left right" — after `left` fills the horizontal slot, `right`
    // has nowhere to go in any alternative and is left as unconsumed
    // trailing garbage.
    assert_eq!(parse_entire("left right", "background-position"), None);
}

#[test]
fn background_position_rejects_two_vertical_keywords() {
    assert_eq!(parse_entire("top bottom", "background-position"), None);
}

#[test]
fn background_position_rejects_trailing_garbage() {
    // "left center 20px" — 2 tokens fully satisfy the 3rd alternative
    // (`left`, `center`), leaving `20px` as leftover with no
    // preceding edge keyword to attach to.
    assert_eq!(
        parse_entire("left center 20px", "background-position"),
        None
    );
}

#[test]
fn background_position_is_case_insensitive() {
    assert_eq!(
        parse("TOP LEFT", "background-position"),
        Some(PropertyValue::BackgroundPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
}

#[test]
fn background_position_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-position"), None, "{keyword}");
    }
}

#[test]
fn background_position_key_maps_to_background_position_property_key() {
    let v = PropertyValue::BackgroundPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(0.0)),
    });
    assert_eq!(v.key(), PropertyKey::BackgroundPosition);
}

// ── background-image (CSS Backgrounds and Borders 3 §2.3) ──

#[test]
fn background_image_parse_none() {
    assert_eq!(
        parse("none", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::None))
    );
}

#[test]
fn background_image_parse_url_unquoted_form() {
    assert_eq!(
        parse("url(foo.png)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(
            "foo.png".to_string()
        )))
    );
}

#[test]
fn background_image_parse_url_quoted_form() {
    assert_eq!(
        parse("url(\"foo.png\")", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Url(
            "foo.png".to_string()
        )))
    );
}

#[test]
fn background_image_is_case_insensitive() {
    // `none` keyword は ASCII case-insensitive (他の keyword-only property
    // と同じ扱い、`background-repeat` の `REPEAT-X`/`No-Repeat` test 参照)。
    assert_eq!(
        parse("NONE", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::None))
    );
}

// ── background-image: <gradient> (CSS Images 4 §3) ──

const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};

fn gradient_stop(color: CssColor, position: Option<Length>) -> GradientColorStop {
    GradientColorStop {
        color: GradientStopColor::Resolved(color),
        position,
    }
}

fn oklab_shorter() -> GradientColorInterpolation {
    GradientColorInterpolation {
        color_space: MixColorSpace::Oklab,
        hue_method: HueInterpolationMethod::Shorter,
    }
}

fn expect_linear_gradient(value: Option<PropertyValue>) -> LinearGradient {
    match value {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(Gradient::Linear(g)))) => g,
        // cov:ignore: this branch only executes when a caller's `parse(...)`
        // unexpectedly fails to produce a linear gradient — every call site
        // below passes, so llvm-cov reports this panic arm as an uncovered
        // added line even though the successful branch above (and therefore
        // this helper itself) is exercised by every one of those call sites.
        other => panic!("expected a linear gradient BackgroundImage, got {other:?}"),
    }
}

fn expect_radial_gradient(value: Option<PropertyValue>) -> RadialGradient {
    match value {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(Gradient::Radial(g)))) => g,
        // cov:ignore: same reasoning as `expect_linear_gradient`'s panic arm.
        other => panic!("expected a radial gradient BackgroundImage, got {other:?}"),
    }
}

fn expect_conic_gradient(value: Option<PropertyValue>) -> ConicGradient {
    match value {
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(Gradient::Conic(g)))) => g,
        // cov:ignore: same reasoning as `expect_linear_gradient`'s panic arm.
        other => panic!("expected a conic gradient BackgroundImage, got {other:?}"),
    }
}

#[test]
fn background_image_parses_linear_gradient_with_default_direction_and_interpolation() {
    // No direction, no `in ...` clause — both spec-mandated defaults
    // (`to bottom` / `Oklab`) are baked in.
    assert_eq!(
        parse("linear-gradient(red, blue)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
            Gradient::Linear(LinearGradient {
                repeating: false,
                direction: LinearGradientDirection::Side(SideOrCorner {
                    horizontal: None,
                    vertical: Some(VerticalSide::Bottom),
                }),
                interpolation: oklab_shorter(),
                stops: Arc::new(vec![gradient_stop(RED, None), gradient_stop(BLUE, None),]),
            })
        )))
    );
}

#[test]
fn background_image_parses_repeating_linear_gradient_sets_repeating_flag() {
    let g = expect_linear_gradient(parse(
        "repeating-linear-gradient(red, blue)",
        "background-image",
    ));
    assert!(g.repeating);
}

#[test]
fn background_image_parses_linear_gradient_angle_direction() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(45deg, red, blue)",
        "background-image",
    ));
    assert_eq!(g.direction, LinearGradientDirection::Angle(Angle(45.0)));
}

#[test]
fn background_image_parses_linear_gradient_unitless_zero_angle() {
    // `<angle> | <zero>` — legacy bare `0` is valid (`parse_angle` doc).
    let g = expect_linear_gradient(parse("linear-gradient(0, red, blue)", "background-image"));
    assert_eq!(g.direction, LinearGradientDirection::Angle(Angle(0.0)));
}

#[test]
fn background_image_rejects_linear_gradient_bare_nonzero_number_as_angle() {
    // Unlike `<zero>`, a bare non-zero number is not a valid `<angle>`.
    assert_eq!(
        parse("linear-gradient(45, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_linear_gradient_unrecognized_angle_unit() {
    assert_eq!(
        parse("linear-gradient(45xyz, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_linear_gradient_angle_overflow_saturates_to_f32_max() {
    // `1e40turn` overflows f32 only *after* the `* 360.0` unit
    // conversion — same "convert, then saturate" policy `parse_length_value`
    // uses for percentages (`parse_angle` doc's "overflow saturation"
    // note).
    let g = expect_linear_gradient(parse(
        "linear-gradient(1e40turn, red, blue)",
        "background-image",
    ));
    assert_eq!(g.direction, LinearGradientDirection::Angle(Angle(f32::MAX)));
}

#[test]
fn background_image_parses_conic_gradient_stop_percentage_overflow_saturates_to_f32_max() {
    let g = expect_conic_gradient(parse("conic-gradient(red 1e40%, blue)", "background-image"));
    assert_eq!(
        g.stops[0].position,
        Some(AnglePercentage::Percent(f32::MAX))
    );
}

#[test]
fn background_image_parses_linear_gradient_to_bottom_explicit() {
    // Explicit `to bottom` — exercises `parse_vertical_side`'s `bottom`
    // arm directly, distinct from the same *value* reached via the
    // omitted-direction default (`..._with_default_direction_and_interpolation`).
    let g = expect_linear_gradient(parse(
        "linear-gradient(to bottom, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.direction,
        LinearGradientDirection::Side(SideOrCorner {
            horizontal: None,
            vertical: Some(VerticalSide::Bottom),
        })
    );
}

#[test]
fn background_image_rejects_linear_gradient_bare_to_keyword() {
    // `to` with no side-or-corner keyword following it — `parse_side_or_corner`
    // rejects when neither axis matched.
    assert_eq!(
        parse("linear-gradient(to, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_linear_gradient_side_followed_by_non_side_keyword() {
    // `right` matches the horizontal axis; `center` matches neither axis
    // of `parse_vertical_side`, exercising its rejection arm before the
    // whole declaration fails on the missing comma.
    assert_eq!(
        parse(
            "linear-gradient(to right center, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_conic_gradient_single_stop() {
    // `<angular-color-stop-list>` requires 2+ stops, same as
    // `GradientColorStop`'s linear/radial sibling.
    assert_eq!(parse("conic-gradient(red)", "background-image"), None);
}

#[test]
fn background_image_parses_linear_gradient_side_and_corner_any_order() {
    // `[left | right] || [top | bottom]` — keyword order doesn't matter.
    let to_top_left = parse(
        "linear-gradient(to top left, red, blue)",
        "background-image",
    );
    let to_left_top = parse(
        "linear-gradient(to left top, red, blue)",
        "background-image",
    );
    let expected = Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
        Gradient::Linear(LinearGradient {
            repeating: false,
            direction: LinearGradientDirection::Side(SideOrCorner {
                horizontal: Some(HorizontalSide::Left),
                vertical: Some(VerticalSide::Top),
            }),
            interpolation: oklab_shorter(),
            stops: Arc::new(vec![gradient_stop(RED, None), gradient_stop(BLUE, None)]),
        }),
    )));
    assert_eq!(to_top_left, expected);
    assert_eq!(to_left_top, expected);
}

#[test]
fn background_image_parses_linear_gradient_stop_positions() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(red 10%, blue 90%)",
        "background-image",
    ));
    assert_eq!(
        *g.stops,
        vec![
            gradient_stop(RED, Some(Length::Percent(10.0))),
            gradient_stop(BLUE, Some(Length::Percent(90.0))),
        ]
    );
}

#[test]
fn background_image_gradient_stop_accepts_currentcolor() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(currentcolor, blue)",
        "background-image",
    ));
    assert_eq!(g.stops[0].color, GradientStopColor::CurrentColor);
}

#[test]
fn background_image_rejects_linear_gradient_single_stop() {
    // CSS Images 3 baseline grammar requires 2+ stops (`BackgroundImage`
    // doc's scope-carving note — Level 4's single-stop relaxation is
    // deferred).
    assert_eq!(parse("linear-gradient(red)", "background-image"), None);
}

#[test]
fn background_image_rejects_linear_gradient_transition_hint() {
    // `<linear-color-hint>` between stops is unimplemented scope — the
    // bare `50%` token isn't a valid `<color>`, so the whole
    // comma-separated stop list fails to parse.
    assert_eq!(
        parse("linear-gradient(red, 50%, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_linear_gradient_color_interpolation_method() {
    let g = expect_linear_gradient(parse(
        "linear-gradient(in oklch longer hue, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.interpolation,
        GradientColorInterpolation {
            color_space: MixColorSpace::Oklch,
            hue_method: HueInterpolationMethod::Longer,
        }
    );
}

#[test]
fn background_image_rejects_hue_method_on_non_polar_interpolation_space() {
    // `<hue-interpolation-method>` is only valid for a polar `<color-space>`
    // (`Lch`/`Oklch`) — same rule `color-mix()` already enforces.
    assert_eq!(
        parse(
            "linear-gradient(in srgb longer hue, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_unimplemented_interpolation_color_space() {
    // `hsl`/`hwb`/`xyz` family have no `<color>` function parser in this
    // crate (`MixColorSpace` doc's scope-carving note), so they aren't
    // offered as gradient interpolation spaces either.
    assert_eq!(
        parse("linear-gradient(in hsl, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_linear_gradient_direction_and_interpolation_together_either_order() {
    // CSS Images 4 §3.1.1's own worked example, plus the reverse
    // ordering — the `||` combinator in `[ [ <angle> | <zero> | to
    // <side-or-corner> ] || <color-interpolation-method> ]?` permits
    // either order, exercising both branches of
    // `parse_linear_gradient_body`'s any-order loop in one gradient
    // rather than direction-only and interpolation-only separately.
    let direction_first = expect_linear_gradient(parse(
        "linear-gradient(in lab to right, #F01, #081)",
        "background-image",
    ));
    let interpolation_first = expect_linear_gradient(parse(
        "linear-gradient(to right in lab, #F01, #081)",
        "background-image",
    ));
    let expected_direction = LinearGradientDirection::Side(SideOrCorner {
        horizontal: Some(HorizontalSide::Right),
        vertical: None,
    });
    let expected_interpolation = GradientColorInterpolation {
        color_space: MixColorSpace::Lab,
        hue_method: HueInterpolationMethod::Shorter,
    };
    assert_eq!(direction_first.direction, expected_direction);
    assert_eq!(direction_first.interpolation, expected_interpolation);
    assert_eq!(interpolation_first.direction, expected_direction);
    assert_eq!(interpolation_first.interpolation, expected_interpolation);
}

#[test]
fn background_image_rejects_radial_gradient_bare_percentage_size() {
    // CSS Images 3 §3.2.1: "Percentages are not allowed here" for the
    // circle-radius `<length [0,∞]>` alternative — a bare `50%` (no
    // second value) doesn't match the 2-value ellipse form either, so
    // `parse_radial_size` has no alternative left to try.
    assert_eq!(
        parse("radial-gradient(50%, red, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_radial_gradient_position_before_shape() {
    // CSS Images 4 §3.2.1's `[ <radial-shape> || <radial-size> ]? [ at
    // <position> ]?` is a *sequence* of two groups — `at <position>`
    // may only follow the shape/size group, never precede it.
    // `parse_radial_shape_size_position_group` claims `at center` as a
    // position-only match, leaving `circle` as an unconsumed leftover
    // token that fails the subsequent `expect_comma()`.
    assert_eq!(
        parse(
            "radial-gradient(at center circle, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_conic_gradient_position_before_from_angle() {
    // Angular sibling of `..._rejects_radial_gradient_position_before_shape`
    // — CSS Images 4 §3.3.1's `[ from [...] ]? [ at <position> ]?` is
    // likewise a sequence, `from` before `at`.
    assert_eq!(
        parse(
            "conic-gradient(at center from 45deg, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_parses_radial_gradient_with_default_shape_size_and_position() {
    // No shape/size/position/interpolation — `ellipse farthest-corner at
    // center` / `Oklab` are all spec-mandated defaults.
    assert_eq!(
        parse("radial-gradient(red, blue)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
            Gradient::Radial(RadialGradient {
                repeating: false,
                shape: RadialShape::Ellipse,
                size: RadialSize::Extent(RadialExtent::FarthestCorner),
                position: CssPosition {
                    horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                    vertical: CssPositionOffset::Start(Length::Percent(50.0)),
                },
                interpolation: oklab_shorter(),
                stops: Arc::new(vec![gradient_stop(RED, None), gradient_stop(BLUE, None)]),
            })
        )))
    );
}

#[test]
fn background_image_parses_repeating_radial_gradient_sets_repeating_flag() {
    let g = expect_radial_gradient(parse(
        "repeating-radial-gradient(red, blue)",
        "background-image",
    ));
    assert!(g.repeating);
}

#[test]
fn background_image_parses_radial_gradient_circle_with_explicit_length() {
    // Spec's own CSS Images 3 §3.2.1 example.
    let g = expect_radial_gradient(parse(
        "radial-gradient(5em circle at top left, yellow, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Circle(Length::Em(5.0)));
    assert_eq!(
        g.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }
    );
}

#[test]
fn background_image_parses_radial_gradient_ellipse_with_two_lengths() {
    let g = expect_radial_gradient(parse(
        "radial-gradient(20px 30px, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Ellipse);
    assert_eq!(
        g.size,
        RadialSize::Ellipse(Length::Px(20.0), Length::Px(30.0))
    );
}

#[test]
fn background_image_parses_radial_gradient_bare_length_with_no_shape_keyword_infers_circle() {
    // Shape omitted + a single bare `<length>` (no percentage) — defaults
    // to circle (CSS Images 3 §3.2.1's "a single `<length>`" rule),
    // distinct from `..._circle_with_explicit_length` above (which
    // spells `circle` explicitly and exercises a different
    // `resolve_radial_shape_and_size` arm).
    let g = expect_radial_gradient(parse(
        "radial-gradient(5px at center, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Circle(Length::Px(5.0)));
}

#[test]
fn background_image_parses_radial_gradient_extent_keyword_infers_ellipse() {
    // Shape omitted + `<radial-extent>` keyword (not "a single <length>")
    // — defaults to ellipse (CSS Images 3 §3.2.1).
    let g = expect_radial_gradient(parse(
        "radial-gradient(closest-side, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Ellipse);
    assert_eq!(g.size, RadialSize::Extent(RadialExtent::ClosestSide));
}

#[test]
fn background_image_parses_radial_gradient_explicit_shape_with_extent_keyword() {
    // Explicit shape keyword *and* explicit extent keyword together —
    // distinct `resolve_radial_shape_and_size` arm from both the
    // shape-omitted case above and the shape-alone case below.
    let g = expect_radial_gradient(parse(
        "radial-gradient(circle closest-side, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Extent(RadialExtent::ClosestSide));
}

#[test]
fn background_image_parses_radial_gradient_explicit_ellipse_shape_with_two_lengths() {
    // Explicit `ellipse` shape keyword *and* explicit 2-length size
    // together — the sibling combination to
    // `..._circle_with_explicit_length` (`circle` + single length).
    let g = expect_radial_gradient(parse(
        "radial-gradient(ellipse 20px 30px, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Ellipse);
    assert_eq!(
        g.size,
        RadialSize::Ellipse(Length::Px(20.0), Length::Px(30.0))
    );
}

#[test]
fn background_image_parses_radial_gradient_shape_keyword_alone_defaults_to_farthest_corner() {
    // Explicit shape keyword, no size at all — `farthest-corner` default
    // still applies (distinct from the fully-omitted default test above,
    // which never names a shape keyword).
    let g = expect_radial_gradient(parse(
        "radial-gradient(circle, red, blue)",
        "background-image",
    ));
    assert_eq!(g.shape, RadialShape::Circle);
    assert_eq!(g.size, RadialSize::Extent(RadialExtent::FarthestCorner));
}

#[test]
fn background_image_parses_radial_gradient_explicit_color_interpolation_method() {
    let g = expect_radial_gradient(parse(
        "radial-gradient(in oklch, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.interpolation,
        GradientColorInterpolation {
            color_space: MixColorSpace::Oklch,
            hue_method: HueInterpolationMethod::Shorter,
        }
    );
}

#[test]
fn background_image_rejects_radial_gradient_circle_shape_with_ellipse_size() {
    // `circle` + a 2-length-percentage (ellipse-only) size is an invalid
    // combination (CSS Images 3 §3.2.1's expanded grammar).
    assert_eq!(
        parse(
            "radial-gradient(circle 20px 30px, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_radial_gradient_negative_length() {
    assert_eq!(
        parse(
            "radial-gradient(circle -5px, red, blue)",
            "background-image"
        ),
        None
    );
}

#[test]
fn background_image_rejects_linear_gradient_double_position_stop() {
    // Level 4's `<color-stop-length> = <length-percentage>{1,2}` (one
    // stop, two positions) is deferred scope (`GradientColorStop` doc)
    // — this pins that the second position is *rejected*, not silently
    // discarded: after `parse_gradient_color_stop` consumes one
    // `<length-percentage>`, the leftover `20%` token makes
    // `parse_comma_separated`'s per-segment parse fail, dropping the
    // whole declaration rather than producing a stop at `10%` alone.
    assert_eq!(
        parse("linear-gradient(red 10% 20%, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_rejects_conic_gradient_double_angle_stop() {
    // Angular sibling of `..._rejects_linear_gradient_double_position_stop`.
    assert_eq!(
        parse("conic-gradient(red 0deg 90deg, blue)", "background-image"),
        None
    );
}

#[test]
fn background_image_parses_conic_gradient_with_default_angle_and_position() {
    assert_eq!(
        parse("conic-gradient(red, blue)", "background-image"),
        Some(PropertyValue::BackgroundImage(BackgroundImage::Gradient(
            Gradient::Conic(ConicGradient {
                repeating: false,
                angle: Angle(0.0),
                position: CssPosition {
                    horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
                    vertical: CssPositionOffset::Start(Length::Percent(50.0)),
                },
                interpolation: oklab_shorter(),
                stops: Arc::new(vec![
                    AngularColorStop {
                        color: GradientStopColor::Resolved(RED),
                        position: None,
                    },
                    AngularColorStop {
                        color: GradientStopColor::Resolved(BLUE),
                        position: None,
                    },
                ]),
            })
        )))
    );
}

#[test]
fn background_image_parses_repeating_conic_gradient_sets_repeating_flag() {
    let g = expect_conic_gradient(parse(
        "repeating-conic-gradient(gold, #f06 20deg)",
        "background-image",
    ));
    assert!(g.repeating);
}

#[test]
fn background_image_parses_conic_gradient_from_angle_and_position() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(from 45deg at 25% 40%, white, black)",
        "background-image",
    ));
    assert_eq!(g.angle, Angle(45.0));
    assert_eq!(
        g.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(25.0)),
            vertical: CssPositionOffset::Start(Length::Percent(40.0)),
        }
    );
}

#[test]
fn background_image_parses_conic_gradient_stop_with_percentage_position() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(#f06 0%, gold 100%)",
        "background-image",
    ));
    assert_eq!(g.stops[0].position, Some(AnglePercentage::Percent(0.0)));
}

#[test]
fn background_image_parses_conic_gradient_stop_with_angle_position() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(#f06 0deg, gold 1turn)",
        "background-image",
    ));
    assert_eq!(
        g.stops[1].position,
        Some(AnglePercentage::Angle(Angle(360.0)))
    );
}

#[test]
fn background_image_parses_conic_gradient_explicit_color_interpolation_method() {
    let g = expect_conic_gradient(parse(
        "conic-gradient(in oklch, red, blue)",
        "background-image",
    ));
    assert_eq!(
        g.interpolation,
        GradientColorInterpolation {
            color_space: MixColorSpace::Oklch,
            hue_method: HueInterpolationMethod::Shorter,
        }
    );
}

#[test]
fn background_image_rejects_bare_string_without_url_wrapper() {
    // `<image>` は `<url> | <gradient>` のみで bare `<string>` を含まない
    // — `parse_url_value` doc の同節参照 (`content_bare_string_is_still_literal_not_image`
    // と同型の regression check)。
    assert_eq!(parse("\"foo.png\"", "background-image"), None);
}

#[test]
fn background_image_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "background-image"), None, "{keyword}");
    }
}

#[test]
fn background_image_rejects_unknown_keyword() {
    assert_eq!(parse("foo", "background-image"), None);
}

#[test]
fn background_image_key_maps_to_background_image_property_key() {
    let v = PropertyValue::BackgroundImage(BackgroundImage::None);
    assert_eq!(v.key(), PropertyKey::BackgroundImage);
}

// ── background-* comma-list (multi-layer) rejection ──

#[test]
fn background_longhands_reject_comma_separated_multi_layer() {
    // All 7 background-* longhands are single-layer only; comma-separated
    // multi-layer (#-list) must be whole-declaration-drop via
    // `DeclParser::parse_value` + `expect_exhausted` (rule.rs:1094 etc.).
    // This pins that a future refactor never silently truncates to the
    // first layer ("first layer wins") — which would paint the wrong
    // background instead of falling through to the previous declaration
    // or the initial value.
    let cases: &[(&str, &str)] = &[
        ("background-repeat", "repeat, no-repeat"),
        ("background-attachment", "scroll, fixed"),
        ("background-clip", "border-box, padding-box"),
        ("background-origin", "padding-box, content-box"),
        ("background-size", "cover, contain"),
        ("background-position", "left top, right bottom"),
        ("background-image", "url(a.png), url(b.png)"),
    ];
    for (name, source) in cases {
        assert_eq!(
            parse_entire(source, name),
            None,
            "{name}: {source:?} should be whole-declaration-drop"
        );
        // The leading layer itself is valid — so the rejection is solely
        // due to the trailing `, <layer>` leftover caught by
        // `expect_exhausted`, not the parser rejecting the first token.
        assert!(
            parse(source, name).is_some(),
            "{name}: first layer of {source:?} should still parse without exhaustion check"
        );
    }
}

#[test]
fn background_image_rejects_comma_list_with_none_and_gradient_variants() {
    // `none` and `<gradient>` are also valid single layers; mixing them
    // with a comma must still be whole-declaration-drop.
    for source in [
        "none, url(b.png)",
        "url(a.png), none",
        "none, none",
        "linear-gradient(red, blue), url(b.png)",
        "url(a.png), linear-gradient(red, blue)",
    ] {
        assert_eq!(
            parse_entire(source, "background-image"),
            None,
            "background-image: {source:?} should be whole-declaration-drop"
        );
    }
}

// ── `background` shorthand (CSS Backgrounds and Borders 3 §2.10) ──

fn expect_background(value: Option<PropertyValue>) -> BackgroundShorthand {
    match value {
        Some(PropertyValue::Background(shorthand)) => shorthand,
        // cov:ignore: this branch only executes when a caller's
        // `parse(..., "background")` unexpectedly fails to parse or
        // parses to the wrong variant; every call site in this test
        // module passes valid `background` shorthand input, so the
        // panic never fires while the tests pass.
        other => panic!("expected PropertyValue::Background, got {other:?}"),
    }
}

/// Spec §2.10 verbatim first worked example: "In the first rule …, only
/// a value for background-color has been given and the other individual
/// properties are set to their initial values." — `body { background:
/// red }` is spec-equivalent to setting all 8 longhands, 7 of them to
/// their initial value.
#[test]
fn background_shorthand_color_only_fills_the_other_7_with_initial_values() {
    let got = expect_background(parse("red", "background"));
    assert_eq!(
        got,
        BackgroundShorthand {
            color: CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            },
            image: BackgroundImage::None,
            repeat: BackgroundRepeat {
                x: BackgroundRepeatKeyword::Repeat,
                y: BackgroundRepeatKeyword::Repeat,
            },
            attachment: BackgroundAttachment::Scroll,
            position: CssPosition {
                horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
                vertical: CssPositionOffset::Start(Length::Percent(0.0)),
            },
            size: BackgroundSize::Explicit {
                width: LengthOrAuto::Auto,
                height: LengthOrAuto::Auto,
            },
            // 0 `<visual-box>` occurrence: the 2 longhands fall back to
            // their own (different) initial values, not to each other.
            clip: VisualBox::BorderBox,
            origin: VisualBox::PaddingBox,
        }
    );
}

/// Spec §2.10 verbatim second worked example: `p { background:
/// url("chess.png") 40% / 10em gray round fixed border-box; }` is
/// spec-equivalent to `background-color: gray; background-position: 40%
/// 50%; background-size: 10em auto; background-repeat: round;
/// background-clip: border-box; background-origin: border-box;
/// background-attachment: fixed; background-image: url(chess.png)`.
/// Single `<visual-box>` occurrence (`border-box`) sets **both**
/// origin and clip to it.
#[test]
fn background_shorthand_spec_example_url_position_size_color_repeat_attachment_box() {
    let got = expect_background(parse(
        "url(\"chess.png\") 40% / 10em gray round fixed border-box",
        "background",
    ));
    assert_eq!(got.image, BackgroundImage::Url("chess.png".to_string()));
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(40.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }
    );
    assert_eq!(
        got.size,
        BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Em(10.0)),
            height: LengthOrAuto::Auto,
        }
    );
    assert_eq!(
        got.color,
        CssColor {
            r: 128,
            g: 128,
            b: 128,
            a: 255
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::Round,
            y: BackgroundRepeatKeyword::Round,
        }
    );
    assert_eq!(got.attachment, BackgroundAttachment::Fixed);
    assert_eq!(got.clip, VisualBox::BorderBox);
    assert_eq!(got.origin, VisualBox::BorderBox);
}

/// Spec §2.10 verbatim third worked example: `div { background:
/// padding-box url(paper.jpg) white center }` is spec-equivalent to
/// `background-color: white; background-image: url(paper.jpg);
/// background-repeat: repeat; background-attachment: scroll;
/// background-position: center; background-clip: padding-box;
/// background-origin: padding-box; background-size: auto auto`. Also
/// exercises `||` reordering: the `<visual-box>` component appears
/// *before* the image/color components in this example.
#[test]
fn background_shorthand_spec_example_box_before_image_and_color() {
    let got = expect_background(parse(
        "padding-box url(paper.jpg) white center",
        "background",
    ));
    assert_eq!(got.clip, VisualBox::PaddingBox);
    assert_eq!(got.origin, VisualBox::PaddingBox);
    assert_eq!(got.image, BackgroundImage::Url("paper.jpg".to_string()));
    assert_eq!(
        got.color,
        CssColor {
            r: 255,
            g: 255,
            b: 255,
            a: 255
        }
    );
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        }
    );
    assert_eq!(got.attachment, BackgroundAttachment::Scroll);
    assert_eq!(
        got.size,
        BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        }
    );
}

/// Spec §2.10's 2nd worked example (`E { background: #CCC
/// url("metal.jpg") top left / 100% auto no-repeat}`), single-layer
/// portion only — this crate does not accept the spec's other example
/// (`background: url(a.png) top left no-repeat, …`, comma-separated
/// multi-layer, see `background_shorthand_rejects_comma_separated_multi_layer`
/// below). `top left` (keyword reordering, only reachable via
/// `parse_bg_position`'s `&&` branch — CSS Position 3 §2's
/// non-reordering 2-value form rejects `top` in the horizontal slot)
/// immediately followed by `/ 100% auto` exercises the atomic
/// position+size `||` component with a non-trivial position.
#[test]
fn background_shorthand_reordered_position_before_slash_size() {
    let got = expect_background(parse(
        "#CCC url(\"metal.jpg\") top left / 100% auto no-repeat",
        "background",
    ));
    assert_eq!(
        got.color,
        CssColor {
            r: 204,
            g: 204,
            b: 204,
            a: 255
        }
    );
    assert_eq!(got.image, BackgroundImage::Url("metal.jpg".to_string()));
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }
    );
    assert_eq!(
        got.size,
        BackgroundSize::Explicit {
            width: LengthOrAuto::Length(Length::Percent(100.0)),
            height: LengthOrAuto::Auto,
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }
    );
}

/// 2 distinct `<visual-box>` occurrences: first sets `background-origin`,
/// second sets `background-clip` (spec §2.10 verbatim, [`BackgroundShorthand`]
/// doc). Neither of the spec's own worked examples exercises 2
/// *different* values (its only 2-occurrence-adjacent example still
/// repeats the same keyword), so this is a crate-authored regression check
/// for the origin-then-clip assignment order specifically.
#[test]
fn background_shorthand_two_distinct_visual_boxes_assign_origin_then_clip() {
    let got = expect_background(parse("content-box border-box", "background"));
    assert_eq!(got.origin, VisualBox::ContentBox);
    assert_eq!(got.clip, VisualBox::BorderBox);
}

/// `<bg-position>`'s 3-4 value edge-offset form (`CssPosition` doc's
/// `parse_bg_position` — branch3) immediately followed by another `||`
/// component. Unlike the longhand `background-position` parse path
/// (where leftover tokens always mean rejection via `expect_exhausted`),
/// the shorthand loop hands leftover tokens to the *next* component —
/// this pins that `bottom 10px right 20px` stops exactly at its own 4
/// tokens and does not swallow `no-repeat`.
#[test]
fn background_shorthand_edge_offset_position_stops_before_next_component() {
    let got = expect_background(parse("bottom 10px right 20px no-repeat", "background"));
    assert_eq!(
        got.position,
        CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }
    );
    assert_eq!(
        got.repeat,
        BackgroundRepeat {
            x: BackgroundRepeatKeyword::NoRepeat,
            y: BackgroundRepeatKeyword::NoRepeat,
        }
    );
}

/// spec `||` grammar: "one or more of them must occur" — 0 component is
/// invalid.
#[test]
fn background_shorthand_rejects_empty_value() {
    assert_eq!(parse("", "background"), None);
}

/// An unrecognized ident matches no component's grammar at all (not
/// `none`/`url()`/gradient, not a `<repeat-style>`/`<attachment>`/
/// `<visual-box>` keyword, not a `<position>` keyword, not a named
/// color) — same "0 component" rejection as the empty-value case above.
#[test]
fn background_shorthand_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "background"), None);
}

/// spec §2.10's `<bg-layer>#? , <final-bg-layer>` grammar allows
/// comma-separated multi-layer values (see the spec's own 4th example,
/// `background: url(a.png) top left no-repeat, url(b.png) center /
/// 100% 100% no-repeat, url(c.png) white`). This crate's single-layer
/// scope carving ([`BackgroundShorthand`] doc's Non-goal section) does
/// not silently take the first layer — it rejects the whole declaration:
/// the parser stops at the first layer's end, and the leftover comma
/// (plus any further layers) fails the caller's `expect_exhausted`
/// check, dropping the declaration entirely.
#[test]
fn background_shorthand_rejects_comma_separated_multi_layer() {
    assert_eq!(
        parse_entire("url(a.png) top, url(b.png) bottom", "background"),
        None
    );
}

/// `||` semantics: each component at most once. A 2nd `<repeat-style>`
/// token has nowhere to go (the `repeat` slot is already filled by the
/// first `repeat`, and `repeat-x` matches no other slot) — leftover,
/// declaration dropped.
#[test]
fn background_shorthand_rejects_repeated_repeat_style_component() {
    assert_eq!(parse_entire("repeat repeat-x", "background"), None);
}

/// `<visual-box>` may occur at most **twice** (spec §2.10 verbatim, "If
/// two values are present…" — never 3). A 3rd occurrence is leftover.
#[test]
fn background_shorthand_rejects_a_third_visual_box_occurrence() {
    assert_eq!(
        parse_entire("border-box padding-box content-box", "background"),
        None
    );
}

/// A `/` component followed by an unparseable `<bg-size>` does not
/// silently drop just the size — `parse_background_position_and_size`'s
/// inner `try_parse` rewinds the whole `/ <bg-size>` attempt (position
/// succeeds alone, size stays absent), leaving `/ bogus` as leftover
/// that matches no other `||` component, so the whole declaration is
/// dropped rather than falling back to the shorthand's `auto auto` size
/// fill.
#[test]
fn background_shorthand_rejects_slash_with_invalid_size() {
    assert_eq!(parse_entire("center / bogus", "background"), None);
}

#[test]
fn background_shorthand_rejects_size_without_a_preceding_position() {
    // The `<bg-position> [ / <bg-size> ]?` slot is atomic — `<bg-size>`
    // is a "then"-clause of a leading `<bg-position>`, never a
    // standalone `||` component on its own (`parse_background_shorthand`
    // doc's grammar section). `/ 100% auto` has no position to attach
    // to, so the leading `/` matches no slot at all and is dropped as
    // leftover, same as `center / bogus` above but exercising the
    // "no position present" edge rather than "position present, size
    // invalid".
    assert_eq!(parse_entire("/ 100% auto", "background"), None);
}

#[test]
fn background_shorthand_key_maps_to_background_property_key() {
    let v = PropertyValue::Background(BackgroundShorthand {
        color: CssColor::TRANSPARENT,
        image: BackgroundImage::None,
        repeat: BackgroundRepeat {
            x: BackgroundRepeatKeyword::Repeat,
            y: BackgroundRepeatKeyword::Repeat,
        },
        attachment: BackgroundAttachment::Scroll,
        position: CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        },
        size: BackgroundSize::Explicit {
            width: LengthOrAuto::Auto,
            height: LengthOrAuto::Auto,
        },
        clip: VisualBox::BorderBox,
        origin: VisualBox::PaddingBox,
    });
    assert_eq!(v.key(), PropertyKey::Background);
}

// ── object-fit (CSS Images Module Level 3 §5.1) ──

#[test]
fn object_fit_parse_all_five_keywords() {
    assert_eq!(
        parse("fill", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::Fill))
    );
    assert_eq!(
        parse("contain", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::Contain))
    );
    assert_eq!(
        parse("cover", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::Cover))
    );
    assert_eq!(
        parse("none", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::None))
    );
    assert_eq!(
        parse("scale-down", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::ScaleDown))
    );
}

#[test]
fn object_fit_is_case_insensitive() {
    assert_eq!(
        parse("SCALE-DOWN", "object-fit"),
        Some(PropertyValue::ObjectFit(ObjectFit::ScaleDown))
    );
}

#[test]
fn object_fit_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "object-fit"), None, "{keyword}");
    }
}

#[test]
fn object_fit_rejects_unknown_keyword() {
    assert_eq!(parse("stretch", "object-fit"), None);
    assert_eq!(parse("16px", "object-fit"), None);
}

#[test]
fn object_fit_key_maps_to_object_fit_property_key() {
    let v = PropertyValue::ObjectFit(ObjectFit::Fill);
    assert_eq!(v.key(), PropertyKey::ObjectFit);
}

// ── object-position (CSS Images Module Level 3 §5.2) ──
//
// `object-position` uses `parse_position_strict`, not
// `parse_bg_position` (`CssPosition` doc's Grammar section) — its Value
// is plain `<position>` (CSS Values 4 §8.3), not `<bg-position>`
// (`<bg-position>` is `background-position`'s own extension, CSS
// Backgrounds 3 §2.6). The `background_position_*` tests above check
// `<bg-position>` coverage (they exercise `parse_bg_position`, which
// *does* accept the 3-value edge-offset form `<bg-position>` adds on top
// of `<position>`) — that is NOT full `<position>` coverage, since the
// 3-value form is exactly what plain `<position>` disallows. These tests
// confirm both: the `object-position` name wires into
// `parse_position_strict` and into its own distinct
// `PropertyValue`/`PropertyKey` (`object_position_parse_reuses_position_grammar`),
// and that the 3-value form specific to `<bg-position>` is rejected
// (`object_position_rejects_bg_position_only_3_value_edge_offset_forms`).

#[test]
fn object_position_parse_reuses_position_grammar() {
    // 1st alternative, bare keyword pair.
    assert_eq!(
        parse("top", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(0.0)),
        }))
    );
    // 3rd alternative (`&&`, either order) — `top left` and `left top`
    // both parse.
    let expected = PropertyValue::ObjectPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
        vertical: CssPositionOffset::Start(Length::Percent(0.0)),
    });
    assert_eq!(parse("left top", "object-position"), Some(expected.clone()));
    assert_eq!(parse("top left", "object-position"), Some(expected));
    // 2nd alternative, bare `<length-percentage>` pair.
    assert_eq!(
        parse("10px 20%", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(10.0)),
            vertical: CssPositionOffset::Start(Length::Percent(20.0)),
        }))
    );
    // 4-value edge-offset form — both axes carry an offset, so this is
    // valid for plain `<position>` too (`<position-four>`,
    // `parse_position_branch3_strict` doc's "Why" section), unlike the
    // 3-value forms `object_position_rejects_bg_position_only_3_value_edge_offset_forms`
    // pins as rejected. Both this and the next case start with a
    // vertical edge (`bottom`/`top`), so they exercise the
    // vertical-exclusive-first branch of `parse_position_branch3_strict`;
    // the horizontal-exclusive-first branch (symmetric offset case with
    // `h_offset == v_offset == true` accepted via horizontal-edge
    // priority) is pinned separately below.
    assert_eq!(
        parse("bottom 10px right 20px", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
    // Same 4-value form with the other vertical edge — also
    // vertical-exclusive-first, not a distinct branch from the case
    // above (both start with `bottom`/`top`; `&&` allows either order
    // but both orders here are still vertical-first).
    assert_eq!(
        parse("top 10px left 20px", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Px(20.0)),
            vertical: CssPositionOffset::Start(Length::Px(10.0)),
        }))
    );
    // Horizontal-edge-first 4-value form — exercises the
    // horizontal-exclusive-first branch of `parse_position_branch3_strict`
    // (the symmetric `h_offset == v_offset == true` accept path where
    // horizontal edge wins). Distinct from the two vertical-first cases
    // above.
    assert_eq!(
        parse("right 20px bottom 10px", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::End(Length::Px(20.0)),
            vertical: CssPositionOffset::End(Length::Px(10.0)),
        }))
    );
    // Ambiguous `center` leading, paired with a bare (offset-less)
    // vertical keyword — exercises `parse_position_branch3_strict`'s
    // ambiguous-`center` branch success arm (`center` and the other
    // axis both carry no offset, so it's symmetric and accepted).
    assert_eq!(
        parse("center bottom", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
            vertical: CssPositionOffset::Start(Length::Percent(100.0)),
        }))
    );
    // Ambiguous `center` leading, paired with a bare (offset-less)
    // horizontal keyword — `parse_position_vertical_edge` fails on
    // `left` (not `top`/`bottom`), falling to the horizontal-edge-
    // after-center check; symmetric (both sides offset-less) so
    // accepted. Distinct code path from `center bottom` above (which
    // never reaches the horizontal-edge-after-center check at all) and
    // from `center right 10px`
    // (`object_position_rejects_bg_position_only_3_value_edge_offset_forms`,
    // which takes the same path but with an offset present).
    assert_eq!(
        parse("center left", "object-position"),
        Some(PropertyValue::ObjectPosition(CssPosition {
            horizontal: CssPositionOffset::Start(Length::Percent(0.0)),
            vertical: CssPositionOffset::Start(Length::Percent(50.0)),
        }))
    );
}

/// `<bg-position>` (CSS Backgrounds 3 §2.6) extends plain `<position>`
/// (CSS Values 4 §8.3) with a 3-value edge-offset form — offset
/// authored on exactly one of the two edge-keyword groups, the other
/// axis a bare keyword/`center`. That spec explicitly calls this out:
/// "For 3-value productions (which are not valid in `<position>`)".
/// `object-position`'s Value is `<position>`, not `<bg-position>`
/// (CSS Images 3 §5.2), so this asymmetric form must be rejected —
/// `parse_position_branch3_strict` doc's "Why" section. All 5 inputs
/// here have offset on exactly one axis; contrast with
/// `object_position_parse_reuses_position_grammar`'s
/// `bottom 10px right 20px` (offset on both axes, still valid).
///
/// Uses `parse_entire`, not the bare `parse` helper: once
/// `parse_position_branch3_strict` rejects the asymmetric 3rd
/// alternative, `parse_position_strict`'s fallback to the 2nd/1st
/// alternative (shared with `parse_bg_position`, `CssPosition` doc's
/// Grammar section) greedily matches a *prefix* of these 3-token inputs
/// (e.g. `right 10px center` → 2nd alternative consumes `right 10px`,
/// leaving `center` over) — same "prefix match, caller enforces full
/// consumption" contract every multi-token value in this crate relies on
/// (`rule::DeclParser`'s `expect_exhausted`, mirrored here by
/// `parse_entirely`). The bare `parse` helper does not enforce that, so
/// it would see the prefix's `Some(..)` and miss the leftover.
#[test]
fn object_position_rejects_bg_position_only_3_value_edge_offset_forms() {
    for source in [
        "right 10px center",
        "right 10px top",
        "bottom 10px right",
        "left 5% center",
        "center bottom 10px",
        // Ambiguous `center` leading, followed by an offset-bearing
        // horizontal edge (`parse_position_vertical_edge` fails on
        // `right`/`left`, falling to the horizontal-edge-after-center
        // check) — same asymmetric shape, distinct code path from
        // `center bottom 10px` above (which hits the vertical-edge-
        // after-center check instead).
        "center right 10px",
    ] {
        assert_eq!(parse_entire(source, "object-position"), None, "{source}");
    }
}

#[test]
fn object_position_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "object-position"), None, "{keyword}");
    }
}

#[test]
fn object_position_key_maps_to_object_position_property_key() {
    let v = PropertyValue::ObjectPosition(CssPosition {
        horizontal: CssPositionOffset::Start(Length::Percent(50.0)),
        vertical: CssPositionOffset::Start(Length::Percent(50.0)),
    });
    assert_eq!(v.key(), PropertyKey::ObjectPosition);
}

// ── opacity (CSS Color 4 §3.3) ──────────────────────────────────────

#[test]
fn opacity_parse_number() {
    assert_eq!(parse("0.5", "opacity"), Some(PropertyValue::Opacity(0.5)));
    assert_eq!(parse("1", "opacity"), Some(PropertyValue::Opacity(1.0)));
    assert_eq!(parse("0", "opacity"), Some(PropertyValue::Opacity(0.0)));
}

#[test]
fn opacity_parse_percentage() {
    assert_eq!(parse("50%", "opacity"), Some(PropertyValue::Opacity(0.5)));
    assert_eq!(parse("100%", "opacity"), Some(PropertyValue::Opacity(1.0)));
    assert_eq!(parse("0%", "opacity"), Some(PropertyValue::Opacity(0.0)));
}

#[test]
fn opacity_out_of_range_values_are_preserved_unclamped_at_parse_time() {
    // CSS Color 4 §3.3: "Opacity values outside the range `[0, 1]` are not
    // invalid, and are preserved in specified values, but are clamped to
    // the range `[0, 1]` in computed values." — the parser (specified
    // layer) must NOT clamp; that is `SpecifiedValues::absolutize_with`'s
    // job (phase 3), pinned separately in `specified.rs`/`cascade.rs`.
    assert_eq!(parse("2", "opacity"), Some(PropertyValue::Opacity(2.0)));
    assert_eq!(parse("-0.5", "opacity"), Some(PropertyValue::Opacity(-0.5)));
    assert_eq!(parse("150%", "opacity"), Some(PropertyValue::Opacity(1.5)));
}

#[test]
fn opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity() {
    // `0 * 10^999` collapses to `0.0 * f32::INFINITY` = NaN internally
    // during cssparser's tokenizer computation (module doc's
    // "Numeric-token NaN stabilization" section), but
    // `expect_number_stable`/`expect_percentage_stable` (which
    // `parse_opacity_value` acquires its value through) recover the
    // spec-correct `0.0` before `parse_opacity_value`'s `!is_nan()`
    // guard ever runs — so `opacity: 0e999` parses successfully to
    // `PropertyValue::Opacity(0.0)`, not `None` (this is the
    // most-visible regression case the fix targets: without it, this
    // declaration used to be dropped and fall back to the initial
    // `1.0`, making a fully-transparent `0e999` render fully opaque).
    assert_eq!(parse("0e999", "opacity"), Some(PropertyValue::Opacity(0.0)));
    assert_eq!(
        parse("0e999%", "opacity"),
        Some(PropertyValue::Opacity(0.0))
    );

    // `+Inf`/`-Inf` are a *different* hazard class — a spec-valid
    // `<number>` overflow (not a NaN collapse) that the phase-3 clamp
    // handles correctly (`f32::clamp` maps either to `1.0`/`0.0`), so
    // the parser must NOT reject them here — doing so would drop the
    // whole declaration and wrongly fall back to the initial `1.0`
    // instead of clamping to `0.0` for a huge negative literal (this
    // was a regression in an earlier iteration of this guard, which
    // used `is_finite()` and rejected these too).
    assert_eq!(
        parse("1e40", "opacity"),
        Some(PropertyValue::Opacity(f32::INFINITY))
    );
    assert_eq!(
        parse("-1e40", "opacity"),
        Some(PropertyValue::Opacity(f32::NEG_INFINITY))
    );

    // Percentage branch: cssparser 0.37.0 tokenizes `<percentage>` by
    // parsing the numeric part as `f64` then dividing by `100.0` *in
    // `f64`* before casting to `f32`, so `1e40%` becomes `1e38` as
    // `f32` (finite — `1e40 / 100 = 1e38 < f32::MAX ≈ 3.4e38`) and does
    // NOT overflow to `Infinity`. The first exponent that does overflow
    // through this path is `1e41%` (`1e41 / 100 = 1e39 > f32::MAX` →
    // `Infinity`). Pin both signs to cover `!is_nan()` passthrough for
    // the percentage branch specifically (the existing `0e999%` case
    // only exercises the `NaN` rejection path, not this `Infinity`
    // passthrough).
    assert_eq!(
        parse("1e41%", "opacity"),
        Some(PropertyValue::Opacity(f32::INFINITY))
    );
    assert_eq!(
        parse("-1e41%", "opacity"),
        Some(PropertyValue::Opacity(f32::NEG_INFINITY))
    );
}

#[test]
fn opacity_rejects_non_number_percentage_tokens() {
    for source in ["auto", "none", "opaque"] {
        assert_eq!(parse(source, "opacity"), None, "{source}");
    }
}

#[test]
fn opacity_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "opacity"), None, "{keyword}");
    }
}

#[test]
fn opacity_key_maps_to_opacity_property_key() {
    let v = PropertyValue::Opacity(0.5);
    assert_eq!(v.key(), PropertyKey::Opacity);
}

// ── isolation (CSS Compositing and Blending Level 1 §3.4.2) ─────────

#[test]
fn isolation_parse_keywords() {
    assert_eq!(
        parse("auto", "isolation"),
        Some(PropertyValue::Isolation(Isolation::Auto))
    );
    assert_eq!(
        parse("isolate", "isolation"),
        Some(PropertyValue::Isolation(Isolation::Isolate))
    );
}

#[test]
fn isolation_rejects_unknown_keyword() {
    for source in ["none", "isolated"] {
        assert_eq!(parse(source, "isolation"), None, "{source}");
    }
}

#[test]
fn isolation_rejects_trailing_garbage() {
    // `parse_isolation` itself only consumes one ident token — a second
    // keyword is leftover input the property parser doesn't reject on
    // its own (this crate's convention: the declaration-level
    // `expect_exhausted` check, exercised here via `parse_entire`,
    // drops the whole declaration instead — same shape as
    // `mix_blend_mode_rejects_trailing_garbage` below).
    assert_eq!(parse_entire("auto isolate", "isolation"), None);
}

#[test]
fn isolation_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "isolation"), None, "{keyword}");
    }
}

#[test]
fn isolation_key_maps_to_isolation_property_key() {
    let v = PropertyValue::Isolation(Isolation::Isolate);
    assert_eq!(v.key(), PropertyKey::Isolation);
}

// ── mix-blend-mode (CSS Compositing and Blending Level 1 §3.4.1) ────

#[test]
fn mix_blend_mode_parse_all_16_keywords() {
    let cases = [
        ("normal", MixBlendMode::Normal),
        ("multiply", MixBlendMode::Multiply),
        ("screen", MixBlendMode::Screen),
        ("overlay", MixBlendMode::Overlay),
        ("darken", MixBlendMode::Darken),
        ("lighten", MixBlendMode::Lighten),
        ("color-dodge", MixBlendMode::ColorDodge),
        ("color-burn", MixBlendMode::ColorBurn),
        ("hard-light", MixBlendMode::HardLight),
        ("soft-light", MixBlendMode::SoftLight),
        ("difference", MixBlendMode::Difference),
        ("exclusion", MixBlendMode::Exclusion),
        ("hue", MixBlendMode::Hue),
        ("saturation", MixBlendMode::Saturation),
        ("color", MixBlendMode::Color),
        ("luminosity", MixBlendMode::Luminosity),
    ];
    for (source, expected) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(source, "mix-blend-mode"),
            Some(PropertyValue::MixBlendMode(expected)),
            "{source}"
        );
    }
}

#[test]
fn mix_blend_mode_rejects_unknown_keyword() {
    for source in ["hsl", "blend"] {
        assert_eq!(parse(source, "mix-blend-mode"), None, "{source}");
    }
}

#[test]
fn mix_blend_mode_rejects_trailing_garbage() {
    // Same shape as `isolation_rejects_trailing_garbage` — a second
    // keyword is leftover input for the declaration-level
    // `expect_exhausted` check to drop, not something
    // `parse_mix_blend_mode` itself rejects.
    assert_eq!(parse_entire("normal multiply", "mix-blend-mode"), None);
}

#[test]
fn mix_blend_mode_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "mix-blend-mode"), None, "{keyword}");
    }
}

#[test]
fn mix_blend_mode_key_maps_to_mix_blend_mode_property_key() {
    let v = PropertyValue::MixBlendMode(MixBlendMode::Multiply);
    assert_eq!(v.key(), PropertyKey::MixBlendMode);
}

// ── mask-image (CSS Masking Level 1 §7.1) ────────────────────────────

#[test]
fn mask_image_parse_none() {
    assert_eq!(
        parse("none", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::None))
    );
}

#[test]
fn mask_image_parse_url_unquoted_form() {
    assert_eq!(
        parse("url(mask.svg#m)", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::Url(
            "mask.svg#m".to_string()
        )))
    );
}

#[test]
fn mask_image_parse_url_quoted_form() {
    assert_eq!(
        parse("url(\"mask.svg#m\")", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::Url(
            "mask.svg#m".to_string()
        )))
    );
}

#[test]
fn mask_image_parse_gradient_reuses_the_shared_gradient_parser() {
    // grammar-shape reuse check (`parse_mask_image` doc) — the gradient
    // internals themselves are already exhaustively covered by
    // `background-image`'s own gradient tests, so this only confirms
    // the `<gradient>` alternative is reachable through `mask-image`.
    assert!(matches!(
        parse("linear-gradient(red, blue)", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::Gradient(
            Gradient::Linear(_)
        )))
    ));
}

#[test]
fn mask_image_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "mask-image"),
        Some(PropertyValue::MaskImage(MaskImage::None))
    );
}

#[test]
fn mask_image_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "mask-image"), None, "{keyword}");
    }
}

#[test]
fn mask_image_key_maps_to_mask_image_property_key() {
    let v = PropertyValue::MaskImage(MaskImage::None);
    assert_eq!(v.key(), PropertyKey::MaskImage);
}

// ── clip-path (CSS Masking Level 1 §5.1) ─────────────────────────────

#[test]
fn clip_path_parse_none() {
    assert_eq!(
        parse("none", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::None))
    );
}

#[test]
fn clip_path_parse_url() {
    assert_eq!(
        parse("url(#my-clip)", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::Url(
            "#my-clip".to_string()
        )))
    );
}

#[test]
fn clip_path_parse_url_quoted_form() {
    assert_eq!(
        parse("url(\"#my-clip\")", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::Url(
            "#my-clip".to_string()
        )))
    );
}

#[test]
fn clip_path_parse_all_7_geometry_box_keywords() {
    let cases = [
        ("border-box", GeometryBox::BorderBox),
        ("padding-box", GeometryBox::PaddingBox),
        ("content-box", GeometryBox::ContentBox),
        ("margin-box", GeometryBox::MarginBox),
        ("fill-box", GeometryBox::FillBox),
        ("stroke-box", GeometryBox::StrokeBox),
        ("view-box", GeometryBox::ViewBox),
    ];
    for (source, expected) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            parse(source, "clip-path"),
            Some(PropertyValue::ClipPath(ClipPath::GeometryBox(expected))),
            "{source}"
        );
    }
}

#[test]
fn clip_path_parses_basic_shapes() {
    // Basic shapes per CSS Shapes Module Level 1 §3.1 — spot check that
    // each of the 5 functions is accepted and lands as ClipPath::BasicShape.
    let cases = [
        "circle(50%)",
        "ellipse(50% 50%)",
        "inset(10px)",
        "polygon(0 0, 100% 0, 100% 100%)",
        "path('M0 0 L10 10')",
    ];
    for source in cases {
        let parsed = parse(source, "clip-path");
        assert!(
            matches!(
                parsed,
                Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
            ),
            "expected BasicShape for {source}, got {parsed:?}"
        );
    }
}

#[test]
fn clip_path_basic_shape_with_geometry_box() {
    // The basic-shape / geometry-box pair — either order.
    let a = parse("circle(50%) border-box", "clip-path");
    assert!(matches!(
        a,
        Some(PropertyValue::ClipPath(ClipPath::BasicShape {
            geometry_box: Some(GeometryBox::BorderBox),
            ..
        }))
    ));
    let b = parse("border-box circle(50%)", "clip-path");
    assert!(matches!(
        b,
        Some(PropertyValue::ClipPath(ClipPath::BasicShape {
            geometry_box: Some(GeometryBox::BorderBox),
            ..
        }))
    ));
    // Also check with padding-box + ellipse at center
    let c = parse("ellipse(50% 50% at center) padding-box", "clip-path");
    assert!(matches!(
        c,
        Some(PropertyValue::ClipPath(ClipPath::BasicShape {
            geometry_box: Some(GeometryBox::PaddingBox),
            ..
        }))
    ));
}

#[test]
fn clip_path_inset_round_and_polygon_fill_rule() {
    assert!(matches!(
        parse("inset(10px 20px round 5px)", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
    ));
    assert!(matches!(
        parse("polygon(evenodd, 0 0, 100% 0, 100% 100%)", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
    ));
    assert!(matches!(
        parse("path(evenodd, 'M0 0 L10 10 Z')", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
    ));
}

#[test]
fn clip_path_inset_vertical_radius_and_polygon_round() {
    for source in [
        "inset(10px round 5px / 8px)",
        "inset(10px round 5px 6px / 7px 8px 9px 10px)",
        "polygon(round 4px, 0 0, 100% 0, 100% 100%)",
        "polygon(evenodd, round 4px, 0 0, 100% 0, 100% 100%)",
    ] {
        assert!(
            matches!(
                parse(source, "clip-path"),
                Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
            ),
            "{source}"
        );
    }
    for source in [
        "inset(10px round 5px / -8px)",
        "polygon(round -4px, 0 0, 100% 0, 100% 100%)",
    ] {
        assert_eq!(parse(source, "clip-path"), None, "{source}");
    }
}

#[test]
fn clip_path_circle_ellipse_position_variants() {
    // circle with position, ellipse with at center, plain circle()
    for source in [
        "circle(at center)",
        "circle(closest-side at 10px 20%)",
        "ellipse(closest-side farthest-side at center)",
        "circle()",
        "ellipse()",
    ] {
        let parsed = parse(source, "clip-path");
        assert!(
            matches!(
                parsed,
                Some(PropertyValue::ClipPath(ClipPath::BasicShape { .. }))
            ),
            "{source}"
        );
    }
}

#[test]
fn clip_path_rejects_invalid_basic_shapes() {
    // Negative radius, empty inset, empty polygon should drop.
    for source in [
        "circle(-10px)",
        "inset()",
        "polygon()",
        "path()",
        "circle(50% 50% 50%)",
        "inset(10px unknown)",
    ] {
        assert_eq!(parse(source, "clip-path"), None, "{source}");
    }
}

#[test]
fn clip_path_is_case_insensitive() {
    assert_eq!(
        parse("NONE", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::None))
    );
    assert_eq!(
        parse("BORDER-BOX", "clip-path"),
        Some(PropertyValue::ClipPath(ClipPath::GeometryBox(
            GeometryBox::BorderBox
        )))
    );
}

#[test]
fn clip_path_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "clip-path"), None, "{keyword}");
    }
}

#[test]
fn clip_path_key_maps_to_clip_path_property_key() {
    let v = PropertyValue::ClipPath(ClipPath::None);
    assert_eq!(v.key(), PropertyKey::ClipPath);
}

// ── transform (CSS Transforms Level 1 §4/§9.1) ───────────────────────

fn expect_transform(value: Option<PropertyValue>) -> Vec<TransformFunction> {
    match value {
        Some(PropertyValue::Transform(v)) => (*v).clone(),
        // cov:ignore: this branch only executes when a caller's
        // `parse(...)` unexpectedly fails to produce a Transform value
        // — every call site below passes.
        other => panic!("expected a Transform PropertyValue, got {other:?}"),
    }
}

#[test]
fn transform_parse_none() {
    assert_eq!(
        parse("none", "transform"),
        Some(PropertyValue::Transform(empty_transform_list()))
    );
}

#[test]
fn transform_parse_matrix() {
    assert_eq!(
        expect_transform(parse("matrix(1, 2, 3, 4, 5, 6)", "transform")),
        vec![TransformFunction::Matrix([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])]
    );
}

#[test]
fn transform_matrix_requires_all_6_arguments() {
    for source in [
        "matrix()",
        "matrix(1, 2, 3, 4, 5)",
        "matrix(1, 2, 3, 4, 5, 6, 7)",
    ] {
        assert_eq!(parse(source, "transform"), None, "{source}");
    }
}

#[test]
fn transform_parse_translate_both_axes() {
    assert_eq!(
        expect_transform(parse("translate(10px, 20%)", "transform")),
        vec![TransformFunction::Translate(
            Length::Px(10.0),
            Length::Percent(20.0)
        )]
    );
}

#[test]
fn transform_translate_single_argument_defaults_ty_to_0() {
    // CSS Transforms Level 1 §9.1: the omitted 2nd argument of
    // `translate()` defaults to `0` (`parse_translate_args` doc).
    assert_eq!(
        expect_transform(parse("translate(10px)", "transform")),
        vec![TransformFunction::Translate(
            Length::Px(10.0),
            Length::Px(0.0)
        )]
    );
}

#[test]
fn transform_translate_requires_at_least_1_argument() {
    assert_eq!(parse("translate()", "transform"), None);
}

#[test]
fn transform_parse_translate_x_and_y() {
    assert_eq!(
        expect_transform(parse("translateX(5px)", "transform")),
        vec![TransformFunction::TranslateX(Length::Px(5.0))]
    );
    assert_eq!(
        expect_transform(parse("translateY(50%)", "transform")),
        vec![TransformFunction::TranslateY(Length::Percent(50.0))]
    );
}

#[test]
fn transform_translate_x_and_y_require_exactly_1_argument() {
    for source in ["translateX()", "translateY()"] {
        assert_eq!(parse(source, "transform"), None, "{source}");
    }
}

#[test]
fn transform_parse_scale_both_arguments() {
    assert_eq!(
        expect_transform(parse("scale(2, 3)", "transform")),
        vec![TransformFunction::Scale(2.0, 3.0)]
    );
}

#[test]
fn transform_scale_single_argument_copies_sx_into_sy() {
    // CSS Transforms Level 1 §9.1: the omitted 2nd argument of
    // `scale()` copies the 1st, unlike `translate()`/`skew()`'s
    // "defaults to 0" (`parse_scale_args` doc).
    assert_eq!(
        expect_transform(parse("scale(2)", "transform")),
        vec![TransformFunction::Scale(2.0, 2.0)]
    );
}

#[test]
fn transform_scale_requires_at_least_1_argument() {
    assert_eq!(parse("scale()", "transform"), None);
}

#[test]
fn transform_parse_scale_x_and_y() {
    assert_eq!(
        expect_transform(parse("scaleX(2)", "transform")),
        vec![TransformFunction::ScaleX(2.0)]
    );
    assert_eq!(
        expect_transform(parse("scaleY(3)", "transform")),
        vec![TransformFunction::ScaleY(3.0)]
    );
}

#[test]
fn transform_parse_rotate() {
    assert_eq!(
        expect_transform(parse("rotate(45deg)", "transform")),
        vec![TransformFunction::Rotate(Angle(45.0))]
    );
}

#[test]
fn transform_rotate_accepts_unitless_zero() {
    // CSS Values 4 §7.1's `<zero>` legacy allowance — `rotate(0)`.
    assert_eq!(
        expect_transform(parse("rotate(0)", "transform")),
        vec![TransformFunction::Rotate(Angle(0.0))]
    );
}

#[test]
fn transform_rotate_rejects_unitless_nonzero() {
    assert_eq!(parse("rotate(45)", "transform"), None);
}

#[test]
fn transform_parse_skew_both_arguments() {
    assert_eq!(
        expect_transform(parse("skew(10deg, 20deg)", "transform")),
        vec![TransformFunction::Skew(Angle(10.0), Angle(20.0))]
    );
}

#[test]
fn transform_skew_single_argument_defaults_second_to_0deg() {
    // Same "defaults to 0" shape as `translate()`, unlike `scale()`'s
    // "copies the 1st" (`parse_skew_args` doc).
    assert_eq!(
        expect_transform(parse("skew(30deg)", "transform")),
        vec![TransformFunction::Skew(Angle(30.0), Angle(0.0))]
    );
}

#[test]
fn transform_parse_skew_x_and_y() {
    assert_eq!(
        expect_transform(parse("skewX(10deg)", "transform")),
        vec![TransformFunction::SkewX(Angle(10.0))]
    );
    assert_eq!(
        expect_transform(parse("skewY(20deg)", "transform")),
        vec![TransformFunction::SkewY(Angle(20.0))]
    );
}

#[test]
fn transform_parse_multiple_functions_space_separated() {
    assert_eq!(
        expect_transform(parse("rotate(45deg) scale(2)", "transform")),
        vec![
            TransformFunction::Rotate(Angle(45.0)),
            TransformFunction::Scale(2.0, 2.0),
        ]
    );
}

#[test]
fn transform_rejects_comma_separated_functions() {
    // `<transform-list> = <transform-function>[+]` is whitespace-
    // separated, not comma-separated (`parse_transform` doc) — a
    // comma between functions is leftover input for the
    // declaration-level `expect_exhausted` check to drop.
    assert_eq!(parse_entire("rotate(1deg), scale(2)", "transform"), None);
}

#[test]
fn transform_rejects_3d_functions() {
    // V2 (3D transform, CSS Transforms Level 1 §10) is out of scope
    // (`TransformFunction` doc's Non-goal note) — these function names
    // are simply unrecognized.
    for source in [
        "translate3d(1px, 2px, 3px)",
        "rotate3d(1, 0, 0, 45deg)",
        "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)",
        "perspective(100px)",
    ] {
        assert_eq!(parse(source, "transform"), None, "{source}");
    }
}

#[test]
fn transform_rejects_unknown_function() {
    assert_eq!(parse("frobnicate(1px)", "transform"), None);
}

#[test]
fn transform_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "transform"), None, "{keyword}");
    }
}

#[test]
fn transform_matrix_zero_mantissa_huge_exponent_argument_resolves_to_zero() {
    // `0e999` collapses to `NaN` internally during cssparser
    // tokenization (module doc's "Numeric-token NaN stabilization"
    // section), but `expect_number_stable` (which `parse_transform_number`
    // acquires its value through) recovers the spec-correct `0.0`
    // before `parse_transform_number`'s `!is_nan()` guard ever runs —
    // unlike ordinary magnitude overflow (`1e40`, preserved as `+Inf`
    // below), this is not a hazard `matrix()` rejects.
    assert_eq!(
        expect_transform(parse("matrix(0e999, 0, 0, 1, 0, 0)", "transform")),
        vec![TransformFunction::Matrix([0.0, 0.0, 0.0, 1.0, 0.0, 0.0])]
    );
}

#[test]
fn transform_matrix_preserves_infinity_argument() {
    // `1e40` overflows to `+Inf` — a spec-valid `<number>` this crate
    // preserves (no range restriction on `matrix()`'s arguments),
    // unlike the NaN case above (`parse_transform_number` doc).
    assert_eq!(
        expect_transform(parse("matrix(1e40, 0, 0, 1, 0, 0)", "transform")),
        vec![TransformFunction::Matrix([
            f32::INFINITY,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0
        ])]
    );
}

#[test]
fn transform_translate_zero_mantissa_huge_exponent_length_resolves_to_zero() {
    // Same recovery as the `matrix()` case above, through
    // `parse_transform_length_percentage` / `parse_length_value` /
    // `next_numeric_stable` — `translate(0e999px)`'s omitted 2nd
    // argument defaults to `Length::Px(0.0)` (`parse_translate_args`
    // doc), same as the (now also `0.0`, not rejected) 1st.
    assert_eq!(
        expect_transform(parse("translate(0e999px)", "transform")),
        vec![TransformFunction::Translate(
            Length::Px(0.0),
            Length::Px(0.0)
        )]
    );
}

#[test]
fn transform_rotate_zero_mantissa_huge_exponent_angle_resolves_to_zero() {
    // Same recovery as above, through `parse_angle_reject_nan` /
    // `parse_angle` / `next_numeric_stable`.
    assert_eq!(
        expect_transform(parse("rotate(0e999deg)", "transform")),
        vec![TransformFunction::Rotate(Angle(0.0))]
    );
}

#[test]
fn transform_key_maps_to_transform_property_key() {
    let v = PropertyValue::Transform(empty_transform_list());
    assert_eq!(v.key(), PropertyKey::Transform);
}

// ── filter (CSS Filter Effects Level 1 §5/§6) ────────────────────────

fn expect_filter(value: Option<PropertyValue>) -> Vec<FilterFunction> {
    match value {
        Some(PropertyValue::Filter(v)) => (*v).clone(),
        // cov:ignore: same reasoning as `expect_transform`'s panic arm.
        other => panic!("expected a Filter PropertyValue, got {other:?}"),
    }
}

#[test]
fn filter_parse_none() {
    assert_eq!(
        parse("none", "filter"),
        Some(PropertyValue::Filter(empty_filter_list()))
    );
}

#[test]
fn filter_parse_blur_with_argument() {
    assert_eq!(
        expect_filter(parse("blur(5px)", "filter")),
        vec![FilterFunction::Blur(Length::Px(5.0))]
    );
}

#[test]
fn filter_blur_omitted_argument_defaults_to_0px() {
    assert_eq!(
        expect_filter(parse("blur()", "filter")),
        vec![FilterFunction::Blur(Length::Px(0.0))]
    );
}

#[test]
fn filter_blur_rejects_negative_length() {
    assert_eq!(parse("blur(-5px)", "filter"), None);
}

#[test]
fn filter_blur_rejects_percentage() {
    // `blur(<length>?)` — no `<percentage>` alternative.
    assert_eq!(parse("blur(50%)", "filter"), None);
}

/// `filter_parse_amount_functions_with_argument`/
/// `filter_amount_functions_preserve_over_100_percent_unclamped` の
/// per-function name/constructor table 用 — clippy `type_complexity`
/// を避けるための alias (意味論的な新型ではない、`RadialShapeSizePositionGroup`
/// と同じ convention)。
type FilterAmountCtorCase = (&'static str, fn(f32) -> FilterFunction);

#[test]
fn filter_parse_amount_functions_with_argument() {
    let cases: [FilterAmountCtorCase; 6] = [
        ("brightness", FilterFunction::Brightness),
        ("contrast", FilterFunction::Contrast),
        ("grayscale", FilterFunction::Grayscale),
        ("invert", FilterFunction::Invert),
        ("saturate", FilterFunction::Saturate),
        ("sepia", FilterFunction::Sepia),
    ];
    for (name, ctor) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            expect_filter(parse(&format!("{name}(0.5)"), "filter")),
            vec![ctor(0.5)],
            "{name}"
        );
        // `<number-percentage>` — percentage alternative.
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            expect_filter(parse(&format!("{name}(50%)"), "filter")),
            vec![ctor(0.5)],
            "{name}%"
        );
    }
    // `opacity()` is tested separately (`filter_opacity_function_...`)
    // to avoid a same-name clash with the `PropertyValue::Opacity`
    // property in this table's `ctor` type.
}

#[test]
fn filter_opacity_function_parses_and_is_distinct_from_the_opacity_property() {
    assert_eq!(
        expect_filter(parse("opacity(0.5)", "filter")),
        vec![FilterFunction::Opacity(0.5)]
    );
}

#[test]
fn filter_amount_functions_omitted_argument_defaults_to_1() {
    for name in [
        "brightness",
        "contrast",
        "grayscale",
        "invert",
        "opacity",
        "saturate",
        "sepia",
    ] {
        let source = format!("{name}()");
        let functions = expect_filter(parse(&source, "filter"));
        assert_eq!(functions.len(), 1, "{name}");
        let amount = match functions[0] {
            FilterFunction::Brightness(v)
            | FilterFunction::Contrast(v)
            | FilterFunction::Grayscale(v)
            | FilterFunction::Invert(v)
            | FilterFunction::Opacity(v)
            | FilterFunction::Saturate(v)
            | FilterFunction::Sepia(v) => v,
            // cov:ignore: unreachable given the `name` list above only
            // dispatches to the 7 arms this match already covers.
            ref other => panic!("unexpected filter function {other:?}"),
        };
        assert_eq!(amount, 1.0, "{name}");
    }
}

#[test]
fn filter_amount_functions_reject_negative() {
    for name in [
        "brightness",
        "contrast",
        "grayscale",
        "invert",
        "opacity",
        "saturate",
        "sepia",
    ] {
        let source = format!("{name}(-0.5)");
        assert_eq!(parse(&source, "filter"), None, "{source}");
    }
}

#[test]
fn filter_amount_functions_preserve_over_100_percent_unclamped() {
    // CSS Filter Effects Level 1 §6.1: values over 100% are allowed
    // for every one of the 7 amount functions — for
    // `grayscale()`/`invert()`/`opacity()`/`sepia()` specifically, the
    // spec adds "but UAs must clamp the values to 1" as a
    // **rendering-time** obligation, not a specified/computed-value
    // transform (`filter`'s own Computed value is "as specified",
    // `FilterFunction` doc's "Range restriction is reject, not clamp"
    // section) — so this crate preserves the raw value unclamped,
    // deferring the clamp to a future paint-side consumer.
    let cases: [FilterAmountCtorCase; 7] = [
        ("brightness", FilterFunction::Brightness),
        ("contrast", FilterFunction::Contrast),
        ("grayscale", FilterFunction::Grayscale),
        ("invert", FilterFunction::Invert),
        ("opacity", FilterFunction::Opacity),
        ("saturate", FilterFunction::Saturate),
        ("sepia", FilterFunction::Sepia),
    ];
    for (name, ctor) in cases {
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            expect_filter(parse(&format!("{name}(2)"), "filter")),
            vec![ctor(2.0)],
            "{name}"
        );
    }
}

#[test]
fn filter_amount_functions_zero_mantissa_huge_exponent_resolves_to_zero() {
    // `0e999` collapses to `NaN` internally (same cssparser tokenizer
    // hazard as `transform`'s numeric arguments), but
    // `expect_number_stable`/`expect_percentage_stable` (which
    // `parse_filter_amount` acquires its value through) recover the
    // spec-correct `0.0` before `parse_filter_amount`'s `v >= 0.0`
    // check ever runs — so it accepts `0.0` normally instead of
    // incidentally rejecting `NaN` (`parse_filter_amount` doc's "No
    // separate `!is_nan()` guard is needed" section).
    assert_eq!(
        expect_filter(parse("brightness(0e999)", "filter")),
        vec![FilterFunction::Brightness(0.0)]
    );
    assert_eq!(
        expect_filter(parse("brightness(0e999%)", "filter")),
        vec![FilterFunction::Brightness(0.0)]
    );
}

#[test]
fn filter_parse_hue_rotate_with_argument() {
    assert_eq!(
        expect_filter(parse("hue-rotate(90deg)", "filter")),
        vec![FilterFunction::HueRotate(Angle(90.0))]
    );
}

#[test]
fn filter_hue_rotate_omitted_argument_defaults_to_0deg() {
    assert_eq!(
        expect_filter(parse("hue-rotate()", "filter")),
        vec![FilterFunction::HueRotate(Angle(0.0))]
    );
}

#[test]
fn filter_hue_rotate_accepts_unitless_zero() {
    assert_eq!(
        expect_filter(parse("hue-rotate(0)", "filter")),
        vec![FilterFunction::HueRotate(Angle(0.0))]
    );
}

#[test]
fn filter_hue_rotate_is_not_normalized_beyond_360deg() {
    // CSS Filter Effects Level 1 §6.1: "Implementations must not
    // normalize this value" — `810deg` stays `810.0`, matching
    // `Angle` doc's own "no normalization" policy.
    assert_eq!(
        expect_filter(parse("hue-rotate(810deg)", "filter")),
        vec![FilterFunction::HueRotate(Angle(810.0))]
    );
}

#[test]
fn filter_hue_rotate_zero_mantissa_huge_exponent_angle_resolves_to_zero() {
    // Same recovery as `transform_rotate_zero_mantissa_huge_exponent_angle_resolves_to_zero`
    // — `hue-rotate()` shares `parse_angle_reject_nan` with `rotate()`.
    assert_eq!(
        expect_filter(parse("hue-rotate(0e999deg)", "filter")),
        vec![FilterFunction::HueRotate(Angle(0.0))]
    );
}

#[test]
fn filter_parse_drop_shadow_full_form() {
    assert_eq!(
        expect_filter(parse("drop-shadow(red 1px 2px 3px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(3.0),
            color: TextShadowColor::Resolved(RED),
        })]
    );
}

#[test]
fn filter_drop_shadow_omitted_color_defaults_to_currentcolor() {
    assert_eq!(
        expect_filter(parse("drop-shadow(1px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
}

#[test]
fn filter_drop_shadow_requires_at_least_the_2_offset_lengths() {
    assert_eq!(parse("drop-shadow(red)", "filter"), None);
    assert_eq!(parse("drop-shadow()", "filter"), None);
}

#[test]
fn filter_drop_shadow_third_length_rejects_negative() {
    // Standard deviation (3rd length) is non-negative — "Values are
    // interpreted as for box-shadow" (`FilterFunction::DropShadow`
    // doc), same as box-shadow's own blur-radius restriction.
    assert_eq!(parse("drop-shadow(1px 2px -3px)", "filter"), None);
}

#[test]
fn filter_drop_shadow_zero_mantissa_huge_exponent_offset_resolves_to_zero() {
    // `0e999` collapses to `NaN` internally during tokenization, but
    // `next_numeric_stable` recovers the spec-correct `0.0` before
    // `parse_shadow_length_reject_nan`'s `!is_nan()` guard ever runs —
    // `parse_drop_shadow_args` inherits this through its verbatim
    // reuse of `parse_text_shadow_item`, same shape as
    // `transform_translate_zero_mantissa_huge_exponent_length_resolves_to_zero`.
    assert_eq!(
        expect_filter(parse("drop-shadow(0e999px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(0.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
}

#[test]
fn filter_drop_shadow_offset_infinity_passes_through_unclamped() {
    // `+Inf`/`-Inf` are a *different* hazard class from `0e999`'s NaN
    // collapse above (ordinary `<number>` magnitude overflow, not a
    // `0 * Infinity` collapse) — legitimate, if extreme, `<length>`
    // values per CSS Values 4 §5, so unlike NaN they must NOT be
    // rejected here (`parse_shadow_length_reject_nan` doc参照).
    assert_eq!(
        expect_filter(parse("drop-shadow(1e40px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(f32::INFINITY),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
    assert_eq!(
        expect_filter(parse("drop-shadow(-1e40px 2px)", "filter")),
        vec![FilterFunction::DropShadow(TextShadowItem {
            offset_x: Length::Px(f32::NEG_INFINITY),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
        })]
    );
}

#[test]
fn filter_parse_url() {
    assert_eq!(
        expect_filter(parse("url(#my-filter)", "filter")),
        vec![FilterFunction::Url("#my-filter".to_string())]
    );
}

#[test]
fn filter_parse_url_quoted_form() {
    assert_eq!(
        expect_filter(parse("url(\"#my-filter\")", "filter")),
        vec![FilterFunction::Url("#my-filter".to_string())]
    );
}

#[test]
fn filter_parse_mixed_url_and_function_list() {
    assert_eq!(
        expect_filter(parse("url(#f) blur(2px)", "filter")),
        vec![
            FilterFunction::Url("#f".to_string()),
            FilterFunction::Blur(Length::Px(2.0)),
        ]
    );
}

#[test]
fn filter_parse_multiple_functions_space_separated() {
    assert_eq!(
        expect_filter(parse("blur(1px) blur(2px)", "filter")),
        vec![
            FilterFunction::Blur(Length::Px(1.0)),
            FilterFunction::Blur(Length::Px(2.0)),
        ]
    );
}

#[test]
fn filter_rejects_comma_separated_functions() {
    // Same whitespace-only shape as `transform` — see
    // `transform_rejects_comma_separated_functions`.
    assert_eq!(parse_entire("blur(1px), blur(2px)", "filter"), None);
}

#[test]
fn filter_rejects_unknown_function() {
    assert_eq!(parse("frobnicate(1px)", "filter"), None);
}

#[test]
fn filter_rejects_css_wide_keyword() {
    for keyword in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(keyword, "filter"), None, "{keyword}");
    }
}

#[test]
fn filter_key_maps_to_filter_property_key() {
    let v = PropertyValue::Filter(empty_filter_list());
    assert_eq!(v.key(), PropertyKey::Filter);
}
