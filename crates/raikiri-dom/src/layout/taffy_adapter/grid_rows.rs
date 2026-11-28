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
    // Rows that an automatic repetition fills leave no line to count from.
    let line_count = i32::try_from(rows).ok();
    let mut after = 0_usize;
    let mut before = 0_usize;
    let mut occupied = 0_usize;
    let mut automatic = 0_usize;
    let mut automatic_span = 0_usize;
    for item in items {
        let placement = TaffyStyle::new(item).grid_row();
        match line_count.and_then(|count| definite_lines(&placement, count)) {
            Some((start, end)) => {
                let count = line_count.unwrap_or_default();
                after = after.max(usize::try_from(end - count).unwrap_or_default());
                before = before.max(usize::try_from(-start).unwrap_or_default());
                occupied += usize::from(start < count && end > 0);
            }
            None => {
                automatic += 1;
                automatic_span =
                    automatic_span.max(span_of(&placement.start).max(span_of(&placement.end)));
            }
        }
    }
    if automatic > 0 {
        after = after.max(automatic_span.saturating_sub(rows));
    }
    // A column flow places the items that overflow the explicit grid in new
    // columns, so only a row flow makes rows for them.
    if matches!(
        style.grid_auto_flow(),
        taffy::GridAutoFlow::Row | taffy::GridAutoFlow::RowDense
    ) {
        let free = rows.saturating_mul(columns.max(1)).saturating_sub(occupied);
        // How many columns an automatic repetition gives depends on the
        // width, so each item is then taken to start a row of its own.
        let per_row = if columns == usize::MAX {
            1
        } else {
            columns.max(1)
        };
        after = after.max(automatic.saturating_sub(free).div_ceil(per_row));
    }
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
    let span = |placement: &GridPlacement<SmolStr>| i32::try_from(span_of(placement)).unwrap_or(1);
    match (line(&placement.start), line(&placement.end)) {
        (Some(start), Some(end)) if start == end => Some((start, start + 1)),
        (Some(start), Some(end)) => Some((start.min(end), start.max(end))),
        (Some(start), None) => Some((start, start + span(&placement.end))),
        (None, Some(end)) => Some((end - span(&placement.start), end)),
        (None, None) => None,
    }
}

/// The tracks a placement spans: its `span` count, or one.
fn span_of(placement: &GridPlacement<SmolStr>) -> usize {
    match placement {
        GridPlacement::Span(span) => usize::from(*span).max(1),
        _ => 1,
    }
}

/// The tracks a repetition gives. An automatic one repeats to fill the grid,
/// so it is taken to give as many as the items need.
fn repeat_len<R: GenericRepetition>(repeat: &R) -> usize {
    match repeat.count() {
        RepetitionCount::Count(count) => repeat.tracks().len() * usize::from(count),
        RepetitionCount::AutoFill | RepetitionCount::AutoFit => usize::MAX,
    }
}
