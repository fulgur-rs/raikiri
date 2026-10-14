//! Laying out only the pages after those already delivered.
//!
//! Once pages are delivered, a later layout of the growing document does not
//! need their content: it resumes where the first undelivered page begins,
//! as a continuation does for pages of another width, and lays the elements
//! wholly before that point out as `display: none`. Counters still count
//! those elements, and the margin boxes of every page, delivered or not,
//! keep their page context. The relayout checks that it places the page
//! start where the earlier layout did; when it does not, the caller lays
//! the whole document out instead.
//!
//! Resuming leaves the content before the page start out of the layout, so
//! it is used only where nothing after that point depends on how that
//! content was laid out:
//!
//! - every ancestor of the page start is a block container in the normal
//!   flow whose height follows its content, so that the content before the
//!   start sits in preceding sibling blocks and leaving it out moves only
//!   where those ancestors begin;
//! - the body does not reach the page of the start, as it spans the page
//!   area from the top of the first page;
//! - the page start is the top of a line or a box, not a line straddling
//!   the page's top edge, so that the relayout can begin its page there;
//! - no content of a page from the start on belongs to those siblings, as
//!   the tail of a float or of an absolutely positioned box would;
//! - the document has no running elements and no `target-counter()`,
//!   `target-counters()` or `target-text()`, which look up the pages of
//!   earlier content;
//! - no absolutely positioned element arrived after the first delivery,
//!   whose pages are checked against the delivered ones, unless it is
//!   already left out for landing on a delivered page.

use raikiri_dom::Document;
use raikiri_dom::generated_content::generated_origin;
use raikiri_style::computed::ComputedValues;
use raikiri_style::property::{ContentComponent, DisplayValue, FloatValue, PositionValue};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto};
use raikiri_traits::NodeKind;

use crate::HtmlDocument;
use crate::document_layout::DocumentLayout;
use crate::render::ResumePoint;

/// How far, in px, the page start's own offset may lie from the offset a
/// relayout gives it, and a page's width from the width it was laid out at.
const OFFSET_TOLERANCE: f32 = 0.01;

/// Where a layout of `document` can resume to lay out page `page` and the
/// pages after it, from `laid_out`, a layout of `document`'s start whose
/// page `page` begins where it will in every later layout. `names` holds
/// the names of the pages before `page`. Returns `None` when the page start
/// cannot be named or resuming there could change the pages.
pub(super) fn resume_point(
    document: &HtmlDocument,
    laid_out: &DocumentLayout,
    page: u32,
    names: &[Option<String>],
) -> Option<ResumePoint> {
    let out = &laid_out.out;
    let slice = out.slices.get(page as usize)?;
    if page == 0 || names.len() != page as usize {
        return None;
    }
    // A relayout lays the content before its first page out on a page it
    // drops, so on that first page the boxes begun before it look as if they
    // began there: its start is the one the relayout resumed at, not one it
    // can name.
    let slot = out.layout_slot_for_page(page);
    if slot > 0 && out.continuations[slot - 1].first_page == page {
        return None;
    }
    // A relayout lays its first page out at that page's own width. A layout
    // that kept the page at another width, when it could not continue there,
    // cannot be reproduced.
    let laid_out_at = match slot {
        0 => 0,
        slot => out.continuations[slot - 1].first_page as usize,
    };
    let width = |page: usize| {
        out.geometries
            .get(page)
            .map(|geometry| geometry.content_box.width)
    };
    if (width(laid_out_at)? - width(page as usize)?).abs() > OFFSET_TOLERANCE {
        return None;
    }
    let (layout_document, layout_cascade) = out.layout_for_page(page);
    let start = layout_document.page_start(layout_cascade, page)?;
    // A relayout begins its first page at the page start's offset, which
    // cannot reproduce a line straddling the page's top edge.
    let offset = layout_document.page_token_offset(page, start.token)?;
    if (offset - start.offset).abs() > OFFSET_TOLERANCE {
        return None;
    }
    let dom = &document.uncascaded.dom;
    let skipped = skipped_before(dom, layout_document, &document.cascade, start.token.node())?;
    if skipped.is_empty() {
        return None;
    }
    let inside = inside_subtrees(dom, &skipped);
    let node_count = dom.node_count();
    // The boxes crossing the page start continue their fragments. The body
    // spans one page area from the top of the first page, so a relayout
    // that begins it above the resumed page cannot tell how far it reaches.
    let body = layout_root(dom, start.token.node());
    let earlier_fragments = layout_document.crossing_boxes(page);
    if earlier_fragments
        .iter()
        .any(|crossing| Some(crossing.node.0 as usize) == body)
    {
        return None;
    }
    for index in page..laid_out.page_count() {
        for fragment in laid_out.page_at(index as usize).fragments() {
            let id = fragment.node().0 as usize;
            let owner = if id < node_count {
                Some(id)
            } else {
                generated_origin(id).map(|(element, _)| element)
            };
            match owner {
                Some(owner) if !inside.get(owner).copied().unwrap_or(true) => {}
                _ => return None,
            }
        }
    }
    let mut page_names = names.to_vec();
    page_names.push(slice.page_name.clone());
    Some(ResumePoint {
        first_page: page,
        start,
        page_names,
        skipped,
        earlier_fragments,
    })
}

/// Whether a layout of `document` may resume at a point found by
/// [`resume_point`]: the features listed in the module documentation are
/// absent. Elements with node id `late_nodes_from` or more arrived after the
/// first delivery.
pub(super) fn can_resume(document: &HtmlDocument, late_nodes_from: Option<usize>) -> bool {
    let cascade = &document.cascade;
    let dom = &document.uncascaded.dom;
    let looks_up_targets = |computed: &ComputedValues| {
        computed.content.iter().any(|component| {
            matches!(
                component,
                ContentComponent::TargetCounter { .. }
                    | ContentComponent::TargetCounters { .. }
                    | ContentComponent::TargetText { .. }
            )
        })
    };
    if cascade
        .computed
        .iter()
        .any(|computed| !computed.running_templates.is_empty() || looks_up_targets(computed))
        || cascade
            .pseudo
            .values()
            .any(|computed| looks_up_targets(computed.as_ref()))
    {
        return false;
    }
    let from = late_nodes_from.unwrap_or(dom.node_count());
    !(from..dom.node_count()).any(|id| {
        dom.get_node(id)
            .is_some_and(|node| node.kind() == NodeKind::Element && node.is_in_document())
            && cascade.computed.get(id).is_some_and(|computed| {
                computed.position == PositionValue::Absolute
                    && computed.display != DisplayValue::None
            })
    })
}

/// The elements wholly before `target` that a resumed layout can leave out:
/// the earlier element siblings of `target` and of each of its ancestors,
/// at every level where the child is a block-level box of a block container
/// in the normal flow. Returns `None` when an ancestor at such a level is
/// not one.
///
/// `layout` is the laid-out document `target`'s page comes from, which tells
/// the inline content of paragraphs apart from block-level boxes. Siblings
/// inside a paragraph are part of the paragraph's lines and are kept.
fn skipped_before(
    dom: &Document,
    layout: &Document,
    cascade: &CascadeResult,
    target: usize,
) -> Option<Vec<usize>> {
    let mut skipped = Vec::new();
    for (child, parent) in dom.inclusive_ancestors(target).zip(dom.ancestors(target)) {
        let inline_content = layout
            .get_node(child)
            .is_some_and(|node| node.in_ifc_subtree());
        if !inline_content {
            if dom.get_node(parent)?.kind() == NodeKind::Element
                && !is_flow_block_container(cascade.computed.get(parent)?)
            {
                return None;
            }
            // Leaving content out shortens a box that holds it only when
            // its height follows its content. The layout sizes the body,
            // and the root above it, itself.
            if dom.get_node(parent)?.kind() == NodeKind::Element
                && !is_layout_root_or_above(dom, parent)
                && !has_content_height(cascade.computed.get(parent)?)
            {
                return None;
            }
            let siblings = &dom.get_node(parent)?.children;
            for &sibling in siblings.iter().take_while(|&&sibling| sibling != child) {
                let is_shown_element = dom
                    .get_node(sibling)
                    .is_some_and(|node| node.kind() == NodeKind::Element && node.is_in_document())
                    && cascade
                        .computed
                        .get(sibling)
                        .is_some_and(|computed| computed.display != DisplayValue::None);
                if is_shown_element {
                    skipped.push(sibling);
                }
            }
        }
    }
    Some(skipped)
}

/// Whether an element with these computed values lays its children out as a
/// block container in the normal flow.
fn is_flow_block_container(computed: &ComputedValues) -> bool {
    matches!(
        (computed.display, computed.float, &computed.position),
        (
            DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem,
            FloatValue::None,
            PositionValue::Static | PositionValue::Relative,
        )
    )
}

/// Whether an element with these computed values takes the height of its
/// content.
fn has_content_height(computed: &ComputedValues) -> bool {
    let unset = |value: &ComputedLengthPercentageOrAuto| {
        matches!(value, ComputedLengthPercentageOrAuto::Auto)
            || matches!(value, ComputedLengthPercentageOrAuto::Px(px) if *px == 0.0)
    };
    matches!(computed.height, ComputedLengthPercentageOrAuto::Auto)
        && matches!(computed.max_height, ComputedLengthPercentageOrAuto::Auto)
        && unset(&computed.min_height)
}

/// The element the layout lays the document out from, the body, found
/// among the ancestors of `node`: the element child of the root element.
fn layout_root(dom: &Document, node: usize) -> Option<usize> {
    dom.inclusive_ancestors(node).find(|&id| is_body(dom, id))
}

/// Whether element `id` is the body or the root element above it.
fn is_layout_root_or_above(dom: &Document, id: usize) -> bool {
    is_body(dom, id) || !element_parent(dom, id)
}

/// Whether element `id` is a child of the root element.
fn is_body(dom: &Document, id: usize) -> bool {
    element_parent(dom, id)
        && dom
            .parent_of(id)
            .is_some_and(|parent| !element_parent(dom, parent))
}

/// Whether node `id` has an element parent.
fn element_parent(dom: &Document, id: usize) -> bool {
    dom.parent_of(id)
        .and_then(|parent| dom.get_node(parent))
        .is_some_and(|parent| parent.kind() == NodeKind::Element)
}

/// Which nodes are `roots` or inside the subtree of one of them.
pub(super) fn inside_subtrees(dom: &Document, roots: &[usize]) -> Vec<bool> {
    let mut inside = vec![false; dom.node_count()];
    let mut stack = roots.to_vec();
    while let Some(id) = stack.pop() {
        if let Some(flag) = inside.get_mut(id) {
            *flag = true;
        }
        if let Some(node) = dom.get_node(id) {
            stack.extend(node.children.iter().copied());
        }
    }
    inside
}

#[cfg(test)]
mod tests;
