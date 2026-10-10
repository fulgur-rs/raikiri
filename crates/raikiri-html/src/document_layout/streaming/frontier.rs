//! The stable frontier of a partly parsed document: the earliest node whose
//! layout may still change once more input arrives.
//!
//! Everything before the frontier in tree order lays out the same way
//! whatever input follows, so pages that end before it can be committed.
//! The frontier is the earliest of:
//!
//! - an open element whose layout depends on children that may not have
//!   arrived yet: tables (auto layout measures every row), flex and grid
//!   containers, multi-column containers (balancing measures all content)
//!   boxes with `break-inside: avoid*`, and shrink-to-fit or
//!   `min-content` wide boxes (floats and absolutely positioned boxes with
//!   an auto width), whose width comes from their widest content;
//! - an open element with auto directionality (`dir=auto`, or `<bdi>`
//!   without a valid `dir`), whose direction comes from its first strong
//!   character, which may not have arrived yet, and which `:dir()` exposes
//!   to the styles of everything inside it;
//! - the start of the trailing inline run of the innermost open block
//!   container, because a paragraph's bidi direction, `text-wrap: balance`
//!   and orphans / widows depend on all of its lines;
//! - the document root, when a selector's result can change as later
//!   siblings or descendants arrive (see
//!   [`raikiri_style::RuleTree::has_forward_dependent_selectors`]); the
//!   style of an open ancestor such as `<body>` could still change.
//!
//! A frontier node joined to the content before it by `break-before:
//! avoid*` or `break-after: avoid*` moves back to that content, since the
//! page boundary between them is not final either. That includes content
//! before an ancestor the frontier node starts.

use raikiri_dom::Document;
use raikiri_style::computed::ComputedValues;
use raikiri_style::property::{
    BreakBetween, BreakInside, ColumnCountValue, DisplayValue, FloatValue, PositionValue,
};
use raikiri_style::{CascadeResult, ComputedColumnWidth, ComputedLengthPercentageOrAuto};
use raikiri_traits::NodeKind;

use crate::document_layout::DocumentLayout;

const XHTML: &str = "http://www.w3.org/1999/xhtml";

/// Where a partly parsed document stops being final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Frontier {
    /// Everything parsed so far is final.
    End,
    /// This node and everything after it in tree order may still change.
    At(usize),
}

/// Find the stable frontier of `document`, a snapshot taken while the tree
/// builder held the element handles `traced` (see
/// [`crate::sink::traced_handles`]).
pub(crate) fn stable_frontier(
    document: &Document,
    cascade: &CascadeResult,
    traced: &[usize],
    forward_dependent_selectors: bool,
) -> Frontier {
    if forward_dependent_selectors {
        return Frontier::At(document.root_index());
    }
    let mut chain: Vec<(usize, usize)> = open_html_elements(document, traced)
        .into_iter()
        .map(|id| (depth(document, id), id))
        .collect();
    chain.sort_unstable();

    // The outermost open element with auto directionality, which precedes
    // everything after it in tree order apart from a trailing inline run
    // that starts before it.
    let mut auto_direction = None;
    for &(_, id) in &chain {
        if auto_direction.is_none() && has_auto_direction(document, id) {
            auto_direction = Some(id);
        }
        if computed(cascade, id).is_none_or(needs_all_children) {
            let held = auto_direction.unwrap_or(id);
            return Frontier::At(join_avoided_breaks(document, cascade, held));
        }
    }
    let innermost_block = chain
        .iter()
        .rev()
        .map(|&(_, id)| id)
        .find(|&id| computed(cascade, id).is_some_and(|values| !is_inline_level(values)));
    let run = innermost_block.and_then(|block| trailing_inline_run(document, cascade, block));
    let held = match (auto_direction, run) {
        (Some(auto), Some(start)) if !is_inclusive_ancestor(document, auto, start) => Some(start),
        (Some(auto), _) => Some(auto),
        (None, run) => run,
    };
    match held {
        Some(id) => Frontier::At(join_avoided_breaks(document, cascade, id)),
        None => Frontier::End,
    }
}

fn is_inclusive_ancestor(document: &Document, ancestor: usize, id: usize) -> bool {
    std::iter::successors(Some(id), |&node| document.parent_of(node)).any(|node| node == ancestor)
}

/// The HTML elements of `document` that may still receive children, given
/// the tree builder's handles `traced`.
pub(crate) fn open_html_elements(document: &Document, traced: &[usize]) -> Vec<usize> {
    open_elements(document, traced)
        .into_iter()
        .filter(|&id| is_html_element_in_document(document, id))
        .collect()
}

/// Narrow the tree builder's handles to the elements that may still receive
/// children.
///
/// The stack of open elements comes first in `traced` and every element on
/// it is a child of an element below it on the stack (or of the document),
/// so a handle is kept when its parent is the document or an element kept
/// before it. Stale entries of the list of active formatting elements are
/// usually children of an element that was closed and are dropped; the
/// rare one that survives is a closed element treated as open, which only
/// moves the frontier earlier. A closed `<head>` survives too, but it never
/// holds the frontier while `<body>` is open.
fn open_elements(document: &Document, traced: &[usize]) -> Vec<usize> {
    let root = document.root_index();
    let mut open: Vec<usize> = Vec::new();
    for &id in traced {
        if open.contains(&id) {
            continue;
        }
        let parent = document.parent_of(id);
        if parent == Some(root) || parent.is_some_and(|parent| open.contains(&parent)) {
            open.push(id);
        }
    }
    open
}

/// How many leading pages of `laid_out`, a layout of the snapshot
/// `document`, are final.
///
/// Content before the frontier is final, and so is everything up to the
/// last node, apart from one thing: content that arrives later can join the
/// last node with an avoided break. Each node of `held` is not final either,
/// nor anything after it. The earliest page holding content at or after any
/// of these points is not final, and neither is the page before it, whose
/// end break depends on what that content does. Every earlier page lies
/// wholly before all of them.
pub(crate) fn final_page_count(
    document: &Document,
    cascade: &CascadeResult,
    frontier: Frontier,
    held: &[usize],
    laid_out: &DocumentLayout,
) -> u32 {
    let order = preorder(document);
    let Some(&last) = order.last() else {
        return 0;
    };
    let mut position = vec![usize::MAX; document.node_count()];
    for (index, &id) in order.iter().enumerate() {
        position[id] = index;
    }
    let mut earliest_unstable = position[join_avoided_breaks(document, cascade, last)];
    if let Frontier::At(id) = frontier {
        earliest_unstable = earliest_unstable.min(position[id]);
    }
    for &id in held {
        if let Some(&index) = position.get(join_avoided_breaks(document, cascade, id)) {
            earliest_unstable = earliest_unstable.min(index);
        }
    }

    let page_count = laid_out.page_count();
    let mut first_page = vec![page_count; document.node_count()];
    for index in 0..page_count {
        for fragment in laid_out.page_at(index as usize).fragments() {
            let node = fragment.node().0 as usize;
            if let Some(page) = first_page.get_mut(node) {
                *page = (*page).min(index);
            }
        }
    }
    let unstable_page = order[earliest_unstable..]
        .iter()
        .map(|&id| first_page[id])
        .min()
        .unwrap_or(page_count);
    unstable_page.saturating_sub(1)
}

/// Every node reachable from the document root, in tree order.
fn preorder(document: &Document) -> Vec<usize> {
    let mut order = Vec::new();
    let mut stack = vec![document.root_index()];
    while let Some(id) = stack.pop() {
        order.push(id);
        if let Some(node) = document.get_node(id) {
            stack.extend(node.children.iter().rev().copied());
        }
    }
    order
}

fn computed(cascade: &CascadeResult, id: usize) -> Option<&ComputedValues> {
    cascade.computed.get(id)
}

fn is_html_element_in_document(document: &Document, id: usize) -> bool {
    document
        .get_node(id)
        .is_some_and(|node| node.kind() == NodeKind::Element && node.is_in_document())
        && document.element_namespace_uri(id) == Some(XHTML)
}

fn depth(document: &Document, id: usize) -> usize {
    std::iter::successors(document.parent_of(id), |&parent| document.parent_of(parent)).count()
}

/// Whether the box's layout, including the position of its first children,
/// depends on children that come later.
fn needs_all_children(values: &ComputedValues) -> bool {
    has_intrinsic_width(values)
        || matches!(
            values.display,
            DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
        )
        || matches!(values.column_count, ColumnCountValue::Count(_))
        || matches!(values.column_width, ComputedColumnWidth::Px(_))
        || values.break_inside != BreakInside::Auto
}

/// Whether the box's width depends on the size of its content: a
/// `min-content` width, or the shrink-to-fit width of a float
/// or an absolutely positioned box (CSS 2 §10.3.5, §10.3.7). A later child
/// with a wider unbreakable line can widen the box and reflow every earlier
/// line in it.
fn has_intrinsic_width(values: &ComputedValues) -> bool {
    match values.width {
        ComputedLengthPercentageOrAuto::MinContent => true,
        ComputedLengthPercentageOrAuto::Auto => {
            values.float != FloatValue::None
                || matches!(
                    values.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
        }
        _ => false,
    }
}

/// Whether the element's directionality is resolved from its text (HTML
/// §3.2.6.4): `dir=auto`, or a `<bdi>` whose `dir` is missing or invalid.
fn has_auto_direction(document: &Document, id: usize) -> bool {
    match document.element_attribute(id, "dir") {
        Some(value) if value.eq_ignore_ascii_case("auto") => true,
        Some(value) if value.eq_ignore_ascii_case("ltr") || value.eq_ignore_ascii_case("rtl") => {
            false
        }
        _ => document
            .get_node(id)
            .and_then(|node| node.tag_name())
            .is_some_and(|tag| tag.eq_ignore_ascii_case("bdi")),
    }
}

fn is_inline_level(values: &ComputedValues) -> bool {
    matches!(
        values.display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineFlex
            | DisplayValue::InlineGrid
            | DisplayValue::InlineTable
    )
}

/// Whether `id` can belong to the inline formatting context of its parent:
/// text, inline-level boxes, floats and absolutely positioned boxes (which
/// are blockified but still sit in the surrounding inline content), and
/// boxes that generate no box of their own.
fn joins_inline_run(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let Some(node) = document.get_node(id) else {
        return false;
    };
    match node.kind() {
        NodeKind::Text => true,
        NodeKind::Element => computed(cascade, id).is_none_or(|values| {
            is_inline_level(values)
                || values.float != FloatValue::None
                || matches!(
                    values.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
                || matches!(values.display, DisplayValue::None | DisplayValue::Contents)
        }),
        _ => false,
    }
}

/// The first node of the run of inline content that ends `block`'s
/// children, if its last child is inline content.
fn trailing_inline_run(
    document: &Document,
    cascade: &CascadeResult,
    block: usize,
) -> Option<usize> {
    let children = &document.get_node(block)?.children;
    let mut start = None;
    for &child in children.iter().rev() {
        if !document
            .get_node(child)
            .is_some_and(|node| node.is_in_document())
        {
            continue;
        }
        if !joins_inline_run(document, cascade, child) {
            break;
        }
        start = Some(child);
    }
    start
}

fn avoids_break(value: BreakBetween) -> bool {
    matches!(
        value,
        BreakBetween::Avoid | BreakBetween::AvoidPage | BreakBetween::AvoidColumn
    )
}

/// Move `id` back over earlier content that it is joined to by an avoided
/// break. A node that starts its parent's content shares the break point
/// before the parent, so the search climbs through first children and a
/// `break-before` on any of them counts for that break point.
fn join_avoided_breaks(document: &Document, cascade: &CascadeResult, id: usize) -> usize {
    let mut result = id;
    let mut current = id;
    let mut avoid_before = false;
    loop {
        avoid_before |=
            computed(cascade, current).is_some_and(|values| avoids_break(values.break_before));
        match previous_element_sibling(document, current) {
            Some(previous) => {
                let avoid_after = computed(cascade, previous)
                    .is_some_and(|values| avoids_break(values.break_after));
                if !(avoid_before || avoid_after) {
                    return result;
                }
                result = previous;
                current = previous;
                avoid_before = false;
            }
            None => match document.parent_of(current) {
                Some(parent) if parent != document.root_index() => current = parent,
                _ => return result,
            },
        }
    }
}

fn previous_element_sibling(document: &Document, id: usize) -> Option<usize> {
    let parent = document.parent_of(id)?;
    let siblings = &document.get_node(parent)?.children;
    let position = siblings.iter().position(|&sibling| sibling == id)?;
    siblings[..position].iter().rev().copied().find(|&sibling| {
        document
            .get_node(sibling)
            .is_some_and(|node| node.kind() == NodeKind::Element && node.is_in_document())
    })
}

#[cfg(test)]
mod tests;
