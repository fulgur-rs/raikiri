use super::super::{CellIndex, TableGrid, border_state::BorderState};
use super::*;

#[test]
fn border_state_resolves_without_document_publication() {
    let mut doc = Document::new();
    let left = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 9.0, CssColor::BLACK),
    );
    let right = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
    );
    let grid = TableGrid {
        n_cols: 2,
        rows: vec![0],
        cells: vec![
            make_cell(left, 0, 0, 1, 1, Dimension::auto()),
            make_cell(right, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: Vec::new(),
    };
    let mut state = BorderState::authored(&doc, &grid);
    let before_paint = doc.nodes[right].collapsed_border;
    super::super::resolve_collapsed_borders(&doc, &mut state, &grid, &CellIndex::new(&grid), 0);
    assert_eq!(
        state.cell(right).unwrap().widths.left,
        LengthPercentage::length(9.0)
    );
    assert_eq!(
        doc.layout_style_for_key(right).unwrap().border.left,
        LengthPercentage::length(2.0)
    );
    assert_eq!(doc.nodes[right].collapsed_border, before_paint);
}

#[test]
fn border_state_authored_ignores_previous_success_and_outer_width() {
    let mut doc = Document::new();
    let cell = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
    );
    doc.layout_style_for_key_mut(cell).unwrap().border.left = LengthPercentage::length(17.0);
    doc.nodes[cell].collapsed_border = Some(Sides::all(
        collapsed_candidate(BorderStyle::Double, 17.0, CssColor::BLACK).border,
    ));
    let grid = TableGrid {
        n_cols: 1,
        rows: vec![0],
        cells: vec![make_cell(cell, 0, 0, 1, 1, Dimension::auto())],
        col_widths: Vec::new(),
    };
    let state = BorderState::authored(&doc, &grid);
    assert_eq!(
        state.cell(cell).unwrap().widths.left,
        LengthPercentage::length(2.0)
    );
    assert_eq!(state.cell(cell).unwrap().paint, None);
    assert_eq!(
        doc.layout_style_for_key(cell).unwrap().border.left,
        LengthPercentage::length(17.0)
    );
    assert!(doc.nodes[cell].collapsed_border.is_some());
}

#[test]
fn border_state_resets_to_authored_not_outer_used() {
    use raikiri_layout_style::{LengthProperty, OwnedPropertyOverride};
    use raikiri_style::ComputedLengthPercentageOrAuto as L;
    let mut doc = Document::new();
    let cell = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
    );
    let grid = TableGrid {
        n_cols: 1,
        rows: vec![0],
        cells: vec![make_cell(cell, 0, 0, 1, 1, Dimension::auto())],
        col_widths: vec![],
    };
    let mut tree = LayoutSession::new(&mut doc);
    let key = tree.nodes.box_key(cell).unwrap();
    let outer =
        OwnedPropertyOverride::length(LengthProperty::BorderLeft, L::Px(17.0), true).unwrap();
    tree.with_overrides(vec![(key, outer)], |tree| {
        let mut state = BorderState::authored(tree, &grid);
        let winner = collapsed_candidate(BorderStyle::Double, 9.0, CssColor::BLACK);
        state.set_side(
            cell,
            super::super::CellBorderSide::Left,
            winner.border,
            winner.taffy_width,
        );
        assert!(state.cell(cell).unwrap().paint.is_some());
        state.reset_authored(tree, &grid);
        assert_eq!(
            state.cell(cell).unwrap().widths.left,
            LengthPercentage::length(2.0)
        );
        assert_eq!(state.cell(cell).unwrap().paint, None);
        tree.with_overrides(state.entries(tree), |tree| {
            assert_eq!(
                tree.layout_style_for_key(cell).unwrap().border.left,
                LengthPercentage::length(2.0)
            );
        });
        state.set_side(
            cell,
            super::super::CellBorderSide::Left,
            winner.border,
            winner.taffy_width,
        );
        tree.with_overrides(state.entries(tree), |tree| {
            assert_eq!(
                tree.layout_style_for_key(cell).unwrap().border.left,
                LengthPercentage::length(9.0)
            );
        });
        assert_eq!(
            tree.layout_style_for_key(cell).unwrap().border.left,
            LengthPercentage::length(17.0)
        );
    });
    assert_eq!(
        tree.layout_style_for_key(cell).unwrap().border.left,
        LengthPercentage::length(2.0)
    );
}

#[test]
fn border_state_remaining_keeps_excluded_cells() {
    let mut doc = Document::new();
    let left = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
    );
    let right = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 9.0, CssColor::BLACK),
    );
    let grid = TableGrid {
        n_cols: 2,
        rows: vec![0],
        cells: vec![
            make_cell(left, 0, 0, 1, 1, Dimension::auto()),
            make_cell(right, 0, 1, 1, 1, Dimension::auto()),
        ],
        col_widths: vec![],
    };
    let shown = TableGrid {
        n_cols: 1,
        rows: vec![0],
        cells: vec![make_cell(right, 0, 0, 1, 1, Dimension::auto())],
        col_widths: vec![],
    };
    let mut state = BorderState::authored(&doc, &grid);
    super::super::resolve_collapsed_borders(&doc, &mut state, &grid, &CellIndex::new(&grid), 0);
    let excluded = state.cell(left).unwrap().clone();
    assert_eq!(excluded.widths.right, LengthPercentage::length(9.0));
    state.reset_authored(&doc, &shown);
    super::super::resolve_collapsed_borders(&doc, &mut state, &shown, &CellIndex::new(&shown), 0);
    assert_eq!(state.cell(left).unwrap(), &excluded);
    assert_eq!(state.entries(&doc).len(), 8);
    assert_eq!(
        doc.layout_style_for_key(left).unwrap().border.right,
        LengthPercentage::length(2.0)
    );
}

#[test]
fn border_state_mixes_width_without_replacing_paint() {
    let mut doc = Document::new();
    let cell = collapsed_cell(
        &mut doc,
        collapsed_candidate(BorderStyle::Solid, 2.0, CssColor::BLACK),
    );
    let grid = TableGrid {
        n_cols: 1,
        rows: vec![0],
        cells: vec![make_cell(cell, 0, 0, 1, 1, Dimension::auto())],
        col_widths: vec![],
    };
    let mut in_place = BorderState::authored(&doc, &grid);
    let mut remaining = in_place.clone();
    let first = collapsed_candidate(BorderStyle::Solid, 11.0, CssColor::BLACK);
    let second = collapsed_candidate(BorderStyle::Double, 29.0, CssColor::BLACK);
    in_place.set_side(
        cell,
        super::super::CellBorderSide::Top,
        first.border,
        first.taffy_width,
    );
    remaining.set_side(
        cell,
        super::super::CellBorderSide::Top,
        second.border,
        second.taffy_width,
    );
    let epoch = doc.layout_cache_context.publication_epoch();
    let columns = super::super::ColumnsInPlace::new(&grid, 7.0, in_place, remaining)
        .with_row_overlaps(&doc, &grid, &[false], &Rect::ZERO, None);
    assert_eq!(
        columns.in_place.cell(cell).unwrap().widths.top,
        LengthPercentage::length(29.0)
    );
    assert_eq!(
        columns.in_place.cell(cell).unwrap().paint.unwrap().top,
        first.border
    );
    assert_eq!(
        columns.remaining.cell(cell).unwrap().paint.unwrap().top,
        second.border
    );
    assert_eq!(doc.layout_cache_context.publication_epoch(), epoch);
    assert_eq!(
        doc.layout_style_for_key(cell).unwrap().border.top,
        LengthPercentage::length(2.0)
    );
    assert_eq!(doc.nodes[cell].collapsed_border, None);
}

#[test]
fn border_state_missing_computed_border_keeps_retained_width() {
    let mut doc = Document::new();
    let cell = bordered_cell(&mut doc, 7.0);
    let grid = TableGrid {
        n_cols: 1,
        rows: vec![0],
        cells: vec![make_cell(cell, 0, 0, 1, 1, Dimension::auto())],
        col_widths: vec![],
    };
    let mut state = BorderState::authored(&doc, &grid);
    super::super::resolve_collapsed_borders(&doc, &mut state, &grid, &CellIndex::new(&grid), 0);
    assert_eq!(
        state.cell(cell).unwrap().widths.left,
        LengthPercentage::length(7.0)
    );
    assert!(state.entries(&doc).is_empty());
    assert_eq!(state.cell(cell).unwrap().paint, None);
}
