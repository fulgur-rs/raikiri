//! DOM §4.2.3 "Mutation algorithms": pre-insert / pre-remove / replace a
//! child, detached Text / Comment / DocumentFragment creation, character
//! data read-write, processing-instruction target, and attribute-name
//! listing.
//!
//! Spec refs:
//! - <https://dom.spec.whatwg.org/#concept-node-ensure-pre-insertion-validity>
//! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
//! - <https://dom.spec.whatwg.org/#concept-node-pre-remove>
//! - <https://dom.spec.whatwg.org/#concept-node-replace>
//!
//! raikiri-dom has no `DocumentType` node, so every "or doctype" branch of
//! the spec algorithms below is dropped rather than implemented as dead
//! code (the DOM interface list a `node` may satisfy narrows from
//! `DocumentFragment, DocumentType, Element, or CharacterData` to
//! `DocumentFragment, Element, or CharacterData`).

use smol_str::SmolStr;

use crate::node::{Node, NodeData};

use super::Document;

/// A DOM §4.2.3 mutation-validity failure, named after the DOMException it
/// corresponds to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomMutationError {
    /// "HierarchyRequestError" (pre-insert / replace-a-child validity, DOM
    /// §4.2.3 steps 1-6).
    HierarchyRequest(String),
    /// "NotFoundError" (a reference or old child is not actually a child of
    /// the given parent).
    NotFound(String),
}

impl Document {
    /// DOM §4.2.3 "pre-insert node into parent before child": move `node` to
    /// become a child of `parent`, immediately before `before` (`None`
    /// inserts at the end).
    ///
    /// `node`'s current parent, if any, is detached first (DOM's "adopt"
    /// step folded into `insert` implies this move semantics).
    /// [`Document::append_child`] is a thin `String`-erroring wrapper over
    /// this method. A [`NodeData::DocumentFragment`] `node` is spliced open
    /// in source order instead of being inserted as a single child, per
    /// [`Document::attach_child`] / [`Document::insert_child_before`]'s
    /// existing fragment-aware semantics.
    pub fn pre_insert(
        &mut self,
        parent: usize,
        node: usize,
        before: Option<usize>,
    ) -> Result<(), DomMutationError> {
        self.ensure_insertion_validity(parent, node, before, None)?;
        self.move_node(parent, node, before);
        Ok(())
    }

    /// DOM §4.2.3 "pre-remove a child from parent": detach `child`, which
    /// must currently be one of `parent`'s children.
    pub fn pre_remove(&mut self, parent: usize, child: usize) -> Result<(), DomMutationError> {
        if self.parent_of(child) != Some(parent) {
            return Err(DomMutationError::NotFound(format!(
                "child index {child} is not a child of parent index {parent}"
            )));
        }
        self.detach_from_parent(child);
        Ok(())
    }

    /// DOM §4.2.3 "replace a child with node within parent": `child` (which
    /// must currently be one of `parent`'s children) is removed and `node`
    /// takes its place, immediately before whatever `child`'s next sibling
    /// was.
    ///
    /// `node == child` is a no-op: the general algorithm's remove-then-
    /// reinsert-before-the-captured-next-sibling nets out to `node`'s
    /// unchanged original position, so the mutation is skipped outright.
    pub fn replace_child(
        &mut self,
        parent: usize,
        node: usize,
        child: usize,
    ) -> Result<(), DomMutationError> {
        self.ensure_insertion_validity(parent, node, Some(child), Some(child))?;
        if node == child {
            return Ok(());
        }
        // Step "let referenceChild be child's next sibling" -- captured
        // before `child` is removed below, then adjusted the same way
        // pre-insert adjusts `before` when it aliases `node` (using node's
        // own next sibling instead of the about-to-be-stale reference).
        let reference = match self.next_sibling(child) {
            Some(sibling) if sibling == node => self.next_sibling(node),
            other => other,
        };
        self.detach_from_parent(child);
        self.detach_from_parent(node);
        match reference {
            Some(reference) => self.insert_child_before(parent, reference, node),
            None => self.attach_child(parent, node),
        }
        Ok(())
    }

    /// Create a detached Text node holding `data`, attached to nothing.
    pub fn create_detached_text(&mut self, data: &str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_text(SmolStr::new(data)));
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Create a detached Comment node holding `data`, attached to nothing.
    pub fn create_detached_comment(&mut self, data: &str) -> usize {
        self.append_comment(None, data)
    }

    /// Create a detached DocumentFragment root, attached to nothing.
    pub fn create_detached_fragment(&mut self) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_document_fragment());
        self.invalidate_layout_cache();
        self.flags_dirty = true;
        id
    }

    /// Read a Text / Comment / ProcessingInstruction node's character data.
    /// `None` for any other kind or an out-of-range `id`.
    pub fn character_data(&self, id: usize) -> Option<&str> {
        match &self.nodes.get(id)?.data {
            NodeData::Text(text) => Some(text.text_content.as_str()),
            NodeData::Comment(value) => Some(value.as_str()),
            NodeData::ProcessingInstruction { data, .. } => Some(data.as_str()),
            _ => None,
        }
    }

    /// Replace a Text / Comment / ProcessingInstruction node's character
    /// data. Errs for an out-of-range `id` or any other node kind.
    ///
    /// A Text node's rewrite invalidates the layout cache: layout caches a
    /// parley shaping of the old character data
    /// ([`crate::node::TextData::text_layout`]), and the next layout pass
    /// must reshape from the new string instead of reusing that stale
    /// glyph run.
    pub fn set_character_data(&mut self, id: usize, data: &str) -> Result<(), String> {
        let Some(node) = self.nodes.get_mut(id) else {
            return Err(format!("character data target index {id} is out of range"));
        };
        match &mut node.data {
            NodeData::Text(text) => text.text_content = SmolStr::new(data),
            NodeData::Comment(value) => *value = SmolStr::new(data),
            NodeData::ProcessingInstruction { data: pi_data, .. } => *pi_data = SmolStr::new(data),
            _ => {
                return Err(format!(
                    "character data target index {id} is not a Text, Comment, or \
                     ProcessingInstruction node"
                ));
            }
        }
        self.invalidate_layout_cache();
        Ok(())
    }

    /// A ProcessingInstruction node's target. `None` for any other kind or
    /// an out-of-range `id`.
    pub fn processing_instruction_target(&self, id: usize) -> Option<&str> {
        match &self.nodes.get(id)?.data {
            NodeData::ProcessingInstruction { target, .. } => Some(target.as_str()),
            _ => None,
        }
    }

    /// Qualified names of an element's attributes, in attribute-list order,
    /// with `"style"` appended when the element has an inline style.
    ///
    /// raikiri-dom stores the `style` attribute in a separate inline-style
    /// slot rather than in the ordinary attribute list (see
    /// [`crate::node::ElementData::inline_style`]), so its original
    /// source-order position among the other attributes is not preserved;
    /// it always sorts last here. `None` for any other kind or an
    /// out-of-range `id` is represented as an empty list.
    pub fn element_attribute_names(&self, id: usize) -> Vec<String> {
        let Some(node) = self.nodes.get(id) else {
            return Vec::new();
        };
        let NodeData::Element(element) = &node.data else {
            return Vec::new();
        };
        let mut names: Vec<String> = element
            .attributes
            .iter()
            .map(|attr| attr.local.to_string())
            .collect();
        if element.inline_style.is_some() {
            names.push("style".to_string());
        }
        names
    }

    // ─── DOM §4.2.3 validity + move helpers ───────────────

    /// Shared validity core of "ensure pre-insert validity"
    /// (<https://dom.spec.whatwg.org/#concept-node-ensure-pre-insertion-validity>)
    /// and "replace a child"'s own validity preamble
    /// (<https://dom.spec.whatwg.org/#concept-node-replace>, steps 1-6),
    /// which repeat the same six checks against a different "reference
    /// node" and a different Document-arity exclusion:
    ///
    /// - `reference` is `before` for pre-insert (checked only when `Some`)
    ///   and `child` for replace-a-child (always checked); either way, a
    ///   `Some` reference must already be a child of `parent` or this
    ///   returns `NotFound`.
    /// - `exclude` is the element (if any) that must not count against a
    ///   Document parent's "at most one Element child" rule because it is
    ///   about to be replaced -- `None` for pre-insert, `Some(child)` for
    ///   replace-a-child.
    fn ensure_insertion_validity(
        &self,
        parent: usize,
        node: usize,
        reference: Option<usize>,
        exclude: Option<usize>,
    ) -> Result<(), DomMutationError> {
        // Step 1: parent must be a Document, DocumentFragment, or Element.
        let Some(parent_node) = self.nodes.get(parent) else {
            return Err(DomMutationError::HierarchyRequest(format!(
                "parent index {parent} is out of range"
            )));
        };
        if !matches!(
            parent_node.data,
            NodeData::Document | NodeData::DocumentFragment | NodeData::Element(_)
        ) {
            return Err(DomMutationError::HierarchyRequest(
                "parent must be a Document, DocumentFragment, or Element node".into(),
            ));
        }

        // Step 2: node must not be a host-including inclusive ancestor of
        // parent. raikiri-dom has no shadow DOM, but a `<template>`
        // element's contents fragment does have a host in this sense (the
        // template element itself, per the HTML template-contents
        // association) even though the two are not linked through ordinary
        // `children` -- `is_inclusive_ancestor` walks that link too. This
        // check is safe to run before validating `node` itself, since it
        // bottoms out on an out-of-range id via `Vec::get`.
        if self.is_inclusive_ancestor(node, parent) {
            return Err(DomMutationError::HierarchyRequest(
                "node is an inclusive ancestor of parent".into(),
            ));
        }
        let Some(node_node) = self.nodes.get(node) else {
            return Err(DomMutationError::HierarchyRequest(format!(
                "node index {node} is out of range"
            )));
        };

        // Step 3: a given reference node must already be a child of parent.
        if let Some(reference) = reference
            && self.parent_of(reference) != Some(parent)
        {
            return Err(DomMutationError::NotFound(format!(
                "reference index {reference} is not a child of parent index {parent}"
            )));
        }

        // Step 4: node must be a DocumentFragment, Element, or CharacterData
        // node (no DocumentType in raikiri-dom).
        if !matches!(
            node_node.data,
            NodeData::DocumentFragment
                | NodeData::Element(_)
                | NodeData::Text(_)
                | NodeData::Comment(_)
                | NodeData::ProcessingInstruction { .. }
        ) {
            return Err(DomMutationError::HierarchyRequest(
                "node must be a DocumentFragment, Element, Text, Comment, or \
                 ProcessingInstruction node"
                    .into(),
            ));
        }

        // Step 5: a Text node can never be a child of a Document.
        if matches!(node_node.data, NodeData::Text(_))
            && matches!(parent_node.data, NodeData::Document)
        {
            return Err(DomMutationError::HierarchyRequest(
                "a Text node cannot be a child of a Document".into(),
            ));
        }

        // Step 6: a Document parent accepts at most one Element child
        // overall (across both the node being inserted/replaced-in and its
        // existing children, `exclude`d children aside).
        if matches!(parent_node.data, NodeData::Document) {
            match &node_node.data {
                NodeData::DocumentFragment => {
                    let element_count = node_node
                        .children
                        .iter()
                        .filter(|&&child| self.is_element(child))
                        .count();
                    let has_text = node_node.children.iter().any(|&child| self.is_text(child));
                    if element_count > 1 || has_text {
                        return Err(DomMutationError::HierarchyRequest(
                            "a DocumentFragment inserted into a Document may contain at most \
                             one Element and no Text node"
                                .into(),
                        ));
                    }
                    if element_count == 1 && self.has_element_child_excluding(parent, exclude) {
                        return Err(DomMutationError::HierarchyRequest(
                            "a Document may only have one Element child".into(),
                        ));
                    }
                }
                NodeData::Element(_) => {
                    if self.has_element_child_excluding(parent, exclude) {
                        return Err(DomMutationError::HierarchyRequest(
                            "a Document may only have one Element child".into(),
                        ));
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Detach `node` from wherever it currently lives and (re)attach it to
    /// `parent`, splicing a `DocumentFragment`'s children in source order
    /// instead of the fragment itself ([`Document::attach_child`] /
    /// [`Document::insert_child_before`]'s existing fragment-aware
    /// semantics). `before = None` appends at the end.
    ///
    /// Precondition: `ensure_insertion_validity` has already accepted this
    /// `(parent, node, before)` triple.
    fn move_node(&mut self, parent: usize, node: usize, before: Option<usize>) {
        // DOM §4.2.3 pre-insert: "let referenceChild be child; if
        // referenceChild is node, then set referenceChild to node's next
        // sibling" -- captured before `node` is detached, so a
        // self-referential `insertBefore(node, node)` keeps node at its
        // original position instead of resolving against a reference that
        // is about to move out from under it.
        let before = match before {
            Some(reference) if reference == node => self.next_sibling(node),
            other => other,
        };
        self.detach_from_parent(node);
        match before {
            Some(before) => self.insert_child_before(parent, before, node),
            None => self.attach_child(parent, node),
        }
    }

    /// `node`'s next sibling in its current parent's child order, or `None`
    /// if `node` is detached or is its parent's last child.
    fn next_sibling(&self, node: usize) -> Option<usize> {
        let parent = self.parent_of(node)?;
        let siblings = &self.nodes[parent].children;
        let position = siblings.iter().position(|&child| child == node)?;
        siblings.get(position + 1).copied()
    }

    /// Iterative DFS over `node`'s subtree (`node` included) checking
    /// whether `parent` is reachable -- the direction-reversed form of
    /// "`node` is a host-including inclusive ancestor of `parent`" (walk
    /// `node`'s descendants instead of `parent`'s ancestors and their
    /// hosts; equivalent, and reuses the same arena adjacency already
    /// available via `children`).
    ///
    /// A `<template>` element's contents fragment is one such host
    /// relationship: the fragment is that element's
    /// [`crate::node::ElementData::template_contents`], not one of its
    /// ordinary `children` (the template element's own `children` stays
    /// empty), so the walk below follows that slot too. Without it, this
    /// method would miss e.g. inserting a `<template>` element (or one of
    /// its ancestors) into its own contents fragment.
    fn is_inclusive_ancestor(&self, node: usize, parent: usize) -> bool {
        let mut stack = vec![node];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = stack.pop() {
            if current == parent {
                return true;
            }
            if visited.insert(current)
                && let Some(current_node) = self.nodes.get(current)
            {
                stack.extend(current_node.children.iter().copied());
                if let NodeData::Element(element) = &current_node.data
                    && let Some(contents) = element.template_contents
                {
                    stack.push(contents);
                }
            }
        }
        false
    }

    fn is_element(&self, id: usize) -> bool {
        matches!(
            self.nodes.get(id).map(|n| &n.data),
            Some(NodeData::Element(_))
        )
    }

    fn is_text(&self, id: usize) -> bool {
        matches!(self.nodes.get(id).map(|n| &n.data), Some(NodeData::Text(_)))
    }

    /// Whether `parent` has an Element child other than `exclude`.
    fn has_element_child_excluding(&self, parent: usize, exclude: Option<usize>) -> bool {
        self.nodes.get(parent).is_some_and(|p| {
            p.children
                .iter()
                .any(|&child| Some(child) != exclude && self.is_element(child))
        })
    }
}

#[cfg(test)]
mod tests;
