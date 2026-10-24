use super::GridRowCounts;
use raikiri_layout_style::{LayoutStyleView, TaffyStyle};
use std::ops::Range;
use taffy::prelude::TaffyAuto;
use taffy::{
    GenericGridTemplateComponent, GenericRepetition, GridContainerStyle, RepetitionCount,
    TrackSizingFunction,
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

/// The sizing function of every row of a grid, from the first implicit row
/// before the explicit grid to the last after it.
#[derive(Debug)]
pub(crate) struct RowSizing(Vec<TrackSizingFunction>);

impl RowSizing {
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
