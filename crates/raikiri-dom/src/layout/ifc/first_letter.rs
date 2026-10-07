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
use std::ops::Range;

#[derive(Clone, Debug)]
pub(crate) struct LetterStyle {
    pub(crate) box_id: usize,
    pub(crate) parent_box: Option<usize>,
    pub(crate) source_owner: usize,
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
    line_origin: Option<usize>,
    pending: bool,
    continuation: Option<(usize, Range<usize>)>,
    open_boxes: usize,
    text_limit: Option<u64>,
    item_limit: Option<u64>,
    checked_through: Option<usize>,
    pub(crate) styles: Vec<LetterStyle>,
}

impl FirstLetter {
    pub(crate) fn new(
        doc: &Document,
        cascade: &CascadeResult,
        origin: usize,
        limits: &shodo::limits::Limits,
    ) -> Self {
        let mut origins = Vec::new();
        let mut line_origin = None;
        let mut current = cascade.has_first_letter_styles().then_some(origin);
        while let Some(id) = current {
            let cv = &cascade.computed[id];
            if !matches!(
                cv.display,
                DisplayValue::Block
                    | DisplayValue::FlowRoot
                    | DisplayValue::ListItem
                    | DisplayValue::InlineBlock
                    | DisplayValue::TableCell
                    | DisplayValue::TableCaption
            ) {
                break;
            }
            if line_origin.is_none()
                && cascade
                    .pseudo
                    .contains_key(&(StyleNodeId::new(id as u64), PseudoElem::FirstLine))
            {
                line_origin = Some(id);
            }
            if cascade
                .pseudo
                .contains_key(&(StyleNodeId::new(id as u64), PseudoElem::FirstLetter))
            {
                origins.push(id);
            }
            // Atomic/BFC roots do not contribute their text to an ancestor's
            // first formatted line. A first block child can contribute it.
            if matches!(
                cv.display,
                DisplayValue::InlineBlock | DisplayValue::TableCell
            ) || cv.float != FloatValue::None
                || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
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
            let Some(node) = doc.get_node(parent) else {
                break; // cov:ignore: parent IDs were validated by the document tree and cascade traversal.
            };
            let blocked = node
                .children
                .iter()
                .take_while(|&&sibling| sibling != id)
                .any(|&sibling| {
                    let sibling_cv = &cascade.computed[sibling];
                    let Some(node) = doc.get_node(sibling) else {
                        return false; // cov:ignore: this document owns and validates the sibling IDs in its child graph.
                    };
                    if sibling_cv.display == DisplayValue::None
                        || sibling_cv.float != FloatValue::None
                        || matches!(
                            sibling_cv.position,
                            PositionValue::Absolute | PositionValue::Fixed
                        )
                        || node.is_non_rendered_html_element()
                    {
                        return false;
                    }
                    match node.kind() {
                        NodeKind::Text => {
                            node.text_content()
                                .unwrap_or("")
                                .chars()
                                .any(|ch| !ch.is_whitespace())
                                || !matches!(
                                    sibling_cv.effective_white_space_collapse,
                                    WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard
                                )
                        }
                        NodeKind::Element => true,
                        _ => false,
                    }
                });
            if blocked {
                break;
            }
            current = Some(parent);
        }
        origins.reverse();
        Self {
            origin,
            pending: !origins.is_empty(),
            origins,
            line_origin,
            continuation: None,
            open_boxes: 0,
            text_limit: limits.max_text_bytes,
            item_limit: limits.max_items,
            checked_through: None,
            styles: Vec::new(),
        }
    }

    pub(crate) fn stop(&mut self) {
        self.pending = false;
        self.continuation = None;
    }

    fn adjacent_range(
        &mut self,
        doc: &Document,
        source: TextSource,
        text: &str,
        preserve_breaks: bool,
    ) -> Result<Option<Range<usize>>, IfcError> {
        let TextSource::Dom { node, .. } = source else {
            return Ok(None);
        };
        let id = node.0 as usize;
        let Some(parent) = doc.parent_of(id).and_then(|parent| doc.get_node(parent)) else {
            return Ok(None);
        };
        let mut pieces = Vec::new();
        let mut length = text.len() as u64;
        for &sibling in parent
            .children
            .iter()
            .skip_while(|&&sibling| sibling != id)
            .skip(1)
        {
            let Some(node) = doc.get_node(sibling) else {
                break; // cov:ignore: source lookahead visits only this document's validated child IDs.
            };
            match node.kind() {
                NodeKind::Comment | NodeKind::ProcessingInstruction => continue,
                NodeKind::Text => {
                    let next = node.text_content().unwrap_or("");
                    length = length.saturating_add(next.len() as u64);
                    if let Some(limit) = self.text_limit
                        && length > limit
                    {
                        return Err(IfcError::Limit(shodo::limits::LimitExceeded {
                            kind: shodo::limits::LimitKind::TextBytes,
                            limit,
                            actual: length,
                        }));
                    }
                    if let Some(limit) = self.item_limit
                        && pieces.len() as u64 >= limit
                    {
                        return Err(IfcError::Limit(shodo::limits::LimitExceeded {
                            kind: shodo::limits::LimitKind::Items,
                            limit,
                            actual: pieces.len() as u64 + 1,
                        }));
                    }
                    pieces.push(next);
                    self.checked_through = Some(sibling);
                }
                _ => break,
            }
        }
        if pieces.is_empty() {
            return Ok(None);
        }
        let mut joined = String::with_capacity(length as usize);
        joined.push_str(text);
        for piece in pieces {
            joined.push_str(piece);
        }
        Ok(shodo::first_letter_range(&joined, preserve_breaks))
    }

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
        if let Some(error) = builder.error() {
            return Err(IfcError::Limit(error));
        }
        let lookahead = self.checked_through.is_none();
        if matches!(source, TextSource::Dom {node, ..} if self.checked_through == Some(node.0 as usize))
        {
            self.checked_through = None;
        }
        let preserve_breaks = !matches!(
            parent.effective_white_space_collapse,
            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard
        );
        let dom_parent = match source {
            TextSource::Dom { node, .. } => doc.parent_of(node.0 as usize),
            TextSource::Generated { .. } => None,
        };
        let selected = if let Some((owner, range)) = self.continuation.take() {
            if dom_parent == Some(owner) {
                Some(range)
            } else {
                None
            }
        } else if self.pending && !text.is_empty() {
            let range = shodo::first_letter_range(text, preserve_breaks);
            if lookahead && range.as_ref().is_none_or(|range| range.end == text.len()) {
                self.adjacent_range(doc, source, text, preserve_breaks)?
                    .or(range)
            } else {
                range
            }
        } else {
            None
        };
        if let Some(full_range) = selected {
            self.pending = false;
            if full_range.end > text.len()
                && let Some(parent) = dom_parent
            {
                self.continuation = Some((
                    parent,
                    full_range.start.saturating_sub(text.len())..full_range.end - text.len(),
                ));
            }
            let range = full_range.start.min(text.len())..full_range.end.min(text.len());
            if range.is_empty() {
                builder.push_text(source, text);
                return Ok(());
            }
            let offset_at = |index: usize| -> Result<u32, IfcError> {
                match source {
                    TextSource::Dom { offset, .. } => u32::try_from(index)
                        .ok()
                        .and_then(|index| offset.checked_add(index))
                        .ok_or(IfcError::Unsupported {
                            node: self.origin,
                            reason: "first-letter source offset exceeds shodo's address range",
                        }),
                    TextSource::Generated { .. } => Ok(0),
                }
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
            let mut resolved = self
                .line_origin
                .and_then(|line_origin| {
                    cascade.first_letter_parent_with_first_line(
                        doc,
                        StyleNodeId::new(self.origins[0] as u64),
                        StyleNodeId::new(line_origin as u64),
                        StyleNodeId::new(actual_parent as u64),
                        generated,
                    )
                })
                .unwrap_or_else(|| parent.clone());
            let context_node = crate::generated_content::generated_origin(source_owner)
                .map_or(source_owner, |(element, _)| element);
            let continuing_box = self.open_boxes > 0;
            let mut box_id = 0;
            for &origin in &self.origins {
                let Some(cv) =
                    cascade.resolve_first_letter_style(StyleNodeId::new(origin as u64), &resolved)
                else {
                    continue;
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
                let edges = style::inline_edges(&cv, context_node, fonts)?;
                if !continuing_box {
                    builder.open_inline(NodeId(box_id as u64), &inline, edges);
                    self.open_boxes += 1;
                }
                if let Some(error) = builder.error() {
                    return Err(IfcError::Limit(error));
                }
                let source_range = match source {
                    TextSource::Dom { .. } => Some(source_start..source_end),
                    TextSource::Generated { .. } => None,
                };
                self.styles.push(LetterStyle {
                    box_id,
                    parent_box,
                    source_owner,
                    source_range,
                    computed: cv.clone(),
                });
                resolved = cv;
            }
            let first_source = match source {
                TextSource::Dom { node, .. } => TextSource::Dom {
                    node,
                    offset: source_start,
                },
                TextSource::Generated { .. } => TextSource::Generated {
                    node: NodeId(box_id as u64),
                },
            };
            // Source offsets refer to the original UTF-8 text, including
            // the unstyled whitespace preceding the typographic unit.
            builder.push_text(first_source, &text[range.clone()]);
            if self.continuation.is_none() {
                for _ in 0..self.open_boxes {
                    builder.close_inline();
                }
                self.open_boxes = 0;
            }
            let rest_source = match source {
                TextSource::Dom { node, .. } => TextSource::Dom {
                    node,
                    offset: source_end,
                },
                TextSource::Generated { .. } => source,
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
