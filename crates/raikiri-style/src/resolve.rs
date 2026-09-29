//! Computed-value types and conversion from specified to computed values (absolutization).
//!
//! This module **separates the value layers**. Its absolutization functions take
//! [`Length`] / [`LengthOrAuto`] / [`LineHeight`] / [`Border`] from
//! [`crate::property`] as **input** and produce the `Computed*` types as **output**.
//! Thus the `Computed*` types identify themselves as computed-layer values.
//! [`crate::computed::ComputedValues`] remains the per-node aggregate struct in
//! `computed.rs`; this module contains only value types and conversion functions.
//!
//! **The reverse does not hold: [`Length`] and similar types do not identify
//! themselves as specified-layer values.** They are specified-layer values when
//! passed to this module, but a value's **origin determines its layer**, not its
//! type. The page path ([`crate::page::cascade_page`]) carries a bag of
//! `PropertyValue`s, so it carries **computed values in [`Length`] too**.
//! The canonical explanation is in the [`Length`] documentation (the section
//! explaining that its layer depends on origin, not its type). The documentation
//! of [`crate::page::PageCascadeResult::declarations`] defines what the page path
//! guarantees. This section only summarizes the layer relationship; individual
//! API contracts appear in their own documentation.
//!
//! # Why absolutization is a separate phase
//!
//! CSS Cascade 5 §7.2 "Inheritance"
//! (<https://www.w3.org/TR/css-cascade-5/#inheriting>) states:
//! "The inherited value of a property on an element is the computed value of the
//! property on the element's parent element." Inheritance therefore carries
//! **computed values**: `em` and `rem` must already be absolutized by then.
//!
//! Absolutization must also be **separate from applying cascade winners**.
//! The `font-size` that defines the basis for `padding: 2em` is known only after
//! all winners for that node have been applied. We cannot absolutize while
//! applying winners one by one (property application order is not guaranteed).
//!
//! The signatures of this module's functions provide the inputs needed to honor
//! that constraint: they are pure functions that do not touch the set of cascade
//! winners and **take the font-size basis as an argument**. A signature only
//! guarantees these two facts, however: the function itself does not apply
//! winners, and the caller explicitly supplies the basis.
//!
//! **The types do not enforce ordering.** The argument is just a
//! [`ComputedLength`]. An incorrect implementation could call a function here
//! after each winner, passing the parent's font-size or an intermediate value
//! from before phase 2. This would still type-check. Thus this module enforces
//! the constraint **by convention**, illustrated by the doctest below.
//!
//! **The cascade pipeline does not rely on that convention**: it limits entry
//! into absolutization to [`SpecifiedValues::finalize`] and
//! [`SpecifiedValues::finalize_as_root`], and encapsulates phase 3 in a private
//! function that does not accept `parent_font_size`. We chose not to add
//! `OwnFontSize` / `ParentFontSize` newtypes: each entry point has only two lines
//! where the constraint matters, whereas newtypes would change the signatures
//! of this module's public functions and all their doctests.
//!
//! [`SpecifiedValues::finalize`]: crate::specified::SpecifiedValues::finalize
//! [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
//!
//! # Expected call order: four stages (phases 1 / 2 / 2.5 / 3)
//!
//! Phase 2.5 (absolutizing line-height) was added later. A box property such as
//! `padding: 2lh` needs this node's line-height to be resolved **first** to use
//! `1lh` (for the same reason that font-size is resolved first in phase 2).
//!
//! `parent_line_height_basis` (the **parent element's** resolved used line-height)
//! is also needed in phase 2 for `font-size`'s self-referential `lh` (see the
//! [`resolve_font_size`] documentation). **This does not reverse the phase 2 →
//! 2.5 order**: the basis is not the result of phase 2.5 for **this** node; it is
//! the result of phase 2.5 for the **parent** node, already resolved in a separate
//! recursive call. The tree walk processes parents before children, so the value
//! is available **before** phase 2 of this node. The caller simply computes
//! `parent_line_height_basis` before calling phase 2 for this node (as below and
//! in [`SpecifiedValues::finalize`]).
//!
//! ```
//! use raikiri_style::{
//!     ComputedLength, ComputedLengthPercentage, ComputedLineHeight, ResolveContext,
//!     resolve_font_size, resolve_length_percentage, resolve_line_height,
//!     used_line_height_length,
//! };
//! use raikiri_style::property::{Length, LineHeight};
//!
//! // The parent's computed font-size and line-height were carried by inheritance.
//! // The parent node has already been processed, so both are known before this
//! // node is processed.
//! let parent_font_size = ComputedLength(16.0);
//! let parent_line_height = ComputedLineHeight::Normal;
//! let ctx = ResolveContext::new(ComputedLength(16.0));
//!
//! // Compute the parent's line-height basis (used for self-referential `lh` in
//! // both `font-size` and `line-height`) before this node's phase 2.
//! let parent_line_height_basis = used_line_height_length(parent_line_height, parent_font_size);
//!
//! // Phase 1: stage cascade winners in specified form (in any order).
//! let specified_font_size = Length::Em(1.5);
//! let specified_line_height = LineHeight::Number(1.5);
//! let specified_padding_top = Length::Lh(2.0);
//!
//! // Phase 2: absolutize font-size against the **parent** basis.
//! let font_size = resolve_font_size(
//!     specified_font_size,
//!     parent_font_size,
//!     parent_line_height_basis,
//!     &ctx,
//! );
//! assert_eq!(font_size, ComputedLength(24.0));
//!
//! // Phase 2.5: absolutize line-height. `<number>` uses this node's newly
//! // resolved font-size; self-referential `lh`/`rlh` uses the parent's line-height
//! // (see the `resolve_line_height` documentation). This example uses `<number>`.
//! let line_height =
//!     resolve_line_height(specified_line_height, font_size, parent_line_height_basis, &ctx);
//! assert_eq!(line_height, ComputedLineHeight::Number(1.5));
//!
//! // Phase 3: absolutize the rest against this node's **resolved** font-size and line-height.
//! // `padding: 2lh` uses the own line-height from phase 2.5 (1.5 * 24px = 36px).
//! let own_line_height = used_line_height_length(line_height, font_size);
//! let padding_top =
//!     resolve_length_percentage(specified_padding_top, font_size, own_line_height, &ctx);
//! assert_eq!(padding_top, ComputedLengthPercentage::Px(72.0)); // 2 * 36
//! ```
//!
//! # Why these computed types do or do not use `#[non_exhaustive]`
//!
//! **This is not a crate-wide rule.** Public enums in `property.rs`, including
//! specified-layer [`Length`] / [`LengthOrAuto`] / [`LineHeight`] / [`Border`],
//! use `#[non_exhaustive]` even for sum types (a convention shared by similar
//! enums in this crate). Only the computed types in this module depart from
//! that convention, due to the **explicit trade-off** below, not merely because
//! they are sum types.
//!
//! - [`ComputedLengthPercentage`] / [`ComputedLengthPercentageOrAuto`] /
//!   [`ComputedLineHeight`] / [`ComputedTabSize`] — **not marked** (the trade-off
//!   below requires downstream consumers to match exhaustively).
//! - [`ComputedBorder`] / [`ResolveContext`] — **marked**. Adding a field does
//!   not make downstream matches fail quietly; source compatibility is a gain.
//! - [`ComputedLength`] — **not marked**, to let downstream code construct
//!   `ComputedLength(16.0)` positionally (`#[non_exhaustive]` would prohibit it).
//!
//! This decision does **not** mean the spec fixes the number of variants.
//! CSS Values 4 §5.6.1 "Computation and Combination of Percentage and Dimension
//! Mixes" (<https://www.w3.org/TR/css-values-4/#combine-mixed>) states verbatim:
//!
//! > The computed value of a percentage-dimension mix is defined as
//! > - a computed dimension if the percentage component is zero or is defined
//! >   specifically to compute to a dimension value
//! > - a computed percentage if the dimension component is zero
//! > - a computed calc() expression otherwise
//!
//! The spec thus defines three forms of computed `<length-percentage>`: px,
//! percentage, and **calc()**. Adding css-variables-and-math support will
//! necessarily add a `Calc` variant. We accept the following **explicit trade-off**:
//!
//! - **Benefit**: downstream code can use exhaustive matches now. We can remove
//!   the defensive `_ => length(0.0)` in `raikiri-dom`'s `layout.rs`, letting
//!   type checking catch a class of failures that would otherwise be silent.
//! - **Cost**: adding `Calc` later requires one coordinated breaking change
//!   across raikiri-style / raikiri-dom / raikiri.
//! - **Reason**: downstream code must handle `calc()`. A compile error is safer
//!   than `#[non_exhaustive]` allowing it to silently fall back to 0px.
//!
//! CSS Values 4 §10.11 "Computed Value"
//! (<https://www.w3.org/TR/css-values-4/#calc-computed-value>) also states:
//! "Where percentages are not resolved at computed-value time, they
//! are not resolved in math functions, e.g. `calc(100% - 100% + 1px)` resolves to
//! `calc(0% + 1px)`, not to `1px`." For properties that retain percentages in
//! the computed layer, the calc() form therefore **survives** as a computed value.
//!
//! Note that `#[non_exhaustive]` provides **source compatibility** (downstream
//! matches need no new arm), not immunity from recompilation: changing a
//! dependency naturally recompiles downstream code.
//!
//! [`ComputedLength`] is outside this trade-off; see that type's documentation.

use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::computed::INITIAL_FONT_SIZE_PX;
use crate::property::{
    Angle, AngularColorStop, BackgroundImage, BackgroundSize, Border, BorderColor, BorderRadius,
    BorderSpacingValue, BorderStyle, BoxShadowItem, CalcLengthPercentage, ConicGradient,
    CssPosition, CssPositionOffset, FlexBasisValue, Gradient, GradientColorStop,
    GridInflexibleBreadth, GridRepeatCount, GridTemplateTracks, GridTrackBreadth, GridTrackList,
    GridTrackListComponent, GridTrackRepeat, GridTrackSize, Length, LengthOrAuto, LengthOrNormal,
    LengthPercentageCalc, LetterSpacingValue, LineHeight, LinearGradient, Outline, OutlineColor,
    OutlineStyle, RadialGradient, RadialSize, TabSize, TextDecorationInset,
    TextDecorationThickness, TextIndentLength, TextShadowColor, TextShadowItem, TextShadowLength,
    TextUnderlineOffset, TransformFunction, VerticalAlign, WordSpacingValue,
};

// ---------------------------------------------------------------------------
// Computed-layer value types
// ---------------------------------------------------------------------------

/// Computed `<length>` — an **absolute length in px**.
///
/// Used for computed values of properties such as `font-size` and
/// `border-*-width`, whose grammar does not accept `<percentage>`.
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §6 "Distance Units: the `<length>` type"
///   (<https://www.w3.org/TR/css-values-4/#lengths>): "The computed value of a
///   length (computed length) is the specified length resolved to an absolute
///   length, and its unit is not distinguished: it can be represented by any
///   absolute length unit (but will be serialized using its canonical unit,
///   px)." This type chooses a representation in px.
/// - CSS Fonts 4 §2.5 "Font size: the font-size property"
///   (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>):
///   "Computed value: an absolute length".
///
/// # Why `#[non_exhaustive]` is absent (outside the module doc's trade-off)
///
/// This type is a newtype with one f32 payload; adding `calc()` does not change
/// its representation. `font-size: calc(1em + 2px)` resolves completely to a
/// `<length>` at computed-value time. See CSS Values 4 §6 above (computed
/// lengths resolve to absolute lengths), and CSS Values 4 §10.11 "Computed Value"
/// (<https://www.w3.org/TR/css-values-4/#calc-computed-value>): "The computed
/// value of a math function is its calculation tree simplified, using all the
/// information available at computed value time. (Such as the em to px ratio,
/// how to resolve percentages in some properties, etc.)"
///
/// **§5.6.1 `#combine-mixed` does not support this claim**: that section covers
/// *mixing percentage and dimension components*, whereas `1em + 2px` adds two
/// dimensions.
///
/// This type is also a tuple struct with one `pub f32` field. Downstream code
/// should be able to construct `ComputedLength(16.0)` positionally;
/// `#[non_exhaustive]` would prohibit that. This differs from how the module
/// documentation treats product types.
///
/// ```
/// use raikiri_style::ComputedLength;
///
/// let fs = ComputedLength(16.0);
/// assert_eq!(fs.px(), 16.0);
/// assert_eq!(ComputedLength::ZERO, ComputedLength(0.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedLength(pub f32);

impl ComputedLength {
    /// `0px`, matching the specified initial value for padding / margin / border-width.
    pub const ZERO: Self = Self(0.0);

    /// Return the value in px.
    pub fn px(self) -> f32 {
        self.0
    }
}

/// Computed `text-decoration-inset`: `auto` or two absolute endpoint lengths.
///
/// The specified property accepts font-relative lengths, so the computed layer
/// must not retain [`crate::property::Length`] values that still need a font
/// basis. Paint consumes this type as an inline-axis trim/extension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTextDecorationInset {
    /// Let the decoration implementation use its automatic endpoint behavior.
    Auto,
    /// Start/end endpoint offsets in CSS pixels.
    Lengths {
        /// Inline-start trim (negative values extend the line).
        start: ComputedLength,
        /// Inline-end trim (negative values extend the line).
        end: ComputedLength,
    },
}

/// Computed `text-decoration-thickness`: a keyword or an absolute CSS length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTextDecorationThickness {
    /// `auto`, the initial value.
    Auto,
    /// `from-font`, retaining font-derived decoration thickness behavior.
    FromFont,
    /// An absolute computed length in CSS pixels.
    Length(ComputedLength),
}

/// Computed `text-underline-offset`: `auto`, a fixed CSS-pixel offset, or a
/// percentage of the font size.
///
/// The property is inherited. A length becomes an absolute value at the
/// declaring element and is lifted back to `px` when a child inherits it. A
/// Percentage terms stay relative (CSS Text Decoration 4 §2.8 says the value
/// inherits as a relative value and therefore scales when the font changes).
/// Consumers resolve percentages against the decorating element's own
/// computed font size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTextUnderlineOffset {
    /// Let the user agent choose the underline offset.
    Auto,
    /// Fixed offset from the underline's zero position in CSS pixels.
    Length(ComputedLength),
    /// Percentage of 1em of the element the value is used on.
    Percent(f32),
    /// Mixed percentage and absolute-length `calc()` after `em` resolution.
    Calc(crate::property::CalcLengthPercentage),
}

/// Computed absolute length plus authored `ch` provenance.
///
/// The computed value remains an absolute fallback for consumers that do not
/// have a shaping context. The optional factor lets a font-aware consumer
/// replace that fallback with the selected face's `0` glyph advance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedLengthWithCh {
    /// Style-layer computed fallback in CSS px.
    pub value: ComputedLength,
    /// Authored `ch` multiplier, when the specified value used `ch`.
    pub ch_factor: Option<f32>,
}

/// Computed `<length-percentage>` — px or a percentage.
///
/// Used for properties such as `padding-*` whose grammar accepts
/// `<length-percentage>` and whose percentage basis (containing-block width) is
/// determined at the **used-value layer**. Percentages **remain** in the
/// computed layer.
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §5.5.1 "Computation and Combination of `<percentage>`"
///   (<https://www.w3.org/TR/css-values-4/#combine-percentages>): "Unless
///   otherwise specified (such as in font-size, which computes its
///   `<percentage>` values to `<length>`), the computed value of a percentage is
///   the specified percentage."
/// - CSS Box 3 `padding-top` propdef
///   (<https://www.w3.org/TR/css-box-3/#propdef-padding-top>):
///   "Value: `<length-percentage [0,∞]>`" / "Computed value: a computed
///   `<length-percentage>` value".
/// - CSS Cascade 5 §4.5 "Used Values"
///   (<https://www.w3.org/TR/css-cascade-5/#used>): resolving against the
///   containing-block width belongs to the used-value layer (taffy's job here).
///
/// [`Percent`](Self::Percent) retains the **authored number unchanged**
/// (`50%` → `Percent(50.0)`, with no `/ 100.0`), matching the convention of
/// specified-layer [`Length::Percent`].
///
/// For the choice not to use `#[non_exhaustive]` and its trade-off, see the
/// [module doc](crate::resolve).
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLengthPercentage, ResolveContext, resolve_length_percentage,
/// };
/// use raikiri_style::property::Length;
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // Pass the percentage through without absolutizing it (its basis is set at used-value time).
/// let p = resolve_length_percentage(Length::Percent(50.0), font_size, None, &ctx);
/// assert_eq!(p, ComputedLengthPercentage::Percent(50.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLengthPercentage {
    /// Absolutized length in px.
    Px(f32),
    /// Percentage — retain the authored number (`50%` → `Percent(50.0)`).
    Percent(f32),
}

/// Computed `text-indent` value. Unlike other length-percentage properties,
/// this property needs a mixed calc form to preserve percentage and px terms
/// through computed-value serialization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTextIndent {
    /// Absolute length in CSS px.
    Px(f32),
    /// Unresolved percentage coefficient (`20%` is `20.0`).
    Percent(f32),
    /// Mixed percentage and absolute-length `calc()`.
    Calc(crate::property::CalcLengthPercentage),
}

/// Computed `letter-spacing` value with percentages retained for later use.
///
/// The renderer-facing [`crate::computed::ComputedValues::letter_spacing`] remains
/// an absolute fallback; this value preserves the CSS computed form for style
/// serialization and inheritance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLetterSpacing {
    /// Computed absolute length in CSS px.
    Px(f32),
    /// Unresolved percentage coefficient (`110%` is `110.0`).
    Percent(f32),
    /// Mixed computed percentage and absolute-length terms.
    Calc(CalcLengthPercentage),
}

/// CSS Text 4 `word-spacing` computed values use the same shape as
/// [`ComputedLetterSpacing`]: percentages remain deferred and mixed calcs keep
/// their percentage and absolute-length terms.
pub type ComputedWordSpacing = ComputedLetterSpacing;

/// Computed `<length-percentage>` plus authored `ch` provenance.
///
/// The absolute fallback remains available to consumers that do not have a
/// shaping context; text layout can replace it with the selected face's
/// U+0030 advance when `ch_factor` is present.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedLengthPercentageWithCh {
    /// Style-layer computed fallback.
    pub value: ComputedLengthPercentage,
    /// Authored `ch` multiplier, when the specified value used `ch`.
    pub ch_factor: Option<f32>,
}

/// Computed `<length-percentage> | auto`.
///
/// Used for computed values of `margin-*` / `width` / `height`.
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Box 3 `margin-top` propdef
///   (<https://www.w3.org/TR/css-box-3/#propdef-margin-top>):
///   "Value: `<length-percentage> | auto`" / "Computed value: the keyword auto
///   or a computed `<length-percentage>` value".
/// - CSS Sizing 3 §3.1.1 "Preferred Size Properties: the width and height
///   properties"
///   (<https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>):
///   "Value: auto | `<length-percentage>` | min-content | max-content |
///   fit-content(`<length-percentage>`)" / "Computed value: as specified, with
///   `<length-percentage>` values computed". The latter directly justifies
///   this type: lengths are absolutized and percentages pass through.
///   (`min-content` / `max-content` / `fit-content()` are unsupported by
///   specified-layer [`LengthOrAuto`]: an existing gap outside this module.)
///
/// The meaning of [`Auto`](Self::Auto) depends on the property (distribution
/// of available space for margins, automatic size calculation for width/height).
/// Keep the variant property-agnostic, as with specified-layer
/// [`LengthOrAuto::Auto`].
///
/// For the choice not to use `#[non_exhaustive]` and its trade-off, see the
/// [module doc](crate::resolve).
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLengthPercentageOrAuto, ResolveContext,
///     resolve_length_percentage_or_auto,
/// };
/// use raikiri_style::property::LengthOrAuto;
///
/// let ctx = ResolveContext::initial();
/// let auto = resolve_length_percentage_or_auto(
///     LengthOrAuto::Auto,
///     ComputedLength(16.0),
///     None,
///     &ctx,
/// );
/// assert_eq!(auto, ComputedLengthPercentageOrAuto::Auto);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLengthPercentageOrAuto {
    /// Absolutized length in px.
    Px(f32),
    /// Percentage — retain the authored number (`50%` → `Percent(50.0)`).
    Percent(f32),
    /// A mixed-unit `calc()` retained for used-value resolution.
    Calc(crate::property::CalcLengthPercentage),
    /// The `auto` keyword.
    Auto,
}

/// Computed `column-width`: an absolute length or `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedColumnWidth {
    /// Automatic column width.
    Auto,
    /// Absolute used length in CSS pixels.
    Px(f32),
}

/// Offset for one axis of a computed `<position>` (the computed counterpart
/// of [`crate::property::CssPositionOffset`]). Edge information (`Start`/`End`)
/// is retained. Converting to a final pixel position (`100% - offset` for `End`)
/// requires the size of the background-positioning area and remains a used-value
/// responsibility (see [`crate::property::CssPositionOffset`]).
///
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ComputedCssPositionOffset {
    /// Offset from the start edge (`left`/`top`).
    Start(ComputedLengthPercentage),
    /// Offset from the end edge (`right`/`bottom`).
    End(ComputedLengthPercentage),
}

/// Computed `<position>` (counterpart of [`crate::property::CssPosition`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ComputedCssPosition {
    /// Horizontal-axis offset.
    pub horizontal: ComputedCssPositionOffset,
    /// Vertical-axis offset.
    pub vertical: ComputedCssPositionOffset,
}

/// Computed `background-size` (the computed counterpart of
/// [`crate::property::BackgroundSize`]).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ComputedBackgroundSize {
    /// `[ <length-percentage [0,∞]> | auto ]{1,2}` — each axis independently.
    Explicit {
        /// Horizontal-axis size.
        width: ComputedLengthPercentageOrAuto,
        /// Vertical-axis size.
        height: ComputedLengthPercentageOrAuto,
    },
    /// The `cover` keyword.
    Cover,
    /// The `contain` keyword.
    Contain,
}

/// Computed `flex-basis`.
///
/// CSS Flexible Box Layout Module Level 1 §7.2.3
/// (<https://www.w3.org/TR/css-flexbox-1/#flex-basis-property>): "Computed
/// value: specified keyword or a computed `<length-percentage>` value".
/// `auto` / `content` remain keywords in the computed layer; other values are
/// absolutized to the [`ComputedLengthPercentage`] shape (`Px` / `Percent`).
/// This is the `content`-keyword counterpart of [`ComputedLengthPercentageOrAuto`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedFlexBasis {
    /// Absolutized length in px.
    Px(f32),
    /// Percentage — retain the authored number.
    Percent(f32),
    /// The `auto` keyword.
    Auto,
    /// The `content` keyword (see the distinction from `auto` in the
    /// [`crate::property::FlexBasisValue`] documentation).
    Content,
    /// The `min-content` keyword remains distinct in the computed layer. The taffy bridge
    /// approximates it as `auto` (see `bridge_flex` in `crates/raikiri-dom/src/layout.rs`).
    MinContent,
    /// The `max-content` keyword — same approximation.
    MaxContent,
    /// The bare `fit-content` keyword — same approximation.
    FitContent,
}

/// Computed `row-gap` / `column-gap`.
///
/// CSS Box Alignment Module Level 3 §8.1 propdef `row-gap`/`column-gap`
/// (<https://www.w3.org/TR/css-align-3/#propdef-row-gap>): "Computed value:
/// specified keyword, else a computed `<length-percentage>` value". Unlike
/// `normal` for `letter-spacing`/`word-spacing`, which resolves to
/// [`ComputedLength`] (see [`resolve_length_or_normal`]), gap's `normal` remains
/// a keyword in the computed layer. The spec says "specified keyword" here,
/// not "Computes to: normal"; the type preserves that distinction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLengthPercentageOrNormal {
    /// Absolutized length in px.
    Px(f32),
    /// Percentage — retain the authored number.
    Percent(f32),
    /// The `normal` keyword — the specified initial value.
    Normal,
}

/// Computed `<track-breadth>` / `<inflexible-breadth>` (CSS Grid Layout
/// Module Level 1 §7.2.1).
///
/// Collapse [`crate::property::GridTrackBreadth`] and
/// [`crate::property::GridInflexibleBreadth`] into **one computed type**.
/// Their separation in the specified layer only enforces the parse-time
/// constraint that `minmax()` rejects `<flex>` (`fr`) on its minimum side
/// (see the "fixed-size constraint" in [`GridTrackSize`]). After parsing,
/// that constraint has already been enforced: a minimum cannot structurally
/// contain [`Self::Flex`]. The `MinMax` arm of `resolve_grid_track_size` always
/// applies [`resolve_grid_inflexible_breadth`] to its minimum, whose input,
/// [`crate::property::GridInflexibleBreadth`], cannot contain `Flex`.
/// There is no benefit in retaining a second computed type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedGridTrackBreadth {
    /// Absolutized length in px.
    Px(f32),
    /// Percentage — retain the authored number.
    Percent(f32),
    /// `<flex>` (`fr`) — retain the authored number (equivalent to `<number>`;
    /// no absolutization needed).
    Flex(f32),
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `auto`.
    Auto,
}

/// Computed `<track-size>` (CSS Grid Layout Module Level 1 §7.2.1).
/// The computed counterpart of [`crate::property::GridTrackSize`]; only
/// `Length`-based payloads change, becoming [`ComputedGridTrackBreadth`] or
/// [`ComputedLengthPercentage`] after absolutization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedGridTrackSize {
    /// Bare `<track-breadth>`.
    Breadth(ComputedGridTrackBreadth),
    /// `minmax( <inflexible-breadth>, <track-breadth> )` — both sides use the same
    /// type, as explained in the [`ComputedGridTrackBreadth`] documentation.
    MinMax(ComputedGridTrackBreadth, ComputedGridTrackBreadth),
    /// `fit-content( <length-percentage> )`.
    FitContent(ComputedLengthPercentage),
}

/// Computed `repeat()` — the computed counterpart of
/// [`crate::property::GridTrackRepeat`]. `count` (`<integer>`/`auto-fill`/
/// `auto-fit`) and `line_names` (`<custom-ident>` only) contain no lengths,
/// so they reuse their specified-layer types
/// ([`crate::property::GridRepeatCount`] / `Vec<Vec<SmolStr>>`).
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedGridTrackRepeat {
    /// Repeat count — reuse the specified-layer type because it has no lengths.
    pub count: GridRepeatCount,
    /// Interleaved line names — unchanged from the specified layer.
    pub line_names: Vec<Vec<SmolStr>>,
    /// Sequence of absolutized track-sizing functions.
    pub tracks: Vec<ComputedGridTrackSize>,
}

/// Computed track-list component — the computed counterpart of
/// [`crate::property::GridTrackListComponent`].
#[derive(Clone, Debug, PartialEq)]
pub enum ComputedGridTrackListComponent {
    /// A single track-sizing function.
    Size(ComputedGridTrackSize),
    /// `repeat()`.
    Repeat(ComputedGridTrackRepeat),
}

/// Computed track list — counterpart of [`crate::property::GridTrackList`].
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedGridTrackList {
    /// Interleaved line names — unchanged from the specified layer.
    pub line_names: Vec<Vec<SmolStr>>,
    /// Sequence of absolutized track-list components.
    pub components: Vec<ComputedGridTrackListComponent>,
}

/// Computed `grid-template-columns` / `grid-template-rows`.
///
/// CSS Grid Layout Module Level 1 §7.2: "Computed value: the keyword `none`
/// or a computed track list" — computed counterpart of
/// [`crate::property::GridTemplateTracks`], with `Arc` for the same reason
/// (see [`crate::property::GridTemplateTracks`]).
#[derive(Clone, Debug, PartialEq)]
pub enum ComputedGridTemplateTracks {
    /// `none` — the specified initial value.
    None,
    /// Absolutized track list.
    List(Arc<ComputedGridTrackList>),
}

/// Shared computed-layer `Arc` for the `grid-auto-columns` / `grid-auto-rows`
/// specified initial value (`auto`). This is the computed counterpart of
/// [`crate::property::initial_grid_auto_track_list`], avoiding per-node allocation.
pub(crate) fn initial_computed_grid_auto_track_list() -> Arc<Vec<ComputedGridTrackSize>> {
    static INITIAL: std::sync::OnceLock<Arc<Vec<ComputedGridTrackSize>>> =
        std::sync::OnceLock::new();
    INITIAL
        .get_or_init(|| {
            Arc::new(vec![ComputedGridTrackSize::Breadth(
                ComputedGridTrackBreadth::Auto,
            )])
        })
        .clone()
}

/// Computed `line-height`.
///
/// # Primary source (§ title + anchor)
///
/// CSS Inline 3 §5.1 "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>) specifies in its
/// property definition table:
///
/// > Value: normal | `<number [0,∞]>` | `<length-percentage [0,∞]>`
/// > Percentages: computed relative to 1em
/// > Computed value: the specified keyword, a number, or a computed `<length>`
/// > value
///
/// Thus **percentages do not exist in the computed layer**: a `<percentage>`
/// is relative to 1em, the element's own computed font-size (CSS Values 4 §6.1.1,
/// `em`, <https://www.w3.org/TR/css-values-4/#em>: "Equal to the computed value
/// of the font-size property of the element on which it is used."). It is
/// therefore absolutized at computed-value time. The absence of a `Percent`
/// variant matches the three forms in the spec's Computed value row one-to-one.
///
/// Retaining [`Number`](Self::Number) as a number in the computed layer is an
/// important spec distinction: children inherit the number and multiply it by
/// **their own** font-size.
///
/// For the choice not to use `#[non_exhaustive]` and its trade-off, see the
/// [module doc](crate::resolve).
///
/// ```
/// use raikiri_style::{ComputedLength, ComputedLineHeight, ResolveContext, resolve_line_height};
/// use raikiri_style::property::{Length, LineHeight};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // Absolutize `<percentage>` against this element's computed font-size.
/// let lh = resolve_line_height(LineHeight::Length(Length::Percent(150.0)), font_size, None, &ctx);
/// assert_eq!(lh, ComputedLineHeight::Length(ComputedLength(30.0)));
///
/// // Pass `<number>` through (a child multiplies it by its own font-size).
/// let n = resolve_line_height(LineHeight::Number(1.5), font_size, None, &ctx);
/// assert_eq!(n, ComputedLineHeight::Number(1.5));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedLineHeight {
    /// The `normal` keyword remains a keyword in the computed layer; paint
    /// resolves it using font metrics.
    Normal,
    /// `<number>` — a unitless multiplier that remains a number when computed.
    Number(f32),
    /// Absolutized `<length>`.
    Length(ComputedLength),
}

/// Computed `tab-size`.
///
/// # Primary source (§ title + anchor)
///
/// CSS Text Module Level 3 §4.2 "Tab Character Size: the tab-size property"
/// (<https://www.w3.org/TR/css-text-3/#tab-size-property>) propdef table:
///
/// > Value: `<number [0,∞]> | <length [0,∞]>`
/// > Initial: 8
/// > Percentages: N/A
/// > Computed value: the specified number or absolute length
///
/// [`Number`](Self::Number) remains a number in the computed layer for the
/// same reason as [`ComputedLineHeight::Number`]. The spec says "A `<number>`
/// represents the measure as a multiple of the advance width of the space
/// character ... of the nearest block container ancestor". Resolution using
/// font metrics is left to a downstream text-layout consumer that this crate
/// does not yet have (see [`crate::property::TabSize`]).
///
/// For the choice not to use `#[non_exhaustive]` and its trade-off, see the
/// [module doc](crate::resolve).
///
/// ```
/// use raikiri_style::{ComputedLength, ComputedTabSize, ResolveContext, resolve_tab_size};
/// use raikiri_style::property::{Length, TabSize};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // Absolutize `<length>` against this element's computed font-size.
/// let ts = resolve_tab_size(TabSize::Length(Length::Em(2.0)), font_size, None, &ctx);
/// assert_eq!(ts, ComputedTabSize::Length(ComputedLength(40.0)));
///
/// // Pass `<number>` through (downstream uses its own font metrics).
/// let n = resolve_tab_size(TabSize::Number(4.0), font_size, None, &ctx);
/// assert_eq!(n, ComputedTabSize::Number(4.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTabSize {
    /// `<number>` — remains a number in the computed layer.
    Number(f32),
    /// Absolutized `<length>`.
    Length(ComputedLength),
}

/// Computed `border-spacing` — two absolute lengths.
///
/// # Primary source (§ title + anchor)
///
/// CSS Tables 3 §6.1 "Separated borders: the border-spacing property"
/// (<https://www.w3.org/TR/css-tables-3/#border-spacing-property>)
/// propdef table:
///
/// > Value: `<length>{1,2}`
/// > Initial: 0
/// > Percentages: N/A
/// > Computed value: two absolute lengths
///
/// Both axes are [`ComputedLength`] values (absolute lengths in px), in the
/// same horizontal / vertical order as specified-layer [`BorderSpacingValue`].
///
/// For the choice not to use `#[non_exhaustive]` and its trade-off, see the
/// [module doc](crate::resolve).
///
/// ```
/// use raikiri_style::{ComputedBorderSpacing, ComputedLength, ResolveContext, resolve_border_spacing};
/// use raikiri_style::property::{BorderSpacingValue, Length};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(20.0);
///
/// // Absolutize each component against this element's computed font-size.
/// let specified = BorderSpacingValue { horizontal: Length::Em(1.0), vertical: Length::Px(5.0) };
/// let computed = resolve_border_spacing(specified, font_size, None, &ctx);
/// assert_eq!(
///     computed,
///     ComputedBorderSpacing { horizontal: ComputedLength(20.0), vertical: ComputedLength(5.0) }
/// );
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedBorderSpacing {
    /// Horizontal (inline-axis) spacing — absolutized.
    pub horizontal: ComputedLength,
    /// Vertical (block-axis) spacing — absolutized.
    pub vertical: ComputedLength,
}

impl ComputedBorderSpacing {
    /// CSSOM serialization of the computed value.
    ///
    /// CSSOM §2.1 "Serializing CSS Values"
    /// (<https://drafts.csswg.org/cssom/#serializing-css-values>):
    /// "If component values can be omitted or replaced with a shorter
    /// representation without changing the meaning of the value,
    /// omit/replace them." Omit the second component when both axes are equal.
    /// WPT `border-spacing-computed.html` checks the shortest serialization:
    /// `"0"` becomes `"0px"`, not `"0px 0px"`.
    pub fn serialized(&self) -> String {
        let h = self.horizontal.px();
        let v = self.vertical.px();
        if h == v {
            format!("{}px", h)
        } else {
            format!("{}px {}px", h, v)
        }
    }
}

/// Computed `border-*` (width / style / color for one side).
///
/// Its shape matches specified-layer [`Border`], except `width` becomes
/// [`ComputedLength`]. `style` and `color` retain their specified keywords in
/// the computed layer (paint resolves [`BorderColor::CurrentColor`] at used-value
/// time).
///
/// # Primary source (§ title + anchor)
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) propdef table:
///
/// - "Value: `<line-width>`" (`<line-width> = <length [0,∞]> | thin | medium |
///   thick`): no `<percentage>` in the grammar, so [`ComputedLength`] suffices.
/// - "Computed value: absolute length, snapped as a border width; **zero if the
///   border style is `none` or `hidden`**": style gating is required in the
///   **computed layer**, and [`resolve_border`] implements it. Snapping to device
///   pixels is not implemented and is outside this module's scope.
///
/// This type uses `#[non_exhaustive]` (see the module doc) so future fields,
/// such as integrated `border-image-*` cascade values, can be added without
/// breaking source compatibility. Specified-layer [`Border`] makes the same
/// choice.
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, SpecifiedValues, resolve_border};
/// use raikiri_style::property::BorderStyle;
///
/// let ctx = ResolveContext::initial();
/// // Initial specified-layer border (the computed initial width is 0px).
/// let initial = SpecifiedValues::initial().border.top;
///
/// // The specified `medium` border-width is 3px. CSS Backgrounds 3 §3.3 says
/// // "The thin, medium, and thick keywords are equivalent to 1px, 3px, and 5px,
/// // respectively." This is normative, not a UA choice. `medium` font-size is
/// // UA-dependent instead (CSS Fonts 4 §2.5.1 "Absolute Size Keyword Mapping").
/// // Since the initial border-style is `none`, its computed width is still 0px.
/// assert_eq!(
///     resolve_border(initial, ComputedLength(20.0), None, &ctx).width(),
///     ComputedLength::ZERO,
/// );
///
/// // With a visible style, the specified width is absolutized unchanged.
/// let mut specified = initial;
/// specified.style = BorderStyle::Solid;
/// let computed = resolve_border(specified, ComputedLength(20.0), None, &ctx);
/// assert_eq!(computed.width(), ComputedLength(3.0));
/// // Pass the specified style and color keywords through.
/// assert_eq!(computed.style(), specified.style);
/// assert_eq!(computed.color, specified.color);
/// // Calls to `computed.style()` and reads of `computed.color` also serve as
/// // a non-vacuous control for the write-path check below.
/// ```
///
/// # Compile-fail check: no public write path
///
/// Both `width` and `style` have `pub(crate)` visibility (see their field docs).
/// A prose-only assertion that this restriction persists could silently regress,
/// for example after a rename. We reuse the technique documented for the
/// `value` field of [`crate::rule::Declaration`].
///
/// `ComputedBorder` already has `#[non_exhaustive]`. Struct literals and `..base`
/// updates **cannot isolate** field visibility: they always fail with `E0639`
/// due to non-exhaustiveness, even if both fields become `pub`. This is the
/// vacuous-check risk noted in the `Declaration` documentation, except that
/// non-exhaustiveness is already present here rather than a future possibility.
/// We therefore use no struct-literal fence.
///
/// Instead, assign directly to a value **owned** by the caller and returned by
/// [`resolve_border`]. That function returns a value, not a reference (see the
/// main doctest above). Thus the confounding `E0594` failure described for
/// `Declaration` cannot occur: `declarations()` returns `&[_]`, so without a
/// `.clone()`, assignment always fails as a write through an immutable reference.
/// Here no borrow intervenes; each assignment's success depends only on the
/// visibility of its own field:
///
/// ```compile_fail
/// use raikiri_style::{ComputedLength, ResolveContext, SpecifiedValues, resolve_border};
///
/// let ctx = ResolveContext::initial();
/// let specified = SpecifiedValues::initial().border.top;
/// let mut computed = resolve_border(specified, ComputedLength(20.0), None, &ctx);
/// computed.width = ComputedLength(999.0);
/// ```
///
/// ```compile_fail
/// use raikiri_style::property::BorderStyle;
/// use raikiri_style::{ComputedLength, ResolveContext, SpecifiedValues, resolve_border};
///
/// let ctx = ResolveContext::initial();
/// let specified = SpecifiedValues::initial().border.top;
/// let mut computed = resolve_border(specified, ComputedLength(20.0), None, &ctx);
/// computed.style = BorderStyle::Solid;
/// ```
///
/// # Non-vacuous control for non-visibility parts of the two fences
///
/// We do not add another control doctest. The main doctest above already uses
/// the same ingredients (`resolve_border` / `SpecifiedValues::initial` /
/// `ComputedLength` / `BorderStyle`). It compiles while calling `.width()` and
/// `.style()` and directly reading the `pub` `color` field (`computed.color` /
/// `specified.color`). If these ingredients drift through renaming or a shape
/// change, that ordinary doctest fails first, exposing a vacuous compile-fail
/// fence above.
///
/// This control cannot detect a change where `resolve_border` returns a
/// reference instead of a value: `.width()` and `.color` compile for either
/// return type. If that change coincides with wider visibility of width/style,
/// both fences would still fail with `E0594` and become vacuous. Revisit this
/// documentation if `resolve_border`'s return type changes.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedBorder {
    /// Absolutized border width.
    ///
    /// When `style` is [`BorderStyle::None`] or [`BorderStyle::Hidden`],
    /// [`resolve_border`] always sets this field to `ComputedLength::ZERO`
    /// (the propdef's style gating). `pub(crate)` prevents outside callers from
    /// breaking that invariant by writing directly; [`Self::width`] is the
    /// public read-only accessor.
    pub(crate) width: ComputedLength,
    /// `border-*-style` — the specified keyword remains when computed.
    ///
    /// Restrict writes along with `width` via `pub(crate)`; only the read-only
    /// accessor [`Self::style`] is public.
    pub(crate) style: BorderStyle,
    /// `border-*-color` — retain `currentcolor` in the computed layer;
    /// paint resolves it at used-value time.
    pub color: BorderColor,
}

impl ComputedBorder {
    /// Read-only accessor for the absolutized border width.
    ///
    /// Always `ComputedLength::ZERO` if [`Self::style`] is
    /// [`BorderStyle::None`] or [`BorderStyle::Hidden`], as enforced by [`resolve_border`].
    pub fn width(&self) -> ComputedLength {
        self.width
    }

    /// Read-only accessor for the computed `border-*-style` value.
    pub fn style(&self) -> BorderStyle {
        self.style
    }
}

/// Computed value of one `text-shadow` entry.
///
/// CSS Text Decoration Module Level 3 §4
/// <https://www.w3.org/TR/css-text-decor-3/#text-shadow-property> defines the
/// computed value as "a list, each item consisting of three absolute lengths
/// plus a computed color". The three lengths (`offset_x`/`offset_y`/
/// `blur_radius`) from specified-layer [`TextShadowItem`] ([`crate::property`])
/// become [`ComputedLength`]. `color` behaves like
/// [`ComputedBorder::color`]: paint handles used-value resolution (see
/// [`TextShadowColor`]).
///
/// Use `#[non_exhaustive]` as for sibling [`ComputedBorder`], so future fields
/// can be added without a breaking change.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedTextShadow {
    /// Absolutized `offset-x`.
    pub offset_x: ComputedLength,
    /// Absolutized `offset-y`.
    pub offset_y: ComputedLength,
    /// Absolutized `blur-radius`; if omitted, its specified-layer value was
    /// eagerly set to `Length::Px(0.0)` (see [`TextShadowItem`]), yielding `ComputedLength::ZERO`.
    pub blur_radius: ComputedLength,
    /// `<color>` — retain `currentcolor` in the computed layer; paint handles
    /// used-value resolution (see [`TextShadowColor`]).
    pub color: TextShadowColor,
}

/// Shared computed-layer Arc for an empty `text-shadow` list (`none`):
/// counterpart of [`crate::property::empty_text_shadow_list`], using the same
/// `OnceLock` shared-slot pattern to avoid per-node allocations.
/// The specified layer (`Arc<Vec<TextShadowItem>>`) and computed layer
/// (`Arc<Vec<ComputedTextShadow>>`) need separate slots because phase 3 changes
/// the type while absolutizing [`Length`] to [`ComputedLength`].
pub(crate) fn empty_computed_text_shadow_list() -> Arc<Vec<ComputedTextShadow>> {
    static EMPTY: OnceLock<Arc<Vec<ComputedTextShadow>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Absolutize a specified [`TextShadowItem`] into [`ComputedTextShadow`]
/// (**phase 3**, using this node's basis).
///
/// All three lengths are `<length>` without percentages (see [`TextShadowItem`]).
/// Delegate to [`resolve_length`], not percentage-aware
/// [`resolve_length_percentage`]. [`resolve_length_or_normal`] likewise delegates
/// the `<length>` components of `letter-spacing`/`word-spacing` to
/// [`resolve_length`] for the same reason. Pass through `color`, which contains
/// no length.
pub(crate) fn resolve_text_shadow_length(
    specified: TextShadowLength,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        TextShadowLength::Length(length) => resolve_length(length, font_size, own_line_height, ctx),
        TextShadowLength::Calc { px, em } => ComputedLength(px + em * font_size.px()),
    }
}

pub fn resolve_text_shadow_item(
    specified: TextShadowItem,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTextShadow {
    let blur_radius =
        resolve_text_shadow_length(specified.blur_radius, font_size, own_line_height, ctx);
    let blur_radius = if matches!(specified.blur_radius, TextShadowLength::Calc { .. }) {
        ComputedLength(blur_radius.px().max(0.0))
    } else {
        blur_radius
    };
    ComputedTextShadow {
        offset_x: resolve_text_shadow_length(specified.offset_x, font_size, own_line_height, ctx),
        offset_y: resolve_text_shadow_length(specified.offset_y, font_size, own_line_height, ctx),
        blur_radius,
        color: specified.color,
    }
}

/// **Lift** one inherited computed `text-shadow` item into specified form
/// to seed inheritance.
///
/// As with [`lift_length_or_normal`], this is lossless and idempotent:
/// [`resolve_length`]'s `Px` arm is the identity, so running the lifted value
/// through phase 3 again does not apply anything twice. Pass through `color`,
/// which contains no length.
pub fn lift_text_shadow_item(computed: ComputedTextShadow) -> TextShadowItem {
    TextShadowItem {
        offset_x: TextShadowLength::Length(Length::Px(computed.offset_x.0)),
        offset_y: TextShadowLength::Length(Length::Px(computed.offset_y.0)),
        blur_radius: TextShadowLength::Length(Length::Px(computed.blur_radius.0)),
        color: computed.color,
    }
}

/// Computed `border-radius`. Absolutize lengths to px and retain percentages
/// until used-value layout.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedBorderRadius {
    /// Top-left corner radius.
    pub top_left: ComputedLengthPercentage,
    /// Top-right corner radius.
    pub top_right: ComputedLengthPercentage,
    /// Bottom-right corner radius.
    pub bottom_right: ComputedLengthPercentage,
    /// Bottom-left corner radius.
    pub bottom_left: ComputedLengthPercentage,
}

impl ComputedBorderRadius {
    /// Fill all corners with the same computed length.
    pub fn all(value: ComputedLength) -> Self {
        let value = ComputedLengthPercentage::Px(value.0);
        Self {
            top_left: value,
            top_right: value,
            bottom_right: value,
            bottom_left: value,
        }
    }

    /// Build a radius from the four physical corner values.
    pub const fn corners(
        top_left: ComputedLengthPercentage,
        top_right: ComputedLengthPercentage,
        bottom_right: ComputedLengthPercentage,
        bottom_left: ComputedLengthPercentage,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }
}

/// One computed `box-shadow` entry.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedBoxShadowItem {
    /// Absolutized horizontal offset.
    pub offset_x: ComputedLength,
    /// Absolutized vertical offset.
    pub offset_y: ComputedLength,
    /// Absolutized blur radius.
    pub blur_radius: ComputedLength,
    /// Absolutized spread distance.
    pub spread_radius: ComputedLength,
    /// Color; retain `currentcolor` until the used-value layer.
    pub color: TextShadowColor,
    /// Whether the shadow is painted inside the border box (`inset`).
    pub inset: bool,
}

/// Computed `outline` (CSS Basic User Interface Module Level 3 §4).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComputedOutline {
    /// Absolutized outline width; zero if the style is `none` or `hidden`.
    pub(crate) width: ComputedLength,
    /// Outline style.
    pub(crate) style: OutlineStyle,
    /// Outline color (`invert`, `currentcolor`, or a resolved `<color>`).
    pub color: OutlineColor,
}

impl ComputedOutline {
    /// Return the absolutized outline width.
    pub fn width(&self) -> ComputedLength {
        self.width
    }

    /// Return the outline style.
    pub fn style(&self) -> OutlineStyle {
        self.style
    }
}

/// One computed `transform` function (the computed counterpart of
/// [`crate::property::TransformFunction`] from CSS Transforms Level 1 §9.1).
/// Absolutize only the length component of each `<length-percentage>` slot;
/// retain percentages symbolically.
///
/// CSS Transforms Level 1 §4 "The transform property"
/// <https://www.w3.org/TR/css-transforms-1/#transform-property> defines the
/// computed value as "as specified, but with lengths made absolute". The six
/// `<number>` slots in `matrix()` and the `<angle>` slots in `rotate()`/`skew()`/
/// `skewX()`/`skewY()` need no such conversion: numbers are already resolved,
/// while the spec does not normalize those angles. Only the lengths in
/// `translate()`/`translateX()`/`translateY()` are affected, becoming either
/// `ComputedLengthPercentage::Px` or `Percent`. This is the same partial
/// absolutization as `ComputedCssPosition` for `background-position` and
/// `object-position`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ComputedTransformFunction {
    /// `matrix(<number>{6})` — all six coefficients remain `<number>`.
    Matrix([f32; 6]),
    /// `translate(<length-percentage>, <length-percentage>)` — absolutize each axis.
    Translate(ComputedLengthPercentage, ComputedLengthPercentage),
    /// `translateX(<length-percentage>)` — absolutized.
    TranslateX(ComputedLengthPercentage),
    /// `translateY(<length-percentage>)` — absolutized.
    TranslateY(ComputedLengthPercentage),
    /// `scale(<number>, <number>)` — retain `<number>`.
    Scale(f32, f32),
    /// `scaleX(<number>)`.
    ScaleX(f32),
    /// `scaleY(<number>)`.
    ScaleY(f32),
    /// `rotate(<angle>)` — retain `<angle>` without normalization.
    Rotate(Angle),
    /// `skew(<angle>, <angle>)`.
    Skew(Angle, Angle),
    /// `skewX(<angle>)`.
    SkewX(Angle),
    /// `skewY(<angle>)`.
    SkewY(Angle),
}

/// Shared computed-layer Arc for an empty `transform` list (`none`). The empty list
/// represents `none`, as in [`crate::property::empty_transform_list`].
pub(crate) fn empty_computed_transform_list() -> Arc<Vec<ComputedTransformFunction>> {
    static EMPTY: OnceLock<Arc<Vec<ComputedTransformFunction>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Absolutize one `transform` function against this node's font-size and
/// line-height. Convert only the length side of `<length-percentage>` slots to
/// `Px`; retain their percentage side as `Percent`.
pub fn resolve_transform_function(
    specified: TransformFunction,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTransformFunction {
    match specified {
        TransformFunction::Matrix(m) => ComputedTransformFunction::Matrix(m),
        TransformFunction::Translate(tx, ty) => ComputedTransformFunction::Translate(
            resolve_length_percentage(tx, font_size, own_line_height, ctx),
            resolve_length_percentage(ty, font_size, own_line_height, ctx),
        ),
        TransformFunction::TranslateX(v) => ComputedTransformFunction::TranslateX(
            resolve_length_percentage(v, font_size, own_line_height, ctx),
        ),
        TransformFunction::TranslateY(v) => ComputedTransformFunction::TranslateY(
            resolve_length_percentage(v, font_size, own_line_height, ctx),
        ),
        TransformFunction::Scale(x, y) => ComputedTransformFunction::Scale(x, y),
        TransformFunction::ScaleX(v) => ComputedTransformFunction::ScaleX(v),
        TransformFunction::ScaleY(v) => ComputedTransformFunction::ScaleY(v),
        TransformFunction::Rotate(a) => ComputedTransformFunction::Rotate(a),
        TransformFunction::Skew(ax, ay) => ComputedTransformFunction::Skew(ax, ay),
        TransformFunction::SkewX(a) => ComputedTransformFunction::SkewX(a),
        TransformFunction::SkewY(a) => ComputedTransformFunction::SkewY(a),
    }
}

/// Shared computed-layer Arc for an empty `box-shadow` list (`none`).
pub(crate) fn empty_computed_box_shadow_list() -> Arc<Vec<ComputedBoxShadowItem>> {
    static EMPTY: OnceLock<Arc<Vec<ComputedBoxShadowItem>>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(Vec::new())).clone()
}

/// Absolutize each `border-radius` corner against this node's font-size and line-height.
pub fn resolve_border_radius(
    specified: BorderRadius,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedBorderRadius {
    let resolve_corner = |value: Length| match value {
        Length::Percent(percent) => ComputedLengthPercentage::Percent(percent),
        value => ComputedLengthPercentage::Px(
            resolve_length(value, font_size, own_line_height, ctx).px(),
        ),
    };
    ComputedBorderRadius {
        top_left: resolve_corner(specified.top_left),
        top_right: resolve_corner(specified.top_right),
        bottom_right: resolve_corner(specified.bottom_right),
        bottom_left: resolve_corner(specified.bottom_left),
    }
}

/// Absolutize one `box-shadow` entry against this node's font-size and line-height.
pub fn resolve_box_shadow_item(
    specified: BoxShadowItem,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedBoxShadowItem {
    ComputedBoxShadowItem {
        offset_x: resolve_length(specified.offset_x, font_size, own_line_height, ctx),
        offset_y: resolve_length(specified.offset_y, font_size, own_line_height, ctx),
        blur_radius: resolve_length(specified.blur_radius, font_size, own_line_height, ctx),
        spread_radius: resolve_length(specified.spread_radius, font_size, own_line_height, ctx),
        color: specified.color,
        inset: specified.inset,
    }
}

/// Absolutize `outline` width (CSS Basic User Interface Module Level 3
/// §4.2). Its computed value is zero for `none` or `hidden` styles; only with a
/// visible style is the specified width absolutized.
pub fn resolve_outline(
    specified: Outline,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedOutline {
    ComputedOutline {
        width: if matches!(specified.style, OutlineStyle::None | OutlineStyle::Hidden) {
            ComputedLength::ZERO
        } else {
            resolve_length(specified.width, font_size, own_line_height, ctx)
        },
        style: specified.style,
        color: specified.color,
    }
}

// ---------------------------------------------------------------------------
// ResolveContext
// ---------------------------------------------------------------------------

/// Document-global reference values needed for absolutization.
///
/// These are the `rem` basis (the root element's computed font-size) and the
/// `rlh` basis (the root element's computed line-height converted to an absolute
/// length by [`used_line_height_length`], or `None` if `normal` cannot be
/// resolved). The second basis was added later.
///
/// # Primary source (§ title + anchor)
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#rem>): `rem` — "Equal to the computed
/// value of the em unit on the root element." /
/// (<https://www.w3.org/TR/css-values-4/#rlh>): `rlh` — "Equal to the value of
/// the lh unit on the root element."
///
/// `#[non_exhaustive]` (see the module doc) lets the struct gain fields
/// without breaking source compatibility. Downstream callers construct it via
/// [`ResolveContext::new`] or [`ResolveContext::with_root_line_height`].
///
/// **But `new` takes positional arguments, so this compatibility does not
/// extend to its constructor.** Adding `root_line_height` illustrates the
/// trade-off: rather than change `new`'s signature, we added
/// [`ResolveContext::with_root_line_height`] for callers who must set it.
/// `new` remains a thin wrapper with `root_line_height: None`. Follow the same
/// approach (another constructor or a builder, not a breaking signature change
/// to `new`) if a field is added later.
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext};
///
/// // Before the root element's font-size is known, and for its own
/// // `font-size: Nrem`, use the initial-value basis.
/// assert_eq!(ResolveContext::initial().root_font_size, ComputedLength(16.0));
/// assert_eq!(
///     ResolveContext::new(ComputedLength(20.0)).root_font_size,
///     ComputedLength(20.0),
/// );
/// // `new` is a thin wrapper for existing callers that do not specify
/// // `root_line_height`; its `rlh` basis is always unresolved (`None`).
/// assert_eq!(ResolveContext::new(ComputedLength(20.0)).root_line_height, None);
/// assert_eq!(
///     ResolveContext::with_root_line_height(ComputedLength(20.0), Some(ComputedLength(24.0)))
///         .root_line_height,
///     Some(ComputedLength(24.0)),
/// );
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolveContext {
    /// Root element's computed font-size: the `rem` basis.
    ///
    /// This [`ComputedLength`] stores an **absolutized computed `<length>`**;
    /// it can directly hold the result of [`resolve_font_size`].
    pub root_font_size: ComputedLength,
    /// Root element's `lh` value: the `rlh` basis.
    ///
    /// An **absolutized length in px** derived by [`used_line_height_length`]
    /// from the root's computed line-height and font-size, or `None` when
    /// `normal` cannot be resolved. `None` reflects the same lack of font
    /// metrics in the style layer as [`ComputedLineHeight::Normal`] (also true
    /// of `cap`/`rcap`; see [`Length::Lh`]). Do not substitute zero or another
    /// fabricated ratio.
    ///
    /// [`Length::Lh`]: crate::property::Length::Lh
    pub root_line_height: Option<ComputedLength>,
}

impl ResolveContext {
    /// Construct with the root element's computed font-size.
    ///
    /// `root_line_height` is unresolved (`None`); use
    /// [`Self::with_root_line_height`] when an `rlh` basis is needed. This thin
    /// wrapper preserves compatibility with callers whose trees have no `rlh`
    /// (see the struct doc's `#[non_exhaustive]` trade-off).
    pub fn new(root_font_size: ComputedLength) -> Self {
        Self {
            root_font_size,
            root_line_height: None,
        }
    }

    /// Construct with both the root element's computed font-size **and** its
    /// `rlh` basis.
    ///
    /// The caller must first absolutize `root_line_height` with
    /// [`used_line_height_length`], passing `None` if the root's computed
    /// line-height is `normal` and cannot be resolved.
    pub fn with_root_line_height(
        root_font_size: ComputedLength,
        root_line_height: Option<ComputedLength>,
    ) -> Self {
        Self {
            root_font_size,
            root_line_height,
        }
    }

    /// Initial context used before the root element's computed font-size is known.
    ///
    /// `root_font_size` is the `font-size` initial value (16px), identical to
    /// [`crate::computed::ComputedValues::initial`]'s `font_size`.
    /// `root_line_height` is unresolved (`None`) too. Root line-height follows
    /// the same no-parent rule and uses its initial value (`normal`); `rlh`
    /// therefore cannot be resolved under this context (see below and
    /// [`SpecifiedValues::finalize_as_root`]).
    ///
    /// The root element's own **`font-size: Nrem`** also uses this basis.
    /// CSS Values 4 §6.1.1 "Font-relative Lengths"
    /// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) says:
    /// "When used in the value of any font-* property on the element they refer
    /// to, the font-relative lengths resolve against the computed metrics of the
    /// parent element—or against the computed metrics corresponding to the
    /// initial values of the font and line-height properties, if the element has
    /// no parent." Thus the root uses the initial-value basis. That section
    /// applies an analogous rule to `lh`/`rlh`. See [`resolve_line_height`] for
    /// the canonical explanation of their asymmetry (`lh` is self-referential;
    /// `rlh` is a tree-global constant). In short, the root element's
    /// `line-height: 1lh` and `1rlh` both reduce to the initial `normal`
    /// line-height, so neither can be resolved.
    ///
    /// **This does not apply to box properties on the root** (such as `padding`).
    /// The cited rule only covers "any font-* property" and "the line-height
    /// property". `rem` in `padding: 2rem` and `rlh` in `padding: 1rlh` use the
    /// root's own computed font-size and line-height by their normal definitions.
    /// Hence phase 3 must use
    /// `ResolveContext::with_root_line_height(own font-size, own rlh basis)`
    /// rather than this initial context, even on the root. This is implemented
    /// by [`SpecifiedValues::finalize_as_root`].
    ///
    /// [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
    pub fn initial() -> Self {
        Self {
            root_font_size: ComputedLength(INITIAL_FONT_SIZE_PX),
            root_line_height: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Absolutization functions (specified → computed)
// ---------------------------------------------------------------------------

/// Convert `pt` to px: `1pt = 1/72in` and CSS defines `1in = 96px`, so `1pt = 96/72px = 4/3px`.
///
/// CSS Values 4 §6.2 "Absolute Lengths"
/// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>): "All of the
/// absolute length units are compatible, and px is their canonical unit."
///
/// Write this as `v * 4.0 / 3.0`, multiplying first. f32 addition and
/// multiplication are not associative: "simplifying" it to `v * (4.0 / 3.0)`
/// changes the resulting f32 bit pattern. **Do not change this expression.**
///
/// (Previously, three bridge helpers in raikiri-dom's `layout.rs` (padding,
/// width-height, and margin) performed the same conversion from `Length::Pt(v)`
/// using `v * 4.0 / 3.0`. Their return wrappers differed:
/// `LengthPercentage`, `Dimension`, and `LengthPercentageAuto`. Bit-for-bit
/// consistency with them was another reason for this evaluation order. All
/// three arms were later removed when the bridge began taking computed-layer
/// types: pt has already become px in cascade phase 3 and never reaches the
/// bridge. There is no longer a cross-check target, but f32 non-associativity
/// independently justifies retaining this exact expression.)
fn pt_to_px(v: f32) -> f32 {
    v * 4.0 / 3.0
}

/// Convert `in` to px. CSS Values 4 §6.2 "Absolute Lengths"
/// (<https://www.w3.org/TR/css-values-4/#absolute-lengths>) conversion table, verbatim:
/// "1in = 2.54cm = 96px".
fn in_to_px(v: f32) -> f32 {
    v * 96.0
}

/// Convert `cm` to px. CSS Values 4 §6.2 conversion table: "1cm = 96px/2.54".
fn cm_to_px(v: f32) -> f32 {
    v * 96.0 / 2.54
}

/// Convert `mm` to px. CSS Values 4 §6.2 conversion table: "1mm = 1/10th of 1cm".
/// Go through [`cm_to_px`] as specified: the spec provides no direct px
/// equivalence, only the ratio to cm (and from cm to inches).
fn mm_to_px(v: f32) -> f32 {
    cm_to_px(v) / 10.0
}

/// Convert `Q` (quarter-millimeter) to px. CSS Values 4 §6.2 conversion
/// table: "1Q = 1/40th of 1cm". Go through [`cm_to_px`] as for [`mm_to_px`].
fn q_to_px(v: f32) -> f32 {
    cm_to_px(v) / 40.0
}

/// Convert `pc` (pica) to px. CSS Values 4 §6.2 conversion table:
/// "1pc = 1/6th of 1in". Go through [`in_to_px`] as for [`mm_to_px`] / [`q_to_px`].
fn pc_to_px(v: f32) -> f32 {
    in_to_px(v) / 6.0
}

/// Convert an already-absolutized [`ComputedLineHeight`] (this element's or
/// the root's) to an **absolute length** usable as an `lh` / `rlh` multiplier.
///
/// # Primary source (§ title + anchor)
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths" [`lh`](https://www.w3.org/TR/css-values-4/#lh)
/// verbatim: "Equal to the computed value of the line-height property of the
/// element on which it is used, converting normal to an absolute length by
/// using only the metrics of the first available font."
///
/// - [`ComputedLineHeight::Length`] — already absolute; return unchanged.
/// - [`ComputedLineHeight::Number`] — its used value is the number multiplied by
///   **this element's** font-size (CSS Inline 3 §5.1
///   <https://www.w3.org/TR/css-inline-3/#propdef-line-height> defines unitless
///   multiplier semantics; line-box height uses exactly this product).
/// - [`ComputedLineHeight::Normal`] — return **`None`**. Resolving `normal` to
///   an absolute length requires "the metrics of the first available font"
///   (real ascent/descent), but `raikiri-style` has no font instance in the
///   style layer. This is the same limit that split out `cap` / `rcap` (see
///   [`crate::property::Length::Lh`]). The spec gives no font-size ratio as a
///   fallback here; do not invent one like those for `ex`/`ch`/`ic`.
///   The caller selects a fallback for the consuming property (see
///   [`resolve_length_percentage`] and related documentation).
///
/// `normal` is the **initial value** for `line-height`. Returning `None` is
/// therefore common for elements using `lh`/`rlh`, not an edge case.
///
/// ```
/// use raikiri_style::{ComputedLength, ComputedLineHeight, used_line_height_length};
///
/// let font_size = ComputedLength(20.0);
///
/// // Leave `<length>` unchanged.
/// assert_eq!(
///     used_line_height_length(ComputedLineHeight::Length(ComputedLength(30.0)), font_size),
///     Some(ComputedLength(30.0)),
/// );
/// // Multiply `<number>` by this element's font-size.
/// assert_eq!(
///     used_line_height_length(ComputedLineHeight::Number(1.5), font_size),
///     Some(ComputedLength(30.0)),
/// );
/// // Cannot resolve `normal` without font metrics.
/// assert_eq!(
///     used_line_height_length(ComputedLineHeight::Normal, font_size),
///     None,
/// );
/// ```
pub fn used_line_height_length(
    line_height: ComputedLineHeight,
    font_size: ComputedLength,
) -> Option<ComputedLength> {
    match line_height {
        ComputedLineHeight::Normal => None,
        ComputedLineHeight::Number(n) => Some(ComputedLength(font_size.0 * n)),
        ComputedLineHeight::Length(l) => Some(l),
    }
}

/// Common helper that multiplies an authored `lh` / `rlh` factor by the basis
/// returned from [`used_line_height_length`]. Pass through `None` when the
/// basis is unresolved (`normal`). Each `resolve_*` function then chooses a
/// fallback (`0px` / `Auto` / `normal`) for its consuming property.
fn resolve_lh_multiplier(v: f32, basis: Option<ComputedLength>) -> Option<ComputedLength> {
    basis.map(|b| ComputedLength(b.0 * v))
}

/// Absolutize specified `font-size` (**phase 2**, using the parent basis).
///
/// `parent_font_size` is the **parent element's** computed font-size. For the
/// parentless root element, pass the initial font-size (16px,
/// [`ComputedLength`]`(16.0)`).
///
/// `self_reference_basis` is the **parent element's** used line-height, used
/// to resolve [`Length::Lh`] (see the `Nlh` row below). The caller first
/// converts it to an absolute length via
/// [`used_line_height_length`]`(parent.line_height, parent.font_size)`.
/// Pass `None` if `normal` cannot be resolved or there is no parent (root).
/// Its name matches the corresponding argument of [`resolve_line_height`]:
/// both are parent bases for self-referential `lh`. The caller's local may
/// still be named `parent_line_height_basis`: caller and callee names need not
/// match, whereas corresponding arguments of sibling functions should.
///
/// # Resolution by unit
///
/// | specified | computed | basis |
/// |---|---|---|
/// | `Npx` | `N` px | identity |
/// | `Npt` / `Ncm` / `Nmm` / `NQ` / `Nin` / `Npc` | per conversion table | CSS Values 4 §6.2 <https://www.w3.org/TR/css-values-4/#absolute-lengths> |
/// | `Nem` / `Nex` / `Nch` | `parent_font_size * N` (`ex`/`ch`: also `* 0.5`) | parent-metrics clause below + fallback docs for [`Length::Ex`] / [`Length::Ch`] |
/// | `Nic` | `parent_font_size * N` | parent-metrics clause below + fallback docs for [`Length::Ic`] |
/// | `Nrem` / `Nrex` / `Nrch` / `Nric` | `ctx.root_font_size * N` (`rex`/`rch`: also `* 0.5`) | CSS Values 4 §6.1.1 `rem` <https://www.w3.org/TR/css-values-4/#rem> |
/// | `N%` | `parent_font_size * N / 100` | CSS Fonts 4 `font-size` propdef "Percentages: refer to parent element's font size" <https://www.w3.org/TR/css-fonts-4/#propdef-font-size> |
/// | `Nlh` | `self_reference_basis * N`, or [`INITIAL_FONT_SIZE_PX`] if the basis is `None` | "Self-reference for `lh` / `rlh`" below |
/// | `Nrlh` | `ctx.root_line_height * N`, or [`INITIAL_FONT_SIZE_PX`] if the basis is `None` | same |
///
/// Without real font metrics in the style layer, `ex` / `rex` / `ch` / `rch` /
/// `ic` / `ric` always use the spec's unknown-metric fallbacks (see the docs
/// for their variants, such as [`Length::Ex`]). `font-size` is a font-* property,
/// so it uses the **parent** basis under the parent-metrics clause, like `em`.
///
/// [`INITIAL_FONT_SIZE_PX`]: crate::computed::INITIAL_FONT_SIZE_PX
///
/// # Self-reference for `lh` / `rlh`
///
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) verbatim:
/// "Similarly, when lh or rlh units are used in the value of the line-height
/// property or font-\* properties on the element they refer to, they resolve
/// against the computed line-height and font metrics of the parent
/// element—or the computed metrics corresponding to the initial values of
/// the font and line-height properties, if the element has no parent."
/// `font-size` is a font-* property, so this clause applies.
///
/// Make the **same distinction** as the "`Length::Lh` — self-reference"
/// and "`Length::Rlh` — a tree-global constant, not self-reference" sections
/// of [`resolve_line_height`]. The quoted clause mentions "lh or rlh" together,
/// so line-height and font-size should resolve them consistently:
///
/// - By definition `lh` refers to "the element on which it is used". On
///   `font-size`, it always refers to itself; fall back to the **parent** basis
///   passed as `self_reference_basis`.
/// - `rlh` refers to "the lh unit on the root element", a tree-global value
///   independent of where it is declared. It is self-referential only when
///   declared on the root. [`crate::specified::SpecifiedValues::finalize_as_root`]
///   handles that case by passing [`ResolveContext::initial`] as `ctx`, so
///   `ctx.root_line_height` is `None`. On a **non-root** element,
///   `font-size: 1rlh` refers to the separately resolved root node and is not
///   self-referential. Like `rlh` on box properties (see [`resolve_length`]),
///   it reads `ctx.root_line_height`, not `self_reference_basis`.
///
/// If the basis is `None` (unresolvable `normal`, or no parent for the root),
/// fall back to **the `font-size` initial value itself** (`medium` =
/// [`INITIAL_FONT_SIZE_PX`]; CSS Fonts 4 `font-size` propdef: "Initial: medium").
/// This matches [`resolve_line_height`], whose unresolvable `Length::Lh` falls
/// back to its own initial `normal` ([`ComputedLineHeight::Normal`]): treat the
/// invalid declaration as if it had not been made.
/// A **single-property resolver** can return its consuming property's actual
/// initial value. General resolvers such as [`resolve_length`] and
/// [`resolve_length_percentage`] serve **multiple** properties (`border-*-width`,
/// `padding`, etc.) and cannot always do that. Likewise,
/// [`resolve_length_percentage_or_auto`] intercepts the general resolver's 0px
/// fallback to return `Auto`, the actual initial for `width`/`height`.
/// The general resolver's uniform 0px fallback differs from border-width's
/// `medium` = 3px initial value: a **known compromise** (see the "0px fallback
/// for border-width: 1lh — unresolved design compromise" section in the
/// [`resolve_length`] documentation), not a rule for dedicated resolvers.
/// Since `resolve_font_size` only serves `font-size`, it need not inherit this
/// compromise.
///
/// # Primary sources (§ title + anchor)
///
/// - CSS Values 4 §6.1.1 "Font-relative Lengths"
///   (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>): "When used
///   in the value of any font-* property on the element they refer to, the
///   font-relative lengths resolve against the computed metrics of the parent
///   element—or against the computed metrics corresponding to the initial values
///   of the font and line-height properties, if the element has no parent."
///   Therefore `em` in `font-size` uses the **parent** basis, avoiding a
///   reference to its own font-size.
/// - CSS Fonts 4 §2.5 "Font size: the font-size property"
///   (<https://www.w3.org/TR/css-fonts-4/#propdef-font-size>):
///   "Percentages: refer to parent element's font size" /
///   "Computed value: an absolute length".
///
/// # Caller contract
///
/// **Pass [`ResolveContext::initial`] when absolutizing the root element's
/// `font-size`.** The `Rem` / `Rex` / `Rch` / `Ric` arms always read
/// `ctx.root_font_size`. Reusing one `ResolveContext::new(root_font_size)` for
/// the entire tree would make `html { font-size: 2rem }` (likewise `2rex` /
/// `2rch` / `2ric`) self-referential. Under the parent-metrics clause of CSS
/// Values 4 §6.1.1, a parentless root instead uses initial values.
/// **Also pass `None` for `self_reference_basis`**: the parentless root's basis
/// is the initial `line-height: normal`, which cannot be resolved.
///
/// In the cascade pipeline, [`SpecifiedValues::finalize_as_root`] honors this
/// contract. Three end-to-end checks in [`mod@crate::cascade`] are
/// `rem_on_root_element_resolves_against_initial_font_size`,
/// `rem_below_root_element_resolves_against_root_computed_font_size`, and
/// `rem_on_root_element_box_property_uses_own_font_size`.
/// Direct callers must honor the contract themselves.
///
/// [`SpecifiedValues::finalize_as_root`]: crate::specified::SpecifiedValues::finalize_as_root
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, resolve_font_size};
/// use raikiri_style::property::Length;
///
/// let ctx = ResolveContext::initial();
///
/// // em compounding: 16px → 1.5em → 1.5em = 24px → 36px
/// let child = resolve_font_size(Length::Em(1.5), ComputedLength(16.0), None, &ctx);
/// assert_eq!(child, ComputedLength(24.0));
/// let grandchild = resolve_font_size(Length::Em(1.5), child, None, &ctx);
/// assert_eq!(grandchild, ComputedLength(36.0));
///
/// // `1lh` resolves against the *parent's* used line-height
/// // — not the declaring element's own font-size. Parent: font-size 16px,
/// // `line-height: 1.5` (unitless) → used line-height 24px.
/// let self_reference_basis = Some(ComputedLength(24.0));
/// let font_size =
///     resolve_font_size(Length::Lh(2.0), ComputedLength(16.0), self_reference_basis, &ctx);
/// assert_eq!(font_size, ComputedLength(48.0));
/// ```
pub fn resolve_font_size(
    specified: Length,
    parent_font_size: ComputedLength,
    self_reference_basis: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        Length::Px(v) => ComputedLength(v),
        Length::Pt(v) => ComputedLength(pt_to_px(v)),
        Length::Cm(v) => ComputedLength(cm_to_px(v)),
        Length::Mm(v) => ComputedLength(mm_to_px(v)),
        Length::Q(v) => ComputedLength(q_to_px(v)),
        Length::In(v) => ComputedLength(in_to_px(v)),
        Length::Pc(v) => ComputedLength(pc_to_px(v)),
        Length::Em(v) => ComputedLength(parent_font_size.0 * v),
        Length::Rem(v) => ComputedLength(ctx.root_font_size.0 * v),
        // ex / ch: unknown-metric fallback = 0.5em (see `Length::Ex` /
        // `Length::Ch`). Use the parent basis because this is font-size itself
        // (parent-metrics clause above, as for `em`).
        Length::Ex(v) | Length::Ch(v) => ComputedLength(parent_font_size.0 * v * 0.5),
        // ic: unknown-metric fallback = 1em (see `Length::Ic`).
        Length::Ic(v) => ComputedLength(parent_font_size.0 * v),
        // rex / rch: use the same fallback against root_font_size (as for `rem`).
        Length::Rex(v) | Length::Rch(v) => ComputedLength(ctx.root_font_size.0 * v * 0.5),
        Length::Ric(v) => ComputedLength(ctx.root_font_size.0 * v),
        // CSS Fonts 4 `font-size` propdef: "Percentages: refer to parent
        // element's font size". This explicitly exempts font-size from the
        // §5.5.1 rule that percentages remain percentages when computed.
        Length::Percent(p) => ComputedLength(parent_font_size.0 * p / 100.0),
        // `lh` / `rlh`: see the self-reference section above. If the basis
        // is `None`, fall back to the initial font-size itself
        // (`INITIAL_FONT_SIZE_PX`), intentionally unlike the generic 0px
        // fallback of `resolve_length` / `resolve_length_percentage`.
        Length::Lh(v) => resolve_lh_multiplier(v, self_reference_basis)
            .unwrap_or(ComputedLength(INITIAL_FONT_SIZE_PX)),
        Length::Rlh(v) => resolve_lh_multiplier(v, ctx.root_line_height)
            .unwrap_or(ComputedLength(INITIAL_FONT_SIZE_PX)),
    }
}

/// Absolutize a specified value for a property that accepts only `<length>`
/// (no `<percentage>` in its grammar): **phase 3**, using this node's basis.
///
/// `font_size` is **this element's** computed font-size, resolved in phase 2.
/// Use [`resolve_font_size`] to absolutize `font-size` itself (parent basis).
///
/// # Percentage handling
///
/// The output type [`ComputedLength`] cannot represent percentages, so this
/// helper's `Length::Percent` arm is a zero fallback. A caller that supports
/// percentages must intercept them before calling this helper.
///
/// - `line-height`: [`resolve_line_height`] intercepts percentages and resolves
///   them against the element's font size.
/// - `letter-spacing`: [`resolve_letter_spacing`] retains the percentage in its
///   dedicated computed type and calls this helper only for absolute lengths.
/// - Properties using [`ComputedLengthPercentage`] must use
///   [`resolve_length_percentage`] instead.
///
/// `resolve_length_or_normal` is an older absolute-length helper; it does not
/// preserve percentages and must not be used for a percentage-aware computed
/// value.
///
/// # `Length::Lh` / `Length::Rlh`
///
/// `own_line_height` is the basis of **the element whose property is being
/// resolved**, already absolutized by the caller via
/// [`used_line_height_length`] (as [`resolve_border`] does for `border-*-width`).
/// `rlh` uses the tree-global `ctx.root_line_height`. The self-reference rule
/// in [`Length::Lh`] applies only to `lh`/`rlh` used as `line-height` itself.
/// Even when `resolve_line_height` delegates a `<length>` component here, it
/// has **already intercepted `Length::Lh` / `Length::Rlh`**, so these arms do
/// not encounter that self-reference.
///
/// # 0px fallback for `border-*-width: 1lh`: unresolved design compromise (Finding B)
///
/// When the basis is `None` (`normal` cannot be resolved, as for cap/rcap),
/// fall back to `0px`. **This is different from the `Percent` arm above**,
/// whose input is impossible under the grammar. `Length::Lh` / `Length::Rlh`
/// are valid `border-*-width` inputs. This is an observable fallback, not a
/// mere completeness case for unreachable input.
///
/// CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) defines the
/// border-width initial as `medium` (= 3px, handled by
/// [`crate::specified::INITIAL_BORDER`]). Zero is the separate gated result
/// when border-style is `none`/`hidden` (handled by `resolve_border`), not a
/// consequence of whether `lh` can be resolved. Thus
/// `border-top-style: solid; border-top-width: 1lh` with `line-height: normal`
/// unexpectedly produces an **invisible** 0px border. Unlike the `Px(0.0)`
/// fallback for `padding` in `resolve_length_percentage` (its true initial),
/// this value does not match border-width's initial; it is merely a placeholder.
///
/// **`letter-spacing` and `word-spacing` have no such mismatch.** If they
/// reach this fallback via [`resolve_length_or_normal`] (for `1lh` with
/// `line-height: normal`), 0px matches their computed `normal` value ("Computes
/// to zero.", CSS Text 3 §7.2/§7.1). Like padding, these are unlike border-width.
///
/// **`vertical-align: <length>` is also closer to spacing than border-width.**
/// If it reaches this fallback via [`resolve_vertical_align`], 0px matches
/// the spec statement in CSS 2.1 §10.8.1: "The value `0cm` means the same as
/// `baseline`." A zero shift has the same effect as `baseline`.
///
/// We recognize the mismatch but **do not fix it here**. The root cause is
/// the inability to resolve `normal` without real font metrics in the style
/// layer (see [`used_line_height_length`]). A proper fix, perhaps allowing
/// `ComputedLength` to represent "unresolved", is separate future work.
/// A border-specific fallback to `medium` might reuse the style-gating logic
/// already in `resolve_border`; consider it as part of that future work.
/// Here 0px is an explicit independent choice, not a shared path with the
/// `Percent` arm. It avoids inventing a metric ratio, but is **not** claimed
/// to be the correct border-width fallback.
pub(crate) fn resolve_length(
    specified: Length,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        Length::Px(v) => ComputedLength(v),
        Length::Pt(v) => ComputedLength(pt_to_px(v)),
        Length::Cm(v) => ComputedLength(cm_to_px(v)),
        Length::Mm(v) => ComputedLength(mm_to_px(v)),
        Length::Q(v) => ComputedLength(q_to_px(v)),
        Length::In(v) => ComputedLength(in_to_px(v)),
        Length::Pc(v) => ComputedLength(pc_to_px(v)),
        Length::Em(v) => ComputedLength(font_size.0 * v),
        Length::Rem(v) => ComputedLength(ctx.root_font_size.0 * v),
        // ex / ch / ic: use unknown-metric fallbacks against this element's
        // font-size (same 0.5em / 1em as `resolve_font_size`, different basis).
        Length::Ex(v) | Length::Ch(v) => ComputedLength(font_size.0 * v * 0.5),
        Length::Ic(v) => ComputedLength(font_size.0 * v),
        Length::Rex(v) | Length::Rch(v) => ComputedLength(ctx.root_font_size.0 * v * 0.5),
        Length::Ric(v) => ComputedLength(ctx.root_font_size.0 * v),
        Length::Percent(_) => ComputedLength::ZERO,
        Length::Lh(v) => resolve_lh_multiplier(v, own_line_height).unwrap_or(ComputedLength::ZERO),
        Length::Rlh(v) => {
            resolve_lh_multiplier(v, ctx.root_line_height).unwrap_or(ComputedLength::ZERO)
        }
    }
}

/// Resolve the non-negative length-or-auto `column-width` value.
pub fn resolve_column_width(
    specified: crate::property::ColumnWidthValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedColumnWidth {
    match specified {
        crate::property::ColumnWidthValue::Auto => ComputedColumnWidth::Auto,
        crate::property::ColumnWidthValue::Length(length) => {
            ComputedColumnWidth::Px(resolve_length(length, font_size, own_line_height, ctx).px())
        }
    }
}

/// Resolve an absolute `word-spacing` length or its `normal` keyword.
///
/// `Normal` becomes [`ComputedLength::ZERO`], as CSS Text 3 §7.1 requires.
/// A `Length` is passed to [`resolve_length`] and the result cannot retain a
/// percentage. `letter-spacing` uses [`resolve_letter_spacing`] instead so its
/// computed percentage and mixed-calc forms remain available.
pub fn resolve_length_or_normal(
    specified: LengthOrNormal,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLength {
    match specified {
        LengthOrNormal::Normal => ComputedLength::ZERO,
        LengthOrNormal::Length(l) => resolve_length(l, font_size, own_line_height, ctx),
    }
}

/// Resolve a spacing length while retaining authored `ch` provenance.
///
/// The ordinary [`resolve_length_or_normal`] API intentionally returns only
/// the computed absolute length. Text layout additionally needs to know that
/// the source was `Nch`, because the actual `0` advance is only available once
/// a font/shaping context is present.
pub fn resolve_length_or_normal_with_ch(
    specified: LengthOrNormal,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthWithCh {
    let ch_factor = match specified {
        LengthOrNormal::Length(Length::Ch(factor)) if factor.is_finite() => Some(factor),
        _ => None,
    };
    ComputedLengthWithCh {
        value: resolve_length_or_normal(specified, font_size, own_line_height, ctx),
        ch_factor,
    }
}

/// Resolve the computed CSS value for `letter-spacing`.
///
/// Simple percentages and mixed calc percentages remain deferred. Relative
/// lengths, including `em` terms in a mixed calc, resolve against the
/// element's computed font size.
pub fn resolve_letter_spacing(
    specified: LetterSpacingValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLetterSpacing {
    match specified {
        LetterSpacingValue::Normal => ComputedLetterSpacing::Px(0.0),
        LetterSpacingValue::Length(Length::Percent(percent)) => {
            ComputedLetterSpacing::Percent(percent)
        }
        LetterSpacingValue::Length(length) => {
            ComputedLetterSpacing::Px(resolve_length(length, font_size, own_line_height, ctx).px())
        }
        LetterSpacingValue::Calc(calc) => {
            let px = calc.px + calc.em * font_size.px() + calc.ch * font_size.px() * 0.5;
            if calc.percent == 0.0 {
                ComputedLetterSpacing::Px(px)
            } else if px == 0.0 {
                ComputedLetterSpacing::Percent(calc.percent)
            } else {
                ComputedLetterSpacing::Calc(CalcLengthPercentage {
                    percent: calc.percent,
                    px,
                })
            }
        }
    }
}

/// Resolve `word-spacing` with the CSS Text 4 computed-value rules shared with
/// `letter-spacing`.
pub fn resolve_word_spacing(
    specified: WordSpacingValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedWordSpacing {
    resolve_letter_spacing(specified, font_size, own_line_height, ctx)
}

/// Resolve the renderer-facing fallback while retaining authored `ch`.
///
/// Percentage and calc values keep the legacy zero fallback until text layout
/// consumes their computed representation.
pub fn resolve_letter_spacing_with_ch(
    specified: LetterSpacingValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthWithCh {
    match specified {
        LetterSpacingValue::Normal => resolve_length_or_normal_with_ch(
            LengthOrNormal::Normal,
            font_size,
            own_line_height,
            ctx,
        ),
        LetterSpacingValue::Length(length) => resolve_length_or_normal_with_ch(
            LengthOrNormal::Length(length),
            font_size,
            own_line_height,
            ctx,
        ),
        LetterSpacingValue::Calc(calc) => ComputedLengthWithCh {
            value: ComputedLength::ZERO,
            ch_factor: calc_ch_factor(calc),
        },
    }
}

/// Resolve the existing absolute renderer/layout fallback for `word-spacing`,
/// retaining authored `ch` provenance just like the letter-spacing path.
pub fn resolve_word_spacing_with_ch(
    specified: WordSpacingValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthWithCh {
    resolve_letter_spacing_with_ch(specified, font_size, own_line_height, ctx)
}

/// Absolutize specified `tab-size` (**phase 3**, using this node's basis).
///
/// The spec's computed value is a "number or absolute length". Pass
/// [`TabSize::Number`] through (see [`ComputedTabSize`] and the scope notes in
/// [`crate::property::TabSize`]: resolving actual tab-stop advances needs font
/// metrics and belongs to a downstream consumer). Delegate [`TabSize::Length`]
/// to [`resolve_length`], not [`resolve_length_percentage`], because [`TabSize`]
/// has no percentages. [`resolve_length_or_normal`] makes the same choice of
/// [`resolve_length`] for a length-only grammar. For
/// [`TabSize::Calc`], resolve `px` + `em * font-size` and clamp negative derived
/// values to zero (CSS Values 4 §10.7; see the arm comment).
pub fn resolve_tab_size(
    specified: TabSize,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTabSize {
    match specified {
        TabSize::Number(n) => ComputedTabSize::Number(n),
        TabSize::Length(l) => {
            ComputedTabSize::Length(resolve_length(l, font_size, own_line_height, ctx))
        }
        TabSize::Calc(calc) => {
            // Percentages were rejected when parsed ("Percentages: N/A"), so
            // resolve only `px` + `em`. CSS Values 4 §10.7 requires clamping
            // derived negative lengths to zero for properties that disallow
            // negative lengths. WPT `tab-size-computed.html` checks
            // `"calc(10px - 0.5em)"` (40px font-size → `-10px`) → `"0px"`.
            let px = calc.px + calc.em * font_size.px();
            ComputedTabSize::Length(ComputedLength(px.max(0.0)))
        }
    }
}

/// Absolutize specified `border-spacing` (**phase 3**, using this node's basis).
///
/// CSS Tables 3 §6.1 propdef: "Computed value: two absolute lengths" —
/// Delegate both axes to [`resolve_length`], not percentage-aware
/// [`resolve_length_percentage`], since [`BorderSpacingValue`] has no
/// percentages. [`resolve_tab_size`] uses [`resolve_length`] for the same
/// reason in its `Length` arm.
///
/// # Computed-time clamp
///
/// Clamp negative computed values derived from `calc()` to zero under CSS
/// Values 4 §10.7 (properties that disallow negative lengths cannot compute
/// negative values). WPT `border-spacing-computed.html` checks
/// `"calc(10px - 0.5em)"` (40px font-size → `-10px`) → `"0px"`.
/// The parser in [`crate::property`] already rejects authored negative values
/// such as `-20px`; only derived values from calc need clamping here.
///
/// ```
/// use raikiri_style::{ComputedBorderSpacing, ComputedLength, ResolveContext, resolve_border_spacing};
/// use raikiri_style::property::{BorderSpacingValue, Length};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(40.0);
///
/// // Clamp negative derived values to zero (WPT computed-value case).
/// let specified = BorderSpacingValue { horizontal: Length::Em(-0.5), vertical: Length::Em(0.5) };
/// // NOTE: parsing rejects `-0.5em`, so the pipeline cannot reach this case.
/// // This doctest checks the clamp itself, not a parsed input.
/// let computed = resolve_border_spacing(specified, font_size, None, &ctx);
/// assert_eq!(computed.horizontal, ComputedLength(0.0));
/// assert_eq!(computed.vertical, ComputedLength(20.0));
/// ```
pub fn resolve_border_spacing(
    specified: BorderSpacingValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedBorderSpacing {
    let horizontal = resolve_length(specified.horizontal, font_size, own_line_height, ctx);
    let vertical = resolve_length(specified.vertical, font_size, own_line_height, ctx);
    ComputedBorderSpacing {
        horizontal: ComputedLength(horizontal.px().max(0.0)),
        vertical: ComputedLength(vertical.px().max(0.0)),
    }
}

/// Absolutize specified `vertical-align: baseline | sub | super | middle |
/// text-top | text-bottom | <length> | <percentage>` (**phase 3**, using this
/// node's basis).
///
/// CSS 2.1 §10.8.1 propdef: "Computed value: for `<percentage>` and
/// `<length>` the absolute length, otherwise as specified" — bare keywords are
/// passed through unchanged.
/// Keywords remain keywords when computed; absolutize `<length>` / `<percentage>`.
/// Shared math processing also supplies mixed `calc()` expressions as
/// [`VerticalAlign::Calc`]; this function resolves their percentage term and
/// returns the result as [`VerticalAlign::Length`] with an absolute px value.
///
/// Resolve `<percentage>` against the element's own used line-height
/// (propdef: "Percentages: refer to the 'line-height' of the element itself").
/// The caller has already absolutized `own_line_height` with
/// [`used_line_height_length`]. If `normal` yields `None`, use `0px` (equivalent
/// to `baseline`). As the [`crate::property::VerticalAlign`] documentation notes
/// under "Implemented: `<percentage>`", this is a documented spec deviation,
/// like the `Length::Lh` fallback from `None` to `0px`. Because `0%` itself is
/// equivalent to `baseline` by spec, only nonzero percentages deviate. Access
/// to font metrics should eventually remove this limitation.
///
/// # Why return [`VerticalAlign`] instead of a new `ComputedVerticalAlign` type
///
/// We do **not** split specified and computed types here as with
/// [`FlexBasisValue`]/[`ComputedFlexBasis`]. `raikiri-paint`'s `walk.rs`
/// (`vertical_align_shift_px`) directly takes [`VerticalAlign`] as the type of
/// [`crate::computed::ComputedValues::vertical_align`].
/// Changing the stored [`VerticalAlign`] to a separate computed type
/// would require changing its signature, while integration with raikiri-paint
/// is outside this crate's scope for this property. Instead, absolutize
/// [`Length`] and return it inside the same enum's
/// [`VerticalAlign::Length`] variant. Structurally, [`crate::page`]'s `fb`
/// helper repacks [`ComputedFlexBasis`] into [`FlexBasisValue`], but unlike
/// that helper there is no `Computed* → specified type` conversion: this
/// function uses the same type before and after absolutization.
pub fn resolve_vertical_align(
    specified: VerticalAlign,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> VerticalAlign {
    match specified {
        VerticalAlign::Baseline
        | VerticalAlign::Sub
        | VerticalAlign::Super
        | VerticalAlign::Middle
        | VerticalAlign::TextTop
        | VerticalAlign::TextBottom
        | VerticalAlign::Top
        | VerticalAlign::Bottom => specified,
        VerticalAlign::Length(Length::Percent(p)) => {
            let px = own_line_height.map(|b| b.0 * p / 100.0).unwrap_or(0.0);
            VerticalAlign::Length(Length::Px(px))
        }
        VerticalAlign::Length(l) => VerticalAlign::Length(Length::Px(
            resolve_length(l, font_size, own_line_height, ctx).px(),
        )),
        VerticalAlign::Calc(value) => {
            let percent_px = own_line_height
                .map(|line_height| line_height.0 * value.percent / 100.0)
                .unwrap_or(0.0);
            VerticalAlign::Length(Length::Px(value.px + percent_px))
        }
    }
}

/// Resolve `text-decoration-inset` lengths against the declaring element's
/// computed font and line-height.
pub fn resolve_text_decoration_inset(
    specified: TextDecorationInset,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTextDecorationInset {
    match specified {
        TextDecorationInset::Auto => ComputedTextDecorationInset::Auto,
        TextDecorationInset::Lengths { start, end } => ComputedTextDecorationInset::Lengths {
            start: resolve_length(start, font_size, own_line_height, ctx),
            end: resolve_length(end, font_size, own_line_height, ctx),
        },
    }
}

/// Resolve `text-decoration-thickness` at computed-value time.
///
/// Keyword values are retained. Lengths become absolute CSS pixels using the
/// declaring element's font-size and line-height basis.
pub fn resolve_text_decoration_thickness(
    specified: TextDecorationThickness,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTextDecorationThickness {
    match specified {
        TextDecorationThickness::Auto => ComputedTextDecorationThickness::Auto,
        TextDecorationThickness::FromFont => ComputedTextDecorationThickness::FromFont,
        TextDecorationThickness::Length(length) => ComputedTextDecorationThickness::Length(
            resolve_length(length, font_size, own_line_height, ctx),
        ),
    }
}

/// Resolve an inherited `text-underline-offset` length-percentage against
/// the declaring element's font metrics. Percentages are kept relative (CSS
/// Text Decoration 4 §2.8) so they rescale with each inheriting element's font
/// size; deferred mixed `calc()` values still use the conservative `auto`
/// fallback.
pub fn resolve_text_underline_offset(
    specified: TextUnderlineOffset,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedTextUnderlineOffset {
    match specified {
        TextUnderlineOffset::Auto => ComputedTextUnderlineOffset::Auto,
        TextUnderlineOffset::Length(Length::Percent(percent)) => {
            ComputedTextUnderlineOffset::Percent(percent)
        }
        TextUnderlineOffset::Length(length) => ComputedTextUnderlineOffset::Length(resolve_length(
            length,
            font_size,
            own_line_height,
            ctx,
        )),
        TextUnderlineOffset::Calc(calc) => match resolve_text_indent_calc(calc, font_size) {
            ComputedTextIndent::Px(px) => ComputedTextUnderlineOffset::Length(ComputedLength(px)),
            ComputedTextIndent::Percent(percent) => ComputedTextUnderlineOffset::Percent(percent),
            ComputedTextIndent::Calc(calc) => ComputedTextUnderlineOffset::Calc(calc),
        },
    }
}

/// Absolutize a specified `<length-percentage>` property (`padding-*`):
/// **phase 3**, using this node's basis.
///
/// Pass `Percent` through **without absolutizing it**: CSS Values 4 §5.5.1
/// (<https://www.w3.org/TR/css-values-4/#combine-percentages>) says "the
/// computed value of a percentage is the specified percentage". Resolution
/// against the containing-block width belongs to the used-value layer (CSS
/// Cascade 5 §4.5 <https://www.w3.org/TR/css-cascade-5/#used>; taffy here).
///
/// # `Length::Lh` / `Length::Rlh`
///
/// `own_line_height` has the same contract as in [`resolve_length`]: the
/// caller already absolutized this element's line-height basis with
/// [`used_line_height_length`]. If it is `None` (`normal` cannot be resolved),
/// fall back to the **`0`** specified initial value for `padding` (CSS Box 3 §4).
/// This is not a fabricated font-metric ratio. It treats a declaration that
/// the style layer cannot resolve as though it were absent. **It is not a
/// formal cascade declaration drop** that falls through to the next candidate:
/// winner selection is complete, and only that winner is replaced by its
/// initial-equivalent value.
pub fn resolve_length_percentage(
    specified: Length,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentage {
    match specified {
        Length::Px(v) => ComputedLengthPercentage::Px(v),
        Length::Pt(v) => ComputedLengthPercentage::Px(pt_to_px(v)),
        Length::Cm(v) => ComputedLengthPercentage::Px(cm_to_px(v)),
        Length::Mm(v) => ComputedLengthPercentage::Px(mm_to_px(v)),
        Length::Q(v) => ComputedLengthPercentage::Px(q_to_px(v)),
        Length::In(v) => ComputedLengthPercentage::Px(in_to_px(v)),
        Length::Pc(v) => ComputedLengthPercentage::Px(pc_to_px(v)),
        Length::Em(v) => ComputedLengthPercentage::Px(font_size.0 * v),
        Length::Rem(v) => ComputedLengthPercentage::Px(ctx.root_font_size.0 * v),
        // ex / ch / ic: same fallback ratios as `resolve_length`.
        Length::Ex(v) | Length::Ch(v) => ComputedLengthPercentage::Px(font_size.0 * v * 0.5),
        Length::Ic(v) => ComputedLengthPercentage::Px(font_size.0 * v),
        Length::Rex(v) | Length::Rch(v) => {
            ComputedLengthPercentage::Px(ctx.root_font_size.0 * v * 0.5)
        }
        Length::Ric(v) => ComputedLengthPercentage::Px(ctx.root_font_size.0 * v),
        Length::Percent(p) => ComputedLengthPercentage::Percent(p),
        Length::Lh(v) => ComputedLengthPercentage::Px(
            resolve_lh_multiplier(v, own_line_height)
                .map(ComputedLength::px)
                .unwrap_or(0.0),
        ),
        Length::Rlh(v) => ComputedLengthPercentage::Px(
            resolve_lh_multiplier(v, ctx.root_line_height)
                .map(ComputedLength::px)
                .unwrap_or(0.0),
        ),
    }
}

/// Authored `ch` coefficient of a mixed `calc()`, when it has one.
///
/// A calc that mixes `ch` with other terms keeps the coefficient as `ch_factor`
/// provenance; [`calc_ch_offset`] gives the remaining absolute part.
pub fn calc_ch_factor(calc: LengthPercentageCalc) -> Option<f32> {
    (calc.ch != 0.0 && calc.ch.is_finite()).then_some(calc.ch)
}

/// Absolute (non-`ch`, non-percentage) part of a mixed `calc()` in CSS px.
///
/// A font-aware consumer resolves a `ch`-bearing calc as
/// `ch_factor * advance + offset`, so a plain `Nch` value has offset zero.
pub fn calc_ch_offset(calc: LengthPercentageCalc, font_size: ComputedLength) -> f32 {
    calc.px + calc.em * font_size.0
}

/// Resolve the deferred `em` coefficient in a `text-indent` calc against the
/// element's computed font size. Percentages remain unresolved for used-value
/// processing. A pure result is collapsed to `Px` or `Percent`; only a mixed
/// result needs the `Calc` computed representation.
pub fn resolve_text_indent_calc(
    specified: LengthPercentageCalc,
    font_size: ComputedLength,
) -> ComputedTextIndent {
    // `ch` keeps only the `0.5em` fallback here; the font-aware consumer
    // replaces it using the factor and offset recorded by the caller.
    let px = specified.px + specified.em * font_size.0 + specified.ch * font_size.0 * 0.5;
    if specified.percent == 0.0 {
        ComputedTextIndent::Px(px)
    } else if px == 0.0 {
        ComputedTextIndent::Percent(specified.percent)
    } else {
        ComputedTextIndent::Calc(crate::property::CalcLengthPercentage {
            percent: specified.percent,
            px,
        })
    }
}

/// Resolve a length-percentage while retaining authored `ch` provenance.
pub fn resolve_length_percentage_with_ch(
    specified: Length,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageWithCh {
    let ch_factor = match specified {
        Length::Ch(factor) if factor.is_finite() => Some(factor),
        _ => None,
    };
    ComputedLengthPercentageWithCh {
        value: resolve_length_percentage(specified, font_size, own_line_height, ctx),
        ch_factor,
    }
}

/// Absolutize specified `<length-percentage> | auto` for **`width` /
/// `height` / `flex-basis`** (**phase 3**, using this node's basis).
/// `margin-*` must use [`resolve_margin_length_or_auto`] instead (Finding A below).
///
/// [`resolve_flex_basis`] makes `flex-basis` the third caller because the spec
/// reuses `<'width'>` (CSS Flexible Box Layout Module Level 1 §7.2.3; see
/// [`crate::property::FlexBasisValue`]). Its grammar literally reuses width's,
/// and its initial is also `auto`. Thus `Auto` is the correct fallback for
/// unresolved `Lh`/`Rlh` on all three properties. This agreement is separate
/// from the reason for excluding margins.
///
/// `Auto` remains a keyword in the computed layer. Handle `Percent` as in
/// [`resolve_length_percentage`]: pass it through for used-value resolution.
///
/// # `Auto` fallback for unresolved `Length::Lh` / `Length::Rlh`
///
/// Do **not** delegate the whole value to [`resolve_length_percentage`]. If the
/// basis (`own_line_height` / `ctx.root_line_height`) is `None` because `normal`
/// cannot be resolved, that function returns `Px(0.0)` (see [`resolve_length_percentage`]). The initial values for
/// `width`/`height` are `auto`, not zero (CSS Sizing 3 §3.1.1
/// <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>).
/// Intercept `Lh`/`Rlh` and return `Auto` when unresolved. This follows the
/// same policy as [`resolve_length_percentage`]—treat an unresolvable
/// declaration as absent—but uses the correct initial for `width`/`height`.
///
/// # Finding A — `margin-*` does not use this function
///
/// Initially margins also used this resolver. The spec says otherwise: the
/// margin initial (CSS Box 3 §3.1
/// <https://www.w3.org/TR/css-box-3/#margin-physical>, "Initial: 0") is a
/// **definite length of zero**, not `auto`. Unlike width/height, `Px(0.0)` is
/// margin's correct initial. Moreover, `auto` on a margin triggers **real
/// layout behavior**, distributing available space (taffy's auto-margin
/// centering; see `length_percentage_auto_to_taffy` in
/// `raikiri-dom/src/layout.rs`). It is not a neutral "unspecified" value.
/// Applying this function's `Auto` fallback to margins would spuriously
/// activate that layout behavior in the common `line-height: normal` case,
/// such as `<div style="line-height: normal; margin-top: 1lh">`.
pub fn resolve_length_percentage_or_auto(
    specified: LengthOrAuto,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrAuto {
    match specified {
        LengthOrAuto::Auto => ComputedLengthPercentageOrAuto::Auto,
        LengthOrAuto::Calc(value) => ComputedLengthPercentageOrAuto::Calc(value),
        LengthOrAuto::Length(Length::Lh(v)) => match resolve_lh_multiplier(v, own_line_height) {
            Some(c) => ComputedLengthPercentageOrAuto::Px(c.px()),
            None => ComputedLengthPercentageOrAuto::Auto,
        },
        LengthOrAuto::Length(Length::Rlh(v)) => {
            match resolve_lh_multiplier(v, ctx.root_line_height) {
                Some(c) => ComputedLengthPercentageOrAuto::Px(c.px()),
                None => ComputedLengthPercentageOrAuto::Auto,
            }
        }
        LengthOrAuto::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrAuto::Px(v),
                ComputedLengthPercentage::Percent(p) => ComputedLengthPercentageOrAuto::Percent(p),
            }
        }
    }
}

/// Absolutize a one-axis `<position>` offset (**phase 3**, using this node's
/// basis). Retain the `Start`/`End` edge and delegate only its
/// `<length-percentage>` payload to [`resolve_length_percentage`] (see the
/// reason for retaining edges in [`ComputedCssPositionOffset`]).
fn resolve_css_position_offset(
    specified: CssPositionOffset,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedCssPositionOffset {
    match specified {
        CssPositionOffset::Start(l) => ComputedCssPositionOffset::Start(resolve_length_percentage(
            l,
            font_size,
            own_line_height,
            ctx,
        )),
        CssPositionOffset::End(l) => ComputedCssPositionOffset::End(resolve_length_percentage(
            l,
            font_size,
            own_line_height,
            ctx,
        )),
    }
}

/// Absolutize a specified `<position>` (for `background-position`, etc.)
/// in **phase 3**, using this node's basis. Delegate each axis to
/// [`resolve_css_position_offset`].
///
/// Since [`CssPosition`] is `#[non_exhaustive]`, no external doctest can
/// construct it with a struct literal (as for [`resolve_border_radius`],
/// [`resolve_box_shadow_item`], and [`resolve_outline`]). Instead, the module's
/// unit tests include `resolve_css_position_absolutizes_each_offset`.
pub fn resolve_css_position(
    specified: CssPosition,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedCssPosition {
    ComputedCssPosition {
        horizontal: resolve_css_position_offset(
            specified.horizontal,
            font_size,
            own_line_height,
            ctx,
        ),
        vertical: resolve_css_position_offset(specified.vertical, font_size, own_line_height, ctx),
    }
}

/// Absolutize specified `background-size: <bg-size>` (**phase 3**, using
/// this node's basis). Pass `cover`/`contain` through as keywords. Delegate
/// each `Explicit` axis to [`resolve_length_percentage_or_auto`]. It has the
/// same shape as `width`/`height`: an unresolved `Lh`/`Rlh` becomes `Auto`.
/// This is correct because [`BackgroundSize`] also has `auto` as its initial.
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, resolve_background_size};
/// use raikiri_style::property::{BackgroundSize, Length, LengthOrAuto};
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(16.0);
///
/// let specified = BackgroundSize::Explicit {
///     width: LengthOrAuto::Length(Length::Em(2.0)),
///     height: LengthOrAuto::Auto,
/// };
/// let computed = resolve_background_size(specified, font_size, None, &ctx);
/// assert_eq!(
///     computed,
///     raikiri_style::ComputedBackgroundSize::Explicit {
///         width: raikiri_style::ComputedLengthPercentageOrAuto::Px(32.0),
///         height: raikiri_style::ComputedLengthPercentageOrAuto::Auto,
///     },
/// );
/// ```
pub fn resolve_background_size(
    specified: BackgroundSize,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedBackgroundSize {
    match specified {
        BackgroundSize::Cover => ComputedBackgroundSize::Cover,
        BackgroundSize::Contain => ComputedBackgroundSize::Contain,
        BackgroundSize::Explicit { width, height } => ComputedBackgroundSize::Explicit {
            width: resolve_length_percentage_or_auto(width, font_size, own_line_height, ctx),
            height: resolve_length_percentage_or_auto(height, font_size, own_line_height, ctx),
        },
    }
}

/// Absolutize a `<gradient>` payload in `background-image` / `mask-image`
/// (**phase 3**, using this node's basis).
///
/// Pass `None` / `Url(String)` through: they already have computed form.
/// For a `Gradient(..)`, absolutize only font-relative parts of its
/// `<length-percentage>` payloads (`GradientColorStop::position`,
/// `RadialSize::Circle`/`Ellipse`, and `CssPosition` in
/// `RadialGradient`/`ConicGradient`) using font-size, root-font-size, and
/// `own_line_height`. Retain `<percentage>` in the computed layer (CSS Values
/// 4 §5.5.1). Its basis, the gradient-box dimensions, belongs to paint and
/// used-value resolution outside this crate. Rather than special-case
/// `Percent`, rely on [`resolve_length_percentage`] to pass it through, as for
/// `padding`/`margin`/`background-position`.
pub fn resolve_background_image(
    specified: BackgroundImage,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> BackgroundImage {
    match specified {
        BackgroundImage::None | BackgroundImage::Url(_) => specified,
        BackgroundImage::Gradient(g) => {
            BackgroundImage::Gradient(resolve_gradient(g, font_size, own_line_height, ctx))
        }
    }
}

/// Absolutize a `<gradient>`'s `<length-percentage>` payloads (CSS Images 4
/// §3) in **phase 3**, using this node's basis. Pass through `Percent` as
/// documented by [`resolve_background_image`].
pub fn resolve_gradient(
    specified: Gradient,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> Gradient {
    match specified {
        Gradient::Linear(g) => {
            Gradient::Linear(resolve_linear_gradient(g, font_size, own_line_height, ctx))
        }
        Gradient::Radial(g) => {
            Gradient::Radial(resolve_radial_gradient(g, font_size, own_line_height, ctx))
        }
        Gradient::Conic(g) => {
            Gradient::Conic(resolve_conic_gradient(g, font_size, own_line_height, ctx))
        }
    }
}

fn absolutized_length(
    specified: Length,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> Length {
    match resolve_length_percentage(specified, font_size, own_line_height, ctx) {
        ComputedLengthPercentage::Px(v) => Length::Px(v),
        ComputedLengthPercentage::Percent(p) => Length::Percent(p),
    }
}

fn absolutized_css_position(
    specified: CssPosition,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> CssPosition {
    let computed = resolve_css_position(specified, font_size, own_line_height, ctx);
    fn lift(l: ComputedLengthPercentage) -> Length {
        match l {
            ComputedLengthPercentage::Px(v) => Length::Px(v),
            ComputedLengthPercentage::Percent(p) => Length::Percent(p),
        }
    }
    fn lift_offset(c: ComputedCssPositionOffset) -> CssPositionOffset {
        match c {
            ComputedCssPositionOffset::Start(l) => CssPositionOffset::Start(lift(l)),
            ComputedCssPositionOffset::End(l) => CssPositionOffset::End(lift(l)),
        }
    }
    CssPosition {
        horizontal: lift_offset(computed.horizontal),
        vertical: lift_offset(computed.vertical),
    }
}

fn resolve_gradient_color_stops(
    stops: &Arc<Vec<GradientColorStop>>,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> Arc<Vec<GradientColorStop>> {
    Arc::new(
        stops
            .iter()
            .map(|stop| GradientColorStop {
                color: stop.color,
                position: stop
                    .position
                    .map(|l| absolutized_length(l, font_size, own_line_height, ctx)),
            })
            .collect(),
    )
}

fn resolve_radial_size(
    specified: RadialSize,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> RadialSize {
    match specified {
        RadialSize::Extent(e) => RadialSize::Extent(e),
        RadialSize::Circle(l) => {
            RadialSize::Circle(absolutized_length(l, font_size, own_line_height, ctx))
        }
        RadialSize::Ellipse(a, b) => RadialSize::Ellipse(
            absolutized_length(a, font_size, own_line_height, ctx),
            absolutized_length(b, font_size, own_line_height, ctx),
        ),
    }
}

fn resolve_linear_gradient(
    specified: LinearGradient,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> LinearGradient {
    LinearGradient {
        repeating: specified.repeating,
        direction: specified.direction,
        interpolation: specified.interpolation,
        stops: resolve_gradient_color_stops(&specified.stops, font_size, own_line_height, ctx),
    }
}

fn resolve_radial_gradient(
    specified: RadialGradient,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> RadialGradient {
    RadialGradient {
        repeating: specified.repeating,
        shape: specified.shape,
        size: resolve_radial_size(specified.size, font_size, own_line_height, ctx),
        position: absolutized_css_position(specified.position, font_size, own_line_height, ctx),
        interpolation: specified.interpolation,
        stops: resolve_gradient_color_stops(&specified.stops, font_size, own_line_height, ctx),
    }
}

fn resolve_conic_gradient(
    specified: ConicGradient,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ConicGradient {
    ConicGradient {
        repeating: specified.repeating,
        angle: specified.angle,
        position: absolutized_css_position(specified.position, font_size, own_line_height, ctx),
        interpolation: specified.interpolation,
        stops: Arc::new(
            specified
                .stops
                .iter()
                .cloned()
                .collect::<Vec<AngularColorStop>>(),
        ),
    }
}

/// Absolutize specified `flex-basis: content | <'width'>` (**phase 3**,
/// using this node's basis).
///
/// Pass `content` through as a keyword (see [`ComputedFlexBasis::Content`] and
/// the scope notes in [`crate::property::FlexBasisValue`]). Delegate the `auto`
/// / `<length-percentage>` branch to [`resolve_length_percentage_or_auto`],
/// matching the `<'width'>` grammar exactly, including its `Auto` fallback for
/// unresolved `Lh`/`Rlh`. Do not duplicate the implementation.
pub fn resolve_flex_basis(
    specified: FlexBasisValue,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedFlexBasis {
    match specified {
        FlexBasisValue::Content => ComputedFlexBasis::Content,
        FlexBasisValue::MinContent => ComputedFlexBasis::MinContent,
        FlexBasisValue::MaxContent => ComputedFlexBasis::MaxContent,
        FlexBasisValue::FitContent => ComputedFlexBasis::FitContent,
        FlexBasisValue::Auto => ComputedFlexBasis::Auto,
        FlexBasisValue::Length(len) => {
            match resolve_length_percentage_or_auto(
                LengthOrAuto::Length(len),
                font_size,
                own_line_height,
                ctx,
            ) {
                ComputedLengthPercentageOrAuto::Auto => ComputedFlexBasis::Auto,
                ComputedLengthPercentageOrAuto::Px(v) => ComputedFlexBasis::Px(v),
                ComputedLengthPercentageOrAuto::Percent(p) => ComputedFlexBasis::Percent(p),
                ComputedLengthPercentageOrAuto::Calc(_) => ComputedFlexBasis::Auto,
            }
        }
    }
}

/// Absolutize specified `<track-breadth>` (**phase 3**, using this node's
/// basis). Pass through keywords and `<flex>`; only `<length-percentage>` changes.
pub fn resolve_grid_track_breadth(
    specified: GridTrackBreadth,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedGridTrackBreadth {
    match specified {
        GridTrackBreadth::Auto => ComputedGridTrackBreadth::Auto,
        GridTrackBreadth::MinContent => ComputedGridTrackBreadth::MinContent,
        GridTrackBreadth::MaxContent => ComputedGridTrackBreadth::MaxContent,
        GridTrackBreadth::Flex(f) => ComputedGridTrackBreadth::Flex(f),
        GridTrackBreadth::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedGridTrackBreadth::Px(v),
                ComputedLengthPercentage::Percent(p) => ComputedGridTrackBreadth::Percent(p),
            }
        }
    }
}

/// Absolutize specified `<inflexible-breadth>` (**phase 3**, using this node's
/// basis). As explained by [`ComputedGridTrackBreadth`], the result type matches
/// [`resolve_grid_track_breadth`]. Since
/// [`crate::property::GridInflexibleBreadth`] has no `<flex>` variant, this
/// function cannot structurally return [`ComputedGridTrackBreadth::Flex`].
pub fn resolve_grid_inflexible_breadth(
    specified: GridInflexibleBreadth,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedGridTrackBreadth {
    match specified {
        GridInflexibleBreadth::Auto => ComputedGridTrackBreadth::Auto,
        GridInflexibleBreadth::MinContent => ComputedGridTrackBreadth::MinContent,
        GridInflexibleBreadth::MaxContent => ComputedGridTrackBreadth::MaxContent,
        GridInflexibleBreadth::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedGridTrackBreadth::Px(v),
                ComputedLengthPercentage::Percent(p) => ComputedGridTrackBreadth::Percent(p),
            }
        }
    }
}

/// Absolutize specified `<track-size>` (**phase 3**, using this node's basis).
pub fn resolve_grid_track_size(
    specified: GridTrackSize,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedGridTrackSize {
    match specified {
        GridTrackSize::Breadth(b) => ComputedGridTrackSize::Breadth(resolve_grid_track_breadth(
            b,
            font_size,
            own_line_height,
            ctx,
        )),
        GridTrackSize::MinMax(min, max) => ComputedGridTrackSize::MinMax(
            resolve_grid_inflexible_breadth(min, font_size, own_line_height, ctx),
            resolve_grid_track_breadth(max, font_size, own_line_height, ctx),
        ),
        GridTrackSize::FitContent(len) => ComputedGridTrackSize::FitContent(
            resolve_length_percentage(len, font_size, own_line_height, ctx),
        ),
    }
}

/// Absolutize specified [`crate::property::GridTrackList`] (**phase 3**,
/// using this node's basis). Clone and carry `line_names` unchanged: they
/// contain no lengths.
pub fn resolve_grid_track_list(
    specified: &GridTrackList,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedGridTrackList {
    ComputedGridTrackList {
        line_names: specified.line_names.clone(),
        components: specified
            .components
            .iter()
            .cloned()
            .map(|c| match c {
                GridTrackListComponent::Size(s) => ComputedGridTrackListComponent::Size(
                    resolve_grid_track_size(s, font_size, own_line_height, ctx),
                ),
                GridTrackListComponent::Repeat(GridTrackRepeat {
                    count,
                    line_names,
                    tracks,
                }) => ComputedGridTrackListComponent::Repeat(ComputedGridTrackRepeat {
                    count,
                    line_names,
                    tracks: tracks
                        .into_iter()
                        .map(|t| resolve_grid_track_size(t, font_size, own_line_height, ctx))
                        .collect(),
                }),
            })
            .collect(),
    }
}

/// Absolutize specified `grid-template-columns` / `grid-template-rows`
/// values of `none | <track-list> | <auto-track-list>` (**phase 3**,
/// using this node's basis).
/// Pass through `none` as a keyword.
pub fn resolve_grid_template_tracks(
    specified: GridTemplateTracks,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedGridTemplateTracks {
    match specified {
        GridTemplateTracks::None => ComputedGridTemplateTracks::None,
        GridTemplateTracks::List(list) => ComputedGridTemplateTracks::List(Arc::new(
            resolve_grid_track_list(&list, font_size, own_line_height, ctx),
        )),
    }
}

/// Absolutize `grid-auto-columns` / `grid-auto-rows` specified `<track-size>+`
/// values (**phase 3**, using this node's basis).
pub fn resolve_grid_auto_track_list(
    specified: &[GridTrackSize],
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> Arc<Vec<ComputedGridTrackSize>> {
    Arc::new(
        specified
            .iter()
            .cloned()
            .map(|t| resolve_grid_track_size(t, font_size, own_line_height, ctx))
            .collect(),
    )
}

/// Absolutize specified `row-gap` / `column-gap` values of
/// `normal | <length-percentage [0,∞]>` (**phase 3**, using this node's basis).
///
/// `normal` stays a keyword when computed (see
/// [`ComputedLengthPercentageOrNormal`]). This deliberately differs from the
/// `normal → ComputedLength::ZERO` behavior for `letter-spacing`/`word-spacing`
/// in [`resolve_length_or_normal`], because the specs use different wording.
/// Delegate `<length-percentage>` to [`resolve_length_percentage`]; percentages
/// pass through for downstream used-value resolution (taffy).
pub fn resolve_length_percentage_or_normal(
    specified: LengthOrNormal,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrNormal {
    match specified {
        LengthOrNormal::Normal => ComputedLengthPercentageOrNormal::Normal,
        LengthOrNormal::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrNormal::Px(v),
                ComputedLengthPercentage::Percent(p) => {
                    ComputedLengthPercentageOrNormal::Percent(p)
                }
            }
        }
    }
}

/// Absolutize **only `margin-*`** specified `<length-percentage> | auto`.
///
/// This has the same shape as [`resolve_length_percentage_or_auto`]: pass the
/// `Auto` keyword through, and delegate `<length-percentage>` to
/// [`resolve_length_percentage`]. But it **does not intercept `Lh`/`Rlh`**.
/// When the basis (`normal`) cannot be resolved, that delegate ([`resolve_length_percentage`]) returns
/// `Px(0.0)`. This is precisely margin's specified initial value (CSS Box 3
/// §3.1), unlike the `Auto` fallback for `width`/`height` in
/// [`resolve_length_percentage_or_auto`]. For margins, `Auto` is neither the
/// correct initial nor harmless: it triggers real auto-margin centering in
/// taffy. Do not change the shared public resolver's fallback for margins:
/// that would also change behavior for its width/height callers and any future
/// callers. Keep the margin-specific change in this function.
pub fn resolve_margin_length_or_auto(
    specified: LengthOrAuto,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLengthPercentageOrAuto {
    match specified {
        LengthOrAuto::Auto => ComputedLengthPercentageOrAuto::Auto,
        // Margin calc support is not yet wired into the taffy margin bridge;
        // retain the safe initial-equivalent until that bridge is added.
        LengthOrAuto::Calc(_) => ComputedLengthPercentageOrAuto::Px(0.0),
        LengthOrAuto::Length(len) => {
            match resolve_length_percentage(len, font_size, own_line_height, ctx) {
                ComputedLengthPercentage::Px(v) => ComputedLengthPercentageOrAuto::Px(v),
                ComputedLengthPercentage::Percent(p) => ComputedLengthPercentageOrAuto::Percent(p),
            }
        }
    }
}

/// Absolutize specified `line-height` (**phase 3 / phase 2.5**, using
/// this node's basis; see the caller's "phase 2.5" documentation in
/// [`crate::specified::SpecifiedValues::finalize`] and
/// [`crate::specified::SpecifiedValues::finalize_as_root`]).
///
/// - Pass `normal` / `<number>` through. Keeping `<number>` in the computed
///   layer matters: children inherit the number and multiply by **their own**
///   font-size.
/// - Absolutize `<percentage>` against **this element's** computed font-size:
///   CSS Inline 3 §5.1 (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>)
///   "Percentages: computed relative to 1em" + CSS Values 4 §6.1.1 `em`
///   (<https://www.w3.org/TR/css-values-4/#em>) "Equal to the computed value of
///   the font-size property of the element on which it is used."
/// - Absolutize `<length>` (other than `Lh` / `Rlh`) under the same rules as
///   [`resolve_length`].
///
/// # `Length::Lh` — self-reference
///
/// `line-height: 1lh` would use this element's computed line-height as its
/// own value. `lh` always means "the element on which it is used", so this
/// self-reference occurs on **every element**.
/// CSS Values 4 §6.1.1 "Font-relative Lengths"
/// (<https://www.w3.org/TR/css-values-4/#font-relative-lengths>) verbatim:
/// "Similarly, when lh or rlh units are used in the value of the line-height
/// property or font-\* properties on the element they refer to, they resolve
/// against the computed line-height and font metrics of the parent
/// element—or the computed metrics corresponding to the initial values of
/// the font and line-height properties, if the element has no parent."
///
/// The caller has already absolutized the **parent's** line-height with
/// [`used_line_height_length`] and passed it as `self_reference_basis`. For a
/// parentless root, pass `None`: the initial `line-height: normal` cannot be
/// resolved (see [`ResolveContext::initial`]). If the basis is `None`, return
/// line-height's own initial, **`normal`** ([`ComputedLineHeight::Normal`]).
/// As with the 0px fallback in [`resolve_length_percentage`], treat an
/// unresolvable declaration like an absent one, using line-height's own
/// natural "unspecified" value.
///
/// # `Length::Rlh` — tree-global constant, not self-reference (unlike `Lh`)
///
/// Although the quote above mentions "lh or rlh" together, this crate
/// does **not** generally treat `rlh` as self-referential. By definition it
/// is "Equal to the value of the lh unit **on the root element**", a tree-global
/// constant independent of the declaring node. A cycle can arise only when
/// the declaration is on the root itself. The root case is handled by
/// [`crate::specified::SpecifiedValues::finalize_as_root`], which passes
/// [`ResolveContext::initial`] as `ctx`, necessarily setting
/// `ctx.root_line_height` to `None`. On a **non-root** node,
/// `line-height: 1rlh` simply refers to the already resolved root node.
/// Like `rlh` in a box property, use `ctx.root_line_height` directly;
/// `self_reference_basis` is the **parent's** line-height and would contradict
/// the definition of `rlh`. The quote's "Similarly" prescribes the same
/// fallback when a self-reference exists; it does not require using the
/// parent basis when `rlh` has no cycle.
pub fn resolve_line_height(
    specified: LineHeight,
    font_size: ComputedLength,
    self_reference_basis: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedLineHeight {
    match specified {
        LineHeight::Normal => ComputedLineHeight::Normal,
        LineHeight::Number(n) => ComputedLineHeight::Number(n),
        LineHeight::Length(Length::Lh(v)) => match resolve_lh_multiplier(v, self_reference_basis) {
            Some(c) => ComputedLineHeight::Length(c),
            None => ComputedLineHeight::Normal,
        },
        LineHeight::Length(Length::Rlh(v)) => {
            match resolve_lh_multiplier(v, ctx.root_line_height) {
                Some(c) => ComputedLineHeight::Length(c),
                None => ComputedLineHeight::Normal,
            }
        }
        LineHeight::Length(len) => ComputedLineHeight::Length(match len {
            // CSS Inline 3 §5.1 "Percentages: computed relative to 1em" —
            // Absolutize percentages against the declaring element's computed
            // font-size; do not use `resolve_length`'s grammar-unreachable
            // 0px arm.
            Length::Percent(p) => ComputedLength(font_size.0 * p / 100.0),
            // `Lh` / `Rlh` were handled above, so `other` cannot contain
            // either (and `own_line_height: None` is a dead argument).
            other => resolve_length(other, font_size, None, ctx),
        }),
    }
}

/// Absolutize one specified `border-*` side (**phase 3**, using this
/// node's basis).
///
/// Absolutize `width`; pass through the specified `style` / `color` keywords.
///
/// # Style gating belongs in the computed layer
///
/// If `style` is `none` or `hidden`, `width` becomes **0px**. The CSS
/// Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>) propdef table says
/// "Computed value: absolute length, snapped as a border width; zero if the
/// border style is `none` or `hidden`". This is a **computed-layer**, not
/// used-layer, requirement in the **TR version** (see "version marker" below).
///
/// **Spec tension (do not silently resolve it)**: the non-normative Note in
/// §3.3 says "Although the initial width is medium, the initial style is none;
/// therefore the used initial width is 0." It describes the **used** layer,
/// whereas the normative propdef table names the **computed** layer. The
/// normative table governs.
///
/// **Version marker**: the statement above is from the TR version
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-width>). The ED
/// (<https://drafts.csswg.org/css-backgrounds-3/#border-width>) moved this
/// gate from the **computed layer to the resolved/used layer** under CSSWG
/// [Issue 11494](https://github.com/w3c/csswg-drafts/issues/11494).
/// It removed "zero if the border style is `none` or `hidden`" from the
/// Computed value row and instead added:
/// "The resolved value for the border-width properties is the used value.
/// If the border-style corresponding to a given border-width is none or
/// hidden, then the used width is 0." **This function follows the TR, not yet
/// the ED.** The TR is the current Recommendation-track version; timing for
/// ED adoption in the W3C process is unknown. Priority is low: current
/// paged-media paths yield a resolved zero under either version, with only
/// the "by computed value" animation interpolation starting point differing.
/// Moving this gate from the computed layer to the used layer would also
/// require bringing back a `used_border_width` equivalent in raikiri-dom.
/// That change is outside this issue; do not make it without a decision.
///
/// **This function is the sole source of gating for both element and page
/// paths.** Previously raikiri-dom's `layout.rs` applied the same gate a layer
/// later with its `used_border_width` helper. It was removed when `layout.rs`
/// moved to consuming [`ComputedBorder`]. Do not reimplement the decision
/// downstream: duplicate spec rules can drift when only one is updated.
///
/// The page path (`@page`) carries a bag of `PropertyValue`s.
/// Phase 3 in [`crate::page::cascade_page`] reassembles `border-*-width`
/// longhands into [`Border`] and **funnels them through this function**;
/// do not repeat `matches!(style, None | Hidden)` on the page path.
/// Longhands lack color, so the caller supplies a placeholder, but this
/// function does not read color to determine width. For an **undeclared**
/// `border-*-style`, use [`crate::specified::INITIAL_BORDER`]'s `style` (= `none`).
/// CSS Paged Media 3 §6 "Page Properties"
/// <https://www.w3.org/TR/css-page-3/#page-properties> says "both the page context
/// and the margin context have a computed value for every property".
///
/// # CAVEAT: border-image
///
/// CSS Backgrounds 3 §3.2 "Line Patterns: the border-style properties"
/// (<https://www.w3.org/TR/css-backgrounds-3/#border-style>) defines `none`:
/// "No border. Color and width are ignored (i.e., the border has width 0). Note
/// this means that the initial value of `border-image-width` will also resolve to
/// zero." This agrees with the §3.3 Computed value row, without a border-image
/// exception. `border-image-*` is not yet implemented (see the Non-goals in
/// `ComputedValues::border`). No reassessment of this gate is needed now, but
/// reread both sections when implementing border-image.
/// `own_line_height` is the basis of this border's element, already absolutized
/// by the caller through [`used_line_height_length`]. It resolves
/// `border-*-width: 1lh` (same contract as [`resolve_length`]). If it is `None`
/// because `normal` cannot be resolved, `resolve_length` returns `0px`.
pub fn resolve_border(
    specified: Border,
    font_size: ComputedLength,
    own_line_height: Option<ComputedLength>,
    ctx: &ResolveContext,
) -> ComputedBorder {
    // `matches!` + else: pass the specified width through for any unknown
    // future `BorderStyle` variant, treating it as visible. This fail-safe
    // prevents a newly added visible style from silently getting zero width.
    // Downstream `layout.rs` consumes the result without repeating the gate.
    let width = if matches!(specified.style, BorderStyle::None | BorderStyle::Hidden) {
        ComputedLength::ZERO
    } else {
        resolve_length(specified.width, font_size, own_line_height, ctx)
    };
    ComputedBorder {
        width,
        style: specified.style,
        color: specified.color,
    }
}

// ---------------------------------------------------------------------------
// Lift computed → specified values (inheritance seeds)
// ---------------------------------------------------------------------------

/// **Lift** the parent's computed `font-size` into specified form to seed
/// inheritance.
///
/// In the three-phase pipeline, phase 1 stages winners in specified types, so
/// an inherited computed value must be converted back to specified form.
/// CSS Values 4 §6 (<https://www.w3.org/TR/css-values-4/#lengths>) permits
/// representing a computed length in any absolute unit. Representing it in
/// px is a **value-preserving identity conversion**, so this lift is lossless.
///
/// `Px` is also a **fixed point** of absolutization ([`resolve_font_size`]'s
/// `Px` arm is the identity). Passing the lifted value through **phase 2**
/// cannot apply it twice. Font-size is absolutized in phase 2, not phase 3:
/// phase-3 resolvers (`resolve_length` and others) use the computed font-size
/// of this node, established by phase 2, as their basis.
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, lift_font_size, resolve_font_size};
///
/// let ctx = ResolveContext::initial();
/// let inherited = ComputedLength(24.0);
///
/// // Lift → absolutize is an identity round trip (`Px` is a fixed point).
/// let lifted = lift_font_size(inherited);
/// assert_eq!(resolve_font_size(lifted, ComputedLength(16.0), None, &ctx), inherited);
/// ```
pub fn lift_font_size(computed: ComputedLength) -> Length {
    Length::Px(computed.0)
}

/// **Lift** the parent's computed `line-height` into specified form to
/// seed inheritance.
///
/// As with [`lift_font_size`], all **three variants** of [`ComputedLineHeight`]
/// lift losslessly:
///
/// - `Length(30px)` remains `30px` through phase 3. A child with a smaller
///   font-size inherits **the same 30px**, rather than resolving the original
///   percentage again. CSS Inline 3 §5.1
///   (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>) specifies
///   "Computed value: … a computed `&lt;length&gt;` value": the percentage is resolved
///   on the declaring element, then the child inherits that length.
///   **Do not re-resolve it against the child's font-size.**
/// - `Number(1.5)` passes through and multiplies the child's own font-size.
/// - `Normal` passes through as a keyword for used-value resolution.
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedLineHeight, ResolveContext, lift_line_height,
///     resolve_line_height,
/// };
/// use raikiri_style::property::{Length, LineHeight};
///
/// let ctx = ResolveContext::initial();
///
/// // Resolve `line-height: 150%` on its declaring element (20px font-size) → 30px.
/// let declared = resolve_line_height(
///     LineHeight::Length(Length::Percent(150.0)),
///     ComputedLength(20.0),
///     None,
///     &ctx,
/// );
/// assert_eq!(declared, ComputedLineHeight::Length(ComputedLength(30.0)));
///
/// // Child (10px font-size) inherits the **same** 30px, not 15px.
/// let child = resolve_line_height(lift_line_height(declared), ComputedLength(10.0), None, &ctx);
/// assert_eq!(child, ComputedLineHeight::Length(ComputedLength(30.0)));
/// ```
pub fn lift_line_height(computed: ComputedLineHeight) -> LineHeight {
    match computed {
        ComputedLineHeight::Normal => LineHeight::Normal,
        ComputedLineHeight::Number(n) => LineHeight::Number(n),
        ComputedLineHeight::Length(l) => LineHeight::Length(Length::Px(l.0)),
    }
}

/// Lift a computed length-percentage back to its specified `Length` form.
///
/// Percentages remain unresolved, so this is a fixed-point operation for both
/// variants of [`ComputedLengthPercentage`].
pub fn lift_length_percentage(computed: ComputedLengthPercentage) -> Length {
    match computed {
        ComputedLengthPercentage::Px(v) => Length::Px(v),
        ComputedLengthPercentage::Percent(p) => Length::Percent(p),
    }
}

/// Lift a computed `text-indent` value into its inherited staging form.
///
/// The parent's `em` term has already been resolved, so a mixed calc is copied
/// with `em: 0` and must not be re-resolved against the child's font size.
pub fn lift_text_indent(computed: ComputedTextIndent) -> TextIndentLength {
    match computed {
        ComputedTextIndent::Px(v) => TextIndentLength::Length(Length::Px(v)),
        ComputedTextIndent::Percent(p) => TextIndentLength::Length(Length::Percent(p)),
        ComputedTextIndent::Calc(value) => TextIndentLength::Calc(LengthPercentageCalc {
            percent: value.percent,
            px: value.px,
            em: 0.0,
            ch: 0.0,
        }),
    }
}

/// **Lift** the parent's computed `letter-spacing` / `word-spacing` into
/// specified form to seed inheritance.
///
/// This has the same lossless fixed-point property as [`lift_font_size`].
/// [`resolve_length_or_normal`]'s `Length` branch delegates to
/// [`resolve_length`], whose `Px` arm is the identity:
/// `ComputedLength(v) -> Length::Px(v)` passes unchanged through phase 3.
///
/// Unlike [`lift_line_height`], this cannot restore `Normal`:
/// `letter-spacing: normal` and `word-spacing: normal` already compute to zero
/// (an absolute length; see [`resolve_length_or_normal`]). This resolver
/// maps `normal` to zero (see [`resolve_length_or_normal`] for details). No `Normal` keyword
/// survives in the computed layer to identify the original declaration.
/// By contrast, [`ComputedLineHeight`] retains its `Normal` variant.
///
/// ```
/// use raikiri_style::{ComputedLength, ResolveContext, lift_length_or_normal, resolve_length_or_normal};
/// use raikiri_style::property::LengthOrNormal;
///
/// let ctx = ResolveContext::initial();
/// let inherited = ComputedLength(2.0);
///
/// // Lift → absolutize is an identity round trip (`Px` is a fixed point).
/// let lifted = lift_length_or_normal(inherited);
/// assert_eq!(
///     resolve_length_or_normal(lifted, ComputedLength(16.0), None, &ctx),
///     inherited
/// );
/// assert_eq!(lifted, LengthOrNormal::Length(raikiri_style::property::Length::Px(2.0)));
/// ```
pub fn lift_length_or_normal(computed: ComputedLength) -> LengthOrNormal {
    LengthOrNormal::Length(Length::Px(computed.0))
}

/// Lift a parent's computed `letter-spacing` value for inheritance.
///
/// Any `em` term has already resolved against the parent, so a lifted mixed
/// calc uses only absolute px and percentage components.
pub fn lift_letter_spacing(computed: ComputedLetterSpacing) -> LetterSpacingValue {
    match computed {
        ComputedLetterSpacing::Px(px) => LetterSpacingValue::Length(Length::Px(px)),
        ComputedLetterSpacing::Percent(percent) => {
            LetterSpacingValue::Length(Length::Percent(percent))
        }
        ComputedLetterSpacing::Calc(calc) => LetterSpacingValue::Calc(LengthPercentageCalc {
            percent: calc.percent,
            px: calc.px,
            em: 0.0,
            ch: 0.0,
        }),
    }
}

/// Lift an inherited computed `word-spacing` value back into the specified
/// staging shape without dropping percentages or mixed calc terms.
pub fn lift_word_spacing(computed: ComputedWordSpacing) -> WordSpacingValue {
    lift_letter_spacing(computed)
}

/// **Lift** the parent's [`ComputedTabSize`] into specified [`TabSize`]
/// for inheritance; as with [`lift_line_height`], this is lossless and stable.
///
/// - `Number(n)` → `TabSize::Number(n)`. The `Number` arm of
///   [`resolve_tab_size`] is the identity, so this is a fixed point.
/// - `Length(l)` → `TabSize::Length(Length::Px(l.px()))`.
///   [`resolve_length`]'s `Px` arm is the identity, so this is a fixed point
///   for the same reason as [`lift_font_size`].
///
/// ```
/// use raikiri_style::{
///     ComputedLength, ComputedTabSize, ResolveContext, lift_tab_size, resolve_tab_size,
/// };
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(16.0);
///
/// // Lift → absolutize is an identity round trip (`Px` is a fixed point).
/// let inherited = ComputedTabSize::Length(ComputedLength(32.0));
/// let lifted = lift_tab_size(inherited);
/// assert_eq!(resolve_tab_size(lifted, font_size, None, &ctx), inherited);
///
/// // `Number` also passes through as a fixed point.
/// let inherited_number = ComputedTabSize::Number(4.0);
/// assert_eq!(
///     resolve_tab_size(lift_tab_size(inherited_number), font_size, None, &ctx),
///     inherited_number
/// );
/// ```
pub fn lift_tab_size(computed: ComputedTabSize) -> TabSize {
    match computed {
        ComputedTabSize::Number(n) => TabSize::Number(n),
        ComputedTabSize::Length(l) => TabSize::Length(Length::Px(l.px())),
    }
}

/// **Lift** the parent's [`ComputedBorderSpacing`] into specified
/// [`BorderSpacingValue`] for inheritance. Like the `Length` arm of
/// [`lift_tab_size`], this is lossless and stable: [`resolve_length`]'s `Px`
/// arm is the identity and the lifted value is already clamped, so clamping
/// again is a no-op.
///
/// ```
/// use raikiri_style::{
///     ComputedBorderSpacing, ComputedLength, ResolveContext, lift_border_spacing,
///     resolve_border_spacing,
/// };
///
/// let ctx = ResolveContext::initial();
/// let font_size = ComputedLength(16.0);
///
/// // Lift → absolutize is an identity round trip (`Px` is fixed; clamping is a no-op).
/// let inherited = ComputedBorderSpacing {
///     horizontal: ComputedLength(10.0),
///     vertical: ComputedLength(20.0),
/// };
/// let lifted = lift_border_spacing(inherited);
/// assert_eq!(resolve_border_spacing(lifted, font_size, None, &ctx), inherited);
/// ```
pub fn lift_border_spacing(computed: ComputedBorderSpacing) -> BorderSpacingValue {
    BorderSpacingValue {
        horizontal: Length::Px(computed.horizontal.px()),
        vertical: Length::Px(computed.vertical.px()),
    }
}

#[cfg(test)]
mod tests;
