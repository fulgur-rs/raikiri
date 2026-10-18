//! A page's body content as a sequence of paint steps, in the order the
//! built-in painter draws it.

use super::generated_boxes::{GeneratedBox, LineBox, PageGeneratedBoxes, PageInlineBoxes};
use super::generated_images::{GeneratedImageId, PageGeneratedImages};
use super::records::{PageFragmentItem, PageFragmentKind};
use crate::{
    Document, Fragment, GeneratedKind, Node, OverflowClip, PositionedGlyphRun, RunSource,
    TextLineId, paint_rules,
};
use raikiri_style::property::{ColumnCountValue, DisplayValue, PositionValue};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::{NodeId, NodeKind, PaintClip, PaintRect, PaintTransform};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

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
    /// Paint everything until the matching [`PaintEvent::PopTransform`]
    /// through this transform, after the enclosing ones: the element's
    /// relative offset that layout leaves to paint, and its `transform`.
    /// Inside it, fragments, clips and the other events keep their layout
    /// coordinates. Elements inside columns are not moved this way; there
    /// the text runs carry the offsets instead.
    PushTransform(PaintTransform),
    /// Close the innermost transform.
    PopTransform,
    /// An element's background, borders and outline. For a replaced element
    /// the fragment's kind is `Replaced` and the same fragment follows in a
    /// [`PaintEvent::Replaced`] event. An element that is not `visible` has
    /// none: the decoration of its box is not listed, though its descendants
    /// that are visible list theirs.
    Box(Fragment<'a>),
    /// The outer `box-shadow`s of a table row or row group of the separated
    /// borders model, whose [`PaintEvent::Box`] events follow inside
    /// [`ClipKind::TableCell`] clips. Only the shadows that are not `inset`
    /// are painted here, once around the whole box, as no cell clip reaches
    /// outside the border box where they show (CSS Backgrounds 3 §7.1).
    OuterBoxShadow(Fragment<'a>),
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
    /// canvas). Follows the element's [`PaintEvent::Box`], if it lists one.
    /// An element that is not `visible` still lists it: its image has no
    /// raster image to paint, but an SVG root keeps its vector source so that
    /// descendants that set `visibility:visible` can paint. A table-display
    /// element that is not visible lists nothing, as a hidden table does.
    Replaced(Fragment<'a>),
    /// Prepared raster marker of a list item. Its placement is owned by this page.
    MarkerImage(NodeId),
    /// One generated inline box decoration; its text follows in paragraph lines.
    GeneratedBox(GeneratedBox<'a>),
    /// One line's piece of a non-atomic inline element's box, listed in
    /// place of the element's [`PaintEvent::Box`] where a translucent
    /// `::first-line` splits the element: its pieces on the first line are
    /// inside the first line's group, and the others are with the rest of
    /// the paragraph. An element on the first line to which the
    /// `::first-line` gives a color or font size other than its own is
    /// listed by its pieces too, even if it has no box on later lines, so
    /// that its piece on the first line carries the values in
    /// [`LineBox::style`]. Paint each as the element's box over the piece's
    /// rectangle, with the borders of the start and end edges it has.
    LineBox(LineBox<'a>),
    /// One image of `::before` or `::after` content placed on this page,
    /// after the decoration of the pseudo-element that holds it. Its URL and
    /// rectangle come from [`Document::page_generated_image`].
    GeneratedImage(GeneratedImageId),
    /// A producer-computed column rule, above the owner's border and below content.
    ColumnRule(crate::ColumnRule),
}

/// The reason reported for a multi-column container by
/// [`Document::paint_order_approximations`].
const LEGACY_COLUMN_CLIPS: &str = "legacy column clipping may be incomplete";

/// The reason reported for a paragraph by
/// [`Document::paint_order_approximations`] when a `::first-letter` in it
/// forms more opacity groups than [`generated_content::MAX_LETTER_GROUPS`]:
/// fragments past the limit composite with the opacity of another group of
/// the same `::first-letter`, or without their own.
///
/// [`generated_content::MAX_LETTER_GROUPS`]: crate::generated_content::MAX_LETTER_GROUPS
const LETTER_GROUP_LIMIT: &str = "::first-letter opacity groups past the id limit are approximated";

#[derive(Clone, Copy)]
enum Frame {
    Visit(usize),
    Paragraph(usize),
    /// An anonymous cell's paragraph in a table part that defers: its
    /// content waits for the end of the layer, in tree order with the other
    /// cells'.
    DeferredParagraph(usize),
    /// The inline content of a paragraph root, listed where this frame pops:
    /// at the end of the enclosing layer when the block defers it (see
    /// [`paint_rules::defers_inline_content`]).
    Content(usize),
    /// Close a layer of deferred inline content: list what its blocks
    /// deferred, then drop the layer.
    EndDeferral,
    /// Reopen the overflow clip of a block around the content its layer
    /// handed to the enclosing one (see
    /// [`paint_rules::merges_clipped_inline_content`]).
    PushClip(usize),
    /// An inline opacity group of a paragraph, by paragraph root and element,
    /// with the lines it lists.
    InlineGroup(usize, usize, LinePart),
    /// The group of a translucent `::first-line` around the first line of a
    /// paragraph, by paragraph root.
    FirstLine(usize),
    /// List the paragraph content held at this index, after the stacking
    /// contexts with negative stack levels painted below it.
    HeldContent(usize),
    PopClip,
    PopOpacity,
    PopTransform,
}

/// How an open layer ends once its subtree is listed.
#[derive(Clone, Copy)]
enum LayerClose {
    /// List what the layer collected.
    Paint,
    /// Hand what the layer collected to the enclosing layer, inside the
    /// overflow clip of the block, by node, that opened it.
    Merge(usize),
    /// List the deferred inline content, and hand the positioned boxes to
    /// the enclosing layer.
    Hoist,
}

/// An open layer of the paint walk: the inline content its plain blocks
/// defer and the positioned boxes that wait for its end, each in tree
/// order, whether blocks inside it may defer (the content of a block in
/// columns stays with its column clip, as in the painter), and how it ends.
struct Layer {
    pending: Vec<Frame>,
    positioned: Vec<Frame>,
    allowed: bool,
    close: LayerClose,
    /// Whether the layer's end is listing its positioned boxes: the boxes it
    /// pops then list where they pop rather than waiting again.
    flushing: bool,
    /// The positioned boxes still to list once the layer's end is listing
    /// them.
    queue: Option<paint_rules::PositionedQueue<Frame>>,
}

/// How the queue of a layer's positioned boxes reads its frames.
fn positioned_frame(frame: &Frame) -> paint_rules::PositionedFrame {
    match frame {
        Frame::Visit(node_id) | Frame::InlineGroup(_, node_id, _) => {
            paint_rules::PositionedFrame::Box(*node_id)
        }
        Frame::PushClip(owner) => paint_rules::PositionedFrame::Open(*owner),
        _ => paint_rules::PositionedFrame::Close,
    }
}

impl Layer {
    fn new(allowed: bool, close: LayerClose) -> Self {
        Self {
            pending: Vec::new(),
            positioned: Vec::new(),
            allowed,
            close,
            flushing: false,
            queue: None,
        }
    }
}

/// The lines of a paragraph a listing of its content covers. A translucent
/// `::first-line` lists the first line in a group of its own, apart from
/// the rest (see [`paint_rules::first_line_opacity_layer`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinePart {
    All,
    First,
    Rest,
}

impl LinePart {
    /// Whether the listing covers line `index` of the paragraph.
    fn admits(self, index: usize) -> bool {
        match self {
            Self::All => true,
            Self::First => index == 0,
            Self::Rest => index != 0,
        }
    }
}

/// The supplied lines of a page's paragraphs, split by the inline opacity
/// group that holds their runs.
#[derive(Default)]
struct TextLines {
    /// Lines with runs outside every inline opacity group, by paragraph root.
    paragraphs: HashMap<NodeId, Vec<TextLineId>>,
    /// Lines with runs in an inline opacity group, by the group's element.
    groups: HashMap<NodeId, Vec<TextLineId>>,
    /// The `::first-letter` groups with runs, by paragraph root and group,
    /// with the group that holds their letters.
    letters: HashMap<(usize, usize), Option<usize>>,
}

mod event_clips;
mod letters;
mod walk;

pub(super) use letters::LetterGroupCache;
use letters::LetterGroups;

/// The elements of a paragraph with inline boxes on its first line and on
/// the lines after it.
#[derive(Default)]
struct LineBoxes {
    first: HashSet<usize>,
    later: HashSet<usize>,
}

/// The fragments of one page, grouped by source node.
struct PageItems<'a> {
    by_node: HashMap<usize, Vec<&'a PageFragmentItem>>,
    content_box: super::records::PageFragmentRect,
    overflow_clips: BTreeMap<NodeId, OverflowClip>,
    generated_boxes: Option<&'a PageGeneratedBoxes>,
    inline_boxes: Option<&'a PageInlineBoxes>,
    generated_images: Option<&'a PageGeneratedImages>,
    cascade: &'a CascadeResult,
    document: &'a Document,
    page: &'a super::records::PageFragment,
    /// The elements with inline boxes on the first line and on lines after
    /// the first, by paragraph root, built when a paragraph is first split
    /// by line.
    line_boxes: RefCell<HashMap<usize, Rc<LineBoxes>>>,
}

impl<'a> PageItems<'a> {
    /// The `::first-letter` opacity groups of paragraph `root`.
    fn letter_groups(&self, root: usize) -> Arc<LetterGroups> {
        self.document.cached_letter_groups(self.cascade, root)
    }

    /// The elements of paragraph `root` with inline boxes on the first line
    /// and on lines after the first.
    fn line_boxes(&self, root: usize, root_node: &Node) -> Rc<LineBoxes> {
        Rc::clone(self.line_boxes.borrow_mut().entry(root).or_insert_with(|| {
            let mut boxes = LineBoxes::default();
            for piece in root_node.ifc_inline_boxes().unwrap_or_default() {
                if piece.line == 0 {
                    boxes.first.insert(piece.node);
                } else {
                    boxes.later.insert(piece.node);
                }
            }
            Rc::new(boxes)
        }))
    }

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
            // The letters of a `::first-letter` group, its own or that of an
            // ancestor block's first letter around them.
            if let (Some(_), Some(Ok(group)), Ok(root)) = (
                run.letter_box,
                group.map(|group| usize::try_from(group.0)),
                usize::try_from(run.line.root.0),
            ) && crate::generated_content::generated_origin(group)
                .is_some_and(|(_, pseudo)| pseudo == PseudoElem::FirstLetter)
            {
                lines
                    .letters
                    .entry((root, group))
                    .or_insert_with(|| self.cached_letter_groups(cascade, root).around(group));
            }
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
    ///
    /// A `::before`, `::after` or `::first-letter` laid out in the paragraph
    /// is such a group too, named by the id of
    /// [`crate::generated_content::generated_node_id`]: its letters and
    /// generated text belong to it before any element around them. An
    /// atomic `::before` or `::after` with a transform is a group too, turned
    /// as a whole (see [`paint_rules::pseudo_transform`]). The
    /// fragments of a `::first-letter` that keep different opacities, or lie
    /// in different groups, form one group each, the others named by ids of
    /// [`crate::generated_content::first_letter_group_id`].
    #[doc(hidden)]
    pub fn text_run_opacity_group(
        &self,
        cascade: &CascadeResult,
        run: &PositionedGlyphRun<'_>,
    ) -> Option<NodeId> {
        // The paragraph retains the fragment of every run a letter box
        // holds.
        if let Some((letter, owner)) = run.letter_box
            && let Some(group) = usize::try_from(run.line.root.0).ok().and_then(|root| {
                self.cached_letter_groups(cascade, root)
                    .nearest(letter, owner)
            })
        {
            return group.map(|group| NodeId::new(group as u64));
        }
        self.run_group_outside_letter(cascade, run)
    }

    /// The opacity group of `run`, text outside every `::first-letter`
    /// group.
    fn run_group_outside_letter(
        &self,
        cascade: &CascadeResult,
        run: &PositionedGlyphRun<'_>,
    ) -> Option<NodeId> {
        let root = usize::try_from(run.line.root.0).ok()?;
        let start = match run.source {
            RunSource::Text(node) => self.parent_of(usize::try_from(node.0).ok()?)?,
            RunSource::Generated(owner, kind @ (GeneratedKind::Before | GeneratedKind::After)) => {
                let owner = usize::try_from(owner.0).ok()?;
                if let Some(group) = generated_group(self, cascade, root, owner, kind) {
                    return Some(NodeId::new(group as u64));
                }
                owner
            }
            _ => return None,
        };
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
        for id in self.inclusive_ancestors(start) {
            if id == root || id == owner {
                return None;
            }
            let node = self.get_node(id)?;
            if node.kind() != NodeKind::Element {
                return None;
            }
            let cv = cascade.computed.get(id)?;
            if is_inline_group(self, cascade, root, id, node, cv) {
                return Some(id);
            }
            // Anything but an inline element or a `display: contents` one is
            // a block boundary: the walk has left the paragraph.
            if !(node.display().is_inline_flow() || node.display() == DisplayValue::Contents) {
                return None;
            }
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
        let Some(root) = self.document_element_index() else {
            return Vec::new(); // cov:ignore: a laid-out page always has a document element
        };
        let mut items = PageItems {
            by_node: HashMap::new(),
            content_box: page.content_box,
            overflow_clips: self.overflow_clips_on_page(page),
            generated_boxes: self.page_projection.generated_boxes.get(&page_index),
            inline_boxes: self.page_projection.inline_boxes.get(&page_index),
            generated_images: self.page_projection.generated_images.get(&page_index),
            cascade,
            document: self,
            page,
            line_boxes: RefCell::default(),
        };
        for item in &page.items {
            if let Ok(node_id) = usize::try_from(item.node_id.0) {
                items.by_node.entry(node_id).or_default().push(item);
            }
        }

        let entered = skip_empty
            .then(|| self.page_paint_nodes(&items, page_index, lines))
            .flatten();

        let events =
            walk::PaintWalk::new(self, cascade, &items, lines, page_index, entered, root).run();
        event_clips::clip_events(self, cascade, &items, events)
    }

    /// The boxes laid out beside the lines of paragraph `root` (floats and
    /// atomic inlines) whose innermost inline opacity group is `group`.
    /// Whether paragraph root `node_id` places its raster marker on this page
    /// inside its first line, so the marker paints with its lines.
    fn has_inline_marker_image(
        &self,
        cascade: &CascadeResult,
        page_index: u32,
        node_id: usize,
    ) -> bool {
        self.page_marker_image(page_index, NodeId::new(node_id as u64))
            .is_some()
            && crate::generated_content::inside_marker_in_flow(cascade, node_id)
            && self
                .get_node(node_id)
                .is_some_and(|node| node.is_ifc_root())
    }

    /// The frame that visits paint child `key`: an anonymous paragraph key
    /// stands for a paragraph laid out outside the source DOM arena.
    fn child_frame(&self, key: usize) -> Frame {
        if self.is_anonymous_cell(key) {
            Frame::Paragraph(key)
        } else {
            Frame::Visit(key)
        }
    }

    /// The frames of a paragraph's boxes laid out beside its lines and of its
    /// inline opacity groups, in paint order. A group is a stacking context
    /// painted at the level of positioned boxes with `z-index: auto` or `0`,
    /// in tree order with them: after the floats and in-flow atomic inlines,
    /// and before positive z-index boxes.
    ///
    /// The frames are split at the paragraph's own inline content: the
    /// stacking contexts with negative stack levels paint below it, the rest
    /// above it (CSS 2.1 Appendix E steps 3 and 7).
    ///
    /// The groups list the lines of `part`. With `first_line`, the group of
    /// the paragraph's translucent `::first-line` comes first among them.
    fn paragraph_frames(
        &self,
        cascade: &CascadeResult,
        root: usize,
        boxes: Vec<usize>,
        groups: Vec<usize>,
        part: LinePart,
        first_line: bool,
    ) -> ParagraphFrames {
        let display = self
            .nodes
            .layout_node(root)
            .map_or(DisplayValue::Block, |node| node.display());
        let mut entries: Vec<(paint_rules::PaintChildKey, Frame)> = boxes
            .into_iter()
            .map(|key| {
                (
                    paint_rules::paint_child_key(key, display, cascade),
                    self.child_frame(key),
                )
            })
            .collect();
        if first_line {
            entries.insert(
                0,
                (
                    paint_rules::STACKING_CONTEXT_PAINT_KEY,
                    Frame::FirstLine(root),
                ),
            );
        }
        if !groups.is_empty() {
            // Each group joins the boxes at its place in tree order before
            // the stable sort, so equal keys keep tree order.
            let position = self.preorder_positions(root);
            let mut groups: Vec<(Option<(usize, u8)>, usize)> = groups
                .into_iter()
                .map(|group| (self.group_tree_position(&position, group), group))
                .collect();
            // Pseudo-element groups are found after the element groups of
            // the same pass; tree order puts them among those.
            groups.sort_by_key(|&(at, _)| at.map_or((usize::MAX, u8::MAX), |at| at));
            for (group_at, group) in groups {
                let at = group_at.map_or(entries.len(), |group_at| {
                    entries
                        .iter()
                        .position(|(_, frame)| match *frame {
                            Frame::Visit(id) | Frame::Paragraph(id) => {
                                position.get(&id).is_some_and(|&at| (at, 0) > group_at)
                            }
                            Frame::InlineGroup(_, id, _) => self
                                .group_tree_position(&position, id)
                                .is_some_and(|at| at > group_at),
                            // The first line starts the paragraph.
                            Frame::FirstLine(_) => false,
                            Frame::PopClip
                            | Frame::PopOpacity
                            | Frame::PopTransform
                            | Frame::Content(_)
                            | Frame::HeldContent(_)
                            | Frame::EndDeferral
                            | Frame::PushClip(_)
                            | Frame::DeferredParagraph(_) => false, // cov:ignore: never listed here
                        })
                        .unwrap_or(entries.len())
                });
                // A positioned group keeps its own z-index level.
                let key = match crate::generated_content::generated_origin(group) {
                    Some(_) => crate::generated_content::computed_for_id(cascade, group)
                        .map_or(paint_rules::STACKING_CONTEXT_PAINT_KEY, |cv| {
                            paint_rules::stacking_context_key(cv)
                        }),
                    None => match self.get_node(group).map(|node| node.position()) {
                        Some(PositionValue::Static) | None => {
                            paint_rules::STACKING_CONTEXT_PAINT_KEY
                        }
                        Some(_) => paint_rules::paint_child_key(group, display, cascade),
                    },
                };
                entries.insert(at, (key, Frame::InlineGroup(root, group, part)));
            }
        }
        entries.sort_by_key(|(key, _)| *key);
        let split = entries.partition_point(|(((bucket, _), _, _), _)| *bucket == 0);
        let above = entries.split_off(split);
        ParagraphFrames {
            below: entries.into_iter().map(|(_, frame)| frame).collect(),
            blocks: Vec::new(),
            above: above.into_iter().map(|(_, frame)| frame).collect(),
        }
    }

    /// Where opacity group `group` stands in the tree order of a paragraph
    /// whose preorder positions are `position`: an element at its own
    /// position, a `::before` or `::first-letter` just after its originating
    /// element, and an `::after` after the element's last descendant. The
    /// second number orders the pieces that share a position.
    #[doc(hidden)]
    pub fn group_tree_position(
        &self,
        position: &HashMap<usize, usize>,
        group: usize,
    ) -> Option<(usize, u8)> {
        let Some((owner, pseudo)) = crate::generated_content::generated_origin(group) else {
            return position.get(&group).map(|&at| (at, 0));
        };
        match pseudo {
            PseudoElem::Before => position.get(&owner).map(|&at| (at, 1)),
            // A first letter can belong to a block around the paragraph,
            // which then starts the paragraph's tree order.
            PseudoElem::FirstLetter => Some((position.get(&owner).copied().unwrap_or(0), 2)),
            _ => {
                let mut last = owner;
                while let Some(&child) = self
                    .get_node(last)
                    .and_then(|node| node.children.last())
                    .filter(|child| position.contains_key(child))
                {
                    last = child;
                }
                position.get(&last).map(|&at| (at, 3))
            }
        }
    }

    /// The preorder index of each node in the subtree of `root`.
    #[doc(hidden)]
    pub fn preorder_positions(&self, root: usize) -> HashMap<usize, usize> {
        self.preorder(root)
            .enumerate()
            .map(|(position, id)| (id, position))
            .collect()
    }

    /// The `groups` of paragraph `root` that list anything on the lines of
    /// `part`: all of them when `part` covers every line.
    fn groups_with_content(
        &self,
        items: &PageItems<'_>,
        root: usize,
        mut groups: Vec<usize>,
        lines: Option<&TextLines>,
        part: LinePart,
    ) -> Vec<usize> {
        if part != LinePart::All {
            groups.retain(|&group| self.group_has_content(items, root, group, lines, part));
        }
        groups
    }

    /// Whether inline opacity group `group` of paragraph `root`, or a group
    /// nested in it, lists anything on the lines of `part`, or a box laid
    /// out beside them.
    fn group_has_content(
        &self,
        items: &PageItems<'_>,
        root: usize,
        group: usize,
        lines: Option<&TextLines>,
        part: LinePart,
    ) -> bool {
        // Nested groups are walked with a stack, as deep as the document.
        let mut pending = vec![group];
        while let Some(group) = pending.pop() {
            let mut content = Vec::new();
            let nested = push_paragraph(self, items, root, Some(group), lines, part, &mut content);
            // A first line on another page lists none of its boxes here.
            if !content.is_empty()
                || self
                    .ifc_boxes_in_group(items.cascade, root, Some(group), part)
                    .iter()
                    .any(|id| part != LinePart::First || items.by_node.contains_key(id))
            {
                return true;
            }
            pending.extend(nested);
        }
        false
    }

    /// The in-flow blocks inside the inline content of paragraph `root`
    /// that paint before its lines (see
    /// [`paint_rules::paints_before_paragraph_lines`]), in tree order.
    fn paragraph_blocks(&self, cascade: &CascadeResult, root: usize) -> Vec<usize> {
        self.get_node(root)
            .map(|node| node.ifc_boxes())
            .unwrap_or_default()
            .into_iter()
            .filter(|&id| paint_rules::paints_before_paragraph_lines(self, cascade, root, id))
            .collect()
    }

    /// The boxes of paragraph `root` laid out beside its lines whose
    /// innermost inline opacity group is `group`, and that belong to the
    /// lines of `part`: an atomic inline to the line it is on, any other box
    /// to the lines after the first.
    fn ifc_boxes_in_group(
        &self,
        cascade: &CascadeResult,
        root: usize,
        group: Option<usize>,
        part: LinePart,
    ) -> Vec<usize> {
        let Some(node) = self.get_node(root) else {
            return Vec::new(); // cov:ignore: element paragraph roots are arena nodes
        };
        let mut boxes = node.ifc_boxes();
        let blocks: HashSet<usize> = self.paragraph_blocks(cascade, root).into_iter().collect();
        boxes.retain(|&id| {
            !blocks.contains(&id)
                && self
                    .parent_of(id)
                    .and_then(|parent| self.inline_opacity_group(cascade, root, parent))
                    == group
                && match part {
                    LinePart::All => true,
                    LinePart::First => paint_rules::atomic_on_first_line(self, root, id),
                    LinePart::Rest => !paint_rules::atomic_on_first_line(self, root, id),
                }
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
        sources.extend(
            items
                .generated_images
                .into_iter()
                .flat_map(BTreeMap::iter)
                .flat_map(|(&(owner, _, _), placement)| [placement.root, owner]),
        );
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
            for id in self.inclusive_ancestors(owner) {
                if !entered.insert(id) {
                    break;
                }
            }
        }
        Some(entered)
    }

    /// Multicolumn containers that may need unprojected legacy column-height
    /// clips, and paragraphs whose `::first-letter` opacity groups are
    /// approximated, each with the reason.
    ///
    /// Explicit fragmentainer clips are projected by
    /// [`Document::page_paint_order`]. Legacy column-height clips synthesized
    /// by the built-in painter are not all represented by layout fragments,
    /// so multicolumn containers retain a conservative approximation warning.
    ///
    /// A `::first-letter` whose fragments need more opacity groups than an
    /// id can name (see [`crate::generated_content::first_letter_group_id`])
    /// lists the later fragments in another group of the same pseudo-element,
    /// or in none of their own, so their opacity is not the one the built-in
    /// painter composites them with.
    #[doc(hidden)]
    pub fn paint_order_approximations(
        &self,
        cascade: &CascadeResult,
    ) -> Vec<(NodeId, &'static str)> {
        let is_multicol = |node_id: usize| {
            self.get_node(node_id)
                .is_some_and(|node| node.is_multicol_container())
                && cascade.computed.get(node_id).is_some_and(
                    |cv| matches!(cv.column_count, ColumnCountValue::Count(n) if n >= 2),
                )
        };
        // Whether the node and all its ancestors are in the document and
        // displayed.
        let is_rendered = |node_id: usize| {
            self.inclusive_ancestors(node_id).all(|id| {
                self.get_node(id).is_some_and(|node| {
                    node.is_in_document()
                        && !(node.kind() == NodeKind::Element
                            && (node.is_display_none() || node.is_hidden_by_text_overflow()))
                })
            })
        };
        let mut found: Vec<usize> = (0..self.nodes.len())
            .filter(|&node_id| {
                self.get_node(node_id)
                    .is_some_and(|node| node.kind() == NodeKind::Element)
                    && is_multicol(node_id)
                    && is_rendered(node_id)
                    && !self.ancestors(node_id).any(is_multicol)
            })
            .collect();
        found.sort_unstable();
        let mut approximations: Vec<_> = found
            .into_iter()
            .map(|node_id| (NodeId::new(node_id as u64), LEGACY_COLUMN_CLIPS))
            .collect();
        approximations.extend(
            (0..self.nodes.len())
                .filter(|&root| {
                    self.get_node(root).is_some_and(|node| {
                        node.is_ifc_root() && node.ifc_letter_fragments().next().is_some()
                    }) && is_rendered(root)
                        && self.cached_letter_groups(cascade, root).approximated
                })
                .map(|root| (NodeId::new(root as u64), LETTER_GROUP_LIMIT)),
        );
        approximations
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
///
/// Only the content on the lines of `part` is listed. An inline element
/// only on the first line, or only on later lines, lists its box with
/// those lines; one on both lists its line pieces on the lines of `part`.
fn push_paragraph<'a>(
    document: &'a Document,
    items: &PageItems<'a>,
    root: usize,
    group: Option<usize>,
    lines: Option<&TextLines>,
    part: LinePart,
    events: &mut Vec<PaintEvent<'a>>,
) -> Vec<usize> {
    let Some(root_node) = document.ifc_layout_node(root) else {
        return Vec::new(); // cov:ignore: paragraph roots come from the arena
    };
    // The elements with boxes on the first line and on lines after it.
    let lines_of_boxes = if part == LinePart::All {
        Rc::default()
    } else {
        items.line_boxes(root, root_node)
    };
    let later = &lines_of_boxes.later;
    let admits_box = |element: usize| match part {
        LinePart::All => true,
        LinePart::First => !later.contains(&element),
        LinePart::Rest => later.contains(&element),
    };
    // An element on the first line and on later ones is split by line: its
    // pieces on the lines of `part` stand for its box. So are the pieces of
    // an element whose color or font size the `::first-line` changes, even
    // where it has no box on later lines. Without prepared pieces on the
    // page, its box goes with the other lines.
    let line_pieces = |element: usize| {
        let pieces = items.inline_boxes?.get(&(root, element))?;
        ((part != LinePart::All
            && later.contains(&element)
            && lines_of_boxes.first.contains(&element))
            || crate::layout::first_line_restyles(root_node, items.cascade, element))
        .then_some(pieces)
    };
    let push_line_boxes = |events: &mut Vec<PaintEvent<'a>>,
                           element: usize,
                           pieces: &[super::generated_boxes::BoxPiece]| {
        let boxes: Vec<Fragment<'a>> = items
            .of(element)
            .iter()
            .filter(|item| item.kind == PageFragmentKind::Box)
            .map(|&item| items.fragment(item))
            .collect();
        // The pieces come in line order and the boxes in flow order, so the
        // search for each piece's box resumes where the last one ended.
        let mut cursor = 0;
        for piece in pieces.iter().filter(|piece| part.admits(piece.line)) {
            // The element's box on the page that holds the piece, such as
            // its box in the piece's column; a piece outside every box later
            // in the flow goes with the box of the last piece.
            if let Some(next) = boxes[cursor..]
                .iter()
                .position(|fragment| contains(fragment.paint_rect(), piece.rect))
            {
                cursor += next;
            }
            let Some(&fragment) = boxes.get(cursor) else {
                return; // cov:ignore: an element with line pieces on the page has a box there
            };
            // The first line's boxes take their values from the
            // `::first-line` around them.
            let style = root_node
                .ifc_first_line_element_style(element)
                .filter(|_| piece.line == 0)
                .unwrap_or(&items.cascade.computed[element]);
            events.push(PaintEvent::LineBox(LineBox {
                fragment,
                style,
                line: TextLineId {
                    root: NodeId::new(root as u64),
                    index: piece.line,
                },
                rect: piece.rect,
                has_start_edge: piece.has_start_edge,
                has_end_edge: piece.has_end_edge,
            }));
        }
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
                        .filter(|line| line.root.0 == root as u64 && part.admits(line.index))
                        .map(|&line| PaintEvent::GroupTextLine(line, owner)),
                );
            }
            None => events.extend(
                lines
                    .paragraphs
                    .get(&NodeId::new(root as u64))
                    .into_iter()
                    .flatten()
                    .filter(|line| part.admits(line.index))
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
    if let Some(group) = group
        && crate::generated_content::generated_origin(group).is_some()
    {
        return push_pseudo_group(
            document,
            items,
            root,
            group,
            lines,
            part,
            &push_lines,
            events,
        );
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
    elements.push((first, Some(GeneratedKind::Before)));
    // A `::first-letter` box wraps letters inside the element they come
    // from, so it follows that element's box and `::before` box, or its
    // `::after` box when the letters are that pseudo-element's.
    elements.push((first, Some(GeneratedKind::FirstLetter)));
    let mut nested = Vec::new();
    let mut texts = Vec::new();
    let mut stack: Vec<_> = children.iter().rev().map(|&id| (id, false)).collect();
    while let Some((node_id, after)) = stack.pop() {
        if after {
            elements.push((node_id, Some(GeneratedKind::After)));
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
                if items.cascade.computed.get(node_id).is_some_and(|cv| {
                    is_inline_group(document, items.cascade, root, node_id, node, cv)
                }) {
                    nested.push(node_id);
                    continue;
                }
                elements.push((node_id, None));
                elements.push((node_id, Some(GeneratedKind::Before)));
                elements.push((node_id, Some(GeneratedKind::FirstLetter)));
                stack.push((node_id, true));
                stack.extend(node.children.iter().rev().map(|&id| (id, false)));
            }
            NodeKind::Text => texts.push(node_id),
            _ => {}
        }
    }
    elements.push((first, Some(GeneratedKind::After)));
    let root_node_ifc = document.ifc_layout_node(root);
    // The `::first-letter` pieces this pass lists: those whose letters its
    // inline opacity group holds, as it holds their glyph runs. A
    // `::first-letter` box belongs to the block whose first letter it is,
    // which can be an ancestor of the paragraph. Pieces whose letters come
    // from no element listed here, such as the root's own text, go first.
    let listed: HashSet<usize> = elements
        .iter()
        .filter(|&&(id, kind)| kind.is_none() && id != first)
        .map(|&(id, _)| id)
        .collect();
    let pieces = first_letter_pieces(document, items, root);
    // A first letter with an opacity of its own is a group nested in the
    // group that holds its letters.
    for own in letter_groups(&pieces, lines, root, group) {
        if !nested.contains(&own) {
            nested.push(own);
        }
    }
    // Each first-letter piece with the element it is listed under, whether
    // its letters come from `::after`, its owner, and the element that holds
    // its letters, which clips it.
    let letters: Vec<(usize, bool, usize, usize, &super::generated_boxes::BoxPiece)> = pieces
        .iter()
        .filter(|letter| letter.own.is_none() && letter.held == group)
        .map(|letter| {
            let clip = letter.anchor.unwrap_or(letter.owner);
            let anchor = letter
                .anchor
                .filter(|anchor| listed.contains(anchor))
                .unwrap_or(first);
            (anchor, letter.after, letter.owner, clip, letter.piece)
        })
        .collect();
    for (node_id, kind) in elements {
        match kind {
            None if items
                .cascade
                .computed
                .get(node_id)
                .is_some_and(paint_rules::generates_box) =>
            {
                if let Some(pieces) = line_pieces(node_id) {
                    push_line_boxes(events, node_id, pieces);
                } else if admits_box(node_id) {
                    push_kind(events, node_id, PageFragmentKind::Box);
                }
            }
            None => {}
            Some(kind) => {
                // A pseudo-element with an opacity of its own is a group,
                // listed after this content with its boxes, images and text.
                // Its `::first-letter` slot still follows.
                if matches!(kind, GeneratedKind::Before | GeneratedKind::After)
                    && let Some(pseudo) =
                        generated_group(document, items.cascade, root, node_id, kind)
                {
                    if pseudo_on_paragraph(items, root, node_id, kind, lines)
                        && !nested.contains(&pseudo)
                    {
                        nested.push(pseudo);
                    }
                    continue;
                }
                let own = items
                    .generated_boxes
                    .and_then(|boxes| boxes.get(&(root, node_id, kind)))
                    .into_iter()
                    .flatten()
                    .filter(|_| kind != GeneratedKind::FirstLetter)
                    .map(|piece| (node_id, kind, node_id, piece));
                // The first letters held by this element's text, or by its
                // `::after` text, which come after the `::after` box.
                let held = letters
                    .iter()
                    .filter(|&&(anchor, after, _, _, _)| {
                        anchor == node_id
                            && match kind {
                                GeneratedKind::FirstLetter => !after,
                                GeneratedKind::After => after,
                                _ => false,
                            }
                    })
                    .map(|&(_, _, owner, clip, piece)| {
                        (owner, GeneratedKind::FirstLetter, clip, piece)
                    });
                for (owner, kind, clip, piece) in own.chain(held) {
                    if !part.admits(piece.line) {
                        continue;
                    }
                    let owner_id = NodeId::new(owner as u64);
                    let Some(style) = root_node_ifc
                        .and_then(|node| column_projection_style(node, items.cascade, piece))
                    else {
                        continue; // cov:ignore: pieces are prepared only for boxes with a style
                    };
                    events.push(PaintEvent::GeneratedBox(GeneratedBox {
                        owner: owner_id,
                        line: crate::TextLineId {
                            root: NodeId::new(root as u64),
                            index: piece.line,
                        },
                        kind,
                        rect: piece.rect,
                        style,
                        clip_owner: NodeId::new(clip as u64),
                        has_start_edge: piece.has_start_edge,
                        has_end_edge: piece.has_end_edge,
                        has_block_start_edge: piece.has_block_start_edge,
                        has_block_end_edge: piece.has_block_end_edge,
                    }));
                }
                // The `::before` images wait for the first-letter slot that
                // always follows, so that first-letter boxes come first.
                let after = match kind {
                    GeneratedKind::FirstLetter => false,
                    GeneratedKind::After => true,
                    _ => continue,
                };
                // The images of a `::before` group are listed in the group.
                if !after
                    && generated_group(
                        document,
                        items.cascade,
                        root,
                        node_id,
                        GeneratedKind::Before,
                    )
                    .is_some()
                {
                    continue;
                }
                // The images are replaced content inside the pseudo-element's
                // inline box, painted above its decoration and above the
                // first-letter boxes wrapping its letters, as the inline
                // boxes of a line are painted before its replaced content
                // (CSS 2.1 Appendix E, inline-level content in tree order).
                events.extend(
                    items
                        .generated_images
                        .into_iter()
                        .flat_map(|images| {
                            images.range((node_id, after, 0)..=(node_id, after, usize::MAX))
                        })
                        .filter(|(_, placement)| {
                            placement.root == root && part.admits(placement.line)
                        })
                        .map(|(&(owner, after, index), _)| {
                            PaintEvent::GeneratedImage(GeneratedImageId::new(owner, after, index))
                        }),
                );
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

/// The element of paragraph `root` whose letters a `::first-letter` piece
/// wraps: the parent of the text, or the originating element of the
/// `::before` or `::after` text, they come from; which of those
/// pseudo-elements, if any, the letters are the text of; and the node that
/// supplies the letters.
fn first_letter_anchor(
    document: &Document,
    root: usize,
    piece: &super::generated_boxes::BoxPiece,
) -> Option<(usize, Option<GeneratedKind>, usize)> {
    let source = document
        .ifc_layout_node(root)?
        .ifc_typographic_fragment(piece.node, piece.source_container, piece.source_owner)?
        .1;
    if let Some((element, pseudo)) = crate::generated_content::generated_origin(source) {
        let kind = match pseudo {
            PseudoElem::After => GeneratedKind::After,
            _ => GeneratedKind::Before,
        };
        return Some((element, Some(kind), source));
    }
    match document.get_node(source)?.kind() {
        NodeKind::Text => document
            .parent_of(source)
            .map(|parent| (parent, None, source)),
        _ => Some((source, None, source)), // cov:ignore: letters come from text or generated content
    }
}

/// One `::first-letter` piece of a paragraph, with the opacity groups
/// around it.
struct LetterPiece<'p> {
    /// The element whose letters the piece wraps, when known.
    anchor: Option<usize>,
    /// Whether the letters are the `::after` text of the anchor.
    after: bool,
    /// The block whose `::first-letter` the piece is.
    owner: usize,
    piece: &'p super::generated_boxes::BoxPiece,
    /// The innermost opacity group around the letters, other than the
    /// piece's own: a `::before` or `::after` group they are the text of,
    /// or the inline group of the anchor.
    held: Option<usize>,
    /// The piece's own opacity group, when its `::first-letter` has one.
    own: Option<usize>,
}

/// The `::first-letter` pieces of paragraph `root` on the page, in paint
/// order.
fn first_letter_pieces<'p>(
    document: &Document,
    items: &PageItems<'p>,
    root: usize,
) -> Vec<LetterPiece<'p>> {
    let groups = items.letter_groups(root);
    items
        .generated_boxes
        .into_iter()
        .flat_map(|boxes| {
            boxes.range(
                (root, 0, GeneratedKind::FirstLetter)
                    ..=(root, usize::MAX, GeneratedKind::FirstLetter),
            )
        })
        .filter(|((_, _, kind), _)| *kind == GeneratedKind::FirstLetter)
        .flat_map(|(&(_, owner, _), pieces)| pieces.iter().map(move |piece| (owner, piece)))
        .map(|(owner, piece)| {
            let source = first_letter_anchor(document, root, piece);
            let anchor = source.map(|(anchor, _, _)| anchor);
            let pseudo = source.and_then(|(_, pseudo, _)| pseudo);
            // The paragraph retains the fragment of every piece.
            let (own, held) = source
                .and_then(|(_, _, letters)| groups.fragment(piece.node, letters))
                .unwrap_or_default();
            LetterPiece {
                anchor,
                after: pseudo == Some(GeneratedKind::After),
                owner,
                piece,
                held,
                own,
            }
        })
        .collect()
}

/// Lists the content of the opacity group of a pseudo-element of paragraph
/// `root`, named by its generated id: the decoration of a `::before` or
/// `::after` with the first letters taken from its text and its images, or
/// the pieces of a `::first-letter`; then the group's text. Returns the
/// groups of the first letters nested in it.
#[allow(clippy::too_many_arguments)]
fn push_pseudo_group<'a>(
    document: &'a Document,
    items: &PageItems<'a>,
    root: usize,
    group: usize,
    lines: Option<&TextLines>,
    part: LinePart,
    push_lines: &dyn Fn(&mut Vec<PaintEvent<'a>>),
    events: &mut Vec<PaintEvent<'a>>,
) -> Vec<usize> {
    let Some((element, pseudo)) = crate::generated_content::generated_origin(group) else {
        return Vec::new(); // cov:ignore: the caller checks the id
    };
    let root_node = document.ifc_layout_node(root);
    let push_piece = |events: &mut Vec<PaintEvent<'a>>,
                      owner: usize,
                      kind: GeneratedKind,
                      clip: usize,
                      piece: &super::generated_boxes::BoxPiece| {
        if !part.admits(piece.line) {
            return;
        }
        let Some(style) =
            root_node.and_then(|node| column_projection_style(node, items.cascade, piece))
        else {
            return; // cov:ignore: pieces are prepared only for boxes with a style
        };
        events.push(PaintEvent::GeneratedBox(GeneratedBox {
            owner: NodeId::new(owner as u64),
            line: TextLineId {
                root: NodeId::new(root as u64),
                index: piece.line,
            },
            kind,
            rect: piece.rect,
            style,
            clip_owner: NodeId::new(clip as u64),
            has_start_edge: piece.has_start_edge,
            has_end_edge: piece.has_end_edge,
            has_block_start_edge: piece.has_block_start_edge,
            has_block_end_edge: piece.has_block_end_edge,
        }));
    };
    let letters = first_letter_pieces(document, items, root);
    let mut nested = Vec::new();
    match pseudo {
        PseudoElem::FirstLetter => {
            for letter in letters.iter().filter(|letter| letter.own == Some(group)) {
                let clip = letter.anchor.unwrap_or(letter.owner);
                push_piece(
                    events,
                    letter.owner,
                    GeneratedKind::FirstLetter,
                    clip,
                    letter.piece,
                );
            }
            // The first letter of a nested block wraps letters inside this
            // one's.
            nested = letter_groups(&letters, lines, root, Some(group));
        }
        PseudoElem::Before | PseudoElem::After => {
            let (kind, after) = if pseudo == PseudoElem::After {
                (GeneratedKind::After, true)
            } else {
                (GeneratedKind::Before, false)
            };
            for piece in items
                .generated_boxes
                .and_then(|boxes| boxes.get(&(root, element, kind)))
                .into_iter()
                .flatten()
            {
                push_piece(events, element, kind, element, piece);
            }
            // The first letters taken from the pseudo-element's text wrap
            // them inside its box, below its images.
            for letter in letters
                .iter()
                .filter(|letter| letter.own.is_none() && letter.held == Some(group))
            {
                push_piece(
                    events,
                    letter.owner,
                    GeneratedKind::FirstLetter,
                    letter.anchor.unwrap_or(letter.owner),
                    letter.piece,
                );
            }
            for own in letter_groups(&letters, lines, root, Some(group)) {
                if !nested.contains(&own) {
                    nested.push(own);
                }
            }
            events.extend(
                items
                    .generated_images
                    .into_iter()
                    .flat_map(|images| {
                        images.range((element, after, 0)..=(element, after, usize::MAX))
                    })
                    .filter(|(_, placement)| placement.root == root && part.admits(placement.line))
                    .map(|(&(owner, after, index), _)| {
                        PaintEvent::GeneratedImage(GeneratedImageId::new(owner, after, index))
                    }),
            );
        }
        _ => {} // cov:ignore: only these pseudo-elements form groups
    }
    push_lines(events);
    nested
}

/// The `::first-letter` groups of paragraph `root` nested directly in
/// `group`: those of the pieces whose letters it holds, and those of the
/// supplied runs, in that order.
fn letter_groups(
    pieces: &[LetterPiece<'_>],
    lines: Option<&TextLines>,
    root: usize,
    group: Option<usize>,
) -> Vec<usize> {
    let mut found: Vec<usize> = pieces
        .iter()
        .filter(|letter| letter.held == group)
        .filter_map(|letter| letter.own)
        .collect();
    let mut from_runs: Vec<usize> = lines
        .into_iter()
        .flat_map(|lines| &lines.letters)
        .filter(|&(&(letter_root, _), &held)| letter_root == root && held == group)
        .map(|(&(_, letter), _)| letter)
        .collect();
    from_runs.sort_unstable();
    found.extend(from_runs);
    let mut seen = HashSet::new();
    found.retain(|&group| seen.insert(group));
    found
}

/// The group the pseudo-element with generated id `id` forms in paragraph
/// `root`: its id, when it has one (see [`paint_rules::pseudo_forms_group`],
/// [`paint_rules::negative_pseudo_group`] and
/// [`paint_rules::pseudo_stacking_group`]).
fn pseudo_opacity_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
) -> Option<usize> {
    (paint_rules::pseudo_forms_group(cascade, id)
        || paint_rules::negative_pseudo_group(document, cascade, root, id)
        || paint_rules::pseudo_stacking_group(document, cascade, root, id))
    .then_some(id)
}

/// The border box of the `::before` or `::after` with generated id `group`
/// on the lines of `part` of paragraph `root` on the page, in the
/// coordinates of the paragraph's lines. An atomic pseudo-element has one.
fn pseudo_box(
    items: &PageItems<'_>,
    root: usize,
    group: usize,
    part: LinePart,
) -> Option<PaintRect> {
    let (owner, pseudo) = crate::generated_content::generated_origin(group)?;
    let kind = match pseudo {
        PseudoElem::Before => GeneratedKind::Before,
        PseudoElem::After => GeneratedKind::After,
        _ => return None, // cov:ignore: only `::before` and `::after` take transforms here
    };
    items
        .generated_boxes?
        .get(&(root, owner, kind))?
        .iter()
        .find(|piece| part.admits(piece.line))
        .map(|piece| piece.rect)
}

/// The group the `::before` or `::after` of `element` forms in paragraph
/// `root`, if any.
fn generated_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    element: usize,
    kind: GeneratedKind,
) -> Option<usize> {
    let pseudo = if kind == GeneratedKind::After {
        PseudoElem::After
    } else {
        PseudoElem::Before
    };
    pseudo_opacity_group(
        document,
        cascade,
        root,
        crate::generated_content::generated_node_id(element, pseudo),
    )
}

/// Whether the `::before` or `::after` of `element` paints anything of
/// paragraph `root` on the page: a decoration, an image, or a line of its
/// text.
fn pseudo_on_paragraph(
    items: &PageItems<'_>,
    root: usize,
    element: usize,
    kind: GeneratedKind,
    lines: Option<&TextLines>,
) -> bool {
    let after = kind == GeneratedKind::After;
    let pseudo = if after {
        PseudoElem::After
    } else {
        PseudoElem::Before
    };
    let id = NodeId::new(crate::generated_content::generated_node_id(element, pseudo) as u64);
    items
        .generated_boxes
        .is_some_and(|boxes| boxes.contains_key(&(root, element, kind)))
        || items.generated_images.is_some_and(|images| {
            images
                .range((element, after, 0)..=(element, after, usize::MAX))
                .any(|(_, placement)| placement.root == root)
        })
        || lines
            .and_then(|lines| lines.groups.get(&id))
            .is_some_and(|lines| lines.iter().any(|line| line.root.0 == root as u64))
}

/// The paragraph line that holds a content image placed on the page.
fn generated_image_line(items: &PageItems<'_>, image: GeneratedImageId) -> Option<TextLineId> {
    let placement = items.generated_images?.get(&image.key()?)?;
    Some(TextLineId {
        root: NodeId::new(placement.root as u64),
        index: placement.line,
    })
}

/// Whether rectangle `outer` contains the centre of `inner`.
fn contains(outer: PaintRect, inner: PaintRect) -> bool {
    let (x, y) = (inner.x + inner.width * 0.5, inner.y + inner.height * 0.5);
    x >= outer.x && x <= outer.x + outer.width && y >= outer.y && y <= outer.y + outer.height
}

/// The frames listed around a paragraph's own inline content.
struct ParagraphFrames {
    /// Stacking contexts with negative stack levels, listed before it.
    below: Vec<Frame>,
    /// In-flow blocks inside the paragraph, listed after the frames below
    /// and before the content.
    blocks: Vec<Frame>,
    /// Everything else, listed after it.
    above: Vec<Frame>,
}

/// List a paragraph's own inline content, `content`, between the frames
/// below it, then its blocks, and the frames above it. The first `own`
/// events, the boxes of the group being listed, go first. Without frames
/// below or blocks, the content is listed at once.
fn list_paragraph<'a>(
    mut content: Vec<PaintEvent<'a>>,
    own: usize,
    frames: ParagraphFrames,
    events: &mut Vec<PaintEvent<'a>>,
    held: &mut Vec<Vec<PaintEvent<'a>>>,
    stack: &mut Vec<Frame>,
) {
    stack.extend(frames.above.into_iter().rev());
    if frames.below.is_empty() && frames.blocks.is_empty() {
        events.append(&mut content);
    } else {
        let rest = content.split_off(own);
        events.append(&mut content);
        stack.push(Frame::HeldContent(held.len()));
        held.push(rest);
        stack.extend(frames.blocks.into_iter().rev());
        stack.extend(frames.below.into_iter().rev());
    }
}

/// Whether the element of `fragment` is not visible, so that its box paints
/// no decoration.
fn fragment_hides_box(cascade: &CascadeResult, fragment: &Fragment<'_>) -> bool {
    usize::try_from(fragment.node().0)
        .ok()
        .and_then(|node| cascade.computed.get(node))
        .is_some_and(paint_rules::hides_own_box)
}

/// How many of the first events of `content`, the content of opacity group
/// `group`, are the group's own boxes.
fn own_box_events(group: usize, content: &[PaintEvent<'_>]) -> usize {
    let origin = crate::generated_content::generated_origin(group);
    content
        .iter()
        .take_while(|event| match (event, origin) {
            (PaintEvent::Box(fragment), None) => fragment.node() == NodeId::new(group as u64),
            (PaintEvent::LineBox(piece), None) => {
                piece.fragment.node() == NodeId::new(group as u64)
            }
            (PaintEvent::GeneratedBox(generated), Some((owner, pseudo))) => {
                generated.owner == NodeId::new(owner as u64)
                    && matches!(
                        (generated.kind, pseudo),
                        (GeneratedKind::Before, PseudoElem::Before)
                            | (GeneratedKind::After, PseudoElem::After)
                            | (GeneratedKind::FirstLetter, PseudoElem::FirstLetter)
                    )
            }
            _ => false,
        })
        .count()
}

/// Whether inline element `id` of paragraph `root` composites its
/// fragments as one group: one with an opacity below one, one positioned
/// below the paragraph's blocks (see
/// [`paint_rules::negative_inline_group`]), or one positioned with a stack
/// level of its own (see [`paint_rules::inline_stacking_group`]).
fn is_inline_group(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    id: usize,
    node: &crate::Node,
    cv: &raikiri_style::ComputedValues,
) -> bool {
    (node.display().is_inline_flow() && cv.opacity < 1.0)
        || paint_rules::negative_inline_group(document, cascade, root, id)
        || paint_rules::inline_stacking_group(document, cascade, root, id)
}

#[cfg(test)]
mod tests;

/// The style of a prepared generated box piece.
fn column_projection_style<'a>(
    root: &'a crate::Node,
    cascade: &'a CascadeResult,
    piece: &super::generated_boxes::BoxPiece,
) -> Option<&'a raikiri_style::ComputedValues> {
    crate::layout::generated_piece_style(
        root,
        cascade,
        piece.node,
        piece.source_container,
        piece.source_owner,
        piece.line,
    )
}
