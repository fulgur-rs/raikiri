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
fn root_preserve_aspect_ratio_uses_the_requested_viewport() {
    let meet = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let stretched = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100" preserveAspectRatio="none"><circle cx="50" cy="50" r="40" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let viewport = SvgViewport {
        width: 200.0,
        height: 100.0,
    };

    let meet = meet
        .rasterize(viewport, SvgRootStyle::default(), None)
        .expect("meet rasterization succeeds");
    let stretched = stretched
        .rasterize(viewport, SvgRootStyle::default(), None)
        .expect("none rasterization succeeds");
    let pixel = |image: &raikiri_traits::DecodedImage, x: usize, y: usize| {
        let index = (y * image.width as usize + x) * 4;
        [
            image.rgba[index],
            image.rgba[index + 1],
            image.rgba[index + 2],
            image.rgba[index + 3],
        ]
    };

    assert_eq!((meet.width, meet.height), (200, 100));
    assert_eq!(pixel(&meet, 140, 50), [0, 0, 0, 0]);
    assert_eq!(pixel(&stretched, 140, 50), [255, 0, 0, 255]);
}

#[test]
fn viewport_override_replaces_important_inline_dimensions() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" style="color:green!important;width:100px!important;height:100px!important"><style>svg { width:100px!important;height:100px!important }</style><rect width="100" height="100" fill="#ff0000"/></svg>"##;
    let resized = super::with_root_viewport_size(
        std::str::from_utf8(source).expect("SVG is UTF-8"),
        200,
        100,
    )
    .expect("viewport override succeeds");
    let tree = super::parse_tree(&resized).expect("adjusted SVG parses");

    assert!(resized.contains("color:green!important"));
    assert_eq!(tree.size().width(), 200.0);
    assert_eq!(tree.size().height(), 100.0);
}

#[test]
fn equal_ratio_viewport_resize_preserves_absolute_geometry_without_view_box() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 200.0,
                height: 200.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .expect("rasterization succeeds");

    let alpha_at = |x: usize, y: usize| image.rgba[(y * image.width as usize + x) * 4 + 3];
    assert_eq!(alpha_at(5, 5), 255);
    assert_eq!(alpha_at(15, 5), 0);
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
fn root_opacity_can_be_neutralized_for_outer_group_compositing() {
    let sources: [&[u8]; 5] = [
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.25"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" style="opacity:0.25"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>svg { opacity:0.25 }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" style="opacity:0.25!important"><rect width="1" height="1" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>svg { opacity:0.25!important }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##,
    ];

    for source in sources {
        let svg = SvgDocument::parse(source).expect("valid SVG");
        let image = svg
            .rasterize(
                SvgViewport {
                    width: 1.0,
                    height: 1.0,
                },
                SvgRootStyle {
                    neutralize_root_opacity: true,
                    ..SvgRootStyle::default()
                },
                None,
            )
            .expect("rasterization succeeds");
        assert_eq!(
            &image.rgba,
            &[255, 0, 0, 255],
            "failed to neutralize source opacity in {}",
            String::from_utf8_lossy(source)
        );
    }
}

#[test]
fn root_opacity_neutralization_preserves_descendant_stylesheet_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>svg, rect { opacity:0.25!important;font-family:&quot;A&amp;B&quot; }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 64]);
}

#[test]
fn root_opacity_neutralization_preserves_inherited_opacity_on_children() {
    let sources: [&[u8]; 3] = [
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><rect width="1" height="1" opacity="inherit" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><rect width="1" height="1" style="opacity:inherit" fill="#ff0000"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><rect width="1" height="1" style="opacity:inherit!important" fill="#ff0000"/></svg>"##,
    ];

    for source in sources {
        let svg = SvgDocument::parse(source).expect("valid SVG");
        let image = svg
            .rasterize(
                SvgViewport {
                    width: 1.0,
                    height: 1.0,
                },
                SvgRootStyle {
                    opacity: 0.5,
                    neutralize_root_opacity: true,
                    ..SvgRootStyle::default()
                },
                None,
            )
            .expect("rasterization succeeds");

        assert_eq!(
            &image.rgba,
            &[255, 0, 0, 128],
            "failed to preserve inherited child opacity in {}",
            String::from_utf8_lossy(source)
        );
    }
}

#[test]
fn root_opacity_neutralization_keeps_nested_inherited_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><g opacity="0.25"><rect width="1" height="1" style="opacity:inherit" fill="#ff0000"/></g></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 16]);
}

#[test]
fn root_opacity_neutralization_preserves_quoted_attribute_selectors() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>[data-name="a'b"] { fill:red } rect /* note */ { opacity:.25 }</style><rect data-name="a'b" width="1" height="1"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 64]);
}

#[test]
fn root_opacity_neutralization_preserves_stylesheet_inherited_opacity() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><style>rect { opacity:inherit }</style><rect width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 128]);
}

#[test]
fn root_opacity_neutralization_does_not_promote_inherited_opacity_importance() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" opacity="0.5"><style>rect { opacity:inherit } .opaque { opacity:1 }</style><rect class="opaque" width="1" height="1" fill="#ff0000"/></svg>"##;
    let svg = SvgDocument::parse(source).expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                opacity: 0.5,
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 255]);
}

#[test]
fn root_opacity_neutralization_rewrites_cdata_stylesheet_content() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>
<![CDATA[svg { opacity:.25!important }]]>
</style><rect width="1" height="1" fill="#ff0000"/></svg>"##,
    )
    .expect("valid SVG");
    let image = svg
        .rasterize(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle {
                neutralize_root_opacity: true,
                ..SvgRootStyle::default()
            },
            None,
        )
        .expect("rasterization succeeds");

    assert_eq!(&image.rgba, &[255, 0, 0, 255]);
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
fn rejects_filter_effects_before_rasterization() {
    let sources: &[&[u8]] = &[
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" filter="url(#f)"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" style="filter: drop-shadow(0 0 1px red)"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>rect { filter: url(#f) }</style><rect width="1" height="1"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><x:style xmlns:x="urn:foreign">rect { filter: url(#f) }</x:style><rect width="1" height="1"/></svg>"##,
        br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" xlink:filter="url(#f)"/></svg>"##,
    ];

    for source in sources {
        assert!(
            SvgDocument::parse(source).is_err(),
            "filter-bearing SVG must be rejected before resvg can execute effects"
        );
    }

    let filter_none = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" style="filter:none"/></svg>"#,
    );
    assert!(filter_none.is_ok(), "filter:none has no pixel effect");

    let non_css_style = SvgDocument::parse(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style type="application/json">rect { filter:url(#f) }</style><rect width="1" height="1"/></svg>"#,
    );
    assert!(non_css_style.is_ok(), "non-CSS style data is not active");
}

#[test]
fn allows_unused_filter_definitions() {
    let svg = SvgDocument::parse(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="unused"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" fill="blue"/></svg>"##,
    );

    assert!(
        svg.is_ok(),
        "an unreferenced filter definition has no effect"
    );
}

#[test]
fn parse_tree_rejects_active_filter_effects_for_every_parse_path() {
    let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><defs><filter id="f"><feFlood flood-color="red"/></filter></defs><rect width="1" height="1" filter="url(#f)"/></svg>"##;

    assert!(matches!(
        super::parse_tree(source),
        Err(SvgError::UnsupportedFilterEffects)
    ));
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
