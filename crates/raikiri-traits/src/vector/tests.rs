use std::sync::Arc;

use super::*;
use crate::image::DecodedImage;

fn black() -> Paint {
    Paint::Color(PaintColor::new(0.0, 0.0, 0.0, 1.0))
}

fn square() -> Path {
    let mut path = Path::new(vec![
        PathSegment::MoveTo((0.0, 0.0)),
        PathSegment::LineTo((10.0, 0.0)),
        PathSegment::LineTo((10.0, 10.0)),
        PathSegment::Close,
    ]);
    path.fill = Some(Fill::new(black()));
    path
}

#[test]
fn an_empty_scene_has_only_its_root() {
    let image = VectorImage::new((20.0, 10.0));
    assert_eq!(image.size, (20.0, 10.0));
    assert_eq!(image.node_count(), 1);
    assert_eq!(image.root.transform, PaintTransform::IDENTITY);
    assert_eq!(image.root.opacity, 1.0);
}

#[test]
fn node_count_includes_nested_groups_and_paths() {
    let mut inner = Group::new(PaintTransform::translate(1.0, 2.0));
    inner.children.push(Node::Path(square()));
    inner.children.push(Node::Path(square()));
    let mut image = VectorImage::new((10.0, 10.0));
    image.root.children.push(Node::Group(inner));
    image.root.children.push(Node::Path(square()));
    // The root, the inner group and three paths.
    assert_eq!(image.node_count(), 5);
}

#[test]
fn estimated_heap_bytes_grows_with_segments_and_dashes() {
    let empty = VectorImage::new((10.0, 10.0));
    let mut image = VectorImage::new((10.0, 10.0));
    image.root.children.push(Node::Path(square()));
    let plain = image.estimated_heap_bytes();
    assert!(plain > empty.estimated_heap_bytes());

    let mut dashed = square();
    let mut stroke = Stroke::new(black(), 2.0);
    stroke.dash = Some(Dash::new(vec![1.0, 2.0, 3.0, 4.0], 0.0));
    dashed.stroke = Some(stroke);
    let mut with_dash = VectorImage::new((10.0, 10.0));
    with_dash.root.children.push(Node::Path(dashed));
    let dash_bytes = 4 * std::mem::size_of::<f32>();
    assert!(with_dash.estimated_heap_bytes() >= plain + dash_bytes);
}

#[test]
fn estimated_heap_bytes_counts_nested_groups() {
    let mut flat = VectorImage::new((10.0, 10.0));
    flat.root.children.push(Node::Path(square()));
    let mut inner = Group::new(PaintTransform::translate(1.0, 2.0));
    inner.children.push(Node::Path(square()));
    let mut nested = VectorImage::new((10.0, 10.0));
    nested.root.children.push(Node::Group(inner));
    // The inner group holds a path of its own, and the outer group holds the
    // inner group.
    let path_bytes = Path::heap_bytes(&square());
    assert!(path_bytes > 0);
    assert!(
        nested.estimated_heap_bytes()
            >= flat.estimated_heap_bytes() + path_bytes - std::mem::size_of::<Node>()
    );
}

#[test]
fn constructors_pick_the_css_defaults() {
    let fill = Fill::new(black());
    assert_eq!((fill.opacity, fill.rule), (1.0, FillRule::NonZero));
    let stroke = Stroke::new(black(), 3.0);
    assert_eq!(stroke.width, 3.0);
    assert_eq!(
        (stroke.cap, stroke.join, stroke.miter_limit, stroke.dash),
        (LineCap::Butt, LineJoin::Miter, 4.0, None)
    );
    let path = Path::new(Vec::new());
    assert_eq!(path.paint_order, PaintOrder::FillThenStroke);
    assert!(path.anti_alias);
    assert!(path.fill.is_none() && path.stroke.is_none());
}

fn stops() -> Vec<GradientStop> {
    vec![
        GradientStop::new(0.0, PaintColor::new(1.0, 0.0, 0.0, 1.0)),
        GradientStop::new(1.0, PaintColor::new(0.0, 0.0, 1.0, 0.5)),
    ]
}

#[test]
fn gradient_constructors_pick_padded_unit_defaults() {
    let linear = LinearGradient::new((0.0, 0.0), (10.0, 0.0), stops());
    assert_eq!(linear.transform, PaintTransform::IDENTITY);
    assert_eq!(linear.spread, SpreadMethod::Pad);
    let radial = RadialGradient::new((5.0, 5.0), 4.0, stops());
    assert_eq!((radial.focal, radial.focal_radius), ((5.0, 5.0), 0.0));
    assert_eq!(radial.transform, PaintTransform::IDENTITY);
    assert_eq!(radial.spread, SpreadMethod::Pad);
}

#[test]
fn gradients_and_clips_are_charged_to_the_scene() {
    let mut plain = VectorImage::new((10.0, 10.0));
    plain.root.children.push(Node::Path(square()));
    let mut painted = square();
    let mut fill = Fill::new(Paint::LinearGradient(Box::new(LinearGradient::new(
        (0.0, 0.0),
        (10.0, 0.0),
        stops(),
    ))));
    fill.opacity = 0.5;
    painted.fill = Some(fill);
    let mut stroked = square();
    stroked.stroke = Some(Stroke::new(
        Paint::RadialGradient(Box::new(RadialGradient::new((5.0, 5.0), 4.0, stops()))),
        1.0,
    ));
    let stop_bytes = 2 * std::mem::size_of::<GradientStop>();
    let mut image = VectorImage::new((10.0, 10.0));
    image.root.children.push(Node::Path(painted));
    image.root.children.push(Node::Path(stroked));
    // Each gradient counts its own record and its stops.
    assert!(
        image.estimated_heap_bytes()
            >= plain.estimated_heap_bytes()
                + 2 * (std::mem::size_of::<LinearGradient>() + stop_bytes)
    );

    // A group with a clip that holds a group, a path and a chained clip.
    let mut clip = ClipPath::new(PaintTransform::translate(1.0, 1.0));
    clip.root.children.push(Node::Path(square()));
    let mut inner = Group::new(PaintTransform::IDENTITY);
    inner.children.push(Node::Path(square()));
    clip.root.children.push(Node::Group(inner));
    clip.clip = Some(Box::new(ClipPath::new(PaintTransform::IDENTITY)));
    // The clip, a path, a group with a path, and the chained clip.
    assert_eq!(clip.node_count(), 1 + 1 + 2 + 1);
    let mut clipped = Group::new(PaintTransform::IDENTITY);
    clipped.children.push(Node::Path(square()));
    clipped.clip = Some(Box::new(clip));
    let mut with_clip = VectorImage::new((10.0, 10.0));
    with_clip.root.children.push(Node::Group(clipped));
    // The root, the group, its path, and the clip.
    assert_eq!(with_clip.node_count(), 1 + 1 + 1 + 5);
    let mut without_clip = with_clip.clone();
    let Node::Group(group) = &mut without_clip.root.children[0] else {
        panic!("a group");
    };
    group.clip = None;
    assert!(with_clip.estimated_heap_bytes() > without_clip.estimated_heap_bytes());
}

fn face(bytes: &[u8]) -> FontFace {
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes.to_vec());
    FontFace::new(data, 0)
}

fn glyph(font: &FontFace) -> Glyph {
    Glyph::new(font.clone(), 7, "a", PaintTransform::IDENTITY)
}

#[test]
fn a_font_face_is_the_same_face_by_the_identity_of_its_bytes() {
    let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(b"font bytes".to_vec());
    let font = FontFace::new(data.clone(), 0);
    assert!(font.is_same_face(&font.clone()));
    assert_eq!(font.bytes(), b"font bytes");
    assert_eq!(font.index(), 0);
    // A copy of the same bytes is an equal face but not the same one.
    let copy = face(b"font bytes");
    assert!(!font.is_same_face(&copy));
    assert_eq!(font, copy);
    assert_ne!(font, face(b"other bytes"));
    // The same bytes with another face index are another face.
    let second = FontFace::new(data, 1);
    assert!(!font.is_same_face(&second));
    assert_ne!(font, second);
}

#[test]
fn a_text_run_counts_as_one_node_and_charges_its_glyphs() {
    let font = face(&[0; 64]);
    let empty = VectorImage::new((10.0, 10.0));
    let mut image = VectorImage::new((10.0, 10.0));
    let mut run = TextRun::new(16.0, vec![glyph(&font), glyph(&font), glyph(&font)]);
    run.variations.push(FontVariation::new(*b"wght", 700.0));
    run.fill = Some(Fill::new(black()));
    image.root.children.push(Node::Text(run));
    assert_eq!(image.node_count(), 2);
    // The glyphs and the strings they hold are counted; the font bytes are
    // shared and are not.
    let glyphs = 3 * std::mem::size_of::<Glyph>();
    assert!(image.estimated_heap_bytes() >= empty.estimated_heap_bytes() + glyphs);
    assert!(image.estimated_heap_bytes() < empty.estimated_heap_bytes() + glyphs + 64 * 1024);
}

#[test]
fn a_text_run_starts_unpainted_and_smoothed() {
    let run = TextRun::new(12.0, Vec::new());
    assert_eq!(run.font_size, 12.0);
    assert!(run.fill.is_none() && run.stroke.is_none());
    assert_eq!(run.paint_order, PaintOrder::FillThenStroke);
    assert!(run.anti_alias);
    assert!(format!("{:?}", glyph(&face(b"abc"))).contains("byte_len: 3"));
}

#[test]
fn a_picture_counts_as_one_node_and_shares_its_pixels() {
    let pixels = Arc::new(DecodedImage {
        width: 64,
        height: 64,
        rgba: vec![255; 64 * 64 * 4],
    });
    let empty = VectorImage::new((10.0, 10.0));
    let mut image = VectorImage::new((10.0, 10.0));
    let picture = RasterImage::new(pixels.clone());
    assert!(picture.smooth);
    image.root.children.push(Node::Image(picture.clone()));
    image.root.children.push(Node::Image(picture));
    assert_eq!(image.node_count(), 3);
    // Only the nodes are charged: the pixels belong to the image cache that
    // decoded them, which two nodes of one picture share.
    assert!(image.estimated_heap_bytes() >= empty.estimated_heap_bytes());
    assert!(image.estimated_heap_bytes() < empty.estimated_heap_bytes() + 64 * 1024);
    assert_eq!(Arc::strong_count(&pixels), 3);
}
