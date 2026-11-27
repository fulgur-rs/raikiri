//! Page placements of the first header and footer groups in ordinary
//! block-flow tables.

use crate::Document;
use raikiri_style::{
    CascadeResult, ComputedLengthPercentageOrAuto as Length,
    property::{
        BreakInside, DisplayValue, FlexDirectionValue, FlexWrapValue, FloatValue, OverflowValue,
        PositionValue,
    },
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
    /// Closing edges after the source footer, excluding the footer's rows.
    /// Copies on earlier pages do not reserve this space.
    pub(crate) closing_edges: f32,
    /// For a footer, the block size below its end of the boxes that the page
    /// may not break from it, and whether that run moves to the next page
    /// even when it does not fit there.
    pub(crate) tied_run: Option<(f32, bool)>,
    /// The table's block start before pagination.
    pub(crate) table_source_y: f32,
    /// The block offset that `position: relative` on the table and the
    /// boxes around it adds to where the flow puts it. The pages break the
    /// flow without it, and every copy is painted with it, as the rows are.
    pub(crate) relative_offset: f32,
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
            closing_edges: 0.0,
            tied_run: None,
            relative_offset: 0.0,
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
    /// The first bottom caption of each table that repeats a header and no
    /// footer, with that header group and the block size the table's closing
    /// padding and border add below the header copy. When pagination moves
    /// the caption alone to the next page with the edges that close the
    /// table's parent, the page opens with a copy of the header above it.
    pub(crate) tail_captions: HashMap<usize, (usize, f32)>,
    /// The boxes of `monolithic` inside each body part around them: a part
    /// whose box runs on under a copy is drawn on under it (see
    /// [`Self::copy_bands`]).
    unbroken_in: HashMap<usize, Vec<usize>>,
    /// The bands of [`Self::copy_bands`] by table, rebuilt by
    /// [`Self::index`].
    bands: HashMap<usize, Vec<(f32, f32, bool)>>,
    owners: Vec<Option<usize>>,
}

impl HeaderRepeats {
    /// Finds block-flow tables below `body` (see `block_flow_tables`) whose first header
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
        for table in block_flow_tables(document, cascade, body) {
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
            // A group that may break inside does not repeat: its rows are
            // ordinary rows of the table, wherever the table stands.
            let does_not_break = |group: usize| {
                matches!(
                    cascade.computed[group].break_inside,
                    BreakInside::Avoid | BreakInside::AvoidPage
                )
            };
            let header = repeatable(header).filter(|&header| does_not_break(header));
            let footer = repeatable(footer).filter(|&footer| does_not_break(footer));
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
            // The anonymous cells around the table's content outside any cell
            // stand for their anonymous rows, whose lines are placed as those
            // of a cell.
            let mut anonymous_cells: Vec<usize> = Vec::new();
            let mut seen_cells = std::collections::HashSet::new();
            // Those of explicit rows too.
            let row_parts = body_rows
                .iter()
                .copied()
                .filter(|&row| document.nodes[row].display() == DisplayValue::TableRow);
            for part in parts.iter().copied().chain(row_parts) {
                for &child in &document.nodes[part].children {
                    if let Some(cell) = document.anonymous_cell_of(child)
                        && seen_cells.insert(cell)
                    {
                        anonymous_cells.push(cell);
                    }
                }
            }
            if body_rows.is_empty() && anonymous_cells.is_empty() {
                continue;
            }
            // Spanning-row breaks need their own table fragmentation pass.
            // Preserve the existing source geometry.
            let spans_rows = grid_rows
                .iter()
                .flat_map(|row| row.cells.iter().copied())
                .any(|cell| super::get_rowspan(document, cell) != 1);
            let mut sources = Vec::new();
            let mut has_fixed = false;
            let mut has_page_boundary = false;
            for group in groups.into_iter().flatten() {
                let mut descendants = vec![group];
                while let Some(id) = descendants.pop() {
                    let computed = &cascade.computed[id];
                    has_fixed |= matches!(document.nodes[id].position(), PositionValue::Fixed);
                    has_page_boundary |= computed.break_before.may_force_page_break()
                        || computed.break_after.may_force_page_break()
                        || matches!(
                            cascade.page_values.get(id),
                            Some(raikiri_style::property::PageValue::Named(_))
                        );
                    sources.push((id, group));
                    descendants.extend(document.nodes[id].children.iter().copied());
                }
            }
            if spans_rows || has_fixed || has_page_boundary {
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
            // A header copy is placed on each page a body row starts or runs
            // on, and on each page a box that does not break runs over. Rows
            // break between their lines and at forced breaks in their cells,
            // so bound those pages by the table's unfragmented height over the
            // body area left beside the copies (at least three quarters of a
            // page beside one group, half a page beside both). Pushing an
            // unfitting row or line to the next page can waste up to one of
            // them per page, hence the doubling; each forced break in the
            // body opens at most one further page. Footer copies close the
            // same pages.
            let forces_break = |id: usize| {
                cascade.computed.get(id).is_some_and(|computed| {
                    computed.break_before.may_force_page_break()
                        || computed.break_after.may_force_page_break()
                })
            };
            let mut forced_breaks = 0usize;
            let mut body_boxes = body_rows.clone();
            while let Some(id) = body_boxes.pop() {
                forced_breaks += usize::from(forces_break(id));
                body_boxes.extend(document.nodes[id].children.iter().copied());
            }
            let table_height = node.layout.size.height;
            let body_area = if header.is_some() && footer.is_some() {
                0.5
            } else {
                0.75
            };
            let flow_pages = Some((table_height / (page_height * body_area)).ceil())
                .filter(|pages| pages.is_finite())
                .map_or(usize::MAX, |pages| pages.max(0.0) as usize);
            let pages = flow_pages
                .saturating_mul(2)
                .saturating_add(forced_breaks)
                .saturating_add(1);
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
            // on its own when it does not fit after the row. A box in an
            // anonymous cell stands for the anonymous row around the cell, and
            // an anonymous cell in an explicit row belongs to that row.
            let row_of = |id: usize| {
                if document.nodes.is_anonymous_cell(id) {
                    document
                        .parent_of(id)
                        .filter(|&parent| {
                            document.nodes[parent].display() == DisplayValue::TableRow
                        })
                        .unwrap_or(id)
                } else {
                    document.anonymous_cell_of(id).unwrap_or(id)
                }
            };
            let last_row = body_rows
                .iter()
                .chain(&anonymous_cells)
                .copied()
                .max_by(|&a, &b| absolute_y(a).total_cmp(&absolute_y(b)))
                .map(row_of);
            for &row in body_rows.iter().chain(&anonymous_cells) {
                if let Some(header) = header {
                    result.rows.insert(row, header);
                }
                if let Some(footer) = footer {
                    result.footer_rows.insert(row, footer);
                    if Some(row_of(row)) == last_row {
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
            for &id in &unbroken {
                for part in document
                    .inclusive_ancestors(id)
                    .skip(1)
                    .take_while(|&part| part != table)
                {
                    result.unbroken_in.entry(part).or_default().push(id);
                }
            }
            result
                .monolithic
                .extend(unbroken.into_iter().map(|id| (id, groups)));
            if let (Some(header), None) = (header, footer)
                && let Some(caption) = document
                    .children_through_contents(table)
                    .iter()
                    .copied()
                    .find(|&id| {
                        !document.nodes[id].is_display_none()
                            && document.nodes[id].display() == DisplayValue::TableCaption
                            && document.nodes[id].caption_side
                                == raikiri_style::property::CaptionSideValue::Bottom
                            && matches!(document.nodes[id].position(), PositionValue::Static)
                    })
            {
                let layout = &node.layout;
                result.tail_captions.insert(
                    caption,
                    (header, layout.padding.bottom + layout.border.bottom),
                );
            }
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

    /// Places copies of the repeated header and footer groups of the table
    /// of body row `row` around a break inside the row: the footer's at the
    /// end of the page `left`, which starts at `left_origin` and ends at
    /// `left_end`, above the table's spacing there, and the header's at the
    /// top of the page `next`, which starts at `next_origin`. The source of
    /// the footer follows only the last fragment of the table's last body
    /// row.
    pub(crate) fn place_copies_around_row_break(
        &mut self,
        row: usize,
        (left, left_origin, left_end): (u32, f32, f32),
        (next, next_origin): (u32, f32),
    ) {
        if let Some(header) = self.rows.get(&row).and_then(|id| self.headers.get_mut(id)) {
            header
                .placements
                .entry(next)
                .or_insert((next_origin, next_origin + header.relative_offset));
        }
        if let Some(footer) = self
            .footer_rows
            .get(&row)
            .and_then(|id| self.headers.get_mut(id))
        {
            footer.placements.entry(left).or_insert((
                left_origin,
                left_end - footer.gap - footer.height + footer.relative_offset,
            ));
        }
    }

    /// Moves the end of body row `row`, the last body row placed so far, down
    /// by `growth`, how far the row grew when lines inside it moved: the
    /// footer's copy on the page the row ends on goes below it. `page_of`
    /// gives the page of a position in the flow, without the table's
    /// relative offset, which `last_row` keeps.
    pub(crate) fn grow_last_row(&mut self, row: usize, growth: f32, page_of: impl Fn(f32) -> u32) {
        if let Some(footer) = self
            .footer_rows
            .get(&row)
            .and_then(|id| self.headers.get_mut(id))
            && let Some((_, bottom)) = footer.last_row
        {
            let bottom = bottom + growth;
            let page = page_of(bottom - footer.relative_offset - 0.001);
            footer.last_row = Some((page, bottom));
        }
    }

    /// Moves the end of the body rows placed so far down to `bottom`, where
    /// body row `row` now ends, when that is lower. `page_of` gives the page
    /// of a position in the flow, without the table's relative offset.
    pub(crate) fn extend_last_row(
        &mut self,
        row: usize,
        bottom: f32,
        page_of: impl Fn(f32) -> u32,
    ) {
        if let Some(footer) = self
            .footer_rows
            .get(&row)
            .and_then(|id| self.headers.get_mut(id))
            && footer.last_row.is_none_or(|(_, end)| end < bottom)
        {
            let page = page_of(bottom - footer.relative_offset - 0.001);
            footer.last_row = Some((page, bottom));
        }
    }

    /// The block size of the copy of the repeated footer below body row
    /// `row`, including its gap to the row, on each page the row leaves when
    /// it breaks, the last body row's included; zero when the row's table
    /// repeats no footer.
    pub(crate) fn footer_copy_height(&self, row: usize) -> f32 {
        self.footer_rows
            .get(&row)
            .and_then(|footer_id| self.headers.get(footer_id))
            .map_or(0.0, |footer| footer.height + footer.gap)
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
        self.bands.clear();
        for header in self.headers.values_mut() {
            header.by_origin = header.placements.values().copied().collect();
            header
                .by_origin
                .sort_by(|left, right| left.0.total_cmp(&right.0));
            // A header copy keeps the spacing below it.
            let gap = if header.footer { 0.0 } else { header.gap };
            self.bands.entry(header.table).or_default().extend(
                header
                    .placements
                    .values()
                    .map(|&(_, y)| (y, y + header.height + gap, header.footer)),
            );
        }
        for bands in self.bands.values_mut() {
            bands.sort_by(|left, right| left.0.total_cmp(&right.0));
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

    /// The block ranges in the flow that the copies of the repeated header
    /// and footer groups of the table around `part`, a body row group, row
    /// or cell of it, take on the pages the table crosses, sorted, a header
    /// copy with the spacing below it, and whether each is a footer's.
    /// Chromium starts the fragments of the body parts that a page break
    /// splits below the header copies and ends them at the footer copies.
    /// Empty for any other box, and for a part around a box that does not
    /// break and runs on under a copy, which Chromium draws on under it.
    pub(crate) fn copy_bands(&self, document: &Document, part: usize) -> &[(f32, f32, bool)] {
        if self.bands.is_empty()
            || self.owner(part).is_some()
            || !matches!(
                document.nodes[part].display(),
                DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
                    | DisplayValue::TableRow
                    | DisplayValue::TableCell
            )
        {
            return &[];
        }
        let Some(bands) = document
            .table_of_row(part)
            .and_then(|table| self.bands.get(&table))
        else {
            return &[];
        };
        // A box on the lines of a cell moves with its line, below the
        // copies.
        // An inline box that is not replaced, such as a line break, breaks
        // with the lines it sits on, which stay clear of the copies.
        let under_copy = |id: usize| {
            if document.nodes[id].display() == DisplayValue::Inline
                && !crate::paint_rules::is_replaced_element(document, id)
            {
                return false;
            }
            let top = absolute_position(document, id).1;
            let bottom = top + document.nodes[id].layout.size.height;
            // The bands do not overlap, so their ends are sorted too.
            let first = bands.partition_point(|&(_, end, _)| end <= top);
            bands.get(first).is_some_and(|&(start, ..)| start < bottom)
        };
        if self
            .unbroken_in
            .get(&part)
            .is_some_and(|boxes| boxes.iter().any(|&id| under_copy(id)))
        {
            return &[];
        }
        bands
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
    // The parts of an anonymous table are children of its parent, which is
    // not one of the boxes they grow.
    let boundary = document.table_parts_boundary(table);
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
                    .take_while(|&id| id != boundary)
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
            .take_while(|&id| id != boundary)
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
    let rows_height =
        table_bottom - table_origin.1 + gap + layout.padding.bottom + layout.border.bottom;
    layout.size.height = layout.size.height.max(rows_height);
    // The grid ends with its rows: the height of the table around it also
    // holds the captions below the grid, which the grid does not cover.
    if let Some(grid) = &mut document.nodes[table].table_grid_box {
        grid.height = (rows_height - grid.y).max(grid.height);
    }
    for (id, cells) in cells_by_part {
        if let Some(clips) = document.table_objects.part_background_cells.get_mut(&id) {
            *clips = cells;
        }
    }
}

/// Table sources whose ancestor boxes follow the shared block flow when a
/// repeated header adds room, a single-line row flex container that holds
/// only the table (see [`sole_flex_table`]) counting as such a box, as does
/// a single-line column flex container (see [`stacking_column_flex`]) for
/// its table items. Bounded
/// or independently placed flows stop the search so their internal shifts
/// cannot escape into their outer siblings.
fn block_flow_tables(document: &Document, cascade: &CascadeResult, body: usize) -> Vec<usize> {
    let mut tables = Vec::new();
    let mut pending: Vec<_> = document.nodes[body]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        if node.is_display_none() {
            continue;
        }
        if node.display() == DisplayValue::Table {
            // A nested table repeats its groups on every page it spans, as
            // one directly in the body does, once layout has given it a
            // height.
            if document.parent_of(id) == Some(body)
                || (node.layout.size.height.is_finite()
                    && shares_growing_block_flow(document, cascade, id))
            {
                tables.push(id);
            }
            continue;
        }
        if node.display() == DisplayValue::Contents {
            pending.extend(node.children.iter().rev().copied());
            continue;
        }
        if let Some(table) = sole_flex_table(document, cascade, id) {
            pending.push(table);
            continue;
        }
        if stacking_column_flex(document, cascade, id) {
            // Only its table items stack as block flow does: any other item
            // fragments as a flex item, with the tables inside it.
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .copied()
                    .filter(|&child| document.nodes[child].display() == DisplayValue::Table),
            );
            continue;
        }
        if !matches!(node.display(), DisplayValue::Block | DisplayValue::FlowRoot)
            || !shares_growing_block_flow(document, cascade, id)
        {
            continue;
        }
        pending.extend(node.children.iter().rev().copied());
    }
    tables
}

/// The table that is the only item of `id` when `id` is a single-line row
/// flex container that grows with it like a block box: the table then
/// fragments as it would in block flow, so its header and footer repeat.
pub(crate) fn sole_flex_table(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> Option<usize> {
    let node = &document.nodes[id];
    let values = &cascade.computed[id];
    if node.display() != DisplayValue::Flex
        || values.flex_direction != FlexDirectionValue::Row
        || values.flex_wrap != FlexWrapValue::NoWrap
        || !shares_growing_block_flow(document, cascade, id)
    {
        return None;
    }
    crate::layout::ifc::assign::only_item(document, id)
        .filter(|&item| document.nodes[item].display() == DisplayValue::Table)
}

/// Whether `id` is a single-line column flex container that grows with its
/// items like a block box, so that they stack as block flow does.
pub(crate) fn stacking_column_flex(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
) -> bool {
    let values = &cascade.computed[id];
    document.nodes[id].display() == DisplayValue::Flex
        && values.flex_direction == FlexDirectionValue::Column
        && values.flex_wrap == FlexWrapValue::NoWrap
        && shares_growing_block_flow(document, cascade, id)
}

fn shares_growing_block_flow(document: &Document, cascade: &CascadeResult, id: usize) -> bool {
    let node = &document.nodes[id];
    let values = &cascade.computed[id];
    !node.is_ifc_root()
        && node.multicol.is_none()
        && !node.size_containment.height
        && !node.used_writing_mode.is_vertical()
        && !values.cssom_writing_mode.is_vertical()
        && matches!(
            values.position,
            PositionValue::Static | PositionValue::Relative
        )
        && values.float == FloatValue::None
        && values.overflow.y == OverflowValue::Visible
        && values.height == Length::Auto
        && values.height_keyword.is_none()
        && matches!(values.min_height, Length::Auto | Length::Px(0.0))
        && values.max_height == Length::Auto
        && values.aspect_ratio.preferred(None).is_none()
}

#[cfg(test)]
mod tests;
