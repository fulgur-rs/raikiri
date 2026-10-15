//! Choose the blocks laid out by the shodo inline engine.

use super::boxes::IfcBoxKind;
use super::error::IfcError;
use super::projection::{
    GeneratedCounters, ProjectedBuilder, ProjectedIfc, box_kind, has_in_flow_generated_text,
    project_anonymous_cell_builder, project_ifc_builder_with, project_ifc_text_builder,
};
use super::root::{IfcBuildMode, IfcRoot, IfcState, RetainedKey, RetainedParagraphs};
use super::style;
use crate::Document;
use crate::layout::{BoxParentScope, box_parent};
use crate::node::NodeFlags;
use raikiri_style::property::{
    ColumnCountValue, DisplayValue, PositionValue, WhiteSpaceCollapse, WritingMode,
};
use raikiri_style::{CascadeResult, ComputedColumnWidth};
use raikiri_traits::{LayoutError, NodeKind};
use rayon::prelude::*;
use shodo::LayoutContext;
use std::collections::HashSet;

/// Whether `idx` generates a box of its own: any display other than `inline`,
/// `contents` and `none`, an absolutely positioned or fixed inline element
/// (blockified, CSS 2.1 9.7), a replaced element or form control (an
/// atomic inline), and an inline element that a flex or grid
/// container blockifies as its item (CSS Display 3, 2.7), and the body layout
/// starts at, whatever its display. The cascade leaves such elements
/// `inline`; only the taffy bridge maps them to blocks.
pub(crate) fn generates_own_box(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    // Ruby boxes are inline boxes here, as in the rest of the block layout.
    let display = if cv.display.is_inline_flow() {
        DisplayValue::Inline
    } else {
        cv.display
    };
    match display {
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
        DisplayValue::Inline => box_parent(doc, cascade, idx, BoxParentScope::LayoutRoot)
            .is_some_and(|parent| cascade.computed[parent].display.is_flex_or_grid_container()),
        _ => true,
    }
}

/// Whether `idx` is the body layout starts at.
pub(crate) fn is_layout_root(doc: &Document, idx: usize) -> bool {
    // The tag check spares the document walk for every other element.
    crate::layout::is_body_element(doc, idx) && crate::layout::find_body(doc) == Some(idx)
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

/// Whether `idx` is a box that lays its own inline content out in lines: a
/// block container (`block`, `flow-root`, `inline-block`, `list-item`, a
/// table cell), or a blockified inline flex or grid item. Flex and grid boxes
/// and tables lay
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
        display
            if display.is_inline_flow_box()
                || matches!(
                    display,
                    DisplayValue::Block
                        | DisplayValue::FlowRoot
                        | DisplayValue::ListItem
                        | DisplayValue::TableCell
                        | DisplayValue::TableCaption
                        | DisplayValue::BlockRuby
                ) =>
        {
            true
        }
        // A row or row group outside any table is laid out as a block (no
        // anonymous table wraps it, see the taffy bridge), so it lays its own
        // inline content out. Inside a table that content belongs to
        // anonymous cells instead.
        DisplayValue::TableRow
        | DisplayValue::TableRowGroup
        | DisplayValue::TableHeaderGroup
        | DisplayValue::TableFooterGroup => !doc.table_objects.lays_out_children(idx),
        // The body is a block whatever its display (see `generates_own_box`).
        DisplayValue::Contents => is_layout_root(doc, idx),
        _ => false,
    }
}

/// Whether a block qualifies for the IFC path. Inline content that makes a
/// line is text other than white space (generated text included), an atomic
/// inline, a `<br>`, or an inline element with an inline-side margin, border,
/// or padding (CSS 2.1, 9.4.2). A vertical block child with inline content can
/// also qualify its ancestor. Once selected, a vertical IFC root lays out its
/// ordinary block children on the physical block axis, including empty ones.
/// Page roots and multicol containers remain on their existing layout paths.
fn needs_ifc_layout(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    fonts: &shodo::font::FontCollection,
    counters: &GeneratedCounters,
) -> Result<bool, IfcError> {
    if has_in_flow_generated_text(doc, cascade, idx, counters)? {
        return Ok(true);
    }
    let cv = &cascade.computed[idx];
    let vertical_block_children = matches!(
        cv.cssom_writing_mode,
        WritingMode::VerticalRl | WritingMode::VerticalLr
    ) && !is_layout_root(doc, idx)
        && matches!(cv.column_count, ColumnCountValue::Auto)
        && matches!(cv.column_width, ComputedColumnWidth::Auto);
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                if text_makes_a_line(doc, cascade, id) {
                    return Ok(true);
                }
            }
            NodeKind::Element => match box_kind(cascade, doc, id) {
                Some(IfcBoxKind::Atomic) => return Ok(true),
                Some(IfcBoxKind::Block) if vertical_block_children => {
                    if has_in_flow_generated_text(doc, cascade, id, counters)? {
                        return Ok(true);
                    }
                    stack.extend(node.children.iter().copied());
                }
                Some(_) => {}
                None => {
                    let cv = &cascade.computed[id];
                    if cv.display == DisplayValue::None
                        || node.is_non_rendered_html_element()
                        || node.is_inline_svg_content()
                    {
                        continue;
                    }
                    if has_in_flow_generated_text(doc, cascade, id, counters)? {
                        return Ok(true);
                    }
                    if cv.display.is_inline_flow() {
                        if node.tag_name() == Some("br") {
                            return Ok(true);
                        }
                        // An edge the engine cannot map yet keeps the
                        // paragraph out at projection; counting it here only
                        // lets projection decide.
                        let edged = style::inline_edges(cv, id, fonts).map_or(true, |e| {
                            e.inline_start_total() + e.inline_end_total() != 0.0
                        });
                        if edged {
                            return Ok(true);
                        }
                    }
                    stack.extend(node.children.iter().copied());
                }
            },
            _ => {}
        }
    }
    Ok(false)
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
/// refuses. [`LayoutError::Internal`] if a table exceeds the native grid's
/// supported row or column bounds. Nothing is marked then.
pub(crate) fn assign_ifc_roots(
    doc: &mut Document,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    for node in &mut doc.nodes {
        node.flags
            .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
        node.ifc = None;
    }
    crate::layout::table::anonymous::prepare(doc, cascade)?;
    // Take the engine state out so the walk can borrow the document.
    let Some(mut state) = doc.ifc.take() else {
        return Ok(());
    };
    // The roots are rebuilt on every pass, so a cached layout would skip the
    // measure callback that fills their lines.
    doc.layout_dirty = true;
    let key = RetainedKey {
        element_generation: cascade.element_generation(),
        node_count: doc.nodes.len(),
        continuation_break: doc.continuation_break,
        anonymous_cells: doc.table_objects.cells.len(),
    };
    if state
        .retained
        .as_ref()
        .is_some_and(|retained| retained.key == key)
    {
        // A pass over the same content and element cascade, such as another
        // pass at other page sizes, builds the same paragraphs: only their
        // lines are broken again. Paragraphs given to a document that does
        // not keep them serve this pass only.
        let roots = if state.retain_paragraphs {
            state
                .retained
                .as_ref()
                .map(|retained| retained.roots.clone())
        } else {
            state.retained.take().map(|retained| retained.roots)
        }
        .unwrap_or_default();
        state.last_build = Some(IfcBuildMode::Retained);
        doc.ifc = Some(state);
        write_roots(doc, roots);
        return check_text_in_paragraphs(doc, cascade);
    }
    state.retained = None;
    // One set of counters serves the whole pass: the document does not
    // change while its paragraphs are collected.
    let counters = GeneratedCounters::default();
    let built = collect_candidates(doc, cascade, &state, &counters).and_then(|mut candidates| {
        candidates.extend(collect_text_candidates(doc, cascade, &state)?);
        for (index, cell) in doc.table_objects.cells.iter().enumerate() {
            let projected = project_anonymous_cell_builder(
                doc,
                cascade,
                cell.owner,
                &cell.content,
                &state.fonts,
                &state.limits,
                &counters,
            )
            .map_err(|error| projection_error(cell.owner, error))?;
            candidates.push(Candidate {
                idx: doc.table_objects.arena_len + index,
                projected,
            });
        }
        candidates.sort_by_key(|candidate| candidate.idx);
        build_all(&mut state, candidates)
    });
    let roots = built.map(|built| {
        built
            .into_iter()
            .map(|(idx, projected)| (idx, IfcRoot::new(projected)))
            .collect::<Vec<_>>()
    });
    if state.retain_paragraphs
        && let Ok(roots) = &roots
    {
        state.retained = Some(RetainedParagraphs {
            key,
            roots: roots.clone(),
        });
    }
    doc.ifc = Some(state);
    write_roots(doc, roots?);
    check_text_in_paragraphs(doc, cascade)
}

/// Refuse a document with text that no paragraph lays out.
fn check_text_in_paragraphs(doc: &Document, cascade: &CascadeResult) -> Result<(), LayoutError> {
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
    counters: &GeneratedCounters,
) -> Result<Vec<Candidate>, LayoutError> {
    let mut candidates = Vec::new();
    // Elements inside a `display: none` subtree generate no boxes, so their
    // paragraphs would be shaped for nothing.
    let mut taken = inside_hidden_subtrees(doc, cascade);
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
        {
            continue;
        }
        if !needs_ifc_layout(doc, cascade, idx, &state.fonts, counters)
            .map_err(|error| projection_error(idx, error))?
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
            counters,
        ) {
            Ok(projected) => projected,
            Err(error) => return Err(projection_error(idx, error)),
        };
        let projected_box_nodes = projected
            .meta
            .boxes
            .iter()
            .map(|box_| {
                #[cfg(test)]
                crate::layout::test_support::IFC_CANDIDATE_BOX_MEMBERSHIP_STEPS
                    .with(|steps| steps.set(steps.get() + 1));
                box_.node
            })
            .collect::<HashSet<_>>();
        let mut stack = doc.nodes[idx].children.clone();
        while let Some(id) = stack.pop() {
            #[cfg(test)]
            crate::layout::test_support::IFC_CANDIDATE_BOX_MEMBERSHIP_STEPS
                .with(|steps| steps.set(steps.get() + 1));
            if projected_box_nodes.contains(&id) {
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

/// Which nodes are descendants of an element with `display: none`.
fn inside_hidden_subtrees(doc: &Document, cascade: &CascadeResult) -> Vec<bool> {
    let mut inside = vec![false; doc.nodes.len()];
    let mut stack = vec![(doc.root, false)];
    while let Some((id, hidden)) = stack.pop() {
        inside[id] = hidden;
        let hides = doc.nodes[id].kind() == NodeKind::Element
            && cascade
                .computed
                .get(id)
                .is_some_and(|computed| computed.display == DisplayValue::None);
        stack.extend(
            doc.nodes[id]
                .children
                .iter()
                .map(|&child| (child, hidden || hides)),
        );
    }
    inside
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

/// Reserve marker advances only on the legacy ruby/multicol projection path.
/// Authored padding, including font-resolved `ch` and percentages, is preserved.
pub(crate) fn prepare_legacy_inside_markers_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
) -> Result<(), LayoutError> {
    doc.legacy_inside_marker_advances.clear();
    let counters = GeneratedCounters::default();
    for idx in 0..doc.nodes.len() {
        if !crate::generated_content::inside_marker_in_flow(cascade, idx)
            || doc.nodes[idx].is_ifc_root()
            || !is_ruby_multicol_flex_projection(doc, cascade, idx)
        {
            continue;
        }
        let snapshots = counters
            .get(doc, cascade)
            .map_err(|error| projection_error(idx, IfcError::CounterSnapshots(error)))?;
        let Some((cv, text)) = crate::generated_content::markers::marker_render_info_with_snapshots(
            doc, cascade, idx, snapshots,
        ) else {
            continue;
        };
        let image_width = doc.list_marker_image_size(idx).map(|size| size.width);
        // Match the legacy painter's normal face, first family and usable size.
        let family = cv
            .font_family
            .first()
            .cloned()
            .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
        let font_size = cv.font_size.px();
        let style = crate::StandaloneStyle {
            families: vec![family],
            font_size: if font_size.is_finite() && font_size > 0.0 {
                font_size
            } else {
                16.0
            },
            ..crate::StandaloneStyle::default()
        };
        let text = if image_width.is_some() { " " } else { &text };
        let advance = image_width.unwrap_or(0.0)
            + doc
                .shape_standalone_text(text, &style, None, crate::StandaloneAlign::Start)
                .map_or(0.0, |shaped| shaped.advance());
        if advance <= 0.0 {
            continue;
        }
        let padding = doc.nodes[idx].style.padding.left.into_raw();
        let deferred = if padding.is_calc() {
            Some(
                doc.calc_values
                    .iter()
                    .find(|value| {
                        std::ptr::eq(
                            std::sync::Arc::as_ptr(value).cast::<()>(),
                            padding.calc_value(),
                        )
                    })
                    .ok_or_else(|| LayoutError::Internal {
                        message: "legacy marker padding calc handle is not registered".to_owned(),
                    })?
                    .value,
            )
        } else if padding.tag() == taffy::CompactLength::PERCENT_TAG {
            Some(raikiri_style::property::CalcLengthPercentage {
                percent: padding.value() * 100.0,
                px: 0.0,
            })
        } else {
            None
        };
        doc.nodes[idx].style.padding.left = if let Some(base) = deferred {
            let pointer = crate::taffy_impl::push_calc(
                &mut doc.calc_values,
                raikiri_style::property::CalcLengthPercentage {
                    percent: base.percent,
                    px: base.px + advance,
                },
                true,
            );
            taffy::LengthPercentage::calc(pointer)
        } else {
            taffy::LengthPercentage::length(padding.value() + advance)
        };
        doc.legacy_inside_marker_advances.insert(idx, advance);
    }
    Ok(())
}

/// Whether the text node `id` puts something on a line: a character other
/// than white space, or white space its `white-space-collapse` value keeps
/// (CSS Text 3, 4.1). Collapsible white space alone is removed, and a
/// whitespace-only run between blocks generates no anonymous block box (CSS
/// 2.1, 9.2.1.1 and 16.6.1). Preserved white space forms a line box that is
/// not zero-height (CSS 2.1, 9.4.2): any white space under `preserve`,
/// `preserve-spaces` and `break-spaces`, and a segment break under
/// `preserve-breaks`.
pub(super) fn text_makes_a_line(doc: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let Some(text) = doc.nodes[id].text_content() else {
        return false; // cov:ignore: callers pass text nodes, which always hold character data.
    };
    if text.chars().any(|c| !is_css_whitespace(c)) {
        return true;
    }
    if text.is_empty() {
        return false;
    }
    match cascade.computed[id].effective_white_space_collapse {
        WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::Discard => false,
        WhiteSpaceCollapse::PreserveBreaks => text.contains(['\n', '\r']),
        // `preserve`, `preserve-spaces` and `break-spaces`.
        _ => true,
    }
}

pub(super) fn is_css_whitespace(c: char) -> bool {
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
    let Some(parent) = box_parent(doc, cascade, text, BoxParentScope::LayoutRoot) else {
        return false;
    };
    cascade.computed[parent].display.is_flex_or_grid_container()
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
fn write_roots(doc: &mut Document, built: Vec<(usize, IfcRoot)>) {
    for (idx, paragraph) in built {
        // Boxes are laid out and painted as nodes of their own, so neither
        // they nor their content belong to the paragraph's subtree.
        let boxes = paragraph
            .boxes
            .iter()
            .map(|box_| {
                #[cfg(test)]
                crate::layout::test_support::IFC_ROOT_BOX_MEMBERSHIP_STEPS
                    .with(|steps| steps.set(steps.get() + 1));
                box_.node
            })
            .collect::<HashSet<_>>();
        let root = doc.table_layout_node_mut(idx);
        root.flags.insert(NodeFlags::IS_IFC_ROOT);
        root.ifc = Some(Box::new(paragraph));
        let mut stack = root.children.clone();
        while let Some(id) = stack.pop() {
            #[cfg(test)]
            crate::layout::test_support::IFC_ROOT_BOX_MEMBERSHIP_STEPS
                .with(|steps| steps.set(steps.get() + 1));
            if boxes.contains(&id) {
                continue;
            }
            doc.nodes[id].flags.insert(NodeFlags::IN_IFC_SUBTREE);
            if idx >= doc.table_objects.arena_len {
                doc.table_objects.paragraph_owner[id] = Some(idx);
            }
            stack.extend(doc.nodes[id].children.iter().copied());
        }
    }
}

#[cfg(test)]
mod tests;
