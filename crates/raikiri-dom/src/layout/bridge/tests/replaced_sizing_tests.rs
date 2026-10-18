use super::*;
use raikiri_traits::PageBox;

/// Lay out `<body>` 100px wide with `build` adding its content, giving each
/// `<img>` a 25×50 natural size.
fn laid_out(build: impl FnOnce(&mut Document, usize)) -> Document {
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
            let mut intrinsic = raikiri_traits::IntrinsicBox::new(25.0, 50.0);
            intrinsic.aspect_ratio = Some(0.5);
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

#[test]
fn a_block_level_image_keeps_its_natural_width() {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        image = doc.append_element(Some(body), "img", Style::default(), Some("display:block"));
    });
    assert_eq!(size(&doc, image), (25.0, 50.0));
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
            "align-self:stretch;justify-self:stretch;width:40px",
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
    assert_eq!(sizes[1], (40.0, 80.0));
    assert_eq!(sizes[2].0 / sizes[2].1, 0.5);
    assert_eq!(sizes[3], (100.0, 200.0));
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
