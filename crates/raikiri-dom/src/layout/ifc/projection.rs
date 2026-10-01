//! Project one in-flow block's inline content into a shodo paragraph.
//!
//! Text, ordinary inline elements, `<br>`, left or right floats, atomic
//! inlines (inline-blocks, images, inline SVG) and block children of the root
//! are projected; a float becomes an anchor in the text, an atomic a
//! placeholder and a block a break between lines, and all three are laid out
//! as boxes of their own. Anything the inline path cannot place yet
//! (positioned boxes, blocks inside inline elements or with vertical margins,
//! boxes in right-to-left paragraphs, form controls and other replaced
//! elements, generated content) is rejected with [`IfcError::Unsupported`]
//! rather than approximated.

use super::boxes::{IfcBox, IfcBoxKind};
use super::error::IfcError;
use super::style;
use crate::Document;
use raikiri_style::property::{
    ClearValue, ContentComponent, Direction, DisplayValue, FloatValue, OverflowValue,
    PositionValue, UnicodeBidi, WhiteSpaceCollapse,
};
use raikiri_style::{
    CascadeResult, ComputedLengthPercentageOrAuto, ComputedTextIndent, ComputedValues, PseudoElem,
    StyleNodeId,
};
use raikiri_traits::NodeKind;
use shodo::font::FontCollection;
use shodo::limits::Limits;
use shodo::node::{InlineEdges, NodeId, OutOfFlowKind, TextSource};
use shodo::style::LineOptions;
use shodo::{LayoutContext, Paragraph, ParagraphBuilder};

/// Tags of replaced elements the inline engine sizes as atomic inlines.
const ATOMIC_TAGS: &[&str] = &["img", "svg"];

/// Replaced and form-control elements the inline engine does not size yet.
/// They are refused whatever their `display` or `float`: an author rule can
/// make a form control `inline-block`, and taffy would size it as an empty
/// block.
const UNSUPPORTED_REPLACED_TAGS: &[&str] = &[
    "canvas", "video", "audio", "iframe", "object", "embed", "input", "button", "select",
    "textarea", "math",
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
    /// Children laid out as boxes of their own, in document order.
    pub(crate) boxes: Vec<IfcBox>,
    /// The root's `direction` is `rtl`: its lines start at the right edge.
    pub(crate) rtl: bool,
    /// Paint offsets of the relatively positioned inline elements, by DOM
    /// node id.
    pub(crate) offsets: Vec<(usize, (f32, f32))>,
    /// Text nodes whose spaces are preserved (not collapsed), in document
    /// order.
    pub(crate) preserved_spaces: Vec<usize>,
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

/// The box kind of a child of an ifc root, from its computed style; `None` for
/// content that belongs to the paragraph.
pub(crate) fn box_kind(cascade: &CascadeResult, doc: &Document, id: usize) -> Option<IfcBoxKind> {
    let cv = cascade.computed.get(id)?;
    let node = doc.get_node(id)?;
    if node.kind() != NodeKind::Element {
        return None;
    }
    if cv.float != FloatValue::None {
        return Some(IfcBoxKind::Float);
    }
    let inline_block = matches!(
        cv.display,
        DisplayValue::InlineBlock | DisplayValue::InlineFlex | DisplayValue::InlineGrid
    );
    let tag = node.tag_name().unwrap_or("");
    let replaced = cv.display == DisplayValue::Inline && ATOMIC_TAGS.contains(&tag);
    if inline_block || replaced {
        return Some(IfcBoxKind::Atomic);
    }
    // `flow-root`, flex and grid boxes avoid floats as formatting contexts of
    // their own; taffy places those in the parent's item loop, which this
    // path does not run, so they stay unsupported.
    // A block-level image or SVG is sized by its replaced-element path, which
    // this path does not reproduce; it stays unsupported.
    if cv.display == DisplayValue::Block && !ATOMIC_TAGS.contains(&tag) {
        return Some(IfcBoxKind::Block);
    }
    None
}

/// A float the inline engine can place: left or right, cleared by physical
/// sides only, in normal position.
fn supported_float(cv: &ComputedValues, node: usize) -> Result<(), IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    if !matches!(cv.float, FloatValue::Left | FloatValue::Right) {
        return Err(unsupported("logical float sides are not placed yet"));
    }
    if !matches!(
        cv.clear,
        ClearValue::None | ClearValue::Left | ClearValue::Right | ClearValue::Both
    ) {
        return Err(unsupported("logical clear sides are not placed yet"));
    }
    if cv.position != PositionValue::Static {
        return Err(unsupported("positioned floats are not placed yet"));
    }
    Ok(())
}

/// An atomic inline the inline engine can place: in normal position.
fn supported_atomic(cv: &ComputedValues, node: usize) -> Result<(), IfcError> {
    if cv.position != PositionValue::Static {
        return Err(IfcError::Unsupported {
            node,
            reason: "positioned atomic inlines are not placed yet",
        });
    }
    Ok(())
}

/// A block child the inline engine can place between lines: in normal
/// position, with no vertical margin (collapsing it with the lines and with
/// other blocks is not modelled), no `auto` side margin, not a scroll
/// container and not cleared.
fn supported_block(cv: &ComputedValues, node: usize) -> Result<(), IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    if cv.position != PositionValue::Static && !style::is_inert_relative(cv) {
        return Err(unsupported("positioned blocks are not placed yet"));
    }
    let zero = |value: ComputedLengthPercentageOrAuto| matches!(value, ComputedLengthPercentageOrAuto::Px(px) if px == 0.0);
    if !zero(cv.margin.top) || !zero(cv.margin.bottom) {
        return Err(unsupported(
            "vertical margins of a block child are not collapsed yet",
        ));
    }
    if matches!(cv.margin.left, ComputedLengthPercentageOrAuto::Auto)
        || matches!(cv.margin.right, ComputedLengthPercentageOrAuto::Auto)
    {
        return Err(unsupported(
            "auto side margins of a block child are not resolved yet",
        ));
    }
    if cv.overflow.x != OverflowValue::Visible || cv.overflow.y != OverflowValue::Visible {
        return Err(unsupported("a block child that clips is not placed yet"));
    }
    if cv.clear != ClearValue::None {
        return Err(unsupported("a cleared block child is not placed yet"));
    }
    Ok(())
}

/// Whether `text` holds a character of a right-to-left script or an explicit
/// bidi control.
fn has_rtl_char(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(
            c as u32,
            0x0590..=0x08FF
                | 0xFB1D..=0xFDFF
                | 0xFE70..=0xFEFF
                | 0x10800..=0x10FFF
                | 0x1E800..=0x1EFFF
                | 0x200E..=0x200F
                | 0x202A..=0x202E
                | 0x2066..=0x2069
        )
    })
}

/// Right-to-left content of a paragraph, outside its boxes (a box lays out
/// its own content).
#[derive(Clone, Copy, Default)]
struct BidiContent {
    /// The root or an inline element has `direction: rtl`, or some text holds
    /// a right-to-left character or an explicit bidi control.
    rtl: bool,
    /// The root or an inline element has a `unicode-bidi` value other than
    /// `normal`.
    unicode_bidi: bool,
}

fn bidi_content(doc: &Document, cascade: &CascadeResult, root: usize) -> BidiContent {
    let mut found = BidiContent::default();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                found.rtl |= node.text_content().is_some_and(has_rtl_char);
            }
            NodeKind::Element => {
                if id != root && box_kind(cascade, doc, id).is_some() {
                    continue;
                }
                let Some(cv) = cascade.computed.get(id) else {
                    continue;
                };
                found.rtl |= cv.direction != Direction::Ltr;
                found.unicode_bidi |= cv.unicode_bidi != UnicodeBidi::Normal;
                stack.extend(node.children.iter().copied());
            }
            _ => {}
        }
    }
    found
}

enum Step {
    /// A node to project, and whether it sits inside an inline element.
    Enter(usize, bool),
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
    let bidi = bidi_content(doc, cascade, root);
    // Without right-to-left content nothing is reordered, so `unicode-bidi`
    // changes nothing. With it, the parley path orders the text without
    // reading `unicode-bidi` and so differs for every other value.
    if bidi.rtl && bidi.unicode_bidi {
        return Err(IfcError::Unsupported {
            node: root,
            reason: "unicode-bidi is not read by the parley path, which orders right-to-left text differently",
        });
    }

    // The cascade has already resolved `inherit`, `match-parent`, and
    // `-internal-center` in the computed `text-align` values.
    let (options, indent) = style::line_options(root_cv, root, fonts)?;
    let mut root_style = style::inline_style(root_cv, root, fonts)?;
    root_style.lang = language_of(doc, root);
    let paragraph_style = style::paragraph_style(root_cv, root, root_style)?;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let mut boxes = Vec::new();
    let mut offsets = Vec::new();
    let mut preserved_spaces = Vec::new();

    let mut stack: Vec<Step> = root_node
        .children
        .iter()
        .rev()
        .map(|&child| Step::Enter(child, false))
        .collect();
    while let Some(step) = stack.pop() {
        let (id, nested) = match step {
            Step::Close => {
                builder.close_inline();
                continue;
            }
            Step::Enter(id, nested) => (id, nested),
        };
        let node = doc.get_node(id).ok_or(IfcError::InvalidNode(id))?;
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                let text = node.text_content().ok_or(IfcError::InvalidNode(id))?;
                if cascade.computed.get(id).is_some_and(|cv| {
                    matches!(
                        cv.effective_white_space_collapse,
                        WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::PreserveSpaces
                    )
                }) {
                    preserved_spaces.push(id);
                }
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
                let tag = node.tag_name().unwrap_or_default();
                if UNSUPPORTED_REPLACED_TAGS.contains(&tag) {
                    return Err(unsupported("this replaced element is not sized yet"));
                }
                if bidi.rtl && box_kind(cascade, doc, id).is_some() {
                    return Err(unsupported(
                        "boxes in right-to-left paragraphs are not placed yet",
                    ));
                }
                if cv.display == DisplayValue::Contents {
                    // A `display: contents` element generates no box: its
                    // children take part in the paragraph as if they were the
                    // element's siblings. Anything that would give the element
                    // a box of its own is left to the parley path.
                    reject_generated_content(cascade, id)?;
                    if cv.float != FloatValue::None
                        || !matches!(cv.position, PositionValue::Static | PositionValue::Relative)
                    {
                        return Err(unsupported(
                            "a display: contents element that is floated or positioned is not projected",
                        ));
                    }
                    // Its children count as nested: a box among them would sit
                    // below an element of the paragraph's subtree, which the
                    // passes that skip that subtree never reach.
                    stack.extend(
                        node.children
                            .iter()
                            .rev()
                            .map(|&child| Step::Enter(child, true)),
                    );
                    continue;
                }
                // A box is laid out relative to the root, while its DOM parent
                // is the inline element, which now has a layout of its own:
                // readers that add up the locations of the DOM parents would
                // shift the box twice.
                if nested
                    && matches!(
                        box_kind(cascade, doc, id),
                        Some(IfcBoxKind::Float | IfcBoxKind::Atomic)
                    )
                {
                    return Err(unsupported(
                        "a box inside an inline element is located from the element, which is not modelled yet",
                    ));
                }
                if cv.float != FloatValue::None {
                    supported_float(cv, id)?;
                    builder.push_out_of_flow(NodeId(id as u64), OutOfFlowKind::Float);
                    boxes.push(IfcBox {
                        node: id,
                        kind: IfcBoxKind::Float,
                    });
                    if let Some(error) = builder.error() {
                        return Err(IfcError::Limit(error));
                    }
                    continue;
                }
                if box_kind(cascade, doc, id) == Some(IfcBoxKind::Atomic) {
                    supported_atomic(cv, id)?;
                    let mut atomic_style = style::inline_style(cv, id, fonts)?;
                    atomic_style.lang = language_of(doc, id);
                    // shodo sizes an atomic from `AtomicSize` alone, margins
                    // included; the edges are not read for atomics.
                    builder.push_atomic(NodeId(id as u64), &atomic_style, InlineEdges::default());
                    boxes.push(IfcBox {
                        node: id,
                        kind: IfcBoxKind::Atomic,
                    });
                    if let Some(error) = builder.error() {
                        return Err(IfcError::Limit(error));
                    }
                    continue;
                }
                if box_kind(cascade, doc, id) == Some(IfcBoxKind::Block) {
                    if nested {
                        return Err(unsupported(
                            "a block inside an inline element is not placed yet",
                        ));
                    }
                    supported_block(cv, id)?;
                    builder.push_block_in_inline(NodeId(id as u64));
                    boxes.push(IfcBox {
                        node: id,
                        kind: IfcBoxKind::Block,
                    });
                    if let Some(error) = builder.error() {
                        return Err(IfcError::Limit(error));
                    }
                    continue;
                }
                if !matches!(cv.position, PositionValue::Static | PositionValue::Relative) {
                    return Err(unsupported("positioned inline boxes are not supported yet"));
                }
                if cv.display != DisplayValue::Inline {
                    return Err(unsupported("only inline-level boxes are projected"));
                }
                reject_generated_content(cascade, id)?;
                let mut inline_style = style::inline_style(cv, id, fonts)?;
                inline_style.lang = language_of(doc, id);
                let edges = style::inline_edges(cv, id, fonts)?;
                // The eligibility check keeps every offset that is not a plain
                // length out of the paragraph.
                if cv.position == PositionValue::Relative
                    && let Some(offset) = style::relative_offset(cv)
                {
                    offsets.push((id, offset));
                }
                builder.open_inline(NodeId(id as u64), &inline_style, edges);
                if tag == "br" {
                    builder.push_forced_break(NodeId(id as u64));
                    builder.close_inline();
                } else {
                    stack.push(Step::Close);
                    stack.extend(
                        node.children
                            .iter()
                            .rev()
                            .map(|&child| Step::Enter(child, true)),
                    );
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
        boxes,
        rtl: root_cv.direction == Direction::Rtl,
        offsets,
        preserved_spaces,
    })
}

#[cfg(test)]
mod tests;
