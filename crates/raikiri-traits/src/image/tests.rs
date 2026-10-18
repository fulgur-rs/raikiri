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
