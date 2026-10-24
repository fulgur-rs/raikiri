use super::*;
use crate::layout::protocol::{LayoutInput, RequestedAxis, RunMode, SizingMode};
use raikiri_layout_geometry::Line;
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
    laid_out_with_natural_box(natural, true, build)
}

/// [`laid_out_with_natural_size`] where the resolver may leave the natural
/// ratio unset (`ratio_known` is false), as `IntrinsicBox::new` does.
fn laid_out_with_natural_box(
    natural: (f32, f32),
    ratio_known: bool,
    build: impl FnOnce(&mut Document, usize),
) -> Document {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    build(&mut doc, body);
    for node in doc.nodes.layout_nodes_mut() {
        if node.tag_name() == Some("img")
            && let crate::node::NodeData::Element(element) = &mut node.data
        {
            let mut intrinsic = raikiri_traits::IntrinsicBox::new(natural.0, natural.1);
            intrinsic.aspect_ratio =
                (ratio_known && natural.0 > 0.0 && natural.1 > 0.0).then(|| natural.0 / natural.1);
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
    let size = doc.nodes[node].layout.size;
    (size.width, size.height)
}

fn canvas_with_css(css: &str) -> (f32, f32) {
    let mut canvas = 0;
    let doc = laid_out(|doc, body| {
        canvas = doc.append_element(
            Some(body),
            "canvas",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;width:200px"));
    let svg = doc.append_element(
        Some(body),
        "svg",
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
        image = doc.append_element(Some(body), "img", Some("display:block"));
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
        image = doc.append_element(Some(body), "img", Some("display:block"));
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
        image = doc.append_element(Some(body), "img", Some("display:block"));
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
            boxes.push(doc.append_element(Some(body), tag, Some("display:block;height:10px")));
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
            Some("display:grid;grid-template-rows:100px;grid-template-columns:100px"),
        );
        image = doc.append_element(Some(grid), "img", None::<&str>);
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
                Some(format!("display:grid;grid-template-rows:100px;grid-template-columns:100px;height:100px;{grid_css}").as_str()),
            );
            images.push(doc.append_element(Some(grid), "img", Some(image_css)));
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
                Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;height:100px;position:relative"),
            );
            images.push(doc.append_element(Some(grid), "img", Some(image_css)));
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
            Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;height:100px"),
        );
        image = doc.append_element(
            Some(grid),
            "img",
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
            Some("display:grid;grid-template-columns:100px;place-items:stretch"),
        );
        image = doc.append_element(Some(grid), "img", None::<&str>);
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
            Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;place-items:stretch"),
        );
        for _ in 0..2 {
            images.push(doc.append_element(Some(grid), "img", None::<&str>));
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
                Some(
                    format!("display:grid;{rows};grid-template-columns:100px;place-items:stretch")
                        .as_str(),
                ),
            );
            for _ in 0..2 {
                images.push(doc.append_element(Some(grid), "img", None::<&str>));
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
                Some("display:grid;grid-template-rows:100px;grid-template-columns:100px;place-items:stretch"),
            );
            images.push(doc.append_element(Some(grid), "img", Some(placement)));
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:300px"),
    );
    let image = doc.append_element(Some(body), "img", Some("display:block"));
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
        parent = doc.append_element(Some(body), "div", Some(box_css));
        image = doc.append_element(Some(parent), "img", Some(image_css));
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
            parent =
                doc.append_element(Some(body), "div", Some("display:inline-block;height:100px"));
            doc.append_element(Some(parent), "img", Some(image_css));
        });
        let widths: Vec<f32> = heights
            .into_iter()
            .map(|percent_height| {
                let intrinsics = {
                    let session_cascade = crate::layout::test_support::cascaded(&doc);
                    let mut session =
                        crate::layout::context::LayoutSession::bridged(&mut doc, &session_cascade);
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
        parent = doc.append_element(Some(body), "div", Some("display:inline-block;height:100px"));
        let container = doc.append_element(
            Some(parent),
            "span",
            Some("display:inline-flex;height:100%"),
        );
        doc.append_element(Some(container), "img", Some("height:100%"));
    });
    let widths: Vec<f32> = [Some(100.0), None, Some(60.0), Some(100.0)]
        .into_iter()
        .map(|percent_height| {
            let intrinsics = {
                let session_cascade = crate::layout::test_support::cascaded(&doc);
                let mut session =
                    crate::layout::context::LayoutSession::bridged(&mut doc, &session_cascade);
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
        parent = doc.append_element(Some(body), "div", Some("display:inline-block"));
        doc.append_element(Some(parent), "img", Some("display:block;height:100%"));
    });
    let widths: Vec<f32> = [Some(100.0), None, Some(60.0), Some(100.0)]
        .into_iter()
        .map(|height| {
            let session_cascade = crate::layout::test_support::cascaded(&doc);
            let mut session =
                crate::layout::context::LayoutSession::bridged(&mut doc, &session_cascade);
            let inputs = LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
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
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            };
            crate::layout::dispatch::block_layout(
                &mut session,
                protocol::NodeId::from(parent),
                inputs,
                None,
            )
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
        image = doc.append_element(Some(body), "img", Some(css));
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
            let container =
                doc.append_element(Some(body), "div", Some("position:relative;height:80px"));
            parent = doc.append_element(
                Some(container),
                "div",
                Some("position:absolute;top:0;bottom:0"),
            );
            image = doc.append_element(Some(parent), "img", Some("height:100%"));
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
            parent = doc.append_element(Some(body), "div", Some(box_css));
            child = doc.append_element(Some(parent), "div", Some("height:100%;min-height:10px"));
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
            Some("position:relative;width:200px;height:100px"),
        );
        parent = doc.append_element(
            Some(container),
            "div",
            Some("position:absolute;writing-mode:vertical-rl;width:50%"),
        );
        image = doc.append_element(Some(parent), "img", Some("width:100%"));
    });
    assert_eq!(size(&doc, image), (100.0, 100.0));
    assert_eq!(size(&doc, parent), (100.0, 100.0));
}

/// Lay out one 25x50 `<img>` with `image_css` in a `<div>` with `container_css`
/// and return its size.
fn image_in_container(container_css: &str, image_css: &str) -> (f32, f32) {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        let container = doc.append_element(Some(body), "div", Some(container_css));
        image = doc.append_element(Some(container), "img", Some(image_css));
    });
    size(&doc, image)
}

/// The containers a replaced box with a keyword width is laid out in, each
/// 100px wide: block, grid, and row and column flex.
const KEYWORD_WIDTH_CONTAINERS: [(&str, &str); 4] = [
    ("block", "display:block"),
    (
        "grid",
        "display:grid;grid-template:100px/100px;height:100px",
    ),
    ("row flex", "display:flex;height:100px"),
    (
        "column flex",
        "display:flex;flex-direction:column;height:200px",
    ),
];

#[test]
fn a_keyword_width_of_an_image_takes_the_height_through_the_ratio() {
    // `min-content`, `max-content` and `fit-content` size a replaced box as
    // `auto` does, so a 100px height gives the 25x50 image a 50px width.
    for width in ["min-content", "max-content", "fit-content"] {
        for (name, container) in KEYWORD_WIDTH_CONTAINERS {
            let css = format!("display:block;width:{width};height:100px");
            assert_eq!(
                image_in_container(container, &css),
                (50.0, 100.0),
                "{name} {width}"
            );
        }
    }
}

#[test]
fn a_keyword_width_of_an_image_keeps_its_natural_size_without_a_height() {
    // A row flex item is stretched to the line, which is the next test.
    for width in ["min-content", "max-content", "fit-content"] {
        for (name, container) in KEYWORD_WIDTH_CONTAINERS {
            if name == "row flex" {
                continue;
            }
            let css = format!("display:block;width:{width}");
            assert_eq!(
                image_in_container(container, &css),
                (25.0, 50.0),
                "{name} {width}"
            );
        }
    }
}

#[test]
fn a_keyword_width_is_not_stretched_in_a_grid() {
    // Only an `auto` size is stretched; a keyword width is not `auto`.
    for width in ["min-content", "max-content", "fit-content"] {
        let css = format!("justify-self:stretch;width:{width}");
        assert_eq!(
            image_in_container(KEYWORD_WIDTH_CONTAINERS[1].1, &css),
            (25.0, 50.0),
            "{width}"
        );
    }
}

#[test]
fn a_keyword_width_is_not_stretched_in_a_column_flex_container() {
    for width in ["min-content", "max-content", "fit-content"] {
        for align in [
            "align-self:stretch",
            "justify-self:stretch",
            "align-self:normal",
        ] {
            let css = format!("{align};width:{width}");
            assert_eq!(
                image_in_container(KEYWORD_WIDTH_CONTAINERS[3].1, &css),
                (25.0, 50.0),
                "{width} {align}"
            );
        }
    }
}

#[test]
fn a_keyword_width_leaves_the_stretched_height_of_a_row_flex_item_to_the_ratio() {
    // The cross size of a row flex item stretches to the 100px line; the width
    // follows through the ratio as it does for `auto`.
    for width in ["auto", "min-content", "max-content", "fit-content"] {
        let css = format!("width:{width}");
        assert_eq!(
            image_in_container(KEYWORD_WIDTH_CONTAINERS[2].1, &css),
            (50.0, 100.0),
            "{width}"
        );
    }
}

#[test]
fn a_keyword_width_with_a_percentage_height_takes_the_ratio_from_the_resolved_height() {
    for width in ["min-content", "max-content", "fit-content"] {
        let css = format!("width:{width};height:100%");
        assert_eq!(
            image_in_container(KEYWORD_WIDTH_CONTAINERS[1].1, &css),
            (50.0, 100.0),
            "grid {width}"
        );
        assert_eq!(
            image_in_container(KEYWORD_WIDTH_CONTAINERS[3].1, &css),
            (100.0, 200.0),
            "column flex {width}"
        );
    }
}

#[test]
fn a_keyword_width_with_a_function_limit_still_follows_an_auto_width() {
    // `fit-content(<length>)` is not one of the keywords that stop a stretch.
    assert_eq!(
        image_in_container(
            KEYWORD_WIDTH_CONTAINERS[1].1,
            "justify-self:stretch;width:fit-content(80px)"
        ),
        (100.0, 200.0)
    );
}

/// Lay out one child `tag` with `css` in a `<div>` with `container_css`, in the
/// SVG namespace for `svg` with a `viewBox` of 2:1, and return its inline
/// offset and its size. An `<img>` has a 25x50 natural size.
fn child_in_container(container_css: &str, tag: &str, css: &str) -> (f32, (f32, f32)) {
    let mut child = 0;
    let doc = laid_out(|doc, body| {
        let container = doc.append_element(Some(body), "div", Some(container_css));
        child = doc.append_element(Some(container), tag, Some(css));
        if tag == "svg" {
            doc.set_element_namespace(child, Some("http://www.w3.org/2000/svg".into()));
            doc.set_element_attributes(child, vec![("viewBox".into(), "0 0 2 1".into())]);
        }
    });
    (doc.nodes[child].layout.location.x, size(&doc, child))
}

#[test]
fn a_stretched_keyword_width_is_aligned_at_the_start_of_the_inline_axis() {
    let grid = KEYWORD_WIDTH_CONTAINERS[1].1;
    let column = KEYWORD_WIDTH_CONTAINERS[3].1;
    for width in ["min-content", "max-content", "fit-content"] {
        for (container, align) in [(grid, "justify-self"), (column, "align-self")] {
            let css = format!("{align}:stretch;width:{width}");
            assert_eq!(
                child_in_container(&format!("{container};direction:rtl"), "img", &css),
                (75.0, (25.0, 50.0)),
                "rtl {align} {width}"
            );
            assert_eq!(
                child_in_container(container, "img", &css),
                (0.0, (25.0, 50.0)),
                "{align} {width}"
            );
        }
    }
}

#[test]
fn a_keyword_width_keeps_the_alignment_it_is_given() {
    let grid = KEYWORD_WIDTH_CONTAINERS[1].1;
    assert_eq!(
        child_in_container(grid, "img", "justify-self:end;width:max-content"),
        (75.0, (25.0, 50.0))
    );
    assert_eq!(
        child_in_container(grid, "img", "justify-self:center;width:max-content"),
        (37.5, (25.0, 50.0))
    );
}

#[test]
fn an_inline_image_with_a_keyword_width_is_blockified_in_a_grid() {
    assert_eq!(
        child_in_container(KEYWORD_WIDTH_CONTAINERS[1].1, "img", "width:max-content"),
        (0.0, (25.0, 50.0))
    );
}

#[test]
fn auto_margins_center_an_image_with_a_keyword_width() {
    for (name, container) in [
        ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
        ("column flex", KEYWORD_WIDTH_CONTAINERS[3].1),
    ] {
        let css = "margin-left:auto;margin-right:auto;width:max-content";
        assert_eq!(
            child_in_container(container, "img", css),
            (37.5, (25.0, 50.0)),
            "{name}"
        );
    }
}

#[test]
fn an_absolutely_positioned_image_with_a_keyword_width_keeps_its_natural_size() {
    let container = "display:grid;width:100px;height:100px;position:relative;justify-items:stretch";
    assert_eq!(
        child_in_container(container, "img", "position:absolute;width:max-content"),
        (0.0, (25.0, 50.0))
    );
    assert_eq!(
        child_in_container(
            container,
            "img",
            "position:absolute;left:0;right:0;width:max-content"
        ),
        (0.0, (25.0, 50.0))
    );
}

#[test]
fn an_svg_without_a_natural_size_fills_the_space_for_a_keyword_width() {
    // Browsers size a viewBox-only SVG as they do with `auto`, whatever the
    // keyword, so it is stretched.
    for width in ["auto", "min-content", "max-content", "fit-content"] {
        for (name, container, align) in [
            ("grid", KEYWORD_WIDTH_CONTAINERS[1].1, "justify-self"),
            ("column flex", KEYWORD_WIDTH_CONTAINERS[3].1, "align-self"),
        ] {
            let css = format!("{align}:stretch;width:{width}");
            assert_eq!(
                child_in_container(container, "svg", &css),
                (0.0, (100.0, 50.0)),
                "{name} {width}"
            );
        }
    }
}

#[test]
fn a_min_content_width_of_a_vertical_image_is_not_stretched_on_both_grid_axes() {
    // A box with a vertical writing mode takes its sizes from its logical
    // properties, so the `auto` handling of a replaced box does not apply to it
    // and the ratio is kept.
    assert_eq!(
        child_in_container(
            KEYWORD_WIDTH_CONTAINERS[1].1,
            "img",
            "writing-mode:vertical-rl;justify-self:stretch;width:min-content"
        ),
        (0.0, (25.0, 50.0))
    );
}

#[test]
fn replaced_content_without_a_natural_size_treats_a_keyword_width_as_auto() {
    // It fills the space an `auto` width fills; there is no natural size to
    // keep.
    for (name, container, align) in [
        ("grid", KEYWORD_WIDTH_CONTAINERS[1].1, "justify-self"),
        ("column flex", KEYWORD_WIDTH_CONTAINERS[3].1, "align-self"),
    ] {
        let auto = child_in_container(container, "iframe", &format!("{align}:stretch"));
        assert_eq!(auto.1.0, 100.0, "{name} auto");
        for width in ["min-content", "max-content", "fit-content"] {
            let css = format!("{align}:stretch;width:{width}");
            assert_eq!(
                child_in_container(container, "iframe", &css),
                auto,
                "{name} {width}"
            );
        }
    }
}

#[test]
fn a_keyword_width_is_not_stretched_when_the_height_is_stretched_in_a_grid() {
    // An `auto` width and height both stretched fill the area and drop the
    // ratio; a keyword width is not `auto`, so only the height is stretched and
    // the width follows through the ratio.
    for width in ["min-content", "max-content", "fit-content"] {
        let css = format!("justify-self:stretch;align-self:stretch;width:{width}");
        assert_eq!(
            image_in_container(KEYWORD_WIDTH_CONTAINERS[1].1, &css),
            (50.0, 100.0),
            "{width}"
        );
    }
    assert_eq!(
        image_in_container(
            KEYWORD_WIDTH_CONTAINERS[1].1,
            "justify-self:stretch;align-self:stretch"
        ),
        (100.0, 100.0)
    );
}

#[test]
fn a_size_contained_image_with_a_keyword_width_is_not_stretched() {
    // Size containment leaves the image without a natural size, which an
    // `auto` width fills the space for; a keyword width is not `auto`, so the
    // box collapses as in browsers.
    for (name, container, align) in [
        ("grid", KEYWORD_WIDTH_CONTAINERS[1].1, "justify-self"),
        ("column flex", KEYWORD_WIDTH_CONTAINERS[3].1, "align-self"),
    ] {
        for (contain, height) in [("size", 0.0), ("inline-size", 50.0)] {
            let css = format!("contain:{contain};{align}:stretch;width:auto");
            assert_eq!(
                child_in_container(container, "img", &css),
                (0.0, (100.0, height)),
                "{name} {contain} auto"
            );
            for width in ["min-content", "max-content", "fit-content"] {
                let css = format!("contain:{contain};{align}:stretch;width:{width}");
                assert_eq!(
                    child_in_container(container, "img", &css),
                    (0.0, (0.0, height)),
                    "{name} {contain} {width}"
                );
            }
        }
    }
}

/// The inline and block offsets and the size of the only `<img>` in a
/// container, for an alignment along the block axis.
fn image_box_in_container(container_css: &str, image_css: &str) -> ((f32, f32), (f32, f32)) {
    let mut image = 0;
    let doc = laid_out(|doc, body| {
        let container = doc.append_element(Some(body), "div", Some(container_css));
        image = doc.append_element(Some(container), "img", Some(image_css));
    });
    let location = doc.nodes[image].layout.location;
    ((location.x, location.y), size(&doc, image))
}

#[test]
fn a_keyword_height_is_not_stretched_in_a_grid_or_a_row_flex_container() {
    // Only an `auto` height is stretched; the keyword sizes the box as `auto`
    // does, so the image keeps its natural size.
    for height in ["min-content", "max-content", "fit-content"] {
        for (name, container) in [
            ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
            ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
        ] {
            for align in ["", "align-self:stretch;", "align-self:normal;"] {
                let css = format!("{align}height:{height}");
                assert_eq!(
                    image_in_container(container, &css),
                    (25.0, 50.0),
                    "{name} {css}"
                );
            }
        }
    }
}

#[test]
fn an_auto_height_is_still_stretched_in_a_row_flex_container() {
    assert_eq!(
        image_in_container(KEYWORD_WIDTH_CONTAINERS[2].1, "height:auto"),
        (50.0, 100.0)
    );
}

#[test]
fn a_keyword_height_takes_the_height_of_a_stretched_width_through_the_ratio() {
    for height in ["min-content", "max-content", "fit-content"] {
        for align in [
            "justify-self:stretch",
            "justify-self:stretch;align-self:stretch",
        ] {
            let css = format!("{align};height:{height}");
            assert_eq!(
                image_in_container(KEYWORD_WIDTH_CONTAINERS[1].1, &css),
                (100.0, 200.0),
                "{css}"
            );
        }
    }
}

#[test]
fn a_keyword_height_follows_the_ratio_of_a_definite_width() {
    for height in ["min-content", "max-content", "fit-content"] {
        for (name, container) in [
            ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
            ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
        ] {
            let css = format!("width:20px;height:{height}");
            assert_eq!(
                image_in_container(container, &css),
                (20.0, 40.0),
                "{name} {height}"
            );
        }
    }
}

#[test]
fn a_keyword_height_keeps_the_alignment_it_is_given() {
    for (name, container) in [
        ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
        ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
    ] {
        for (align, top) in [
            ("align-self:end", 50.0),
            ("align-self:center", 25.0),
            ("align-self:start", 0.0),
        ] {
            let css = format!("{align};height:max-content");
            assert_eq!(
                image_box_in_container(container, &css),
                ((0.0, top), (25.0, 50.0)),
                "{name} {align}"
            );
        }
    }
}

#[test]
fn a_stretched_keyword_height_is_aligned_at_the_start_of_the_block_axis() {
    // The stretch a keyword height gives up falls back to the start edge, which
    // `wrap-reverse` moves to the bottom of a row flex container.
    assert_eq!(
        image_box_in_container(
            "display:flex;flex-wrap:wrap-reverse;height:100px",
            "height:max-content"
        ),
        ((0.0, 50.0), (25.0, 50.0))
    );
    assert_eq!(
        image_box_in_container(
            KEYWORD_WIDTH_CONTAINERS[1].1,
            "margin-top:10px;align-self:stretch;height:max-content"
        ),
        ((0.0, 10.0), (25.0, 50.0))
    );
}

#[test]
fn a_keyword_height_function_limit_still_follows_an_auto_height() {
    // `fit-content(<length>)` is not one of the keywords that stop a stretch.
    assert_eq!(
        image_in_container(
            KEYWORD_WIDTH_CONTAINERS[1].1,
            "align-self:stretch;height:fit-content(80px)"
        ),
        (50.0, 100.0)
    );
    assert_eq!(
        image_in_container(KEYWORD_WIDTH_CONTAINERS[2].1, "height:fit-content(80px)"),
        (50.0, 100.0)
    );
}

#[test]
fn an_svg_without_a_natural_size_is_not_stretched_by_a_keyword_height() {
    // A viewBox-only SVG is stretched by `align-self` when its height is
    // `auto`, so the keyword is what stops it.
    for height in ["min-content", "max-content", "fit-content"] {
        for (name, container) in [
            ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
            ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
        ] {
            let css = format!("align-self:stretch;height:{height}");
            assert_eq!(
                child_in_container(container, "svg", &css),
                (0.0, (100.0, 50.0)),
                "{name} {height}"
            );
        }
    }
}

#[test]
fn a_keyword_height_is_not_stretched_in_a_column_flex_container() {
    // The height is the main size of the item there: it takes the width the
    // container stretches through the ratio, as `auto` does.
    for height in ["auto", "min-content", "max-content", "fit-content"] {
        let css = format!("height:{height}");
        assert_eq!(
            image_in_container(KEYWORD_WIDTH_CONTAINERS[3].1, &css),
            (100.0, 200.0),
            "{height}"
        );
    }
}

#[test]
fn a_keyword_height_is_aligned_at_the_start_of_a_reversed_row() {
    assert_eq!(
        image_box_in_container(
            "display:flex;flex-direction:row-reverse;height:100px",
            "height:max-content"
        ),
        ((75.0, 0.0), (25.0, 50.0))
    );
}

#[test]
fn an_absolutely_positioned_image_with_a_keyword_height_keeps_its_natural_size() {
    for (name, container) in [
        ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
        ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
    ] {
        for css in [
            "position:absolute;top:0;bottom:0;height:max-content",
            "position:absolute;align-self:stretch;height:max-content",
        ] {
            assert_eq!(
                image_box_in_container(&format!("{container};position:relative"), css),
                ((0.0, 0.0), (25.0, 50.0)),
                "{name} {css}"
            );
        }
    }
}

#[test]
fn a_vertical_image_with_a_keyword_height_is_not_stretched() {
    // The physical height is the inline size of a box with a vertical writing
    // mode; its block size is the physical width, which does not stop a stretch
    // of the height.
    for (name, container) in [
        ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
        ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
    ] {
        for (declaration, size) in [
            ("height:max-content", (25.0, 50.0)),
            ("inline-size:min-content", (25.0, 50.0)),
            ("block-size:max-content", (50.0, 100.0)),
            ("width:max-content", (50.0, 100.0)),
        ] {
            let css = format!("writing-mode:vertical-rl;align-self:stretch;{declaration}");
            assert_eq!(image_in_container(container, &css), size, "{name} {css}");
        }
    }
}

#[test]
fn a_block_with_a_keyword_height_is_not_stretched_either() {
    // Only an `auto` height is stretched, whatever the box is: the block sizes
    // itself by its 40px child.
    for height in ["min-content", "max-content", "fit-content"] {
        for (name, container) in [
            ("grid", KEYWORD_WIDTH_CONTAINERS[1].1),
            ("row flex", KEYWORD_WIDTH_CONTAINERS[2].1),
        ] {
            let mut block = 0;
            let doc = laid_out(|doc, body| {
                let container = doc.append_element(Some(body), "div", Some(container));
                block = doc.append_element(
                    Some(container),
                    "div",
                    Some(&format!("align-self:stretch;width:30px;height:{height}")),
                );
                doc.append_element(Some(block), "div", Some("height:40px"));
            });
            assert_eq!(size(&doc, block), (30.0, 40.0), "{name} {height}");
        }
    }
    let mut block = 0;
    let doc = laid_out(|doc, body| {
        let container = doc.append_element(Some(body), "div", Some(KEYWORD_WIDTH_CONTAINERS[2].1));
        block = doc.append_element(Some(container), "div", Some("width:30px"));
        doc.append_element(Some(block), "div", Some("height:40px"));
    });
    assert_eq!(size(&doc, block), (30.0, 100.0));
}

/// Size of a 20x10 `<img>` that a resolver gave no natural ratio, with `css`.
fn image_without_natural_ratio(css: &str) -> (f32, f32) {
    let mut image = 0;
    let doc = laid_out_with_natural_box((20.0, 10.0), false, |doc, body| {
        image = doc.append_element(
            Some(body),
            "img",
            Some(format!("display:block;{css}").as_str()),
        );
    });
    size(&doc, image)
}

#[test]
fn an_auto_ratio_prefers_the_ratio_of_the_natural_size_when_the_resolver_gave_none() {
    assert_eq!(
        image_without_natural_ratio("width:100px;aspect-ratio:auto 1/1"),
        (100.0, 50.0)
    );
}

#[test]
fn an_authored_ratio_replaces_the_ratio_of_the_natural_size_when_the_resolver_gave_none() {
    assert_eq!(
        image_without_natural_ratio("width:100px;aspect-ratio:1/1"),
        (100.0, 100.0)
    );
}

/// The border-box size of an inline SVG that has a `viewBox` and no size
/// attributes, laid out in a block container styled `container_css`.
fn ratio_only_svg_in_container(container_css: &str, view_box: &str, svg_css: &str) -> (f32, f32) {
    let mut svg = 0;
    let doc = laid_out(|doc, body| {
        let container = doc.append_element(Some(body), "div", Some(container_css));
        svg = doc.append_element(Some(container), "svg", Some(svg_css));
        doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
        doc.set_element_attributes(svg, vec![("viewBox".into(), view_box.into())]);
    });
    size(&doc, svg)
}

#[test]
fn a_ratio_only_svg_in_block_flow_is_brought_within_the_bounds_of_both_sides() {
    // The width of an svg that has a ratio only is the room its container
    // offers; the `min-*` and `max-*` sizes of both sides then bring it within
    // the constraint table of CSS 2.1 section 10.4. The sizes are those
    // Chromium gives.
    let wide = "0 0 2 1";
    let square = "0 0 1 1";
    let block = |width: u32| format!("display:block;width:{width}px");
    for (container, view_box, css, expected) in [
        (
            block(50),
            wide,
            "max-width:100px;min-height:100px",
            (100.0, 100.0),
        ),
        (block(50), square, "min-height:100px", (100.0, 100.0)),
        (
            block(50),
            wide,
            "max-width:60px;min-height:100px",
            (60.0, 100.0),
        ),
        (block(100), wide, "max-height:20px", (40.0, 20.0)),
        (
            block(300),
            wide,
            "max-height:50px;min-width:10px",
            (100.0, 50.0),
        ),
        (block(300), wide, "max-height:50px", (100.0, 50.0)),
        (block(50), wide, "min-width:80px", (80.0, 40.0)),
        (block(200), wide, "max-width:80px", (80.0, 40.0)),
        (block(500), wide, "", (500.0, 250.0)),
        (block(500), wide, "min-height:300px", (600.0, 300.0)),
        (block(500), wide, "max-height:100px", (200.0, 100.0)),
        (
            block(500),
            wide,
            "min-height:300px;max-width:400px",
            (400.0, 300.0),
        ),
        // The room is what the margins leave; padding counts in a border-box.
        (
            block(100),
            wide,
            "margin:0 10px;min-height:100px",
            (200.0, 100.0),
        ),
        (
            block(100),
            wide,
            "padding:10px;min-height:100px;box-sizing:border-box",
            (180.0, 100.0),
        ),
        // A min above its max wins.
        (
            block(200),
            wide,
            "min-width:80px;max-width:50px",
            (80.0, 40.0),
        ),
        (
            block(200),
            wide,
            "min-height:80px;max-height:50px",
            (160.0, 80.0),
        ),
        // An authored width is not moved: only the height follows the bounds.
        (
            block(50),
            wide,
            "width:100%;min-height:100px",
            (50.0, 100.0),
        ),
        // Flex items, floats, grid items and absolutely positioned boxes take
        // the same room.
        (
            "display:flex;width:500px".to_string(),
            wide,
            "",
            (500.0, 250.0),
        ),
        (
            "display:flex;width:500px".to_string(),
            wide,
            "max-width:100px",
            (100.0, 50.0),
        ),
        (
            "display:flex;flex-direction:column;width:500px".to_string(),
            wide,
            "",
            (500.0, 250.0),
        ),
        (
            block(500),
            wide,
            "float:left;min-height:300px",
            (600.0, 300.0),
        ),
        (
            "display:grid;width:500px".to_string(),
            wide,
            "min-height:300px",
            (600.0, 300.0),
        ),
        (
            "display:block;position:relative;width:500px;height:400px".to_string(),
            wide,
            "position:absolute;min-height:300px",
            (600.0, 300.0),
        ),
    ] {
        assert_eq!(
            ratio_only_svg_in_container(&container, view_box, css),
            expected,
            "{container} {view_box} {css}"
        );
    }
}

#[test]
fn a_ratio_only_svg_in_a_shrink_to_fit_container_stays_empty() {
    // The room is not known there, so a width of `auto` contributes nothing.
    for css in ["", "max-width:100px", "max-height:100px"] {
        assert_eq!(
            ratio_only_svg_in_container("display:inline-block", "0 0 2 1", css),
            (0.0, 0.0),
            "{css}"
        );
    }
}

#[test]
fn a_keyword_min_or_max_width_of_a_ratio_only_svg_takes_the_width_its_height_gives() {
    // The width an authored height gives through the ratio is not limited by
    // the numeric bounds of the side it lands on, in either direction.
    let container = "display:block;width:100px;height:100px";
    for (view_box, css, expected) in [
        (
            "0 0 1 1",
            "min-width:max-content;max-width:50px;height:100%",
            (100.0, 100.0),
        ),
        (
            "0 0 1 1",
            "min-height:max-content;max-height:50px;width:100%",
            (100.0, 100.0),
        ),
        ("0 0 1 1", "min-width:max-content;height:50%", (50.0, 50.0)),
        (
            "0 0 2 1",
            "max-width:max-content;height:60px;min-width:10px",
            (120.0, 60.0),
        ),
    ] {
        assert_eq!(
            ratio_only_svg_in_container(container, view_box, css),
            expected,
            "{view_box} {css}"
        );
    }
}

#[test]
fn a_keyword_min_or_max_size_of_an_image_takes_the_size_its_other_side_gives() {
    // The 25x50 image gets the width its authored height gives through the
    // ratio, and the height its authored width gives, whatever the numeric
    // bound on the side the keyword is on says.
    let container = "display:block;width:200px";
    for (css, expected) in [
        (
            "min-width:max-content;max-width:10px;height:100px",
            (50.0, 100.0),
        ),
        (
            "min-height:max-content;max-height:10px;width:100px",
            (100.0, 200.0),
        ),
        (
            "max-width:max-content;min-width:80px;height:100px",
            (80.0, 100.0),
        ),
        ("max-width:max-content;height:100px", (50.0, 100.0)),
    ] {
        assert_eq!(image_in_container(container, css), expected, "{css}");
    }
}

/// Lay out one 20x50 `<img>` with `css` in the 100px wide body and return its
/// size and node.
fn natural_20x50_image(css: &str) -> (Document, usize) {
    let mut image = 0;
    let doc = laid_out_with_natural_size((20.0, 50.0), |doc, body| {
        image = doc.append_element(Some(body), "img", Some(css));
    });
    (doc, image)
}

#[test]
fn a_border_box_aspect_ratio_relates_the_border_boxes_of_an_image() {
    // Sizes a browser gives. The ratio of a `border-box` image relates its
    // border box, so the padding on one side only does not skew it.
    let padded = "aspect-ratio:1;padding-right:80px;padding-bottom:50px;box-sizing:border-box";
    for (css, expected) in [
        (format!("display:block;{padded}"), (100.0, 100.0)),
        (padded.to_string(), (100.0, 100.0)),
        (
            format!("display:block;{padded};width:100px"),
            (100.0, 100.0),
        ),
        (
            "aspect-ratio:1;padding-right:20px;padding-bottom:40px;box-sizing:border-box;\
             height:100px"
                .to_string(),
            (100.0, 100.0),
        ),
        (
            "display:block;aspect-ratio:1;padding:4px 10px;box-sizing:border-box;width:60px"
                .to_string(),
            (60.0, 60.0),
        ),
    ] {
        let (doc, image) = natural_20x50_image(&css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn a_content_box_aspect_ratio_relates_the_content_boxes_of_an_image() {
    let (doc, image) = natural_20x50_image(
        "display:block;aspect-ratio:1;padding-right:80px;padding-bottom:50px;width:100px",
    );

    // The 100px width is the content box: 100x100 plus the padding.
    assert_eq!(size(&doc, image), (180.0, 150.0));
}

#[test]
fn a_ratio_the_image_states_replaces_the_natural_ratio_under_min_and_max_sizes() {
    // The natural 20x50 size keeps its width, and the 1:1 ratio the image
    // states gives the height, until a `min-*` or `max-*` size moves them.
    for (css, expected) in [
        ("display:block;aspect-ratio:1", (20.0, 20.0)),
        (
            "display:block;aspect-ratio:1;min-height:100px",
            (100.0, 100.0),
        ),
        ("display:block;aspect-ratio:1;max-height:10px", (10.0, 10.0)),
    ] {
        let (doc, image) = natural_20x50_image(css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn min_and_max_sizes_move_a_border_box_image_along_its_border_box_ratio() {
    for (css, expected) in [
        (
            "display:block;aspect-ratio:2;padding-right:40px;padding-bottom:20px;\
             box-sizing:border-box;min-height:100px",
            (200.0, 100.0),
        ),
        (
            "display:block;aspect-ratio:1;padding:4px 10px;box-sizing:border-box;min-width:80px",
            (80.0, 80.0),
        ),
        (
            "display:block;aspect-ratio:3;padding-right:10px;padding-bottom:5px;\
             box-sizing:border-box;max-width:60px",
            (30.0, 10.0),
        ),
    ] {
        let (doc, image) = natural_20x50_image(css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn an_auto_ratio_of_an_image_relates_the_content_box_of_its_natural_ratio() {
    // `auto 2` keeps the natural 2:5 ratio of the content box, whatever the
    // box-sizing and padding: 20x50 content plus 80x50 of padding.
    let (doc, image) = natural_20x50_image(
        "display:block;aspect-ratio:auto 2;padding-right:80px;padding-bottom:50px;\
         box-sizing:border-box",
    );
    assert_eq!(size(&doc, image), (100.0, 100.0));
}

#[test]
fn a_degenerate_aspect_ratio_keeps_the_content_box_ratio_of_the_natural_ratio() {
    // `aspect-ratio: 0` behaves as `auto`: the natural 2:5 ratio of the
    // content box, 20x50 of content plus 80x50 of padding.
    for ratio in ["0", "0 / 1", "1 / 0"] {
        let (doc, image) = natural_20x50_image(&format!(
            "display:block;width:100px;aspect-ratio:{ratio};padding-right:80px;\
             padding-bottom:50px;box-sizing:border-box"
        ));
        assert_eq!(size(&doc, image), (100.0, 100.0), "{ratio}");
    }
}

#[test]
fn the_ratio_of_an_aspect_ratio_relates_the_content_box_when_it_is_auto_or_natural() {
    // `ratio_in_content_box` is set for `auto` with a `<ratio>` or with the
    // natural ratio of the image, and for no other `aspect-ratio`.
    for (css, expected) in [
        ("aspect-ratio:auto", true),
        ("aspect-ratio:auto 2", true),
        ("aspect-ratio:0", true),
        ("aspect-ratio:2", false),
    ] {
        let (doc, image) = natural_20x50_image(css);
        assert_eq!(doc.nodes[image].ratio_in_content_box, expected, "{css}");
    }
    // Without a natural ratio, `auto` alone leaves nothing to relate.
    let mut image = 0;
    let doc = laid_out_with_natural_box((0.0, 0.0), false, |doc, body| {
        image = doc.append_element(Some(body), "img", Some("aspect-ratio:auto"));
    });
    assert!(!doc.nodes[image].ratio_in_content_box);
}

#[test]
fn a_border_box_image_is_sized_by_its_border_box_ratio_as_an_inline_block() {
    // The same sizes as for a block, as a shrink-to-fit box.
    for (css, expected) in [
        (
            "display:inline-block;aspect-ratio:1;padding-right:80px;padding-bottom:50px;\
             box-sizing:border-box",
            (100.0, 100.0),
        ),
        (
            "display:inline-block;aspect-ratio:1;padding-right:20px;padding-bottom:40px;\
             box-sizing:border-box;height:100px",
            (100.0, 100.0),
        ),
        (
            "display:inline-block;aspect-ratio:1;padding-right:20px;padding-bottom:10px;\
             box-sizing:border-box;min-width:80px",
            (80.0, 80.0),
        ),
        (
            "display:inline-block;aspect-ratio:1;padding-right:20px;padding-bottom:10px;\
             box-sizing:border-box;max-width:30px",
            (30.0, 30.0),
        ),
        (
            "display:inline-block;aspect-ratio:1;padding-right:20px;padding-bottom:10px;\
             box-sizing:border-box;max-height:30px",
            (30.0, 30.0),
        ),
        // The natural ratio relates the content box.
        (
            "display:inline-block;padding-right:80px;padding-bottom:50px;box-sizing:border-box",
            (100.0, 100.0),
        ),
        (
            "display:inline-block;padding-right:20px;padding-bottom:40px;\
             box-sizing:border-box;height:100px",
            (44.0, 100.0),
        ),
    ] {
        let (doc, image) = natural_20x50_image(css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn a_keyword_min_or_max_size_of_a_border_box_image_transfers_along_its_border_box_ratio() {
    // A 2:1 border box 100px high is 200px wide; one 100px wide is 50px high.
    // The content box ratio would give the 40px of padding at the right and
    // the 10px at the bottom a share of the transfer.
    let padded = "display:block;aspect-ratio:2;padding-right:40px;padding-bottom:10px;\
                  box-sizing:border-box";
    for (css, expected) in [
        (
            format!("{padded};height:100px;min-width:min-content"),
            (200.0, 100.0),
        ),
        (
            format!("{padded};width:100px;min-height:min-content"),
            (100.0, 50.0),
        ),
        (
            format!("{padded};width:100px;max-height:max-content"),
            (100.0, 50.0),
        ),
    ] {
        let (doc, image) = natural_20x50_image(&css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn a_border_box_aspect_ratio_relates_the_border_boxes_of_a_ratio_only_svg() {
    // Sizes a browser gives for a 20:50 `viewBox`: the `aspect-ratio` replaces
    // its ratio, and relates the border box of a `border-box` svg.
    let container = "display:block;width:100px";
    let padded = "display:block;padding-right:30px;padding-bottom:10px;box-sizing:border-box";
    for (css, expected) in [
        (format!("{padded};aspect-ratio:1;height:60px"), (60.0, 60.0)),
        (
            format!("{padded};aspect-ratio:2;height:60px"),
            (120.0, 60.0),
        ),
        (format!("{padded};aspect-ratio:2;width:60px"), (60.0, 30.0)),
    ] {
        assert_eq!(
            ratio_only_svg_in_container(container, "0 0 20 50", &css),
            expected,
            "{css}"
        );
    }
}

#[test]
fn a_block_level_image_with_an_authored_height_takes_its_width_from_the_content_box_height() {
    // Sizes a browser gives for a 20x50 image. The height is authored and the
    // width is `auto`, so the width follows from the content height through
    // the natural ratio whatever side the padding is on.
    for (css, expected) in [
        ("height:100px", (40.0, 100.0)),
        ("height:100px;padding-right:80px", (120.0, 100.0)),
        ("height:100px;padding-left:80px", (120.0, 100.0)),
        (
            "height:100px;padding-right:80px;box-sizing:border-box",
            (120.0, 100.0),
        ),
        (
            "height:100px;padding-left:80px;box-sizing:border-box",
            (120.0, 100.0),
        ),
        (
            "height:100px;padding-right:80px;padding-bottom:50px;box-sizing:border-box",
            (100.0, 100.0),
        ),
        ("height:100px;padding-bottom:50px", (40.0, 150.0)),
        (
            "height:100px;padding-bottom:50px;box-sizing:border-box",
            (20.0, 100.0),
        ),
        ("height:100px;padding-top:30px", (40.0, 130.0)),
        (
            "height:100px;padding-top:30px;box-sizing:border-box",
            (28.0, 100.0),
        ),
        ("height:100px;padding:10px", (60.0, 120.0)),
        (
            "height:100px;padding:10px;box-sizing:border-box",
            (52.0, 100.0),
        ),
        ("height:100px;border:10px solid red", (60.0, 120.0)),
        (
            "height:100px;border:10px solid red;box-sizing:border-box",
            (52.0, 100.0),
        ),
        (
            "height:100px;padding:0 30px;box-sizing:border-box",
            (100.0, 100.0),
        ),
        // A height bound moves the content height, and the width follows it.
        ("height:100px;max-height:60px", (24.0, 60.0)),
        (
            "height:100px;max-height:60px;padding-right:80px",
            (104.0, 60.0),
        ),
        (
            "height:30px;min-height:60px;padding-right:80px",
            (104.0, 60.0),
        ),
        (
            "height:100px;max-height:60px;padding:5px;box-sizing:border-box",
            (30.0, 60.0),
        ),
    ] {
        let css = format!("display:block;{css}");
        let (doc, image) = natural_20x50_image(&css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn a_width_bound_does_not_change_the_authored_height_of_a_block_level_image() {
    // Sizes a browser gives for a 20x50 image. A `min-width` or `max-width`
    // clamps the width the authored height gives, and the height stays; a
    // bound on one side does not carry across the ratio onto a side with a
    // length of its own either.
    for (css, expected) in [
        ("display:block;height:100px;max-width:30px", (30.0, 100.0)),
        ("display:block;height:100px;min-width:60px", (60.0, 100.0)),
        (
            "display:block;height:100px;max-width:30px;padding:0 5px",
            (40.0, 100.0),
        ),
        (
            "display:block;height:100px;min-width:60px;padding:0 5px",
            (70.0, 100.0),
        ),
        (
            "display:block;height:100px;max-width:30px;padding:0 5px;box-sizing:border-box",
            (30.0, 100.0),
        ),
        (
            "display:block;height:20px;min-width:60px;padding:0 10px",
            (80.0, 20.0),
        ),
        (
            "display:block;height:100px;max-height:60px;max-width:10px",
            (10.0, 60.0),
        ),
        (
            "display:block;height:calc(50px + 50px);max-width:30px",
            (30.0, 100.0),
        ),
        ("display:block;width:100px;max-height:30px", (100.0, 30.0)),
        ("display:block;width:10px;min-height:300px", (10.0, 300.0)),
        (
            "display:block;width:100px;min-width:60px;max-height:30px",
            (100.0, 30.0),
        ),
        (
            "display:block;width:100px;max-height:30px;padding:5px;box-sizing:border-box",
            (100.0, 30.0),
        ),
        // A percentage is an authored size too; the body is 100px wide.
        ("display:block;width:50%;max-height:30px", (50.0, 30.0)),
        // A border or a `border-box` sizing moves the width the bound clamps,
        // not the height.
        (
            "display:block;height:100px;min-width:60px;padding:0 5px;box-sizing:border-box",
            (60.0, 100.0),
        ),
        (
            "display:block;height:100px;min-width:30px;padding:0 20px;box-sizing:border-box",
            (80.0, 100.0),
        ),
        (
            "display:block;height:100px;min-width:60px;border:5px solid red",
            (70.0, 110.0),
        ),
        (
            "display:block;height:100px;max-width:30px;border:5px solid red;box-sizing:border-box",
            (30.0, 100.0),
        ),
        (
            "display:block;height:100px;aspect-ratio:1;min-width:150px;padding:0 10px",
            (170.0, 100.0),
        ),
        (
            "display:block;width:100px;max-height:30px;border:5px solid red",
            (110.0, 40.0),
        ),
        ("float:left;height:100px;max-width:30px", (30.0, 100.0)),
        (
            "display:inline-block;height:100px;max-width:30px",
            (30.0, 100.0),
        ),
    ] {
        let (doc, image) = natural_20x50_image(css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn a_keyword_min_or_max_size_takes_the_size_the_other_side_gives_through_the_ratio() {
    // Sizes a browser gives for a 20x50 image. The keyword size of a side
    // that nothing authored is the size the natural size takes within the
    // numeric bounds, along the ratio the box states; a side that is authored
    // across it gives the size through that ratio, also where both sides are
    // authored and sizing itself leaves the ratio out.
    for (css, expected) in [
        ("aspect-ratio:1;min-height:min-content", (20.0, 20.0)),
        ("aspect-ratio:2;min-height:max-content", (20.0, 10.0)),
        (
            "aspect-ratio:1;height:100px;max-height:min-content",
            (20.0, 20.0),
        ),
        (
            "aspect-ratio:2;height:100px;max-height:min-content",
            (20.0, 10.0),
        ),
        ("max-width:10px;min-height:min-content", (10.0, 25.0)),
        ("min-width:50px;max-height:min-content", (50.0, 125.0)),
        ("max-height:10px;min-width:min-content", (4.0, 10.0)),
        (
            "aspect-ratio:2;max-height:5px;min-width:min-content",
            (10.0, 5.0),
        ),
        (
            "aspect-ratio:2;max-width:10px;min-height:min-content",
            (10.0, 5.0),
        ),
        (
            "aspect-ratio:2;width:30px;height:100px;min-width:min-content",
            (200.0, 100.0),
        ),
        (
            "aspect-ratio:2;width:30px;height:100px;min-width:fit-content",
            (200.0, 100.0),
        ),
        (
            "aspect-ratio:2;width:30px;height:100px;max-height:min-content",
            (30.0, 15.0),
        ),
        (
            "aspect-ratio:1;width:30px;height:100px;max-height:min-content",
            (30.0, 30.0),
        ),
        (
            "aspect-ratio:2;width:300px;height:10px;min-height:max-content",
            (300.0, 150.0),
        ),
        // The natural ratio stays where the box states none.
        (
            "width:30px;height:100px;min-width:min-content",
            (40.0, 100.0),
        ),
        (
            "width:30px;height:100px;max-width:min-content",
            (30.0, 100.0),
        ),
        // Padding takes part in the ratio of a `border-box` box only.
        (
            "aspect-ratio:2;padding-right:40px;padding-bottom:10px;\
             box-sizing:border-box;min-height:min-content",
            (60.0, 30.0),
        ),
        (
            "aspect-ratio:2;padding-right:40px;padding-bottom:10px;min-height:min-content",
            (60.0, 20.0),
        ),
        (
            "padding-right:40px;padding-bottom:10px;box-sizing:border-box;height:100px;\
             max-width:min-content",
            (76.0, 100.0),
        ),
        (
            "aspect-ratio:2;padding-right:40px;padding-bottom:10px;box-sizing:border-box;\
             width:10px;height:100px;min-width:min-content",
            (200.0, 100.0),
        ),
        (
            "aspect-ratio:2;padding-right:40px;padding-bottom:10px;width:60px;height:100px;\
             min-width:min-content",
            (240.0, 110.0),
        ),
    ] {
        let css = format!("display:block;{css}");
        let (doc, image) = natural_20x50_image(&css);
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}

#[test]
fn an_image_with_an_authored_height_takes_the_same_width_in_any_parent() {
    // Sizes a browser gives for a 20x50 image: a flex or grid item, an inline
    // image, a float and a percentage height all take the width from the
    // content height, whatever the padding.
    for (parent, css, expected) in [
        (None, "height:100px;padding-right:80px", (120.0, 100.0)),
        (
            None,
            "height:100px;padding:10px;box-sizing:border-box",
            (52.0, 100.0),
        ),
        (
            None,
            "float:left;height:100px;padding-right:80px",
            (120.0, 100.0),
        ),
        (
            Some("display:flex;width:200px"),
            "height:100px;padding:10px",
            (60.0, 120.0),
        ),
        (
            Some("display:flex;width:200px;height:100px"),
            "",
            (40.0, 100.0),
        ),
        (
            Some("display:flex;width:200px;height:100px;align-items:flex-start"),
            "",
            (20.0, 50.0),
        ),
        (
            Some("display:grid;grid-template-columns:200px;justify-items:start"),
            "height:100px;padding:10px",
            (60.0, 120.0),
        ),
        (
            Some("display:grid;grid-template-columns:200px;grid-template-rows:100px"),
            "",
            (20.0, 50.0),
        ),
        (
            Some("height:200px"),
            "display:block;height:50%;padding-right:80px",
            (120.0, 100.0),
        ),
    ] {
        let mut image = 0;
        let doc = laid_out_with_natural_size((20.0, 50.0), |doc, body| {
            let parent = match parent {
                Some(parent_css) => doc.append_element(Some(body), "div", Some(parent_css)),
                None => body,
            };
            image = doc.append_element(Some(parent), "img", Some(css));
        });
        assert_eq!(size(&doc, image), expected, "{parent:?} {css}");
    }
}

#[test]
fn a_flex_item_image_takes_the_other_side_from_the_size_its_container_gives() {
    // Sizes a browser gives for a 20x50 image in a flex container 200px wide
    // and 100px high. A main size the container grows the item to, or a cross
    // size it stretches the item to, gives the other side through the ratio,
    // and the item cannot shrink below the main size a stretched cross size
    // gives it.
    let column = "display:flex;flex-direction:column;width:200px;height:100px";
    let column_start = format!("{column};align-items:flex-start");
    let row_start = "display:flex;width:200px;height:100px;align-items:flex-start";
    for (parent, css, expected) in [
        (column, "", (200.0, 500.0)),
        (column, "flex-grow:1", (200.0, 500.0)),
        (column, "align-self:stretch", (200.0, 500.0)),
        (column, "min-height:0", (200.0, 100.0)),
        (column, "min-height:300px", (200.0, 300.0)),
        (column, "max-height:60px", (200.0, 60.0)),
        (column, "flex-basis:content;max-height:60px", (200.0, 60.0)),
        (column, "width:50px", (50.0, 125.0)),
        (column, "aspect-ratio:1", (200.0, 200.0)),
        (column, "padding:0 10px", (200.0, 450.0)),
        (column, "padding:0 10%", (200.0, 400.0)),
        (
            column,
            "aspect-ratio:1;padding:0 10px;box-sizing:border-box",
            (200.0, 200.0),
        ),
        (column_start.as_str(), "", (20.0, 50.0)),
        (column_start.as_str(), "flex-grow:1", (40.0, 100.0)),
        (column_start.as_str(), "flex-basis:80px", (32.0, 80.0)),
        (
            column_start.as_str(),
            "flex-basis:60px;flex-grow:1",
            (40.0, 100.0),
        ),
        (
            column_start.as_str(),
            "aspect-ratio:1;flex-grow:1",
            (100.0, 100.0),
        ),
        (
            column_start.as_str(),
            "flex-grow:1;max-height:60px",
            (24.0, 60.0),
        ),
        (
            column_start.as_str(),
            "flex-grow:1;padding:0 10px",
            (60.0, 100.0),
        ),
        (
            column_start.as_str(),
            "flex-grow:1;padding:0 10%",
            (80.0, 100.0),
        ),
        (
            column_start.as_str(),
            "flex-grow:1;padding:0 10px;box-sizing:border-box",
            (60.0, 100.0),
        ),
        (
            column_start.as_str(),
            "aspect-ratio:1;flex-grow:1;padding:0 10px;box-sizing:border-box",
            (100.0, 100.0),
        ),
        (
            column_start.as_str(),
            "flex-grow:1;min-width:max-content",
            (40.0, 100.0),
        ),
        (
            column_start.as_str(),
            "flex-grow:1;max-width:30px",
            (30.0, 100.0),
        ),
        (row_start, "flex-grow:1", (200.0, 500.0)),
        (row_start, "", (20.0, 50.0)),
    ] {
        let mut image = 0;
        let doc = laid_out_with_natural_size((20.0, 50.0), |doc, body| {
            let parent = doc.append_element(Some(body), "div", Some(parent));
            image = doc.append_element(Some(parent), "img", Some(css));
        });
        assert_eq!(size(&doc, image), expected, "{parent} / {css}");
    }
}

#[test]
fn a_stretched_image_in_a_wrapping_flex_container_keeps_its_content_size_on_the_main_axis() {
    // Sizes a browser gives for a 20x50 image in a flex container 200px wide
    // and 100px high. A container that wraps its items does not take the cross
    // size it stretches an item to as definite while it sizes the item along
    // the main axis, so the item keeps its content size there; a cross size the
    // item states itself, or a main size the container grows it to, still give
    // the other side through the ratio.
    let column_wrap = "display:flex;flex-direction:column;flex-wrap:wrap;width:200px;height:100px";
    let row_wrap = "display:flex;flex-wrap:wrap;width:200px;height:100px";
    let column_wrap_reverse =
        "display:flex;flex-direction:column;flex-wrap:wrap-reverse;width:200px;height:100px";
    let column_wrap_start = format!("{column_wrap};align-items:flex-start");
    for (parent, css, expected) in [
        (column_wrap, "", (200.0, 50.0)),
        (column_wrap_reverse, "", (200.0, 50.0)),
        (column_wrap_start.as_str(), "", (20.0, 50.0)),
        (column_wrap_start.as_str(), "flex-grow:1", (40.0, 100.0)),
        (column_wrap, "width:50px", (50.0, 125.0)),
        (column_wrap, "height:60px", (200.0, 60.0)),
        (column_wrap, "flex-grow:1", (200.0, 100.0)),
        (row_wrap, "", (20.0, 100.0)),
        (row_wrap, "flex-grow:1", (200.0, 500.0)),
    ] {
        let mut image = 0;
        let doc = laid_out_with_natural_size((20.0, 50.0), |doc, body| {
            let parent = doc.append_element(Some(body), "div", Some(parent));
            image = doc.append_element(Some(parent), "img", Some(css));
        });
        assert_eq!(size(&doc, image), expected, "{parent} / {css}");
    }
}

#[test]
fn a_grid_item_image_stretched_along_one_axis_takes_the_other_side_through_the_ratio() {
    // Sizes a browser gives for a 20x50 image in a 200px wide grid cell that is
    // 100px high.
    let grid = "display:grid;grid-template-columns:200px;grid-template-rows:100px";
    for (css, expected) in [
        ("align-self:stretch", (40.0, 100.0)),
        ("justify-self:stretch", (200.0, 500.0)),
        ("align-self:stretch;justify-self:stretch", (200.0, 100.0)),
        ("align-self:end;justify-self:end", (20.0, 50.0)),
    ] {
        let mut image = 0;
        let doc = laid_out_with_natural_size((20.0, 50.0), |doc, body| {
            let parent = doc.append_element(Some(body), "div", Some(grid));
            image = doc.append_element(Some(parent), "img", Some(css));
        });
        assert_eq!(size(&doc, image), expected, "{css}");
    }
}
