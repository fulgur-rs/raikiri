//! A page's body content as a sequence of paint steps, in the order the
//! built-in painter draws it.

use super::generated_boxes::{GeneratedBox, PageGeneratedBoxes};
use super::records::{PageFragmentItem, PageFragmentKind};
use crate::{
    Document, Fragment, GeneratedKind, OverflowClip, PositionedGlyphRun, RunSource, TextLineId,
    paint_rules,
};
use raikiri_style::property::{ColumnCountValue, DisplayValue};
use raikiri_style::{CascadeResult, PseudoElem, StyleNodeId};
use raikiri_traits::{NodeId, NodeKind, PaintClip, PaintRect};
use std::collections::{BTreeMap, HashMap, HashSet};

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
    /// One cell area of a source table row or group. The clip encloses only
    /// the box event, preserving separated-border gaps for the table below.
    TableCell,
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
    /// One paragraph line from the supplied positioned glyph runs, including
    /// generated content and ellipses: the runs of the line that no inline
    /// opacity group holds (see [`Document::text_run_opacity_group`]).
    /// Emitted once per line.
    TextLine(TextLineId),
    /// The runs of one paragraph line held by the inline opacity group of
    /// this element, the innermost one around them. Emitted once per line
    /// and group, inside that group's [`PaintEvent::PushOpacity`].
    GroupTextLine(TextLineId, NodeId),
    /// The content of a replaced element (an image, an inline SVG or a
    /// canvas). Follows the element's [`PaintEvent::Box`].
    Replaced(Fragment<'a>),
    /// Prepared raster marker of a list item. Its placement is owned by this page.
    MarkerImage(NodeId),
    /// One generated inline box decoration; its text follows in paragraph lines.
    GeneratedBox(GeneratedBox<'a>),
    /// A producer-computed column rule, above the owner's border and below content.
    ColumnRule(crate::ColumnRule),
}

/// The reason reported for a multi-column container by
/// [`Document::paint_order_approximations`].
const LEGACY_COLUMN_CLIPS: &str = "legacy column clipping may be incomplete";

enum Frame {
    Visit(usize),
    Paragraph(usize),
    /// An inline opacity group of a paragraph, by paragraph root and element.
    InlineGroup(usize, usize),
    PopClip,
    PopOpacity,
}

/// The supplied lines of a page's paragraphs, split by the inline opacity
/// group that holds their runs.
#[derive(Default)]
struct TextLines {
    /// Lines with runs outside every inline opacity group, by paragraph root.
    paragraphs: HashMap<NodeId, Vec<TextLineId>>,
    /// Lines with runs in an inline opacity group, by the group's element.
    groups: HashMap<NodeId, Vec<TextLineId>>,
}

/// The fragments of one page, grouped by source node.
struct PageItems<'a> {
    by_node: HashMap<usize, Vec<&'a PageFragmentItem>>,
    content_box: super::records::PageFragmentRect,
    overflow_clips: BTreeMap<NodeId, OverflowClip>,
    generated_boxes: Option<&'a PageGeneratedBoxes>,
    cascade: &'a CascadeResult,
    document: &'a Document,
    page: &'a super::records::PageFragment,
}

impl<'a> PageItems<'a> {
    fn of(&self, node_id: usize) -> &[&'a PageFragmentItem] {
        self.by_node.get(&node_id).map_or(&[], Vec::as_slice)
    }

    fn fragment(&self, item: &'a PageFragmentItem) -> Fragment<'a> {
        Fragment::new(item, self.content_box).with_overflow_clip(
            item.own_overflow_source
                .map(|index| {
                    self.document
                        .placement_overflow_clip_on_page(index, self.page)
                        .clip
                })
                .or_else(|| {
                    self.overflow_clips
                        .get(&item.node_id)
                        .filter(|_| item.kind != PageFragmentKind::Text)
                        .map(|entry| entry.clip)
                }),
        )
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
        self.page_paint_order_impl(cascade, page_index, None, true)
    }

    /// Paint order with one text event per line in the supplied page runs.
    ///
    /// The runs must come from this document and page. Their line identities
    /// replace text fragments without reshaping or extracting text again.
    #[doc(hidden)]
    pub fn page_paint_order_for_text_runs<'a>(
        &'a self,
        cascade: &'a CascadeResult,
        page_index: u32,
        runs: &[PositionedGlyphRun<'a>],
    ) -> Vec<PaintEvent<'a>> {
        let mut lines = TextLines::default();
        let mut seen = HashSet::new();
        for run in runs {
            let group = self.text_run_opacity_group(cascade, run);
            if seen.insert((run.line, group)) {
                match group {
                    Some(group) => lines.groups.entry(group),
                    None => lines.paragraphs.entry(run.line.root),
                }
                .or_default()
                .push(run.line);
            }
        }
        for paragraph in lines
            .paragraphs
            .values_mut()
            .chain(lines.groups.values_mut())
        {
            paragraph.sort_unstable_by_key(|line| line.index);
        }
        self.page_paint_order_impl(cascade, page_index, Some(&lines), true)
    }

    /// The element whose inline opacity group paints `run`, if any.
    ///
    /// Opacity below one makes an element a group that composites its
    /// complete paint at once (CSS Color 4 §3.3). For a non-atomic inline
    /// element, that paint is its fragments on the paragraph's lines; the
    /// run belongs to the innermost such element around its text, inside
    /// its paragraph. Runs of the paragraph root's own content, list markers
    /// and ellipses belong to no inline group. Elements with
    /// `display: contents` form no group.
    #[doc(hidden)]
    pub fn text_run_opacity_group(
        &self,
        cascade: &CascadeResult,
        run: &PositionedGlyphRun<'_>,
    ) -> Option<NodeId> {
        let start = match run.source {
            RunSource::Text(node) => self.parent_of(usize::try_from(node.0).ok()?)?,
            RunSource::Generated(owner, GeneratedKind::Before | GeneratedKind::After) => {
                usize::try_from(owner.0).ok()?
            }
            _ => return None,
        };
        let root = usize::try_from(run.line.root.0).ok()?;
        self.inline_opacity_group(cascade, root, start)
            .map(|group| NodeId::new(group as u64))
    }

    /// The innermost inline opacity group of paragraph `root` that contains
    /// `start`, walking up from `start` itself to the paragraph root.
    fn inline_opacity_group(
        &self,
        cascade: &CascadeResult,
        root: usize,
        start: usize,
    ) -> Option<usize> {
        let owner = self.ifc_source_owner(root);
        let mut current = Some(start);
        while let Some(id) = current {
            if id == root || id == owner {
                return None;
            }
            let node = self.get_node(id)?;
            if node.kind() != NodeKind::Element {
                return None;
            }
            let cv = cascade.computed.get(id)?;
            if is_inline_opacity_group(cv) {
                return Some(id);
            }
            // Anything but an inline element or a `display: contents` one is
            // a block boundary: the walk has left the paragraph.
            if !matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents) {
                return None;
            }
            current = self.parent_of(id);
        }
        None
    }

    /// The paint order of one page. With `skip_empty`, subtrees that list
    /// nothing on the page are not walked; the events are the same.
    fn page_paint_order_impl<'a>(
        &'a self,
        cascade: &'a CascadeResult,
        page_index: u32,
        lines: Option<&TextLines>,
        skip_empty: bool,
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
            overflow_clips: self.overflow_clips_on_page(page),
            generated_boxes: self.page_projection.generated_boxes.get(&page_index),
            cascade,
            document: self,
            page,
        };
        for item in &page.items {
            if let Ok(node_id) = usize::try_from(item.node_id.0) {
                items.by_node.entry(node_id).or_default().push(item);
            }
        }

        let entered = skip_empty
            .then(|| self.page_paint_nodes(&items, page_index, lines))
            .flatten();

        let mut events = Vec::new();
        // An explicit stack, like the painter's, so a deep DOM cannot
        // overflow the call stack.
        let mut stack = vec![Frame::Visit(root)];
        while let Some(frame) = stack.pop() {
            let node_id = match frame {
                Frame::Paragraph(key) => {
                    let groups = push_paragraph(self, &items, key, None, lines, &mut events);
                    stack.extend(
                        groups
                            .into_iter()
                            .rev()
                            .map(|group| Frame::InlineGroup(key, group)),
                    );
                    continue;
                }
                Frame::InlineGroup(root, group) => {
                    // The group composites after the paragraph's own content,
                    // like other z-index: auto stacking contexts: its inline
                    // boxes and text, then its atomic inlines and floats,
                    // then the groups nested in it.
                    let Some(alpha) = cascade
                        .computed
                        .get(group)
                        .and_then(paint_rules::opacity_layer)
                    else {
                        continue; // cov:ignore: groups are found by their opacity
                    };
                    events.push(PaintEvent::PushOpacity(alpha));
                    stack.push(Frame::PopOpacity);
                    let nested =
                        push_paragraph(self, &items, root, Some(group), lines, &mut events);
                    stack.extend(
                        nested
                            .into_iter()
                            .rev()
                            .map(|nested| Frame::InlineGroup(root, nested)),
                    );
                    let mut boxes = self.ifc_boxes_in_group(cascade, root, Some(group));
                    if let Some(cv) = cascade.computed.get(root) {
                        paint_rules::sort_paint_children(&mut boxes, cv.display, cascade);
                    }
                    stack.extend(boxes.into_iter().rev().map(Frame::Visit));
                    continue;
                }
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
            // A subtree with nothing on this page lists no events.
            if entered
                .as_ref()
                .is_some_and(|entered| !entered.contains(&node_id))
            {
                continue;
            }
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
                    if paint_rules::generates_box(cv)
                        && !paint_rules::is_visibility_hidden_table(cv)
                        && !paint_rules::hides_empty_table_cell(self, cascade, node_id)
                    {
                        let replaced_content =
                            node.is_inline_svg_root() || self.is_canvas_element(node_id);
                        for &item in &own {
                            let fragment = items.fragment(item);
                            if let Some(cells) = self
                                .anonymous_table_part_background_cells(node_id)
                                .filter(|_| !paint_rules::is_visibility_hidden_table_part(cv))
                                .filter(|_| paint_rules::paints_table_part_decoration(cv))
                            {
                                let rect = fragment.paint_rect();
                                let top = rect.y - item.rect.y + item.box_y;
                                // The cells cover the whole part, but this
                                // fragment paints only inside its own slice:
                                // a cell outside it would clip away everything,
                                // and on a long table most cells are outside.
                                let in_slice = |clip: &PaintRect| {
                                    clip.x < rect.x + rect.width
                                        && clip.y < rect.y + rect.height
                                        && clip.x + clip.width > rect.x
                                        && clip.y + clip.height > rect.y
                                };
                                for cell in cells {
                                    let clip = PaintRect::new(
                                        rect.x + cell.x,
                                        top + cell.y,
                                        cell.width,
                                        cell.height,
                                    );
                                    if !in_slice(&clip) {
                                        continue;
                                    }
                                    events.push(PaintEvent::PushClip(
                                        PaintClip::new(clip),
                                        ClipKind::TableCell,
                                    ));
                                    events.push(PaintEvent::Box(fragment));
                                    events.push(PaintEvent::PopClip);
                                }
                            } else {
                                events.push(PaintEvent::Box(fragment));
                            }
                            if item.kind == PageFragmentKind::Replaced || replaced_content {
                                if let Some(clip) = fragment.overflow_clip() {
                                    events.push(PaintEvent::PushClip(clip, ClipKind::Overflow));
                                }
                                events.push(PaintEvent::Replaced(fragment));
                                if fragment.overflow_clip().is_some() {
                                    events.push(PaintEvent::PopClip);
                                }
                            }
                        }
                    }
                    if let Some(rules) = self
                        .page_projection
                        .column_rules
                        .get(&page_index)
                        .and_then(|rules| rules.get(&NodeId::new(node_id as u64)))
                    {
                        let clip = items.overflow_clips.get(&NodeId::new(node_id as u64));
                        if let Some(entry) = clip {
                            events.push(PaintEvent::PushClip(entry.clip, ClipKind::Overflow));
                        }
                        events.extend(rules.iter().copied().map(PaintEvent::ColumnRule));
                        if clip.is_some() {
                            events.push(PaintEvent::PopClip);
                        }
                    }
                    let image_marker = self
                        .page_marker_image(page_index, NodeId::new(node_id as u64))
                        .is_some();
                    let inline_marker =
                        crate::generated_content::inside_marker_in_flow(cascade, node_id)
                            && node.is_ifc_root();
                    if image_marker && !inline_marker {
                        events.push(PaintEvent::MarkerImage(NodeId::new(node_id as u64)));
                    }
                    if let Some(lines) = lines {
                        let marker = crate::generated_content::generated_node_id(
                            node_id,
                            raikiri_style::PseudoElem::Marker,
                        );
                        events.extend(
                            lines
                                .paragraphs
                                .get(&NodeId::new(marker as u64))
                                .into_iter()
                                .flatten()
                                .copied()
                                .map(PaintEvent::TextLine),
                        );
                    }
                    // The clip is the padding box of the element's whole box
                    // resolved from shared source geometry. It opens after the element's own
                    // box and closes after its subtree.
                    if let Some(entry) = items.overflow_clips.get(&NodeId::new(node_id as u64)) {
                        // A closed vertical axis hides descendants on pages
                        // the box does not reach. An open vertical axis still
                        // admits them and must preserve the horizontal clip.
                        if own.is_empty() && entry.clip.clip_y {
                            continue;
                        }
                        events.push(PaintEvent::PushClip(entry.clip, ClipKind::Overflow));
                        stack.push(Frame::PopClip);
                    }
                    let mut groups = Vec::new();
                    if node.is_ifc_root() {
                        if image_marker && inline_marker {
                            events.push(PaintEvent::MarkerImage(NodeId::new(node_id as u64)));
                        }
                        groups = push_paragraph(self, &items, node_id, None, lines, &mut events);
                    }
                    let mut children = if node.is_inline_svg_root()
                        || self.anonymous_table_contents_paint_only(node_id)
                    {
                        Vec::new()
                    } else if node.is_ifc_root() {
                        // The paragraph's own text and inline elements were
                        // listed above; only the boxes laid out beside its
                        // lines are visited like ordinary children. Those in
                        // an inline opacity group are visited in the group.
                        self.ifc_boxes_in_group(cascade, node_id, None)
                    } else if let Some(children) = self.anonymous_table_paint_sequence(node_id) {
                        children
                    } else {
                        node.children.clone()
                    };
                    paint_rules::sort_paint_children(&mut children, cv.display, cascade);
                    stack.extend(
                        groups
                            .into_iter()
                            .rev()
                            .map(|group| Frame::InlineGroup(node_id, group)),
                    );
                    stack.extend(children.into_iter().rev().map(|key| {
                        if self.get_node(key).is_none() && self.ifc_layout_node(key).is_some() {
                            Frame::Paragraph(key)
                        } else {
                            Frame::Visit(key)
                        }
                    }));
                }
                // A text node laid out as an anonymous flex or grid item is a
                // paragraph of its own. Any other text node outside a
                // paragraph has no lines to draw.
                NodeKind::Text if node.is_ifc_root() => {
                    // A lone text node has no inline elements, so no groups.
                    push_paragraph(self, &items, node_id, None, lines, &mut events);
                }
                _ => {} // cov:ignore: comments and other node kinds are out of the document
            }
        }
        let mut line_clips = HashMap::new();
        let mut line_overflow_chains = HashMap::new();
        // Only the paragraphs with lines or generated boxes in the events
        // need their line clips.
        let listed_roots: HashSet<usize> = events
            .iter()
            .filter_map(|event| match event {
                PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => Some(line.root),
                PaintEvent::GeneratedBox(fragment) => Some(fragment.line.root),
                _ => None,
            })
            .filter_map(|root| usize::try_from(root.0).ok())
            .collect();
        for root in &self.page_projection.text_roots {
            if (root.fragment_clip.is_none() && root.overflow_chain.is_none())
                || !listed_roots.contains(&root.node)
            {
                continue;
            }
            let indices: Vec<usize> = if let Some((owner, PseudoElem::Marker)) =
                crate::generated_content::generated_origin(root.node)
            {
                self.page_projection
                    .markers
                    .get(&owner)
                    .map_or_else(Vec::new, |marker| (0..marker.line_count()).collect())
            } else if let Some(positioned) =
                crate::PositionedLines::new(self, cascade, root.node, root.fragmentainer)
            {
                (0..positioned.all_lines().len())
                    .filter(|&index| positioned.line_offset(index).is_some())
                    .collect()
            } else {
                Vec::new()
            };
            if let Some(chain) = root.overflow_chain {
                for &index in &indices {
                    line_overflow_chains.insert(
                        TextLineId {
                            root: NodeId::new(root.node as u64),
                            index,
                        },
                        chain,
                    );
                }
            }
            let Some(mut clip) = root.fragment_clip else {
                continue;
            };
            let source = crate::generated_content::generated_origin(root.node)
                .map_or_else(|| self.ifc_source_owner(root.node), |(owner, _)| owner);
            let mut repeat = root.is_repeat;
            if let Some(shift) = self
                .table_objects
                .headers
                .shift(source, page.content_origin_y)
            {
                let Some(shift) = shift else { continue };
                clip.y += shift - page.content_origin_y;
                repeat = true;
            }
            clip.x += page.content_box.x;
            clip.y += page.content_box.y - if repeat { 0.0 } else { page.content_origin_y };
            for &index in &indices {
                line_clips.insert(
                    TextLineId {
                        root: NodeId::new(root.node as u64),
                        index,
                    },
                    PaintClip::new(clip),
                );
            }
        }
        let mut clipped_events = Vec::with_capacity(events.len());
        for event in events {
            let mut chain = match event {
                PaintEvent::Box(fragment)
                | PaintEvent::Text(fragment)
                | PaintEvent::Replaced(fragment) => fragment.overflow_chain(),
                PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => {
                    line_overflow_chains.get(&line).copied()
                }
                PaintEvent::GeneratedBox(fragment) => {
                    line_overflow_chains.get(&fragment.line).copied()
                }
                PaintEvent::ColumnRule(rule) => rule.overflow_chain,
                PaintEvent::MarkerImage(owner) => {
                    items.of(owner.0 as usize).first().and_then(|item| {
                        if crate::generated_content::inside_marker_in_flow(
                            cascade,
                            owner.0 as usize,
                        ) {
                            item.own_overflow_source.or(item.overflow_chain)
                        } else {
                            item.overflow_chain
                        }
                    })
                }
                _ => None,
            };
            let mut overflow = Vec::new();
            while let Some(index) = chain {
                overflow.push(self.placement_overflow_clip_on_page(index, page).clip);
                chain = self.page_projection.placement_overflow_clips[index].parent;
            }
            for &clip in overflow.iter().rev() {
                clipped_events.push(PaintEvent::PushClip(clip, ClipKind::Overflow));
            }
            let clip = match event {
                PaintEvent::Box(fragment)
                | PaintEvent::Text(fragment)
                | PaintEvent::Replaced(fragment) => fragment.fragmentainer_clip(),
                PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => {
                    line_clips.get(&line).copied()
                }
                PaintEvent::GeneratedBox(fragment) => line_clips.get(&fragment.line).copied(),
                PaintEvent::ColumnRule(rule) => rule.fragmentainer_clip,
                PaintEvent::MarkerImage(owner) => items
                    .of(owner.0 as usize)
                    .first()
                    .and_then(|item| items.fragment(item).fragmentainer_clip()),
                _ => None,
            };
            if let Some(clip) = clip {
                clipped_events.push(PaintEvent::PushClip(clip, ClipKind::Fragmentainer));
                clipped_events.push(event);
                clipped_events.push(PaintEvent::PopClip);
            } else {
                clipped_events.push(event);
            }
            clipped_events.extend(std::iter::repeat_n(PaintEvent::PopClip, overflow.len()));
        }
        clipped_events
    }

    /// The boxes laid out beside the lines of paragraph `root` (floats and
    /// atomic inlines) whose innermost inline opacity group is `group`.
    fn ifc_boxes_in_group(
        &self,
        cascade: &CascadeResult,
        root: usize,
        group: Option<usize>,
    ) -> Vec<usize> {
        let Some(node) = self.get_node(root) else {
            return Vec::new(); // cov:ignore: element paragraph roots are arena nodes
        };
        let mut boxes = node.ifc_boxes();
        boxes.retain(|&id| {
            self.parent_of(id)
                .and_then(|parent| self.inline_opacity_group(cascade, root, parent))
                == group
        });
        boxes
    }

    /// The nodes the paint walk of one page has to enter: every node that can
    /// list an event on the page, and its ancestors. A node's paint parent
    /// (its DOM parent, the paragraph root beside whose lines it is laid out,
    /// or the owner of its anonymous table cell) is always one of its DOM
    /// ancestors, so the walk reaches all of them.
    ///
    /// `None` when a source of events cannot be traced to a node of the
    /// arena; the walk then enters every node.
    fn page_paint_nodes(
        &self,
        items: &PageItems<'_>,
        page_index: u32,
        lines: Option<&TextLines>,
    ) -> Option<HashSet<usize>> {
        let projection = &self.page_projection;
        let ids = |ids: &mut dyn Iterator<Item = NodeId>| -> Vec<usize> {
            ids.filter_map(|id| usize::try_from(id.0).ok()).collect()
        };
        let mut sources: Vec<usize> = items.by_node.keys().copied().collect();
        sources.extend(ids(&mut lines
            .into_iter()
            .flat_map(|lines| lines.paragraphs.keys().chain(lines.groups.keys()))
            .copied()));
        sources.extend(
            items
                .generated_boxes
                .into_iter()
                .flat_map(BTreeMap::keys)
                .flat_map(|&(root, node, _)| [root, node]),
        );
        sources.extend(ids(&mut projection
            .column_rules
            .get(&page_index)
            .into_iter()
            .flat_map(BTreeMap::keys)
            .copied()));
        sources.extend(ids(&mut projection
            .image_markers
            .get(&page_index)
            .into_iter()
            .flat_map(BTreeMap::keys)
            .copied()));
        sources.extend(ids(&mut items.overflow_clips.keys().copied()));
        // An opacity group is listed wherever its element is visited, even
        // when nothing inside it is on the page.
        sources.extend(projection.opacity_layers.iter().copied());

        let mut entered = HashSet::new();
        for source in sources {
            // Generated content and anonymous table cells are painted
            // within the subtree of the element that owns them.
            let owner = crate::generated_content::generated_origin(source)
                .map_or_else(|| self.ifc_source_owner(source), |(owner, _)| owner);
            self.get_node(owner)?;
            let mut current = Some(owner);
            while let Some(id) = current {
                if !entered.insert(id) {
                    break;
                }
                current = self.parent_of(id);
            }
        }
        Some(entered)
    }

    /// Multicolumn containers that may need unprojected legacy column-height
    /// clips, each with the reason.
    ///
    /// Explicit fragmentainer clips are projected by
    /// [`Document::page_paint_order`]. Legacy column-height clips synthesized
    /// by the built-in painter are not all represented by layout fragments,
    /// so multicolumn containers retain a conservative approximation warning.
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
            .map(|node_id| (NodeId::new(node_id as u64), LEGACY_COLUMN_CLIPS))
            .collect()
    }
}

/// Lists a paragraph's inline element boxes, then its text, in document order.
///
/// The painter draws a paragraph from its lines: the backgrounds of inline
/// elements first, then the text. Boxes laid out beside the lines (floats and
/// atomic inlines) are visited as the paragraph root's children instead, so
/// their subtrees are skipped here.
///
/// With `group`, only the content held by that inline opacity group is
/// listed: the group element's own boxes and generated boxes, and its
/// descendants'. Either way the subtree of each inline opacity group nested
/// directly in the listed content is skipped, and those groups are returned
/// in document order for the caller to list after this content.
fn push_paragraph<'a>(
    document: &'a Document,
    items: &PageItems<'a>,
    root: usize,
    group: Option<usize>,
    lines: Option<&TextLines>,
    events: &mut Vec<PaintEvent<'a>>,
) -> Vec<usize> {
    let Some(root_node) = document.ifc_layout_node(root) else {
        return Vec::new(); // cov:ignore: paragraph roots come from the arena
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
    let push_lines = |events: &mut Vec<PaintEvent<'a>>| {
        let Some(lines) = lines else { return };
        match group {
            Some(group) => {
                let owner = NodeId::new(group as u64);
                events.extend(
                    lines
                        .groups
                        .get(&owner)
                        .into_iter()
                        .flatten()
                        .filter(|line| line.root.0 == root as u64)
                        .map(|&line| PaintEvent::GroupTextLine(line, owner)),
                );
            }
            None => events.extend(
                lines
                    .paragraphs
                    .get(&NodeId::new(root as u64))
                    .into_iter()
                    .flatten()
                    .copied()
                    .map(PaintEvent::TextLine),
            ),
        }
    };
    if root_node.kind() == NodeKind::Text {
        if lines.is_some() {
            push_lines(events);
        } else {
            push_kind(events, root, PageFragmentKind::Text);
        }
        return Vec::new();
    }
    let beside_lines: HashSet<usize> = root_node.ifc_boxes().into_iter().collect();
    // The listed content starts at the group element itself, or inside the
    // paragraph root around the root's own generated content.
    let (first, children) = match group {
        Some(group) => {
            let Some(node) = document.get_node(group) else {
                return Vec::new(); // cov:ignore: groups are arena elements
            };
            (group, node.children.as_slice())
        }
        None => (root, root_node.children.as_slice()),
    };
    let mut elements = Vec::new();
    if group.is_some() {
        elements.push((first, None));
    }
    elements.push((first, Some(false)));
    let mut nested = Vec::new();
    let mut texts = Vec::new();
    let mut stack: Vec<_> = children.iter().rev().map(|&id| (id, false)).collect();
    while let Some((node_id, after)) = stack.pop() {
        if after {
            elements.push((node_id, Some(true)));
            continue;
        }
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
                if items
                    .cascade
                    .computed
                    .get(node_id)
                    .is_some_and(is_inline_opacity_group)
                {
                    nested.push(node_id);
                    continue;
                }
                elements.push((node_id, None));
                elements.push((node_id, Some(false)));
                stack.push((node_id, true));
                stack.extend(node.children.iter().rev().map(|&id| (id, false)));
            }
            NodeKind::Text => texts.push(node_id),
            _ => {}
        }
    }
    elements.push((first, Some(true)));
    for (node_id, after) in elements {
        match after {
            None if items
                .cascade
                .computed
                .get(node_id)
                .is_some_and(paint_rules::generates_box) =>
            {
                push_kind(events, node_id, PageFragmentKind::Box);
            }
            None => {}
            Some(after) => {
                let pseudo = if after {
                    PseudoElem::After
                } else {
                    PseudoElem::Before
                };
                for piece in items
                    .generated_boxes
                    .and_then(|boxes| boxes.get(&(root, node_id, after)))
                    .into_iter()
                    .flatten()
                {
                    let owner = NodeId::new(node_id as u64);
                    events.push(PaintEvent::GeneratedBox(GeneratedBox {
                        owner,
                        line: crate::TextLineId {
                            root: NodeId::new(root as u64),
                            index: piece.line,
                        },
                        kind: if after {
                            crate::GeneratedKind::After
                        } else {
                            crate::GeneratedKind::Before
                        },
                        rect: piece.rect,
                        style: &items.cascade.pseudo[&(StyleNodeId::new(node_id as u64), pseudo)],
                        clip_owner: owner,
                        has_start_edge: piece.has_start_edge,
                        has_end_edge: piece.has_end_edge,
                    }));
                }
            }
        }
    }
    if lines.is_some() {
        push_lines(events);
    } else {
        for node_id in texts {
            push_kind(events, node_id, PageFragmentKind::Text);
        }
    }
    nested
}

/// Whether an element inside a paragraph composites its fragments on the
/// paragraph's lines as one opacity group.
fn is_inline_opacity_group(cv: &raikiri_style::ComputedValues) -> bool {
    cv.display == DisplayValue::Inline && paint_rules::opacity_layer(cv).is_some()
}

#[cfg(test)]
mod tests;
