use super::*;

#[test]
fn default_page_box_fits_the_raster_budget() {
    let mut budget = RasterBufferBudget::new();
    let size = budget
        .reserve_page_box(PageBox::A4)
        .expect("A4 raster size");
    assert_eq!((size.width(), size.height()), (794, 1123));
    assert_eq!(size.pixels(), 794 * 1123);
    assert_eq!(size.bytes(), 794 * 1123 * 4);
    assert_eq!(budget.reserved_bytes(), size.bytes());
}

#[test]
fn oversized_edge_is_rejected_without_reserving_bytes() {
    let mut budget = RasterBufferBudget::new();
    let error = budget
        .reserve_pixels(MAX_RASTER_EDGE + 1, 1)
        .expect_err("page edge must be capped");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::RasterEdge,
            limit,
            actual,
        } if limit == u64::from(MAX_RASTER_EDGE) && actual == u64::from(MAX_RASTER_EDGE + 1)
    ));
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn integer_dimensions_must_be_positive() {
    let mut budget = RasterBufferBudget::new();
    for (width, height) in [(0, 1), (1, 0), (0, 0)] {
        let error = budget
            .reserve_pixels(width, height)
            .expect_err("zero dimensions must be rejected");
        assert!(matches!(error, RenderError::Configuration(_)));
    }
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn oversized_single_page_byte_count_is_rejected() {
    let mut budget = RasterBufferBudget::new();
    let error = budget
        .reserve_pixels(4097, 4096)
        .expect_err("page byte count must be capped");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::RasterPageBytes,
            limit: MAX_PAGE_RASTER_BYTES,
            actual,
        } if actual == 4097 * 4096 * 4
    ));
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn aggregate_page_bytes_are_checked_before_reserving_the_next_page() {
    let mut budget = RasterBufferBudget::new();
    for _ in 0..4 {
        budget
            .reserve_pixels(4096, 4096)
            .expect("each 64 MiB page fits the per-page cap");
    }
    assert_eq!(budget.reserved_bytes(), MAX_DOCUMENT_RASTER_BYTES);

    let error = budget
        .reserve_pixels(1, 1)
        .expect_err("aggregate document bytes must be capped");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::RasterDocumentBytes,
            limit: MAX_DOCUMENT_RASTER_BYTES,
            actual,
        } if actual == MAX_DOCUMENT_RASTER_BYTES + 4
    ));
    assert_eq!(budget.reserved_bytes(), MAX_DOCUMENT_RASTER_BYTES);
}

#[test]
fn aggregate_addition_overflow_returns_a_document_limit_error() {
    let mut budget = RasterBufferBudget {
        reserved_bytes: u64::MAX,
    };
    let error = budget
        .reserve_pixels(1, 1)
        .expect_err("aggregate addition must not wrap");
    assert!(matches!(
        error,
        RenderError::LimitExceeded {
            kind: LimitKind::RasterDocumentBytes,
            actual: u64::MAX,
            ..
        }
    ));
    assert_eq!(budget.reserved_bytes(), u64::MAX);
}

#[test]
fn page_dimensions_must_be_finite_and_positive() {
    let mut budget = RasterBufferBudget::new();
    for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut page_box = PageBox::new();
        page_box.width = invalid;
        page_box.height = 1.0;
        let error = budget
            .reserve_page_box(page_box)
            .expect_err("invalid page width must be rejected");
        assert!(matches!(error, RenderError::Configuration(_)));
    }
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn checked_pixel_and_byte_multiplications_reject_overflow() {
    let pixel_overflow =
        checked_pixel_bytes(u64::MAX, 2).expect_err("pixel multiplication must not wrap");
    assert!(matches!(
        pixel_overflow,
        RenderError::LimitExceeded {
            kind: LimitKind::RasterPageBytes,
            actual: u64::MAX,
            ..
        }
    ));

    let byte_overflow = checked_pixel_bytes(u64::MAX / 4 + 1, 1)
        .expect_err("RGBA byte multiplication must not wrap");
    assert!(matches!(
        byte_overflow,
        RenderError::LimitExceeded {
            kind: LimitKind::RasterPageBytes,
            actual: u64::MAX,
            ..
        }
    ));
}
