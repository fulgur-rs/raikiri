//! Running elements (CSS GCPM 3 §1.2,
//! <https://www.w3.org/TR/css-gcpm-3/#running-elements>) and the string
//! values page-margin `string()` / `element()` references show.
//!
//! `position: running(<name>)` removes an element from the normal flow; its
//! computed `display` is `none` and the display it lays out with is kept on
//! its [`raikiri_style::computed::RunningTemplate`]. A margin box whose
//! `content` is `element(<name>, <keyword>?)` shows, on each page, one of the
//! elements of that name, chosen relative to the page:
//!
//! - [`RunningIndex`] records where each running element sits in the page
//!   sequence and answers the per-page selection.
//! - [`layout_running_element`] lays the chosen element out again, on its
//!   own, at the width the margin box gives it, as a one-page layout.
//! - [`element_string_value`] is the text an element contributes to a
//!   `string-set` `content()` value, and to a margin box that shows a
//!   running element as text.

use std::collections::HashMap;

use raikiri_style::computed::ComputedValues;
use raikiri_style::property::{
    BreakBetween, DisplayValue, FloatValue, PositionValue, StringFetchMode,
};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto, PageCascadeResult};
use raikiri_traits::{LayoutError, NodeId, PageBox, PaintRect};
use smol_str::SmolStr;

use crate::document::Document;
use crate::layout::{
    PageContentInsets, PageLayoutControl, PageMargins, PageSlice,
    layout_pages_with_page_geometry_and_control, page_content_insets_for_page,
    page_margins_for_page,
};
use crate::node::NodeData;

/// Height of the page a running element is measured on. A running element
/// is not fragmented: everything it holds lands on this one page.
const MEASURE_HEIGHT: f32 = 1.0e6;

/// Where one running element sits in the page sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    node: NodeId,
    /// The page the element is assigned on.
    page: u32,
    /// No rendered content of [`Self::page`] precedes the element.
    starts_page: bool,
}

/// The running elements of one document, by name, in document order.
#[derive(Debug, Default)]
pub struct RunningIndex {
    pools: HashMap<SmolStr, Pool>,
}

/// The running elements of one name, in document order.
#[derive(Debug, Default)]
struct Pool {
    placements: Vec<Placement>,
    /// The placements are on non-decreasing pages, as they usually are, so
    /// the elements of a page are found by binary search.
    by_page: bool,
}

/// Whether any element of `cascade` is a running element.
fn has_running_elements(cascade: &CascadeResult) -> bool {
    cascade
        .computed
        .iter()
        .any(|computed| !computed.running_templates.is_empty())
}

impl RunningIndex {
    /// Index the running elements of `document`, laid out on `pages`.
    ///
    /// `pages` gives each laid-out page as the projected document that holds
    /// its fragments and the page's index. Node ids are those of
    /// `document`; a page may come from a re-laid-out copy that keeps them.
    /// A running element has no fragments of its own, so it is placed by the
    /// rendered content around it: on the first page of the next rendered
    /// node after it in document order, or, with nothing after it, on the
    /// last page of the content before it.
    pub fn build<'d>(
        document: &Document,
        cascade: &CascadeResult,
        pages: impl IntoIterator<Item = (&'d Document, u32)>,
    ) -> Self {
        if !has_running_elements(cascade) {
            return Self::default();
        }
        let mut rendered_pages = HashMap::<usize, (u32, u32)>::new();
        for (projected, page) in pages {
            for fragment in projected.page_fragments(page) {
                rendered_pages
                    .entry(fragment.node().0 as usize)
                    .and_modify(|(first, last)| {
                        *first = (*first).min(page);
                        *last = (*last).max(page);
                    })
                    .or_insert((page, page));
            }
        }
        Self::from_rendered_pages(document, cascade, &rendered_pages)
    }

    /// Index the running elements of `document`, given for each node that
    /// has fragments the first and last page they are on.
    fn from_rendered_pages(
        document: &Document,
        cascade: &CascadeResult,
        rendered_pages: &HashMap<usize, (u32, u32)>,
    ) -> Self {
        let mut index = Self::default();
        let order = preorder(document);
        for (position, &(node, subtree_end)) in order.iter().enumerate() {
            let Some(template) = cascade
                .computed
                .get(node)
                .and_then(|computed| computed.running_templates.first())
            else {
                continue;
            };
            let next = order[subtree_end..]
                .iter()
                .find_map(|(id, _)| rendered_pages.get(id).map(|(first, _)| *first));
            // Ancestors of the element enclose it; they do not precede it.
            let previous = order[..position]
                .iter()
                .rev()
                .filter(|(_, end)| *end <= position)
                .find_map(|(id, _)| rendered_pages.get(id).map(|(_, last)| *last));
            let page = next.or(previous).unwrap_or(0);
            index
                .pools
                .entry(template.name.clone())
                .or_default()
                .placements
                .push(Placement {
                    node: NodeId(node as u64),
                    page,
                    starts_page: previous.is_none_or(|last| last < page),
                });
        }
        for pool in index.pools.values_mut() {
            pool.by_page = pool.placements.is_sorted_by_key(|placement| placement.page);
        }
        index
    }

    /// Every running element of the index, in no particular order and
    /// listed once per name it is registered under.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.pools
            .values()
            .flat_map(|pool| pool.placements.iter().map(|placement| placement.node))
    }

    /// The running element `element(<name>, <fetch>)` shows on page `page`
    /// (CSS GCPM 3 §1.2.2, <https://www.w3.org/TR/css-gcpm-3/#element-syntax>).
    ///
    /// The keywords read the elements assigned on the page and the entry
    /// value, the last element assigned on an earlier page:
    /// - `first`: the first element assigned on the page, else the entry
    ///   value.
    /// - `start`: the first element assigned on the page when no content of
    ///   the page comes before it, else the entry value.
    /// - `last`: the last element assigned on the page, else the entry value.
    /// - `first-except`: nothing on a page that has an assignment, else the
    ///   entry value.
    pub fn select(&self, name: &str, fetch: StringFetchMode, page: u32) -> Option<NodeId> {
        let Pool {
            placements: pool,
            by_page,
        } = self.pools.get(name)?;
        let (before, on_page) = if *by_page {
            let start = pool.partition_point(|placement| placement.page < page);
            let end = start + pool[start..].partition_point(|placement| placement.page == page);
            (&pool[..start], &pool[start..end])
        } else {
            (&pool[..], &pool[..])
        };
        let entry = before
            .iter()
            .rev()
            .find(|placement| placement.page < page)
            .map(|placement| placement.node);
        let mut on_page = on_page.iter().filter(|placement| placement.page == page);
        match fetch {
            StringFetchMode::Start => match on_page.next() {
                Some(first) if first.starts_page => Some(first.node),
                _ => entry,
            },
            StringFetchMode::Last => on_page
                .next_back()
                .map(|placement| placement.node)
                .or(entry),
            StringFetchMode::FirstExcept => match on_page.next() {
                Some(_) => None,
                None => entry,
            },
            // `first`, and any keyword added later, which the spec defaults
            // to `first`.
            _ => on_page.next().map(|placement| placement.node).or(entry),
        }
    }
}

/// In-document nodes in document order, each with the position just past
/// its subtree.
fn preorder(document: &Document) -> Vec<(usize, usize)> {
    let mut order = Vec::new();
    // (node, index in `order` once entered); a node is closed after its
    // children by a second visit.
    let mut stack = vec![(document.root_index(), false)];
    let mut open = Vec::new();
    while let Some((id, closing)) = stack.pop() {
        if closing {
            let start = open.pop().unwrap_or_default();
            let end = order.len();
            if let Some(entry) = order.get_mut(start) {
                *entry = (id, end);
            }
            continue;
        }
        let Some(node) = document.get_node(id) else {
            continue;
        };
        if id != document.root_index() && !node.is_in_document() {
            continue;
        }
        open.push(order.len());
        order.push((id, order.len() + 1));
        stack.push((id, true));
        for &child in node.children.iter().rev() {
            stack.push((child, false));
        }
    }
    order
}

/// The page box of one page with its margins, padding and border resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedPageGeometry {
    /// The page box.
    pub page_box: PageBox,
    /// The page margins.
    pub margins: PageMargins,
    /// The page box's border and padding, inside the margins.
    pub content_insets: PageContentInsets,
    /// The page area: the page box less its margins and content insets.
    pub content_box: PaintRect,
}

/// Resolve the margins and content insets `page` sets on `page_box`.
pub fn resolve_page_geometry(page: &PageCascadeResult, page_box: PageBox) -> ResolvedPageGeometry {
    let margins = page_margins_for_page(page, page_box);
    let content_insets = page_content_insets_for_page(page, page_box);
    let content_box = PaintRect::new(
        margins.left + content_insets.left,
        margins.top + content_insets.top,
        content_insets.page_area_width(margins, page_box),
        content_insets.page_area_height(margins, page_box),
    );
    ResolvedPageGeometry {
        page_box,
        margins,
        content_insets,
        content_box,
    }
}

/// A running element laid out on its own by [`layout_running_element`].
///
/// [`Self::document`] is a copy of the source document that holds only the
/// element's subtree and its ancestors, numbered on its own, and laid out as
/// one page of [`Self::slice`]. The top-left content edge of the element's
/// containing block is the page origin `(0, 0)`.
#[derive(Debug)]
pub struct RunningElementPage {
    /// For each node of [`Self::document`], the node of the source document
    /// it was copied from.
    pub source_nodes: Vec<usize>,
    /// The laid-out copy.
    pub document: Document,
    /// The copy's cascade.
    pub cascade: CascadeResult,
    /// The copy's one page.
    pub slice: PageSlice,
    /// The page: the containing block's width by [`Self::height`], with no
    /// page margins.
    pub geometry: ResolvedPageGeometry,
    /// Height from the containing block's top edge to the bottom margin edge
    /// of the element.
    pub height: f32,
}

/// Lay running element `node` of `source` out on its own at `width` CSS px.
///
/// `source` is the laid-out document: its replaced elements already carry
/// their resolved intrinsic sizes, so nothing is fetched again. Only the
/// element's subtree and its ancestors stay in the copy that is laid out;
/// the ancestors keep what they pass down by inheritance and lose their own
/// boxes, so the element's containing block is `width` wide. The element is
/// laid out with its own computed values, which it inherits from its place
/// in the document (CSS GCPM 3 §1.2.1). `None` when `node` is not a running
/// element.
pub fn layout_running_element(
    source: &Document,
    cascade: &CascadeResult,
    node: NodeId,
    width: f32,
) -> Result<Option<RunningElementPage>, LayoutError> {
    let index = node.0 as usize;
    let Some(template) = cascade
        .computed
        .get(index)
        .and_then(|computed| computed.running_templates.first())
    else {
        return Ok(None);
    };
    // The element's ancestors from the root down, then its subtree in
    // preorder, copied out of the document with the cascade renumbered to
    // match.
    let mut nodes = Vec::new();
    let mut parent = source.parent_of(index);
    while let Some(id) = parent {
        nodes.push(id);
        parent = source.parent_of(id);
    }
    nodes.reverse();
    let ancestor_count = nodes.len();
    let mut stack = vec![index];
    while let Some(id) = stack.pop() {
        nodes.push(id);
        if let Some(n) = source.get_node(id) {
            stack.extend(n.children.iter().rev().copied());
        }
    }
    let mut document = source.extract_nodes(&nodes);
    document.mark_in_document_flags();
    let mut cascade = cascade.extract(&nodes);
    let index = ancestor_count;

    for ancestor in 0..index {
        // What the ancestor passes down by inheritance, without its own box.
        let mut boxless = ComputedValues::inherit_from(&cascade.computed[ancestor]);
        boxless.display = DisplayValue::Block;
        cascade.computed[ancestor] = boxless;
    }
    {
        let element = &mut cascade.computed[index];
        element.display = template.display;
        element.running_templates.clear();
        element.position = PositionValue::Static;
        element.float = FloatValue::None;
        element.break_before = BreakBetween::Auto;
        element.break_after = BreakBetween::Auto;
    }
    // No @page margins: the containing block is the whole measuring page.
    // Replacing the page also gives the copy its own cascade generation, so
    // layout state cached against the source cascade is not reused.
    cascade.replace_page(PageCascadeResult::default());

    let mut page_box = PageBox::new();
    page_box.width = width;
    page_box.height = MEASURE_HEIGHT;
    let control = PageLayoutControl::new(Some(1));
    let slices = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box,
        &[],
        &[],
        &control,
    )?;
    let Some(slice) = slices.into_iter().next() else {
        return Ok(None); // cov:ignore: a document with a body always emits a page slice.
    };
    let measuring = resolve_page_geometry(&cascade.page, page_box);
    document.project_pages_with_control(
        &cascade,
        page_box,
        std::slice::from_ref(&slice),
        &[(
            measuring.page_box,
            measuring.margins,
            measuring.content_insets,
        )],
        &control,
    )?;

    // Margin percentages refer to the containing block's width.
    let margin_bottom = match cascade.computed[index].margin.bottom {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        ComputedLengthPercentageOrAuto::Percent(percent) => percent / 100.0 * width,
        ComputedLengthPercentageOrAuto::Calc(calc) => calc.px + calc.percent / 100.0 * width,
        _ => 0.0,
    }
    .max(0.0);
    let height = document
        .page_fragments(0)
        .filter(|fragment| fragment.node().0 as usize >= index)
        .map(|fragment| {
            let rect = fragment.paint_rect();
            rect.y + rect.height
        })
        .fold(0.0_f32, f32::max)
        + margin_bottom;

    let mut used_box = PageBox::new();
    used_box.width = width;
    used_box.height = height;
    let geometry = resolve_page_geometry(&cascade.page, used_box);
    Ok(Some(RunningElementPage {
        source_nodes: nodes,
        document,
        cascade,
        slice,
        geometry,
        height,
    }))
}

/// The string value of element `idx`: the text that a bare `content()` or
/// `content(text)` in `string-set` assigns, and that a page-margin
/// `element()` shows as text.
///
/// Approximates CSS GCPM 3 §1.1.1.1's "the string value of the element,
/// determined as if `white-space: normal` had been set". The spec gives no
/// normative algorithm for "string value" beyond that sentence, so this
/// concatenates every descendant [`NodeData::Text`] node's character data
/// in document order (skipping any subtree with `IS_IN_DOCUMENT` cleared)
/// and collapses runs of [`is_css_document_white_space`] characters to a
/// single space with both ends trimmed.
///
/// Deliberately neither [`str::split_ascii_whitespace`] nor
/// [`str::split_whitespace`]: the former's Rust-defined ASCII whitespace set
/// additionally includes U+000C FORM FEED, and the latter's Unicode
/// `White_Space` set additionally includes U+00A0 NO-BREAK SPACE. CSS Text 3
/// §4.1 <https://www.w3.org/TR/css-text-3/#white-space-rules> scopes
/// `white-space: normal` collapsing to "spaces (U+0020), tabs (U+0009), and
/// segment breaks" only, and §4
/// <https://www.w3.org/TR/css-text-3/#white-space-processing> requires every
/// other control character — form feed included — to "be rendered as a
/// visible glyph", not collapsed or stripped. See
/// [`is_css_document_white_space`]'s doc for the full character set and its
/// citations.
///
/// Does not include generated content (`::before`/`::after`) — that's
/// `content(before)` / `content(after)`'s job.
pub fn element_string_value(doc: &Document, idx: usize) -> String {
    let mut raw = String::new();
    collect_descendant_text(doc, idx, &mut raw);
    raw.split(is_css_document_white_space)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The predicate behind [`element_string_value`]'s whitespace
/// collapsing: CSS Text 3's [document white space
/// characters](https://www.w3.org/TR/css-text-3/#white-space-rules) —
/// "spaces (U+0020), tabs (U+0009), and segment breaks" (§4.1) — plus
/// carriage return (U+000D), which §4
/// <https://www.w3.org/TR/css-text-3/#white-space-processing> states is
/// "treated identically to spaces (U+0020) in all respects", even though CR
/// is not itself a segment break (a segment break is line feed (U+000A) for
/// HTML-parsed content, per the same section).
///
/// CR-inclusion is not dead weight against some non-HTML-normalized
/// [`Document`]: an HTML parser folds a *literal* CR/CRLF byte in the source
/// to LF during input-stream preprocessing, but a numeric character
/// reference such as `&#x0D;` is decoded to U+000D during tokenization,
/// *after* that normalization step, so a real DOM text node produced by a
/// conformant parser can and does contain a literal U+000D. §4's own
/// closing sentence on this point: "the character is preserved — and the
/// above rule observable — when encoded using an escape sequence
/// (`&#x0d;`)."
///
/// **Deliberately excludes form feed (U+000C)**, unlike
/// [`char::is_ascii_whitespace`] / [`str::split_ascii_whitespace`]: §4's
/// control-character rule requires every Unicode category `Cc` control
/// character "other than tabs (U+0009), line feeds (U+000A), carriage
/// returns (U+000D) and sequences that form a segment break" — form feed
/// included — to "be rendered as a visible glyph", i.e. treated as ordinary
/// text, not collapsed or trimmed away.
fn is_css_document_white_space(c: char) -> bool {
    matches!(c, '\u{0020}' | '\u{0009}' | '\u{000A}' | '\u{000D}')
}

/// DFS accumulator for [`element_string_value`] — appends every
/// descendant [`NodeData::Text`] node's raw character data (pre-whitespace-
/// collapse) to `out`, in document order.
///
/// Explicit-stack iterative DFS (reverse-push children so the `Vec` pops
/// them in original, i.e. document, order) rather than recursion, so a
/// deeply nested DOM cannot exhaust the call stack.
fn collect_descendant_text(doc: &Document, root_idx: usize, out: &mut String) {
    let mut stack: Vec<usize> = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let node = &doc.nodes[idx];
        if !node.is_in_document() {
            continue;
        }
        if let NodeData::Text(t) = &node.data {
            out.push_str(t.text_content.as_str());
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
}

#[cfg(test)]
mod tests;
