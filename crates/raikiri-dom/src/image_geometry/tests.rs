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
