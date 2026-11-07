//! The `::first-line` styles of the first formatted line of a paragraph.

use super::{boxes::IfcBoxKind, projection::box_kind};
use crate::Document;
use crate::generated_content::{is_collapsed_white_space, is_in_flow_generated_text};
use crate::text_decoration::has_paintable_line;
use raikiri_style::property::{ContentComponent, DisplayValue, WhiteSpaceCollapse};
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
    /// The values the paragraph itself has on the first line.
    root_values: ComputedValues,
    /// The block-level `::before` or `::after` of the paragraph itself whose
    /// first line is the first line, when it holds it among the paragraph's
    /// lines. shodo gives its first-line styles only to the line that starts
    /// the paragraph, and the line of a block box starts after the box's start
    /// token: values that shape or size the line cannot reach it, and only
    /// those looked up when it is painted do.
    own_block: Option<PseudoElem>,
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
    /// The `::first-line` box whose background covers the line, with the
    /// block it belongs to.
    background: Option<(usize, ComputedValues)>,
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

    /// The `::first-line` box whose background covers the content of the
    /// first line, below the boxes of the inline elements on it, with the
    /// block it belongs to.
    ///
    /// Only the innermost block that has a `::first-line` of its own paints
    /// one, as Chromium does: it shadows the backgrounds of the blocks around
    /// it, even when it sets none itself.
    pub(crate) fn background(&self) -> Option<&(usize, ComputedValues)> {
        self.background.as_ref()
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
        if line_origins.is_empty() || !cascade.has_first_line_styles() {
            return None;
        }
        let own_block = match first_content(doc, cascade, root) {
            Start::Block => return None,
            Start::OwnBlockBox(pseudo) => Some(pseudo),
            Start::Inline => None,
        };
        let root_id = StyleNodeId::new(root as u64);
        let origins: Vec<StyleNodeId> = line_origins
            .iter()
            .map(|&id| StyleNodeId::new(id as u64))
            .collect();
        // Resolve the root and each decorating origin in one inheritance walk.
        // The ordinary pseudo decides whether a box paints a decoration.
        let mut decorating = Vec::new();
        let mut background = None;
        let root_values =
            cascade.first_line_block_style(doc, &origins, root_id, |origin, style| {
                if own_block.is_some() {
                    return;
                }
                if decorates(cascade, origin) {
                    decorating.push((origin.0 as usize, style.clone()));
                }
                if origins.last() == Some(&origin) && paints_background(style) {
                    background = Some((origin.0 as usize, style.clone()));
                }
            })?;
        // The line belongs to the block box, and the innermost `::first-line`
        // sits inside it: the decoration and the background are those of that
        // box alone, resolved over the values of the block (so `currentColor`
        // is the block's color), and the boxes around do not reach the line.
        if let Some(pseudo) = own_block
            && let Some(&origin) = origins.last()
            && let Some(style) = cascade.first_line_block_pseudo_style(&origins, root_id, pseudo)
        {
            if decorates(cascade, origin) {
                decorating.push((origin.0 as usize, style.clone()));
            }
            if paints_background(&style) {
                background = Some((origin.0 as usize, style));
            }
        }
        Some(Self {
            root: root_id,
            origins,
            root_values,
            own_block,
            styles: FirstLineStyles {
                decorating,
                background,
                ..FirstLineStyles::default()
            },
        })
    }

    /// The values the paragraph has on its first line.
    pub(crate) fn root_values(&self) -> &ComputedValues {
        &self.root_values
    }

    /// Whether the first line is the first line of a block-level `::before`
    /// or `::after` of the paragraph itself. The paragraph's own values and
    /// those of its content are then no concern of its first line.
    pub(crate) fn in_own_block(&self) -> bool {
        self.own_block.is_some()
    }

    /// The values of `element` on the first line.
    pub(crate) fn element(
        &self,
        doc: &Document,
        cascade: &CascadeResult,
        element: usize,
    ) -> Option<ComputedValues> {
        if self.in_own_block() {
            return None;
        }
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
        if self.in_own_block() {
            return None;
        }
        cascade.first_line_style(
            doc,
            &self.origins,
            self.root,
            StyleNodeId::new(element as u64),
            Some(pseudo),
        )
    }

    /// The values the text of the block-level `pseudo` of `element` has on
    /// the first line, when the first line is the first line of that very box.
    pub(crate) fn generated_block(
        &self,
        cascade: &CascadeResult,
        element: usize,
        pseudo: PseudoElem,
    ) -> Option<ComputedValues> {
        if self.own_block != Some(pseudo) || StyleNodeId::new(element as u64) != self.root {
            return None;
        }
        cascade.first_line_block_pseudo_style(&self.origins, self.root, pseudo)
    }

    /// Record the values the text node `text` has on the first line.
    pub(crate) fn note_text(&mut self, doc: &Document, cascade: &CascadeResult, text: usize) {
        if let Some(parent) = doc.parent_of(text)
            && !self.styles.texts.contains_key(&parent)
            && let Some(values) = self.element(doc, cascade, parent)
        {
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

    /// Record `values`, those the text of the block-level generated box `id`
    /// has on the first line. The box itself keeps its own style: only the
    /// text inside it is on the `::first-line`.
    pub(crate) fn note_generated_block_text(&mut self, id: usize, values: ComputedValues) {
        self.styles.texts.insert(id, values);
    }

    /// The values recorded.
    pub(crate) fn into_styles(self) -> FirstLineStyles {
        self.styles
    }
}

/// Whether the `::first-line` box of `origin` has a text decoration to paint.
fn decorates(cascade: &CascadeResult, origin: StyleNodeId) -> bool {
    cascade
        .pseudo
        .get(&(origin, PseudoElem::FirstLine))
        .is_some_and(|ordinary| has_paintable_line(ordinary.text_decoration_line))
}

/// Whether a `::first-line` box with `style` has a background to paint.
fn paints_background(style: &ComputedValues) -> bool {
    style.background_color.a > 0 || style.has_background_image()
}

/// Whether text of only white space in a box with `cv` generates no content.
fn collapses_blank_text(cv: &ComputedValues) -> bool {
    cv.effective_white_space_collapse == WhiteSpaceCollapse::Collapse
        || cv.effective_white_space_collapse == WhiteSpaceCollapse::Discard
}

/// Whether the block-level generated box with `cv` holds content a line can
/// show. A `content` that is resolved while the paragraph is laid out, such
/// as a counter, a quote or an attribute, is not judged here and counts as
/// content.
fn holds_lines(cv: &ComputedValues) -> bool {
    !cv.content.iter().all(|part| {
        matches!(part, ContentComponent::Literal(text)
            if is_collapsed_white_space(text, cv.effective_white_space_collapse))
    })
}

/// What the first in-flow content of a paragraph is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    /// Inline content: the first formatted line is the paragraph's own.
    Inline,
    /// A block of its own: the first formatted line is the first line of
    /// that block, not of the paragraph's lines.
    Block,
    /// The block-level `::before` or `::after` of the paragraph itself, which
    /// holds lines. Its lines stay among the paragraph's, so its first line is
    /// the paragraph's first. One without lines is a [`Start::Block`].
    OwnBlockBox(PseudoElem),
}

/// What the first in-flow content of paragraph `root` is.
fn first_content(doc: &Document, cascade: &CascadeResult, root: usize) -> Start {
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
                if is_in_flow_generated_text(cascade, id, pseudo)
                    && let Some(cv) = cascade.pseudo.get(&(StyleNodeId::new(id as u64), pseudo))
                {
                    if cv.display.is_inline_flow() || cv.display == DisplayValue::Contents {
                        return Start::Inline;
                    }
                    return if id == root && holds_lines(cv) {
                        Start::OwnBlockBox(pseudo)
                    } else {
                        Start::Block
                    };
                }
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
                            return Start::Inline;
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
                                Some(IfcBoxKind::Block) => return Start::Block,
                                Some(_) => return Start::Inline,
                                None => {}
                            }
                            if node.tag_name() == Some("br") {
                                return Start::Inline;
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
    Start::Inline
}
