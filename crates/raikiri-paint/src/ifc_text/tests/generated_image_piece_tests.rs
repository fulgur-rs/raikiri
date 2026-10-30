use super::*;
use crate::generated_image_diagnostics::{self as diagnostics, ImagePieceWork};
use std::sync::Arc;

struct SharedPixels(Arc<raikiri_traits::DecodedImage>);

impl raikiri_traits::ImagePixelSource for SharedPixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<raikiri_traits::DecodedImage>> {
        Some(self.0.clone())
    }
}

fn image_spans(count: usize, css: &str) -> (Document, CascadeResult, usize) {
    image_spans_with_count(count, css, 1)
}

fn image_spans_with_count(
    count: usize,
    css: &str,
    images_per_span: usize,
) -> (Document, CascadeResult, usize) {
    let (mut document, cascade, root) = paragraph("white-space:nowrap", |document, root| {
        let style = document.append_element(Some(0), "style", None::<&str>);
        document.append_text(
            style,
            format!("span::before{{content:url(https://images.test/a.png)}}{css}"),
        );
        for _ in 0..count {
            let span = document.append_element(Some(root), "span", None::<&str>);
            document.append_text(span, "a");
        }
    });
    let pixels = SharedPixels(Arc::new(raikiri_traits::DecodedImage::new(
        1,
        1,
        vec![0, 0, 255, 255],
    )));
    document.prepare_list_marker_images(&cascade, &pixels, None);
    lay_out(&mut document, &cascade);
    let positioned = PositionedLines::new(&document, &cascade, root, None).unwrap();
    let lines: Vec<_> = positioned.lines().collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].content_images.len(), count * images_per_span);
    (document, cascade, root)
}

fn paint_images(document: &Document, cascade: &CascadeResult, root: usize) -> Scene {
    let mut scene = Scene::new();
    draw_ifc_lines(
        &mut scene,
        document,
        cascade,
        root,
        IfcPosition {
            x: 0.0,
            y: 0.0,
            shift_y: 0.0,
        },
        &crate::text::DecorationContext::default(),
        None,
        &[],
    );
    scene
}

fn image_count(scene: &Scene) -> usize {
    scene
        .commands
        .iter()
        .filter(|command| {
            matches!(command,
                RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_))
            )
        })
        .count()
}

#[test]
fn optional_piece_diagnostics_preserve_all_generated_images() {
    let (document, cascade, root) = image_spans(8, "");
    assert_eq!(diagnostics::finish(), ImagePieceWork::default());
    assert_eq!(image_count(&paint_images(&document, &cascade, root)), 8);
    assert_eq!(diagnostics::finish(), ImagePieceWork::default());
    diagnostics::begin();
    assert_eq!(image_count(&paint_images(&document, &cascade, root)), 8);
    let work = diagnostics::finish();
    assert_eq!(work.image_lookups, 8);
    assert!(work.predicate_visits + work.index_visits > 0);
    assert!(format!("{work:?}").contains("image_lookups: 8"));
    assert_eq!(image_count(&paint_images(&document, &cascade, root)), 8);
    assert_eq!(diagnostics::finish(), ImagePieceWork::default());
}

#[test]
fn generated_image_piece_lookup_has_a_linear_work_bound() {
    for count in [8, 16, 32] {
        let (document, cascade, root) = image_spans(count, "");
        let pieces = document
            .ifc_layout_node(root)
            .unwrap()
            .ifc_inline_boxes()
            .unwrap();
        diagnostics::begin();
        assert_eq!(image_count(&paint_images(&document, &cascade, root)), count);
        let work = diagnostics::finish();
        assert_eq!(work.image_lookups, count as u64);
        assert!(
            work.predicate_visits + work.index_visits <= pieces.len() as u64,
            "piece matching must visit each line's pieces at most once: {work:?}"
        );
    }
}

#[test]
fn generated_images_keep_parent_and_typographic_opacity_groups() {
    let mut atomic_positions = Vec::new();
    for (css, expected) in [
        ("", 1.0),
        ("span{opacity:.5}", 0.5),
        ("span::before{opacity:.5}", 0.5),
        ("div::first-line{opacity:.5}", 0.5),
        ("div::first-letter{opacity:.5}", 1.0),
        ("span::before{display:inline-block}", 1.0),
        (
            "div::first-line{opacity:.5}div::first-letter{opacity:.5}",
            0.5,
        ),
        (
            "span::before{display:inline-block;transform:translateX(3px)}",
            1.0,
        ),
    ] {
        let (document, cascade, root) = image_spans(4, css);
        let scene = paint_images(&document, &cascade, root);
        let mut alphas = vec![1.0_f32];
        let mut image_alphas = Vec::new();
        let mut image_positions = Vec::new();
        for command in &scene.commands {
            match command {
                RenderCommand::PushLayer(layer) => {
                    alphas.push(alphas.last().unwrap() * layer.alpha)
                }
                RenderCommand::PushClipLayer(_) => alphas.push(*alphas.last().unwrap()),
                RenderCommand::PopLayer => {
                    alphas.pop().unwrap();
                }
                RenderCommand::Fill(fill) if matches!(fill.brush, anyrender::Paint::Image(_)) => {
                    image_alphas.push(*alphas.last().unwrap());
                    image_positions.push(fill.transform.as_coeffs()[4]);
                }
                _ => {}
            }
        }
        assert_eq!(image_alphas, vec![expected; 4], "{css}");
        assert!(
            image_positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{css}"
        );
        assert_eq!(alphas, [1.0]);
        if css == "span::before{display:inline-block}" {
            atomic_positions = image_positions;
        } else if css.contains("translateX") {
            assert_eq!(
                image_positions,
                atomic_positions.iter().map(|x| x + 3.0).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn multiple_images_of_one_pseudo_share_its_first_piece_and_group() {
    let (document, cascade, root) = image_spans_with_count(
        4,
        "span::before{content:url(https://images.test/a.png) url(https://images.test/a.png);opacity:.5}",
        2,
    );
    diagnostics::begin();
    let scene = paint_images(&document, &cascade, root);
    let work = diagnostics::finish();
    assert_eq!(image_count(&scene), 8);
    assert_eq!(work.image_lookups, 8);
    let pieces = document
        .ifc_layout_node(root)
        .unwrap()
        .ifc_inline_boxes()
        .unwrap();
    assert_eq!(work.index_visits, pieces.len() as u64);
    assert_eq!(work.predicate_visits, 0);
    let layers: Vec<_> = scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::PushLayer(layer) => Some(layer.alpha),
            _ => None,
        })
        .collect();
    assert_eq!(layers, vec![0.5; 4]);
}

#[test]
fn text_only_lines_do_not_build_image_piece_indexes() {
    let (mut document, cascade, root) = paragraph("", |document, root| {
        document.append_text(root, "text");
    });
    lay_out(&mut document, &cascade);
    diagnostics::begin();
    assert_eq!(image_count(&paint_images(&document, &cascade, root)), 0);
    assert_eq!(diagnostics::finish(), ImagePieceWork::default());
}
