use crate::overrides::OverrideKind;
use crate::{BoxStyle, LengthProperty, OverrideError, OwnedPropertyOverride};

impl OwnedPropertyOverride {
    /// Whether [`Self::apply_physical_sizing`] accepts this entry.
    pub fn is_physical_sizing(&self) -> bool {
        use LengthProperty::*;
        match self.as_borrowed().0 {
            OverrideKind::Length(property, _) => matches!(
                property,
                Width
                    | Height
                    | MinWidth
                    | MinHeight
                    | MaxWidth
                    | MaxHeight
                    | BorderTop
                    | BorderRight
                    | BorderBottom
                    | BorderLeft
            ),
            OverrideKind::FlexShrink(_) | OverrideKind::AspectRatio(_) => true,
            _ => false,
        }
    }

    /// Write one physical sizing override into a [`BoxStyle`] snapshot.
    ///
    /// Only the affected field is replaced, and the previous value is not
    /// kept: apply entries to a clone the caller owns for one measurement, so
    /// that session state never sees the change.
    /// The caller must retain calc owners before publishing their raw handles,
    /// and keep them alive while the snapshot is read. Applying an entry does
    /// not transfer ownership into the snapshot.
    ///
    /// # Errors
    /// Rejects properties outside the physical sizing set.
    pub fn apply_physical_sizing(&self, style: &mut BoxStyle) -> Result<(), OverrideError> {
        match self.as_borrowed().0 {
            OverrideKind::Length(LengthProperty::Width, value) => {
                style.size.width = value.dimension();
            }
            OverrideKind::Length(LengthProperty::Height, value) => {
                style.size.height = value.dimension();
            }
            OverrideKind::Length(LengthProperty::MinWidth, value) => {
                style.min_size.width = value.auto(false);
            }
            OverrideKind::Length(LengthProperty::MinHeight, value) => {
                style.min_size.height = value.auto(false);
            }
            OverrideKind::Length(LengthProperty::MaxWidth, value) => {
                style.max_size.width = value.auto(false);
            }
            OverrideKind::Length(LengthProperty::MaxHeight, value) => {
                style.max_size.height = value.auto(false);
            }
            // A box style holds no flex properties, so the probe's shrink factor
            // is accepted and changes nothing.
            OverrideKind::FlexShrink(_) => {}
            OverrideKind::Length(LengthProperty::BorderTop, value) => {
                style.border.top = value.lp(crate::PercentBasis::Taffy);
            }
            OverrideKind::Length(LengthProperty::BorderRight, value) => {
                style.border.right = value.lp(crate::PercentBasis::Taffy);
            }
            OverrideKind::Length(LengthProperty::BorderBottom, value) => {
                style.border.bottom = value.lp(crate::PercentBasis::Taffy);
            }
            OverrideKind::Length(LengthProperty::BorderLeft, value) => {
                style.border.left = value.lp(crate::PercentBasis::Taffy);
            }
            OverrideKind::AspectRatio(value) => style.aspect_ratio = value,
            _ => return Err(OverrideError::UnsupportedPhysicalSizing),
        }
        Ok(())
    }
}
