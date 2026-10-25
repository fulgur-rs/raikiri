use super::{
    AlignedCalc, CalcLengthPercentage, CalcSlot, ComputedValues, PrepareError, PreparedStyle,
};
use std::sync::Arc;

/// Immutable calc bindings that own payloads without retaining computed values.
///
/// Slots come from the computed-property traversal, once per property or track
/// component. Each slot lookup uses binary search during validation and getters.
#[derive(Debug)]
#[doc(hidden)]
pub struct PreparedData {
    bindings: Box<[(CalcSlot, Arc<AlignedCalc>)]>,
}

impl PreparedData {
    /// Retain factory-supplied payloads for every calc slot in `values`.
    /// The factory runs during preparation, never from a layout getter.
    ///
    /// # Errors
    /// Rejects any payload whose expression bits or range differ from its slot.
    pub fn prepare_with(
        values: &ComputedValues,
        mut owner: impl FnMut(CalcSlot, CalcLengthPercentage, bool) -> Arc<AlignedCalc>,
    ) -> Result<Self, PrepareError> {
        let mut bindings = Vec::new();
        PreparedStyle::visit_values_calcs(values, |slot, expression, range| {
            let payload = owner(slot, expression, range);
            if !payload.matches(expression, range) {
                return Err(PrepareError::Mismatch(slot));
            }
            bindings.push((slot, payload));
            Ok(())
        })?;
        bindings.sort_unstable_by_key(|(slot, _)| slot.search_key());
        Ok(Self {
            bindings: bindings.into_boxed_slice(),
        })
    }

    /// Borrow calc owners when capturing inputs that can outlive their binding.
    pub fn owners(&self) -> impl ExactSizeIterator<Item = &Arc<AlignedCalc>> {
        self.bindings.iter().map(|(_, owner)| owner)
    }

    pub(super) fn payload(&self, slot: CalcSlot) -> Option<&AlignedCalc> {
        let index = self
            .bindings
            .binary_search_by_key(&slot.search_key(), |(slot, _)| slot.search_key())
            .ok()?;
        Some(&self.bindings[index].1)
    }

    pub(super) fn validate(&self, values: &ComputedValues) -> Result<(), PrepareError> {
        let mut count = 0;
        PreparedStyle::visit_values_calcs(values, |slot, expression, range| {
            count += 1;
            let payload = self.payload(slot).ok_or(PrepareError::Missing(slot))?;
            if !payload.matches(expression, range) {
                return Err(PrepareError::Mismatch(slot));
            }
            Ok(())
        })?;
        if count != self.bindings.len() {
            return Err(PrepareError::Unexpected);
        }
        Ok(())
    }
}
