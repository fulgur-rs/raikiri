use super::*;
use crate::node::NodeFlags;
use crate::paint_rules::CollapsedCorners;

fn publish_paint(tree: &mut LayoutSession<'_>, state: &BorderState) {
    #[cfg(test)]
    let mut applied = 0;
    for cell in &state.cells {
        tree.table_layout_node_mut(cell.node_id).collapsed_border =
            state.physical_paint(cell.paint);
        // The junction heights only hold where the cell's top and bottom
        // borders are centered on their grid lines.
        let centered = state.block_outer().is_some();
        tree.table_layout_node_mut(cell.node_id).collapsed_corners = state
            .physical_corners(cell.corners)
            .map(|corners| CollapsedCorners {
                junction_heights: Some(corners.junction_heights)
                    .filter(|_| centered)
                    .unwrap_or_default(),
                ..corners
            });
        tree.table_layout_node_mut(cell.node_id).flags.set(
            NodeFlags::CENTERED_COLLAPSED_BLOCK_BORDERS,
            state.block_outer().is_some(),
        );
        #[cfg(test)]
        {
            applied += 1;
            tests::border_scope_tests::after_paint_entry(tree, cell.node_id, applied);
        }
    }
    for &(row, paint) in &state.rows {
        tree.nodes[row].collapsed_border = state.physical_paint(paint);
        #[cfg(test)]
        {
            applied += 1;
            tests::border_scope_tests::after_paint_entry(tree, row, applied);
        }
    }
}

type PaintSnapshot = Vec<(
    usize,
    Option<Sides<ComputedBorder>>,
    Option<CollapsedCorners>,
    bool,
)>;

fn save_paint(tree: &LayoutSession<'_>, state: &BorderState) -> PaintSnapshot {
    state
        .cells
        .iter()
        .map(|cell| cell.node_id)
        .chain(state.rows.iter().map(|&(row, _)| row))
        .map(|idx| {
            let node = tree.ifc_layout_node(idx).expect("paint target is live");
            (
                idx,
                node.collapsed_border,
                node.collapsed_corners,
                node.centered_collapsed_block_borders(),
            )
        })
        .collect()
}

fn restore_paint(tree: &mut LayoutSession<'_>, saved: PaintSnapshot) {
    for (idx, paint, corners, centered) in saved.into_iter().rev() {
        tree.table_layout_node_mut(idx).collapsed_border = paint;
        tree.table_layout_node_mut(idx).collapsed_corners = corners;
        tree.table_layout_node_mut(idx)
            .flags
            .set(NodeFlags::CENTERED_COLLAPSED_BLOCK_BORDERS, centered);
    }
}

pub(super) fn with_table_borders<R, E>(
    tree: &mut LayoutSession<'_>,
    state: &BorderState,
    f: impl FnOnce(&mut LayoutSession<'_>) -> Result<R, E>,
) -> Result<R, E> {
    let saved = save_paint(tree, state);
    let entries = state.entries(tree);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_overrides(entries, |tree| {
            publish_paint(tree, state);
            f(tree)
        })
    }));
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => {
            restore_paint(tree, saved);
            Err(error)
        }
        Err(payload) => {
            restore_paint(tree, saved);
            std::panic::resume_unwind(payload)
        }
    }
}

pub(super) fn with_temporary_borders<R>(
    tree: &mut LayoutSession<'_>,
    state: &BorderState,
    f: impl FnOnce(&mut LayoutSession<'_>) -> R,
) -> R {
    let saved = save_paint(tree, state);
    let entries = state.entries(tree);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_overrides(entries, |tree| {
            publish_paint(tree, state);
            f(tree)
        })
    }));
    restore_paint(tree, saved);
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
