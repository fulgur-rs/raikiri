use super::*;
use crate::node::arena::BoxKey;
use raikiri_layout_style::{LengthProperty as P, OwnedPropertyOverride as O};
use raikiri_style::ComputedLengthPercentageOrAuto as L;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CellBorderState {
    pub(super) node_id: usize,
    pub(super) widths: Rect<LengthPercentage>,
    pub(super) paint: Option<Sides<ComputedBorder>>,
    authored: Option<Sides<ComputedBorder>>,
}

#[derive(Clone, Debug)]
pub(super) struct BorderState {
    pub(super) cells: Vec<CellBorderState>,
    pub(super) rows: Vec<(usize, Option<Sides<ComputedBorder>>)>,
    slots: HashMap<usize, usize>,
    row_slots: HashMap<usize, usize>,
    row_widths: HashMap<usize, Rect<LengthPercentage>>,
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
            // A box without computed borders never supplies a candidate and
            // receives no changed width. Do not turn its retained calc into pixels.
            if cell.authored.is_none() {
                continue;
            }
            let key = doc
                .nodes
                .box_key(cell.node_id)
                .expect("border cell has a box key");
            for (property, width) in [
                (P::BorderTop, cell.widths.top),
                (P::BorderRight, cell.widths.right),
                (P::BorderBottom, cell.widths.bottom),
                (P::BorderLeft, cell.widths.left),
            ] {
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
