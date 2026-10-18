//! The `::first-line` styles of the first formatted line of a paragraph.

use super::{boxes::IfcBoxKind, projection::box_kind};
use crate::Document;
use crate::generated_content::is_in_flow_generated_text;
use crate::text_decoration::has_paintable_line;
use raikiri_style::property::{DisplayValue, WhiteSpaceCollapse};
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem, StyleNodeId};
use raikiri_traits::NodeKind;
use std::collections::HashMap;

/// The `::first-line` styles that reach the first formatted line of one
/// paragraph, and the values its text has there (CSS Pseudo 4 §2.1.2).
pub(crate) struct FirstLine {
    /// The paragraph.
    root: StyleNodeId,
    /// The blocks whose `::first-line` reaches the line, outermost first.
    origins: Vec<StyleNodeId>,
    styles: FirstLineStyles,
}

/// The values elements and text have on the first line of a paragraph.
#[derive(Debug, Default)]
pub(crate) struct FirstLineStyles {
    /// The values text has, by the element or generated box it sits
    /// directly in.
    texts: HashMap<usize, ComputedValues>,
    /// The values of the inline elements and generated boxes of the line.
    elements: HashMap<usize, ComputedValues>,
    /// The `::first-line` boxes that decorate the line, by the block they
    /// belong to, outermost first.
    decorating: Vec<(usize, ComputedValues)>,
}

impl FirstLineStyles {
    /// The values text has on the first line when it sits directly in the
    /// element or generated box `owner`.
    pub(crate) fn text(&self, owner: usize) -> Option<&ComputedValues> {
        self.texts.get(&owner)
    }

    /// The values the inline element or generated box `id` has on the first
    /// line.
    pub(crate) fn element(&self, id: usize) -> Option<&ComputedValues> {
        self.elements.get(&id)
    }

    /// The `::first-line` boxes with a text decoration, by the block they
    /// belong to, outermost first. Their decorations reach all the text of
    /// the first line.
    pub(crate) fn decorating(&self) -> &[(usize, ComputedValues)] {
        &self.decorating
    }
}

impl FirstLine {
    /// The first-line styles of paragraph `root`, given the blocks
    /// `line_origins` (outermost first) that have a `::first-line` style
    /// reaching the paragraph's first formatted line. `None` when no style
    /// reaches it: the first formatted line of the paragraph is then in a
    /// block of its own.
    pub(crate) fn new(
        doc: &Document,
        cascade: &CascadeResult,
        root: usize,
        line_origins: &[usize],
    ) -> Option<Self> {
        if line_origins.is_empty()
            || !cascade.has_first_line_styles()
            || starts_with_block(doc, cascade, root)
        {
            return None;
        }
        let decorating = line_origins
            .iter()
            .filter_map(|&id| {
                let style = cascade
                    .pseudo
                    .get(&(StyleNodeId::new(id as u64), PseudoElem::FirstLine))?;
                has_paintable_line(style.text_decoration_line).then(|| (id, style.as_ref().clone()))
            })
            .collect();
        Some(Self {
            root: StyleNodeId::new(root as u64),
            origins: line_origins
                .iter()
                .map(|&id| StyleNodeId::new(id as u64))
                .collect(),
            styles: FirstLineStyles {
                decorating,
                ..FirstLineStyles::default()
            },
        })
    }

    /// The values of `element` on the first line.
    pub(crate) fn element(
        &self,
        doc: &Document,
        cascade: &CascadeResult,
        element: usize,
    ) -> Option<ComputedValues> {
        cascade.first_line_style(
            doc,
            &self.origins,
            self.root,
            StyleNodeId::new(element as u64),
            None,
        )
    }

    /// The values of the `pseudo` box of `element` on the first line.
    pub(crate) fn generated(
        &self,
        doc: &Document,
        cascade: &CascadeResult,
        element: usize,
        pseudo: PseudoElem,
    ) -> Option<ComputedValues> {
        cascade.first_line_style(
            doc,
            &self.origins,
            self.root,
            StyleNodeId::new(element as u64),
            Some(pseudo),
        )
    }

    /// Record the values the text node `text` has on the first line.
    pub(crate) fn note_text(&mut self, doc: &Document, cascade: &CascadeResult, text: usize) {
        let Some(parent) = doc.parent_of(text) else {
            return; // cov:ignore: the paragraph's text nodes all have a parent element.
        };
        if self.styles.texts.contains_key(&parent) {
            return;
        }
        if let Some(values) = self.element(doc, cascade, parent) {
            let values = cascade.first_line_text_style(StyleNodeId::new(text as u64), &values);
            self.styles.texts.insert(parent, values);
        }
    }

    /// Record `values`, those of the inline element `id` on the first line.
    pub(crate) fn note_element(&mut self, id: usize, values: ComputedValues) {
        self.styles.elements.insert(id, values);
    }

    /// Record `values`, those of the generated box `id` on the first line,
    /// which holds its text itself.
    pub(crate) fn note_generated(&mut self, id: usize, values: ComputedValues) {
        self.styles.texts.insert(id, values.clone());
        self.styles.elements.insert(id, values);
    }

    /// The values recorded.
    pub(crate) fn into_styles(self) -> FirstLineStyles {
        self.styles
    }
}

/// Whether text of only white space in a box with `cv` generates no content.
fn collapses_blank_text(cv: &ComputedValues) -> bool {
    cv.effective_white_space_collapse == WhiteSpaceCollapse::Collapse
        || cv.effective_white_space_collapse == WhiteSpaceCollapse::Discard
}

/// Whether the first in-flow content of paragraph `root` is a block: the
/// first formatted line is then the first line of that block, not of the
/// paragraph's own lines.
fn starts_with_block(doc: &Document, cascade: &CascadeResult, root: usize) -> bool {
    enum Visit {
        Enter(usize),
        Pseudo(usize, PseudoElem),
    }
    let mut stack = vec![Visit::Pseudo(root, PseudoElem::After)];
    stack.extend(
        doc.nodes[root]
            .children
            .iter()
            .rev()
            .map(|&child| Visit::Enter(child)),
    );
    stack.push(Visit::Pseudo(root, PseudoElem::Before));
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::Pseudo(id, pseudo) => {
                if !is_in_flow_generated_text(cascade, id, pseudo) {
                    continue;
                }
                let Some(cv) = cascade.pseudo.get(&(StyleNodeId::new(id as u64), pseudo)) else {
                    continue; // cov:ignore: in-flow generated text requires a retained pseudo style.
                };
                return !(cv.display.is_inline_flow() || cv.display == DisplayValue::Contents);
            }
            Visit::Enter(id) => {
                let node = &doc.nodes[id];
                match node.kind() {
                    NodeKind::Text => {
                        let collapses = doc
                            .parent_of(id)
                            .is_some_and(|parent| collapses_blank_text(&cascade.computed[parent]));
                        let text = node.text_content().unwrap_or("");
                        if !text.is_empty() && (!collapses || !text.trim().is_empty()) {
                            return false;
                        }
                    }
                    NodeKind::Element => {
                        if node.display() == DisplayValue::None
                            || node.is_non_rendered_html_element()
                        {
                            continue;
                        }
                        if node.display() != DisplayValue::Contents {
                            match box_kind(cascade, doc, id) {
                                Some(IfcBoxKind::OutOfFlow | IfcBoxKind::Float) => continue,
                                Some(IfcBoxKind::Block) => return true,
                                Some(_) => return false,
                                None => {}
                            }
                            if node.tag_name() == Some("br") {
                                return false;
                            }
                        }
                        stack.push(Visit::Pseudo(id, PseudoElem::After));
                        stack.extend(node.children.iter().rev().map(|&child| Visit::Enter(child)));
                        stack.push(Visit::Pseudo(id, PseudoElem::Before));
                    }
                    _ => {}
                }
            }
        }
    }
    false
}
