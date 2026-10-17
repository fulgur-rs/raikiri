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
    assert_eq!(sizes, vec![(100.0, 100.0); 2]);
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
