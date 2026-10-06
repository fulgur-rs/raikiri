//! Project one in-flow block's inline content into a shodo paragraph.
//!
//! Text, the text of in-flow `::before` and `::after`, ordinary inline
//! elements, `<br>`, left or right floats, atomic inlines (inline-blocks,
//! replaced elements, form controls) and block children of the root are
//! projected; a float becomes an anchor in the text, an atomic a placeholder
//! and a block a break between lines, and all three are laid out as boxes of
//! their own, inside inline elements as well. Anything the inline path cannot
//! place yet (positioned boxes) is rejected with [`IfcError::Unsupported`]
//! rather than approximated.

use super::boxes::{IfcBox, IfcBoxKind};
use super::error::IfcError;
use super::style;
use crate::Document;
use crate::generated_content::{generated_node_id, generated_text, is_in_flow_generated_text};
use crate::target::{CounterSnapshot, CounterSnapshotLimitExceeded};
use raikiri_style::property::{ClearValue, Direction, DisplayValue, FloatValue, PositionValue};
use raikiri_style::{CascadeResult, ComputedTextIndent, ComputedValues, PseudoElem};
use raikiri_traits::NodeKind;
use shodo::font::FontCollection;
use shodo::geometry::WritingMode;
use shodo::limits::Limits;
use shodo::node::{InlineEdges, NodeId, OutOfFlowKind, TextSource};
use shodo::style::LineOptions;
use shodo::{LayoutContext, Paragraph, ParagraphBuilder};

/// Tags of replaced elements the inline engine sizes as atomic inlines.
pub(crate) const ATOMIC_TAGS: &[&str] = &["img", "svg"];

/// Replaced elements, form controls and MathML roots: in a paragraph they are
/// atomic inlines (CSS 2.1 10.3.2, 10.8.1) sized as nodes of their own (the
/// leaf measurement: their authored or intrinsic size, zero when neither is
/// known), whose content (fallback content, a control's label, MathML) is
/// not part of the paragraph's text.
pub(crate) const REPLACED_BOX_TAGS: &[&str] = &[
    "canvas", "video", "audio", "iframe", "object", "embed", "input", "button", "select",
    "textarea", "math",
];

/// A built paragraph together with the block's line options.
#[derive(Debug)]
pub(crate) struct ProjectedIfc {
    /// The shaped paragraph.
    pub(crate) paragraph: Paragraph,
    /// Writing mode used to shape the paragraph.
    pub(crate) writing_mode: WritingMode,
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
    /// `<br>` elements with a physical `clear`: the line after each starts
    /// below the floats it clears.
    pub(crate) cleared_breaks: Vec<(usize, taffy::Clear)>,
    /// The root is a fixed box: its containing block is the page area.
    pub(crate) fixed: bool,
    /// The root's lines end in an ellipsis where they overflow it.
    pub(crate) ellipsis: bool,
}

/// Whether `node` or one of its ancestors has an authored vertical writing
/// mode. Such text is laid out horizontally, without autospacing: real
/// vertical writing is not supported.
fn under_vertical_writing(doc: &Document, cascade: &CascadeResult, node: usize) -> bool {
    use raikiri_style::property::WritingMode;
    let mut current = Some(node);
    while let Some(id) = current {
        if matches!(
            cascade
                .authored_writing_modes
                .get(id)
                .and_then(|mode| *mode),
            Some(
                WritingMode::VerticalRl
                    | WritingMode::VerticalLr
                    | WritingMode::SidewaysRl
                    | WritingMode::SidewaysLr
            )
        ) {
            return true;
        }
        current = doc.parent_of(id);
    }
    false
}

/// The text style of `node` with its document context: its `lang`, and no
/// autospacing under a vertical writing mode.
fn styled(
    doc: &Document,
    cascade: &CascadeResult,
    cv: &ComputedValues,
    node: usize,
    fonts: &FontCollection,
) -> Result<shodo::style::InlineStyle, IfcError> {
    let mut inline = style::inline_style(cv, node, fonts)?;
    inline.lang = language_of(doc, node);
    style::resolve_auto_emphasis_position(cv, &mut inline);
    if under_vertical_writing(doc, cascade, node) {
        inline.text_autospace = shodo::style::TextAutospace::NoAutospace;
    }
    Ok(inline)
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

/// Whether the line height calculation quirk applies to the document: it is
/// in quirks or limited-quirks mode (Quirks Mode Standard, 3.3).
fn line_height_quirk(doc: &Document) -> bool {
    matches!(
        doc.quirks_mode(),
        raikiri_traits::QuirksMode::Quirks | raikiri_traits::QuirksMode::LimitedQuirks
    )
}

/// The counters of the document, computed once per layout pass and only when
/// some paragraph lays out generated text.
#[derive(Default)]
pub(crate) struct GeneratedCounters(
    std::cell::OnceCell<Result<Vec<CounterSnapshot>, CounterSnapshotLimitExceeded>>,
);

impl GeneratedCounters {
    fn get(
        &self,
        doc: &Document,
        cascade: &CascadeResult,
    ) -> Result<&[CounterSnapshot], CounterSnapshotLimitExceeded> {
        match self
            .0
            .get_or_init(|| crate::target::counter_snapshots(doc, cascade))
        {
            Ok(snapshots) => Ok(snapshots),
            Err(error) => Err(*error),
        }
    }
}

/// Whether `element` has a `::before` or `::after` that the paragraph lays
/// out as text.
pub(crate) fn has_in_flow_generated_text(cascade: &CascadeResult, element: usize) -> bool {
    [PseudoElem::Before, PseudoElem::After]
        .into_iter()
        .any(|pseudo| is_in_flow_generated_text(cascade, element, pseudo))
}

/// Push the text of the `pseudo` of `element` into the paragraph, as an
/// inline box of its own with the pseudo-element's style (CSS 2.1 12.1: the
/// generated box is a child of its element, before or after its content).
/// Only the text of `content` is laid out: images contribute nothing. A
/// block-level pseudo-element is approximated by a line of its own: a forced
/// break after `::before`, before `::after`. Floated and out-of-flow
/// pseudo-elements are not part of the paragraph; the painter draws them as
/// overlays.
#[allow(clippy::too_many_arguments)]
fn push_generated(
    builder: &mut ParagraphBuilder,
    doc: &Document,
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    fonts: &FontCollection,
    counters: &GeneratedCounters,
) -> Result<(), IfcError> {
    if !is_in_flow_generated_text(cascade, element, pseudo) {
        return Ok(());
    }
    let Some((cv, text)) =
        generated_text(doc, cascade, element, pseudo, counters.get(doc, cascade)?)
    else {
        return Ok(());
    };
    if text.is_empty() {
        return Ok(());
    }
    let id = NodeId(generated_node_id(element, pseudo) as u64);
    let inline_level = matches!(
        cv.display,
        DisplayValue::Inline
            | DisplayValue::Contents
            | DisplayValue::InlineBlock
            | DisplayValue::InlineFlex
            | DisplayValue::InlineGrid
            | DisplayValue::InlineTable
    );
    let inline_style = styled(doc, cascade, cv, element, fonts)?;
    let edges = if inline_level {
        style::inline_edges(cv, element, fonts)?
    } else {
        InlineEdges::default()
    };
    builder.open_inline(id, &inline_style, edges);
    if !inline_level && pseudo == PseudoElem::After {
        builder.push_forced_break(id);
    }
    builder.push_text(TextSource::Generated { node: id }, &text);
    if !inline_level && pseudo == PseudoElem::Before {
        builder.push_forced_break(id);
    }
    builder.close_inline();
    match builder.error() {
        Some(error) => Err(IfcError::Limit(error)),
        None => Ok(()),
    }
}

/// The box kind of a child of an ifc root, from its computed style; `None` for
/// content that belongs to the paragraph.
pub(crate) fn box_kind(cascade: &CascadeResult, doc: &Document, id: usize) -> Option<IfcBoxKind> {
    let cv = cascade.computed.get(id)?;
    let node = doc.get_node(id)?;
    if node.kind() != NodeKind::Element
        || matches!(cv.display, DisplayValue::None | DisplayValue::Contents)
    {
        return None;
    }
    // An absolutely positioned or fixed box is out of flow whatever its
    // display and float (CSS 2.1 9.7: its float computes to none).
    if matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed) {
        return Some(IfcBoxKind::OutOfFlow);
    }
    // Logical float sides are not mapped to physical ones: the box is laid
    // out as not floated, as the taffy bridge maps it.
    if matches!(cv.float, FloatValue::Left | FloatValue::Right) {
        return Some(IfcBoxKind::Float);
    }
    // An inline-level table is an atomic inline too (CSS 2.1 17.4).
    let inline_block = matches!(
        cv.display,
        DisplayValue::InlineBlock
            | DisplayValue::InlineFlex
            | DisplayValue::InlineGrid
            | DisplayValue::InlineTable
    );
    let tag = node.tag_name().unwrap_or("");
    let replaced = cv.display == DisplayValue::Inline
        && (ATOMIC_TAGS.contains(&tag) || REPLACED_BOX_TAGS.contains(&tag));
    if inline_block || replaced {
        return Some(IfcBoxKind::Atomic);
    }
    // Every block-level box is a block child: one in the paragraph's
    // formatting context, or one that establishes its own (flow roots, flex,
    // grid and table boxes, scroll containers), which the line loop places
    // as taffy's block algorithm does. A table-internal box directly in a
    // paragraph would be wrapped in an anonymous table (CSS 2.1 17.2.1),
    // which raikiri does not create; it is laid out as a block between the
    // lines, as taffy's block algorithm lays a block child out.
    if matches!(
        cv.display,
        DisplayValue::Block
            | DisplayValue::FlowRoot
            | DisplayValue::Flex
            | DisplayValue::Grid
            | DisplayValue::ListItem
            | DisplayValue::Table
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
            | DisplayValue::TableRow
            | DisplayValue::TableCell
            | DisplayValue::TableCaption
    ) {
        return Some(IfcBoxKind::Block);
    }
    None
}

/// A physical `clear` side; `None` for `none` and the logical sides, which
/// the taffy bridge maps to none.
fn physical_clear(clear: ClearValue) -> Option<taffy::Clear> {
    match clear {
        ClearValue::Left => Some(taffy::Clear::Left),
        ClearValue::Right => Some(taffy::Clear::Right),
        ClearValue::Both => Some(taffy::Clear::Both),
        _ => None,
    }
}

enum Step {
    /// A node to project.
    Enter(usize),
    /// The `::after` of an element, after its content.
    After(usize),
    Close,
}

/// Everything of a paragraph's projection except the shaping. The builder
/// holds the text and styles as owned data and is `Send`, so the shaping can
/// run on another thread while the document stays where it is.
pub(crate) struct ProjectedBuilder {
    /// The paragraph's content, not shaped yet.
    pub(crate) builder: ParagraphBuilder,
    /// Writing mode used to shape the paragraph.
    pub(crate) writing_mode: WritingMode,
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
    /// `<br>` elements with a physical `clear`: the line after each starts
    /// below the floats it clears.
    pub(crate) cleared_breaks: Vec<(usize, taffy::Clear)>,
    /// The root is a fixed box: its containing block is the page area.
    pub(crate) fixed: bool,
    /// The root's lines end in an ellipsis where they overflow it.
    pub(crate) ellipsis: bool,
}

impl ProjectedBuilder {
    /// Shape the paragraph.
    ///
    /// # Errors
    /// [`IfcError::Limit`] when a shodo resource limit is exceeded.
    pub(crate) fn build(
        self,
        cx: &mut LayoutContext,
        fonts: &FontCollection,
    ) -> Result<ProjectedIfc, IfcError> {
        let paragraph = self.builder.build(cx, fonts).map_err(IfcError::Limit)?;
        Ok(ProjectedIfc {
            paragraph,
            writing_mode: self.writing_mode,
            options: self.options,
            indent: self.indent,
            boxes: self.boxes,
            rtl: self.rtl,
            offsets: self.offsets,
            cleared_breaks: self.cleared_breaks,
            fixed: self.fixed,
            ellipsis: self.ellipsis,
        })
    }
}

/// Build the shodo paragraph for the in-flow block `root`: the walk and the
/// shaping in one call.
///
/// # Errors
/// The errors of [`project_ifc_builder`] and [`ProjectedBuilder::build`].
#[cfg(test)]
pub(crate) fn project_ifc(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    cx: &mut LayoutContext,
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedIfc, IfcError> {
    project_ifc_builder(doc, cascade, root, fonts, limits)?.build(cx, fonts)
}

/// Build the shodo paragraph for a text node that is a flex or grid item of
/// its own: the walk and the shaping in one call.
///
/// # Errors
/// The errors of [`project_ifc_text_builder`] and [`ProjectedBuilder::build`].
#[cfg(test)]
pub(crate) fn project_ifc_text(
    doc: &Document,
    cascade: &CascadeResult,
    text: usize,
    cx: &mut LayoutContext,
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedIfc, IfcError> {
    project_ifc_text_builder(doc, cascade, text, fonts, limits)?.build(cx, fonts)
}

/// Fill a paragraph builder with the text node `text` alone.
///
/// A flex or grid container wraps each run of its text in an anonymous item
/// (CSS Flexbox 1, 4; CSS Grid 1, 6), whose box has no edges of its own and
/// whose inherited properties come from the container. The text node's own
/// computed values are those inherited values, so they style the paragraph.
///
/// # Errors
/// [`IfcError::InvalidNode`] for an unknown, detached or non-text node,
/// [`IfcError::Unsupported`] for a style the inline engine does not map, and
/// [`IfcError::Limit`] when a shodo resource limit is exceeded.
pub(crate) fn project_ifc_text_builder(
    doc: &Document,
    cascade: &CascadeResult,
    text: usize,
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedBuilder, IfcError> {
    let node = doc.get_node(text).ok_or(IfcError::InvalidNode(text))?;
    let cv = cascade
        .computed
        .get(text)
        .ok_or(IfcError::InvalidNode(text))?;
    if !node.is_in_document() || node.kind() != NodeKind::Text {
        return Err(IfcError::InvalidNode(text));
    }
    let content = node.text_content().ok_or(IfcError::InvalidNode(text))?;
    // As for an element root: right-to-left content is ordered without
    // reading `unicode-bidi`.
    let (options, indent) = style::line_options(cv, text, fonts)?;
    let root_style = styled(doc, cascade, cv, text, fonts)?;
    let mut paragraph_style = style::paragraph_style(cv, text, root_style)?;
    paragraph_style.line_height_quirk = line_height_quirk(doc);
    let writing_mode = paragraph_style.writing_mode;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let cleared_breaks = Vec::new();
    builder.push_text(
        TextSource::Dom {
            node: NodeId(text as u64),
            offset: 0,
        },
        content,
    );
    if let Some(error) = builder.error() {
        return Err(IfcError::Limit(error));
    }
    Ok(ProjectedBuilder {
        builder,
        writing_mode,
        options,
        indent,
        boxes: Vec::new(),
        rtl: cv.direction == Direction::Rtl,
        offsets: Vec::new(),
        cleared_breaks,
        fixed: false,
        // An anonymous flex or grid item has no overflow of its own.
        ellipsis: false,
    })
}

/// [`project_ifc_builder_with`] with counters of its own.
///
/// # Errors
/// As [`project_ifc_builder_with`].
#[cfg(test)]
pub(crate) fn project_ifc_builder(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedBuilder, IfcError> {
    project_ifc_builder_with(
        doc,
        cascade,
        root,
        fonts,
        limits,
        &GeneratedCounters::default(),
    )
}

/// Walk the box `root` and fill a paragraph builder, without shaping. `fonts`
/// is read for font-relative lengths (`ch`); `counters` are the document's
/// counters, shared between the paragraphs of one layout pass.
///
/// # Errors
/// [`IfcError::InvalidNode`] for an unknown or detached node,
/// [`IfcError::Unsupported`] for a root or a node the inline path does not
/// project (an internal inconsistency: the assignment asks only for roots it
/// accepts), and [`IfcError::Limit`] when a shodo resource limit is exceeded
/// while the content is pushed.
pub(crate) fn project_ifc_builder_with(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    fonts: &FontCollection,
    limits: &Limits,
    counters: &GeneratedCounters,
) -> Result<ProjectedBuilder, IfcError> {
    let root_node = doc.get_node(root).ok_or(IfcError::InvalidNode(root))?;
    let root_cv = cascade
        .computed
        .get(root)
        .ok_or(IfcError::InvalidNode(root))?;
    if !root_node.is_in_document()
        || root_node.kind() != NodeKind::Element
        || !super::assign::can_be_ifc_root(doc, cascade, root)
    {
        return Err(IfcError::Unsupported {
            node: root,
            reason: "root must be a box that lays out its own inline content",
        });
    }
    // The cascade has already resolved `inherit`, `match-parent`, and
    // `-internal-center` in the computed `text-align` values.
    let (options, indent) = style::line_options(root_cv, root, fonts)?;
    let root_style = styled(doc, cascade, root_cv, root, fonts)?;
    let mut paragraph_style = style::paragraph_style(root_cv, root, root_style)?;
    // The engine applies the line height calculation quirk per line, to the
    // root and to every inline box (Quirks Mode Standard, 3.3-3.4).
    paragraph_style.line_height_quirk = line_height_quirk(doc);
    let writing_mode = paragraph_style.writing_mode;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let mut boxes = Vec::new();
    let mut offsets = Vec::new();
    let mut cleared_breaks = Vec::new();
    push_generated(
        &mut builder,
        doc,
        cascade,
        root,
        PseudoElem::Before,
        fonts,
        counters,
    )?;

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
            Step::After(id) => {
                push_generated(
                    &mut builder,
                    doc,
                    cascade,
                    id,
                    PseudoElem::After,
                    fonts,
                    counters,
                )?;
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
                // Columns and column groups render nothing of their own
                // outside a table either.
                if matches!(
                    cv.display,
                    DisplayValue::None | DisplayValue::TableColumn | DisplayValue::TableColumnGroup
                ) || node.is_non_rendered_html_element()
                {
                    continue;
                }
                let unsupported = |reason: &'static str| IfcError::Unsupported { node: id, reason };
                let tag = node.tag_name().unwrap_or_default();
                if cv.display == DisplayValue::Contents {
                    // A `display: contents` element generates no box: its
                    // children take part in the paragraph as if they were the
                    // element's siblings. `float` and `position` apply to a
                    // box, so they change nothing here (CSS Display 3, 2.5).
                    push_generated(
                        &mut builder,
                        doc,
                        cascade,
                        id,
                        PseudoElem::Before,
                        fonts,
                        counters,
                    )?;
                    stack.push(Step::After(id));
                    stack.extend(node.children.iter().rev().map(|&child| Step::Enter(child)));
                    continue;
                }
                // A box inside an inline element is laid out relative to the
                // root like any other box of the paragraph; readers find its
                // layout parent with `Document::layout_parent_of`.
                if box_kind(cascade, doc, id) == Some(IfcBoxKind::OutOfFlow) {
                    builder.push_out_of_flow(NodeId(id as u64), OutOfFlowKind::Absolute);
                    boxes.push(IfcBox {
                        node: id,
                        kind: IfcBoxKind::OutOfFlow,
                    });
                    if let Some(error) = builder.error() {
                        return Err(IfcError::Limit(error));
                    }
                    continue;
                }
                if box_kind(cascade, doc, id) == Some(IfcBoxKind::Float) {
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
                    let atomic_style = styled(doc, cascade, cv, id, fonts)?;
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
                // A block inside an inline element splits the element around
                // it (CSS 2.1 9.2.1.1): the lines before and after it are the
                // element's, the block sits between them.
                if box_kind(cascade, doc, id) == Some(IfcBoxKind::Block) {
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
                if cv.display != DisplayValue::Inline {
                    return Err(unsupported("only inline-level boxes are projected"));
                }
                let edges = style::inline_edges(cv, id, fonts)?;
                let inline_style = styled(doc, cascade, cv, id, fonts)?;
                // The eligibility check keeps every offset that is not a plain
                // length out of the paragraph.
                if cv.position == PositionValue::Relative {
                    offsets.push((id, style::relative_offset_in_lines(cv)));
                }
                // `clear` on a line break moves the next line below the
                // floats (CSS 2.1, 9.5.2): the line loop looks for the break.
                if tag == "br"
                    && let Some(clear) = physical_clear(cv.clear)
                {
                    cleared_breaks.push((id, clear));
                }
                builder.open_inline(NodeId(id as u64), &inline_style, edges);
                if tag == "br" {
                    builder.push_forced_break(NodeId(id as u64));
                    builder.close_inline();
                } else {
                    push_generated(
                        &mut builder,
                        doc,
                        cascade,
                        id,
                        PseudoElem::Before,
                        fonts,
                        counters,
                    )?;
                    if tag == "wbr" {
                        builder.push_text(
                            TextSource::Generated {
                                node: NodeId(id as u64),
                            },
                            "\u{200B}",
                        );
                    }
                    stack.push(Step::Close);
                    stack.push(Step::After(id));
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
    push_generated(
        &mut builder,
        doc,
        cascade,
        root,
        PseudoElem::After,
        fonts,
        counters,
    )?;
    Ok(ProjectedBuilder {
        builder,
        writing_mode,
        options,
        indent,
        boxes,
        rtl: root_cv.direction == Direction::Rtl,
        offsets,
        cleared_breaks,
        fixed: root_cv.position == PositionValue::Fixed,
        ellipsis: style::ends_in_ellipsis(root_cv),
    })
}

#[cfg(test)]
mod tests;
