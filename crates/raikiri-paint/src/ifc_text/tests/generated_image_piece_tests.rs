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
    assert_eq!(lines[0].content_images.len(), count);
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
