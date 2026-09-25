use super::{SvgDocument, SvgError, SvgRootStyle, SvgViewport};

const HALF_RED_RECT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1" viewBox="0 0 2 1"><rect width="1" height="1" fill="#ff0000" fill-opacity="0.5"/></svg>"##;

#[test]
fn rasterizes_at_requested_size_and_returns_straight_alpha_rgba() {
    let svg = SvgDocument::parse(HALF_RED_RECT).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 4.0,
                height: 2.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .expect("bounded rasterization succeeds");

    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 128]);
    assert_eq!(&image.rgba[8..12], &[0, 0, 0, 0]);
}

#[test]
fn intrinsic_dimensions_are_independent_of_raster_size() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="1in" viewBox="0 0 2 1"/>"#,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 7.0,
                height: 3.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(svg.intrinsic_size().width, Some(192.0));
    assert_eq!(svg.intrinsic_size().height, Some(96.0));
    assert_eq!(svg.intrinsic_size().aspect_ratio, Some(2.0));
    assert_eq!((image.width, image.height), (7, 3));
}

#[test]
fn view_box_ratio_is_preserved_when_natural_dimensions_are_absent() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"><rect width="4" height="2"/></svg>"#,
    )
    .expect("valid SVG");

    assert_eq!(svg.intrinsic_size().width, None);
    assert_eq!(svg.intrinsic_size().height, None);
    assert_eq!(svg.intrinsic_size().aspect_ratio, Some(2.0));
}

#[test]
fn inherited_current_color_is_applied_before_rasterization() {
    let svg = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="currentColor"/></svg>"#,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                inherited_color: [0, 128, 0, 255],
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[0, 128, 0, 255]);
}

#[test]
fn host_opacity_and_visibility_apply_to_the_completed_svg() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let half_opacity = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");
    let hidden = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                visible: false,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("hidden SVG still has a transparent raster");

    assert_eq!(&half_opacity.rgba, &[255, 0, 0, 128]);
    assert_eq!(&hidden.rgba, &[0, 0, 0, 0]);
}

#[test]
fn rejects_doctypes_and_non_fragment_image_references() {
    let with_doctype = br#"<!DOCTYPE svg [<!ENTITY x "y">]><svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    assert!(matches!(
        SvgDocument::parse(with_doctype),
        Err(SvgError::UnsupportedDoctype)
    ));

    for href in [
        "https://example.test/image.png",
        "file:///etc/passwd",
        "data:image/png;base64,AA==",
    ] {
        let source = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><image href=\"{href}\"/></svg>"
        );
        assert!(matches!(
            SvgDocument::parse(source.as_bytes()),
            Err(SvgError::ExternalReference)
        ));
    }
}

#[test]
fn rejects_output_above_the_caller_byte_limit() {
    let svg = SvgDocument::parse(HALF_RED_RECT).expect("valid SVG");
    let error = svg
        .rasterize(
            SvgViewport {
                width: 4.0,
                height: 2.0,
            },
            SvgRootStyle::default(),
            Some(31),
        )
        .expect_err("32 output bytes exceed the 31-byte caller limit");

    assert!(matches!(
        error,
        SvgError::OutputLimitExceeded {
            bytes: 32,
            limit: 31,
        }
    ));
}
