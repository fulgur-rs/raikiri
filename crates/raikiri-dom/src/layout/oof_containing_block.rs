//! Containing-block selection used by fixed-position classification.

use super::fixed;
use crate::Document;
use raikiri_style::CascadeResult;

/// The containing-block target selected for a fixed-position box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainingBlockSelection {
    /// The page area, which also stands in for the initial containing block.
    Page,
    /// The padding box established by `id`.
    Box { id: usize },
    /// Inline geometry is retained by the inline layout path.
    Kept { reason: KeptReason },
}

/// Why fixed positioning remains with the inline layout path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeptReason {
    /// The containing block is a non-atomic inline box.
    Inline { id: usize },
}

fn selection_for_box(
    document: &Document,
    id: usize,
    cv: &raikiri_style::ComputedValues,
) -> ContainingBlockSelection {
    if fixed::generates_inline_box_with_properties(
        document,
        id,
        cv,
        document.nodes[id].box_properties(),
    ) {
        return ContainingBlockSelection::Kept {
            reason: KeptReason::Inline { id },
        };
    }
    ContainingBlockSelection::Box { id }
}

/// Select a fixed box's nearest containing-block target among its ancestors.
pub(crate) fn select_fixed_containing_block_from_ancestors(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> ContainingBlockSelection {
    document
        .ancestors(id)
        .find_map(|ancestor| {
            cascade
                .computed
                .get(ancestor)
                .filter(|cv| {
                    fixed::establishes_fixed_containing_block_for_layout(document, ancestor, cv)
                })
                .map(|cv| selection_for_box(document, ancestor, cv))
        })
        .unwrap_or(ContainingBlockSelection::Page)
}
