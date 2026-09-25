use super::*;
use crate::property::{
    GeometryBox, HyphenateLimitChars, HyphenateLimitCharsValue, Length, TextShadowColor,
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
    // CSS Backgrounds 3 §2.2: background-color initial は `transparent`
    // (= rgba(0, 0, 0, 0))
    assert_eq!(cv.background_color, CssColor::TRANSPARENT);
    // literal を保持する (`INITIAL_FONT_SIZE_PX` check
    // と同じ理由 — `initial_font_family()` 参照に書き換えると自己参照になり
    // 同 helper の誤編集を検出できなくなる)。`*cv.font_family` で
    // `Arc<Vec<Atom>>` を `Vec<Atom>` に deref してから比較する。
    assert_eq!(*cv.font_family, vec![Atom::from("serif")]);
    // CSS Fonts 4 §2.5: font-size initial は `medium` = 本実装では 16px
    // (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>)。
    // **この 16.0 は意図的な literal** — `INITIAL_FONT_SIZE_PX` 参照に
    // 書き換えると同 const の誤編集を検出できなくなる (同 const の doc も参照)。
    assert_eq!(cv.font_size, ComputedLength(16.0));
    assert_eq!(cv.font_weight, 400.0);
    // CSS Inline 3 §5.1: line-height initial は `normal`
    assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    assert_eq!(cv.display, DisplayValue::Inline);
    assert_eq!(cv.list_style_type, ListStyleType::Disc);
    assert_eq!(cv.list_style_position, ListStylePosition::Outside);
    // CSS Lists 3 §4: counter-* の spec initial は `none`、本 impl では
    // empty list 表現 (<https://www.w3.org/TR/css-lists-3/#auto-numbering>)。
    assert!(cv.counter_reset.is_empty());
    assert!(cv.counter_increment.is_empty());
    assert!(cv.counter_set.is_empty());
    // CSS Content 3 §1 (content: Initial: normal) + CSS GCPM 3 §1.1.1
    // (string-set: Initial: none) — どちらも空 list 表現
    assert!(cv.content.is_empty());
    assert!(cv.string_set.is_empty());
    // CSS GCPM 3 §1.2.1: position initial は `static` →
    // running() seed 無し。
    assert!(cv.running_templates.is_empty());
    // CSS Text 3 §6.1: text-align initial は `start`。
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
    // CSS Writing Modes 4 §2.1: direction initial は `ltr`。
    assert_eq!(cv.direction, Direction::Ltr);
    // CSS Fonts 4 §2.4: font-style initial は `normal`。
    assert_eq!(cv.font_style, FontStyle::Normal);
    // CSS Fonts Module Level 3 §6.6: font-variant-caps initial は
    // `normal`。
    assert_eq!(cv.font_variant_caps, FontVariantCaps::Normal);
    // CSS Text Module Level 3 §2.1: text-transform initial は `none`。
    assert_eq!(cv.text_transform, TextTransform::None);
    assert_eq!(cv.text_combine_upright, TextCombineUpright::None);
    assert_eq!(cv.text_orientation, TextOrientation::Mixed);
    assert_eq!(cv.unicode_bidi, UnicodeBidi::Normal);
    // CSS Display 3 §4: visibility initial は `visible`。
    assert_eq!(cv.visibility, Visibility::Visible);
    // CSS Text 3 §8.1: text-indent initial は `0`。
    assert_eq!(cv.text_indent, ComputedTextIndent::Px(0.0));
    // CSS Text 3 §5.1: word-break initial は `normal`。
    assert_eq!(cv.word_break, WordBreak::Normal);
    // CSS Text 3 §5.4: overflow-wrap initial は `normal`。
    assert_eq!(cv.overflow_wrap, OverflowWrap::Normal);
    // CSS Text 4: word-space-transform initial is `none`.
    assert_eq!(cv.word_space_transform, WordSpaceTransform::None);
    // `letter-spacing: normal` keeps a zero CSS computed value and the same
    // zero renderer fallback.
    assert_eq!(cv.letter_spacing, ComputedLength::ZERO);
    assert_eq!(cv.letter_spacing_computed, ComputedLetterSpacing::Px(0.0));
    assert_eq!(cv.word_spacing, ComputedLength::ZERO);
    assert_eq!(cv.word_spacing_computed, ComputedLetterSpacing::Px(0.0));
    // CSS Box 3 §4.1: padding initial = 0 (all 4 sides)。
    assert_eq!(cv.padding, Sides::all(ComputedLengthPercentage::Px(0.0)));
    // CSS Box 3 §3.1: margin initial は 0 on each side。
    assert_eq!(
        cv.margin,
        Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))
    );
    // CSS Backgrounds 3 §3 (currentcolor へ格上げ済み): border initial は
    // 各 side {style: none, color: `currentcolor` (BorderColor::CurrentColor)}。
    // hazard case 2 (author `color:red` + border-color 省略 → cascade static
    // side が initial 直行) の enum coverage — used-value resolution は
    // paint scope で `color` property に対して確定。
    //
    // width は **0px** — specified の initial は `medium` (3px) だが CSS
    // Backgrounds 3 §3.3 <https://www.w3.org/TR/css-backgrounds-3/#border-width>
    // の "Computed value: … zero if the border style is `none` or `hidden`"
    // により computed 層で潰れる。specified 側の
    // initial は `crate::specified` の
    // `initial_border_width_is_gated_to_zero_at_computed_layer` が check する。
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
    // CSS Sizing 3 §3.1.1: width initial は `auto`。
    assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Auto);
    // CSS Sizing 3 §3.1.1: height initial は `auto`。
    assert_eq!(cv.height, ComputedLengthPercentageOrAuto::Auto);
    // CSS Sizing 3 §3.3: box-sizing initial は `content-box`。
    assert_eq!(cv.box_sizing, BoxSizing::ContentBox);
    // CSS2 §9.9.1: z-index initial は `auto`。
    assert_eq!(cv.z_index, ZIndexValue::Auto);
    // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
    // break-after / break-inside initial は共に `auto`。
    assert_eq!(cv.break_before, BreakBetween::Auto);
    assert_eq!(cv.break_after, BreakBetween::Auto);
    assert_eq!(cv.break_inside, BreakInside::Auto);
    // CSS2 §9.5.1 / §9.5.2: float / clear の initial は共に `none`。
    assert_eq!(cv.float, FloatValue::None);
    assert_eq!(cv.clear, ClearValue::None);
    // CSS Text 3 §3: white-space initial は `normal`。
    assert_eq!(cv.white_space, WhiteSpace::Normal);
    // CSS Text 4: white-space-collapse initial is `collapse`.
    assert_eq!(cv.white_space_collapse, WhiteSpaceCollapse::Collapse);
    // CSS Text 4 §5.4: text-wrap-style initial is `auto`.
    assert_eq!(cv.text_wrap_style, TextWrapStyle::Auto);
    // CSS Text 3 §5.3: hyphens initial は `manual`。
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
    // CSS Display 3 §2: display initial は `inline`
    // (anchor は `ComputedValues::display` field doc 側)。
    assert_eq!(ComputedValues::initial().display, DisplayValue::Inline);
}

// ── inherit_from (delegation の pin) ──

/// 全 field が initial から離れた親 fixture。
///
/// inherited / non-inherited のどちらの分岐が壊れても検出できるよう、
/// **全 field を non-initial 値**にしてある (non-inherited が親から漏れれば
/// initial との比較で落ち、inherited が initial に落ちれば親との比較で落ちる)。
fn non_initial_parent() -> ComputedValues {
    ComputedValues {
        color: CssColor {
            r: 200,
            g: 100,
            b: 50,
            a: 255,
        },
        background_color: CssColor {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        },
        font_family: Arc::new(vec![Atom::from("sans-serif")]),
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
        // non-initial 値 (上記 fixture doc の全 field 非 initial 方針)。
        text_justify: TextJustify::InterWord,
        text_align_last: TextAlignLast::Justify,
        // CSS Writing Modes 4 §2.1: `Rtl` — initial (`Ltr`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        direction: Direction::Rtl,
        // CSSOM's `cssom_writing_mode: VerticalRl` is reachable. Setting the
        // renderer-facing `writing_mode` to this same value is synthetic:
        // real cascades keep that field at `HorizontalTb`. This lets the test
        // prove inheritance copies the CSSOM value and re-applies the renderer
        // fallback rather than copying `parent.writing_mode` verbatim.
        writing_mode: WritingMode::VerticalRl,
        cssom_writing_mode: WritingMode::VerticalRl,
        ruby_position: RubyPosition::Under,
        // CSS Text 3 §8.1: initial (`0`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        text_indent: ComputedTextIndent::Px(9.0),
        text_indent_ch_factor: None,
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
        // 3 field とも initial と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
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
        // `Sub` — initial (`Baseline`) と異なる値 (non_initial_parent の
        // 趣旨どおり全 field を非 initial に)。
        vertical_align: VerticalAlign::Sub,
        // CSS Fonts 4 §2.4: `Italic` — initial (`Normal`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        font_style: FontStyle::Italic,
        // CSS Fonts Module Level 3 §6.6: `SmallCaps` — initial
        // (`Normal`) と異なる値 (non_initial_parent の趣旨どおり全
        // field を非 initial に)。
        font_variant_caps: FontVariantCaps::SmallCaps,
        // CSS Text Module Level 3 §2.1: `Uppercase` — initial (`None`)
        // と異なる値 (non_initial_parent の趣旨どおり全 field を非
        // initial に)。
        text_transform: TextTransform::Uppercase,
        text_combine_upright: TextCombineUpright::All,
        text_orientation: TextOrientation::Sideways,
        unicode_bidi: UnicodeBidi::IsolateOverride,
        // CSS Display 3 §4: `Hidden` — initial (`Visible`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        visibility: Visibility::Hidden,
        // CSS2 §9.9.1: `Integer(3)` — initial (`Auto`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        z_index: ZIndexValue::Integer(3),
        // CSS Text 3 §5.1: `KeepAll` — initial (`Normal`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        word_break: WordBreak::KeepAll,
        line_break: LineBreak::Auto,
        // CSS Text 3 §5.4: `Anywhere` — initial (`Normal`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        overflow_wrap: OverflowWrap::Anywhere,
        // CSS Text 3 §7.2 / §7.1: initial (`0`、`normal` の computed
        // value) と異なる値 (non_initial_parent の趣旨どおり全 field を
        // 非 initial に)。
        letter_spacing: ComputedLength(2.0),
        letter_spacing_computed: ComputedLetterSpacing::Px(2.0),
        letter_spacing_ch_factor: None,
        word_spacing: ComputedLength(4.0),
        word_spacing_computed: ComputedLetterSpacing::Px(4.0),
        word_spacing_ch_factor: None,
        // CSS Text Module Level 3 §4.2: initial (`8`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        tab_size: ComputedTabSize::Number(3.0),
        // CSS Fragmentation Module Level 3 §3.1 / §3.2: initial
        // (`Auto`) と異なる値 (non_initial_parent の趣旨どおり全 field
        // を非 initial に)。
        break_before: BreakBetween::Page,
        break_after: BreakBetween::AvoidPage,
        break_inside: BreakInside::AvoidPage,
        // CSS2 §9.5.1 / §9.5.2: `Left`/`Both` — initial (`None`/`None`)
        // と異なる値 (non_initial_parent の趣旨どおり全 field を非
        // initial に)。
        float: FloatValue::Left,
        clear: ClearValue::Both,
        // CSS Text 3 §3: `Pre` — initial (`Normal`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        white_space: WhiteSpace::Pre,
        // CSS Text 4: `PreserveBreaks` differs from the `Collapse` initial.
        white_space_collapse: WhiteSpaceCollapse::PreserveBreaks,
        text_wrap: TextWrapMode::Nowrap,
        text_wrap_style: TextWrapStyle::Stable,
        // CSS Text 3 §5.3: `None` — initial (`Manual`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        hyphens: Hyphens::None,
        // CSS Text 4: an explicit string differs from the `auto` initial.
        hyphenate_character: HyphenateCharacter::String("—".into()),
        hyphenate_limit_chars: HyphenateLimitChars {
            total: HyphenateLimitCharsValue::Integer(9),
            before: HyphenateLimitCharsValue::Integer(4),
            after: HyphenateLimitCharsValue::Integer(3),
        },
        // CSS Flexible Box Layout Module Level 1 §5.1/§5.2: initial
        // (`Row`/`NoWrap`) と異なる値 (non_initial_parent の趣旨どおり
        // 全 field を非 initial に)。
        flex_direction: FlexDirectionValue::Column,
        flex_wrap: FlexWrapValue::Wrap,
        // CSS Flexible Box Layout Module Level 1 §7.2.1/§7.2.2: initial
        // (`0`/`1`) と異なる値。
        flex_grow: 2.0,
        flex_shrink: 3.0,
        // CSS Flexible Box Layout Module Level 1 §7.2.3: initial
        // (`auto`) と異なる値。
        flex_basis: ComputedFlexBasis::Px(50.0),
        // CSS Flexible Box Layout Module Level 1 §4.2: initial (`0`)
        // と異なる値。
        order: 5,
        // CSS Box Alignment Module Level 3 §5.1/§5.1/§7.2: initial
        // (`normal`) と異なる値。
        justify_content: ContentAlignmentValue::SpaceBetween,
        align_content: ContentAlignmentValue::Center,
        align_items: SelfAlignmentValue::FlexEnd,
        // CSS Box Alignment Module Level 3 §6.2: initial (`auto`) と
        // 異なる値。
        align_self: AlignSelfValue::Value(SelfAlignmentValue::Center),
        // CSS Box Alignment Module Level 3 §8.1: initial (`normal`) と
        // 異なる値。
        row_gap: ComputedLengthPercentageOrNormal::Px(6.0),
        column_gap: ComputedLengthPercentageOrNormal::Percent(10.0),
        // CSS Content 3 §2.4.1: `quotes` — initial (空 list) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        quotes: Arc::new(vec![(SmolStr::new("«"), SmolStr::new("»"))]),
        quotes_auto: false,
        // CSS Text Decoration Module Level 3 §4: initial (`none` =
        // 空 list) と異なる値 (non_initial_parent の趣旨どおり全 field を
        // 非 initial に)。
        text_shadow: Arc::new(vec![ComputedTextShadow {
            offset_x: ComputedLength(1.0),
            offset_y: ComputedLength(2.0),
            blur_radius: ComputedLength(3.0),
            color: TextShadowColor::Resolved(CssColor::BLACK),
        }]),
        // CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/§7.7/§8.3:
        // initial (`none`/`auto`/`row`) と異なる値。
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
        // CSS Box Alignment Module Level 3 §7.1/§6.1: initial
        // (`normal`/`auto`) と異なる値。
        justify_items: SelfAlignmentValue::Center,
        justify_self: AlignSelfValue::Value(SelfAlignmentValue::End),
        // CSS Fragmentation Module Level 3 §3.3: initial (`2`) と異なる値
        // (non_initial_parent の趣旨どおり全 field を非 initial に)。
        orphans: 5,
        widows: 7,
        // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.6/§2.7/§2.8/§2.9: 全て
        // non-inherited なので、initial と異なる値にしておく
        // (non_initial_parent の趣旨どおり)。
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
        // CSS Backgrounds and Borders 3 §2.3: non-inherited, initial
        // (`None`) と異なる値にしておく (non_initial_parent の趣旨どおり)。
        background_image: BackgroundImage::Url("fixture.png".into()),
        // CSS Images Module Level 3 §5.1/§5.2: 全て non-inherited なので、
        // initial (`fill` / `50% 50%`) と異なる値にしておく
        // (non_initial_parent の趣旨どおり)。
        object_fit: ObjectFit::Cover,
        object_position: ComputedCssPosition {
            horizontal: ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(3.0)),
            vertical: ComputedCssPositionOffset::End(ComputedLengthPercentage::Percent(10.0)),
        },
        // CSS Color 4 §3.3: non-inherited なので initial (`1`) と異なる
        // 値にしておく (non_initial_parent の趣旨どおり)。
        opacity: 0.75,
        // CSS Compositing and Blending Level 1 §3.4.2/§3.4.1: 両方
        // non-inherited なので initial (`auto`/`normal`) と異なる値に
        // しておく (non_initial_parent の趣旨どおり)。
        isolation: Isolation::Isolate,
        mix_blend_mode: MixBlendMode::Multiply,
        // CSS Masking Level 1 §7.1/§5.1: 両方 non-inherited なので
        // initial (`none`) と異なる値にしておく (non_initial_parent の
        // 趣旨どおり)。
        mask_image: MaskImage::Url("mask.svg".to_string()),
        clip_path: ClipPath::GeometryBox(GeometryBox::PaddingBox),
        // CSS Transforms Level 1 §4/CSS Filter Effects Level 1 §5:
        // 両方 non-inherited なので initial (`none` = 空 list) と
        // 異なる値にしておく (non_initial_parent の趣旨どおり)。
        transform: Arc::new(vec![ComputedTransformFunction::TranslateX(
            ComputedLengthPercentage::Px(32.0),
        )]),
        filter: Arc::new(vec![FilterFunction::Blur(Length::Px(3.0))]),
        // CSS Tables 3 §4: table-layout は non-inherited なので initial
        // (`auto`) と異なる値にしておく (non_initial_parent の趣旨どおり)。
        table_layout: TableLayoutValue::Fixed,
        // CSS Tables 3 §6: border-collapse は inherited なので initial
        // (`separate`) と異なる値にしておく (同上)。
        border_collapse: BorderCollapseValue::Collapse,
        // CSS Tables 3 §6.1: border-spacing は inherited なので initial
        // (両軸 `0px`) と異なる値にしておく (同上)。
        border_spacing: ComputedBorderSpacing {
            horizontal: ComputedLength(10.0),
            vertical: ComputedLength(20.0),
        },
        // CSS Tables 3 §7: caption-side は inherited なので initial
        // (`top`) と異なる値にしておく (同上)。
        caption_side: CaptionSideValue::Bottom,
        // CSS Tables 3 §8: empty-cells は inherited なので initial
        // (`show`) と異なる値にしておく (同上)。
        empty_cells: EmptyCellsValue::Hide,
        // CSS Multi-column Layout 1: non-inherited fields use non-initial
        // values so `inherit_from` assertions exercise the reset.
        column_count: ColumnCountValue::Count(3),
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

    // inherited — 親からコピー (CSS Cascade 5 §7.2: inheritance が運ぶのは
    // computed value)。
    assert_eq!(child.color, parent.color);
    assert_eq!(child.font_family, parent.font_family);
    assert_eq!(child.font_size, parent.font_size);
    assert_eq!(child.font_weight, parent.font_weight);
    assert_eq!(child.list_style_type, parent.list_style_type);
    assert_eq!(child.list_style_position, parent.list_style_position);
    assert_eq!(child.text_align, parent.text_align);
    // CSS Text 4: text-spacing-trim is inherited.
    assert_eq!(child.text_spacing_trim, parent.text_spacing_trim);
    // CSS Text 3 §8.2.1: hanging-punctuation は inherited。
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
    // CSS Writing Modes 4 §2.1: direction は inherited。
    assert_eq!(child.direction, parent.direction);
    // CSS Writing Modes 4 §3.2: the CSSOM computed keyword is inherited
    // verbatim; the renderer-facing field stays normalized independently.
    assert_eq!(child.writing_mode, WritingMode::HorizontalTb);
    assert_eq!(child.cssom_writing_mode, parent.cssom_writing_mode);
    // CSS Fonts 4 §2.4: font-style は inherited。
    assert_eq!(child.font_style, parent.font_style);
    // CSS Fonts Module Level 3 §6.6: font-variant-caps は inherited。
    assert_eq!(child.font_variant_caps, parent.font_variant_caps);
    // CSS Text Module Level 3 §2.1: text-transform は inherited。
    assert_eq!(child.text_transform, parent.text_transform);
    assert_eq!(child.text_combine_upright, parent.text_combine_upright);
    assert_eq!(child.text_orientation, parent.text_orientation);
    assert_eq!(child.unicode_bidi, UnicodeBidi::Normal);
    // CSS Display 3 §4: visibility は inherited。
    assert_eq!(child.visibility, parent.visibility);
    // CSS Text 3 §8.1: text-indent は inherited。
    assert_eq!(child.text_indent, parent.text_indent);
    // CSS Text 3 §5.1: word-break は inherited。
    assert_eq!(child.word_break, parent.word_break);
    // CSS Text 3 §5.4: overflow-wrap は inherited。
    assert_eq!(child.overflow_wrap, parent.overflow_wrap);
    // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing は共に
    // inherited。
    assert_eq!(child.letter_spacing, parent.letter_spacing);
    assert_eq!(
        child.letter_spacing_computed,
        parent.letter_spacing_computed
    );
    assert_eq!(child.word_spacing, parent.word_spacing);
    assert_eq!(child.word_spacing_computed, parent.word_spacing_computed);
    // CSS Text 3 §3: white-space は inherited。
    assert_eq!(child.white_space, parent.white_space);
    // CSS Text 4: white-space-collapse is inherited.
    assert_eq!(child.white_space_collapse, parent.white_space_collapse);
    // CSS Text 3 §5.3: hyphens は inherited。
    assert_eq!(child.hyphens, parent.hyphens);
    // CSS Text 4: hyphenate-character is inherited.
    assert_eq!(child.hyphenate_character, parent.hyphenate_character);
    // CSS Text 4: hyphenate-limit-chars is inherited as computed values.
    assert_eq!(child.hyphenate_limit_chars, parent.hyphenate_limit_chars);
    // CSS Text Module Level 3 §4.2: tab-size は inherited。
    assert_eq!(child.tab_size, parent.tab_size);
    // CSS Text Decoration Module Level 3 §4: text-shadow は inherited。
    assert_eq!(child.text_shadow, parent.text_shadow);
    // `line-height` の computed `<length>` は子で **再解決されない**
    // (CSS Inline 3: percentage は宣言要素で絶対化済)。
    assert_eq!(child.line_height, parent.line_height);
    // CSS Content 3 §2.4.1: quotes は inherited。
    assert_eq!(child.quotes, parent.quotes);
    assert_eq!(child.quotes_auto, parent.quotes_auto);
    // CSS Fragmentation Module Level 3 §3.3: orphans / widows は共に
    // inherited。
    assert_eq!(child.orphans, parent.orphans);
    assert_eq!(child.widows, parent.widows);
    // CSS Tables 3 §6.1: border-spacing は inherited。
    assert_eq!(child.border_spacing, parent.border_spacing);
    // CSS Tables 3 §7: caption-side は inherited。
    assert_eq!(child.caption_side, parent.caption_side);
    // CSS Tables 3 §8: empty-cells は inherited。
    assert_eq!(child.empty_cells, parent.empty_cells);
    assert_eq!(child.custom_properties, parent.custom_properties);

    // non-inherited — initial に戻る。
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
    // specified の initial border-width は `medium` (3px) だが computed 層では
    // style gating で 0px (CSS Backgrounds 3 §3.3)。delegation が
    // `resolve_border` を通っている証拠でもある。
    assert_eq!(child.border, initial.border);
    assert_eq!(child.border.top.width, ComputedLength::ZERO);
    assert_eq!(child.border_radius, initial.border_radius);
    assert_eq!(child.box_shadow, initial.box_shadow);
    assert_eq!(child.outline, initial.outline);
    assert_eq!(child.width, initial.width);
    assert_eq!(child.height, initial.height);
    // CSS Multi-column Layout 1: both longhands are non-inherited.
    assert_eq!(child.column_count, initial.column_count);
    assert_eq!(child.column_width, initial.column_width);
    assert_eq!(child.box_sizing, initial.box_sizing);
    // CSS Overflow 3 §3.1: overflow-x/overflow-y は
    // non-inherited。
    assert_eq!(child.overflow, initial.overflow);
    // CSS Text Decoration Module Level 3 §2.1/§2.2/§2.3:
    // text-decoration-line/-style/-color は non-inherited。
    assert_eq!(child.text_decoration_line, initial.text_decoration_line);
    assert_eq!(child.text_decoration_style, initial.text_decoration_style);
    assert_eq!(child.text_decoration_color, initial.text_decoration_color);
    assert_eq!(child.text_decoration_inset, initial.text_decoration_inset);
    // CSS 2.1 §10.8.1: vertical-align は non-inherited。
    assert_eq!(child.vertical_align, initial.vertical_align);
    // CSS2 §9.9.1: z-index は non-inherited。
    assert_eq!(child.z_index, initial.z_index);
    // CSS Fragmentation Module Level 3 §3.1 / §3.2: break-before /
    // break-after / break-inside は non-inherited。
    assert_eq!(child.break_before, initial.break_before);
    assert_eq!(child.break_after, initial.break_after);
    assert_eq!(child.break_inside, initial.break_inside);
    // CSS2 §9.5.1 / §9.5.2: float / clear は共に non-inherited。
    assert_eq!(child.float, initial.float);
    assert_eq!(child.clear, initial.clear);
    // CSS Flexible Box Layout Module Level 1 §5.1/§5.2/§7.2.1/§7.2.2/
    // §7.2.3: flex-* は non-inherited。
    assert_eq!(child.flex_direction, initial.flex_direction);
    assert_eq!(child.flex_wrap, initial.flex_wrap);
    assert_eq!(child.flex_grow, initial.flex_grow);
    assert_eq!(child.flex_shrink, initial.flex_shrink);
    assert_eq!(child.flex_basis, initial.flex_basis);
    // CSS Box Alignment Module Level 3 §5.1 (justify-content /
    // align-content) / §7.2 (align-items) / §6.2 (align-self): all
    // non-inherited。
    assert_eq!(child.justify_content, initial.justify_content);
    assert_eq!(child.align_content, initial.align_content);
    assert_eq!(child.align_items, initial.align_items);
    assert_eq!(child.align_self, initial.align_self);
    // CSS Box Alignment Module Level 3 §8.1: row-gap / column-gap は
    // non-inherited。
    assert_eq!(child.row_gap, initial.row_gap);
    assert_eq!(child.column_gap, initial.column_gap);
    // CSS Grid Layout Module Level 1 §7.2/§7.3/§7.6/§7.7/§8.3: grid-*
    // は全て non-inherited。
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
    // justify-self は共に non-inherited。
    assert_eq!(child.justify_items, initial.justify_items);
    assert_eq!(child.justify_self, initial.justify_self);
    // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.6/§2.7/§2.8/§2.9: 全て
    // non-inherited。
    assert_eq!(child.background_repeat, initial.background_repeat);
    assert_eq!(child.background_attachment, initial.background_attachment);
    assert_eq!(child.background_clip, initial.background_clip);
    assert_eq!(child.background_origin, initial.background_origin);
    assert_eq!(child.background_size, initial.background_size);
    assert_eq!(child.background_position, initial.background_position);
    assert_eq!(child.background_image, initial.background_image);
    // CSS Images Module Level 3 §5.1/§5.2: 全て non-inherited。
    assert_eq!(child.object_fit, initial.object_fit);
    assert_eq!(child.object_position, initial.object_position);
    // CSS Color 4 §3.3: opacity は non-inherited。
    assert_eq!(child.opacity, initial.opacity);
    // CSS Compositing and Blending Level 1 §3.4.1/§3.4.2: 両方
    // non-inherited。
    assert_eq!(child.isolation, initial.isolation);
    assert_eq!(child.mix_blend_mode, initial.mix_blend_mode);
    // CSS Masking Level 1 §7.1/§5.1: 両方 non-inherited。
    assert_eq!(child.mask_image, initial.mask_image);
    assert_eq!(child.clip_path, initial.clip_path);
    // CSS Transforms Level 1 §4/CSS Filter Effects Level 1 §5: 両方
    // non-inherited。
    assert_eq!(child.transform, initial.transform);
    assert_eq!(child.filter, initial.filter);
}

/// `inherit_from` の結果は `ResolveContext` の中身に依存しない。
///
/// `ComputedValues::inherit_from` の doc が主張する invariant の check —
/// `SpecifiedValues::inherit_from` の出力に font-relative な値が 1 つも
/// 含まれないので、`finalize` は `rem` arm を踏まず context を参照しない。
/// 将来 lift 側が `Px` 以外を返すようになったら (= 不動点性が壊れたら)
/// ここが落ちて delegation の前提が崩れたことを知らせる。
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

/// 親が initial なら child も initial (inheritance chain の terminate)。
#[test]
fn inherit_from_initial_parent_yields_initial() {
    assert_eq!(
        ComputedValues::inherit_from(&ComputedValues::initial()),
        ComputedValues::initial(),
    );
}
