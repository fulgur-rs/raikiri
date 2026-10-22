use super::*;

fn publish_paint(tree: &mut LayoutSession<'_>, state: &BorderState) {
    #[cfg(test)]
    let mut applied = 0;
    for cell in &state.cells {
        tree.table_layout_node_mut(cell.node_id).collapsed_border = cell.paint;
        #[cfg(test)]
        {
            applied += 1;
            tests::border_scope_tests::after_paint_entry(tree, cell.node_id, applied);
        }
    }
    for &(row, paint) in &state.rows {
        tree.nodes[row].collapsed_border = paint;
        #[cfg(test)]
        {
            applied += 1;
            tests::border_scope_tests::after_paint_entry(tree, row, applied);
        }
    }
}

type PaintSnapshot = Vec<(usize, Option<Sides<ComputedBorder>>)>;

fn save_paint(tree: &LayoutSession<'_>, state: &BorderState) -> PaintSnapshot {
    state
        .cells
        .iter()
        .map(|cell| cell.node_id)
        .chain(state.rows.iter().map(|&(row, _)| row))
        .map(|idx| {
            (
                idx,
                tree.ifc_layout_node(idx)
                    .expect("paint target is live")
                    .collapsed_border,
            )
        })
        .collect()
}

fn restore_paint(tree: &mut LayoutSession<'_>, saved: PaintSnapshot) {
    for (idx, paint) in saved.into_iter().rev() {
        tree.table_layout_node_mut(idx).collapsed_border = paint;
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
