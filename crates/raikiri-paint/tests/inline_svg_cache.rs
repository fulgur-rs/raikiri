//! Reuse and invalidation of inline SVG through the public painter.

use anyrender::Scene;
use anyrender::recording::RenderCommand;
use raikiri_html::{LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources};

fn layout_body(body: &str) -> raikiri_html::DocumentLayout {
    let resources = RenderResources::new();
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0}}</style>{body}"
    );
    let doc = raikiri_html::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(layout) = raikiri_html::layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!()
    };
    layout
}

fn layout() -> raikiri_html::DocumentLayout {
    layout_body(r#"<svg width="20" height="20"><path d="M0 0H20V20H0Z" fill="red"/></svg>"#)
}

fn paint(
    document: &raikiri_dom::Document,
    cascade: &raikiri_style::CascadeResult,
    page_box: raikiri_traits::PageBox,
) -> peniko::ImageData {
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, document, cascade, page_box).unwrap();
    scene
        .commands
        .into_iter()
        .find_map(|command| match command {
            RenderCommand::Fill(fill) => match fill.brush {
                anyrender::types::Paint::Image(image) => Some(image.image),
                _ => None,
            },
            _ => None,
        })
        .expect("inline SVG image")
}

#[test]
fn a_hundred_public_paints_share_the_same_inline_svg_pixels() {
    let layout = layout();
    let page = layout.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let first = paint(document, cascade, page_box);
    for _ in 1..100 {
        let next = paint(document, cascade, page_box);
        assert_eq!(first.data.data(), next.data.data());
        assert!(std::ptr::eq(
            first.data.data().as_ptr(),
            next.data.data().as_ptr()
        ));
    }
}

#[test]
fn source_attribute_and_inline_style_changes_are_seen_by_public_paint() {
    let layout = layout();
    let page = layout.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut document = document.clone();
    let path = (0..document.node_count())
        .find(|&id| document.get_node(id).unwrap().tag_name() == Some("path"))
        .unwrap();
    let first = paint(&document, cascade, page_box);
    document.set_element_attributes(
        path,
        vec![
            ("d".into(), "M0 0H20V20H0Z".into()),
            ("fill".into(), "blue".into()),
        ],
    );
    let next = paint(&document, cascade, page_box);
    assert_ne!(first.data.data(), next.data.data());
    assert_eq!(
        next.data.data(),
        paint(&document.clone(), cascade, page_box).data.data()
    );
    document.set_element_inline_style(path, Some("fill:green".into()));
    let styled = paint(&document, cascade, page_box);
    assert_ne!(next.data.data(), styled.data.data());
    assert_eq!(
        styled.data.data(),
        paint(&document.clone(), cascade, page_box).data.data()
    );
}

#[test]
fn direct_public_cascade_changes_do_not_reuse_stale_rasters() {
    let layout = layout();
    let page = layout.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut cascade = cascade.clone();
    let svg = (0..document.node_count())
        .find(|&id| document.get_node(id).unwrap().is_inline_svg_root())
        .unwrap();
    let mut previous = paint(document, &cascade, page_box);
    let generation = cascade.element_generation();
    for at in 0..8 {
        match at {
            0 => cascade.computed[svg].opacity = 0.5,
            1 => cascade.opacity_specified[svg] = !cascade.opacity_specified[svg],
            2 => cascade.background_color_specified[svg] = !cascade.background_color_specified[svg],
            3 => cascade.computed[svg].font_size.0 += 2.0,
            4 => cascade.computed[svg].font_weight += 100.0,
            5 => cascade.computed[svg].color.b = 123,
            6 => cascade.computed[svg].font_style = raikiri_style::property::FontStyle::Italic,
            _ => {
                cascade.computed[svg].font_family =
                    std::sync::Arc::new(vec![raikiri_style::property::FontFamilyName::named(
                        "Ahem",
                    )])
            }
        }
        assert_eq!(generation, cascade.element_generation());
        let next = paint(document, &cascade, page_box);
        assert!(!std::ptr::eq(
            previous.data.data().as_ptr(),
            next.data.data().as_ptr()
        ));
        assert_eq!(
            next.data.data(),
            paint(&document.clone(), &cascade, page_box).data.data()
        );
        assert!(std::ptr::eq(
            next.data.data().as_ptr(),
            paint(document, &cascade, page_box).data.data().as_ptr()
        ));
        previous = next;
    }
}

#[test]
fn hidden_roots_visible_descendants_and_viewports_match_a_fresh_document() {
    let layout = layout_body(
        r#"<svg width="20" height="20" style="visibility:hidden"><path style="visibility:visible;stroke:red;stroke-width:1em" d="M0 0H10V10H0Z"/></svg>"#,
    );
    let page = layout.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let mut document = document.clone();
    let mut cascade = cascade.clone();
    let svg = (0..document.node_count())
        .find(|&id| document.get_node(id).unwrap().is_inline_svg_root())
        .unwrap();
    let path = (0..document.node_count())
        .find(|&id| document.get_node(id).unwrap().tag_name() == Some("path"))
        .unwrap();
    let first = paint(&document, &cascade, page_box);
    assert!(std::ptr::eq(
        first.data.data().as_ptr(),
        paint(&document, &cascade, page_box).data.data().as_ptr()
    ));
    cascade.computed[svg].visibility = raikiri_style::property::Visibility::Visible;
    cascade.computed[path].font_size.0 += 10.0;
    let changed = paint(&document, &cascade, page_box);
    assert!(!std::ptr::eq(
        first.data.data().as_ptr(),
        changed.data.data().as_ptr()
    ));
    assert_eq!(
        changed.data.data(),
        paint(&document.clone(), &cascade, page_box).data.data()
    );
    document.set_element_attribute(svg, "width", "30").unwrap();
    cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    raikiri_dom::layout_single_page(&mut document, &cascade, page_box).unwrap();
    let resized = paint(&document, &cascade, page_box);
    assert_eq!(resized.width, 30);
    assert_eq!(
        resized.data.data(),
        paint(&document.clone(), &cascade, page_box).data.data()
    );
}

#[test]
fn failed_inline_svg_paints_keep_reporting_their_warnings() {
    let layout = layout_body(
        r#"<svg width="20" height="20"><image href="https://svg.test/missing.png" width="20" height="20"/></svg>"#,
    );
    let page = layout.page(0).unwrap();
    let (document, cascade, page_box, _) = page.paint_inputs();
    let resources = RenderResources::new();
    for _ in 0..100 {
        let mut scene = Scene::new();
        let warnings = raikiri_paint::paint_single_page_with_images_and_warnings(
            &mut scene, document, cascade, page_box, &resources,
        )
        .unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].details,
            "inline SVG could not be parsed or rasterized"
        );
    }
}
