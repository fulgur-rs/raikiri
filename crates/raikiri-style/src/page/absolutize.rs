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
    Outline, OutlineColor, OutlineStyle, OverflowXY, PropertyValue, Sides, TextDecorationInset,
    TextDecorationShorthand, TextDecorationThickness, TextIndentValue, TextShadowItem,
    TransformFunction, resolve_overflow, resolve_writing_mode,
};
use crate::resolve::{
    ComputedBackgroundSize, ComputedCssPositionOffset, ComputedFlexBasis,
    ComputedGridTemplateTracks, ComputedGridTrackBreadth, ComputedGridTrackList,
    ComputedGridTrackListComponent, ComputedGridTrackSize, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrNormal,
    ResolveContext, lift_border_spacing, lift_length_or_normal, lift_line_height, lift_tab_size,
    resolve_background_size, resolve_border, resolve_border_radius, resolve_border_spacing,
    resolve_box_shadow_item, resolve_css_position, resolve_flex_basis,
    resolve_grid_auto_track_list, resolve_grid_template_tracks, resolve_length,
    resolve_length_or_normal, resolve_length_percentage, resolve_length_percentage_or_auto,
    resolve_length_percentage_or_normal, resolve_line_height, resolve_margin_length_or_auto,
    resolve_outline, resolve_tab_size, resolve_vertical_align,
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
    /// One `text-shadow` item: absolutize the 3 lengths (`offset-x`/
    /// `offset-y`/`blur-radius`), round-tripped back into the specified-layer
    /// `Length::Px` shape (`resolve_length` is the percentage-less resolver —
    /// text-shadow's lengths don't allow `<percentage>`, `TextShadowItem`
    /// doc). `color` carries no length (`TextShadowColor` doc) — passed
    /// through unchanged, same as `Border`'s `color` field above.
    fn text_shadow_item(self, specified: TextShadowItem) -> TextShadowItem {
        TextShadowItem {
            offset_x: Length::Px(
                resolve_length(
                    specified.offset_x,
                    self.font_size,
                    self.own_line_height,
                    self.ctx,
                )
                .px(),
            ),
            offset_y: Length::Px(
                resolve_length(
                    specified.offset_y,
                    self.font_size,
                    self.own_line_height,
                    self.ctx,
                )
                .px(),
            ),
            blur_radius: Length::Px(
                resolve_length(
                    specified.blur_radius,
                    self.font_size,
                    self.own_line_height,
                    self.ctx,
                )
                .px(),
            ),
            color: specified.color,
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
/// Sibling of the element path's [`crate::specified::SpecifiedValues::finalize`]
/// second half (`absolutize_with`). Both funnel into the *same*
/// [`crate::resolve`] functions, so the spec rules (`em` / `rem` basis,
/// percentage pass-through, border style gating) have a single implementation
/// per rule; only the integration logic differs, because the page path carries a
/// `PropertyValue` bag instead of a typed struct.
///
/// # What this function does not resolve, and why that is fine
///
/// `<percentage>` on the box properties stays a percentage — it is a
/// used-value-layer input (CSS Values 4 §5.5.1), not a phase-3 concern. The
/// spec citation lives in [`PageCascadeResult::declarations`](super::PageCascadeResult::declarations) (canonical).
///
/// `text-align: match-parent` is **not** handled here either, but for a
/// different reason than it used to be: it is now fully
/// resolved by **phase 2**
/// ([`crate::cascade::resolve_against_inherited`], which runs before this
/// function) — inherited-value dependence is phase 2's shape, not phase 3's,
/// so by the time a value reaches this function `TextAlign::MatchParent`
/// should never appear. It stays in the pass-through arm below (alongside
/// `Color` / `Display` / …) simply because, once resolved, `text-align`'s
/// computed value carries no length for phase 3 to touch — same as before,
/// just for a different underlying reason. `direction` joins the same arm as
/// a new, ordinary computed-equivalent keyword (CSS Writing Modes 4 §2.1,
/// computed value = specified value, no phase-2 or phase-3 work at all).
///
/// # No wildcard arm
///
/// The match is exhaustive without `_`, like its two siblings
/// ([`crate::cascade::apply_value`] / [`crate::cascade::resolve_against_inherited`]).
/// A new [`PropertyValue`] variant that carries a length must be classified
/// here explicitly; a catch-all would let it reach the public `declarations`
/// map as a specified value — the exact shape of a regression this crate has
/// already hit once. (What this guard does *not* catch is a new **payload**
/// case inside an existing variant — gap (a).)
///
/// # The `value` parameter is phase-2 output, enforced by its type
///
/// `value` is a
/// [`crate::cascade::ResolvedAgainstInherited`]
/// rather than a raw [`PropertyValue`] — see that type's doc for what this
/// does and does not guarantee ("narrowed, not closed").
///
/// # `own_line_height`
///
/// The page context's own `lh` basis — [`page_context_line_height_basis`](super::cascade::page_context_line_height_basis)'s
/// output, threaded alongside `font_size` for the same reason: `1lh` in
/// `padding`/`margin`/`border-*-width` needs this context's *own* resolved
/// line-height (not the root's — that is `ctx.root_line_height`, used only
/// for `rlh`).
///
/// # `overflow_pair`
///
/// [`page_context_overflow_pair`](super::cascade::page_context_overflow_pair)'s output — the page context's raw
/// `overflow-x`/`overflow-y` winners, threaded in for the same reason
/// `border_styles` is: CSS Overflow 3 §3.1's cross-axis coupling
/// ([`resolve_overflow`]) needs *both* axes at once, and this function
/// otherwise only sees one [`PropertyValue`] winner at a time.
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
        // `font-size` is phase 2's output (`Length::Px`); re-absolutizing it
        // here would be a second application against the *wrong* basis (its
        // own value instead of the inheritance parent's).
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
        // `text-decoration-line`/`-style`/`-color` carry no length and
        // computed value = specified keyword(s)/color (see
        // `TextDecorationLine`/`TextDecorationStyle`/`TextDecorationColor`
        // docs) — nothing for phase 3 to absolutize. The `text-decoration`
        // shorthand itself does NOT join this bucket: its `-thickness`
        // component carries `<length-percentage>` (ED §2.4.1), so it gets
        // its own fall-through transform arm below (same "structurally
        // unreachable, pinned directly" shape as `Margin`/`Border`/
        // `Background` above).
        | PropertyValue::TextDecorationLine(_)
        | PropertyValue::TextDecorationStyle(_)
        | PropertyValue::TextDecorationColor(_)
        // `text-decoration-skip-ink`/`-skip-spaces`/`text-emphasis-position`/
        // `text-underline-position` carry no length and computed value =
        // specified keyword(s) (see each type's doc) — nothing for phase 3
        // to absolutize, same bucket shape as the 3 `text-decoration`
        // longhands above.
        | PropertyValue::TextDecorationSkipInk(_)
        | PropertyValue::TextDecorationSkipSpaces(_)
        | PropertyValue::TextEmphasisPosition(_)
        | PropertyValue::TextUnderlinePosition(_)
        // `font-style` carries no length at this crate's scope
        // (`normal`/`italic`/`oblique` implemented, `oblique`'s `<angle>`
        // argument is not — see `FontStyle`'s doc) and computed value =
        // specified keyword — nothing for phase 3 to absolutize.
        | PropertyValue::FontStyle(_)
        // `font-variant-caps` carries no length either (see
        // `FontVariantCaps`'s doc) and computed value = specified keyword —
        // nothing for phase 3 to absolutize.
        | PropertyValue::FontVariantCaps(_)
        // `text-transform` carries no length (see `TextTransform`'s doc)
        // and computed value = specified keyword — nothing for phase 3 to
        // absolutize.
        | PropertyValue::TextTransform(_)
        // `visibility` carries no length (see `Visibility`'s doc) and
        // computed value = specified keyword — nothing for phase 3 to
        // absolutize.
        | PropertyValue::Visibility(_)
        // `z-index` carries no length (see `ZIndexValue`'s doc) and computed
        // value = specified value — nothing for phase 3 to absolutize.
        | PropertyValue::ZIndex(_)
        // `word-break` (CSS Text 3 §5.1) carries no length either and
        // computed value = specified keyword (see `WordBreak`'s doc) —
        // nothing for phase 3 to absolutize.
        | PropertyValue::WordBreak(_)
        // `overflow-wrap` (legacy alias `word-wrap`, CSS Text 3 §5.4)
        // carries no length either (see `OverflowWrap`'s doc) — same as
        // `WordBreak` above.
        | PropertyValue::OverflowWrap(_)
        // `break-before`/`break-after` (CSS Fragmentation Module Level 3
        // §3.1, legacy shorthand `page-break-before`/`page-break-after`
        // included) carry no length either (see `BreakBetween`'s doc) —
        // nothing for phase 3 to absolutize.
        | PropertyValue::BreakBefore(_)
        | PropertyValue::BreakAfter(_)
        // `break-inside` (CSS Fragmentation Module Level 3 §3.2, legacy
        // shorthand `page-break-inside` included) carries no length
        // either (see `BreakInside`'s doc) — same as `BreakBefore`/
        // `BreakAfter` above.
        | PropertyValue::BreakInside(_)
        // `float`/`clear` (CSS2 §9.5.1/§9.5.2) carry no length either. The
        // §9.7 `display` recomputation `float` drives on the element path
        // (`crate::property::resolve_display_for_float`) is **not**
        // applied here — a page box is not an element in a visual
        // formatting context, so §9.7's clause has no subject in this
        // path; `Float`/`Clear` are opaque pass-through values here, the
        // same treatment `ZIndex` gets (see that arm's doc).
        | PropertyValue::Float(_)
        | PropertyValue::Clear(_)
        // `white-space` (CSS Text 3 §3) carries no length either and
        // computed value = specified keyword (see `WhiteSpace`'s doc) —
        // nothing for phase 3 to absolutize.
        | PropertyValue::WhiteSpace(_)
        // `text-wrap` (CSS Text 4 §5 subset) carries no length either —
        // same as `WhiteSpace` above.
        | PropertyValue::TextWrap(_)
        // `hyphens` (CSS Text 3 §5.3) carries no length either and computed
        // value = specified keyword (see `Hyphens`'s doc) — same as
        // `WhiteSpace` above.
        | PropertyValue::Hyphens(_)
        | PropertyValue::LineBreak(_)
        | PropertyValue::TextJustify(_)
        | PropertyValue::TextAlignAll(_)
        | PropertyValue::TextAlignLast(_)
        | PropertyValue::TextCombineUpright(_)
        | PropertyValue::TextOrientation(_)
        | PropertyValue::UnicodeBidi(_)
        // `page` (CSS Paged Media 3 §8.1) carries no length and computed
        // value = specified value — nothing for phase 3 to absolutize.
        | PropertyValue::Page(_)
        // `column-count` carries only a keyword/integer and has no page-side
        // length to absolutize. `column-width:auto` is likewise already
        // computed-equivalent; the length-bearing forms have dedicated arms
        // below so relative units do not leak into the page declaration bag.
        | PropertyValue::ColumnCount(_)
        | PropertyValue::ColumnWidth(ColumnWidthValue::Auto)
        // `flex-direction`/`flex-wrap` (CSS Flexible Box Layout Module
        // Level 1 §5.1/§5.2) carry no length and computed value = specified
        // keyword — nothing for phase 3 to absolutize.
        | PropertyValue::FlexDirection(_)
        | PropertyValue::FlexWrap(_)
        // `flex-grow`/`flex-shrink` (§7.2.1/§7.2.2) carry a bare
        // `<number>`, not a length — nothing for phase 3 to absolutize
        // (unlike `FlexBasis` below, which does carry a
        // `<length-percentage>` and gets its own transform arm).
        | PropertyValue::FlexGrow(_)
        | PropertyValue::FlexShrink(_)
        // `flex-flow` shorthand — neither component carries a length (see
        // `place-content` below for the same shape); structurally
        // unreachable here regardless (`expand_shorthand_into` expands it
        // before this function runs).
        | PropertyValue::FlexFlow(_)
        // `order` (§4.2) carries a bare `<integer>`, not a length —
        // nothing for phase 3 to absolutize.
        | PropertyValue::Order(_)
        // `justify-content`/`align-content` (CSS Box Alignment Module Level
        // 3 §5.1) / `align-items` (§7.2) / `align-self` (§6.2) carry no
        // length — same shape as `WordBreak` above.
        | PropertyValue::JustifyContent(_)
        | PropertyValue::AlignContent(_)
        | PropertyValue::AlignItems(_)
        | PropertyValue::AlignSelf(_)
        // `place-content` shorthand joins the same bucket as identity
        // pass-through — neither of its 2 components carries a length (see
        // `TextDecoration` above for the same shorthand-with-no-length-
        // components shape); it is structurally unreachable here regardless
        // (`expand_shorthand_into` expands it before this function runs).
        | PropertyValue::PlaceContent(_)
        // `quotes` (CSS Content 3 §2.4.1) carries no length and computed
        // value = specified value (`ComputedValues::quotes` doc) — nothing
        // for phase 3 to absolutize.
        | PropertyValue::Quotes(_)
        // `grid-template-areas` (CSS Grid Layout Module Level 1 §7.3) —
        // computed value is the keyword `none` or the authored string list
        // itself (`GridTemplateAreasValue` doc), not a parsed/absolutized
        // form — nothing for phase 3 to absolutize, same shape as
        // `GridTemplateAreasValue`'s specified/computed-equivalent layering
        // (unlike `GridTemplateColumns`/`GridTemplateRows` below, which do
        // carry `<length-percentage>` in their track sizes and get their
        // own transform arm).
        | PropertyValue::GridTemplateAreas(_)
        // `grid-auto-flow` (§7.7) carries no length and computed value =
        // specified keyword(s) — same shape as `WordBreak` above.
        | PropertyValue::GridAutoFlow(_)
        // `grid-row-start`/`-end`/`grid-column-start`/`-end` (§8.3) carry no
        // length (`GridLineValue` doc) — same shape as `WordBreak` above.
        | PropertyValue::GridRowStart(_)
        | PropertyValue::GridRowEnd(_)
        | PropertyValue::GridColumnStart(_)
        | PropertyValue::GridColumnEnd(_)
        // `grid-row`/`grid-column` shorthands (§8.4) join the same bucket as
        // identity pass-through — neither longhand they expand to carries a
        // length either, and they are structurally unreachable here
        // regardless (same `expand_shorthand_into` reasoning as
        // `PlaceContent` above).
        | PropertyValue::GridRow(_)
        | PropertyValue::GridColumn(_)
        // `justify-items` (CSS Box Alignment Module Level 3 §7.1) /
        // `justify-self` (§6.1) carry no length — same shape as
        // `AlignItems`/`AlignSelf` above.
        | PropertyValue::JustifyItems(_)
        | PropertyValue::JustifySelf(_)
        // `place-items`/`place-self` shorthands (§7.3/§6.3) join the same
        // bucket as identity pass-through, same reasoning as
        // `PlaceContent`/`GridRow` above.
        | PropertyValue::PlaceItems(_)
        | PropertyValue::PlaceSelf(_)
        // `orphans`/`widows` (CSS Fragmentation Module Level 3 §3.3) carry a
        // bare positive `<integer>`, not a length, and computed value =
        // specified integer — nothing for phase 3 to absolutize, same shape
        // as `FlexGrow`/`FlexShrink` above.
        | PropertyValue::Orphans(_)
        | PropertyValue::Widows(_)
        // `background-repeat`/`background-attachment`/`background-clip`/
        // `background-origin` (CSS Backgrounds and Borders 3 §2.4/§2.5/§2.7/§2.8)
        // carry no length — computed value = specified keyword(s), same
        // shape as `WordBreak` above. `background-size`/`background-position`
        // do carry `<length-percentage>` and get their own transform arms
        // below (next to `Width`/`Height`).
        | PropertyValue::BackgroundRepeat(_)
        | PropertyValue::BackgroundAttachment(_)
        | PropertyValue::BackgroundClip(_)
        | PropertyValue::BackgroundOrigin(_)

        // `object-fit` (CSS Images Module Level 3 §5.1) carries no length
        // either — keyword-only payload, same shape as `WordBreak` above.
        // `object-position` (§5.2) does carry `<length-percentage>` (reuses
        // `CssPosition`, `background-position`'s type) and gets its own
        // transform arm below, next to `BackgroundPosition`.
        | PropertyValue::ObjectFit(_)
        // `isolation` (CSS Compositing and Blending Level 1 §3.4.2) /
        // `mix-blend-mode` (§3.4.1) — keyword-only payloads, same shape as
        // `ObjectFit` above.
        | PropertyValue::Isolation(_)
        | PropertyValue::MixBlendMode(_)

        // `clip-path` (§5.1) — `None`/`Url(String)`/`GeometryBox(..)` all
        // carry no length payload (`ClipPath` doc's scope note — no
        // `<basic-shape>` support, so no embedded length at all).
        | PropertyValue::ClipPath(_)
        // `filter` (§5) — Computed value is plain "as specified": no
        // absolutization is spec-required at all, so passing every
        // embedded `Length`/`Angle` through untouched (including
        // `drop-shadow()`'s, reused from `TextShadowItem`) is not a gap,
        // just this property's actual computed-value definition
        // (`FilterFunction` doc's "Range restriction is reject, not clamp"
        // section establishes the same "as specified" fact for a different
        // purpose).
        | PropertyValue::Filter(_)
        // `table-layout` (CSS Tables 3 §4) / `border-collapse` (CSS Tables
        // 3 §6) / `caption-side` (§7) / `empty-cells` (§8) carry no length
        // and computed value = specified keyword
        // (`TableLayoutValue`/`BorderCollapseValue` docs) — nothing for
        // phase 3 to absolutize. A page box is not a table wrapper box, so
        // none of these properties has layout meaning in this path; all are
        // opaque pass-through values here, the same treatment `ZIndex` gets
        // (see that arm's doc). (`border-spacing` §6.1 *does* carry
        // `<length>` and gets its own transform arm next to `TabSize`.)
        | PropertyValue::TableLayout(_)
        | PropertyValue::BorderCollapse(_)
        | PropertyValue::CaptionSide(_)
        | PropertyValue::EmptyCells(_)) => v,
        // `column-width` and the width component of `columns` are
        // length-bearing. Resolve relative units against the page context,
        // matching the other single-length properties above.
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
        // `None`/`Url(String)` are computed-equivalent. `Gradient(..)`'s
        // `<length-percentage>` payloads (`GradientColorStop::position`,
        // `RadialSize::Circle`/`Ellipse`, `RadialGradient`/`ConicGradient`'s
        // `CssPosition`) are partially absolutized: font-relative lengths
        // (`em`/`rem`/`ex`/`ch`/`ic`/`pt`/`cm`/`mm`/`q`/`in`/`pc`/`lh`/`rlh`)
        // resolve against this page context's own `font-size`/
        // `own_line_height` basis, while `<percentage>` stays symbolic for
        // paint-time box-size resolution — same split as
        // `background-position`/`object-position` (`resolve_css_position`/
        // `resolve_length_percentage`), see `resolve_background_image` doc.
        PropertyValue::BackgroundImage(img) => PropertyValue::BackgroundImage(
            crate::resolve::resolve_background_image(img, font_size, own_line_height, ctx),
        ),
        PropertyValue::MaskImage(img) => PropertyValue::MaskImage(
            crate::resolve::resolve_background_image(img, font_size, own_line_height, ctx),
        ),
        // ── font-size: larger / smaller ──────────────────────────────────
        // ⚠️ **structurally unreachable through `cascade_page`, not a "safety
        // net"** — step 3 (phase 2) in `cascade_page` maps *every* winner
        // through `resolve_against_inherited` before this function ever runs,
        // and that function's `FontSizeRelative` arm always converges to
        // `PropertyValue::FontSize(Length::Px(_))`. There is no second entry
        // point that could skip step 3 (unlike the shorthand fall-throughs
        // below, which *are* reachable via a direct internal call).
        //
        // The arm still exists — not folded into the `Color`/`FontSize`/…
        // bucket above, and not `unreachable!` — for the same two reasons the
        // shorthand fall-throughs keep real arms: the crate keeps the cascade
        // panic-free (a deliberate design policy, see the shorthand
        // fall-through note in `crate::cascade::apply_value`'s doc), and this
        // function's `pub(crate)` visibility means test code *can* call it
        // directly with an
        // unresolved `FontSizeRelative`, bypassing step 3 (as
        // `absolutize_in_page_context_font_size_relative_safety_net` does).
        //
        // The basis is deliberately `font_size` (this context's own,
        // phase-2-resolved value) rather than the inheritance parent's — this
        // function has no parent basis available, and the arm is bug-only
        // regardless, so spec correctness here is not a design goal. What
        // matters is: no panic, and convergence to the same `FontSize(Px(_))`
        // shape the real (phase 2) path produces, so a caller reading
        // `declarations` never observes an unresolved `FontSizeRelative`.
        PropertyValue::FontSizeRelative(rel) => {
            PropertyValue::FontSize(Length::Px(resolve_relative_font_size(rel, font_size.px())))
        }
        // ── line-height ───────────────────────────────────────────────────
        // CSS Inline 3 §5.1 <https://www.w3.org/TR/css-inline-3/#propdef-line-height>:
        // `<percentage>` is "computed relative to 1em" of the declaring
        // context. `normal` / `<number>` survive as keywords by spec.
        //
        // `lh`/`rlh` used *within* this declaration's own value (self-reference)
        // use `ctx.root_line_height` — the page context's
        // CSS Values 4 §6.1.1 self-reference "parent" is the root element
        // (`page_context_line_height_basis` doc), the same source
        // `ctx.root_line_height` already carries.
        PropertyValue::LineHeight(lh) => PropertyValue::LineHeight(lift_line_height(
            resolve_line_height(lh, font_size, ctx.root_line_height, ctx),
        )),
        // ── text-indent ───────────────────────────────────────────────────
        // CSS Text 3 §8.1 — same `<length-percentage>` absolutization shape
        // as `padding-top` (`lp` helper above). The fact that this property
        // is *inherited* at the element cascade layer doesn't change this
        // function's job: `value` already arrived through
        // `resolve_against_inherited` (this arm is phase 3, run after phase
        // 2), so by this point it is this page context's own winner (or a
        // value already carried through inheritance) — either way, just a
        // `Length` that needs absolutizing against this context's own
        // `font_size`/`own_line_height` basis, same as every other box
        // property here.
        PropertyValue::TextIndent(v) => {
            PropertyValue::TextIndent(TextIndentValue {
                length: basis.lp(v.length),
                hanging: v.hanging,
                each_line: v.each_line,
            })
        }
        // ── padding ───────────────────────────────────────────────────────
        PropertyValue::PaddingTop(v) => PropertyValue::PaddingTop(basis.lp(v)),
        PropertyValue::PaddingRight(v) => PropertyValue::PaddingRight(basis.lp(v)),
        PropertyValue::PaddingBottom(v) => PropertyValue::PaddingBottom(basis.lp(v)),
        PropertyValue::PaddingLeft(v) => PropertyValue::PaddingLeft(basis.lp(v)),
        // Shorthand fall-through — **unreachable through `cascade_page`**.
        // `crate::rule::expand_shorthand_into` runs at both boundaries that feed
        // this function: the parse exit (`parse_page_declaration_block`, this
        // module's own `@page`-block parser) and the `@page` cascade entry
        // (the candidate loop in `cascade_page` itself — the sibling of
        // `crate::cascade`'s `collect_cascaded`). The
        // entry-side expansion is what covers the post-parse mutation path
        // through the `pub` field `PageRule::declarations`.
        //
        // ⚠️ **これは "safety" net ではない — 到達したら既に bug である**
        // (element 側の同種の framing 訂正と同旨)。到達した
        // winner は `PropertyKey::Margin` 等の独立 key に park したまま
        // absolutize され、well-formed に見える値のまま `MarginTop` を読む
        // consumer から黙って消える — まさにこの fall-through が引き起こす
        // 典型的な failure mode である。degraded ではなく deterministic に
        // CSS Cascading
        // L4 §3 <https://www.w3.org/TR/css-cascade-4/#shorthand> 違反であり、
        // 本 arm はそれを穏当に見せない。
        //
        // The arms are kept rather than folded into `unreachable!` for the same
        // reason `cascade::apply_value` keeps its shorthand arms: the guarantee
        // above is only partly compile-time enforced — the carve-outs are in the
        // expansion `match`'s own doc — and the crate
        // keeps the cascade panic-free as a deliberate design policy. Behaviour
        // is pinned directly
        // by `tests::absolutize_in_page_context_shorthand_fall_throughs`.
        PropertyValue::Padding(sides) => PropertyValue::Padding(sides.map(|l| basis.lp(l))),
        // `padding-inline`/`padding-block` shorthand fall-through — same
        // shape and same unreachability rationale as `Padding` above
        // (`crate::property::PropertyValue::PaddingInline` doc covers the
        // physical-mapping choice). Pinned directly by
        // `tests::absolutize_in_page_context_logical_shorthand_fall_throughs`.
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
        // Shorthand fall-through (see `Padding` above).
        PropertyValue::Margin(sides) => PropertyValue::Margin(sides.map(|l| basis.margin_lpa(l))),
        // `margin-inline`/`margin-block` shorthand fall-through — same shape
        // as `PaddingInline`/`PaddingBlock` above.
        PropertyValue::MarginInline(pair) => {
            PropertyValue::MarginInline(pair.map(|l| basis.margin_lpa(l)))
        }
        PropertyValue::MarginBlock(pair) => {
            PropertyValue::MarginBlock(pair.map(|l| basis.margin_lpa(l)))
        }
        // ── border-*-width (absolutized **and** style-gated) ──────────────
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
        // Shorthand fall-through (see `Padding` above). Each side gates on the
        // style it carries itself, which is where a `border` shorthand's style
        // lives.
        PropertyValue::Border(sides) => PropertyValue::Border(sides.map(|b| basis.border(b))),
        // `border-style` / `border-width` / `border-color` shorthand
        // fall-throughs (same unreachability rationale — rule.rs expands
        // them first). Styles and colors carry no lengths (passthrough);
        // widths absolutize against the companion side styles, exactly
        // like the `BorderTopWidth` longhand arms above.
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
        // CSS Backgrounds and Borders 3 §5/§6.1 and CSS UI 3 §4: all
        // length components are computed against this page context's own
        // font-size/line-height. Percentages are intentionally outside this
        // task's parser contract, so every surviving component round-trips
        // as `Length::Px` after phase 3.
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
        // CSS Backgrounds and Borders 3 §2.9/§2.6: both carry
        // `<length-percentage>` components, absolutized against this page
        // context's own font-size/line-height (same basis as `Width`/
        // `Height` above).
        PropertyValue::BackgroundSize(v) => PropertyValue::BackgroundSize(basis.background_size(v)),
        PropertyValue::BackgroundPosition(v) => {
            PropertyValue::BackgroundPosition(basis.css_position(v))
        }
        // `background` shorthand fall-through (see `Padding` above for the
        // unreachability rationale) — unreachable in practice
        // (`expand_shorthand_into` expands it before this function ever sees
        // a winner). `position`/`size`/`image` are the length-bearing
        // components this function absolutizes here — same basis as the
        // `BackgroundPosition`/`BackgroundSize`/`BackgroundImage` longhand
        // arms just above (`Flex`/`Gap` below use the same "only the
        // length-bearing fields get transformed" shape). `image`'s
        // `Gradient` payload's font-relative `<length-percentage>` values
        // are absolutized while `<percentage>` stays symbolic (same split as
        // `BackgroundImage` above). Pinned directly by
        // `tests::absolutize_in_page_context_shorthand_fall_throughs`'s
        // `Background` case.
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
        // ED §2.4.1/§2.9.1: both carry `<length-percentage>` components,
        // absolutized against this page context's own font-size/line-height
        // (same basis and same `Length::Px(resolve_length(..).px())` shape
        // as `OutlineOffset` above). `auto`/`from-font` survive as keywords
        // by spec, same as `LineHeight::Normal`.
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
        // CSS Text Decoration 4 §2.8: fixed underline offsets resolve
        // against the declaring page context's font-size; percentages stay
        // relative. Deferred mixed math falls back to `auto`.
        PropertyValue::TextUnderlineOffset(value) => {
            PropertyValue::TextUnderlineOffset(match value {
                LengthOrAuto::Auto => LengthOrAuto::Auto,
                // Percentages inherit as relative values.
                LengthOrAuto::Length(Length::Percent(percent)) => {
                    LengthOrAuto::Length(Length::Percent(percent))
                }
                LengthOrAuto::Length(length) => LengthOrAuto::Length(Length::Px(
                    resolve_length(length, font_size, own_line_height, ctx).px(),
                )),
                LengthOrAuto::Calc(_) => LengthOrAuto::Auto,
            })
        }
        // `text-decoration` shorthand fall-through (see `Padding` above for
        // the unreachability rationale) — unreachable in practice
        // (`expand_shorthand_into` expands it before this function ever sees
        // a winner). `thickness` is the only length-bearing component this
        // function absolutizes here (same basis as the
        // `TextDecorationThickness` longhand arm just above); the other 3
        // pass through via the struct-update `..shorthand`. Pinned directly
        // by `tests::absolutize_in_page_context_shorthand_fall_throughs`'s
        // `TextDecoration` case.
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
        // `font` shorthand fall-through (see `Padding` above for the
        // unreachability rationale) — unreachable in practice
        // (`expand_shorthand_into` expands it before this function ever sees
        // a winner). `size`/`line-height` are the length-bearing components
        // this function absolutizes here — same basis as the `FontSize`/
        // `LineHeight` longhand arms just above (`Flex`/`Gap`/`Background`
        // above use the same "only the length-bearing fields get
        // transformed" shape). Pinned directly by
        // `tests::absolutize_in_page_context_shorthand_fall_throughs`'s
        // `Font` case.
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
        // CSS Images Module Level 3 §5.2: carries `<length-percentage>`
        // components via the reused `CssPosition` type (same shape and same
        // `css_position` helper as `BackgroundPosition` above).
        PropertyValue::ObjectPosition(v) => PropertyValue::ObjectPosition(basis.css_position(v)),
        // ── opacity ───────────────────────────────────────────────────────
        // CSS Color 4 §3.3: "Opacity values outside the range `[0, 1]` are
        // not invalid, and are preserved in specified values, but are
        // clamped to the range `[0, 1]` in computed values." This is the
        // page-context sibling of
        // `crate::specified::SpecifiedValues::absolutize_with`'s opacity
        // clamp — a real phase-3 transform, but not a length
        // absolutization, so it does not need `font_size`/`own_line_height`/
        // `ctx` the way the arms above do.
        PropertyValue::Opacity(o) => PropertyValue::Opacity(o.clamp(0.0, 1.0)),
        // ── overflow-x / overflow-y ──────────────────────────────────────────
        // CSS Overflow 3 §3.1 cross-axis coupling — this axis's own winner
        // (`v`) paired with the *other* axis's winner (`overflow_pair`,
        // `page_context_overflow_pair`'s output), same shape as
        // `border_width` pairing `w` with `border_styles.top` above.
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
        // `overflow` shorthand fall-through (see `Border` above) — unreachable
        // in practice (`expand_shorthand_into` expands it before this
        // function ever sees a winner), not a safety net if it were.
        PropertyValue::Overflow(pair) => PropertyValue::Overflow(resolve_overflow(pair)),
        // ── writing-mode ─────────────────────────────────────────────────
        // CSS Writing Modes 4 §3.2 — same-node-only normalization (no length,
        // no cross-property/parent dependency), applied here rather than in
        // `resolve_against_inherited` for the same reason `overflow-x`/
        // `overflow-y`'s cross-axis coupling is: it does not depend on the
        // inheritance parent, only phase 3 runs it. See `WritingMode` doc's
        // Non-goal section and `resolve_writing_mode` doc for why every
        // non-`horizontal-tb` keyword collapses here.
        PropertyValue::WritingMode(v) => PropertyValue::WritingMode(resolve_writing_mode(v)),
        PropertyValue::RubyPosition(v) => PropertyValue::RubyPosition(v),
        // ── letter-spacing / word-spacing ───────────────────────────────────
        // CSS Text 3 §7.2 / §7.1: `normal | <length>`, absolutized the same
        // way `crate::specified::SpecifiedValues::absolutize_with` does
        // (`resolve_length_or_normal`), then mapped back into the
        // specified-layer `LengthOrNormal` shape (`lift_length_or_normal`)
        // that `PropertyValue` carries — same round-trip as `lp`/`lpa` above,
        // reusing the shared element-path functions directly since neither
        // needs page-context-specific integration logic.
        PropertyValue::LetterSpacing(v) => PropertyValue::LetterSpacing(lift_length_or_normal(
            resolve_length_or_normal(v, font_size, own_line_height, ctx),
        )),
        PropertyValue::WordSpacing(v) => PropertyValue::WordSpacing(lift_length_or_normal(
            resolve_length_or_normal(v, font_size, own_line_height, ctx),
        )),
        // ── tab-size ─────────────────────────────────────────────────────
        // CSS Text Module Level 3 §4.2: `<number [0,∞]> | <length [0,∞]>`,
        // absolutized the same way
        // `crate::specified::SpecifiedValues::absolutize_with` does
        // (`resolve_tab_size`), then mapped back into the specified-layer
        // `TabSize` shape (`lift_tab_size`) that `PropertyValue` carries —
        // same round-trip as `LetterSpacing`/`WordSpacing` above, reusing
        // the shared element-path functions directly since neither needs
        // page-context-specific integration logic.
        PropertyValue::TabSize(v) => {
            PropertyValue::TabSize(lift_tab_size(resolve_tab_size(v, font_size, own_line_height, ctx)))
        }
        // ── border-spacing ─────────────────────────────────────────────────
        // CSS Tables 3 §6.1: `<length>{1,2}`, absolutized the same way
        // `crate::specified::SpecifiedValues::absolutize_with` does
        // (`resolve_border_spacing`), then mapped back into the
        // specified-layer `BorderSpacingValue` shape (`lift_border_spacing`)
        // that `PropertyValue` carries — same round-trip as
        // `LetterSpacing`/`WordSpacing`/`TabSize` above, reusing the shared
        // element-path functions directly since neither needs
        // page-context-specific integration logic.
        PropertyValue::BorderSpacing(v) => {
            PropertyValue::BorderSpacing(lift_border_spacing(resolve_border_spacing(
                v,
                font_size,
                own_line_height,
                ctx,
            )))
        }
        // ── text-shadow ─────────────────────────────────────────────────────
        // CSS Text Decoration Module Level 3 §4: each item's 3 lengths
        // (`offset-x`/`offset-y`/`blur-radius`) are absolutized against this
        // context's own `font_size`/`own_line_height` basis (`text_shadow_item`
        // helper above); `<color>` carries no length and passes through.
        // `none` (empty list) needs no allocation — `items` is reused as-is.
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
        // CSS 2.1 §10.8.1 — the bare keywords are preserved as-is,
        // `<length>` / `<percentage>` absolutized (`<percentage>` is
        // `used_line_height_length` basis with `0px` fallback when
        // `line-height: normal` — `resolve_vertical_align` doc).
        // `resolve_vertical_align` already returns the same
        // `VerticalAlign` shape `PropertyValue::VerticalAlign` carries (no
        // separate `Computed*` type exists for this property — see that
        // function's doc for why — so unlike `lp`/`fb`/`lift_length_or_normal`
        // above there is no round-trip conversion needed here).
        PropertyValue::VerticalAlign(v) => {
            PropertyValue::VerticalAlign(resolve_vertical_align(v, font_size, own_line_height, ctx))
        }
        // ── flex-basis ────────────────────────────────────────────────────
        // CSS Flexible Box Layout Module Level 1 §7.2.3 — `content`/`auto`
        // preserved as keywords, `<length-percentage>` absolutized (`fb`
        // helper above).
        PropertyValue::FlexBasis(v) => PropertyValue::FlexBasis(basis.fb(v)),
        // Shorthand fall-through (see `Padding` above) — unreachable in
        // practice (`expand_shorthand_into` expands it before this function
        // ever sees a winner). `grow`/`shrink` carry no length; `basis`
        // gets the same `fb` treatment as the `FlexBasis` longhand above.
        PropertyValue::Flex(f) => PropertyValue::Flex(FlexShorthand {
            grow: f.grow,
            shrink: f.shrink,
            basis: basis.fb(f.basis),
        }),
        // ── row-gap / column-gap ─────────────────────────────────────────
        // CSS Box Alignment Module Level 3 §8.1 — `normal` preserved as a
        // keyword (`lpn` helper above, unlike `letter-spacing`/
        // `word-spacing`'s `normal → 0` collapse).
        PropertyValue::RowGap(v) => PropertyValue::RowGap(basis.lpn(v)),
        PropertyValue::ColumnGap(v) => PropertyValue::ColumnGap(basis.lpn(v)),
        // Shorthand fall-through (see `Padding` above) — unreachable in
        // practice, same shape as `Flex` above.
        PropertyValue::Gap(g) => PropertyValue::Gap(GapShorthand {
            row: basis.lpn(g.row),
            column: basis.lpn(g.column),
        }),
        // ── grid-template-columns / grid-template-rows ──────────────────────
        // CSS Grid Layout Module Level 1 §7.2 — `none` preserved as a
        // keyword, `<length-percentage>` inside the track list absolutized
        // (`gtt` helper above).
        PropertyValue::GridTemplateColumns(v) => PropertyValue::GridTemplateColumns(basis.gtt(v)),
        PropertyValue::GridTemplateRows(v) => PropertyValue::GridTemplateRows(basis.gtt(v)),
        // ── grid-auto-columns / grid-auto-rows ───────────────────────────────
        // CSS Grid Layout Module Level 1 §7.6 — same track-size
        // absolutization as `GridTemplateColumns` above (`gatl` helper).
        PropertyValue::GridAutoColumns(v) => PropertyValue::GridAutoColumns(basis.gatl(&v)),
        PropertyValue::GridAutoRows(v) => PropertyValue::GridAutoRows(basis.gatl(&v)),
        // ── transform ────────────────────────────────────────────────────
        // CSS Transforms Level 1 §4: Computed value is "as specified, but
        // with lengths made absolute" — `translate()`/`translateX()`/
        // `translateY()`'s `<length-percentage>` slot の length 側だけを
        // `lp` と同じ split で絶対化し percentage 側は `Percent` のまま残す
        // (`resolve_length_percentage` と同じ、`background-position` の
        // `css_position` helper と同型)。`matrix` の 6 `<number>` slot と
        // `rotate`/`skew` 系の `<angle>` slot はそのまま。
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
