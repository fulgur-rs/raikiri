//! Layout-only anonymous table cells, without changing the source DOM.

use crate::Document;
use crate::node::{Node, NodeFlags};
use raikiri_style::CascadeResult;
use raikiri_style::property::DisplayValue;
use raikiri_traits::NodeKind;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct AnonymousCell {
    pub(crate) owner: usize,
    pub(crate) node: Node,
}

#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) marker: usize,
    pub(crate) cells: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TableObjects {
    pub(crate) arena_len: usize,
    pub(crate) cells: Vec<AnonymousCell>,
    pub(crate) rows: HashMap<usize, Vec<Row>>,
    pub(crate) paragraph_owner: Vec<Option<usize>>,
}

fn whitespace(doc: &Document, id: usize) -> bool {
    doc.nodes[id].kind() == NodeKind::Text
        && doc.nodes[id].text_content().is_some_and(|text| {
            text.chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        })
}

fn children(doc: &Document, owner: usize) -> Vec<usize> {
    fn append(doc: &Document, id: usize, out: &mut Vec<usize>) {
        let node = &doc.nodes[id];
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || node.display == DisplayValue::None
            || !matches!(node.kind(), NodeKind::Element | NodeKind::Text)
        {
            return;
        }
        if node.display == DisplayValue::Contents {
            for &child in &node.children {
                append(doc, child, out);
            }
        } else {
            out.push(id);
        }
    }
    let mut out = Vec::new();
    for &id in &doc.nodes[owner].children {
        append(doc, id, &mut out);
    }
    out
}

fn anonymous_cell(
    doc: &Document,
    objects: &mut TableObjects,
    owner: usize,
    content: Vec<usize>,
) -> usize {
    let mut node = doc.nodes[owner].clone();
    node.children = content;
    node.style = taffy::Style::default();
    node.display = DisplayValue::TableCell;
    node.ifc = None;
    node.cache.clear();
    node.flags
        .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
    node.order_modified_children = Box::default();
    node.multicol = None;
    node.computed_border = None;
    node.collapsed_border = None;
    node.needs_relative_block_paint_offset = false;
    node.unrounded_layout = taffy::Layout::new();
    let key = objects.arena_len + objects.cells.len();
    objects.cells.push(AnonymousCell { owner, node });
    key
}

fn row(
    doc: &Document,
    objects: &mut TableObjects,
    owner: usize,
    ids: &[usize],
    marker: Option<usize>,
) -> Row {
    let mut cells = Vec::new();
    let mut pending = Vec::new();
    let flush = |pending: &mut Vec<usize>, cells: &mut Vec<usize>, objects: &mut TableObjects| {
        // Inter-cell white space generates no anonymous cell. Keep white
        // space within an actual content run for the paragraph to collapse.
        if pending.iter().any(|&id| !whitespace(doc, id)) {
            cells.push(anonymous_cell(doc, objects, owner, std::mem::take(pending)));
        } else {
            pending.clear();
        }
    };
    for &id in ids {
        if doc.nodes[id].display == DisplayValue::TableCell {
            flush(&mut pending, &mut cells, objects);
            cells.push(id);
        } else {
            pending.push(id);
        }
    }
    flush(&mut pending, &mut cells, objects);
    Row {
        marker: marker.unwrap_or_else(|| cells.first().copied().unwrap_or(owner)),
        cells,
    }
}

fn rows(doc: &Document, objects: &mut TableObjects, owner: usize, reorder: bool) -> Vec<Row> {
    let mut ids = children(doc, owner);
    if reorder {
        let first_head = ids
            .iter()
            .copied()
            .find(|&id| doc.nodes[id].display == DisplayValue::TableHeaderGroup);
        let first_foot = ids
            .iter()
            .copied()
            .find(|&id| doc.nodes[id].display == DisplayValue::TableFooterGroup);
        ids.sort_by_key(|&id| {
            if Some(id) == first_head {
                0
            } else if Some(id) == first_foot {
                2
            } else {
                1
            }
        });
    }
    let mut out = Vec::new();
    let mut pending = Vec::new();
    let flush = |pending: &mut Vec<usize>, out: &mut Vec<Row>, objects: &mut TableObjects| {
        if pending.iter().any(|&id| !whitespace(doc, id)) {
            out.push(row(doc, objects, owner, pending, None));
        }
        pending.clear();
    };
    for id in ids {
        match doc.nodes[id].display {
            DisplayValue::TableRow => {
                flush(&mut pending, &mut out, objects);
                out.push(row(doc, objects, id, &children(doc, id), Some(id)));
            }
            DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup => {
                flush(&mut pending, &mut out, objects);
                out.extend(rows(doc, objects, id, false));
            }
            DisplayValue::TableCaption
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup => {
                flush(&mut pending, &mut out, objects);
            }
            _ => pending.push(id),
        }
    }
    flush(&mut pending, &mut out, objects);
    out
}

pub(crate) fn prepare(doc: &mut Document, cascade: &CascadeResult) {
    let mut objects = TableObjects {
        arena_len: doc.nodes.len(),
        paragraph_owner: vec![None; doc.nodes.len()],
        ..TableObjects::default()
    };
    for id in 0..doc.nodes.len() {
        if doc.nodes[id].is_in_document()
            && matches!(
                doc.nodes[id].display,
                DisplayValue::Table | DisplayValue::InlineTable
            )
            && !super::super::ifc::assign::can_be_ifc_root(doc, cascade, id)
        {
            let table_rows = rows(doc, &mut objects, id, true);
            objects.rows.insert(id, table_rows);
        }
    }
    doc.table_objects = objects;
}

impl Document {
    /// The layout view of an IFC root, including a layout-only anonymous cell.
    /// Internal paragraph keys are distinct from the source DOM node arena.
    #[doc(hidden)]
    pub fn ifc_layout_node(&self, key: usize) -> Option<&Node> {
        self.nodes.get(key).or_else(|| {
            self.table_objects
                .cells
                .get(key.checked_sub(self.table_objects.arena_len)?)
                .map(|cell| &cell.node)
        })
    }

    pub(crate) fn table_layout_node_mut(&mut self, key: usize) -> &mut Node {
        if key < self.nodes.len() {
            &mut self.nodes[key]
        } else {
            &mut self.table_objects.cells[key - self.table_objects.arena_len].node
        }
    }

    /// Anonymous paragraphs owned by this source table or row, with internal keys.
    #[doc(hidden)]
    pub fn anonymous_table_cells(&self, owner: usize) -> impl Iterator<Item = (usize, &Node)> {
        self.table_objects
            .cells
            .iter()
            .enumerate()
            .filter(move |(_, cell)| cell.owner == owner)
            .map(|(index, cell)| (self.table_objects.arena_len + index, &cell.node))
    }

    /// Source boxes painted beside anonymous paragraphs, in source child order.
    #[doc(hidden)]
    pub fn anonymous_table_paint_children(&self, owner: usize) -> Option<Vec<usize>> {
        let cells: Vec<_> = self
            .table_objects
            .cells
            .iter()
            .filter(|cell| cell.owner == owner)
            .collect();
        if cells.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for child in children(self, owner) {
            if let Some(cell) = cells
                .iter()
                .find(|cell| cell.node.children.contains(&child))
            {
                if cell.node.children.first() == Some(&child) {
                    if cell.node.is_ifc_root() {
                        out.extend(cell.node.ifc_boxes());
                    } else {
                        out.extend(cell.node.children.iter().copied());
                    }
                }
            } else {
                out.push(child);
            }
        }
        Some(out)
    }

    /// Source owner of a paragraph, retaining real DOM ids for decorations.
    #[doc(hidden)]
    pub fn ifc_source_owner(&self, key: usize) -> usize {
        self.table_objects
            .cells
            .get(key.wrapping_sub(self.table_objects.arena_len))
            .map_or(key, |cell| cell.owner)
    }
}
