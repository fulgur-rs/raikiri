use super::*;
use raikiri_traits::PageBox;

/// Lay out `<body>` 100px wide with `build` adding its content, giving each
/// `<img>` a 25×50 natural size.
fn laid_out(build: impl FnOnce(&mut Document, usize)) -> Document {
    laid_out_with_natural_size((25.0, 50.0), build)
}

/// [`laid_out`] with the natural size of each `<img>` given. A zero size is the
/// empty box a resolver's fallback leaves, with no ratio.
fn laid_out_with_natural_size(
    natural: (f32, f32),
    build: impl FnOnce(&mut Document, usize),
) -> Document {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:100px"),
    );
    build(&mut doc, body);
    for node in doc.nodes.layout_nodes_mut() {
        if node.tag_name() == Some("img")
            && let crate::node::NodeData::Element(element) = &mut node.data
        {
            let mut intrinsic = raikiri_traits::IntrinsicBox::new(natural.0, natural.1);
            intrinsic.aspect_ratio =
                (natural.0 > 0.0 && natural.1 > 0.0).then(|| natural.0 / natural.1);
            element.image_intrinsic_size = Some(intrinsic);
        }
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    doc
}

fn size(doc: &Document, node: usize) -> (f32, f32) {
    let size = doc.nodes[node].unrounded_layout.size;
    (size.width, size.height)
}

fn canvas_with_css(css: &str) -> (f32, f32) {
    let mut canvas = 0;
    let doc = laid_out(|doc, body| {
        canvas = doc.append_element(
            Some(body),
            "canvas",
            Style::default(),
            Some(format!("display:block;{css}").as_str()),
        );
        doc.set_element_attribute(canvas, "width", "10")
            .expect("canvas width");
        doc.set_element_attribute(canvas, "height", "10")
            .expect("canvas height");
    });
    size(&doc, canvas)
}

#[test]
fn min_width_min_content_transfers_height_bound_through_natural_ratio() {
    assert_eq!(
        canvas_with_css("width:10px;height:100px;min-width:min-content"),
        (100.0, 100.0)
    );
}

#[test]
fn min_height_min_content_transfers_width_bound_through_natural_ratio() {
    assert_eq!(
        canvas_with_css("width:100px;height:10px;min-height:min-content"),
        (100.0, 100.0)
    );
}

#[test]
fn max_width_min_content_transfers_height_bound_through_natural_ratio() {
    assert_eq!(
        canvas_with_css("width:1000px;height:100px;max-width:min-content"),
        (100.0, 100.0)
    );
}

#[test]
fn max_height_min_content_transfers_width_bound_through_natural_ratio() {
    assert_eq!(
        canvas_with_css("width:100px;height:1000px;max-height:min-content"),
        (100.0, 100.0)
    );
}

#[test]
fn ratio_only_svg_uses_viewbox_ratio_for_intrinsic_keyword_size() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;width:200px"),
    );
    let svg = doc.append_element(
        Some(body),
        "svg",
        Style::default(),
        Some("display:block;min-height:max-content"),
    );
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(svg, vec![("viewBox".into(), "0 0 1000 500".into())]);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let mut page = PageBox::new();
    page.width = 200.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).expect("layout");

    assert_eq!(size(&doc, svg), (200.0, 100.0));
}

#[test]
fn a_block_level_image_keeps_its_natural_width() {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        image = doc.append_element(Some(body), "img", Style::default(), Some("display:block"));
    });
    assert_eq!(size(&doc, image), (25.0, 50.0));
}

#[test]
fn an_empty_flex_or_grid_image_keeps_its_natural_size() {
    let mut images = Vec::new();
    let doc = laid_out(|doc, body| {
        for display in ["flex", "grid"] {
            images.push(doc.append_element(
                Some(body),
                "img",
                Style::default(),
                Some(format!("display:{display}").as_str()),
            ));
        }
    });
    for image in images {
        assert_eq!(size(&doc, image), (25.0, 50.0));
    }
}

#[test]
fn a_decoded_image_keeps_its_own_natural_size_whatever_its_file_name() {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        image = doc.append_element(Some(body), "img", Style::default(), Some("display:block"));
        doc.set_element_attribute(image, "src", "green.png")
            .unwrap();
    });
    // The 100×50 size that stands in for an image nobody decoded is not
    // authored on this one.
    assert_eq!(size(&doc, image), (25.0, 50.0));
}

#[test]
fn an_unresolved_green_image_keeps_the_stand_in_size_over_a_fallback_box() {
    let mut image = 0;
    let doc = laid_out_with_natural_size((0.0, 0.0), |doc, body| {
        image = doc.append_element(Some(body), "img", Style::default(), Some("display:block"));
        doc.set_element_attribute(image, "src", "green.png")
            .unwrap();
    });
    // A resolver's fallback stores an empty box; it is not a decoded size.
    assert_eq!(size(&doc, image), (100.0, 50.0));
}

#[test]
fn replaced_content_without_a_natural_size_still_fills_the_width() {
    let mut boxes = Vec::new();
    let doc = laid_out(|doc, body| {
        for tag in ["object", "iframe", "video", "embed"] {
            boxes.push(doc.append_element(
                Some(body),
                tag,
                Style::default(),
                Some("display:block;height:10px"),
            ));
        }
    });
    for node in boxes {
        assert_eq!(size(&doc, node), (100.0, 10.0));
    }
}

#[test]
fn a_grid_image_keeps_its_natural_size_under_normal_alignment() {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        let grid = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-template-rows:100px;grid-template-columns:100px"),
        );
        image = doc.append_element(Some(grid), "img", Style::default(), None::<&str>);
    });
    assert_eq!(size(&doc, image), (25.0, 50.0));
}

#[test]
fn a_grid_image_stretched_on_both_axes_fills_its_area() {
    let mut images = Vec::new();
    let doc = laid_out(|doc, body| {
        for (grid_css, image_css) in [
            ("", "align-self:stretch;justify-self:stretch"),
            ("align-items:stretch;justify-items:stretch", ""),
            ("place-items:stretch", "grid-row:span 1"),
        ] {
            let grid = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some(format!("display:grid;grid-template-rows:100px;grid-template-columns:100px;height:100px;{grid_css}").as_str()),
            );
            images.push(doc.append_element(Some(grid), "img", Style::default(), Some(image_css)));
        }
    });
    let sizes: Vec<_> = images.into_iter().map(|image| size(&doc, image)).collect();
    assert_eq!(sizes, vec![(100.0, 100.0); 3]);
}

#[test]
fn a_grid_image_stretched_on_one_axis_keeps_its_ratio() {
    let mut images = Vec::new();
    let doc = laid_out(|doc, body| {
        for image_css in [
            "justify-self:stretch",
            "align-self:stretch;justify-self:stretch;position:absolute",
            "align-self:stretch;justify-self:stretch;margin-bottom:auto",
        ] {
            let grid = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;height:100px;position:relative"),
            );
            images.push(doc.append_element(Some(grid), "img", Style::default(), Some(image_css)));
        }
    });
    let sizes: Vec<_> = images.into_iter().map(|image| size(&doc, image)).collect();
    assert_eq!(sizes[0], (100.0, 200.0));
    assert_eq!(sizes[1].0 / sizes[1].1, 0.5);
    assert_eq!(sizes[2], (100.0, 200.0));
}

#[test]
fn a_grid_image_with_a_definite_width_and_stretch_alignment_ignores_its_ratio() {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        let grid = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;height:100px"),
        );
        image = doc.append_element(
            Some(grid),
            "img",
            Style::default(),
            Some("align-self:stretch;justify-self:stretch;width:40px"),
        );
    });
    assert_eq!(size(&doc, image), (40.0, 100.0));
}

#[test]
fn a_stretched_grid_image_in_an_auto_row_keeps_its_ratio() {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        let grid = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-template-columns:100px;place-items:stretch"),
        );
        image = doc.append_element(Some(grid), "img", Style::default(), None::<&str>);
    });
    assert_eq!(size(&doc, image), (100.0, 200.0));
}

#[test]
fn stretched_grid_images_in_implicit_auto_rows_keep_their_ratio() {
    let mut images = Vec::new();
    let doc = laid_out(|doc, body| {
        let grid = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;place-items:stretch"),
        );
        for _ in 0..2 {
            images.push(doc.append_element(Some(grid), "img", Style::default(), None::<&str>));
        }
    });
    assert_eq!(size(&doc, images[1]), (100.0, 200.0));
}

#[test]
fn stretched_grid_images_fill_fixed_implicit_rows_but_not_repeated_ones() {
    let mut images = Vec::new();
    let doc = laid_out(|doc, body| {
        for rows in [
            "grid-template-rows:100px;grid-auto-rows:100px",
            "grid-template-rows:repeat(1, 100px)",
        ] {
            let grid = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some(
                    format!("display:grid;{rows};grid-template-columns:100px;place-items:stretch")
                        .as_str(),
                ),
            );
            for _ in 0..2 {
                images.push(doc.append_element(Some(grid), "img", Style::default(), None::<&str>));
            }
        }
    });
    let sizes: Vec<_> = images.into_iter().map(|image| size(&doc, image)).collect();
    // Implicit rows of a fixed size take the ratio-free stretch as well.
    assert_eq!(&sizes[..2], &[(100.0, 100.0); 2]);
    // A repeat() row list is not inspected and keeps the ratio.
    assert_eq!(sizes[2], (100.0, 200.0));
}

#[test]
fn a_lone_stretched_grid_image_placed_in_an_implicit_row_keeps_its_ratio() {
    let mut images = Vec::new();
    let doc = laid_out(|doc, body| {
        for placement in ["grid-row:2", "grid-row:span 2"] {
            let grid = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;place-items:stretch"),
            );
            images.push(doc.append_element(Some(grid), "img", Style::default(), Some(placement)));
        }
    });
    let sizes: Vec<_> = images.into_iter().map(|image| size(&doc, image)).collect();
    assert_eq!(sizes[0], (100.0, 200.0));
    assert_eq!(sizes[1].0 / sizes[1].1, 0.5);
}

#[test]
fn a_block_level_fallback_image_keeps_its_supplied_width() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0;width:300px"),
    );
    let image = doc.append_element(Some(body), "img", Style::default(), Some("display:block"));
    doc.set_element_attribute(image, "src", "support/green.png")
        .unwrap();
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 400.0;
    crate::layout::page_pipeline::layout_pages(&mut doc, &cascade, page).unwrap();
    assert_eq!(size(&doc, image), (100.0, 50.0));
}

/// Lay out one `<img>` with `image_css` inside a `<div>` styled `box_css`,
/// and return the sizes of the box and of the image.
fn image_in_box(box_css: &str, image_css: &str) -> ((f32, f32), (f32, f32)) {
    let (mut parent, mut image) = (0, 0);
    let doc = laid_out(|doc, body| {
        parent = doc.append_element(Some(body), "div", Style::default(), Some(box_css));
        image = doc.append_element(Some(parent), "img", Style::default(), Some(image_css));
    });
    (size(&doc, parent), size(&doc, image))
}

#[test]
fn a_shrink_to_fit_box_takes_its_width_from_the_percentage_height_of_its_image() {
    // The image is as tall as the content box of its parent, so the width the
    // ratio gives it, not its natural width, is the width of the parent.
    for box_css in [
        "position:absolute;height:100px",
        "display:inline-block;height:100px",
        "position:absolute;height:100px;padding:6px",
        "position:absolute;height:112px;border:6px solid;box-sizing:border-box",
    ] {
        let (parent, image) = image_in_box(box_css, "height:100%");
        assert_eq!(image, (50.0, 100.0), "{box_css}");
        assert_eq!(parent.0 - (parent.1 - 100.0), 50.0, "{box_css}");
    }
}

#[test]
fn a_shrink_to_fit_box_takes_its_width_from_the_percentage_height_of_its_block_image() {
    // A block-level image reaches its parent through block flow, not through
    // a paragraph, and is sized by the same content height.
    for box_css in [
        "position:absolute;height:100px",
        "display:inline-block;height:100px",
        "position:absolute;height:100px;padding:6px",
        "position:absolute;height:112px;border:6px solid;box-sizing:border-box",
    ] {
        let (parent, image) = image_in_box(box_css, "display:block;height:100%");
        assert_eq!(image, (50.0, 100.0), "{box_css}");
        assert_eq!(parent.0 - (parent.1 - 100.0), 50.0, "{box_css}");
    }
}

#[test]
fn a_calc_percentage_height_of_a_block_image_sizes_its_shrink_to_fit_parent() {
    let (parent, image) = image_in_box(
        "position:absolute;height:100px",
        "display:block;height:calc(100% - 20px)",
    );
    assert_eq!(image, (40.0, 80.0));
    assert_eq!(parent, (40.0, 100.0));
}

#[test]
fn a_percentage_height_of_a_block_image_against_an_auto_height_keeps_the_natural_width() {
    let (parent, image) = image_in_box("display:inline-block", "display:block;height:100%");
    assert_eq!(image, (25.0, 50.0));
    assert_eq!(parent.0, 25.0);
}

#[test]
fn a_length_height_of_a_block_image_does_not_follow_the_height_of_its_parent() {
    let (parent, image) = image_in_box(
        "position:absolute;height:100px",
        "display:block;height:20px",
    );
    assert_eq!(image, (10.0, 20.0));
    assert_eq!(parent, (10.0, 100.0));
}

#[test]
fn a_calc_percentage_height_of_an_image_sizes_its_shrink_to_fit_parent() {
    let (parent, image) =
        image_in_box("position:absolute;height:100px", "height:calc(100% - 20px)");
    assert_eq!(image, (40.0, 80.0));
    assert_eq!(parent, (40.0, 100.0));
}

#[test]
fn a_percentage_height_against_an_auto_height_keeps_the_natural_width() {
    let (parent, image) = image_in_box("display:inline-block", "height:100%");
    assert_eq!(image, (25.0, 50.0));
    assert_eq!(parent.0, 25.0);
}

#[test]
fn a_length_height_of_an_image_does_not_follow_the_height_of_its_parent() {
    let (parent, image) = image_in_box("position:absolute;height:100px", "height:20px");
    assert_eq!(image, (10.0, 20.0));
    assert_eq!(parent, (10.0, 100.0));
}

#[test]
fn the_width_of_a_percentage_height_image_follows_the_height_it_is_measured_against() {
    // The same image is measured against a series of heights, and against
    // none: the size measured for one never answers another.
    let heights = [Some(100.0), None, Some(60.0), Some(100.0)];
    for (image_css, expected) in [
        ("height:100%", [50.0, 25.0, 30.0, 50.0]),
        ("height:calc(100% - 20px)", [40.0, 25.0, 20.0, 40.0]),
        ("min-height:100%", [50.0, 25.0, 30.0, 50.0]),
        ("min-height:calc(100% - 20px)", [40.0, 25.0, 25.0, 40.0]),
        ("max-height:50%", [25.0, 25.0, 15.0, 25.0]),
        ("max-height:calc(50% - 10px)", [20.0, 25.0, 10.0, 20.0]),
    ] {
        let mut parent = 0;
        let mut doc = laid_out(|doc, body| {
            parent = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:inline-block;height:100px"),
            );
            doc.append_element(Some(parent), "img", Style::default(), Some(image_css));
        });
        let widths: Vec<f32> = heights
            .into_iter()
            .map(|percent_height| {
                let intrinsics = {
                    let mut session = crate::layout::context::LayoutSession::new(&mut doc);
                    crate::layout::ifc::boxes::intrinsics_of_boxes(
                        &mut session,
                        parent,
                        100.0,
                        percent_height,
                        None,
                    )
                };
                let root = doc.nodes[parent].ifc.as_ref().unwrap();
                root.paragraph
                    .intrinsic_sizes(
                        &mut shodo::LayoutContext::new(),
                        &root.options,
                        &intrinsics.engine,
                    )
                    .max_content
            })
            .collect();
        assert_eq!(widths, expected, "{image_css}");
    }
}

#[test]
fn the_width_of_a_percentage_height_container_follows_the_height_it_is_measured_against() {
    // The image sits in a flex container whose own height is a percentage, so
    // the width of the container follows the height of the paragraph root
    // through the image, and so does the width of the image in the container.
    let mut parent = 0;
    let mut doc = laid_out(|doc, body| {
        parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:inline-block;height:100px"),
        );
        let container = doc.append_element(
            Some(parent),
            "span",
            Style::default(),
            Some("display:inline-flex;height:100%"),
        );
        doc.append_element(
            Some(container),
            "img",
            Style::default(),
            Some("height:100%"),
        );
    });
    let widths: Vec<f32> = [Some(100.0), None, Some(60.0), Some(100.0)]
        .into_iter()
        .map(|percent_height| {
            let intrinsics = {
                let mut session = crate::layout::context::LayoutSession::new(&mut doc);
                crate::layout::ifc::boxes::intrinsics_of_boxes(
                    &mut session,
                    parent,
                    100.0,
                    percent_height,
                    None,
                )
            };
            let root = doc.nodes[parent].ifc.as_ref().unwrap();
            root.paragraph
                .intrinsic_sizes(
                    &mut shodo::LayoutContext::new(),
                    &root.options,
                    &intrinsics.engine,
                )
                .max_content
        })
        .collect();
    assert_eq!(widths, [50.0, 25.0, 30.0, 50.0]);
}

#[test]
fn the_width_of_a_block_level_image_follows_the_height_of_the_box_it_is_measured_in() {
    // The same box is measured at a series of heights, and at none: the width
    // measured for one never answers another.
    let mut parent = 0;
    let mut doc = laid_out(|doc, body| {
        parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:inline-block"),
        );
        doc.append_element(
            Some(parent),
            "img",
            Style::default(),
            Some("display:block;height:100%"),
        );
    });
    let widths: Vec<f32> = [Some(100.0), None, Some(60.0), Some(100.0)]
        .into_iter()
        .map(|height| {
            let mut session = crate::layout::context::LayoutSession::new(&mut doc);
            let inputs = taffy::LayoutInput {
                run_mode: taffy::RunMode::ComputeSize,
                sizing_mode: taffy::SizingMode::InherentSize,
                axis: taffy::RequestedAxis::Both,
                known_dimensions: Size {
                    width: None,
                    height,
                },
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: Size {
                    width: Some(100.0),
                    height: None,
                },
                available_space: Size {
                    width: taffy::AvailableSpace::MaxContent,
                    height: taffy::AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: taffy::Line::FALSE,
            };
            crate::taffy_impl::block_layout(&mut session, taffy::NodeId::from(parent), inputs, None)
                .size
                .width
        })
        .collect();
    assert_eq!(widths, [50.0, 25.0, 30.0, 50.0]);
}

/// Lay out one `<img>` with `css` on `<body>` and return its size.
fn image_size(css: &str) -> (f32, f32) {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        image = doc.append_element(Some(body), "img", Style::default(), Some(css));
    });
    size(&doc, image)
}

#[test]
fn a_max_height_brings_an_image_without_a_size_within_it_keeping_the_ratio() {
    // The image is 25x50, so a height capped at 25px is 12.5px wide.
    for display in ["inline", "block", "inline-block"] {
        assert_eq!(
            image_size(&format!("display:{display};max-height:25px")),
            (12.5, 25.0),
            "{display}"
        );
    }
}

#[test]
fn a_max_width_brings_an_image_without_a_size_within_it_keeping_the_ratio() {
    for display in ["inline", "block", "inline-block"] {
        assert_eq!(
            image_size(&format!("display:{display};max-width:10px")),
            (10.0, 20.0),
            "{display}"
        );
    }
}

#[test]
fn a_calc_max_width_brings_an_image_of_any_display_within_it() {
    // A percentage resolves against the 100px body: 10px + 10% is 20px.
    for display in ["inline", "block", "inline-block"] {
        assert_eq!(
            image_size(&format!("display:{display};max-width:calc(10px + 10%)")),
            (20.0, 40.0),
            "{display}"
        );
    }
}

#[test]
fn an_image_without_a_size_follows_the_constraint_table_of_css_2_1() {
    for (css, expected) in [
        // Both maxima are exceeded: the one that shrinks it most wins.
        ("max-width:10px;max-height:40px", (10.0, 20.0)),
        ("max-width:20px;max-height:20px", (10.0, 20.0)),
        // Over a max in one axis and under a min in the other.
        ("min-width:50px;max-height:20px", (50.0, 20.0)),
        ("max-width:10px;min-height:80px", (10.0, 80.0)),
        // A min raises the image the same way.
        ("min-width:50px", (50.0, 100.0)),
        ("min-height:100px", (50.0, 100.0)),
        // A percentage resolves against the 100px body.
        ("max-width:20%", (20.0, 40.0)),
        ("max-width:calc(10px + 10%)", (20.0, 40.0)),
        // The constraints apply to the border box with `border-box`.
        (
            "box-sizing:border-box;padding:5px;max-width:20px",
            (20.0, 30.0),
        ),
        ("padding:5px;max-width:10px", (20.0, 30.0)),
    ] {
        assert_eq!(image_size(css), expected, "{css}");
    }
}

#[test]
fn a_size_on_one_side_of_an_image_is_not_changed_by_the_constraint_table() {
    // The specified side wins, the other side is cut by its own max, and the
    // ratio no longer holds.
    assert_eq!(image_size("height:50px;max-width:10px"), (10.0, 50.0));
    assert_eq!(image_size("width:25px;max-height:20px"), (25.0, 20.0));
    assert_eq!(
        image_size("width:20px;height:20px;max-width:10px"),
        (10.0, 20.0)
    );
}

#[test]
fn a_shrink_to_fit_box_takes_its_width_from_the_percentage_max_height_of_its_image() {
    // A max-height of 20% of the 100px content box is 20px, which the ratio
    // carries over to a 10px width for the parent to wrap.
    for box_css in [
        "position:absolute;height:100px",
        "display:inline-block;height:100px",
    ] {
        for display in ["inline", "block", "inline-block"] {
            let (parent, image) =
                image_in_box(box_css, &format!("display:{display};max-height:20%"));
            assert_eq!(image, (10.0, 20.0), "{box_css} {display}");
            assert_eq!(parent.0, 10.0, "{box_css} {display}");
        }
    }
}

#[test]
fn an_image_with_both_sides_authored_keeps_them_in_every_container() {
    // Both sides are fixed by the author, so the ratio and the constraint
    // table have nothing to decide.
    for box_css in [
        "display:block",
        "display:flex;flex-direction:column",
        "display:flex;flex-direction:row",
        "display:grid",
    ] {
        let (_, image) = image_in_box(box_css, "width:20px;height:20px");
        assert_eq!(image, (20.0, 20.0), "{box_css}");
    }
}

#[test]
fn a_percentage_max_height_without_a_definite_height_does_not_constrain_an_image() {
    assert_eq!(image_size("max-height:50%"), (25.0, 50.0));
}

#[test]
fn a_percentage_height_of_an_image_ignores_the_content_height_of_an_absolute_box() {
    // The height of the box follows from its content, so the percentage
    // height of the image is `auto` (CSS 2.1 10.5), as in an inline block.
    let (inline_block, inline_block_image) = image_in_box("display:inline-block", "height:100%");
    assert_eq!(inline_block_image, (25.0, 50.0));
    for box_css in [
        "position:absolute",
        "position:absolute;top:0",
        "position:absolute;bottom:0",
        "position:fixed",
    ] {
        let (parent, image) = image_in_box(box_css, "height:100%");
        assert_eq!(image, (25.0, 50.0), "{box_css}");
        assert_eq!(parent.1, inline_block.1, "{box_css}");
    }
}

#[test]
fn a_percentage_height_of_an_image_resolves_against_an_absolute_box_with_a_definite_height() {
    // A `height` of its own, or both `top` and `bottom` in a container of a
    // definite height, fix the height of the box.
    for box_css in [
        "position:absolute;height:100px",
        "position:absolute;height:100px;top:0",
        "position:absolute;height:50px;top:0;bottom:0",
    ] {
        let (_, image) = image_in_box(box_css, "height:100%");
        let expected = if box_css.contains("height:50px") {
            (25.0, 50.0)
        } else {
            (50.0, 100.0)
        };
        assert_eq!(image, expected, "{box_css}");
    }
    let (parent, image) = {
        let (mut parent, mut image) = (0, 0);
        let doc = laid_out(|doc, body| {
            let container = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("position:relative;height:80px"),
            );
            parent = doc.append_element(
                Some(container),
                "div",
                Style::default(),
                Some("position:absolute;top:0;bottom:0"),
            );
            image = doc.append_element(Some(parent), "img", Style::default(), Some("height:100%"));
        });
        (size(&doc, parent), size(&doc, image))
    };
    assert_eq!(parent.1, 80.0);
    assert_eq!(image, (40.0, 80.0));
}

#[test]
fn a_percentage_height_of_an_image_resolves_against_an_absolute_box_pinned_by_its_min_and_max_height()
 {
    // A `min-height` that a `max-height` does not exceed fixes the height of
    // the box whatever its content, although its `height` is `auto`.
    for box_css in [
        "position:absolute;min-height:100px;max-height:100px",
        "position:absolute;min-height:100px;max-height:50px",
        "position:fixed;min-height:100px;max-height:100px",
    ] {
        let (parent, image) = image_in_box(box_css, "height:100%");
        assert_eq!(image, (50.0, 100.0), "{box_css}");
        assert_eq!(parent.1, 100.0, "{box_css}");
    }
    // A range of heights leaves the height to the content.
    let (_, image) = image_in_box(
        "position:absolute;min-height:50px;max-height:100px",
        "height:100%",
    );
    assert_eq!(image, (25.0, 50.0));
}

#[test]
fn a_percentage_height_of_a_block_is_auto_in_an_absolute_box_that_sizes_to_its_content() {
    let height = |box_css: &str| {
        let (mut parent, mut child) = (0, 0);
        let doc = laid_out(|doc, body| {
            parent = doc.append_element(Some(body), "div", Style::default(), Some(box_css));
            child = doc.append_element(
                Some(parent),
                "div",
                Style::default(),
                Some("height:100%;min-height:10px"),
            );
        });
        (size(&doc, parent).1, size(&doc, child).1)
    };
    assert_eq!(height("position:absolute"), (10.0, 10.0));
    assert_eq!(height("position:absolute;height:30px"), (30.0, 30.0));
    assert_eq!(height("display:inline-block"), (10.0, 10.0));
}

#[test]
fn a_replaced_box_in_a_vertical_shrink_to_fit_box_resolves_its_percentage_width() {
    // The box is 50% of the 200px container, 100px wide. Its image takes the
    // whole width, and the 1:1 ratio makes it 100px tall, which the box
    // takes as the size of its single line.
    let (mut parent, mut image) = (0, 0);
    let doc = laid_out_with_natural_size((1.0, 1.0), |doc, body| {
        let container = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("position:relative;width:200px;height:100px"),
        );
        parent = doc.append_element(
            Some(container),
            "div",
            Style::default(),
            Some("position:absolute;writing-mode:vertical-rl;width:50%"),
        );
        image = doc.append_element(Some(parent), "img", Style::default(), Some("width:100%"));
    });
    assert_eq!(size(&doc, image), (100.0, 100.0));
    assert_eq!(size(&doc, parent), (100.0, 100.0));
}
