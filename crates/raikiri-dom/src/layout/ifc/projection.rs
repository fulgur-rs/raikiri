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
use super::first_letter::{FirstLetter, LetterStyle};
use super::first_line::FirstLineStyles;
use super::generated_atomics::GeneratedAtomic;
use super::generated_blocks::GeneratedBlock;
use super::style;
use crate::Document;
use crate::generated_content::{
    GeneratedPiece, generated_image_node_id, generated_node_id,
    generated_pieces_compact as generated_pieces, generated_text_compact as generated_text,
    is_in_flow_generated_text,
};
use crate::node::BoxPropertiesRef;
use crate::target::{CompactCounterSnapshots, CounterSnapshotLimitExceeded};
use raikiri_layout_style::values::Clear;
use raikiri_style::property::{
    ClearValue, ColumnCountValue, Direction, DisplayValue, FloatValue, OverflowValue,
    PositionValue, TextWrapMode,
};
use raikiri_style::{
    CascadeResult, ComputedColumnWidth, ComputedTextIndent, ComputedValues, PseudoElem,
};
use raikiri_traits::NodeKind;
use shodo::font::FontCollection;
use shodo::geometry::WritingMode;
use shodo::limits::Limits;
use shodo::node::{InlineEdges, NodeId, OutOfFlowKind, TextSource};
use shodo::style::LineOptions;
use shodo::{LayoutContext, Paragraph, ParagraphBuilder};
use std::collections::HashMap;
use std::sync::Arc;

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

/// A paragraph's projection: the paragraph `P` together with the block's
/// line options and the boxes around it.
#[derive(Debug)]
pub(crate) struct Projected<P> {
    /// The paragraph: shaped in a [`ProjectedIfc`], not yet in a
    /// [`ProjectedBuilder`].
    pub(crate) paragraph: P,
    /// Everything of the projection besides the paragraph.
    pub(crate) meta: ProjectedMeta,
    /// The paragraphs its intrinsic widths are measured from when block-level
    /// `::before` or `::after` boxes break its lines: see
    /// [`BlockIntrinsics`].
    pub(crate) block_intrinsics: Option<Box<BlockIntrinsics<P>>>,
}

/// The paragraphs a paragraph with block-level `::before` and `::after`
/// boxes is measured from. Such a box is a block between the lines, whose
/// content is laid out in the paragraph's lines between its tokens but sized
/// by the box itself: its width, its minimum and maximum widths, and its
/// edges decide what the box adds to the intrinsic widths of the paragraph
/// (CSS Sizing 3 §5.1), as for any block child. So the box's content is
/// measured as a paragraph of its own, and the rest of the paragraph without
/// it.
#[derive(Debug)]
pub(crate) struct BlockIntrinsics<P> {
    /// The paragraph with only the tokens of the boxes in `contents`.
    pub(crate) paragraph: P,
    /// The content of each box.
    pub(crate) contents: Vec<BlockContent<P>>,
}

/// A block-level pseudo-element's content measured apart from the paragraph
/// (see [`BlockIntrinsics`]), broken as the paragraph breaks it.
#[derive(Debug)]
pub(crate) struct BlockContent<P> {
    /// The id of the box's start token.
    pub(crate) start: u64,
    /// The content as a paragraph of its own.
    pub(crate) paragraph: P,
    /// The content holds an image, which breaks are found around unless the
    /// paragraph has an out-of-flow box.
    has_images: bool,
}

/// A built paragraph together with the block's line options.
pub(crate) type ProjectedIfc = Projected<Paragraph>;

/// Everything of a paragraph's projection except the shaping. The builder
/// holds the text and styles as owned data and is `Send`, so the shaping can
/// run on another thread while the document stays where it is.
pub(crate) type ProjectedBuilder = Projected<ParagraphBuilder>;

/// The part of a [`Projected`] paragraph that the shaping does not change.
#[derive(Debug)]
pub(crate) struct ProjectedMeta {
    pub(crate) ruby_containers: Arc<super::ruby::RubyContainers>,
    /// Writing mode used to shape the paragraph.
    pub(crate) writing_mode: WritingMode,
    /// Line options of the block root.
    pub(crate) options: LineOptions,
    /// The block's raw `text-indent`, to be resolved against its width.
    pub(crate) indent: ComputedTextIndent,
    /// Children laid out as boxes of their own, in document order.
    pub(crate) boxes: Vec<IfcBox>,
    /// Fixed-size image marker inserted ahead of the root's ordinary content.
    pub(crate) marker_atomic: Option<(NodeId, shodo::AtomicSize)>,
    /// Minimum baseline supplied by a prepared outside image marker.
    pub(crate) outside_marker_height: Option<f32>,
    /// Fixed-size images of `::before` and `::after` content.
    pub(crate) generated_atomics: Vec<(NodeId, shodo::AtomicSize)>,
    /// Block-level `::before` and `::after` boxes, in document order.
    pub(crate) generated_blocks: Vec<GeneratedBlock>,
    /// Atomic `::before` and `::after` boxes without content.
    pub(crate) generated_boxes: Vec<GeneratedAtomic>,
    /// The root's `direction` is `rtl`: its lines start at the right edge.
    pub(crate) rtl: bool,
    /// Paint offsets of the relatively positioned inline elements, by DOM
    /// node id.
    pub(crate) offsets: Vec<(usize, (f32, f32))>,
    /// The insets of the elements in `offsets` whose offset depends on the
    /// root's size, by index in `offsets`; their offset there is that of a
    /// zero width and a height that is not definite.
    pub(crate) percent_insets: Vec<(usize, style::InlineRelativeInsets)>,
    /// `<br>` elements with a physical `clear`: the line after each starts
    /// below the floats it clears.
    pub(crate) cleared_breaks: Vec<(usize, Clear)>,
    /// The text nodes after the root in the anonymous flex or grid item the
    /// root starts: one item holds a run of text nodes, which `display:
    /// contents` elements between them can keep from being siblings. Empty
    /// for any other paragraph.
    pub(crate) run_tail: Vec<usize>,
    /// The root is a fixed box: its containing block is the page area.
    pub(crate) fixed: bool,
    /// The root's lines end in an ellipsis where they overflow it.
    pub(crate) ellipsis: bool,
    /// The root's text does not wrap (`text-wrap-mode: nowrap`). It decides
    /// for lines without recorded content.
    pub(crate) nowrap: bool,
    /// Whether each text source (a DOM text node or a generated
    /// pseudo-element), atomic inline or inline box of the paragraph, by node
    /// id, sits in content that does not wrap (`text-wrap-mode: nowrap`). A
    /// line made only of such content stays beside floats even when it
    /// overflows the space there.
    pub(crate) text_nowrap: Arc<HashMap<u64, bool>>,
    pub(crate) letter_styles: Vec<LetterStyle>,
    /// The opacity of the translucent `::first-line` boxes around the
    /// paragraph's first line, multiplied (see
    /// [`crate::paint_rules::first_line_opacity_layer`]).
    pub(crate) first_line_opacity: Option<f32>,
    /// The values the text and inline elements of the paragraph have on its
    /// first line. Empty when no `::first-line` style reaches the paragraph.
    pub(crate) first_line_styles: Arc<FirstLineStyles>,
}

/// The text style of `node` with its document context: its `lang`, and no
/// autospacing under a vertical writing mode.
pub(super) fn styled(
    doc: &Document,
    _cascade: &CascadeResult,
    cv: &ComputedValues,
    node: usize,
    fonts: &FontCollection,
) -> Result<shodo::style::InlineStyle, IfcError> {
    let mut inline = style::inline_style(cv, node, fonts)?;
    inline.lang = language_of(doc, node);
    style::resolve_auto_emphasis_position(cv, &mut inline);
    if doc.nodes[node].under_vertical_writing {
        inline.text_autospace = shodo::style::TextAutospace::NoAutospace;
    }
    Ok(inline)
}

/// The effective `lang` of `node`: the nearest ancestor-or-self `lang`
/// attribute, trimmed and lowercased. An empty value means "unknown" and stops
/// inheritance.
pub(crate) fn language_of(doc: &Document, node: usize) -> Option<String> {
    let value = doc
        .inclusive_ancestors(node)
        .find_map(|id| doc.get_node(id).and_then(|n| n.attribute("lang")))?
        .trim();
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}

/// The style a `<br>` gives its line strut in quirks mode: the style of the
/// box it sits in, with the `<br>`'s own `line-height` (resolved, like
/// `normal` or a number, against that box's font) and baseline alignment.
/// The box is the nearest ancestor that generates one: `display: contents`
/// generates none, while the document element is blockified. Its
/// `vertical-align` places that box, not the break.
/// `None` outside quirks mode, where the break takes the enclosing box's
/// strut unchanged.
fn break_strut_style(
    doc: &Document,
    cascade: &CascadeResult,
    br: usize,
    br_style: &shodo::style::InlineStyle,
    fonts: &FontCollection,
) -> Result<Option<shodo::style::InlineStyle>, IfcError> {
    if !line_height_quirk(doc) {
        return Ok(None);
    }
    let owner = doc
        .ancestors(br)
        .find(|&id| {
            super::assign::is_layout_root(doc, id)
                || cascade
                    .computed
                    .get(id)
                    .is_some_and(|_| doc.nodes[id].display() != DisplayValue::Contents)
        })
        .unwrap_or(br);
    let owner_cv = cascade
        .computed
        .get(owner)
        .ok_or(IfcError::InvalidNode(owner))?;
    let mut style = styled(doc, cascade, owner_cv, owner, fonts)?;
    style.line_height = br_style.line_height;
    style.vertical_align = shodo::style::VerticalAlign::default();
    Ok(Some(style))
}

/// Whether the line height calculation quirk applies to the document: it is
/// in quirks or limited-quirks mode (Quirks Mode Standard, 3.3).
fn line_height_quirk(doc: &Document) -> bool {
    matches!(
        doc.quirks_mode(),
        raikiri_traits::QuirksMode::Quirks | raikiri_traits::QuirksMode::LimitedQuirks
    )
}

/// The counters of the document, read from
/// [`Document::shared_compact_counter_snapshots`] only when some paragraph lays out
/// generated text.
#[derive(Default)]
pub(crate) struct GeneratedCounters {
    counters: std::cell::OnceCell<crate::target::SharedCompactCounterSnapshots>,
    pub(super) predecessors: super::first_letter::PredecessorCache,
}

impl GeneratedCounters {
    pub(super) fn get(
        &self,
        doc: &Document,
        cascade: &CascadeResult,
    ) -> Result<&CompactCounterSnapshots, CounterSnapshotLimitExceeded> {
        match self
            .counters
            .get_or_init(|| doc.shared_compact_counter_snapshots(cascade))
        {
            Ok(snapshots) => Ok(snapshots),
            Err(error) => Err(*error),
        }
    }
}

/// Whether `element` has generated text that can make a line, or a prepared
/// inside marker image. A blank inline `::before` or `::after` (only empty
/// strings, no margin, border or padding; see
/// [`crate::generated_content::is_line_making_generated_text`]) lays out a
/// phantom line (CSS 2.1 §9.4.2), so it does not count.
pub(crate) fn has_line_making_generated_text(
    doc: &Document,
    cascade: &CascadeResult,
    element: usize,
    counters: &GeneratedCounters,
) -> Result<bool, IfcError> {
    for pseudo in [PseudoElem::Marker, PseudoElem::Before, PseudoElem::After] {
        if !is_in_flow_generated_text(cascade, element, pseudo)
            || (pseudo != PseudoElem::Marker
                && crate::generated_content::is_blank_inline_generated_text(
                    cascade, element, pseudo,
                ))
        {
            continue;
        }
        if pseudo != PseudoElem::Marker
            || doc.list_marker_image_size(element).is_some()
            || crate::generated_content::markers::marker_render_info_with_compact_snapshots(
                doc,
                cascade,
                element,
                counters.get(doc, cascade)?,
            )
            .is_some_and(|(_, text)| !text.is_empty())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Record the paint offset of `id` when it is relatively positioned, with
/// its insets, by index in `offsets`, when the offset depends on the root's
/// size or writing mode. `rtl` is the root's direction.
pub(super) fn push_relative_offset(
    offsets: &mut Vec<(usize, (f32, f32))>,
    percent_insets: &mut Vec<(usize, style::InlineRelativeInsets)>,
    id: usize,
    cv: &ComputedValues,
    properties: BoxPropertiesRef<'_>,
    rtl: bool,
) {
    let Some(insets) = style::InlineRelativeInsets::of_box(cv, properties) else {
        return;
    };
    if insets.depends_on_containing_block() {
        percent_insets.push((offsets.len(), insets));
    }
    offsets.push((id, insets.offset(0.0, None, rtl)));
}

/// The images of `::before` and `::after` content of a paragraph: atomic
/// inlines of a prepared size (CSS Content 3 §2.2).
struct GeneratedImages {
    atomics: Vec<(NodeId, shodo::AtomicSize)>,
    writing_mode: WritingMode,
    /// Paint offsets of the relatively positioned `::before` and `::after`
    /// boxes, by the inline engine's node id of the pseudo-element.
    offsets: Vec<(usize, (f32, f32))>,
    /// The block-level `::before` and `::after` boxes, in document order.
    blocks: Vec<GeneratedBlock>,
    /// The atomic `::before` and `::after` boxes without content.
    boxes: Vec<GeneratedAtomic>,
    /// The insets of the boxes in `offsets` whose offset depends on the
    /// root's size, by index in `offsets`.
    percent_insets: Vec<(usize, style::InlineRelativeInsets)>,
    /// The root's `direction` is `rtl`.
    rtl: bool,
    /// With the limits of the root's paragraph, the contents of its
    /// block-level pseudo-elements, which are left out of the paragraph and
    /// measured apart; `None` when the content stays in the paragraph.
    block_contents: Option<(Limits, Vec<BlockContent<ParagraphBuilder>>)>,
}

/// The atomic size of an image of `width` by `height` CSS pixels, its
/// baseline at its bottom margin edge (CSS 2.1 10.8.1).
fn image_atomic_size(
    size: raikiri_traits::ImageRasterSize,
    writing_mode: WritingMode,
) -> shodo::AtomicSize {
    let horizontal = writing_mode == WritingMode::HorizontalTb;
    shodo::AtomicSize {
        inline_size: if horizontal { size.width } else { size.height },
        block_size: if horizontal { size.height } else { size.width },
        ..shodo::AtomicSize::default()
    }
}

/// Push the content of the `pseudo` of `element` into the paragraph, as an
/// inline box of its own with the pseudo-element's style (CSS 2.1 12.1: the
/// generated box is a child of its element, before or after its content).
/// Text is laid out as text; each prepared image of `::before` and `::after`
/// is an atomic inline in its place (CSS Content 3 §2.2), and an image that
/// was not prepared represents nothing. A block-level pseudo-element is a
/// block box between the lines, marked by a start and an end token around its
/// content (see [`super::generated_blocks`]); even an empty one keeps its
/// height and edges. Floated and out-of-flow pseudo-elements are not part of
/// the paragraph; the painter draws them as overlays.
#[allow(clippy::too_many_arguments)]
fn push_generated(
    builder: &mut ParagraphBuilder,
    doc: &Document,
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    fonts: &FontCollection,
    counters: &GeneratedCounters,
    first_letter: &mut FirstLetter,
    text_nowrap: &mut HashMap<u64, bool>,
    images: &mut GeneratedImages,
) -> Result<(), IfcError> {
    if !is_in_flow_generated_text(cascade, element, pseudo) {
        return Ok(());
    }
    let (cv, text, pieces) = if pseudo == PseudoElem::Marker {
        let Some((cv, text)) =
            generated_text(doc, cascade, element, pseudo, counters.get(doc, cascade)?)
        else {
            return Ok(());
        };
        (cv, text, Vec::new())
    } else {
        let Some((cv, pieces)) =
            generated_pieces(doc, cascade, element, pseudo, counters.get(doc, cascade)?)
        else {
            return Ok(()); // cov:ignore: an in-flow `::before` or `::after` has a computed style with non-empty content, so its pieces always exist
        };
        // Only the images that were prepared take part; the others
        // represent nothing. A kept image carries its node id from here on.
        let mut kept = Vec::with_capacity(pieces.len());
        for piece in pieces {
            match piece {
                GeneratedPiece::Text(text) => kept.push(GeneratedPiece::Text(text)),
                GeneratedPiece::Image(index) => {
                    let (Some(id), Some(size)) = (
                        generated_image_node_id(element, pseudo, index),
                        doc.content_image_size(element, pseudo, index),
                    ) else {
                        continue;
                    };
                    images.atomics.push((
                        NodeId(id as u64),
                        image_atomic_size(size, images.writing_mode),
                    ));
                    kept.push(GeneratedPiece::Image(id));
                }
            }
        }
        let pieces = kept;
        let text: String = pieces
            .iter()
            .filter_map(|piece| match piece {
                GeneratedPiece::Text(text) => Some(text.as_str()),
                GeneratedPiece::Image(_) => None,
            })
            .collect();
        (cv, text, pieces)
    };
    let has_images = pieces
        .iter()
        .any(|piece| matches!(piece, GeneratedPiece::Image(_)));
    let atomic = pseudo != PseudoElem::Marker && cv.display.is_atomic_inline_level();
    let id = NodeId(generated_node_id(element, pseudo) as u64);
    let inline_level = pseudo == PseudoElem::Marker
        || (cv.display.is_inline_level() || matches!(cv.display, DisplayValue::Contents));
    // A writing mode other than its box parent's makes the box an orthogonal
    // flow (CSS Writing Modes 4 §7.3). A `display: contents` element has no
    // box, so the box parent is its own.
    let box_owner = if doc.nodes[element].display() == DisplayValue::Contents {
        crate::layout::box_parent(
            doc,
            cascade,
            element,
            crate::layout::BoxParentScope::Document,
        )
        .unwrap_or(element)
    } else {
        element
    };
    let orthogonal = cv.cssom_writing_mode.is_vertical()
        != cascade.computed[box_owner].cssom_writing_mode.is_vertical();
    let block = match (!inline_level)
        .then(|| super::generated_blocks::end_token_id(id.0))
        .flatten()
    {
        Some(end) => {
            let (mut options, indent) = style::line_options(cv, element, fonts)?;
            let rtl = cv.direction == Direction::Rtl;
            if rtl != images.rtl {
                super::generated_blocks::align_physically(&mut options, rtl);
            }
            Some(
                GeneratedBlock::new(id.0, end, cv, (options, indent), orthogonal)
                    .on_root_axes(images.writing_mode, images.rtl),
            )
        }
        None => None,
    };
    let empty = text.is_empty() && !has_images;
    // The content of a box set in the paragraph's lines is measured apart
    // for the paragraph's intrinsic widths; an empty box has none.
    let block = block.map(|mut block| {
        block.content_apart = !empty && !orthogonal;
        block
    });
    let (edges, edge_percents) = if inline_level
        && pseudo != PseudoElem::Marker
        && (!empty || cv.display != DisplayValue::Contents)
    {
        style::inline_edges(cv, element, fonts)?
    } else {
        Default::default()
    };
    // Block-axis decoration alone does not make an empty inline create a line.
    let has_edges = [
        edges.margin,
        edges.border,
        edges.padding,
        edge_percents.margin,
        edge_percents.padding,
    ]
    .iter()
    .any(|side| side.inline_start != 0.0 || side.inline_end != 0.0);
    if empty && block.is_none() && !atomic && !has_edges {
        return Ok(());
    }
    // The pseudo-element's own relative offset moves its text and images
    // like those of a relatively positioned inline element (CSS 2.1 §9.4.3).
    // With `display: contents` it generates no box for the offset to move.
    if pseudo != PseudoElem::Marker
        && cv.display != DisplayValue::Contents
        && cv.position == PositionValue::Relative
    {
        push_relative_offset(
            &mut images.offsets,
            &mut images.percent_insets,
            id.0 as usize,
            cv,
            BoxPropertiesRef::from_computed(cv),
            images.rtl,
        );
    }
    // Atomic text is outside this IFC. A block after-pseudo starts a later
    // line, unless, as in Chromium, it is the first thing in its paragraph.
    if atomic
        || (!inline_level
            && pseudo == PseudoElem::After
            && (block.is_none() || !first_letter.opens_with(doc, cascade, element)))
    {
        first_letter.stop();
    }
    let marker_fallback;
    let cv = if pseudo == PseudoElem::Marker
        && !cascade
            .pseudo
            .contains_key(&(raikiri_style::StyleNodeId::new(element as u64), pseudo))
    {
        marker_fallback = raikiri_style::ComputedValues::inherit_marker_from(cv);
        &marker_fallback
    } else {
        cv
    };
    text_nowrap.insert(id.0, cv.effective_text_wrap_mode == TextWrapMode::Nowrap);
    let mut inline_style = styled(doc, cascade, cv, element, fonts)?;
    // An inside bullet is as wide as Chromium's, whatever its glyph's advance.
    let mut edges = edges;
    if pseudo == PseudoElem::Marker
        && crate::generated_content::markers::is_symbol_marker(cv)
        && let Some((advance, ascent)) =
            super::standalone::measure_run(fonts, &inline_style, images.writing_mode, &text)
    {
        (edges.margin.inline_start, edges.margin.inline_end) =
            crate::generated_content::markers::inside_symbol_marker_margins(
                cv.font_size.px(),
                advance,
                ascent,
            );
    }
    let marker_body_style = if pseudo == PseudoElem::Marker
        && inline_style.text_wrap_mode == shodo::style::TextWrapMode::NoWrap
        && text.ends_with([' ', '\t'])
    {
        let mut body = inline_style.clone();
        body.unicode_bidi = shodo::style::UnicodeBidi::Normal;
        // Keep the marker's text indivisible while exposing the whitespace
        // boundary to the enclosing paragraph's wrapping rules.
        inline_style.text_wrap_mode = shodo::style::TextWrapMode::Wrap;
        Some(body)
    } else {
        None
    };
    if let Some(block) = &block {
        // As in Chromium, a block-level box with no letter in it, empty or
        // only white space, takes the first formatted line, which then holds
        // no letter.
        if text
            .trim_matches([' ', '\t', '\n', '\r', '\u{000c}'])
            .is_empty()
            && !has_images
        {
            first_letter.stop();
        } else {
            first_letter.start_block();
        }
        builder.push_block_in_inline(NodeId(block.start));
        images.blocks.push(block.clone());
    }
    // A box whose content is measured apart keeps only its tokens here. Its
    // content is pushed as the paragraph pushes it, so that the first letter
    // is found the same way.
    if let Some(block) = &block
        && block.content_apart
        && let Some((limits, contents)) = images.block_contents.as_mut()
    {
        let paragraph_style = style::paragraph_style(cv, element, inline_style.clone())?;
        let mut paragraph = ParagraphBuilder::new(&paragraph_style, limits);
        let mut has_images = false;
        // The paragraph has already taken the same content, so it fails here
        // only as it would have failed there.
        let pushed = pieces.iter().try_for_each(|piece| match piece {
            GeneratedPiece::Text(text) => first_letter.push_with_counters(
                &mut paragraph,
                doc,
                cascade,
                TextSource::Generated { node: id },
                cv,
                text,
                fonts,
                counters,
            ),
            GeneratedPiece::Image(image) => {
                first_letter.stop();
                has_images = true;
                paragraph.push_atomic(NodeId(*image as u64), &inline_style, InlineEdges::default());
                Ok(())
            }
        });
        pushed?;
        // A limit the content exceeds is reported when it is built.
        contents.push(BlockContent {
            start: block.start,
            paragraph,
            has_images,
        });
        // The box held the first letter, or took the first formatted line.
        first_letter.stop();
        builder.push_block_in_inline(NodeId(block.end));
        return builder.error().map(IfcError::Limit).map_or(Ok(()), Err);
    }
    if empty && atomic {
        // An atomic box without content is still a box: an empty atomic
        // inline of its own size in place of the pseudo-element.
        images.boxes.push(GeneratedAtomic::new(id.0, cv));
        builder.push_atomic(id, &inline_style, InlineEdges::default());
    }
    // Empty non-atomic inlines still need their edges projected. The line
    // engine decides whether those edges make a line in the document's mode.
    let open_inline = !empty || (!atomic && has_edges);
    if open_inline {
        if let Some(first_line) = first_letter.first_line.as_mut() {
            first_line.note_margins(id.0 as usize, &edges, &edge_percents);
        }
        let first_line = if pseudo == PseudoElem::Marker {
            None
        } else if inline_level {
            first_letter
                .first_line
                .as_ref()
                .and_then(|first_line| first_line.generated(doc, cascade, element, pseudo))
        } else {
            // A block-level box of the paragraph itself has the first line
            // when it comes first and its content makes a line.
            first_letter.first_line.as_ref().and_then(|first_line| {
                first_line.generated_block(cascade, element, pseudo, &text, has_images)
            })
        };
        match first_line {
            Some(values) if inline_level => {
                let first_style = styled(doc, cascade, &values, element, fonts)?;
                if let Some(first_line) = first_letter.first_line.as_mut() {
                    first_line.note_generated(id.0 as usize, values);
                }
                builder.open_inline_with_first_line(id, &inline_style, &first_style, edges)
            }
            Some(values) => {
                // The line of a block box starts after its start token, so
                // shodo has no first line to style: only its paint takes the
                // values, and only for the text, not the box around it.
                if let Some(first_line) = first_letter.first_line.as_mut() {
                    first_line.note_generated_block_text(cascade, id.0 as usize, values);
                }
                builder.open_inline(id, &inline_style, edges)
            }
            None => builder.open_inline(id, &inline_style, edges),
        }
        .set_inline_edge_percents(edge_percents);
    }
    if !inline_level && block.is_none() && pseudo == PseudoElem::After {
        builder.push_forced_break(id);
    }
    if pseudo == PseudoElem::Marker {
        if let Some(body_style) = marker_body_style {
            let body = text.trim_end_matches([' ', '\t']);
            builder.open_inline(id, &body_style, InlineEdges::default());
            builder.push_text(TextSource::Generated { node: id }, body);
            builder.close_inline();
            builder.push_text(TextSource::Generated { node: id }, &text[body.len()..]);
        } else {
            builder.push_text(TextSource::Generated { node: id }, &text);
        }
    } else {
        for piece in &pieces {
            match piece {
                GeneratedPiece::Text(text) if atomic => {
                    builder.push_text(TextSource::Generated { node: id }, text);
                }
                GeneratedPiece::Text(text) => {
                    first_letter.push_with_counters(
                        builder,
                        doc,
                        cascade,
                        TextSource::Generated { node: id },
                        cv,
                        text,
                        fonts,
                        counters,
                    )?;
                }
                GeneratedPiece::Image(image) => {
                    // An image is in-flow content ahead of any later letter
                    // (CSS Pseudo 4 §2.3), and its break opportunities come
                    // from the pseudo-element around it.
                    first_letter.stop();
                    let image = NodeId(*image as u64);
                    text_nowrap
                        .insert(image.0, cv.effective_text_wrap_mode == TextWrapMode::Nowrap);
                    builder.push_atomic(image, &inline_style, InlineEdges::default());
                }
            }
        }
    }
    if !inline_level && block.is_none() && pseudo == PseudoElem::Before {
        builder.push_forced_break(id);
    }
    if open_inline {
        builder.close_inline();
    }
    if let Some(block) = &block {
        // The box held the first letter, or took the first formatted line.
        first_letter.stop();
        builder.push_block_in_inline(NodeId(block.end));
    }
    match builder.error() {
        Some(error) => Err(IfcError::Limit(error)),
        None => Ok(()),
    }
}

/// The box kind of a child of an ifc root, from its computed style; `None` for
/// content that belongs to the paragraph.
pub(crate) fn box_kind(cascade: &CascadeResult, doc: &Document, id: usize) -> Option<IfcBoxKind> {
    box_kind_in(
        cascade,
        doc,
        crate::layout::box_tree::BoxTree::for_layout(doc),
        id,
    )
}

pub(crate) fn box_kind_in(
    cascade: &CascadeResult,
    doc: &Document,
    boxes: crate::layout::box_tree::BoxTree<'_>,
    id: usize,
) -> Option<IfcBoxKind> {
    let cv = cascade.computed.get(id)?;
    let properties = boxes.properties(id)?;
    let node = doc.get_node(id)?;
    if node.kind() != NodeKind::Element
        || matches!(
            properties.display(),
            DisplayValue::None | DisplayValue::Contents
        )
    {
        return None;
    }
    // An absolutely positioned or fixed box is out of flow whatever its
    // display and float (CSS 2.1 9.7: its float computes to none).
    if matches!(
        properties.position(),
        PositionValue::Absolute | PositionValue::Fixed
    ) {
        return Some(IfcBoxKind::OutOfFlow);
    }
    // Logical float sides are not mapped to physical ones: the box is laid
    // out as not floated, as the taffy bridge maps it.
    if matches!(cv.float, FloatValue::Left | FloatValue::Right) {
        return Some(IfcBoxKind::Float);
    }
    // An inline-level table is an atomic inline too (CSS 2.1 17.4).
    let inline_block = properties.display().is_atomic_inline_level();
    let tag = node.tag_name().unwrap_or("");
    let replaced = properties.display().is_inline_flow()
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
    if properties.display().is_block_flex_or_grid_container()
        || matches!(
            properties.display(),
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::Table
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableRow
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
                | DisplayValue::BlockRuby
        )
    {
        // A block-level box in ruby content is inlinized: it is an atomic
        // inline in the line (CSS Ruby 1 §2.2).
        let in_ruby = is_inlinized_in_ruby(boxes, id);
        return Some(if in_ruby {
            IfcBoxKind::Atomic
        } else {
            IfcBoxKind::Block
        });
    }
    None
}

/// The rendered legend of `root` when `root` is an HTML `fieldset` whose
/// content is a paragraph: the first `legend` child that is neither floated
/// nor absolutely positioned, when it is a block. The legend sits over the
/// block-start border of the fieldset, above all its lines, so it is the
/// first box of the paragraph wherever it stands among the children, and the
/// line loop places it over the border (HTML Living Standard, "Rendering"
/// §15.3.13 "The fieldset and legend elements").
///
/// As for a fieldset with block content, one in a vertical writing mode or
/// one that clips its overflow keeps the legend as a block between its lines,
/// and so do a multicol fieldset, whose lines are as wide as a column, and a
/// ruby fieldset, which lays out its content as ruby.
// spec: https://html.spec.whatwg.org/multipage/rendering.html#the-fieldset-and-legend-elements
pub(crate) fn paragraph_legend(
    cascade: &CascadeResult,
    doc: &Document,
    root: usize,
) -> Option<usize> {
    let fieldset = doc.get_node(root)?;
    let cv = cascade.computed.get(root)?;
    if !fieldset.is_html_fieldset()
        || cv.cssom_writing_mode.is_vertical()
        || cv.overflow.x != OverflowValue::Visible
        || cv.overflow.y != OverflowValue::Visible
        || !matches!(cv.column_count, ColumnCountValue::Auto)
        || !matches!(cv.column_width, ComputedColumnWidth::Auto)
        || fieldset.display().is_ruby_container()
    {
        return None;
    }
    let legend = fieldset.children.iter().copied().find(|&child| {
        let node = &doc.nodes[child];
        node.is_in_document()
            && node.tag_name() == Some("legend")
            && node.namespace_uri().is_none()
            && node.display() != DisplayValue::None
            && !matches!(
                box_kind(cascade, doc, child),
                Some(IfcBoxKind::Float | IfcBoxKind::OutOfFlow)
            )
    })?;
    (box_kind(cascade, doc, legend) == Some(IfcBoxKind::Block)).then_some(legend)
}

/// How many inline boxes between a block-level box and a ruby box are looked
/// through for [`is_inlinized_in_ruby`].
const MAX_INLINIZE_DEPTH: usize = 64;

/// Whether the block-level box `id` is in ruby content: the nearest box around
/// it that is not an inline box is a ruby container or a ruby internal box.
/// Ruby content is inline-level, so such a box is inlinized (CSS Ruby 1 §2.2).
fn is_inlinized_in_ruby(boxes: crate::layout::box_tree::BoxTree<'_>, id: usize) -> bool {
    let mut current = id;
    for _ in 0..MAX_INLINIZE_DEPTH {
        let Some(parent) = boxes.parent(current, crate::layout::BoxParentScope::LayoutRoot) else {
            return false;
        };
        match boxes.display(parent) {
            Some(display) if display.is_ruby() => return true,
            Some(DisplayValue::Inline) => current = parent,
            _ => return false,
        }
    }
    false
}

/// A physical `clear` side; `None` for `none` and the logical sides, which
/// the taffy bridge maps to none.
fn physical_clear(clear: ClearValue) -> Option<Clear> {
    match clear {
        ClearValue::Left => Some(Clear::Left),
        ClearValue::Right => Some(Clear::Right),
        ClearValue::Both => Some(Clear::Both),
        _ => None,
    }
}

enum Step {
    /// A node to project.
    Enter(usize),
    /// The `::after` of an element, after its content.
    After(usize),
    Before(usize),
    Close,
}

impl ProjectedBuilder {
    /// Shape the paragraph.
    ///
    /// # Errors
    /// [`IfcError::Limit`] when a shodo resource limit is exceeded.
    pub(crate) fn build(
        mut self,
        cx: &mut LayoutContext,
        fonts: &FontCollection,
    ) -> Result<ProjectedIfc, IfcError> {
        // The text shows a float or an absolutely positioned box as the same
        // character as an atomic inline, and the override cannot tell them
        // apart, so a paragraph that has one keeps the Unicode opportunities.
        let kinds = || self.meta.boxes.iter().map(|inline_box| inline_box.kind);
        let has_atomic = self.meta.marker_atomic.is_some()
            || !self.meta.generated_atomics.is_empty()
            || !self.meta.generated_boxes.is_empty()
            || kinds().any(|kind| kind == IfcBoxKind::Atomic);
        let has_out_of_flow =
            kinds().any(|kind| matches!(kind, IfcBoxKind::Float | IfcBoxKind::OutOfFlow));
        if has_atomic && !has_out_of_flow {
            self.paragraph
                .with_line_break_override(super::atomic_breaks::around_atomics);
        }
        let paragraph = self.paragraph.build(cx, fonts).map_err(IfcError::Limit)?;
        let block_intrinsics = match self.block_intrinsics {
            Some(apart) => {
                let BlockIntrinsics {
                    mut paragraph,
                    contents,
                } = *apart;
                if has_atomic && !has_out_of_flow {
                    paragraph.with_line_break_override(super::atomic_breaks::around_atomics);
                }
                let contents = contents
                    .into_iter()
                    .map(|mut content| {
                        if content.has_images && !has_out_of_flow {
                            content
                                .paragraph
                                .with_line_break_override(super::atomic_breaks::around_atomics);
                        }
                        Ok(BlockContent {
                            start: content.start,
                            paragraph: content.paragraph.build(cx, fonts)?,
                            has_images: content.has_images,
                        })
                    })
                    .collect::<Result<_, _>>()
                    .map_err(IfcError::Limit)?;
                Some(Box::new(BlockIntrinsics {
                    paragraph: paragraph.build(cx, fonts).map_err(IfcError::Limit)?,
                    contents,
                }))
            }
            None => None,
        };
        Ok(Projected {
            paragraph,
            meta: self.meta,
            block_intrinsics,
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
/// The errors of [`project_ifc_text_run_builder`] and [`ProjectedBuilder::build`].
#[cfg(test)]
pub(crate) fn project_ifc_text(
    doc: &Document,
    cascade: &CascadeResult,
    text: usize,
    cx: &mut LayoutContext,
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedIfc, IfcError> {
    let container = doc.parent_of(text).unwrap_or(text);
    project_ifc_text_run_builder(doc, cascade, container, text, &[text], fonts, limits)?
        .build(cx, fonts)
}

/// Fill a paragraph builder with the text nodes of `run`, in order: the
/// contents of one anonymous flex or grid item of `container`. The node `text`
/// of the run is the node the paragraph is owned by.
///
/// A flex or grid container wraps each run of its text in an anonymous item
/// (CSS Flexbox 1, 4; CSS Grid 1, 6), whose box has no edges of its own and
/// whose inherited properties come from the container. Those values style the
/// paragraph, and so give its strut and line options.
///
/// Text nodes that are not siblings can still make one run, since a `display:
/// contents` element between them generates no box (CSS Display 3, 2.5). Such
/// an element is no inline box here; a text node whose values shape its text
/// differently from the item's is wrapped in an inline box of its own, with no
/// edges, and the rest is pushed with the item's style.
///
/// # Errors
/// [`IfcError::InvalidNode`] for a run without `text`, or an unknown, detached
/// or non-text node in it, or an unknown `container`,
/// [`IfcError::Unsupported`] for a style the inline engine does not map, and
/// [`IfcError::Limit`] when a shodo resource limit is exceeded.
pub(crate) fn project_ifc_text_run_builder(
    doc: &Document,
    cascade: &CascadeResult,
    container: usize,
    text: usize,
    run: &[usize],
    fonts: &FontCollection,
    limits: &Limits,
) -> Result<ProjectedBuilder, IfcError> {
    let mut members = Vec::with_capacity(run.len());
    for &member in run {
        let node = doc.get_node(member).ok_or(IfcError::InvalidNode(member))?;
        let member_cv = cascade
            .computed
            .get(member)
            .ok_or(IfcError::InvalidNode(member))?;
        if !node.is_in_document() || node.kind() != NodeKind::Text {
            return Err(IfcError::InvalidNode(member));
        }
        let content = node.text_content().ok_or(IfcError::InvalidNode(member))?;
        members.push((member, content, member_cv));
    }
    if !members.iter().any(|(member, ..)| *member == text) {
        return Err(IfcError::InvalidNode(text));
    }
    // The anonymous item inherits from the container, not from the text node
    // the run happens to start with: that node can sit under a `display:
    // contents` element with values of its own.
    let container_cv = cascade
        .computed
        .get(container)
        .ok_or(IfcError::InvalidNode(container))?;
    let cv = &ComputedValues::inherit_from(container_cv);
    // As for an element root: right-to-left content is ordered without
    // reading `unicode-bidi`.
    let (options, indent) = style::line_options(cv, container, fonts)?;
    let root_style = styled(doc, cascade, cv, container, fonts)?;
    let mut paragraph_style = style::paragraph_style(cv, container, root_style.clone())?;
    paragraph_style.line_height_quirk = line_height_quirk(doc);
    let writing_mode = paragraph_style.writing_mode;
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let cleared_breaks = Vec::new();
    let mut text_nowrap = HashMap::new();
    let mut reached_root = false;
    for (member, content, member_cv) in members {
        reached_root |= member == text;
        text_nowrap.insert(
            member as u64,
            member_cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
        );
        // The text before the root holds only white space, which no box
        // renders at the start of the item whatever its `white-space` is (as
        // for a flex container's own white space), so it adds nothing.
        if !reached_root {
            continue;
        }
        // A text node under a `display: contents` element has the values that
        // element gives it, which can shape the text differently from the
        // item's. The element is no box here, so the text alone gets an inline
        // box to carry its own style.
        let own_style =
            Some(styled(doc, cascade, member_cv, member, fonts)?).filter(|own| *own != root_style);
        if let Some(own) = &own_style {
            builder.open_inline(NodeId(member as u64), own, InlineEdges::default());
        }
        builder.push_text(
            TextSource::Dom {
                node: NodeId(member as u64),
                offset: 0,
            },
            content,
        );
        if own_style.is_some() {
            builder.close_inline();
        }
    }
    if let Some(error) = builder.error() {
        return Err(IfcError::Limit(error));
    }
    Ok(Projected {
        paragraph: builder,
        meta: ProjectedMeta {
            ruby_containers: super::ruby::RubyContainers::none(),
            writing_mode,
            options,
            indent,
            boxes: Vec::new(),
            marker_atomic: None,
            outside_marker_height: None,
            generated_atomics: Vec::new(),
            generated_blocks: Vec::new(),
            generated_boxes: Vec::new(),
            rtl: cv.direction == Direction::Rtl,
            offsets: Vec::new(),
            percent_insets: Vec::new(),
            cleared_breaks,
            run_tail: run
                .iter()
                .copied()
                .filter(|&member| member != text)
                .collect(),
            fixed: false,
            // An anonymous flex or grid item has no overflow of its own.
            ellipsis: false,
            nowrap: cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
            text_nowrap: Arc::new(text_nowrap),
            letter_styles: Vec::new(),
            // An anonymous item has no first formatted line of an element.
            first_line_opacity: None,
            first_line_styles: FirstLineStyles::none(),
        },
        block_intrinsics: None,
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
    project_children_builder(doc, cascade, root, None, fonts, limits, counters)
}

/// Project an anonymous cell's source children using its owner's inherited text style.
pub(crate) fn project_anonymous_cell_builder(
    doc: &Document,
    cascade: &CascadeResult,
    owner: usize,
    children: &[crate::layout::table::anonymous::Content],
    fonts: &FontCollection,
    limits: &Limits,
    counters: &GeneratedCounters,
) -> Result<ProjectedBuilder, IfcError> {
    project_children_builder(doc, cascade, owner, Some(children), fonts, limits, counters)
}

/// Project `root`'s content (see [`project_children_builder_as`]); a
/// horizontal paragraph with block-level `::before` or `::after` boxes is
/// projected once more with their content apart, for its intrinsic widths
/// (see [`BlockIntrinsics`]).
#[allow(clippy::too_many_arguments)]
fn project_children_builder(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    anonymous_children: Option<&[crate::layout::table::anonymous::Content]>,
    fonts: &FontCollection,
    limits: &Limits,
    counters: &GeneratedCounters,
) -> Result<ProjectedBuilder, IfcError> {
    let project = |blocks_apart| {
        project_children_builder_as(
            doc,
            cascade,
            root,
            anonymous_children,
            fonts,
            limits,
            counters,
            blocks_apart,
        )
    };
    let (mut projected, _) = project(false)?;
    if projected.meta.writing_mode == WritingMode::HorizontalTb
        && projected
            .meta
            .generated_blocks
            .iter()
            .any(|block| block.content_apart)
    {
        let (apart, contents) = project(true)?;
        if !contents.is_empty() {
            projected.block_intrinsics = Some(Box::new(BlockIntrinsics {
                paragraph: apart.paragraph,
                contents,
            }));
        }
    }
    Ok(projected)
}

/// Project `root`'s content. With `blocks_apart`, the content of its
/// block-level `::before` and `::after` boxes is left out of the paragraph and
/// kept in [`Projected::block_intrinsics`] with that paragraph.
#[allow(clippy::too_many_arguments)]
fn project_children_builder_as(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    anonymous_children: Option<&[crate::layout::table::anonymous::Content]>,
    fonts: &FontCollection,
    limits: &Limits,
    counters: &GeneratedCounters,
    blocks_apart: bool,
) -> Result<(ProjectedBuilder, Vec<BlockContent<ParagraphBuilder>>), IfcError> {
    let root_node = doc.get_node(root).ok_or(IfcError::InvalidNode(root))?;
    let root_cv = cascade
        .computed
        .get(root)
        .ok_or(IfcError::InvalidNode(root))?;
    if !root_node.is_in_document()
        || root_node.kind() != NodeKind::Element
        || (anonymous_children.is_none() && !super::assign::can_be_ifc_root(doc, root))
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
    let mut first_letter =
        FirstLetter::new_with_predecessors(doc, cascade, root, limits, &counters.predecessors);
    // The first formatted line of a table cell's anonymous content is not
    // found from the cell's own `::first-line`.
    if anonymous_children.is_some() {
        first_letter.first_line = None;
    }
    if let Some(first_line) = &first_letter.first_line
        && !first_line.in_own_block()
    {
        paragraph_style.first_line =
            Some(styled(doc, cascade, first_line.root_values(), root, fonts)?);
    }
    let mut builder = ParagraphBuilder::new(&paragraph_style, limits);
    let mut boxes = Vec::new();
    let mut ruby_containers = super::ruby::RubyContainers::default();
    let mut offsets = Vec::new();
    let mut percent_insets = Vec::new();
    let mut cleared_breaks = Vec::new();
    let mut text_nowrap = HashMap::new();
    let mut generated_images = GeneratedImages {
        atomics: Vec::new(),
        writing_mode,
        offsets: Vec::new(),
        blocks: Vec::new(),
        boxes: Vec::new(),
        percent_insets: Vec::new(),
        rtl: root_cv.direction == Direction::Rtl,
        block_contents: blocks_apart.then(|| (limits.clone(), Vec::new())),
    };
    let marker_atomic = if anonymous_children.is_some() {
        None
    } else if crate::generated_content::inside_marker_in_flow(cascade, root)
        && let Some(size) = doc.list_marker_image_size(root)
    {
        let id = NodeId(generated_node_id(root, PseudoElem::Marker) as u64);
        let marker_cv = crate::generated_content::computed_for_id(cascade, id.0 as usize)
            .ok_or(IfcError::InvalidNode(root))?;
        let marker_fallback;
        let marker_cv = if cascade.pseudo.contains_key(&(
            raikiri_style::StyleNodeId::new(root as u64),
            PseudoElem::Marker,
        )) {
            marker_cv
        } else {
            marker_fallback = raikiri_style::ComputedValues::inherit_marker_from(marker_cv);
            &marker_fallback
        };
        text_nowrap.insert(
            id.0,
            marker_cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
        );
        let mut marker_style = styled(doc, cascade, marker_cv, root, fonts)?;
        // The image is indivisible. Permit a break after its preserved suffix
        // space so the next word can move independently to the following line.
        marker_style.text_wrap_mode = shodo::style::TextWrapMode::Wrap;
        builder.open_inline(id, &marker_style, InlineEdges::default());
        builder.push_atomic(id, &marker_style, InlineEdges::default());
        builder.push_text(TextSource::Generated { node: id }, " ");
        builder.close_inline();
        Some((
            id,
            shodo::AtomicSize {
                inline_size: if writing_mode == shodo::geometry::WritingMode::HorizontalTb {
                    size.width
                } else {
                    size.height
                },
                block_size: if writing_mode == shodo::geometry::WritingMode::HorizontalTb {
                    size.height
                } else {
                    size.width
                },
                ..shodo::AtomicSize::default()
            },
        ))
    } else {
        push_generated(
            &mut builder,
            doc,
            cascade,
            root,
            PseudoElem::Marker,
            fonts,
            counters,
            &mut first_letter,
            &mut text_nowrap,
            &mut generated_images,
        )?;
        None
    };
    // The rendered legend of a fieldset comes before the `::before` of the
    // fieldset, which is part of the content below the legend.
    let legend = anonymous_children
        .is_none()
        .then(|| paragraph_legend(cascade, doc, root))
        .flatten();
    if anonymous_children.is_none() && legend.is_none() {
        push_generated(
            &mut builder,
            doc,
            cascade,
            root,
            PseudoElem::Before,
            fonts,
            counters,
            &mut first_letter,
            &mut text_nowrap,
            &mut generated_images,
        )?;
    }

    // A ruby container that lays out its own content (a flex item, an
    // absolutely positioned ruby, or a floated `block ruby`) keeps its ruby
    // layout.
    let root_plan = if anonymous_children.is_none() && root_node.display().is_ruby_container() {
        super::ruby::plan(doc, cascade, root)
    } else {
        None
    };
    if let Some(plan) = &root_plan {
        super::ruby::push(
            &mut builder,
            doc,
            cascade,
            root,
            plan,
            fonts,
            limits,
            &mut super::ruby::Sink {
                containers: &mut ruby_containers,
                boxes: &mut boxes,
                text_nowrap: &mut text_nowrap,
                offsets: &mut offsets,
                percent_insets: &mut percent_insets,
                first_letter: &mut first_letter,
                counters,
                rtl: root_cv.direction == Direction::Rtl,
            },
            true,
        )?; // cov:ignore: ruby content fails only on the style and counter errors the plain inline path reports for the same content.
    }
    let mut stack: Vec<Step> = match anonymous_children {
        _ if root_plan.is_some() => Vec::new(),
        Some(children) => children
            .iter()
            .rev()
            .map(|entry| match entry {
                crate::layout::table::anonymous::Content::Node(id) => Step::Enter(*id),
                crate::layout::table::anonymous::Content::Before(id) => Step::Before(*id),
                crate::layout::table::anonymous::Content::After(id) => Step::After(*id),
            })
            .collect(),
        None => {
            // The first summary of a `details` comes first in its paragraph
            // too, so the text around it joins up after it.
            let summary = crate::layout::page::summary_out_of_place(doc, cascade, root);
            // The rendered legend of a fieldset comes first too: it sits over
            // the border, above all the lines.
            let rest = root_node
                .children
                .iter()
                .copied()
                .filter(|&id| Some(id) != summary && Some(id) != legend)
                .map(Step::Enter);
            let legend = legend
                .into_iter()
                .flat_map(|legend| [Step::Enter(legend), Step::Before(root)]);
            summary
                .into_iter()
                .map(Step::Enter)
                .chain(legend)
                .chain(rest)
                .rev()
                .collect()
        }
    };
    while let Some(step) = stack.pop() {
        let id = match step {
            Step::Close => {
                builder.close_inline();
                continue;
            }
            Step::Before(id) => {
                push_generated(
                    &mut builder,
                    doc,
                    cascade,
                    id,
                    PseudoElem::Before,
                    fonts,
                    counters,
                    &mut first_letter,
                    &mut text_nowrap,
                    &mut generated_images,
                )?;
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
                    &mut first_letter,
                    &mut text_nowrap,
                    &mut generated_images,
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
            // Text a closed `details` hides has no box (HTML LS §4.11.1).
            NodeKind::Text if node.display() == DisplayValue::None => {}
            NodeKind::Text => {
                let text = node.text_content().ok_or(IfcError::InvalidNode(id))?;
                let parent_style = &cascade.computed[doc.parent_of(id).unwrap_or(id)];
                if let Some(first_line) = first_letter.first_line.as_mut() {
                    first_line.note_text(doc, cascade, id);
                }
                text_nowrap.insert(
                    id as u64,
                    parent_style.effective_text_wrap_mode == TextWrapMode::Nowrap,
                );
                // A continuation break splits the text so the line after it
                // starts at the requested byte, as it did on an earlier page.
                let split = doc
                    .continuation_break
                    .filter(|&(node, _)| node == id)
                    .map(|(_, offset)| offset as usize)
                    .filter(|&offset| offset <= text.len() && text.is_char_boundary(offset));
                match split {
                    Some(offset) => {
                        if offset > 0 {
                            first_letter.push_with_counters(
                                &mut builder,
                                doc,
                                cascade,
                                TextSource::Dom {
                                    node: NodeId(id as u64),
                                    offset: 0,
                                },
                                parent_style,
                                &text[..offset],
                                fonts,
                                counters,
                            )?; // cov:ignore: an earlier layout pushed this text unsplit, so it would have failed there
                        }
                        first_letter.stop();
                        builder.push_forced_break(NodeId(id as u64));
                        if offset < text.len() {
                            builder.push_text(
                                TextSource::Dom {
                                    node: NodeId(id as u64),
                                    offset: offset as u32,
                                },
                                &text[offset..],
                            );
                        }
                    }
                    None => {
                        first_letter.push_with_counters(
                            &mut builder,
                            doc,
                            cascade,
                            TextSource::Dom {
                                node: NodeId(id as u64),
                                offset: 0,
                            },
                            parent_style,
                            text,
                            fonts,
                            counters,
                        )?;
                    }
                }
            }
            NodeKind::Comment | NodeKind::ProcessingInstruction => {}
            NodeKind::Element => {
                let cv = cascade.computed.get(id).ok_or(IfcError::InvalidNode(id))?;
                // The parts of an anonymous table are one block-level box
                // between the lines around them: the first stands for the
                // table, and the others are laid out inside it.
                if let Some(table) = doc.anonymous_table_of(id) {
                    if doc.anonymous_table_parts(table).and_then(<[usize]>::first) == Some(&id) {
                        first_letter.stop();
                        builder.push_block_in_inline(NodeId(table as u64));
                        boxes.push(IfcBox {
                            node: table,
                            kind: IfcBoxKind::Block,
                        });
                        if let Some(error) = builder.error() {
                            return Err(IfcError::Limit(error));
                        }
                    }
                    continue;
                }
                // `<style>`, `<script>` and friends render nothing even when
                // an author rule gives them a display type.
                // Columns and column groups render nothing of their own
                // outside a table either.
                if matches!(
                    node.display(),
                    DisplayValue::None | DisplayValue::TableColumn | DisplayValue::TableColumnGroup
                ) || node.is_unrendered_html_element()
                {
                    continue;
                }
                let unsupported = |reason: &'static str| IfcError::Unsupported { node: id, reason };
                let tag = node.tag_name().unwrap_or_default();
                if node.display() == DisplayValue::Contents {
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
                        &mut first_letter,
                        &mut text_nowrap,
                        &mut generated_images,
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
                    first_letter.stop();
                    let atomic_style = styled(doc, cascade, cv, id, fonts)?;
                    // The break opportunities around an atomic come from its
                    // parent's style, not its own.
                    let parent_style = &cascade.computed[doc.parent_of(id).unwrap_or(id)];
                    text_nowrap.insert(
                        id as u64,
                        parent_style.effective_text_wrap_mode == TextWrapMode::Nowrap,
                    );
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
                    first_letter.stop();
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
                if !node.display().is_inline_flow() {
                    return Err(unsupported("only inline-level boxes are projected"));
                }
                if node.display() == DisplayValue::Ruby
                    && let Some(plan) = super::ruby::plan(doc, cascade, id)
                {
                    super::ruby::push(
                        &mut builder,
                        doc,
                        cascade,
                        id,
                        &plan,
                        fonts,
                        limits,
                        &mut super::ruby::Sink {
                            containers: &mut ruby_containers,
                            boxes: &mut boxes,
                            text_nowrap: &mut text_nowrap,
                            offsets: &mut offsets,
                            percent_insets: &mut percent_insets,
                            first_letter: &mut first_letter,
                            counters,
                            rtl: root_cv.direction == Direction::Rtl,
                        },
                        false,
                    )?; // cov:ignore: ruby content fails only on the style and counter errors the plain inline path reports for the same content.
                    continue;
                }
                let (edges, edge_percents) = style::inline_edges(cv, id, fonts)?;
                let inline_style = styled(doc, cascade, cv, id, fonts)?;
                push_relative_offset(
                    &mut offsets,
                    &mut percent_insets,
                    id,
                    cv,
                    node.box_properties(),
                    root_cv.direction == Direction::Rtl,
                );
                // `clear` on a line break moves the next line below the
                // floats (CSS 2.1, 9.5.2): the line loop looks for the break.
                if tag == "br"
                    && let Some(clear) = physical_clear(cv.clear)
                {
                    cleared_breaks.push((id, clear));
                }
                if tag == "br" {
                    first_letter.stop();
                    // The break is not wrapped in an inline box of its own: in
                    // quirks mode a box would credit its strut to every line it
                    // ends, while a `<br>` adds its strut only to a line
                    // holding nothing else (Quirks Mode Standard 3.3). That
                    // strut is the enclosing box's with the `<br>`'s own
                    // line-height, as in Chromium; outside quirks mode the
                    // `<br>`'s own style takes no part in the line.
                    match break_strut_style(doc, cascade, id, &inline_style, fonts)? {
                        Some(style) => {
                            builder.push_forced_break_with_style(NodeId(id as u64), &style);
                        }
                        None => {
                            builder.push_forced_break(NodeId(id as u64));
                        }
                    }
                } else {
                    text_nowrap.insert(
                        id as u64,
                        cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
                    );
                    if let Some(first_line) = first_letter.first_line.as_mut() {
                        first_line.note_margins(id, &edges, &edge_percents);
                    }
                    // On the first line the element's values come from the
                    // `::first-line` it sits in; its box geometry does not.
                    match first_letter
                        .first_line
                        .as_ref()
                        .and_then(|first_line| first_line.element(doc, cascade, id))
                    {
                        Some(values) => {
                            let first_style = styled(doc, cascade, &values, id, fonts)?;
                            if let Some(first_line) = first_letter.first_line.as_mut() {
                                first_line.note_element(id, values);
                            }
                            builder.open_inline_with_first_line(
                                NodeId(id as u64),
                                &inline_style,
                                &first_style,
                                edges,
                            )
                        }
                        None => builder.open_inline(NodeId(id as u64), &inline_style, edges),
                    }
                    .set_inline_edge_percents(edge_percents);
                    push_generated(
                        &mut builder,
                        doc,
                        cascade,
                        id,
                        PseudoElem::Before,
                        fonts,
                        counters,
                        &mut first_letter,
                        &mut text_nowrap,
                        &mut generated_images,
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
    if anonymous_children.is_none() {
        push_generated(
            &mut builder,
            doc,
            cascade,
            root,
            PseudoElem::After,
            fonts,
            counters,
            &mut first_letter,
            &mut text_nowrap,
            &mut generated_images,
        )?;
    }
    // The outside marker of a list item sits on the item's first line even
    // when nothing else makes one, so an item that holds no line content is
    // one line high (CSS Lists 3, 3.1). The line ends in a break, like the
    // line of a `<br>`, and its strut is the item's own. Anything the item
    // holds besides the marker (white space, an empty inline element, a float,
    // an out-of-flow box) stays in front of the break. The check stops at the
    // first child that makes a line, so an item with text is cheap to rule out.
    if anonymous_children.is_none()
        && root_plan.is_none()
        && super::assign::marker_holds_only_line(doc, cascade, root, fonts, counters)?
    {
        builder.push_forced_break(NodeId(generated_node_id(root, PseudoElem::Marker) as u64));
    }
    // The root's own `::before` and `::after` start and end the paragraph.
    for block in &mut generated_images.blocks {
        block.of_root = [PseudoElem::Before, PseudoElem::After]
            .into_iter()
            .find(|&pseudo| block.start == generated_node_id(root, pseudo) as u64);
    }
    let contents = generated_images
        .block_contents
        .take()
        .map(|(_, contents)| contents)
        .unwrap_or_default();
    Ok((
        Projected {
            paragraph: builder,
            meta: ProjectedMeta {
                ruby_containers: ruby_containers.into_shared(),
                writing_mode,
                options,
                indent,
                boxes,
                marker_atomic,
                outside_marker_height: super::outside_marker::height_in(
                    doc,
                    cascade,
                    Some((fonts, limits)),
                    root,
                ),
                generated_atomics: generated_images.atomics,
                generated_blocks: generated_images.blocks,
                generated_boxes: generated_images.boxes,
                rtl: root_cv.direction == Direction::Rtl,
                percent_insets: {
                    // The generated boxes' offsets follow the elements'.
                    let first = offsets.len();
                    percent_insets.extend(
                        generated_images
                            .percent_insets
                            .into_iter()
                            .map(|(index, insets)| (first + index, insets)),
                    );
                    percent_insets
                },
                offsets: {
                    offsets.extend(generated_images.offsets);
                    offsets
                },
                cleared_breaks,
                run_tail: Vec::new(),
                fixed: anonymous_children.is_none()
                    && super::super::fixed::is_page_fixed(doc, cascade, root),
                ellipsis: anonymous_children.is_none() && style::ends_in_ellipsis(root_cv),
                nowrap: root_cv.effective_text_wrap_mode == TextWrapMode::Nowrap,
                text_nowrap: Arc::new(text_nowrap),
                letter_styles: first_letter.styles,
                first_line_opacity: first_line_opacity(doc, cascade, root, &counters.predecessors),
                first_line_styles: first_letter
                    .first_line
                    .map_or_else(FirstLineStyles::none, |first_line| {
                        Arc::new(first_line.into_styles())
                    }),
            },
            block_intrinsics: None,
        },
        contents,
    ))
}

/// The product of the opacities below one of the `::first-line` boxes
/// around the first line of paragraph `root`, if any. The blocks whose first
/// formatted line it is are found only when one of them has a translucent
/// `::first-line`, as that takes a walk over the whole document, shared by
/// the paragraphs of one pass.
fn first_line_opacity(
    doc: &Document,
    cascade: &CascadeResult,
    root: usize,
    predecessors: &super::first_letter::PredecessorCache,
) -> Option<f32> {
    let translucent = |id: usize| {
        cascade
            .pseudo
            .get(&(
                raikiri_style::StyleNodeId::new(id as u64),
                PseudoElem::FirstLine,
            ))
            .filter(|cv| cv.opacity < 1.0)
            .map(|cv| cv.opacity)
    };
    let ancestors = doc.ancestors(root).any(|id| translucent(id).is_some());
    if !ancestors && translucent(root).is_none() {
        return None;
    }
    let (_, origins) =
        super::first_letter::typographic_origins(doc, cascade, root, predecessors, ancestors);
    let opacity: f32 = origins.into_iter().filter_map(translucent).product();
    (opacity < 1.0).then_some(opacity)
}

#[cfg(test)]
mod tests;
