use std::cell::Cell;

use crate::layout::ifc::test_support::{Fixture, ahem_fonts, sheet_fixture};
use crate::{PageSlice, layout_single_page};
use raikiri_traits::PageBox;

#[derive(Clone, Copy, Debug, Default)]
struct Work {
    geometry_lines: usize,
    line_candidates: usize,
    paragraph_nodes: usize,
    clip_probes: usize,
}

thread_local! {
    static WORK: Cell<Work> = const { Cell::new(Work {
        geometry_lines: 0,
        line_candidates: 0,
        paragraph_nodes: 0,
        clip_probes: 0,
    }) };
}

pub(crate) fn geometry_lines(lines: usize) {
    WORK.with(|counter| {
        let mut work = counter.get();
        work.geometry_lines += lines;
        counter.set(work);
    });
}

pub(crate) fn line_candidate() {
    WORK.with(|counter| {
        let mut work = counter.get();
        work.line_candidates += 1;
        counter.set(work);
    });
}

pub(crate) fn paragraph_node() {
    WORK.with(|counter| {
        let mut work = counter.get();
        work.paragraph_nodes += 1;
        counter.set(work);
    });
}

pub(crate) fn clip_probe() {
    WORK.with(|counter| {
        let mut work = counter.get();
        work.clip_probes += 1;
        counter.set(work);
    });
}

#[test]
fn all_page_text_preparation_visits_a_long_paragraph_linearly() {
    let mut observations = Vec::new();
    for flavor in 0..3 {
        let rich = flavor != 0;
        for page_count in [8usize, 32, 128] {
            let line_count = page_count * 10;
            let Fixture {
                mut doc,
                cascade,
                root,
            } = sheet_fixture(
                if rich {
                    "div::first-line{opacity:.5}"
                } else {
                    ""
                },
                "width:10px;line-height:10px",
                |doc, root| {
                    if rich {
                        for index in 0..line_count {
                            let span = doc.append_element(
                                Some(root),
                                "span",
                                crate::LayoutStyle::default(),
                                Some(if flavor == 2 {
                                    "opacity:.5"
                                } else if index % 2 == 0 {
                                    "color:blue"
                                } else {
                                    "color:red"
                                }),
                            );
                            doc.append_text(span, if index + 1 == line_count { "X" } else { "X " });
                        }
                    } else {
                        doc.append_text(root, vec!["X"; line_count].join(" "));
                    }
                },
            );
            doc.set_font_collection(ahem_fonts());
            let mut page = PageBox::new();
            page.width = 100.0;
            page.height = 100.0;
            layout_single_page(&mut doc, &cascade, page).unwrap();
            assert_eq!(
                doc.ifc_layout_node(root)
                    .unwrap()
                    .ifc_lines()
                    .unwrap()
                    .len(),
                line_count
            );
            let slices: Vec<_> = (0..page_count)
                .map(|index| PageSlice {
                    page_index: index as u32,
                    content_origin_y: index as f32 * 100.0,
                    page_name: None,
                })
                .collect();
            WORK.set(Work::default());
            doc.project_pages(&cascade, page, &slices, &[]).unwrap();
            let mut drawn = 0;
            let mut event_count = 0;
            for index in 0..page_count {
                let runs = doc.page_text_runs(&cascade, index as u32);
                drawn += runs
                    .iter()
                    .map(|run| run.text.matches('X').count())
                    .sum::<usize>();
                event_count += doc
                    .page_paint_order_for_text_runs(&cascade, index as u32, &runs)
                    .len();
            }
            assert_eq!(drawn, line_count);
            assert!(event_count >= line_count);
            let work = WORK.get();
            eprintln!(
                "flavor={flavor}, rich={rich}, pages={page_count}, lines={line_count}, events={event_count}, work={work:?}"
            );
            observations.push((rich, page_count, line_count, doc.node_count(), work));
        }
    }
    for (rich, pages, lines, nodes, work) in observations {
        assert!(
            work.geometry_lines <= 8 * lines + 32 * pages,
            "geometry rescans: rich={rich}, pages={pages}, lines={lines}, {work:?}"
        );
        assert!(
            work.line_candidates <= 8 * lines + 32 * pages,
            "line rescans: rich={rich}, pages={pages}, lines={lines}, {work:?}"
        );
        assert!(
            work.paragraph_nodes <= 16 * nodes + 64 * pages,
            "paragraph DOM rescans: rich={rich}, pages={pages}, nodes={nodes}, {work:?}"
        );
    }
}

#[test]
fn opacity_groups_with_atomic_boxes_use_shared_box_membership() {
    for spans in [20, 80, 320] {
        let fixture = sheet_fixture(
            "div::first-line{opacity:.5}",
            "width:20px;line-height:10px",
            |doc, root| {
                for _ in 0..spans {
                    let span = doc.append_element(
                        Some(root),
                        "span",
                        crate::LayoutStyle::default(),
                        Some("opacity:.5"),
                    );
                    doc.append_text(span, "X ");
                    doc.append_element(
                        Some(span),
                        "i",
                        crate::LayoutStyle::default(),
                        Some("display:inline-block;width:1px;height:1px"),
                    );
                }
            },
        );
        let (mut doc, cascade) = (fixture.doc, fixture.cascade);
        doc.set_font_collection(ahem_fonts());
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        let slices = crate::layout_pages(&mut doc, &cascade, page).unwrap();
        WORK.set(Work::default());
        doc.project_pages(&cascade, page, &slices, &[]).unwrap();
        let mut events = 0;
        let mut glyphs = 0;
        for slice in &slices {
            let runs = doc.page_text_runs(&cascade, slice.page_index);
            glyphs += runs
                .iter()
                .map(|run| run.text.matches('X').count())
                .sum::<usize>();
            events += doc
                .page_paint_order_for_text_runs(&cascade, slice.page_index, &runs)
                .len();
        }
        assert_eq!(glyphs, spans);
        let work = WORK.get();
        eprintln!(
            "atomic spans={spans}, pages={}, events={events}, work={work:?}",
            slices.len()
        );
        // Rest preserves groups with off-page atomic boxes, so their empty
        // group events count as output in the bound.
        assert!(
            work.paragraph_nodes <= 32 * doc.node_count() + 16 * events + 64 * slices.len(),
            "{work:?}"
        );
    }
}

#[test]
fn multicol_event_clip_queries_do_not_scan_every_line_in_every_column() {
    use crate::fragment::{FragmentRect, LayoutFragment};
    use crate::node::MulticolTextFragment;
    for count in [32usize, 128, 512] {
        let fixture = sheet_fixture(
            "",
            "width:10px;white-space:pre;line-height:10px",
            |doc, root| {
                doc.append_text(root, vec!["X"; count].join("\n"));
            },
        );
        let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
        doc.set_font_collection(ahem_fonts());
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_single_page(&mut doc, &cascade, page).unwrap();
        assert_eq!(doc.nodes[root].ifc_lines().unwrap().len(), count);
        doc.nodes[root].ifc.as_mut().unwrap().multicol_fragments = Some(
            (0..count)
                .map(|column| MulticolTextFragment {
                    line_start: column,
                    line_end: column + 1,
                    fragmentainer: column,
                    x: column as f32 * 20.0,
                    y: 0.0,
                })
                .collect(),
        );
        doc.fragment_tree.fragments.clear();
        for column in 0..count {
            let rect = FragmentRect {
                x: column as f32 * 20.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            };
            doc.fragment_tree
                .try_push(LayoutFragment {
                    node_id: root,
                    parent: None,
                    fragmentainer: column,
                    rect,
                    fragmentainer_clip: Some(rect),
                    fragment_index: column,
                    fragment_count: count,
                    line_start: Some(column),
                    line_end: Some(column + 1),
                })
                .unwrap();
        }
        WORK.set(Work::default());
        doc.project_pages(
            &cascade,
            page,
            &[PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            &[],
        )
        .unwrap();
        let runs = doc.page_text_runs(&cascade, 0);
        assert_eq!(
            runs.iter()
                .map(|run| run.text.matches('X').count())
                .sum::<usize>(),
            count
        );
        let events = doc.page_paint_order_for_text_runs(&cascade, 0, &runs);
        assert!(!events.is_empty());
        assert!(doc.page_projection.text_index.clip_range_count() <= 2 * count);
        let probes = WORK.get().clip_probes;
        eprintln!("clip columns={count}, probes={probes}");
        assert!(
            probes <= 8 * count * (count.ilog2() as usize + 2) + 256,
            "{count} columns required {probes} clip probes"
        );
    }
}
