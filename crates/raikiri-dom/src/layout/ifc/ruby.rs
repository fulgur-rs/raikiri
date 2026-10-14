//! Ruby containers in an inline formatting context (CSS Ruby 1).
//!
//! A `display: ruby` element is handed to shodo as ruby segments: bases that
//! stay in the line and annotations that shodo places beside them. The
//! element's children are grouped into segments as CSS Ruby 1 §2.2 groups
//! them into anonymous boxes: a run of base-level content followed by a run
//! of annotation-level content forms one segment, consecutive `ruby-text`
//! boxes form one annotation level, and every `ruby-text-container` forms a
//! level of its own.
//!
//! Content that this mapping does not cover yet (atomic inlines, blocks,
//! generated content, misparented annotations) leaves the container to the
//! ordinary inline path, which lays its bases and annotations out in the
//! line.

use super::assign::is_css_whitespace;
use super::boxes::{IfcBox, IfcBoxKind};
use super::error::IfcError;
use super::first_letter::FirstLetter;
use super::projection::{GeneratedCounters, box_kind, push_relative_offset, styled};
use super::style;
use crate::Document;
use crate::generated_content::is_in_flow_generated_text;
use raikiri_style::property::{
    DisplayValue, RubyPosition, TextWrapMode, Visibility, WhiteSpaceCollapse,
};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::NodeKind;
use shodo::font::FontCollection;
use shodo::limits::Limits;
use shodo::node::{NodeId, OutOfFlowKind, TextSource};
use shodo::{ParagraphBuilder, RubyContent};
use std::collections::HashMap;

/// A ruby base or annotation: an element of its own, or an anonymous box
/// around a run of its parent's children.
#[derive(Debug)]
struct Part {
    /// The `ruby-base` or `ruby-text` element; `None` for an anonymous box.
    element: Option<usize>,
    /// The element whose style the box's text starts from: the element
    /// itself, or the parent of an anonymous box.
    style_owner: usize,
    /// The DOM children laid out in the box.
    children: Vec<usize>,
    /// White space between this box and the next one of its level: intra-
    /// level white space, which gets a column of its own (CSS Ruby 1 §2.3).
    tail: Vec<usize>,
}

impl Part {
    fn new(element: Option<usize>, style_owner: usize, children: Vec<usize>) -> Self {
        Self {
            element,
            style_owner,
            children,
            tail: Vec::new(),
        }
    }
}

/// One level of annotations: a `ruby-text-container`, or the anonymous one
/// around consecutive `ruby-text` boxes.
#[derive(Debug)]
struct Level {
    container: Option<usize>,
    annotations: Vec<Part>,
}

/// One ruby segment: bases and the annotation levels paired with them.
#[derive(Debug, Default)]
struct Segment {
    /// Text nodes of white space between this segment and the previous one,
    /// laid out in the line between the two (CSS Ruby 1 §2.3).
    gap: Vec<usize>,
    bases: Vec<Part>,
    levels: Vec<Level>,
}

/// How a ruby container is laid out.
#[derive(Debug)]
pub(super) struct Plan {
    segments: Vec<Segment>,
    /// Floats and absolutely positioned boxes inside the container, in
    /// source order, each with the index of the segment it is anchored
    /// before. They generate no ruby boxes, so one inside a segment is
    /// anchored after that segment; an index past the last segment anchors
    /// it at the end of the container.
    out_of_flow: Vec<(usize, usize)>,
}

/// The paragraph data a ruby container adds to, besides the builder.
pub(super) struct Sink<'a> {
    pub(super) boxes: &'a mut Vec<IfcBox>,
    pub(super) text_nowrap: &'a mut HashMap<u64, bool>,
    pub(super) offsets: &'a mut Vec<(usize, (f32, f32))>,
    pub(super) percent_insets: &'a mut Vec<(usize, style::InlineRelativeInsets)>,
    pub(super) first_letter: &'a mut FirstLetter,
    pub(super) counters: &'a GeneratedCounters,
    /// The paragraph root's `direction` is `rtl`.
    pub(super) rtl: bool,
}

/// Whether `id` is a text node of white space alone.
fn is_white_space_text(doc: &Document, id: usize) -> bool {
    let node = &doc.nodes[id];
    node.kind() == NodeKind::Text
        && node
            .text_content()
            .is_some_and(|text| text.chars().all(is_css_whitespace))
}

/// Whether `id` takes no part in layout: a node outside the document, a
/// comment, or an element that renders nothing.
fn is_skipped(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let node = &doc.nodes[id];
    !node.is_in_document()
        || matches!(
            node.kind(),
            NodeKind::Comment | NodeKind::ProcessingInstruction
        )
        || (node.kind() == NodeKind::Element
            && (cascade.computed[id].display == DisplayValue::None
                || node.is_non_rendered_html_element()))
}

/// Whether `id` is a float or an absolutely positioned box.
fn is_out_of_flow(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    matches!(
        box_kind(cascade, doc, id),
        Some(IfcBoxKind::OutOfFlow | IfcBoxKind::Float)
    )
}

/// Whether `id` has `::before` or `::after` content in the line.
fn has_generated_content(cascade: &CascadeResult, id: usize) -> bool {
    [PseudoElem::Before, PseudoElem::After]
        .into_iter()
        .any(|pseudo| is_in_flow_generated_text(cascade, id, pseudo))
}

/// How deeply the content of a ruby container may nest elements and ruby
/// containers before the container is left to the ordinary inline path.
const MAX_DEPTH: usize = 64;

/// Builds the segments of a ruby container.
struct Planner<'a> {
    doc: &'a Document,
    cascade: &'a CascadeResult,
    /// How deeply the container is nested in other ruby content.
    depth: usize,
    segments: Vec<Segment>,
    current: Segment,
    /// The current segment has annotations: base content starts the next one.
    annotated: bool,
    /// Base-level children not yet closed into an anonymous base.
    pending: Vec<usize>,
    /// The parent of the pending children.
    pending_owner: usize,
    /// White space after a base, kept when another base follows it.
    between: Vec<usize>,
    /// White space after the last annotation, before the next segment.
    gap: Vec<usize>,
    out_of_flow: Vec<(usize, usize)>,
    /// Floats and absolutely positioned boxes found in the content of the
    /// child being added, anchored once it is in the current segment.
    found: Vec<usize>,
}

impl Planner<'_> {
    /// Whether the content of `id` is plain inline content: text, inline
    /// boxes and ruby containers, without generated content. Floats and
    /// absolutely positioned boxes are set aside.
    fn is_plain_inline(&mut self, id: usize) -> bool {
        self.is_plain_inline_at(id, 0)
    }

    fn is_plain_inline_at(&mut self, id: usize, depth: usize) -> bool {
        let (doc, cascade) = (self.doc, self.cascade);
        // The content is laid out recursively, so deeply nested content is
        // left to the ordinary inline path, which walks the tree iteratively.
        if self.depth + depth > MAX_DEPTH {
            return false;
        }
        if is_skipped(doc, cascade, id) || doc.nodes[id].kind() == NodeKind::Text {
            return true;
        }
        if is_out_of_flow(doc, cascade, id) {
            self.found.push(id);
            return true;
        }
        if doc.nodes[id].kind() != NodeKind::Element
            || box_kind(cascade, doc, id).is_some()
            || has_generated_content(cascade, id)
        {
            return false;
        }
        match cascade.computed[id].display {
            // An element with `display: contents` generates no box: its
            // children take its place (CSS Display 3 §2.5).
            DisplayValue::Inline | DisplayValue::Contents => doc.nodes[id]
                .children
                .iter()
                .all(|&child| self.is_plain_inline_at(child, depth + 1)),
            DisplayValue::Ruby => match plan_at(doc, cascade, id, self.depth + depth + 1) {
                Some(nested) => {
                    self.found
                        .extend(nested.out_of_flow.into_iter().map(|(_, id)| id));
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// Whether `id` is a ruby base or annotation element with plain content.
    fn is_plain_part(&mut self, id: usize) -> bool {
        box_kind(self.cascade, self.doc, id).is_none()
            && !has_generated_content(self.cascade, id)
            && self.doc.nodes[id]
                .children
                .iter()
                .all(|&child| self.is_plain_inline_at(child, 1))
    }

    /// Close the pending children into an anonymous base. A run of white
    /// space alone generates no base: between two bases it stays in the line
    /// after the first, and between a base and its annotations it is
    /// discarded (CSS Ruby 1 §2.3).
    fn close_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return;
        }
        if pending.iter().all(|&id| is_white_space_text(self.doc, id)) {
            // White space after annotations never reaches this point: it is
            // collected as the gap before the next segment.
            if !self.current.bases.is_empty() {
                self.between.extend(pending);
            }
            return;
        }
        let part = Part::new(None, self.pending_owner, pending);
        self.push_base_part(part);
    }

    /// Base content after annotations starts a new segment.
    fn start_base(&mut self) {
        if self.annotated {
            let next = Segment {
                gap: std::mem::take(&mut self.gap),
                ..Segment::default()
            };
            self.segments
                .push(std::mem::replace(&mut self.current, next));
            self.annotated = false;
        }
    }

    fn push_base_part(&mut self, part: Part) {
        self.start_base();
        let between = std::mem::take(&mut self.between);
        if let Some(previous) = self.current.bases.last_mut() {
            previous.tail.extend(between);
        }
        self.current.bases.push(part);
    }

    fn push_base(&mut self, element: usize) {
        self.close_pending();
        let part = Part::new(
            Some(element),
            element,
            self.doc.nodes[element].children.clone(),
        );
        self.push_base_part(part);
    }

    /// Start annotation-level content: white space before it is discarded.
    fn start_annotations(&mut self) {
        self.close_pending();
        self.between.clear();
        self.gap.clear();
    }

    fn push_annotation(&mut self, element: usize) {
        let gap = std::mem::take(&mut self.gap);
        self.start_annotations();
        let part = Part::new(
            Some(element),
            element,
            self.doc.nodes[element].children.clone(),
        );
        // Consecutive annotations outside a container share an anonymous one.
        match self.current.levels.last_mut() {
            Some(level) if self.annotated && level.container.is_none() => {
                if let Some(previous) = level.annotations.last_mut() {
                    previous.tail = gap;
                }
                level.annotations.push(part);
            }
            _ => self.current.levels.push(Level {
                container: None,
                annotations: vec![part],
            }),
        }
        self.annotated = true;
    }

    /// The annotations of the `ruby-text-container` `container`, or `None`
    /// when its content is not plain.
    fn container_annotations(&mut self, container: usize) -> Option<Vec<Part>> {
        let doc = self.doc;
        let mut annotations: Vec<Part> = Vec::new();
        let mut run: Vec<usize> = Vec::new();
        // White space alone between two annotations is intra-level white
        // space after the first; at the ends of the container it is dropped.
        let close = |run: &mut Vec<usize>, annotations: &mut Vec<Part>| {
            let run = std::mem::take(run);
            if run.iter().all(|&id| is_white_space_text(doc, id)) {
                if let Some(previous) = annotations.last_mut() {
                    previous.tail = run;
                }
            } else {
                annotations.push(Part::new(None, container, run));
            }
        };
        for child in flattened_children(doc, self.cascade, container) {
            if is_skipped(doc, self.cascade, child) {
                continue;
            }
            if is_out_of_flow(doc, self.cascade, child) {
                self.found.push(child);
                continue;
            }
            if doc.nodes[child].kind() == NodeKind::Element
                && self.cascade.computed[child].display == DisplayValue::RubyText
            {
                if !self.is_plain_part(child) {
                    return None;
                }
                close(&mut run, &mut annotations);
                annotations.push(Part::new(
                    Some(child),
                    child,
                    doc.nodes[child].children.clone(),
                ));
            } else if self.is_plain_inline(child) {
                run.push(child);
            } else {
                return None;
            }
        }
        let trailing = std::mem::take(&mut run);
        if !trailing.iter().all(|&id| is_white_space_text(doc, id)) {
            annotations.push(Part::new(None, container, trailing));
        } else if let Some(last) = annotations.last_mut() {
            last.tail.clear();
        }
        Some(annotations)
    }

    /// Add the bases of the `ruby-base-container` `container`.
    fn push_base_container(&mut self, container: usize) -> Option<()> {
        self.close_pending();
        self.start_base();
        self.pending_owner = container;
        for base in flattened_children(self.doc, self.cascade, container) {
            if is_skipped(self.doc, self.cascade, base) {
                continue;
            }
            if is_out_of_flow(self.doc, self.cascade, base) {
                self.found.push(base);
                continue;
            }
            if self.doc.nodes[base].kind() == NodeKind::Element
                && self.cascade.computed[base].display == DisplayValue::RubyBase
            {
                if !self.is_plain_part(base) {
                    return None;
                }
                self.push_base(base);
            } else if self.is_plain_inline(base) {
                self.pending.push(base);
            } else {
                return None;
            }
        }
        self.close_pending();
        Some(())
    }

    /// Add the child `child` of the ruby container `ruby`, and anchor what
    /// its content sets aside after the segment it joins.
    fn push_child(&mut self, ruby: usize, child: usize) -> Option<()> {
        self.push_child_content(ruby, child)?;
        let index = self.segments.len() + 1;
        let found = std::mem::take(&mut self.found);
        self.out_of_flow
            .extend(found.into_iter().map(|id| (index, id)));
        Some(())
    }

    fn push_child_content(&mut self, ruby: usize, child: usize) -> Option<()> {
        let (doc, cascade) = (self.doc, self.cascade);
        if is_skipped(doc, cascade, child) {
            return Some(());
        }
        if doc.nodes[child].kind() == NodeKind::Text {
            if self.annotated && is_white_space_text(doc, child) {
                self.gap.push(child);
            } else {
                if !is_white_space_text(doc, child) {
                    self.start_base();
                }
                self.pending.push(child);
            }
            return Some(());
        }
        // A positioned or floated box generates no ruby box, whatever its
        // display. It is anchored before the current segment until that
        // segment has content, and after it from then on.
        if is_out_of_flow(doc, cascade, child) {
            let started = !self.current.bases.is_empty()
                || !self.current.levels.is_empty()
                || self.pending.iter().any(|&id| !is_white_space_text(doc, id));
            let index = self.segments.len() + usize::from(started);
            self.out_of_flow.push((index, child));
            return Some(());
        }
        let internal =
            box_kind(cascade, doc, child).is_none() && !has_generated_content(cascade, child);
        // The children of an element with `display: contents` take its
        // place in the ruby container (CSS Display 3 §2.5), so ruby parts
        // among them, at any depth of such wrappers, are parts of this
        // container.
        if internal && holds_ruby_parts(doc, cascade, child) {
            for &id in &doc.nodes[child].children {
                self.push_child(ruby, id)?;
            }
            return Some(());
        }
        match cascade.computed[child].display {
            DisplayValue::RubyBase if self.is_plain_part(child) => self.push_base(child),
            DisplayValue::RubyText if self.is_plain_part(child) => self.push_annotation(child),
            DisplayValue::RubyTextContainer if internal => {
                let annotations = self.container_annotations(child)?;
                self.start_annotations();
                self.current.levels.push(Level {
                    container: Some(child),
                    annotations,
                });
                self.annotated = true;
            }
            DisplayValue::RubyBaseContainer if internal => {
                self.push_base_container(child)?;
                self.pending_owner = ruby;
            }
            DisplayValue::RubyBase
            | DisplayValue::RubyText
            | DisplayValue::RubyTextContainer
            | DisplayValue::RubyBaseContainer => return None,
            _ if self.is_plain_inline(child) => {
                self.start_base();
                self.pending.push(child);
            }
            _ => return None,
        }
        Some(())
    }
}

/// The children of the ruby-internal container `container`, with every
/// `display: contents` element holding ruby parts replaced by its children
/// (CSS Display 3 §2.5).
fn flattened_children(doc: &Document, cascade: &CascadeResult, container: usize) -> Vec<usize> {
    let mut children = Vec::new();
    let mut stack: Vec<usize> = doc.nodes[container]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = stack.pop() {
        if holds_ruby_parts(doc, cascade, id) {
            stack.extend(doc.nodes[id].children.iter().rev());
        } else {
            children.push(id);
        }
    }
    children
}

/// Whether `id` is an element with `display: contents` whose children, seen
/// through further such elements, include a ruby base or annotation part.
fn holds_ruby_parts(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let is_contents = |id: usize| {
        doc.nodes[id].kind() == NodeKind::Element
            && cascade.computed[id].display == DisplayValue::Contents
            && box_kind(cascade, doc, id).is_none()
            && !has_generated_content(cascade, id)
    };
    if !is_contents(id) {
        return false;
    }
    let mut stack = doc.nodes[id].children.clone();
    while let Some(id) = stack.pop() {
        if doc.nodes[id].kind() != NodeKind::Element {
            continue;
        }
        if matches!(
            cascade.computed[id].display,
            DisplayValue::RubyBase
                | DisplayValue::RubyText
                | DisplayValue::RubyBaseContainer
                | DisplayValue::RubyTextContainer
        ) {
            return true;
        }
        if is_contents(id) {
            stack.extend(&doc.nodes[id].children);
        }
    }
    false
}

/// How the ruby container `ruby` is laid out, or `None` when its content is
/// not plain enough to hand to shodo.
pub(super) fn plan(doc: &Document, cascade: &CascadeResult, ruby: usize) -> Option<Plan> {
    plan_at(doc, cascade, ruby, 0)
}

fn plan_at(doc: &Document, cascade: &CascadeResult, ruby: usize, depth: usize) -> Option<Plan> {
    if has_generated_content(cascade, ruby) || depth > MAX_DEPTH {
        return None;
    }
    let mut planner = Planner {
        doc,
        cascade,
        depth,
        segments: Vec::new(),
        current: Segment::default(),
        annotated: false,
        pending: Vec::new(),
        pending_owner: ruby,
        between: Vec::new(),
        gap: Vec::new(),
        out_of_flow: Vec::new(),
        found: Vec::new(),
    };
    for &child in &doc.nodes[ruby].children {
        planner.push_child(ruby, child)?;
    }
    planner.close_pending();
    let mut segments = planner.segments;
    let current = planner.current;
    if !current.bases.is_empty() || !current.levels.is_empty() {
        segments.push(current);
    }
    Some(Plan {
        segments,
        out_of_flow: planner.out_of_flow,
    })
}

/// The text nodes laid out in `nodes` and their descendants, in order.
fn texts(doc: &Document, cascade: &CascadeResult, nodes: &[usize]) -> Vec<usize> {
    let mut texts = Vec::new();
    let mut stack: Vec<usize> = nodes.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if is_skipped(doc, cascade, id) || is_out_of_flow(doc, cascade, id) {
            continue;
        }
        if doc.nodes[id].kind() == NodeKind::Text {
            texts.push(id);
        }
        stack.extend(doc.nodes[id].children.iter().rev());
    }
    texts
}

/// Whether text goes through the paragraph's `::first-letter`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Letter {
    /// Base text: the first letter of the paragraph may be in it.
    Base,
    /// Annotation text, which never holds the paragraph's first letter.
    Annotation,
}

/// Lays the content of ruby parts out in builders of their own.
struct Content<'a, 'b> {
    /// Content goes into the paragraph's own line, not into the content of
    /// a ruby base or annotation.
    in_line: bool,
    doc: &'a Document,
    cascade: &'a CascadeResult,
    fonts: &'a FontCollection,
    limits: &'a Limits,
    sink: &'a mut Sink<'b>,
}

impl Content<'_, '_> {
    /// Record the line data of the element `id`: its relative offset and
    /// whether its content wraps.
    fn record_element(&mut self, id: usize) {
        let cv = &self.cascade.computed[id];
        push_relative_offset(
            self.sink.offsets,
            self.sink.percent_insets,
            id,
            cv,
            self.sink.rtl,
        );
        self.sink.text_nowrap.insert(
            id as u64,
            cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
        );
    }

    /// The style of a paragraph of ruby content whose text starts from the
    /// style of `owner`.
    fn paragraph(&self, owner: usize) -> Result<shodo::style::ParagraphStyle, IfcError> {
        Ok(shodo::style::ParagraphStyle {
            root: styled(
                self.doc,
                self.cascade,
                &self.cascade.computed[owner],
                owner,
                self.fonts,
            )?, // cov:ignore: inline_style rejects only keywords added to its style enums after it was written.
            ..shodo::style::ParagraphStyle::default()
        })
    }

    /// The content of `part`, and the node shodo gives its box. An anonymous
    /// box has no element: it takes the id of its first text, or else of
    /// `text`, the first text of its segment.
    fn part(
        &mut self,
        part: &Part,
        letter: Letter,
        text: usize,
    ) -> Result<(NodeId, RubyContent), IfcError> {
        let owner = part.style_owner;
        let paragraph = self.paragraph(owner)?;
        let mut builder = ParagraphBuilder::new(&paragraph, self.limits);
        if let Some(element) = part.element {
            self.record_element(element);
        }
        let texts = texts(self.doc, self.cascade, &part.children);
        let first_text = texts.first().copied();
        // An anonymous box drops the white space at its edges: it is source
        // formatting around the ruby's children (CSS Ruby 1 §2.3).
        let ends = if part.element.is_none() {
            (first_text, texts.last().copied())
        } else {
            (None, None)
        };
        let in_line = std::mem::replace(&mut self.in_line, false);
        let pushed = self.push_nodes(&mut builder, &part.children, ends, letter);
        self.in_line = in_line;
        pushed?;
        let node = part.element.or(first_text).unwrap_or(text);
        Ok((NodeId(node as u64), RubyContent::from_builder(builder)))
    }

    /// Push the inline content `nodes`. White space that can collapse is
    /// removed at the start of the first text and at the end of the last
    /// one (`ends`), the edges of the box (CSS Ruby 1 §2.3).
    fn push_nodes(
        &mut self,
        builder: &mut ParagraphBuilder,
        nodes: &[usize],
        ends: (Option<usize>, Option<usize>),
        letter: Letter,
    ) -> Result<(), IfcError> {
        for &id in nodes {
            if is_skipped(self.doc, self.cascade, id) || is_out_of_flow(self.doc, self.cascade, id)
            {
                continue;
            }
            let node = &self.doc.nodes[id];
            if node.kind() == NodeKind::Text {
                self.push_text(builder, id, ends, letter)?;
                continue;
            }
            let cv = &self.cascade.computed[id];
            if node.tag_name() == Some("br") {
                // A forced break ends the first formatted line.
                self.sink.first_letter.stop();
                builder.push_forced_break(NodeId(id as u64));
                continue;
            }
            if cv.display == DisplayValue::Ruby
                && let Some(plan) = plan(self.doc, self.cascade, id)
            {
                // The outer ruby container anchors what this one sets aside.
                self.ruby(builder, id, &plan.segments, &[], false)?;
                continue;
            }
            if cv.display == DisplayValue::Contents {
                self.push_nodes(builder, &node.children, ends, letter)?;
                continue;
            }
            let edges = style::inline_edges(cv, id, self.fonts)?;
            let inline = styled(self.doc, self.cascade, cv, id, self.fonts)?;
            self.record_element(id);
            builder.open_inline(NodeId(id as u64), &inline, edges);
            self.push_nodes(builder, &node.children, ends, letter)?;
            builder.close_inline();
        }
        Ok(())
    }

    fn push_text(
        &mut self,
        builder: &mut ParagraphBuilder,
        id: usize,
        ends: (Option<usize>, Option<usize>),
        letter: Letter,
    ) -> Result<(), IfcError> {
        let text = self.doc.nodes[id].text_content().unwrap_or_default();
        let parent = self.doc.parent_of(id).unwrap_or(id);
        let parent_cv = &self.cascade.computed[parent];
        self.sink.text_nowrap.insert(
            id as u64,
            parent_cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
        );
        let collapses = parent_cv.effective_white_space_collapse == WhiteSpaceCollapse::Collapse;
        let start = if collapses && ends.0 == Some(id) {
            text.len() - text.trim_start_matches(is_css_whitespace).len()
        } else {
            0
        };
        let end = if collapses && ends.1 == Some(id) {
            text.trim_end_matches(is_css_whitespace).len().max(start)
        } else {
            text.len()
        };
        if start == end {
            return Ok(());
        }
        // A line that an earlier layout started in this text starts at the
        // same byte again.
        if self.in_line
            && let Some(offset) = self
                .doc
                .continuation_break
                .filter(|&(node, _)| node == id)
                .map(|(_, offset)| offset as usize)
                .filter(|&offset| start <= offset && offset < end && text.is_char_boundary(offset))
        {
            if offset > start {
                self.push_text_range(builder, id, text, start..offset, letter)?;
            }
            self.sink.first_letter.stop();
            builder.push_forced_break(NodeId(id as u64));
            return self.push_text_range(builder, id, text, offset..end, letter);
        }
        self.push_text_range(builder, id, text, start..end, letter)
    }

    /// Push the bytes `range` of the text node `id`, whose text is `text`.
    fn push_text_range(
        &mut self,
        builder: &mut ParagraphBuilder,
        id: usize,
        text: &str,
        range: std::ops::Range<usize>,
        letter: Letter,
    ) -> Result<(), IfcError> {
        let parent_cv = &self.cascade.computed[self.doc.parent_of(id).unwrap_or(id)];
        let (start, end) = (range.start, range.end);
        let source = TextSource::Dom {
            node: NodeId(id as u64),
            offset: start as u32,
        };
        if letter == Letter::Base {
            self.sink.first_letter.push_with_counters(
                builder,
                self.doc,
                self.cascade,
                source,
                parent_cv,
                &text[start..end],
                self.fonts,
                self.sink.counters,
            )
        } else {
            builder.push_text(source, &text[start..end]);
            Ok(())
        }
    }

    /// Push the ruby container `ruby`, grouped into `segments`, with the
    /// boxes it sets aside anchored as `out_of_flow` says.
    ///
    /// shodo gives every segment a container box of its own, without edges.
    /// The container's edges go on one inline box around all the segments,
    /// and each segment's box takes the id of a text of the segment, so the
    /// container's own box is the only one with its id. A container that is
    /// the paragraph's root (`root`) is a block box of its own and gets no
    /// inline box.
    fn ruby(
        &mut self,
        builder: &mut ParagraphBuilder,
        ruby: usize,
        segments: &[Segment],
        out_of_flow: &[(usize, usize)],
        root: bool,
    ) -> Result<(), IfcError> {
        let cv = &self.cascade.computed[ruby];
        let container = styled(self.doc, self.cascade, cv, ruby, self.fonts)?;
        if !root {
            let edges = style::inline_edges(cv, ruby, self.fonts)?;
            self.record_element(ruby);
            builder.open_inline(NodeId(ruby as u64), &container, edges);
        }
        let mut anchors = out_of_flow.iter().peekable();
        for (index, segment) in segments.iter().enumerate() {
            while let Some(&(_, id)) = anchors.next_if(|&&(before, _)| before <= index) {
                self.anchor(builder, id);
            }
            self.push_nodes(builder, &segment.gap, (None, None), Letter::Base)?;
            // A segment names its boxes without an element of their own
            // after its first text, or else after its first node; one without
            // any content draws nothing.
            let parts = || {
                segment
                    .bases
                    .iter()
                    .chain(segment.levels.iter().flat_map(|level| &level.annotations))
            };
            let Some(id) = parts()
                .find_map(|part| {
                    texts(self.doc, self.cascade, &part.children)
                        .first()
                        .copied()
                })
                .or_else(|| {
                    parts().find_map(|part| {
                        part.element.or_else(|| {
                            part.children
                                .iter()
                                .copied()
                                .find(|&id| !is_skipped(self.doc, self.cascade, id))
                        })
                    })
                })
            else {
                continue;
            };
            // A line that an earlier layout started at this segment starts
            // here again.
            if self.in_line && self.doc.continuation_break == Some((id, 0)) {
                self.sink.first_letter.stop();
                builder.push_forced_break(NodeId(id as u64));
            }
            let spaces = intra_level_spaces(segment);
            let mut bases = Vec::with_capacity(segment.bases.len());
            for (index, part) in segment.bases.iter().enumerate() {
                let (node, content) = self.part(part, Letter::Base, id)?;
                bases.push(shodo::RubyBase {
                    node,
                    content,
                    align: shodo::RubyAlign::SpaceAround,
                });
                if index + 1 < segment.bases.len() && spaces.contains(&index) {
                    let (node, content) = self.space(part, id)?;
                    bases.push(shodo::RubyBase {
                        node,
                        content,
                        align: shodo::RubyAlign::SpaceAround,
                    });
                }
            }
            let mut levels = Vec::with_capacity(segment.levels.len());
            for level in &segment.levels {
                levels.push(self.level(ruby, level, &spaces, id)?);
            }
            // Automatic and whole-segment spans always validate.
            if let Ok(value) = shodo::Ruby::new(bases, levels) {
                builder.push_ruby(NodeId(id as u64), &container, value);
            }
        }
        for &(_, id) in anchors {
            self.anchor(builder, id);
        }
        if !root {
            builder.close_inline();
        }
        Ok(())
    }

    /// Anchor the float or absolutely positioned box `id` here in the line.
    fn anchor(&mut self, builder: &mut ParagraphBuilder, id: usize) {
        let float = box_kind(self.cascade, self.doc, id) == Some(IfcBoxKind::Float);
        builder.push_out_of_flow(
            NodeId(id as u64),
            if float {
                OutOfFlowKind::Float
            } else {
                OutOfFlowKind::Absolute
            },
        );
        self.sink.boxes.push(IfcBox {
            node: id,
            kind: if float {
                IfcBoxKind::Float
            } else {
                IfcBoxKind::OutOfFlow
            },
        });
    }

    /// The column of intra-level white space after `part`: its white space,
    /// or an empty box when only another level has white space there. An
    /// empty box takes the id of `text`, the segment's first text.
    fn space(&mut self, part: &Part, text: usize) -> Result<(NodeId, RubyContent), IfcError> {
        let owner = part.style_owner;
        let node = part.tail.first().copied().unwrap_or(text);
        let paragraph = self.paragraph(owner)?;
        let mut builder = ParagraphBuilder::new(&paragraph, self.limits);
        for &id in &part.tail {
            let text = self.doc.nodes[id].text_content().unwrap_or_default();
            builder.push_text(
                TextSource::Dom {
                    node: NodeId(id as u64),
                    offset: 0,
                },
                text,
            );
        }
        Ok((NodeId(node as u64), RubyContent::from_builder(builder)))
    }

    fn level(
        &mut self,
        ruby: usize,
        level: &Level,
        spaces: &[usize],
        text: usize,
    ) -> Result<shodo::RubyLevel, IfcError> {
        if let Some(container) = level.container {
            self.record_element(container);
        }
        let style_owner = level
            .container
            .or_else(|| level.annotations.first().map(|part| part.style_owner))
            .unwrap_or(ruby);
        // The anonymous annotation of a container that holds no annotation
        // box of its own spans the whole segment (CSS Ruby 1 §2.2).
        let spans_all = matches!(level.annotations.as_slice(), [part] if part.element.is_none());
        let mut annotations = Vec::with_capacity(level.annotations.len());
        for (index, part) in level.annotations.iter().enumerate() {
            let (node, content) = self.part(part, Letter::Annotation, text)?;
            annotations.push(shodo::RubyAnnotation {
                node,
                content,
                span: if spans_all {
                    shodo::RubySpan::All
                } else {
                    shodo::RubySpan::Auto
                },
                visibility: visibility(self.cascade.computed[part.style_owner].visibility),
            });
            if index + 1 < level.annotations.len() && spaces.contains(&index) {
                let (node, content) = self.space(part, text)?;
                annotations.push(shodo::RubyAnnotation {
                    node,
                    content,
                    span: shodo::RubySpan::Auto,
                    visibility: shodo::RubyVisibility::Visible,
                });
            }
        }
        Ok(shodo::RubyLevel {
            annotations,
            style: shodo::RubyStyle {
                position: position(self.cascade.computed[style_owner].ruby_position),
                ..shodo::RubyStyle::default()
            },
        })
    }
}

/// The positions after which a segment has intra-level white space in any
/// level: there, every level gets a column of white space, so that the
/// boxes after it stay paired (CSS Ruby 1 §2.3, §2.4).
fn intra_level_spaces(segment: &Segment) -> Vec<usize> {
    let mut spaces = Vec::new();
    let lists = std::iter::once(segment.bases.as_slice()).chain(
        segment
            .levels
            .iter()
            .map(|level| level.annotations.as_slice()),
    );
    for list in lists {
        for (index, part) in list.iter().enumerate() {
            if index + 1 < list.len() && !part.tail.is_empty() && !spaces.contains(&index) {
                spaces.push(index);
            }
        }
    }
    spaces
}

/// shodo's side for the `ruby-position` of an annotation level.
fn position(position: RubyPosition) -> shodo::RubyPosition {
    match position {
        RubyPosition::Over => shodo::RubyPosition::Over,
        RubyPosition::Under => shodo::RubyPosition::Under,
        RubyPosition::InterCharacter => shodo::RubyPosition::InterCharacter,
    }
}

/// shodo's visibility for an annotation of `visibility`.
fn visibility(visibility: Visibility) -> shodo::RubyVisibility {
    match visibility {
        Visibility::Hidden => shodo::RubyVisibility::Hidden,
        Visibility::Collapse => shodo::RubyVisibility::Collapse,
        _ => shodo::RubyVisibility::Visible,
    }
}

/// Push the ruby container `ruby`, laid out as `plan` says. `root` is set
/// when the container is the root of the paragraph.
///
/// # Errors
/// [`IfcError::Unsupported`] for a style the inline engine does not map, and
/// the errors of the paragraph's `::first-letter`.
#[allow(clippy::too_many_arguments)]
pub(super) fn push(
    builder: &mut ParagraphBuilder,
    doc: &Document,
    cascade: &CascadeResult,
    ruby: usize,
    plan: &Plan,
    fonts: &FontCollection,
    limits: &Limits,
    sink: &mut Sink<'_>,
    root: bool,
) -> Result<(), IfcError> {
    Content {
        in_line: true,
        doc,
        cascade,
        fonts,
        limits,
        sink,
    }
    .ruby(builder, ruby, &plan.segments, &plan.out_of_flow, root)
}

#[cfg(test)]
mod tests;
