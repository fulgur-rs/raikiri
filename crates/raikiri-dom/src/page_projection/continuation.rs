//! Where a page's content begins in source order.
//!
//! The paginator lays the whole document out once at one inline size. When a
//! later page has a different content width, a caller lays a copy of the
//! document out again at that width and resumes from the source position at
//! which the page began in the first layout. This module finds that position
//! on a projected page, measures it in another layout, and prepares the
//! cascade of the relayout so that nothing before the position adds a break.

use super::records::{PageFragmentItem, PageFragmentKind};
use crate::Document;
use crate::layout::is_floating_box_for_pagination;
use raikiri_style::CascadeResult;
use raikiri_style::property::{BreakBetween, BreakInside, PageValue, PositionValue};
use raikiri_traits::NodeKind;

/// The first content of a page, in source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageStartToken {
    /// The page begins with the first fragment of this node: a block-level
    /// box, or a text node whose paragraph starts on the page.
    Node(usize),
    /// The page begins inside a paragraph, with the line whose first source
    /// text is byte `offset` of text node `text`.
    Line {
        /// Text node holding the first character of the line.
        text: usize,
        /// UTF-8 byte offset of that character within the text node.
        offset: u32,
    },
}

impl PageStartToken {
    /// The node the token belongs to.
    pub fn node(self) -> usize {
        match self {
            Self::Node(node) | Self::Line { text: node, .. } => node,
        }
    }
}

/// Where a page's content begins, and how far below the page's content
/// origin that content starts.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PageStart {
    /// The first content of the page.
    pub token: PageStartToken,
    /// Block offset of the token's top edge from the page's content origin,
    /// in CSS px. Non-zero when a repeated header or a preserved margin comes
    /// first.
    pub offset: f32,
}

/// Preorder positions of every node reachable from the document root.
fn preorder(document: &Document) -> Vec<u32> {
    let mut order = vec![u32::MAX; document.nodes.len()];
    let mut next = 0_u32;
    let mut stack = vec![document.root];
    while let Some(id) = stack.pop() {
        order[id] = next;
        next = next.saturating_add(1);
        stack.extend(document.nodes[id].children.iter().rev().copied());
    }
    order
}

/// Whether `node` or one of its ancestors is laid out outside the normal
/// flow, so its start does not mark where the flow of a page begins.
fn outside_normal_flow(document: &Document, cascade: &CascadeResult, node: usize) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if document.nodes[id].kind() == NodeKind::Element
            && let Some(computed) = cascade.computed.get(id)
            && (is_floating_box_for_pagination(document, cascade, id)
                || matches!(
                    computed.position,
                    PositionValue::Absolute | PositionValue::Fixed | PositionValue::Running(_)
                ))
        {
            return true;
        }
        current = document.parent_of(id);
    }
    false
}

/// The first DOM text of paragraph line `line` of the inline formatting
/// root `root`, as a text node and a byte offset in it.
fn line_start(document: &Document, root: usize, line: usize) -> Option<(usize, u32)> {
    let lines = document
        .ifc_layout_node(root)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?;
    let (node, range) = lines.lines.get(line)?.owners().next()?;
    Some((node.0 as usize, range.start))
}

impl Document {
    /// Where projected page `page_index` begins in source order.
    ///
    /// Reads the projection of the most recent
    /// [`Document::project_pages_with_control`]. Repeated content, boxes
    /// outside the normal flow and inline-level boxes are skipped. Returns
    /// `None` when the page has no starting content this function can name,
    /// for example a page that only continues one tall box, or one that
    /// starts inside a multicolumn container.
    pub fn page_start(&self, cascade: &CascadeResult, page_index: u32) -> Option<PageStart> {
        self.first_page_start(cascade, [page_index])
            .map(|(_, start)| start)
    }

    /// The first of the projected pages `page_indices`, in the order given,
    /// whose start [`Document::page_start`] can name, with that start.
    ///
    /// Walks the document's source order once for all the pages.
    pub fn first_page_start(
        &self,
        cascade: &CascadeResult,
        page_indices: impl IntoIterator<Item = u32>,
    ) -> Option<(u32, PageStart)> {
        let order = preorder(self);
        page_indices.into_iter().find_map(|page_index| {
            Some((page_index, self.page_start_in(cascade, page_index, &order)?))
        })
    }

    /// [`Document::page_start`], given the source order `order` of
    /// [`preorder`].
    fn page_start_in(
        &self,
        cascade: &CascadeResult,
        page_index: u32,
        order: &[u32],
    ) -> Option<PageStart> {
        let page = self
            .page_projection
            .pages
            .iter()
            .find(|page| page.page_index == page_index)?;
        let mut start = page
            .items
            .iter()
            .filter(|item| !item.is_repeat && !item.is_table_header_repeat)
            .filter_map(|item| Some((item, self.item_start(item)?)))
            .filter(|(item, _)| !outside_normal_flow(self, cascade, item.node_id.0 as usize))
            .min_by_key(|(item, _)| order[item.node_id.0 as usize])?
            .1;
        let target = start.token.node();
        // Columns are balanced over their whole container, so content
        // inside one cannot resume from a single page start.
        let in_columns = std::iter::successors(self.parent_of(target), |&id| self.parent_of(id))
            .any(|id| self.nodes[id].multicol.is_some());
        if in_columns {
            return None;
        }
        // A line that the earlier layout let straddle the page's top edge
        // starts the resumed page at its top. Other space above the token is
        // reproduced only when it is a margin or a repeated header: the tail
        // of a box that started on an earlier page ends elsewhere in another
        // layout, so a page with such a tail starts at the token instead.
        start.offset = start.offset.max(0.0);
        let continues_above = page.items.iter().any(|item| {
            let node = item.node_id.0 as usize;
            !item.is_repeat
                && !item.is_table_header_repeat
                && item.fragment_index > 0
                && item.rect.y < start.offset
                && !self.is_ancestor_or_self(node, target)
                && !self.is_ancestor_or_self(target, node)
        });
        if continues_above {
            start.offset = 0.0;
        }
        Some(start)
    }

    /// The page start `item` names when it is the first content of its
    /// page: the first fragment of a block-level box, or a text line.
    fn item_start(&self, item: &PageFragmentItem) -> Option<PageStart> {
        let node = item.node_id.0 as usize;
        match item.kind {
            PageFragmentKind::Box | PageFragmentKind::Replaced => {
                let block_level = !self.nodes[node].in_ifc_subtree() && node != self.root;
                (item.fragment_index == 0 && block_level).then_some(PageStart {
                    token: PageStartToken::Node(node),
                    offset: item.box_y,
                })
            }
            PageFragmentKind::Text => {
                let lines = self.ifc_text_lines(node)?;
                let first = lines.lines.first()?;
                let line = lines.lines.get(item.line_range?.start as usize)?;
                let token = match line.line {
                    0 => PageStartToken::Node(node),
                    index => {
                        let (text, offset) = line_start(self, lines.root, index)?;
                        PageStartToken::Line { text, offset }
                    }
                };
                Some(PageStart {
                    token,
                    offset: item.box_y + (line.top - first.top),
                })
            }
        }
    }

    /// Whether `ancestor` is `node` or one of its ancestors.
    fn is_ancestor_or_self(&self, ancestor: usize, node: usize) -> bool {
        std::iter::successors(Some(node), |&id| self.parent_of(id)).any(|id| id == ancestor)
    }

    /// Block offset of `token`'s top edge from the content origin of
    /// projected page `page_index`, or `None` when the page does not hold
    /// the token's start.
    ///
    /// A `Line` token is found only when a paragraph line starts at exactly
    /// its text position, which [`Document::set_continuation_break`] ensures.
    pub fn page_token_offset(&self, page_index: u32, token: PageStartToken) -> Option<f32> {
        let page = self
            .page_projection
            .pages
            .iter()
            .find(|page| page.page_index == page_index)?;
        let node = token.node();
        page.items
            .iter()
            .filter(|item| {
                item.node_id.0 as usize == node && !item.is_repeat && !item.is_table_header_repeat
            })
            .find_map(|item| match token {
                PageStartToken::Node(_) => (item.fragment_index == 0).then_some(item.box_y),
                PageStartToken::Line { text, offset } => {
                    let range = item.line_range?;
                    let lines = self.ifc_text_lines(text)?;
                    let first = lines.lines.first()?;
                    lines
                        .lines
                        .get(range.start as usize..range.end as usize)?
                        .iter()
                        .find(|line| {
                            line_start(self, lines.root, line.line) == Some((text, offset))
                        })
                        .map(|line| item.box_y + (line.top - first.top))
                }
            })
    }

    /// Insert a forced line break before byte `offset` of text node `text`
    /// in the next layout, or remove it with `None`.
    ///
    /// A relayout at another inline size uses this so that a paragraph that
    /// a page split in an earlier layout has a line starting at the same text.
    pub fn set_continuation_break(&mut self, at: Option<(usize, u32)>) {
        if self.continuation_break != at {
            self.continuation_break = at;
            self.layout_dirty = true;
        }
    }

    /// Keep the content before `token` from adding breaks of its own in a
    /// relayout that resumes at `token`.
    ///
    /// Boxes wholly before the token lose their forced breaks and page
    /// names; the token and its ancestors lose break avoidance, and the
    /// paragraph of a `Line` token keeps no minimum of lines on either side.
    /// The relayout then breaks exactly where it places the token's page.
    pub fn prepare_continuation_cascade(&self, cascade: &mut CascadeResult, token: PageStartToken) {
        let target = token.node();
        let order = preorder(self);
        let target_order = order[target];
        let mut ancestors = vec![false; self.nodes.len()];
        for id in std::iter::successors(Some(target), |&id| self.parent_of(id)) {
            ancestors[id] = true;
        }
        let page_values = &mut cascade.page_values;
        for (id, computed) in cascade.computed.iter_mut().enumerate() {
            if ancestors[id] {
                computed.break_inside = BreakInside::Auto;
                computed.orphans = 1;
                computed.widows = 1;
                computed.break_before = BreakBetween::Auto;
            } else if order[id] < target_order {
                computed.break_before = BreakBetween::Auto;
                computed.break_after = BreakBetween::Auto;
                if let Some(page) = page_values.get_mut(id) {
                    *page = PageValue::Auto;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
