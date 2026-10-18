use super::*;
use std::io::Cursor;

/// A hand-encoded 2x1 RGBA8 PNG (red, green).
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 244, 34, 127, 138, 0, 0, 0, 16, 73, 68, 65, 84, 120, 156, 99, 249, 207, 192, 240, 159,
    17, 72, 0, 0, 16, 33, 3, 3, 30, 93, 32, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn decodes_raw_bytes_without_url_or_provider() {
    let decoded = ImageDecoder.decode(TINY_PNG, 32 * 1024 * 1024).unwrap();
    assert_eq!((decoded.width, decoded.height), (2, 1));
    assert_eq!(decoded.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
}

#[test]
fn rejects_invalid_image_bytes() {
    let error = ImageDecoder
        .decode(b"not an image", 32 * 1024 * 1024)
        .unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn normalizes_common_raster_formats_to_rgba8() {
    let source = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 255, 0]).unwrap();
    for format in [
        image::ImageFormat::Jpeg,
        image::ImageFormat::Gif,
        image::ImageFormat::WebP,
    ] {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(source.clone())
            .write_to(&mut encoded, format)
            .unwrap();
        let decoded = ImageDecoder
            .decode(encoded.get_ref(), 32 * 1024 * 1024)
            .unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.rgba.len(), 8);
        assert!(decoded.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}

#[test]
fn rejects_output_over_budget_before_normalization() {
    assert!(ImageDecoder.decode(TINY_PNG, 7).is_err());
    assert_eq!(ImageDecoder.decode(TINY_PNG, 8).unwrap().rgba.len(), 8);
}

#[test]
fn decodes_a_png_whose_chunk_crc_is_wrong() {
    let mut corrupt = TINY_PNG.to_vec();
    // The IDAT chunk's CRC follows the signature, IHDR, and IDAT's 16 data bytes.
    corrupt[8 + 25 + 8 + 16] ^= 0xFF;
    let decoded = ImageDecoder.decode(&corrupt, 32 * 1024 * 1024).unwrap();
    assert_eq!(decoded.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
}

/// A 3x2 PNG, red-green-blue over cyan-magenta-yellow, tagged with the EXIF
/// orientation `orientation`.
fn tagged_png(orientation: u16) -> Vec<u8> {
    use image::ImageEncoder as _;
    // Little-endian TIFF header, then one IFD entry: Orientation (0x0112),
    // type SHORT, one value.
    let mut exif = b"II*\0\x08\0\0\0".to_vec();
    exif.extend_from_slice(&1u16.to_le_bytes());
    exif.extend_from_slice(&0x0112u16.to_le_bytes());
    exif.extend_from_slice(&3u16.to_le_bytes());
    exif.extend_from_slice(&1u32.to_le_bytes());
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0, 0]);
    exif.extend_from_slice(&0u32.to_le_bytes());
    let mut encoded = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut encoded);
    encoder.set_exif_metadata(exif).unwrap();
    encoder
        .write_image(
            &[
                255, 0, 0, 0, 255, 0, 0, 0, 255, // red, green, blue
                0, 255, 255, 255, 0, 255, 255, 255, 0, // cyan, magenta, yellow
            ],
            3,
            2,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    encoded
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CYAN: [u8; 4] = [0, 255, 255, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

#[test]
fn turns_an_image_by_its_exif_orientation() {
    let decode = |orientation| {
        let decoded = ImageDecoder
            .decode(&tagged_png(orientation), 32 * 1024 * 1024)
            .unwrap();
        let pixels: Vec<[u8; 4]> = decoded
            .rgba
            .chunks_exact(4)
            .map(|pixel| pixel.try_into().unwrap())
            .collect();
        ((decoded.width, decoded.height), pixels)
    };
    // Stored as it is.
    assert_eq!(
        decode(1),
        ((3, 2), vec![RED, GREEN, BLUE, CYAN, MAGENTA, YELLOW])
    );
    // A quarter turn clockwise swaps the sides.
    assert_eq!(
        decode(6),
        ((2, 3), vec![CYAN, RED, MAGENTA, GREEN, YELLOW, BLUE])
    );
    assert_eq!(
        decode(3),
        ((3, 2), vec![YELLOW, MAGENTA, CYAN, BLUE, GREEN, RED])
    );
}

/// A 1x1 PNG whose only pixel is stored as (0x99, 0, 0) and tagged with
/// `profile` as its embedded ICC profile.
fn icc_tagged_png(profile: Vec<u8>) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut encoded = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut encoded);
    encoder.set_icc_profile(profile).unwrap();
    encoder
        .write_image(&[0x99, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    encoded
}

/// An sRGB profile whose red and green primaries trade places.
fn swapped_red_green_profile() -> Vec<u8> {
    let mut profile = moxcms::ColorProfile::new_srgb();
    std::mem::swap(&mut profile.red_colorant, &mut profile.green_colorant);
    profile.encode().unwrap()
}

#[test]
fn converts_a_tagged_image_to_srgb() {
    let decoded = ImageDecoder
        .decode(&icc_tagged_png(swapped_red_green_profile()), 1024)
        .unwrap();
    assert_eq!(decoded.rgba, [0, 0x99, 0, 255]);
}

#[test]
fn keeps_an_image_whose_profile_cannot_be_used_as_stored() {
    let decoded = ImageDecoder
        .decode(&icc_tagged_png(b"not an icc profile".to_vec()), 1024)
        .unwrap();
    assert_eq!(decoded.rgba, [0x99, 0, 0, 255]);
}
