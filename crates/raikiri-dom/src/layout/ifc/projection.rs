//! Project one in-flow block's inline content into a shodo paragraph.
//!
//! The first slice handles text, ordinary inline elements, and `<br>`.
//! Anything the inline path cannot place yet (floats, positioned boxes,
//! block children, inline-blocks, replaced elements, generated content) is
//! rejected with [`IfcError::Unsupported`] rather than approximated.

use super::error::IfcError;
use super::style;
use crate::Document;
use raikiri_style::property::{ContentComponent, DisplayValue, FloatValue, PositionValue};
use raikiri_style::{CascadeResult, ComputedTextIndent, PseudoElem, StyleNodeId};
use raikiri_traits::NodeKind;
use shodo::font::FontCollection;
use shodo::limits::Limits;
use shodo::node::{NodeId, TextSource};
use shodo::style::LineOptions;
use shodo::{LayoutContext, Paragraph, ParagraphBuilder};

/// Tags of replaced or form-control elements, which are atomic inlines.
const ATOMIC_TAGS: &[&str] = &[
    "img", "svg", "canvas", "video", "audio", "iframe", "object", "embed", "input", "button",
    "select", "textarea", "math",
];

/// A built paragraph together with the block's line options.
#[derive(Debug)]
pub(crate) struct ProjectedIfc {
    /// The shaped paragraph.
    pub(crate) paragraph: Paragraph,
    /// Line options of the block root.
    pub(crate) options: LineOptions,
    /// The block's raw `text-indent`, to be resolved against its width.
    pub(crate) indent: ComputedTextIndent,
    /// Node id of the block root.
    pub(crate) root: usize,
}

/// The effective `lang` of `node`: the nearest ancestor-or-self `lang`
/// attribute, trimmed and lowercased. An empty value means "unknown" and stops
/// inheritance.
pub(crate) fn language_of(doc: &Document, node: usize) -> Option<String> {
    let mut current = Some(node);
    while let Some(id) = current {
        if let Some(value) = doc.get_node(id).and_then(|n| n.attribute("lang")) {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_ascii_lowercase());
        }
        current = doc.parent_of(id);
    }
    None
}

/// Generated `::before` / `::after` content is painted as an overlay, not laid
/// out in the paragraph, so a node that renders any is not projected.
fn reject_generated_content(cascade: &CascadeResult, node: usize) -> Result<(), IfcError> {
    for pseudo in [PseudoElem::Before, PseudoElem::After] {
        let Some(cv) = cascade.pseudo.get(&(StyleNodeId::new(node as u64), pseudo)) else {
            continue;
        };
        let renders = cv.display != DisplayValue::None
            && !cv.content.is_empty()
            && !cv
                .content
                .iter()
                .any(|part| matches!(part, ContentComponent::None));
        if renders {
            return Err(IfcError::Unsupported {
                node,
                reason: "generated content is painted as an overlay",
            });
        }
    }
    Ok(())
}

enum Step {
    Enter(usize),
    Close,
}

/// Build the shodo paragraph for the in-flow block `root`.
///
/// # Errors
/// [`IfcError::InvalidNode`] for an unknown or detached node,
/// [`IfcError::Unsupported`] for anything the first slice does not place, and
/// [`IfcError::Limit`] when a shodo resource limit is exceeded.
pub(crate) fn project_ifc(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    cx: &mut LayoutContext,
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedIfc, IfcError> {
    let root_node = doc.get_node(root).ok_or(IfcError::InvalidNode(root))?;
    let root_cv = cascade
        .computed
        .get(root)
        .ok_or(IfcError::InvalidNode(root))?;
    if !root_node.is_in_document()
        || root_node.kind() != NodeKind::Element
        || root_cv.display != DisplayValue::Block
    {
        return Err(IfcError::Unsupported {
            node: root,
            reason: "root must be an in-flow block",
        });
    }
    reject_generated_content(cascade, root)?;

    // The cascade has already resolved `inherit`, `match-parent`, and
    // `-internal-center` in the computed `text-align` values.
    let (options, indent) = style::line_options(root_cv, root)?;
    let mut root_style = style::inline_style(root_cv, root)?;
    root_style.lang = language_of(doc, root);
    let paragraph_style = style::paragraph_style(root_cv, root, root_style)?;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);

    let mut stack: Vec<Step> = root_node
        .children
        .iter()
        .rev()
        .map(|&child| Step::Enter(child))
        .collect();
    while let Some(step) = stack.pop() {
        let id = match step {
            Step::Close => {
                builder.close_inline();
                continue;
            }
            Step::Enter(id) => id,
        };
        let node = doc.get_node(id).ok_or(IfcError::InvalidNode(id))?;
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                let text = node.text_content().ok_or(IfcError::InvalidNode(id))?;
                builder.push_text(
                    TextSource::Dom {
                        node: NodeId(id as u64),
                        offset: 0,
                    },
                    text,
                );
            }
            NodeKind::Comment | NodeKind::ProcessingInstruction => {}
            NodeKind::Element => {
                let cv = cascade.computed.get(id).ok_or(IfcError::InvalidNode(id))?;
                // `<style>`, `<script>` and friends render nothing even when
                // an author rule gives them a display type.
                if cv.display == DisplayValue::None || node.is_non_rendered_html_element() {
                    continue;
                }
                let unsupported = |reason: &'static str| IfcError::Unsupported { node: id, reason };
                if cv.float != FloatValue::None {
                    return Err(unsupported("floats are not placed by the inline path yet"));
                }
                if !matches!(cv.position, PositionValue::Static | PositionValue::Relative) {
                    return Err(unsupported("positioned inline boxes are not supported yet"));
                }
                if cv.display != DisplayValue::Inline {
                    return Err(unsupported("only inline-level boxes are projected"));
                }
                let tag = node.tag_name().unwrap_or_default();
                if ATOMIC_TAGS.contains(&tag) {
                    return Err(unsupported("atomic inlines are not measured yet"));
                }
                reject_generated_content(cascade, id)?;
                let mut inline_style = style::inline_style(cv, id)?;
                inline_style.lang = language_of(doc, id);
                let edges = style::inline_edges(cv, id)?;
                builder.open_inline(NodeId(id as u64), &inline_style, edges);
                if tag == "br" {
                    builder.push_forced_break(NodeId(id as u64));
                    builder.close_inline();
                } else {
                    stack.push(Step::Close);
                    stack.extend(node.children.iter().rev().map(|&child| Step::Enter(child)));
                }
            }
            _ => {
                return Err(IfcError::Unsupported {
                    node: id,
                    reason: "node kind is not part of an inline formatting context",
                });
            }
        }
        if let Some(error) = builder.error() {
            return Err(IfcError::Limit(error));
        }
    }
    let paragraph = builder.build(cx, fonts).map_err(IfcError::Limit)?;
    Ok(ProjectedIfc {
        paragraph,
        options,
        indent,
        root,
    })
}

#[cfg(test)]
mod tests;
