//! Source sibling positions and lazy reverse walks for margin adjacency.

use super::{
    CascadeResult, DisplayValue, Document, NodeKind, is_floating_box_for_pagination,
    takes_no_block_space,
};
use raikiri_layout_style::values::Position;
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
            let Some(index) = self.index_in(document, parent, current) else {
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

    /// The last box before `node_id` among its siblings that takes block
    /// space, when only empty boxes and boxes out of the flow lie between
    /// the two.
    pub(super) fn previous_block(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
    ) -> Option<usize> {
        let parent = document.parent_of(node_id)?;
        let index = self.index_in(document, parent, node_id)?;
        document.nodes[parent].children[..index]
            .iter()
            .rev()
            .find_map(|&id| last_block(document, cascade, id))
            .flatten()
    }

    /// The siblings that come before `node_id` among the children of its
    /// parent, found from the cached positions instead of a walk over the
    /// children.
    pub(super) fn preceding_siblings<'d>(
        &mut self,
        document: &'d Document,
        node_id: usize,
    ) -> &'d [usize] {
        document
            .parent_of(node_id)
            .and_then(|parent| {
                self.index_in(document, parent, node_id)
                    .map(|index| &document.nodes[parent].children[..index])
            })
            .unwrap_or(&[])
    }

    /// The position of `child` among the children of `parent`, from the
    /// positions cached for `parent` the first time they are asked for.
    fn index_in(&mut self, document: &Document, parent: usize, child: usize) -> Option<usize> {
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
        positions.get(&child).copied()
    }
}

/// The last box of `node_id` that takes block space, `Some(None)` when
/// something else that takes space comes first, and `None` when it holds
/// nothing in the flow.
fn last_block(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> Option<Option<usize>> {
    match flow_box(document, cascade, node_id) {
        FlowBox::Skip | FlowBox::Phantom => None,
        FlowBox::Contents => document.nodes[node_id]
            .children
            .iter()
            .rev()
            .find_map(|&id| last_block(document, cascade, id)),
        FlowBox::Box if takes_no_block_space(document, node_id) => None,
        FlowBox::Box => {
            Some((document.nodes[node_id].kind() == NodeKind::Element).then_some(node_id))
        }
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
        FlowBox::Skip | FlowBox::Phantom => None,
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
    /// An empty inline element that lays out a phantom line (see
    /// [`is_phantom_inline`]): no content, which most walks skip, though the
    /// anonymous block around its line is still a box of no height.
    Phantom,
}

/// Shared by lazy adjacency and the eager box collectors elsewhere in
/// pagination, so both walks use the same whitespace and out-of-flow rules.
pub(super) fn flow_box(document: &Document, cascade: &CascadeResult, node_id: usize) -> FlowBox {
    let node = &document.nodes[node_id];
    // White space that the style preserves still generates lines.
    let collapse = cascade.computed[node_id].effective_white_space_collapse;
    let collapsed_text = node.kind() == NodeKind::Text
        && node
            .text_content()
            .is_some_and(|text| crate::generated_content::is_collapsed_white_space(text, collapse));
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
    } else if is_phantom_inline(document, cascade, node_id) {
        FlowBox::Phantom
    } else {
        FlowBox::Box
    }
}

/// Whether `node_id` is an inline box that lays out no line: a non-atomic
/// inline element without block size, outside a flex or grid container,
/// whose descendants are only collapsed white space, boxes with
/// `display: none`, absolutely positioned boxes and such inline boxes. A
/// float inside keeps the box, as the walks that stop at floats look at
/// direct children only. CSS 2.1 §9.4.2 treats a line
/// holding nothing but these as if it did not exist (a phantom line box), as
/// Chromium does when it breaks pages. An inline box with inline-axis
/// margins, borders or padding, or with generated content, has block size
/// and lays out a line, as do a line break (`br`, and `wbr`, which Chromium
/// gives a line too) and a replaced element.
fn is_phantom_inline(document: &Document, cascade: &CascadeResult, node_id: usize) -> bool {
    if !is_empty_inline_box(document, node_id) || in_flex_or_grid_container(document, node_id) {
        return false;
    }
    // An explicit stack, as the inline elements can nest without limit.
    let mut pending = document.nodes[node_id].children.clone();
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        match node.kind() {
            NodeKind::Text => {
                let collapse = cascade.computed[id].effective_white_space_collapse;
                if !node.text_content().is_some_and(|text| {
                    crate::generated_content::is_collapsed_white_space(text, collapse)
                }) {
                    return false;
                }
            }
            NodeKind::Element => match node.display() {
                DisplayValue::None => {}
                _ if node.layout_position() == Position::Absolute => {}
                DisplayValue::Contents => pending.extend_from_slice(&node.children),
                _ if !is_floating_box_for_pagination(document, cascade, id)
                    && is_empty_inline_box(document, id) =>
                {
                    pending.extend_from_slice(&node.children);
                }
                _ => return false,
            },
            _ => {}
        }
    }
    true
}

/// Whether `node_id` is a `display: inline` element that is neither a line
/// break opportunity (`br`, `wbr`) nor replaced and has no block size in its
/// writing mode.
fn is_empty_inline_box(document: &Document, node_id: usize) -> bool {
    let node = &document.nodes[node_id];
    let block_size = if node.used_writing_mode.is_vertical() {
        node.layout.size.width
    } else {
        node.layout.size.height
    };
    node.kind() == NodeKind::Element
        && node.display() == DisplayValue::Inline
        && block_size <= 0.0
        && !node
            .tag_name()
            .is_some_and(|tag| tag.eq_ignore_ascii_case("br") || tag.eq_ignore_ascii_case("wbr"))
        && !crate::layout::page::is_replaced_for_pagination(document, node_id)
}

/// Whether the box `node_id` is an item of a flex or grid container, whose
/// items are blockified, looking through `display: contents` parents.
fn in_flex_or_grid_container(document: &Document, node_id: usize) -> bool {
    let mut parent = document.parent_of(node_id);
    while let Some(id) = parent.filter(|&id| document.nodes[id].display() == DisplayValue::Contents)
    {
        parent = document.parent_of(id);
    }
    parent.is_some_and(|id| document.nodes[id].display().is_flex_or_grid_container())
}
