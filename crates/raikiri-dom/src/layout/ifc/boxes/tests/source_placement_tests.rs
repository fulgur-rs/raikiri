use super::*;

fn context(width: f32) -> FragmentationContext {
    FragmentationContext {
        rtl: false,
        available_width: width * 2.0,
        available_height: Some(40.0),
        column_fill: raikiri_style::property::ColumnFillValue::Auto,
        column_width: width,
        column_count: 2,
        column_gap: 0.0,
        column_index: 0,
        origin_x: 0.0,
        origin_y: 0.0,
        orphans: 1,
        widows: 1,
    }
}

fn reset_counts() {
    SOURCE_INPUTS.with(|count| count.set(0));
    SOURCE_COMPARISONS.with(|count| count.set(0));
    SOURCE_DUPLICATES.with(|count| count.set(0));
}

#[test]
fn source_placement_membership_work_stays_linear() {
    let mut measurements = Vec::new();
    for count in [128, 256, 512] {
        let width = count as f32 + 20.0;
        let mut floats = Vec::new();
        let (mut doc, cascade, root) = crate::layout::test_support::ahem_paragraph_with(
            &format!("width:{width}px"),
            |doc, root| {
                for _ in 0..count {
                    floats.push(doc.append_element(
                        Some(root),
                        "span",
                        Some("float:left;width:1px;height:1px"),
                    ));
                }
                doc.append_text(root, "A");
            },
        );
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        crate::layout::layout_single_page(
            with_ahem(&mut doc),
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .expect("preparation layout");
        reset_counts();
        let output = layout_with_boxes_in_fragmentainers(
            &mut doc,
            root,
            FlowGeometry::horizontal(width, (0.0, 0.0), 0.0),
            context(width),
            true,
        );
        assert_eq!(output.fragment_box_placements.len(), count);
        assert_eq!(output.lines.len(), 1);
        for (index, placement) in output.fragment_box_placements.iter().enumerate() {
            assert_eq!(placement.node_id, floats[index]);
            assert_eq!(placement.fragmentainer, 0);
            assert_eq!(
                (
                    placement.rect.x,
                    placement.rect.y,
                    placement.rect.width,
                    placement.rect.height
                ),
                (index as f32, 0.0, 1.0, 1.0)
            );
        }
        let inputs = SOURCE_INPUTS.with(|count| count.get());
        let comparisons = SOURCE_COMPARISONS.with(|count| count.get());
        let duplicates = SOURCE_DUPLICATES.with(|count| count.get());
        assert_eq!(inputs, count);
        assert_eq!(duplicates, 0);
        eprintln!(
            "source placements={count}, inputs={inputs}, comparisons={comparisons}, duplicates={duplicates}"
        );
        measurements.push((count, comparisons));
    }
    for (count, comparisons) in measurements {
        assert!(
            comparisons <= count * 8,
            "{count} placements required {comparisons} comparisons"
        );
    }
}

#[test]
fn source_placement_first_arrivals_keep_geometry_and_order_across_batches() {
    let placement = |node_id, fragmentainer, y| IfcBoxFragment {
        node_id,
        fragmentainer,
        rect: crate::fragment::FragmentRect {
            x: 3.0,
            y,
            width: 7.0,
            height: 80.0,
        },
    };
    let first = [placement(2, 0, 9.0), placement(1, 0, 4.0)];
    let second = [
        placement(1, 1, -36.0),
        placement(3, 1, 5.0),
        placement(2, 1, -31.0),
        placement(3, 2, -35.0),
    ];
    let mut sources = Vec::new();
    let mut ids = SourcePlacementIds::default();
    reset_counts();
    remember_source_placements(&mut sources, &mut ids, &first);
    remember_source_placements(&mut sources, &mut ids, &second);
    assert_eq!(sources, [first[0], first[1], second[1]]);
    assert_eq!(SOURCE_INPUTS.with(|count| count.get()), 6);
    assert_eq!(SOURCE_DUPLICATES.with(|count| count.get()), 3);
}

#[test]
fn source_placement_carry_preserves_first_column_geometry() {
    let (mut doc, cascade, root) =
        crate::layout::test_support::ahem_paragraph_with("width:100px", |doc, root| {
            doc.append_element(
                Some(root),
                "span",
                Some("float:left;width:10px;height:80px"),
            );
            for _ in 0..12 {
                doc.append_text(root, "A");
                doc.append_element(Some(root), "br", None::<&str>);
            }
        });
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(
        with_ahem(&mut doc),
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .expect("preparation layout");
    reset_counts();
    let output = layout_with_boxes_in_fragmentainers(
        &mut doc,
        root,
        FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0),
        context(100.0),
        true,
    );
    let placements = &output.fragment_box_placements;
    assert_eq!(placements.len(), 2);
    assert_eq!(placements[0].node_id, placements[1].node_id);
    for (column, placement) in placements.iter().enumerate() {
        assert_eq!(placement.fragmentainer, column);
        assert_eq!(
            (
                placement.rect.x,
                placement.rect.y,
                placement.rect.width,
                placement.rect.height
            ),
            (0.0, -(column as f32) * 40.0, 10.0, 80.0)
        );
    }
    assert_eq!(output.lines.len(), 12);
    assert_eq!(line_start_x(&output.lines[0]), Some(10.0));
    assert_eq!(line_start_x(&output.lines[4]), Some(10.0));
    assert_eq!(line_start_x(&output.lines[8]), Some(0.0));
    assert_eq!(SOURCE_INPUTS.with(|count| count.get()), 1);
    assert_eq!(SOURCE_DUPLICATES.with(|count| count.get()), 0);
}
