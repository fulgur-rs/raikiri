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
use taffy::Line as StyleLine;
use taffy::prelude::TaffyGridLine;
use taffy::{Direction, GridPlacement};

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

fn lines(row_start: u16, row_end: u16, column_start: u16) -> GridItemLines {
    GridItemLines {
        row_start,
        row_end,
        column_start,
    }
}

fn placement(start: i16, end: i16) -> StyleLine<GridPlacement<smol_str::SmolStr>> {
    StyleLine {
        start: GridPlacement::from_line_index(start),
        end: GridPlacement::from_line_index(end),
    }
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
        [lines(1, 2, 1), lines(1, 2, 2), lines(2, 3, 1)]
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
fn a_placed_box_gets_the_area_its_lines_bound() {
    let info = reported_grid();
    let padding_box = Edges {
        left: 6.0,
        right: 300.0,
        top: 3.0,
        bottom: 40.0,
    };
    let area = info.resolve_absolute_grid_area(
        placement(1, 2),
        placement(2, 3),
        Direction::Ltr,
        padding_box,
    );

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
        info.resolve_absolute_grid_area(
            placement(1, 2),
            placement(1, 2),
            Direction::Ltr,
            padding_box,
        ),
        padding_box
    );
    assert_eq!(*info.detailed(), taffy::DetailedLayoutInfo::None);
}
