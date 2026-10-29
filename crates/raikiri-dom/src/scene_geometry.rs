//! Immutable layout geometry for dogfooding page snapshots.

mod index;
use crate::{Document, IfcTextLines, Node};
use raikiri_traits::NodeKind;
use std::collections::HashMap;
use std::sync::Arc;

type SceneRect = (f32, f32, f32, f32);
type PieceRectsByNode = HashMap<usize, Vec<SceneRect>>;

/// Layout-only facts for one source node, in document DFS order.
#[doc(hidden)]
#[derive(Debug)]
pub struct SceneGeometryNode {
    node: usize,
    parent: Option<usize>,
    border_box: SceneRect,
    line_count: usize,
}
impl SceneGeometryNode {
    /// Source arena index.
    pub fn node(&self) -> usize {
        self.node
    }
    /// Parent record index in the prepared DFS.
    pub fn parent(&self) -> Option<usize> {
        self.parent
    }
    /// Original border box in shared document coordinates.
    pub fn border_box(&self) -> SceneRect {
        self.border_box
    }
    /// Full paragraph line count retained by the existing snapshot contract.
    pub fn line_count(&self) -> usize {
        self.line_count
    }
}

/// Shared geometry, independent of cascade and page selection.
#[doc(hidden)]
#[derive(Debug)]
pub struct SceneGeometry {
    records: Vec<SceneGeometryNode>,
    pieces: Vec<(usize, SceneRect)>,
    axes: [index::RectIndex; 2],
}
impl SceneGeometry {
    /// Source nodes in DFS order.
    pub fn records(&self) -> &[SceneGeometryNode] {
        &self.records
    }
    /// Owner record and original geometry of one rectangle.
    pub fn piece(&self, piece: usize) -> (usize, SceneRect) {
        self.pieces[piece]
    }
    /// Rectangle candidates in original DFS and piece order. Current styles
    /// decide whether each piece receives the ordinary or zero translation.
    pub fn candidates(&self, horizontal: bool, translation: f32, end: f32) -> Vec<usize> {
        let index = &self.axes[usize::from(horizontal)];
        let mut candidates = index.query(translation, end);
        if translation != 0.0 {
            candidates.extend(index.query(0.0, end));
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }
    fn build(dom: &Document) -> Self {
        let mut records = Vec::new();
        let mut pieces = Vec::new();
        let mut stack: Vec<_> = dom
            .document_element_index()
            .into_iter()
            .map(|root| (root, 0.0, 0.0, None))
            .collect();
        let mut ifc = IfcParagraphs::default();
        while let Some((idx, parent_x, parent_y, parent)) = stack.pop() {
            let Some(node) = dom.get_node(idx) else {
                continue; // cov:ignore: source child indices always name arena nodes
            };
            if is_skipped_by_paint(node) {
                continue;
            }
            let (abs, border_box) = node_border_box(dom, idx, node, (parent_x, parent_y));
            ifc.enter_root(dom, idx, node, abs);
            let lines = ifc.lines_of(node, idx);
            let rects = ifc.node_rects(dom, idx, node, abs, border_box, lines);
            #[cfg(test)]
            tests::count_harvest(1, lines.map_or(0, |lines| lines.lines.len()));
            let record = records.len();
            records.push(SceneGeometryNode {
                node: idx,
                parent,
                border_box,
                line_count: lines.map_or(0, |lines| lines.lines.len()),
            });
            pieces.extend(rects.into_iter().map(|rect| (record, rect)));
            if node.kind() == NodeKind::Element {
                let (x, y) = if dom.contributes_layout_offset(idx) {
                    abs
                } else {
                    (parent_x, parent_y)
                };
                stack.extend(
                    node.children
                        .iter()
                        .rev()
                        .map(|&child| (child, x, y, Some(record))),
                );
            }
        }
        let axes = [
            index::RectIndex::new(&pieces, false),
            index::RectIndex::new(&pieces, true),
        ];
        Self {
            records,
            pieces,
            axes,
        }
    }
}
impl Document {
    /// Layout geometry shared by successive dogfooding page snapshots.
    #[doc(hidden)]
    pub fn scene_geometry(&self) -> &SceneGeometry {
        self.page_projection
            .scene_geometry
            .get_or_init(|| Arc::new(SceneGeometry::build(self)))
            .as_ref()
    }
}
fn is_skipped_by_paint(node: &Node) -> bool {
    !node.is_in_document()
        || node.is_non_rendered_html_element()
        || node.is_display_none()
        || node.is_hidden_by_text_overflow()
}

/// The origin of a node and its border box in document coordinates.
fn node_border_box(
    dom: &Document,
    idx: usize,
    node: &Node,
    (parent_abs_x, parent_abs_y): (f32, f32),
) -> ((f32, f32), SceneRect) {
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
    origin: HashMap<usize, (f32, f32)>,
    /// The content width and whether lines stack right to left, for each
    /// vertical paragraph.
    vertical: HashMap<usize, (f32, bool)>,
    /// The inline element pieces of each paragraph.
    pieces: HashMap<usize, PieceRectsByNode>,
    /// The lines each text node of a paragraph shows up on.
    text_lines: HashMap<usize, IfcTextLines>,
}

impl IfcParagraphs {
    /// Record the paragraph that `node` roots, if it roots one; `abs` is
    /// where the node sits.
    fn enter_root(&mut self, dom: &Document, idx: usize, node: &Node, (abs_x, abs_y): (f32, f32)) {
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
        self.text_lines.extend(dom.ifc_text_lines_by_node(idx));
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
        (abs_x, abs_y): (f32, f32),
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

/// The paragraph root above a node inside an ifc paragraph.
fn ifc_root_of(dom: &Document, idx: usize) -> Option<usize> {
    dom.ancestors(idx)
        .find(|&id| dom.get_node(id).is_some_and(|node| node.is_ifc_root()))
}

#[cfg(test)]
mod tests;
