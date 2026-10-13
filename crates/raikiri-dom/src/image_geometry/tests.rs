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
fn calc_positions_resolve_from_the_start_edge() {
    let calc = raikiri_style::property::CalcLengthPercentage {
        percent: 50.0,
        px: -10.0,
    };
    assert_eq!(
        position_offset(ComputedCssPositionOffset::Calc(calc), 200.0),
        90.0
    );
    // Positions are not clamped.
    let calc = raikiri_style::property::CalcLengthPercentage {
        percent: 10.0,
        px: -50.0,
    };
    assert_eq!(
        position_offset(ComputedCssPositionOffset::Calc(calc), 200.0),
        -30.0
    );
}

#[test]
fn calc_background_sizes_resolve_against_the_area_and_clamp_at_zero() {
    let calc = |percent, px| {
        ComputedLengthPercentageOrAuto::Calc(raikiri_style::property::CalcLengthPercentage {
            percent,
            px,
        })
    };
    let natural = ImageIntrinsicSize {
        width: Some(10.0),
        height: Some(10.0),
        aspect_ratio: Some(1.0),
    };
    let size = ComputedBackgroundSize::Explicit {
        width: calc(50.0, -10.0),
        height: calc(25.0, 5.0),
    };
    assert_eq!(
        background_image_dimensions(&size, 200.0, 100.0, natural),
        Some((90.0, 30.0))
    );
    let size = ComputedBackgroundSize::Explicit {
        width: calc(10.0, -50.0),
        height: calc(25.0, 5.0),
    };
    assert_eq!(
        background_image_dimensions(&size, 200.0, 100.0, natural),
        None
    );
}

#[test]
fn background_axis_helpers_reject_non_finite_edges() {
    let computed = raikiri_style::ComputedValues::initial();
    let position = &computed.background_layers()[0].position;
    let repeat = &computed.background_layers()[0].repeat;
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
        &computed.background_layers()[0].position,
        &computed.background_layers()[0].repeat,
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
            &computed.background_layers()[0].position,
            &computed.background_layers()[0].repeat,
        )
        .is_none()
    );
}

#[test]
fn background_dimensions_fill_missing_natural_sizes() {
    let auto = raikiri_style::ComputedValues::initial().background_layers()[0].size;
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
            &computed.background_layers()[0].position,
            &computed.background_layers()[0].repeat,
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
    let mut repeat = computed.background_layers()[0].repeat;
    repeat.x = BackgroundRepeatKeyword::NoRepeat;
    repeat.y = BackgroundRepeatKeyword::Space;
    let tiles = |image_h| {
        background_tiles(
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 100.0, 100.0),
            (30.0, image_h),
            &computed.background_layers()[0].position,
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
        &computed.background_layers()[0].position.vertical,
        &BackgroundRepeatKeyword::Space,
        None,
    );
    assert_eq!(empty, Some(vec![0.0]));
}

#[test]
fn round_tiles_bound_degenerate_counts() {
    let position = raikiri_style::ComputedValues::initial().background_layers()[0].position;
    assert_eq!(
        round_axis_tiles(100_000.0, 1.0, BackgroundRepeatKeyword::Round),
        (10.0, Some(10_000))
    );
    // Capped tiles still cover the painting area.
    let mut repeat = raikiri_style::ComputedValues::initial().background_layers()[0].repeat;
    repeat.x = BackgroundRepeatKeyword::Round;
    let tiles = background_tiles(
        (0.0, 0.0, 20_000.0, 10.0),
        (0.0, 0.0, 20_000.0, 10.0),
        (1.0, 10.0),
        &position,
        &repeat,
    )
    .expect("round tiles");
    assert_eq!(tiles.x.len(), 10_000);
    assert_eq!(tiles.width, 2.0);
    // A painting area beyond the tile bound keeps the positioning tiles.
    let tiles = background_tiles(
        (0.0, 0.0, 10_000.0, 10.0),
        (-20_000.0, 0.0, 40_000.0, 10.0),
        (1.0, 10.0),
        &position,
        &repeat,
    )
    .expect("round tiles");
    assert_eq!(tiles.x.len(), 10_000);
    assert_eq!(tiles.x[0], 0.0);
    // Two `round` axes share the tile budget.
    let mut both = repeat;
    both.y = BackgroundRepeatKeyword::Round;
    let tiles = background_tiles(
        (0.0, 0.0, 20_000.0, 20_000.0),
        (0.0, 0.0, 20_000.0, 20_000.0),
        (1.0, 1.0),
        &position,
        &both,
    )
    .expect("round tiles");
    assert!(tiles.x.len() * tiles.y.len() <= 1 << 22);
    assert_eq!(tiles.x.len(), 2_048);
    // Offset tiles that spill past both edges still fit the budget.
    let mut shifted = position;
    shifted.horizontal = ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(0.5));
    shifted.vertical = ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(0.5));
    let tiles = background_tiles(
        (0.0, 0.0, 2_048.0, 2_048.0),
        (0.0, 0.0, 2_048.0, 2_048.0),
        (1.0, 1.0),
        &shifted,
        &both,
    )
    .expect("round tiles");
    assert!(tiles.x.len() * tiles.y.len() <= 1 << 22);
    // One `round` axis gives up tiles to a repeated one.
    let mut mixed = both;
    mixed.y = BackgroundRepeatKeyword::Repeat;
    let tiles = background_tiles(
        (0.0, 0.0, 20_000.0, 2_000.0),
        (0.0, 0.0, 20_000.0, 2_000.0),
        (1.0, 1.0),
        &position,
        &mixed,
    )
    .expect("round tiles");
    assert_eq!(tiles.y.len(), 2_000);
    assert!(tiles.x.len() * tiles.y.len() <= 1 << 22);
    let mut repeated = both;
    repeated.x = BackgroundRepeatKeyword::Repeat;
    repeated.y = BackgroundRepeatKeyword::Repeat;
    // One-pixel tiles over a page-sized box are all placed.
    let tiles = background_tiles(
        (0.0, 0.0, 794.0, 1_123.0),
        (0.0, 0.0, 794.0, 1_123.0),
        (1.0, 1.0),
        &position,
        &repeated,
    )
    .expect("repeated tiles");
    assert_eq!((tiles.x.len(), tiles.y.len()), (794, 1_123));
    // A vertical `round` axis gives up tiles the same way.
    let mut vertical = repeated;
    vertical.y = BackgroundRepeatKeyword::Round;
    let tiles = background_tiles(
        (0.0, 0.0, 2_000.0, 20_000.0),
        (0.0, 0.0, 2_000.0, 20_000.0),
        (1.0, 1.0),
        &position,
        &vertical,
    )
    .expect("round tiles");
    assert_eq!(tiles.x.len(), 2_000);
    assert!(tiles.x.len() * tiles.y.len() <= 1 << 22);
    // Repeated tiny tiles over a far larger area paint nothing.
    assert!(
        background_tiles(
            (0.0, 0.0, 9_000.0, 9_000.0),
            (0.0, 0.0, 9_000.0, 9_000.0),
            (1.0, 1.0),
            &position,
            &repeated,
        )
        .is_none()
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

#[test]
fn round_tiles_follow_position_and_space_tiles_cover_the_painting_area() {
    let computed = raikiri_style::ComputedValues::initial();
    let mut position = computed.background_layers()[0].position;
    position.horizontal = ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(5.0));
    // A rescaled `round` tile is placed by `background-position` and repeats
    // over the painting area like `repeat`.
    let round = axis_origins(
        1.0,
        72.0,
        0.0,
        74.0,
        36.0,
        32.0,
        &position.horizontal,
        &BackgroundRepeatKeyword::Round,
        Some(2),
    );
    assert_eq!(round, Some(vec![-30.0, 6.0, 42.0]));
    // Spaced tiles keep their pitch over a painting area larger than the
    // positioning area.
    let space = axis_origins(
        35.0,
        65.0,
        0.0,
        135.0,
        30.0,
        30.0,
        &computed.background_layers()[0].position.horizontal,
        &BackgroundRepeatKeyword::Space,
        None,
    );
    assert_eq!(space, Some(vec![0.0, 35.0, 70.0, 105.0]));
}
