use super::*;

#[test]
fn resolves_count_and_percentage_gap_against_used_width() {
    let context = FragmentationContext::resolve(
        100.0,
        Some(60.0),
        MulticolStyle {
            count: Some(2),
            width: None,
            column_fill: raikiri_style::property::ColumnFillValue::Balance,
            gap: 0.0,
            gap_percent: Some(10.0),
            height_definite: true,
            horizontal: true,
            orphans: 2,
            widows: 2,
        },
    )
    .expect("positive used width should resolve");
    assert_eq!(context.column_count, 2);
    assert!((context.column_gap - 10.0).abs() < f32::EPSILON);
    assert!((context.column_width - 45.0).abs() < f32::EPSILON);
    let second = context.in_column(1, context.column_offset_x(1), 12.0);
    assert_eq!(second.column_index, 1);
    assert_eq!(second.origin_y, 12.0);
    assert!(
        FragmentationContext::resolve(
            0.0,
            None,
            MulticolStyle {
                count: None,
                width: None,
                column_fill: raikiri_style::property::ColumnFillValue::Balance,
                gap: 0.0,
                gap_percent: None,
                height_definite: false,
                horizontal: true,
                orphans: 2,
                widows: 2,
            },
        )
        .is_none()
    );
    let inferred = FragmentationContext::resolve(
        100.0,
        None,
        MulticolStyle {
            count: None,
            width: Some(30.0),
            column_fill: raikiri_style::property::ColumnFillValue::Balance,
            gap: 5.0,
            gap_percent: None,
            height_definite: false,
            horizontal: true,
            orphans: 2,
            widows: 2,
        },
    )
    .expect("auto count should derive from width");
    assert_eq!(inferred.column_count, 3);
    let non_finite_gap = FragmentationContext::resolve(
        100.0,
        None,
        MulticolStyle {
            count: Some(1),
            width: None,
            column_fill: raikiri_style::property::ColumnFillValue::Balance,
            gap: f32::NAN,
            gap_percent: None,
            height_definite: false,
            horizontal: true,
            orphans: 2,
            widows: 2,
        },
    )
    .expect("non-finite gap should fail closed to zero");
    assert_eq!(non_finite_gap.column_gap, 0.0);
}

#[test]
fn fragment_tree_retains_break_points_until_clear() {
    let mut tree = FragmentTree::default();
    let parent = tree
        .try_push(LayoutFragment {
            node_id: 1,
            parent: None,
            fragmentainer: 0,
            rect: FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 20.0,
            },
            fragmentainer_clip: None,
            fragment_index: 0,
            fragment_count: 1,
            line_start: None,
            line_end: None,
        })
        .expect("first fragment");
    assert_eq!(parent, 0);
    let nested = tree
        .try_push(LayoutFragment {
            node_id: 2,
            parent: None,
            fragmentainer: 0,
            rect: FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 10.0,
            },
            fragmentainer_clip: None,
            fragment_index: 0,
            fragment_count: 1,
            line_start: None,
            line_end: None,
        })
        .expect("nested fragment");
    tree.reparent_roots(2, parent);
    assert_eq!(tree.fragments[nested].parent, Some(parent));
    tree.record_break(BreakToken {
        node_id: 1,
        child_index: 2,
        line_index: 0,
    });
    assert_eq!(tree.fragments.len(), 2);
    assert_eq!(tree.break_tokens.len(), 1);
    tree.clear();
    assert!(tree.fragments.is_empty());
    assert!(tree.break_tokens.is_empty());
}

#[test]
fn fragment_tree_budget_is_shared_across_nodes_and_resets_on_clear() {
    let mut tree = FragmentTree {
        limit: 2,
        ..FragmentTree::default()
    };
    let fragment = LayoutFragment {
        node_id: 1,
        parent: None,
        fragmentainer: 0,
        rect: FragmentRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    };
    assert_eq!(tree.try_push(fragment), Some(0));
    assert_eq!(
        tree.try_push(LayoutFragment {
            node_id: 2,
            ..fragment
        }),
        Some(1)
    );
    assert_eq!(
        tree.try_push(LayoutFragment {
            node_id: 3,
            ..fragment
        }),
        None
    );
    assert_eq!(tree.fragments.len(), 2);
    assert!(tree.limit_exceeded);
    tree.clear();
    assert!(!tree.limit_exceeded);
    assert_eq!(tree.try_push(fragment), Some(0));
}
