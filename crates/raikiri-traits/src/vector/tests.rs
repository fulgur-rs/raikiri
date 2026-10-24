use super::*;

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
    assert!(path.fill.is_none() && path.stroke.is_none());
}
