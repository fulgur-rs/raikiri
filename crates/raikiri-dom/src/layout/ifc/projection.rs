//! Project one in-flow block's inline content into a shodo paragraph.
//!
//! Text, the text of in-flow `::before` and `::after`, ordinary inline
//! elements, `<br>`, left or right floats, atomic inlines (inline-blocks,
//! replaced elements, form controls) and block children of the root are
//! projected; a float becomes an anchor in the text, an atomic a placeholder
//! and a block a break between lines, and all three are laid out as boxes of
//! their own. Anything the inline path cannot place yet (positioned boxes,
//! blocks inside inline elements) is rejected with [`IfcError::Unsupported`]
//! rather than approximated.

use super::boxes::{IfcBox, IfcBoxKind};
use super::error::IfcError;
use super::style;
use crate::Document;
use crate::generated_content::{generated_node_id, generated_text, is_in_flow_generated_text};
use crate::target::CounterSnapshot;
use raikiri_style::property::{
    ClearValue, Direction, DisplayValue, FloatValue, OverflowValue, PositionValue,
    WhiteSpaceCollapse,
};
use raikiri_style::{CascadeResult, ComputedTextIndent, ComputedValues, PseudoElem};
use raikiri_traits::NodeKind;
use shodo::font::FontCollection;
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
const REPLACED_BOX_TAGS: &[&str] = &[
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

/// Whether `node` or one of its ancestors has an authored vertical writing
/// mode. Such text is laid out horizontally, without autospacing, as on the
/// parley path.
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

/// The counters of the document, computed once per layout pass and only when
/// some paragraph lays out generated text.
#[derive(Default)]
pub(crate) struct GeneratedCounters(std::cell::OnceCell<Vec<CounterSnapshot>>);

impl GeneratedCounters {
    fn get(&self, doc: &Document, cascade: &CascadeResult) -> &[CounterSnapshot] {
        self.0
            .get_or_init(|| crate::target::counter_snapshots(doc, cascade))
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
        generated_text(doc, cascade, element, pseudo, counters.get(doc, cascade))
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
    if node.kind() != NodeKind::Element {
        return None;
    }
    if cv.float != FloatValue::None {
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
    // `flow-root`, flex and grid boxes avoid floats as formatting contexts of
    // their own; taffy places those in the parent's item loop, which this
    // path does not run, so they stay unsupported.
    // A block-level image or SVG is sized by its replaced-element path, which
    // this path does not reproduce; it stays unsupported.
    if cv.display == DisplayValue::Block && !ATOMIC_TAGS.contains(&tag) {
        return Some(IfcBoxKind::Block);
    }
    // A table-internal box directly in a paragraph would be wrapped in an
    // anonymous table (CSS 2.1 17.2.1), which raikiri does not create; it is
    // laid out as a block between the lines, as the block algorithm lays it
    // out on the parley path.
    if matches!(
        cv.display,
        DisplayValue::TableRowGroup
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

/// A float the inline engine can place: left or right, cleared by physical
/// sides only, in normal position or relatively positioned.
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
    // A relatively positioned float is placed as a float and drawn offset
    // from that place by the painter, as a block box is.
    if !matches!(cv.position, PositionValue::Static | PositionValue::Relative) {
        return Err(unsupported("positioned floats are not placed yet"));
    }
    Ok(())
}

/// An atomic inline the inline engine can place: in normal position, or
/// relatively positioned (it keeps its place on the line and is drawn offset
/// from it).
fn supported_atomic(cv: &ComputedValues, node: usize) -> Result<(), IfcError> {
    if !matches!(cv.position, PositionValue::Static | PositionValue::Relative) {
        return Err(IfcError::Unsupported {
            node,
            reason: "positioned atomic inlines are not placed yet",
        });
    }
    Ok(())
}

/// A block child the inline engine can place between lines: in normal
/// position, with no `auto` side margin, not a scroll container and not
/// cleared. Its vertical margins collapse with those of the block children
/// next to it and are added to the lines around it.
fn supported_block(cv: &ComputedValues, node: usize) -> Result<(), IfcError> {
    let unsupported = |reason: &'static str| IfcError::Unsupported { node, reason };
    if cv.position != PositionValue::Static && !style::is_inert_relative(cv) {
        return Err(unsupported("positioned blocks are not placed yet"));
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
pub(crate) fn has_rtl_char(text: &str) -> bool {
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

enum Step {
    /// A node to project, and whether it sits inside an inline element.
    Enter(usize, bool),
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
            options: self.options,
            indent: self.indent,
            boxes: self.boxes,
            rtl: self.rtl,
            offsets: self.offsets,
            preserved_spaces: self.preserved_spaces,
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
    // As for an element root: with right-to-left content, the parley path
    // orders the text without reading `unicode-bidi`.
    let (options, indent) = style::line_options(cv, text, fonts)?;
    let root_style = styled(doc, cascade, cv, text, fonts)?;
    let paragraph_style = style::paragraph_style(cv, text, root_style)?;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let preserved_spaces = if matches!(
        cv.effective_white_space_collapse,
        WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::PreserveSpaces
    ) {
        vec![text]
    } else {
        Vec::new()
    };
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
        options,
        indent,
        boxes: Vec::new(),
        rtl: cv.direction == Direction::Rtl,
        offsets: Vec::new(),
        preserved_spaces,
    })
}

/// Walk the box `root` and fill a paragraph builder, without
/// shaping. `fonts` is read for font-relative lengths (`ch`).
///
/// # Errors
/// [`IfcError::InvalidNode`] for an unknown or detached node,
/// [`IfcError::Unsupported`] for anything the first slice does not place, and
/// [`IfcError::Limit`] when a shodo resource limit is exceeded while the
/// content is pushed.
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

/// [`project_ifc_builder`] with the document's counters shared between the
/// paragraphs of one layout pass.
///
/// # Errors
/// As [`project_ifc_builder`].
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
    let paragraph_style = style::paragraph_style(root_cv, root, root_style)?;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let mut boxes = Vec::new();
    let mut offsets = Vec::new();
    let mut preserved_spaces = Vec::new();
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
        .map(|&child| Step::Enter(child, false))
        .collect();
    while let Some(step) = stack.pop() {
        let (id, nested) = match step {
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
                let inline_style = styled(doc, cascade, cv, id, fonts)?;
                let edges = style::inline_edges(cv, id, fonts)?;
                // The eligibility check keeps every offset that is not a plain
                // length out of the paragraph.
                if cv.position == PositionValue::Relative {
                    offsets.push((id, style::relative_offset_in_lines(cv)));
                }
                // `clear` on a line break moves the next line below the
                // floats (CSS 2.1, 9.5.2); the engine's forced break carries
                // no clearance.
                if tag == "br" && cv.clear != ClearValue::None {
                    return Err(unsupported("a cleared line break is not placed yet"));
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
                    stack.push(Step::Close);
                    stack.push(Step::After(id));
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
