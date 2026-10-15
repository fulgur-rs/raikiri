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
