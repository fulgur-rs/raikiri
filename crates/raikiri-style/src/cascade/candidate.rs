//! Compact records the cascade keeps for each candidate declaration.

use crate::layer::LayerPosition;
use crate::ruletree::Origin;
use crate::specified::CascadePrecedence;

use super::collect::{RankedDecl, Specificity, cascade_rank};
use super::rollback::Rollback;

/// Where one candidate declaration stands in the cascade order, apart from
/// its value: origin and importance, cascade layer, specificity and source
/// order (CSS Cascading 5 §6.1 <https://drafts.csswg.org/css-cascade-5/#cascade-sort>).
///
/// The fields are private so that the rank always agrees with the origin and
/// importance it is derived from. The layer position is stored as its two
/// parts, which keeps the record at 16 bytes instead of the 20 a nested
/// [`LayerPosition`] (with its own padding) would take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Precedence {
    /// Origin plus `!important` precedence from [`cascade_rank`].
    rank: u8,
    origin: Origin,
    /// The importance the candidate takes, which is not always that of its
    /// declaration: animations are never important.
    important: bool,
    layer_attached: bool,
    layer_rank: u32,
    specificity: Specificity,
    source_order: u32,
}

const _: () = assert!(
    std::mem::size_of::<Precedence>() <= 16,
    "Precedence grew past 16 bytes, which candidate records count on"
);

impl Precedence {
    pub(crate) fn new(
        origin: Origin,
        important: bool,
        specificity: Specificity,
        source_order: u32,
        layer: LayerPosition,
    ) -> Self {
        Self {
            rank: cascade_rank(origin, important),
            origin,
            important,
            layer_attached: layer.attached,
            layer_rank: layer.rank,
            specificity,
            source_order,
        }
    }

    #[cfg(test)]
    pub(crate) fn origin(self) -> Origin {
        self.origin
    }

    #[cfg(test)]
    pub(crate) fn important(self) -> bool {
        self.important
    }

    pub(crate) fn layer(self) -> LayerPosition {
        LayerPosition {
            attached: self.layer_attached,
            rank: self.layer_rank,
        }
    }

    #[cfg(test)]
    pub(crate) fn specificity(self) -> Specificity {
        self.specificity
    }

    #[cfg(test)]
    pub(crate) fn source_order(self) -> u32 {
        self.source_order
    }

    /// The ranking winner selection compares, for the candidate found at
    /// `idx` of the candidates being ranked.
    pub(crate) fn ranked(self, idx: usize) -> RankedDecl {
        RankedDecl {
            rank: self.rank,
            layer_priority: self.layer().priority(self.important),
            specificity: self.specificity,
            source_order: self.source_order,
            idx,
        }
    }

    /// What [`super::rollback::select_layered_winner`] inspects about the
    /// candidate at `idx` that rolls the cascade back as `rollback` says.
    pub(crate) fn layered(
        self,
        idx: usize,
        rollback: Rollback,
    ) -> (CascadePrecedence, Origin, LayerPosition, bool, Rollback) {
        let ranked = self.ranked(idx);
        (
            (
                ranked.rank,
                ranked.layer_priority,
                ranked.specificity,
                ranked.source_order,
                idx,
            ),
            self.origin,
            self.layer(),
            self.important,
            rollback,
        )
    }
}

#[cfg(test)]
mod tests;
