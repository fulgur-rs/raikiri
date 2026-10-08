//! Layout-only anonymous table cells, without changing the source DOM.

use crate::Document;
use crate::node::{Node, NodeFlags};
use raikiri_style::property::DisplayValue;
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::LayoutError;
use raikiri_traits::NodeKind;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Content {
    Node(usize),
    Before(usize),
    After(usize),
}

impl Content {
    fn node(self) -> usize {
        match self {
            Self::Node(id) | Self::Before(id) | Self::After(id) => id,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AnonymousCell {
    pub(crate) owner: usize,
    pub(crate) content: Vec<Content>,
    pub(crate) node: Node,
}

#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) marker: usize,
    pub(crate) cells: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TableObjects {
    pub(crate) project_explicit_parts: bool,
    pub(crate) headers: super::headers::HeaderRepeats,
    pub(crate) arena_len: usize,
    pub(crate) cells: Vec<AnonymousCell>,
    cells_by_owner: HashMap<usize, Vec<usize>>,
    cell_by_content: HashMap<Content, usize>,
    content_by_owner: HashMap<usize, Vec<Content>>,
    overlay_contents_by_owner: HashMap<usize, Vec<usize>>,
    overlay_contents_owner: HashMap<usize, usize>,
    prototypes_by_owner: HashMap<usize, Node>,
    pub(crate) rows: HashMap<usize, Vec<Row>>,
    pub(crate) paragraph_owner: Vec<Option<usize>>,
    pub(crate) part_background_cells: HashMap<usize, Vec<raikiri_traits::PaintRect>>,
    pub(crate) materialized_parts: HashSet<usize>,
}

fn whitespace(doc: &Document, content: Content) -> bool {
    let Content::Node(id) = content else {
        return false;
    };
    doc.nodes[id].kind() == NodeKind::Text
        && doc.nodes[id].text_content().is_some_and(|text| {
            text.chars()
                .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        })
}

fn children(
    doc: &Document,
    cascade: &CascadeResult,
    owner: usize,
    objects: &mut TableObjects,
) -> Vec<Content> {
    let mut out = Vec::new();
    let mut pending: Vec<_> = doc.nodes[owner]
        .children
        .iter()
        .rev()
        .map(|&id| Content::Node(id))
        .collect();
    if crate::generated_content::is_in_flow_generated_text(cascade, owner, PseudoElem::After) {
        pending.insert(0, Content::After(owner));
    }
    if crate::generated_content::is_in_flow_generated_text(cascade, owner, PseudoElem::Before) {
        pending.push(Content::Before(owner));
    }
    while let Some(content) = pending.pop() {
        let Content::Node(id) = content else {
            out.push(content);
            continue;
        };
        let node = &doc.nodes[id];
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || node.display == DisplayValue::None
            || !matches!(node.kind(), NodeKind::Element | NodeKind::Text)
        {
            continue;
        }
        if node.display == DisplayValue::Contents {
            let has_overlay = [PseudoElem::Before, PseudoElem::After]
                .into_iter()
                .any(|pseudo| {
                    cascade
                        .pseudo
                        .get(&(raikiri_style::StyleNodeId::new(id as u64), pseudo))
                        .is_some_and(|cv| {
                            cv.display != DisplayValue::None
                                && !cv.content.is_empty()
                                && !cv.content.iter().any(|part| {
                                    matches!(part, raikiri_style::property::ContentComponent::None)
                                })
                        })
                        && !crate::generated_content::is_in_flow_generated_text(cascade, id, pseudo)
                });
            if has_overlay {
                objects
                    .overlay_contents_by_owner
                    .entry(owner)
                    .or_default()
                    .push(id);
                objects.overlay_contents_owner.insert(id, owner);
            }
            if crate::generated_content::is_in_flow_generated_text(cascade, id, PseudoElem::After) {
                pending.push(Content::After(id));
            }
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|&child| Content::Node(child)),
            );
            if crate::generated_content::is_in_flow_generated_text(cascade, id, PseudoElem::Before)
            {
                pending.push(Content::Before(id));
            }
        } else {
            out.push(content);
        }
    }
    out
}

fn anonymous_prototype(doc: &Document, owner: usize) -> Node {
    let source = &doc.nodes[owner];
    // Anonymous boxes have no source attributes, children or retained layout
    // payloads. Text/style projection reads the original owner before layout;
    // the temporary Taffy view is restored before any source metadata is read.
    let mut node = Node::new_element(
        source.tag_name().unwrap_or("div").into(),
        taffy::Style::default(),
        None,
    );
    if let crate::node::NodeData::Element(source_data) = &source.data {
        let target = node.data.as_element_mut().expect("element prototype");
        target.namespace = source_data.namespace.clone();
        target.prefix = source_data.prefix.clone();
    }
    node.parent = source.parent;
    node.flags = source.flags;
    node.flags
        .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
    node.style.display = taffy::Display::Block;
    node.style.direction = source.style.direction;
    node.computed_border = Some(raikiri_style::ComputedValues::initial().border);
    node.display = DisplayValue::TableCell;
    node.border_collapse = source.border_collapse;
    node.border_spacing = source.border_spacing;
    node.caption_side = source.caption_side;
    node.authored_writing_mode = source.authored_writing_mode;
    node
}

fn anonymous_cell(
    doc: &Document,
    cascade: &CascadeResult,
    objects: &mut TableObjects,
    owner: usize,
    content: Vec<Content>,
) -> usize {
    let prototype = objects
        .prototypes_by_owner
        .entry(owner)
        .or_insert_with(|| anonymous_prototype(doc, owner));
    #[cfg(test)]
    tests::record_owner_clone(prototype);
    let mut node = prototype.clone();
    node.children = content
        .iter()
        .filter_map(|entry| match entry {
            Content::Node(id) => Some(*id),
            _ => None,
        })
        .collect();
    let visible_generated = content.iter().any(|entry| {
        let pseudo = match entry {
            Content::Node(_) => return false,
            Content::Before(_) => PseudoElem::Before,
            Content::After(_) => PseudoElem::After,
        };
        cascade
            .pseudo
            .get(&(raikiri_style::StyleNodeId::new(entry.node() as u64), pseudo))
            .is_some_and(|cv| cv.visibility != raikiri_style::property::Visibility::Hidden)
    });
    node.hides_empty_table_cell =
        crate::paint_rules::hides_anonymous_table_cell(doc, cascade, owner, &node.children)
            && (cascade.computed[owner].visibility == raikiri_style::property::Visibility::Hidden
                || !visible_generated);
    let key = objects.arena_len + objects.cells.len();
    let index = objects.cells.len();
    objects.cells_by_owner.entry(owner).or_default().push(index);
    for &entry in &content {
        objects.cell_by_content.insert(entry, index);
    }
    objects.cells.push(AnonymousCell {
        owner,
        content,
        node,
    });
    key
}

fn row(
    doc: &Document,
    cascade: &CascadeResult,
    objects: &mut TableObjects,
    owner: usize,
    ids: &[Content],
    marker: Option<usize>,
    row_count: &mut usize,
) -> Result<Row, LayoutError> {
    // Apply the native grid's exact row-index bound before allocating a
    // projected row or its anonymous cells.
    u16::try_from(*row_count).map_err(|_| super::table_column_error())?;
    *row_count += 1;
    let mut cells = Vec::new();
    let mut pending = Vec::new();
    let mut column = 0;
    let flush = |pending: &mut Vec<Content>,
                 cells: &mut Vec<usize>,
                 objects: &mut TableObjects,
                 column: &mut u16|
     -> Result<(), LayoutError> {
        // Inter-cell white space generates no anonymous cell. Keep white
        // space within an actual content run for the paragraph to collapse.
        if pending.iter().any(|&id| !whitespace(doc, id)) {
            *column = super::next_table_column(*column, 1)?;
            cells.push(anonymous_cell(
                doc,
                cascade,
                objects,
                owner,
                std::mem::take(pending),
            ));
        } else {
            pending.clear();
        }
        Ok(())
    };
    if marker.is_some() {
        objects.content_by_owner.insert(owner, ids.to_vec());
    }
    for &entry in ids {
        let id = entry.node();
        if matches!(entry, Content::Node(_)) && doc.nodes[id].display == DisplayValue::TableCell {
            flush(&mut pending, &mut cells, objects, &mut column)?;
            column = super::next_table_column(column, super::get_colspan(doc, id))?;
            cells.push(id);
        } else {
            pending.push(entry);
        }
    }
    flush(&mut pending, &mut cells, objects, &mut column)?;
    Ok(Row {
        marker: marker.unwrap_or_else(|| cells.first().copied().unwrap_or(owner)),
        cells,
    })
}

fn rows(
    doc: &Document,
    cascade: &CascadeResult,
    objects: &mut TableObjects,
    owner: usize,
    reorder: bool,
    row_count: &mut usize,
) -> Result<Vec<Row>, LayoutError> {
    let mut ids = children(doc, cascade, owner, objects);
    objects.content_by_owner.insert(owner, ids.clone());
    if reorder {
        let first_head = ids.iter().copied().find(|&id| {
            matches!(id, Content::Node(_))
                && doc.nodes[id.node()].display == DisplayValue::TableHeaderGroup
        });
        let first_foot = ids.iter().copied().find(|&id| {
            matches!(id, Content::Node(_))
                && doc.nodes[id.node()].display == DisplayValue::TableFooterGroup
        });
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
    let flush = |pending: &mut Vec<Content>,
                 out: &mut Vec<Row>,
                 objects: &mut TableObjects,
                 row_count: &mut usize|
     -> Result<(), LayoutError> {
        if pending.iter().any(|&id| !whitespace(doc, id)) {
            out.push(row(doc, cascade, objects, owner, pending, None, row_count)?);
        }
        pending.clear();
        Ok(())
    };
    for entry in ids {
        let id = entry.node();
        if !matches!(entry, Content::Node(_)) {
            pending.push(entry);
            continue;
        }
        match doc.nodes[id].display {
            DisplayValue::TableRow => {
                flush(&mut pending, &mut out, objects, row_count)?;
                let content = children(doc, cascade, id, objects);
                out.push(row(
                    doc,
                    cascade,
                    objects,
                    id,
                    &content,
                    Some(id),
                    row_count,
                )?);
            }
            DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup => {
                flush(&mut pending, &mut out, objects, row_count)?;
                out.extend(rows(doc, cascade, objects, id, false, row_count)?);
            }
            DisplayValue::TableCaption
            | DisplayValue::TableColumn
            | DisplayValue::TableColumnGroup => {
                flush(&mut pending, &mut out, objects, row_count)?;
            }
            _ => pending.push(entry),
        }
    }
    flush(&mut pending, &mut out, objects, row_count)?;
    Ok(out)
}

pub(crate) fn prepare(doc: &mut Document, cascade: &CascadeResult) -> Result<(), LayoutError> {
    // Source parts may no longer contain anonymous cells on this pass. Their
    // previous parent-relative boxes must not shift ordinary table cells.
    for &id in &doc.table_objects.materialized_parts {
        doc.nodes[id].unrounded_layout = taffy::Layout::new();
    }
    // An unsuccessful rebuild must not expose cells from the previous pass.
    doc.table_objects = TableObjects::default();
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
        {
            let table_rows = rows(doc, cascade, &mut objects, id, true, &mut 0)?;
            objects.rows.insert(id, table_rows);
            objects.prototypes_by_owner.clear();
        }
    }
    doc.table_objects = objects;
    Ok(())
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
            .cells_by_owner
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|&index| {
                #[cfg(test)]
                OWNER_CELL_VISITS.with(|visits| visits.set(visits.get() + 1));
                (
                    self.table_objects.arena_len + index,
                    &self.table_objects.cells[index].node,
                )
            })
    }

    /// Source boxes beside anonymous paragraphs, followed by pseudo overlays.
    #[doc(hidden)]
    pub fn anonymous_table_paint_children(&self, owner: usize) -> Option<Vec<usize>> {
        self.anonymous_table_paint_sequence(owner).map(|items| {
            items
                .into_iter()
                .filter(|&key| key < self.nodes.len())
                .collect()
        })
    }

    /// Reconstructed paragraph and source-box order, followed by pseudo overlays.
    #[doc(hidden)]
    pub fn anonymous_table_paint_sequence(&self, owner: usize) -> Option<Vec<usize>> {
        self.table_objects.cells_by_owner.get(&owner)?;
        let mut out = Vec::new();
        for &content in &self.table_objects.content_by_owner[&owner] {
            #[cfg(test)]
            OWNER_CELL_VISITS.with(|visits| visits.set(visits.get() + 1));
            if let Some(&index) = self.table_objects.cell_by_content.get(&content) {
                let cell = &self.table_objects.cells[index];
                if cell.content.first() == Some(&content) {
                    if cell.node.is_ifc_root() {
                        out.push(self.table_objects.arena_len + index);
                        out.extend(cell.node.ifc_boxes());
                    } else {
                        out.extend(cell.node.children.iter().copied());
                    }
                }
            } else {
                out.push(content.node());
            }
        }
        // Flattened descendants already occur in the normal paint sequence.
        // Retain only the wrappers' overlay paint, outside cell shaping.
        out.extend(
            self.table_objects
                .overlay_contents_by_owner
                .get(&owner)
                .into_iter()
                .flatten(),
        );
        Some(out)
    }

    /// Whether flattened contents are visited only for their pseudo overlays.
    #[doc(hidden)]
    pub fn anonymous_table_contents_paint_only(&self, id: usize) -> bool {
        self.table_objects
            .overlay_contents_owner
            .get(&id)
            .is_some_and(|owner| self.table_objects.cells_by_owner.contains_key(owner))
    }

    /// Whether this generated source was projected into an anonymous cell.
    #[doc(hidden)]
    pub fn anonymous_table_pseudo_is_projected(&self, id: usize, pseudo: PseudoElem) -> bool {
        let content = match pseudo {
            PseudoElem::Before => Content::Before(id),
            PseudoElem::After => Content::After(id),
            _ => return false,
        };
        self.table_objects.cell_by_content.contains_key(&content)
    }

    /// Cell areas of a source row or group, relative to its used box.
    /// Separated table backgrounds leave the spaces between cells uncovered.
    #[doc(hidden)]
    pub fn anonymous_table_part_background_cells(
        &self,
        owner: usize,
    ) -> Option<&[raikiri_traits::PaintRect]> {
        self.table_objects
            .part_background_cells
            .get(&owner)
            .map(Vec::as_slice)
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

#[cfg(test)]
thread_local! {
    static OWNER_CELL_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static OWNER_CLONE_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
