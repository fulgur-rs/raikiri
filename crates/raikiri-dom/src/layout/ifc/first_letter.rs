//! Source-preserving projection and retained paint for typographic letters.

use super::{error::IfcError, projection::styled, style};
use crate::{Document, generated_content::generated_node_id};
use raikiri_style::property::{DisplayValue, FloatValue, PositionValue, WhiteSpaceCollapse};
use raikiri_style::{CascadeResult, ComputedValues, PseudoElem, StyleNodeId};
use raikiri_traits::NodeKind;
use shodo::{
    ParagraphBuilder,
    font::FontCollection,
    node::{NodeId, TextSource},
};
use std::{collections::VecDeque, ops::Range};

#[derive(Clone, Debug)]
pub(crate) struct LetterStyle {
    pub(crate) box_id: usize,
    pub(crate) parent_box: Option<usize>,
    pub(crate) source_owner: usize,
    pub(crate) source_container: Option<usize>,
    pub(crate) source_range: Option<Range<u32>>,
    pub(crate) computed: ComputedValues,
}

impl LetterStyle {
    pub(crate) fn owns(&self, source: TextSource) -> bool {
        match source {
            TextSource::Dom { node, offset } => {
                node.0 as usize == self.source_owner
                    && self
                        .source_range
                        .as_ref()
                        .is_some_and(|range| range.contains(&offset))
            }
            TextSource::Generated { node } => node.0 as usize == self.box_id,
        }
    }
}

pub(crate) struct FirstLetter {
    origin: usize,
    origins: Vec<usize>,
    line_origins: Vec<usize>,
    pending: bool,
    continuation: VecDeque<(usize, Range<usize>)>,
    started: bool,
    open_boxes: usize,
    text_limit: Option<u64>,
    item_limit: Option<u64>,
    checked_through: Option<usize>,
    pub(crate) styles: Vec<LetterStyle>,
}

impl FirstLetter {
    #[cfg(test)]
    pub(crate) fn new(
        doc: &Document,
        cascade: &CascadeResult,
        origin: usize,
        limits: &shodo::limits::Limits,
    ) -> Self {
        Self::new_with_predecessors(doc, cascade, origin, limits, &PredecessorCache::default())
    }

    pub(crate) fn new_with_predecessors(
        doc: &Document,
        cascade: &CascadeResult,
        origin: usize,
        limits: &shodo::limits::Limits,
        predecessors: &PredecessorCache,
    ) -> Self {
        let mut origins = Vec::new();
        let mut line_origins = Vec::new();
        let mut current = cascade.has_first_letter_styles().then_some(origin);
        while let Some(id) = current {
            let cv = &cascade.computed[id];
            let transparent =
                cv.display == DisplayValue::Contents && !super::assign::is_layout_root(doc, id);
            if !transparent
                && (!matches!(
                    cv.display,
                    DisplayValue::Block
                        | DisplayValue::FlowRoot
                        | DisplayValue::ListItem
                        | DisplayValue::InlineBlock
                        | DisplayValue::TableCell
                        | DisplayValue::TableCaption
                        | DisplayValue::Inline
                        | DisplayValue::Contents
                ) || !super::assign::can_be_ifc_root(doc, cascade, id))
            {
                break;
            }
            if !transparent
                && cascade
                    .pseudo
                    .contains_key(&(StyleNodeId::new(id as u64), PseudoElem::FirstLine))
            {
                line_origins.push(id);
            }
            if !transparent
                && cascade
                    .pseudo
                    .contains_key(&(StyleNodeId::new(id as u64), PseudoElem::FirstLetter))
            {
                origins.push(id);
            }
            // Atomic/BFC roots do not contribute their text to an ancestor's
            // first formatted line. A first block child can contribute it.
            if !transparent
                && (matches!(
                    cv.display,
                    DisplayValue::InlineBlock | DisplayValue::TableCell
                ) || cv.float != FloatValue::None
                    || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed))
            {
                break;
            }
            let Some(parent) = doc.parent_of(id) else {
                break; // cov:ignore: eligible IFC element roots have a document parent; the document root is not a block-container cascade node.
            };
            if crate::generated_content::is_in_flow_generated_text(
                cascade,
                parent,
                PseudoElem::Before,
            ) {
                break;
            }
            let blocked = predecessors.has_preceding(doc, cascade, id);
            if blocked {
                break;
            }
            current = Some(parent);
        }
        origins.reverse();
        line_origins.reverse();
        Self {
            origin,
            pending: !origins.is_empty(),
            origins,
            line_origins,
            continuation: VecDeque::new(),
            started: false,
            open_boxes: 0,
            text_limit: limits.max_text_bytes,
            item_limit: limits.max_items,
            checked_through: None,
            styles: Vec::new(),
        }
    }

    pub(crate) fn stop(&mut self) {
        self.pending = false;
        self.continuation.clear();
    }

    fn adjacent_range(
        &mut self,
        doc: &Document,
        cascade: &CascadeResult,
        source: TextSource,
        text: &str,
        preserve_breaks: bool,
        counters: &super::projection::GeneratedCounters,
    ) -> Result<Option<Range<usize>>, IfcError> {
        // Follow the projected inline stream, rather than DOM siblinghood:
        // punctuation and graphemes can cross ordinary inline box boundaries.
        enum Visit {
            Enter(usize),
            Pseudo(usize, PseudoElem),
        }
        let mut stack = vec![Visit::Pseudo(self.origin, PseudoElem::After)];
        stack.extend(
            doc.get_node(self.origin)
                .into_iter()
                .flat_map(|node| node.children.iter().rev())
                .map(|&id| Visit::Enter(id)),
        );
        stack.push(Visit::Pseudo(self.origin, PseudoElem::Before));
        let owner = match source {
            TextSource::Dom { node, .. } | TextSource::Generated { node } => node.0 as usize,
        };
        let mut started = false;
        let mut joined = String::new();
        let mut pieces = Vec::new();
        while let Some(visit) = stack.pop() {
            let (id, value, barrier) = match visit {
                Visit::Pseudo(id, pseudo) => {
                    if !crate::generated_content::is_in_flow_generated_text(cascade, id, pseudo) {
                        continue;
                    }
                    let Some((cv, value)) = crate::generated_content::generated_text(
                        doc,
                        cascade,
                        id,
                        pseudo,
                        counters.get(doc, cascade)?,
                    ) else {
                        continue; // cov:ignore: in-flow generated text requires a retained pseudo style and nonempty content, so generated_text always returns Some.
                    };
                    (
                        generated_node_id(id, pseudo),
                        std::borrow::Cow::Owned(value),
                        !matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents),
                    )
                }
                Visit::Enter(id) => {
                    let node = doc.get_node(id).ok_or(IfcError::InvalidNode(id))?;
                    match node.kind() {
                        NodeKind::Text => (
                            id,
                            std::borrow::Cow::Borrowed(node.text_content().unwrap_or("")),
                            false,
                        ),
                        NodeKind::Element => {
                            let cv = &cascade.computed[id];
                            if cv.display == DisplayValue::None
                                || node.is_non_rendered_html_element()
                            {
                                continue;
                            }
                            if cv.display != DisplayValue::Contents {
                                match super::projection::box_kind(cascade, doc, id) {
                                    Some(
                                        super::boxes::IfcBoxKind::OutOfFlow
                                        | super::boxes::IfcBoxKind::Float,
                                    ) => continue,
                                    Some(_) => break,
                                    None => {}
                                }
                                if node.tag_name() == Some("br") || node.tag_name() == Some("wbr") {
                                    break;
                                }
                            }
                            stack.push(Visit::Pseudo(id, PseudoElem::After));
                            stack.extend(node.children.iter().rev().map(|&id| Visit::Enter(id)));
                            stack.push(Visit::Pseudo(id, PseudoElem::Before));
                            continue;
                        }
                        _ => continue,
                    }
                }
            };
            if !started {
                if id != owner {
                    if barrier {
                        break;
                    }
                    continue;
                }
                started = true;
            }
            // A later line or atomic box cannot complete this typographic unit.
            if barrier && id != owner {
                break;
            }
            let value = if id == owner { text } else { value.as_ref() };
            let length = joined.len().saturating_add(value.len()) as u64;
            if let Some(limit) = self.text_limit
                && length > limit
            {
                return Err(IfcError::Limit(shodo::limits::LimitExceeded {
                    kind: shodo::limits::LimitKind::TextBytes,
                    limit,
                    actual: length,
                }));
            }
            if id != owner
                && let Some(limit) = self.item_limit
                && pieces.len() as u64 > limit
            {
                return Err(IfcError::Limit(shodo::limits::LimitExceeded {
                    kind: shodo::limits::LimitKind::Items,
                    limit,
                    actual: pieces.len() as u64,
                }));
            }
            let start = joined.len();
            joined.push_str(value);
            pieces.push((id, start..joined.len()));
            self.checked_through = Some(id);
            if barrier
                || shodo::first_letter_range(&joined, preserve_breaks)
                    .is_some_and(|range| range.end < joined.len())
            {
                break;
            }
        }
        let Some(range) = shodo::first_letter_range(&joined, preserve_breaks) else {
            return Ok(None);
        };
        for (id, piece) in pieces.iter().skip(1) {
            if piece.start >= range.end {
                break;
            }
            self.continuation.push_back((
                *id,
                range.start.saturating_sub(piece.start).min(piece.len())
                    ..range.end.min(piece.end) - piece.start,
            ));
        }
        Ok(Some(range.start.min(text.len())..range.end.min(text.len())))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push(
        &mut self,
        builder: &mut ParagraphBuilder,
        doc: &Document,
        cascade: &CascadeResult,
        source: TextSource,
        parent: &ComputedValues,
        text: &str,
        fonts: &FontCollection,
    ) -> Result<(), IfcError> {
        self.push_with_counters(
            builder,
            doc,
            cascade,
            source,
            parent,
            text,
            fonts,
            &super::projection::GeneratedCounters::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_with_counters(
        &mut self,
        builder: &mut ParagraphBuilder,
        doc: &Document,
        cascade: &CascadeResult,
        source: TextSource,
        parent: &ComputedValues,
        text: &str,
        fonts: &FontCollection,
        counters: &super::projection::GeneratedCounters,
    ) -> Result<(), IfcError> {
        if let Some(error) = builder.error() {
            return Err(IfcError::Limit(error));
        }
        if self.origins.is_empty() {
            builder.push_text(source, text);
            return Ok(());
        }
        // Generated pseudo text has its own virtual text identity. Retain
        // byte offsets just as for DOM text so several generated owners and
        // an unselected remainder cannot share one typographic source ID.
        let (node, offset) = match source {
            TextSource::Dom { node, offset } => (node, offset),
            TextSource::Generated { node } => (node, 0),
        };
        let source = TextSource::Dom { node, offset };
        let lookahead = self.checked_through.is_none();
        if matches!(source, TextSource::Dom {node, ..} | TextSource::Generated {node} if self.checked_through == Some(node.0 as usize))
        {
            self.checked_through = None;
        }
        let preserve_breaks = !matches!(
            parent.effective_white_space_collapse,
            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard
        );
        let source_owner = match source {
            TextSource::Dom { node, .. } | TextSource::Generated { node } => node.0 as usize,
        };
        let selected = if self
            .continuation
            .front()
            .is_some_and(|(owner, _)| *owner == source_owner)
        {
            self.continuation.pop_front().map(|(_, range)| range)
        } else if self.pending && !text.is_empty() {
            let range = shodo::first_letter_range(text, preserve_breaks);
            if lookahead && range.as_ref().is_none_or(|range| range.end == text.len()) {
                self.adjacent_range(doc, cascade, source, text, preserve_breaks, counters)?
                    .or(range)
            } else {
                range
            }
        } else {
            None
        };
        if let Some(full_range) = selected {
            self.pending = false;
            let range = full_range.start.min(text.len())..full_range.end.min(text.len());
            if range.is_empty() {
                builder.push_text(source, text);
                return Ok(());
            }
            let offset_at = |index: usize| -> Result<u32, IfcError> {
                u32::try_from(index)
                    .ok()
                    .and_then(|index| offset.checked_add(index))
                    .ok_or(IfcError::Unsupported {
                        node: self.origin,
                        reason: "first-letter source offset exceeds shodo's address range",
                    })
            };
            let source_start = offset_at(range.start)?;
            let source_end = offset_at(range.end)?;
            builder.push_text(source, &text[..range.start]);
            let source_owner = match source {
                TextSource::Dom { node, .. } | TextSource::Generated { node } => node.0 as usize,
            };
            let (actual_parent, generated) =
                crate::generated_content::generated_origin(source_owner).map_or_else(
                    || (doc.parent_of(source_owner).unwrap_or(source_owner), None),
                    |(element, pseudo)| (element, Some(pseudo)),
                );
            let line_origins: Vec<_> = self
                .line_origins
                .iter()
                .map(|&id| StyleNodeId::new(id as u64))
                .collect();
            let mut resolved = cascade
                .first_letter_parent_with_first_lines(
                    doc,
                    StyleNodeId::new(self.origins[0] as u64),
                    &line_origins,
                    StyleNodeId::new(actual_parent as u64),
                    generated,
                )
                .unwrap_or_else(|| parent.clone());
            let context_node = crate::generated_content::generated_origin(source_owner)
                .map_or(source_owner, |(element, _)| element);
            let continuing_box = self.open_boxes > 0;
            let mut box_id = 0;
            for &origin in &self.origins {
                let Some(cv) =
                    cascade.resolve_first_letter_style(StyleNodeId::new(origin as u64), &resolved)
                else {
                    continue; // cov:ignore: origins are retained only for first-letter styles present in this immutable cascade.
                };
                if cv.float != FloatValue::None {
                    return Err(IfcError::Unsupported {
                        node: origin,
                        reason: "floating ::first-letter requires drop-cap box layout",
                    });
                }
                let parent_box = (box_id != 0).then_some(box_id);
                box_id = generated_node_id(origin, PseudoElem::FirstLetter);
                let inline = styled(doc, cascade, &cv, context_node, fonts)?;
                let mut edges = style::inline_edges(&cv, context_node, fonts)?;
                if self.started && !continuing_box {
                    edges.margin.inline_start = 0.0;
                    edges.border.inline_start = 0.0;
                    edges.padding.inline_start = 0.0;
                }
                let container = |owner| {
                    crate::generated_content::generated_origin(owner)
                        .map_or_else(|| doc.parent_of(owner), |_| Some(owner))
                };
                let crosses_box = self
                    .continuation
                    .iter()
                    .any(|(owner, _)| container(*owner) != container(source_owner));
                if crosses_box {
                    edges.margin.inline_end = 0.0;
                    edges.border.inline_end = 0.0;
                    edges.padding.inline_end = 0.0;
                }
                if !continuing_box {
                    builder.open_inline(NodeId(box_id as u64), &inline, edges);
                    self.open_boxes += 1;
                }
                if let Some(error) = builder.error() {
                    return Err(IfcError::Limit(error));
                }
                let source_range = Some(source_start..source_end);
                self.styles.push(LetterStyle {
                    box_id,
                    parent_box,
                    source_owner,
                    source_container: crate::generated_content::generated_origin(source_owner)
                        .map_or_else(
                            || {
                                let mut container = doc.parent_of(source_owner);
                                while let Some(id) = container {
                                    if id == self.origin {
                                        return None;
                                    }
                                    if cascade.computed[id].display != DisplayValue::Contents {
                                        break;
                                    }
                                    container = doc.parent_of(id);
                                }
                                container
                            },
                            |_| Some(source_owner),
                        ),
                    source_range,
                    computed: cv.clone(),
                });
                resolved = cv;
            }
            let first_source = TextSource::Dom {
                node,
                offset: source_start,
            };
            // Source offsets refer to the original UTF-8 text, including
            // the unstyled whitespace preceding the typographic unit.
            builder.push_text(first_source, &text[range.clone()]);
            self.started = true;
            let container = |owner| {
                crate::generated_content::generated_origin(owner)
                    .map_or_else(|| doc.parent_of(owner), |_| Some(owner))
            };
            if self
                .continuation
                .front()
                .is_none_or(|(owner, _)| container(*owner) != container(source_owner))
            {
                for _ in 0..self.open_boxes {
                    builder.close_inline();
                }
                self.open_boxes = 0;
            }
            let rest_source = TextSource::Dom {
                node,
                offset: source_end,
            };
            builder.push_text(rest_source, &text[range.end..]);
            return Ok(());
        }
        if self.pending {
            // A preserved line break or punctuation-only first content has
            // no typographic unit. Collapsed whitespace can precede a later node.
            if text.chars().any(|ch| !ch.is_whitespace())
                || (preserve_breaks && text.contains(['\n', '\r']))
            {
                self.pending = false;
            }
        }
        builder.push_text(source, text);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

/// Immutable eligibility data shared by all roots in one projection pass.
/// Each source node and sibling edge is inspected once, including Contents
/// subtrees; a wide comment prefix is never scanned again for another root.
#[derive(Default)]
pub(crate) struct PredecessorCache(std::cell::OnceCell<Vec<bool>>);

#[cfg(test)]
thread_local! {
    static PREDECESSOR_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl PredecessorCache {
    fn has_preceding(&self, doc: &Document, cascade: &CascadeResult, node: usize) -> bool {
        self.0.get_or_init(|| Self::build(doc, cascade))[node]
    }

    fn build(doc: &Document, cascade: &CascadeResult) -> Vec<bool> {
        let mut contributes = vec![false; doc.node_count()];
        let mut blocked = vec![false; doc.node_count()];
        let mut pending: Vec<_> = doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.parent.is_none())
            .map(|(id, _)| (id, false))
            .collect();
        while let Some((id, visited)) = pending.pop() {
            let node = &doc.nodes[id];
            if !visited {
                pending.push((id, true));
                pending.extend(node.children.iter().map(|&child| (child, false)));
                continue;
            }
            #[cfg(test)]
            PREDECESSOR_VISITS.with(|visits| visits.set(visits.get() + 1));
            let cv = &cascade.computed[id];
            contributes[id] =
                if cv.display == DisplayValue::None || node.is_non_rendered_html_element() {
                    false
                } else if cv.display == DisplayValue::Contents {
                    [PseudoElem::Before, PseudoElem::After]
                        .iter()
                        .any(|&pseudo| {
                            crate::generated_content::is_in_flow_generated_text(cascade, id, pseudo)
                        })
                        || node.children.iter().any(|&child| contributes[child])
                } else if cv.float != FloatValue::None
                    || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
                {
                    false
                } else {
                    match node.kind() {
                        NodeKind::Text => {
                            node.text_content()
                                .unwrap_or("")
                                .chars()
                                .any(|ch| !ch.is_whitespace())
                                || !matches!(
                                    cv.effective_white_space_collapse,
                                    WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard
                                )
                        }
                        NodeKind::Element => true,
                        _ => false,
                    }
                };
            let mut preceding = false;
            for &child in &node.children {
                #[cfg(test)]
                PREDECESSOR_VISITS.with(|visits| visits.set(visits.get() + 1));
                blocked[child] = preceding;
                preceding |= contributes[child];
            }
        }
        blocked
    }
}
