use crate::overrides::OverrideKind;
use crate::{LengthProperty, OverrideError, OwnedPropertyOverride};
use raikiri_style::SmolStr;
use taffy::{Dimension, LengthPercentageAuto, Style};

/// The previous value of one field in the temporary sizing compatibility layer.
#[must_use]
pub struct LegacySizingUndo(SizingField);

enum SizingField {
    Width(Dimension),
    Height(Dimension),
    MinWidth(LengthPercentageAuto),
    MinHeight(LengthPercentageAuto),
    MaxWidth(LengthPercentageAuto),
    MaxHeight(LengthPercentageAuto),
    AspectRatio(Option<f32>),
}

impl LegacySizingUndo {
    /// Restore the affected field while retaining unrelated changes.
    pub fn restore(self, style: &mut Style<SmolStr>) {
        match self.0 {
            SizingField::Width(value) => style.size.width = value,
            SizingField::Height(value) => style.size.height = value,
            SizingField::MinWidth(value) => style.min_size.width = value,
            SizingField::MinHeight(value) => style.min_size.height = value,
            SizingField::MaxWidth(value) => style.max_size.width = value,
            SizingField::MaxHeight(value) => style.max_size.height = value,
            SizingField::AspectRatio(value) => style.aspect_ratio = value,
        }
    }
}

impl OwnedPropertyOverride {
    /// Apply a physical sizing entry to an existing transposed snapshot.
    ///
    /// The adapter retains calc owners under the same lifetime contract as
    /// [`Self::apply_legacy_sizing`]. Only the affected field is replaced.
    ///
    /// # Errors
    /// Rejects properties outside the temporary sizing compatibility layer.
    pub fn apply_transposed_legacy_sizing(
        &self,
        style: &mut Style<SmolStr>,
    ) -> Result<LegacySizingUndo, OverrideError> {
        use crate::LengthProperty::*;
        let kind = match self.as_borrowed().0 {
            OverrideKind::Length(property, value) => OverrideKind::Length(
                match property {
                    Width => Height,
                    Height => Width,
                    MinWidth => MinHeight,
                    MinHeight => MinWidth,
                    MaxWidth => MaxHeight,
                    MaxHeight => MaxWidth,
                    _ => return Err(OverrideError::UnsupportedLegacySizing),
                },
                value,
            ),
            OverrideKind::AspectRatio(ratio) => {
                OverrideKind::AspectRatio(ratio.map(|ratio| 1.0 / ratio))
            }
            _ => return Err(OverrideError::UnsupportedLegacySizing),
        };
        apply_sizing(kind, style)
    }

    /// Apply one physical sizing override for legacy layout consumers.
    ///
    /// This compatibility layer can be removed when those consumers query views.
    /// The adapter must retain calc owners before publishing their raw handles,
    /// and keep them alive while either the applied or restored values are read.
    /// Applying an entry does not transfer ownership into the legacy style.
    ///
    /// # Errors
    /// Rejects properties outside the temporary sizing compatibility layer.
    pub fn apply_legacy_sizing(
        &self,
        style: &mut Style<SmolStr>,
    ) -> Result<LegacySizingUndo, OverrideError> {
        apply_sizing(self.as_borrowed().0, style)
    }
}

fn apply_sizing(
    kind: OverrideKind<'_>,
    style: &mut Style<SmolStr>,
) -> Result<LegacySizingUndo, OverrideError> {
    use std::mem::replace;

    let previous = match kind {
        OverrideKind::Length(LengthProperty::Width, value) => {
            SizingField::Width(replace(&mut style.size.width, value.dimension()))
        }
        OverrideKind::Length(LengthProperty::Height, value) => {
            SizingField::Height(replace(&mut style.size.height, value.dimension()))
        }
        OverrideKind::Length(LengthProperty::MinWidth, value) => {
            SizingField::MinWidth(replace(&mut style.min_size.width, value.auto(false)))
        }
        OverrideKind::Length(LengthProperty::MinHeight, value) => {
            SizingField::MinHeight(replace(&mut style.min_size.height, value.auto(false)))
        }
        OverrideKind::Length(LengthProperty::MaxWidth, value) => {
            SizingField::MaxWidth(replace(&mut style.max_size.width, value.auto(false)))
        }
        OverrideKind::Length(LengthProperty::MaxHeight, value) => {
            SizingField::MaxHeight(replace(&mut style.max_size.height, value.auto(false)))
        }
        OverrideKind::AspectRatio(value) => {
            SizingField::AspectRatio(replace(&mut style.aspect_ratio, value))
        }
        _ => return Err(OverrideError::UnsupportedLegacySizing),
    };
    Ok(LegacySizingUndo(previous))
}
