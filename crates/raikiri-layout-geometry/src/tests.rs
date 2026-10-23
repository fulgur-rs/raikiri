use super::*;

#[test]
fn size_point_and_line_expose_named_fields() {
    let size = Size {
        width: 3.0_f32,
        height: 4.0,
    };
    let point = Point { x: 1.0_f32, y: 2.0 };
    let line = Line {
        start: 5_i32,
        end: 9,
    };
    assert_eq!((size.width, size.height), (3.0, 4.0));
    assert_eq!((point.x, point.y), (1.0, 2.0));
    assert_eq!((line.start, line.end), (5, 9));
}

#[test]
fn zero_constants_are_all_zero() {
    assert_eq!(
        Size::<f32>::ZERO,
        Size {
            width: 0.0,
            height: 0.0
        }
    );
    assert_eq!(Point::<f32>::ZERO, Point { x: 0.0, y: 0.0 });
    assert_eq!(
        Edges::<f32>::ZERO,
        Edges {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
        }
    );
}

#[test]
fn edges_name_the_four_sides() {
    let edges = Edges {
        left: 1.0_f32,
        right: 2.0,
        top: 3.0,
        bottom: 4.0,
    };
    assert_eq!(
        (edges.left, edges.right, edges.top, edges.bottom),
        (1.0, 2.0, 3.0, 4.0)
    );
}

#[test]
fn box_layout_exposes_placement_fields_and_content_box_helpers() {
    let layout = BoxLayout {
        order: 7,
        location: Point { x: 10.0, y: 20.0 },
        size: Size {
            width: 100.0,
            height: 50.0,
        },
        scrollable_overflow_rect: Edges {
            left: 0.0,
            right: 140.0,
            top: 0.0,
            bottom: 90.0,
        },
        scrollbar_size: Size {
            width: 0.0,
            height: 0.0,
        },
        border: Edges {
            left: 1.0,
            right: 1.0,
            top: 2.0,
            bottom: 2.0,
        },
        padding: Edges {
            left: 4.0,
            right: 4.0,
            top: 3.0,
            bottom: 3.0,
        },
        margin: Edges::ZERO,
    };
    assert_eq!(layout.order, 7);
    assert_eq!(layout.content_box_width(), 90.0);
    assert_eq!(layout.content_box_height(), 40.0);
    assert_eq!(
        layout.content_box_size(),
        Size {
            width: 90.0,
            height: 40.0
        }
    );
    assert_eq!(layout.content_box_x(), 15.0);
    assert_eq!(layout.content_box_y(), 25.0);
    assert_eq!(layout.scroll_width(), 42.0);
    assert_eq!(layout.scroll_height(), 44.0);
}

#[test]
fn default_box_layout_is_zero() {
    let layout = BoxLayout::default();
    assert_eq!(layout.order, 0);
    assert_eq!(layout.location, Point::ZERO);
    assert_eq!(layout.size, Size::ZERO);
    assert_eq!(layout.border, Edges::ZERO);
}
