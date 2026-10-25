use super::*;

#[test]
fn a_raster_without_pixels_is_dropped_with_a_warning() {
    let mut warnings = Vec::new();
    let empty = DecodedImage::new(0, 3, Vec::new());

    assert_eq!(require_pixels(empty, 7, &mut warnings), None);

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].node_id, Some(NodeId::new(7)));
    assert!(matches!(
        warnings[0].kind,
        WarningKind::ResourceFallback {
            kind: ResourceKind::Image,
            url: None,
        }
    ));
    assert_eq!(
        warnings[0].details,
        "inline SVG produced an empty raster size"
    );
}

#[test]
fn a_raster_with_pixels_is_kept_without_a_warning() {
    let mut warnings = Vec::new();
    let raster = DecodedImage::new(1, 1, vec![0, 0, 0, 0]);

    assert_eq!(
        require_pixels(raster.clone(), 7, &mut warnings),
        Some(raster)
    );
    assert!(warnings.is_empty());
}

#[test]
fn a_raster_is_inked_when_any_pixel_is_not_fully_transparent() {
    let mut raster = DecodedImage::new(2, 1, vec![9, 9, 9, 0, 9, 9, 9, 0]);
    assert!(!has_ink(&raster));

    raster.rgba[7] = 1;
    assert!(has_ink(&raster));
}
