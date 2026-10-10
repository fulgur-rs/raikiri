//! Tests for the box-model property parsers in `parse/box_model.rs`.

use super::*;

fn red() -> CssColor {
    CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    }
}

// ── box-sizing (CSS Sizing 3 §3.3) ────────
//
// Verification anchors:
//   #1 content-box → Some(BoxSizing::ContentBox)
//   #2 border-box  → Some(BoxSizing::BorderBox)
//   #3 padding-box → None (outside the spec; keyword removed from the CSS UI 3 draft)
//   #4 initial and #5 non-inheritance tests live in crate::computed.
//
// Same structure as sibling `display_*` / `text_align_*` keyword parser tests.

#[test]
fn box_sizing_parse_content_box() {
    assert_eq!(
        parse("content-box", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::ContentBox))
    );
}

#[test]
fn box_sizing_parse_border_box() {
    assert_eq!(
        parse("border-box", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::BorderBox))
    );
}

#[test]
fn box_sizing_rejects_unknown_ident() {
    // spec-invalid (→ drop):
    // `padding-box` appeared in the CSS UI 3 draft but was removed in css-sizing-3.
    //   (spec note "supersedes the one in `[CSS-UI-3]`").
    // `margin-box` is an arbitrary identifier outside the grammar.
    assert_eq!(parse("padding-box", "box-sizing"), None);
    assert_eq!(parse("margin-box", "box-sizing"), None);
    assert_eq!(parse("bogus", "box-sizing"), None);
}

#[test]
fn box_sizing_rejects_css_wide_keyword() {
    // (b) Unsupported: CSS-wide keywords are not implemented yet; silently drop them.
    // Canonical reference: the "CSS-wide keyword" section of the PropertyValue doc.
    assert_eq!(parse("inherit", "box-sizing"), None);
    assert_eq!(parse("initial", "box-sizing"), None);
    assert_eq!(parse("unset", "box-sizing"), None);
    assert_eq!(parse("revert", "box-sizing"), None);
    assert_eq!(parse("revert-layer", "box-sizing"), None);
}

#[test]
fn box_sizing_rejects_non_ident() {
    assert_eq!(parse("16px", "box-sizing"), None);
    assert_eq!(parse("100", "box-sizing"), None);
}

#[test]
fn box_sizing_is_case_insensitive() {
    // CSS Values 3 §3.1 "Pre-defined Keywords": keywords are ASCII case-insensitive,
    // just like the sibling `display_is_case_insensitive` test.
    assert_eq!(
        parse("CONTENT-BOX", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::ContentBox))
    );
    assert_eq!(
        parse("Border-Box", "box-sizing"),
        Some(PropertyValue::BoxSizing(BoxSizing::BorderBox))
    );
}

#[test]
fn box_sizing_key_returns_box_sizing() {
    // PropertyValue::BoxSizing → PropertyKey::BoxSizing (cascade winner
    // Discriminant selection path, symmetric with sibling `Display` / `TextAlign` key().
    let v = PropertyValue::BoxSizing(BoxSizing::BorderBox);
    assert_eq!(v.key(), PropertyKey::BoxSizing);
}

// ── position: running() (CSS GCPM 3 §1.2.1) ──
//
// Verification items 1–6 come from the task description;
// the canonical shape was later amended. Siblings follow the counter-* /
// content / string-set SmolStr wire-through pattern.

#[test]
fn position_parse_running_header() {
    // Verification 1: position: running(header)
    // → PropertyValue::Position(PositionValue::Running("header"))
    assert_eq!(
        parse("running(header)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("header")
        )))
    );
}

#[test]
fn position_parse_running_footer() {
    // Verification 2: smoke test a different name to pin that SmolStr::new is used.
    assert_eq!(
        parse("running(footer)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("footer")
        )))
    );
}

#[test]
fn position_parse_static() {
    // Verification 5 baseline: position: static → PositionValue::Static.
    // apply_value is a no-op, leaving the initial running_templates from inherit_from
    // (an empty Vec). Thus the cascade winner suppresses an earlier running(...) value
    // for ID purposes (verified by `static_position_wins_over_running` in cascade.rs).
    assert_eq!(
        parse("static", "position"),
        Some(PropertyValue::Position(PositionValue::Static))
    );
}

#[test]
fn position_running_case_insensitive_function_name() {
    // Verification 4: function names are ASCII case-insensitive (CSS convention),
    // but custom identifiers preserve case.
    assert_eq!(
        parse("RUNNING(header)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("header")
        )))
    );
}

#[test]
fn position_running_rejects_none_custom_ident() {
    // Verification 6: reject `running(none)`. Although `none` is not a
    // spec-defined position keyword, reject it as a custom identifier too: otherwise
    // an `element(none)` reference could silently match at runtime (the same rule
    // as string-set).
    assert_eq!(parse("running(none)", "position"), None);
}

#[test]
fn position_running_rejects_reserved_css_wide_keyword() {
    // spec CSS Values 4 §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>:
    // <custom-ident> excludes CSS-wide keywords and `default`.
    // Drop declarations such as position: running(inherit).
    assert_eq!(parse("running(inherit)", "position"), None);
    assert_eq!(parse("running(initial)", "position"), None);
    assert_eq!(parse("running(unset)", "position"), None);
    assert_eq!(parse("running(revert)", "position"), None);
    assert_eq!(parse("running(default)", "position"), None);
}

#[test]
fn position_rejects_missing_custom_ident() {
    // Spec §1.2.1: `running() = running( <custom-ident> )` requires an argument.
    // An empty argument is malformed; drop the declaration.
    assert_eq!(parse("running()", "position"), None);
}

#[test]
fn position_parse_sticky() {
    // CSS Positioned Layout Module Level 3 §3 sticky positioning
    // <https://www.w3.org/TR/css-position-3/#sticky-pos>:
    // `position: sticky` is a valid position value. During parsing, this crate
    // preserves it as PositionValue::Sticky (layout integration is future work).
    assert_eq!(
        parse("sticky", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
    // Identifiers are ASCII case-insensitive (following cssparser's expect_ident_matching,
    // just as for `static`).
    assert_eq!(
        parse("Sticky", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
    assert_eq!(
        parse("STICKY", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
}

#[test]
fn position_rejects_out_of_scope_keywords() {
    // Accept relative / absolute / fixed (position:relative offsets are implemented).
    // Also accept `sticky`.
    assert_eq!(
        parse("relative", "position"),
        Some(PropertyValue::Position(PositionValue::Relative))
    );
    assert_eq!(
        parse("absolute", "position"),
        Some(PropertyValue::Position(PositionValue::Absolute))
    );
    assert_eq!(
        parse("fixed", "position"),
        Some(PropertyValue::Position(PositionValue::Fixed))
    );
    // Drop any other keyword.
    assert_eq!(parse("inherit", "position"), None);
    assert_eq!(parse("initial", "position"), None);
}

#[test]
fn position_rejects_running_with_extra_arg() {
    // For `running(a, b)`, parse_nested_block uses parse_entirely to
    // detect the extra token, causing the declaration to be dropped.
    assert_eq!(parse("running(a, b)", "position"), None);
}

// ── padding (CSS Box 3 §4.1 physical + §4.2 shorthand) ──
//
// Primary sources:
// - https://www.w3.org/TR/css-box-3/#padding-physical
//   "Negative values for padding properties are invalid." — non-negative
//   Enforce the constraint at parse time: parse_padding_side checks >= 0.0
//   for all Length variants; negative values drop the declaration.
// - https://www.w3.org/TR/css-box-3/#padding-shorthand
//   `<'padding-top'>{1,4}` — 1-4 value expansion (top/right/bottom/left).

fn padding_sides(top: Length, right: Length, bottom: Length, left: Length) -> Sides<Length> {
    Sides {
        top,
        right,
        bottom,
        left,
    }
}

// Verification #3: parse four longhands over five units (px / % / em / pt).
#[test]
fn padding_top_parses_px() {
    assert_eq!(
        parse("10px", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(10.0)))
    );
}

#[test]
fn padding_right_parses_percentage() {
    // Spec grammar `<length-percentage>` accepts `%`.
    assert_eq!(
        parse("5%", "padding-right"),
        Some(PropertyValue::PaddingRight(Length::Percent(5.0)))
    );
}

#[test]
fn padding_bottom_parses_em() {
    assert_eq!(
        parse("1em", "padding-bottom"),
        Some(PropertyValue::PaddingBottom(Length::Em(1.0)))
    );
}

#[test]
fn padding_left_parses_pt() {
    assert_eq!(
        parse("12pt", "padding-left"),
        Some(PropertyValue::PaddingLeft(Length::Pt(12.0)))
    );
}

// Verification #4 — shorthand 1-4 value expansion (CSS Box 3 §4.2).
#[test]
fn padding_shorthand_one_value_all_sides() {
    // 1 value → 4 sides = value
    let px10 = Length::Px(10.0);
    assert_eq!(
        parse("10px", "padding"),
        Some(PropertyValue::Padding(Sides::all(px10)))
    );
}

#[test]
fn padding_shorthand_two_values_top_bottom_left_right() {
    // 2 values → top/bottom = 1st, left/right = 2nd
    let px10 = Length::Px(10.0);
    let px20 = Length::Px(20.0);
    assert_eq!(
        parse("10px 20px", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            px10, px20, px10, px20
        )))
    );
}

#[test]
fn padding_shorthand_three_values_top_horizontal_bottom() {
    // 3 values → top = 1st, left/right = 2nd, bottom = 3rd
    let px10 = Length::Px(10.0);
    let px20 = Length::Px(20.0);
    let px30 = Length::Px(30.0);
    assert_eq!(
        parse("10px 20px 30px", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            px10, px20, px30, px20
        )))
    );
}

#[test]
fn padding_shorthand_four_values_clockwise() {
    // 4 values → top / right / bottom / left (clockwise from top)
    assert_eq!(
        parse("10px 20px 30px 40px", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            Length::Px(10.0),
            Length::Px(20.0),
            Length::Px(30.0),
            Length::Px(40.0),
        )))
    );
}

#[test]
fn padding_shorthand_mixed_units() {
    // Spec (CSS Box 3) §4.2 permits per-value `<'padding-top'>` = `<length-percentage>`;
    // mixed units are also valid (padding: 10px 5% 1em 12pt).
    assert_eq!(
        parse("10px 5% 1em 12pt", "padding"),
        Some(PropertyValue::Padding(padding_sides(
            Length::Px(10.0),
            Length::Percent(5.0),
            Length::Em(1.0),
            Length::Pt(12.0),
        )))
    );
}

// Verification #5 — non-negative constraint (spec-literal claim).
#[test]
fn padding_top_rejects_negative_px() {
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid.".
    assert_eq!(parse("-5px", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_percentage() {
    // Negative percentages are likewise invalid per spec.
    assert_eq!(parse("-10%", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_em() {
    // Negative em (font-relative) is also invalid per spec.
    assert_eq!(parse("-1em", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_rem() {
    // Check nonnegative filtering across all Length variants (rem).
    assert_eq!(parse("-0.5rem", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_pt() {
    // Check nonnegative filtering across all Length variants (pt).
    assert_eq!(parse("-3pt", "padding-top"), None);
}

#[test]
fn padding_top_accepts_zero() {
    // Zero (the lower bound) is valid in the closed spec interval `[0,∞]`.
    // `0px` takes the Dimension arm; bare `0` takes the Number arm under
    // the CSS Values 3 §5 unitless-zero clause, then passes the nonnegative filter.
    assert_eq!(
        parse("0px", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(0.0)))
    );
    assert_eq!(
        parse("0", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(0.0)))
    );
}

#[test]
fn padding_shorthand_rejects_any_negative_value() {
    // For `padding: 10px -5px`, the {1,4} multiplier in CSS Box 3 §4.2 requires
    // each iteration to be valid `<'padding-top'>`. The second value, `-5px`,
    // violates CSS Box 3 §4.1's `[0,∞]` constraint; try_parse rewinds and
    // parse_padding_shorthand returns Some for the one-value form. DeclParser's
    // expect_exhausted then detects the remaining `-5px` and drops the whole
    // declaration. Check this through rule.rs, the actual caller path (parse_value
    // alone returns Some(all(10px)), but leftover tokens make it invalid).
    let decls_2 = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px -5px;",
    )));
    assert!(
        decls_2.is_empty(),
        "`padding: 10px -5px` must drop via expect_exhausted leftover"
    );
    // Likewise, a negative fourth value in the four-value form drops via leftover tokens.
    let decls_4 = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px 20px 30px -40px;",
    )));
    assert!(
        decls_4.is_empty(),
        "`padding: 10px 20px 30px -40px` must drop via expect_exhausted leftover"
    );
}

// Verification #6: reject `auto`, which is absent from the spec grammar.
#[test]
fn padding_top_rejects_auto_keyword() {
    // CSS Box 3 §4.1 grammar contains only `<length-percentage>`; `auto` belongs to
    // the margin extension, not padding. It is naturally rejected by fall-through
    // in the Dimension / Percentage arms of parse_length_value.
    assert_eq!(parse("auto", "padding-top"), None);
}

#[test]
fn padding_shorthand_rejects_auto_keyword() {
    // Reject `auto` in shorthand too (first value fails and the whole declaration drops).
    assert_eq!(parse("auto", "padding"), None);
}

#[test]
fn padding_shorthand_mixed_with_auto_drops_via_leftover() {
    // For `padding: 10px auto`, the first value succeeds, then `auto` as the second
    // makes try_parse rewind; parse_padding_shorthand returns Some for the
    // one-value form. This is observable through parse_value alone, but DeclParser's
    // expect_exhausted detects leftover `auto` and drops the declaration. Also check
    // that rule.rs, the actual caller path, drops it.
    let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px auto;",
    )));
    assert!(
        decls.is_empty(),
        "`padding: 10px auto` must drop via expect_exhausted leftover"
    );
}

// Verification #6: drop unsupported units outside our grammar (such as cqw / cap).
#[test]
fn padding_top_rejects_unsupported_unit() {
    // (b) Unsupported: cqw / cap and similar units are valid per spec, but
    // not implemented. parse_length_value drops them and propagates `None`,
    // dropping the declaration. `ch` / `lh` / `rlh` and the viewport units have
    // moved to the accepted set (see `padding_top_accepts_ch` /
    // `padding_top_accepts_lh`).
    assert_eq!(parse("10cqw", "padding-top"), None);
    assert_eq!(parse("5cap", "padding-top"), None);
}

#[test]
fn padding_top_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.
    assert_eq!(
        parse("5lh", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Lh(5.0)))
    );
    assert_eq!(
        parse("1rlh", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Rlh(1.0)))
    );
}

#[test]
fn padding_top_accepts_ch() {
    assert_eq!(
        parse("2ch", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Ch(2.0)))
    );
}

#[test]
fn padding_top_accepts_cm() {
    assert_eq!(
        parse("2cm", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Cm(2.0)))
    );
}

#[test]
fn padding_top_rejects_negative_cm() {
    // Check nonnegative filtering across all Length variants (cm, new absolute unit).
    assert_eq!(parse("-1cm", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_ex() {
    // Check nonnegative filtering across all Length variants (ex, new font-relative unit).
    assert_eq!(parse("-1ex", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_lh() {
    // Check nonnegative filtering across all Length variants (`lh`/`rlh`),
    // directly pinning that `Lh`/`Rlh` were added to the OR-pattern in
    // `Length::payload`.
    assert_eq!(parse("-1lh", "padding-top"), None);
    assert_eq!(parse("-1rlh", "padding-top"), None);
}

/// Directly exercise the remaining added units (`rex` / `rch` / `ic` / `ric` /
/// `mm` / `Q`) via `Length::payload`. Tests at other call sites
/// (font-size / width / height / margin / border-width / line-height)
/// cover only Ex / Ch / Cm / In / Pc; this test is needed for patch coverage
/// of every arm of the `Length::payload` OR-pattern.
#[test]
fn padding_top_accepts_remaining_additional_units() {
    assert_eq!(
        parse("1rex", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Rex(1.0)))
    );
    assert_eq!(
        parse("1rch", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Rch(1.0)))
    );
    assert_eq!(
        parse("1ic", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Ic(1.0)))
    );
    assert_eq!(
        parse("1ric", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Ric(1.0)))
    );
    assert_eq!(
        parse("1mm", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Mm(1.0)))
    );
    assert_eq!(
        parse("40Q", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Q(40.0)))
    );
}

// Verification — Sides::all constructor + PropertyKey mapping smoke.
#[test]
fn padding_key_maps_to_padding_property_keys() {
    // Each of five discriminants (four longhands and one shorthand) returns its own PropertyKey.
    // Verify discriminant integrity for cascade winner selection.
    assert_eq!(
        PropertyValue::PaddingTop(Length::Px(0.0)).key(),
        PropertyKey::PaddingTop
    );
    assert_eq!(
        PropertyValue::PaddingRight(Length::Px(0.0)).key(),
        PropertyKey::PaddingRight
    );
    assert_eq!(
        PropertyValue::PaddingBottom(Length::Px(0.0)).key(),
        PropertyKey::PaddingBottom
    );
    assert_eq!(
        PropertyValue::PaddingLeft(Length::Px(0.0)).key(),
        PropertyKey::PaddingLeft
    );
    assert_eq!(
        PropertyValue::Padding(Sides::all(Length::Px(0.0))).key(),
        PropertyKey::Padding
    );
}

#[test]
fn sides_all_constructor_replicates_value() {
    // Sides::all(v) fills all four fields with v.
    let sides = Sides::all(Length::Px(7.5));
    assert_eq!(sides.top, Length::Px(7.5));
    assert_eq!(sides.right, Length::Px(7.5));
    assert_eq!(sides.bottom, Length::Px(7.5));
    assert_eq!(sides.left, Length::Px(7.5));
}

#[test]
fn padding_shorthand_five_values_dropped_by_leftover() {
    // The helper leaves any fifth or later value unconsumed as leftovers.
    // parse_value alone returns Some for the four-value form, but the caller's
    // expect_exhausted (in rule.rs) detects leftovers and drops the declaration;
    // check the drop through rule.rs.
    let source = "padding: 10px 20px 30px 40px 50px;";
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let decls = crate::rule::parse_declaration_block(&mut parser);
    assert!(
        decls.is_empty(),
        "5-value form must be dropped by expect_exhausted"
    );
}

#[test]
fn padding_case_insensitive_unit() {
    // CSS spec: unit identifiers are ASCII case-insensitive.
    assert_eq!(
        parse("10PX", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(10.0)))
    );
    assert_eq!(
        parse("2EM", "padding-bottom"),
        Some(PropertyValue::PaddingBottom(Length::Em(2.0)))
    );
}

#[test]
fn position_key_maps_to_position_property_key() {
    // PropertyValue::Position → PropertyKey::Position (discriminant integrity for
    // cascade winner selection, following existing sibling counter-* / content /
    // string-set patterns).
    let v = PropertyValue::Position(PositionValue::Static);
    assert_eq!(v.key(), PropertyKey::Position);
    let v = PropertyValue::Position(PositionValue::Running(SmolStr::new("hdr")));
    assert_eq!(v.key(), PropertyKey::Position);
}

// ── overflow-x / overflow-y / overflow (CSS Overflow 3 §3.1) ──
//
// Value grammar (§3.1 spec verbatim): visible | hidden | clip | scroll |
// auto. Initial: visible / Inherited: no. `overflow` shorthand grammar:
// `<'overflow-block'>{1,2}` (mapped to physical x/y — see `OverflowValue`
// doc's Non-goal note).

#[test]
fn overflow_x_parse_all_five_keywords() {
    assert_eq!(
        parse("visible", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Visible))
    );
    assert_eq!(
        parse("hidden", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Hidden))
    );
    assert_eq!(
        parse("clip", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Clip))
    );
    assert_eq!(
        parse("scroll", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Scroll))
    );
    assert_eq!(
        parse("auto", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Auto))
    );
}

#[test]
fn overflow_y_parse_all_five_keywords() {
    // Sibling of `overflow_x_parse_all_five_keywords` — same grammar,
    // separate `PropertyValue` variant / `PropertyKey`.
    assert_eq!(
        parse("visible", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Visible))
    );
    assert_eq!(
        parse("hidden", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Hidden))
    );
    assert_eq!(
        parse("clip", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Clip))
    );
    assert_eq!(
        parse("scroll", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Scroll))
    );
    assert_eq!(
        parse("auto", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Auto))
    );
}

#[test]
fn overflow_is_case_insensitive() {
    assert_eq!(
        parse("HIDDEN", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Hidden))
    );
    assert_eq!(
        parse("Auto", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Auto))
    );
}

#[test]
fn overflow_legacy_overlay_alias_maps_to_auto() {
    // CSS Overflow 3 keeps `overlay` as a legacy alias for `auto`. The alias
    // must normalize identically for both physical longhands and shorthand.
    assert_eq!(
        parse("overlay", "overflow-x"),
        Some(PropertyValue::OverflowX(OverflowValue::Auto))
    );
    assert_eq!(
        parse("OVERLAY", "overflow-y"),
        Some(PropertyValue::OverflowY(OverflowValue::Auto))
    );
    assert_eq!(
        parse("overlay", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY::both(
            OverflowValue::Auto
        )))
    );
}

#[test]
fn overflow_rejects_unknown_keyword() {
    assert_eq!(parse("bogus", "overflow-x"), None);
    assert_eq!(parse("collapse", "overflow-y"), None);
    // `padding-box` etc. are not part of this property's grammar.
    assert_eq!(parse("padding-box", "overflow-x"), None);
}

#[test]
fn overflow_rejects_css_wide_keyword() {
    // (b) not supported — CSS-wide keyword is unimplemented (future work),
    // silent drop (`PropertyValue` doc's "CSS-wide keyword" section is canonical).
    for kw in ["inherit", "initial", "unset", "revert", "revert-layer"] {
        assert_eq!(parse(kw, "overflow-x"), None);
        assert_eq!(parse(kw, "overflow-y"), None);
        assert_eq!(parse(kw, "overflow"), None);
    }
}

#[test]
fn overflow_rejects_non_ident() {
    assert_eq!(parse("16px", "overflow-x"), None);
    assert_eq!(parse(r#""hidden""#, "overflow-y"), None);
}

#[test]
fn overflow_x_key_maps_to_overflow_x_property_key() {
    let v = PropertyValue::OverflowX(OverflowValue::Hidden);
    assert_eq!(v.key(), PropertyKey::OverflowX);
}

#[test]
fn overflow_y_key_maps_to_overflow_y_property_key() {
    let v = PropertyValue::OverflowY(OverflowValue::Scroll);
    assert_eq!(v.key(), PropertyKey::OverflowY);
}

#[test]
fn overflow_key_maps_to_overflow_property_key() {
    let v = PropertyValue::Overflow(OverflowXY::both(OverflowValue::Auto));
    assert_eq!(v.key(), PropertyKey::Overflow);
}

#[test]
fn overflow_shorthand_one_value_spreads_to_both_axes() {
    // §3.1 "If there is only one component value, it applies to all
    // sides" (paraphrase of the shared `<'overflow-block'>{1,2}`
    // expansion rule this crate maps onto physical x/y).
    assert_eq!(
        parse("hidden", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY::both(
            OverflowValue::Hidden
        )))
    );
}

#[test]
fn overflow_shorthand_two_values_set_x_then_y() {
    // §3.1 verbatim: "The overflow property is a shorthand property that
    // sets the specified values of overflow-x and overflow-y in that
    // order."
    assert_eq!(
        parse("hidden scroll", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        }))
    );
}

#[test]
fn overflow_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // Mirrors `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`
    // — this helper consumes only 2 values; a 3rd is left unconsumed for
    // the `expect_exhausted` caller in `rule.rs` to reject the whole
    // declaration. `parse_value` itself does not call `expect_exhausted`,
    // so this direct call only demonstrates the helper's own consumption,
    // not the end-to-end drop (that is `rule.rs`'s job, pinned by
    // `rule::tests::overflow_shorthand_three_values_declaration_dropped`).
    assert_eq!(
        parse("hidden scroll auto", "overflow"),
        Some(PropertyValue::Overflow(OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        }))
    );
}

#[test]
fn z_index_key_maps_to_z_index_property_key() {
    let v = PropertyValue::ZIndex(ZIndexValue::Auto);
    assert_eq!(v.key(), PropertyKey::ZIndex);
    let v = PropertyValue::ZIndex(ZIndexValue::Integer(-1));
    assert_eq!(v.key(), PropertyKey::ZIndex);
}

#[test]
fn z_index_parses_auto_and_integers() {
    assert_eq!(
        parse("auto", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Auto))
    );
    assert_eq!(
        parse("0", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(0)))
    );
    assert_eq!(
        parse("3", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(3)))
    );
    assert_eq!(
        parse("-1", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(-1)))
    );
    assert_eq!(
        parse("2147483647", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-2147483648", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
}

#[test]
fn z_index_clamps_out_of_i32_range() {
    // cssparser 0.37.0 tokenizer.rs:1084-1091 clamps out-of-i32-range integer
    // literals to i32::MAX/MIN rather than wrapping or erroring; parse_z_index
    // delegates to `expect_integer()` so the clamp propagates.
    assert_eq!(
        parse("99999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-99999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
    // Just beyond boundaries also clamp.
    assert_eq!(
        parse("2147483648", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-2147483649", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
    // Very large magnitude (far beyond i32) still clamps.
    assert_eq!(
        parse("99999999999999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MAX)))
    );
    assert_eq!(
        parse("-99999999999999999999", "z-index"),
        Some(PropertyValue::ZIndex(ZIndexValue::Integer(i32::MIN)))
    );
}

// ── resolve_overflow (CSS Overflow 3 §3.1 cross-axis computed-value
// coupling) ──
//
// Spec verbatim: "The visible/clip values of overflow compute to
// auto/hidden (respectively) if one of overflow-x or overflow-y is
// neither visible nor clip."

#[test]
fn resolve_overflow_both_visible_is_unaffected() {
    let pair = OverflowXY::both(OverflowValue::Visible);
    assert_eq!(resolve_overflow(pair), pair);
}

#[test]
fn resolve_overflow_visible_x_computes_to_auto_when_y_is_hidden() {
    let pair = OverflowXY {
        x: OverflowValue::Visible,
        y: OverflowValue::Hidden,
    };
    assert_eq!(
        resolve_overflow(pair),
        OverflowXY {
            x: OverflowValue::Auto,
            y: OverflowValue::Hidden,
        }
    );
}

#[test]
fn resolve_overflow_clip_x_computes_to_hidden_when_y_is_scroll() {
    let pair = OverflowXY {
        x: OverflowValue::Clip,
        y: OverflowValue::Scroll,
    };
    assert_eq!(
        resolve_overflow(pair),
        OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        }
    );
}

#[test]
fn resolve_overflow_visible_and_clip_do_not_gate_each_other() {
    // The gate condition is "the *other* axis is neither visible nor
    // clip" — `visible` and `clip` are each themselves one of the two
    // values the gate exempts, so pairing them together never satisfies
    // the condition for either axis. Both stay as specified.
    let pair = OverflowXY {
        x: OverflowValue::Visible,
        y: OverflowValue::Clip,
    };
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_eq!(
        resolve_overflow(pair),
        pair,
        "visible/clip do not gate each other"
    );
}

#[test]
fn resolve_overflow_non_visible_non_clip_values_pass_through_unchanged() {
    // `hidden`/`scroll`/`auto` are not rewritten by the coupling
    // regardless of the other axis's value (the rule only ever rewrites
    // `visible`/`clip`).
    for this in [
        OverflowValue::Hidden,
        OverflowValue::Scroll,
        OverflowValue::Auto,
    ] {
        for other in [
            OverflowValue::Visible,
            OverflowValue::Hidden,
            OverflowValue::Clip,
            OverflowValue::Scroll,
            OverflowValue::Auto,
        ] {
            let pair = OverflowXY { x: this, y: other };
            assert_eq!(resolve_overflow(pair).x, this);
        }
    }
}

#[test]
fn resolve_overflow_both_non_visible_non_clip_is_unaffected() {
    let pair = OverflowXY {
        x: OverflowValue::Scroll,
        y: OverflowValue::Auto,
    };
    assert_eq!(resolve_overflow(pair), pair);
}

// ── margin longhand + shorthand (CSS Box 3 §3.1/§3.2) ──
//
// Primary source:
// - #margin-physical (§3.1): `<length-percentage> | auto`, initial 0, non-inherited.
// - #margin-shorthand (§3.2): `<'margin-top'>{1,4}` with 1/2/3/4 value expansion.

#[test]
fn margin_top_parse_px() {
    // Verification 3-a: `margin-top: 10px` → MarginTop(Length(Px(10))).
    assert_eq!(
        parse("10px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            10.0
        ))))
    );
}

#[test]
fn margin_right_parse_auto() {
    // Verification 3-b: `margin-right: auto` → MarginRight(Auto). Pin acceptance
    // of the §3.1 `auto` alternative in a per-side longhand.
    assert_eq!(
        parse("auto", "margin-right"),
        Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_bottom_parse_percentage() {
    // Verification 3-c: `margin-bottom: 50%` → MarginBottom(Length(Percent(50))).
    assert_eq!(
        parse("50%", "margin-bottom"),
        Some(PropertyValue::MarginBottom(LengthOrAuto::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn margin_left_parse_em() {
    // Verification 3-d: `margin-left: 2em` → MarginLeft(Length(Em(2))).
    // Accept em through `<length-percentage>` mode (the parse_length_value mode
    // arg = true).
    assert_eq!(
        parse("2em", "margin-left"),
        Some(PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Em(
            2.0
        ))))
    );
}

#[test]
fn margin_side_accepts_negative_length() {
    // Task non-goal: negative margins are valid per spec (§3.1 "Negative values
    // for margin properties are allowed"). Pin their acceptance in longhands too.
    assert_eq!(
        parse("-10px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            -10.0
        ))))
    );
}

#[test]
fn margin_top_accepts_zero() {
    // Spec `<length-percentage> | auto`: zero is a valid length. `0px` takes the Dimension arm;
    // bare `0` takes the Number arm under the CSS Values 3 §5 unitless-zero clause.
    // Margins have no nonnegative filter, so these values pass unchanged.
    assert_eq!(
        parse("0px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            0.0
        ))))
    );
    assert_eq!(
        parse("0", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            0.0
        ))))
    );
}

#[test]
fn margin_side_case_insensitive_auto() {
    // CSS spec: identifier keywords are ASCII case-insensitive. Check acceptance of `AUTO`
    // (as proof that expect_ident_matching ignores case and a canary against
    // regressions if the helper changes).
    assert_eq!(
        parse("AUTO", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_side_rejects_unsupported_unit() {
    // `cap` (§6.1.1 font-relative lengths) remains
    // unsupported (dropped by parse_length_value). `cm` / `lh` / `rlh` have
    // moved to the accepted set. Pin knock-on drops independently of the margin-side helper.
    assert_eq!(parse("1cap", "margin-top"), None);
}

#[test]
fn margin_side_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.
    assert_eq!(
        parse("1lh", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Lh(
            1.0
        ))))
    );
    assert_eq!(
        parse("2rlh", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Rlh(
            2.0
        ))))
    );
}

#[test]
fn margin_side_accepts_absolute_unit() {
    assert_eq!(
        parse("1cm", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Cm(
            1.0
        ))))
    );
}

#[test]
fn margin_side_rejects_bogus_ident() {
    // Drop identifiers outside the `<length-percentage> | auto` grammar.
    assert_eq!(parse("fill-available", "margin-top"), None);
    assert_eq!(parse("initial", "margin-top"), None);
}

#[test]
fn margin_shorthand_one_value_spreads_all_sides() {
    // Verification 4-a (§3.2 "If there is only one component value, it
    // applies to all sides"): `margin: 10px` sets all four sides to 10px.
    let want = Sides::all(LengthOrAuto::Length(Length::Px(10.0)));
    assert_eq!(parse("10px", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_two_values_top_bottom_and_right_left() {
    // Verification 4-b (§3.2 "If there are two values, the top and bottom
    // margins are set to the first value and the right and left margins
    // are set to the second"): top/bottom = 10px, right/left = 20px.
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(10.0)),
        left: LengthOrAuto::Length(Length::Px(20.0)),
    };
    assert_eq!(
        parse("10px 20px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_three_values_top_horiz_bottom() {
    // Verification 4-c (§3.2 "If there are three values, the top is set to
    // the first value, the left and right are set to the second, and the
    // bottom is set to the third"): top = 10px, right/left = 20px,
    // bottom = 30px.
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(30.0)),
        left: LengthOrAuto::Length(Length::Px(20.0)),
    };
    assert_eq!(
        parse("10px 20px 30px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_four_values_clockwise() {
    // Verification 4-d (§3.2 "If there are four values they apply to the
    // top, right, bottom, and left, respectively"): clockwise from top.
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(30.0)),
        left: LengthOrAuto::Length(Length::Px(40.0)),
    };
    assert_eq!(
        parse("10px 20px 30px 40px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_all_auto() {
    // Verification 5-a: `margin: auto` (one auto value) sets all four sides to Auto.
    // Pin parsing of the browser convention for "block-level centering".
    let want = Sides::all(LengthOrAuto::Auto);
    assert_eq!(parse("auto", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_zero_and_auto_horizontal_center() {
    // Verification 5-b: `margin: 0 auto` (two mixed values) is the canonical form
    // for block-level horizontal centering: top/bottom = 0px, right/left = auto.
    // Under the unitless-zero clause of CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>,
    // accept bare `0` as Length::Px(0.0)
    // (`parse_length_value_accepts_unitless_zero_only`
    // (also pinned here).
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(0.0)),
        right: LengthOrAuto::Auto,
        bottom: LengthOrAuto::Length(Length::Px(0.0)),
        left: LengthOrAuto::Auto,
    };
    assert_eq!(parse("0 auto", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_mixed_units() {
    // Grammar coverage: mix units and auto across all four shorthand values.
    // Cover the 32n `<length-percentage> | auto` grammar in one assertion.
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Auto,
        bottom: LengthOrAuto::Length(Length::Percent(50.0)),
        left: LengthOrAuto::Length(Length::Em(2.0)),
    };
    assert_eq!(
        parse("10px auto 50% 2em", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_rejects_empty_input() {
    // Empty value: parse_margin_side fails on the first value, and `?` in
    // parse_margin_shorthand propagates None (drop the declaration).
    assert_eq!(parse("", "margin"), None);
}

#[test]
fn margin_shorthand_rejects_bogus_ident() {
    // Drop an identifier outside the grammar in the first-value position.
    assert_eq!(parse("bogus", "margin"), None);
}

#[test]
fn margin_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // For shorthand with five or more values, this helper consumes four and returns
    // with the fifth and later values unconsumed. DeclParser::parse_value's
    // expect_exhausted ultimately drops the declaration, so a rule.rs test
    // (`margin_shorthand_five_values_declaration_dropped`) checks end-to-end
    // behavior. This test pins that parse_value alone (without the caller's
    // expect_exhausted) returns Some for the first four values.
    let want = Sides {
        top: LengthOrAuto::Length(Length::Px(10.0)),
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Length(Length::Px(30.0)),
        left: LengthOrAuto::Length(Length::Px(40.0)),
    };
    assert_eq!(
        parse("10px 20px 30px 40px 50px", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_shorthand_case_insensitive_auto_and_units() {
    // Pin case-insensitive dispatch along the shorthand path too.
    let want = Sides {
        top: LengthOrAuto::Auto,
        right: LengthOrAuto::Length(Length::Px(20.0)),
        bottom: LengthOrAuto::Auto,
        left: LengthOrAuto::Length(Length::Px(20.0)),
    };
    assert_eq!(
        parse("AUTO 20PX", "margin"),
        Some(PropertyValue::Margin(want))
    );
}

#[test]
fn margin_longhand_keys_map_correctly() {
    // Four longhands and one shorthand map to their respective keys (pinning
    // cascade winner selection and sibling `position_key_maps_to_position_property_key`
    // discriminant integrity). Include the shorthand
    // `Margin` key, observable on PropertyValue before expansion.
    let top = PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(top.key(), PropertyKey::MarginTop);
    let right = PropertyValue::MarginRight(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(right.key(), PropertyKey::MarginRight);
    let bottom = PropertyValue::MarginBottom(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(bottom.key(), PropertyKey::MarginBottom);
    let left = PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(1.0)));
    assert_eq!(left.key(), PropertyKey::MarginLeft);
    let shorthand = PropertyValue::Margin(Sides::all(LengthOrAuto::Length(Length::Px(1.0))));
    assert_eq!(shorthand.key(), PropertyKey::Margin);
}

#[test]
fn sides_all_spreads_value_to_all_four() {
    // Sides::all helper ( reused by shorthand 1-value + initial value):
    // One value sets top/right/bottom/left alike. Pin correct behavior of the
    // Clone path (the last side consumes the moved value).
    let s = Sides::all(LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.top, LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.right, LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.bottom, LengthOrAuto::Length(Length::Px(3.5)));
    assert_eq!(s.left, LengthOrAuto::Length(Length::Px(3.5)));
}

// ── CSS Logical Properties and Values 1 §4.2/§4.4 margin-inline-*/
//    margin-block-*/padding-inline-*/padding-block-* longhand +
//    margin-inline/margin-block/padding-inline/padding-block shorthand ──
//
// Physical fixed-mapping rationale (writing-mode always HorizontalTb
// since raikiri does not implement a vertical-writing rendering
// pipeline, inline axis additionally assumes `direction: ltr`) is
// `PropertyValue::PaddingInline` doc's canonical record — not repeated
// per test here.

#[test]
fn margin_inline_start_parses_to_margin_left() {
    // §4.2 physical fixed-mapping: `margin-inline-start` produces the
    // exact same `PropertyValue` as `margin-left` (no dedicated variant
    // — see the "Why the eight longhands do not have dedicated
    // variants" section of the `PropertyValue::PaddingInline` doc).
    assert_eq!(
        parse("5px", "margin-inline-start"),
        Some(PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(
            5.0
        ))))
    );
}

#[test]
fn margin_inline_end_parses_to_margin_right() {
    assert_eq!(
        parse("auto", "margin-inline-end"),
        Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_block_start_parses_to_margin_top() {
    assert_eq!(
        parse("5px", "margin-block-start"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            5.0
        ))))
    );
}

#[test]
fn margin_block_end_parses_to_margin_bottom() {
    assert_eq!(
        parse("auto", "margin-block-end"),
        Some(PropertyValue::MarginBottom(LengthOrAuto::Auto))
    );
}

#[test]
fn padding_inline_start_parses_to_padding_left() {
    assert_eq!(
        parse("5px", "padding-inline-start"),
        Some(PropertyValue::PaddingLeft(Length::Px(5.0)))
    );
}

#[test]
fn padding_inline_end_parses_to_padding_right() {
    assert_eq!(
        parse("5%", "padding-inline-end"),
        Some(PropertyValue::PaddingRight(Length::Percent(5.0)))
    );
}

#[test]
fn padding_block_start_parses_to_padding_top() {
    assert_eq!(
        parse("5px", "padding-block-start"),
        Some(PropertyValue::PaddingTop(Length::Px(5.0)))
    );
}

#[test]
fn padding_block_end_parses_to_padding_bottom() {
    assert_eq!(
        parse("5px", "padding-block-end"),
        Some(PropertyValue::PaddingBottom(Length::Px(5.0)))
    );
}

#[test]
fn padding_inline_block_start_end_reject_negative() {
    // CSS Box 3 §4.1 "Negative values for padding properties are
    // invalid." applies identically here (`parse_padding_side` reuse).
    assert_eq!(parse("-5px", "padding-inline-start"), None);
    assert_eq!(parse("-5px", "padding-inline-end"), None);
    assert_eq!(parse("-5px", "padding-block-start"), None);
    assert_eq!(parse("-5px", "padding-block-end"), None);
}

#[test]
fn margin_inline_shorthand_one_value_spreads_to_start_and_end() {
    // CSS Logical Properties and Values 1 §4.2 "If only one value is
    // given, it applies to both the start and end edges."
    assert_eq!(
        parse("12px", "margin-inline"),
        Some(PropertyValue::MarginInline(StartEnd::both(
            LengthOrAuto::Length(Length::Px(12.0))
        )))
    );
}

#[test]
fn margin_inline_shorthand_two_value() {
    assert_eq!(
        parse("5px auto", "margin-inline"),
        Some(PropertyValue::MarginInline(StartEnd {
            start: LengthOrAuto::Length(Length::Px(5.0)),
            end: LengthOrAuto::Auto,
        }))
    );
}

#[test]
fn margin_block_shorthand_two_value() {
    assert_eq!(
        parse("auto 5px", "margin-block"),
        Some(PropertyValue::MarginBlock(StartEnd {
            start: LengthOrAuto::Auto,
            end: LengthOrAuto::Length(Length::Px(5.0)),
        }))
    );
}

#[test]
fn padding_inline_shorthand_one_value_spreads_to_start_and_end() {
    assert_eq!(
        parse("12px", "padding-inline"),
        Some(PropertyValue::PaddingInline(StartEnd::both(Length::Px(
            12.0
        ))))
    );
}

#[test]
fn padding_inline_shorthand_two_value() {
    assert_eq!(
        parse("5px 10px", "padding-inline"),
        Some(PropertyValue::PaddingInline(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(10.0),
        }))
    );
}

#[test]
fn padding_block_shorthand_two_value() {
    assert_eq!(
        parse("5px 10px", "padding-block"),
        Some(PropertyValue::PaddingBlock(StartEnd {
            start: Length::Px(5.0),
            end: Length::Px(10.0),
        }))
    );
}

#[test]
fn padding_inline_shorthand_rejects_negative_component() {
    // `padding-inline: 10px -5px` — same shape as
    // `padding_shorthand_rejects_any_negative_value`: the 2nd component
    // (`-5px`) fails §4.1's `[0,∞]` constraint, `try_parse` rewinds, and
    // `parse_padding_logical_shorthand` returns the 1-value form
    // (`Some(StartEnd::both(10px))`) with `-5px` left unconsumed — it is
    // the caller's `expect_exhausted` (`crate::rule::parse_declaration_block`)
    // that detects the leftover token and drops the whole declaration.
    // At the bare `parse_value` level (this file's `parse` test helper,
    // which never runs `expect_exhausted`), the 1-value form is the
    // *correct* observed value, not a bug — pinned directly below so a
    // reader doesn't mistake it for one.
    assert_eq!(
        parse("10px -5px", "padding-inline"),
        Some(PropertyValue::PaddingInline(StartEnd::both(Length::Px(
            10.0
        ))))
    );
    let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding-inline: 10px -5px;",
    )));
    // cov:ignore: the failure-message branch of this `assert!` only
    // executes when the assertion fails; it passes here, so llvm-cov
    // reports the macro's condition-false region as an uncovered added
    // line even though the assertion itself runs and does its job.
    assert!(
        decls.is_empty(),
        "`padding-inline: 10px -5px` must drop via expect_exhausted leftover"
    );
    // 1st component negative — no rewind opportunity, `?` propagates
    // `None` directly from `parse_padding_logical_shorthand` itself.
    assert_eq!(parse("-5px 10px", "padding-inline"), None);
}

#[test]
fn margin_inline_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // 3rd+ value: helper consumes only 2, leftover is unconsumed (caller
    // `expect_exhausted` drops the whole declaration at the rule.rs
    // layer — same shape as `margin_shorthand_leaves_extra_values_for_caller_exhausted_check`).
    assert_eq!(
        parse("5px 10px 15px", "margin-inline"),
        Some(PropertyValue::MarginInline(StartEnd {
            start: LengthOrAuto::Length(Length::Px(5.0)),
            end: LengthOrAuto::Length(Length::Px(10.0)),
        }))
    );
}

#[test]
fn logical_margin_padding_keys_map_to_their_physical_counterparts() {
    // Discriminant integrity for cascade winner selection: the eight longhands
    // map to physical keys rather than dedicated keys (as in
    // `margin_longhand_keys_map_correctly`); the four shorthands each have their own key.
    assert_eq!(
        PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Px(1.0))).key(),
        PropertyKey::MarginLeft
    );
    assert_eq!(
        PropertyValue::MarginInline(StartEnd::both(LengthOrAuto::Length(Length::Px(1.0)))).key(),
        PropertyKey::MarginInline
    );
    assert_eq!(
        PropertyValue::MarginBlock(StartEnd::both(LengthOrAuto::Length(Length::Px(1.0)))).key(),
        PropertyKey::MarginBlock
    );
    assert_eq!(
        PropertyValue::PaddingInline(StartEnd::both(Length::Px(1.0))).key(),
        PropertyKey::PaddingInline
    );
    assert_eq!(
        PropertyValue::PaddingBlock(StartEnd::both(Length::Px(1.0))).key(),
        PropertyKey::PaddingBlock
    );
}

#[test]
fn logical_margin_padding_property_names_resolve_to_physical_property_keys() {
    // `property_key_for_name` — the deferred (`var()`) path's key
    // lookup (`parse_value`'s deferred-detection branch) must agree with
    // `parse_value`'s own non-deferred arm for every logical longhand,
    // or `resolve_deferred_value`'s `value.key() == key` optimized path
    // (`cascade::project_deferred_value` doc) silently breaks.
    assert_eq!(
        property_key_for_name("margin-inline-start"),
        Some(PropertyKey::MarginLeft)
    );
    assert_eq!(
        property_key_for_name("margin-inline-end"),
        Some(PropertyKey::MarginRight)
    );
    assert_eq!(
        property_key_for_name("margin-block-start"),
        Some(PropertyKey::MarginTop)
    );
    assert_eq!(
        property_key_for_name("margin-block-end"),
        Some(PropertyKey::MarginBottom)
    );
    assert_eq!(
        property_key_for_name("padding-inline-start"),
        Some(PropertyKey::PaddingLeft)
    );
    assert_eq!(
        property_key_for_name("padding-inline-end"),
        Some(PropertyKey::PaddingRight)
    );
    assert_eq!(
        property_key_for_name("padding-block-start"),
        Some(PropertyKey::PaddingTop)
    );
    assert_eq!(
        property_key_for_name("padding-block-end"),
        Some(PropertyKey::PaddingBottom)
    );
    assert_eq!(
        property_key_for_name("margin-inline"),
        Some(PropertyKey::MarginInline)
    );
    assert_eq!(
        property_key_for_name("margin-block"),
        Some(PropertyKey::MarginBlock)
    );
    assert_eq!(
        property_key_for_name("padding-inline"),
        Some(PropertyKey::PaddingInline)
    );
    assert_eq!(
        property_key_for_name("padding-block"),
        Some(PropertyKey::PaddingBlock)
    );
}

// ── border longhand + shorthand (CSS Backgrounds 3 §3) ──

#[test]
fn border_top_width_parse_px() {
    // Verification #1: parse("1px", "border-top-width") =
    // Some(PropertyValue::BorderTopWidth(Length::Px(1.0))).
    assert_eq!(
        parse("1px", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))
    );
}

// ── width (CSS Sizing 3 §3.1.1) ────────────────────────
//
// Primary source:
// https://www.w3.org/TR/css-sizing-3/#preferred-size-properties
// Value: `auto | <length-percentage [0,∞]> | min-content | max-content |
//         fit-content(<length-percentage>)`
// Initial: auto, Inherited: no.
//
// This task accepts only `auto` and nonnegative `<length-percentage>`;
// min-content / max-content / fit-content() remain unsupported (b).

#[test]
fn width_parse_auto_keyword() {
    // Verification #1: `width: auto` → Width(Auto). It has the same shape as the initial
    // value. Pin the priority branch of the grammar (parse_width's try_parse
    // identifier branch).
    assert_eq!(
        parse("auto", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn border_top_width_parse_medium_keyword() {
    // Verification #2: parse("medium", "border-top-width") =
    // Some(PropertyValue::BorderTopWidth(Length::Px(3.0))).
    // Pin the normative §3.3 mapping of thin/medium/thick to 1/3/5 px.
    assert_eq!(
        parse("medium", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))
    );
}

#[test]
fn width_parse_length_px() {
    // Verification #2: `width: 100px` → Width(Length(Px(100))).
    // Check the transition from returning `None` in the old
    // `unknown_property_returns_none` canary to returning a real variant (the
    // canary subsequently moved through `float` to `cursor`).
    assert_eq!(
        parse("100px", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(
            100.0
        ))))
    );
}

#[test]
fn border_width_thin_thick_keywords_map_to_1px_5px() {
    // Spec §3.3 values: thin=1px, thick=5px.
    // Smoke-test each arm for four sides to catch cross-arm wiring mistakes
    // (e.g. wrongly wiring the `top` arm to the `right` arm).
    assert_eq!(
        parse("thin", "border-right-width"),
        Some(PropertyValue::BorderRightWidth(Length::Px(1.0)))
    );
    assert_eq!(
        parse("thick", "border-left-width"),
        Some(PropertyValue::BorderLeftWidth(Length::Px(5.0)))
    );
}

#[test]
fn width_parse_length_percentage() {
    // Verification #3: `width: 50%` → Width(Length(Percent(50))).
    // The Percent branch through parse_length_value(allow_percentage=true).
    assert_eq!(
        parse("50%", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Percent(
            50.0
        ))))
    );
}

#[test]
fn border_width_rejects_negative() {
    // Verification #6: parse("-1px", "border-top-width") = None.
    // Spec `<line-width>` = `<length [0,∞]>`; negatives violate grammar → drop.
    assert_eq!(parse("-1px", "border-top-width"), None);
    // Em / Rem / Pt have the same constraint (all unit-bearing variants).
    assert_eq!(parse("-1em", "border-bottom-width"), None);
}

#[test]
fn border_top_width_accepts_zero() {
    // In spec `<line-width>` = `<length [0,∞]>`, zero is the closed interval's lower bound.
    // `0px` takes the Dimension arm; bare `0` takes the Number arm via
    // CSS Values 3 §5's unitless-zero clause. It passes parse_border_width_side's
    // `>= 0.0` nonnegative filter.
    assert_eq!(
        parse("0px", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(0.0)))
    );
    assert_eq!(
        parse("0", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(0.0)))
    );
}

#[test]
fn border_shorthand_accepts_bare_zero_width() {
    // Follow-on coverage: `0 solid`
    // uses bare zero in the shorthand width slot, a canonical form.
    // parse_border_shorthand's width slot calls parse_border_width_side_res,
    // which gets Length::Px(0.0) via parse_length_value's Number arm;
    // the style slot is Solid; omitted color takes the spec initial value of
    // `BorderColor::CurrentColor` (CSS Backgrounds 3 §3.1).
    let border = Border {
        width: Length::Px(0.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("0 solid", "border"),
        Some(PropertyValue::Border(Sides::all(border)))
    );
}

#[test]
fn border_width_rejects_percentage() {
    // The `<line-width>` grammar excludes `<percentage>` (unlike padding).
    // In `parse_length_value(input, false)`, Percentage tokens themselves
    // are rejected in `<length>` mode.
    assert_eq!(parse("50%", "border-top-width"), None);
}

#[test]
fn border_width_rejects_unknown_keyword() {
    // Drop keywords outside the §3.3 set (`auto` / `fat` / `bold`, etc.).
    assert_eq!(parse("auto", "border-top-width"), None);
    assert_eq!(parse("fat", "border-top-width"), None);
}

#[test]
fn border_width_accepts_css_wide_keyword() {
    // CSS Cascading 4 §7.3 + CSS Cascading 5 §7.3.5: every `border-*-width` longhand
    // accepts the five CSS-wide keywords as a lone value (see `CssWideKeyword`).
    assert_eq!(
        parse("inherit", "border-top-width"),
        Some(PropertyValue::BorderTopWidthCssWide(
            CssWideKeyword::Inherit
        ))
    );
    assert_eq!(
        parse("initial", "border-top-width"),
        Some(PropertyValue::BorderTopWidthCssWide(
            CssWideKeyword::Initial
        ))
    );
    assert_eq!(
        parse("unset", "border-top-width"),
        Some(PropertyValue::BorderTopWidthCssWide(CssWideKeyword::Unset))
    );
    assert_eq!(
        parse("revert", "border-top-width"),
        Some(PropertyValue::BorderTopWidthCssWide(CssWideKeyword::Revert))
    );
    assert_eq!(
        parse("revert-layer", "border-top-width"),
        Some(PropertyValue::BorderTopWidthCssWide(
            CssWideKeyword::RevertLayer
        ))
    );
    // ASCII case-insensitive (CSS Values 3 §3.1).
    assert_eq!(
        parse("INHERIT", "border-right-width"),
        Some(PropertyValue::BorderRightWidthCssWide(
            CssWideKeyword::Inherit
        ))
    );
}

#[test]
fn border_width_accepts_absolute_unit() {
    // The `<length [0,∞]>` half of `<line-width>` excludes `<percentage>` but
    // includes other absolute units. Check the newly added `pc` through
    // the border-width path (`allow_percentage=false`) as well.
    assert_eq!(
        parse("1pc", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Pc(1.0)))
    );
}

#[test]
fn border_width_rejects_negative_absolute_unit() {
    // Check nonnegative filtering across unit-bearing variants (cm, new absolute unit).
    assert_eq!(parse("-1cm", "border-top-width"), None);
}

#[test]
fn border_width_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.`<line-width>`
    // grammar (`<length [0,∞]> | thin | medium | thick`) has no
    // self-reference concern the way `font-size` / `line-height` do
    // (`Length::Lh` doc), so `border-*-width` accepts them unfiltered.
    assert_eq!(
        parse("2lh", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Lh(2.0)))
    );
    assert_eq!(
        parse("1rlh", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Rlh(1.0)))
    );
}

#[test]
fn border_style_shorthand_expansion_1_to_4_values() {
    use PropertyValue::BorderStyle as BS;
    // 1 value → all sides.
    assert_eq!(
        parse("double", "border-style"),
        Some(BS(Sides::all(BorderStyle::Double)))
    );
    // 2 values → vertical / horizontal.
    assert_eq!(
        parse("solid dotted", "border-style"),
        Some(BS(Sides {
            top: BorderStyle::Solid,
            right: BorderStyle::Dotted,
            bottom: BorderStyle::Solid,
            left: BorderStyle::Dotted,
        }))
    );
    // 4 values → clockwise.
    assert_eq!(
        parse("solid dotted dashed double", "border-style"),
        Some(BS(Sides {
            top: BorderStyle::Solid,
            right: BorderStyle::Dotted,
            bottom: BorderStyle::Dashed,
            left: BorderStyle::Double,
        }))
    );
    // Invalid keyword drops the whole declaration (exhaustion
    // enforced by the caller — `parse_entire` mirrors DeclParser).
    assert_eq!(parse_entire("solid wavy", "border-style"), None);
    assert_eq!(parse("", "border-style"), None);
}

#[test]
fn border_width_shorthand_keywords_and_lengths() {
    use PropertyValue::BorderWidth as BW;
    assert_eq!(
        parse("medium", "border-width"),
        Some(BW(Sides::all(Length::Px(BORDER_WIDTH_MEDIUM_PX))))
    );
    assert_eq!(
        parse("1px 2px", "border-width"),
        Some(BW(Sides {
            top: Length::Px(1.0),
            right: Length::Px(2.0),
            bottom: Length::Px(1.0),
            left: Length::Px(2.0),
        }))
    );
    // Negative lengths are grammar violations.
    assert_eq!(parse("-1px", "border-width"), None);
}

#[test]
fn border_color_shorthand_currentcolor_and_named() {
    use PropertyValue::BorderColor as BC;
    assert_eq!(
        parse("black", "border-color"),
        Some(BC(Sides::all(BorderColor::Resolved(CssColor::BLACK))))
    );
    assert_eq!(
        parse("currentcolor", "border-color"),
        Some(BC(Sides::all(BorderColor::CurrentColor)))
    );
}

#[test]
fn border_top_style_parse_solid() {
    // Verification #3: parse("solid", "border-top-style") =
    // Some(PropertyValue::BorderTopStyle(BorderStyle::Solid)).
    assert_eq!(
        parse("solid", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
    );
}

#[test]
fn width_parse_length_em() {
    // Check the font-relative unit path: accept em via parse_length_value.
    assert_eq!(
        parse("2em", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Em(2.0))))
    );
}

#[test]
fn border_style_all_10_variants_accepted() {
    // Smoke-test all ten alternatives in spec §3.2 `<line-style>` (catch
    // regressions that remove an arm).
    assert_eq!(
        parse("none", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::None))
    );
    assert_eq!(
        parse("hidden", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Hidden))
    );
    assert_eq!(
        parse("dotted", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Dotted))
    );
    assert_eq!(
        parse("dashed", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Dashed))
    );
    assert_eq!(
        parse("double", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Double))
    );
    assert_eq!(
        parse("groove", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Groove))
    );
    assert_eq!(
        parse("ridge", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Ridge))
    );
    assert_eq!(
        parse("inset", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Inset))
    );
    assert_eq!(
        parse("outset", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Outset))
    );
}

#[test]
fn width_rejects_negative_px() {
    // Verification #4: `width: -10px` → None (spec grammar `[0,∞]` violation).
    // The parse_width post-filter enforces this (as for padding).
    assert_eq!(parse("-10px", "width"), None);
}

#[test]
fn width_rejects_negative_percentage() {
    // Check the Percent branch of the nonnegative Length OR-pattern.
    assert_eq!(parse("-50%", "width"), None);
}

#[test]
fn width_rejects_negative_em() {
    // Check the Em branch of the nonnegative Length OR-pattern.
    assert_eq!(parse("-2em", "width"), None);
}

#[test]
fn width_accepts_zero() {
    // Spec `[0,∞]` is a closed interval; its lower bound, zero, is valid.
    assert_eq!(
        parse("0px", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
    );
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>
    // Under the unitless-zero clause, bare `0` is also accepted as Px(0.0)
    // (width uses `<length-percentage [0,∞]>`; the helper gets it via the Number arm
    // and it passes the parse_width nonnegative filter).
    assert_eq!(
        parse("0", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
    );
}

#[test]
fn border_style_rejects_unknown_keyword() {
    // Drop values outside `<line-style>` grammar (`wavy` comes from CSS Text Decoration 4,
    // not border-style).
    assert_eq!(parse("wavy", "border-top-style"), None);
}

#[test]
fn border_style_accepts_css_wide_keyword() {
    // CSS Cascading 4 §7.3 + CSS Cascading 5 §7.3.5: every `border-*-style` longhand
    // accepts the five CSS-wide keywords as a lone value (see `CssWideKeyword`).
    assert_eq!(
        parse("inherit", "border-top-style"),
        Some(PropertyValue::BorderTopStyleCssWide(
            CssWideKeyword::Inherit
        ))
    );
    assert_eq!(
        parse("initial", "border-top-style"),
        Some(PropertyValue::BorderTopStyleCssWide(
            CssWideKeyword::Initial
        ))
    );
    assert_eq!(
        parse("unset", "border-top-style"),
        Some(PropertyValue::BorderTopStyleCssWide(CssWideKeyword::Unset))
    );
    assert_eq!(
        parse("revert", "border-top-style"),
        Some(PropertyValue::BorderTopStyleCssWide(CssWideKeyword::Revert))
    );
    assert_eq!(
        parse("revert-layer", "border-top-style"),
        Some(PropertyValue::BorderTopStyleCssWide(
            CssWideKeyword::RevertLayer
        ))
    );
    assert_eq!(
        parse("INHERIT", "border-right-style"),
        Some(PropertyValue::BorderRightStyleCssWide(
            CssWideKeyword::Inherit
        ))
    );
}

#[test]
fn border_style_case_insensitive() {
    // CSS Values 3 §3.1: keywords are ASCII case-insensitive.
    assert_eq!(
        parse("SOLID", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
    );
}

#[test]
fn width_and_inline_size_keep_the_min_content_keyword() {
    // CSS Sizing 3 §3.1.1 intrinsic sizing keyword, kept for layout.
    assert_eq!(
        parse("min-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::MinContent))
    );
    assert_eq!(
        parse("MIN-CONTENT", "inline-size"),
        Some(PropertyValue::InlineSize(LengthOrAuto::MinContent))
    );
    // Height and the min/max sizes still fold the keyword into `auto`.
    assert_eq!(
        parse("min-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn width_rejects_max_content_keyword() {
    assert_eq!(
        parse("max-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn width_rejects_fit_content_function() {
    assert_eq!(
        parse("fit-content(50%)", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("fit-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn width_rejects_unsupported_unit() {
    // (b) Unsupported: cqw / cap and similar units are valid per spec but
    // not implemented. parse_length_value drops them and propagates None.
    // `ch` / `lh` / `rlh` and the viewport units have
    // moved to the accepted set.
    // (see `width_accepts_absolute_unit` / `width_accepts_lh`).
    assert_eq!(parse("10cqw", "width"), None);
    assert_eq!(parse("5cap", "width"), None);
}

#[test]
fn width_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.
    assert_eq!(
        parse("5lh", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Lh(5.0))))
    );
    assert_eq!(
        parse("1rlh", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Rlh(1.0))))
    );
}

#[test]
fn width_accepts_absolute_unit() {
    // `1in` equals 96px; the specified layer preserves the authored unit,
    // and resolve.rs handles absolute conversion (see `resolve::tests::length_additional_absolute_units_convert_per_spec_table`).
    assert_eq!(
        parse("1in", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::In(1.0))))
    );
}

#[test]
fn width_case_insensitive_auto() {
    // CSS spec: identifier keywords are ASCII case-insensitive. Check acceptance of `AUTO`
    // (as in sibling `margin_side_case_insensitive_auto`).
    assert_eq!(
        parse("AUTO", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn border_top_color_parse_hex() {
    // For border-*-color hex forms, `parse_color` (the same helper used by
    // background-color) accepts hex/named/rgb(a)/transparent, and `parse_border_color`
    // wraps the result in `BorderColor::Resolved` for the static cascade layer.
    assert_eq!(
        parse("#ff0000", "border-top-color"),
        Some(PropertyValue::BorderTopColor(BorderColor::Resolved(
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        )))
    );
}

#[test]
fn border_color_named_and_rgb() {
    // Distribute a smoke test for each of four side arms across three color forms
    // (named / rgb / transparent) to detect cross-arm regressions (background-color pattern).
    // `BorderColor::Resolved` wrap.
    assert_eq!(
        parse("red", "border-right-color"),
        Some(PropertyValue::BorderRightColor(BorderColor::Resolved(
            CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        )))
    );
    assert_eq!(
        parse("rgb(0, 0, 255)", "border-bottom-color"),
        Some(PropertyValue::BorderBottomColor(BorderColor::Resolved(
            CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255
            }
        )))
    );
    assert_eq!(
        parse("transparent", "border-left-color"),
        Some(PropertyValue::BorderLeftColor(BorderColor::Resolved(
            CssColor::TRANSPARENT
        )))
    );
}

#[test]
fn border_top_color_parse_currentcolor() {
    // CSS Backgrounds 3 §3.1 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
    // "Initial: currentcolor": check that an explicit author declaration of
    // `border-*-color: currentcolor` remains a `BorderColor::CurrentColor` variant
    // (cascade-side coverage for hazard case 1; used-value resolution belongs to
    // painting).
    assert_eq!(
        parse("currentcolor", "border-top-color"),
        Some(PropertyValue::BorderTopColor(BorderColor::CurrentColor))
    );
    // CSS Color 3 §4.4 keywords are ASCII case-insensitive.
    assert_eq!(
        parse("CurrentColor", "border-right-color"),
        Some(PropertyValue::BorderRightColor(BorderColor::CurrentColor))
    );
    assert_eq!(
        parse("CURRENTCOLOR", "border-bottom-color"),
        Some(PropertyValue::BorderBottomColor(BorderColor::CurrentColor))
    );
}

#[test]
fn border_shorthand_all_three_components() {
    // Verification #5: parse("1px solid red", "border") expands a shorthand into
    // Border {width: 1px, style: Solid, color: red} for all four sides.
    // Wrap the color slot in `BorderColor::Resolved`.
    let border = Border {
        width: Length::Px(1.0),
        style: BorderStyle::Solid,
        color: BorderColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
    };
    assert_eq!(
        parse("1px solid red", "border"),
        Some(PropertyValue::Border(Sides::all(border)))
    );
}

#[test]
fn border_shorthand_any_order() {
    // Spec §3.4 grammar uses `||` (any order). Rather than all six permutations,
    // smoke-test three orders (color-first / style-first / mixed).
    let expected = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
    };
    // color first
    assert_eq!(
        parse("red 2px dashed", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
    // style first
    assert_eq!(
        parse("dashed 2px red", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
}

#[test]
fn border_shorthand_omitted_components_use_initial() {
    // spec §3.4 "Omitted values are set to their initial values" —
    // Omitted width → medium (3px); omitted style → None; omitted color →
    // `currentcolor` keyword (`BorderColor::CurrentColor`, spec §3.1
    // initial).
    // Only one component (color): width and style use their initial values:
    let with_only_color = Border {
        width: Length::Px(3.0), // medium initial
        style: BorderStyle::None,
        color: BorderColor::Resolved(CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }),
    };
    assert_eq!(
        parse("red", "border"),
        Some(PropertyValue::Border(Sides::all(with_only_color)))
    );
    // Only one component (style): width and color use their initial values:
    let with_only_style = Border {
        width: Length::Px(3.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    assert_eq!(
        parse("solid", "border"),
        Some(PropertyValue::Border(Sides::all(with_only_style)))
    );
}

#[test]
fn border_width_medium_is_consistent_across_its_independent_call_sites() {
    // Before this fix, `medium` = 3px was written
    // as 3 independent `Length::Px(3.0)` literals — the `medium` keyword
    // branch in `parse_border_width_side`, the border shorthand's
    // omitted-width default in `parse_border_shorthand`, and
    // `crate::specified::INITIAL_BORDER`'s `width` field — with no test
    // tying them together, so they could silently drift apart. All 3 now
    // derive from `BORDER_WIDTH_MEDIUM_PX`; this test exercises all 3
    // through real behavior (not literal-vs-literal) and pins that they
    // still agree with each other and with the const, so a future edit
    // that touches only one of them fails loudly here instead of
    // drifting silently. The sibling tests
    // `border_top_width_parse_medium_keyword` and
    // `border_shorthand_omitted_components_use_initial` independently
    // check the *absolute* value (`3.0`) as a literal — do not fold those
    // into a reference to the const, or nothing catches an accidental
    // edit to the const itself (see the const's doc).
    let via_keyword = parse("medium", "border-top-width");
    assert_eq!(
        via_keyword,
        Some(PropertyValue::BorderTopWidth(Length::Px(
            BORDER_WIDTH_MEDIUM_PX
        )))
    );

    let via_shorthand_omission = parse("solid", "border");
    // cov:ignore: this let-else panic branch is unreached as long as the
    // test passes — `parse("solid", "border")` always matches
    // `Some(PropertyValue::Border(_))`, so llvm-cov marks the panic-message
    // literal "uncovered" the same way it does for any other panic-only
    // branch (same false-positive class as r7r1).
    let Some(PropertyValue::Border(sides)) = via_shorthand_omission else {
        panic!("expected `border: solid` to parse to a Border shorthand value");
    };
    assert_eq!(sides.top.width, Length::Px(BORDER_WIDTH_MEDIUM_PX));

    assert_eq!(
        crate::specified::INITIAL_BORDER.width,
        Length::Px(BORDER_WIDTH_MEDIUM_PX)
    );
}

#[test]
fn border_default_matches_initial_border() {
    // `Border::default()` (public, umbrella-facing
    // constructor) and `crate::specified::INITIAL_BORDER` (`pub(crate)`,
    // cascade-internal optimized path) encode the same CSS Backgrounds 3
    // initial value. Precision on what this actually catches (the sibling
    // test just above, `border_width_medium_is_consistent_across_its_independent_call_sites`,
    // warns explicitly against a "vacuous pin" of this shape):
    //
    // - `style` / `color`: each side hardcodes `BorderStyle::None` /
    //   `BorderColor::CurrentColor` independently (no shared constant), so
    //   this assert is a real independent-literal drift check for those 2
    //   fields — same rationale as the sibling test.
    // - `width`: both sides already read `BORDER_WIDTH_MEDIUM_PX` (this
    //   fn's own body and `INITIAL_BORDER`'s definition), so an edit to
    //   that const moves both sides together and this assert alone would
    //   NOT catch it — that drift is what the sibling test's real,
    //   behavior-driven exercise of the const (plus
    //   `border_top_width_parse_medium_keyword`'s absolute-value literal
    //   pin) already covers. This test's width leg is a
    //   both-must-reference-the-same-const structural check, not an
    //   independent value check — do not treat it as one.
    assert_eq!(Border::default(), crate::specified::INITIAL_BORDER);
}

#[test]
fn border_new_is_default() {
    // `Border::new()` is documented as a thin
    // `Self::default()` wrapper (same shape as
    // `raikiri_traits::page::PageBox::new`) — check that the two stay
    // equivalent.
    assert_eq!(Border::new(), Border::default());
}

#[test]
fn border_shorthand_color_slot_accepts_currentcolor() {
    // Sibling case: the border shorthand's color slot uses the same
    // `parse_border_color` as the four longhands, so it also accepts
    // the `currentcolor` keyword.
    let expected = Border {
        width: Length::Px(1.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("1px solid currentcolor", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
    // Argument order is flexible: style first.
    assert_eq!(
        parse("solid currentcolor 1px", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
}

#[test]
fn border_shorthand_empty_returns_none() {
    // The `||` grammar requires at least one component; zero returns None.
    assert_eq!(parse("", "border"), None);
}

#[test]
fn border_shorthand_unknown_keyword_only_returns_none() {
    // An unknown keyword (matching no width/style/color slot) makes all slots None
    // on the first iteration, then `matched=false` breaks; the zero-component
    // guard returns None and drops the declaration.
    assert_eq!(parse("garbage", "border"), None);
}

#[test]
fn border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check() {
    // For `border: 1px 2px`, the first iteration sets width=1px. On the second,
    // the width slot is occupied and `2px` matches no other slot (style/color),
    // so parsing falls through. The caller's `expect_exhausted` owns the leftover.
    // The helper alone holds the first value and returns Some (the parse_value path
    // drops the declaration end-to-end; check in a rule.rs test).
    let expected = Border {
        width: Length::Px(1.0),
        style: BorderStyle::None,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    let mut input = ParserInput::new("1px 2px");
    let mut parser = Parser::new(&mut input);
    let result = parse_border_shorthand(&mut parser);
    assert_eq!(result, Some(Sides::all(expected)));
    // `2px` remains unconsumed: the parser cursor points just before "2px".
    assert!(!parser.is_exhausted());
}

#[test]
fn border_longhand_keys_map_correctly() {
    // Twelve longhands and one shorthand map to their respective keys,
    // pinning cascade winner selection like sibling `margin_longhand_keys_map_correctly`
    // (the same discriminant-integrity pattern).
    assert_eq!(
        PropertyValue::BorderTopWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderTopWidth
    );
    assert_eq!(
        PropertyValue::BorderRightWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderRightWidth
    );
    assert_eq!(
        PropertyValue::BorderBottomWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderBottomWidth
    );
    assert_eq!(
        PropertyValue::BorderLeftWidth(Length::Px(1.0)).key(),
        PropertyKey::BorderLeftWidth
    );
    assert_eq!(
        PropertyValue::BorderTopStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderTopStyle
    );
    assert_eq!(
        PropertyValue::BorderRightStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderRightStyle
    );
    assert_eq!(
        PropertyValue::BorderBottomStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderBottomStyle
    );
    assert_eq!(
        PropertyValue::BorderLeftStyle(BorderStyle::Solid).key(),
        PropertyKey::BorderLeftStyle
    );
    assert_eq!(
        PropertyValue::BorderTopColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderTopColor
    );
    assert_eq!(
        PropertyValue::BorderRightColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderRightColor
    );
    assert_eq!(
        PropertyValue::BorderBottomColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderBottomColor
    );
    assert_eq!(
        PropertyValue::BorderLeftColor(BorderColor::CurrentColor).key(),
        PropertyKey::BorderLeftColor
    );
    let default_border = Border {
        width: Length::Px(3.0),
        style: BorderStyle::None,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    assert_eq!(
        PropertyValue::Border(Sides::all(default_border)).key(),
        PropertyKey::Border
    );
}

#[test]
fn width_key_maps_to_width_property_key() {
    // PropertyValue::Width → PropertyKey::Width (discriminant integrity for
    // cascade winner selection, following the sibling padding/margin pattern).
    assert_eq!(
        PropertyValue::Width(LengthOrAuto::Auto).key(),
        PropertyKey::Width
    );
    assert_eq!(
        PropertyValue::Width(LengthOrAuto::Length(Length::Px(100.0))).key(),
        PropertyKey::Width
    );
}

#[test]
fn logical_preferred_sizes_stay_logical_until_the_cascade() {
    // CSS Logical Properties 1 §4.1: the physical axis of inline-size and
    // block-size depends on writing-mode, so parsing keeps them logical.
    assert_eq!(
        property_key_for_name("inline-size"),
        Some(PropertyKey::InlineSize)
    );
    assert_eq!(
        property_key_for_name("block-size"),
        Some(PropertyKey::BlockSize)
    );
    assert_eq!(
        parse("120px", "inline-size"),
        Some(PropertyValue::InlineSize(LengthOrAuto::Length(Length::Px(
            120.0
        ))))
    );
    assert_eq!(
        parse("80px", "block-size"),
        Some(PropertyValue::BlockSize(LengthOrAuto::Length(Length::Px(
            80.0
        ))))
    );
    assert_eq!(parse("-1px", "inline-size"), None);
    assert_eq!(
        PropertyValue::InlineSize(LengthOrAuto::Auto).key(),
        PropertyKey::InlineSize
    );
    assert_eq!(
        PropertyValue::BlockSize(LengthOrAuto::Auto).key(),
        PropertyKey::BlockSize
    );
}

// ── height (CSS Sizing 3 §3.1.1) ─────────────
//
// Primary source:
// - #preferred-size-properties: `auto | <length-percentage [0,∞]> |
//   min-content | max-content | fit-content(<length-percentage>)`,
//   initial `auto`, Inheritance `No`.
//
// Current scope has only two branches, `auto` and nonnegative `<length-percentage>`;
// other sizing/global keywords and calc() / var() are silently dropped
// (see the "Scope carving" section of the parse_height doc).
//
// Sibling case: the same nonnegative `<length-percentage>` + `auto` grammar
// as `width`; both share the `LengthOrAuto` payload type.

#[test]
fn height_parse_auto() {
    // Verification 1 (task doc): the `auto` identifier is also the spec initial value
    // (§3.1.1 "Initial: auto"). Pin acceptance when the declaration arrives
    // as the cascade winner.
    assert_eq!(
        parse("auto", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_parse_px() {
    // Verification 2 (task doc): nonnegative px is valid `<length-percentage>`
    // (§3.1.1), in the same shape as sibling `width_parse_length_px`.
    assert_eq!(
        parse("100px", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(
            100.0
        ))))
    );
}

#[test]
fn height_parse_percentage() {
    // Verification 3 (task doc): accept percentages via parse_length_value's
    // allow_percentage = true path. Resolving containing-block percentages to
    // actual dimensions is downstream work.
    assert_eq!(
        parse("50%", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn height_rejects_negative_length() {
    // Verification 4 (task doc): under §3.1.1 `<length-percentage [0,∞]>`,
    // `-10px` is invalid per spec and must be dropped. Follow the sibling
    // padding nonnegative filter, and pin rejection symmetric with the accepted
    // `-10px` margin (§3.1).
    assert_eq!(parse("-10px", "height"), None);
}

#[test]
fn height_accepts_zero() {
    // Spec `<length-percentage [0,∞]>`: zero is the closed interval's lower bound. `0px` takes the Dimension arm;
    // bare `0` takes the Number arm under CSS Values 3 §5's unitless-zero clause.
    // Both pass parse_height's `>= 0.0` nonnegative filter.
    assert_eq!(
        parse("0px", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(0.0))))
    );
    assert_eq!(
        parse("0", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(0.0))))
    );
}

#[test]
fn height_rejects_negative_percentage() {
    // Check that the nonnegative filter also applies to Percent variants (as in
    // parse_padding_side, a sibling of Verification 4).
    assert_eq!(parse("-10%", "height"), None);
}

#[test]
fn height_rejects_unsupported_sizing_keyword() {
    assert_eq!(
        parse("min-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("max-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("fit-content(50%)", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
    assert_eq!(
        parse("fit-content", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_rejects_css_wide_keyword() {
    // (b) Unsupported: CSS-wide keywords are not implemented yet; silently drop them.
    // Canonical reference: the "CSS-wide keyword" section of the PropertyValue doc.
    assert_eq!(parse("inherit", "height"), None);
    assert_eq!(parse("initial", "height"), None);
    assert_eq!(parse("unset", "height"), None);
    assert_eq!(parse("revert", "height"), None);
    assert_eq!(parse("revert-layer", "height"), None);
}

#[test]
fn height_case_insensitive_auto() {
    // CSS spec: identifier keywords are ASCII case-insensitive
    // (cssparser's `expect_ident_matching` convention, like sibling
    // `margin_side_case_insensitive_auto`).
    assert_eq!(
        parse("AUTO", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_rejects_unsupported_unit() {
    // `cap` (§6.1.1 font-relative lengths) remains
    // unsupported (dropped by parse_length_value). `cm` / `lh` / `rlh` have
    // moved to the accepted set (see `height_accepts_absolute_unit` /
    // `height_accepts_lh`). Follow the sibling
    // `margin_side_rejects_unsupported_unit` pattern.
    assert_eq!(parse("1cap", "height"), None);
}

#[test]
fn height_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`.
    assert_eq!(
        parse("1.5lh", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Lh(1.5))))
    );
    assert_eq!(
        parse("2rlh", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Rlh(
            2.0
        ))))
    );
}

#[test]
fn height_accepts_absolute_unit() {
    // CSS Values 4 §6.2 absolute lengths.
    assert_eq!(
        parse("1cm", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Cm(1.0))))
    );
}

#[test]
fn height_rejects_negative_absolute_unit() {
    assert_eq!(parse("-1cm", "height"), None);
}

#[test]
fn height_parse_em_and_rem() {
    // Grammar coverage: font-relative units (`em` / `rem`) are also
    // accepted in `<length-percentage>` mode. Resolution is downstream
    // (font-size context / root font-size context).
    assert_eq!(
        parse("1.2em", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Em(1.2))))
    );
    assert_eq!(
        parse("2rem", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Rem(
            2.0
        ))))
    );
}

#[test]
fn height_key_maps_to_height_property_key() {
    // Follow the sibling `margin_longhand_keys_map_correctly` pattern: pin
    // cascade winner selection's discriminant integrity.
    let v = PropertyValue::Height(LengthOrAuto::Auto);
    assert_eq!(v.key(), PropertyKey::Height);
    let v = PropertyValue::Height(LengthOrAuto::Length(Length::Px(100.0)));
    assert_eq!(v.key(), PropertyKey::Height);
}

// ── border-radius / box-shadow / outline (CSS Backgrounds 3 / CSS UI 3 §4)
// ──

#[test]
fn border_radius_expands_one_to_four_lengths_in_clockwise_order() {
    assert_eq!(
        parse_entire("1px 2px 3px 4px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius::corners(
            Length::Px(1.0),
            Length::Px(2.0),
            Length::Px(3.0),
            Length::Px(4.0)
        )))
    );
    assert_eq!(
        parse("1px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0).into(),
            top_right: Length::Px(1.0).into(),
            bottom_right: Length::Px(1.0).into(),
            bottom_left: Length::Px(1.0).into(),
        }))
    );
    assert_eq!(
        parse("1px 2em", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0).into(),
            top_right: Length::Em(2.0).into(),
            bottom_right: Length::Px(1.0).into(),
            bottom_left: Length::Em(2.0).into(),
        }))
    );
    assert_eq!(
        parse("1px 2px 3px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0).into(),
            top_right: Length::Px(2.0).into(),
            bottom_right: Length::Px(3.0).into(),
            bottom_left: Length::Px(2.0).into(),
        }))
    );
    assert_eq!(
        parse("1px 2px 3px 4px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0).into(),
            top_right: Length::Px(2.0).into(),
            bottom_right: Length::Px(3.0).into(),
            bottom_left: Length::Px(4.0).into(),
        }))
    );
}

#[test]
fn border_radius_accepts_percentages_and_rejects_negative_lengths() {
    assert_eq!(
        parse_entire("50% 25%", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Percent(50.0).into(),
            top_right: Length::Percent(25.0).into(),
            bottom_right: Length::Percent(50.0).into(),
            bottom_left: Length::Percent(25.0).into(),
        }))
    );
    assert_eq!(
        parse_entire("25%", "border-top-left-radius"),
        Some(PropertyValue::BorderRadiusTopLeft(
            Length::Percent(25.0).into()
        ))
    );
    assert_eq!(
        parse_entire("inherit", "border-radius"),
        Some(PropertyValue::BorderRadiusInherit)
    );
    assert_eq!(parse_entire("1px -2px", "border-radius"), None);
}

#[test]
fn border_radius_accepts_independent_horizontal_and_vertical_axes() {
    for (css, expected) in [
        ("30px / 15px", [[30.0, 15.0]; 4]),
        (
            "1px 2px 3px / 4px 5px",
            [[1.0, 4.0], [2.0, 5.0], [3.0, 4.0], [2.0, 5.0]],
        ),
        (
            "1px 2px / 3px 4px 5px 6px",
            [[1.0, 3.0], [2.0, 4.0], [1.0, 5.0], [2.0, 6.0]],
        ),
    ] {
        let Some(PropertyValue::BorderRadius(radius)) = parse_entire(css, "border-radius") else {
            panic!("{css}")
        };
        for (corner, [x, y]) in [
            radius.top_left,
            radius.top_right,
            radius.bottom_right,
            radius.bottom_left,
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(corner.horizontal, Length::Px(x));
            assert_eq!(corner.vertical, Length::Px(y));
        }
    }
    let corner = CornerRadius::new(Length::Percent(50.0), Length::Percent(25.0));
    assert_eq!(
        parse_entire("50% / 25%", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: corner,
            top_right: corner,
            bottom_right: corner,
            bottom_left: corner,
        }))
    );
    for (property, expected) in [
        (
            "border-top-left-radius",
            PropertyValue::BorderRadiusTopLeft(corner),
        ),
        (
            "border-top-right-radius",
            PropertyValue::BorderRadiusTopRight(corner),
        ),
        (
            "border-bottom-right-radius",
            PropertyValue::BorderRadiusBottomRight(corner),
        ),
        (
            "border-bottom-left-radius",
            PropertyValue::BorderRadiusBottomLeft(corner),
        ),
    ] {
        assert_eq!(parse_entire("50% 25%", property), Some(expected));
        let value = parse_entire("30px 15px", property).unwrap();
        assert_eq!(
            super::super::serialize_value(&value),
            Some("30px 15px".to_owned())
        );
        let value = parse_entire("30px", property).unwrap();
        assert_eq!(
            super::super::serialize_value(&value),
            Some("30px".to_owned())
        );
        assert!(parse_entire("30px -15px", property).is_none());
        assert!(parse_entire("30px 15px 10px", property).is_none());
    }
    for css in [
        "30px /",
        "/ 15px",
        "30px / -15px",
        "-30px / 15px",
        "1px 2px 3px 4px 5px / 6px",
        "1px / 2px 3px 4px 5px 6px",
        "1px / 2px / 3px",
    ] {
        assert!(parse_entire(css, "border-radius").is_none(), "{css}");
    }
}

#[test]
fn box_shadow_parses_none_multiple_entries_and_optional_components() {
    assert_eq!(
        parse("none", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![])))
    );

    let value = parse("red 1px -2px 3px 4px, 2px 3px", "box-shadow");
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    let Some(PropertyValue::BoxShadow(shadows)) = value else {
        panic!("box-shadow should parse to a shadow list");
    };
    assert_eq!(
        shadows.as_ref(),
        &[
            BoxShadowItem {
                offset_x: Length::Px(1.0),
                offset_y: Length::Px(-2.0),
                blur_radius: Length::Px(3.0),
                spread_radius: Length::Px(4.0),
                color: TextShadowColor::Resolved(red()),
                inset: false,
            },
            BoxShadowItem {
                offset_x: Length::Px(2.0),
                offset_y: Length::Px(3.0),
                blur_radius: Length::Px(0.0),
                spread_radius: Length::Px(0.0),
                color: TextShadowColor::CurrentColor,
                inset: false,
            },
        ]
    );
}

#[test]
fn box_shadow_parses_inset_any_order_and_rejects_invalid_components() {
    assert!(matches!(
        parse("inset 1px 2px", "box-shadow"),
        Some(PropertyValue::BoxShadow(shadows)) if shadows[0].inset
    ));
    assert!(matches!(
        parse("red 1px 2px 3px -4px inset", "box-shadow"),
        Some(PropertyValue::BoxShadow(shadows))
            if shadows[0].inset && shadows[0].color == TextShadowColor::Resolved(red())
    ));

    assert_eq!(parse_entire("1px 2px 3px 4px 5px", "box-shadow"), None);
    assert_eq!(parse("1px 2px -3px", "box-shadow"), None);
    assert_eq!(
        parse("1px 2px 3px -4px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(2.0),
            blur_radius: Length::Px(3.0),
            spread_radius: Length::Px(-4.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(parse("1px 2px 10%", "box-shadow"), None);
    assert_eq!(parse_entire("inset 1px 2px inset", "box-shadow"), None);
}

#[test]
fn box_shadow_zero_mantissa_huge_exponent_offset_or_spread_resolves_to_zero_but_preserves_infinity()
{
    // `0e999` is a zero-mantissa, huge-exponent literal that
    // cssparser's tokenizer collapses to `NaN` internally (module doc's
    // "Numeric-token NaN stabilization" section), but
    // `next_numeric_stable` corrects it before `parse_length_value`
    // (and so `parse_shadow_length_reject_nan`'s `!is_nan()` guard)
    // ever sees the token — for offset-x, offset-y, and spread-radius
    // alike (all three carry no sign restriction, unlike blur-radius's
    // `[0,∞]` incidental filter). Each resolves to the spec-correct
    // `Length::Px(0.0)`, and the whole declaration parses successfully.
    assert_eq!(
        parse("0e999px 1px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(0.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 0e999px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(0.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 1px 1px 0e999px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(1.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );

    // `+Inf`/`-Inf` are a *different* hazard class — ordinary `<number>`
    // magnitude overflow, a legitimate (if extreme) `<length>` per CSS
    // Values 4 §5 — and must NOT be rejected here. Both signs are
    // checked (not just `+Inf`) because an earlier iteration of the
    // sibling `opacity` guard used `is_finite()` and wrongly dropped
    // the negative-overflow case too (`opacity_zero_mantissa_huge_exponent_resolves_to_zero_but_preserves_infinity`
    // see the doc).
    assert_eq!(
        parse("1e40px 1px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(f32::INFINITY),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("-1e40px 1px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(f32::NEG_INFINITY),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 1px 1px 1e40px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(1.0),
            spread_radius: Length::Px(f32::INFINITY),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
    assert_eq!(
        parse("1px 1px 1px -1e40px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(1.0),
            spread_radius: Length::Px(f32::NEG_INFINITY),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
}

#[test]
fn box_shadow_blur_radius_zero_mantissa_huge_exponent_resolves_to_zero() {
    // Same recovery as
    // `box_shadow_zero_mantissa_huge_exponent_offset_or_spread_resolves_to_zero`
    // above, for blur-radius (3rd slot) — `0e999px` resolves to
    // `Length::Px(0.0)` before `parse_box_shadow_lengths`'s
    // `value.payload() >= 0.0` check ever runs, so it is accepted
    // normally rather than incidentally rejected.
    assert_eq!(
        parse("1px 1px 0e999px", "box-shadow"),
        Some(PropertyValue::BoxShadow(Arc::new(vec![BoxShadowItem {
            offset_x: Length::Px(1.0),
            offset_y: Length::Px(1.0),
            blur_radius: Length::Px(0.0),
            spread_radius: Length::Px(0.0),
            color: TextShadowColor::CurrentColor,
            inset: false,
        }])))
    );
}

#[test]
fn outline_parses_any_order_and_fills_initial_components() {
    assert_eq!(
        parse("solid 2px red", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(2.0),
            style: OutlineStyle::Solid,
            color: OutlineColor::Resolved(red()),
        }))
    );
    assert_eq!(
        parse("red", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: OutlineStyle::None,
            color: OutlineColor::Resolved(red()),
        }))
    );
    assert_eq!(
        parse("auto", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: OutlineStyle::Auto,
            color: OutlineColor::Invert,
        }))
    );
    assert_eq!(
        parse("auto 2px red", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(2.0),
            style: OutlineStyle::Auto,
            color: OutlineColor::Resolved(red()),
        }))
    );
    assert_eq!(
        parse("auto", "outline-style"),
        Some(PropertyValue::OutlineStyle(OutlineStyle::Auto))
    );
    assert_eq!(parse("hidden", "outline-style"), None);
    assert_eq!(parse("auto", "border-top-style"), None);
    assert_eq!(parse("hidden", "outline"), None);
    assert_eq!(
        PropertyValue::Outline(Outline {
            width: Length::Px(1.0),
            style: OutlineStyle::None,
            color: OutlineColor::Invert,
        })
        .key(),
        PropertyKey::Outline
    );
}

#[test]
fn outline_color_accepts_invert_currentcolor_and_resolved_colors() {
    assert_eq!(
        parse("invert", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::Invert))
    );
    assert_eq!(
        parse("InVeRt", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::Invert))
    );
    assert_eq!(
        parse("currentcolor", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::CurrentColor))
    );
    assert_eq!(
        parse("red", "outline-color"),
        Some(PropertyValue::OutlineColor(OutlineColor::Resolved(red())))
    );
    assert_eq!(
        parse("solid invert", "outline"),
        Some(PropertyValue::Outline(Outline {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: OutlineStyle::Solid,
            color: OutlineColor::Invert,
        }))
    );
    // `invert` is outline-only; border-color parsing remains unchanged.
    assert_eq!(parse("invert", "border-top-color"), None);
}

#[test]
fn outline_offset_parses_length_and_rejects_non_length() {
    // CSS UI 3 §4.5 <https://www.w3.org/TR/css-ui-3/#outline-offset> — `<length>`,
    // initial `0`, non-inherited. Negative values are valid (inset).
    assert_eq!(
        parse("5px", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Px(5.0)))
    );
    assert_eq!(
        parse("0", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Px(0.0)))
    );
    assert_eq!(
        parse("-3px", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Px(-3.0)))
    );
    assert_eq!(
        parse("2em", "outline-offset"),
        Some(PropertyValue::OutlineOffset(Length::Em(2.0)))
    );
    assert_eq!(
        PropertyValue::OutlineOffset(Length::Px(4.0)).key(),
        PropertyKey::OutlineOffset
    );
    // `<percentage>` is not part of the grammar — reject.
    assert_eq!(parse("5%", "outline-offset"), None);
    // `auto` / `none` are not valid for this property.
    assert_eq!(parse("auto", "outline-offset"), None);
    assert_eq!(parse("none", "outline-offset"), None);
}

#[test]
fn border_right_shorthand_parses_width_style_color() {
    // CSS Backgrounds 3 §3.4 single-side shorthand: `border-right` accepts the same
    // `||` components as `border` but targets only the right side.
    let expected = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("2px dashed", "border-right"),
        Some(PropertyValue::BorderRight(expected))
    );
    // Any order, omitted components fill with initial values.
    assert_eq!(
        parse("dashed 2px", "border-right"),
        Some(PropertyValue::BorderRight(expected))
    );
    assert_eq!(
        parse("dashed", "border-right"),
        Some(PropertyValue::BorderRight(Border {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: BorderStyle::Dashed,
            color: BorderColor::CurrentColor,
        }))
    );
    assert_eq!(
        parse("2px", "border-right").unwrap().key(),
        PropertyKey::BorderRight
    );
}

#[test]
fn border_and_border_right_accept_css_wide_keywords() {
    // CSS Cascading 4 §7.3 + CSS Cascading 5 §7.3.5: `border` and `border-right`
    // accept the five CSS-wide keywords as a lone value.
    for (kw, expected) in [
        ("inherit", CssWideKeyword::Inherit),
        ("initial", CssWideKeyword::Initial),
        ("unset", CssWideKeyword::Unset),
        ("revert", CssWideKeyword::Revert),
        ("revert-layer", CssWideKeyword::RevertLayer),
    ] {
        assert_eq!(
            parse(kw, "border"),
            Some(PropertyValue::BorderCssWide(expected)),
            "border: {kw}"
        );
        assert_eq!(
            parse(kw, "border-right"),
            Some(PropertyValue::BorderRightCssWide(expected)),
            "border-right: {kw}"
        );
        // Longhands share the same contract.
        assert_eq!(
            parse(kw, "border-right-width"),
            Some(PropertyValue::BorderRightWidthCssWide(expected))
        );
        assert_eq!(
            parse(kw, "border-right-style"),
            Some(PropertyValue::BorderRightStyleCssWide(expected))
        );
        assert_eq!(
            parse(kw, "border-right-color"),
            Some(PropertyValue::BorderRightColorCssWide(expected))
        );
    }
    // Case-insensitive.
    assert_eq!(
        parse("INHERIT", "border-right"),
        Some(PropertyValue::BorderRightCssWide(CssWideKeyword::Inherit))
    );
}

#[test]
fn border_css_wide_combined_with_components_is_invalid() {
    // A CSS-wide keyword must be the lone value: `border-right: inherit solid`
    // and `border: inherit solid` leave a leftover token for the caller's
    // `expect_exhausted`, which drops the declaration. `parse_entire` models that
    // exhaustiveness here; `parse` alone would return the prefix.
    assert_eq!(parse_entire("inherit solid", "border-right"), None);
    assert_eq!(parse_entire("inherit solid", "border"), None);
    assert_eq!(parse_entire("solid inherit", "border-right"), None);
    assert_eq!(parse_entire("1px inherit", "border"), None);
    // Shorthand keys for the CSS-wide forms share the shorthand key so cascade
    // winners compete per shorthand before expansion.
    assert_eq!(
        PropertyValue::BorderCssWide(CssWideKeyword::Inherit).key(),
        PropertyKey::Border
    );
    assert_eq!(
        PropertyValue::BorderRightCssWide(CssWideKeyword::Inherit).key(),
        PropertyKey::BorderRight
    );
}

#[test]
fn border_right_expansion_preserves_declaration_order() {
    // CSS Cascading 4 §3 + §6.1 order of appearance: expanding `border-right`
    // in place as width, style, color preserves declaration order so a later
    // longhand in the same block wins (see `crate::rule::expand_border_right`).
    use crate::rule::{expand_border_right, expand_border_right_css_wide};
    let border = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::CurrentColor,
    };
    let mut values = Vec::new();
    expand_border_right(border, |v| values.push(v));
    assert_eq!(
        values,
        vec![
            PropertyValue::BorderRightWidth(Length::Px(2.0)),
            PropertyValue::BorderRightStyle(BorderStyle::Dashed),
            PropertyValue::BorderRightColor(BorderColor::CurrentColor),
        ]
    );
    let mut wide = Vec::new();
    expand_border_right_css_wide(CssWideKeyword::Inherit, |v| wide.push(v));
    assert_eq!(
        wide,
        vec![
            PropertyValue::BorderRightWidthCssWide(CssWideKeyword::Inherit),
            PropertyValue::BorderRightStyleCssWide(CssWideKeyword::Inherit),
            PropertyValue::BorderRightColorCssWide(CssWideKeyword::Inherit),
        ]
    );
}

#[test]
fn border_left_shorthand_parses_width_style_color() {
    // Left counterpart of `border_right_shorthand_parses_width_style_color`:
    // same `||` grammar, only the expansion target differs. Required by the
    // conic reference files, which pair `border-left` with `border-right`
    // quadrants.
    let expected = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("2px dashed", "border-left"),
        Some(PropertyValue::BorderLeft(expected))
    );
    assert_eq!(
        parse("dashed 2px", "border-left"),
        Some(PropertyValue::BorderLeft(expected))
    );
    assert_eq!(
        parse("dashed", "border-left"),
        Some(PropertyValue::BorderLeft(Border {
            width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
            style: BorderStyle::Dashed,
            color: BorderColor::CurrentColor,
        }))
    );
    assert_eq!(
        parse("2px", "border-left").unwrap().key(),
        PropertyKey::BorderLeft
    );
    assert_eq!(
        parse("50px solid black", "border-left"),
        Some(PropertyValue::BorderLeft(Border {
            width: Length::Px(50.0),
            style: BorderStyle::Solid,
            color: BorderColor::Resolved(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 255
            }),
        }))
    );
}

#[test]
fn border_left_accepts_css_wide_keywords() {
    // Left counterpart of the right-side CSS-wide test: all five keywords
    // expand to the three left longhands.
    for (kw, expected) in [
        ("inherit", CssWideKeyword::Inherit),
        ("initial", CssWideKeyword::Initial),
        ("unset", CssWideKeyword::Unset),
        ("revert", CssWideKeyword::Revert),
        ("revert-layer", CssWideKeyword::RevertLayer),
    ] {
        assert_eq!(
            parse(kw, "border-left"),
            Some(PropertyValue::BorderLeftCssWide(expected)),
            "border-left: {kw}"
        );
    }
    assert_eq!(
        parse("INHERIT", "border-left"),
        Some(PropertyValue::BorderLeftCssWide(CssWideKeyword::Inherit))
    );
    assert_eq!(
        PropertyValue::BorderLeftCssWide(CssWideKeyword::Inherit).key(),
        PropertyKey::BorderLeft
    );
}

#[test]
fn border_left_expansion_preserves_declaration_order() {
    // Left counterpart of `border_right_expansion_preserves_declaration_order`.
    use crate::rule::{expand_border_left, expand_border_left_css_wide};
    let border = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::CurrentColor,
    };
    let mut values = Vec::new();
    expand_border_left(border, |v| values.push(v));
    assert_eq!(
        values,
        vec![
            PropertyValue::BorderLeftWidth(Length::Px(2.0)),
            PropertyValue::BorderLeftStyle(BorderStyle::Dashed),
            PropertyValue::BorderLeftColor(BorderColor::CurrentColor),
        ]
    );
    let mut wide = Vec::new();
    expand_border_left_css_wide(CssWideKeyword::Inherit, |v| wide.push(v));
    assert_eq!(
        wide,
        vec![
            PropertyValue::BorderLeftWidthCssWide(CssWideKeyword::Inherit),
            PropertyValue::BorderLeftStyleCssWide(CssWideKeyword::Inherit),
            PropertyValue::BorderLeftColorCssWide(CssWideKeyword::Inherit),
        ]
    );
}

#[test]
fn css_wide_keyword_roundtrips_through_all() {
    // Pin `CssWideKeyword::ALL` (generated by `css_keywords!`) so the dead-code
    // lint stays quiet and future keyword additions force an explicit update here.
    assert_eq!(CssWideKeyword::ALL.len(), 5);
    for kw in CssWideKeyword::ALL {
        assert_eq!(CssWideKeyword::from_css_ident(kw.as_css_str()), Some(*kw));
    }
}

#[test]
#[allow(clippy::type_complexity)]
fn border_all_longhands_accept_css_wide_keywords() {
    // Cover every `border-*-width/style/color` dispatch arm in `parse_value`
    // (see `parse_css_wide_keyword`): all twelve longhands accept all five keywords.
    let cases: &[(&str, fn(CssWideKeyword) -> PropertyValue)] = &[
        ("border-top-width", PropertyValue::BorderTopWidthCssWide),
        ("border-right-width", PropertyValue::BorderRightWidthCssWide),
        (
            "border-bottom-width",
            PropertyValue::BorderBottomWidthCssWide,
        ),
        ("border-left-width", PropertyValue::BorderLeftWidthCssWide),
        ("border-top-style", PropertyValue::BorderTopStyleCssWide),
        ("border-right-style", PropertyValue::BorderRightStyleCssWide),
        (
            "border-bottom-style",
            PropertyValue::BorderBottomStyleCssWide,
        ),
        ("border-left-style", PropertyValue::BorderLeftStyleCssWide),
        ("border-top-color", PropertyValue::BorderTopColorCssWide),
        ("border-right-color", PropertyValue::BorderRightColorCssWide),
        (
            "border-bottom-color",
            PropertyValue::BorderBottomColorCssWide,
        ),
        ("border-left-color", PropertyValue::BorderLeftColorCssWide),
    ];
    for (name, ctor) in cases {
        for kw in CssWideKeyword::ALL {
            assert_eq!(
                parse(kw.as_css_str(), name),
                Some(ctor(*kw)),
                "{name}: {kw:?}"
            );
        }
    }
}

#[test]
fn border_css_wide_serializes_to_keyword() {
    // Cover `serialize_value`'s `CssWideKeyword` arm for every border variant.
    for kw in CssWideKeyword::ALL {
        let expected = kw.as_css_str().to_owned();
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderRightCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderTopWidthCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderRightWidthCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderBottomWidthCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderLeftWidthCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderTopStyleCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderRightStyleCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderBottomStyleCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderLeftStyleCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderTopColorCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderRightColorCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderBottomColorCssWide(*kw)),
            Some(expected.clone())
        );
        assert_eq!(
            crate::property::serialize_value(&PropertyValue::BorderLeftColorCssWide(*kw)),
            Some(expected.clone())
        );
    }
    // `border-right` shorthand itself is not canonically serialized (expanded first).
    assert_eq!(
        crate::property::serialize_value(&PropertyValue::BorderRight(Border {
            width: Length::Px(2.0),
            style: BorderStyle::Solid,
            color: BorderColor::CurrentColor,
        })),
        None
    );
}

#[test]
fn border_right_shorthand_rejects_empty_and_unknown() {
    // Cover `parse_border_right_shorthand`'s empty-declaration guard (see
    // `parse_border_shorthand`'s `||` contract): empty and unknown-only drop.
    assert_eq!(parse("", "border-right"), None);
    assert_eq!(parse("garbage", "border-right"), None);
    assert_eq!(parse_entire("1px 2px", "border-right"), None);
}

/// The `border-top` / `border-bottom` single-side shorthands (CSS Backgrounds 3
/// §3.4), each paired with its value constructor, CSS-wide constructor, and
/// shorthand key.
#[allow(clippy::type_complexity)]
const BORDER_TOP_BOTTOM_SHORTHANDS: &[(
    &str,
    fn(Border) -> PropertyValue,
    fn(CssWideKeyword) -> PropertyValue,
    PropertyKey,
)] = &[
    (
        "border-top",
        PropertyValue::BorderTop,
        PropertyValue::BorderTopCssWide,
        PropertyKey::BorderTop,
    ),
    (
        "border-bottom",
        PropertyValue::BorderBottom,
        PropertyValue::BorderBottomCssWide,
        PropertyKey::BorderBottom,
    ),
];

#[test]
fn border_top_and_bottom_shorthands_parse_width_style_color() {
    // Top/bottom counterparts of `border_right_shorthand_parses_width_style_color`:
    // same `||` grammar, only the expansion target differs.
    let blue = BorderColor::Resolved(CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    });
    for (name, ctor, _, key) in BORDER_TOP_BOTTOM_SHORTHANDS {
        let expected = Border {
            width: Length::Px(2.0),
            style: BorderStyle::Dashed,
            color: BorderColor::Resolved(red()),
        };
        assert_eq!(
            parse("2px dashed red", name),
            Some(ctor(expected)),
            "{name}"
        );
        // Any order.
        assert_eq!(
            parse("red dashed 2px", name),
            Some(ctor(expected)),
            "{name}"
        );
        // Omitted components fill with their initial values.
        assert_eq!(
            parse("thick", name),
            Some(ctor(Border {
                width: Length::Px(5.0),
                style: BorderStyle::None,
                color: BorderColor::CurrentColor,
            })),
            "{name}: thick"
        );
        assert_eq!(
            parse("blue", name),
            Some(ctor(Border {
                width: Length::Px(BORDER_WIDTH_MEDIUM_PX),
                style: BorderStyle::None,
                color: blue,
            })),
            "{name}: blue"
        );
        assert_eq!(parse("2px", name).unwrap().key(), *key, "{name}");
    }
}

#[test]
fn border_top_and_bottom_accept_css_wide_keywords() {
    // Top/bottom counterparts of `border_and_border_right_accept_css_wide_keywords`.
    for (name, _, wide, key) in BORDER_TOP_BOTTOM_SHORTHANDS {
        for kw in CssWideKeyword::ALL {
            assert_eq!(
                parse(kw.as_css_str(), name),
                Some(wide(*kw)),
                "{name}: {kw:?}"
            );
        }
        assert_eq!(
            parse("INHERIT", name),
            Some(wide(CssWideKeyword::Inherit)),
            "{name}"
        );
        assert_eq!(wide(CssWideKeyword::Inherit).key(), *key, "{name}");
        // A CSS-wide keyword must be the lone value.
        assert_eq!(parse_entire("inherit solid", name), None, "{name}");
        assert_eq!(parse_entire("solid inherit", name), None, "{name}");
    }
}

#[test]
fn border_top_and_bottom_shorthands_reject_empty_and_unknown() {
    // Cover the empty-declaration guards of `parse_border_top_shorthand` and
    // `parse_border_bottom_shorthand`: empty, unknown-only, and duplicated
    // components drop.
    for (name, ..) in BORDER_TOP_BOTTOM_SHORTHANDS {
        assert_eq!(parse("", name), None, "{name}");
        assert_eq!(parse("garbage", name), None, "{name}");
        assert_eq!(parse_entire("1px 2px", name), None, "{name}");
        assert_eq!(parse_entire("solid dashed", name), None, "{name}");
        assert_eq!(parse_entire("-1px solid", name), None, "{name}");
    }
}

#[test]
fn border_top_and_bottom_expansion_preserves_declaration_order() {
    // Top/bottom counterparts of `border_right_expansion_preserves_declaration_order`.
    use crate::rule::{
        expand_border_bottom, expand_border_bottom_css_wide, expand_border_top,
        expand_border_top_css_wide,
    };
    let border = Border {
        width: Length::Px(2.0),
        style: BorderStyle::Dashed,
        color: BorderColor::CurrentColor,
    };
    let mut values = Vec::new();
    expand_border_top(border, |v| values.push(v));
    expand_border_bottom(border, |v| values.push(v));
    assert_eq!(
        values,
        vec![
            PropertyValue::BorderTopWidth(Length::Px(2.0)),
            PropertyValue::BorderTopStyle(BorderStyle::Dashed),
            PropertyValue::BorderTopColor(BorderColor::CurrentColor),
            PropertyValue::BorderBottomWidth(Length::Px(2.0)),
            PropertyValue::BorderBottomStyle(BorderStyle::Dashed),
            PropertyValue::BorderBottomColor(BorderColor::CurrentColor),
        ]
    );
    let mut wide = Vec::new();
    expand_border_top_css_wide(CssWideKeyword::Initial, |v| wide.push(v));
    expand_border_bottom_css_wide(CssWideKeyword::Initial, |v| wide.push(v));
    assert_eq!(
        wide,
        vec![
            PropertyValue::BorderTopWidthCssWide(CssWideKeyword::Initial),
            PropertyValue::BorderTopStyleCssWide(CssWideKeyword::Initial),
            PropertyValue::BorderTopColorCssWide(CssWideKeyword::Initial),
            PropertyValue::BorderBottomWidthCssWide(CssWideKeyword::Initial),
            PropertyValue::BorderBottomStyleCssWide(CssWideKeyword::Initial),
            PropertyValue::BorderBottomColorCssWide(CssWideKeyword::Initial),
        ]
    );
}

#[test]
fn border_top_and_bottom_serialize_like_border_right() {
    // Counterpart of `border_css_wide_serializes_to_keyword`: the CSS-wide forms
    // serialize to the keyword; the value form is expanded first and so is not
    // canonically serialized.
    for (name, ctor, wide, _) in BORDER_TOP_BOTTOM_SHORTHANDS {
        for kw in CssWideKeyword::ALL {
            assert_eq!(
                crate::property::serialize_value(&wide(*kw)),
                Some(kw.as_css_str().to_owned()),
                "{name}: {kw:?}"
            );
        }
        assert_eq!(
            crate::property::serialize_value(&ctor(Border {
                width: Length::Px(2.0),
                style: BorderStyle::Solid,
                color: BorderColor::CurrentColor,
            })),
            None,
            "{name}"
        );
    }
}

#[test]
fn border_top_and_bottom_expand_in_declaration_block_and_reset_omitted_parts() {
    // CSS Backgrounds 3 §3.4: a single-side shorthand resets all three of its
    // side's longhands, so an earlier longhand in the same block is overridden
    // by the omitted component's initial value, and a later longhand wins.
    let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "border-bottom-width: 5px; border-bottom: red; border-top: 1px solid; \
         border-top-color: blue",
    )));
    let values: Vec<_> = decls.iter().map(|d| d.value.clone()).collect();
    assert_eq!(
        values,
        vec![
            PropertyValue::BorderBottomWidth(Length::Px(5.0)),
            PropertyValue::BorderBottomWidth(Length::Px(BORDER_WIDTH_MEDIUM_PX)),
            PropertyValue::BorderBottomStyle(BorderStyle::None),
            PropertyValue::BorderBottomColor(BorderColor::Resolved(red())),
            PropertyValue::BorderTopWidth(Length::Px(1.0)),
            PropertyValue::BorderTopStyle(BorderStyle::Solid),
            PropertyValue::BorderTopColor(BorderColor::CurrentColor),
            PropertyValue::BorderTopColor(BorderColor::Resolved(CssColor {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            })),
        ]
    );
}
