//! Shared selection and geometry for out-of-flow containing blocks.

use super::fixed;
use super::*;
use crate::layout::context::LayoutSession;
use raikiri_style::{CascadeResult, ComputedValues};
use taffy::{Direction as TaffyDirection, Point, Position as TaffyPosition, Size};

/// The selected containing-block target, independent of resolved geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainingBlockSelection {
    /// The page area, which also stands in for the initial containing block.
    Page,
    /// The padding box established by `id`.
    Box { id: usize },
    /// The existing layout is retained because the target is outside the
    /// current traversal root or has inline geometry.
    Kept { reason: KeptReason },
}

/// Why the correction pass must preserve the existing containing block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeptReason {
    /// The containing block is a non-atomic inline box.
    Inline { id: usize },
    /// The containing block is outside the traversal root.
    AncestorOutsideRoot { id: usize },
}

/// A containing block resolved for layout.
#[derive(Clone, Copy)]
pub(crate) enum ContainingBlock {
    /// The page area, which also stands in for the initial containing block.
    Page,
    /// The padding box of `id`, resolved in the current layout coordinate space.
    Box {
        id: usize,
        origin: Point<f32>,
        size: Size<f32>,
        direction: TaffyDirection,
    },
    /// The existing layout is retained for an inline or out-of-root target.
    Kept { reason: KeptReason },
}

impl ContainingBlock {
    /// Return this resolved area's target without requiring geometry.
    pub(crate) fn selection(self) -> ContainingBlockSelection {
        match self {
            Self::Page => ContainingBlockSelection::Page,
            Self::Box { id, .. } => ContainingBlockSelection::Box { id },
            Self::Kept { reason } => ContainingBlockSelection::Kept { reason },
        }
    }
}

/// An out-of-flow positioning area resolved for layout.
#[derive(Clone, Copy)]
pub(super) struct Area {
    pub(super) origin: Point<f32>,
    pub(super) size: Size<f32>,
    pub(super) direction: TaffyDirection,
    pub(super) vertical: bool,
}

/// Whether the writing mode `id` lays its lines out in is vertical.
pub(super) fn is_vertical(document: &Document, id: usize) -> bool {
    crate::taffy_impl::used_writing_mode(document, id).is_vertical()
}

fn selection_for_box(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
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

/// Select absolute and fixed containing-block targets inherited below `root`.
pub(crate) fn select_root_containing_blocks(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> (ContainingBlockSelection, ContainingBlockSelection) {
    let root_cv = cascade.computed.get(root);
    let fixed_cb = match root_cv {
        Some(cv) if fixed::establishes_fixed_containing_block_for_layout(document, root, cv) => {
            selection_for_box(document, root, cv)
        }
        _ => ContainingBlockSelection::Page,
    };
    // Preserve the current root-boundary rule for absolute positioning. A
    // containing ancestor outside this traversal root keeps its existing layout.
    let abs_cb = match root_cv {
        Some(cv) if fixed::establishes_absolute_containing_block(document, root, cv) => {
            selection_for_box(document, root, cv)
        }
        _ => document
            .ancestors(root)
            .find(|id| {
                cascade.computed.get(*id).is_some_and(|cv| {
                    fixed::establishes_absolute_containing_block(document, *id, cv)
                })
            })
            .map_or(ContainingBlockSelection::Page, |id| {
                ContainingBlockSelection::Kept {
                    reason: KeptReason::AncestorOutsideRoot { id },
                }
            }),
    };

    (abs_cb, fixed_cb)
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

/// Select absolute and fixed containing-block targets inherited below `id`.
pub(crate) fn select_descendant_containing_blocks(
    document: &Document,
    id: usize,
    cv: &ComputedValues,
    abs_cb: ContainingBlockSelection,
    fixed_cb: ContainingBlockSelection,
) -> (ContainingBlockSelection, ContainingBlockSelection) {
    // The box's own transform or filter contains its descendants, not the box itself.
    let contains_fixed = fixed::establishes_fixed_containing_block_for_layout(document, id, cv);
    let abs_cb = if contains_fixed || fixed::establishes_absolute_containing_block(document, id, cv)
    {
        selection_for_box(document, id, cv)
    } else {
        abs_cb
    };
    let fixed_cb = if contains_fixed { abs_cb } else { fixed_cb };
    (abs_cb, fixed_cb)
}

/// Resolve a selected target using the layout geometry at its establishing box.
pub(crate) fn resolve_containing_block_selection(
    document: &Document,
    cascade: &CascadeResult,
    selection: ContainingBlockSelection,
    origin: Point<f32>,
) -> ContainingBlock {
    match selection {
        ContainingBlockSelection::Page => ContainingBlock::Page,
        ContainingBlockSelection::Kept { reason } => ContainingBlock::Kept { reason },
        ContainingBlockSelection::Box { id } => {
            let cv = cascade
                .computed
                .get(id)
                .expect("a box containing-block target has computed values");
            let layout = document.nodes[id].unrounded_layout;
            let border = layout.border;
            ContainingBlock::Box {
                id,
                origin: Point {
                    x: origin.x + border.left,
                    y: origin.y + border.top,
                },
                size: Size {
                    width: (layout.size.width - border.left - border.right).max(0.0),
                    height: (layout.size.height - border.top - border.bottom).max(0.0),
                },
                direction: if cv.direction == Direction::Rtl {
                    TaffyDirection::Rtl
                } else {
                    TaffyDirection::Ltr
                },
            }
        }
    }
}

/// Visit every fixed or absolutely positioned box below the layout root,
/// before walking its descendants, with its selected containing block.
///
/// The callback runs before descendant containing blocks are resolved, so a
/// callback that relays out the current box is reflected in nested geometry.
pub(crate) fn walk_out_of_flow_boxes(
    document: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    root: usize,
    mut visit: impl FnMut(&mut LayoutSession<'_>, usize, Point<f32>, ContainingBlock),
) {
    let root_origin = document
        .inclusive_ancestors(root)
        .fold(Point::ZERO, |origin, id| {
            let location = document.nodes[id].unrounded_layout.location;
            Point {
                x: origin.x + location.x,
                y: origin.y + location.y,
            }
        });
    let (abs_selection, fixed_selection) = select_root_containing_blocks(document, cascade, root);
    let abs_cb = resolve_containing_block_selection(document, cascade, abs_selection, root_origin);
    let fixed_cb =
        resolve_containing_block_selection(document, cascade, fixed_selection, root_origin);
    // (node, its location parent origin, paragraph state, absolute CB, fixed CB)
    let in_paragraph = document.nodes[root].is_ifc_root();
    let mut stack: Vec<(usize, Point<f32>, bool, ContainingBlock, ContainingBlock)> = document
        .nodes[root]
        .children
        .iter()
        .rev()
        .map(|&child| (child, root_origin, in_paragraph, abs_cb, fixed_cb))
        .collect();

    while let Some((id, parent_origin, in_paragraph, abs_cb, fixed_cb)) = stack.pop() {
        let node = &document.layout_node_with_style(id).unwrap();
        if !node.is_in_document() || node.is_display_none() {
            continue;
        }
        let Some(cv) = cascade.computed.get(id) else {
            continue; // cov:ignore: the cascade has a value for every arena node
        };
        let out_of_flow = node.taffy_position() == TaffyPosition::Absolute;
        let cb = match node.position() {
            PositionValue::Fixed => Some(fixed_cb),
            PositionValue::Absolute => Some(abs_cb),
            _ => None,
        };
        if out_of_flow && let Some(cb) = cb {
            visit(document, id, parent_origin, cb);
        }

        let node = &document.layout_node_with_style(id).unwrap();
        // The inline engine places paragraph boxes relative to the paragraph,
        // not to the inline elements between them.
        let inline = in_paragraph
            && !node.is_ifc_root()
            && node.display().is_inline_flow()
            && !matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            );
        let (origin, in_paragraph) = if inline {
            (parent_origin, true)
        } else {
            let location = node.unrounded_layout.location;
            let origin = Point {
                x: parent_origin.x + location.x,
                y: parent_origin.y + location.y,
            };
            (origin, node.is_ifc_root())
        };
        let (previous_abs_selection, previous_fixed_selection) =
            (abs_cb.selection(), fixed_cb.selection());
        let (abs_selection, fixed_selection) = select_descendant_containing_blocks(
            document,
            id,
            cv,
            previous_abs_selection,
            previous_fixed_selection,
        );
        let abs_cb = if abs_selection == previous_abs_selection {
            abs_cb
        } else {
            resolve_containing_block_selection(document, cascade, abs_selection, origin)
        };
        let fixed_cb = if fixed_selection == previous_fixed_selection {
            fixed_cb
        } else {
            resolve_containing_block_selection(document, cascade, fixed_selection, origin)
        };
        stack.extend(
            node.children
                .iter()
                .rev()
                .map(|&child| (child, origin, in_paragraph, abs_cb, fixed_cb)),
        );
    }
}

/// Convert a coordinate in the containing block area to the stored parent-relative location.
pub(crate) fn parent_local_position(
    area_origin: Point<f32>,
    area_position: Point<f32>,
    parent_origin: Point<f32>,
) -> Point<f32> {
    Point {
        x: area_origin.x + area_position.x - parent_origin.x,
        y: area_origin.y + area_position.y - parent_origin.y,
    }
}

#[cfg(test)]
mod tests;
