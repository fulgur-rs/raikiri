//! What the grid algorithm reports about a grid container is read back through
//! the wrapper as the report says it, and a report without tracks reads as an
//! empty grid.

use super::*;
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::taffy_adapter::algorithms::compute_grid_layout;
use crate::layout::taffy_adapter::grid_info::GridItemLines;
use crate::layout::taffy_adapter::{GridInfo, GridRowCounts};
use crate::layout::test_support::cascaded;
use raikiri_layout_style::{LayoutStyleView, PreparedStyle, StyleMetadata, ViewContext};
use raikiri_style::ComputedValues;
use raikiri_style::property::GridLineValue;
use taffy::Direction;

/// A grid of two columns (50px, 70px) and one explicit row (20px), with three
/// children of 10px height: the third one lands in an implicit row.
fn reported_grid() -> GridInfo {
    let mut document = Document::new();
    let grid = document.append_element(
        Some(0),
        "grid",
        Some(
            "display:grid;width:300px;grid-template-columns:50px 70px;grid-template-rows:20px;\
             padding:3px 4px 5px 6px",
        ),
    );
    for _ in 0..3 {
        document.append_element(Some(grid), "item", Some("display:block;height:10px"));
    }
    document.mark_in_document_flags();
    let cascade = cascaded(&document);
    let mut session = LayoutSession::bridged(&mut document, &cascade);
    let input = LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: false,
            height: false,
        },
        parent_size: Size {
            width: Some(300.0),
            height: None,
        },
        available_space: Size {
            width: AvailableSpace::Definite(300.0),
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line {
            start: false,
            end: false,
        },
    };
    compute_grid_layout(&mut session, NodeId::from(grid), input);
    session
        .context
        .grid_layout_info
        .remove(&grid)
        .expect("the grid algorithm reports the grid")
}

fn lines(row_start: u16, row_end: u16, column_start: u16, column_end: u16) -> GridItemLines {
    GridItemLines {
        row_start,
        row_end,
        column_start,
        column_end,
    }
}

/// Run `check` on the view of a box placed between the grid lines `row` and
/// between the grid lines `column`.
fn placed<R>(
    row: (i32, i32),
    column: (i32, i32),
    check: impl FnOnce(&LayoutStyleView<'_>) -> R,
) -> R {
    let mut values = ComputedValues::initial();
    values.grid_row_start = GridLineValue::Line(row.0);
    values.grid_row_end = GridLineValue::Line(row.1);
    values.grid_column_start = GridLineValue::Line(column.0);
    values.grid_column_end = GridLineValue::Line(column.1);
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    check(&LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &[],
        &[],
    ))
}

#[test]
fn a_reported_grid_lists_its_items_columns_and_rows() {
    let info = reported_grid();

    assert_eq!(info.column_count(), 2);
    assert_eq!(
        info.row_counts(),
        GridRowCounts {
            negative_implicit: 0,
            explicit: 1,
            positive_implicit: 1,
        }
    );
    let items = info.items();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items.collect::<Vec<_>>(),
        [lines(1, 2, 1, 2), lines(1, 2, 2, 3), lines(2, 3, 1, 2)]
    );
}

#[test]
fn the_area_of_an_item_is_the_extent_of_the_tracks_it_spans() {
    let info = reported_grid();
    let area = |index| {
        info.item_area_size(index)
            .map(|size| (size.width, size.height))
    };

    assert_eq!(area(0), Some((50.0, 20.0)));
    assert_eq!(area(1), Some((70.0, 20.0)));
    assert_eq!(area(2), Some((50.0, 10.0)));
    assert_eq!(area(3), None);
}

#[test]
fn the_inline_range_of_an_item_runs_over_the_columns_it_spans() {
    let info = reported_grid();

    // The columns start after the 6px of padding.
    assert_eq!(info.item_inline_range(0), Some((6.0, 56.0)));
    assert_eq!(info.item_inline_range(1), Some((56.0, 126.0)));
    assert_eq!(info.item_inline_range(2), Some((6.0, 56.0)));
    assert_eq!(info.item_inline_range(3), None);
    assert_eq!(GridInfo::NONE.item_inline_range(0), None);
}

#[test]
fn a_placed_box_gets_the_area_its_lines_bound() {
    let info = reported_grid();
    let padding_box = Edges {
        left: 6.0,
        right: 300.0,
        top: 3.0,
        bottom: 40.0,
    };
    let area = placed((1, 2), (2, 3), |style| {
        info.resolve_absolute_grid_area(style, Direction::Ltr, padding_box)
    });

    // The second column starts after the first one's 50px, and the first row
    // is 20px high, both from the content box origin at (6, 3).
    assert_eq!(
        (area.left, area.right, area.top, area.bottom),
        (56.0, 126.0, 3.0, 23.0)
    );
}

#[test]
fn a_report_without_tracks_reads_as_an_empty_grid() {
    let info = GridInfo::NONE;
    let padding_box = Edges {
        left: 1.0,
        right: 2.0,
        top: 3.0,
        bottom: 4.0,
    };

    assert_eq!(info.column_count(), 0);
    assert_eq!(info.row_counts(), GridRowCounts::default());
    assert_eq!(info.items().len(), 0);
    assert_eq!(info.item_area_size(0), None);
    assert_eq!(
        placed((1, 2), (1, 2), |style| info.resolve_absolute_grid_area(
            style,
            Direction::Ltr,
            padding_box
        )),
        padding_box
    );
    assert_eq!(*info.detailed(), taffy::DetailedLayoutInfo::None);
}
