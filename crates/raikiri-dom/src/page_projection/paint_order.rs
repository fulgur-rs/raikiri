//! A page's body content as a sequence of paint steps, in the order the
//! built-in painter draws it.

use super::generated_boxes::{GeneratedBox, LineBox, PageGeneratedBoxes, PageInlineBoxes};
use super::generated_images::{GeneratedImageId, PageGeneratedImages};
use super::records::{PageFragmentItem, PageFragmentKind};
use crate::{
    Document, Fragment, GeneratedKind, OverflowClip, PositionedGlyphRun, RunSource, TextLineId,
    paint_rules,
};
use raikiri_style::property::{ColumnCountValue, DisplayValue, PositionValue};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::{NodeId, NodeKind, PaintClip, PaintRect, PaintTransform};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// opacity group holds (see `Document::text_run_opacity_group`).
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
    /// rectangle come from `Document::page_generated_image`.
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
    /// [`paint_rules::merges_clipped_layer`]).
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
    /// Discard the clip opened at this event index if its hidden table
    /// contributes no paint on this page.
    PopClipIfPainted(usize),
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Default, Clone)]
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
mod paragraph;
mod structure;
mod walk;

pub(super) use letters::LetterGroupCache;
use letters::LetterGroups;
use paragraph::push_paragraph;
pub(super) use structure::ParagraphStructureCache;

/// The elements of a paragraph with inline boxes on its first line and on
/// the lines after it.
#[derive(Debug, Default)]
struct LineBoxes {
    first: HashSet<usize>,
    later: HashSet<usize>,
}

type ParagraphListings<'a> =
    HashMap<(usize, Option<usize>, LinePart), std::rc::Rc<paragraph::ListedParagraph<'a>>>;

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
    source_lines: Option<TextLines>,
    paragraph_presence: RefCell<HashMap<(usize, usize, LinePart), bool>>,
    #[cfg(test)]
    full_paragraph_listing: bool,
    paragraph_listings: RefCell<ParagraphListings<'a>>,
    paragraph_nodes: RefCell<Option<HashMap<usize, Arc<HashSet<usize>>>>>,
    paragraph_slots: RefCell<HashMap<usize, Arc<structure::PageParagraphSlots>>>,
}

impl<'a> PageItems<'a> {
    /// The `::first-letter` opacity groups of paragraph `root`.
    fn letter_groups(&self, root: usize) -> Arc<LetterGroups> {
        self.document.cached_letter_groups(self.cascade, root)
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
        let Some(page_position) = self.page_projection.page_position(page_index) else {
            return Vec::new();
        };
        let page = &self.page_projection.pages[page_position];
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
            source_lines: lines.cloned(),
            paragraph_presence: Default::default(),
            #[cfg(test)]
            full_paragraph_listing: !skip_empty,
            paragraph_listings: Default::default(),
            paragraph_nodes: Default::default(),
            paragraph_slots: Default::default(),
        };
        for item in &page.items {
            if let Ok(node_id) = usize::try_from(item.node_id.0) {
                items.by_node.entry(node_id).or_default().push(item);
            }
        }

        let entered = skip_empty
            .then(|| self.page_paint_nodes(&items, page_index, lines))
            .flatten();

        let events = walk::PaintWalk::new(
            self,
            cascade,
            &items,
            items.source_lines.as_ref(),
            page_index,
            entered,
            root,
        )
        .run();
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
            let structure = self.paragraph_structure(cascade, root);
            let position = &structure.positions;
            let mut groups: Vec<(Option<(usize, u8)>, usize)> = groups
                .into_iter()
                .map(|group| (self.group_tree_position(position, group), group))
                .collect();
            // Pseudo-element groups are found after the element groups of
            // the same pass; tree order puts them among those.
            groups.sort_by_key(|&(at, _)| at.map_or((usize::MAX, u8::MAX), |at| at));
            // The first entry beyond a tree position is found from prefix
            // maxima, even when the original boxes are not in tree order.
            // Previously inserted groups are sorted and cannot be that
            // first greater entry; merge them once instead of shifting the
            // growing vector for each group.
            let mut maximum = None;
            let prefix: Vec<_> = entries
                .iter()
                .map(|(_, frame)| {
                    if let Frame::Visit(id) | Frame::Paragraph(id) = frame {
                        maximum = maximum.max(position.get(id).map(|&at| (at, 0)));
                    }
                    maximum
                })
                .collect();
            let mut additions = Vec::with_capacity(groups.len());
            for (group_at, group) in groups {
                let at = group_at.map_or(entries.len(), |group_at| {
                    prefix.partition_point(|maximum| {
                        maximum.is_none_or(|maximum| maximum <= group_at)
                    })
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
                additions.push((at, (key, Frame::InlineGroup(root, group, part))));
            }
            let mut merged = Vec::with_capacity(entries.len() + additions.len());
            let mut additions = additions.into_iter().peekable();
            for (index, entry) in entries.into_iter().enumerate() {
                while additions.peek().is_some_and(|(at, _)| *at == index) {
                    merged.push(additions.next().unwrap().1);
                }
                merged.push(entry);
            }
            merged.extend(additions.map(|(_, entry)| entry));
            entries = merged;
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
            .map(|(position, id)| {
                #[cfg(test)]
                super::work_tests::paragraph_node();
                (id, position)
            })
            .collect()
    }

    /// The `groups` of paragraph `root` that list anything on the lines of
    /// `part`: all of them when `part` covers every line.
    fn groups_with_content<'a>(
        &'a self,
        items: &PageItems<'a>,
        root: usize,
        mut groups: Vec<usize>,
        part: LinePart,
    ) -> Vec<usize> {
        if part != LinePart::All {
            groups.retain(|&group| self.group_has_content(items, root, group, part));
        }
        groups
    }

    /// Whether inline opacity group `group` of paragraph `root`, or a group
    /// nested in it, lists anything on the lines of `part`, or a box laid
    /// out beside them.
    fn group_has_content<'a>(
        &'a self,
        items: &PageItems<'a>,
        root: usize,
        group: usize,
        part: LinePart,
    ) -> bool {
        let key = (root, group, part);
        if let Some(&present) = items.paragraph_presence.borrow().get(&key) {
            return present;
        }
        let mut pending = vec![(group, false)];
        while let Some((group, children_ready)) = pending.pop() {
            let key = (root, group, part);
            if items.paragraph_presence.borrow().contains_key(&key) {
                continue;
            }
            let listed = paragraph::listed_paragraph(self, items, root, Some(group), part);
            let own = !listed.events.is_empty()
                || self
                    .ifc_boxes_in_group(items.cascade, root, Some(group), part)
                    .iter()
                    .any(|id| part != LinePart::First || items.by_node.contains_key(id));
            if own {
                items.paragraph_presence.borrow_mut().insert(key, true);
            } else if children_ready {
                let present = listed.nested.iter().any(|&child| {
                    items
                        .paragraph_presence
                        .borrow()
                        .get(&(root, child, part))
                        .copied()
                        .unwrap_or(false)
                });
                items.paragraph_presence.borrow_mut().insert(key, present);
            } else {
                pending.push((group, true));
                pending.extend(listed.nested.iter().rev().map(|&child| (child, false)));
            }
        }
        items.paragraph_presence.borrow()[&(root, group, part)]
    }

    /// The in-flow blocks inside the inline content of paragraph `root`
    /// that paint before its lines (see
    /// [`paint_rules::paints_before_paragraph_lines`]), in tree order.
    fn uncached_paragraph_blocks(&self, cascade: &CascadeResult, root: usize) -> Vec<usize> {
        self.get_node(root)
            .map(|node| node.ifc_boxes())
            .unwrap_or_default()
            .into_iter()
            .filter(|&id| paint_rules::paints_before_paragraph_lines(self, cascade, root, id))
            .collect()
    }

    fn paragraph_blocks(&self, cascade: &CascadeResult, root: usize) -> Vec<usize> {
        self.paragraph_structure(cascade, root).blocks.clone()
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
        if self.get_node(root).is_none() {
            return Vec::new();
        }
        self.paragraph_structure(cascade, root)
            .boxes
            .get(&(group, part))
            .cloned()
            .unwrap_or_default()
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
