//! How the sizes of the rows of a grid are read from its style: whether they
//! all follow their content, and which of the rows a grid reports are sized by
//! content or free space.

use crate::layout::taffy_adapter::{GridRowCounts, RowSizing, rows_size_to_content};
use raikiri_layout_style::{LayoutStyleView, PreparedStyle, StyleMetadata, ViewContext};
use raikiri_style::property::GridRepeatCount;
use raikiri_style::{
    ComputedGridTemplateTracks as Template, ComputedGridTrackBreadth as B, ComputedGridTrackList,
    ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
    ComputedLengthPercentage as LP, ComputedValues,
};
use std::sync::Arc;

/// A row template made of `components`, with no line names.
fn template(components: Vec<C>) -> Template {
    Template::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![]; components.len() + 1],
        components,
    }))
}

fn repeat(count: GridRepeatCount, tracks: Vec<S>) -> C {
    C::Repeat(ComputedGridTrackRepeat {
        count,
        line_names: vec![vec![]; tracks.len() + 1],
        tracks,
    })
}

/// Run `check` on the view of a grid whose `grid-template-rows` is `rows` and
/// whose `grid-auto-rows` is `auto_rows`.
fn with_view<R>(
    rows: Template,
    auto_rows: Vec<S>,
    check: impl FnOnce(&LayoutStyleView<'_>) -> R,
) -> R {
    let mut values = ComputedValues::initial();
    values.grid_template_rows = rows;
    values.grid_auto_rows = Arc::new(auto_rows);
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    check(&LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &[],
        &[],
    ))
}

fn counts(negative_implicit: u16, explicit: u16, positive_implicit: u16) -> GridRowCounts {
    GridRowCounts {
        negative_implicit,
        explicit,
        positive_implicit,
    }
}

/// For every row the grid is reported to have, whether the row is sized by
/// content or free space. A range past the last row reads as `false`, so the
/// row count is checked as well.
fn flags(rows: Template, auto_rows: Vec<S>, row_counts: GridRowCounts) -> Vec<bool> {
    let total = usize::from(row_counts.negative_implicit)
        + usize::from(row_counts.explicit)
        + usize::from(row_counts.positive_implicit);
    with_view(rows, auto_rows, |view| {
        let sizing = RowSizing::of(view, row_counts);
        assert!(
            !sizing.any_sized_by_content_or_free_space(0..total + 1),
            "the rows reach no further than the counts say"
        );
        (0..total)
            .map(|row| sizing.any_sized_by_content_or_free_space(row..row + 1))
            .collect()
    })
}

const FIXED: S = S::Breadth(B::Px(10.0));
const AUTO: S = S::Breadth(B::Auto);

#[test]
fn implicit_rows_are_auto_without_grid_auto_rows() {
    assert_eq!(
        flags(Template::None, vec![], counts(1, 0, 2)),
        [true, true, true]
    );
}

#[test]
fn fixed_rows_are_sized_by_neither_content_nor_free_space() {
    let fixed = || {
        template(vec![
            C::Size(FIXED),
            C::Size(S::MinMax(B::Px(5.0), B::Px(9.0))),
        ])
    };
    assert_eq!(flags(fixed(), vec![], counts(0, 2, 0)), [false, false]);
}

#[test]
fn an_intrinsic_or_flexible_bound_makes_a_row_follow_content_or_free_space() {
    let rows = template(vec![
        C::Size(FIXED),
        C::Size(S::MinMax(B::Px(5.0), B::Auto)),
        C::Size(S::MinMax(B::Px(5.0), B::Flex(1.0))),
        C::Size(S::MinMax(B::Px(5.0), B::MinContent)),
        C::Size(S::MinMax(B::MaxContent, B::Px(9.0))),
        C::Size(S::FitContent(LP::Px(30.0))),
    ]);
    assert_eq!(
        flags(rows, vec![], counts(0, 6, 0)),
        [false, true, true, true, true, true]
    );
}

#[test]
fn a_counted_repetition_repeats_its_tracks() {
    let rows = template(vec![
        C::Size(AUTO),
        repeat(GridRepeatCount::Count(2), vec![FIXED, AUTO]),
    ]);
    assert_eq!(
        flags(rows, vec![], counts(0, 5, 0)),
        [true, false, true, false, true]
    );
}

#[test]
fn an_automatic_repetition_takes_the_rows_the_other_components_leave() {
    for count in [GridRepeatCount::AutoFill, GridRepeatCount::AutoFit] {
        let rows = template(vec![C::Size(FIXED), repeat(count, vec![AUTO])]);
        assert_eq!(flags(rows, vec![], counts(0, 3, 0)), [false, true, true]);
    }
}

#[test]
fn the_template_is_cut_at_the_explicit_rows() {
    let rows = template(vec![C::Size(FIXED), C::Size(FIXED), C::Size(AUTO)]);
    assert_eq!(flags(rows, vec![], counts(0, 2, 0)), [false, false]);
}

#[test]
fn explicit_rows_the_template_does_not_reach_are_sized_as_implicit_rows() {
    let rows = template(vec![C::Size(FIXED)]);
    assert_eq!(
        flags(rows, vec![], counts(0, 3, 1)),
        [false, true, true, true]
    );
}

#[test]
fn implicit_rows_cycle_through_grid_auto_rows() {
    let auto_rows = || vec![FIXED, AUTO];
    // The rows before the explicit grid end with the last of `grid-auto-rows`.
    assert_eq!(
        flags(Template::None, auto_rows(), counts(3, 0, 0)),
        [true, false, true]
    );
    // The rows after it start with the first.
    assert_eq!(
        flags(Template::None, auto_rows(), counts(0, 0, 3)),
        [false, true, false]
    );
}

#[test]
fn a_range_past_the_last_row_is_sized_by_neither() {
    with_view(Template::None, vec![], |view| {
        let sizing = RowSizing::of(view, counts(0, 0, 2));
        assert!(sizing.any_sized_by_content_or_free_space(1..2));
        assert!(!sizing.any_sized_by_content_or_free_space(1..3));
    });
}

#[test]
fn rows_size_to_content_when_every_row_track_is_intrinsic_and_uncapped() {
    let check =
        |rows: Template, auto_rows: Vec<S>| with_view(rows, auto_rows, rows_size_to_content);
    // No tracks at all: every implicit row is `auto`.
    assert!(check(Template::None, vec![]));
    assert!(check(template(vec![C::Size(AUTO)]), vec![]));
    assert!(check(
        template(vec![
            C::Size(S::Breadth(B::MinContent)),
            C::Size(S::Breadth(B::MaxContent))
        ]),
        vec![]
    ));
    assert!(check(
        template(vec![repeat(GridRepeatCount::Count(2), vec![AUTO])]),
        vec![]
    ));
    assert!(check(Template::None, vec![AUTO]));
    // A length, a flexible track or a capped one holds the row to a size.
    assert!(!check(template(vec![C::Size(FIXED)]), vec![]));
    assert!(!check(
        template(vec![C::Size(S::Breadth(B::Flex(1.0)))]),
        vec![]
    ));
    assert!(!check(
        template(vec![C::Size(S::FitContent(LP::Px(30.0)))]),
        vec![]
    ));
    assert!(!check(
        template(vec![
            C::Size(AUTO),
            repeat(GridRepeatCount::Count(2), vec![FIXED])
        ]),
        vec![]
    ));
    assert!(!check(Template::None, vec![AUTO, FIXED]));
}
