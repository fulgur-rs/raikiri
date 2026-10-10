use super::*;

#[test]
fn absent_natural_dimensions_use_the_default_object_size_and_known_ratio() {
    let position = raikiri_style::ComputedValues::initial().object_position;
    for (width, height, ratio, expected) in [
        (None, None, None, (300.0, 150.0)),
        (None, None, Some(4.0), (300.0, 75.0)),
        (Some(50.0), None, Some(2.0), (50.0, 25.0)),
        (None, Some(20.0), Some(2.0), (40.0, 20.0)),
        (Some(50.0), None, None, (50.0, 150.0)),
        (None, Some(20.0), None, (300.0, 20.0)),
        (Some(10.0), Some(20.0), None, (10.0, 20.0)),
        (
            Some(f32::NAN),
            Some(-1.0),
            Some(f32::INFINITY),
            (300.0, 150.0),
        ),
    ] {
        let rect = object_image_rect(
            (0.0, 0.0, 400.0, 300.0),
            ImageIntrinsicSize {
                width,
                height,
                aspect_ratio: ratio,
            },
            ObjectFit::None,
            &position,
        )
        .unwrap();
        assert_eq!((rect.2, rect.3), expected);
        assert_eq!(rect.0, (400.0 - expected.0) / 2.0);
        assert_eq!(rect.1, (300.0 - expected.1) / 2.0);
    }
}

#[test]
fn empty_and_non_finite_content_boxes_have_no_object_rectangle() {
    let position = raikiri_style::ComputedValues::initial().object_position;
    let natural = ImageIntrinsicSize {
        width: Some(4.0),
        height: Some(2.0),
        aspect_ratio: None,
    };
    for content in [
        (0.0, 0.0, 0.0, 10.0),
        (0.0, 0.0, 10.0, -1.0),
        (f64::NAN, 0.0, 10.0, 10.0),
        (0.0, 0.0, f64::INFINITY, 10.0),
    ] {
        assert!(object_image_rect(content, natural, ObjectFit::Fill, &position).is_none());
    }
}

#[test]
fn end_percent_positions_resolve_from_the_end_edge() {
    assert_eq!(
        position_offset(
            ComputedCssPositionOffset::End(ComputedLengthPercentage::Percent(25.0)),
            100.0
        ),
        75.0
    );
}

#[test]
fn background_axis_helpers_reject_non_finite_edges() {
    let computed = raikiri_style::ComputedValues::initial();
    let position = &computed.background_position;
    let repeat = &computed.background_repeat;
    assert!(
        axis_origins(
            f64::INFINITY,
            100.0,
            0.0,
            100.0,
            20.0,
            20.0,
            &position.horizontal,
            &repeat.x,
            None,
        )
        .is_none()
    );
    assert!(
        axis_origins(
            0.0,
            100.0,
            0.0,
            f64::INFINITY,
            20.0,
            20.0,
            &position.horizontal,
            &repeat.x,
            None,
        )
        .is_none()
    );
    let (tile, count) = round_axis_tiles(0.0, 20.0, BackgroundRepeatKeyword::Round);
    assert_eq!((tile, count), (20.0, Some(1)));
}

#[test]
fn background_tiles_cover_the_painting_area() {
    let computed = raikiri_style::ComputedValues::initial();
    let tiles = background_tiles(
        (10.0, 10.0, 100.0, 50.0),
        (0.0, 0.0, 120.0, 70.0),
        (40.0, 30.0),
        &computed.background_position,
        &computed.background_repeat,
    )
    .expect("repeat tiles");
    assert_eq!((tiles.width, tiles.height), (40.0, 30.0));
    assert_eq!(tiles.x, vec![-30.0, 10.0, 50.0, 90.0]);
    assert_eq!(tiles.y, vec![-20.0, 10.0, 40.0]);
    assert!(
        background_tiles(
            (0.0, 0.0, 100.0, 50.0),
            (0.0, 0.0, 0.0, 50.0),
            (40.0, 30.0),
            &computed.background_position,
            &computed.background_repeat,
        )
        .is_none()
    );
}

#[test]
fn background_dimensions_fill_missing_natural_sizes() {
    let auto = raikiri_style::ComputedValues::initial().background_size;
    let size = |width, height, ratio, area: (f64, f64)| {
        background_image_dimensions(
            &auto,
            area.0,
            area.1,
            ImageIntrinsicSize {
                width,
                height,
                aspect_ratio: ratio,
            },
        )
    };
    let area = (400.0, 300.0);
    assert_eq!(size(Some(50.0), None, Some(2.0), area), Some((50.0, 25.0)));
    assert_eq!(size(None, Some(20.0), Some(2.0), area), Some((40.0, 20.0)));
    // A ratio alone fits the positioning area, or the default object size
    // when the area is empty.
    assert_eq!(size(None, None, Some(2.0), area), Some((400.0, 200.0)));
    assert_eq!(size(None, None, Some(4.0), (0.0, 0.0)), Some((300.0, 75.0)));
    assert_eq!(size(Some(50.0), None, None, area), Some((50.0, 150.0)));
    assert_eq!(size(None, Some(20.0), None, area), Some((300.0, 20.0)));
    assert_eq!(size(None, None, None, area), Some((300.0, 150.0)));
}

#[test]
fn background_tiles_reject_unusable_images_and_positions() {
    let computed = raikiri_style::ComputedValues::initial();
    let tiles = |positioning, image| {
        background_tiles(
            positioning,
            (0.0, 0.0, 100.0, 100.0),
            image,
            &computed.background_position,
            &computed.background_repeat,
        )
    };
    assert!(tiles((0.0, 0.0, 100.0, 100.0), (0.0, 10.0)).is_none());
    assert!(tiles((0.0, 0.0, 100.0, 100.0), (f64::NAN, 10.0)).is_none());
    assert!(tiles((0.0, 0.0, f64::INFINITY, 100.0), (10.0, 10.0)).is_none());
    assert!(tiles((0.0, f64::INFINITY, 100.0, 100.0), (10.0, 10.0)).is_none());
}

#[test]
fn background_tiles_follow_no_repeat_and_space() {
    let computed = raikiri_style::ComputedValues::initial();
    let mut repeat = computed.background_repeat;
    repeat.x = BackgroundRepeatKeyword::NoRepeat;
    repeat.y = BackgroundRepeatKeyword::Space;
    let tiles = |image_h| {
        background_tiles(
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 100.0, 100.0),
            (30.0, image_h),
            &computed.background_position,
            &repeat,
        )
        .expect("tiles")
    };
    let spaced = tiles(30.0);
    assert_eq!(spaced.x, vec![0.0]);
    assert_eq!(spaced.y, vec![0.0, 35.0, 70.0]);
    // One tile that does not fit twice is placed by `background-position`.
    assert_eq!(tiles(60.0).y, vec![0.0]);
    // An empty positioning area cannot space tiles either.
    let empty = axis_origins(
        0.0,
        0.0,
        0.0,
        100.0,
        30.0,
        30.0,
        &computed.background_position.vertical,
        &BackgroundRepeatKeyword::Space,
        None,
    );
    assert_eq!(empty, Some(vec![0.0]));
}

#[test]
fn round_tiles_bound_degenerate_counts() {
    let position = raikiri_style::ComputedValues::initial().background_position;
    assert_eq!(
        round_axis_tiles(100_000.0, 1.0, BackgroundRepeatKeyword::Round),
        (1.0, Some(1))
    );
    let origins = axis_origins(
        0.0,
        100.0,
        0.0,
        100.0,
        1.0,
        1.0,
        &position.horizontal,
        &BackgroundRepeatKeyword::Round,
        Some(20_000),
    );
    assert_eq!(origins, Some(vec![0.0]));
}
