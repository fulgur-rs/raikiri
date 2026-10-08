use super::*;
use crate::property::{
    FontFeatureSetting, FontFeatureSettings, FontSynthesisStyle, FontVariantEastAsianVariant,
    FontVariantEastAsianWidth, FontVariationSetting, FontVariationSettings, GeometryBox,
    HyphenateLimitChars, HyphenateLimitCharsValue, Length, TextShadowColor,
};
use crate::resolve::{
    ComputedGridTrackBreadth, ComputedGridTrackList, ComputedGridTrackListComponent,
};

#[test]
fn custom_property_environment_equality_uses_effective_bindings() {
    let empty = empty_custom_properties();
    let inherited = CustomPropertyEnvironment::from_local(
        &empty,
        HashMap::from([(SmolStr::from("--x"), Some(SmolStr::from("red")))]),
    );
    let redeclared = CustomPropertyEnvironment::from_local(
        &inherited,
        HashMap::from([(SmolStr::from("--x"), Some(SmolStr::from("red")))]),
    );
    assert_eq!(inherited, redeclared);

    // A tombstone removes an inherited binding, just as the old flat map
    // did; an absent name and a tombstone are therefore equivalent.
    let tombstone = CustomPropertyEnvironment::from_local(
        &inherited,
        HashMap::from([(SmolStr::from("--x"), None)]),
    );
    assert_eq!(tombstone, empty);
    assert_ne!(tombstone, inherited);

    let debug = format!("{inherited:?}");
    assert!(debug.contains("has_parent: true"));
}

#[test]
fn custom_property_environment_drop_is_iterative_for_deep_chain() {
    const DEPTH: usize = 5_000;
    let handle = std::thread::Builder::new()
        .name("custom-property-environment-drop".into())
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut environment = empty_custom_properties();
            for index in 0..DEPTH {
                let local = HashMap::from([(
                    SmolStr::from(format!("--x{index}")),
                    Some(SmolStr::from("value")),
                )]);
                environment = CustomPropertyEnvironment::from_local(&environment, local);
            }
            drop(environment);
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn initial_values_match_spec() {
    let cv = ComputedValues::initial();
    assert_eq!(cv.color, CssColor::BLACK);
    // CSS Backgrounds 3 §2.2: the initial background-color is `transparent`
    // (= rgba(0, 0, 0, 0))
    assert_eq!(cv.background_color, CssColor::TRANSPARENT);
    // Keep a literal here (for the same reason as the `INITIAL_FONT_SIZE_PX` check:
    // using `initial_font_family()` would make this test self-referential
    // and hide mistakes in that helper). Dereference `Arc<Vec<Atom>>` to
    // `Vec<Atom>` with `*cv.font_family` before comparing.
    assert_eq!(
        *cv.font_family,
        vec![crate::property::FontFamilyName::generic("serif")]
    );
    // CSS Fonts 4 §2.5: the initial font-size is `medium`, or 16px here
    // (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>).
    // **This 16.0 is intentionally a literal**: using `INITIAL_FONT_SIZE_PX`
    // would hide mistakes in that constant (see its documentation too).
    assert_eq!(cv.font_size, ComputedLength(16.0));
    assert_eq!(cv.font_weight, 400.0);
    // CSS Inline 3 §5.1: the initial line-height is `normal`
    assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    assert_eq!(cv.display, DisplayValue::Inline);
    assert_eq!(cv.list_style_type, ListStyleType::Disc);
    assert_eq!(cv.list_style_position, ListStylePosition::Outside);
    // CSS Lists 3 §4: the specified initial counter-* value is `none`,
    // represented here as an empty list (<https://www.w3.org/TR/css-lists-3/#auto-numbering>).
    assert!(cv.counter_reset.is_empty());
    assert!(cv.counter_increment.is_empty());
    assert!(cv.counter_set.is_empty());
    // CSS Content 3 §1 (content: Initial: normal) + CSS GCPM 3 §1.1.1
    // (string-set: Initial: none) — both are represented as empty lists
    assert!(cv.content.is_empty());
    assert!(cv.string_set.is_empty());
    // CSS GCPM 3 §1.2.1: the initial position is `static`, so
    // there is no running() seed.
    assert!(cv.running_templates.is_empty());
    // CSS Text 3 §6.1: the initial text-align is `start`.
    assert_eq!(cv.text_align, TextAlign::Start);
    // CSS Text 4: text-spacing-trim initial is `normal`.
    assert_eq!(cv.text_spacing_trim, TextSpacingTrim::Normal);
    // CSS Text Decoration 4: text-decoration-skip-ink initial is `auto`.
    assert_eq!(cv.text_decoration_skip_ink, TextDecorationSkipInk::Auto);
    // CSS Text Decoration 4: text-decoration-skip-spaces initial is `start end`.
    assert_eq!(
        cv.text_decoration_skip_spaces,
        TextDecorationSkipSpaces::StartEnd
    );
    // CSS Writing Modes 4 §2.1: the initial direction is `ltr`.
    assert_eq!(cv.direction, Direction::Ltr);
    // CSS Fonts 4 §2.4: the initial font-style is `normal`.
    assert_eq!(cv.font_style, FontStyle::Normal);
    assert_eq!(cv.font_kerning, FontKerning::Auto);
    assert_eq!(cv.font_optical_sizing, FontOpticalSizing::Auto);
    assert_eq!(cv.font_variant_emoji, FontVariantEmoji::Normal);
    assert_eq!(cv.font_language_override, FontLanguageOverride::Normal);
    assert_eq!(cv.font_variant_ligatures, FontVariantLigatures::Normal);
    assert_eq!(cv.font_synthesis, FontSynthesisValue::initial());
    assert_eq!(cv.font_variant_position, FontVariantPosition::Normal);
    assert_eq!(cv.font_palette, FontPaletteValue::Normal);
    assert_eq!(cv.font_variant_numeric, FontVariantNumeric::initial());
    assert_eq!(cv.font_variant_east_asian, FontVariantEastAsian::initial());
    assert_eq!(cv.font_variation_settings, FontVariationSettings::Normal);
    assert_eq!(cv.font_feature_settings, FontFeatureSettings::Normal);
    // CSS Fonts Module Level 3 §6.6: the initial font-variant-caps is
    // `normal`.
    assert_eq!(cv.font_variant_caps, FontVariantCaps::Normal);
    // CSS Text Module Level 3 §2.1: the initial text-transform is `none`.
    assert_eq!(cv.text_transform, TextTransform::None);
    assert_eq!(cv.text_combine_upright, TextCombineUpright::None);
    assert_eq!(cv.text_orientation, TextOrientation::Mixed);
    assert_eq!(cv.unicode_bidi, UnicodeBidi::Normal);
    // CSS Display 3 §4: the initial visibility is `visible`.
    assert_eq!(cv.visibility, Visibility::Visible);
    // CSS Text 3 §8.1: the initial text-indent is `0`.
    assert_eq!(cv.text_indent, ComputedTextIndent::Px(0.0));
    // CSS Text 3 §5.1: the initial word-break is `normal`.
    assert_eq!(cv.word_break, WordBreak::Normal);
    // CSS Text 3 §5.4: the initial overflow-wrap is `normal`.
    assert_eq!(cv.overflow_wrap, OverflowWrap::Normal);
    // CSS Text 4: word-space-transform initial is `none`.
    assert_eq!(cv.word_space_transform, WordSpaceTransform::None);
    // `letter-spacing: normal` keeps a zero CSS computed value and the same
    // zero renderer fallback.
    assert_eq!(cv.letter_spacing, ComputedLength::ZERO);
    assert_eq!(cv.letter_spacing_computed, ComputedLetterSpacing::Px(0.0));
    assert_eq!(cv.word_spacing, ComputedLength::ZERO);
    assert_eq!(cv.word_spacing_computed, ComputedLetterSpacing::Px(0.0));
    // CSS Box 3 §4.1: the initial padding is 0 on all four sides.
    assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    // CSS Box 3 §3.1: the initial margin is 0 on each side.
    assert_eq!(
        cv.margin,
        Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))
    );
    // CSS Backgrounds 3 §3 (now using currentcolor): the initial border has
    // {style: none, color: `currentcolor` (BorderColor::CurrentColor)} on each side.
    // Enum coverage for hazard case 2 (author `color:red` with border-color omitted,
    // so the cascade sends the static side straight to its initial value).
    // At paint time, used-value resolution resolves against the `color` property.
    //
    // The width is **0px**: the specified initial value is `medium` (3px), but CSS
    // Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
    // says "Computed value: … zero if the border style is `none` or `hidden`",
    // so the computed value is gated to zero. The specified
    // initial value is checked in `crate::specified` by
    // `initial_border_width_is_gated_to_zero_at_computed_layer`.
    assert_eq!(
        cv.border,
        Sides::all(ComputedBorder {
            width: ComputedLength(0.0),
            style: BorderStyle::None,
            color: BorderColor::CurrentColor,
        })
    );
    assert_eq!(
        cv.border_radius,
        ComputedBorderRadius::all(ComputedLength::ZERO)
    );
    assert!(cv.box_shadow.is_empty());
    assert_eq!(cv.outline.width(), ComputedLength::ZERO);
    assert_eq!(cv.outline.style(), OutlineStyle::None);
    assert_eq!(cv.outline.color, OutlineColor::Invert);
    // CSS Sizing 3 §3.1.1: the initial width is `auto`.
    assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    // CSS Sizing 3 §3.1.1: the initial height is `auto`.
    assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Auto);
    // CSS Sizing 3 §3.3: the initial box-sizing is `content-box`.
    assert_eq!(cv.box_sizing, BoxSizing::ContentBox);
    // CSS2 §9.9.1: the initial z-index is `auto`.
    assert_eq!(cv.z_index, ZIndexValue::Auto);
    // CSS Fragmentation Module Level 3 §3.1 / §3.2: the initial values of
    // break-before / break-after / break-inside are all `auto`.
    assert_eq!(cv.break_before, BreakBetween::Auto);
    assert_eq!(cv.break_after, BreakBetween::Auto);
    assert_eq!(cv.break_inside, BreakInside::Auto);
    // CSS2 §9.5.1 / §9.5.2: the initial float / clear values are both `none`.
    assert_eq!(cv.float, FloatValue::None);
    assert_eq!(cv.clear, ClearValue::None);
    // CSS Text 3 §3: the initial white-space is `normal`.
    assert_eq!(cv.white_space, WhiteSpace::Normal);
    // CSS Text 4: white-space-collapse initial is `collapse`.
    assert_eq!(cv.white_space_collapse, WhiteSpaceCollapse::Collapse);
    // CSS Text 4 §5.4: text-wrap-style initial is `auto`.
    assert_eq!(cv.text_wrap_style, TextWrapStyle::Auto);
    // CSS Text 3 §5.3: the initial hyphens is `manual`.
    assert_eq!(cv.hyphens, Hyphens::Manual);
    // CSS Text 4: hyphenate-character initial is `auto`.
    assert_eq!(cv.hyphenate_character, HyphenateCharacter::Auto);
    assert_eq!(cv.hyphenate_limit_chars, HyphenateLimitChars::INITIAL);
    assert_eq!(cv.text_emphasis_style, TextEmphasisStyle::None);
    assert_eq!(cv.text_emphasis_color, TextDecorationColor::CurrentColor);
}

#[test]
fn computed_values_is_send_and_clone() {
    fn assert_send<T: Send>() {}
    fn assert_clone<T: Clone>() {}
    assert_send::<ComputedValues>();
    assert_clone::<ComputedValues>();
}

// ── display initial ─────

#[test]
fn initial_display_is_inline() {
    // CSS Display 3 §2: the initial display is `inline`
    // (see the anchor in the `ComputedValues::display` field documentation).
    assert_eq!(ComputedValues::initial().display, DisplayValue::Inline);
}

// ── inherit_from (delegation contract) ──

/// A parent fixture with every field set away from its initial value.
///
/// This lets the test detect errors in either inherited or non-inherited fields:
/// **every field has a non-initial value**. If a non-inherited field leaks from
/// the parent, it differs from the initial value; if an inherited field resets, it differs from the parent.
fn non_initial_parent() -> ComputedValues {
    ComputedValues {
        color: CssColor {
            r: 200,
            g: 100,
            b: 50,
            a: 255,
        },
        background_color_expression: None,
        background_color: CssColor {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        },
        font_family: Arc::new(vec![crate::property::FontFamilyName::generic("sans-serif")]),
        font_size: ComputedLength(24.0),
        font_weight: 700.0,
        line_height: ComputedLineHeight::Length(ComputedLength(30.0)),
        display: DisplayValue::Block,
        list_style_type: ListStyleType::Named(SmolStr::new("upper-roman")),
        list_style_position: ListStylePosition::Inside,
        list_style_image: BackgroundImage::Url("marker.png".into()),
        counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
        counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
        counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
        content: Arc::new(vec![ContentComponent::Literal(SmolStr::new("x"))]),
        string_set: Arc::new(vec![(SmolStr::new("s"), Vec::new())]),
        running_templates: vec![RunningTemplate {
            name: SmolStr::new("hdr"),
        }],
        text_align: TextAlign::Center,
        hanging_punctuation: HangingPunctuation::First,
        // CSS Text 4: explicit autospace and word-space-transform values for inheritance coverage.
        text_autospace: TextAutospace::NoAutospace,
        word_space_transform: WordSpaceTransform::IdeographicSpaceAutoPhrase,
        text_spacing_trim: TextSpacingTrim::TrimBoth,
        // A non-initial value, following the fixture rule above for every field.
        text_justify: TextJustify::InterWord,
        text_align_last: TextAlignLast::Justify,
        // CSS Writing Modes 4 §2.1: `Rtl` differs from the initial `Ltr`
        // (as required for every field of non_initial_parent).
        direction: Direction::Rtl,
        // CSSOM's `cssom_writing_mode: VerticalRl` is reachable. Setting the
        // renderer-facing `writing_mode` to this same value is synthetic:
        // real cascades keep that field at `HorizontalTb`. This lets the test
        // prove inheritance copies the CSSOM value and re-applies the renderer
        // fallback rather than copying `parent.writing_mode` verbatim.
        writing_mode: WritingMode::VerticalRl,
        cssom_writing_mode: WritingMode::VerticalRl,
        ruby_position: RubyPosition::Under,
        // CSS Text 3 §8.1: differs from the initial `0`
        // (as required for every field of non_initial_parent).
        text_indent: ComputedTextIndent::Px(9.0),
        text_indent_ch_factor: None,
        text_indent_ch_offset: 0.0,
        text_indent_ch_font: None,
        text_indent_ch_inherited: false,
        text_indent_hanging: false,
        text_indent_each_line: false,
        padding: Sides::all(ComputedLengthPercentage::Px(7.0)),
        padding_ch: Sides::all(None),
        margin: Sides::all(ComputedLengthPercentageOrAuto::Px(12.0)),
        margin_ch: Sides::all(None),
        border: Sides::all(ComputedBorder {
            width: ComputedLength(5.0),
            style: BorderStyle::Solid,
            color: BorderColor::Resolved(CssColor::BLACK),
        }),
        border_radius: ComputedBorderRadius {
            top_left: ComputedLengthPercentage::Px(1.0),
            top_right: ComputedLengthPercentage::Px(2.0),
            bottom_right: ComputedLengthPercentage::Px(3.0),
            bottom_left: ComputedLengthPercentage::Px(4.0),
        },
        box_shadow: Arc::new(vec![ComputedBoxShadowItem {
            offset_x: ComputedLength(1.0),
            offset_y: ComputedLength(2.0),
            blur_radius: ComputedLength(3.0),
            spread_radius: ComputedLength(4.0),
            color: TextShadowColor::Resolved(CssColor::BLACK),
            inset: false,
        }]),
        outline: ComputedOutline {
            width: ComputedLength(4.0),
            style: OutlineStyle::Solid,
            color: OutlineColor::Resolved(CssColor::BLACK),
        },
        outline_offset: ComputedLength(5.0),
        width: ComputedLengthPercentageOrAuto::Px(200.0),
        width_ch: None,
        height: ComputedLengthPercentageOrAuto::Px(200.0),
        height_ch: None,
        max_width: ComputedLengthPercentageOrAuto::Px(200.0),
        max_height: ComputedLengthPercentageOrAuto::Px(200.0),
        min_width: ComputedLengthPercentageOrAuto::Px(200.0),
        min_height: ComputedLengthPercentageOrAuto::Px(200.0),
        min_block_size: None,
        vertical_logical_size: None,
        top: ComputedLengthPercentageOrAuto::Px(10.0),
        right: ComputedLengthPercentageOrAuto::Px(20.0),
        bottom: ComputedLengthPercentageOrAuto::Px(30.0),
        left: ComputedLengthPercentageOrAuto::Px(40.0),
        position: PositionValue::Relative,
        box_sizing: BoxSizing::BorderBox,
        // CSS Overflow 3 §3.1: `Hidden`/`Scroll` — non-initial (`visible`)
        // pair, and one that is also stable under `resolve_overflow`
        // (neither axis is `visible`/`clip`, so the cross-axis coupling
        // is a no-op here) so this fixture stays a plain "non-initial
        // parent", not an accidental probe of the coupling itself.
        overflow: OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        },
        // All three fields differ from their initial values
        // (as required for every field of non_initial_parent).
        text_decoration_line: TextDecorationLine::UNDERLINE,
        text_decoration_style: TextDecorationStyle::Wavy,
        text_decoration_color: TextDecorationColor::Resolved(CssColor::BLACK),
        text_decoration_thickness: ComputedTextDecorationThickness::Length(ComputedLength(6.0)),
        text_decoration_skip_ink: TextDecorationSkipInk::All,
        text_decoration_skip_spaces: TextDecorationSkipSpaces::End,
        text_decoration_inset: ComputedTextDecorationInset::Lengths {
            start: ComputedLength(3.0),
            end: ComputedLength(4.0),
        },
        text_decoration_inset_start_ch: None,
        text_decoration_inset_end_ch: None,
        text_underline_offset: ComputedTextUnderlineOffset::Length(ComputedLength(5.0)),
        text_underline_position: TextUnderlinePosition {
            from_font: false,
            under: true,
            left: true,
            right: false,
        },
        text_emphasis_position: TextEmphasisPosition::Position {
            vertical: TextEmphasisVEdge::Under,
            horizontal: Some(TextEmphasisHEdge::Left),
        },
        text_emphasis_style: TextEmphasisStyle::String(SmolStr::new("*")),
        text_emphasis_color: TextDecorationColor::Resolved(CssColor {
            r: 12,
            g: 34,
            b: 56,
            a: 255,
        }),
        // `Sub` differs from the initial `Baseline` (as required for
        // every field of non_initial_parent).
        vertical_align: VerticalAlign::Sub,
        // CSS Fonts 4 §2.4: `Italic` differs from the initial `Normal`
        // (as required for every field of non_initial_parent).
        font_style: FontStyle::Italic,
        font_kerning: FontKerning::None,
        font_optical_sizing: FontOpticalSizing::None,
        font_variant_emoji: FontVariantEmoji::Emoji,
        font_language_override: FontLanguageOverride::String(SmolStr::new("KSW")),
        font_variant_ligatures: FontVariantLigatures::NoHistoricalLigatures,
        font_synthesis: FontSynthesisValue {
            weight: false,
            style: FontSynthesisStyle::ObliqueOnly,
            small_caps: true,
            position: false,
        },
        font_variant_position: FontVariantPosition::Super,
        font_palette: FontPaletteValue::Palette(SmolStr::new("--parent")),
        font_variant_numeric: FontVariantNumeric {
            oldstyle_nums: true,
            tabular_nums: true,
            stacked_fractions: true,
            ordinal: true,
            slashed_zero: true,
            ..FontVariantNumeric::initial()
        },
        font_variant_east_asian: FontVariantEastAsian {
            variant: Some(FontVariantEastAsianVariant::Jis78),
            width: Some(FontVariantEastAsianWidth::ProportionalWidth),
            ruby: true,
        },
        font_variation_settings: FontVariationSettings::Settings(vec![FontVariationSetting {
            tag: SmolStr::new("wght"),
            value: 640.0,
        }]),
        font_feature_settings: FontFeatureSettings::Features(vec![FontFeatureSetting {
            tag: *b"kern",
            value: 0,
        }]),
        // CSS Fonts Module Level 3 §6.6: `SmallCaps` differs from the initial
        // `Normal` (as required for every field of
        // non_initial_parent).
        font_variant_caps: FontVariantCaps::SmallCaps,
        // CSS Text Module Level 3 §2.1: `Uppercase` differs from the initial
        // `None` (as required for every field of
        // non_initial_parent).
        text_transform: TextTransform::Uppercase,
        text_combine_upright: TextCombineUpright::All,
        text_orientation: TextOrientation::Sideways,
        unicode_bidi: UnicodeBidi::IsolateOverride,
        // CSS Display 3 §4: `Hidden` differs from the initial `Visible`
        // (as required for every field of non_initial_parent).
        visibility: Visibility::Hidden,
        // CSS2 §9.9.1: `Integer(3)` differs from the initial `Auto`
        // (as required for every field of non_initial_parent).
        z_index: ZIndexValue::Integer(3),
        // CSS Text 3 §5.1: `KeepAll` differs from the initial `Normal`
        // (as required for every field of non_initial_parent).
        word_break: WordBreak::KeepAll,
        line_break: LineBreak::Auto,
        // CSS Text 3 §5.4: `Anywhere` differs from the initial `Normal`
        // (as required for every field of non_initial_parent).
        overflow_wrap: OverflowWrap::Anywhere,
        // CSS Text 3 §7.2 / §7.1: differs from the initial `0` (the computed
        // value of `normal`), so every field of non_initial_parent stays
        // non-initial.
        letter_spacing: ComputedLength(2.0),
        letter_spacing_computed: ComputedLetterSpacing::Px(2.0),
        letter_spacing_ch_factor: None,
        letter_spacing_ch_offset: 0.0,
        letter_spacing_ch_font: None,
        word_spacing: ComputedLength(4.0),
        word_spacing_computed: ComputedLetterSpacing::Px(4.0),
        word_spacing_ch_factor: None,
        word_spacing_ch_offset: 0.0,
        word_spacing_ch_font: None,
        // CSS Text Module Level 3 §4.2: differs from the initial `8`
        // (as required for every field of non_initial_parent).
        tab_size: ComputedTabSize::Number(3.0),
        // CSS Fragmentation Module Level 3 §3.1 / §3.2: differs from the initial
        // `Auto` (as required for every field of
        // non_initial_parent).
        break_before: BreakBetween::Page,
        break_after: BreakBetween::AvoidPage,
        break_inside: BreakInside::AvoidPage,
        // CSS2 §9.5.1 / §9.5.2: `Left`/`Both` differ from the initial `None`/`None`
        // (as required for every field of
        // non_initial_parent).
        float: FloatValue::Left,
        clear: ClearValue::Both,
        // CSS Text 3 §3: `Pre` differs from the initial `Normal`
        // (as required for every field of non_initial_parent).
        white_space: WhiteSpace::Pre,
        // CSS Text 4: `PreserveBreaks` differs from the `Collapse` initial.
        white_space_collapse: WhiteSpaceCollapse::PreserveBreaks,
        text_wrap: TextWrapMode::Nowrap,
        text_wrap_style: TextWrapStyle::Stable,
        effective_white_space_collapse: WhiteSpaceCollapse::PreserveBreaks,
        effective_text_wrap_mode: TextWrapMode::Nowrap,
        // CSS Text 3 §5.3: `None` differs from the initial `Manual`
        // (as required for every field of non_initial_parent).
        hyphens: Hyphens::None,
        // CSS Text 4: an explicit string differs from the `auto` initial.
        hyphenate_character: HyphenateCharacter::String("—".into()),
        hyphenate_limit_chars: HyphenateLimitChars {
            total: HyphenateLimitCharsValue::Integer(9),
            before: HyphenateLimitCharsValue::Integer(4),
            after: HyphenateLimitCharsValue::Integer(3),
        },
        // CSS Flexible Box Layout Module Level 1 §5.1/§5.2: differs from the initial
        // `Row`/`NoWrap` (as required for every field of
        // non_initial_parent).
        flex_direction: FlexDirectionValue::Column,
        flex_wrap: FlexWrapValue::Wrap,
        // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2: differs from the initial
        // `0`/`1`.
        flex_grow: 2.0,
        flex_shrink: 3.0,
        // CSS Flexible Box Layout Module Level 1 §7.2.3: differs from the initial
        // `auto`.
        flex_basis: ComputedFlexBasis::Px(50.0),
        // CSS Flexible Box Layout Module Level 1 §4.2: differs from the initial `0`
        // value.
        order: 5,
        // CSS Box Alignment Module Level 3 §5.1/§5.1/§7.2: differs from the initial
        // `normal`.
        justify_content: ContentAlignmentValue::SpaceBetween,
        align_content: ContentAlignmentValue::Center,
        align_items: SelfAlignmentValue::FlexEnd,
        // CSS Box Alignment Module Level 3 §6.2: differs from the initial
        // `auto`.
        align_self: AlignSelfValue::Value(SelfAlignmentValue::Center),
        // CSS Box Alignment Module Level 3 §8.1: differs from the initial
        // `normal`.
        row_gap: ComputedLengthPercentageOrNormal::Px(6.0),
        column_gap: ComputedLengthPercentageOrNormal::Percent(10.0),
        // CSS Content 3 §2.4.1: `quotes` differs from the initial empty list
        // (as required for every field of non_initial_parent).
        quotes: Arc::new(vec![(SmolStr::new("«"), SmolStr::new("»"))]),
        quotes_auto: false,
        // CSS Text Decoration Module Level 3 §4: differs from the initial `none`
        // (an empty list), as required for every field of
        // non_initial_parent.
        text_shadow: Arc::new(vec![ComputedTextShadow {
            offset_x: ComputedLength(1.0),
            offset_y: ComputedLength(2.0),
            blur_radius: ComputedLength(3.0),
            color: TextShadowColor::Resolved(CssColor::BLACK),
        }]),
        // CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/§7.7/§8.3:
        // these values differ from the initial `none`/`auto`/`row`.
        grid_template_columns: ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
            line_names: vec![vec![], vec![]],
            components: vec![ComputedGridTrackListComponent::Size(
                ComputedGridTrackSize::Breadth(ComputedGridTrackBreadth::Px(100.0)),
            )],
        })),
        grid_template_rows: ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
            line_names: vec![vec![], vec![]],
            components: vec![ComputedGridTrackListComponent::Size(
                ComputedGridTrackSize::Breadth(ComputedGridTrackBreadth::Percent(50.0)),
            )],
        })),
        grid_template_areas: GridTemplateAreasValue::Areas(Arc::new(
            crate::property::GridTemplateAreas {
                row_strings: vec!["a".into()],
                areas: vec![crate::property::GridTemplateAreaEntry {
                    name: "a".into(),
                    row_start: 1,
                    row_end: 2,
                    column_start: 1,
                    column_end: 2,
                }],
                row_count: 1,
                column_count: 1,
            },
        )),
        grid_auto_columns: Arc::new(vec![ComputedGridTrackSize::Breadth(
            ComputedGridTrackBreadth::MinContent,
        )]),
        grid_auto_rows: Arc::new(vec![ComputedGridTrackSize::Breadth(
            ComputedGridTrackBreadth::MaxContent,
        )]),
        grid_auto_flow: GridAutoFlowValue::ColumnDense,
        grid_row_start: GridLineValue::Line(2),
        grid_row_end: GridLineValue::Span(3),
        grid_column_start: GridLineValue::Named("foo".into()),
        grid_column_end: GridLineValue::NamedLine("bar".into(), 2),
        // CSS Box Alignment Module Level 3 §7.1/§6.1: differs from the initial
        // `normal`/`auto`.
        justify_items: SelfAlignmentValue::Center,
        justify_self: AlignSelfValue::Value(SelfAlignmentValue::End),
        // CSS Fragmentation Module Level 3 §3.3: differs from the initial `2`
        // (as required for every field of non_initial_parent).
        orphans: 5,
        widows: 7,
        // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.6/§2.7/§2.8/§2.9:
        // these fields are all non-inherited, so give them non-initial values
        // (as required for non_initial_parent).
        background_repeat: BackgroundRepeat {
            x: BackgroundRepeatKeyword::Round,
            y: BackgroundRepeatKeyword::Space,
        },
        background_attachment: BackgroundAttachment::Fixed,
        background_clip: VisualBox::ContentBox,
        background_origin: VisualBox::ContentBox,
        background_size: ComputedBackgroundSize::Cover,
        background_position: ComputedCssPosition {
            horizontal: ComputedCssPositionOffset::End(ComputedLengthPercentage::Px(5.0)),
            vertical: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(25.0)),
        },
        // CSS Backgrounds and Borders 3 §2.3: non-inherited, so use a value
        // different from the initial `None` (as required for non_initial_parent).
        background_image: BackgroundImage::Url("fixture.png".into()),
        // CSS Images Module Level 3 §5.1/§5.2: all are non-inherited,
        // so give them values different from the initial `fill` / `50% 50%`
        // (as required for non_initial_parent).
        object_fit: ObjectFit::Cover,
        object_position: ComputedCssPosition {
            horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(3.0)),
            vertical: ComputedCssPositionOffset::End(ComputedLengthPercentage::Percent(10.0)),
        },
        // CSS Color 4 §3.3: non-inherited, so use a value different
        // from the initial `1` (as required for non_initial_parent).
        opacity: 0.75,
        // CSS Compositing and Blending Level 1 §3.4.2/§3.4.1: both are
        // non-inherited, so use values different from the initial `auto`/`normal`
        // (as required for non_initial_parent).
        isolation: Isolation::Isolate,
        mix_blend_mode: MixBlendMode::Multiply,
        // CSS Masking Level 1 §7.1/§5.1: both are non-inherited,
        // so use values different from the initial `none`
        // (as required for non_initial_parent).
        mask_image: MaskImage::Url("mask.svg".to_string()),
        clip_path: ClipPath::GeometryBox(GeometryBox::PaddingBox),
        // The transform-origin fields use their initial values in this fixture.
        transform_origin: ComputedValues::initial().transform_origin,
        transform_origin_z: crate::resolve::ComputedLength(0.0),
        // CSS Transforms Level 1 §4/CSS Filter Effects Level 1 §5:
        // transform and filter are non-inherited, so use values different
        // from their initial `none` (empty lists) in non_initial_parent.
        transform: Arc::new(vec![ComputedTransformFunction::TranslateX(
            ComputedLengthPercentage::Px(32.0),
        )]),
        filter: Arc::new(vec![FilterFunction::Blur(Length::Px(3.0))]),
        // CSS Tables 3 §4: table-layout is non-inherited, so use a value
        // different from the initial `auto` (as required for non_initial_parent).
        table_layout: TableLayoutValue::Fixed,
        text_overflow: crate::property::TextOverflowValue::Ellipsis,
        // CSS Tables 3 §6: border-collapse is inherited, so use a value
        // different from the initial `separate` (as above).
        border_collapse: BorderCollapseValue::Collapse,
        // CSS Tables 3 §6.1: border-spacing is inherited, so use a value
        // different from the initial `0px` on both axes (as above).
        border_spacing: ComputedBorderSpacing {
            horizontal: ComputedLength(10.0),
            vertical: ComputedLength(20.0),
        },
        // CSS Tables 3 §7: caption-side is inherited, so use a value
        // different from the initial `top` (as above).
        caption_side: CaptionSideValue::Bottom,
        // CSS Tables 3 §8: empty-cells is inherited, so use a value
        // different from the initial `show` (as above).
        empty_cells: EmptyCellsValue::Hide,
        // CSS Multi-column Layout 1: non-inherited fields use non-initial
        // values so `inherit_from` assertions exercise the reset.
        column_count: ColumnCountValue::Count(3),
        column_fill: ColumnFillValue::Auto,
        column_width: ComputedColumnWidth::Px(24.0),
        custom_properties: CustomPropertyEnvironment::from_map(HashMap::from([(
            SmolStr::new("--fixture"),
            SmolStr::new("1px"),
        )])),
        local_custom_properties: CustomPropertyEnvironment::from_map(HashMap::from([(
            SmolStr::new("--fixture"),
            SmolStr::new("1px"),
        )])),
    }
}

/// Inherited fields come from the parent; non-inherited fields use their
/// initial values.
#[test]
fn inherit_from_copies_inherited_and_resets_non_inherited() {
    let parent = non_initial_parent();
    let child = ComputedValues::inherit_from(&parent);
    let initial = ComputedValues::initial();

    // Inherited fields are copied from the parent (CSS Cascade 5 §7.2:
    // inheritance carries the computed value).
    assert_eq!(child.color, parent.color);
    assert_eq!(child.font_family, parent.font_family);
    assert_eq!(child.font_size, parent.font_size);
    assert_eq!(child.font_weight, parent.font_weight);
    assert_eq!(child.list_style_type, parent.list_style_type);
    assert_eq!(child.list_style_position, parent.list_style_position);
    assert_eq!(child.text_align, parent.text_align);
    // CSS Text 4: text-spacing-trim is inherited.
    assert_eq!(child.text_spacing_trim, parent.text_spacing_trim);
    // CSS Text 3 §8.2.1: hanging-punctuation is inherited.
    assert_eq!(child.hanging_punctuation, parent.hanging_punctuation);
    // CSS Text 4: text-autospace is inherited.
    assert_eq!(child.text_autospace, parent.text_autospace);
    // CSS Text 4: word-space-transform is inherited.
    assert_eq!(child.word_space_transform, parent.word_space_transform);
    // CSS Text Decoration 4: text-decoration-skip-ink is inherited.
    assert_eq!(
        child.text_decoration_skip_ink,
        parent.text_decoration_skip_ink
    );
    // CSS Text Decoration 4: text-decoration-skip-spaces is inherited.
    assert_eq!(
        child.text_decoration_skip_spaces,
        parent.text_decoration_skip_spaces
    );
    // CSS Writing Modes 4 §2.1: direction is inherited.
    assert_eq!(child.direction, parent.direction);
    // CSS Writing Modes 4 §3.2: the CSSOM computed keyword is inherited
    // verbatim; the renderer-facing field stays normalized independently.
    assert_eq!(child.writing_mode, WritingMode::HorizontalTb);
    assert_eq!(child.cssom_writing_mode, parent.cssom_writing_mode);
    // CSS Fonts 4 §2.4: font-style is inherited.
    assert_eq!(child.font_style, parent.font_style);
    assert_eq!(
        child.font_variation_settings,
        parent.font_variation_settings
    );
    assert_eq!(child.font_feature_settings, parent.font_feature_settings);
    // CSS Fonts 4 §9.3: font-variant-emoji is inherited.
    assert_eq!(child.font_variant_emoji, parent.font_variant_emoji);
    // CSS Fonts 4 §6.13: font-language-override is inherited.
    assert_eq!(child.font_language_override, parent.font_language_override);
    // CSS Fonts 4 §6.4: font-variant-ligatures is inherited.
    assert_eq!(child.font_variant_ligatures, parent.font_variant_ligatures);
    // CSS Fonts 4 §2.8.5: font-synthesis is inherited as specified keywords.
    assert_eq!(child.font_synthesis, parent.font_synthesis);
    // CSS Fonts 3 §6.5: font-variant-position is inherited as specified.
    assert_eq!(child.font_variant_position, parent.font_variant_position);
    // CSS Fonts 4 §9.1: font-palette is inherited as specified.
    assert_eq!(child.font_palette, parent.font_palette);
    // CSS Fonts Module Level 3 §6.6: font-variant-caps is inherited.
    assert_eq!(child.font_variant_caps, parent.font_variant_caps);
    // CSS Text Module Level 3 §2.1: text-transform is inherited.
    assert_eq!(child.text_transform, parent.text_transform);
    assert_eq!(child.text_combine_upright, parent.text_combine_upright);
    assert_eq!(child.text_orientation, parent.text_orientation);
    assert_eq!(child.unicode_bidi, UnicodeBidi::Normal);
    // CSS Display 3 §4: visibility is inherited.
    assert_eq!(child.visibility, parent.visibility);
    // CSS Text 3 §8.1: text-indent is inherited.
    assert_eq!(child.text_indent, parent.text_indent);
    // CSS Text 3 §5.1: word-break is inherited.
    assert_eq!(child.word_break, parent.word_break);
    // CSS Text 3 §5.4: overflow-wrap is inherited.
    assert_eq!(child.overflow_wrap, parent.overflow_wrap);
    // CSS Text 3 §7.2 / §7.1: letter-spacing and word-spacing are
    // both inherited.
    assert_eq!(child.letter_spacing, parent.letter_spacing);
    assert_eq!(
        child.letter_spacing_computed,
        parent.letter_spacing_computed
    );
    assert_eq!(child.word_spacing, parent.word_spacing);
    assert_eq!(child.word_spacing_computed, parent.word_spacing_computed);
    // CSS Text 3 §3: white-space is inherited.
    assert_eq!(child.white_space, parent.white_space);
    // CSS Text 4: white-space-collapse is inherited.
    assert_eq!(child.white_space_collapse, parent.white_space_collapse);
    assert_eq!(
        child.effective_white_space_collapse,
        parent.effective_white_space_collapse
    );
    assert_eq!(
        child.effective_text_wrap_mode,
        parent.effective_text_wrap_mode
    );
    // CSS Text 3 §5.3: hyphens is inherited.
    assert_eq!(child.hyphens, parent.hyphens);
    // CSS Text 4: hyphenate-character is inherited.
    assert_eq!(child.hyphenate_character, parent.hyphenate_character);
    // CSS Text 4: hyphenate-limit-chars is inherited as computed values.
    assert_eq!(child.hyphenate_limit_chars, parent.hyphenate_limit_chars);
    // CSS Text Module Level 3 §4.2: tab-size is inherited.
    assert_eq!(child.tab_size, parent.tab_size);
    // CSS Text Decoration Module Level 3 §4: text-shadow is inherited.
    assert_eq!(child.text_shadow, parent.text_shadow);
    // The computed `<length>` for `line-height` is **not resolved again**
    // in the child (CSS Inline 3: percentages are made absolute on the declaring element).
    assert_eq!(child.line_height, parent.line_height);
    // CSS Content 3 §2.4.1: quotes is inherited.
    assert_eq!(child.quotes, parent.quotes);
    assert_eq!(child.quotes_auto, parent.quotes_auto);
    // CSS Fragmentation Module Level 3 §3.3: orphans and widows are
    // both inherited.
    assert_eq!(child.orphans, parent.orphans);
    assert_eq!(child.widows, parent.widows);
    // CSS Tables 3 §6.1: border-spacing is inherited.
    assert_eq!(child.border_spacing, parent.border_spacing);
    // CSS Tables 3 §7: caption-side is inherited.
    assert_eq!(child.caption_side, parent.caption_side);
    // CSS Tables 3 §8: empty-cells is inherited.
    assert_eq!(child.empty_cells, parent.empty_cells);
    assert_eq!(child.custom_properties, parent.custom_properties);

    // Non-inherited fields reset to their initial values.
    assert_eq!(child.background_color, initial.background_color);
    assert_eq!(child.display, initial.display);
    assert!(child.counter_reset.is_empty());
    assert!(child.counter_increment.is_empty());
    assert!(child.counter_set.is_empty());
    assert!(child.content.is_empty());
    assert!(child.string_set.is_empty());
    assert!(child.running_templates.is_empty());
    assert_eq!(child.padding, initial.padding);
    assert_eq!(child.margin, initial.margin);
    // The specified initial border-width is `medium` (3px), but the computed
    // value is 0px after style gating (CSS Backgrounds 3 §3.3). This also shows
    // that delegation passes through `resolve_border`.
    assert_eq!(child.border, initial.border);
    assert_eq!(child.border.top.width, ComputedLength::ZERO);
    assert_eq!(child.border_radius, initial.border_radius);
    assert_eq!(child.box_shadow, initial.box_shadow);
    assert_eq!(child.outline, initial.outline);
    assert_eq!(child.width, initial.width);
    assert_eq!(child.height, initial.height);
    // CSS Multi-column Layout 1: all three longhands are non-inherited.
    assert_eq!(child.column_count, initial.column_count);
    assert_eq!(child.column_fill, initial.column_fill);
    assert_eq!(child.column_width, initial.column_width);
    assert_eq!(child.box_sizing, initial.box_sizing);
    // CSS Overflow 3 §3.1: overflow-x/overflow-y are
    // non-inherited.
    assert_eq!(child.overflow, initial.overflow);
    // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3:
    // text-decoration-line/-style/-color are non-inherited.
    assert_eq!(child.text_decoration_line, initial.text_decoration_line);
    assert_eq!(child.text_decoration_style, initial.text_decoration_style);
    assert_eq!(child.text_decoration_color, initial.text_decoration_color);
    assert_eq!(child.text_decoration_inset, initial.text_decoration_inset);
    // CSS 2.1 §10.8.1: vertical-align is non-inherited.
    assert_eq!(child.vertical_align, initial.vertical_align);
    // CSS2 §9.9.1: z-index is non-inherited.
    assert_eq!(child.z_index, initial.z_index);
    // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
    // break-after / break-inside are non-inherited.
    assert_eq!(child.break_before, initial.break_before);
    assert_eq!(child.break_after, initial.break_after);
    assert_eq!(child.break_inside, initial.break_inside);
    // CSS2 §9.5.1 / §9.5.2: float / clear are both non-inherited.
    assert_eq!(child.float, initial.float);
    assert_eq!(child.clear, initial.clear);
    // CSS Flexible Box Layout Module Level 1 §5.1/§5.2/§7.2.1/§7.2.2/
    // §7.2.3: flex-* are non-inherited.
    assert_eq!(child.flex_direction, initial.flex_direction);
    assert_eq!(child.flex_wrap, initial.flex_wrap);
    assert_eq!(child.flex_grow, initial.flex_grow);
    assert_eq!(child.flex_shrink, initial.flex_shrink);
    assert_eq!(child.flex_basis, initial.flex_basis);
    // CSS Box Alignment Module Level 3 §5.1 (justify-content /
    // align-content) / §7.2 (align-items) / §6.2 (align-self): all
    // non-inherited.
    assert_eq!(child.justify_content, initial.justify_content);
    assert_eq!(child.align_content, initial.align_content);
    assert_eq!(child.align_items, initial.align_items);
    assert_eq!(child.align_self, initial.align_self);
    // CSS Box Alignment Module Level 3 §8.1: row-gap / column-gap are
    // non-inherited.
    assert_eq!(child.row_gap, initial.row_gap);
    assert_eq!(child.column_gap, initial.column_gap);
    // CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/§7.7/§8.3: grid-*
    // are all non-inherited.
    assert_eq!(child.grid_template_columns, initial.grid_template_columns);
    assert_eq!(child.grid_template_rows, initial.grid_template_rows);
    assert_eq!(child.grid_template_areas, initial.grid_template_areas);
    assert_eq!(child.grid_auto_columns, initial.grid_auto_columns);
    assert_eq!(child.grid_auto_rows, initial.grid_auto_rows);
    assert_eq!(child.grid_auto_flow, initial.grid_auto_flow);
    assert_eq!(child.grid_row_start, initial.grid_row_start);
    assert_eq!(child.grid_row_end, initial.grid_row_end);
    assert_eq!(child.grid_column_start, initial.grid_column_start);
    assert_eq!(child.grid_column_end, initial.grid_column_end);
    // CSS Box Alignment Module Level 3 §7.1/§6.1: justify-items /
    // justify-self are both non-inherited.
    assert_eq!(child.justify_items, initial.justify_items);
    assert_eq!(child.justify_self, initial.justify_self);
    // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.6/§2.7/§2.8/§2.9: all
    // are non-inherited.
    assert_eq!(child.background_repeat, initial.background_repeat);
    assert_eq!(child.background_attachment, initial.background_attachment);
    assert_eq!(child.background_clip, initial.background_clip);
    assert_eq!(child.background_origin, initial.background_origin);
    assert_eq!(child.background_size, initial.background_size);
    assert_eq!(child.background_position, initial.background_position);
    assert_eq!(child.background_image, initial.background_image);
    // CSS Images Module Level 3 §5.1/§5.2: all are non-inherited.
    assert_eq!(child.object_fit, initial.object_fit);
    assert_eq!(child.object_position, initial.object_position);
    // CSS Color 4 §3.3: opacity is non-inherited.
    assert_eq!(child.opacity, initial.opacity);
    // CSS Compositing and Blending Level 1 §3.4.1/§3.4.2: both are
    // non-inherited.
    assert_eq!(child.isolation, initial.isolation);
    assert_eq!(child.mix_blend_mode, initial.mix_blend_mode);
    // CSS Masking Level 1 §7.1/§5.1: both are non-inherited.
    assert_eq!(child.mask_image, initial.mask_image);
    assert_eq!(child.clip_path, initial.clip_path);
    // CSS Transforms Level 1 §4/CSS Filter Effects Level 1 §5: both are
    // non-inherited.
    assert_eq!(child.transform, initial.transform);
    assert_eq!(child.filter, initial.filter);
}

/// The result of `inherit_from` does not depend on the contents of `ResolveContext`.
///
/// This checks the invariant documented on `ComputedValues::inherit_from`:
/// `SpecifiedValues::inherit_from` produces no font-relative values,
/// so `finalize` never enters the `rem` arm or reads the context.
/// If a future lift returns a value other than `Px` (breaking the fixed-point
/// property), this test will fail and expose the broken delegation assumption.
#[test]
fn inherit_from_is_independent_of_resolve_context() {
    use crate::resolve::ResolveContext;
    use crate::specified::SpecifiedValues;

    let parent = non_initial_parent();
    let expected = ComputedValues::inherit_from(&parent);
    for root_font_size in [
        ComputedLength(1.0),
        ComputedLength(16.0),
        ComputedLength(999.0),
    ] {
        let mut via_staging = SpecifiedValues::inherit_from(&parent)
            .finalize(&parent, &ResolveContext::new(root_font_size));
        // `ComputedValues::inherit_from` also carries the crate-private
        // custom-property inheritance bridge; mirror it here so this
        // test continues to compare the same staging path.
        via_staging.custom_properties = parent.custom_properties.clone();
        assert_eq!(
            via_staging, expected,
            "inherit_from must not depend on the rem basis ({root_font_size:?})"
        );
    }
}

/// If the parent is initial, the child is initial too (the inheritance chain terminates).
#[test]
fn inherit_from_initial_parent_yields_initial() {
    assert_eq!(
        ComputedValues::inherit_from(&ComputedValues::initial()),
        ComputedValues::initial(),
    );
}
