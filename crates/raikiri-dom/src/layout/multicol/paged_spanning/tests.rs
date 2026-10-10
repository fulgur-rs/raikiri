use super::*;
use crate::fragment::FragmentRect;
use crate::layout::page_pipeline::PageLayoutControl;
use taffy::Style;

fn source_fragments() -> (Document, usize, usize, usize) {
    let mut tree = Document::new();
    let owner = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = tree.append_element(Some(owner), "p", Style::default(), None::<&str>);
    let container = tree
        .fragment_tree
        .try_push(LayoutFragment {
            node_id: owner,
            parent: None,
            fragmentainer: 0,
            rect: FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 60.0,
            },
            fragmentainer_clip: None,
            fragment_index: 0,
            fragment_count: 1,
            line_start: None,
            line_end: None,
        })
        .unwrap();
    tree.fragment_tree
        .try_push(LayoutFragment {
            node_id: child,
            parent: Some(container),
            fragmentainer: 0,
            rect: FragmentRect {
                x: 0.0,
                y: 20.0,
                width: 40.0,
                height: 20.0,
            },
            fragmentainer_clip: Some(FragmentRect {
                x: 0.0,
                y: 20.0,
                width: 40.0,
                height: 40.0,
            }),
            fragment_index: 0,
            fragment_count: 1,
            line_start: Some(0),
            line_end: Some(1),
        })
        .unwrap();
    (tree, owner, child, container)
}

#[test]
fn a_fallback_child_shift_moves_its_column_clip_with_the_source_box() {
    let (mut tree, _, child, container) = source_fragments();
    let mut shifts = HashMap::new();
    shift_child(&mut tree, &mut shifts, child, 4.0);
    shift_child(&mut tree, &mut shifts, child, 6.0);
    apply_shifts(&mut tree, container, &shifts);
    let source = tree.fragment_tree.fragments[1];
    assert_eq!(
        source.rect,
        FragmentRect {
            x: 0.0,
            y: 30.0,
            width: 40.0,
            height: 20.0
        }
    );
    assert_eq!(
        source.fragmentainer_clip,
        Some(FragmentRect {
            x: 0.0,
            y: 30.0,
            width: 40.0,
            height: 40.0
        })
    );
    assert_eq!(tree.fragment_tree.fragments[0].rect.y, 0.0);
    assert_eq!(tree.nodes[child].unrounded_layout.location.y, 10.0);
}

#[test]
fn replacement_work_limit_rejects_before_installing_a_partial_fragment_stream() {
    let (mut tree, _, child, _) = source_fragments();
    tree.fragment_tree.limit = 8;
    let original = tree.fragment_tree.fragments.clone();
    let replacements = HashMap::from([(child, vec![original[1]; 7])]);
    let control = PageLayoutControl::new(None);
    let mut work = ProjectionWork::new(&control, 8);
    assert!(matches!(
        replace_fragments(&mut tree, replacements, &mut work),
        Err(LayoutError::FragmentLimitExceeded { limit: 8 })
    ));
    assert_eq!(tree.fragment_tree.fragments, original);
}

#[test]
fn invalid_page_capacity_keeps_the_measured_paragraph_instead_of_planning_fragments() {
    let (mut tree, cascade, root) =
        crate::layout::test_support::ahem_paragraph("A\nB", "width:40px;margin:0");
    layout_single_page(
        crate::layout::test_support::with_ahem(&mut tree),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let parent = tree.parent_of(root).unwrap();
    let measured = tree.nodes[root].unrounded_layout;
    let entries = [(
        root,
        LayoutOutput::from_outer_size(measured.size),
        measured,
        0.0,
        0.0,
        20.0,
    )];
    let context = FragmentationContext {
        available_width: 100.0,
        available_height: None,
        column_fill: ColumnFillValue::Balance,
        column_width: 40.0,
        column_count: 2,
        column_gap: 20.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    };
    let control = PageLayoutControl::new(None);
    let mut work = ProjectionWork::new(&control, tree.fragment_tree.limit);
    let valid = paragraph_group::paginate(
        &tree,
        parent,
        &entries,
        context,
        0.0,
        &|_| (0, 0.0, 60.0),
        &mut work,
    );
    assert!(matches!(valid, Ok(Some(group)) if !group.fragments.is_empty()));
    for height in [0.0, f32::INFINITY, f32::NAN] {
        let control = PageLayoutControl::new(None);
        let mut work = ProjectionWork::new(&control, tree.fragment_tree.limit);
        let result = paragraph_group::paginate(
            &tree,
            parent,
            &entries,
            context,
            0.0,
            &|_| (0, 0.0, height),
            &mut work,
        );
        assert!(matches!(result, Ok(None)));
    }
}
