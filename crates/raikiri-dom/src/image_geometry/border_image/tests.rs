use super::*;

fn natural(width: f32, height: f32) -> Option<ImageIntrinsicSize> {
    Some(ImageIntrinsicSize {
        width: Some(width),
        height: Some(height),
        aspect_ratio: None,
    })
}

/// `border-image: <81×81 image> <slice> / <width> / <outset> <repeat>` with
/// numbers everywhere.
fn image(slice: f32, repeat: BorderImageRepeatKeyword) -> ComputedBorderImage {
    let mut image = ComputedBorderImage::initial();
    image.slice.offsets = Sides::all(BorderImageSliceOffset::Number(slice));
    image.repeat.horizontal = repeat;
    image.repeat.vertical = repeat;
    image
}

fn part(geometry: &BorderImageGeometry, area: Rect) -> &BorderImagePart {
    geometry
        .parts
        .iter()
        .find(|part| part.area == area)
        .unwrap_or_else(|| panic!("no part at {area:?} in {geometry:#?}"))
}

#[test]
fn nine_slices_map_to_the_border_areas() {
    let mut image = image(27.0, BorderImageRepeatKeyword::Stretch);
    image.slice.fill = true;
    let geometry = border_image_geometry(
        (10.0, 20.0, 100.0, 60.0),
        Sides::all(27.0),
        &image,
        natural(81.0, 81.0),
    )
    .expect("geometry");
    assert_eq!(geometry.image_size, (81.0, 81.0));
    assert_eq!(geometry.parts.len(), 9);
    let corner = part(&geometry, (10.0, 20.0, 27.0, 27.0));
    assert_eq!(corner.source, (0.0, 0.0, 27.0, 27.0));
    assert_eq!(
        (corner.x.clone(), corner.y.clone()),
        (vec![10.0], vec![20.0])
    );
    let top = part(&geometry, (37.0, 20.0, 46.0, 27.0));
    assert_eq!(top.source, (27.0, 0.0, 27.0, 27.0));
    assert_eq!((top.tile_width, top.tile_height), (46.0, 27.0));
    let bottom_right = part(&geometry, (83.0, 53.0, 27.0, 27.0));
    assert_eq!(bottom_right.source, (54.0, 54.0, 27.0, 27.0));
    let middle = part(&geometry, (37.0, 47.0, 46.0, 6.0));
    assert_eq!(middle.source, (27.0, 27.0, 27.0, 27.0));
}

#[test]
fn the_middle_is_left_out_without_fill() {
    let geometry = border_image_geometry(
        (0.0, 0.0, 90.0, 90.0),
        Sides::all(27.0),
        &image(27.0, BorderImageRepeatKeyword::Stretch),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    assert_eq!(geometry.parts.len(), 8);
}

#[test]
fn space_distributes_the_leftover_around_whole_tiles() {
    // 35px between the corners holds one 27px tile with 4px on each side.
    let geometry = border_image_geometry(
        (0.0, 0.0, 89.0, 89.0),
        Sides::all(27.0),
        &image(27.0, BorderImageRepeatKeyword::Space),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    let top = part(&geometry, (27.0, 0.0, 35.0, 27.0));
    assert_eq!(top.x, vec![31.0]);
    assert_eq!(top.tile_width, 27.0);
    let left = part(&geometry, (0.0, 27.0, 27.0, 35.0));
    assert_eq!(left.y, vec![31.0]);
}

#[test]
fn space_leaves_a_part_empty_when_no_tile_fits() {
    let geometry = border_image_geometry(
        (0.0, 0.0, 74.0, 74.0),
        Sides::all(27.0),
        &image(27.0, BorderImageRepeatKeyword::Space),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    assert_eq!(geometry.parts.len(), 4, "only the corners");
}

#[test]
fn repeat_centers_the_tiles() {
    let geometry = border_image_geometry(
        (0.0, 0.0, 94.0, 94.0),
        Sides::all(27.0),
        &image(27.0, BorderImageRepeatKeyword::Repeat),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    // 40px: a centered tile at 33.5 and partial tiles on both sides.
    let top = part(&geometry, (27.0, 0.0, 40.0, 27.0));
    assert_eq!(top.x, vec![6.5, 33.5, 60.5]);
}

#[test]
fn round_resizes_tiles_to_a_whole_number() {
    let geometry = border_image_geometry(
        (0.0, 0.0, 124.0, 124.0),
        Sides::all(27.0),
        &image(27.0, BorderImageRepeatKeyword::Round),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    // 70px / 27px rounds to three tiles.
    let top = part(&geometry, (27.0, 0.0, 70.0, 27.0));
    assert_eq!(top.x.len(), 3);
    assert!((top.tile_width - 70.0 / 3.0).abs() < 1e-9);
    assert_eq!(top.tile_height, 27.0);
}

#[test]
fn edges_keep_the_slice_aspect_ratio_before_repeating() {
    // A 10px border shows the 27px slices at 10/27 scale.
    let geometry = border_image_geometry(
        (0.0, 0.0, 100.0, 100.0),
        Sides::all(10.0),
        &image(27.0, BorderImageRepeatKeyword::Repeat),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    let top = part(&geometry, (10.0, 0.0, 80.0, 10.0));
    assert!((top.tile_width - 10.0).abs() < 1e-9);
    let left = part(&geometry, (0.0, 10.0, 10.0, 80.0));
    assert!((left.tile_height - 10.0).abs() < 1e-9);
}

#[test]
fn outset_and_widths_resolve_against_the_border() {
    let mut image = image(27.0, BorderImageRepeatKeyword::Stretch);
    image.outset = Sides {
        top: BorderImageOutsetSide::Length(5.0),
        right: BorderImageOutsetSide::Number(1.0),
        bottom: BorderImageOutsetSide::Number(0.0),
        left: BorderImageOutsetSide::Number(0.0),
    };
    image.width = Sides {
        top: BorderImageWidthSide::LengthPercentage(ComputedLengthPercentage::Px(6.0)),
        right: BorderImageWidthSide::Number(2.0),
        bottom: BorderImageWidthSide::LengthPercentage(ComputedLengthPercentage::Percent(10.0)),
        left: BorderImageWidthSide::Auto,
    };
    let geometry = border_image_geometry(
        (0.0, 0.0, 100.0, 100.0),
        Sides::all(4.0),
        &image,
        natural(81.0, 81.0),
    )
    .expect("geometry");
    assert_eq!(geometry.area, (0.0, -5.0, 104.0, 105.0));
    // left: `auto` is the 27px slice; right: twice the 4px border.
    let top_left = part(&geometry, (0.0, -5.0, 27.0, 6.0));
    assert_eq!(top_left.source, (0.0, 0.0, 27.0, 27.0));
    part(&geometry, (96.0, -5.0, 8.0, 6.0));
    // bottom: 10% of the 105px area height.
    part(&geometry, (0.0, 89.5, 27.0, 10.5));
}

#[test]
fn overlapping_widths_scale_down_together() {
    let mut image = image(27.0, BorderImageRepeatKeyword::Stretch);
    image.width = Sides::all(BorderImageWidthSide::LengthPercentage(
        ComputedLengthPercentage::Px(40.0),
    ));
    let geometry = border_image_geometry(
        (0.0, 0.0, 60.0, 100.0),
        Sides::all(1.0),
        &image,
        natural(81.0, 81.0),
    )
    .expect("geometry");
    // 60 / 80 = 0.75: every width becomes 30px and the top edge is empty.
    part(&geometry, (0.0, 0.0, 30.0, 30.0));
    part(&geometry, (0.0, 30.0, 30.0, 40.0));
    assert_eq!(geometry.parts.len(), 6);
}

#[test]
fn an_image_without_a_natural_size_covers_the_area() {
    let mut image = image(0.0, BorderImageRepeatKeyword::Stretch);
    image.slice.offsets = Sides::all(BorderImageSliceOffset::Percent(25.0));
    image.width = Sides::all(BorderImageWidthSide::Auto);
    let geometry = border_image_geometry((0.0, 0.0, 80.0, 40.0), Sides::all(5.0), &image, None)
        .expect("geometry");
    assert_eq!(geometry.image_size, (80.0, 40.0));
    // `auto` falls back to the border width without a natural size.
    let top = part(&geometry, (5.0, 0.0, 70.0, 5.0));
    assert_eq!(top.source, (20.0, 0.0, 40.0, 10.0));
}

#[test]
fn slices_past_the_image_size_count_as_the_whole_image() {
    let geometry = border_image_geometry(
        (0.0, 0.0, 100.0, 100.0),
        Sides::all(10.0),
        &image(500.0, BorderImageRepeatKeyword::Stretch),
        natural(81.0, 81.0),
    )
    .expect("geometry");
    // Each corner shows the whole image; the edges are empty.
    assert_eq!(geometry.parts.len(), 4);
    assert_eq!(geometry.parts[0].source, (0.0, 0.0, 81.0, 81.0));
}

#[test]
fn zero_widths_draw_nothing() {
    assert_eq!(
        border_image_geometry(
            (0.0, 0.0, 100.0, 100.0),
            Sides::all(0.0),
            &image(27.0, BorderImageRepeatKeyword::Stretch),
            natural(81.0, 81.0),
        ),
        None
    );
}

#[test]
fn tiny_tiles_are_stretched_instead() {
    let mut image = image(27.0, BorderImageRepeatKeyword::Repeat);
    image.width = Sides::all(BorderImageWidthSide::LengthPercentage(
        ComputedLengthPercentage::Px(0.001),
    ));
    let geometry = border_image_geometry(
        (0.0, 0.0, 10_000.0, 10_000.0),
        Sides::all(1.0),
        &image,
        natural(81.0, 81.0),
    )
    .expect("geometry");
    assert!(geometry.parts.iter().all(|part| part.x.len() == 1));
}

#[test]
fn a_dense_filled_middle_is_stretched_instead() {
    // 27px slices drawn 0.25px wide make 0.25px middle tiles: about 4000
    // per axis, each within the per-axis bound.
    let mut image = image(27.0, BorderImageRepeatKeyword::Repeat);
    image.slice.fill = true;
    image.width = Sides::all(BorderImageWidthSide::LengthPercentage(
        ComputedLengthPercentage::Px(0.25),
    ));
    let geometry = border_image_geometry(
        (0.0, 0.0, 1000.0, 1000.0),
        Sides::all(1.0),
        &image,
        natural(81.0, 81.0),
    )
    .expect("geometry");
    let middle = part(&geometry, (0.25, 0.25, 999.5, 999.5));
    assert_eq!((middle.x.len(), middle.y.len()), (1, 1));
    assert!(
        geometry
            .parts
            .iter()
            .all(|part| part.x.len() * part.y.len() <= 65_536)
    );
}

#[test]
fn auto_widths_use_each_natural_dimension_on_its_own() {
    let mut image = image(10.0, BorderImageRepeatKeyword::Stretch);
    image.width = Sides::all(BorderImageWidthSide::Auto);
    // A natural width but no natural height, as some SVG images have.
    let natural = Some(ImageIntrinsicSize {
        width: Some(40.0),
        height: None,
        aspect_ratio: None,
    });
    let geometry = border_image_geometry((0.0, 0.0, 100.0, 80.0), Sides::all(4.0), &image, natural)
        .expect("geometry");
    // Left and right use the 10px slice; top and bottom fall back to the
    // 4px border.
    part(&geometry, (0.0, 0.0, 10.0, 4.0));
}
