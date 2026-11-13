use super::*;
use crate::node::arena::BoxKey;
use crate::paint_rules::CollapsedCorners;
use raikiri_layout_geometry::Edges;
use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use std::collections::HashMap;
use std::rc::Rc;

/// The bits of an origin that hold a box's order in the table; the bits
/// above them hold its kind. Where collapsed borders of equal width and
/// style meet, the one from the more specific kind of box covers the
/// junction: cell, row, row group, column, column group, then table, as in
/// the conflict resolution of CSS 2.1 §17.6.2.1.
pub(super) const ORIGIN_ORDER_MASK: u32 = (1 << 28) - 1;
const ROW_ORIGIN: u32 = 1 << 28;
const ROW_GROUP_ORIGIN: u32 = 2 << 28;
pub(super) const COLUMN_ORIGIN: u32 = 3 << 28;
pub(super) const COLUMN_GROUP_ORIGIN: u32 = 4 << 28;
pub(super) const TABLE_ORIGIN: u32 = 5 << 28;

#[derive(Debug)]
pub(super) struct BlockMetrics {
    widths: Vec<Line<LengthPercentage>>,
    seeds: Vec<(Line<LengthPercentage>, Option<Line<ComputedBorder>>)>,
    outer: Line<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CellBorderState {
    pub(super) node_id: usize,
    pub(super) widths: Edges<LengthPercentage>,
    pub(super) paint: Option<Sides<ComputedBorder>>,
    authored: Option<Sides<ComputedBorder>>,
    /// The table order of the box each side's border comes from, which
    /// decides between equal lines where they meet.
    pub(super) origins: Sides<u32>,
    /// Which side paints each corner, once the junctions are resolved.
    pub(super) corners: Option<CollapsedCorners>,
}

#[derive(Clone, Debug)]
pub(super) struct BorderState {
    pub(super) cells: Vec<CellBorderState>,
    pub(super) rows: Vec<(usize, Option<Sides<ComputedBorder>>)>,
    slots: HashMap<usize, usize>,
    row_slots: HashMap<usize, usize>,
    row_widths: HashMap<usize, Edges<LengthPercentage>>,
    /// The table order of each row and row group, as [`Self::origin_of`]
    /// gives it for a cell.
    part_origins: HashMap<usize, u32>,
    block_metrics: Option<Rc<BlockMetrics>>,
    /// The state keeps the left and right sides swapped, so that the first
    /// column of a table whose columns run from right to left is resolved on
    /// its left, as the first column of any table is. [`Self::physical_paint`]
    /// and [`Self::entries`] turn the sides around again for the boxes.
    flipped: bool,
}

fn mirrored_edges(edges: Edges<LengthPercentage>) -> Edges<LengthPercentage> {
    Edges {
        left: edges.right,
        right: edges.left,
        ..edges
    }
}

fn mirrored_sides(sides: Sides<ComputedBorder>) -> Sides<ComputedBorder> {
    Sides {
        left: sides.right,
        right: sides.left,
        ..sides
    }
}

impl BorderState {
    pub(super) fn authored(doc: &LayoutSession<'_>, grid: &TableGrid, flipped: bool) -> Self {
        let cells: Vec<_> = grid
            .cells
            .iter()
            .map(|cell| {
                let mut state = Self::authored_cell(doc, cell.node_id, flipped);
                state.origins = Sides::all(Self::origin_of(grid, cell.row, Some(cell.col_start)));
                state
            })
            .collect();
        let slots = cells
            .iter()
            .enumerate()
            .map(|(slot, cell)| (cell.node_id, slot))
            .collect();
        // Anonymous rows alias their first cell; that cell owns its paint.
        // The row groups around the rows follow them: their borders, too,
        // resolve into the cells and are not painted by the group.
        let mut rows: Vec<_> = grid
            .rows
            .iter()
            .copied()
            .filter(|&row| {
                doc.ifc_layout_node(row)
                    .is_some_and(|node| node.display() == DisplayValue::TableRow)
            })
            .map(|row| (row, None))
            .collect();
        let mut part_origins = HashMap::new();
        for (index, &row) in grid.rows.iter().enumerate() {
            let origin = Self::origin_of(grid, u16::try_from(index).unwrap_or(u16::MAX), None);
            part_origins.entry(row).or_insert(origin | ROW_ORIGIN);
            if let Some(group) = row_group_of(doc, row)
                && !part_origins.contains_key(&group)
            {
                part_origins.insert(group, origin | ROW_GROUP_ORIGIN);
                rows.push((group, None));
            }
        }
        let row_slots = rows
            .iter()
            .enumerate()
            .map(|(slot, &(row, _))| (row, slot))
            .collect();
        let row_widths = rows
            .iter()
            .map(|&(row, _)| {
                let widths = doc.table_style(row).border();
                (
                    row,
                    if flipped {
                        mirrored_edges(widths)
                    } else {
                        widths
                    },
                )
            })
            .collect();
        Self {
            cells,
            slots,
            rows,
            row_slots,
            row_widths,
            part_origins,
            block_metrics: None,
            flipped,
        }
    }

    fn authored_cell(doc: &LayoutSession<'_>, node_id: usize, flipped: bool) -> CellBorderState {
        let node = doc
            .ifc_layout_node(node_id)
            .expect("table cell layout view");
        let widths = node.computed_border.as_ref().map_or_else(
            || doc.table_style(node_id).border(),
            |border| super::super::bridge::taffy_border_widths(border, &mut Vec::new()),
        );
        let (widths, authored) = if flipped {
            (
                mirrored_edges(widths),
                node.computed_border.map(mirrored_sides),
            )
        } else {
            (widths, node.computed_border)
        };
        CellBorderState {
            node_id,
            widths,
            paint: None,
            authored,
            origins: Sides::all(0),
            corners: None,
        }
    }

    /// The order in the table of a box that starts at `row`, and at `col`
    /// for a cell: a row comes just before its first cell. The kind of box
    /// is added on top by the caller, and outranks this order.
    fn origin_of(grid: &TableGrid, row: u16, col: Option<u16>) -> u32 {
        let stride = u32::from(grid.n_cols) + 1;
        (u32::from(row) + 1)
            .saturating_mul(stride)
            .saturating_add(col.map_or(0, |col| u32::from(col) + 1))
            .min(ORIGIN_ORDER_MASK)
    }

    /// Whether the state keeps the left and right sides swapped.
    pub(super) fn flipped(&self) -> bool {
        self.flipped
    }

    /// The sides `paint` of this state as the box paints them.
    pub(super) fn physical_paint(
        &self,
        paint: Option<Sides<ComputedBorder>>,
    ) -> Option<Sides<ComputedBorder>> {
        if self.flipped {
            paint.map(mirrored_sides)
        } else {
            paint
        }
    }

    /// The corners `corners` of this state as the box paints them.
    pub(super) fn physical_corners(
        &self,
        corners: Option<CollapsedCorners>,
    ) -> Option<CollapsedCorners> {
        if self.flipped {
            corners.map(CollapsedCorners::mirrored)
        } else {
            corners
        }
    }

    pub(super) fn cell(&self, node: usize) -> Option<&CellBorderState> {
        self.slots.get(&node).map(|&slot| &self.cells[slot])
    }

    // Snapshot the full winners before table-edge paint suppression. The
    // layout projection halves them; later paint resolution cannot change them.
    pub(super) fn block_metrics_before_table_edges(
        &self,
        doc: &LayoutSession<'_>,
        grid: &TableGrid,
        table: usize,
        basis: Option<f32>,
    ) -> BlockMetrics {
        let mut outer = Line {
            start: 0.0f32,
            end: 0.0f32,
        };
        let widths = grid
            .cells
            .iter()
            .map(|cell| {
                let borders = self
                    .cell(cell.node_id)
                    .expect("block metric cell exists")
                    .widths;
                let edge = |side, on_edge: bool, own_width| {
                    if !on_edge {
                        return own_width;
                    }
                    self.candidate(doc, cell.node_id, side)
                        .into_iter()
                        .chain(self.candidate(doc, table, side))
                        .reduce(collapsed_border_winner)
                        .map_or(own_width, |winner| winner.taffy_width)
                };
                let top = cell.row == 0;
                let bottom = usize::from(cell.row) + usize::from(cell.row_span) >= grid.rows.len();
                let widths = Line {
                    start: edge(CellBorderSide::Top, top, borders.top),
                    end: edge(CellBorderSide::Bottom, bottom, borders.bottom),
                };
                if top {
                    outer.start = outer.start.max(resolve_length(widths.start, basis) / 2.0);
                }
                if bottom {
                    outer.end = outer.end.max(resolve_length(widths.end, basis) / 2.0);
                }
                widths
            })
            .collect();
        BlockMetrics {
            widths,
            seeds: Vec::new(),
            outer,
        }
    }

    pub(super) fn install_block_metrics(&mut self, mut metrics: BlockMetrics) {
        metrics.seeds = self
            .cells
            .iter()
            .map(|cell| {
                (
                    Line {
                        start: cell.widths.top,
                        end: cell.widths.bottom,
                    },
                    cell.paint.or(cell.authored).map(|paint| Line {
                        start: paint.top,
                        end: paint.bottom,
                    }),
                )
            })
            .collect();
        self.block_metrics = Some(Rc::new(metrics));
    }

    pub(super) fn block_outer(&self) -> Option<Line<f32>> {
        self.block_metrics.as_ref().map(|metrics| metrics.outer)
    }

    pub(super) fn used_widths(&self, node: usize) -> Edges<LengthPercentage> {
        let slot = self.slots[&node];
        let mut widths = self.cells[slot].widths;
        if let Some(metrics) = &self.block_metrics {
            widths.top =
                LengthPercentage::length(resolve_length(metrics.widths[slot].start, None) / 2.0);
            widths.bottom =
                LengthPercentage::length(resolve_length(metrics.widths[slot].end, None) / 2.0);
        }
        widths
    }

    pub(super) fn candidate(
        &self,
        doc: &LayoutSession<'_>,
        node: usize,
        side: CellBorderSide,
    ) -> Option<CollapsedBorderCandidate> {
        self.candidate_of(doc, node, side, true)
    }

    /// The authored border of a row, row group or other table part, even
    /// once its paint has been suppressed: a later resolution of the grid
    /// without its collapsed rows needs it again for the cells that then lie
    /// on the part's edge.
    pub(super) fn part_candidate(
        &self,
        doc: &LayoutSession<'_>,
        node: usize,
        side: CellBorderSide,
    ) -> Option<CollapsedBorderCandidate> {
        self.candidate_of(doc, node, side, false)
    }

    fn candidate_of(
        &self,
        doc: &LayoutSession<'_>,
        node: usize,
        side: CellBorderSide,
        with_paint: bool,
    ) -> Option<CollapsedBorderCandidate> {
        let (mut paint, widths, origin) = if let Some(cell) = self.cell(node) {
            (
                cell.paint.or(cell.authored)?,
                cell.widths,
                *side_of(&cell.origins, side),
            )
        } else if let Some(&slot) = self.row_slots.get(&node) {
            (
                self.rows[slot].1.filter(|_| with_paint).or_else(|| {
                    let authored = doc.ifc_layout_node(node)?.computed_border;
                    if self.flipped {
                        authored.map(mirrored_sides)
                    } else {
                        authored
                    }
                })?,
                *self.row_widths.get(&node)?,
                self.part_origins.get(&node).copied().unwrap_or(ROW_ORIGIN),
            )
        } else {
            let physical = if self.flipped { side.flipped() } else { side };
            return border_candidate(doc, node, physical);
        };
        let width = match side {
            CellBorderSide::Top => widths.top,
            CellBorderSide::Right => widths.right,
            CellBorderSide::Bottom => widths.bottom,
            CellBorderSide::Left => widths.left,
        };
        Some(CollapsedBorderCandidate {
            border: *border_side(&mut paint, side),
            taffy_width: width,
            origin,
        })
    }

    /// Give `node`'s `side` the border `winner`, and the box it comes from.
    pub(super) fn take_side(
        &mut self,
        node: usize,
        side: CellBorderSide,
        winner: CollapsedBorderCandidate,
    ) {
        self.set_side(node, side, winner.border, winner.taffy_width);
        let cell = &mut self.cells[self.slots[&node]];
        *side_of_mut(&mut cell.origins, side) = winner.origin;
    }

    /// The border `node`'s `side` paints, with the box it comes from, for a
    /// cell of this state.
    pub(super) fn painted_side(
        &self,
        node: usize,
        side: CellBorderSide,
    ) -> Option<(ComputedBorder, u32)> {
        let cell = self.cell(node)?;
        let paint = cell.paint.or(cell.authored)?;
        Some((*side_of(&paint, side), *side_of(&cell.origins, side)))
    }

    /// Record which side paints each corner of `node`.
    pub(super) fn set_corners(&mut self, node: usize, corners: CollapsedCorners) {
        let slot = self.slots[&node];
        self.cells[slot].corners = Some(corners);
    }

    pub(super) fn set_side(
        &mut self,
        node: usize,
        side: CellBorderSide,
        border: ComputedBorder,
        width: LengthPercentage,
    ) {
        let cell = &mut self.cells[*self
            .slots
            .get(&node)
            .expect("resolved cell belongs to border state")];
        match side {
            CellBorderSide::Top => cell.widths.top = width,
            CellBorderSide::Right => cell.widths.right = width,
            CellBorderSide::Bottom => cell.widths.bottom = width,
            CellBorderSide::Left => cell.widths.left = width,
        }
        if let Some(authored) = cell.authored {
            *border_side(cell.paint.get_or_insert(authored), side) = border;
        }
    }

    pub(super) fn reset_authored(&mut self, doc: &LayoutSession<'_>, grid: &TableGrid) {
        for cell in &grid.cells {
            let slot = *self
                .slots
                .get(&cell.node_id)
                .expect("reset cell belongs to border state");
            self.cells[slot] = Self::authored_cell(doc, cell.node_id, self.flipped);
            if let Some(metrics) = &self.block_metrics {
                let (widths, paint) = metrics.seeds[slot];
                self.cells[slot].widths.top = widths.start;
                self.cells[slot].widths.bottom = widths.end;
                if let (Some(seed), Some(authored)) = (paint, self.cells[slot].authored) {
                    let paint = self.cells[slot].paint.get_or_insert(authored);
                    paint.top = seed.start;
                    paint.bottom = seed.end;
                }
            }
        }
        for row in grid
            .rows
            .iter()
            .flat_map(|&row| [Some(row), row_group_of(doc, row)])
            .flatten()
        {
            if let Some(&slot) = self.row_slots.get(&row) {
                self.rows[slot].1 = None;
            }
        }
    }

    pub(super) fn suppress_row(&mut self, doc: &Document, row: usize) {
        if let Some(computed) = doc
            .ifc_layout_node(row)
            .and_then(|node| node.computed_border)
        {
            let slot = *self
                .row_slots
                .get(&row)
                .expect("resolved row belongs to border state");
            let suppressed = Sides {
                top: computed.top.without_line(),
                right: computed.right.without_line(),
                bottom: computed.bottom.without_line(),
                left: computed.left.without_line(),
            };
            self.rows[slot].1 = Some(if self.flipped {
                mirrored_sides(suppressed)
            } else {
                suppressed
            });
        }
    }

    pub(super) fn entries(&self, doc: &Document) -> Vec<(BoxKey, O)> {
        let mut entries = Vec::with_capacity(self.cells.len() * 4);
        for cell in &self.cells {
            // Keep retained inline widths when there is no computed border.
            // Only the centered block metrics can supply used edges then.
            if cell.authored.is_none() && self.block_metrics.is_none() {
                continue;
            }
            let widths = self.used_widths(cell.node_id);
            let widths = if self.flipped {
                mirrored_edges(widths)
            } else {
                widths
            };
            let key = doc
                .nodes
                .box_key(cell.node_id)
                .expect("border cell has a box key");
            for (property, width) in [
                (P::BorderTop, widths.top),
                (P::BorderRight, widths.right),
                (P::BorderBottom, widths.bottom),
                (P::BorderLeft, widths.left),
            ] {
                if cell.authored.is_none() && matches!(property, P::BorderLeft | P::BorderRight) {
                    continue;
                }
                let px = super::super::resolve_length_percentage(width, None)
                    .expect("computed border widths are physical lengths");
                entries.push((
                    key,
                    O::length(property, L::Px(px), true).expect("physical collapsed border"),
                ));
            }
        }
        entries
    }

    #[cfg(test)]
    pub(super) fn retained(doc: &LayoutSession<'_>, grid: &TableGrid) -> Self {
        let mut state = Self::authored(doc, grid, false);
        for cell in &mut state.cells {
            cell.widths = doc.table_style(cell.node_id).border();
            cell.paint = doc.ifc_layout_node(cell.node_id).unwrap().collapsed_border;
        }
        for (row, paint) in &mut state.rows {
            *paint = doc.nodes[*row].collapsed_border;
        }
        state
    }
}

/// The value of `sides` on `side`.
fn side_of<T>(sides: &Sides<T>, side: CellBorderSide) -> &T {
    match side {
        CellBorderSide::Top => &sides.top,
        CellBorderSide::Right => &sides.right,
        CellBorderSide::Bottom => &sides.bottom,
        CellBorderSide::Left => &sides.left,
    }
}

/// The value of `sides` on `side`, to change.
fn side_of_mut<T>(sides: &mut Sides<T>, side: CellBorderSide) -> &mut T {
    match side {
        CellBorderSide::Top => &mut sides.top,
        CellBorderSide::Right => &mut sides.right,
        CellBorderSide::Bottom => &mut sides.bottom,
        CellBorderSide::Left => &mut sides.left,
    }
}
