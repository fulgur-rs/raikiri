//! Page placements of the first header and footer groups in ordinary
//! block-flow tables.

use crate::Document;
use raikiri_style::{
    CascadeResult,
    property::{BreakBetween, DisplayValue, FloatValue, PositionValue},
};
use std::collections::{BTreeMap, HashMap};

/// A header or footer group repeated on each page of its table.
#[derive(Debug, Clone)]
pub(crate) struct HeaderRepeat {
    pub(crate) table: usize,
    pub(crate) source_y: f32,
    pub(crate) height: f32,
    pub(crate) gap: f32,
    pub(crate) body_started: bool,
    /// Whether the group is the table's footer, which closes each page of
    /// the table rather than opening it.
    pub(crate) footer: bool,
    /// For a footer, the page a body row of the table last ended on and the
    /// block end of that row: the footer copy of the page follows it.
    pub(crate) last_row: Option<(u32, f32)>,
    /// For a footer, the flow shift when pagination reached its source.
    pub(crate) visit_shift: Option<f32>,
    /// The table's block start before pagination.
    pub(crate) table_source_y: f32,
    /// Page origin and header y per page index. Call
    /// [`HeaderRepeats::index`] after changing it.
    pub(crate) placements: BTreeMap<u32, (f32, f32)>,
    /// `placements` ordered by page origin, rebuilt by [`HeaderRepeats::index`].
    by_origin: Vec<(f32, f32)>,
}

impl HeaderRepeat {
    fn new(table: (usize, f32), source_y: f32, height: f32, gap: f32, footer: bool) -> Self {
        let (table, table_source_y) = table;
        Self {
            table,
            table_source_y,
            source_y,
            height,
            gap,
            body_started: false,
            footer,
            last_row: None,
            visit_shift: None,
            placements: BTreeMap::new(),
            by_origin: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HeaderRepeats {
    /// The repeated header and footer groups, by group.
    pub(crate) headers: HashMap<usize, HeaderRepeat>,
    /// The repeated header group of each body row.
    pub(crate) rows: HashMap<usize, usize>,
    /// The repeated footer group of each body row.
    pub(crate) footer_rows: HashMap<usize, usize>,
    /// Body boxes that do not break across pages, with the header and footer
    /// groups of their table: content they carry past a page edge runs under
    /// the copies of those groups.
    pub(crate) monolithic: Vec<(usize, [Option<usize>; 2])>,
    /// The last body row of each table that repeats its footer.
    pub(crate) last_rows: std::collections::HashSet<usize>,
    /// The repeated header group of each table whose body rows are all
    /// anonymous: their boxes are not placed as rows.
    pub(crate) anonymous_body_tables: HashMap<usize, usize>,
    owners: Vec<Option<usize>>,
}

impl HeaderRepeats {
    /// Finds the tables among the children of `body` whose first header
    /// group, first footer group, or both, repeat on each page of the table.
    ///
    /// A body row is a row outside those groups, or a box of the table's
    /// content that sits in an anonymous row. `monolithic` tells which boxes
    /// do not break across pages.
    pub(crate) fn prepare(
        document: &Document,
        cascade: &CascadeResult,
        body: usize,
        page_height: f32,
        absolute_y: impl Fn(usize) -> f32,
        monolithic: impl Fn(usize) -> bool,
    ) -> Self {
        let mut result = Self::default();
        result.owners.resize(document.nodes.len(), None);
        if document.nodes[body].multicol.is_some() {
            return result;
        }
        // Projection copies every repeated box onto every continuation page
        // and charges each copy to the shared fragment budget. Tables whose
        // copies would claim more than half of it keep ordinary
        // fragmentation, so a wide header on a long table cannot turn a
        // layout that succeeds without repetition into a limit failure.
        let mut copy_budget = document.fragment_tree.limit / 2;
        for &table in &document.nodes[body].children {
            let node = document.ifc_layout_node(table).unwrap();
            let cv = &cascade.computed[table];
            if node.is_display_none()
                || node.display() != DisplayValue::Table
                || cv.writing_mode.is_vertical()
                || cv.cssom_writing_mode.is_vertical()
                || cv.float != FloatValue::None
                || !matches!(
                    node.position(),
                    PositionValue::Static | PositionValue::Relative
                )
            {
                continue;
            }
            let first_group = |display: DisplayValue| {
                node.children.iter().copied().find(|&id| {
                    !document.nodes[id].is_display_none() && document.nodes[id].display() == display
                })
            };
            let header = first_group(DisplayValue::TableHeaderGroup);
            let footer = first_group(DisplayValue::TableFooterGroup);
            if header.is_none() && footer.is_none() {
                continue;
            }
            let grid_rows = document
                .table_objects
                .rows
                .get(&table)
                .expect("table preparation records every rendered table");
            // This pass refreshes row and cell boxes, explicit or anonymous
            // (an anonymous row is marked by its first anonymous cell). An
            // anonymous row holds only anonymous cells. Cells flattened out
            // of a `display: contents` row keep the existing table
            // projection instead.
            let anonymous = |id: usize| document.is_anonymous_cell(id);
            if !grid_rows.iter().all(|row| {
                if anonymous(row.marker) {
                    return row.cells.iter().all(|&cell| anonymous(cell));
                }
                document.nodes[row.marker].display() == DisplayValue::TableRow
                    && row.cells.iter().all(|&cell| {
                        anonymous(cell) || document.parent_of(cell) == Some(row.marker)
                    })
            }) {
                continue;
            }
            // Oversized groups keep ordinary fragmentation rather than
            // consuming a continuation page's entire body area.
            let repeatable = |group: Option<usize>| {
                group.filter(|&id| {
                    let height = document.nodes[id].layout.size.height;
                    height.is_finite() && height > 0.0 && height <= page_height / 4.0
                })
            };
            let (header, footer) = (repeatable(header), repeatable(footer));
            if header.is_none() && footer.is_none() {
                continue;
            }
            let groups = [header, footer];
            // The body rows: explicit rows outside the repeated groups, and
            // the boxes of the table's content in anonymous cells outside
            // them, which stand for their anonymous rows.
            let mut body_rows = Vec::new();
            let mut parts = vec![table];
            parts.extend(node.children.iter().copied().filter(|&child| {
                !groups.contains(&Some(child))
                    && !document.nodes[child].is_display_none()
                    && matches!(
                        document.nodes[child].display(),
                        DisplayValue::TableRowGroup
                            | DisplayValue::TableHeaderGroup
                            | DisplayValue::TableFooterGroup
                    )
            }));
            for &part in &parts {
                for &child in &document.nodes[part].children {
                    if groups.contains(&Some(child)) || document.nodes[child].is_display_none() {
                        continue;
                    }
                    if document.nodes[child].display() == DisplayValue::TableRow
                        || (document.anonymous_cell_of(child).is_some()
                            && document.nodes[child].kind() == raikiri_traits::NodeKind::Element)
                    {
                        body_rows.push(child);
                    }
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
            let bottom_caption = document.children_through_contents(table).iter().any(|&id| {
                document.nodes[id].display() == DisplayValue::TableCaption
                    && document.nodes[id].caption_side
                        == raikiri_style::property::CaptionSideValue::Bottom
            });
            let mut sources = Vec::new();
            let mut has_fixed = false;
            let mut has_page_boundary = false;
            for group in groups.into_iter().flatten() {
                let mut descendants = vec![group];
                while let Some(id) = descendants.pop() {
                    let computed = &cascade.computed[id];
                    has_fixed |= matches!(document.nodes[id].position(), PositionValue::Fixed);
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
                    sources.push((id, group));
                    descendants.extend(document.nodes[id].children.iter().copied());
                }
            }
            if spans_rows || bottom_caption || has_fixed || has_page_boundary {
                continue;
            }
            // A body box that does not break, or one inside a cell of a body
            // row, runs over the pages after the one it starts on. So does a
            // box with no content of its own, which pagination cuts at the
            // page edges.
            let unbroken_box = |id: usize| monolithic(id) || document.nodes[id].children.is_empty();
            let mut unbroken = Vec::new();
            for &row in &body_rows {
                if document.nodes[row].display() != DisplayValue::TableRow {
                    if unbroken_box(row) {
                        unbroken.push(row);
                    }
                    continue;
                }
                for &cell in &document.nodes[row].children {
                    let contents: &[usize] =
                        if document.nodes[cell].display() == DisplayValue::TableCell {
                            &document.nodes[cell].children
                        } else {
                            std::slice::from_ref(&cell)
                        };
                    unbroken.extend(contents.iter().copied().filter(|&id| {
                        document.nodes[id].kind() == raikiri_traits::NodeKind::Element
                            && !document.nodes[id].is_display_none()
                            && unbroken_box(id)
                    }));
                }
            }
            // A header copy is placed on a page where a body row starts, or
            // where a box that does not break runs on. Bound those pages by
            // the rows, and by the table's unfragmented height over the body
            // area left beside the copies (at least three quarters of a page
            // beside one group, half a page beside both). Pushing an
            // unfitting row to the next page can waste up to one row per
            // page, hence the doubling; forced row breaks each open at most
            // one further page. Footer copies close the same pages.
            let forced_breaks = body_rows
                .iter()
                .filter(|&&row| {
                    let computed = &cascade.computed[row];
                    matches!(
                        computed.break_before,
                        BreakBetween::Page | BreakBetween::Always
                    ) || matches!(
                        computed.break_after,
                        BreakBetween::Page | BreakBetween::Always
                    )
                })
                .count();
            let table_height = node.layout.size.height;
            let body_area = if header.is_some() && footer.is_some() {
                0.5
            } else {
                0.75
            };
            let flow_pages = Some((table_height / (page_height * body_area)).ceil())
                .filter(|pages| pages.is_finite())
                .map_or(usize::MAX, |pages| pages.max(0.0) as usize);
            let row_pages = if unbroken.is_empty() {
                body_rows.len() + 1
            } else {
                usize::MAX
            };
            let pages = row_pages.min(
                flow_pages
                    .saturating_mul(2)
                    .saturating_add(forced_breaks)
                    .saturating_add(1),
            );
            let copies = sources.len().saturating_mul(pages);
            if copies > copy_budget {
                continue;
            }
            copy_budget -= copies;
            let gap =
                if node.border_collapse == raikiri_style::property::BorderCollapseValue::Collapse {
                    0.0
                } else {
                    node.border_spacing.vertical.0.max(0.0)
                };
            // The last body row is followed by the footer itself, which moves
            // on its own when it does not fit after the row.
            let last_row = body_rows
                .iter()
                .copied()
                .max_by(|&a, &b| absolute_y(a).total_cmp(&absolute_y(b)));
            for &row in &body_rows {
                if let Some(header) = header {
                    result.rows.insert(row, header);
                }
                if let Some(footer) = footer {
                    result.footer_rows.insert(row, footer);
                    if Some(row) == last_row {
                        result.last_rows.insert(row);
                    }
                }
            }
            for (id, group) in sources {
                result.owners[id] = Some(group);
            }
            if let Some(header) = header
                && body_rows
                    .iter()
                    .all(|&row| document.nodes[row].display() != DisplayValue::TableRow)
            {
                result.anonymous_body_tables.insert(table, header);
            }
            result
                .monolithic
                .extend(unbroken.into_iter().map(|id| (id, groups)));
            for (group, is_footer) in [(header, false), (footer, true)] {
                if let Some(group) = group {
                    result.headers.insert(
                        group,
                        HeaderRepeat::new(
                            (table, absolute_y(table)),
                            absolute_y(group),
                            document.nodes[group].layout.size.height,
                            gap,
                            is_footer,
                        ),
                    );
                }
            }
        }
        result
    }

    /// The block size a copy of the repeated header takes above body row
    /// `row` on a continuation page, including its gap to the row, or zero
    /// when the row's table repeats no header.
    pub(crate) fn repeated_height_before(&self, row: usize) -> f32 {
        self.rows
            .get(&row)
            .and_then(|header_id| self.headers.get(header_id))
            .map_or(0.0, |header| header.height + header.gap)
    }

    /// The block size a copy of the repeated footer takes below body row
    /// `row` on a page the table continues past, including its gap to the
    /// row, or zero when the row's table repeats no footer or `row` is its
    /// last body row.
    pub(crate) fn repeated_height_after(&self, row: usize) -> f32 {
        if self.last_rows.contains(&row) {
            return 0.0;
        }
        self.footer_rows
            .get(&row)
            .and_then(|footer_id| self.headers.get(footer_id))
            .map_or(0.0, |footer| footer.height + footer.gap)
    }

    /// Whether `group` is a repeated header group.
    pub(crate) fn repeats_header(&self, group: usize) -> bool {
        self.headers
            .get(&group)
            .is_some_and(|header| !header.footer)
    }

    /// Whether `source` belongs to a repeated footer group.
    pub(crate) fn is_footer_source(&self, source: usize) -> bool {
        self.owner(source)
            .and_then(|group| self.headers.get(&group))
            .is_some_and(|group| group.footer)
    }

    pub(crate) fn owner(&self, source: usize) -> Option<usize> {
        self.owners.get(source).copied().flatten()
    }

    /// Order each header's placements by page origin for [`Self::shift`].
    ///
    /// Call after the placements are final; page origins increase with the
    /// page index, so the order matches the page order.
    pub(crate) fn index(&mut self) {
        for header in self.headers.values_mut() {
            header.by_origin = header.placements.values().copied().collect();
            header
                .by_origin
                .sort_by(|left, right| left.0.total_cmp(&right.0));
        }
    }

    pub(crate) fn shift(&self, source: usize, page_origin: f32) -> Option<Option<f32>> {
        let header = self.headers.get(&self.owner(source)?)?;
        let start = header
            .by_origin
            .partition_point(|&(origin, _)| origin < page_origin - 0.001);
        Some(
            header
                .by_origin
                .get(start)
                .filter(|&&(origin, _)| (origin - page_origin).abs() <= 0.001)
                .map(|&(_, y)| y - header.source_y),
        )
    }

    /// The pages of `source` as a repeated header box: page index, page
    /// origin, and the box's translation on that page.
    pub(crate) fn placements_of(
        &self,
        source: usize,
    ) -> Option<impl Iterator<Item = (u32, f32, f32)>> {
        let header = self.headers.get(&self.owner(source)?)?;
        Some(
            header
                .placements
                .iter()
                .map(move |(&page, &(origin, y))| (page, origin, y - header.source_y)),
        )
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
    let origin = document
        .layout_origin_of(node)
        .expect("valid layout node has an origin");
    (origin.x, origin.y)
}

pub(crate) fn finish_boxes(document: &mut Document, headers: &HeaderRepeats) {
    for header in headers.headers.values() {
        finish_table_boxes(document, header.table, header.gap);
    }
}

/// Grows the rows, row groups and box of `table` over the cells that
/// pagination moved, and refreshes the cell rectangles that clip each part's
/// background. `gap` is the spacing kept below the last row.
pub(crate) fn finish_table_boxes(document: &mut Document, table: usize, gap: f32) {
    let table_origin = absolute_position(document, table);
    let mut parts = Vec::new();
    let mut pending = document.nodes[table].children.clone();
    while let Some(id) = pending.pop() {
        match document.nodes[id].display() {
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
    for &part in parts.iter().chain(std::iter::once(&table)) {
        // A source row's own cells, then the anonymous cells wrapping the
        // rest of its content, or of a group's or the table's content
        // that has no row; anonymous cells are placed in their owner.
        let mut cells = Vec::new();
        if document.nodes[part].display() == DisplayValue::TableRow {
            for &cell in &document.nodes[part].children {
                let node = document.ifc_layout_node(cell).unwrap();
                if node.display() == DisplayValue::TableCell && !node.is_display_none() {
                    let (x, y) = absolute_position(document, cell);
                    let size = node.layout.size;
                    cells.push((x, y, size, node.hides_empty_table_cell));
                }
            }
        }
        let part_origin = absolute_position(document, part);
        for (_, node) in document.anonymous_table_cells(part) {
            let layout = node.layout;
            cells.push((
                part_origin.0 + layout.location.x,
                part_origin.1 + layout.location.y,
                layout.size,
                node.hides_empty_table_cell,
            ));
        }
        if cells.is_empty() {
            // A row without cells still takes its own used height, which the
            // parts around it enclose.
            if document.nodes[part].display() == DisplayValue::TableRow {
                let bottom = part_origin.1 + document.nodes[part].layout.size.height;
                table_bottom = table_bottom.max(bottom);
                let owners: Vec<usize> = document
                    .inclusive_ancestors(part)
                    .skip(1)
                    .take_while(|&id| id != table)
                    .collect();
                for id in owners {
                    let top = absolute_position(document, id).1;
                    let height = &mut document.nodes[id].layout.size.height;
                    *height = height.max(bottom - top);
                }
            }
            continue;
        }
        // The traversal above reaches rows only through explicit
        // table-part parents, so every ancestor before the table owns
        // the row's cell-background intersections. Only their heights
        // change below, so their positions are taken once.
        let owners: Vec<(usize, (f32, f32))> = document
            .inclusive_ancestors(part)
            .take_while(|&id| id != table)
            .map(|id| (id, absolute_position(document, id)))
            .collect();
        for (x, y, size, hidden) in cells {
            table_bottom = table_bottom.max(y + size.height);
            for &(id, origin) in &owners {
                let bottom = y + size.height - origin.1;
                document.nodes[id].layout.size.height =
                    document.nodes[id].layout.size.height.max(bottom);
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
            }
        }
    }
    let layout = &mut document.nodes[table].layout;
    layout.size.height = layout
        .size
        .height
        .max(table_bottom - table_origin.1 + gap + layout.padding.bottom + layout.border.bottom);
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
