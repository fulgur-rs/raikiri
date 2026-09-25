use super::*;
use crate::computed::INITIAL_FONT_SIZE_PX;
use crate::property::HyphenateLimitCharsValue;
use crate::property::TextShadowColor;
use crate::property::{
    FontSynthesisStyle, FontVariantEastAsianVariant, FontVariantEastAsianWidth, GeometryBox,
};
use crate::resolve::{
    ComputedBorder, ComputedBorderRadius, ComputedBoxShadowItem, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackSize, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal, ComputedLetterSpacing,
    ComputedLineHeight, ComputedOutline, ComputedTabSize, ComputedTextDecorationInset,
    ComputedTextIndent, ComputedTextShadow, ComputedTextUnderlineOffset,
};

/// `root_font_size` = 16px の共通 context。
const CTX: ResolveContext = ResolveContext {
    root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
    root_line_height: None,
};

/// `finalize` の `parent: &ComputedValues` として渡す、font-size だけ
/// 差し替えた fixture (旧 `PARENT_FS: ComputedLength` の後継)。`text_align` / `direction` は本 module の phase 2/3 length 系
/// test では無関係なので initial (`Start` / `Ltr`) のまま。
fn parent_with_font_size(px: f32) -> ComputedValues {
    ComputedValues {
        font_size: ComputedLength(px),
        ..ComputedValues::initial()
    }
}

/// The specified initial values finalize to the computed initial values.
#[test]
fn initial_specified_finalizes_to_initial_computed() {
    assert_eq!(
        SpecifiedValues::initial().finalize(&ComputedValues::initial(), &ResolveContext::initial()),
        ComputedValues::initial(),
    );
}

#[test]
fn box_ch_provenance_keeps_authored_factor_and_font() {
    let mut specified = SpecifiedValues::initial();
    specified.width = LengthOrAuto::Length(Length::Ch(2.0));
    specified.height = LengthOrAuto::Length(Length::Ch(3.0));
    specified.padding = Sides::all(Length::Ch(1.0));
    specified.margin = Sides::all(LengthOrAuto::Length(Length::Ch(4.0)));
    let computed = specified.clone().finalize(&ComputedValues::initial(), &CTX);

    assert_eq!(
        computed.width_ch.as_ref().map(|value| value.factor),
        Some(2.0)
    );
    assert_eq!(
        computed.height_ch.as_ref().map(|value| value.factor),
        Some(3.0)
    );
    assert_eq!(
        computed.padding_ch.top.as_ref().map(|value| value.factor),
        Some(1.0)
    );
    assert_eq!(
        computed.margin_ch.left.as_ref().map(|value| value.factor),
        Some(4.0)
    );
    let width_font = &computed.width_ch.as_ref().expect("width marker").font;
    assert_eq!(width_font.family, computed.font_family);
    assert_eq!(width_font.size, computed.font_size);
    assert_eq!(computed.width, ComputedLengthPercentageOrAuto::Px(16.0));

    specified.width = LengthOrAuto::Length(Length::Percent(25.0));
    specified.height = LengthOrAuto::Auto;
    specified.padding = Sides::all(Length::Px(2.0));
    specified.margin = Sides::all(LengthOrAuto::Auto);
    let ordinary = specified.finalize(&ComputedValues::initial(), &CTX);
    assert!(ordinary.width_ch.is_none());
    assert!(ordinary.height_ch.is_none());
    assert!(ordinary.padding_ch.top.is_none());
    assert!(ordinary.margin_ch.top.is_none());
}

/// specified の border initial は `medium` (3px) / `none` / `currentcolor` で、
/// computed 層では style gating により width が 0px に潰れる
/// (CSS Backgrounds 3 §3.3 "Computed value: … zero if the border style is
/// `none` or `hidden`")。
#[test]
fn initial_border_width_is_gated_to_zero_at_computed_layer() {
    assert_eq!(SpecifiedValues::initial().border.top.width, Length::Px(3.0));
    assert_eq!(
        ComputedValues::initial().border.top.width,
        ComputedLength::ZERO
    );
}

/// CSS Color 4 §3.3: 範囲外の specified `opacity` は phase 3
/// ([`SpecifiedValues::finalize`]) で `[0, 1]` に clamp される —
/// `border` の style gating (直上の test) と同型の「specified 層では
/// 保持、computed 層で変換」pattern。end-to-end (実 cascade 経由) の
/// 同じ主張は `mod@crate::cascade` の `opacity_*` test が check する。
#[test]
fn opacity_out_of_range_specified_clamps_at_finalize() {
    let over = SpecifiedValues {
        opacity: 2.0,
        ..SpecifiedValues::initial()
    };
    assert_eq!(
        over.finalize(&ComputedValues::initial(), &ResolveContext::initial())
            .opacity,
        1.0
    );
    let under = SpecifiedValues {
        opacity: -0.5,
        ..SpecifiedValues::initial()
    };
    assert_eq!(
        under
            .finalize(&ComputedValues::initial(), &ResolveContext::initial())
            .opacity,
        0.0
    );
}

/// CSS Compositing and Blending Level 1 §3.4.1/§3.4.2: どちらも常に
/// keyword で、`finalize` は素通しするだけ (`opacity` のような
/// range-clamp transform は無い)。
#[test]
fn isolation_and_mix_blend_mode_pass_through_finalize_unchanged() {
    let specified = SpecifiedValues {
        isolation: Isolation::Isolate,
        mix_blend_mode: MixBlendMode::Multiply,
        ..SpecifiedValues::initial()
    };
    let computed = specified.finalize(&ComputedValues::initial(), &ResolveContext::initial());
    assert_eq!(computed.isolation, Isolation::Isolate);
    assert_eq!(computed.mix_blend_mode, MixBlendMode::Multiply);
}

/// CSS Masking Level 1 §7.1/§5.1: both always specified-layer data
/// (`MaskImage`/`ClipPath` doc's scope notes) — `finalize` moves the
/// value through unchanged, same shape as
/// `isolation_and_mix_blend_mode_pass_through_finalize_unchanged`
/// above.
#[test]
fn mask_image_and_clip_path_pass_through_finalize_unchanged() {
    let specified = SpecifiedValues {
        mask_image: MaskImage::Url("mask.svg".to_string()),
        clip_path: ClipPath::GeometryBox(GeometryBox::PaddingBox),
        ..SpecifiedValues::initial()
    };
    let computed = specified.finalize(&ComputedValues::initial(), &ResolveContext::initial());
    assert_eq!(computed.mask_image, MaskImage::Url("mask.svg".to_string()));
    assert_eq!(
        computed.clip_path,
        ClipPath::GeometryBox(GeometryBox::PaddingBox)
    );
}

/// CSS Transforms Level 1 §4 / CSS Filter Effects Level 1 §5: both
/// always specified-layer data (`TransformFunction`/`FilterFunction`
/// doc's scope notes) — `finalize` moves the value through unchanged,
/// same shape as
/// `mask_image_and_clip_path_pass_through_finalize_unchanged` above.
#[test]
fn transform_and_filter_pass_through_finalize_unchanged() {
    let transform = Arc::new(vec![TransformFunction::TranslateX(Length::Em(2.0))]);
    let filter = Arc::new(vec![FilterFunction::Blur(Length::Px(3.0))]);
    let specified = SpecifiedValues {
        transform: transform.clone(),
        filter: filter.clone(),
        ..SpecifiedValues::initial()
    };
    let computed = specified.finalize(&ComputedValues::initial(), &ResolveContext::initial());
    // `transform` の length 側だけが absolutize される — 2em は own font-size
    // 16px 基準で 32px へ。`filter` は "as specified" なので素通し。
    assert_eq!(
        computed.transform,
        Arc::new(vec![crate::resolve::ComputedTransformFunction::TranslateX(
            crate::resolve::ComputedLengthPercentage::Px(32.0)
        )])
    );
    assert_eq!(computed.filter, filter);
}

// -----------------------------------------------------------------
// inherit_from — inherited / non-inherited の分類
// -----------------------------------------------------------------

fn parent_fixture() -> ComputedValues {
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
        line_height: ComputedLineHeight::Number(1.5),
        display: DisplayValue::Block,
        list_style_type: ListStyleType::Named(SmolStr::new("upper-roman")),
        list_style_position: ListStylePosition::Inside,
        list_style_image: BackgroundImage::Url("marker.png".into()),
        counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
        counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
        counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
        content: empty_content_list(),
        string_set: empty_string_set_entries(),
        running_templates: vec![RunningTemplate {
            name: SmolStr::new("hdr"),
        }],
        text_align: TextAlign::Center,
        hanging_punctuation: HangingPunctuation::First,
        text_autospace: TextAutospace::NoAutospace,
        word_space_transform: WordSpaceTransform::SpaceAutoPhrase,
        text_spacing_trim: TextSpacingTrim::TrimBoth,
        text_justify: TextJustify::InterWord,
        text_align_last: TextAlignLast::Justify,
        direction: Direction::Rtl,
        // `VerticalRl` — non-initial. This helper feeds
        // `SpecifiedValues::inherit_from`, which copies the preserved CSSOM
        // keyword from `cssom_writing_mode`; renderer normalization happens
        // later in `SpecifiedValues::absolutize_with`.
        writing_mode: WritingMode::VerticalRl,
        cssom_writing_mode: WritingMode::VerticalRl,
        ruby_position: RubyPosition::Under,
        text_indent: ComputedTextIndent::Px(9.0),
        text_indent_ch_factor: None,
        text_indent_ch_font: None,
        text_indent_ch_inherited: false,
        text_indent_hanging: true,
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
        max_width: ComputedLengthPercentageOrAuto::Auto,
        max_height: ComputedLengthPercentageOrAuto::Auto,
        min_width: ComputedLengthPercentageOrAuto::Auto,
        min_height: ComputedLengthPercentageOrAuto::Auto,
        min_block_size: None,
        top: ComputedLengthPercentageOrAuto::Px(10.0),
        right: ComputedLengthPercentageOrAuto::Px(20.0),
        bottom: ComputedLengthPercentageOrAuto::Px(30.0),
        left: ComputedLengthPercentageOrAuto::Px(40.0),
        position: PositionValue::Relative,
        box_sizing: BoxSizing::BorderBox,
        overflow: OverflowXY {
            x: OverflowValue::Hidden,
            y: OverflowValue::Scroll,
        },
        text_decoration_line: TextDecorationLine::UNDERLINE,
        text_decoration_style: TextDecorationStyle::Wavy,
        text_decoration_color: TextDecorationColor::Resolved(CssColor::BLACK),
        text_decoration_thickness: crate::resolve::ComputedTextDecorationThickness::Length(
            ComputedLength(6.0),
        ),
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
            from_font: true,
            under: false,
            left: false,
            right: true,
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
        vertical_align: VerticalAlign::Sub,
        font_style: FontStyle::Italic,
        font_kerning: FontKerning::Normal,
        font_optical_sizing: FontOpticalSizing::None,
        font_variant_emoji: FontVariantEmoji::Text,
        font_language_override: FontLanguageOverride::String(SmolStr::new("ENG")),
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
        font_variant_caps: FontVariantCaps::SmallCaps,
        text_transform: TextTransform::Uppercase,
        text_combine_upright: TextCombineUpright::All,
        text_orientation: TextOrientation::Sideways,
        unicode_bidi: UnicodeBidi::IsolateOverride,
        visibility: Visibility::Hidden,
        z_index: ZIndexValue::Integer(3),
        word_break: WordBreak::KeepAll,
        line_break: LineBreak::Auto,
        overflow_wrap: OverflowWrap::Anywhere,
        letter_spacing: ComputedLength(2.0),
        letter_spacing_computed: ComputedLetterSpacing::Px(2.0),
        letter_spacing_ch_factor: None,
        word_spacing: ComputedLength(4.0),
        word_spacing_computed: ComputedLetterSpacing::Px(4.0),
        word_spacing_ch_factor: None,
        tab_size: ComputedTabSize::Length(ComputedLength(11.0)),
        break_before: BreakBetween::Page,
        break_after: BreakBetween::AvoidPage,
        break_inside: BreakInside::AvoidPage,
        float: FloatValue::Left,
        clear: ClearValue::Both,
        white_space: WhiteSpace::Pre,
        white_space_collapse: WhiteSpaceCollapse::PreserveBreaks,
        text_wrap: TextWrapMode::Nowrap,
        text_wrap_style: TextWrapStyle::Stable,
        hyphens: Hyphens::None,
        hyphenate_character: HyphenateCharacter::String("—".into()),
        hyphenate_limit_chars: HyphenateLimitChars {
            total: HyphenateLimitCharsValue::Integer(7),
            before: HyphenateLimitCharsValue::Integer(2),
            after: HyphenateLimitCharsValue::Integer(1),
        },
        flex_direction: FlexDirectionValue::Column,
        flex_wrap: FlexWrapValue::Wrap,
        flex_grow: 2.0,
        flex_shrink: 3.0,
        flex_basis: ComputedFlexBasis::Px(50.0),
        order: 5,
        justify_content: ContentAlignmentValue::SpaceBetween,
        align_content: ContentAlignmentValue::Center,
        align_items: SelfAlignmentValue::FlexEnd,
        align_self: AlignSelfValue::Value(SelfAlignmentValue::Center),
        row_gap: ComputedLengthPercentageOrNormal::Px(6.0),
        column_gap: ComputedLengthPercentageOrNormal::Percent(10.0),
        quotes: Arc::new(vec![(SmolStr::new("«"), SmolStr::new("»"))]),
        quotes_auto: false,
        text_shadow: Arc::new(vec![ComputedTextShadow {
            offset_x: ComputedLength(1.0),
            offset_y: ComputedLength(2.0),
            blur_radius: ComputedLength(3.0),
            color: TextShadowColor::Resolved(CssColor::BLACK),
        }]),
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
        justify_items: SelfAlignmentValue::Center,
        justify_self: AlignSelfValue::Value(SelfAlignmentValue::End),
        orphans: 5,
        widows: 7,
        // CSS Backgrounds and Borders 3 §2.4/§2.5/§2.6/§2.7/§2.8/§2.9: 全て
        // non-inherited なので、initial と異なる値にしておく。
        background_repeat: BackgroundRepeat {
            x: BackgroundRepeatKeyword::Round,
            y: BackgroundRepeatKeyword::Space,
        },
        background_attachment: BackgroundAttachment::Fixed,
        background_clip: VisualBox::ContentBox,
        background_origin: VisualBox::ContentBox,
        background_size: crate::resolve::ComputedBackgroundSize::Cover,
        background_position: crate::resolve::ComputedCssPosition {
            horizontal: crate::resolve::ComputedCssPositionOffset::End(
                ComputedLengthPercentage::Px(5.0),
            ),
            vertical: crate::resolve::ComputedCssPositionOffset::Start(
                ComputedLengthPercentage::Percent(25.0),
            ),
        },
        // CSS Backgrounds and Borders 3 §2.3: non-inherited, initial と
        // 異なる値にしておく (fixture の趣旨どおり)。
        background_image: BackgroundImage::Url("fixture.png".to_string()),
        // CSS Images Module Level 3 §5.1/§5.2: 全て non-inherited なので、
        // initial (`fill` / `50% 50%`) と異なる値にしておく。
        object_fit: ObjectFit::Cover,
        object_position: crate::resolve::ComputedCssPosition {
            horizontal: crate::resolve::ComputedCssPositionOffset::Start(
                ComputedLengthPercentage::Px(3.0),
            ),
            vertical: crate::resolve::ComputedCssPositionOffset::End(
                ComputedLengthPercentage::Percent(10.0),
            ),
        },
        // CSS Color 4 §3.3: non-inherited なので initial (`1`) と
        // 異なる値にしておく。
        opacity: 0.25,
        // CSS Compositing and Blending Level 1 §3.4.2: non-inherited
        // なので initial (`auto`) と異なる値にしておく。
        isolation: Isolation::Isolate,
        // CSS Compositing and Blending Level 1 §3.4.1: non-inherited
        // なので initial (`normal`) と異なる値にしておく。
        mix_blend_mode: MixBlendMode::Multiply,
        // CSS Masking Level 1 §7.1/§5.1: 両方 non-inherited なので
        // initial (`none`) と異なる値にしておく。
        mask_image: MaskImage::Url("mask.svg".to_string()),
        clip_path: ClipPath::GeometryBox(GeometryBox::PaddingBox),
        // CSS Transforms Level 1 §4/CSS Filter Effects Level 1 §5:
        // 両方 non-inherited なので initial (`none` = 空 list) と
        // 異なる値にしておく。
        transform: Arc::new(vec![crate::resolve::ComputedTransformFunction::TranslateX(
            crate::resolve::ComputedLengthPercentage::Px(48.0),
        )]),
        filter: Arc::new(vec![FilterFunction::Blur(Length::Px(3.0))]),
        // CSS Tables 3 §4: table-layout は non-inherited なので initial
        // (`auto`) と異なる値にしておく。
        table_layout: TableLayoutValue::Fixed,
        // CSS Tables 3 §6: border-collapse は inherited なので initial
        // (`separate`) と異なる値にしておく。
        border_collapse: BorderCollapseValue::Collapse,
        // CSS Tables 3 §6.1: border-spacing は inherited なので initial
        // (両軸 `0px`) と異なる値にしておく。
        border_spacing: crate::resolve::ComputedBorderSpacing {
            horizontal: crate::resolve::ComputedLength(10.0),
            vertical: crate::resolve::ComputedLength(20.0),
        },
        // CSS Tables 3 §7: caption-side は inherited なので initial
        // (`top`) と異なる値にしておく。
        caption_side: CaptionSideValue::Bottom,
        // CSS Tables 3 §8: empty-cells は inherited なので initial
        // (`show`) と異なる値にしておく。
        empty_cells: EmptyCellsValue::Hide,
        column_count: ColumnCountValue::Count(3),
        column_width: crate::resolve::ComputedColumnWidth::Px(24.0),
        custom_properties: crate::computed::empty_custom_properties(),
        local_custom_properties: crate::computed::empty_custom_properties(),
    }
}

#[test]
fn inherit_from_keeps_text_underline_offset_percentage_relative() {
    let mut parent = parent_fixture();
    parent.text_underline_offset = ComputedTextUnderlineOffset::Percent(50.0);
    let child = SpecifiedValues::inherit_from(&parent);
    // CSS Text Decoration 4 §2.8: a percentage inherits as a relative
    // value so it rescales against the child's own font size.
    assert_eq!(
        child.text_underline_offset,
        crate::property::TextUnderlineOffset::Length(Length::Percent(50.0))
    );
}

#[test]
fn inherited_text_underline_offset_calc_keeps_resolved_length_and_relative_percent() {
    let mut parent = parent_fixture();
    parent.font_size = ComputedLength(16.0);
    parent.text_underline_offset =
        ComputedTextUnderlineOffset::Calc(crate::property::CalcLengthPercentage {
            percent: -50.0,
            px: 32.0,
        });
    let mut child = SpecifiedValues::inherit_from(&parent);
    child.font_size = Length::Px(40.0);

    let computed = child.finalize(&parent, &CTX);
    assert_eq!(
        computed.text_underline_offset,
        ComputedTextUnderlineOffset::Calc(crate::property::CalcLengthPercentage {
            percent: -50.0,
            px: 32.0,
        }),
    );
}

#[test]
fn inherit_from_copies_inherited_fields() {
    let parent = parent_fixture();
    let child = SpecifiedValues::inherit_from(&parent);
    assert_eq!(child.color, parent.color);
    assert_eq!(child.font_family, parent.font_family);
    // `inherit_from` の `parent.font_family.clone()` は Arc reference-count increment —
    // deep-clone regression なら ptr_eq が false になる (`Arc::ptr_eq`
    // behavioral-proxy methodology、`mod@crate::cascade` test 群と同型)。
    assert!(Arc::ptr_eq(&child.font_family, &parent.font_family));
    assert_eq!(child.font_weight, 700.0);
    // CSS Lists 3 §3: both list-style longhands are inherited.
    assert_eq!(child.list_style_type, parent.list_style_type);
    assert_eq!(child.list_style_position, parent.list_style_position);
    // CSS Text 3 §6.1: text-align は inherited。
    assert_eq!(child.text_align, TextAlign::Center);
    // CSS Text 3 §8.2.1: hanging-punctuation は inherited。
    assert_eq!(child.hanging_punctuation, HangingPunctuation::First);
    // CSS Text 4: text-autospace は inherited。
    assert_eq!(child.text_autospace, TextAutospace::NoAutospace);
    // CSS Text 4: word-space-transform は inherited。
    assert_eq!(
        child.word_space_transform,
        WordSpaceTransform::SpaceAutoPhrase
    );
    // CSS Text Decoration 4: text-decoration-skip-ink は inherited。
    assert_eq!(child.text_decoration_skip_ink, TextDecorationSkipInk::All);
    // CSS Text Decoration 4: text-decoration-skip-spaces は inherited。
    assert_eq!(
        child.text_decoration_skip_spaces,
        TextDecorationSkipSpaces::End
    );
    // CSS Text Decoration 4 §2.7: text-underline-position is inherited.
    assert_eq!(
        child.text_underline_position,
        parent.text_underline_position
    );
    // CSS Text Decoration 4 §3.4: text-emphasis-position is inherited.
    assert_eq!(child.text_emphasis_position, parent.text_emphasis_position);
    // CSS Text 4: text-spacing-trim is inherited.
    assert_eq!(child.text_spacing_trim, TextSpacingTrim::TrimBoth);
    // CSS Writing Modes 4 §2.1: direction は inherited。
    assert_eq!(child.direction, Direction::Rtl);
    // CSS Writing Modes 4 §3.2: writing-mode は inherited。この staging
    // 層 (`SpecifiedValues::inherit_from`) は `resolve_writing_mode` を
    // 呼ばない素通しコピーなので、`computed::tests::non_initial_parent`
    // の同種 assertion と異なりここでは verbatim 一致を期待してよい
    // (`parent_fixture` の doc comment参照)。
    assert_eq!(child.writing_mode, WritingMode::VerticalRl);
    // CSS Fonts 4 §2.4: font-style は inherited。
    assert_eq!(child.font_style, FontStyle::Italic);
    // CSS Fonts 3 `font-kerning` is inherited.
    assert_eq!(child.font_kerning, FontKerning::Normal);
    // CSS Fonts 4 §8.1: `font-optical-sizing` is inherited.
    assert_eq!(child.font_optical_sizing, FontOpticalSizing::None);
    // CSS Fonts 4 §9.3: font-variant-emoji is inherited.
    assert_eq!(child.font_variant_emoji, FontVariantEmoji::Text);
    // CSS Fonts 4 §6.13: font-language-override is inherited.
    assert_eq!(
        child.font_language_override,
        FontLanguageOverride::String(SmolStr::new("ENG"))
    );
    assert_eq!(
        child.font_variant_ligatures,
        FontVariantLigatures::NoHistoricalLigatures
    );
    assert_eq!(
        child.font_synthesis,
        FontSynthesisValue {
            weight: false,
            style: FontSynthesisStyle::ObliqueOnly,
            small_caps: true,
            position: false,
        }
    );
    // CSS Fonts 3 §6.5: font-variant-position is inherited as specified.
    assert_eq!(child.font_variant_position, FontVariantPosition::Super);
    // CSS Fonts 4 §9.1: font-palette is inherited as specified.
    assert_eq!(
        child.font_palette,
        FontPaletteValue::Palette(SmolStr::new("--parent"))
    );
    // CSS Fonts Module Level 3 §6.6: font-variant-caps は inherited.
    assert_eq!(child.font_variant_caps, FontVariantCaps::SmallCaps);
    // CSS Text Module Level 3 §2.1: text-transform は inherited。
    assert_eq!(child.text_transform, TextTransform::Uppercase);
    assert_eq!(child.text_combine_upright, TextCombineUpright::All);
    assert_eq!(child.unicode_bidi, UnicodeBidi::Normal);
    // CSS Writing Modes 3 §5.1: text-orientation is inherited.
    assert_eq!(child.text_orientation, TextOrientation::Sideways);
    // CSS Display 3 §4: visibility は inherited。
    assert_eq!(child.visibility, Visibility::Hidden);
    // CSS Text 3 §8.1: text-indent は inherited — computed → specified
    // の lift (`lift_text_indent`)。
    assert_eq!(child.text_indent, TextIndentLength::Length(Length::Px(9.0)),);
    // CSS Text Decoration 4 §2.8: the fixed computed offset lifts back
    // to a specified `px` length without being re-based on the child.
    assert_eq!(
        child.text_underline_offset,
        TextUnderlineOffset::Length(Length::Px(5.0))
    );
    // CSS Text 3 §8.1: hanging/each-line flags inherit like the length.
    assert!(child.text_indent_hanging);
    assert!(!child.text_indent_each_line);
    // CSS Text 3 §5.1: word-break は inherited。
    assert_eq!(child.word_break, WordBreak::KeepAll);
    // CSS Text 3 §5.4: overflow-wrap は inherited。
    assert_eq!(child.overflow_wrap, OverflowWrap::Anywhere);
    // CSS Text 3 §3: white-space は inherited。
    assert_eq!(child.white_space, WhiteSpace::Pre);
    // CSS Text 4: white-space-collapse is inherited.
    assert_eq!(
        child.white_space_collapse,
        WhiteSpaceCollapse::PreserveBreaks
    );
    // CSS Text 3 §5.3: hyphens は inherited。
    assert_eq!(child.hyphens, Hyphens::None);
    // CSS Text 4: hyphenate-character is inherited as the decoded string.
    assert_eq!(child.hyphenate_character, parent.hyphenate_character);
    // CSS Text 4: hyphenate-limit-chars inherits its computed triple.
    assert_eq!(child.hyphenate_limit_chars, parent.hyphenate_limit_chars);
    // computed → specified の lift (px 表現)。
    assert_eq!(child.font_size, Length::Px(24.0));
    assert_eq!(child.line_height, LineHeight::Number(1.5));
    // CSS Text 3 §7.2 / §7.1: letter-spacing / word-spacing は共に
    // inherited。computed → specified の lift (px 表現、`lift_font_size`
    // と同型)。
    assert_eq!(
        child.letter_spacing,
        LetterSpacingValue::Length(Length::Px(2.0))
    );
    assert_eq!(
        child.word_spacing,
        WordSpacingValue::Length(Length::Px(4.0))
    );
    // CSS Text Module Level 3 §4.2: tab-size は inherited。computed
    // `<length>` → specified `Px` の lift (`lift_tab_size` 経由、
    // `lift_font_size` と同型)。
    assert_eq!(child.tab_size, TabSize::Length(Length::Px(11.0)));
    // CSS Content 3 §2.4.1: quotes は inherited。`inherit_from` の
    // `parent.quotes.clone()` は Arc reference-count increment —
    // deep-clone regression なら ptr_eq が false になる (`font_family`
    // 同 assertion と同じ methodology)。
    assert_eq!(child.quotes, parent.quotes);
    assert!(Arc::ptr_eq(&child.quotes, &parent.quotes));
    // CSS Text Decoration Module Level 3 §4: text-shadow は
    // inherited。computed → specified の per-item lift
    // (`lift_text_shadow_item`、px 表現) — `<color>` は素通し。
    assert_eq!(
        *child.text_shadow,
        vec![TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(2.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(3.0)),
            color: TextShadowColor::Resolved(CssColor::BLACK),
        }]
    );
    // CSS Fragmentation Module Level 3 §3.3: orphans / widows は共に
    // inherited。
    assert_eq!(child.orphans, 5);
    assert_eq!(child.widows, 7);
    // CSS Tables 3 §6.1: border-spacing は inherited。computed
    // two-length → specified `Px` の lift (`lift_border_spacing` 経由、
    // `lift_tab_size` と同型)。
    assert_eq!(
        child.border_spacing,
        BorderSpacingValue {
            horizontal: Length::Px(10.0),
            vertical: Length::Px(20.0),
        }
    );
    // CSS Tables 3 §7: caption-side は inherited (素朴なコピー)。
    assert_eq!(child.caption_side, CaptionSideValue::Bottom);
    // CSS Tables 3 §8: empty-cells は inherited (素朴なコピー)。
    assert_eq!(child.empty_cells, EmptyCellsValue::Hide);
}

#[test]
fn inherit_from_preserves_word_spacing_percent_and_mixed_calc() {
    let cases = [
        (
            ComputedLetterSpacing::Percent(110.0),
            WordSpacingValue::Length(Length::Percent(110.0)),
        ),
        (
            ComputedLetterSpacing::Calc(crate::property::CalcLengthPercentage {
                percent: -15.0,
                px: 10.0,
            }),
            WordSpacingValue::Calc(crate::property::LengthPercentageCalc {
                percent: -15.0,
                px: 10.0,
                em: 0.0,
            }),
        ),
    ];
    for (computed_value, specified_value) in cases {
        let parent = ComputedValues {
            word_spacing_computed: computed_value,
            ..ComputedValues::initial()
        };
        let child = SpecifiedValues::inherit_from(&parent);
        assert_eq!(child.word_spacing, specified_value);
        let finalized = child.finalize(&parent, &CTX);
        assert_eq!(finalized.word_spacing_computed, computed_value);
        assert_eq!(finalized.word_spacing, ComputedLength::ZERO);
    }
}

#[test]
fn inherit_from_preserves_letter_spacing_percent_and_mixed_calc() {
    let cases = [
        (
            ComputedLetterSpacing::Percent(110.0),
            LetterSpacingValue::Length(Length::Percent(110.0)),
        ),
        (
            ComputedLetterSpacing::Calc(crate::property::CalcLengthPercentage {
                percent: -15.0,
                px: 10.0,
            }),
            LetterSpacingValue::Calc(crate::property::LengthPercentageCalc {
                percent: -15.0,
                px: 10.0,
                em: 0.0,
            }),
        ),
    ];
    for (computed_value, specified_value) in cases {
        let parent = ComputedValues {
            letter_spacing_computed: computed_value,
            ..ComputedValues::initial()
        };
        let child = SpecifiedValues::inherit_from(&parent);
        assert_eq!(child.letter_spacing, specified_value);
        let finalized = child.finalize(&parent, &CTX);
        assert_eq!(finalized.letter_spacing_computed, computed_value);
        assert_eq!(finalized.letter_spacing, ComputedLength::ZERO);
    }
}

#[test]
fn inherit_from_leaves_non_inherited_fields_at_initial() {
    let parent = parent_fixture();
    let child = SpecifiedValues::inherit_from(&parent);
    let initial = SpecifiedValues::initial();
    assert_eq!(child.background_color, CssColor::TRANSPARENT);
    assert_eq!(child.display, DisplayValue::Inline);
    assert!(child.counter_reset.is_empty());
    assert!(child.counter_increment.is_empty());
    assert!(child.counter_set.is_empty());
    assert!(child.content.is_empty());
    assert!(child.string_set.is_empty());
    assert!(child.running_templates.is_empty());
    assert_eq!(child.padding, initial.padding);
    assert_eq!(child.margin, initial.margin);
    assert_eq!(child.border, initial.border);
    assert_eq!(child.width, LengthOrAuto::Auto);
    assert_eq!(child.height, LengthOrAuto::Auto);
    // CSS Multi-column Layout 1: both longhands are non-inherited.
    assert_eq!(child.column_count, initial.column_count);
    assert_eq!(child.column_width, initial.column_width);
    assert_eq!(child.box_sizing, BoxSizing::ContentBox);
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
    // CSS Backgrounds and Borders 3 §2.3/§2.4/§2.5/§2.6/§2.7/§2.8/§2.9: 全て
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

/// `line-height: 150%` を親が宣言していた場合、親の computed は
/// `Length(ComputedLength(px))` であり、子は**その length をそのまま**継承する
/// (CSS Inline 3 §5.1 — percentage は宣言要素で絶対化される)。
#[test]
fn inherit_from_lifts_computed_line_height_length_without_re_resolving() {
    let parent = ComputedValues {
        line_height: ComputedLineHeight::Length(ComputedLength(30.0)),
        ..ComputedValues::initial()
    };
    let child = SpecifiedValues::inherit_from(&parent);
    assert_eq!(child.line_height, LineHeight::Length(Length::Px(30.0)));
    // 子の font-size が 10px でも 15px にはならない。
    let computed = child.finalize(&parent_with_font_size(10.0), &CTX);
    assert_eq!(
        computed.line_height,
        ComputedLineHeight::Length(ComputedLength(30.0))
    );
}

// -----------------------------------------------------------------
// finalize — phase 2 / phase 3 の基準
// -----------------------------------------------------------------

/// CSS Tables 3 §6.1: `border-spacing` の各軸は自 node の font-size
/// 基準で絶対化される (phase 3、`tab_size` の `Length` arm と同型)。
/// WPT border-spacing-computed.html の `"10px 20px"` (two lengths の
/// まま) と `"0"` → `"0px"` (shortest serialization) の pin。
#[test]
fn finalize_resolves_border_spacing_against_own_font_size() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Px(40.0);
    sv.border_spacing = BorderSpacingValue {
        horizontal: Length::Em(0.5),
        vertical: Length::Px(10.0),
    };
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.border_spacing.horizontal, ComputedLength(20.0));
    assert_eq!(cv.border_spacing.vertical, ComputedLength(10.0));
    assert_eq!(cv.border_spacing.serialized(), "20px 10px");
    // initial (`0`) は shortest-serializable。
    let initial_cv = SpecifiedValues::initial().finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(initial_cv.border_spacing.serialized(), "0px");
}

/// CSS Tables 3 §7 / §8: `caption-side` / `empty-cells` は keyword の
/// ため `finalize` を素通しする (`border_collapse` と同じ)。
#[test]
fn finalize_passes_caption_side_and_empty_cells_through_unchanged() {
    let mut sv = SpecifiedValues::initial();
    sv.caption_side = CaptionSideValue::Bottom;
    sv.empty_cells = EmptyCellsValue::Hide;
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.caption_side, CaptionSideValue::Bottom);
    assert_eq!(cv.empty_cells, EmptyCellsValue::Hide);
}

/// phase 2 の `em` は **親** の font-size 基準、phase 3 の `em` は
/// **自 node の (phase 2 で確定した)** font-size 基準
/// (CSS Values 4 §6.1.1)。両者を取り違えると padding が 32px になる。
#[test]
fn finalize_uses_parent_font_size_for_font_size_and_own_for_the_rest() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Em(2.0); // 親 16px → 32px
    sv.padding = Sides::all(Length::Em(1.0)); // 自 32px → 32px
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.font_size, ComputedLength(32.0));
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
}

/// 追加した `ex` も `em` と同じ parent/own 非対称を
/// 持つ (unknown-metric fallback `0.5em`、`Length::Ex` doc)。数値は
/// `finalize_uses_parent_font_size_for_font_size_and_own_for_the_rest`
/// と揃える (`32px` / `32px`) — multiplier を変えて `ex` の `0.5` 係数を
/// 通しても同じ基準規則になることを示す。padding 側に **親** (16px) を
/// 誤って使うと `16 * 0.5 * 2 = 16px` になり、`32px` にならないため
/// parent/own の取り違えを検出できる。
#[test]
fn finalize_resolves_ex_against_parent_for_font_size_and_own_for_padding() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Ex(4.0); // 親 16px 基準 → 0.5 * 4 * 16 = 32px
    sv.padding = Sides::all(Length::Ex(2.0)); // 自 (phase 2 で確定した) 32px 基準 → 0.5 * 2 * 32 = 32px
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.font_size, ComputedLength(32.0));
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(32.0));
}

/// `vertical-align: <length>` absolutizes against the declaring node's
/// own (phase-2-resolved) `font-size` — same basis `letter-spacing`/
/// `word-spacing` use (`resolve_vertical_align` doc).
#[test]
fn finalize_resolves_vertical_align_length_against_own_font_size() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Px(20.0);
    sv.vertical_align = VerticalAlign::Length(Length::Em(1.5)); // 1.5 * 20 = 30px
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.font_size, ComputedLength(20.0));
    assert_eq!(cv.vertical_align, VerticalAlign::Length(Length::Px(30.0)));
}

/// The 6 bare keywords (`baseline`/`sub`/`super`/`middle`/`text-top`/
/// `text-bottom`) pass through `finalize` unchanged — no relative
/// resolution needed (`VerticalAlign` doc's "Scope carving" section).
#[test]
fn finalize_passes_vertical_align_keywords_through_unchanged() {
    let mut sv = SpecifiedValues::initial();
    sv.vertical_align = VerticalAlign::Middle;
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.vertical_align, VerticalAlign::Middle);
}

#[test]
fn finalize_resolves_vertical_align_percentage_against_own_line_height() {
    // `line-height: 20px` → used 20px → 50% = 10px
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Px(16.0);
    sv.line_height = LineHeight::Length(Length::Px(20.0));
    sv.vertical_align = VerticalAlign::Length(Length::Percent(50.0));
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.vertical_align, VerticalAlign::Length(Length::Px(10.0)));
}

#[test]
fn finalize_vertical_align_percentage_falls_back_to_zero_when_line_height_normal() {
    // `line-height: normal` → `used_line_height_length` is None →
    // spec-deviation fallback to 0px (baseline-equivalent), pinned as
    // documented deviation (`VerticalAlign` doc + `resolve_vertical_align` doc).
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Px(16.0);
    sv.line_height = LineHeight::Normal;
    sv.vertical_align = VerticalAlign::Length(Length::Percent(50.0));
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.vertical_align, VerticalAlign::Length(Length::Px(0.0)));
}

/// `padding: 1lh` needs the **already-resolved own** line-height as its
/// basis — this is exactly the phase-3 reordering
/// this design describes: `absolutize_with` must capture `line_height`
/// into a local *before* resolving `padding`/`margin`/`border`/`width`/
/// `height`, or this would be unable to read it at all.
#[test]
fn finalize_resolves_lh_against_own_line_height_for_padding() {
    let mut sv = SpecifiedValues::initial();
    sv.line_height = LineHeight::Number(2.0); // 自 font-size (16px, inherited) 基準 → used 32px
    sv.padding = Sides::all(Length::Lh(1.5)); // 1.5 * 32 = 48px
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.line_height, ComputedLineHeight::Number(2.0));
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(48.0));
}

/// When the own line-height is unresolvable (`normal`, the initial value
/// — the common case, not an edge case), `1lh` falls back to padding's
/// own spec initial `0` rather than a fabricated length (独立実装: see
/// `crate::resolve::resolve_length_percentage` doc for why). // doc-pointer-lint:ignore: opt-out-3, #[cfg(test)] mod tests (#[test]-item doc) — rustdoc-blind, confirmed via わざと壊して確かめる
#[test]
fn finalize_resolves_lh_falls_back_to_zero_when_line_height_normal() {
    let mut sv = SpecifiedValues::initial(); // line_height stays `normal`
    sv.padding = Sides::all(Length::Lh(1.5));
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.line_height, ComputedLineHeight::Normal);
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(0.0));
}

/// `<percentage>` は property ごとに扱いが違う: `font-size` は length に
/// なり、`padding` / `margin` / `width` / `height` は computed 層に
/// percentage のまま残る (CSS Values 4 §5.5.1 + CSS Box 3 の各 propdef)。
#[test]
fn finalize_keeps_box_percentages_and_resolves_font_size_percentage() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Percent(150.0);
    sv.padding = Sides::all(Length::Percent(25.0));
    sv.margin = Sides::all(LengthOrAuto::Length(Length::Percent(10.0)));
    sv.width = LengthOrAuto::Length(Length::Percent(50.0));
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.font_size, ComputedLength(24.0));
    assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(25.0));
    assert_eq!(
        cv.margin.bottom,
        ComputedLengthPercentageOrAuto::Percent(10.0)
    );
    assert_eq!(cv.width, ComputedLengthPercentageOrAuto::Percent(50.0));
}

/// `rem` は phase 2 / phase 3 のどちらでも `ctx.root_font_size` 基準
/// (CSS Values 4 §6.1.1 <https://www.w3.org/TR/css-values-4/#rem>)。
#[test]
fn finalize_resolves_rem_against_context_root_font_size() {
    let ctx = ResolveContext::new(ComputedLength(20.0));
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Rem(2.0);
    sv.margin = Sides::all(LengthOrAuto::Length(Length::Rem(0.5)));
    let cv = sv.finalize(&parent_with_font_size(64.0), &ctx);
    assert_eq!(cv.font_size, ComputedLength(40.0));
    assert_eq!(cv.margin.top, ComputedLengthPercentageOrAuto::Px(10.0));
}

// -----------------------------------------------------------------
// finalize — text-align: match-parent (CSS Text 3 §6.1)
// -----------------------------------------------------------------

#[test]
fn finalize_resolves_match_parent_against_parent_text_align_and_direction() {
    let mut sv = SpecifiedValues::initial();
    sv.text_align = TextAlign::MatchParent;
    let parent = ComputedValues {
        text_align: TextAlign::Start,
        direction: Direction::Rtl,
        ..ComputedValues::initial()
    };
    let cv = sv.finalize(&parent, &CTX);
    // Start + Rtl → Right (CSS Text 3 §6.1 verbatim table).
    assert_eq!(cv.text_align, TextAlign::Right);
}

/// CSS Text 3 §6.1 says that `match-parent` is interpreted against the
/// parent's direction, not the element's own direction.
#[test]
fn finalize_match_parent_uses_parent_direction_not_own_direction_winner() {
    let mut sv = SpecifiedValues::initial();
    sv.text_align = TextAlign::MatchParent;
    // Own winner for `direction` already applied to the staging value —
    // simulates `direction: rtl` being cascaded on *this* node.
    sv.direction = Direction::Rtl;

    // Parent disagrees: Ltr.
    let parent = ComputedValues {
        text_align: TextAlign::Start,
        direction: Direction::Ltr,
        ..ComputedValues::initial()
    };
    let cv = sv.finalize(&parent, &CTX);
    // Must resolve against the *parent's* Ltr (→ Left), not the node's
    // own Rtl (which would give Right).
    assert_eq!(cv.text_align, TextAlign::Left);
    // The node's own `direction` winner is unaffected — it is a wholly
    // separate property and still flows through to the child's computed
    // value normally.
    assert_eq!(cv.direction, Direction::Rtl);
}

#[test]
fn finalize_copies_non_start_end_parent_text_align_verbatim() {
    let mut sv = SpecifiedValues::initial();
    sv.text_align = TextAlign::MatchParent;
    let parent = ComputedValues {
        text_align: TextAlign::Center,
        ..ComputedValues::initial()
    };
    let cv = sv.finalize(&parent, &CTX);
    assert_eq!(cv.text_align, TextAlign::Center);
}

#[test]
fn finalize_leaves_non_match_parent_text_align_untouched() {
    let mut sv = SpecifiedValues::initial();
    sv.text_align = TextAlign::Center;
    let parent = ComputedValues {
        text_align: TextAlign::Start,
        direction: Direction::Rtl,
        ..ComputedValues::initial()
    };
    let cv = sv.finalize(&parent, &CTX);
    assert_eq!(cv.text_align, TextAlign::Center);
}

/// CSS Text 3 §6.1 verbatim: "Computes to start when specified on the
/// root element." — the parent-direction table does **not** apply here,
/// even if the node itself declares a `direction`.
#[test]
fn finalize_as_root_resolves_match_parent_to_start() {
    let mut sv = SpecifiedValues::initial();
    sv.text_align = TextAlign::MatchParent;
    sv.direction = Direction::Rtl;
    assert_eq!(sv.finalize_as_root().text_align, TextAlign::Start);
}

#[test]
fn finalize_as_root_leaves_non_match_parent_text_align_untouched() {
    let mut sv = SpecifiedValues::initial();
    sv.text_align = TextAlign::Right;
    assert_eq!(sv.finalize_as_root().text_align, TextAlign::Right);
}

/// `direction` itself is a plain inherited pass-through — no
/// `match-parent`-style resolution, symmetric with `TextAlign::Center`
/// et al.
#[test]
fn finalize_passes_direction_through_unchanged() {
    let mut sv = SpecifiedValues::initial();
    sv.direction = Direction::Rtl;
    let cv = sv.clone().finalize(&ComputedValues::initial(), &CTX);
    assert_eq!(cv.direction, Direction::Rtl);
    assert_eq!(sv.finalize_as_root().direction, Direction::Rtl);
}

/// The renderer-facing `ComputedValues::writing_mode` fallback collapses
/// every non-`horizontal-tb` keyword to `HorizontalTb`. The separate CSSOM
/// computed value preserves the specified keyword through both finalize paths.
#[test]
fn finalize_preserves_cssom_writing_mode_and_collapses_renderer_fallback() {
    for specified in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ] {
        let mut sv = SpecifiedValues::initial();
        sv.writing_mode = specified;
        let cv = sv.clone().finalize(&ComputedValues::initial(), &CTX);
        assert_eq!(cv.writing_mode, WritingMode::HorizontalTb);
        assert_eq!(cv.cssom_writing_mode, specified);
        let root = sv.finalize_as_root();
        assert_eq!(root.writing_mode, WritingMode::HorizontalTb);
        assert_eq!(root.cssom_writing_mode, specified);
    }
}

/// Inheritance preserves the parent's CSSOM computed keyword while the
/// renderer-facing `writing_mode` field retains its horizontal fallback.
#[test]
fn inherit_from_then_finalize_preserves_cssom_writing_mode() {
    let parent = ComputedValues {
        writing_mode: WritingMode::VerticalRl,
        cssom_writing_mode: WritingMode::VerticalRl,
        ..ComputedValues::initial()
    };
    let child = ComputedValues::inherit_from(&parent);
    assert_eq!(child.writing_mode, WritingMode::HorizontalTb);
    assert_eq!(child.cssom_writing_mode, WritingMode::VerticalRl);
}

/// root element では `rem` の基準が phase 2 と phase 3 で異なる
/// (`SpecifiedValues::finalize_as_root` の doc に spec verbatim)。
///
/// - `font-size: 2rem` → **32px** (initial 16px 基準 — font-\* property 上の
///   自己参照 unit なので parent-metrics 条項が発火する)
/// - `padding: 2rem` → **40px** (自 font-size 20px 基準 — box property は
///   条項の対象外で `rem` は素の定義「root element の computed font-size」)
#[test]
fn finalize_as_root_uses_initial_for_font_size_and_own_for_box_properties() {
    let mut fs_case = SpecifiedValues::initial();
    fs_case.font_size = Length::Rem(2.0);
    assert_eq!(fs_case.finalize_as_root().font_size, ComputedLength(32.0));

    let mut box_case = SpecifiedValues::initial();
    box_case.font_size = Length::Px(20.0);
    box_case.padding = Sides::all(Length::Rem(2.0));
    let cv = box_case.finalize_as_root();
    assert_eq!(cv.font_size, ComputedLength(20.0));
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(40.0));
}

/// root element の `font-size: Nem` も親が無いので initial 16px 基準。
#[test]
fn finalize_as_root_resolves_em_font_size_against_initial() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Em(1.5);
    assert_eq!(sv.finalize_as_root().font_size, ComputedLength(24.0));
}

/// root element の **box property** の `1rlh` は自分の確定済 line-height
/// を基準にする (`rem_on_root_element_box_property_uses_own_font_size`
/// の `rem` と同じ非対称の `rlh` 版。self-reference 条項の対象は
/// `line-height` 自身の値だけで、`padding` はその対象外)。
/// `finalize_as_root` は own line-height を phase 2.5 で確定させてから
/// `ResolveContext::with_root_line_height` を組み立てる — この test は
/// その配線がここまで届くことを直接 check する。
#[test]
fn finalize_as_root_resolves_rlh_using_own_line_height_basis() {
    let mut sv = SpecifiedValues::initial();
    sv.font_size = Length::Px(20.0);
    sv.line_height = LineHeight::Number(2.0); // own font-size 20px → used 40px
    sv.padding = Sides::all(Length::Rlh(1.5)); // 1.5 * 40 = 60px
    let cv = sv.finalize_as_root();
    assert_eq!(cv.line_height, ComputedLineHeight::Number(2.0));
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(60.0));
}

/// root element には親が無いので、`line-height` 自身の値としての
/// `1lh`/`1rlh` (自己参照) は常に「initial values」= `normal` 基準に
/// 帰着し、常に unresolved になる (CSS Values 4 §6.1.1 "if the element
/// has no parent" — `finalize_as_root` doc 参照)。上の test と対で、
/// 「box property の rlh は自分の line-height を使う」「line-height 自身の
/// lh/rlh は self-reference で常に normal」の 2 つの非対称を区別する。
#[test]
fn finalize_as_root_line_height_self_reference_is_always_normal() {
    let mut sv = SpecifiedValues::initial();
    sv.line_height = LineHeight::Length(Length::Lh(1.0));
    assert_eq!(
        sv.finalize_as_root().line_height,
        ComputedLineHeight::Normal
    );

    let mut sv_rlh = SpecifiedValues::initial();
    sv_rlh.line_height = LineHeight::Length(Length::Rlh(1.0));
    assert_eq!(
        sv_rlh.finalize_as_root().line_height,
        ComputedLineHeight::Normal
    );
}

/// 全 4 side が独立に絶対化される (`Sides::map` が side を取り違えない)。
#[test]
fn finalize_absolutizes_each_side_independently() {
    let mut sv = SpecifiedValues::initial();
    sv.padding = Sides {
        top: Length::Px(1.0),
        right: Length::Em(1.0),
        bottom: Length::Pt(3.0),
        left: Length::Percent(5.0),
    };
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(cv.padding.top, ComputedLengthPercentage::Px(1.0));
    assert_eq!(cv.padding.right, ComputedLengthPercentage::Px(16.0));
    assert_eq!(cv.padding.bottom, ComputedLengthPercentage::Px(4.0));
    assert_eq!(cv.padding.left, ComputedLengthPercentage::Percent(5.0));
}

/// `text-shadow` の list-shaped phase 3 — 直上
/// `finalize_absolutizes_each_side_independently` の `Sides<Length>`
/// precedent を可変長 list に一般化したもの。各 item の
/// 3 length (`offset_x`/`offset_y`/`blur_radius`) が own-node font-size
/// basis で独立に絶対化される (`resolve_text_shadow_item`)、`<color>` は
/// 素通し ([`TextShadowColor`] doc)。
#[test]
fn finalize_absolutizes_each_text_shadow_item_independently() {
    let mut sv = SpecifiedValues::initial();
    sv.text_shadow = Arc::new(vec![
        TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Em(1.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Rem(2.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Pt(3.0)),
            color: TextShadowColor::CurrentColor,
        },
        TextShadowItem {
            offset_x: crate::property::TextShadowLength::Length(Length::Px(4.0)),
            offset_y: crate::property::TextShadowLength::Length(Length::Px(5.0)),
            blur_radius: crate::property::TextShadowLength::Length(Length::Px(0.0)),
            color: TextShadowColor::Resolved(CssColor::BLACK),
        },
    ]);
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert_eq!(
        *cv.text_shadow,
        vec![
            ComputedTextShadow {
                // 1em * own font-size (16px, `SpecifiedValues::initial`).
                offset_x: ComputedLength(16.0),
                // 2rem * root font-size (16px, `CTX`).
                offset_y: ComputedLength(32.0),
                // 3pt = 3 * 4/3 px = 4px.
                blur_radius: ComputedLength(4.0),
                color: TextShadowColor::CurrentColor,
            },
            ComputedTextShadow {
                offset_x: ComputedLength(4.0),
                offset_y: ComputedLength(5.0),
                blur_radius: ComputedLength::ZERO,
                color: TextShadowColor::Resolved(CssColor::BLACK),
            },
        ]
    );
}

/// 空 list (`none`) は allocation せず shared computed-empty-Arc slot を
/// 再利用する — [`Self::absolutize_with`] の `text_shadow` arm doc 参照。
#[test]
fn finalize_empty_text_shadow_list_reuses_shared_computed_empty_arc() {
    let sv = SpecifiedValues::initial();
    assert!(sv.text_shadow.is_empty());
    let cv = sv.finalize(&parent_with_font_size(16.0), &CTX);
    assert!(cv.text_shadow.is_empty());
    assert!(Arc::ptr_eq(
        &cv.text_shadow,
        &ComputedValues::initial().text_shadow
    ));
}
