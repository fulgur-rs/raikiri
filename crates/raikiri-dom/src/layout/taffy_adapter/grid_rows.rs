use super::GridRowCounts;
use raikiri_layout_style::{LayoutStyleView, TaffyStyle};
use smol_str::SmolStr;
use std::ops::Range;
use taffy::prelude::TaffyAuto;
use taffy::{
    GenericGridTemplateComponent, GenericRepetition, GridContainerStyle, GridItemStyle,
    GridPlacement, Line, RepetitionCount, TrackSizingFunction,
};

/// Whether the rows of the grid laid out with `style` all size to their
/// content: every explicit and implicit row track is intrinsic and uncapped,
/// so its items grow with what they hold.
pub(crate) fn rows_size_to_content(style: &LayoutStyleView<'_>) -> bool {
    let style = TaffyStyle::new(*style);
    // `auto`, `min-content` and `max-content` grow with the content;
    // `fit-content()` caps it.
    let auto = |track: TrackSizingFunction| {
        track.min.is_intrinsic()
            && (track.max.is_auto() || track.max.is_min_content() || track.max.is_max_content())
    };
    style.grid_template_rows().is_none_or(|mut components| {
        components.all(|component| match component {
            GenericGridTemplateComponent::Single(track) => auto(track),
            GenericGridTemplateComponent::Repeat(repetition) => repetition.tracks().all(auto),
        })
    }) && style.grid_auto_rows().all(auto)
}

/// Whether no column of the grid laid out with `style`, explicit or implicit,
/// has a `min-content` or `max-content` minimum track size.
///
/// Those tracks take the content contribution of their items whatever the
/// items state as their minimum, as every other intrinsic one does not.
pub(crate) fn columns_have_no_content_minimum(style: &LayoutStyleView<'_>) -> bool {
    let style = TaffyStyle::new(*style);
    let sized_by_content =
        |track: TrackSizingFunction| track.min.is_min_content() || track.min.is_max_content();
    style.grid_template_columns().is_none_or(|mut components| {
        components.all(|component| match component {
            GenericGridTemplateComponent::Single(track) => !sized_by_content(track),
            GenericGridTemplateComponent::Repeat(repetition) => {
                !repetition.tracks().any(sized_by_content)
            }
        })
    }) && !style.grid_auto_columns().any(sized_by_content)
}

/// Whether the width of some column of the grid laid out with `style` follows
/// the items in it: a track whose minimum or maximum size is `auto`,
/// `min-content`, `max-content` or `fit-content()`.
///
/// The columns are the ones the template declares, or the implicit ones that
/// `grid-auto-columns` sizes when it declares none. A column that is a length,
/// a percentage or a flexible track with such a minimum is as wide as it is
/// whatever the items are.
pub(crate) fn columns_follow_their_items(style: &LayoutStyleView<'_>) -> bool {
    let style = TaffyStyle::new(*style);
    let follows = |track: TrackSizingFunction| track.min.is_intrinsic() || track.max.is_intrinsic();
    let mut template = style
        .grid_template_columns()
        .into_iter()
        .flatten()
        .peekable();
    if template.peek().is_none() {
        // Without `grid-auto-columns`, implicit columns are `auto`.
        let mut implicit = style.grid_auto_columns().peekable();
        return implicit.peek().is_none() || implicit.any(follows);
    }
    template.any(|component| match component {
        GenericGridTemplateComponent::Single(track) => follows(track),
        GenericGridTemplateComponent::Repeat(repetition) => repetition.tracks().any(follows),
    })
}

/// The sizing function of every row of a grid, from the first implicit row
/// before the explicit grid to the last after it.
#[derive(Debug)]
pub(crate) struct RowSizing(Vec<TrackSizingFunction>);

impl RowSizing {
    /// Whether the rows of the grid laid out with `style` repeat to fill its
    /// height (`repeat(auto-fill, ...)` or `repeat(auto-fit, ...)`): how many
    /// there are depends on whether that height is known.
    pub(crate) fn repeat_to_fill(style: &LayoutStyleView<'_>) -> bool {
        let style = TaffyStyle::new(*style);
        style.grid_template_rows().is_some_and(|mut components| {
            components.any(|component| {
                matches!(
                    component,
                    GenericGridTemplateComponent::Repeat(repeat)
                        if matches!(repeat.count(), RepetitionCount::AutoFill | RepetitionCount::AutoFit)
                )
            })
        })
    }

    /// Whether a row of the grid laid out with `style` is at least a positive
    /// length tall whatever its items hold: an explicit row, or with `items`
    /// an implicit one, whose minimum track size is such a length.
    /// Percentages are left out, as the grid's own block size is what they
    /// would resolve against.
    pub(crate) fn have_a_length(style: &LayoutStyleView<'_>, items: bool) -> bool {
        let style = TaffyStyle::new(*style);
        let length = |track: TrackSizingFunction| {
            track
                .min
                .definite_value(None, |_, _| 0.0)
                .is_some_and(|px| px > 0.0)
        };
        style.grid_template_rows().is_some_and(|mut components| {
            components.any(|component| match component {
                GenericGridTemplateComponent::Single(track) => length(track),
                GenericGridTemplateComponent::Repeat(repetition) => repetition.tracks().any(length),
            })
        }) || items && style.grid_auto_rows().any(length)
    }

    /// The sizing function of every row of the grid laid out with `style`, as
    /// the grid algorithm expands them for the row counts `rows` (CSS Grid 1
    /// §7.2 explicit tracks, §7.6 implicit tracks).
    pub(crate) fn of(style: &LayoutStyleView<'_>, rows: GridRowCounts) -> Self {
        let style = TaffyStyle::new(*style);
        let negative = usize::from(rows.negative_implicit);
        let explicit = usize::from(rows.explicit);
        let positive = usize::from(rows.positive_implicit);
        let auto_rows = style.grid_auto_rows().collect::<Vec<_>>();
        // Without `grid-auto-rows`, implicit rows are `auto`.
        let implicit = |index: usize| {
            let count = auto_rows.len().max(1);
            auto_rows
                .get(index % count)
                .copied()
                .unwrap_or(TrackSizingFunction::AUTO)
        };
        let mut functions = Vec::with_capacity(negative + explicit + positive);
        // The implicit rows before the explicit grid end with the last of
        // `grid-auto-rows`.
        let offset = if auto_rows.is_empty() {
            0
        } else {
            auto_rows.len() - negative % auto_rows.len()
        };
        functions.extend((0..negative).map(|index| implicit(offset + index)));
        // An automatic repetition takes the explicit rows that the other
        // components leave.
        let fixed_count = style
            .grid_template_rows()
            .into_iter()
            .flatten()
            .map(|component| match component {
                GenericGridTemplateComponent::Single(_) => 1,
                GenericGridTemplateComponent::Repeat(repeat) => match repeat.count() {
                    RepetitionCount::Count(count) => repeat.tracks().len() * usize::from(count),
                    RepetitionCount::AutoFill | RepetitionCount::AutoFit => 0,
                },
            })
            .sum::<usize>();
        let mut template = Vec::with_capacity(explicit);
        for component in style.grid_template_rows().into_iter().flatten() {
            match component {
                GenericGridTemplateComponent::Single(function) => template.push(function),
                GenericGridTemplateComponent::Repeat(repeat) => {
                    let count = match repeat.count() {
                        RepetitionCount::Count(count) => repeat.tracks().len() * usize::from(count),
                        RepetitionCount::AutoFill | RepetitionCount::AutoFit => {
                            explicit.saturating_sub(fixed_count)
                        }
                    };
                    template.extend(repeat.tracks().cycle().take(count));
                }
            }
        }
        template.truncate(explicit);
        let template_len = template.len();
        functions.extend(template);
        // Explicit rows the template does not reach, if any, are sized as the
        // implicit rows after them.
        functions.extend((0..explicit - template_len + positive).map(implicit));
        Self(functions)
    }

    /// Whether the rows `spanned`, numbered from the first row of the grid,
    /// include one that is sized by its content or by the free space of the
    /// grid, and so has no length before the grid is laid out. `false` when
    /// `spanned` reaches past the last row.
    pub(crate) fn any_sized_by_content_or_free_space(&self, spanned: Range<usize>) -> bool {
        self.0.get(spanned).is_some_and(|rows| {
            rows.iter()
                .any(|row| row.min.is_intrinsic() || row.max.is_intrinsic() || row.max.is_fr())
        })
    }
}

/// Whether some row of the grid laid out with `style` that its in-flow
/// `items` bring into being has an `auto` maximum track size, which
/// `align-content: normal` and `stretch` grow to fill a definite height (CSS
/// Grid 1 §12.8).
///
/// The explicit rows are all there. An item placed on a row line by number
/// brings in the implicit rows up to it on either side of the explicit grid.
/// The other items are placed automatically: in a row flow those that the
/// free explicit cells do not hold fill implicit rows a row of columns at a
/// time, which an automatic repetition never lets them do, and in either flow
/// an item that spans more rows than the explicit grid has adds the rest.
/// A named line is taken to be one the explicit grid has.
pub(crate) fn some_row_stretches<'a>(
    style: &LayoutStyleView<'_>,
    items: impl IntoIterator<Item = LayoutStyleView<'a>>,
) -> bool {
    let style = TaffyStyle::new(*style);
    let stretches = |track: &TrackSizingFunction| track.max.is_auto();
    let rows = style
        .grid_template_rows()
        .into_iter()
        .flatten()
        .map(|component| match component {
            GenericGridTemplateComponent::Single(_) => 1,
            GenericGridTemplateComponent::Repeat(repeat) => repeat_len(&repeat),
        })
        .fold(0, usize::saturating_add);
    let columns = style
        .grid_template_columns()
        .into_iter()
        .flatten()
        .map(|component| match component {
            GenericGridTemplateComponent::Single(_) => 1,
            GenericGridTemplateComponent::Repeat(repeat) => repeat_len(&repeat),
        })
        .fold(0, usize::saturating_add);
    let explicit = style.grid_template_rows().is_some_and(|mut components| {
        components.any(|component| match component {
            GenericGridTemplateComponent::Single(track) => stretches(&track),
            GenericGridTemplateComponent::Repeat(repetition) => {
                repetition.tracks().any(|track| stretches(&track))
            }
        })
    });
    let row_flow = matches!(
        style.grid_auto_flow(),
        taffy::GridAutoFlow::Row | taffy::GridAutoFlow::RowDense
    );
    let dense = matches!(
        style.grid_auto_flow(),
        taffy::GridAutoFlow::RowDense | taffy::GridAutoFlow::ColumnDense
    );
    let placements = items
        .into_iter()
        .map(|item| {
            let item = TaffyStyle::new(item);
            (item.grid_row(), item.grid_column())
        })
        .collect::<Vec<_>>();
    let (before, after) = match (i32::try_from(rows), i32::try_from(columns)) {
        (Ok(row_count), Ok(column_count)) => {
            let axes = |(row, column): &(Line<GridPlacement<SmolStr>>, _)| {
                let row = AxisPlacement::of(row, row_count);
                let column = AxisPlacement::of(column, column_count);
                if row_flow {
                    [row, column]
                } else {
                    [column, row]
                }
            };
            let items = placements.iter().map(axes).collect::<Vec<_>>();
            let counts = if row_flow {
                [row_count, column_count]
            } else {
                [column_count, row_count]
            };
            let extents = placed_extents(&items, counts, dense);
            let (start, end) = extents[usize::from(!row_flow)];
            (
                usize::try_from(-start).unwrap_or_default(),
                usize::try_from(end - row_count).unwrap_or_default(),
            )
        }
        _ => estimated_implicit_rows(&placements, rows, row_flow),
    };
    // The implicit rows after the explicit grid take the entries of
    // `grid-auto-rows` in turn from the first, and those before it from the
    // last, so an entry no row reaches sizes nothing.
    let auto_rows = style.grid_auto_rows().collect::<Vec<_>>();
    explicit
        || auto_rows.iter().take(after).any(stretches)
        || auto_rows.iter().rev().take(before).any(stretches)
}

/// The lines, numbered from zero at the start of the explicit grid that has
/// `count` row lines past the first, between which a placement on a numbered
/// line puts an item, or `None` for an item placed automatically.
fn definite_lines(placement: &Line<GridPlacement<SmolStr>>, count: i32) -> Option<(i32, i32)> {
    let line = |placement: &GridPlacement<SmolStr>| match placement {
        GridPlacement::Line(line) if line.as_i16() > 0 => Some(i32::from(line.as_i16()) - 1),
        GridPlacement::Line(line) if line.as_i16() < 0 => {
            Some(count + 1 + i32::from(line.as_i16()))
        }
        _ => None,
    };
    let span = |placement: &GridPlacement<SmolStr>| match placement {
        GridPlacement::Span(span) => i32::from(*span).max(1),
        _ => 1,
    };
    match (line(&placement.start), line(&placement.end)) {
        (Some(start), Some(end)) if start == end => Some((start, start + 1)),
        (Some(start), Some(end)) => Some((start.min(end), start.max(end))),
        (Some(start), None) => Some((start, start + span(&placement.end))),
        (None, Some(end)) => Some((end - span(&placement.start), end)),
        (None, None) => None,
    }
}

/// The tracks a placement that no numbered line fixes spans: the count of a
/// `span` at either end, or one.
fn span_of(placement: &Line<GridPlacement<SmolStr>>) -> usize {
    let span = |placement: &GridPlacement<SmolStr>| match placement {
        GridPlacement::Span(span) => usize::from(*span).max(1),
        _ => 1,
    };
    span(&placement.start).max(span(&placement.end))
}

/// The placement of a grid item between the lines of one axis.
type ItemLines = Line<GridPlacement<SmolStr>>;

/// How many implicit rows the grid makes before and after its explicit
/// grid of `rows` rows for items with the row and column `placements`, when
/// an automatic repetition leaves the count of its rows or columns unknown
/// and automatic placement cannot be followed cell by cell.
///
/// Rows that repeat to fill the grid leave no line to count from, and are
/// taken to hold every item. Columns that do are taken to hold every item
/// in the explicit rows, so only an item on a numbered row line past them,
/// or, without any, each item a row flow places automatically, brings in
/// an implicit row; in either flow an item that spans more rows than the
/// explicit grid has adds the rest.
fn estimated_implicit_rows(
    placements: &[(ItemLines, ItemLines)],
    rows: usize,
    row_flow: bool,
) -> (usize, usize) {
    let Ok(count) = i32::try_from(rows) else {
        return (0, 0);
    };
    let mut after = 0_usize;
    let mut before = 0_usize;
    let mut automatic = 0_usize;
    for (placement, _) in placements {
        match definite_lines(placement, count) {
            Some((start, end)) => {
                after = after.max(usize::try_from(end - count).unwrap_or_default());
                before = before.max(usize::try_from(-start).unwrap_or_default());
            }
            None => {
                automatic += 1;
                after = after.max(span_of(placement).saturating_sub(rows));
            }
        }
    }
    if row_flow && rows == 0 {
        after = after.max(automatic);
    }
    (before, after)
}

/// Where a grid item sits along one axis of its grid: between the lines a
/// numbered line places it, numbered from zero at the start of the explicit
/// grid, or anywhere automatic placement puts it; and how many tracks it
/// spans.
#[derive(Clone, Copy)]
struct AxisPlacement {
    lines: Option<(i32, i32)>,
    span: i32,
}

impl AxisPlacement {
    fn of(placement: &Line<GridPlacement<SmolStr>>, count: i32) -> Self {
        let lines = definite_lines(placement, count);
        let span = lines.map_or_else(
            || i32::try_from(span_of(placement)).unwrap_or(1),
            |(start, end)| end - start,
        );
        Self { lines, span }
    }
}

/// The lines, along the axis the auto-placement cursor moves across and the
/// one it moves along, between which the grid places `items` (CSS Grid 1
/// §8.5): the explicit grid of `counts` tracks, and the implicit tracks the
/// items bring in on either side. Each item gives its placement in the same
/// order of axes, as `dense` or sparse packing fills the grid.
fn placed_extents(items: &[[AxisPlacement; 2]], counts: [i32; 2], dense: bool) -> [(i32, i32); 2] {
    type Area = [(i32, i32); 2];
    let overlaps = |placed: &[Area], area: Area| {
        placed.iter().any(|other| {
            (0..2).all(|axis| area[axis].0 < other[axis].1 && other[axis].0 < area[axis].1)
        })
    };
    let mut placed = Vec::<Area>::with_capacity(items.len());
    // Items placed on numbered lines along both axes.
    for item in items {
        if let [
            AxisPlacement {
                lines: Some(major), ..
            },
            AxisPlacement {
                lines: Some(minor), ..
            },
        ] = item
        {
            placed.push([*major, *minor]);
        }
    }
    // Items locked to a track the cursor moves across take the first cells
    // in it that are free, past those placed in it before when sparse.
    let mut track_cursors = Vec::<(i32, i32)>::new();
    for item in items {
        let [
            AxisPlacement {
                lines: Some(major), ..
            },
            AxisPlacement { lines: None, span },
        ] = item
        else {
            continue;
        };
        let cursor = track_cursors
            .iter()
            .find(|(track, _)| *track == major.0)
            .map(|&(_, at)| at);
        let mut minor = if dense { 0 } else { cursor.unwrap_or(0) };
        while overlaps(&placed, [*major, (minor, minor + span)]) {
            minor += 1;
        }
        placed.push([*major, (minor, minor + span)]);
        track_cursors.retain(|(track, _)| *track != major.0);
        track_cursors.push((major.0, minor + span));
    }
    // The tracks the cursor moves along hold the explicit ones, every item
    // placed on a line along them, and the widest span of the others.
    let mut minor_start = 0;
    let mut minor_end = counts[1];
    let mut widest = 0;
    for item in items {
        match item[1].lines {
            Some((start, end)) => {
                minor_start = minor_start.min(start);
                minor_end = minor_end.max(end);
            }
            None => widest = widest.max(item[1].span),
        }
    }
    for area in &placed {
        minor_start = minor_start.min(area[1].0);
        minor_end = minor_end.max(area[1].1);
    }
    minor_end = minor_end.max(minor_start + widest);
    let major_start = placed.iter().map(|area| area[0].0).fold(0, i32::min);
    // The remaining items follow the cursor.
    let mut cursor = (major_start, minor_start);
    for item in items {
        let [
            AxisPlacement {
                lines: None,
                span: major_span,
            },
            minor,
        ] = item
        else {
            continue;
        };
        let area = match minor.lines {
            Some(lines) => {
                let mut major = if dense {
                    major_start
                } else {
                    cursor.0 + i32::from(lines.0 < cursor.1)
                };
                while overlaps(&placed, [(major, major + major_span), lines]) {
                    major += 1;
                }
                cursor = (major, lines.0);
                [(major, major + major_span), lines]
            }
            None => {
                if dense {
                    cursor = (major_start, minor_start);
                }
                loop {
                    let area = [
                        (cursor.0, cursor.0 + major_span),
                        (cursor.1, cursor.1 + minor.span),
                    ];
                    if area[1].1 > minor_end {
                        cursor = (cursor.0 + 1, minor_start);
                    } else if overlaps(&placed, area) {
                        cursor.1 += 1;
                    } else {
                        break area;
                    }
                }
            }
        };
        placed.push(area);
    }
    let major = placed.iter().fold((0, counts[0]), |(start, end), area| {
        (start.min(area[0].0), end.max(area[0].1))
    });
    [major, (minor_start, minor_end)]
}

/// The tracks a repetition gives. An automatic one repeats to fill the grid,
/// so it is taken to give as many as the items need.
fn repeat_len<R: GenericRepetition>(repeat: &R) -> usize {
    match repeat.count() {
        RepetitionCount::Count(count) => repeat.tracks().len() * usize::from(count),
        RepetitionCount::AutoFill | RepetitionCount::AutoFit => usize::MAX,
    }
}
