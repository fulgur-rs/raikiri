// Frozen pre-index extraction oracle.
use super::super::*;
use raikiri_dom::IfcTextLines;

pub fn build_page_scene_for_page_named(
    dom: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
    content_origin_y: Pt,
    page_name: Option<String>,
) -> PageScene {
    let area = PageArea::of(dom, cascade, page_box, content_origin_y);
    let page_metadata = page_metadata_for(page_box, page_index, page_name);

    let root_arena_idx = dom.document_element_index();
    let root_id = root_arena_idx.map(|idx| NodeId::new(idx as u64));
    let body_id = body_node_id(dom);

    let mut node_ids: Vec<NodeId> = Vec::new();
    let mut fragments: BTreeMap<NodeId, Vec<Fragment>> = BTreeMap::new();
    let mut drawables = PageDrawables::default();

    // A root with `display: none` generates no boxes, whatever an earlier
    // layout left on its descendants, and prints blank paper.
    let root_generates_box =
        cascade.root_element_computed().display != raikiri_style::property::DisplayValue::None;
    if let Some(root_idx) = root_arena_idx.filter(|_| root_generates_box) {
        // DFS from the document element, matching the paint walk. Absolute
        // y stays in the shared document coordinate space until the fragment
        // is committed, which makes page intersection independent of nesting.
        let mut stack = vec![(root_idx, 0.0, 0.0, false, false)];
        let mut ifc = IfcParagraphs::default();
        while let Some((idx, parent_abs_x, parent_abs_y, in_viewport_fixed, inside_fixed_cb)) =
            stack.pop()
        {
            super::indexed_tests::count_old_work(1, 0);
            let Some(node) = dom.get_node(idx) else {
                continue; // cov:ignore: DFS stack holds only document-subtree indices, always valid in a well-formed Document
            };
            if is_skipped_by_paint(node) {
                continue;
            }
            let cv = &cascade.computed[idx];
            let (in_viewport_fixed, child_fixed_cb) = fixed_flags(
                dom,
                idx,
                node,
                cv,
                idx == root_idx,
                (in_viewport_fixed, inside_fixed_cb),
            );
            let ((abs_x, abs_y), border_box) =
                node_border_box(dom, idx, node, (parent_abs_x, parent_abs_y));
            ifc.enter_root(dom, idx, node, (abs_x, abs_y));
            let ifc_lines = ifc.lines_of(node, idx);
            let rects = ifc.node_rects(dom, idx, node, (abs_x, abs_y), border_box, ifc_lines);
            let on_page = area.rects_on_page(rects, in_viewport_fixed);

            if !on_page.is_empty() {
                let node_id = NodeId::new(idx as u64);
                node_ids.push(node_id);
                fragments
                    .entry(node_id)
                    .or_default()
                    .extend(page_fragments(page_index, on_page));

                // `crate::entries` keeps primitive snapshots; the live DOM is
                // still the paint truth until drawables become the renderer
                // input.
                match node.kind() {
                    NodeKind::Element => {
                        let entry = block_entry(dom, cv, node_id, border_box);
                        drawables.block_styles.insert(node_id, entry);
                    }
                    NodeKind::Text => {
                        let entry = paragraph_entry(ifc_lines);
                        drawables.paragraphs.insert(node_id, entry);
                    }
                    _ => {} // cov:ignore: in-document body descendants via parsing are only Element or Text (Comment/PI bits cleared, template fragments detached, DFS starts at the document element)
                }
            }

            if node.kind() == NodeKind::Element {
                // The children of an inline element of an inline engine
                // paragraph are located from the paragraph's root.
                let (base_x, base_y) = if dom.contributes_layout_offset(idx) {
                    (abs_x, abs_y)
                } else {
                    (parent_abs_x, parent_abs_y)
                };
                for &child in node.children.iter().rev() {
                    stack.push((child, base_x, base_y, in_viewport_fixed, child_fixed_cb));
                }
            }
        }
    } // cov:ignore: parse always synthesizes <body>, so the missing-body path is unreachable with a parsed Document

    PageScene {
        page_metadata,
        node_ids,
        fragments,
        drawables,
        root_id,
        body_id,
        body_offset_pt: area.body_offset,
        // Nor does it draw the page's margin boxes.
        margin_boxes: if root_generates_box {
            cascade.page.margin_boxes().to_vec()
        } else {
            Vec::new()
        },
        content_origin_y,
    }
}

/// A border box in document coordinates: `(x, y, width, height)`.
type SceneRect = (Pt, Pt, Pt, Pt);

/// The inline element pieces of one paragraph, by node.
type PieceRectsByNode = HashMap<usize, Vec<SceneRect>>;

/// Where the content of one page sits.
struct PageArea {
    /// Origin of the page area in physical page coordinates.
    body_offset: (Pt, Pt),
    /// Width of the page area.
    width: Pt,
    /// Height of the page area.
    height: Pt,
    /// Whether the pages run along the x axis, and from the right when they
    /// do; `None` when they run down the y axis.
    axis: Option<bool>,
    /// How far the document coordinates of the content move to land in the
    /// page area.
    translation: (Pt, Pt),
}

impl PageArea {
    fn of(
        dom: &Document,
        cascade: &CascadeResult,
        page_box: PageBox,
        content_origin_y: Pt,
    ) -> Self {
        let margins = raikiri_dom::page_margins(cascade, page_box);
        let insets = raikiri_dom::page_content_insets(cascade, page_box);
        let height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);
        let width = insets.page_area_width(margins, page_box);
        let axis = raikiri_dom::pages_follow_x_axis(dom, cascade);
        PageArea {
            body_offset: (margins.left + insets.left, margins.top + insets.top),
            width,
            height,
            axis,
            translation: scene_translation(dom, axis, content_origin_y, width),
        }
    }

    /// The rectangles that reach the page area once moved into it. A box
    /// fixed to the viewport stays where it is.
    fn rects_on_page(&self, rects: Vec<SceneRect>, in_viewport_fixed: bool) -> Vec<SceneRect> {
        let (dx, dy) = if in_viewport_fixed {
            (0.0, 0.0)
        } else {
            self.translation
        };
        rects
            .into_iter()
            .map(|(x, y, w, h)| (x + dx, y + dy, w, h))
            .filter(|&(x, y, w, h)| {
                let (start, extent, end) = if self.axis.is_some() {
                    (x, w.max(0.0), self.width)
                } else {
                    (y, h.max(0.0), self.height)
                };
                if extent == 0.0 {
                    start >= 0.0 && start <= end
                } else {
                    start < end && start + extent > 0.0
                }
            })
            .collect()
    }
}

/// How far the document coordinates of the page's content move to land at
/// the origin of the page area, along the axis the pages run on.
fn scene_translation(
    dom: &Document,
    page_axis: Option<bool>,
    content_origin_y: Pt,
    content_width: Pt,
) -> (Pt, Pt) {
    match page_axis {
        None => (0.0, -content_origin_y),
        Some(false) => (-content_origin_y, 0.0),
        Some(true) => (
            content_origin_y + content_width
                - dom.laid_out_page_area_width().unwrap_or(content_width),
            0.0,
        ),
    }
}

/// The size, index, name and orientation of a page.
fn page_metadata_for(
    page_box: PageBox,
    page_index: u32,
    page_name: Option<String>,
) -> PageMetadata {
    PageMetadata {
        size: (page_box.width, page_box.height),
        page_index,
        page_name,
        orientation: if page_box.width > page_box.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        },
    }
}

/// The `<body>` that paint takes as the root of the page content, if any.
fn body_node_id(dom: &Document) -> Option<NodeId> {
    raikiri_dom::paint_rules::find_paint_root(dom)
        .filter(|&idx| dom.get_node(idx).and_then(|node| node.tag_name()) == Some("body"))
        .map(|idx| NodeId::new(idx as u64))
}

/// Whether paint skips the node and its subtree: outside the document, not
/// rendered, `display: none`, or removed by a `text-overflow` ellipsis.
fn is_skipped_by_paint(node: &Node) -> bool {
    !node.is_in_document()
        || node.is_non_rendered_html_element()
        || node.is_display_none()
        || node.is_hidden_by_text_overflow()
}

/// Whether a node is fixed to the viewport and whether its descendants have a
/// fixed containing block, given the same two flags of its parent.
fn fixed_flags(
    dom: &Document,
    idx: usize,
    node: &Node,
    cv: &ComputedValues,
    is_root: bool,
    (in_viewport_fixed, inside_fixed_cb): (bool, bool),
) -> (bool, bool) {
    let element_box = node.kind() == NodeKind::Element
        && (is_root || raikiri_dom::paint_rules::generates_box(cv));
    (
        in_viewport_fixed
            || (element_box
                && !inside_fixed_cb
                && cv.position == raikiri_style::property::PositionValue::Fixed),
        inside_fixed_cb
            || (element_box && raikiri_dom::establishes_fixed_containing_block(dom, idx, cv)),
    )
}

/// The origin of a node and its border box in document coordinates.
fn node_border_box(
    dom: &Document,
    idx: usize,
    node: &Node,
    (parent_abs_x, parent_abs_y): (Pt, Pt),
) -> ((Pt, Pt), SceneRect) {
    let layout = node.layout();
    let (abs_x, abs_y) = dom
        .layout_origin_of(idx)
        .map(|origin| (origin.x, origin.y))
        .unwrap_or((
            parent_abs_x + layout.location.x,
            parent_abs_y + layout.location.y,
        ));
    // Descendants keep the wrapper origin; only the table's own
    // border box and style snapshot exclude the caption area.
    let border_box = node.table_grid_box().map_or(
        (abs_x, abs_y, layout.size.width, layout.size.height),
        |rect| (abs_x + rect.x, abs_y + rect.y, rect.width, rect.height),
    );
    ((abs_x, abs_y), border_box)
}

/// Content-box origin and inline element pieces of every paragraph laid out
/// by the inline engine, by root. Its text nodes have no layout of their own,
/// and its inline elements are recorded as one rectangle per line; both are
/// measured from the root's content box. A root is visited before its
/// descendants.
#[derive(Default)]
struct IfcParagraphs {
    /// Where the content box of each paragraph starts.
    origin: HashMap<usize, (Pt, Pt)>,
    /// The content width and whether lines stack right to left, for each
    /// vertical paragraph.
    vertical: HashMap<usize, (Pt, bool)>,
    /// The inline element pieces of each paragraph.
    pieces: HashMap<usize, PieceRectsByNode>,
    /// The lines each text node of a paragraph shows up on.
    text_lines: HashMap<usize, IfcTextLines>,
}

impl IfcParagraphs {
    /// Record the paragraph that `node` roots, if it roots one; `abs` is
    /// where the node sits.
    fn enter_root(&mut self, dom: &Document, idx: usize, node: &Node, (abs_x, abs_y): (Pt, Pt)) {
        if !node.is_ifc_root() {
            return;
        }
        let layout = node.layout();
        let origin = (
            abs_x + layout.border.left + layout.padding.left,
            abs_y + layout.border.top + layout.padding.top,
        );
        self.origin.insert(idx, origin);
        if let Some(mode) = node.ifc_writing_mode().filter(|mode| mode.is_vertical()) {
            self.vertical.insert(
                idx,
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
        let mut by_node: PieceRectsByNode = HashMap::new();
        for piece in node.ifc_inline_boxes().unwrap_or_default() {
            let rect = piece.border_box;
            by_node.entry(piece.node).or_default().push((
                origin.0 + rect.x,
                origin.1 + rect.y,
                rect.width,
                rect.height,
            ));
        }
        self.pieces.insert(idx, by_node);
        let lines = dom.ifc_text_lines_by_node(idx);
        super::indexed_tests::count_old_work(
            0,
            lines.values().map(|lines| lines.lines.len()).sum(),
        );
        self.text_lines.extend(lines);
    }

    /// The lines a text node shows up on in its paragraph.
    fn lines_of(&self, node: &Node, idx: usize) -> Option<&IfcTextLines> {
        if node.kind() == NodeKind::Text {
            self.text_lines.get(&idx)
        } else {
            None
        }
    }

    /// Border boxes of the node in document coordinates: one for most
    /// nodes, one per line for an inline element of an ifc paragraph.
    fn node_rects(
        &self,
        dom: &Document,
        idx: usize,
        node: &Node,
        (abs_x, abs_y): (Pt, Pt),
        border_box: SceneRect,
        ifc_lines: Option<&IfcTextLines>,
    ) -> Vec<SceneRect> {
        if let Some(owned) = ifc_lines {
            let (root_x, root_y) = self
                .origin
                .get(&owned.root)
                .copied()
                .unwrap_or((abs_x, abs_y));
            let first_top = owned.lines.first().map_or(0.0, |l| l.top);
            let last_bottom = owned.lines.last().map_or(0.0, |l| l.bottom);
            let block_size = (last_bottom - first_top).max(0.0);
            if let Some(&(content_width, right_to_left)) = self.vertical.get(&owned.root) {
                let x = if right_to_left {
                    root_x + content_width - last_bottom
                } else {
                    root_x + first_top
                };
                vec![(x, root_y, block_size, owned.width)]
            } else {
                vec![(root_x, root_y + first_top, owned.width, block_size)]
            }
        } else {
            let pieces: Vec<SceneRect> = if node.in_ifc_subtree() {
                ifc_root_of(dom, idx)
                    .and_then(|root| self.pieces.get(&root))
                    .and_then(|pieces| pieces.get(&idx))
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if pieces.is_empty() {
                vec![border_box]
            } else {
                pieces
            }
        }
    }
}

/// The snapshot of a text node: a text node outside every paragraph has no
/// lines.
fn paragraph_entry(ifc_lines: Option<&IfcTextLines>) -> ParagraphEntry {
    ParagraphEntry {
        line_count: ifc_lines.map_or(0, |owned| owned.lines.len()),
        ..ParagraphEntry::default()
    }
}

/// One fragment of the page per rectangle.
fn page_fragments(page_index: u32, rects: Vec<SceneRect>) -> impl Iterator<Item = Fragment> {
    rects
        .into_iter()
        .map(move |(x, y, width, height)| Fragment {
            page_index,
            x,
            y,
            width,
            height,
        })
}

/// The primitive snapshot of an element's style and size.
fn block_entry(
    dom: &Document,
    cv: &ComputedValues,
    node_id: NodeId,
    border_box: SceneRect,
) -> BlockEntry {
    BlockEntry {
        background_color: (
            cv.background_color.r,
            cv.background_color.g,
            cv.background_color.b,
            cv.background_color.a,
        ),
        border_widths: (
            cv.border.top.width().0,
            cv.border.right.width().0,
            cv.border.bottom.width().0,
            cv.border.left.width().0,
        ),
        id: element_id(dom, node_id),
        layout_size: Some((border_box.2, border_box.3)),
        ..BlockEntry::default()
    }
}

/// The paragraph root above a node inside an ifc paragraph.
fn ifc_root_of(dom: &Document, idx: usize) -> Option<usize> {
    dom.ancestors(idx)
        .find(|&id| dom.get_node(id).is_some_and(|node| node.is_ifc_root()))
}

/// Get an Element node's `id` attribute through the
/// [`raikiri_traits::Dom`] trait.
///
/// `Document::get_node(usize) -> &raikiri_dom::Node` (the raw arena accessor
/// used by this module's DFS) has no attribute lookup (see
/// `crates/raikiri-dom/src/node.rs`). Instead, perform a second, trait-based
/// lookup through `NodeId` (`Dom::node` → `Node::as_element` → `Element::id`;
/// see `crates/raikiri-dom/src/dom_impl.rs`). This second path is necessary
/// because the raw arena accessor and the trait-based element accessor do
/// not share element-attribute access. An empty `id=""` yields `None`, as
/// specified by the trait's default contract.
fn element_id(dom: &Document, node_id: NodeId) -> Option<String> {
    use raikiri_traits::{Dom as _, Element as _, Node as _};
    dom.node(node_id)?.as_element()?.id().map(str::to_string)
}
