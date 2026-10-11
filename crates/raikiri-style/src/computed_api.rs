//! The computed-value types a painter reads from [`ComputedValues`], gathered
//! under computed-layer names.
//!
//! [`ComputedValues`] mixes two kinds of field types. Some have a dedicated
//! computed representation (the `Computed*` structs in this crate's resolve
//! layer). Others reuse a type from [`crate::property`] because the computed
//! value has the same shape as the declared one: keywords such as `visibility`
//! or `text-transform`, colors, and the background-image and filter payloads.
//! This module names every field type a painter needs without going through
//! [`crate::property`], so a downstream crate can depend on the computed layer
//! only:
//!
//! - Types with their own computed representation are re-exported unchanged.
//! - Field types shared with the declared layer get a `Computed*` type alias.
//!   Enum variants work through the alias in patterns and expressions
//!   (`ComputedVisibility::Hidden`); a glob `use` of the variants does not, so
//!   write them qualified. The enums are `#[non_exhaustive]`, so a match outside
//!   this crate needs a wildcard arm.
//! - Types that only appear inside those payloads (gradient stops, border and
//!   outline colors, shadow lengths, ...) are re-exported under their own
//!   names, which are layer-neutral.
//!
//! Each alias documents what the computed value guarantees beyond its type.
//! Where a payload type is wider than the values the cascade produces (gradient
//! lengths), a later release may replace the alias with a dedicated computed
//! type; that is a breaking change for code that names the variant payloads.

pub use crate::computed::{ComputedBorderImage, ComputedValues};
pub use crate::property::{CornerRadius, CssColor, Sides};
pub use crate::resolve::{
    ComputedBackgroundSize, ComputedBorder, ComputedBorderRadius, ComputedBoxShadowItem,
    ComputedCssPosition, ComputedCssPositionOffset, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedOutline, ComputedTextDecorationInset,
    ComputedTextDecorationThickness, ComputedTextShadow, ComputedTransformFunction,
};

// Payload types that appear inside the computed field types above and below.
pub use crate::property::{
    Angle, AnglePercentage, AngularColorStop, BorderColor, BorderImageOutsetSide,
    BorderImageRepeat, BorderImageRepeatKeyword, BorderImageSlice, BorderImageSliceOffset,
    BorderImageWidthSide, BorderStyle, CalcLengthPercentage, ConicGradient, CssPosition,
    CssPositionOffset, GradientColorInterpolation, GradientColorStop, GradientStopColor,
    HorizontalSide, Length, LinearGradient, LinearGradientDirection, OutlineColor, OutlineStyle,
    OverflowValue, RadialExtent, RadialGradient, RadialShape, RadialSize, SideOrCorner,
    TextShadowColor, TextShadowItem, TextShadowLength, VerticalSide,
};

use crate::property;

/// Computed `display` ([`ComputedValues::display`]).
pub type ComputedDisplay = property::DisplayValue;

/// Computed `visibility` ([`ComputedValues::visibility`]).
pub type ComputedVisibility = property::Visibility;

/// Computed `direction` ([`ComputedValues::direction`]): the inline base
/// direction property, not the `:dir()` directionality of [`crate::Direction`].
pub type ComputedDirection = property::Direction;

/// Computed `white-space` ([`ComputedValues::white_space`]).
pub type ComputedWhiteSpace = property::WhiteSpace;

/// Computed `white-space-collapse` ([`ComputedValues::white_space_collapse`]).
pub type ComputedWhiteSpaceCollapse = property::WhiteSpaceCollapse;

/// Computed `text-transform` ([`ComputedValues::text_transform`]).
pub type ComputedTextTransform = property::TextTransform;

/// Computed `text-decoration-line` ([`ComputedValues::text_decoration_line`]).
pub type ComputedTextDecorationLine = property::TextDecorationLine;

/// Computed `text-decoration-style` ([`ComputedValues::text_decoration_style`]).
pub type ComputedTextDecorationStyle = property::TextDecorationStyle;

/// Computed `text-decoration-color` ([`ComputedValues::text_decoration_color`]).
///
/// `CurrentColor` is the computed value CSS Color 4 specifies for
/// `currentcolor`; a painter resolves it against [`ComputedValues::color`].
/// The same holds for the `CurrentColor` variants of [`BorderColor`],
/// [`OutlineColor`] and [`TextShadowColor`].
pub type ComputedTextDecorationColor = property::TextDecorationColor;

/// Computed `overflow` ([`ComputedValues::overflow`]), one value per axis.
pub type ComputedOverflow = property::OverflowXY;

/// Computed `background-repeat` ([`ComputedValues::background_repeat`]).
pub type ComputedBackgroundRepeat = property::BackgroundRepeat;

/// Computed `background-attachment` ([`ComputedValues::background_attachment`]).
pub type ComputedBackgroundAttachment = property::BackgroundAttachment;

/// Computed `<visual-box>` of `background-clip` and `background-origin`
/// ([`ComputedValues::background_clip`], [`ComputedValues::background_origin`]).
pub type ComputedVisualBox = property::VisualBox;

/// Computed `background-image` ([`ComputedValues::background_image`]) and
/// `list-style-image` ([`ComputedValues::list_style_image`]).
///
/// `Url` holds the URL as written; resolving it against a base URL is the
/// consumer's job. Gradient payloads are absolutized by the cascade: every
/// length inside them is [`Length::Px`] or [`Length::Percent`], with
/// percentages kept relative to the gradient box.
pub type ComputedBackgroundImage = property::BackgroundImage;

/// The gradient payload of [`ComputedBackgroundImage::Gradient`]; see that
/// alias for the length guarantee.
pub type ComputedGradient = property::Gradient;

/// Computed `filter` ([`ComputedValues::filter`]).
///
/// Filter Effects 1 gives `filter` an as-specified computed value, so the
/// functions keep their declared lengths and colors.
pub type ComputedFilterFunction = property::FilterFunction;

/// Computed `mix-blend-mode` ([`ComputedValues::mix_blend_mode`]).
pub type ComputedMixBlendMode = property::MixBlendMode;

/// Computed `isolation` ([`ComputedValues::isolation`]).
pub type ComputedIsolation = property::Isolation;

/// Computed `clip-path` ([`ComputedValues::clip_path`]).
pub type ComputedClipPath = property::ClipPath;

/// Computed `list-style-type` ([`ComputedValues::list_style_type`]).
pub type ComputedListStyleType = property::ListStyleType;

/// Computed `list-style-position` ([`ComputedValues::list_style_position`]).
pub type ComputedListStylePosition = property::ListStylePosition;

/// Computed `column-span`: a non-inherited keyword, initially `none`.
pub type ComputedColumnSpan = crate::property::ColumnSpanValue; // cov:ignore: a computed type alias emits no executable instructions.
