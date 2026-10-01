//! Choose the blocks laid out by the shodo inline engine.

use super::boxes::{IfcBox, IfcBoxKind};
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
use raikiri_style::property::{BackgroundImage, ColumnCountValue, DisplayValue, FloatValue};
use raikiri_style::property::{
    PositionValue, TextDecorationLine, TextTransform, VerticalAlign, VisualBox, WordSpaceTransform,
};
use raikiri_traits::NodeKind;
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

/// Whether `idx` is a box that lays its own inline content out in lines: a
/// block container (`block`, `flow-root`, `inline-block`, `list-item`) or a
/// blockified inline flex or grid item. Flex, grid and table boxes lay their
/// children out by algorithms of their own, and a multicol container is
/// refused by the caller.
pub(crate) fn can_be_ifc_root(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    // Replaced elements, form controls and inline SVG lay their content out
    // by other means: text inside them (an SVG `<title>`, a button label,
    // fallback content) is not a paragraph of theirs.
    let node = &doc.nodes[idx];
    let tag = node.tag_name().unwrap_or("");
    if node.is_inline_svg_content()
        || node.is_inline_svg_root()
        || super::projection::ATOMIC_TAGS.contains(&tag)
        || super::projection::UNSUPPORTED_REPLACED_TAGS.contains(&tag)
    {
        return false;
    }
    generates_own_box(doc, cascade, idx)
        && matches!(
            cascade.computed[idx].display,
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::InlineBlock
                | DisplayValue::ListItem
                | DisplayValue::Inline
        )
}

fn is_horizontal(cascade: &CascadeResult, idx: usize) -> bool {
    cascade.computed[idx].cssom_writing_mode == raikiri_style::property::WritingMode::HorizontalTb
}

fn is_multicol(cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    !matches!(cv.column_count, ColumnCountValue::Auto)
        || !matches!(cv.column_width, ComputedColumnWidth::Auto)
}

/// Whether a `text-transform` value includes `full-width`. shodo's mapping
/// covers fewer characters than the parley path, so such text is not handed
/// to the inline engine.
fn has_full_width_transform(value: TextTransform) -> bool {
    matches!(
        value,
        TextTransform::FullWidth
            | TextTransform::CapitalizeFullWidth
            | TextTransform::UppercaseFullWidth
            | TextTransform::LowercaseFullWidth
            | TextTransform::FullWidthFullSizeKana
            | TextTransform::CapitalizeFullWidthFullSizeKana
            | TextTransform::UppercaseFullWidthFullSizeKana
            | TextTransform::LowercaseFullWidthFullSizeKana
    )
}

/// Whether the painter can draw the text of an element (the root or a
/// descendant). Emphasis marks are drawn by neither path.
fn is_paintable_element(cascade: &CascadeResult, id: usize) -> bool {
    let cv = &cascade.computed[id];
    cv.word_space_transform == WordSpaceTransform::None
        && cv.background_clip != VisualBox::Text
        && !has_full_width_transform(cv.text_transform)
}

/// Whether the painter can draw an inline element of the paragraph with its
/// box. A relative offset in lengths moves the element and its content
/// together; one that needs the containing block (a percentage or calc()) or
/// a stacking context (`z-index`) is not modelled, and an opacity group wraps
/// the element (the walk pushes a layer per element), which the lines do not
/// carry. A background image would have to be laid out across the pieces of
/// the element on its lines, which the painter does not do.
fn is_paintable_descendant(cascade: &CascadeResult, id: usize) -> bool {
    let cv = &cascade.computed[id];
    is_paintable_element(cascade, id)
        && style::relative_offset(cv).is_some()
        && cv.opacity >= 1.0
        && matches!(cv.background_image, BackgroundImage::None)
}

/// Every element and text of the paragraph rooted at `idx` can be drawn.
fn paragraph_is_paintable(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    // A fixed box is sized against its nearest positioned ancestor by taffy,
    // which can be zero wide; breaking lines at that width wraps every word.
    // The parley path shapes at the page width beforehand and hides the error.
    if cascade.computed[idx].position == PositionValue::Fixed {
        return false;
    }
    if !is_paintable_element(cascade, idx) {
        return false;
    }
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element {
            // A box is painted on its own, so its content does not reach the
            // paragraph painter.
            if box_kind(cascade, doc, id).is_some() {
                continue;
            }
            if !is_paintable_descendant(cascade, id) {
                return false;
            }
            stack.extend(node.children.iter().copied());
        }
    }
    true
}

/// Whether a float next to the paragraph, or next to any of its ancestors,
/// can reach its lines. The paragraph's own floats are placed against the
/// lines they are anchored in; an outer float can hold them lower than that
/// line (a float is never placed above an earlier one), which the line layout
/// does not see.
fn has_float_beside(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let mut node = idx;
    while let Some(parent) = doc.parent_of(node) {
        let floats_beside = doc.nodes[parent].children.iter().any(|&sibling| {
            sibling != node
                && doc.nodes[sibling].is_in_document()
                && doc.nodes[sibling].kind() == NodeKind::Element
                && cascade.computed[sibling].float != FloatValue::None
        });
        if floats_beside {
            return true;
        }
        node = parent;
    }
    false
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

/// Whether a decoration line meets an inline whose `vertical-align` is
/// measured from the line box or the parent's content area rather than from
/// the baseline (`top`, `bottom`, `middle`, `text-top`, `text-bottom`). The
/// painter places a decoration at the baseline of the element that declares
/// it; for those values neither a parley-path counterpart nor a hand-computed
/// position has been checked, so such paragraphs stay on the parley path.
fn decoration_meets_a_line_relative_inline(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
) -> bool {
    let decorated =
        |id: usize| cascade.computed[id].text_decoration_line != TextDecorationLine::NONE;
    let mut any_decoration = false;
    let mut current = Some(idx);
    while let Some(id) = current {
        any_decoration |= decorated(id);
        current = doc.parent_of(id);
    }
    let mut line_relative = false;
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document()
            || node.kind() != NodeKind::Element
            || box_kind(cascade, doc, id).is_some()
        {
            continue;
        }
        any_decoration |= decorated(id);
        line_relative |= matches!(
            cascade.computed[id].vertical_align,
            VerticalAlign::Top
                | VerticalAlign::Bottom
                | VerticalAlign::Middle
                | VerticalAlign::TextTop
                | VerticalAlign::TextBottom
        );
        stack.extend(node.children.iter().copied());
    }
    any_decoration && line_relative
}

/// Whether a block child of the paragraph would take a decoration from the
/// root or one of its ancestors. A decoration propagates into in-flow block
/// children (CSS Text Decoration 3, 2.1), and the lines do not carry it there.
fn has_block_child_under_a_decoration(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    boxes: &[IfcBox],
) -> bool {
    if !boxes.iter().any(|b| b.kind == IfcBoxKind::Block) {
        return false;
    }
    let mut current = Some(idx);
    while let Some(id) = current {
        if cascade.computed[id].text_decoration_line != TextDecorationLine::NONE {
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

/// Clear every IFC mark, then mark the eligible roots and their subtrees.
///
/// Runs in three steps: a walk over the document that decides which blocks
/// are paragraph roots and fills their builders, the shaping of those
/// builders, which touches neither the document nor the cascade, and the
/// writing of the shaped paragraphs and marks back, in document order.
pub(crate) fn assign_ifc_roots(doc: &mut Document, cascade: &CascadeResult) {
    for node in &mut doc.nodes {
        node.flags
            .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
        node.ifc = None;
    }
    // Take the engine state out so the walk can borrow the document.
    let Some(mut state) = doc.ifc.take() else {
        return;
    };
    // The roots are rebuilt on every pass, so a cached layout would skip the
    // measure callback that fills their lines.
    doc.layout_dirty = true;
    let mut candidates = collect_candidates(doc, cascade, &state);
    candidates.extend(collect_text_candidates(doc, cascade, &state));
    candidates.sort_by_key(|candidate| candidate.idx);
    let built = build_all(&mut state, candidates);
    write_roots(doc, built);
    doc.ifc = Some(state);
}

/// Walk the document in index order and return the eligible paragraphs, in
/// ascending index order, with their builders.
///
/// The content of an accepted paragraph is taken here, before it is shaped:
/// that content is text and inline elements, none of which can be a root,
/// while the inside of its boxes is not taken and may hold roots of its own.
/// So a paragraph that later fails to shape changes no other paragraph's
/// eligibility.
fn collect_candidates(doc: &Document, cascade: &CascadeResult, state: &IfcState) -> Vec<Candidate> {
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
            || !is_horizontal(cascade, idx)
        {
            continue;
        }
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
        if blocked
            || !has_inline_content(doc, cascade, idx, &state.fonts)
            || !paragraph_is_paintable(doc, cascade, idx)
            || inside_unsized_fixed_box(doc, cascade, idx)
            || decoration_meets_a_line_relative_inline(doc, cascade, idx)
        {
            continue;
        }
        let Ok(projected) = project_ifc_builder(doc, cascade, idx, &state.fonts, &state.limits)
        else {
            continue;
        };
        if has_block_child_under_a_decoration(doc, cascade, idx, &projected.boxes) {
            continue;
        }
        if projected
            .boxes
            .iter()
            .any(|b| box_has_a_page_break(doc, cascade, b.node))
        {
            continue;
        }
        let has_own_floats = projected.boxes.iter().any(|b| b.kind == IfcBoxKind::Float);
        if has_own_floats && has_float_beside(doc, cascade, idx) {
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
    candidates
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
) -> Vec<Candidate> {
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
        if blocked
            || !is_horizontal(cascade, idx)
            || !is_paintable_element(cascade, parent)
            || inside_unsized_fixed_box(doc, cascade, idx)
        {
            continue;
        }
        let Ok(projected) =
            project_ifc_text_builder(doc, cascade, idx, &state.fonts, &state.limits)
        else {
            continue;
        };
        candidates.push(Candidate { idx, projected });
    }
    candidates
}

/// Shape every candidate. A candidate that fails to shape is dropped and
/// stays on the parley path.
///
/// With enough candidates, and when the document allowed it, the candidates
/// are shaped on several threads: each worker owns a layout context (it is
/// `Send` but not `Sync`) and the font collection is shared by reference.
/// The result keeps the candidates' order either way.
fn build_all(state: &mut IfcState, candidates: Vec<Candidate>) -> Vec<(usize, ProjectedIfc)> {
    if !state.parallel_build || candidates.len() < state.parallel_threshold {
        state.last_build = Some(IfcBuildMode::Sequential);
        return candidates
            .into_iter()
            .filter_map(|candidate| {
                candidate
                    .projected
                    .build(&mut state.layout_cx, &state.fonts)
                    .ok()
                    .map(|projected| (candidate.idx, projected))
            })
            .collect();
    }
    state.last_build = Some(IfcBuildMode::Parallel);
    let fonts = &state.fonts;
    candidates
        .into_par_iter()
        .map_init(LayoutContext::new, |cx, candidate| {
            candidate
                .projected
                .build(cx, fonts)
                .ok()
                .map(|projected| (candidate.idx, projected))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
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
