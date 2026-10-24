use super::*;

#[test]
fn empty_templates_match_the_legacy_style_iterator_presence() {
    let values = ComputedValues::initial();
    let prepared = PreparedStyle::prepare(&values, StyleMetadata::default());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let legacy = taffy::Style::<raikiri_style::SmolStr>::default();
    assert_eq!(
        view.grid_template_columns().is_some(),
        legacy.grid_template_columns().is_some()
    );
    assert_eq!(
        view.grid_template_rows().is_some(),
        legacy.grid_template_rows().is_some()
    );
    assert_eq!(
        view.grid_template_column_names().is_some(),
        legacy.grid_template_column_names().is_some()
    );
    assert_eq!(
        view.grid_template_row_names().is_some(),
        legacy.grid_template_row_names().is_some()
    );
    assert_eq!(view.grid_template_columns().unwrap().len(), 0);
    assert_eq!(view.grid_template_column_names().unwrap().len(), 0);
}
use raikiri_style::{
    ComputedGridTemplateTracks, ComputedGridTrackBreadth as B, ComputedGridTrackList,
    ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
    SmolStr, property::*,
};
use std::sync::Arc;
use taffy::{GenericGridTemplateComponent as T, GenericRepetition, GridContainerStyle};

fn resolve(pointer: *const (), basis: f32) -> f32 {
    CalcRef::from_handle(pointer).resolve(basis)
}

#[test]
fn grid_track_calc_slots_preserve_repeat_axis_and_range() {
    let expr = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let payload = AlignedCalc::new(expr, true);
    let max_expr = CalcLengthPercentage {
        px: 20.0,
        percent: 50.0,
    };
    let max_payload = AlignedCalc::new(max_expr, true);
    let mut cv = ComputedValues::initial();
    cv.grid_template_rows = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![], vec![]],
        components: vec![C::Repeat(ComputedGridTrackRepeat {
            count: GridRepeatCount::AutoFit,
            line_names: vec![vec![], vec![]],
            tracks: vec![S::MinMax(B::Calc(expr), B::Calc(max_expr))],
        })],
    }));
    cv.grid_auto_columns = Arc::new(vec![S::Breadth(B::Calc(expr))]);
    let slots = [
        CalcSlot::Grid {
            axis: Axis::Y,
            list: GridList::Template,
            component: 0,
            repeat_track: Some(0),
            part: TrackPart::Min,
        },
        CalcSlot::Grid {
            axis: Axis::Y,
            list: GridList::Template,
            component: 0,
            repeat_track: Some(0),
            part: TrackPart::Max,
        },
        CalcSlot::Grid {
            axis: Axis::X,
            list: GridList::Auto,
            component: 0,
            repeat_track: None,
            part: TrackPart::Min,
        },
        CalcSlot::Grid {
            axis: Axis::X,
            list: GridList::Auto,
            component: 0,
            repeat_track: None,
            part: TrackPart::Max,
        },
    ];
    let bindings = slots.map(|slot| PreparedCalc {
        slot,
        payload: if slot == slots[1] {
            &max_payload
        } else {
            &payload
        },
    });
    let mut swapped = bindings;
    swapped[0].payload = &max_payload;
    swapped[1].payload = &payload;
    assert_eq!(
        PreparedStyle::new(&cv, StyleMetadata::default(), &swapped).unwrap_err(),
        PrepareError::Mismatch(slots[0])
    );
    assert!(PreparedStyle::new(&cv, StyleMetadata::default(), &bindings[..3]).is_err());
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &bindings).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    let mut rows = v.grid_template_rows().unwrap();
    let T::Repeat(repeat) = rows.next().unwrap() else {
        panic!("repeat")
    };
    assert_eq!(repeat.count(), taffy::RepetitionCount::AutoFit);
    let track = repeat.tracks().next().unwrap();
    let auto = v.grid_auto_columns().next().unwrap();
    for basis in [0.0, 100.0, 200.0] {
        let expected = (basis * 0.25 - 10.0_f32).max(0.0);
        assert_eq!(
            track.min.definite_value(Some(basis), resolve),
            Some(expected)
        );
        assert_eq!(
            track.max.definite_value(Some(basis), resolve),
            Some(basis * 0.5 + 20.0)
        );
        assert_eq!(
            auto.max.definite_value(Some(basis), resolve),
            Some(expected)
        );
    }
}

#[test]
fn grid_track_and_placement_variants_match_bridge_values() {
    let sizes = vec![
        S::Breadth(B::Px(12.0)),
        S::Breadth(B::Percent(25.0)),
        S::Breadth(B::Flex(2.0)),
        S::Breadth(B::MinContent),
        S::Breadth(B::MaxContent),
        S::Breadth(B::Auto),
        S::MinMax(B::Px(5.0), B::Flex(3.0)),
        S::FitContent(raikiri_style::ComputedLengthPercentage::Px(40.0)),
        S::FitContent(raikiri_style::ComputedLengthPercentage::Percent(35.0)),
    ];
    let mut cv = ComputedValues::initial();
    cv.grid_auto_rows = Arc::new(sizes);
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    let tracks = v.grid_auto_rows().collect::<Vec<_>>();
    assert_eq!(tracks.len(), 9);
    assert_eq!(tracks[0].min, taffy::MinTrackSizingFunction::length(12.0));
    assert_eq!(tracks[1].max, taffy::MaxTrackSizingFunction::percent(0.25));
    assert_eq!(tracks[2].min, taffy::MinTrackSizingFunction::auto());
    assert_eq!(tracks[2].max, taffy::MaxTrackSizingFunction::fr(2.0));
    assert_eq!(
        tracks[7].max,
        taffy::MaxTrackSizingFunction::fit_content_px(40.0)
    );
    assert_eq!(
        tracks[8].max,
        taffy::MaxTrackSizingFunction::fit_content_percent(0.35)
    );
    for (input, expected) in [
        (GridAutoFlowValue::Row, taffy::GridAutoFlow::Row),
        (GridAutoFlowValue::Column, taffy::GridAutoFlow::Column),
        (GridAutoFlowValue::RowDense, taffy::GridAutoFlow::RowDense),
        (
            GridAutoFlowValue::ColumnDense,
            taffy::GridAutoFlow::ColumnDense,
        ),
    ] {
        cv.grid_auto_flow = input;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        assert_eq!(v.grid_auto_flow(), expected);
    }
    for (input, expected) in [
        (GridLineValue::Auto, taffy::GridPlacement::Auto),
        (
            GridLineValue::Line(i32::MIN),
            taffy::style_helpers::line(i16::MIN),
        ),
        (
            GridLineValue::Line(i32::MAX),
            taffy::style_helpers::line(i16::MAX),
        ),
        (
            GridLineValue::Span(u32::MAX),
            taffy::GridPlacement::Span(u16::MAX),
        ),
        (
            GridLineValue::NamedLine(SmolStr::new("x"), i32::MIN),
            taffy::GridPlacement::NamedLine(SmolStr::new("x"), i16::MIN),
        ),
        (
            GridLineValue::SpanNamed(SmolStr::new("x"), u32::MAX),
            taffy::GridPlacement::NamedSpan(SmolStr::new("x"), u16::MAX),
        ),
    ] {
        cv.grid_row_start = input;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        assert_eq!(v.grid_row().start, expected);
    }
}

#[test]
fn grid_iterators_borrow_names_and_preserve_cursor() {
    let name = SmolStr::new("a-long-line-name-that-needs-shared-heap-storage");
    let repeat = ComputedGridTrackRepeat {
        count: GridRepeatCount::Count(2),
        line_names: vec![vec![name.clone()], vec![]],
        tracks: vec![S::Breadth(B::Percent(25.0))],
    };
    let mut cv = ComputedValues::initial();
    cv.grid_template_columns = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![name.clone()], vec![], vec![name.clone()]],
        components: vec![C::Size(S::Breadth(B::Px(10.0))), C::Repeat(repeat)],
    }));
    cv.grid_column_start = GridLineValue::Named(name.clone());
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    let mut tracks = v.grid_template_columns().unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(matches!(tracks.next(), Some(T::Single(_))));
    let mut cloned = tracks.clone();
    assert_eq!(cloned.len(), 1);
    for iter in [&mut tracks, &mut cloned] {
        let T::Repeat(repeat) = iter.next().unwrap() else {
            panic!("expected repeat")
        };
        assert_eq!(repeat.count(), taffy::RepetitionCount::Count(2));
        assert_eq!(repeat.tracks().len(), 1);
        let mut names = repeat.lines_names();
        let bound = names.next().unwrap().next().unwrap();
        assert_eq!(bound.as_ptr(), name.as_ptr());
        assert_eq!(names.len(), 1);
        assert_eq!(iter.len(), 0);
    }
    let placement = v.grid_column();
    let taffy::GridPlacement::NamedLine(bound, 0) = placement.start else {
        panic!("expected named line")
    };
    assert_eq!(bound.as_ptr(), name.as_ptr());
    assert_eq!(
        v.grid_template_column_names()
            .unwrap()
            .next()
            .unwrap()
            .next()
            .unwrap()
            .as_ptr(),
        name.as_ptr()
    );
}

#[test]
fn unnamed_area_dimensions_are_preserved() {
    let mut cv = ComputedValues::initial();
    cv.grid_template_areas = GridTemplateAreasValue::Areas(Arc::new(GridTemplateAreas {
        row_strings: vec![],
        areas: vec![],
        row_count: 2,
        column_count: 3,
    }));
    let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
    let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert_eq!(v.grid_template_area_row_count(), 2);
    assert_eq!(v.grid_template_area_column_count(), 3);
    assert_eq!(v.grid_template_areas().unwrap().count(), 0);
}

#[test]
fn repeat_counts_and_area_coordinates_saturate_like_the_bridge() {
    for (count, expected) in [
        (GridRepeatCount::Count(1), taffy::RepetitionCount::Count(1)),
        (
            GridRepeatCount::Count(u32::MAX),
            taffy::RepetitionCount::Count(u16::MAX),
        ),
        (GridRepeatCount::AutoFill, taffy::RepetitionCount::AutoFill),
        (GridRepeatCount::AutoFit, taffy::RepetitionCount::AutoFit),
    ] {
        let mut cv = ComputedValues::initial();
        cv.grid_template_rows = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
            line_names: vec![],
            components: vec![C::Repeat(ComputedGridTrackRepeat {
                count,
                line_names: vec![],
                tracks: vec![S::Breadth(B::Auto)],
            })],
        }));
        cv.grid_template_areas = GridTemplateAreasValue::Areas(Arc::new(GridTemplateAreas {
            row_strings: vec![],
            row_count: u32::MAX,
            column_count: u32::MAX,
            areas: vec![GridTemplateAreaEntry {
                name: SmolStr::new("area"),
                row_start: 1,
                row_end: u32::MAX,
                column_start: 1,
                column_end: u32::MAX,
            }],
        }));
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        let mut tracks = v.grid_template_rows().unwrap();
        let T::Repeat(repeat) = tracks.next().unwrap() else {
            panic!("repeat")
        };
        assert_eq!(repeat.count(), expected);
        let mut areas = v.grid_template_areas().unwrap();
        assert_eq!(areas.size_hint(), (1, Some(1)));
        let area = areas.next().unwrap();
        assert_eq!(area.row_end, u16::MAX);
        assert_eq!(area.column_end, u16::MAX);
        assert_eq!(v.grid_template_area_row_count(), u16::MAX);
        assert_eq!(v.grid_template_area_column_count(), u16::MAX);
    }
}

#[test]
fn bare_named_placement_keeps_area_lookup_distinct_from_explicit_one() {
    let name = SmolStr::new("area");
    for (value, expected) in [
        (GridLineValue::Named(name.clone()), 0),
        (GridLineValue::NamedLine(name.clone(), 1), 1),
    ] {
        let mut cv = ComputedValues::initial();
        cv.grid_row_start = value.clone();
        cv.grid_row_end = value.clone();
        cv.grid_column_start = value.clone();
        cv.grid_column_end = value;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let v = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        for placement in [v.grid_row(), v.grid_column()] {
            for edge in [placement.start, placement.end] {
                let taffy::GridPlacement::NamedLine(actual, index) = edge else {
                    panic!("expected named placement")
                };
                assert_eq!(actual, name);
                assert_eq!(index, expected);
            }
        }
    }
}

#[test]
fn a_box_has_grid_placement_when_any_of_its_four_lines_is_not_auto() {
    let placed = |set: fn(&mut ComputedValues, GridLineValue)| {
        let mut cv = ComputedValues::initial();
        set(&mut cv, GridLineValue::Line(2));
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        LayoutStyleView::new(&p, ViewContext::default(), &[], &[]).has_grid_placement()
    };
    let initial = ComputedValues::initial();
    let p = PreparedStyle::new(&initial, StyleMetadata::default(), &[]).unwrap();
    let view = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
    assert!(!view.has_grid_placement());
    assert!(placed(|cv, line| cv.grid_row_start = line));
    assert!(placed(|cv, line| cv.grid_row_end = line));
    assert!(placed(|cv, line| cv.grid_column_start = line));
    assert!(placed(|cv, line| cv.grid_column_end = line));
    for line in [
        GridLineValue::Span(2),
        GridLineValue::Named(SmolStr::new("area")),
    ] {
        let mut cv = ComputedValues::initial();
        cv.grid_column_end = line;
        let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
        let view = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
        assert!(view.has_grid_placement());
    }
}

#[test]
fn has_grid_placement_agrees_with_the_projected_placements() {
    let lines = [
        GridLineValue::Auto,
        GridLineValue::Line(0),
        GridLineValue::Line(2),
        GridLineValue::Line(-1),
        GridLineValue::Named(SmolStr::new("area")),
        GridLineValue::NamedLine(SmolStr::new("area"), 2),
        GridLineValue::Span(0),
        GridLineValue::Span(2),
        GridLineValue::SpanNamed(SmolStr::new("area"), 2),
    ];
    let setters: [fn(&mut ComputedValues, GridLineValue); 4] = [
        |cv, line| cv.grid_row_start = line,
        |cv, line| cv.grid_row_end = line,
        |cv, line| cv.grid_column_start = line,
        |cv, line| cv.grid_column_end = line,
    ];
    for line in lines {
        for set in setters {
            let mut cv = ComputedValues::initial();
            set(&mut cv, line.clone());
            let p = PreparedStyle::new(&cv, StyleMetadata::default(), &[]).unwrap();
            let view = LayoutStyleView::new(&p, ViewContext::default(), &[], &[]);
            let (row, column) = (view.grid_row(), view.grid_column());
            let projected = [row.start, row.end, column.start, column.end]
                .iter()
                .any(|placement| !matches!(placement, taffy::GridPlacement::Auto));
            assert_eq!(view.has_grid_placement(), projected, "{line:?}");
        }
    }
}

#[test]
fn shared_values_and_grid_payloads_survive_external_owner_drop() {
    use raikiri_style::{
        ComputedGridTemplateTracks, ComputedGridTrackBreadth as B, ComputedGridTrackList,
        ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
        property::GridRepeatCount,
    };
    let mut cv = ComputedValues::initial();
    cv.grid_template_columns = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![], vec![]],
        components: vec![C::Repeat(ComputedGridTrackRepeat {
            count: GridRepeatCount::Count(2),
            line_names: vec![vec![], vec![]],
            tracks: vec![S::MinMax(
                B::Calc(CalcLengthPercentage {
                    px: -5.0,
                    percent: 20.0,
                }),
                B::Calc(CalcLengthPercentage {
                    px: 7.0,
                    percent: 40.0,
                }),
            )],
        })],
    }));
    let owner = Arc::new(cv);
    let weak = Arc::downgrade(&owner);
    let prepared = PreparedStyle::prepare_shared(owner.clone(), StyleMetadata::default());
    drop(owner);
    assert!(weak.upgrade().is_some());
    let view = LayoutStyleView::new(&prepared, ViewContext::default(), &[], &[]);
    let component = view.grid_template_columns().unwrap().next().unwrap();
    let taffy::GenericGridTemplateComponent::Repeat(repeat) = component else {
        panic!("repeat");
    };
    assert_eq!(repeat.count(), taffy::RepetitionCount::Count(2));
    assert_eq!(repeat.tracks().count(), 1);
    let slot = |part| CalcSlot::Grid {
        axis: Axis::X,
        list: GridList::Template,
        component: 0,
        repeat_track: Some(0),
        part,
    };
    assert_eq!(prepared.payload(slot(TrackPart::Min)).resolve(0.0), 0.0);
    assert_eq!(prepared.payload(slot(TrackPart::Min)).resolve(100.0), 15.0);
    assert_eq!(prepared.payload(slot(TrackPart::Max)).resolve(100.0), 47.0);
    drop(prepared);
    assert!(weak.upgrade().is_none());
}
