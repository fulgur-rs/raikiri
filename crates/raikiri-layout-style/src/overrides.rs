use crate::{AlignedCalc, Axis, LengthProperty};
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    property::OverflowValue,
};

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
    /// A computed calc needs an aligned payload rather than an inline expression.
    UnpreparedCalc,
    /// The payload uses another property's range.
    WrongRange,
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
    pub fn length(property: LengthProperty, value: LengthValue<'a>) -> Result<Self, OverrideError> {
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
