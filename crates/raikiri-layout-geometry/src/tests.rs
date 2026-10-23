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

#[test]
fn sizes_map_zip_and_transpose_per_axis() {
    let size = Size {
        width: 2.0_f32,
        height: 5.0,
    };
    assert_eq!(
        size.map(|v| v * 2.0),
        Size {
            width: 4.0,
            height: 10.0
        }
    );
    assert_eq!(
        size.zip_map(
            Size {
                width: 1_u8,
                height: 3
            },
            |a, b| a + f32::from(b)
        ),
        Size {
            width: 3.0,
            height: 8.0
        }
    );
    assert_eq!(
        size.transpose(),
        Size {
            width: 5.0,
            height: 2.0
        }
    );
    assert_eq!(size.transpose().transpose(), size);
}

#[test]
fn sizes_add_subtract_and_take_the_larger_axis_values() {
    let a = Size {
        width: 7.0_f32,
        height: 1.0,
    };
    let b = Size {
        width: 3.0_f32,
        height: 4.0,
    };
    assert_eq!(
        a + b,
        Size {
            width: 10.0,
            height: 5.0
        }
    );
    assert_eq!(
        a - b,
        Size {
            width: 4.0,
            height: -3.0
        }
    );
    assert_eq!(
        a.f32_max(b),
        Size {
            width: 7.0,
            height: 4.0
        }
    );
}

#[test]
fn a_missing_dimension_is_filled_in_from_the_alternative() {
    let some = Size {
        width: Some(1.0_f32),
        height: None,
    };
    let alt = Size {
        width: Some(9.0),
        height: Some(8.0),
    };
    assert_eq!(
        some.or(alt),
        Size {
            width: Some(1.0),
            height: Some(8.0)
        }
    );
    assert_eq!(Size::NONE.or(Size::NONE), Size::<Option<f32>>::NONE);
}

#[test]
fn the_missing_dimension_follows_the_aspect_ratio() {
    let width_only = Size {
        width: Some(100.0_f32),
        height: None,
    };
    let height_only = Size {
        width: None,
        height: Some(50.0_f32),
    };
    let both = Size {
        width: Some(1.0_f32),
        height: Some(2.0),
    };
    assert_eq!(
        width_only.maybe_apply_aspect_ratio(Some(2.0)),
        Size {
            width: Some(100.0),
            height: Some(50.0)
        }
    );
    assert_eq!(
        height_only.maybe_apply_aspect_ratio(Some(2.0)),
        Size {
            width: Some(100.0),
            height: Some(50.0)
        }
    );
    assert_eq!(both.maybe_apply_aspect_ratio(Some(2.0)), both);
    assert_eq!(width_only.maybe_apply_aspect_ratio(None), width_only);
    assert_eq!(
        Size::<Option<f32>>::NONE.maybe_apply_aspect_ratio(Some(2.0)),
        Size::NONE
    );
}

#[test]
fn points_map_transpose_add_and_become_sizes() {
    let point = Point { x: 1.0_f32, y: 2.0 };
    assert_eq!(point.map(|v| v + 1.0), Point { x: 2.0, y: 3.0 });
    assert_eq!(point.transpose(), Point { x: 2.0, y: 1.0 });
    assert_eq!(
        point + Point { x: 10.0, y: 20.0 },
        Point { x: 11.0, y: 22.0 }
    );
    assert_eq!(
        Size::from(point),
        Size {
            width: 1.0,
            height: 2.0
        }
    );
    assert_eq!(Point::<Option<f32>>::NONE, Point { x: None, y: None });
}

#[test]
fn lines_map_and_have_flag_constants() {
    let line = Line { start: 1, end: 4 };
    assert_eq!(line.map(|v| v * 3), Line { start: 3, end: 12 });
    assert_eq!(
        Line::TRUE,
        Line {
            start: true,
            end: true
        }
    );
    assert_eq!(
        Line::FALSE,
        Line {
            start: false,
            end: false
        }
    );
}

#[test]
fn edges_sum_per_axis_transpose_map_and_add() {
    let edges = Edges {
        left: 1.0_f32,
        right: 2.0,
        top: 4.0,
        bottom: 8.0,
    };
    assert_eq!(edges.horizontal_axis_sum(), 3.0);
    assert_eq!(edges.vertical_axis_sum(), 12.0);
    assert_eq!(
        edges.sum_axes(),
        Size {
            width: 3.0,
            height: 12.0
        }
    );
    assert_eq!(
        edges.transpose(),
        Edges {
            left: 4.0,
            right: 8.0,
            top: 1.0,
            bottom: 2.0
        }
    );
    assert_eq!(edges.transpose().transpose(), edges);
    assert_eq!(
        edges.map(|v| -v),
        Edges {
            left: -1.0,
            right: -2.0,
            top: -4.0,
            bottom: -8.0
        }
    );
    assert_eq!(edges + edges, edges.map(|v| v * 2.0));
    assert_eq!(Edges::<Option<f32>>::NONE.left, None);
}

#[test]
fn the_union_of_edges_reaches_furthest_on_every_side() {
    let a = Edges {
        left: -5.0_f32,
        right: 10.0,
        top: 0.0,
        bottom: 3.0,
    };
    let b = Edges {
        left: -1.0_f32,
        right: 20.0,
        top: -2.0,
        bottom: 1.0,
    };
    let expected = Edges {
        left: -5.0,
        right: 20.0,
        top: -2.0,
        bottom: 3.0,
    };
    assert_eq!(a.union(b), expected);
    assert_eq!(b.union(a), expected);
    assert_eq!(
        a.union(Edges::ZERO),
        Edges {
            left: -5.0,
            right: 10.0,
            top: 0.0,
            bottom: 3.0
        }
    );
}

#[test]
fn optional_lengths_do_math_where_both_sides_are_present() {
    let some = Some(5.0_f32);
    assert_eq!(some.maybe_min(Some(3.0)), Some(3.0));
    assert_eq!(some.maybe_max(Some(3.0)), Some(5.0));
    assert_eq!(some.maybe_add(Some(3.0)), Some(8.0));
    assert_eq!(some.maybe_sub(Some(3.0)), Some(2.0));
    assert_eq!(some.maybe_clamp(Some(6.0), Some(9.0)), Some(6.0));
    assert_eq!(some.maybe_clamp(Some(1.0), Some(4.0)), Some(4.0));
}

#[test]
fn a_missing_right_hand_side_leaves_the_left_unchanged() {
    let some = Some(5.0_f32);
    assert_eq!(some.maybe_min(None), some);
    assert_eq!(some.maybe_max(None), some);
    assert_eq!(some.maybe_add(None), some);
    assert_eq!(some.maybe_sub(None), some);
    assert_eq!(some.maybe_clamp(None, None), some);
    assert_eq!(some.maybe_clamp(Some(7.0), None), Some(7.0));
    assert_eq!(some.maybe_clamp(None, Some(2.0)), Some(2.0));
}

#[test]
fn a_missing_left_hand_side_stays_missing() {
    let none: Option<f32> = None;
    assert_eq!(none.maybe_min(Some(1.0)), None);
    assert_eq!(none.maybe_max(Some(1.0)), None);
    assert_eq!(none.maybe_add(Some(1.0)), None);
    assert_eq!(none.maybe_sub(Some(1.0)), None);
    assert_eq!(none.maybe_clamp(Some(0.0), Some(1.0)), None);
    assert_eq!(none.maybe_clamp(None, None), None);
    assert_eq!(none.maybe_min(1.0), None);
    assert_eq!(none.maybe_add(1.0), None);
}

#[test]
fn optional_lengths_do_math_against_a_plain_length() {
    let some = Some(5.0_f32);
    assert_eq!(some.maybe_min(3.0), Some(3.0));
    assert_eq!(some.maybe_max(8.0), Some(8.0));
    assert_eq!(some.maybe_add(1.5), Some(6.5));
    assert_eq!(some.maybe_sub(1.5), Some(3.5));
    assert_eq!(some.maybe_clamp(6.0, 9.0), Some(6.0));
}

#[test]
fn a_plain_length_does_math_against_an_optional_one() {
    assert_eq!(5.0_f32.maybe_min(Some(3.0)), 3.0);
    assert_eq!(5.0_f32.maybe_min(None), 5.0);
    assert_eq!(5.0_f32.maybe_max(Some(8.0)), 8.0);
    assert_eq!(5.0_f32.maybe_max(None), 5.0);
    assert_eq!(5.0_f32.maybe_add(Some(2.0)), 7.0);
    assert_eq!(5.0_f32.maybe_add(None), 5.0);
    assert_eq!(5.0_f32.maybe_sub(Some(2.0)), 3.0);
    assert_eq!(5.0_f32.maybe_sub(None), 5.0);
    assert_eq!(5.0_f32.maybe_clamp(Some(6.0), Some(9.0)), 6.0);
    assert_eq!(5.0_f32.maybe_clamp(Some(1.0), Some(4.0)), 4.0);
    assert_eq!(5.0_f32.maybe_clamp(Some(7.0), None), 7.0);
    assert_eq!(5.0_f32.maybe_clamp(None, Some(2.0)), 2.0);
    assert_eq!(5.0_f32.maybe_clamp(None, None), 5.0);
}

#[test]
fn clamping_applies_the_upper_limit_first_so_the_lower_limit_wins() {
    // CSS resolves `min-width` above `max-width`.
    assert_eq!(5.0_f32.maybe_clamp(Some(10.0), Some(3.0)), 10.0);
    assert_eq!(Some(5.0_f32).maybe_clamp(Some(10.0), Some(3.0)), Some(10.0));
}

#[test]
fn math_on_sizes_applies_to_each_axis_on_its_own() {
    let size = Size {
        width: Some(10.0_f32),
        height: None,
    };
    let rhs = Size {
        width: Some(4.0_f32),
        height: Some(4.0),
    };
    assert_eq!(
        size.maybe_min(rhs),
        Size {
            width: Some(4.0),
            height: None
        }
    );
    assert_eq!(
        size.maybe_max(rhs),
        Size {
            width: Some(10.0),
            height: None
        }
    );
    assert_eq!(
        size.maybe_add(rhs),
        Size {
            width: Some(14.0),
            height: None
        }
    );
    assert_eq!(
        size.maybe_sub(rhs),
        Size {
            width: Some(6.0),
            height: None
        }
    );
    assert_eq!(
        size.maybe_clamp(Size::NONE, rhs),
        Size {
            width: Some(4.0),
            height: None
        }
    );

    let plain = Size {
        width: 10.0_f32,
        height: 20.0,
    };
    let limit = Size {
        width: Some(15.0_f32),
        height: None,
    };
    assert_eq!(
        plain.maybe_min(limit),
        Size {
            width: 10.0,
            height: 20.0
        }
    );
    assert_eq!(
        plain.maybe_add(limit),
        Size {
            width: 25.0,
            height: 20.0
        }
    );
}

#[test]
fn nan_is_ignored_by_min_and_max_like_f32_min_and_max() {
    assert_eq!(f32::NAN.maybe_min(Some(3.0)), 3.0);
    assert_eq!(3.0_f32.maybe_max(Some(f32::NAN)), 3.0);
    assert_eq!(Some(f32::NAN).maybe_max(Some(2.0)), Some(2.0));
}

#[test]
fn box_layout_constructors_set_the_order_only() {
    let layout = BoxLayout::with_order(9);
    assert_eq!(layout.order, 9);
    assert_eq!(BoxLayout { order: 0, ..layout }, BoxLayout::new());
    assert_eq!(BoxLayout::new(), BoxLayout::default());
}

#[test]
fn scroll_extents_never_go_below_zero() {
    let layout = BoxLayout {
        size: Size {
            width: 100.0,
            height: 100.0,
        },
        scrollable_overflow_rect: Edges {
            left: 0.0,
            right: 10.0,
            top: 0.0,
            bottom: 10.0,
        },
        ..BoxLayout::new()
    };
    assert_eq!(layout.scroll_width(), 0.0);
    assert_eq!(layout.scroll_height(), 0.0);
}

#[test]
fn a_scrollbar_larger_than_the_box_counts_only_as_much_as_the_box() {
    let layout = BoxLayout {
        size: Size {
            width: 10.0,
            height: 10.0,
        },
        scrollbar_size: Size {
            width: 1e3,
            height: 1e3,
        },
        scrollable_overflow_rect: Edges {
            left: 0.0,
            right: 30.0,
            top: 0.0,
            bottom: 25.0,
        },
        ..BoxLayout::new()
    };
    assert_eq!(layout.scroll_width(), 30.0);
    assert_eq!(layout.scroll_height(), 25.0);
}
