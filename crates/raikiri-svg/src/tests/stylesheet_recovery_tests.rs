use super::*;

#[test]
fn malformed_or_nested_declarations_do_not_discard_later_paint() {
    for css in [
        "rect{x:calc(min(1px,2px) + 3);fill:red}",
        "rect{unknown:fn(min(1,2), \"};g{\");fill:red}",
        "rect{unknown:fn(min(1,2), \"{\");fill:rgb(255,0,0)}",
        "rect{fill: ; fill:red}",
        "rect{fill:red !IMPORTANT;fill:blue}",
        "rect{width;height:20px;fill:red} rect{fill:blue} rect{fill:red}",
        "@media print{rect{unknown:\"}\";fill:blue}} rect{fill:red}",
        "rect{width;fill:red}",
        "rect{x:{a;b};fill:red}",
        "rect{fill:red !IMPORTANT;stroke:blue}",
        "rect{unknown:fn(fn(0));fill:red ! /* priority */ IMPORTANT}",
    ] {
        let source = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><style>{css}</style><rect width=\"20\" height=\"20\" fill=\"blue\"/></svg>"
        );
        let svg = SvgDocument::parse(source.as_bytes()).unwrap();
        let viewport = SvgViewport {
            width: 20.0,
            height: 20.0,
        };
        let raster = svg
            .rasterize(viewport, SvgRootStyle::default(), None)
            .unwrap();
        assert_eq!(
            &raster.rgba[(10 * 20 + 10) * 4..(10 * 20 + 10) * 4 + 4],
            &[255, 0, 0, 255],
            "{css}"
        );
        let prepared = svg
            .styled_source(viewport, SvgRootStyle::default())
            .unwrap();
        let reparsed = SvgDocument::parse(prepared.as_bytes()).unwrap();
        let raster = reparsed
            .rasterize(viewport, SvgRootStyle::default(), None)
            .unwrap();
        assert_eq!(
            &raster.rgba[(10 * 20 + 10) * 4..(10 * 20 + 10) * 4 + 4],
            &[255, 0, 0, 255],
            "{css}: {prepared}"
        );
    }
}

#[test]
fn invalid_first_declaration_does_not_discard_later_geometry() {
    let svg = SvgDocument::parse(br#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50"><style>rect{width;height:30px}</style><rect y="10" width="50" height="10" fill="red"/></svg>"#).unwrap();
    let raster = svg
        .rasterize(
            SvgViewport {
                width: 50.0,
                height: 50.0,
            },
            SvgRootStyle::default(),
            None,
        )
        .unwrap();
    assert_eq!(
        &raster.rgba[(35 * 50 + 25) * 4..(35 * 50 + 25) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(
        &raster.rgba[(45 * 50 + 25) * 4..(45 * 50 + 25) * 4 + 4],
        &[0, 0, 0, 0]
    );
}

#[test]
fn recovered_paint_references_are_checked_before_the_backend_runs() {
    let source = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><style>g{unknown:fn(min(1,2));fill:url(#a)}</style><g><defs><pattern id="a" width="5" height="5" patternUnits="userSpaceOnUse"><rect width="5" height="5"/></pattern></defs></g><rect width="20" height="20" fill="url(#a)"/></svg>"##;
    assert!(
        matches!(SvgDocument::parse(source), Err(SvgError::InvalidDocument(message)) if message.contains("form a cycle"))
    );
}

#[test]
fn recovered_rules_are_bounded_before_selector_aliases_clone_declarations() {
    let selectors = vec!["rect"; 512].join(",");
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='1' height='1'><style>{selectors}{{width;{}}}</style><rect width='1' height='1'/></svg>",
        "fill:red;".repeat(512)
    );
    assert!(
        matches!(SvgDocument::parse(source.as_bytes()), Err(SvgError::InvalidDocument(message)) if message.contains("resource limit"))
    );
}

#[test]
fn exported_values_survive_backend_incompatible_declarations() {
    let value = "fn(min(1,2), \"};g{\")";
    let source = format!(
        "<svg xmlns='{SVG_NAMESPACE}' width='1' height='1'><style>rect{{unknown:{value};fill:red}}</style><rect width='1' height='1'/></svg>"
    );
    let svg = SvgDocument::parse(source.as_bytes()).unwrap();
    let exported = svg
        .styled_source(
            SvgViewport {
                width: 1.0,
                height: 1.0,
            },
            SvgRootStyle::default(),
        )
        .unwrap();
    assert!(exported.contains(value), "{exported}");
    let tree = super::super::parse_tree(&exported).unwrap();
    assert!(!tree.root().children().is_empty());
}
