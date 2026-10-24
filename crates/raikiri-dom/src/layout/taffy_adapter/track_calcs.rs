//! The calc payloads that the grid tracks of a style point at, for tests that
//! check who owns them.

use raikiri_layout_style::{LayoutStyleView, TaffyStyle};
use taffy::{GenericGridTemplateComponent, GenericRepetition, GridContainerStyle};

/// Which list of grid tracks of a style to read.
#[derive(Clone, Copy)]
pub(crate) enum TrackList {
    TemplateColumns,
    TemplateRows,
    AutoColumns,
    AutoRows,
}

/// The calc payloads of one grid track: those of its minimum and of its
/// maximum sizing function, null where the function holds no calc.
pub(crate) struct TrackCalcs {
    /// Whether the track belongs to a `repeat()`.
    pub(crate) in_repetition: bool,
    pub(crate) min: *const (),
    pub(crate) max: *const (),
}

/// The calc payloads of the tracks of `list`, in track order, those inside a
/// `repeat()` once for each track it lists.
pub(crate) fn track_calcs(style: &LayoutStyleView<'_>, list: TrackList) -> Vec<TrackCalcs> {
    let style = TaffyStyle::new(*style);
    let calcs = |track: taffy::TrackSizingFunction, in_repetition| TrackCalcs {
        in_repetition,
        min: track.min.into_raw().calc_value(),
        max: track.max.into_raw().calc_value(),
    };
    let template = match list {
        TrackList::TemplateColumns => style.grid_template_columns(),
        TrackList::TemplateRows => style.grid_template_rows(),
        TrackList::AutoColumns => {
            return style.grid_auto_columns().map(|t| calcs(t, false)).collect();
        }
        TrackList::AutoRows => return style.grid_auto_rows().map(|t| calcs(t, false)).collect(),
    };
    let mut tracks = Vec::new();
    for component in template.into_iter().flatten() {
        match component {
            GenericGridTemplateComponent::Single(track) => tracks.push(calcs(track, false)),
            GenericGridTemplateComponent::Repeat(repeat) => {
                tracks.extend(repeat.tracks().map(|track| calcs(track, true)));
            }
        }
    }
    tracks
}
