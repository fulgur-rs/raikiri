use super::*;
use crate::fragment::FragmentRect;
use crate::layout::page_pipeline::PageLayoutControl;

fn source_fragments() -> (Document, usize, usize, usize) {
    let mut tree = Document::new();
    let owner = tree.append_element(Some(0), "div", None::<&str>);
    let child = tree.append_element(Some(owner), "p", None::<&str>);
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
    let mut edits = FragmentEdits::new(&tree.fragment_tree);
    edits.shift_children(container, &shifts);
    let control = PageLayoutControl::new(None);
    let mut work = ProjectionWork::new(&control, tree.fragment_tree.limit);
    edits.finish(&mut tree, &mut work).unwrap();
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
    assert_eq!(tree.nodes[child].layout.location.y, 10.0);
}

#[test]
fn replacement_work_limit_rejects_before_installing_a_partial_fragment_stream() {
    let (mut tree, _, child, _) = source_fragments();
    tree.fragment_tree.limit = 8;
    let original = tree.fragment_tree.fragments.clone();
    let mut edits = FragmentEdits::new(&tree.fragment_tree);
    edits.replace(child, vec![original[1]; 7]);
    let control = PageLayoutControl::new(None);
    let mut work = ProjectionWork::new(&control, 8);
    assert!(matches!(
        edits.finish(&mut tree, &mut work),
        Err(LayoutError::FragmentLimitExceeded { limit: 8 })
    ));
    assert_eq!(tree.fragment_tree.fragments, original);
}

// A later owner can replace or move the records of an earlier owner's
// replacements; every change applies to the records that describe a node at
// that moment, and records placed in a replaced record follow its first
// replacement.
#[test]
fn edits_apply_to_the_records_a_node_has_now() {
    let (mut tree, owner, child, container) = source_fragments();
    let grandchild = tree.append_element(Some(child), "span", None::<&str>);
    let mut placed = tree.fragment_tree.fragments[1];
    placed.node_id = grandchild;
    placed.parent = Some(1);
    placed.rect.y = 5.0;
    tree.fragment_tree.try_push(placed).unwrap();
    let mut edits = FragmentEdits::new(&tree.fragment_tree);
    let mut split = tree.fragment_tree.fragments[1];
    split.rect.y = 0.0;
    let mut second = split;
    second.fragmentainer = 1;
    edits.replace(child, vec![split, second]);
    // The grandchild is placed in the child's first replacement now.
    edits.shift_following(child, &HashMap::from([(grandchild, 3.0)]));
    edits.shift_children(container, &HashMap::from([(child, 2.0)]));
    edits.grow(owner, 10.0);
    // A node without records gets none.
    edits.replace(tree.nodes.len() + 1, vec![split]);
    let control = PageLayoutControl::new(None);
    let mut work = ProjectionWork::new(&control, tree.fragment_tree.limit);
    edits.finish(&mut tree, &mut work).unwrap();
    let fragments = &tree.fragment_tree.fragments;
    assert_eq!(fragments.len(), 4);
    assert_eq!(fragments[0].rect.height, 70.0);
    assert_eq!((fragments[1].node_id, fragments[1].rect.y), (child, 2.0));
    assert_eq!(
        (fragments[2].node_id, fragments[2].fragmentainer),
        (child, 1)
    );
    assert_eq!(fragments[2].parent, Some(0));
    assert_eq!(
        (fragments[3].node_id, fragments[3].parent),
        (grandchild, Some(1))
    );
    assert_eq!(fragments[3].rect.y, 8.0);
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
    let measured = tree.nodes[root].layout;
    let entries = [(
        root,
        LayoutOutput::from_outer_size(measured.size),
        measured,
        0.0,
        0.0,
        20.0,
    )];
    let context = FragmentationContext {
        rtl: false,
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
        &LayoutSession::resumed(&mut tree, &cascade),
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
            &LayoutSession::resumed(&mut tree, &cascade),
            &entries,
            context,
            0.0,
            &|_| (0, 0.0, height),
            &mut work,
        );
        assert!(matches!(result, Ok(None)));
    }
}

/// A spanner of `height` that holds `children` empty blocks, laid out on a
/// page. Returns the document with its cascade and the spanner.
fn spanner_with_blocks(
    children: usize,
    height: f32,
) -> (Document, raikiri_style::CascadeResult, usize) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    document.append_element(Some(html), "head", None::<&str>);
    let body = document.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font:10px/10px Ahem"),
    );
    let spanner = document.append_element(
        Some(body),
        "div",
        Some(format!("display:block;height:{height}px")),
    );
    for _ in 0..children {
        document.append_element(Some(spanner), "div", Some("display:block"));
    }
    let rules = raikiri_style::build_rule_tree(&document);
    let cascade = raikiri_style::cascade(&document, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 100.0;
    page.height = 1000.0;
    crate::layout::layout_single_page(&mut document, &cascade, page).unwrap();
    (document, cascade, spanner)
}

/// Pages of 10px each, as `(index, origin, height)`.
fn short_pages(y: f32) -> (u32, f32, f32) {
    let index = (y / 10.0).floor() as u32;
    (index, index as f32 * 10.0, 10.0)
}

/// The scan of a spanner that starts 5px into the first page, so that it
/// crosses the page ends at 10, 20, ..., 200.
fn scan_cost(control: &PageLayoutControl<'_>) -> (Result<bool, LayoutError>, usize) {
    let (mut document, cascade, spanner) = spanner_with_blocks(50, 200.0);
    let tree = crate::layout::context::LayoutSession::bridged(&mut document, &cascade);
    let mut work = ProjectionWork::new(control, 10_000);
    let result = breaks_at_every_page_end(&tree, spanner, 5.0, 10.0, &short_pages, &mut work);
    (result, 10_000 - work.remaining())
}

#[test]
fn the_scan_of_a_spanner_charges_each_box_and_each_page_end_once() {
    let (result, cost) = scan_cost(&PageLayoutControl::new(None));
    assert!(result.unwrap());
    // The spanner and its 50 blocks are visited once, whatever the number of
    // page ends, and each of the 20 page ends is charged.
    assert_eq!(cost, 51 + 20);
}

#[test]
fn the_scan_of_a_spanner_stops_where_discovery_truncates_at_the_page_limit() {
    let control = PageLayoutControl::for_geometry_discovery(Some(3));
    let (result, cost) = scan_cost(&control);
    // Pages 1 and 2 exist; the cut at the end of page 2 would continue on page
    // 3, which discovery does not lay out.
    assert!(result.unwrap());
    assert!(control.page_limit_reached());
    assert_eq!(cost, 51 + 3);
}

#[test]
fn the_scan_of_a_spanner_fails_at_the_page_limit_of_a_strict_control() {
    let (result, cost) = scan_cost(&PageLayoutControl::new(Some(3)));
    assert!(matches!(
        result,
        Err(LayoutError::PageLimitExceeded {
            limit: 3,
            actual: 4
        })
    ));
    assert_eq!(cost, 51 + 3);
}

#[test]
fn the_scan_of_a_spanner_checks_for_an_abort_while_it_visits_the_boxes() {
    // The control is asked once per box visited. At each page end it is asked
    // again when the page end is charged and when its page is looked up.
    let asked = std::cell::Cell::new(0_usize);
    let abort = || {
        asked.set(asked.get() + 1);
        asked.get() > 10
    };
    let control = PageLayoutControl::new(None).with_abort_check(&abort);
    let (result, cost) = scan_cost(&control);
    assert!(matches!(result, Err(LayoutError::Aborted)));
    assert_eq!(asked.get(), 11);
    assert_eq!(cost, 10);
}
