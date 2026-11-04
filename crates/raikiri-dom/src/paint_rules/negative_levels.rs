//! Ownership and order of negative boxes in contexts with negative flex/grid items.

use super::*;
use std::collections::HashMap;

/// The negative boxes and stacking contexts whose output needs a negative pass.
/// Contexts without negative flex/grid items keep their existing streaming path.
#[derive(Default)]
pub struct NegativeLevels {
    contexts: HashSet<usize>,
    boxes: HashMap<usize, (usize, i32, usize)>,
    first_lines: HashMap<usize, usize>,
    first_line_contexts: HashSet<usize>,
    first_line_groups: HashMap<usize, usize>,
}

impl NegativeLevels {
    /// Index stacking-context ownership and order-modified tree order once.
    pub fn new(
        document: &Document,
        cascade: &CascadeResult,
        root: usize,
        separate_first_line: bool,
    ) -> Self {
        let mut result = Self::default();
        let mut pending = vec![(root, root, false, None, None)];
        let mut first_line_atoms = HashMap::new();
        let mut tree_order = DocumentOrder::new(document);
        let mut rank = 0;
        while let Some((id, owner, after, inherited_first_line, paragraph)) = pending.pop() {
            if after {
                result.index_pseudo(cascade, id, PseudoElem::After, owner, rank, paragraph);
                rank += 1;
                continue;
            }
            let Some(node) = document.get_node(id) else {
                continue;
            };
            let Some(cv) = cascade.computed.get(id) else {
                continue;
            };
            if !node.is_in_document()
                || node.is_non_rendered_html_element()
                || cv.display == DisplayValue::None
            {
                continue;
            }
            let generates = element_generates_box(document, id, cv);
            if separate_first_line
                && first_line_opacity_layer(document, id).is_some()
                && let Some(line) = node.ifc_lines().and_then(|lines| lines.first())
            {
                for fragment in line.fragments() {
                    if let shodo::Fragment::Atomic(atomic) = fragment {
                        first_line_atoms.insert(atomic.node.0 as usize, id);
                    }
                }
            }
            let first_line = first_line_atoms
                .get(&id)
                .copied()
                .filter(|&paragraph| {
                    owner == paragraph || !tree_order.is_ancestor_or_self(paragraph, owner)
                })
                .or(inherited_first_line);
            let leveled = generates && takes_stack_level(document, cascade, id, cv);
            if id != root
                && leveled
                && let ZIndexValue::Integer(level) = cv.z_index
                && level < 0
            {
                result.boxes.insert(id, (owner, level, rank));
                if let Some(paragraph) = paragraph {
                    result.first_line_groups.insert(id, paragraph);
                }
                if let Some(paragraph) = first_line {
                    result.first_lines.insert(id, paragraph);
                }
                if is_z_indexed_item(document, cascade, id, cv) {
                    if let Some(paragraph) = first_line {
                        result.first_line_contexts.insert(paragraph);
                    } else {
                        result.contexts.insert(owner);
                    }
                }
            }
            rank += 1;
            let context = if id == root
                || (generates
                    && (establishes_stacking_context(document, id, cv)
                        || is_z_indexed_item(document, cascade, id, cv)
                        || matches!(cv.position, PositionValue::Fixed | PositionValue::Sticky)
                        || (leveled && matches!(cv.z_index, ZIndexValue::Integer(_)))))
            {
                id
            } else {
                owner
            };
            let child_paragraph = if node.is_ifc_root() {
                Some(id)
            } else if node.kind() == NodeKind::Element
                && (cv.display.is_inline_flow() || cv.display == DisplayValue::Contents)
            {
                paragraph
            } else {
                None
            };
            result.index_pseudo(
                cascade,
                id,
                PseudoElem::Before,
                context,
                rank,
                child_paragraph,
            );
            rank += 1;
            pending.push((id, context, true, None, child_paragraph));
            let mut children = node.children.clone();
            if cv.display.is_flex_or_grid_container() {
                children.sort_by_key(|&child| {
                    cascade.computed.get(child).map_or(0, |cv| {
                        if matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed) {
                            0
                        } else {
                            cv.order
                        }
                    })
                });
            }
            let child_first_line = if context == id { None } else { first_line };
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, context, false, child_first_line, child_paragraph)),
            );
        }
        result.boxes.retain(|id, _| {
            result
                .first_lines
                .get(id)
                .is_none_or(|paragraph| result.first_line_contexts.contains(paragraph))
        });
        result
            .first_lines
            .retain(|id, _| result.boxes.contains_key(id));
        if result.is_empty() {
            return Self::default();
        }
        result.first_line_groups.retain(|id, paragraph| {
            result.first_line_contexts.contains(paragraph)
                && result.boxes.get(id).is_some_and(|&(owner, _, _)| {
                    owner == *paragraph || !tree_order.is_ancestor_or_self(*paragraph, owner)
                })
        });
        result
    }

    fn index_pseudo(
        &mut self,
        cascade: &CascadeResult,
        element: usize,
        pseudo: PseudoElem,
        owner: usize,
        rank: usize,
        paragraph: Option<usize>,
    ) {
        let id = crate::generated_content::generated_node_id(element, pseudo);
        if let Some(cv) = crate::generated_content::computed_for_id(cascade, id)
            && matches!(cv.position, PositionValue::Relative | PositionValue::Sticky)
            && let ZIndexValue::Integer(level) = cv.z_index
            && level < 0
        {
            self.boxes.insert(id, (owner, level, rank));
            if let Some(paragraph) = paragraph {
                self.first_line_groups.insert(id, paragraph);
            }
        }
    }

    /// Whether this context needs its ordinary output held behind negative boxes.
    pub fn holds(&self, context: usize) -> bool {
        self.contexts.contains(&context)
    }

    /// Whether the walk can use its streaming path without recording any clips.
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty() && self.first_line_contexts.is_empty()
    }

    /// The translucent first line containing a box instead of its source context.
    pub fn first_line_owner(&self, node: usize) -> Option<usize> {
        self.first_lines.get(&node).copied()
    }

    /// Whether a first-line group needs to order negative descendants.
    pub fn holds_first_line(&self, root: usize) -> bool {
        self.first_line_contexts.contains(&root)
    }

    /// The first-line owner of only the first-line part of an inline group.
    pub fn first_line_group_owner(&self, group: usize) -> Option<usize> {
        self.first_line_groups.get(&group).copied()
    }

    /// The owning context, stack level and order-modified rank of a negative box.
    pub fn box_key(&self, node: usize) -> Option<(usize, i32, usize)> {
        self.boxes.get(&node).copied()
    }

    /// Whether an inline negative group joins the indexed negative pass.
    pub fn leaves_inline_group(
        &self,
        document: &Document,
        cascade: &CascadeResult,
        root: usize,
        id: usize,
    ) -> bool {
        self.boxes
            .get(&id)
            .is_some_and(|(owner, _, _)| self.contexts.contains(owner))
            && (negative_inline_group(document, cascade, root, id)
                || negative_pseudo_group(document, cascade, root, id))
    }

    /// Whether text belongs to an inline group moved into the negative pass.
    pub fn text_leaves(
        &self,
        document: &Document,
        cascade: &CascadeResult,
        root: usize,
        start: usize,
    ) -> bool {
        self.leaves_inline_group(document, cascade, root, start)
            || document
                .inclusive_ancestors(
                    crate::generated_content::generated_origin(start)
                        .map_or(start, |(element, _)| element),
                )
                .take_while(|&id| id != root)
                .any(|id| self.leaves_inline_group(document, cascade, root, id))
    }
}
