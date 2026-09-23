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
//   #3 padding-box → None (spec 外、CSS UI 3 draft の削除済 keyword)
//   #4 initial + #5 non-inheritance test は crate::computed 側
//
// sibling: `display_*` / `text_align_*` の keyword parser test 群と同構造。

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
    // - `padding-box` は CSS-UI 3 draft 相当だが css-sizing-3 では削除済み
    //   (spec note "supersedes the one in `[CSS-UI-3]`")、
    // - `margin-box` は grammar 外の任意 ident。
    assert_eq!(parse("padding-box", "box-sizing"), None);
    assert_eq!(parse("margin-box", "box-sizing"), None);
    assert_eq!(parse("bogus", "box-sizing"), None);
}

#[test]
fn box_sizing_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
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
    // CSS Values 3 §3.1 "Pre-defined Keywords": keyword は ASCII case-insensitive
    // (sibling `display_is_case_insensitive` と同 flavor)。
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
    // 選択の discriminant 導線、sibling `Display` / `TextAlign` key() と対称)。
    let v = PropertyValue::BoxSizing(BoxSizing::BorderBox);
    assert_eq!(v.key(), PropertyKey::BoxSizing);
}

// ── position: running() (CSS GCPM 3 §1.2.1) ──
//
// Verification items 1-6 は task description 由来、
// canonical shape は後に amended。sibling は counter-* /
// content / string-set の SmolStr wire-through pattern。

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
    // Verification 2: 別 name の smoke — SmolStr::new が生きていることを pin。
    assert_eq!(
        parse("running(footer)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("footer")
        )))
    );
}

#[test]
fn position_parse_static() {
    // Verification 5 baseline: position: static → PositionValue::Static。
    // apply_value は no-op、running_templates は inherit_from の initial
    // (空 Vec) が残る = cascade winner が earlier running(...) を suppress する
    // ID 用途 (cascade.rs 側の `static_position_wins_over_running` で検証)。
    assert_eq!(
        parse("static", "position"),
        Some(PropertyValue::Position(PositionValue::Static))
    );
}

#[test]
fn position_running_case_insensitive_function_name() {
    // Verification 4: function name は ASCII case-insensitive (CSS spec 慣行)、
    // custom-ident は case-preserving。
    assert_eq!(
        parse("RUNNING(header)", "position"),
        Some(PropertyValue::Position(PositionValue::Running(
            SmolStr::new("header")
        )))
    );
}

#[test]
fn position_running_rejects_none_custom_ident() {
    // Verification 6: `running(none)` reject。`none` は position property
    // spec-defined keyword ではないが、runtime resolve で `element(none)` 参照が
    // silent match するのを避けるため custom-ident としても弾く (string-set
    // と同じ規約)。
    assert_eq!(parse("running(none)", "position"), None);
}

#[test]
fn position_running_rejects_reserved_css_wide_keyword() {
    // spec CSS Values 4 §4.2 <https://www.w3.org/TR/css-values-4/#custom-idents>:
    // <custom-ident> は CSS-wide keyword + `default` 除外。
    // position: running(inherit) 等は declaration drop。
    assert_eq!(parse("running(inherit)", "position"), None);
    assert_eq!(parse("running(initial)", "position"), None);
    assert_eq!(parse("running(unset)", "position"), None);
    assert_eq!(parse("running(revert)", "position"), None);
    assert_eq!(parse("running(default)", "position"), None);
}

#[test]
fn position_rejects_missing_custom_ident() {
    // spec §1.2.1: `running() = running( <custom-ident> )` — argument 必須。
    // 空 argument は malformed、declaration drop。
    assert_eq!(parse("running()", "position"), None);
}

#[test]
fn position_parse_sticky() {
    // CSS Positioned Layout Module Level 3 §3 sticky positioning
    // <https://www.w3.org/TR/css-position-3/#sticky-pos>:
    // `position: sticky` は有効な position 値。本 crate では parse 段階で
    // PositionValue::Sticky として保持する (layout 連携は将来)。
    assert_eq!(
        parse("sticky", "position"),
        Some(PropertyValue::Position(PositionValue::Sticky))
    );
    // ident は ASCII case-insensitive (cssparser の expect_ident_matching 準拠、
    // `static` と同様)。
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
    // relative / absolute / fixed は受理する (position:relative offset 実装)。
    // `sticky` も受理。
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
    // それ以外の keyword は drop
    assert_eq!(parse("inherit", "position"), None);
    assert_eq!(parse("initial", "position"), None);
}

#[test]
fn position_rejects_running_with_extra_arg() {
    // `running(a, b)` — parse_nested_block が parse_entirely 経由で
    // 余剰 token を検知し、declaration drop になる。
    assert_eq!(parse("running(a, b)", "position"), None);
}

// ── padding (CSS Box 3 §4.1 physical + §4.2 shorthand) ──
//
// Primary sources:
// - https://www.w3.org/TR/css-box-3/#padding-physical
//   "Negative values for padding properties are invalid." — non-negative
//   constraint を parse-time enforce (parse_padding_side が全 Length variant
//   で >= 0.0 check、負値 = declaration drop)。
// - https://www.w3.org/TR/css-box-3/#padding-shorthand
//   `<'padding-top'>{1,4}` — 1-4 value expansion (top/right/bottom/left)。

fn padding_sides(top: Length, right: Length, bottom: Length, left: Length) -> Sides<Length> {
    Sides {
        top,
        right,
        bottom,
        left,
    }
}

// Verification #3 — longhand parse 4 arm (px / % / em / pt の 5 unit)。
#[test]
fn padding_top_parses_px() {
    assert_eq!(
        parse("10px", "padding-top"),
        Some(PropertyValue::PaddingTop(Length::Px(10.0)))
    );
}

#[test]
fn padding_right_parses_percentage() {
    // spec grammar `<length-percentage>` — % 受理。
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

// Verification #4 — shorthand 1-4 value expansion (CSS Box 3 §4.2)。
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
    // spec (CSS Box 3) §4.2 は per-value `<'padding-top'>` = `<length-percentage>` を許容 —
    // 混合 unit も spec-valid (padding: 10px 5% 1em 12pt)。
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

// Verification #5 — non-negative constraint (spec-literal claim)。
#[test]
fn padding_top_rejects_negative_px() {
    // spec (CSS Box 3) §4.1: "Negative values for padding properties are invalid."。
    assert_eq!(parse("-5px", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_percentage() {
    // 負 percentage も同様に spec-invalid。
    assert_eq!(parse("-10%", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_em() {
    // 負 em (font-relative) も spec-invalid。
    assert_eq!(parse("-1em", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_rem() {
    // 全 Length variant 経路の non-negative check check (rem)。
    assert_eq!(parse("-0.5rem", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_pt() {
    // 全 Length variant 経路の non-negative check check (pt)。
    assert_eq!(parse("-3pt", "padding-top"), None);
}

#[test]
fn padding_top_accepts_zero() {
    // zero (bound の下端) は spec grammar `[0,∞]` の閉区間で有効。
    // `0px` は Dimension arm、bare `0` は CSS Values 3 §5 unitless-zero clause
    // の Number arm を通し、非負 filter を pass。
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
    // `padding: 10px -5px` — spec (CSS Box 3) §4.2 の {1,4} multiplier は各 iteration が
    // 有効 `<'padding-top'>` であることを要求。2 番目 `-5px` は spec (CSS Box 3) §4.1
    // `[0,∞]` 制約違反で fail、try_parse rewind で 1-value form の Some を
    // parse_padding_shorthand が返す。ここで DeclParser の expect_exhausted
    // が leftover `-5px` を検知して declaration ごと drop する — 実 caller
    // 経路として rule.rs 経由で drop を check (parse_value 単体では
    // Some(all(10px)) が観測されるが、それは leftover 込みで invalid)。
    let decls_2 = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px -5px;",
    )));
    assert!(
        decls_2.is_empty(),
        "`padding: 10px -5px` must drop via expect_exhausted leftover"
    );
    // 4 value form 内の 4 番目が負値 case — 同様 leftover 経由 drop。
    let decls_4 = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px 20px 30px -40px;",
    )));
    assert!(
        decls_4.is_empty(),
        "`padding: 10px 20px 30px -40px` must drop via expect_exhausted leftover"
    );
}

// Verification #6 — `auto` keyword reject (spec grammar に無い)。
#[test]
fn padding_top_rejects_auto_keyword() {
    // spec (CSS Box 3) §4.1 grammar = `<length-percentage>` のみ、`auto` は margin 側の
    // extension で padding には無い。parse_length_value の Dimension /
    // Percentage arm fall-through で自然 reject。
    assert_eq!(parse("auto", "padding-top"), None);
}

#[test]
fn padding_shorthand_rejects_auto_keyword() {
    // shorthand も同様 auto reject (1st value で fail、全体 drop)。
    assert_eq!(parse("auto", "padding"), None);
}

#[test]
fn padding_shorthand_mixed_with_auto_drops_via_leftover() {
    // `padding: 10px auto` — 1st 成功 (10px)、2nd で auto → try_parse rewind、
    // 1-value form の Some を parse_padding_shorthand が返す。ここまでは
    // parse_value 単体で観測可能だが、DeclParser の expect_exhausted が
    // leftover `auto` を検知して declaration drop する — 実 caller 経路の
    // check として rule.rs 経由でも drop することを確認。
    let decls = crate::rule::parse_declaration_block(&mut Parser::new(&mut ParserInput::new(
        "padding: 10px auto;",
    )));
    assert!(
        decls.is_empty(),
        "`padding: 10px auto` must drop via expect_exhausted leftover"
    );
}

// Verification #6 — spec grammar 外 unit の drop (vw / cap 等、非対応)。
#[test]
fn padding_top_rejects_unsupported_unit() {
    // (b) 非対応 — vw / cap 等は spec-valid だが
    // 未対応、parse_length_value 側で drop、`None`
    // propagate → declaration drop。`ch` / `lh` / `rlh` はそれぞれ受理側へ移った
    // (`padding_top_accepts_ch` / `padding_top_accepts_lh` 参照)。
    assert_eq!(parse("10vw", "padding-top"), None);
    assert_eq!(parse("5cap", "padding-top"), None);
}

#[test]
fn padding_top_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
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
    // 全 Length variant 経路の non-negative check check (cm、新規 absolute unit)。
    assert_eq!(parse("-1cm", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_ex() {
    // 全 Length variant 経路の non-negative check check (ex、新規 font-relative unit)。
    assert_eq!(parse("-1ex", "padding-top"), None);
}

#[test]
fn padding_top_rejects_negative_lh() {
    // 全 Length variant 経路の non-negative check check (`lh`/`rlh`、
    // `Length::payload` の OR-pattern に `Lh`/`Rlh`
    // を足し忘れていないことの直接 pin)。
    assert_eq!(parse("-1lh", "padding-top"), None);
    assert_eq!(parse("-1rlh", "padding-top"), None);
}

/// 追加した残り unit (`rex` / `rch` / `ic` / `ric` /
/// `mm` / `Q`) を `Length::payload` 経由で直接 exercise する — 他 call site
/// (font-size / width / height / margin / border-width / line-height) の
/// テストは Ex / Ch / Cm / In / Pc しか通さないため、`Length::payload` の
/// OR-pattern 全 arm の patch coverage には本 test が要る。
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

// Verification — Sides::all constructor + PropertyKey mapping smoke。
#[test]
fn padding_key_maps_to_padding_property_keys() {
    // 5 discriminant (4 longhand + 1 shorthand) が個別 PropertyKey を返すこと。
    // cascade winner selection の discriminant integrity 確認。
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
    // Sides::all(v) は 4 field を全て v で埋める。
    let sides = Sides::all(Length::Px(7.5));
    assert_eq!(sides.top, Length::Px(7.5));
    assert_eq!(sides.right, Length::Px(7.5));
    assert_eq!(sides.bottom, Length::Px(7.5));
    assert_eq!(sides.left, Length::Px(7.5));
}

#[test]
fn padding_shorthand_five_values_dropped_by_leftover() {
    // 5 個目以降は本 helper が consume せず leftover として残す。
    // parse_value 単体では 4-value form の Some を返すが、caller (rule.rs)
    // の expect_exhausted が leftover を検知して declaration drop するので、
    // rule.rs 経由で drop 確認。
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
    // CSS spec: unit identifier は ASCII case-insensitive。
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
    // PropertyValue::Position → PropertyKey::Position (cascade winner 選択の
    // discriminant integrity、既存 sibling counter-* / content / string-set と
    // 同じ pattern)。
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
    // Verification 3-a: `margin-top: 10px` → MarginTop(Length(Px(10)))。
    assert_eq!(
        parse("10px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            10.0
        ))))
    );
}

#[test]
fn margin_right_parse_auto() {
    // Verification 3-b: `margin-right: auto` → MarginRight(Auto)。§3.1 の
    // `auto` alternative の受理を per-side longhand で pin。
    assert_eq!(
        parse("auto", "margin-right"),
        Some(PropertyValue::MarginRight(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_bottom_parse_percentage() {
    // Verification 3-c: `margin-bottom: 50%` → MarginBottom(Length(Percent(50)))。
    assert_eq!(
        parse("50%", "margin-bottom"),
        Some(PropertyValue::MarginBottom(LengthOrAuto::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn margin_left_parse_em() {
    // Verification 3-d: `margin-left: 2em` → MarginLeft(Length(Em(2)))。
    // `<length-percentage>` mode 経由で em 受理 (parse_length_value の mode
    // arg = true)。
    assert_eq!(
        parse("2em", "margin-left"),
        Some(PropertyValue::MarginLeft(LengthOrAuto::Length(Length::Em(
            2.0
        ))))
    );
}

#[test]
fn margin_side_accepts_negative_length() {
    // Task Non-goals: negative margin は spec-valid (§3.1 "Negative values
    // for margin properties are allowed")。longhand も含めて受理を pin。
    assert_eq!(
        parse("-10px", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Length(Length::Px(
            -10.0
        ))))
    );
}

#[test]
fn margin_top_accepts_zero() {
    // spec `<length-percentage> | auto` — 0 は valid length。`0px` は Dimension arm、
    // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す。
    // margin は non-negative filter を持たないため素通り。
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
    // CSS spec: ident keyword は ASCII case-insensitive。`AUTO` 受理を check
    // (expect_ident_matching が case-insensitive の証拠、helper 変更で
    // regression した際の canary)。
    assert_eq!(
        parse("AUTO", "margin-top"),
        Some(PropertyValue::MarginTop(LengthOrAuto::Auto))
    );
}

#[test]
fn margin_side_rejects_unsupported_unit() {
    // `cap` (§6.1.1 font-relative lengths) は現状
    // 未対応 (parse_length_value 側で drop)。`cm` / `lh` / `rlh` は
    // それぞれ受理側へ移った — margin-side helper に非依存で波及ドロップを pin。
    assert_eq!(parse("1cap", "margin-top"), None);
}

#[test]
fn margin_side_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
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
    // `<length-percentage> | auto` grammar 外 ident は declaration drop。
    assert_eq!(parse("fill-available", "margin-top"), None);
    assert_eq!(parse("initial", "margin-top"), None);
}

#[test]
fn margin_shorthand_one_value_spreads_all_sides() {
    // Verification 4-a (§3.2 "If there is only one component value, it
    // applies to all sides"): `margin: 10px` → 全 4 side = 10px。
    let want = Sides::all(LengthOrAuto::Length(Length::Px(10.0)));
    assert_eq!(parse("10px", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_two_values_top_bottom_and_right_left() {
    // Verification 4-b (§3.2 "If there are two values, the top and bottom
    // margins are set to the first value and the right and left margins
    // are set to the second"): top/bottom = 10px, right/left = 20px。
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
    // bottom = 30px。
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
    // top, right, bottom, and left, respectively"): clockwise from top。
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
    // Verification 5-a: `margin: auto` (1 value auto) → 全 4 side = Auto。
    // browser の "block-level centering" 慣用の parse pin。
    let want = Sides::all(LengthOrAuto::Auto);
    assert_eq!(parse("auto", "margin"), Some(PropertyValue::Margin(want)));
}

#[test]
fn margin_shorthand_zero_and_auto_horizontal_center() {
    // Verification 5-b: `margin: 0 auto` (2 value mixed) は block-level
    // horizontal centering の canonical form。top/bottom = 0px, right/left = auto。
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths> の
    // unitless-zero clause により bare `0` は Length::Px(0.0) 受理
    // (`parse_length_value_accepts_unitless_zero_only`
    // で pin)。
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
    // grammar coverage: 4-value shorthand で unit / auto を全て混在させる。
    // 32n `<length-percentage> | auto` の grammar 網羅を単一 assertion に集約。
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
    // 空 value: parse_margin_side 1st fail → parse_margin_shorthand `?`
    // 上位伝播で None (declaration drop)。
    assert_eq!(parse("", "margin"), None);
}

#[test]
fn margin_shorthand_rejects_bogus_ident() {
    // 1st value 位置に grammar 外 ident → declaration drop。
    assert_eq!(parse("bogus", "margin"), None);
}

#[test]
fn margin_shorthand_leaves_extra_values_for_caller_exhausted_check() {
    // 5+ value shorthand: 本 helper は 4 value 消費、5th 以降は unconsumed で
    // return。DeclParser::parse_value の expect_exhausted で最終的に
    // declaration drop されるので、rule.rs 側 test
    // (`margin_shorthand_five_values_declaration_dropped`) で end-to-end
    // 挙動を check する。本 test は parse_value 単体 (caller expect_exhausted
    // 経由なし) では 4 value までは Some が返る shape の pin。
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
    // shorthand path でも case-insensitive dispatch が生きている pin。
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
    // 4 longhand + shorthand variant → 対応 key (cascade winner 選択の
    // discriminant integrity)。sibling `position_key_maps_to_position_property_key`
    // と同 pattern。shorthand `Margin` key も expansion 前の PropertyValue
    // 段で観測可能なため includes。
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
    // 1 value → top/right/bottom/left が全て同値、Clone 経路 (最後の side は
    // move 消費) が正しく動く pin。
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
    // — `PropertyValue::PaddingInline` doc's "なぜ 8 longhand が専用
    // variant を持たないか" section).
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
    // cascade winner selection の discriminant integrity — 8 longhand は
    // 専用 key を持たず物理 key へ写像 (`margin_longhand_keys_map_correctly`
    // と同 pattern)。4 shorthand は自分専用の key を持つ。
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
    // Some(PropertyValue::BorderTopWidth(Length::Px(1.0)))。
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
// Initial: auto、Inherited: no。
//
// 本 task では `auto` + non-negative `<length-percentage>` のみ受理、
// min-content / max-content / fit-content() は (b) 非対応。

#[test]
fn width_parse_auto_keyword() {
    // Verification #1: `width: auto` → Width(Auto)。initial 値と同 shape で
    // grammar 上位優先分岐 (parse_width の try_parse ident branch) が生きて
    // いることを pin。
    assert_eq!(
        parse("auto", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn border_top_width_parse_medium_keyword() {
    // Verification #2: parse("medium", "border-top-width") =
    // Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))。
    // spec §3.3 規定値 (normative equivalence) の 1/3/5 px mapping。
    assert_eq!(
        parse("medium", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Px(3.0)))
    );
}

#[test]
fn width_parse_length_px() {
    // Verification #2: `width: 100px` → Width(Length(Px(100)))。
    // 従来 `unknown_property_returns_none` canary で `None` だった箇所が
    // 実 variant を返すようになった transition check (canary はその後
    // `float` を経て `cursor` に移設済み)。
    assert_eq!(
        parse("100px", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(
            100.0
        ))))
    );
}

#[test]
fn border_width_thin_thick_keywords_map_to_1px_5px() {
    // spec §3.3 規定値: thin=1px、thick=5px。
    // 4 side 各 arm の smoke — arm cross-copy regression check (`top` arm を
    // `right` arm に誤 wire しても本 test で fail する)。
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
    // Verification #3: `width: 50%` → Width(Length(Percent(50)))。
    // parse_length_value(allow_percentage=true) 経由の Percent branch。
    assert_eq!(
        parse("50%", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Percent(
            50.0
        ))))
    );
}

#[test]
fn border_width_rejects_negative() {
    // Verification #6: parse("-1px", "border-top-width") = None。
    // spec `<line-width>` = `<length [0,∞]>` — 負値は grammar 違反 → drop。
    assert_eq!(parse("-1px", "border-top-width"), None);
    // Em / Rem / Pt も同 constraint (unit-bearing variant 全て)。
    assert_eq!(parse("-1em", "border-bottom-width"), None);
}

#[test]
fn border_top_width_accepts_zero() {
    // spec `<line-width>` = `<length [0,∞]>` — 0 は閉区間下端。`0px` は
    // Dimension arm、bare `0` は CSS Values 3 §5 unitless-zero clause の
    // Number arm を通す。parse_border_width_side の
    // `>= 0.0` 非負 filter を pass。
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
    // は shorthand の width slot を bare-zero で埋めた canonical form。
    // parse_border_shorthand の width slot が parse_border_width_side_res 経由で
    // parse_length_value Number arm を通して Length::Px(0.0) を取り、
    // style slot は Solid、color slot は省略で spec initial =
    // `BorderColor::CurrentColor` (CSS Backgrounds 3 §3.1)。
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
    // `<line-width>` grammar は `<percentage>` を含まない (padding とは
    // 違う点)。`parse_length_value(input, false)` の
    // `<length>` mode で Percentage token 自体が reject される。
    assert_eq!(parse("50%", "border-top-width"), None);
}

#[test]
fn border_width_rejects_unknown_keyword() {
    // spec §3.3 の keyword 集合外は drop (`auto` / `fat` / `bold` etc.)。
    assert_eq!(parse("auto", "border-top-width"), None);
    assert_eq!(parse("fat", "border-top-width"), None);
}

#[test]
fn border_width_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "border-top-width"), None);
    assert_eq!(parse("initial", "border-top-width"), None);
    assert_eq!(parse("unset", "border-top-width"), None);
    assert_eq!(parse("revert", "border-top-width"), None);
    assert_eq!(parse("revert-layer", "border-top-width"), None);
}

#[test]
fn border_width_accepts_absolute_unit() {
    // `<line-width>` の `<length [0,∞]>` half は `<percentage>` を含まないが
    // 他 absolute unit は含む — 追加した `pc` を
    // border-width 経路 (`allow_percentage=false`) でも check する。
    assert_eq!(
        parse("1pc", "border-top-width"),
        Some(PropertyValue::BorderTopWidth(Length::Pc(1.0)))
    );
}

#[test]
fn border_width_rejects_negative_absolute_unit() {
    // 全 unit-bearing variant の non-negative check check (cm、新規 absolute unit)。
    assert_eq!(parse("-1cm", "border-top-width"), None);
}

#[test]
fn border_width_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。`<line-width>`
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
    // Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))。
    assert_eq!(
        parse("solid", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
    );
}

#[test]
fn width_parse_length_em() {
    // font-relative unit 経路 check — parse_length_value 経由で em を受理。
    assert_eq!(
        parse("2em", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Em(2.0))))
    );
}

#[test]
fn border_style_all_10_variants_accepted() {
    // spec §3.2 `<line-style>` の 10 alternative 全てを smoke (arm 削り
    // regression 検知)。
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
    // Verification #4: `width: -10px` → None (spec grammar `[0,∞]` violation)。
    // parse_width の post-filter が enforce (padding と同 pattern)。
    assert_eq!(parse("-10px", "width"), None);
}

#[test]
fn width_rejects_negative_percentage() {
    // 全 Length variant OR-pattern check の check (Percent 分岐)。
    assert_eq!(parse("-50%", "width"), None);
}

#[test]
fn width_rejects_negative_em() {
    // 全 Length variant OR-pattern check の check (Em 分岐)。
    assert_eq!(parse("-2em", "width"), None);
}

#[test]
fn width_accepts_zero() {
    // spec `[0,∞]` の closed interval — 下端 0 は有効。
    assert_eq!(
        parse("0px", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
    );
    // CSS Values 3 §5 <https://www.w3.org/TR/css-values-3/#lengths>
    // unitless-zero clause 経由: bare `0` も同 Px(0.0)
    // として受理 (width は `<length-percentage [0,∞]>`、helper が Number arm で
    // 拾い parse_width の非負 filter を pass)。
    assert_eq!(
        parse("0", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(0.0))))
    );
}

#[test]
fn border_style_rejects_unknown_keyword() {
    // `<line-style>` grammar 外 (`wavy` は CSS Text Decoration 4 由来、
    // border-style では invalid) は drop。
    assert_eq!(parse("wavy", "border-top-style"), None);
}

#[test]
fn border_style_rejects_css_wide_keyword() {
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "border-top-style"), None);
    assert_eq!(parse("initial", "border-top-style"), None);
    assert_eq!(parse("unset", "border-top-style"), None);
    assert_eq!(parse("revert", "border-top-style"), None);
    assert_eq!(parse("revert-layer", "border-top-style"), None);
}

#[test]
fn border_style_case_insensitive() {
    // CSS Values 3 §3.1: keyword は ASCII case-insensitive。
    assert_eq!(
        parse("SOLID", "border-top-style"),
        Some(PropertyValue::BorderTopStyle(BorderStyle::Solid))
    );
}

#[test]
fn width_rejects_min_content_keyword() {
    // Intrinsic sizing keyword — now accepted as valid parsing (placeholder Auto).
    assert_eq!(
        parse("min-content", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
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
    // (b) 非対応 — vw / cap 等は spec-valid だが
    // 未対応、parse_length_value 側で drop、None
    // propagate。`ch` / `lh` / `rlh` は
    // それぞれ受理側へ移った
    // (`width_accepts_absolute_unit` / `width_accepts_lh` 参照)。
    assert_eq!(parse("10vw", "width"), None);
    assert_eq!(parse("5cap", "width"), None);
}

#[test]
fn width_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
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
    // `1in` = 96px 相当 (specified 層は authored unit をそのまま保持、
    // 絶対化は resolve.rs の責務 — check: `resolve::tests::length_additional_absolute_units_convert_per_spec_table`)。
    assert_eq!(
        parse("1in", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::In(1.0))))
    );
}

#[test]
fn width_case_insensitive_auto() {
    // CSS spec: ident keyword は ASCII case-insensitive。`AUTO` 受理を check
    // (sibling `margin_side_case_insensitive_auto` と同 pattern)。
    assert_eq!(
        parse("AUTO", "width"),
        Some(PropertyValue::Width(LengthOrAuto::Auto))
    );
}

#[test]
fn border_top_color_parse_hex() {
    // border-*-color の hex form は `parse_color` (background-color と同じ
    // helper) が hex/named/rgb(a)/transparent を受理し、`parse_border_color`
    // が `BorderColor::Resolved` で wrap して cascade static side に届く。
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
    // 4 side 各 arm の smoke + 3 color form (named / rgb / transparent) を
    // 分散して cross-arm regression 検知 (background-color test の pattern)。
    // `BorderColor::Resolved` wrap。
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
    // "Initial: currentcolor" — author 明示 `border-*-color: currentcolor` が
    // `BorderColor::CurrentColor` variant として保持されることを check する
    // (hazard case 1 の cascade-side coverage、used-value resolution は
    // paint scope 責務)。
    assert_eq!(
        parse("currentcolor", "border-top-color"),
        Some(PropertyValue::BorderTopColor(BorderColor::CurrentColor))
    );
    // CSS Color 3 §4.4 keyword は ASCII case-insensitive。
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
    // Verification #5: parse("1px solid red", "border") = shorthand 経由で
    // 全 4 side の Border {width: 1px, style: Solid, color: red} を expand。
    // color slot は `BorderColor::Resolved` に wrap。
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
    // spec §3.4 grammar は `||` (any-order)。全 6 permutation を check する
    // 代わりに 3 order (color-first / style-first / mixed) を smoke。
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
    // width 省略 → medium (3px)、style 省略 → None、color 省略 →
    // `currentcolor` keyword (`BorderColor::CurrentColor`、spec §3.1
    // initial)。
    // 1 component only (color) — width と style は initial:
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
    // 1 component only (style) — width と color は initial:
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
    // sibling: border shorthand の color slot は 4 longhand と同じ
    // `parse_border_color` を経由するため、`currentcolor` keyword も
    // shorthand から受理される。
    let expected = Border {
        width: Length::Px(1.0),
        style: BorderStyle::Solid,
        color: BorderColor::CurrentColor,
    };
    assert_eq!(
        parse("1px solid currentcolor", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
    // 引数 order は自由。style first。
    assert_eq!(
        parse("solid currentcolor 1px", "border"),
        Some(PropertyValue::Border(Sides::all(expected)))
    );
}

#[test]
fn border_shorthand_empty_returns_none() {
    // `||` grammar は at least 1 component 必須。0 component は None。
    assert_eq!(parse("", "border"), None);
}

#[test]
fn border_shorthand_unknown_keyword_only_returns_none() {
    // 未知 keyword (width/style/color いずれの slot にも match しない) は
    // 1st iteration で全 slot None、`matched=false` で break、0-component
    // guard で None (declaration drop)。
    assert_eq!(parse("garbage", "border"), None);
}

#[test]
fn border_shorthand_two_widths_leaves_leftover_for_caller_exhausted_check() {
    // `border: 1px 2px` — 1st iteration で width=1px、2nd iteration で
    // width slot 満了、`2px` は他 slot (style/color) に match しないため
    // fall-through break。leftover は caller の `expect_exhausted` 責務。
    // 本 helper 単体としては 1st を確保して Some を返す (parse_value 経路
    // では end-to-end で declaration drop する — rule.rs test で check 予定)。
    let expected = Border {
        width: Length::Px(1.0),
        style: BorderStyle::None,
        color: BorderColor::CurrentColor, // spec §3.1 initial
    };
    let mut input = ParserInput::new("1px 2px");
    let mut parser = Parser::new(&mut input);
    let result = parse_border_shorthand(&mut parser);
    assert_eq!(result, Some(Sides::all(expected)));
    // 2px は unconsumed のまま — parser cursor は "2px" の直前を指す。
    assert!(!parser.is_exhausted());
}

#[test]
fn border_longhand_keys_map_correctly() {
    // 12 longhand + 1 shorthand variant → 対応 key (cascade winner 選択の
    // discriminant integrity)。sibling `margin_longhand_keys_map_correctly`
    // と同 pattern。
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
    // PropertyValue::Width → PropertyKey::Width (cascade winner 選択の
    // discriminant integrity、既存 sibling padding/margin と同じ pattern)。
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
fn logical_size_aliases_use_physical_horizontal_writing_mode_axes() {
    // CSS Sizing 3 logical preferred-size properties map to width/height
    // while this engine's supported writing mode is horizontal-tb.
    assert_eq!(
        property_key_for_name("inline-size"),
        Some(PropertyKey::Width)
    );
    assert_eq!(
        property_key_for_name("block-size"),
        Some(PropertyKey::Height)
    );
    assert_eq!(
        parse("120px", "inline-size"),
        Some(PropertyValue::Width(LengthOrAuto::Length(Length::Px(
            120.0
        ))))
    );
    assert_eq!(
        parse("80px", "block-size"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(
            80.0
        ))))
    );
}

// ── height (CSS Sizing 3 §3.1.1) ─────────────
//
// Primary source:
// - #preferred-size-properties: `auto | <length-percentage [0,∞]> |
//   min-content | max-content | fit-content(<length-percentage>)`,
//   initial `auto`, Inheritance `No`.
//
// 現状 scope は `auto` + 非負 `<length-percentage>` の 2 分岐のみ、
// 他 sizing keyword / global keyword / calc() / var() は silent drop
// (parse_height doc の Scope carving 節参照)。
//
// sibling: sibling `width` と同 shape の非負 `<length-percentage>` +
// `auto` grammar、payload 型は共通 `LengthOrAuto`。

#[test]
fn height_parse_auto() {
    // Verification 1 (task doc): `auto` ident は spec initial value でもある
    // (§3.1.1 "Initial: auto") — cascade winner として declaration が到達
    // した場合の受理 pattern を pin。
    assert_eq!(
        parse("auto", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_parse_px() {
    // Verification 2 (task doc): 非負 px は spec-valid `<length-percentage>`
    // (§3.1.1)。sibling `width_parse_length_px` と同 shape。
    assert_eq!(
        parse("100px", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(Length::Px(
            100.0
        ))))
    );
}

#[test]
fn height_parse_percentage() {
    // Verification 3 (task doc): percentage 受理 (parse_length_value の
    // allow_percentage = true 経路)。resolve (containing block % → 実寸)
    // は下流責務。
    assert_eq!(
        parse("50%", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Length(
            Length::Percent(50.0)
        )))
    );
}

#[test]
fn height_rejects_negative_length() {
    // Verification 4 (task doc): `<length-percentage [0,∞]>` (§3.1.1) の
    // 非負制約により `-10px` は spec-invalid → drop。sibling
    // padding の非負フィルタ pattern と同 shape、margin の `-10px` 受理
    // (§3.1) との対称的な reject を pin。
    assert_eq!(parse("-10px", "height"), None);
}

#[test]
fn height_accepts_zero() {
    // spec `<length-percentage [0,∞]>` — 0 は閉区間下端。`0px` は Dimension arm、
    // bare `0` は CSS Values 3 §5 unitless-zero clause の Number arm を通す。
    // parse_height の `>= 0.0` 非負 filter を pass。
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
    // 非負フィルタが Percent variant にも効く check (parse_padding_side の
    // 同 pattern、Verification 4 の姉妹)。
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
    // (b) 非対応 — CSS-wide keyword は未実装 (将来対応)、silent drop。
    // canonical: PropertyValue doc「CSS-wide keyword」節。
    assert_eq!(parse("inherit", "height"), None);
    assert_eq!(parse("initial", "height"), None);
    assert_eq!(parse("unset", "height"), None);
    assert_eq!(parse("revert", "height"), None);
    assert_eq!(parse("revert-layer", "height"), None);
}

#[test]
fn height_case_insensitive_auto() {
    // CSS spec: ident keyword は ASCII case-insensitive
    // (`expect_ident_matching` の cssparser 慣行、sibling
    // `margin_side_case_insensitive_auto` と同 pattern)。
    assert_eq!(
        parse("AUTO", "height"),
        Some(PropertyValue::Height(LengthOrAuto::Auto))
    );
}

#[test]
fn height_rejects_unsupported_unit() {
    // `cap` (§6.1.1 font-relative lengths) は現状
    // 未対応 (parse_length_value 側で drop)。`cm` / `lh` / `rlh` は
    // それぞれ受理側へ移った (`height_accepts_absolute_unit` /
    // `height_accepts_lh` 参照)。sibling
    // `margin_side_rejects_unsupported_unit` と同 pattern。
    assert_eq!(parse("1cap", "height"), None);
}

#[test]
fn height_accepts_lh() {
    // CSS Values 4 §6.1.1 `lh`/`rlh`。
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
    // CSS Values 4 §6.2 absolute lengths。
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
    // grammar coverage: font-relative units (`em` / `rem`) も
    // `<length-percentage>` mode で受理される。resolve は下流
    // (font-size context / root font-size context) 責務。
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
    // sibling `margin_longhand_keys_map_correctly` と同 pattern — cascade
    // winner selection の discriminant integrity を pin。
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
        parse("1px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Px(1.0),
            bottom_right: Length::Px(1.0),
            bottom_left: Length::Px(1.0),
        }))
    );
    assert_eq!(
        parse("1px 2em", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Em(2.0),
            bottom_right: Length::Px(1.0),
            bottom_left: Length::Em(2.0),
        }))
    );
    assert_eq!(
        parse("1px 2px 3px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Px(2.0),
            bottom_right: Length::Px(3.0),
            bottom_left: Length::Px(2.0),
        }))
    );
    assert_eq!(
        parse("1px 2px 3px 4px", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Px(1.0),
            top_right: Length::Px(2.0),
            bottom_right: Length::Px(3.0),
            bottom_left: Length::Px(4.0),
        }))
    );
}

#[test]
fn border_radius_accepts_percentages_and_rejects_negative_lengths() {
    assert_eq!(
        parse_entire("50% 25%", "border-radius"),
        Some(PropertyValue::BorderRadius(BorderRadius {
            top_left: Length::Percent(50.0),
            top_right: Length::Percent(25.0),
            bottom_right: Length::Percent(50.0),
            bottom_left: Length::Percent(25.0),
        }))
    );
    assert_eq!(
        parse_entire("25%", "border-top-left-radius"),
        Some(PropertyValue::BorderRadiusTopLeft(Length::Percent(25.0)))
    );
    assert_eq!(
        parse_entire("inherit", "border-radius"),
        Some(PropertyValue::BorderRadiusInherit)
    );
    assert_eq!(parse_entire("1px -2px", "border-radius"), None);
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
    // doc参照).
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
