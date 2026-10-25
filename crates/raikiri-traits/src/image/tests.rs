use super::*;

struct EmptySource;
impl ImagePixelSource for EmptySource {
    fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
        None
    }
}

#[test]
fn image_pixel_source_is_object_safe() {
    fn _assert<T: ?Sized>() {}
    _assert::<dyn ImagePixelSource>();
}

#[test]
fn empty_source_returns_none() {
    let url = Url::parse("file:///tmp/x.png").unwrap();
    assert!(EmptySource.get_decoded(&url).is_none());
}

#[test]
fn a_source_has_no_vector_form_unless_it_says_so() {
    let url = Url::parse("file:///tmp/x.svg").unwrap();
    let request = ImageVectorRequest::new(ImageRasterSize {
        width: 3.0,
        height: 2.0,
    });
    assert_eq!(request.size.width, 3.0);
    assert!(EmptySource.get_vector(&url, &request).is_none());
}

/// A hand-encoded 2x1 RGBA8 PNG (red, green).
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240, 159,
    17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// Offset of the IDAT chunk's CRC in `TINY_PNG`.
const IDAT_CRC_AT: usize = 8 + 25 + 8 + 16;

#[test]
fn a_png_with_correct_crcs_is_borrowed_unchanged() {
    assert!(matches!(png_with_repaired_crcs(TINY_PNG), Cow::Borrowed(_)));
}

#[test]
fn a_bad_chunk_crc_is_rewritten_and_nothing_else_changes() {
    let mut corrupt = TINY_PNG.to_vec();
    corrupt[IDAT_CRC_AT] ^= 0xFF;
    let repaired = png_with_repaired_crcs(&corrupt);
    assert!(matches!(repaired, Cow::Owned(_)));
    assert_eq!(&*repaired, TINY_PNG);
}

#[test]
fn non_png_and_truncated_bytes_are_left_alone() {
    assert!(matches!(
        png_with_repaired_crcs(b"GIF89a"),
        Cow::Borrowed(b"GIF89a")
    ));
    let mut truncated = TINY_PNG[..IDAT_CRC_AT + 2].to_vec();
    truncated[IDAT_CRC_AT] ^= 0xFF;
    assert_eq!(&*png_with_repaired_crcs(&truncated), &truncated[..]);
    let huge_length = [&TINY_PNG[..8], &[0xFF, 0xFF, 0xFF, 0xFF], b"IHDR"].concat();
    assert_eq!(&*png_with_repaired_crcs(&huge_length), &huge_length[..]);
}

/// An sRGB profile whose red and green primaries trade places, so a pixel
/// stored as "red" is really sRGB green.
fn swapped_red_green_profile() -> Vec<u8> {
    let mut profile = moxcms::ColorProfile::new_srgb();
    std::mem::swap(&mut profile.red_colorant, &mut profile.green_colorant);
    profile.encode().unwrap()
}

#[test]
fn a_profile_with_swapped_primaries_swaps_red_and_green() {
    let mut pixels = vec![0x99, 0, 0, 255, 0, 0x99, 0, 255, 0, 0, 0x99, 255];
    assert!(convert_rgba8_to_srgb(
        &swapped_red_green_profile(),
        &mut pixels
    ));
    assert_eq!(pixels, [0, 0x99, 0, 255, 0x99, 0, 0, 255, 0, 0, 0x99, 255]);
}

#[test]
fn conversion_keeps_alpha_as_it_is() {
    let mut pixels = vec![0x99, 0, 0, 77, 0x99, 0, 0, 0];
    assert!(convert_rgba8_to_srgb(
        &swapped_red_green_profile(),
        &mut pixels
    ));
    assert_eq!([pixels[3], pixels[7]], [77, 0]);
}

#[test]
fn an_srgb_profile_leaves_the_pixels_byte_for_byte() {
    let profile = moxcms::ColorProfile::new_srgb().encode().unwrap();
    let original: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 200]).collect();
    let mut pixels = original.clone();
    assert!(!convert_rgba8_to_srgb(&profile, &mut pixels));
    assert_eq!(pixels, original);
}

#[test]
fn a_wide_gamut_profile_is_converted_to_srgb() {
    let profile = moxcms::ColorProfile::new_display_p3().encode().unwrap();
    // Display P3 (200, 100, 50) is a more saturated color than sRGB's
    // (200, 100, 50), so the sRGB values have to move apart.
    let mut pixels = vec![200, 100, 50, 255];
    assert!(convert_rgba8_to_srgb(&profile, &mut pixels));
    assert!(pixels[0] > 200 && pixels[2] < 50, "{pixels:?}");
    assert_eq!(pixels[3], 255);
}

#[test]
fn a_profile_that_cannot_be_read_leaves_the_pixels_alone() {
    let mut pixels = vec![0x99, 0, 0, 255];
    assert!(!convert_rgba8_to_srgb(b"not an icc profile", &mut pixels));
    assert_eq!(pixels, [0x99, 0, 0, 255]);
}

#[test]
fn a_profile_for_another_color_model_leaves_the_pixels_alone() {
    // Byte 16 of the header names the data color space.
    let mut profile = swapped_red_green_profile();
    profile[16..20].copy_from_slice(b"GRAY");
    let mut pixels = vec![0x99, 0, 0, 255];
    assert!(!convert_rgba8_to_srgb(&profile, &mut pixels));
    assert_eq!(pixels, [0x99, 0, 0, 255]);
}

#[test]
fn a_buffer_that_is_not_whole_pixels_is_left_alone() {
    let profile = swapped_red_green_profile();
    let mut partial = vec![0x99, 0, 0, 255, 1];
    assert!(!convert_rgba8_to_srgb(&profile, &mut partial));
    assert_eq!(partial, [0x99, 0, 0, 255, 1]);
    assert!(!convert_rgba8_to_srgb(&profile, &mut []));
}

#[test]
fn a_buffer_longer_than_one_pass_converts_all_of_it() {
    let profile = swapped_red_green_profile();
    // More than two passes of 16 Ki pixels each.
    let pixel = [0x99, 0, 0, 255];
    let mut pixels: Vec<u8> = pixel.iter().copied().cycle().take(4 * 40_000).collect();
    assert!(convert_rgba8_to_srgb(&profile, &mut pixels));
    let converted = [0, 0x99, 0, 255];
    assert!(pixels.chunks_exact(4).all(|chunk| chunk == converted));
}

/// An RGB profile that describes sRGB through a color lookup table instead of
/// matrix and curves: sRGB's own curves feed a 2×2×2 table of PCS XYZ values,
/// carried for each of `intents`. `nudge` adds that much XYZ to the table's
/// yellow corner and takes it from the magenta one, which moves the colors in
/// between but leaves every gray and every single-channel ramp where sRGB puts
/// it.
fn srgb_table_profile(nudge: f64, intents: &[moxcms::RenderingIntent]) -> Vec<u8> {
    use moxcms::{LutMultidimensionalType, LutStore, LutWarehouse, Matrix3d, Vector3d};

    let srgb = moxcms::ColorProfile::new_srgb();
    let mut clut = Vec::new();
    for corner in [
        [0, 0, 0],
        [0, 0, 1],
        [0, 1, 0],
        [0, 1, 1],
        [1, 0, 0],
        [1, 0, 1],
        [1, 1, 0],
        [1, 1, 1],
    ] {
        let [r, g, b] = corner.map(f64::from);
        let moved = match corner {
            [1, 1, 0] => nudge,
            [1, 0, 1] => -nudge,
            _ => 0.0,
        };
        let colorants = [srgb.red_colorant, srgb.green_colorant, srgb.blue_colorant];
        let xyz = [
            r * colorants[0].x + g * colorants[1].x + b * colorants[2].x,
            r * colorants[0].y + g * colorants[1].y + b * colorants[2].y,
            r * colorants[0].z + g * colorants[1].z + b * colorants[2].z,
        ];
        // PCS XYZ is stored with 1.0 at 0x8000.
        clut.extend(
            xyz.map(|value| ((value + moved) * 32768.0).round().clamp(0.0, 65535.0) as u16),
        );
    }
    let table = LutWarehouse::Multidimensional(LutMultidimensionalType {
        num_input_channels: 3,
        num_output_channels: 3,
        grid_points: [2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        clut: Some(LutStore::Store16(clut)),
        a_curves: vec![srgb.red_trc.clone().unwrap(); 3],
        b_curves: vec![moxcms::ToneReprCurve::Lut(vec![]); 3],
        m_curves: vec![],
        matrix: Matrix3d::IDENTITY,
        bias: Vector3d::default(),
    });

    let mut profile = srgb;
    profile.red_trc = None;
    profile.green_trc = None;
    profile.blue_trc = None;
    profile.cicp = None;
    for intent in intents {
        match intent {
            moxcms::RenderingIntent::Perceptual => {
                profile.lut_a_to_b_perceptual = Some(table.clone());
            }
            moxcms::RenderingIntent::Saturation => {
                profile.lut_a_to_b_saturation = Some(table.clone());
            }
            _ => profile.lut_a_to_b_colorimetric = Some(table.clone()),
        }
    }
    profile.encode().unwrap()
}

/// Colors for the table profile tests: a gray, a red, and a yellow between the
/// red and green ramps.
const GRAY_RED_YELLOW: [u8; 12] = [100, 100, 100, 255, 200, 0, 0, 255, 200, 200, 0, 255];

#[test]
fn a_table_profile_that_only_moves_mixed_colors_is_still_converted() {
    let profile = srgb_table_profile(0.1, &[moxcms::RenderingIntent::RelativeColorimetric]);
    let mut pixels = GRAY_RED_YELLOW;
    assert!(convert_rgba8_to_srgb(&profile, &mut pixels));
    // The gray and the red sit on ramps the table leaves alone.
    for (moved, stored) in pixels[..8].iter().zip(&GRAY_RED_YELLOW[..8]) {
        assert!(moved.abs_diff(*stored) <= 1, "{pixels:?}");
    }
    // The yellow does not.
    assert!(
        pixels[8..11]
            .iter()
            .zip(&GRAY_RED_YELLOW[8..11])
            .any(|(moved, stored)| moved.abs_diff(*stored) > 8),
        "{pixels:?}"
    );
}

#[test]
fn a_profile_with_a_table_for_one_other_intent_is_still_converted() {
    for intent in [
        moxcms::RenderingIntent::Perceptual,
        moxcms::RenderingIntent::Saturation,
    ] {
        let profile = srgb_table_profile(0.1, &[intent]);
        let mut pixels = GRAY_RED_YELLOW;
        assert!(convert_rgba8_to_srgb(&profile, &mut pixels), "{intent:?}");
        assert_ne!(pixels, GRAY_RED_YELLOW, "{intent:?}");
    }
}

#[test]
fn an_rgb_profile_with_neither_tables_nor_curves_leaves_the_pixels_alone() {
    let mut profile = moxcms::ColorProfile::new_srgb();
    profile.red_trc = None;
    profile.green_trc = None;
    profile.blue_trc = None;
    let profile = profile.encode().unwrap();
    let mut pixels = GRAY_RED_YELLOW;
    assert!(!convert_rgba8_to_srgb(&profile, &mut pixels));
    assert_eq!(pixels, GRAY_RED_YELLOW);
}
