use super::*;
use crate::layout::ifc::test_support::{ahem_fonts, block_fixture};
use shodo::LayoutContext;
#[cfg(debug_assertions)]
use std::cell::Cell;

#[cfg(debug_assertions)]
thread_local! {
    static PROBES: Cell<usize> = const { Cell::new(0) };
}

#[cfg(debug_assertions)]
pub(in crate::layout::ifc::boxes) fn note_probe() {
    PROBES.with(|count| count.set(count.get() + 1));
}

fn projected_breaks(cleared: usize, total: usize) -> super::super::super::projection::ProjectedIfc {
    let fixture = block_fixture("white-space:pre;line-height:10px", |doc, root| {
        doc.append_text(root, "A");
        for index in 0..total {
            let side = if index < cleared {
                ["left", "right", "both"][index % 3]
            } else {
                "none"
            };
            doc.append_element(
                Some(root),
                "br",
                taffy::Style::default(),
                Some(format!("display:inline;clear:{side}").as_str()),
            );
            doc.append_text(root, "A");
        }
        doc.append_text(root, "\nB");
    });
    crate::layout::ifc::projection::project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut LayoutContext::new(),
        &ahem_fonts(),
        &shodo::limits::Limits::default(),
    )
    .unwrap()
}

fn paragraph_lines(root: &IfcRoot) -> Vec<shodo::Line> {
    root.paragraph.break_all(
        &mut LayoutContext::new(),
        &root.options,
        200.0,
        &AtomicSizes::EMPTY,
    )
}

#[test]
fn clear_break_lookup_preserves_first_entries_and_replayed_queries() {
    let mut projected = projected_breaks(9, 16);
    let expected = projected.meta.cleared_breaks.clone();
    projected.meta.cleared_breaks.reverse();
    projected
        .meta
        .cleared_breaks
        .push((expected[0].0, taffy::Clear::Right));
    let root = IfcRoot::new(projected);
    let copy = root.without_lines();
    let lines = paragraph_lines(&root);
    assert!(lines.iter().any(|line| line.forced_break().is_none()));
    for line in lines.iter().rev().chain(&lines).chain(lines.iter().rev()) {
        let value = line.forced_break().and_then(|forced| {
            expected
                .iter()
                .find(|(node, _)| *node == forced.node.0 as usize)
                .map(|(_, side)| *side)
        });
        assert_eq!(cleared_break(line, &root.cleared_breaks), value);
        assert_eq!(cleared_break(line, &copy.cleared_breaks), value);
    }
}

#[cfg(debug_assertions)]
#[test]
fn clear_break_lookup_probes_are_linear_in_queries() {
    for (cleared, queries) in [(256, 256), (32, 512), (512, 32), (0, 128)] {
        let root = IfcRoot::new(projected_breaks(cleared, cleared.max(queries)));
        let lines = paragraph_lines(&root);
        let forced: Vec<_> = lines
            .iter()
            .filter(|line| line.forced_break().is_some())
            .collect();
        PROBES.with(|count| count.set(0));
        for line in forced.iter().rev().cycle().take(queries) {
            std::hint::black_box(cleared_break(line, &root.cleared_breaks));
        }
        let probes = PROBES.with(Cell::get);
        println!("cleared={cleared} queries={queries}: probes={probes}");
        assert!(
            probes <= 4 * queries,
            "{probes} probes exceed linear bound {}",
            4 * queries
        );
    }
}

fn float_fixture(floats: bool, extra: &str) -> crate::layout::ifc::test_support::Fixture {
    let tail = format!("D\nE{extra}");
    block_fixture(
        "width:200px;white-space:pre;line-height:10px",
        |doc, root| {
            if floats {
                for (side, height) in [("left", 30), ("right", 50)] {
                    doc.append_element(
                        Some(root),
                        "div",
                        taffy::Style::default(),
                        Some(
                            format!("display:block;float:{side};width:20px;height:{height}px")
                                .as_str(),
                        ),
                    );
                }
            }
            doc.append_text(root, "A");
            for (clear, text) in [
                ("left", ""),
                ("none", "B"),
                ("right", "C"),
                ("both", tail.as_str()),
            ] {
                doc.append_element(
                    Some(root),
                    "br",
                    taffy::Style::default(),
                    Some(format!("display:inline;clear:{clear}").as_str()),
                );
                doc.append_text(root, text);
            }
        },
    )
}

#[test]
fn cleared_break_positions_survive_trial_and_relayout_with_and_without_floats() {
    for floats in [false, true] {
        let mut fixture = float_fixture(floats, "");
        crate::layout::layout_single_page(
            with_ahem(&mut fixture.doc),
            &fixture.cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let expected = if floats {
            vec![0.0, 30.0, 40.0, 50.0, 60.0, 70.0]
        } else {
            vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0]
        };
        let positions = |lines: &[shodo::Line]| {
            lines
                .iter()
                .map(shodo::Line::block_offset)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            positions(fixture.doc.nodes[fixture.root].ifc_lines().unwrap()),
            expected
        );
        for perform in [false, true, false, true] {
            let lines = layout_with_boxes(
                &mut fixture.doc,
                fixture.root,
                FlowGeometry::horizontal(200.0, (0.0, 0.0), 0.0),
                None,
                perform,
            );
            assert_eq!(positions(&lines.lines), expected);
        }
        crate::layout::layout_single_page(
            with_ahem(&mut fixture.doc),
            &fixture.cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        assert_eq!(
            positions(fixture.doc.nodes[fixture.root].ifc_lines().unwrap()),
            expected
        );
    }
}

fn line_geometry(lines: &IfcLines) -> String {
    let metrics: Vec<_> = lines
        .lines
        .iter()
        .map(|line| {
            (
                line.block_offset(),
                line.block_size(),
                line.inline_size(),
                line_start_x(line),
                line.text_range(),
                line.forced_break().map(|forced| forced.node.0),
            )
        })
        .collect();
    let ranges = lines.fragmentainer_line_ranges.as_ref().map(|ranges| {
        ranges
            .iter()
            .map(|range| {
                (
                    range.line_start,
                    range.line_end,
                    range.fragmentainer,
                    range.x,
                    range.y,
                )
            })
            .collect::<Vec<_>>()
    });
    format!(
        "{metrics:?}/{ranges:?}/{:?}/{:?}",
        lines.height, lines.unfragmented_tail_column
    )
}

#[test]
fn clear_break_fragmentainer_geometry_survives_trial_and_commit() {
    for floats in [false, true] {
        let mut fixture = float_fixture(floats, "\nF\nG\nH\nI");
        crate::layout::layout_single_page(
            with_ahem(&mut fixture.doc),
            &fixture.cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        let context = FragmentationContext {
            available_width: 200.0,
            available_height: Some(60.0),
            column_fill: raikiri_style::property::ColumnFillValue::Auto,
            column_width: 60.0,
            column_count: 3,
            column_gap: 10.0,
            column_index: 0,
            origin_x: 0.0,
            origin_y: 0.0,
            orphans: 1,
            widows: 1,
        };
        let mut expected = None;
        for perform in [false, true, false, true] {
            let lines = layout_with_boxes_in_fragmentainers(
                &mut fixture.doc,
                fixture.root,
                FlowGeometry::horizontal(200.0, (0.0, 0.0), 0.0),
                context,
                perform,
            );
            assert!(lines.fragmentainer_line_ranges.as_ref().unwrap().len() >= 2);
            let geometry = line_geometry(&lines);
            if let Some(expected) = &expected {
                assert_eq!(&geometry, expected);
            } else {
                expected = Some(geometry);
            }
        }
    }
}

#[test]
#[ignore = "manual release performance measurement"]
fn clear_break_scaling_probe() {
    use std::hint::black_box;
    use std::time::Instant;
    for (name, count, queries) in [
        ("empty", 0, 16),
        ("small", 1, 1),
        ("B256_L256", 256, 256),
        ("B1024_L1024", 1024, 1024),
        ("B32_L1024", 32, 1024),
        ("B1024_L32", 1024, 32),
    ] {
        let root = IfcRoot::new(projected_breaks(count, count.max(queries)));
        let lines = paragraph_lines(&root);
        let forced: Vec<_> = lines
            .iter()
            .filter(|line| line.forced_break().is_some())
            .collect();
        let started = Instant::now();
        for _ in 0..1_000 {
            for line in forced.iter().rev().cycle().take(queries) {
                black_box(cleared_break(
                    black_box(line),
                    black_box(&root.cleared_breaks),
                ));
            }
        }
        println!(
            "{name}: us_per_iteration={:.3}",
            started.elapsed().as_secs_f64() * 1e6 / 1_000.0
        );
        let projected: Vec<_> = (0..20)
            .map(|_| projected_breaks(count, count.max(queries)))
            .collect();
        let started = Instant::now();
        let roots: Vec<_> = projected.into_iter().map(IfcRoot::new).collect();
        let build_us = started.elapsed().as_secs_f64() * 1e6 / 20.0;
        let started = Instant::now();
        for _ in 0..1_000 {
            black_box(roots[0].without_lines());
        }
        println!(
            "{name}: build_us={build_us:.3} clone_us={:.3}",
            started.elapsed().as_secs_f64() * 1e6 / 1_000.0
        );
    }
}
