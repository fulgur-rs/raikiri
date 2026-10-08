//! Page placements of the first header group in ordinary block-flow tables.

use crate::Document;
use raikiri_style::{
    CascadeResult,
    property::{BreakBetween, DisplayValue, FloatValue, PositionValue, WritingMode},
};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
pub(crate) struct HeaderRepeat {
    pub(crate) table: usize,
    pub(crate) source_y: f32,
    pub(crate) height: f32,
    pub(crate) gap: f32,
    pub(crate) body_started: bool,
    pub(crate) placements: BTreeMap<u32, (f32, f32)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HeaderRepeats {
    pub(crate) headers: HashMap<usize, HeaderRepeat>,
    pub(crate) rows: HashMap<usize, usize>,
    owners: Vec<Option<usize>>,
}

impl HeaderRepeats {
    pub(crate) fn prepare(
        document: &Document,
        cascade: &CascadeResult,
        body: usize,
        page_height: f32,
        absolute_y: impl Fn(usize) -> f32,
    ) -> Self {
        let mut result = Self::default();
        result.owners.resize(document.nodes.len(), None);
        if document.nodes[body].multicol.is_some() {
            return result;
        }
        for &table in &document.nodes[body].children {
            let node = &document.nodes[table];
            let cv = &cascade.computed[table];
            if node.is_display_none()
                || cv.display != DisplayValue::Table
                || cv.writing_mode != WritingMode::HorizontalTb
                || cv.cssom_writing_mode != WritingMode::HorizontalTb
                || cv.float != FloatValue::None
                || !matches!(cv.position, PositionValue::Static | PositionValue::Relative)
            {
                continue;
            }
            let Some(header) = node.children.iter().copied().find(|&id| {
                !document.nodes[id].is_display_none()
                    && cascade.computed[id].display == DisplayValue::TableHeaderGroup
            }) else {
                continue;
            };
            let grid_rows = document
                .table_objects
                .rows
                .get(&table)
                .expect("table preparation records every rendered table");
            // This pass refreshes explicit row/cell boxes. Flattened Contents
            // and anonymous cells keep the existing table projection instead.
            if !grid_rows.iter().all(|row| {
                row.marker < document.nodes.len()
                    && document.nodes[row.marker].display == DisplayValue::TableRow
                    && row.cells.iter().all(|&cell| {
                        cell < document.nodes.len() && document.parent_of(cell) == Some(row.marker)
                    })
            }) {
                continue;
            }
            let height = document.nodes[header].unrounded_layout.size.height;
            // Oversized headers keep ordinary fragmentation rather than
            // consuming a continuation page's entire body area.
            if !height.is_finite() || height <= 0.0 || height > page_height / 4.0 {
                continue;
            }
            let mut body_rows = Vec::new();
            for &child in &node.children {
                if child == header {
                    continue;
                }
                match cascade.computed[child].display {
                    DisplayValue::TableRow => body_rows.push(child),
                    DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup => {
                        body_rows.extend(document.nodes[child].children.iter().copied().filter(
                            |&row| {
                                cascade.computed[row].display == DisplayValue::TableRow
                                    && !document.nodes[row].is_display_none()
                            },
                        ));
                    }
                    _ => {}
                }
            }
            if body_rows.is_empty() {
                continue;
            }
            // Spanning-row breaks and bottom captions need their own table
            // fragmentation pass. Preserve the existing source geometry.
            let spans_rows = grid_rows
                .iter()
                .flat_map(|row| row.cells.iter().copied())
                .any(|cell| super::get_rowspan(document, cell) != 1);
            let bottom_caption = node.children.iter().any(|&id| {
                cascade.computed[id].display == DisplayValue::TableCaption
                    && document.nodes[id].caption_side
                        == raikiri_style::property::CaptionSideValue::Bottom
            });
            let mut descendants = vec![header];
            let mut header_sources = Vec::new();
            let mut has_fixed = false;
            let mut has_page_boundary = false;
            while let Some(id) = descendants.pop() {
                let computed = &cascade.computed[id];
                has_fixed |= matches!(computed.position, PositionValue::Fixed);
                has_page_boundary |= matches!(
                    computed.break_before,
                    BreakBetween::Page | BreakBetween::Always
                ) || matches!(
                    computed.break_after,
                    BreakBetween::Page | BreakBetween::Always
                ) || matches!(
                    cascade.page_values.get(id),
                    Some(raikiri_style::property::PageValue::Named(_))
                );
                header_sources.push(id);
                descendants.extend(document.nodes[id].children.iter().copied());
            }
            if spans_rows || bottom_caption || has_fixed || has_page_boundary {
                continue;
            }
            let gap =
                if node.border_collapse == raikiri_style::property::BorderCollapseValue::Collapse {
                    0.0
                } else {
                    node.border_spacing.vertical.0.max(0.0)
                };
            for row in body_rows {
                result.rows.insert(row, header);
            }
            for id in header_sources {
                result.owners[id] = Some(header);
            }
            result.headers.insert(
                header,
                HeaderRepeat {
                    table,
                    source_y: absolute_y(header),
                    height,
                    gap,
                    body_started: false,
                    placements: BTreeMap::new(),
                },
            );
        }
        result
    }

    pub(crate) fn owner(&self, source: usize) -> Option<usize> {
        self.owners.get(source).copied().flatten()
    }

    pub(crate) fn shift(&self, source: usize, page_origin: f32) -> Option<Option<f32>> {
        let header = self.headers.get(&self.owner(source)?)?;
        Some(header.placements.values().find_map(|&(origin, y)| {
            ((origin - page_origin).abs() <= 0.001).then_some(y - header.source_y)
        }))
    }
}

impl Document {
    /// Translation of a complete repeated table-header subtree on this page.
    ///
    /// `None` identifies an ordinary source; `Some(None)` excludes this
    /// header from the page. The walker applies a translation only at the
    /// subtree root, preserving its enclosing opacity and clipping groups.
    #[doc(hidden)]
    pub fn repeated_table_header_root_shift(
        &self,
        node: usize,
        content_origin_y: f32,
    ) -> Option<Option<f32>> {
        self.table_objects.headers.headers.get(&node)?;
        self.table_objects.headers.shift(node, content_origin_y)
    }
}

fn absolute_position(document: &Document, node: usize) -> (f32, f32) {
    let mut position = (0.0, 0.0);
    let mut current = Some(node);
    while let Some(id) = current {
        let layout = document.nodes[id].unrounded_layout;
        position.0 += layout.location.x;
        position.1 += layout.location.y;
        current = document.layout_parent_of(id);
    }
    position
}

pub(crate) fn finish_boxes(document: &mut Document, headers: &HeaderRepeats) {
    for header in headers.headers.values() {
        let table = header.table;
        let table_origin = absolute_position(document, table);
        let mut parts = Vec::new();
        let mut pending = document.nodes[table].children.clone();
        while let Some(id) = pending.pop() {
            match document.nodes[id].display {
                DisplayValue::TableRow
                | DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup => {
                    parts.push(id);
                    pending.extend(document.nodes[id].children.iter().copied());
                }
                _ => {}
            }
        }
        let mut cells_by_part: HashMap<usize, Vec<raikiri_traits::PaintRect>> = HashMap::new();
        let mut table_bottom = table_origin.1;
        for &part in &parts {
            if document.nodes[part].display != DisplayValue::TableRow {
                continue;
            }
            let children = document.nodes[part].children.clone();
            for cell in children {
                let node = &document.nodes[cell];
                if node.display != DisplayValue::TableCell || node.is_display_none() {
                    continue;
                }
                let (x, y) = absolute_position(document, cell);
                let size = node.unrounded_layout.size;
                table_bottom = table_bottom.max(y + size.height);
                let hidden = node.hides_empty_table_cell;
                let mut current = Some(part);
                while let Some(id) = current.filter(|&id| id != table) {
                    // The traversal above reaches rows only through explicit
                    // table-part parents, so every ancestor before the table
                    // owns the row's cell-background intersections.
                    let origin = absolute_position(document, id);
                    let bottom = y + size.height - origin.1;
                    document.nodes[id].unrounded_layout.size.height =
                        document.nodes[id].unrounded_layout.size.height.max(bottom);
                    if !hidden {
                        cells_by_part
                            .entry(id)
                            .or_default()
                            .push(raikiri_traits::PaintRect::new(
                                x - origin.0,
                                y - origin.1,
                                size.width,
                                size.height,
                            ));
                    }
                    current = document.parent_of(id);
                }
            }
        }
        let layout = &mut document.nodes[table].unrounded_layout;
        layout.size.height = layout.size.height.max(
            table_bottom - table_origin.1
                + header.gap
                + layout.padding.bottom
                + layout.border.bottom,
        );
        let table_height = layout.size.height;
        if let Some(grid) = &mut document.nodes[table].table_grid_box {
            grid.height = (table_height - grid.y).max(grid.height);
        }
        for (id, cells) in cells_by_part {
            if let Some(clips) = document.table_objects.part_background_cells.get_mut(&id) {
                *clips = cells;
            }
        }
    }
}
