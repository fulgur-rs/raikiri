use super::*;
use raikiri_style::{
    ComputedGridTemplateTracks, ComputedGridTrackBreadth as B, ComputedGridTrackList,
    ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
    SmolStr, property::*,
};
use std::sync::Arc;
use taffy::{
    GenericGridTemplateComponent as T, GenericRepetition, GridContainerStyle, GridItemStyle,
};

#[allow(unsafe_code)]
fn resolve(pointer: *const (), basis: f32) -> f32 {
    // SAFETY: Each test keeps its prepared payloads alive during resolution.
    unsafe { resolve_calc(pointer, basis) }
}

#[test]
fn grid_track_calc_slots_preserve_repeat_axis_and_range() {
    let expr = CalcLengthPercentage {
        px: -10.0,
        percent: 25.0,
    };
    let payload = AlignedCalc::new(expr, true);
    let mut cv = ComputedValues::initial();
    cv.grid_template_rows = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![], vec![]],
        components: vec![C::Repeat(ComputedGridTrackRepeat {
            count: GridRepeatCount::AutoFit,
            line_names: vec![vec![], vec![]],
            tracks: vec![S::MinMax(B::Calc(expr), B::Calc(expr))],
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
        payload: &payload,
    });
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
            Some(expected)
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
    let taffy::GridPlacement::NamedLine(bound, 1) = placement.start else {
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
