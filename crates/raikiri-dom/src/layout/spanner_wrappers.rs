//! Column spanners inside block wrappers.
//!
//! A `column-span: all` box spans the columns of the multicol container it
//! sits in, whatever plain blocks lie between them (CSS Multi-column Layout 1,
//! 6.1): the wrappers are split around the spanner, the content before it
//! flows in columns, the spanner crosses them, and the content after it flows
//! in columns again. The column strategies place the children of a container
//! and look for spanners among those children only, so a wrapper holding a
//! spanner would be placed as one unit in one column.
//!
//! A wrapper that has nothing of its own to draw or lay out is the same as no
//! wrapper at all, so it is laid out like a `display: contents` element:
//! its children become the children of the container. A wrapper that draws a
//! background, border or any other effect, or that changes how its children
//! are placed, would need its box split in pieces; it stays as it is.

use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
use raikiri_style::property::WhiteSpaceCollapse;

/// Lets the spanners of multicol containers reach the columns through the
/// wrappers around them, by laying those wrappers out as `display: contents`.
pub(super) fn lift_spanner_wrappers(doc: &mut LayoutSession<'_>, cascade: &CascadeResult) {
    let mut wrappers = Vec::new();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() == NodeKind::Element
            && doc.nodes[idx].column_span_all
            && is_spanner(doc, idx)
        {
            collect_wrappers(doc, cascade, idx, &mut wrappers);
        }
    }
    // A wrapper that holds several spanners is found once for each.
    wrappers.sort_unstable();
    wrappers.dedup();
    for idx in wrappers {
        let direction = super::bridge::direction_for_axes(
            &cascade.computed[idx],
            super::bridge::uses_vertical_axes(cascade, idx),
        );
        lift_into_container(doc, idx, direction);
        let facts = super::box_style_data::recipes::BoxFacts::for_node(&doc.nodes[idx], direction);
        let recipe = super::box_style_data::recipes::spanner(facts);
        doc.document
            .box_style_data
            .publish_recipe(crate::node::arena::BoxKey::Source(idx), recipe);
    }
}

/// Adds to `wrappers` the plain wrappers between the spanner `spanner` and
/// the multicol container it spans, unless something else lies in between or
/// no such container is found.
fn collect_wrappers(
    doc: &Document,
    cascade: &CascadeResult,
    spanner: usize,
    wrappers: &mut Vec<usize>,
) {
    let mut chain = Vec::new();
    let mut ancestor = doc.nodes[spanner].parent;
    while let Some(id) = ancestor {
        let node = &doc.nodes[id];
        if node.kind() != NodeKind::Element {
            return;
        }
        if node.display() == DisplayValue::Contents {
            // It makes no box, so what lies above it holds the spanner.
        } else if node.multicol.is_some_and(|style| style.horizontal)
            && !node.used_writing_mode.is_vertical()
        {
            // The lines that the wrappers hold go in anonymous blocks of the
            // container.
            if chain
                .iter()
                .all(|&wrapper| keeps_line_layout(doc, cascade, wrapper, id))
            {
                wrappers.extend(chain);
            }
            return;
        } else if is_plain_wrapper(doc, cascade, id) {
            chain.push(id);
        } else {
            return;
        }
        ancestor = node.parent;
    }
}

/// Whether the box of `id` has nothing of its own: it is a block in the flow
/// that is sized by its content, takes no space around it, clips nothing,
/// draws nothing and starts no stacking context or containing block, and it
/// does not change how the lines of its content are laid out.
fn is_plain_wrapper(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    doc.layout_node_with_style(id).is_some_and(|node| {
        let zero = LengthPercentageAuto::length(0.0);
        let defaults: crate::LayoutStyle = crate::LayoutStyle::default();
        let style = node.style;
        let cv = &cascade.computed[id];
        node.display() == DisplayValue::Block
            && node.taffy_position() == defaults.position
            && !style.float.is_floated()
            && style.clear == defaults.clear
            && style.overflow == defaults.overflow
            && [
                style.margin.top,
                style.margin.right,
                style.margin.bottom,
                style.margin.left,
            ]
            .into_iter()
            .all(|margin| margin == zero || margin.is_auto())
            && style.padding == Rect::zero()
            && style.border == Rect::zero()
            && style.size == defaults.size
            && style.min_size == defaults.min_size
            && style.max_size == defaults.max_size
            && !node.paints_whole_box_effects
            && !node.paints_background_color
            && !node.claims_absolute_oof
            && !node.claims_fixed_oof
            && !node.has_before_or_after_content
            && [
                raikiri_style::PseudoElem::FirstLine,
                raikiri_style::PseudoElem::FirstLetter,
            ]
            .into_iter()
            .all(|pseudo| {
                !cascade
                    .pseudo
                    .contains_key(&(raikiri_style::StyleNodeId::new(id as u64), pseudo))
            })
            && node.multicol.is_none()
            && !node.column_span_all
            && node.break_before == BreakBetween::Auto
            && node.break_after == BreakBetween::Auto
            && node.break_inside == raikiri_style::property::BreakInside::Auto
            && cv.contain == raikiri_style::property::Contain::NONE
            && cv.isolation == raikiri_style::property::Isolation::Auto
            && cv.z_index == raikiri_style::property::ZIndexValue::Auto
            && !node.used_writing_mode.is_vertical()
            && style.aspect_ratio.is_none()
            && node.tag_name().is_none_or(|tag| {
                !(ATOMIC_TAGS.contains(&tag)
                    || REPLACED_BOX_TAGS.contains(&tag)
                    || matches!(
                        tag,
                        "fieldset" | "legend" | "details" | "summary" | "dialog"
                    ))
            })
            && !crate::paint_rules::lays_out_as_replaced(doc, id)
    })
}

/// Whether the lines that the box of `id` holds directly are laid out as they
/// would be in `container`, the box that holds them once `id` makes no box. A
/// line of text that lies between two blocks of a wrapper goes in an
/// anonymous block of the container, which has the style of a child of the
/// container that no rule matches: whatever else the wrapper's style says
/// about text, fonts, decoration or direction would be lost with its box.
fn keeps_line_layout(doc: &Document, cascade: &CascadeResult, id: usize, container: usize) -> bool {
    let (wrapper, container) = (&cascade.computed[id], &cascade.computed[container]);
    if !holds_lines(doc, cascade, id, container) {
        return true;
    }
    let mut anonymous = ComputedValues::inherit_from(container);
    // Both are blocks, and the margins were checked to take no space.
    anonymous.display = wrapper.display;
    anonymous.margin = wrapper.margin;
    anonymous == *wrapper
}

/// Whether the box of `id` has text that renders, or an inline-level box,
/// among its children, or among those of a `display: contents` child. Document
/// white space (space, tab and line breaks, CSS Text 3, 4.1.1) makes no line
/// between blocks when it collapses both in the wrapper and in `container`,
/// whose anonymous block would take it; any other white space, such as
/// U+00A0, always renders.
fn holds_lines(
    doc: &Document,
    cascade: &CascadeResult,
    id: usize,
    container: &ComputedValues,
) -> bool {
    let drops_white_space = |cv: &ComputedValues| {
        matches!(
            cv.effective_white_space_collapse,
            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard
        )
    };
    doc.nodes[id].children.iter().any(|&child| {
        let node = &doc.nodes[child];
        match node.kind() {
            NodeKind::Text => node.text_content().is_some_and(|text| {
                let dropped = text.chars().all(|c| c.is_ascii_whitespace())
                    && drops_white_space(container)
                    && drops_white_space(&cascade.computed[id]);
                !text.is_empty() && !dropped
            }),
            NodeKind::Element => cascade.computed.get(child).is_some_and(|cv| {
                if cv.display == DisplayValue::Contents {
                    holds_lines(doc, cascade, child, container)
                } else {
                    cv.display.is_inline_level()
                }
            }),
            _ => false,
        }
    })
}

/// Makes the box of `id` a `display: contents` one, as the bridge does for
/// an element that is one.
fn lift_into_container(doc: &mut LayoutSession<'_>, id: usize, direction: taffy::Direction) {
    *doc.layout_node_with_style_mut(id).unwrap().style = taffy::Style {
        direction,
        ..crate::LayoutStyle::default()
    };
    doc.nodes[id].unrounded_layout = taffy::Layout::default();
    doc.nodes[id].multicol_auto_width = true;
    doc.nodes[id].has_logical_min_block_size = false;
    let position = doc.nodes[id].position().clone();
    doc.set_box_properties(id, DisplayValue::Contents, position);
}

#[cfg(test)]
mod tests;
