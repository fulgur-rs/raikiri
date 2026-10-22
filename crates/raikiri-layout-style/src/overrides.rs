use crate::{AlignedCalc, Axis, LengthProperty};
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P,
    property::{ClearValue, OverflowValue},
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
    /// The property is outside the temporary legacy sizing compatibility layer.
    UnsupportedLegacySizing,
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
    Clear(ClearValue),
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
    /// Override clearance for a fragmentation pass.
    pub const fn clear(value: ClearValue) -> Self {
        Self(OverrideKind::Clear(value))
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
/// A reversible update of one sparse override property.
///
/// Restoring this token preserves changes to every other property.
#[derive(Debug)]
pub struct OwnedOverrideUndo {
    key: OwnedPropertyOverride,
    previous: Option<OwnedPropertyOverride>,
}
impl OwnedOverrideUndo {
    /// Restore only the property captured by this token.
    pub fn restore(self, entries: &mut Vec<OwnedPropertyOverride>) {
        if let Some(previous) = self.previous {
            previous.install(entries);
        } else if let Some(index) = entries
            .iter()
            .position(|entry| self.key.same_property(entry))
        {
            entries.remove(index);
        }
    }
}

impl OwnedPropertyOverride {
    /// Replace one property, retaining the previous entry for a later restore.
    pub fn replace_in(&self, entries: &mut Vec<Self>) -> OwnedOverrideUndo {
        let key = self.clone();
        let replacement = self.clone();
        let previous =
            if let Some(entry) = entries.iter_mut().find(|entry| self.same_property(entry)) {
                Some(std::mem::replace(entry, replacement))
            } else {
                entries.push(replacement);
                None
            };
        OwnedOverrideUndo { key, previous }
    }

    /// Override clearance for a fragmentation pass.
    pub const fn clear(value: ClearValue) -> Self {
        Self(OwnedOverrideKind::Inline(PropertyOverride::clear(value)))
    }

    /// Replace the entry for this property, preserving a sparse pass layer.
    pub fn install(self, entries: &mut Vec<Self>) {
        if let Some(previous) = entries.iter_mut().find(|entry| self.same_property(entry)) {
            *previous = self;
        } else {
            entries.push(self);
        }
    }

    fn same_property(&self, other: &Self) -> bool {
        use OverrideKind::*;
        match (self.as_borrowed().0, other.as_borrowed().0) {
            (Length(a, _), Length(b, _)) => a == b,
            (Overflow(a, _), Overflow(b, _)) => a == b,
            (AspectRatio(_), AspectRatio(_))
            | (Clear(_), Clear(_))
            | (FlexShrink(_), FlexShrink(_))
            | (AlignSelf(_), AlignSelf(_))
            | (JustifySelf(_), JustifySelf(_)) => true,
            _ => false,
        }
    }

    /// Project a physical override into the transitional retained style.
    ///
    /// Scalar conversion is shared with the view getters. The adapter must retain
    /// calc owners before publishing the resulting raw handles.
    pub fn apply_to_style(&self, style: &mut taffy::Style<raikiri_style::SmolStr>) {
        use LengthProperty::*;
        match self.as_borrowed().0 {
            OverrideKind::Length(property, value) => match property {
                Width => style.size.width = value.dimension(),
                Height => style.size.height = value.dimension(),
                MinWidth => style.min_size.width = value.auto(false),
                MinHeight => style.min_size.height = value.auto(false),
                MaxWidth => style.max_size.width = value.auto(false),
                MaxHeight => style.max_size.height = value.auto(false),
                MarginTop => style.margin.top = value.auto(true),
                MarginRight => style.margin.right = value.auto(true),
                MarginBottom => style.margin.bottom = value.auto(true),
                MarginLeft => style.margin.left = value.auto(true),
                PaddingTop => style.padding.top = value.lp(crate::PercentBasis::Taffy),
                PaddingRight => style.padding.right = value.lp(crate::PercentBasis::Taffy),
                PaddingBottom => style.padding.bottom = value.lp(crate::PercentBasis::Taffy),
                PaddingLeft => style.padding.left = value.lp(crate::PercentBasis::Taffy),
                BorderTop => style.border.top = value.lp(crate::PercentBasis::Taffy),
                BorderRight => style.border.right = value.lp(crate::PercentBasis::Taffy),
                BorderBottom => style.border.bottom = value.lp(crate::PercentBasis::Taffy),
                BorderLeft => style.border.left = value.lp(crate::PercentBasis::Taffy),
                Top => style.inset.top = value.auto(false),
                Right => style.inset.right = value.auto(false),
                Bottom => style.inset.bottom = value.auto(false),
                Left => style.inset.left = value.auto(false),
                FlexBasis => style.flex_basis = value.dimension(),
                ColumnGap => style.gap.width = value.lp(crate::PercentBasis::Taffy),
                RowGap => style.gap.height = value.lp(crate::PercentBasis::Taffy),
                // cov:ignore: constructors reject preparation-only override targets.
                LogicalWidth | LogicalHeight | MinBlock => {
                    unreachable!("validated physical override")
                }
            },
            OverrideKind::AspectRatio(value) => style.aspect_ratio = value,
            OverrideKind::Overflow(axis, value) => {
                let target = match axis {
                    Axis::X => &mut style.overflow.x,
                    Axis::Y => &mut style.overflow.y,
                };
                *target = crate::core::overflow(value);
            }
            OverrideKind::Clear(value) => style.clear = crate::core::clear(value),
            OverrideKind::FlexShrink(value) => style.flex_shrink = value,
            OverrideKind::AlignSelf(value) => style.align_self = value,
            OverrideKind::JustifySelf(value) => style.justify_self = value,
        }
    }

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
