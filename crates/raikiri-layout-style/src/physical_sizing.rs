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

    /// Whether the entry sets a physical inset. A positioning pass holds such
    /// an entry in a scope to put a box where its static position is; it is
    /// not a sizing override, and [`Self::apply_physical_sizing`] rejects it.
    pub fn is_physical_inset(&self) -> bool {
        use LengthProperty::*;
        matches!(
            self.as_borrowed().0,
            OverrideKind::Length(Top | Right | Bottom | Left, _)
        )
    }

    /// Whether the entry sets a physical margin. A positioning pass holds such
    /// an entry in a scope to keep an `auto` margin at zero beside insets it
    /// sets itself; like an inset it is not a sizing override.
    pub fn is_physical_margin(&self) -> bool {
        use LengthProperty::*;
        matches!(
            self.as_borrowed().0,
            OverrideKind::Length(MarginTop | MarginRight | MarginBottom | MarginLeft, _)
        )
    }

    /// Whether the entry sets physical padding. A positioning pass can hold
    /// used padding in a scope so its measurements and final placement use the
    /// same containing-block inline basis. It is not a sizing override.
    pub fn is_physical_padding(&self) -> bool {
        use LengthProperty::*;
        matches!(
            self.as_borrowed().0,
            OverrideKind::Length(PaddingTop | PaddingRight | PaddingBottom | PaddingLeft, _)
        )
    }

    /// Whether the entry sets a self-alignment. A positioning pass holds such
    /// an entry in a scope, next to the insets it sets, to align a box inside
    /// the area those insets bound; like an inset it is not a sizing override.
    pub fn is_self_alignment(&self) -> bool {
        matches!(
            self.as_borrowed().0,
            OverrideKind::AlignSelf(_) | OverrideKind::JustifySelf(_)
        )
    }

    /// Write one physical sizing override into a `BoxStyle` snapshot.
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
