//! Shared paragraph structure and page-local sparse slot selections.

use super::{LineBoxes, LinePart, PageItems, is_inline_group};
use crate::{Document, GeneratedKind, paint_rules};
use raikiri_style::CascadeResult;
use raikiri_traits::NodeKind;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The inline content of a paragraph or of one of its inline opacity groups,
/// in document order.
#[derive(Debug)]
pub(super) struct InlineContent {
    /// The elements whose own boxes or generated boxes are listed, each with
    /// the pseudo-element it stands for if it is not the element's own box.
    pub(super) elements: Vec<(usize, Option<GeneratedKind>)>,
    /// The inline opacity groups nested directly in the content.
    pub(super) nested: Vec<usize>,
    /// The text nodes of the content.
    pub(super) texts: Vec<usize>,
}

/// Walk the inline content below `children`, starting at element `first`.
/// The walk stops at the boxes laid out beside the lines and at the
/// inline opacity groups, which are returned to list after the content.
fn collect_content(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    group: Option<usize>,
    beside_lines: &HashSet<usize>,
    first: usize,
    children: &[usize],
) -> InlineContent {
    let mut elements = Vec::new();
    if group.is_some() {
        elements.push((first, None));
    }
    elements.push((first, Some(GeneratedKind::Before)));
    // A `::first-letter` box wraps letters inside the element they come
    // from, so it follows that element's box and `::before` box, or its
    // `::after` box when the letters are that pseudo-element's.
    elements.push((first, Some(GeneratedKind::FirstLetter)));
    let mut nested = Vec::new();
    let mut texts = Vec::new();
    let mut stack: Vec<_> = children.iter().rev().map(|&id| (id, false)).collect();
    while let Some((node_id, after)) = stack.pop() {
        #[cfg(test)]
        crate::page_projection::work_tests::paragraph_node();
        if after {
            elements.push((node_id, Some(GeneratedKind::After)));
            continue;
        }
        if beside_lines.contains(&node_id) {
            continue;
        }
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: child ids come from the arena
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            continue;
        }
        match node.kind() {
            NodeKind::Element if !node.is_display_none() && !node.is_hidden_by_text_overflow() => {
                if cascade
                    .computed
                    .get(node_id)
                    .is_some_and(|cv| is_inline_group(document, cascade, root, node_id, node, cv))
                {
                    nested.push(node_id);
                    continue;
                }
                elements.push((node_id, None));
                elements.push((node_id, Some(GeneratedKind::Before)));
                elements.push((node_id, Some(GeneratedKind::FirstLetter)));
                stack.push((node_id, true));
                stack.extend(node.children.iter().rev().map(|&id| (id, false)));
            }
            NodeKind::Text => texts.push(node_id),
            _ => {}
        }
    }
    elements.push((first, Some(GeneratedKind::After)));
    InlineContent {
        elements,
        nested,
        texts,
    }
}

/// Immutable paragraph slots and source order, shared by every page.
#[derive(Debug, Default)]
pub(super) struct ParagraphStructure {
    pub(super) contents: HashMap<Option<usize>, InlineContent>,
    pub(super) members: HashMap<Option<usize>, HashSet<usize>>,
    pub(super) slots: HashMap<usize, (Option<usize>, Vec<usize>)>,
    text_slots: HashMap<usize, (Option<usize>, usize)>,
    parents: HashMap<usize, Option<usize>>,
    group_rank: HashMap<usize, usize>,
    box_groups: HashMap<usize, Option<usize>>,
    rest_groups: HashSet<usize>,
    pub(super) holds_negative: bool,
    pub(super) blocks: Vec<usize>,
    pub(super) boxes: HashMap<(Option<usize>, LinePart), Vec<usize>>,
    pub(super) positions: HashMap<usize, usize>,
    pub(super) line_boxes: Arc<LineBoxes>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ParagraphStructureCache(
    std::cell::RefCell<(u64, HashMap<usize, Arc<ParagraphStructure>>)>,
);

impl ParagraphStructureCache {
    pub(crate) fn clear(&mut self) {
        *self.0.get_mut() = Default::default();
    }
}

impl Document {
    pub(super) fn paragraph_structure(
        &self,
        cascade: &CascadeResult,
        root: usize,
    ) -> Arc<ParagraphStructure> {
        let generation = cascade.element_generation();
        let mut cache = self.page_projection.paragraph_structure.0.borrow_mut();
        if cache.0 != generation {
            *cache = (generation, HashMap::new());
        }
        Arc::clone(cache.1.entry(root).or_insert_with(|| {
            let mut structure = ParagraphStructure {
                positions: self.preorder_positions(root),
                ..Default::default()
            };
            if let Some(root_node) = self.ifc_layout_node(root) {
                let mut boxes = LineBoxes::default();
                for piece in root_node.ifc_inline_boxes().unwrap_or_default() {
                    #[cfg(test)]
                    crate::page_projection::work_tests::paragraph_node();
                    if piece.line == 0 {
                        boxes.first.insert(piece.node);
                    } else {
                        boxes.later.insert(piece.node);
                    }
                }
                structure.line_boxes = Arc::new(boxes);
                let beside_lines: HashSet<_> = root_node.ifc_boxes().into_iter().collect();
                let mut pending = vec![None];
                while let Some(group) = pending.pop() {
                    let first = group.unwrap_or(root);
                    let node = group.and_then(|id| self.get_node(id)).unwrap_or(root_node);
                    let content = collect_content(
                        self,
                        cascade,
                        root,
                        group,
                        &beside_lines,
                        first,
                        &node.children,
                    );
                    for (rank, &child) in content.nested.iter().enumerate() {
                        structure.parents.insert(child, group);
                        structure.group_rank.insert(child, rank);
                    }
                    pending.extend(content.nested.iter().copied().map(Some));
                    let mut members = HashSet::new();
                    for (position, &(id, kind)) in content.elements.iter().enumerate() {
                        structure
                            .slots
                            .entry(id)
                            .or_insert_with(|| (group, Vec::new()))
                            .1
                            .push(position);
                        if kind.is_none() && id != first {
                            members.insert(id);
                        }
                    }
                    for (position, &text) in content.texts.iter().enumerate() {
                        structure.text_slots.insert(text, (group, position));
                    }
                    structure.members.insert(group, members);
                    structure.contents.insert(group, content);
                }
            }
            structure.holds_negative = paint_rules::holds_negative_stack_level(self, cascade, root);
            structure.blocks = self.uncached_paragraph_blocks(cascade, root);
            let blocks: HashSet<_> = structure.blocks.iter().copied().collect();
            for id in self
                .ifc_layout_node(root)
                .map(|node| node.ifc_boxes())
                .unwrap_or_default()
            {
                #[cfg(test)]
                crate::page_projection::work_tests::paragraph_node();
                if blocks.contains(&id) {
                    continue;
                }
                let group = self
                    .parent_of(id)
                    .and_then(|parent| self.inline_opacity_group(cascade, root, parent));
                structure.box_groups.insert(id, group);
                structure
                    .boxes
                    .entry((group, LinePart::All))
                    .or_default()
                    .push(id);
                let first = paint_rules::atomic_on_first_line(self, root, id);
                structure
                    .boxes
                    .entry((
                        group,
                        if first {
                            LinePart::First
                        } else {
                            LinePart::Rest
                        },
                    ))
                    .or_default()
                    .push(id);
                if !first && let Some(group) = group {
                    structure.rest_groups.insert(group);
                }
            }
            Arc::new(structure)
        }))
    }
}

impl PageItems<'_> {
    /// Page-local candidates, partitioned by paragraph before any listing.
    pub(super) fn active_paragraph_nodes(&self, root: usize) -> Arc<HashSet<usize>> {
        let mut cache = self.paragraph_nodes.borrow_mut();
        let nodes = cache.get_or_insert_with(|| {
            let mut nodes: HashMap<usize, HashSet<usize>> = HashMap::new();
            let mut add = |root, node| {
                nodes.entry(root).or_default().insert(node);
            };
            for &id in self.by_node.keys() {
                let root = self
                    .document
                    .table_objects
                    .paragraph_owner
                    .get(id)
                    .copied()
                    .flatten()
                    .or_else(|| {
                        self.document
                            .get_node(id)
                            .filter(|node| node.is_ifc_root())
                            .map(|_| id)
                    })
                    .or_else(|| self.document.ifc_root_of(id));
                if let Some(root) = root {
                    add(root, id);
                }
            }
            for &(root, node) in self.inline_boxes.into_iter().flat_map(|boxes| boxes.keys()) {
                add(root, node);
            }
            for &(root, node, _) in self
                .generated_boxes
                .into_iter()
                .flat_map(|boxes| boxes.keys())
            {
                add(root, node);
            }
            for (&(node, _, _), placement) in self
                .generated_images
                .into_iter()
                .flat_map(|images| images.iter())
            {
                add(placement.root, node);
            }
            if let Some(lines) = self.source_lines.as_ref() {
                for (&group, lines) in &lines.groups {
                    let owner = crate::generated_content::generated_origin(group.0 as usize)
                        .map_or(group.0 as usize, |(owner, _)| owner);
                    for line in lines {
                        add(line.root.0 as usize, owner);
                    }
                }
            }

            nodes
                .into_iter()
                .map(|(root, nodes)| (root, Arc::new(nodes)))
                .collect()
        });
        nodes.get(&root).cloned().unwrap_or_default()
    }

    pub(super) fn page_paragraph_slots(
        &self,
        root: usize,
        structure: &ParagraphStructure,
    ) -> Arc<PageParagraphSlots> {
        let mut cache = self.paragraph_slots.borrow_mut();
        Arc::clone(cache.entry(root).or_insert_with(|| {
            let mut selected = PageParagraphSlots::default();
            let mut active = HashSet::new();
            for node in self.active_paragraph_nodes(root).iter() {
                #[cfg(test)]
                crate::page_projection::work_tests::paragraph_node();
                if let Some((group, positions)) = structure.slots.get(node) {
                    active.extend(group);
                    selected
                        .elements
                        .entry(*group)
                        .or_default()
                        .extend(positions);
                }
                if let Some(&(group, position)) = structure.text_slots.get(node) {
                    active.extend(group);
                    selected.texts.entry(group).or_default().push(position);
                }
                if let Some(group) = structure.box_groups.get(node) {
                    active.extend(group);
                }
            }
            // Letter groups may be the only page source left inside a DOM
            // group; activate their holders before filtering nested groups.
            let mut held = Vec::new();
            for letter in super::paragraph::first_letter_pieces(self.document, self, root) {
                held.extend(letter.held);
            }
            if let Some(lines) = &self.source_lines {
                held.extend(
                    lines
                        .letters
                        .iter()
                        .filter(|((letter_root, _), _)| *letter_root == root)
                        .filter_map(|(_, &held)| held),
                );
            }
            for group in held {
                if structure.parents.contains_key(&group) {
                    active.insert(group);
                } else if let Some((owner, _)) = crate::generated_content::generated_origin(group)
                    && let Some((group, positions)) = structure.slots.get(&owner)
                {
                    active.extend(group);
                    selected
                        .elements
                        .entry(*group)
                        .or_default()
                        .extend(positions);
                }
            }
            selected.first_nested = nested_groups(structure, active.iter().copied());
            active.extend(&structure.rest_groups);
            selected.rest_nested = nested_groups(structure, active.into_iter());
            for positions in selected.texts.values_mut() {
                positions.sort_unstable();
            }
            Arc::new(selected)
        }))
    }

    pub(super) fn paragraph_slots(
        &self,
        root: usize,
        structure: &ParagraphStructure,
        group: Option<usize>,
    ) -> Vec<usize> {
        #[cfg(test)]
        if self.full_paragraph_listing {
            return (0..structure.contents[&group].elements.len()).collect();
        }
        let slots = self.page_paragraph_slots(root, structure);
        // Root and group pseudo slots also anchor first letters from elsewhere.
        let mut selected = slots.elements.get(&group).cloned().unwrap_or_default();
        let first = group.unwrap_or(root);
        if let Some((owner, positions)) = structure.slots.get(&first)
            && *owner == group
        {
            selected.extend(positions);
        }
        selected
    }
}

#[derive(Default)]
pub(super) struct PageParagraphSlots {
    pub(super) elements: HashMap<Option<usize>, Vec<usize>>,
    pub(super) texts: HashMap<Option<usize>, Vec<usize>>,
    pub(super) first_nested: HashMap<Option<usize>, Vec<usize>>,
    pub(super) rest_nested: HashMap<Option<usize>, Vec<usize>>,
}

fn nested_groups(
    structure: &ParagraphStructure,
    groups: impl Iterator<Item = usize>,
) -> HashMap<Option<usize>, Vec<usize>> {
    let mut nested: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    let mut seen = HashSet::new();
    for mut group in groups {
        while seen.insert(group) {
            #[cfg(test)]
            crate::page_projection::work_tests::paragraph_node();
            let Some(&parent) = structure.parents.get(&group) else {
                break;
            };
            nested.entry(parent).or_default().push(group);
            let Some(parent) = parent else {
                break;
            };
            group = parent;
        }
    }
    for children in nested.values_mut() {
        children.sort_unstable_by_key(|group| structure.group_rank[group]);
    }
    nested
}

#[cfg(test)]
mod tests;
