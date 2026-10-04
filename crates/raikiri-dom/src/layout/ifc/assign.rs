//! Choose the blocks laid out by the shodo inline engine.

use super::boxes::IfcBoxKind;
use super::error::IfcError;
use super::projection::{
    GeneratedCounters, ProjectedBuilder, ProjectedIfc, box_kind, has_in_flow_generated_text,
    project_ifc_builder_with, project_ifc_text_builder,
};
use super::root::{IfcBuildMode, IfcRoot, IfcState};
use super::style;
use crate::Document;
use crate::node::NodeFlags;
use raikiri_style::CascadeResult;
use raikiri_style::property::{ColumnCountValue, DisplayValue, PositionValue};
use raikiri_traits::{LayoutError, NodeKind};
use rayon::prelude::*;
use shodo::LayoutContext;

/// Whether `idx` generates a box of its own: any display other than `inline`,
/// `contents` and `none`, an absolutely positioned or fixed inline element
/// (blockified, CSS 2.1 9.7), a replaced element or form control (an
/// atomic inline), and an inline element that a flex or grid
/// container blockifies as its item (CSS Display 3, 2.7), and the body layout
/// starts at, whatever its display. The cascade leaves such elements
/// `inline`; only the taffy bridge maps them to blocks.
pub(crate) fn generates_own_box(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    match cv.display {
        // Layout starts at the body, which always generates a block box: CSS
        // Display 3, 2.7 blockifies the root element (`inline` and
        // `contents` alike), and the body is the root of what raikiri lays
        // out.
        DisplayValue::Inline | DisplayValue::Contents if is_layout_root(doc, idx) => true,
        DisplayValue::Contents | DisplayValue::None => false,
        DisplayValue::Inline
            if matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed) =>
        {
            true
        }
        // Replaced elements and form controls are atomic inlines: their box is
        // their own, and so is the content inside it (a button label, the
        // text of a textarea, the fallback content of an object).
        DisplayValue::Inline
            if doc.nodes[idx]
                .tag_name()
                .is_some_and(|tag| super::projection::REPLACED_BOX_TAGS.contains(&tag)) =>
        {
            true
        }
        DisplayValue::Inline => box_parent(doc, cascade, idx).is_some_and(|parent| {
            matches!(
                cascade.computed[parent].display,
                DisplayValue::Flex
                    | DisplayValue::InlineFlex
                    | DisplayValue::Grid
                    | DisplayValue::InlineGrid
            )
        }),
        _ => true,
    }
}

/// Whether `idx` is the body layout starts at.
fn is_layout_root(doc: &Document, idx: usize) -> bool {
    doc.nodes[idx].tag_name() == Some("body") && crate::layout::find_body(doc) == Some(idx)
}

/// Whether `idx` is an ancestor of the body layout starts at: it is not laid
/// out, so it is never a paragraph (the body is the root of its own content).
fn is_above_the_layout_root(doc: &Document, idx: usize) -> bool {
    doc.nodes[idx].tag_name() == Some("html")
        && crate::layout::find_body(doc).is_some_and(|body| {
            let mut ancestor = doc.parent_of(body);
            while let Some(id) = ancestor {
                if id == idx {
                    return true;
                }
                ancestor = doc.parent_of(id);
            }
            false
        })
}

/// The nearest element ancestor of `idx` that generates a box: elements with
/// `display: contents` are skipped, their children take part in their
/// parent's layout (CSS Display 3, 2.5). `None` past the root element.
fn box_parent(doc: &Document, cascade: &CascadeResult, idx: usize) -> Option<usize> {
    let mut parent = doc.parent_of(idx)?;
    while doc.nodes[parent].kind() == NodeKind::Element
        && cascade.computed[parent].display == DisplayValue::Contents
        && !is_layout_root(doc, parent)
    {
        parent = doc.parent_of(parent)?;
    }
    (doc.nodes[parent].kind() == NodeKind::Element).then_some(parent)
}

/// Whether `idx` is a box that lays its own inline content out in lines: a
/// block container (`block`, `flow-root`, `inline-block`, `list-item`, a
/// table cell), a blockified inline flex or grid item, or a table box holding
/// only inline-level children. Flex and grid boxes and tables with rows lay
/// their children out by algorithms of their own. A table caption is a block
/// container (the table algorithm lays it out at the table's width). A
/// multicol container is a root like any block container: its lines are
/// split in columns.
pub(crate) fn can_be_ifc_root(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    // Images and inline SVG lay their content out by other means: text
    // inside them (an SVG `<title>`) is not a paragraph of theirs. Form
    // controls and the other replaced elements lay their content (a label,
    // fallback content) out as a paragraph of their own.
    let node = &doc.nodes[idx];
    let tag = node.tag_name().unwrap_or("");
    if node.is_inline_svg_content()
        || node.is_inline_svg_root()
        || super::projection::ATOMIC_TAGS.contains(&tag)
        || is_above_the_layout_root(doc, idx)
    {
        return false;
    }
    if !generates_own_box(doc, cascade, idx) {
        return false;
    }
    match cascade.computed[idx].display {
        DisplayValue::Block
        | DisplayValue::FlowRoot
        | DisplayValue::InlineBlock
        | DisplayValue::ListItem
        | DisplayValue::Inline
        | DisplayValue::TableCell
        | DisplayValue::TableCaption => true,
        // The body is a block whatever its display (see `generates_own_box`).
        DisplayValue::Contents => is_layout_root(doc, idx),
        // A table box whose children are all inline-level is one anonymous
        // cell's content (CSS 2.1 17.2.1); one with rows, row groups or
        // block children is laid out by the table algorithm.
        DisplayValue::Table | DisplayValue::InlineTable => {
            holds_only_inline_level_children(doc, cascade, idx)
        }
        _ => false,
    }
}

/// Whether every in-document element child of `idx` is inline-level
/// (`inline`, or an atomic `inline-block`), so its children form one run of
/// inline content.
fn holds_only_inline_level_children(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    doc.nodes[idx].children.iter().all(|&child| {
        let node = &doc.nodes[child];
        node.kind() != NodeKind::Element
            || !node.is_in_document()
            || matches!(
                cascade.computed[child].display,
                DisplayValue::Inline | DisplayValue::InlineBlock | DisplayValue::None
            )
    })
}

/// Whether the paragraph has inline content that makes a line: text other
/// than white space (generated text included), an atomic inline, a `<br>`,
/// or an inline element with a margin, border or padding on an inline side
/// (CSS 2.1, 9.4.2: a line box without any of these is treated as
/// zero-height). The content of its boxes does not count, and neither do
/// floats and block children alone: they are not inline content, so a block
/// that holds only those is laid out by the block algorithm.
fn has_inline_content(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    fonts: &shodo::font::FontCollection,
) -> bool {
    if has_in_flow_generated_text(cascade, idx) {
        return true;
    }
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                if node
                    .text_content()
                    .is_some_and(|text| text.chars().any(|c| !c.is_ascii_whitespace()))
                {
                    return true;
                }
            }
            NodeKind::Element => match box_kind(cascade, doc, id) {
                Some(IfcBoxKind::Atomic) => return true,
                Some(_) => {}
                None => {
                    let cv = &cascade.computed[id];
                    if cv.display == DisplayValue::None
                        || node.is_non_rendered_html_element()
                        || node.is_inline_svg_content()
                    {
                        continue;
                    }
                    if has_in_flow_generated_text(cascade, id) {
                        return true;
                    }
                    if cv.display == DisplayValue::Inline {
                        if node.tag_name() == Some("br") {
                            return true;
                        }
                        // An edge the engine cannot map yet keeps the
                        // paragraph out at projection; counting it here only
                        // lets projection decide.
                        let edged = style::inline_edges(cv, id, fonts).map_or(true, |e| {
                            e.inline_start_total() + e.inline_end_total() != 0.0
                        });
                        if edged {
                            return true;
                        }
                    }
                    stack.extend(node.children.iter().copied());
                }
            },
            _ => {}
        }
    }
    false
}

/// A paragraph the walk accepted, waiting to be shaped.
struct Candidate {
    idx: usize,
    projected: ProjectedBuilder,
}

/// What a projection or shaping error of the paragraph `root` means for the
/// layout: there is no other path to lay a paragraph out, so a limit and a
/// refusal both fail it.
fn projection_error(root: usize, error: IfcError) -> LayoutError {
    match error {
        IfcError::Limit(limit) => LayoutError::IfcLimitExceeded {
            node: root,
            limit: limit.to_string(),
        },
        IfcError::Unsupported { node, reason } => LayoutError::IfcUnsupported { node, reason },
        IfcError::CounterSnapshots(error) => LayoutError::CounterSnapshotLimitExceeded {
            limit: error.limit,
            actual: error.actual,
        },
        IfcError::InvalidNode(node) => LayoutError::IfcUnsupported {
            node,
            reason: "the node is not in the document",
        },
    }
}

fn build_error(root: usize, error: IfcError) -> LayoutError {
    match error {
        IfcError::Limit(limit) => LayoutError::IfcLimitExceeded {
            node: root,
            limit: limit.to_string(),
        },
        IfcError::CounterSnapshots(error) => LayoutError::CounterSnapshotLimitExceeded {
            limit: error.limit,
            actual: error.actual,
        },
        other => LayoutError::Internal {
            message: other.to_string(),
        },
    }
}

/// Clear every IFC mark, then mark the eligible roots and their subtrees.
///
/// Runs in three steps: a walk over the document that decides which blocks
/// are paragraph roots and fills their builders, the shaping of those
/// builders, which touches neither the document nor the cascade, and the
/// writing of the shaped paragraphs and marks back, in document order.
///
/// # Errors
/// [`LayoutError::IfcLimitExceeded`] when a paragraph exceeds a limit of the
/// engine, and [`LayoutError::IfcUnsupported`] for a paragraph the engine
/// refuses. Nothing is marked then.
pub(crate) fn assign_ifc_roots(
    doc: &mut Document,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    for node in &mut doc.nodes {
        node.flags
            .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
        node.ifc = None;
    }
    // Take the engine state out so the walk can borrow the document.
    let Some(mut state) = doc.ifc.take() else {
        return Ok(());
    };
    // The roots are rebuilt on every pass, so a cached layout would skip the
    // measure callback that fills their lines.
    doc.layout_dirty = true;
    let built = collect_candidates(doc, cascade, &state).and_then(|mut candidates| {
        candidates.extend(collect_text_candidates(doc, cascade, &state)?);
        candidates.sort_by_key(|candidate| candidate.idx);
        build_all(&mut state, candidates)
    });
    let result = built.map(|built| write_roots(doc, built));
    doc.ifc = Some(state);
    result?;
    match text_outside_paragraphs(doc, cascade) {
        Some(node) => Err(LayoutError::IfcUnsupported {
            node,
            reason: "text outside any paragraph",
        }),
        None => Ok(()),
    }
}

/// The first rendered text node with more than collapsible white space that
/// no paragraph lays out: it would be neither measured nor drawn. Text of
/// non-rendered elements (`head`, `script`, `template`, ...), of elements
/// that do not generate boxes (`display: none` on an ancestor), and SVG
/// content (drawn by the SVG renderer) is not text of the document flow.
fn text_outside_paragraphs(doc: &Document, cascade: &CascadeResult) -> Option<usize> {
    (0..doc.nodes.len()).find(|&idx| {
        let node = &doc.nodes[idx];
        if node.kind() != NodeKind::Text
            || !node.is_in_document()
            || node.is_inline_svg_content()
            || node
                .flags
                .intersects(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE)
            || node.text_content().is_none_or(|text| {
                text.chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
            })
        {
            return false;
        }
        let mut ancestor = doc.parent_of(idx);
        while let Some(id) = ancestor {
            let element = &doc.nodes[id];
            if element.kind() == NodeKind::Element
                && (element.is_non_rendered_html_element()
                    || cascade.computed[id].display == DisplayValue::None)
            {
                return false;
            }
            ancestor = doc.parent_of(id);
        }
        true
    })
}

/// Walk the document in index order and return the eligible paragraphs, in
/// ascending index order, with their builders.
///
/// The content of an accepted paragraph is taken here, before it is shaped:
/// that content is text and inline elements, none of which can be a root,
/// while the inside of its boxes is not taken and may hold roots of its own.
/// So a paragraph that later fails to shape changes no other paragraph's
/// eligibility.
fn collect_candidates(
    doc: &Document,
    cascade: &CascadeResult,
    state: &IfcState,
) -> Result<Vec<Candidate>, LayoutError> {
    let mut candidates = Vec::new();
    let mut taken = vec![false; doc.nodes.len()];
    let counters = GeneratedCounters::default();
    for idx in 0..doc.nodes.len() {
        let node = &doc.nodes[idx];
        // A root is any box that lays its own inline content out, whatever
        // its parent's layout: a block child of another paragraph is a box of
        // that paragraph and the root of its own content. Inline content of a
        // paragraph never qualifies, so `taken` only saves the work.
        if node.kind() != NodeKind::Element
            || !node.is_in_document()
            || taken[idx]
            || is_ruby_multicol_flex_projection(doc, cascade, idx)
            || !can_be_ifc_root(doc, cascade, idx)
            || !has_inline_content(doc, cascade, idx, &state.fonts)
        {
            continue;
        }
        // From here on the box is a paragraph: anything that keeps it from
        // the engine is a refusal. A fixed box is a paragraph like any other:
        // taffy sizes it against its parent on either path.
        let projected = match project_ifc_builder_with(
            doc,
            cascade,
            idx,
            &state.fonts,
            &state.limits,
            &counters,
        ) {
            Ok(projected) => projected,
            Err(error) => return Err(projection_error(idx, error)),
        };
        let mut stack = doc.nodes[idx].children.clone();
        while let Some(id) = stack.pop() {
            if projected.boxes.iter().any(|b| b.node == id) {
                continue;
            }
            taken[id] = true;
            stack.extend(doc.nodes[id].children.iter().copied());
        }
        taken[idx] = true;
        candidates.push(Candidate { idx, projected });
    }
    Ok(candidates)
}

/// Keep empty ruby items in multicol containers on the legacy projection
/// path, which lays each ruby item out as a flex item. The IFC path currently
/// treats the base and annotation as unrelated atomic inlines.
fn is_ruby_multicol_flex_projection(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let ColumnCountValue::Count(count) = cascade.computed[idx].column_count else {
        return false;
    };
    let has_multiple_columns = count > 1;

    let mut has_ruby = false;
    let mut has_break = false;
    for &child in &doc.nodes[idx].children {
        let node = &doc.nodes[child];
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                if node
                    .text_content()
                    .is_some_and(|text| text.chars().any(|c| !is_css_whitespace(c)))
                {
                    return false;
                }
            }
            NodeKind::Element => match node.tag_name() {
                Some("ruby") => has_ruby = true,
                Some("br") => has_break = true,
                _ if cascade.computed[child].display == DisplayValue::None => {}
                _ => return false,
            },
            _ => {} // cov:ignore: comments and processing instructions carry no inline content.
        }
    }

    if !has_ruby || (has_multiple_columns && !has_break) {
        return false;
    }

    let mut stack = doc.nodes[idx].children.clone();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() || cascade.computed[id].display == DisplayValue::None {
            continue;
        }
        if node.kind() == NodeKind::Text
            && node
                .text_content()
                .is_some_and(|text| text.chars().any(|c| !is_css_whitespace(c)))
        {
            return false;
        }
        stack.extend(node.children.iter().copied());
    }

    true
}

fn is_css_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}')
}

/// Whether `text` is laid out by a flex or grid container as an anonymous
/// item of its own: it is a child of the container (through `display:
/// contents` elements) and holds more than
/// collapsible white space (CSS Flexbox 1, 4; CSS Grid 1, 6). The white-space
/// test matches the item collection of the taffy tree.
fn is_anonymous_item_text(doc: &Document, cascade: &CascadeResult, text: usize) -> bool {
    let node = &doc.nodes[text];
    if node.kind() != NodeKind::Text || !node.is_in_document() {
        return false;
    }
    let Some(parent) = box_parent(doc, cascade, text) else {
        return false;
    };
    matches!(
        cascade.computed[parent].display,
        DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Grid
            | DisplayValue::InlineGrid
    ) && node.text_content().is_some_and(|text| {
        !text
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
    })
}

/// The text nodes that are anonymous flex or grid items and can be laid out
/// as paragraphs of their own, with their builders. Such a text node is the
/// root of its paragraph; it has no boxes, so only the checks on its style and
/// its ancestors apply.
fn collect_text_candidates(
    doc: &Document,
    cascade: &CascadeResult,
    state: &IfcState,
) -> Result<Vec<Candidate>, LayoutError> {
    let mut candidates = Vec::new();
    for idx in 0..doc.nodes.len() {
        if !is_anonymous_item_text(doc, cascade, idx) {
            continue;
        }
        match project_ifc_text_builder(doc, cascade, idx, &state.fonts, &state.limits) {
            Ok(projected) => candidates.push(Candidate { idx, projected }),
            Err(error) => return Err(projection_error(idx, error)),
        }
    }
    Ok(candidates)
}

/// Shape every candidate. A candidate over a limit of the engine fails the
/// layout (the first such candidate in document order is reported).
///
/// With enough candidates, and when the document allowed it, the candidates
/// are shaped on several threads: each worker owns a layout context (it is
/// `Send` but not `Sync`) and the font collection is shared by reference.
/// The result keeps the candidates' order either way.
fn build_all(
    state: &mut IfcState,
    candidates: Vec<Candidate>,
) -> Result<Vec<(usize, ProjectedIfc)>, LayoutError> {
    let built = |idx: usize, result: Result<ProjectedIfc, IfcError>| {
        result
            .map(|projected| (idx, projected))
            .map_err(|error| build_error(idx, error))
    };
    if !state.parallel_build || candidates.len() < state.parallel_threshold {
        state.last_build = Some(IfcBuildMode::Sequential);
        return candidates
            .into_iter()
            .map(|candidate| {
                built(
                    candidate.idx,
                    candidate
                        .projected
                        .build(&mut state.layout_cx, &state.fonts),
                )
            })
            .collect();
    }
    state.last_build = Some(IfcBuildMode::Parallel);
    let fonts = &state.fonts;
    // Collected in order, so the reported error is the first in document
    // order whichever thread met it first.
    candidates
        .into_par_iter()
        .map_init(LayoutContext::new, |cx, candidate| {
            (candidate.idx, candidate.projected.build(cx, fonts))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(idx, result)| built(idx, result))
        .collect()
}

/// Store each shaped paragraph on its root and mark the root's subtree.
fn write_roots(doc: &mut Document, built: Vec<(usize, ProjectedIfc)>) {
    for (idx, projected) in built {
        // Boxes are laid out and painted as nodes of their own, so neither
        // they nor their content belong to the paragraph's subtree.
        let boxes: Vec<usize> = projected.boxes.iter().map(|b| b.node).collect();
        doc.nodes[idx].flags.insert(NodeFlags::IS_IFC_ROOT);
        doc.nodes[idx].ifc = Some(Box::new(IfcRoot::new(projected)));
        let mut stack = doc.nodes[idx].children.clone();
        while let Some(id) = stack.pop() {
            if boxes.contains(&id) {
                continue;
            }
            doc.nodes[id].flags.insert(NodeFlags::IN_IFC_SUBTREE);
            stack.extend(doc.nodes[id].children.iter().copied());
        }
    }
}

#[cfg(test)]
mod tests;
