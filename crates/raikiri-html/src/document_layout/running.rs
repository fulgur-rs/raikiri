//! Running elements placed into page margin boxes (CSS GCPM 3 §1.2,
//! <https://www.w3.org/TR/css-gcpm-3/#running-elements>).
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
//!   own, at the width the margin box gives it. The result is a one-page
//!   layout that is read through the same [`Page`] accessors as a document
//!   page, so a painter draws it with the code it already has.

use std::collections::HashMap;

use raikiri_dom::{Document, PageLayoutControl, layout_pages_with_page_geometry_and_control};
use raikiri_style::computed::ComputedValues;
use raikiri_style::property::{
    BreakBetween, DisplayValue, FloatValue, PositionValue, StringFetchMode,
};
use raikiri_style::{CascadeResult, ComputedLengthPercentageOrAuto, PageCascadeResult};
use raikiri_traits::{NodeId, PageBox, RenderError};
use smol_str::SmolStr;

use super::Page;
use crate::render::{ResolvedPageGeometry, resolve_page_geometry};

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
pub(crate) struct RunningIndex {
    pools: HashMap<SmolStr, Vec<Placement>>,
}

impl RunningIndex {
    /// Index the running elements of `document`.
    ///
    /// `rendered_pages` gives, for a node that has fragments, the first and
    /// last page they are on. A running element itself has none, so it is
    /// placed by the rendered content around it: on the first page of the
    /// next rendered node after it in document order, or, with nothing after
    /// it, on the last page of the content before it.
    pub(crate) fn build(
        document: &Document,
        cascade: &CascadeResult,
        rendered_pages: &HashMap<usize, (u32, u32)>,
    ) -> Self {
        let mut index = Self::default();
        if !cascade
            .computed
            .iter()
            .any(|computed| !computed.running_templates.is_empty())
        {
            return index;
        }
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
                .push(Placement {
                    node: NodeId(node as u64),
                    page,
                    starts_page: previous.is_none_or(|last| last < page),
                });
        }
        index
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
    pub(crate) fn select(&self, name: &str, fetch: StringFetchMode, page: u32) -> Option<NodeId> {
        let pool = self.pools.get(name)?;
        let entry = pool
            .iter()
            .rev()
            .find(|placement| placement.page < page)
            .map(|placement| placement.node);
        let mut on_page = pool.iter().filter(|placement| placement.page == page);
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

    /// Whether `node` is a running element of this document.
    pub(crate) fn contains(&self, node: NodeId) -> bool {
        self.pools
            .values()
            .any(|pool| pool.iter().any(|placement| placement.node == node))
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

/// A running element laid out on its own at a margin box's width.
///
/// The element is laid out with its own computed values, which it inherits
/// from its place in the document (CSS GCPM 3 §1.2.1), inside a containing
/// block of the requested width. [`Self::page`] reads the result through
/// the accessors of a document page: the element's top-left content edge of
/// the containing block is the page origin `(0, 0)`, so a painter offsets
/// everything by the margin box's content-box origin.
pub struct RunningElementLayout {
    node: NodeId,
    document: Document,
    cascade: CascadeResult,
    slice: raikiri_dom::PageSlice,
    geometry: ResolvedPageGeometry,
    style: PageCascadeResult,
    height: f32,
}

impl RunningElementLayout {
    /// The running element.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Width of the containing block the element was laid out in.
    pub fn width(&self) -> f32 {
        self.geometry.page_box.width
    }

    /// Height from the containing block's top edge to the bottom margin edge
    /// of the element, the space it takes in the margin box.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// The laid-out element as a page of [`Self::width`] by
    /// [`Self::height`] with no page margins.
    ///
    /// Besides the element's own fragments, the page can have a box fragment
    /// for `<body>`. It carries no decoration and extends below
    /// [`Self::height`], down the page the element was measured on, so a
    /// painter that clips to the margin box draws nothing of it.
    pub fn page(&self) -> Page<'_> {
        Page {
            slice: &self.slice,
            geometry: &self.geometry,
            style: &self.style,
            document: &self.document,
            cascade: &self.cascade,
            page_count: 1,
            paired_style: None,
            running: None,
        }
    }
}

/// Lay running element `node` out at `width` CSS px.
///
/// `source` is the laid-out document: its replaced elements already carry
/// their resolved intrinsic sizes, so nothing is fetched again. Only the
/// element's subtree and its ancestors stay in the copy that is laid out;
/// the ancestors keep what they pass down by inheritance and lose their own
/// boxes, so the element's containing block is the margin box content area.
pub(crate) fn layout_running_element(
    source: &Document,
    cascade: &CascadeResult,
    node: NodeId,
    width: f32,
) -> Result<Option<RunningElementLayout>, RenderError> {
    let index = node.0 as usize;
    let Some(template) = cascade
        .computed
        .get(index)
        .and_then(|computed| computed.running_templates.first())
    else {
        // cov:ignore: the caller only passes nodes the running index found in the document.
        return Ok(None);
    };
    let width = if width.is_finite() {
        width.max(0.0)
    } else {
        0.0
    };

    let mut ancestors = Vec::new();
    let mut parent = source.parent_of(index);
    while let Some(id) = parent {
        ancestors.push(id);
        parent = source.parent_of(id);
    }
    let mut in_subtree = vec![false; source.node_count()];
    let mut stack = vec![index];
    while let Some(id) = stack.pop() {
        if let Some(flag) = in_subtree.get_mut(id) {
            *flag = true;
        }
        if let Some(n) = source.get_node(id) {
            stack.extend(n.children.iter().copied());
        }
    }

    let mut document = source.clone();
    document.retain_children(|child| in_subtree[child] || ancestors.contains(&child));
    document.mark_in_document_flags();

    let mut cascade = cascade.clone();
    for &ancestor in &ancestors {
        let Some(original) = cascade.computed.get(ancestor) else {
            continue;
        };
        // What the ancestor passes down by inheritance, without its own box.
        let mut boxless = ComputedValues::inherit_from(original);
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
    )
    .map_err(RenderError::from)?;
    let Some(slice) = slices.into_iter().next() else {
        return Ok(None); // cov:ignore: a document with a body always emits a page slice.
    };
    let measuring = resolve_page_geometry(&cascade.page, page_box);
    document
        .project_pages_with_control(
            &cascade,
            page_box,
            std::slice::from_ref(&slice),
            &[(
                measuring.page_box,
                measuring.margins,
                measuring.content_insets,
            )],
            &control,
        )
        .map_err(RenderError::from)?;

    // Margin percentages refer to the containing block's width.
    let margin_bottom = match cascade.computed[index].margin.bottom {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        ComputedLengthPercentageOrAuto::Percent(percent) => percent / 100.0 * width,
        _ => 0.0,
    }
    .max(0.0);
    let height = document
        .page_fragments(0)
        .filter(|fragment| {
            in_subtree
                .get(fragment.node().0 as usize)
                .copied()
                .unwrap_or(false)
        })
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
    let style = cascade.page.clone();
    Ok(Some(RunningElementLayout {
        node,
        document,
        cascade,
        slice,
        geometry,
        style,
        height,
    }))
}

#[cfg(test)]
mod tests;
