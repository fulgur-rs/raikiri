//! Source sibling positions and lazy reverse walks for margin adjacency.

use super::{
    CascadeResult, DisplayValue, Document, NodeKind, is_floating_box_for_pagination,
    takes_no_block_space,
};
use raikiri_layout_style::values::Position;
use raikiri_style::property::WhiteSpaceCollapse;
use std::collections::HashMap;

/// Source positions reused only within one pagination pass. Pagination moves
/// layout boxes, but does not mutate source child lists during this pass.
/// Box classifications and sizes are read anew because layout can change them.
#[derive(Default)]
pub(super) struct SiblingAdjacency {
    positions: HashMap<usize, HashMap<usize, usize>>,
}

impl SiblingAdjacency {
    /// Whether only margins separate the preceding box from this node.
    /// Empty block boxes may intervene; only contents boxes lift their
    /// children into the sibling sequence.
    pub(super) fn adjoins(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        previous: usize,
    ) -> bool {
        #[cfg(test)]
        super::tests::record_sibling_adjacency_work([1, 0, 0]);
        let mut current = node_id;
        for parent in document.ancestors(node_id) {
            let positions = self.positions.entry(parent).or_insert_with(|| {
                let children = &document.nodes[parent].children;
                #[cfg(test)]
                super::tests::record_sibling_adjacency_work([0, children.len(), 0]);
                children
                    .iter()
                    .enumerate()
                    .map(|(i, &id)| (id, i))
                    .collect()
            });
            let Some(&index) = positions.get(&current) else {
                return false;
            };
            for &id in document.nodes[parent].children[..index].iter().rev() {
                if let Some(found) = preceding_box(document, cascade, id, previous) {
                    return found;
                }
            }
            if document.nodes[parent].display() != DisplayValue::Contents {
                return false;
            }
            current = parent;
        }
        false
    }
}

/// Return a decision at the first preceding matching or nonempty box,
/// without visiting the rest of the sibling prefix or copying it to a stack.
fn preceding_box(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    previous: usize,
) -> Option<bool> {
    match flow_box(document, cascade, node_id) {
        FlowBox::Skip => None,
        FlowBox::Contents => document.nodes[node_id]
            .children
            .iter()
            .rev()
            .find_map(|&id| preceding_box(document, cascade, id, previous)),
        FlowBox::Box => {
            #[cfg(test)]
            super::tests::record_sibling_adjacency_work([0, 0, 1]);
            if node_id == previous {
                Some(true)
            } else if !takes_no_block_space(document, node_id) {
                Some(false)
            } else {
                None
            }
        }
    }
}

/// How a source node contributes to its parent's in-flow box sequence.
pub(super) enum FlowBox {
    Skip,
    Contents,
    Box,
}

/// Shared by lazy adjacency and the eager box collectors elsewhere in
/// pagination, so both walks use the same whitespace and out-of-flow rules.
pub(super) fn flow_box(document: &Document, cascade: &CascadeResult, node_id: usize) -> FlowBox {
    let node = &document.nodes[node_id];
    // White space that the style preserves still generates lines.
    let collapse = cascade.computed[node_id].effective_white_space_collapse;
    let collapsed_text = node.kind() == NodeKind::Text
        && node.text_content().is_some_and(|text| {
            text.chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
                && match collapse {
                    WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard => true,
                    WhiteSpaceCollapse::PreserveBreaks => !text.contains(['\n', '\r']),
                    _ => text.is_empty(),
                }
        });
    // Comments and other non-element, non-text nodes generate no box.
    if !matches!(node.kind(), NodeKind::Element | NodeKind::Text)
        || collapsed_text
        || node.display() == DisplayValue::None
        || node.layout_position() == Position::Absolute
        || is_floating_box_for_pagination(document, cascade, node_id)
    {
        FlowBox::Skip
    } else if node.kind() != NodeKind::Text && node.display() == DisplayValue::Contents {
        FlowBox::Contents
    } else {
        FlowBox::Box
    }
}
