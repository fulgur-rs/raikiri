//! Layout-only anonymous table boxes, without changing the source DOM:
//! cells that wrap loose content in tables, rows and row groups, and tables
//! that wrap rows and row groups outside any table.

use crate::Document;
use crate::node::arena::BoxKey;
use crate::node::{Node, NodeFlags};
use raikiri_layout_geometry::BoxLayout;
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
}

/// An anonymous table around a run of rows and row groups whose parent is
/// a block container (CSS 2.1 §17.2.1 rule 3, CSS Tables 3 §3.9).
#[derive(Debug, Clone)]
pub(crate) struct AnonymousTable {
    /// The block container that holds the wrapped parts in the DOM.
    pub(crate) parent: usize,
    /// The wrapped rows and row groups, in document order.
    pub(crate) parts: Vec<usize>,
    /// The used boxes of `parts` relative to the table, as table layout
    /// last placed them. Their stored boxes add the table's location, so
    /// boxes summed along DOM ancestors place them in the table.
    pub(crate) relative_parts: Vec<BoxLayout>,
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
    pub(crate) cells: Vec<AnonymousCell>,
    /// The layout views of `cells`, moved into the node arena once built.
    cell_nodes: Vec<Node>,
    pub(crate) tables: Vec<AnonymousTable>,
    /// The anonymous table index of each wrapped row or row group.
    table_by_part: HashMap<usize, usize>,
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

impl TableObjects {
    /// Whether a table lays out the children of `owner` as its rows or
    /// cells: `owner` is a table or one of its row groups or rows.
    pub(crate) fn lays_out_children(&self, owner: usize) -> bool {
        self.content_by_owner.contains_key(&owner)
    }

    /// The anonymous table index that wraps the source part `id`.
    pub(crate) fn table_of_part(&self, id: usize) -> Option<usize> {
        self.table_by_part.get(&id).copied()
    }
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
    // An anonymous table has no generated content of its own.
    let source = doc.is_source_node(owner);
    if source
        && crate::generated_content::is_in_flow_generated_text(cascade, owner, PseudoElem::After)
    {
        pending.insert(0, Content::After(owner));
    }
    if source
        && crate::generated_content::is_in_flow_generated_text(cascade, owner, PseudoElem::Before)
    {
        pending.push(Content::Before(owner));
    }
    while let Some(content) = pending.pop() {
        let Content::Node(id) = content else {
            out.push(content);
            continue;
        };
        let node = doc.ifc_layout_node(id).unwrap();
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || node.display() == DisplayValue::None
            || !matches!(node.kind(), NodeKind::Element | NodeKind::Text)
        {
            continue;
        }
        if node.display() == DisplayValue::Contents {
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

/// The layout node that the anonymous cells in the source box `owner` are
/// copied from.
fn anonymous_prototype(doc: &Document, owner: usize) -> Node {
    let source = doc.ifc_layout_node(owner).unwrap();
    // Anonymous boxes have no source attributes, children or retained layout
    // payloads. Text/style projection reads the original owner instead.
    let mut node = Node::new_element(source.tag_name().unwrap_or("div").into(), None);
    if let crate::node::NodeData::Element(source_data) = &source.data {
        let target = node.data.as_element_mut().expect("element prototype");
        target.namespace = source_data.namespace.clone();
        target.prefix = source_data.prefix.clone();
    }
    // The cell's box sits in its owner's box, where its location is measured
    // from.
    node.parent = Some(owner);
    node.flags = source.flags;
    node.flags
        .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
    node.computed_border = Some(raikiri_style::ComputedValues::initial().border);
    node.set_display(DisplayValue::TableCell);
    node.border_collapse = source.border_collapse;
    node.border_spacing = source.border_spacing;
    node.caption_side = source.caption_side;
    node.authored_writing_mode = source.authored_writing_mode;
    node.used_writing_mode = source.used_writing_mode;
    node.under_vertical_writing = source.under_vertical_writing;
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
    let index = objects.cells.len();
    let key = doc.nodes.anonymous_cell_key(index);
    objects.cells_by_owner.entry(owner).or_default().push(index);
    for &entry in &content {
        objects.cell_by_content.insert(entry, index);
    }
    objects.cells.push(AnonymousCell { owner, content });
    objects.cell_nodes.push(node);
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
        if matches!(entry, Content::Node(_)) && doc.nodes[id].display() == DisplayValue::TableCell {
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
                && doc.nodes[id.node()].display() == DisplayValue::TableHeaderGroup
        });
        let first_foot = ids.iter().copied().find(|&id| {
            matches!(id, Content::Node(_))
                && doc.nodes[id.node()].display() == DisplayValue::TableFooterGroup
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
        match doc.nodes[id].display() {
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

fn is_row_or_group(display: DisplayValue) -> bool {
    matches!(
        display,
        DisplayValue::TableRow
            | DisplayValue::TableRowGroup
            | DisplayValue::TableHeaderGroup
            | DisplayValue::TableFooterGroup
    )
}

/// Whether `parent` lays its children out as block-level boxes, so that a
/// run of rows among them can be wrapped in an anonymous block-level table.
///
/// Inline content would make the parent a paragraph root, whose children the
/// inline engine lays out instead; its rows remain blocks there. List items,
/// whose markers sit on their first line, and vertical and multicol
/// containers keep their rows as blocks as well.
fn holds_only_block_level_boxes(doc: &Document, cascade: &CascadeResult, parent: usize) -> bool {
    let node = doc.ifc_layout_node(parent).unwrap();
    let cv = &cascade.computed[parent];
    if !matches!(
        node.display(),
        DisplayValue::Block
            | DisplayValue::FlowRoot
            | DisplayValue::TableCell
            | DisplayValue::TableCaption
    ) || cv.cssom_writing_mode.is_vertical()
        || !matches!(
            cv.column_count,
            raikiri_style::property::ColumnCountValue::Auto
        )
        || !matches!(cv.column_width, raikiri_style::ComputedColumnWidth::Auto)
        || node.is_inline_svg_root()
    {
        return false;
    }
    let generates = |pseudo| {
        cascade
            .pseudo
            .get(&(raikiri_style::StyleNodeId::new(parent as u64), pseudo))
            .is_some_and(|cv| cv.display != DisplayValue::None && !cv.content.is_empty())
    };
    if generates(PseudoElem::Before) || generates(PseudoElem::After) {
        return false;
    }
    node.children.iter().all(|&child| {
        let child_node = doc.ifc_layout_node(child).unwrap();
        !child_node.is_in_document()
            || child_node.is_non_rendered_html_element()
            || if child_node.kind() == NodeKind::Text {
                // White space that `white-space` preserves forms a line.
                !crate::layout::ifc::assign::text_makes_a_line(doc, cascade, child)
            } else {
                // Only text and elements are in the document.
                let display = child_node.display();
                display == DisplayValue::None
                    || (!display.is_inline_level() && display != DisplayValue::Contents)
            }
    })
}

/// Group the rows and row groups outside any table into the runs that one
/// anonymous table each wraps. White space and boxes that generate nothing
/// between two parts do not end a run.
fn anonymous_table_runs(doc: &Document, cascade: &CascadeResult) -> Vec<AnonymousTable> {
    let mut tables = Vec::new();
    for parent in 0..doc.nodes.len() {
        let node = doc.ifc_layout_node(parent).unwrap();
        if !node.is_in_document()
            || node.kind() != NodeKind::Element
            || !node
                .children
                .iter()
                .any(|&child| is_row_or_group(doc.nodes[child].display()))
            || !holds_only_block_level_boxes(doc, cascade, parent)
        {
            continue;
        }
        let mut run = Vec::new();
        for &child in &node.children {
            let child_node = doc.ifc_layout_node(child).unwrap();
            if child_node.kind() == NodeKind::Element
                && child_node.is_in_document()
                && is_row_or_group(child_node.display())
            {
                run.push(child);
            } else if child_node.kind() == NodeKind::Element
                && child_node.is_in_document()
                && !child_node.is_non_rendered_html_element()
                && child_node.display() != DisplayValue::None
                && !run.is_empty()
            {
                tables.push(AnonymousTable {
                    parent,
                    parts: std::mem::take(&mut run),
                    relative_parts: Vec::new(),
                });
            }
        }
        if !run.is_empty() {
            tables.push(AnonymousTable {
                parent,
                parts: run,
                relative_parts: Vec::new(),
            });
        }
    }
    tables
}

/// The layout view of the anonymous table in `parent` around `parts`. It
/// inherits its inherited table properties from `parent` and has initial
/// values for the rest.
fn anonymous_table_node(doc: &Document, parent: usize, parts: &[usize]) -> Node {
    let source = doc.ifc_layout_node(parent).unwrap();
    let mut node = Node::new_element("div".into(), None);
    if let crate::node::NodeData::Element(source_data) = &source.data {
        let target = node.data.as_element_mut().expect("element prototype");
        target.namespace = source_data.namespace.clone();
    }
    node.parent = Some(parent);
    node.children = parts.to_vec();
    node.flags = NodeFlags::IS_IN_DOCUMENT | NodeFlags::IS_TABLE_ROOT;
    node.computed_border = Some(raikiri_style::ComputedValues::initial().border);
    node.set_display(DisplayValue::Table);
    node.border_collapse = source.border_collapse;
    node.border_spacing = source.border_spacing;
    node.caption_side = source.caption_side;
    node.authored_writing_mode = None;
    node.used_writing_mode = source.used_writing_mode;
    node.under_vertical_writing = source.under_vertical_writing;
    node
}

pub(crate) fn prepare(doc: &mut Document, cascade: &CascadeResult) -> Result<(), LayoutError> {
    // Source parts may no longer contain anonymous cells on this pass. Their
    // previous parent-relative boxes must not shift ordinary table cells.
    for &id in &doc.table_objects.materialized_parts {
        doc.nodes[id].unrounded_layout = BoxLayout::new();
    }
    // An unsuccessful rebuild must not expose cells from the previous pass.
    doc.table_objects = TableObjects::default();
    doc.set_anonymous_cells(Vec::new());
    let tables = anonymous_table_runs(doc, cascade);
    let table_nodes = tables
        .iter()
        .map(|table| anonymous_table_node(doc, table.parent, &table.parts))
        .collect();
    // The tables are set first: their number fixes the keys of the cells.
    doc.set_anonymous_tables(table_nodes);
    let mut objects = TableObjects {
        paragraph_owner: vec![None; doc.nodes.len()],
        ..TableObjects::default()
    };
    for (index, table) in tables.iter().enumerate() {
        for &part in &table.parts {
            objects.table_by_part.insert(part, index);
        }
    }
    let table_keys = (0..tables.len()).map(|index| doc.nodes.anonymous_table_key(index));
    let source_tables = (0..doc.nodes.len()).filter(|&id| {
        doc.nodes[id].is_in_document()
            && matches!(
                doc.nodes[id].display(),
                DisplayValue::Table | DisplayValue::InlineTable
            )
    });
    for id in source_tables.chain(table_keys).collect::<Vec<_>>() {
        match rows(doc, cascade, &mut objects, id, true, &mut 0) {
            Ok(table_rows) => {
                objects.rows.insert(id, table_rows);
                objects.prototypes_by_owner.clear();
            }
            Err(error) => {
                doc.set_anonymous_tables(Vec::new());
                return Err(error);
            }
        }
    }
    objects.tables = tables;
    doc.set_anonymous_cells(std::mem::take(&mut objects.cell_nodes));
    doc.table_objects = objects;
    for index in 0..doc.table_objects.tables.len() {
        let key = doc.nodes.anonymous_table_key(index);
        let owner = doc.table_objects.tables[index].parent;
        let facts = crate::layout::box_style_data::recipes::TableFacts::from_owner(
            doc, cascade, key, owner,
        );
        let recipe = crate::layout::box_style_data::recipes::anonymous_table(facts);
        doc.box_style_data
            .publish_recipe(BoxKey::AnonymousTable(index), recipe);
    }
    for index in 0..doc.table_objects.cells.len() {
        let key = doc.nodes.anonymous_cell_key(index);
        let owner = doc.table_objects.cells[index].owner;
        let facts = crate::layout::box_style_data::recipes::TableFacts::from_owner(
            doc, cascade, key, owner,
        );
        let recipe = crate::layout::box_style_data::recipes::anonymous_cell(facts);
        doc.box_style_data
            .publish_recipe(BoxKey::AnonymousCell(index), recipe);
    }
    Ok(())
}

impl Document {
    /// The layout view of an IFC root, including a layout-only anonymous cell.
    /// Internal paragraph keys are distinct from the source DOM node arena.
    #[doc(hidden)]
    pub fn ifc_layout_node(&self, key: usize) -> Option<&Node> {
        self.nodes.layout_node(key)
    }

    /// The layout view of the anonymous cell `key`, or `None` when `key`
    /// names a source node.
    #[doc(hidden)]
    pub fn anonymous_table_cell(&self, key: usize) -> Option<&Node> {
        self.is_anonymous_cell(key).then(|| &self.nodes[key])
    }

    /// The layout key of the anonymous cell that wraps the source box `id`,
    /// if table fixup wrapped it in one.
    pub(crate) fn anonymous_cell_of(&self, id: usize) -> Option<usize> {
        if self.table_objects.cells.is_empty() {
            return None;
        }
        self.table_objects
            .cell_by_content
            .get(&Content::Node(id))
            .map(|&index| self.nodes.anonymous_cell_key(index))
    }

    pub(crate) fn table_layout_node_mut(&mut self, key: usize) -> &mut Node {
        &mut self.nodes[key]
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
                let key = self.nodes.anonymous_cell_key(index);
                (key, &self.nodes[key])
            })
    }

    /// Source boxes beside anonymous paragraphs, followed by pseudo overlays.
    #[doc(hidden)]
    pub fn anonymous_table_paint_children(&self, owner: usize) -> Option<Vec<usize>> {
        self.anonymous_table_paint_sequence(owner).map(|items| {
            items
                .into_iter()
                .filter(|&key| !self.nodes.is_anonymous_cell(key))
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
                let key = self.nodes.anonymous_cell_key(index);
                let node = self.ifc_layout_node(key).unwrap();
                if self.table_objects.cells[index].content.first() == Some(&content) {
                    if node.is_ifc_root() {
                        out.push(key);
                        out.extend(node.ifc_boxes());
                    } else {
                        out.extend(node.children.iter().copied());
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
    /// An anonymous table takes its parent, which it inherits from.
    #[doc(hidden)]
    pub fn ifc_source_owner(&self, key: usize) -> usize {
        match self.nodes.box_key(key) {
            Some(BoxKey::AnonymousCell(index)) => self.table_objects.cells[index].owner,
            Some(BoxKey::AnonymousTable(index)) => self.table_objects.tables[index].parent,
            _ => key,
        }
    }

    /// Whether a layout key names a layout-only anonymous table cell.
    pub(crate) fn is_anonymous_cell(&self, key: usize) -> bool {
        self.nodes.is_anonymous_cell(key)
    }

    /// Whether a layout key names a layout-only anonymous table.
    pub(crate) fn is_anonymous_table(&self, key: usize) -> bool {
        self.nodes.is_anonymous_table(key)
    }

    /// The layout key of the anonymous table that wraps the source row or
    /// row group `id`, if table fixup wrapped it in one.
    pub(crate) fn anonymous_table_of(&self, id: usize) -> Option<usize> {
        self.table_objects
            .table_of_part(id)
            .map(|index| self.nodes.anonymous_table_key(index))
    }

    /// The box at which table structure ends above the parts of `table`:
    /// the table itself, or the parent of an anonymous table, which is not
    /// a DOM ancestor of the parts it wraps.
    pub(crate) fn table_parts_boundary(&self, table: usize) -> usize {
        match self.nodes.box_key(table) {
            Some(BoxKey::AnonymousTable(index)) => self.table_objects.tables[index].parent,
            _ => table,
        }
    }

    /// The rows and row groups that the anonymous table `table` wraps.
    pub(crate) fn anonymous_table_parts(&self, table: usize) -> Option<&[usize]> {
        match self.nodes.box_key(table) {
            Some(BoxKey::AnonymousTable(index)) => Some(&self.table_objects.tables[index].parts),
            _ => None,
        }
    }

    /// Record the used boxes of an anonymous table's parts relative to the
    /// table, then store them as their parent sees them.
    pub(crate) fn place_anonymous_table_parts(&mut self, table: usize) {
        let Some(BoxKey::AnonymousTable(index)) = self.nodes.box_key(table) else {
            return;
        };
        let relative = self.table_objects.tables[index]
            .parts
            .iter()
            .map(|&part| self.nodes[part].unrounded_layout)
            .collect();
        self.table_objects.tables[index].relative_parts = relative;
        self.offset_anonymous_table_parts(table);
    }

    /// Store the boxes of an anonymous table's parts relative to their
    /// parent: the box relative to the table plus the table's location.
    pub(crate) fn offset_anonymous_table_parts(&mut self, table: usize) {
        let Some(BoxKey::AnonymousTable(index)) = self.nodes.box_key(table) else {
            return;
        };
        let origin = self.nodes[table].unrounded_layout.location;
        let anonymous = &self.table_objects.tables[index];
        for (&part, relative) in anonymous.parts.iter().zip(&anonymous.relative_parts) {
            let mut layout = *relative;
            layout.location.x += origin.x;
            layout.location.y += origin.y;
            self.nodes[part].unrounded_layout = layout;
        }
    }

    /// Whether a layout key names a source DOM node.
    pub(crate) fn is_source_node(&self, key: usize) -> bool {
        matches!(self.nodes.box_key(key), Some(BoxKey::Source(_)))
    }
}

#[cfg(test)]
thread_local! {
    static OWNER_CELL_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static OWNER_CLONE_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
