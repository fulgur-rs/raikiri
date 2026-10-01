//! Choose the blocks laid out by the shodo inline engine.

use super::boxes::IfcBoxKind;
use super::error::IfcError;
use super::projection::{
    ProjectedBuilder, ProjectedIfc, box_kind, project_ifc_builder, project_ifc_text_builder,
};
use super::root::{IfcBuildMode, IfcRoot, IfcState};
use super::style;
use crate::Document;
use crate::layout::page_pipeline::{page_break_is_forced, selected_page_name};
use crate::node::NodeFlags;
use raikiri_style::CascadeResult;
use raikiri_style::ComputedColumnWidth;
use raikiri_style::ComputedLengthPercentageOrAuto;
use raikiri_style::property::PositionValue;
use raikiri_style::property::{ColumnCountValue, DisplayValue};
use raikiri_traits::{LayoutError, NodeKind};
use rayon::prelude::*;
use shodo::LayoutContext;

/// Whether `idx` generates a box of its own: any display other than `inline`,
/// `contents` and `none`, and an inline element that a flex or grid container
/// blockifies as its item (CSS Display 3, 2.7). The cascade leaves such an
/// item `inline`; only the taffy bridge maps it to a block.
pub(crate) fn generates_own_box(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    match cascade.computed[idx].display {
        DisplayValue::Contents | DisplayValue::None => false,
        DisplayValue::Inline => doc.parent_of(idx).is_some_and(|parent| {
            doc.nodes[parent].kind() == NodeKind::Element
                && matches!(
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

/// Form controls, whose content the painter draws as a control.
const FORM_CONTROL_TAGS: &[&str] = &["input", "button", "select", "textarea"];

/// Whether `idx` is a box that lays its own inline content out in lines: a
/// block container (`block`, `flow-root`, `inline-block`, `list-item`, a
/// table cell), a blockified inline flex or grid item, or a table box holding
/// only inline-level children. Flex and grid boxes and tables with rows lay
/// their children out by algorithms of their own; the table algorithm places a
/// caption only when the table is empty, so a caption is not a root either. A
/// multicol container is refused by the caller.
pub(crate) fn can_be_ifc_root(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    // Images, inline SVG and form controls lay their content out by other
    // means: text inside them (an SVG `<title>`, a button label) is not a
    // paragraph of theirs. The fallback content of the other replaced
    // elements is laid out as ordinary content, as on the parley path.
    let node = &doc.nodes[idx];
    let tag = node.tag_name().unwrap_or("");
    if node.is_inline_svg_content()
        || node.is_inline_svg_root()
        || super::projection::ATOMIC_TAGS.contains(&tag)
        || FORM_CONTROL_TAGS.contains(&tag)
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
        | DisplayValue::TableCell => true,
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

fn is_multicol(cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    !matches!(cv.column_count, ColumnCountValue::Auto)
        || !matches!(cv.column_width, ComputedColumnWidth::Auto)
}

/// Whether the paragraph sits inside a fixed box without an authored width.
/// taffy sizes such a box against its nearest positioned ancestor, which can
/// be zero wide, and the paragraph would wrap at that width.
fn inside_unsized_fixed_box(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let mut current = doc.parent_of(idx);
    while let Some(id) = current {
        let cv = &cascade.computed[id];
        if cv.position == PositionValue::Fixed
            && matches!(cv.width, ComputedLengthPercentageOrAuto::Auto)
        {
            return true;
        }
        current = doc.parent_of(id);
    }
    false
}

/// Whether the paragraph has inline content that makes a line: text other
/// than white space, an atomic inline, a `<br>`, or an inline element with a
/// margin, border or padding on an inline side (CSS 2.1, 9.4.2: a line box
/// without any of these is treated as zero-height). The content of its boxes
/// does not count, and neither do floats and block children alone: they are
/// not inline content, so a block that holds only those is laid out by the
/// block algorithm.
fn has_inline_content(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    fonts: &shodo::font::FontCollection,
) -> bool {
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

/// An element that pagination may break before or after on its own: one
/// with a forced page break or a page name.
fn starts_a_page_of_its_own(cascade: &CascadeResult, id: usize) -> bool {
    let computed = &cascade.computed[id];
    page_break_is_forced(computed.break_before)
        || page_break_is_forced(computed.break_after)
        || selected_page_name(cascade, id).is_some()
}

/// The box `idx` of a paragraph, or an element inside it, starts a page of
/// its own. Pagination would move it alone, and the lines around the box
/// would not follow. Inline elements of the paragraph are not page break
/// candidates (break-before and break-after apply to block-level boxes), so
/// they are not looked at.
fn box_has_a_page_break(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let mut stack = vec![idx];
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if node.kind() != NodeKind::Element {
            continue;
        }
        if starts_a_page_of_its_own(cascade, id) {
            return true;
        }
        stack.extend(node.children.iter().copied());
    }
    false
}

/// A paragraph the walk accepted, waiting to be shaped.
struct Candidate {
    idx: usize,
    projected: ProjectedBuilder,
}

/// A paragraph the engine does not lay out: an error when the engine is
/// required for every paragraph, otherwise nothing (the paragraph stays on
/// the parley path).
fn refuse(engine_only: bool, node: usize, reason: &'static str) -> Result<(), LayoutError> {
    if engine_only {
        Err(LayoutError::IfcUnsupported { node, reason })
    } else {
        Ok(())
    }
}

/// What a projection or shaping error of the paragraph `root` means for the
/// layout: a limit is always an error (there is nothing to fall back to once
/// the engine is the only path); a refusal is one only in engine-only mode.
fn projection_error(engine_only: bool, root: usize, error: IfcError) -> Result<(), LayoutError> {
    match error {
        IfcError::Limit(limit) => Err(LayoutError::IfcLimitExceeded {
            node: root,
            limit: limit.to_string(),
        }),
        IfcError::Unsupported { node, reason } => refuse(engine_only, node, reason),
        IfcError::InvalidNode(node) => refuse(engine_only, node, "the node is not in the document"),
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
/// refuses when [`Document::inline_formatting_engine_only`] is set. Nothing is
/// marked then.
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
    result
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
    let engine_only = state.engine_only;
    let mut candidates = Vec::new();
    let mut taken = vec![false; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        let node = &doc.nodes[idx];
        // A root is any box that lays its own inline content out, whatever
        // its parent's layout: a block child of another paragraph is a box of
        // that paragraph and the root of its own content. Inline content of a
        // paragraph never qualifies, so `taken` only saves the work.
        if node.kind() != NodeKind::Element
            || !node.is_in_document()
            || taken[idx]
            || !can_be_ifc_root(doc, cascade, idx)
            || !has_inline_content(doc, cascade, idx, &state.fonts)
        {
            continue;
        }
        // From here on the box is a paragraph: anything that keeps it from
        // the engine is a refusal.
        // The root itself counts: a multicol container is laid out by its own
        // dispatch, which would find no children once they are hidden. A
        // paragraph's own content is caught by `taken` above; the inside of
        // its boxes is not taken and may hold paragraphs of its own.
        let mut ancestor = Some(idx);
        let mut blocked = false;
        while let Some(id) = ancestor {
            if is_multicol(cascade, id) {
                blocked = true;
                break;
            }
            ancestor = doc.parent_of(id);
        }
        let refusal = if blocked {
            Some("a paragraph in a multicol container is not laid out")
        } else if cascade.computed[idx].position == PositionValue::Fixed {
            // taffy sizes a fixed box against its nearest positioned
            // ancestor, which can be zero wide; breaking lines at that width
            // wraps every word. The parley path shapes at the page width
            // beforehand and hides the error.
            Some("a fixed root is not laid out")
        } else if inside_unsized_fixed_box(doc, cascade, idx) {
            Some("a paragraph in a fixed box without a width is not laid out")
        } else {
            None
        };
        if let Some(reason) = refusal {
            refuse(engine_only, idx, reason)?;
            continue;
        }
        let projected = match project_ifc_builder(doc, cascade, idx, &state.fonts, &state.limits) {
            Ok(projected) => projected,
            Err(error) => {
                projection_error(engine_only, idx, error)?;
                continue;
            }
        };
        if projected
            .boxes
            .iter()
            .any(|b| box_has_a_page_break(doc, cascade, b.node))
        {
            refuse(
                engine_only,
                idx,
                "a box with a forced page break inside a paragraph is not laid out",
            )?;
            continue;
        }
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

/// Whether `text` is laid out by a flex or grid container as an anonymous
/// item of its own: it is a direct child of the container and holds more than
/// collapsible white space (CSS Flexbox 1, 4; CSS Grid 1, 6). The white-space
/// test matches the item collection of the taffy tree.
fn is_anonymous_item_text(doc: &Document, cascade: &CascadeResult, text: usize) -> bool {
    let node = &doc.nodes[text];
    if node.kind() != NodeKind::Text || !node.is_in_document() {
        return false;
    }
    let Some(parent) = doc.parent_of(text) else {
        return false;
    };
    doc.nodes[parent].kind() == NodeKind::Element
        && matches!(
            cascade.computed[parent].display,
            DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
        )
        && node.text_content().is_some_and(|text| {
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
    let engine_only = state.engine_only;
    let mut candidates = Vec::new();
    for idx in 0..doc.nodes.len() {
        if !is_anonymous_item_text(doc, cascade, idx) {
            continue;
        }
        let Some(parent) = doc.parent_of(idx) else {
            continue;
        };
        let mut ancestor = Some(parent);
        let mut blocked = false;
        while let Some(id) = ancestor {
            if is_multicol(cascade, id) {
                blocked = true;
                break;
            }
            ancestor = doc.parent_of(id);
        }
        let refusal = if blocked {
            Some("a paragraph in a multicol container is not laid out")
        } else if inside_unsized_fixed_box(doc, cascade, idx) {
            Some("a paragraph in a fixed box without a width is not laid out")
        } else {
            None
        };
        if let Some(reason) = refusal {
            refuse(engine_only, idx, reason)?;
            continue;
        }
        match project_ifc_text_builder(doc, cascade, idx, &state.fonts, &state.limits) {
            Ok(projected) => candidates.push(Candidate { idx, projected }),
            Err(error) => projection_error(engine_only, idx, error)?,
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
            .map_err(|error| match error {
                IfcError::Limit(limit) => LayoutError::IfcLimitExceeded {
                    node: idx,
                    limit: limit.to_string(),
                },
                other => LayoutError::Internal {
                    message: other.to_string(),
                },
            })
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
