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
//!   and boxes with `break-inside: avoid*`;
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
use raikiri_style::{CascadeResult, ComputedColumnWidth};
use raikiri_traits::NodeKind;

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
    let mut chain: Vec<(usize, usize)> = open_elements(document, traced)
        .into_iter()
        .filter(|&id| is_html_element_in_document(document, id))
        .map(|id| (depth(document, id), id))
        .collect();
    chain.sort_unstable();

    for &(_, id) in &chain {
        if computed(cascade, id).is_none_or(needs_all_children) {
            return Frontier::At(join_avoided_breaks(document, cascade, id));
        }
    }
    let innermost_block = chain
        .iter()
        .rev()
        .map(|&(_, id)| id)
        .find(|&id| computed(cascade, id).is_some_and(|values| !is_inline_level(values)));
    let Some(block) = innermost_block else {
        return Frontier::End;
    };
    match trailing_inline_run(document, cascade, block) {
        Some(start) => Frontier::At(join_avoided_breaks(document, cascade, start)),
        None => Frontier::End,
    }
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
    matches!(
        values.display,
        DisplayValue::Table
            | DisplayValue::InlineTable
            | DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Grid
            | DisplayValue::InlineGrid
    ) || matches!(values.column_count, ColumnCountValue::Count(_))
        || matches!(values.column_width, ComputedColumnWidth::Px(_))
        || values.break_inside != BreakInside::Auto
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
