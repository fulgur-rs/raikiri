use super::*;
use crate::node::arena::BoxKey;
use raikiri_layout_geometry::Edges;
use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use std::collections::HashMap;
use std::rc::Rc;

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
}

#[derive(Clone, Debug)]
pub(super) struct BorderState {
    pub(super) cells: Vec<CellBorderState>,
    pub(super) rows: Vec<(usize, Option<Sides<ComputedBorder>>)>,
    slots: HashMap<usize, usize>,
    row_slots: HashMap<usize, usize>,
    row_widths: HashMap<usize, Edges<LengthPercentage>>,
    block_metrics: Option<Rc<BlockMetrics>>,
}

impl BorderState {
    pub(super) fn authored(doc: &LayoutSession<'_>, grid: &TableGrid) -> Self {
        let cells: Vec<_> = grid
            .cells
            .iter()
            .map(|cell| Self::authored_cell(doc, cell.node_id))
            .collect();
        let slots = cells
            .iter()
            .enumerate()
            .map(|(slot, cell)| (cell.node_id, slot))
            .collect();
        // Anonymous rows alias their first cell; that cell owns its paint.
        let rows: Vec<_> = grid
            .rows
            .iter()
            .copied()
            .filter(|&row| {
                doc.ifc_layout_node(row)
                    .is_some_and(|node| node.display() == DisplayValue::TableRow)
            })
            .map(|row| (row, None))
            .collect();
        let row_slots = rows
            .iter()
            .enumerate()
            .map(|(slot, &(row, _))| (row, slot))
            .collect();
        let row_widths = rows
            .iter()
            .map(|&(row, _)| (row, doc.table_style(row).border()))
            .collect();
        Self {
            cells,
            slots,
            rows,
            row_slots,
            row_widths,
            block_metrics: None,
        }
    }

    fn authored_cell(doc: &LayoutSession<'_>, node_id: usize) -> CellBorderState {
        let node = doc
            .ifc_layout_node(node_id)
            .expect("table cell layout view");
        let widths = node.computed_border.as_ref().map_or_else(
            || doc.table_style(node_id).border(),
            |border| super::super::bridge::taffy_border_widths(border, &mut Vec::new()),
        );
        CellBorderState {
            node_id,
            widths,
            paint: None,
            authored: node.computed_border,
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
        let (mut paint, widths) = if let Some(cell) = self.cell(node) {
            (cell.paint.or(cell.authored)?, cell.widths)
        } else if let Some(&slot) = self.row_slots.get(&node) {
            (
                self.rows[slot]
                    .1
                    .or(doc.ifc_layout_node(node)?.computed_border)?,
                *self.row_widths.get(&node)?,
            )
        } else {
            return border_candidate(doc, node, side);
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
        })
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
            self.cells[slot] = Self::authored_cell(doc, cell.node_id);
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
        for row in &grid.rows {
            if let Some(&slot) = self.row_slots.get(row) {
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
            self.rows[slot].1 = Some(Sides {
                top: computed.top.without_line(),
                right: computed.right.without_line(),
                bottom: computed.bottom.without_line(),
                left: computed.left.without_line(),
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
        let mut state = Self::authored(doc, grid);
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
