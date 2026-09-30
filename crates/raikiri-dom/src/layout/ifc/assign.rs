//! Choose the blocks laid out by the shodo inline engine.

use super::boxes::{IfcBox, IfcBoxKind};
use super::projection::{box_kind, project_ifc};
use super::root::IfcRoot;
use super::style;
use crate::Document;
use crate::node::NodeFlags;
use raikiri_style::CascadeResult;
use raikiri_style::ComputedColumnWidth;
use raikiri_style::ComputedLengthPercentageOrAuto;
use raikiri_style::property::{ColumnCountValue, DisplayValue, FloatValue};
use raikiri_style::property::{
    PositionValue, TextDecorationLine, TextTransform, VerticalAlign, VisualBox, WordSpaceTransform,
};
use raikiri_traits::NodeKind;

fn is_block_container(display: DisplayValue) -> bool {
    matches!(display, DisplayValue::Block | DisplayValue::FlowRoot)
}

fn is_horizontal(cascade: &CascadeResult, idx: usize) -> bool {
    cascade.computed[idx].cssom_writing_mode == raikiri_style::property::WritingMode::HorizontalTb
}

fn is_multicol(cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    !matches!(cv.column_count, ColumnCountValue::Auto)
        || !matches!(cv.column_width, ComputedColumnWidth::Auto)
}

/// Every in-flow child of `parent` other than `idx` is block-level, so the
/// parent does not lay `idx` out inside an inline line box.
fn parent_holds_only_blocks(doc: &Document, cascade: &CascadeResult, parent: usize) -> bool {
    doc.nodes[parent].children.iter().all(|&child| {
        let node = &doc.nodes[child];
        if !node.is_in_document() {
            return true;
        }
        match node.kind() {
            NodeKind::Text => node
                .text_content()
                .is_none_or(|text| text.chars().all(|c| c.is_ascii_whitespace())),
            NodeKind::Element => {
                // Only an inline-level sibling puts the paragraph inside an
                // inline formatting context of its parent; a block-level box
                // of any inner display type does not.
                !matches!(
                    cascade.computed[child].display,
                    DisplayValue::Inline
                        | DisplayValue::InlineBlock
                        | DisplayValue::InlineFlex
                        | DisplayValue::InlineGrid
                        | DisplayValue::InlineTable
                        | DisplayValue::Contents
                )
            }
            _ => true,
        }
    })
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

/// Descendants are drawn without their own boxes: a relative offset moves the
/// element on the parley path (taffy applies it) and an opacity group wraps it
/// (the walk pushes a layer per element), neither of which the lines carry. A
/// relative box that moves nothing and makes no stacking context is drawn in
/// place.
fn is_paintable_descendant(cascade: &CascadeResult, id: usize) -> bool {
    let cv = &cascade.computed[id];
    is_paintable_element(cascade, id)
        && (cv.position != PositionValue::Relative || style::is_inert_relative(cv))
        && cv.opacity >= 1.0
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

/// Whether the paragraph itself holds text other than white space; text inside
/// its boxes (floats, atomic inlines, blocks) does not count.
fn has_visible_text(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
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
            NodeKind::Element => {
                if box_kind(cascade, doc, id).is_none() {
                    stack.extend(node.children.iter().copied());
                }
            }
            _ => {}
        }
    }
    false
}

/// Clear every IFC mark, then mark the eligible roots and their subtrees.
pub(crate) fn assign_ifc_roots(doc: &mut Document, cascade: &CascadeResult) {
    for node in &mut doc.nodes {
        node.flags
            .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
        node.ifc = None;
    }
    // Take the engine state out so `project_ifc` can borrow the document.
    let Some(mut state) = doc.ifc.take() else {
        return;
    };
    // The roots are rebuilt on every pass, so a cached layout would skip the
    // measure callback that fills their lines.
    doc.layout_dirty = true;
    let mut taken = vec![false; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        let node = &doc.nodes[idx];
        if node.kind() != NodeKind::Element
            || !node.is_in_document()
            || cascade.computed[idx].display != DisplayValue::Block
            || !is_horizontal(cascade, idx)
            || taken[idx]
        {
            continue;
        }
        let Some(parent) = doc.parent_of(idx) else {
            continue;
        };
        if doc.nodes[parent].kind() != NodeKind::Element
            || !is_block_container(cascade.computed[parent].display)
            || !parent_holds_only_blocks(doc, cascade, parent)
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
            || !has_visible_text(doc, cascade, idx)
            || !paragraph_is_paintable(doc, cascade, idx)
            || inside_unsized_fixed_box(doc, cascade, idx)
            || decoration_meets_a_line_relative_inline(doc, cascade, idx)
        {
            continue;
        }
        let Ok(projected) = project_ifc(
            doc,
            cascade,
            idx,
            &mut state.layout_cx,
            &state.fonts,
            &state.limits,
        ) else {
            continue;
        };
        if has_block_child_under_a_decoration(doc, cascade, idx, &projected.boxes) {
            continue;
        }
        let has_own_floats = projected.boxes.iter().any(|b| b.kind == IfcBoxKind::Float);
        if has_own_floats && has_float_beside(doc, cascade, idx) {
            continue;
        }
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
            taken[id] = true;
            stack.extend(doc.nodes[id].children.iter().copied());
        }
        taken[idx] = true;
    }
    doc.ifc = Some(state);
}

#[cfg(test)]
mod tests;
