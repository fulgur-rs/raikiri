use super::convert::{FromTaffy, IntoTaffy};
use raikiri_layout_geometry::{Edges, Size};
use smol_str::SmolStr;
use taffy::{DetailedGridInfo, DetailedLayoutInfo, Direction, GridPlacement, Line};

/// The grid lines an item of a grid occupies, as 1-based line numbers: the
/// item spans the rows from `row_start` up to, not including, `row_end`, and
/// starts at the column line `column_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridItemLines {
    pub(crate) row_start: u16,
    pub(crate) row_end: u16,
    pub(crate) column_start: u16,
}

/// How many rows a grid has before, in and after its explicit grid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GridRowCounts {
    /// The implicit rows before the explicit grid.
    pub(crate) negative_implicit: u16,
    /// The rows the template declares.
    pub(crate) explicit: u16,
    /// The implicit rows after the explicit grid.
    pub(crate) positive_implicit: u16,
}

/// What the grid algorithm reports about a grid container: where its tracks
/// are, which lines each item occupies, and what is needed to resolve the grid
/// area of an out-of-flow box.
///
/// It is the library's report, wrapped. The library hands it over once per
/// grid layout and takes it back when it positions the out-of-flow boxes of
/// the container, so the wrapper moves it without copying and the rest of the
/// layout reads it through the methods here.
///
/// A value that reports no tracks reads as an empty grid: no items, no
/// columns, no item area, and the padding box as the grid area of any
/// placement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridInfo(DetailedLayoutInfo<SmolStr>);

impl GridInfo {
    /// The report of a container that has none.
    pub(crate) const NONE: Self = Self(DetailedLayoutInfo::None);

    /// The report of the grid algorithm for a grid container.
    #[inline]
    pub(crate) fn new(grid: DetailedGridInfo<SmolStr>) -> Self {
        Self(DetailedLayoutInfo::Grid(Box::new(grid)))
    }

    /// The report as the library's type, for the library to read.
    #[inline]
    pub(super) fn detailed(&self) -> &DetailedLayoutInfo<SmolStr> {
        &self.0
    }

    /// The grid the report describes, if it describes one.
    #[inline]
    fn grid(&self) -> Option<&DetailedGridInfo<SmolStr>> {
        match &self.0 {
            DetailedLayoutInfo::Grid(grid) => Some(grid),
            DetailedLayoutInfo::None => None,
        }
    }

    /// The number of columns, implicit ones included.
    pub(crate) fn column_count(&self) -> usize {
        self.grid().map_or(0, |grid| grid.columns.positions.len())
    }

    /// The grid lines of each item, in source order of the items in flow.
    pub(crate) fn items(&self) -> impl ExactSizeIterator<Item = GridItemLines> + '_ {
        self.grid()
            .map_or(&[][..], |grid| grid.items.as_slice())
            .iter()
            .map(|item| GridItemLines {
                row_start: item.row_start,
                row_end: item.row_end,
                column_start: item.column_start,
            })
    }

    /// How many rows the grid has before, in and after its explicit grid.
    pub(crate) fn row_counts(&self) -> GridRowCounts {
        let rows = self.grid().map(|grid| &grid.rows);
        GridRowCounts {
            negative_implicit: rows.map_or(0, |rows| rows.negative_implicit_tracks),
            explicit: rows.map_or(0, |rows| rows.explicit_tracks),
            positive_implicit: rows.map_or(0, |rows| rows.positive_implicit_tracks),
        }
    }

    /// The size of the grid area of the item `index`, which is the extent of
    /// the tracks the item spans without the gaps around them.
    pub(crate) fn item_area_size(&self, index: usize) -> Option<Size<f32>> {
        let (_, size) = self.grid()?.item_grid_area(index)?;
        Some(Size::from_taffy(size))
    }

    /// The grid area that the grid placement `row` and `column` of an
    /// out-of-flow box resolves to, relative to the border box of the container.
    ///
    /// `padding_box` is the padding box of the container and `direction` its
    /// inline direction.
    pub(crate) fn resolve_absolute_grid_area(
        &self,
        row: Line<GridPlacement<SmolStr>>,
        column: Line<GridPlacement<SmolStr>>,
        direction: Direction,
        padding_box: Edges<f32>,
    ) -> Edges<f32> {
        self.grid().map_or(padding_box, |grid| {
            Edges::from_taffy(grid.resolve_absolute_grid_area(
                row,
                column,
                direction,
                padding_box.into_taffy(),
            ))
        })
    }
}
