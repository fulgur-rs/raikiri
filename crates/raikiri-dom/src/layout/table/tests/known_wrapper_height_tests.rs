use super::*;

fn fixture(caption: bool) -> (Document, [usize; 2]) {
    let mut tables = [0; 2];
    let fixture =
        crate::layout::ifc::test_support::block_fixture("width:200px;height:300px", |doc, root| {
            for table in &mut tables {
                *table = doc.append_element(
                    Some(root),
                    "div",
                    Some("display:table;width:100px;padding:20px;box-sizing:border-box"),
                );
                if caption {
                    doc.append_element(
                        Some(*table),
                        "div",
                        Some("display:table-caption;width:60px;height:10px"),
                    );
                }
                doc.append_element(
                    Some(*table),
                    "div",
                    Some("display:block;width:60px;height:60px"),
                );
            }
        });
    (fixture.doc, tables)
}

fn input(run_mode: RunMode, height: f32) -> LayoutInput {
    LayoutInput {
        run_mode,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: None,
            height: Some(height),
        },
        known_dimensions_are_definite: Size {
            width: false,
            height: true,
        },
        parent_size: Size {
            width: Some(200.0),
            height: Some(300.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(100.0),
            height: AvailableSpace::Definite(height),
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

#[test]
fn measured_table_height_keeps_both_cache_orders_separate() {
    for mode in [RunMode::ComputeSize, RunMode::PerformLayout] {
        for order in [[false, true, false], [true, false, true]] {
            let (mut doc, [table, _]) = fixture(true);
            with_layout_session(&mut doc, |tree| {
                let heights = order.map(|wrapper| {
                    let compute = |tree: &mut LayoutSession<'_>| {
                        tree.compute_child_layout(NodeId::from(table), input(mode, 110.0))
                            .size
                            .height
                    };
                    let height = if wrapper {
                        tree.with_measured_table_height(table, compute)
                    } else {
                        compute(tree)
                    };
                    if mode == RunMode::PerformLayout {
                        assert_eq!(
                            tree.nodes[table].table_grid_box.unwrap().height,
                            height - 10.0
                        );
                    }
                    height
                });
                assert_eq!(
                    heights,
                    order.map(|wrapper| if wrapper { 110.0 } else { 80.0 })
                );
            });
        }
    }
}

#[test]
fn measured_table_height_restores_the_outer_domain_after_unwind() {
    let (mut doc, [outer, inner]) = fixture(false);
    with_layout_session(&mut doc, |tree| {
        let height = |tree: &mut LayoutSession<'_>, table| {
            tree.compute_child_layout(NodeId::from(table), input(RunMode::ComputeSize, 100.0))
                .size
                .height
        };
        tree.with_measured_table_height(outer, |tree| {
            assert_eq!(height(tree, outer), 100.0);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tree.with_measured_table_height(inner, |tree| {
                    assert_eq!(height(tree, inner), 100.0);
                    panic!("table layout unwind");
                });
            }));
            assert!(result.is_err());
            assert_eq!(height(tree, outer), 100.0);
            assert_eq!(height(tree, inner), 60.0);
        });
        assert_eq!(height(tree, outer), 60.0);
    });
}
