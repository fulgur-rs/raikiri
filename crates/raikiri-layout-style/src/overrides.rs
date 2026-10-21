use crate::{AlignedCalc, Axis, LengthProperty};
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    property::OverflowValue,
};
use std::sync::Arc;

/// A length override with an explicit calc owner or resolution tag.
#[derive(Clone, Copy, Debug)]
pub struct LengthValue<'a> {
    pub(crate) value: L,
    pub(crate) payload: Option<&'a AlignedCalc>,
    pub(crate) resolved: bool,
}

/// An invalid override rejected before it reaches a getter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverrideError {
    /// A preparation-only source is not a physical override target.
    PreparationOnly,
    /// A computed calc needs an aligned payload rather than an inline expression.
    UnpreparedCalc,
    /// The payload uses another property's range.
    WrongRange,
    /// An already resolved value must be a physical pixel length.
    ResolvedNonPixel,
}
impl std::fmt::Display for OverrideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid length override: {self:?}")
    }
}
impl std::error::Error for OverrideError {}
impl<'a> LengthValue<'a> {
    /// A computed length without a calc payload.
    ///
    /// # Errors
    /// Rejects inline calc expressions. Use the calc constructor for those.
    pub fn computed(value: L) -> Result<Self, OverrideError> {
        if matches!(value, L::Calc(_) | L::FitContentOf(P::Calc(_))) {
            return Err(OverrideError::UnpreparedCalc);
        }
        Ok(Self {
            value,
            payload: None,
            resolved: false,
        })
    }
    /// A calc whose owner is borrowed for the override's lifetime.
    pub fn calc(payload: &'a AlignedCalc) -> Self {
        Self {
            value: L::Calc(payload.expression()),
            payload: Some(payload),
            resolved: false,
        }
    }
    /// A value already resolved in physical pixels.
    pub const fn resolved(px: f32) -> Self {
        Self {
            value: L::Px(px),
            payload: None,
            resolved: true,
        }
    }
}

/// A sparse override entry; later entries win within each layer.
#[derive(Clone, Copy, Debug)]
pub struct PropertyOverride<'a>(pub(crate) OverrideKind<'a>);
#[derive(Clone, Copy, Debug)]
pub(crate) enum OverrideKind<'a> {
    Length(LengthProperty, LengthValue<'a>),
    AspectRatio(Option<f32>),
    Overflow(Axis, OverflowValue),
    FlexShrink(f32),
    AlignSelf(Option<taffy::AlignSelf>),
    JustifySelf(Option<taffy::AlignSelf>),
}
impl<'a> PropertyOverride<'a> {
    /// Override one physical length property.
    ///
    /// # Errors
    /// Rejects a calc payload whose range differs from the target property.
    /// Also rejects preparation-only logical source slots.
    pub fn length(property: LengthProperty, value: LengthValue<'a>) -> Result<Self, OverrideError> {
        if matches!(
            property,
            LengthProperty::LogicalWidth | LengthProperty::LogicalHeight | LengthProperty::MinBlock
        ) {
            return Err(OverrideError::PreparationOnly);
        }
        if value
            .payload
            .is_some_and(|p| p.non_negative() != property.non_negative())
        {
            return Err(OverrideError::WrongRange);
        }
        Ok(Self(OverrideKind::Length(property, value)))
    }
    /// Override the ratio, including explicitly disabling it.
    pub const fn aspect_ratio(value: Option<f32>) -> Self {
        Self(OverrideKind::AspectRatio(value))
    }
    /// Override physical overflow.
    pub const fn overflow(axis: Axis, value: OverflowValue) -> Self {
        Self(OverrideKind::Overflow(axis, value))
    }
    /// Override flex shrink during an intrinsic probe.
    pub const fn flex_shrink(value: f32) -> Self {
        Self(OverrideKind::FlexShrink(value))
    }
    /// Override self alignment, including explicit auto.
    pub const fn align_self(value: Option<taffy::AlignSelf>) -> Self {
        Self(OverrideKind::AlignSelf(value))
    }
    /// Override inline self alignment, including explicit auto.
    pub const fn justify_self(value: Option<taffy::AlignSelf>) -> Self {
        Self(OverrideKind::JustifySelf(value))
    }
}

/// A sparse override that owns any calc payload it introduces.
///
/// Borrowed projections never clone the Arc or allocate while querying.
#[derive(Clone, Debug)]
pub struct OwnedPropertyOverride(OwnedOverrideKind);
#[derive(Clone, Debug)]
enum OwnedOverrideKind {
    Inline(PropertyOverride<'static>),
    Length {
        property: LengthProperty,
        value: L,
        payload: Option<Arc<AlignedCalc>>,
        resolved: bool,
    },
}
impl OwnedPropertyOverride {
    /// Construct a physical length override, preparing a calc owner if needed.
    /// `resolved` marks values already resolved in physical pixels.
    ///
    /// # Errors
    /// Rejects preparation-only logical slots and non-pixel resolved values.
    pub fn length(
        property: LengthProperty,
        value: L,
        resolved: bool,
    ) -> Result<Self, OverrideError> {
        if resolved && !matches!(value, L::Px(_)) {
            return Err(OverrideError::ResolvedNonPixel);
        }
        let payload = match value {
            L::Calc(expression) | L::FitContentOf(P::Calc(expression)) => Some(Arc::new(
                AlignedCalc::new(expression, property.non_negative()),
            )),
            _ => None,
        };
        PropertyOverride::length(
            property,
            LengthValue {
                value,
                payload: payload.as_deref(),
                resolved,
            },
        )?;
        Ok(Self(OwnedOverrideKind::Length {
            property,
            value,
            payload,
            resolved,
        }))
    }

    /// Borrow a payload owner for retention before an adapter publishes raw handles.
    pub fn calc_owner(&self) -> Option<&Arc<AlignedCalc>> {
        match &self.0 {
            OwnedOverrideKind::Length { payload, .. } => payload.as_ref(),
            OwnedOverrideKind::Inline(_) => None,
        }
    }

    /// Borrow the entry without cloning its payload owner.
    pub fn as_borrowed(&self) -> PropertyOverride<'_> {
        match &self.0 {
            OwnedOverrideKind::Inline(entry) => *entry,
            OwnedOverrideKind::Length {
                property,
                value,
                payload,
                resolved,
            } => PropertyOverride(OverrideKind::Length(
                *property,
                LengthValue {
                    value: *value,
                    payload: payload.as_deref(),
                    resolved: *resolved,
                },
            )),
        }
    }
    /// Override the ratio, including explicitly disabling it.
    pub const fn aspect_ratio(value: Option<f32>) -> Self {
        Self(OwnedOverrideKind::Inline(PropertyOverride::aspect_ratio(
            value,
        )))
    }
    /// Override physical overflow.
    pub const fn overflow(axis: Axis, value: OverflowValue) -> Self {
        Self(OwnedOverrideKind::Inline(PropertyOverride::overflow(
            axis, value,
        )))
    }
    /// Override flex shrink during an intrinsic probe.
    pub const fn flex_shrink(value: f32) -> Self {
        Self(OwnedOverrideKind::Inline(PropertyOverride::flex_shrink(
            value,
        )))
    }
    /// Override self alignment, including explicit auto.
    pub const fn align_self(value: Option<taffy::AlignSelf>) -> Self {
        Self(OwnedOverrideKind::Inline(PropertyOverride::align_self(
            value,
        )))
    }
    /// Override inline self alignment, including explicit auto.
    pub const fn justify_self(value: Option<taffy::AlignSelf>) -> Self {
        Self(OwnedOverrideKind::Inline(PropertyOverride::justify_self(
            value,
        )))
    }
}
