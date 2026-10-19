//! The walk over the laid-out tree that collects the sources of page
//! fragments: each box and text line of the document at its coordinates after
//! pagination, the paragraphs and list markers that own text, and the clips of
//! boxes placed more than once.

use super::column_projection::{ColumnProjection, TextGroups};
use super::{
    BoxRect, CascadeResult, DisplayValue, Document, IfcTextLines, LayoutError, NodeId, NodeKind,
    OverflowClipSource, PageFragmentInsets, PageFragmentSource, PaintInsets, PaintRect,
    ParagraphCache, ParagraphProjection, PlacementOverflowClipSource, ProjectedTextRoot,
    ProjectionWork, finite_nonnegative,
};
use crate::fragment::LayoutFragment;
use crate::node::Node;
use std::collections::{HashMap, HashSet, hash_map::Entry};

/// What the walk collected: the sources of the page fragments in the order
/// they were met, the text roots, and the paragraphs it prepared on the way.
pub(super) struct WalkedSources {
    pub(super) nodes: Vec<PageFragmentSource>,
    pub(super) text_roots: Vec<ProjectedTextRoot>,
    pub(super) placement_overflow_clips: Vec<PlacementOverflowClipSource>,
    pub(super) paragraph_cache: ParagraphCache,
    /// The nodes placed more than once, and everything below them.
    pub(super) repeated_placement_nodes: HashSet<usize>,
}

/// Walk the tree from the root element, in document order, and collect the
/// sources of the page fragments.
pub(super) fn walk_sources(
    document: &Document,
    cascade: &CascadeResult,
    body_id: usize,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<WalkedSources, LayoutError> {
    let root = crate::layout::initial_containing_block::root_element(document).unwrap_or(body_id);
    let mut walk = SourceWalk {
        document,
        cascade,
        fragments: FragmentIndex::new(document),
        stack: vec![ProjectionVisit::root(root)],
        paragraphs: Paragraphs::default(),
        sources: Vec::new(),
        text_roots: Vec::new(),
        placement_overflow_clips: Vec::new(),
    };
    while let Some(visit) = walk.stack.pop() {
        walk.visit(visit, work)?;
    }
    // The arena index is the stable NodeId projection used by
    // `raikiri_traits::Dom`; sorting by it reproduces fulgur's deterministic
    // BTreeMap iteration order whatever order the walk met the nodes in.
    walk.sources.sort_by_key(|source| source.node_id);
    Ok(WalkedSources {
        nodes: walk.sources,
        text_roots: walk.text_roots,
        placement_overflow_clips: walk.placement_overflow_clips,
        paragraph_cache: walk.paragraphs.cache,
        repeated_placement_nodes: walk.fragments.repeated,
    })
}

/// A node the walk has yet to visit, and the space it is visited in.
struct ProjectionVisit {
    node: usize,
    /// Origin of the parent's space, which the node's own location is
    /// measured from.
    parent_origin: (f32, f32),
    /// The node is inside a subtree repeated on every page.
    repeat: bool,
    /// The layout fragment the node is visited through.
    fragment: Option<usize>,
    /// Origin of that fragment.
    fragment_origin: (f32, f32),
    /// The visit enters the node through its own fragment record, rather
    /// than reaching it from its parent.
    is_fragment_visit: bool,
    /// Intersection of the explicit clips of the fragments entered so far.
    fragment_clip: Option<PaintRect>,
    /// The nearest enclosing placement overflow clip, as an index into the
    /// clips the walk collects.
    overflow_chain: Option<usize>,
    /// The paragraph root and column that list the node among their children.
    paragraph_column: Option<(usize, usize)>,
}

impl ProjectionVisit {
    /// The visit that starts the walk, at the origin of the page.
    fn root(node: usize) -> Self {
        Self {
            node,
            parent_origin: (0.0, 0.0),
            repeat: false,
            fragment: None,
            fragment_origin: (0.0, 0.0),
            is_fragment_visit: false,
            fragment_clip: None,
            overflow_chain: None,
            paragraph_column: None,
        }
    }
}

/// The layout fragments of a document, indexed the ways the walk looks them
/// up.
pub(super) struct FragmentIndex<'a> {
    fragments: &'a [LayoutFragment],
    /// Indices of the records that place a node, by the record they hang
    /// under.
    by_parent_and_node: HashMap<(Option<usize>, usize), Vec<usize>>,
    /// Every node that has a record.
    nodes: HashSet<usize>,
    /// The nodes placed more than once, and everything below them.
    pub(super) repeated: HashSet<usize>,
}

impl<'a> FragmentIndex<'a> {
    pub(super) fn new(document: &'a Document) -> Self {
        let fragments = document.layout_fragments();
        let mut by_parent_and_node = HashMap::<(Option<usize>, usize), Vec<usize>>::new();
        let mut nodes = HashSet::new();
        let mut counts = HashMap::<usize, usize>::new();
        for (index, fragment) in fragments.iter().enumerate() {
            by_parent_and_node
                .entry((fragment.parent, fragment.node_id))
                .or_default()
                .push(index);
            nodes.insert(fragment.node_id);
            *counts.entry(fragment.node_id).or_default() += 1;
        }
        let mut repeated = HashSet::new();
        let mut pending: Vec<usize> = counts
            .into_iter()
            .filter_map(|(node, count)| (count > 1).then_some(node))
            .collect();
        while let Some(node) = pending.pop() {
            if repeated.insert(node) {
                pending.extend(document.nodes[node].children.iter().copied());
            }
        }
        Self {
            fragments,
            by_parent_and_node,
            nodes,
            repeated,
        }
    }

    /// The records that place `node` under the record `parent`.
    pub(super) fn placements(&self, parent: Option<usize>, node: usize) -> &[usize] {
        self.by_parent_and_node
            .get(&(parent, node))
            .map_or(&[], Vec::as_slice)
    }
}

/// The clip of `fragment` once it is placed at `origin`, inside the fragments
/// clipped by `outer`.
pub(super) fn fragment_clip(
    fragment: &LayoutFragment,
    origin: (f32, f32),
    outer: Option<PaintRect>,
) -> Option<PaintRect> {
    fragment
        .fragmentainer_clip
        .map(|clip| {
            let clip = PaintRect::new(
                origin.0 + clip.x - fragment.rect.x,
                origin.1 + clip.y - fragment.rect.y,
                clip.width,
                clip.height,
            );
            if let Some(parent) = outer {
                let x = clip.x.max(parent.x);
                let y = clip.y.max(parent.y);
                PaintRect::new(
                    x,
                    y,
                    ((clip.x + clip.width).min(parent.x + parent.width) - x).max(0.0),
                    ((clip.y + clip.height).min(parent.y + parent.height) - y).max(0.0),
                )
            } else {
                clip
            }
        })
        .or(outer)
}

/// The area `node` gives its page fragment: its table grid box, measured from
/// the origin of `area`, when it has one, else `area` itself.
pub(super) fn fragment_area(node: &Node, area: PaintRect) -> PaintRect {
    PaintRect::new(
        area.x + node.table_grid_box.map_or(0.0, |rect| rect.x),
        area.y + node.table_grid_box.map_or(0.0, |rect| rect.y),
        node.table_grid_box.map_or(area.width, |rect| rect.width),
        node.table_grid_box.map_or(area.height, |rect| rect.height),
    )
}

/// A node being visited, with the area it has where the walk reached it.
#[derive(Clone, Copy)]
struct PlacedBox<'n> {
    id: usize,
    node: &'n Node,
    area: PaintRect,
    is_repeat: bool,
}

struct SourceWalk<'a> {
    document: &'a Document,
    cascade: &'a CascadeResult,
    fragments: FragmentIndex<'a>,
    stack: Vec<ProjectionVisit>,
    paragraphs: Paragraphs,
    sources: Vec<PageFragmentSource>,
    text_roots: Vec<ProjectedTextRoot>,
    placement_overflow_clips: Vec<PlacementOverflowClipSource>,
}

impl<'a> SourceWalk<'a> {
    fn visit(
        &mut self,
        mut visit: ProjectionVisit,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<(), LayoutError> {
        let (document, cascade, fragments) =
            (self.document, self.cascade, self.fragments.fragments);
        work.check()?;
        let node_id = visit.node;
        if self.fragments.repeated.contains(&node_id) {
            work.charge(1)?;
        }
        let Some(node) = document.get_node(node_id).filter(|node| {
            node.is_in_document() && !node.is_non_rendered_html_element() && !node.is_display_none()
        }) else {
            return Ok(());
        };
        let layout = node.unrounded_layout;
        if !visit.is_fragment_visit
            && let Some(origin) = document.layout_origin_of(node_id)
        {
            let parent_origin = document
                .layout_parent_of(node_id)
                .and_then(|parent| document.layout_origin_of(parent));
            let parent_x = parent_origin.map_or(0.0, |parent| parent.x);
            let parent_y = parent_origin.map_or(0.0, |parent| parent.y);
            visit.parent_origin.0 += origin.x - parent_x - layout.location.x;
            visit.parent_origin.1 += origin.y - parent_y - layout.location.y;
        }
        if self.enter_fragments(&visit, node_id, (layout.location.x, layout.location.y)) {
            return Ok(());
        }
        let own_fragment = visit
            .fragment
            .filter(|_| visit.is_fragment_visit)
            .map(|index| &fragments[index]);
        // The box where the walk reached the node, before the lines of its
        // paragraph move it.
        let reached = PlacedBox {
            id: node_id,
            node,
            area: PaintRect::new(
                visit.parent_origin.0 + layout.location.x,
                visit.parent_origin.1 + layout.location.y,
                finite_nonnegative(own_fragment.map_or(layout.size.width, |f| f.rect.width)),
                finite_nonnegative(own_fragment.map_or(layout.size.height, |f| f.rect.height)),
            ),
            // A fixed-position subtree is painted in every committed page. The
            // existing layout pass already computes one viewport-relative box;
            // preserve that geometry and mark the records as complete repeats
            // instead of clipping them as in-flow content.
            is_repeat: visit.repeat
                || crate::layout::fixed::is_page_fixed(document, cascade, node_id),
        };
        let own_overflow_source = self.placement_overflow_source(&reached, visit.overflow_chain);
        let content_overflow_chain = own_overflow_source.or(visit.overflow_chain);
        self.text_roots
            .extend(self.list_marker_root(&reached, &visit, own_fragment));
        self.enter_paragraph_roots(&reached, &visit, content_overflow_chain, work)?;
        // An inline element of an inline engine paragraph is where its pieces
        // are on the lines; its recorded location is relative to its nearest
        // inline ancestor and does not follow lines that pagination moved.
        let placed = PlacedBox {
            area: self
                .paragraphs
                .inline_element_area(document, node_id, node)
                .unwrap_or(reached.area),
            ..reached
        };
        self.push_sources(&placed, &visit, own_overflow_source);
        if node.kind() == NodeKind::Element {
            self.push_children(&placed, &visit, content_overflow_chain);
        }
        Ok(())
    }

    /// Replace the visit of a node that has layout fragment records with a
    /// visit through each of them, and tell whether the visit ended here.
    ///
    /// A nested column container has a placement record and a retained
    /// container root for the same source node. Enter that root before
    /// visiting the node's DOM children, whose records belong to it. A node
    /// reached from its parent that has records only under other fragments is
    /// not visited here at all.
    fn enter_fragments(
        &mut self,
        visit: &ProjectionVisit,
        node_id: usize,
        location: (f32, f32),
    ) -> bool {
        let indices = self.fragments.placements(visit.fragment, node_id);
        for &index in indices.iter().rev() {
            let fragment = &self.fragments.fragments[index];
            let origin =
                fragment.paint_origin(visit.parent_origin, location, visit.fragment_origin);
            self.stack.push(ProjectionVisit {
                node: node_id,
                parent_origin: (origin.0 - location.0, origin.1 - location.1),
                repeat: visit.repeat,
                fragment: Some(index),
                fragment_origin: origin,
                is_fragment_visit: true,
                fragment_clip: fragment_clip(fragment, origin, visit.fragment_clip),
                overflow_chain: visit.overflow_chain,
                paragraph_column: visit.paragraph_column,
            });
        }
        !indices.is_empty() || (!visit.is_fragment_visit && self.fragments.nodes.contains(&node_id))
    }

    /// Record the overflow clip of a box that is placed more than once, and
    /// return its index among the clips.
    fn placement_overflow_source(
        &mut self,
        placed: &PlacedBox<'_>,
        parent: Option<usize>,
    ) -> Option<usize> {
        let clips = self.fragments.repeated.contains(&placed.id)
            && crate::paint_rules::clips_element_overflow(self.document, self.cascade, placed.id);
        clips.then(|| {
            let border_box = fragment_area(placed.node, placed.area);
            let layout = placed.node.unrounded_layout;
            let (border, padding) = (layout.border, layout.padding);
            let index = self.placement_overflow_clips.len();
            self.placement_overflow_clips
                .push(PlacementOverflowClipSource {
                    node: NodeId::new(placed.id as u64),
                    parent,
                    source: OverflowClipSource {
                        border_box,
                        geometry: crate::paint_rules::OverflowClipGeometry::new(
                            &self.cascade.computed[placed.id],
                            border_box,
                            PaintInsets::new(border.top, border.right, border.bottom, border.left),
                            PaintInsets::new(
                                padding.top,
                                padding.right,
                                padding.bottom,
                                padding.left,
                            ),
                            crate::paint_rules::is_replaced_element(self.document, placed.id),
                        ),
                        is_repeat: placed.is_repeat,
                    },
                });
            index
        })
    }

    /// The text root of the marker of a list item whose marker is not laid
    /// out inside its first paragraph.
    fn list_marker_root(
        &self,
        placed: &PlacedBox<'_>,
        visit: &ProjectionVisit,
        own_fragment: Option<&LayoutFragment>,
    ) -> Option<ProjectedTextRoot> {
        (placed.node.display() == DisplayValue::ListItem
            && !(crate::generated_content::inside_marker_in_flow(self.cascade, placed.id)
                && placed.node.is_ifc_root())
            && own_fragment.is_none_or(|fragment| fragment.fragment_index == 0))
        .then(|| ProjectedTextRoot {
            node: crate::generated_content::generated_node_id(
                placed.id,
                raikiri_style::PseudoElem::Marker,
            ),
            x: placed.area.x,
            y: placed.area.y,
            width: 0.0,
            lines_along_x: false,
            is_repeat: placed.is_repeat,
            fragmentainer: None,
            fragment_clip: visit.fragment_clip,
            overflow_chain: visit.overflow_chain,
        })
    }

    /// Enter the paragraphs a node is the root of: the node itself, and the
    /// anonymous table cells it owns.
    fn enter_paragraph_roots(
        &mut self,
        placed: &PlacedBox<'_>,
        visit: &ProjectionVisit,
        overflow_chain: Option<usize>,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<(), LayoutError> {
        let document = self.document;
        let roots = std::iter::once((placed.id, placed.node))
            .filter(|(_, node)| node.is_ifc_root())
            .chain(document.anonymous_table_cells(placed.id));
        for (root_id, root_node) in roots {
            let column = self.paragraph_column(root_node, visit);
            let root_layout = root_node.unrounded_layout;
            let (root_x, root_y) = if root_id == placed.id {
                (placed.area.x, placed.area.y)
            } else {
                (
                    placed.area.x + root_layout.location.x,
                    placed.area.y + root_layout.location.y,
                )
            };
            let origin = self
                .paragraphs
                .place_root(root_id, root_node, (root_x, root_y));
            if origin.0.is_finite() && origin.1.is_finite() {
                self.text_roots.push(ProjectedTextRoot {
                    node: root_id,
                    x: origin.0,
                    y: origin.1,
                    width: root_layout.content_box_width(),
                    lines_along_x: self.paragraphs.vertical.contains_key(&root_id),
                    is_repeat: placed.is_repeat,
                    fragmentainer: column,
                    fragment_clip: visit.fragment_clip,
                    overflow_chain,
                });
            }
            self.paragraphs.take_column(
                document,
                self.cascade,
                root_id,
                column,
                self.fragments.repeated.contains(&root_id),
                work,
            )?;
        }
        Ok(())
    }

    /// Add the sources of the page fragments a node gives: its box, or the
    /// pieces and lines it has in an inline-engine paragraph.
    fn push_sources(
        &mut self,
        placed: &PlacedBox<'_>,
        visit: &ProjectionVisit,
        own_overflow_source: Option<usize>,
    ) {
        let node = placed.node;
        if !gives_source(node, placed.area) {
            return;
        }
        let ifc_lines = self
            .paragraphs
            .take_text_lines(self.document, placed.id, node);
        let source = self.box_source(placed, visit, own_overflow_source);
        if let Some(groups) = ifc_lines {
            for group in groups {
                let line =
                    self.paragraphs
                        .line_source(&source, group, (placed.area.x, placed.area.y));
                self.sources.push(line);
            }
        } else if let Some((origin, pieces)) =
            self.paragraphs.pieces_of(self.document, placed.id, node)
        {
            for (rect, column) in pieces {
                self.sources
                    .push(piece_source(&source, origin, rect, *column));
            }
        } else {
            self.sources.push(source);
        }
    }

    /// The source of the page fragment of the box of a node.
    fn box_source(
        &self,
        placed: &PlacedBox<'_>,
        visit: &ProjectionVisit,
        own_overflow_source: Option<usize>,
    ) -> PageFragmentSource {
        let node = placed.node;
        let layout = node.unrounded_layout;
        let area = fragment_area(node, placed.area);
        PageFragmentSource {
            node_id: NodeId::new(placed.id as u64),
            node_kind: node.kind(),
            is_img: node.tag_name() == Some("img"),
            abs_x: area.x,
            abs_y: area.y,
            width: area.width,
            height: area.height,
            content_insets: (node.kind() == NodeKind::Element
                && !self.paragraphs.fragmented_inline_nodes.contains(&placed.id))
            .then(|| {
                PageFragmentInsets::new(
                    layout.border.top + layout.padding.top,
                    layout.border.right + layout.padding.right,
                    layout.border.bottom + layout.padding.bottom,
                    layout.border.left + layout.padding.left,
                )
            }),
            line_metrics: (node.kind() == NodeKind::Text).then(Vec::new),
            lines_along_x: false,
            line_start_offset: 0,
            is_repeat: placed.is_repeat,
            fragmentainer: visit.fragment.map_or(0, |index| {
                self.fragments.fragments[index].fragmentainer as u32
            }),
            fragment_clip: visit.fragment_clip,
            overflow_chain: visit.overflow_chain,
            own_overflow_source,
        }
    }

    /// The column of the paragraph of `node` that this visit draws, when it
    /// went through a fragment that selects one.
    fn paragraph_column(&self, node: &Node, visit: &ProjectionVisit) -> Option<usize> {
        visit
            .fragment
            .and_then(|index| node.paragraph_column_of_fragment(&self.fragments.fragments[index]))
    }

    /// Queue the children of an element for a visit.
    fn push_children(
        &mut self,
        placed: &PlacedBox<'_>,
        visit: &ProjectionVisit,
        overflow_chain: Option<usize>,
    ) {
        let (node, node_id) = (placed.node, placed.id);
        let paragraph_column = if node.is_ifc_root() {
            self.paragraph_column(node, visit).and_then(|column| {
                self.paragraphs
                    .cache
                    .get(&node_id)
                    .filter(|prepared| prepared.children.is_some())
                    .map(|_| (node_id, column))
            })
        } else {
            visit.paragraph_column
        };
        let children = paragraph_column.map_or(node.children.as_slice(), |(root, column)| {
            self.paragraphs.cache[&root]
                .children
                .as_ref()
                .unwrap()
                .get(&(node_id, column))
                .map_or(&[], Vec::as_slice)
        });
        // The children of an inline element of an inline engine paragraph
        // are located from the paragraph's root, not from the element.
        let parent_origin = if self.document.contributes_layout_offset(node_id) {
            (placed.area.x, placed.area.y)
        } else {
            visit.parent_origin
        };
        for &child_id in children.iter().rev() {
            self.stack.push(ProjectionVisit {
                node: child_id,
                parent_origin,
                repeat: placed.is_repeat,
                fragment: visit.fragment,
                fragment_origin: visit.fragment_origin,
                is_fragment_visit: false,
                fragment_clip: visit.fragment_clip,
                overflow_chain,
                paragraph_column,
            });
        }
    }
}

/// Whether a node at `area` gives a page fragment: an element does, and text
/// does unless it is only whitespace, as long as the coordinates are finite.
pub(super) fn gives_source(node: &Node, area: PaintRect) -> bool {
    let renders = match node.kind() {
        NodeKind::Text => node
            .text_content()
            .is_some_and(|text| !text.trim().is_empty()),
        kind => kind == NodeKind::Element,
    };
    renders && area.x.is_finite() && area.y.is_finite()
}

/// The origin of the content box of a paragraph, and the pieces of an inline
/// element measured from it, each with the column it is in.
type InlinePieces<'p> = ((f32, f32), &'p [(BoxRect, u32)]);

/// The paragraphs of the inline engine the walk has entered: where each one
/// sits, and the pieces and lines it gives to the boxes and text below it.
#[derive(Default)]
pub(super) struct Paragraphs {
    /// The content-box origin of each root. Its text nodes have no layout of
    /// their own, and their lines are measured from here. A root is visited
    /// before its text.
    pub(super) origin: HashMap<usize, (f32, f32)>,
    /// The content width of each root whose lines stack along x, and whether
    /// they stack from right to left.
    pub(super) vertical: HashMap<usize, (f32, bool)>,
    /// Each inline element's rectangle is the union of its pieces after
    /// pagination moves their lines.
    piece_bounds: HashMap<usize, HashMap<usize, Vec<(BoxRect, u32)>>>,
    fragmented_inline_nodes: HashSet<usize>,
    text_lines: HashMap<usize, TextGroups>,
    cache: ParagraphCache,
}

impl Paragraphs {
    /// Record where the paragraph of `root_id` starts, given the origin of
    /// its border box, and which way its lines stack. Returns the origin of
    /// its content box.
    fn place_root(&mut self, root_id: usize, root: &Node, border_origin: (f32, f32)) -> (f32, f32) {
        let layout = root.unrounded_layout;
        let origin = (
            border_origin.0 + layout.border.left + layout.padding.left,
            border_origin.1 + layout.border.top + layout.padding.top,
        );
        self.origin.insert(root_id, origin);
        if let Some(mode) = root.ifc_writing_mode()
            && mode.is_vertical()
        {
            self.vertical.insert(
                root_id,
                (
                    layout.content_box_width(),
                    matches!(
                        mode,
                        shodo::geometry::WritingMode::VerticalRl
                            | shodo::geometry::WritingMode::SidewaysRl
                    ),
                ),
            );
        }
        origin
    }

    /// Take the pieces and lines of the paragraph of `root_id` that `column`
    /// shows, preparing the paragraph the first time it is entered.
    fn take_column(
        &mut self,
        document: &Document,
        cascade: &CascadeResult,
        root_id: usize,
        column: Option<usize>,
        repeated: bool,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<(), LayoutError> {
        if let Entry::Vacant(entry) = self.cache.entry(root_id) {
            entry.insert(ParagraphProjection::prepare(
                document,
                cascade,
                root_id,
                column.unwrap_or(0),
                work,
            )?);
        }
        let selected = column_data(&self.cache[&root_id], column, repeated, work)?;
        self.fragmented_inline_nodes.extend(selected.fragmented);
        self.piece_bounds.insert(root_id, selected.bounds);
        self.text_lines.extend(selected.owners);
        Ok(())
    }

    /// Where the pieces of `node`, an inline element of an inline-engine
    /// paragraph, lie: the origin of the content box of the paragraph, and
    /// the pieces measured from it. `None` for any other node, and for an
    /// element of a paragraph the walk has not entered.
    fn pieces_of(
        &self,
        document: &Document,
        node_id: usize,
        node: &Node,
    ) -> Option<InlinePieces<'_>> {
        let root = (node.kind() == NodeKind::Element && node.in_ifc_subtree())
            .then(|| document.ifc_root_of(node_id))
            .flatten()?;
        let origin = *self.origin.get(&root)?;
        let pieces = self.piece_bounds.get(&root)?.get(&node_id);
        Some((origin, pieces.map_or(&[], Vec::as_slice)))
    }

    /// The area of an inline element of an inline-engine paragraph, from the
    /// first of its pieces.
    fn inline_element_area(
        &self,
        document: &Document,
        node_id: usize,
        node: &Node,
    ) -> Option<PaintRect> {
        let ((root_x, root_y), pieces) = self.pieces_of(document, node_id, node)?;
        let (rect, _) = pieces.first()?;
        Some(PaintRect::new(
            root_x + rect.x,
            root_y + rect.y,
            finite_nonnegative(rect.width),
            finite_nonnegative(rect.height),
        ))
    }

    /// Take the lines of a text node of an inline-engine paragraph. A text
    /// node of a paragraph that has no line of it gets none, and `None` is
    /// for text outside the inline engine.
    fn take_text_lines(
        &mut self,
        document: &Document,
        node_id: usize,
        node: &Node,
    ) -> Option<TextGroups> {
        (node.kind() == NodeKind::Text)
            .then(|| {
                self.text_lines.remove(&node_id).or_else(|| {
                    document
                        .ifc_root_of(node_id)
                        .and_then(|root| self.cache.get(&root))
                        .filter(|prepared| prepared.combined.owners.contains_key(&node_id))
                        .map(|_| Vec::new())
                })
            })
            .flatten()
    }

    /// The source of the page fragment of a group of lines of a text node,
    /// from the source of the node's box. `fallback_origin` stands for the
    /// origin of the paragraph if the walk has not met it.
    pub(super) fn line_source(
        &self,
        source: &PageFragmentSource,
        (offset_x, start, column, owned): (f32, usize, u32, IfcTextLines),
        fallback_origin: (f32, f32),
    ) -> PageFragmentSource {
        let (root_x, root_y) = self
            .origin
            .get(&owned.root)
            .copied()
            .unwrap_or(fallback_origin);
        let first_top = owned.lines.first().map_or(0.0, |line| line.top);
        let last_bottom = owned.lines.last().map_or(0.0, |line| line.bottom);
        let block_extent = finite_nonnegative(last_bottom - first_top);
        let inline_extent = finite_nonnegative(owned.width);
        // Lines are measured along the paragraph's block axis, from its
        // block-start content edge.
        let (abs_x, abs_y, width, height) = match self.vertical.get(&owned.root) {
            Some(&(content_width, true)) => (
                root_x + content_width - last_bottom,
                root_y + offset_x,
                block_extent,
                inline_extent,
            ),
            Some(_) => (
                root_x + first_top,
                root_y + offset_x,
                block_extent,
                inline_extent,
            ),
            None => (
                root_x + offset_x,
                root_y + first_top,
                inline_extent,
                block_extent,
            ),
        };
        PageFragmentSource {
            abs_x,
            abs_y,
            width,
            height,
            lines_along_x: self.vertical.contains_key(&owned.root),
            line_metrics: Some(
                owned
                    .lines
                    .iter()
                    .map(|line| (line.top - first_top, line.bottom - first_top))
                    .collect(),
            ),
            line_start_offset: start as u32,
            fragmentainer: column,
            ..source.clone()
        }
    }
}

/// The source of the page fragment of one piece of an inline element, from
/// the source of the element's box and the origin of its paragraph.
pub(super) fn piece_source(
    source: &PageFragmentSource,
    (root_x, root_y): (f32, f32),
    rect: &BoxRect,
    column: u32,
) -> PageFragmentSource {
    PageFragmentSource {
        abs_x: root_x + rect.x,
        abs_y: root_y + rect.y,
        width: finite_nonnegative(rect.width),
        height: finite_nonnegative(rect.height),
        fragmentainer: column,
        ..source.clone()
    }
}

/// The pieces and lines of a prepared paragraph that `column` shows. A
/// paragraph in columns shows those of its column; any other shows all of
/// them, numbered after the column it is in. Charges `work` for the records
/// it hands out again when `repeated`.
fn column_data(
    prepared: &ParagraphProjection,
    column: Option<usize>,
    repeated: bool,
    work: &mut ProjectionWork<'_, '_>,
) -> Result<ColumnProjection, LayoutError> {
    if prepared.multicol
        && let Some(column) = column
    {
        return Ok(match prepared.columns.get(&column) {
            Some(data) => {
                work.charge(data.record_count)?;
                data.clone()
            }
            None => ColumnProjection::default(),
        });
    }
    if repeated {
        work.charge(prepared.combined.record_count)?;
    }
    let mut data = prepared.combined.clone();
    if !prepared.multicol {
        let number = column.unwrap_or(0) as u32;
        for pieces in data.bounds.values_mut() {
            for (_, piece_number) in pieces {
                *piece_number = number;
            }
        }
        for groups in data.owners.values_mut() {
            for (_, _, group_number, _) in groups {
                *group_number = number;
            }
        }
    }
    Ok(data)
}
