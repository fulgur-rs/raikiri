//! Phase 3 for the page context: `absolutize_in_page_context` and the
//! per-value helpers it uses to map computed values back into `PropertyValue`.

use std::sync::Arc;

use crate::cascade::{ResolvedAgainstInherited, resolve_relative_font_size};
use crate::property::{
    BackgroundShorthand, BackgroundSize, Border, BorderColor, BorderRadius, BorderStyle,
    BoxShadowItem, ColumnWidthValue, ColumnsShorthand, CssPosition, CssPositionOffset,
    FlexBasisValue, FlexShorthand, FontShorthand, FontShorthandSize, GapShorthand,
    GridInflexibleBreadth, GridTemplateTracks, GridTrackBreadth, GridTrackList,
    GridTrackListComponent, GridTrackRepeat, GridTrackSize, Length, LengthOrAuto, LengthOrNormal,
    LengthPercentageCalc, Outline, OutlineColor, OutlineStyle, OverflowXY, PropertyValue, Sides,
    TextDecorationInset, TextDecorationShorthand, TextDecorationThickness, TextIndentLength,
    TextIndentValue, TextShadowItem, TextShadowLength, TextUnderlineOffset, TransformFunction,
    resolve_overflow, resolve_writing_mode,
};
use crate::resolve::{
    ComputedBackgroundSize, ComputedCssPositionOffset, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ComputedTextIndent, ResolveContext, lift_border_spacing, lift_letter_spacing, lift_line_height,
    lift_tab_size, lift_text_indent, lift_word_spacing, resolve_background_size, resolve_border,
    resolve_border_radius, resolve_border_spacing, resolve_box_shadow_item, resolve_css_position,
    resolve_flex_basis, resolve_grid_auto_track_list, resolve_grid_template_tracks, resolve_length,
    resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_letter_spacing, resolve_line_height,
    resolve_margin_length_or_auto, resolve_outline, resolve_tab_size, resolve_text_indent_calc,
    resolve_text_shadow_item, resolve_vertical_align, resolve_word_spacing,
};

/// Basis for resolving lengths in the page context: the page context's own
/// `font-size`, its own `lh` basis, and the shared [`ResolveContext`].
#[derive(Clone, Copy)]
struct PageLengthBasis<'a> {
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &'a ResolveContext,
}

impl PageLengthBasis<'_> {
    /// `<length-percentage>` → computed, mapped back into the specified-layer
    /// `Length` shape that `PropertyValue` carries (`Px` / `Percent` only —
    /// `Em` / `Rem` / `Pt` are gone after this).
    fn lp(self, specified: Length) -> Length {
        match resolve_length_percentage(specified, self.font_size, self.own_line_height, self.ctx) {
            ComputedLengthPercentage::Px(v) => Length::Px(v),
            ComputedLengthPercentage::Percent(p) => Length::Percent(p),
        }
    }
    /// `<length-percentage> | auto` — same mapping, `auto` preserved.
    fn lpa(self, specified: LengthOrAuto) -> LengthOrAuto {
        match resolve_length_percentage_or_auto(
            specified,
            self.font_size,
            self.own_line_height,
            self.ctx,
        ) {
            ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
            ComputedLengthPercentageOrAuto::Percent(p) => LengthOrAuto::Length(Length::Percent(p)),
            ComputedLengthPercentageOrAuto::Calc(value) => LengthOrAuto::Calc(value),
        }
    }
    /// `<length-percentage> | auto` for `margin-*` specifically —
    /// same mapping as [`Self::lpa`] but
    /// routed through [`resolve_margin_length_or_auto`], whose unresolvable-`lh`/
    /// `rlh` fallback is `Px(0.0)` (margin's true spec initial), not `Auto`
    /// (which is `width`/`height`'s initial, and which would trigger real
    /// auto-margin layout behaviour if used here — see that function's doc).
    fn margin_lpa(self, specified: LengthOrAuto) -> LengthOrAuto {
        match resolve_margin_length_or_auto(
            specified,
            self.font_size,
            self.own_line_height,
            self.ctx,
        ) {
            ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
            ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
            ComputedLengthPercentageOrAuto::Percent(p) => LengthOrAuto::Length(Length::Percent(p)),
            ComputedLengthPercentageOrAuto::Calc(value) => LengthOrAuto::Calc(value),
        }
    }
    /// `flex-basis: content | <'width'>` — same mapping as [`Self::lpa`], with
    /// `content` preserved as its own keyword (`ComputedFlexBasis::Content`
    /// has no `LengthOrAuto` equivalent, so it maps back to
    /// `FlexBasisValue::Content` directly rather than routing through the
    /// `Auto`/`Px`/`Percent` cases `lpa` shares).
    fn fb(self, specified: FlexBasisValue) -> FlexBasisValue {
        match resolve_flex_basis(specified, self.font_size, self.own_line_height, self.ctx) {
            ComputedFlexBasis::Auto => FlexBasisValue::Auto,
            ComputedFlexBasis::Content => FlexBasisValue::Content,
            ComputedFlexBasis::MinContent => FlexBasisValue::MinContent,
            ComputedFlexBasis::MaxContent => FlexBasisValue::MaxContent,
            ComputedFlexBasis::FitContent => FlexBasisValue::FitContent,
            ComputedFlexBasis::Px(v) => FlexBasisValue::Length(Length::Px(v)),
            ComputedFlexBasis::Percent(p) => FlexBasisValue::Length(Length::Percent(p)),
        }
    }
    /// `background-size: <bg-size>` — same round-trip as [`Self::lpa`], applied
    /// per axis via [`resolve_background_size`]; `cover`/`contain` carry no
    /// length and pass straight through.
    fn background_size(self, specified: BackgroundSize) -> BackgroundSize {
        fn lift(computed: ComputedLengthPercentageOrAuto) -> LengthOrAuto {
            match computed {
                ComputedLengthPercentageOrAuto::Auto => LengthOrAuto::Auto,
                ComputedLengthPercentageOrAuto::Px(v) => LengthOrAuto::Length(Length::Px(v)),
                ComputedLengthPercentageOrAuto::Percent(p) => {
                    LengthOrAuto::Length(Length::Percent(p))
                }
                ComputedLengthPercentageOrAuto::Calc(value) => LengthOrAuto::Calc(value),
            }
        }
        match resolve_background_size(specified, self.font_size, self.own_line_height, self.ctx) {
            ComputedBackgroundSize::Cover => BackgroundSize::Cover,
            ComputedBackgroundSize::Contain => BackgroundSize::Contain,
            ComputedBackgroundSize::Explicit { width, height } => BackgroundSize::Explicit {
                width: lift(width),
                height: lift(height),
            },
        }
    }
    /// `background-position: <position>` — same round-trip as [`Self::lp`],
    /// applied to each edge/offset via [`resolve_css_position`].
    fn css_position(self, specified: CssPosition) -> CssPosition {
        fn lift(computed: ComputedLengthPercentage) -> Length {
            match computed {
                ComputedLengthPercentage::Px(v) => Length::Px(v),
                ComputedLengthPercentage::Percent(p) => Length::Percent(p),
            }
        }
        fn lift_offset(computed: ComputedCssPositionOffset) -> CssPositionOffset {
            match computed {
                ComputedCssPositionOffset::Start(l) => CssPositionOffset::Start(lift(l)),
                ComputedCssPositionOffset::End(l) => CssPositionOffset::End(lift(l)),
            }
        }
        let computed =
            resolve_css_position(specified, self.font_size, self.own_line_height, self.ctx);
        CssPosition {
            horizontal: lift_offset(computed.horizontal),
            vertical: lift_offset(computed.vertical),
        }
    }
    /// `row-gap` / `column-gap`: `normal | <length-percentage [0,∞]>` —
    /// same mapping shape as [`Self::lp`], with `normal` preserved (unlike
    /// `letter-spacing`/`word-spacing`'s `lift_length_or_normal`, which
    /// collapses `normal` to a resolved `ComputedLength` — gap's `normal`
    /// stays a keyword at the computed layer, see
    /// [`ComputedLengthPercentageOrNormal`] doc).
    fn lpn(self, specified: LengthOrNormal) -> LengthOrNormal {
        match resolve_length_percentage_or_normal(
            specified,
            self.font_size,
            self.own_line_height,
            self.ctx,
        ) {
            ComputedLengthPercentageOrNormal::Normal => LengthOrNormal::Normal,
            ComputedLengthPercentageOrNormal::Px(v) => LengthOrNormal::Length(Length::Px(v)),
            ComputedLengthPercentageOrNormal::Percent(p) => {
                LengthOrNormal::Length(Length::Percent(p))
            }
        }
    }
    /// `grid-template-columns` / `grid-template-rows` — round-trips through
    /// [`resolve_grid_template_tracks`] and maps the `Computed*` mirror
    /// types back to the specified-layer shape (`none` preserved as a
    /// keyword, same as [`Self::fb`]'s `Auto`/`Content` handling).
    fn gtt(self, specified: GridTemplateTracks) -> GridTemplateTracks {
        match resolve_grid_template_tracks(
            specified,
            self.font_size,
            self.own_line_height,
            self.ctx,
        ) {
            ComputedGridTemplateTracks::None => GridTemplateTracks::None,
            ComputedGridTemplateTracks::List(list) => {
                GridTemplateTracks::List(Arc::new(grid_track_list_from_computed(&list)))
            }
        }
    }
    /// `grid-auto-columns` / `grid-auto-rows` — same round-trip shape as
    /// [`Self::gtt`], for the `<track-size>+` (no `none`, no `<line-names>`)
    /// grammar.
    fn gatl(self, specified: &[GridTrackSize]) -> Arc<Vec<GridTrackSize>> {
        let computed =
            resolve_grid_auto_track_list(specified, self.font_size, self.own_line_height, self.ctx);
        Arc::new(
            computed
                .iter()
                .cloned()
                .map(grid_track_size_from_computed)
                .collect(),
        )
    }
    /// One `border-*` side: absolutize the width and apply the style gate.
    ///
    /// Routed through [`resolve_border`] rather than re-testing
    /// `none` / `hidden` locally — that function's doc calls itself the single
    /// source of the gate and forbids re-implementing the rule elsewhere.
    fn border(self, specified: Border) -> Border {
        let computed = resolve_border(specified, self.font_size, self.own_line_height, self.ctx);
        Border {
            width: Length::Px(computed.width.px()),
            style: computed.style,
            color: computed.color,
        }
    }
    /// A `border-*-width` longhand, gated by the side's computed style.
    ///
    /// `color` is a placeholder — [`resolve_border`] never reads it, and the
    /// longhand carries no colour. Building the `Border` here (instead of
    /// branching on `style` locally) is what keeps the gate single-sourced.
    fn border_width(self, width: Length, style: BorderStyle) -> Length {
        self.border(Border {
            width,
            style,
            color: BorderColor::CurrentColor,
        })
        .width
    }
    /// An `outline-width` longhand, gated by the winning `outline-style`.
    ///
    /// Routing through [`resolve_outline`] keeps the element and page paths
    /// on the same CSS UI 3 §4.2 computed-value rule: width is zero when the
    /// style is `none` (or the retained-but-parser-rejected `hidden` keyword).
    fn outline_width(self, width: Length, style: OutlineStyle) -> Length {
        Length::Px(
            resolve_outline(
                Outline {
                    width,
                    style,
                    color: OutlineColor::Invert,
                },
                self.font_size,
                self.own_line_height,
                self.ctx,
            )
            .width()
            .px(),
        )
    }
    /// One `text-shadow` item: resolve all three lengths through the same
    /// computed-value helper used by element styles, then store the absolute
    /// pixel values back in the page's specified-value bag. This also applies
    /// the calculated blur-radius clamp consistently.
    fn text_shadow_item(self, specified: TextShadowItem) -> TextShadowItem {
        let computed =
            resolve_text_shadow_item(specified, self.font_size, self.own_line_height, self.ctx);
        TextShadowItem {
            offset_x: TextShadowLength::Length(Length::Px(computed.offset_x.px())),
            offset_y: TextShadowLength::Length(Length::Px(computed.offset_y.px())),
            blur_radius: TextShadowLength::Length(Length::Px(computed.blur_radius.px())),
            color: computed.color,
        }
    }

    /// `border-radius` の 4 corner を computed 長から `PropertyValue` が
    /// 運ぶ `Length::Px` 表現へ戻す。page bag は element path の
    /// `ComputedValues` と同じ computed-value 契約を持つが、既存の bag の
    /// API を壊さないため payload 型は specified 側を再利用する。
    fn border_radius_value(self, specified: BorderRadius) -> BorderRadius {
        let computed =
            resolve_border_radius(specified, self.font_size, self.own_line_height, self.ctx);
        let length = |value: ComputedLengthPercentage| match value {
            ComputedLengthPercentage::Px(px) => Length::Px(px),
            ComputedLengthPercentage::Percent(percent) => Length::Percent(percent),
        };
        BorderRadius {
            top_left: length(computed.top_left),
            top_right: length(computed.top_right),
            bottom_right: length(computed.bottom_right),
            bottom_left: length(computed.bottom_left),
        }
    }

    /// `box-shadow` の 1 item を computed 長から page bag の
    /// `BoxShadowItem` へ戻す。`color` は resolve 層で長さを持たないため
    /// そのまま保持する。
    fn box_shadow_item(self, specified: BoxShadowItem) -> BoxShadowItem {
        let computed =
            resolve_box_shadow_item(specified, self.font_size, self.own_line_height, self.ctx);
        BoxShadowItem {
            offset_x: Length::Px(computed.offset_x.px()),
            offset_y: Length::Px(computed.offset_y.px()),
            blur_radius: Length::Px(computed.blur_radius.px()),
            spread_radius: Length::Px(computed.spread_radius.px()),
            color: computed.color,
            inset: computed.inset,
        }
    }

    /// `outline` の width を絶対化した computed value を
    /// `PropertyValue` の payload に戻す。outline は box model の寸法へ
    /// 影響しないため、page phase では値の解決だけを行う。
    fn outline_value(self, specified: Outline) -> Outline {
        let computed = resolve_outline(specified, self.font_size, self.own_line_height, self.ctx);
        Outline {
            width: Length::Px(computed.width().px()),
            style: computed.style(),
            color: computed.color,
        }
    }

    /// `transform` の `<length-percentage>` slot を絶対化し page bag の
    /// `TransformFunction` へ戻す。`lp` と同じ round-trip — length 側は
    /// `Px` へ、percentage 側は `Percent` のまま残す。`matrix` の 6
    /// `<number>` slot と `rotate`/`skew` 系の `<angle>` slot はそのまま。
    fn transform_function(self, specified: TransformFunction) -> TransformFunction {
        match specified {
            TransformFunction::Matrix(m) => TransformFunction::Matrix(m),
            TransformFunction::Translate(tx, ty) => {
                TransformFunction::Translate(self.lp(tx), self.lp(ty))
            }
            TransformFunction::TranslateX(v) => TransformFunction::TranslateX(self.lp(v)),
            TransformFunction::TranslateY(v) => TransformFunction::TranslateY(self.lp(v)),
            TransformFunction::Scale(x, y) => TransformFunction::Scale(x, y),
            TransformFunction::ScaleX(v) => TransformFunction::ScaleX(v),
            TransformFunction::ScaleY(v) => TransformFunction::ScaleY(v),
            TransformFunction::Rotate(a) => TransformFunction::Rotate(a),
            TransformFunction::Skew(ax, ay) => TransformFunction::Skew(ax, ay),
            TransformFunction::SkewX(a) => TransformFunction::SkewX(a),
            TransformFunction::SkewY(a) => TransformFunction::SkewY(a),
        }
    }
}

/// [`ComputedGridTrackBreadth`] → specified [`GridTrackBreadth`] —
/// round-trip half of [`PageLengthBasis::gtt`]/[`PageLengthBasis::gatl`], same "computed-equivalent
/// specified representation" convention as [`PageLengthBasis::fb`]/[`PageLengthBasis::lpn`] (`Px`/
/// `Percent` only, no other unit ever appears post-absolutization).
fn grid_track_breadth_from_computed(b: ComputedGridTrackBreadth) -> GridTrackBreadth {
    match b {
        ComputedGridTrackBreadth::Px(v) => GridTrackBreadth::Length(Length::Px(v)),
        ComputedGridTrackBreadth::Percent(p) => GridTrackBreadth::Length(Length::Percent(p)),
        ComputedGridTrackBreadth::Flex(f) => GridTrackBreadth::Flex(f),
        ComputedGridTrackBreadth::MinContent => GridTrackBreadth::MinContent,
        ComputedGridTrackBreadth::MaxContent => GridTrackBreadth::MaxContent,
        ComputedGridTrackBreadth::Auto => GridTrackBreadth::Auto,
    }
}
/// [`ComputedGridTrackBreadth`] → specified [`GridInflexibleBreadth`] —
/// sibling of [`grid_track_breadth_from_computed`] for `minmax()`'s min
/// side. `ComputedGridTrackBreadth::Flex` never reaches
/// here (`ComputedGridTrackBreadth`'s doc "collapse" note: the min side
/// is only ever produced by [`crate::resolve::resolve_grid_inflexible_breadth`],
/// which has no `Flex` variant to produce it from) — the arm is a
/// defensive fallback to keep the match exhaustive, not a reachable case.
fn grid_inflexible_breadth_from_computed(b: ComputedGridTrackBreadth) -> GridInflexibleBreadth {
    match b {
        ComputedGridTrackBreadth::Px(v) => GridInflexibleBreadth::Length(Length::Px(v)),
        ComputedGridTrackBreadth::Percent(p) => GridInflexibleBreadth::Length(Length::Percent(p)),
        ComputedGridTrackBreadth::MinContent => GridInflexibleBreadth::MinContent,
        ComputedGridTrackBreadth::MaxContent => GridInflexibleBreadth::MaxContent,
        // cov:ignore: unreachable per this fn's doc; kept for match
        // exhaustiveness.
        ComputedGridTrackBreadth::Flex(_) | ComputedGridTrackBreadth::Auto => {
            GridInflexibleBreadth::Auto
        }
    }
}
/// [`ComputedGridTrackSize`] → specified [`GridTrackSize`] — round-trip
/// half of [`PageLengthBasis::gtt`]/[`PageLengthBasis::gatl`].
fn grid_track_size_from_computed(s: ComputedGridTrackSize) -> GridTrackSize {
    match s {
        ComputedGridTrackSize::Breadth(b) => {
            GridTrackSize::Breadth(grid_track_breadth_from_computed(b))
        }
        ComputedGridTrackSize::MinMax(min, max) => GridTrackSize::MinMax(
            grid_inflexible_breadth_from_computed(min),
            grid_track_breadth_from_computed(max),
        ),
        ComputedGridTrackSize::FitContent(lp) => GridTrackSize::FitContent(match lp {
            ComputedLengthPercentage::Px(v) => Length::Px(v),
            ComputedLengthPercentage::Percent(p) => Length::Percent(p),
        }),
    }
}
/// [`ComputedGridTrackList`] → specified [`GridTrackList`] — round-trip
/// half of [`PageLengthBasis::gtt`]. `line_names` carry no length so they clone through
/// unchanged.
fn grid_track_list_from_computed(list: &ComputedGridTrackList) -> GridTrackList {
    GridTrackList {
        line_names: list.line_names.clone(),
        components: list
            .components
            .iter()
            .cloned()
            .map(|c| match c {
                ComputedGridTrackListComponent::Size(s) => {
                    GridTrackListComponent::Size(grid_track_size_from_computed(s))
                }
                ComputedGridTrackListComponent::Repeat(r) => {
                    GridTrackListComponent::Repeat(GridTrackRepeat {
                        count: r.count,
                        line_names: r.line_names,
                        tracks: r
                            .tracks
                            .into_iter()
                            .map(grid_track_size_from_computed)
                            .collect(),
                    })
                }
            })
            .collect(),
    }
}

/// **phase 3** for the page context — absolutize one winner against the page
/// context's own `font-size` and apply the `border-*-width` style gate.
///
/// Page-path sibling of the element path's
/// [`crate::specified::SpecifiedValues::finalize`] (`absolutize_with`); both
/// call the same [`crate::resolve`] functions. Keyword-only and already
/// computed-equivalent values pass through unchanged; length-bearing values
/// have relative units resolved to `px`, while `<percentage>` stays symbolic
/// for the used-value layer (see
/// [`PageCascadeResult::declarations`](super::PageCascadeResult::declarations)).
///
/// The match has no wildcard arm, like
/// [`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]:
/// a new length-bearing [`PropertyValue`] variant must be classified here
/// explicitly rather than leak into `declarations` as a specified value.
///
/// # Parameters
///
/// - `value`: phase-2 output ([`crate::cascade::ResolvedAgainstInherited`]).
/// - `font_size`: this page context's own resolved `font-size` (`em` basis).
/// - `own_line_height`: this context's own `lh` basis, from
///   [`page_context_line_height_basis`](super::cascade::page_context_line_height_basis);
///   `rlh` uses `ctx.root_line_height` instead.
/// - `border_styles` / `outline_style`: the context's style winners, which gate
///   `border-*-width` / `outline-width`.
/// - `overflow_pair`: the raw `overflow-x`/`overflow-y` winners from
///   [`page_context_overflow_pair`](super::cascade::page_context_overflow_pair),
///   since [`resolve_overflow`]'s cross-axis coupling needs both axes while
///   this function sees one winner at a time.
pub(super) fn absolutize_in_page_context(
    value: ResolvedAgainstInherited,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
    border_styles: Sides<BorderStyle>,
    outline_style: OutlineStyle,
    overflow_pair: OverflowXY,
) -> PropertyValue {
    let value = value.into_property_value();
    let basis = PageLengthBasis {
        font_size,
        own_line_height,
        ctx,
    };

    match value {
        // ── already computed-equivalent after phase 2 ──────────────────────
        // `font-size` is already phase-2 output (`Length::Px`); re-absolutizing it
        // here would apply it against its own value instead of the parent's.
        v @ (PropertyValue::Color(_)
        | PropertyValue::CustomProperty(_)
        | PropertyValue::Deferred(_)
        | PropertyValue::Grid(_)
        | PropertyValue::GridArea(_)
        | PropertyValue::BackgroundColor(_)
        | PropertyValue::FontFamily(_)
        | PropertyValue::FontSize(_)
        | PropertyValue::FontWeight(_)
        | PropertyValue::Display(_)
        | PropertyValue::ListStyleType(_)
        | PropertyValue::ListStyleImage(_)
        | PropertyValue::ListStylePosition(_)
        | PropertyValue::CounterReset(_)
        | PropertyValue::CounterResetInherit
        | PropertyValue::CounterIncrement(_)
        | PropertyValue::CounterSet(_)
        | PropertyValue::Content(_)
        | PropertyValue::StringSet(_)
        | PropertyValue::Position(_)
        | PropertyValue::TextAlign(_)
        | PropertyValue::HangingPunctuation(_)
        | PropertyValue::TextAutospace(_)
        | PropertyValue::WordSpaceTransform(_)
        | PropertyValue::TextSpacingTrim(_)
        | PropertyValue::Direction(_)
        | PropertyValue::BorderTopStyle(_)
        | PropertyValue::BorderRightStyle(_)
        | PropertyValue::BorderBottomStyle(_)
        | PropertyValue::BorderLeftStyle(_)
        | PropertyValue::BorderTopColor(_)
        | PropertyValue::BorderRightColor(_)
        | PropertyValue::BorderBottomColor(_)
        | PropertyValue::BorderLeftColor(_)
        | PropertyValue::OutlineStyle(_)
        | PropertyValue::OutlineColor(_)
        | PropertyValue::BoxSizing(_)
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextEmphasisStyle(_)
        | PropertyValue::TextEmphasisColor(_)
        | PropertyValue::TextEmphasis(_)
        | PropertyValue::TextUnderlinePosition(_)
        | PropertyValue::FontStyle(_)
        | PropertyValue::FontKerning(_)
        | PropertyValue::FontOpticalSizing(_)
        | PropertyValue::FontVariantEmoji(_)
        | PropertyValue::FontLanguageOverride(_)
        | PropertyValue::FontVariantLigatures(_)
        | PropertyValue::FontSynthesis(_)
        | PropertyValue::FontVariantPosition(_)
        | PropertyValue::FontPalette(_)
        | PropertyValue::FontVariantNumeric(_)
        | PropertyValue::FontVariantEastAsian(_)
        | PropertyValue::FontVariantCaps(_)
        | PropertyValue::TextTransform(_)
        | PropertyValue::Visibility(_)
        | PropertyValue::ZIndex(_)
        | PropertyValue::WordBreak(_)
        | PropertyValue::OverflowWrap(_)
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        | PropertyValue::BreakInside(_)
        | PropertyValue::Float(_)
        | PropertyValue::Clear(_)
        | PropertyValue::WhiteSpace(_)
        | PropertyValue::WhiteSpaceCollapse(_)
        | PropertyValue::TextWrap(_)
        | PropertyValue::TextWrapStyle(_)
        | PropertyValue::TextWrapShorthand(_)
        | PropertyValue::TextSpacingShorthand(_)
        | PropertyValue::Hyphens(_)
        | PropertyValue::HyphenateCharacter(_)
        | PropertyValue::HyphenateLimitChars(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        | PropertyValue::Page(_)
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(ColumnWidthValue::Auto)
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        | PropertyValue::FlexFlow(_)
        | PropertyValue::Order(_)
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        | PropertyValue::PlaceContent(_)
        | PropertyValue::Quotes(_)
        | PropertyValue::GridTemplateAreas(_)
        | PropertyValue::GridAutoFlow(_)
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        | PropertyValue::GridRow(_)
        | PropertyValue::GridColumn(_)
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        | PropertyValue::PlaceItems(_)
        | PropertyValue::PlaceSelf(_)
        | PropertyValue::Orphans(_)
        | PropertyValue::Widows(_)
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)

        | PropertyValue::ObjectFit(_)
        | PropertyValue::Isolation(_)
        | PropertyValue::MixBlendMode(_)

        | PropertyValue::ClipPath(_)
        // `filter`'s computed value is as specified: embedded lengths stay untouched.
        | PropertyValue::Filter(_)
        | PropertyValue::TableLayout(_)
        | PropertyValue::BorderCollapse(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)) => v,
        PropertyValue::ColumnWidth(ColumnWidthValue::Length(length)) => {
            PropertyValue::ColumnWidth(ColumnWidthValue::Length(Length::Px(
                resolve_length(length, font_size, own_line_height, ctx).px(),
            )))
        }
        // cov:ignore: page-context shorthand absolutization is defensive and not part of the focused layout path.
        PropertyValue::Columns(shorthand) => PropertyValue::Columns(ColumnsShorthand {
            width: match shorthand.width {
                ColumnWidthValue::Auto => ColumnWidthValue::Auto,
                ColumnWidthValue::Length(length) => ColumnWidthValue::Length(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
            },
            count: shorthand.count,
        }),
        // ── background-image / mask-image ───────────────────────────────
        PropertyValue::BackgroundImage(img) => PropertyValue::BackgroundImage(
            crate::resolve::resolve_background_image(img, font_size, own_line_height, ctx),
        ),
        PropertyValue::MaskImage(img) => PropertyValue::MaskImage(
            crate::resolve::resolve_background_image(img, font_size, own_line_height, ctx),
        ),
        // ── font-size: larger / smaller ──────────────────────────────────
        // Unreachable through `cascade_page` (phase 2 resolves it); kept panic-free
        // for direct callers, converging to the same `FontSize(Px(_))` shape.
        PropertyValue::FontSizeRelative(rel) => {
            PropertyValue::FontSize(Length::Px(resolve_relative_font_size(rel, font_size.px())))
        }
        // ── line-height ───────────────────────────────────────────────────
        // `lh`/`rlh` self-references resolve against the root, hence
        // `ctx.root_line_height` rather than `own_line_height`.
        PropertyValue::LineHeight(lh) => PropertyValue::LineHeight(lift_line_height(
            resolve_line_height(lh, font_size, ctx.root_line_height, ctx),
        )),
        // ── text-indent ───────────────────────────────────────────────────
        PropertyValue::TextIndent(v) => {
            let length = match v.length {
                TextIndentLength::Length(length) => TextIndentLength::Length(basis.lp(length)),
                TextIndentLength::Calc(calc) => {
                    lift_text_indent(resolve_text_indent_calc(calc, font_size))
                }
            };
            PropertyValue::TextIndent(TextIndentValue {
                length,
                hanging: v.hanging,
                each_line: v.each_line,
            })
        }
        // ── padding ───────────────────────────────────────────────────────
        PropertyValue::PaddingTop(v) => PropertyValue::PaddingTop(basis.lp(v)),
        PropertyValue::PaddingRight(v) => PropertyValue::PaddingRight(basis.lp(v)),
        PropertyValue::PaddingBottom(v) => PropertyValue::PaddingBottom(basis.lp(v)),
        PropertyValue::PaddingLeft(v) => PropertyValue::PaddingLeft(basis.lp(v)),
        // Shorthand fall-throughs: unreachable through `cascade_page` (shorthands are
        // expanded first) but kept as real arms, not `unreachable!`, so the cascade
        // stays panic-free for direct callers.
        PropertyValue::Padding(sides) => PropertyValue::Padding(sides.map(|l| basis.lp(l))),
        PropertyValue::PaddingInline(pair) => {
            PropertyValue::PaddingInline(pair.map(|l| basis.lp(l)))
        }
        PropertyValue::PaddingBlock(pair) => PropertyValue::PaddingBlock(pair.map(|l| basis.lp(l))),
        // ── margin ────────────────────────────────────────────────────────
        PropertyValue::MarginTop(v) => PropertyValue::MarginTop(basis.margin_lpa(v)),
        PropertyValue::MarginRight(v) => PropertyValue::MarginRight(basis.margin_lpa(v)),
        PropertyValue::MarginBottom(v) => PropertyValue::MarginBottom(basis.margin_lpa(v)),
        PropertyValue::MarginLeft(v) => PropertyValue::MarginLeft(basis.margin_lpa(v)),
        // Inherit markers are normally resolved in phase 2. Keep them
        // panic-free if an internal caller bypasses that phase.
        v @ (PropertyValue::MarginTopInherit
        | PropertyValue::MarginRightInherit
        | PropertyValue::MarginBottomInherit
        | PropertyValue::MarginLeftInherit
        | PropertyValue::MarginInherit) => v,
        PropertyValue::Margin(sides) => PropertyValue::Margin(sides.map(|l| basis.margin_lpa(l))),
        PropertyValue::MarginInline(pair) => {
            PropertyValue::MarginInline(pair.map(|l| basis.margin_lpa(l)))
        }
        PropertyValue::MarginBlock(pair) => {
            PropertyValue::MarginBlock(pair.map(|l| basis.margin_lpa(l)))
        }
        // ── border-*-width (absolutized **and** style-gated) ──────────────
        // A `none`/`hidden` style computes the width to 0, hence `border_styles`.
        PropertyValue::BorderTopWidth(w) => {
            PropertyValue::BorderTopWidth(basis.border_width(w, border_styles.top))
        }
        PropertyValue::BorderRightWidth(w) => {
            PropertyValue::BorderRightWidth(basis.border_width(w, border_styles.right))
        }
        PropertyValue::BorderBottomWidth(w) => {
            PropertyValue::BorderBottomWidth(basis.border_width(w, border_styles.bottom))
        }
        PropertyValue::BorderLeftWidth(w) => {
            PropertyValue::BorderLeftWidth(basis.border_width(w, border_styles.left))
        }
        // Each side gates on the style it carries itself, not on `border_styles`.
        PropertyValue::Border(sides) => PropertyValue::Border(sides.map(|b| basis.border(b))),
        PropertyValue::BorderStyle(sides) => PropertyValue::BorderStyle(sides),
        PropertyValue::BorderWidth(sides) => {
            PropertyValue::BorderWidth(Sides {
                top: basis.border_width(sides.top, border_styles.top),
                right: basis.border_width(sides.right, border_styles.right),
                bottom: basis.border_width(sides.bottom, border_styles.bottom),
                left: basis.border_width(sides.left, border_styles.left),
            })
        }
        PropertyValue::BorderColor(sides) => PropertyValue::BorderColor(sides),
        // ── border-radius / box-shadow / outline ─────────────────────────
        PropertyValue::BorderRadius(v) => PropertyValue::BorderRadius(basis.border_radius_value(v)),
        PropertyValue::BorderRadiusInherit => PropertyValue::BorderRadiusInherit,
        PropertyValue::BorderRadiusTopLeft(v) => PropertyValue::BorderRadiusTopLeft(Length::Px(
            resolve_length(v, font_size, own_line_height, ctx).px(),
        )),
        PropertyValue::BorderRadiusTopRight(v) => PropertyValue::BorderRadiusTopRight(Length::Px(
            resolve_length(v, font_size, own_line_height, ctx).px(),
        )),
        PropertyValue::BorderRadiusBottomRight(v) => {
            PropertyValue::BorderRadiusBottomRight(Length::Px(
                resolve_length(v, font_size, own_line_height, ctx).px(),
            ))
        }
        PropertyValue::BorderRadiusBottomLeft(v) => PropertyValue::BorderRadiusBottomLeft(Length::Px(
            resolve_length(v, font_size, own_line_height, ctx).px(),
        )),
        PropertyValue::BoxShadow(items) => PropertyValue::BoxShadow(if items.is_empty() {
            items
        } else {
            Arc::new(
                items
                    .iter()
                    .map(|item| basis.box_shadow_item(*item))
                    .collect(),
            )
        }),
        PropertyValue::Outline(v) => PropertyValue::Outline(basis.outline_value(v)),
        PropertyValue::OutlineWidth(v) => {
            PropertyValue::OutlineWidth(basis.outline_width(v, outline_style))
        }
        PropertyValue::OutlineOffset(v) => {
            PropertyValue::OutlineOffset(Length::Px(
                resolve_length(v, font_size, own_line_height, ctx).px(),
            ))
        }
        // ── width / height / max-* / min-* ──────────────────────────────
        PropertyValue::Width(v) => PropertyValue::Width(basis.lpa(v)),
        PropertyValue::Height(v) => PropertyValue::Height(basis.lpa(v)),
        PropertyValue::MaxWidth(v) => PropertyValue::MaxWidth(basis.lpa(v)),
        PropertyValue::MaxHeight(v) => PropertyValue::MaxHeight(basis.lpa(v)),
        PropertyValue::MinWidth(v) => PropertyValue::MinWidth(basis.lpa(v)),
        PropertyValue::MinHeight(v) => PropertyValue::MinHeight(basis.lpa(v)),
        PropertyValue::MinBlockSize(v) => PropertyValue::MinBlockSize(basis.lpa(v)),
        PropertyValue::Top(v) => PropertyValue::Top(basis.lpa(v)),
        PropertyValue::Right(v) => PropertyValue::Right(basis.lpa(v)),
        PropertyValue::Bottom(v) => PropertyValue::Bottom(basis.lpa(v)),
        PropertyValue::Left(v) => PropertyValue::Left(basis.lpa(v)),
        // ── background-size / background-position ───────────────────────
        PropertyValue::BackgroundSize(v) => PropertyValue::BackgroundSize(basis.background_size(v)),
        PropertyValue::BackgroundPosition(v) => {
            PropertyValue::BackgroundPosition(basis.css_position(v))
        }
        PropertyValue::Background(shorthand) => PropertyValue::Background(BackgroundShorthand {
            image: crate::resolve::resolve_background_image(
                shorthand.image,
                font_size,
                own_line_height,
                ctx,
            ),
            position: basis.css_position(shorthand.position),
            size: basis.background_size(shorthand.size),
            ..shorthand
        }),
        // ── text-decoration-thickness / text-decoration-inset ────────────
        PropertyValue::TextDecorationThickness(t) => {
            PropertyValue::TextDecorationThickness(match t {
                TextDecorationThickness::Auto | TextDecorationThickness::FromFont => t,
                TextDecorationThickness::Length(l) => {
                    TextDecorationThickness::Length(Length::Px(
                        resolve_length(l, font_size, own_line_height, ctx).px(),
                    ))
                }
            })
        }
        PropertyValue::TextDecorationInset(inset) => {
            PropertyValue::TextDecorationInset(match inset {
                TextDecorationInset::Auto => TextDecorationInset::Auto,
                TextDecorationInset::Lengths { start, end } => {
                    TextDecorationInset::Lengths {
                        start: Length::Px(resolve_length(start, font_size, own_line_height, ctx).px()),
                        end: Length::Px(resolve_length(end, font_size, own_line_height, ctx).px()),
                    }
                }
            })
        }
        PropertyValue::TextUnderlineOffset(value) => {
            PropertyValue::TextUnderlineOffset(match value {
                TextUnderlineOffset::Auto => TextUnderlineOffset::Auto,
                // Percentages inherit as relative values.
                TextUnderlineOffset::Length(Length::Percent(percent)) => {
                    TextUnderlineOffset::Length(Length::Percent(percent))
                }
                TextUnderlineOffset::Length(length) => TextUnderlineOffset::Length(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
                TextUnderlineOffset::Calc(calc) => match resolve_text_indent_calc(calc, font_size) {
                    ComputedTextIndent::Px(px) => {
                        TextUnderlineOffset::Length(Length::Px(px))
                    }
                    ComputedTextIndent::Percent(percent) => {
                        TextUnderlineOffset::Length(Length::Percent(percent))
                    }
                    ComputedTextIndent::Calc(calc) => {
                        TextUnderlineOffset::Calc(LengthPercentageCalc {
                            percent: calc.percent,
                            px: calc.px,
                            em: 0.0,
                        })
                    }
                },
            })
        }
        PropertyValue::TextDecoration(shorthand) => {
            PropertyValue::TextDecoration(TextDecorationShorthand {
                thickness: match shorthand.thickness {
                    TextDecorationThickness::Auto | TextDecorationThickness::FromFont => {
                        shorthand.thickness
                    }
                    TextDecorationThickness::Length(l) => {
                        TextDecorationThickness::Length(Length::Px(
                            resolve_length(l, font_size, own_line_height, ctx).px(),
                        ))
                    }
                },
                ..shorthand
            })
        }
        PropertyValue::Font(shorthand) => PropertyValue::Font(FontShorthand {
            size: match shorthand.size {
                FontShorthandSize::Absolute(length) => FontShorthandSize::Absolute(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
                FontShorthandSize::Relative(relative) => FontShorthandSize::Absolute(Length::Px(
                    resolve_relative_font_size(relative, font_size.px()),
                )),
            },
            line_height: lift_line_height(resolve_line_height(
                shorthand.line_height,
                font_size,
                ctx.root_line_height,
                ctx,
            )),
            ..shorthand
        }),
        // ── object-position ──────────────────────────────────────────────
        PropertyValue::ObjectPosition(v) => PropertyValue::ObjectPosition(basis.css_position(v)),
        // ── opacity ───────────────────────────────────────────────────────
        PropertyValue::Opacity(o) => PropertyValue::Opacity(o.clamp(0.0, 1.0)),
        // ── overflow-x / overflow-y ──────────────────────────────────────────
        // Cross-axis coupling: each axis resolves against the other axis's winner.
        PropertyValue::OverflowX(v) => PropertyValue::OverflowX(
            resolve_overflow(OverflowXY {
                x: v,
                y: overflow_pair.y,
            })
            .x,
        ),
        PropertyValue::OverflowY(v) => PropertyValue::OverflowY(
            resolve_overflow(OverflowXY {
                x: overflow_pair.x,
                y: v,
            })
            .y,
        ),
        PropertyValue::Overflow(pair) => PropertyValue::Overflow(resolve_overflow(pair)),
        // ── writing-mode ─────────────────────────────────────────────────
        PropertyValue::WritingMode(v) => PropertyValue::WritingMode(resolve_writing_mode(v)),
        PropertyValue::RubyPosition(v) => PropertyValue::RubyPosition(v),
        // ── letter-spacing / word-spacing ───────────────────────────────────
        PropertyValue::LetterSpacing(v) => {
            PropertyValue::LetterSpacing(lift_letter_spacing(resolve_letter_spacing(
                v,
                font_size,
                own_line_height,
                ctx,
            )))
        }
        PropertyValue::WordSpacing(v) => {
            PropertyValue::WordSpacing(lift_word_spacing(resolve_word_spacing(
                v,
                font_size,
                own_line_height,
                ctx,
            )))
        }
        // ── tab-size ─────────────────────────────────────────────────────
        PropertyValue::TabSize(v) => {
            PropertyValue::TabSize(lift_tab_size(resolve_tab_size(v, font_size, own_line_height, ctx)))
        }
        // ── border-spacing ─────────────────────────────────────────────────
        PropertyValue::BorderSpacing(v) => {
            PropertyValue::BorderSpacing(lift_border_spacing(resolve_border_spacing(
                v,
                font_size,
                own_line_height,
                ctx,
            )))
        }
        // ── text-shadow ─────────────────────────────────────────────────────
        PropertyValue::TextShadow(items) => PropertyValue::TextShadow(if items.is_empty() {
            items
        } else {
            Arc::new(
                items
                    .iter()
                    .map(|item| basis.text_shadow_item(*item))
                    .collect(),
            )
        }),
        // ── vertical-align ───────────────────────────────────────────────
        PropertyValue::VerticalAlign(v) => {
            PropertyValue::VerticalAlign(resolve_vertical_align(v, font_size, own_line_height, ctx))
        }
        // ── flex-basis ────────────────────────────────────────────────────
        PropertyValue::FlexBasis(v) => PropertyValue::FlexBasis(basis.fb(v)),
        PropertyValue::Flex(f) => PropertyValue::Flex(FlexShorthand {
            grow: f.grow,
            shrink: f.shrink,
            basis: basis.fb(f.basis),
        }),
        // ── row-gap / column-gap ─────────────────────────────────────────
        PropertyValue::RowGap(v) => PropertyValue::RowGap(basis.lpn(v)),
        PropertyValue::ColumnGap(v) => PropertyValue::ColumnGap(basis.lpn(v)),
        PropertyValue::Gap(g) => PropertyValue::Gap(GapShorthand {
            row: basis.lpn(g.row),
            column: basis.lpn(g.column),
        }),
        // ── grid-template-columns / grid-template-rows ──────────────────────
        PropertyValue::GridTemplateColumns(v) => PropertyValue::GridTemplateColumns(basis.gtt(v)),
        PropertyValue::GridTemplateRows(v) => PropertyValue::GridTemplateRows(basis.gtt(v)),
        // ── grid-auto-columns / grid-auto-rows ───────────────────────────────
        PropertyValue::GridAutoColumns(v) => PropertyValue::GridAutoColumns(basis.gatl(&v)),
        PropertyValue::GridAutoRows(v) => PropertyValue::GridAutoRows(basis.gatl(&v)),
        // ── transform ────────────────────────────────────────────────────
        PropertyValue::Transform(items) => {
            if items.is_empty() {
                PropertyValue::Transform(items)
            } else {
                PropertyValue::Transform(Arc::new(
                    items
                        .iter()
                        .map(|f| basis.transform_function(*f))
                        .collect(),
                ))
            }
        }
    }
}
