use super::*;
use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::taffy_adapter::algorithms::grid_item_row_rooms;
use crate::layout::test_support::cascaded;

#[test]
fn provisional_rows_include_only_in_flow_items_and_do_not_publish_layout() {
    let mut doc = Document::new();
    let grid = doc.append_element(
        Some(0),
        "div",
        Some("display:grid;width:100px;height:100px"),
    );
    let first = doc.append_element(
        Some(grid),
        "div",
        Some("display:block;width:10px;height:40px"),
    );
    doc.append_element(
        Some(grid),
        "div",
        Some("display:block;position:absolute;height:1px"),
    );
    let hidden = doc.append_element(Some(grid), "div", Some("display:none;height:1px"));
    let second = doc.append_element(
        Some(grid),
        "div",
        Some("display:block;width:10px;height:40px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.nodes[hidden].layout.size.height = 7.0;
    let before = tree
        .nodes
        .iter()
        .map(|node| node.layout)
        .collect::<Vec<_>>();
    let rooms = grid_item_row_rooms(
        &mut tree,
        NodeId::from(grid),
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            available_space: Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            ..LayoutInput::HIDDEN
        },
    );
    assert_eq!(
        rooms,
        [(NodeId::from(first), 50.0), (NodeId::from(second), 50.0)]
    );
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        before
    );
    assert!(!tree.context.grid_layout_info.contains_key(&grid));
    assert!(tree.nodes[grid].grid_item_row_starts.is_empty());
    assert!(tree.nodes[grid].grid_item_rows.is_empty());
    assert!(tree.nodes[grid].grid_item_column_starts.is_empty());
    assert!(tree.nodes[grid].grid_item_areas.is_empty());
    assert!(tree.nodes[grid].grid_items_without_baseline.is_empty());
    assert_eq!(tree.nodes[grid].grid_column_count, 0);
}

#[test]
fn baseline_replay_keeps_nested_geometry_and_grid_metadata_private() {
    use crate::layout::test_support::with_ahem;

    for display in ["flex", "grid"] {
        let mut doc = Document::new();
        let grid = doc.append_element(
            Some(0),
            "div",
            Some("display:grid;width:20px;height:100px;grid-template-columns:auto auto"),
        );
        for (font, padding, text) in [(20, 0, "A"), (10, 20, "B")] {
            let item = doc.append_element(Some(grid), "div", Some(&format!(
                "display:{display};width:0;font-family:Ahem;font-size:{font}px;line-height:{font}px;padding-top:{padding}px;white-space:nowrap;align-self:baseline"
            )));
            let nested = doc.append_element(Some(item), "div", Some("display:grid"));
            doc.append_text(nested, text);
        }
        let last = doc.append_element(
            Some(grid),
            "div",
            Some("display:block;height:10px;grid-row:2;grid-column:1 / span 2"),
        );
        doc.mark_in_document_flags();
        with_ahem(&mut doc);
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        tree.nodes[grid].grid_column_count = 7;
        let snapshot = |tree: &LayoutSession<'_>| {
            tree.nodes
                .iter()
                .map(|node| {
                    (
                        node.layout,
                        node.grid_item_row_starts.to_vec(),
                        node.grid_item_rows.to_vec(),
                        node.grid_item_column_starts.to_vec(),
                        node.grid_items_without_baseline.to_vec(),
                        node.grid_item_areas.to_vec(),
                        node.grid_column_count,
                        node.exported_first_baseline,
                        node.flex_first_baseline,
                    )
                })
                .collect::<Vec<_>>()
        };
        let before = snapshot(&tree);
        let rooms = grid_item_row_rooms(
            &mut tree,
            grid.into(),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                available_space: Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                ..LayoutInput::HIDDEN
            },
        );
        // The first row combines the 28px ascent and 4px descent; the second
        // receives the remaining free space after its 10px base size.
        assert_eq!(rooms.last(), Some(&(last.into(), 39.0)), "{display}");
        assert_eq!(snapshot(&tree), before, "{display}");
        assert!(!tree.context.grid_layout_info.contains_key(&grid));
        assert!(tree.context.baseline_measure_owners.is_empty());
    }
}

#[test]
fn measuring_a_baseline_with_percentage_edges_keeps_size_and_geometry() {
    use crate::layout::test_support::with_ahem;

    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Some("display:flex;width:100px"));
    let grid = doc.append_element(
        Some(parent),
        "div",
        Some("display:grid;padding-top:10%;font-family:Ahem;font-size:20px;line-height:20px"),
    );
    doc.append_text(grid, "A");
    doc.mark_in_document_flags();
    with_ahem(&mut doc);
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.context.flex_item_percent_bases.push((
        parent,
        Size {
            width: Some(100.0),
            height: None,
        },
    ));
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        known_dimensions: Size {
            width: Some(20.0),
            height: Some(30.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(20.0),
            height: AvailableSpace::Definite(30.0),
        },
        ..LayoutInput::HIDDEN
    };
    let sized = tree.compute_child_layout(grid.into(), inputs);
    assert_eq!(sized.baselines.first, None);
    tree.context.baseline_measure_owners.push(grid);
    let measured = tree.compute_child_layout(grid.into(), inputs);
    tree.context.baseline_measure_owners.pop();
    assert_eq!(measured.size, sized.size);
    assert_eq!(measured.baselines.first, Some(26.0));
    assert_eq!(tree.nodes[grid].layout.size, Size::ZERO);
}

#[test]
fn nested_baseline_measure_requests_restore_the_previous_owner_after_a_panic() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Some("display:grid"));
    let child = doc.append_element(Some(parent), "div", Some("display:flex"));
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.context.baseline_measure_owners.push(0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.with_baseline_measure(parent, |tree| {
            tree.with_baseline_measure(child, |tree| {
                assert_eq!(tree.context.baseline_measure_owners, [0, parent, child]);
                panic!("baseline measurement interrupted");
            });
        });
    }));
    assert!(result.is_err());
    assert_eq!(tree.context.baseline_measure_owners, [0]);
}

#[test]
fn a_vertical_baseline_measure_keeps_the_horizontal_owner_and_exports_no_baseline() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "div", Some("display:grid"));
    let child = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;writing-mode:vertical-rl;width:10px;height:20px"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    tree.context.baseline_measure_owners.push(parent);
    let output = tree.with_baseline_measure(child, |tree| {
        assert_eq!(tree.context.baseline_measure_owners, [parent]);
        tree.compute_child_layout(
            child.into(),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                ..LayoutInput::HIDDEN
            },
        )
    });
    assert_eq!(output.baselines.first, None);
    assert_eq!(tree.context.baseline_measure_owners, [parent]);
    assert_eq!(tree.nodes[child].layout.size, Size::ZERO);
}

#[test]
fn measuring_an_overflowing_auto_margin_flex_baseline_keeps_geometry_private() {
    let mut doc = Document::new();
    let root = doc.append_element(
        Some(0),
        "div",
        Some("display:flex;width:100px;height:100px"),
    );
    doc.append_element(
        Some(root),
        "div",
        Some("width:20px;height:200px;flex:none;margin:auto 0"),
    );
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    let mut tree = LayoutSession::bridged(&mut doc, &cascade);
    let before: Vec<_> = tree.nodes.iter().map(|node| node.layout).collect();
    let inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        known_dimensions: Size {
            width: Some(100.0),
            height: Some(100.0),
        },
        parent_size: Size {
            width: Some(800.0),
            height: Some(600.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(100.0),
            height: AvailableSpace::Definite(100.0),
        },
        ..LayoutInput::HIDDEN
    };
    let measured =
        tree.with_baseline_measure(root, |tree| tree.compute_child_layout(root.into(), inputs));
    assert_eq!(measured.baselines.first, Some(200.0));
    assert_eq!(
        measured.size,
        Size {
            width: 100.0,
            height: 100.0
        }
    );
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| node.layout)
            .collect::<Vec<_>>(),
        before
    );
}
