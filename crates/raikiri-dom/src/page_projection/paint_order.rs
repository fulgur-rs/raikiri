//! A page's body content as a sequence of paint steps, in the order the
//! built-in painter draws it.

use super::records::{PageFragmentItem, PageFragmentKind};
use crate::{Document, Fragment, Node, paint_rules};
use raikiri_style::property::{ColumnCountValue, OverflowValue};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::{NodeId, NodeKind, PaintClip, PaintRect};
use std::collections::{HashMap, HashSet};

/// What a clip in [`PaintEvent::PushClip`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipKind {
    /// `overflow` other than `visible`: the padding box of the element's
    /// whole box. Where a page break cuts the box, the clip runs past the
    /// page.
    Overflow,
    /// A column of a multi-column container.
    Fragmentainer,
}

/// One step of painting a page's body content, in paint order.
///
/// Every `Push*` event is closed by the matching `Pop*` event later in the
/// same page's list, and pushes and pops nest.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum PaintEvent<'a> {
    /// Clip everything until the matching [`PaintEvent::PopClip`].
    PushClip(PaintClip, ClipKind),
    /// Close the innermost clip.
    PopClip,
    /// Composite everything until the matching [`PaintEvent::PopOpacity`]
    /// as one group with this opacity.
    PushOpacity(f32),
    /// Close the innermost opacity group.
    PopOpacity,
    /// An element's background, borders and outline. For a replaced element
    /// the fragment's kind is `Replaced` and the same fragment follows in a
    /// [`PaintEvent::Replaced`] event.
    Box(Fragment<'a>),
    /// The lines of a text node.
    Text(Fragment<'a>),
    /// The content of a replaced element (an image, an inline SVG or a
    /// canvas). Follows the element's [`PaintEvent::Box`].
    Replaced(Fragment<'a>),
}

/// The reason reported for a multi-column container by
/// [`Document::paint_order_approximations`].
const COLUMNS_WITHOUT_CLIPS: &str = "columns are listed without column clips";

enum Frame {
    Visit(usize),
    PopClip,
    PopOpacity,
}

/// The fragments of one page, grouped by source node.
struct PageItems<'a> {
    by_node: HashMap<usize, Vec<&'a PageFragmentItem>>,
    content_box: super::records::PageFragmentRect,
}

impl<'a> PageItems<'a> {
    fn of(&self, node_id: usize) -> &[&'a PageFragmentItem] {
        self.by_node.get(&node_id).map_or(&[], Vec::as_slice)
    }

    fn fragment(&self, item: &'a PageFragmentItem) -> Fragment<'a> {
        Fragment::new(item, self.content_box)
    }
}

impl Document {
    /// The body content of page `page_index` in paint order.
    ///
    /// Every fragment in the list is one of [`Document::page_fragments`] for
    /// the same page. Which nodes are on the page is taken from those
    /// fragments; `page_name` is accepted so the named-page check can move
    /// here later and is not consulted yet.
    #[doc(hidden)]
    pub fn page_paint_order<'a>(
        &'a self,
        cascade: &'a CascadeResult,
        page_index: u32,
        _page_name: Option<&str>,
    ) -> Vec<PaintEvent<'a>> {
        let Some(page) = self
            .page_projection
            .pages
            .iter()
            .find(|page| page.page_index == page_index)
        else {
            return Vec::new();
        };
        let Some(root) = paint_rules::find_paint_root(self) else {
            return Vec::new(); // cov:ignore: a laid-out page always has a body to paint from
        };
        let mut items = PageItems {
            by_node: HashMap::new(),
            content_box: page.content_box,
        };
        for item in &page.items {
            if let Ok(node_id) = usize::try_from(item.node_id.0) {
                items.by_node.entry(node_id).or_default().push(item);
            }
        }

        let mut events = Vec::new();
        // An explicit stack, like the painter's, so a deep DOM cannot
        // overflow the call stack.
        let mut stack = vec![Frame::Visit(root)];
        while let Some(frame) = stack.pop() {
            let node_id = match frame {
                Frame::PopClip => {
                    events.push(PaintEvent::PopClip);
                    continue;
                }
                Frame::PopOpacity => {
                    events.push(PaintEvent::PopOpacity);
                    continue;
                }
                Frame::Visit(node_id) => node_id,
            };
            let Some(node) = self.get_node(node_id) else {
                continue; // cov:ignore: node ids on the stack come from the arena
            };
            if !node.is_in_document() || node.is_non_rendered_html_element() {
                continue;
            }
            match node.kind() {
                NodeKind::Element => {
                    if node.is_display_none() || node.is_hidden_by_text_overflow() {
                        continue;
                    }
                    let Some(cv) = cascade.computed.get(node_id) else {
                        continue; // cov:ignore: the cascade has a value for every arena node
                    };
                    // The group stays open until the element's whole subtree
                    // has been listed.
                    if let Some(alpha) = paint_rules::opacity_layer(cv) {
                        events.push(PaintEvent::PushOpacity(alpha));
                        stack.push(Frame::PopOpacity);
                    }
                    let own: Vec<&PageFragmentItem> = items
                        .of(node_id)
                        .iter()
                        .copied()
                        .filter(|item| item.kind != PageFragmentKind::Text)
                        .collect();
                    if !paint_rules::is_visibility_hidden_table(cv) {
                        let replaced_content =
                            node.is_inline_svg_root() || self.is_canvas_element(node_id);
                        for &item in &own {
                            let fragment = items.fragment(item);
                            events.push(PaintEvent::Box(fragment));
                            if item.kind == PageFragmentKind::Replaced || replaced_content {
                                events.push(PaintEvent::Replaced(fragment));
                            }
                        }
                    }
                    // The clip is the padding box of the element's whole box
                    // (see `overflow_clip`). It opens after the element's own
                    // box and closes after its subtree.
                    if paint_rules::clips_overflow(cv) {
                        let Some(&item) = own.first() else {
                            // The element's box is not on this page, so the
                            // painter's clip for it, built from the whole box,
                            // lies wholly outside the page and nothing inside
                            // it is drawn here. Any opacity group opened above
                            // is still closed by its pending frame.
                            continue;
                        };
                        let fragment = items.fragment(item);
                        let clip = overflow_clip(node, cv, item, fragment);
                        events.push(PaintEvent::PushClip(clip, ClipKind::Overflow));
                        stack.push(Frame::PopClip);
                    }
                    if node.is_ifc_root() {
                        push_paragraph(self, &items, node_id, &mut events);
                    }
                    let mut children = if node.is_inline_svg_root() {
                        Vec::new()
                    } else if node.is_ifc_root() {
                        // The paragraph's own text and inline elements were
                        // listed above; only the boxes laid out beside its
                        // lines are visited like ordinary children.
                        node.ifc_boxes()
                    } else {
                        node.children.clone()
                    };
                    paint_rules::sort_paint_children(&mut children, cv.display, cascade);
                    stack.extend(children.into_iter().rev().map(Frame::Visit));
                }
                // A text node laid out as an anonymous flex or grid item is a
                // paragraph of its own. Any other text node outside a
                // paragraph has no lines to draw.
                NodeKind::Text if node.is_ifc_root() => {
                    push_paragraph(self, &items, node_id, &mut events);
                }
                _ => {} // cov:ignore: comments and other node kinds are out of the document
            }
        }
        events
    }

    /// Multi-column containers whose subtree is listed without column
    /// clips, each with the reason.
    ///
    /// [`Document::page_paint_order`] lists the content of every column
    /// without a [`ClipKind::Fragmentainer`] clip around it, so content that
    /// overflows a column is not cut at the column's edge.
    #[doc(hidden)]
    pub fn paint_order_approximations(
        &self,
        cascade: &CascadeResult,
    ) -> Vec<(NodeId, &'static str)> {
        let is_multicol = |node_id: usize| {
            cascade
                .computed
                .get(node_id)
                .is_some_and(|cv| matches!(cv.column_count, ColumnCountValue::Count(n) if n >= 2))
        };
        // Whether the node and all its ancestors are in the document and
        // displayed.
        let is_rendered = |node_id: usize| {
            let mut cursor = Some(node_id);
            while let Some(id) = cursor {
                let Some(node) = self.get_node(id) else {
                    return false; // cov:ignore: ancestor ids come from the arena
                };
                if !node.is_in_document()
                    || (node.kind() == NodeKind::Element
                        && (node.is_display_none() || node.is_hidden_by_text_overflow()))
                {
                    return false;
                }
                cursor = self.parent_of(id);
            }
            true
        };
        let mut found: Vec<usize> = (0..self.nodes.len())
            .filter(|&node_id| {
                self.get_node(node_id)
                    .is_some_and(|node| node.kind() == NodeKind::Element)
                    && is_multicol(node_id)
                    && is_rendered(node_id)
                    && !std::iter::successors(self.parent_of(node_id), |&id| self.parent_of(id))
                        .any(is_multicol)
            })
            .collect();
        found.sort_unstable();
        found
            .into_iter()
            .map(|node_id| (NodeId::new(node_id as u64), COLUMNS_WITHOUT_CLIPS))
            .collect()
    }
}

/// Lists a paragraph's inline element boxes, then its text, in document order.
///
/// The painter draws a paragraph from its lines: the backgrounds of inline
/// elements first, then the text. Boxes laid out beside the lines (floats and
/// atomic inlines) are visited as the paragraph root's children instead, so
/// their subtrees are skipped here.
fn push_paragraph<'a>(
    document: &'a Document,
    items: &PageItems<'a>,
    root: usize,
    events: &mut Vec<PaintEvent<'a>>,
) {
    let Some(root_node) = document.get_node(root) else {
        return; // cov:ignore: paragraph roots come from the arena
    };
    let push_kind = |events: &mut Vec<PaintEvent<'a>>, node_id: usize, kind| {
        for &item in items.of(node_id) {
            if item.kind == kind {
                let fragment = items.fragment(item);
                events.push(match kind {
                    PageFragmentKind::Text => PaintEvent::Text(fragment),
                    _ => PaintEvent::Box(fragment),
                });
            }
        }
    };
    if root_node.kind() == NodeKind::Text {
        push_kind(events, root, PageFragmentKind::Text);
        return;
    }
    let beside_lines: HashSet<usize> = root_node.ifc_boxes().into_iter().collect();
    let mut elements = Vec::new();
    let mut texts = Vec::new();
    let mut stack: Vec<usize> = root_node.children.iter().rev().copied().collect();
    while let Some(node_id) = stack.pop() {
        if beside_lines.contains(&node_id) {
            continue;
        }
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: child ids come from the arena
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            continue;
        }
        match node.kind() {
            NodeKind::Element if !node.is_display_none() && !node.is_hidden_by_text_overflow() => {
                elements.push(node_id);
                stack.extend(node.children.iter().rev().copied());
            }
            NodeKind::Text => texts.push(node_id),
            _ => {}
        }
    }
    for node_id in elements {
        push_kind(events, node_id, PageFragmentKind::Box);
    }
    for node_id in texts {
        push_kind(events, node_id, PageFragmentKind::Text);
    }
}

/// The padding box of the element's whole border box, snapped the way the
/// painter snaps an overflow clip: the origin is floored so pixel-snapped
/// descendant backgrounds are not cut by antialiasing, and an
/// `overflow: clip` edge is floored too.
///
/// Like the painter's clip, it is built from the box before a page break cut
/// it, so on a page holding only part of the box it runs past the page.
fn overflow_clip(
    node: &Node,
    cv: &ComputedValues,
    item: &PageFragmentItem,
    fragment: Fragment<'_>,
) -> PaintClip {
    let rect = fragment.paint_rect();
    let top = rect.y - item.rect.y + item.box_y;
    let padding = node.unrounded_layout.padding;
    let right = rect.x + rect.width - padding.right;
    let bottom = top + item.box_height - padding.bottom;
    let x0 = (rect.x + padding.left).floor();
    let y0 = (top + padding.top).floor();
    let x1 = if matches!(cv.overflow.x, OverflowValue::Clip) {
        right.floor()
    } else {
        right
    };
    let y1 = if matches!(cv.overflow.y, OverflowValue::Clip) {
        bottom.floor()
    } else {
        bottom
    };
    PaintClip::new(PaintRect::new(x0, y0, x1 - x0, y1 - y0))
}
