use super::*;
use crate::layout::test_support::{ahem_paragraph_with, with_ahem};
use taffy::Style;

mod paged_spanning_tests;

fn committed_column_projection(style: &str) -> (Document, CascadeResult) {
    let (mut doc, cascade, root) = crate::layout::test_support::ahem_paragraph(
        "A\nB\nC\nD",
        &format!("width:40px;opacity:0.5;{style}"),
    );
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    set_committed_column_fragments(&mut doc, root);
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    (doc, cascade)
}

fn set_committed_column_fragments(doc: &mut Document, root: usize) {
    use crate::fragment::{FragmentRect, LayoutFragment};
    use crate::node::MulticolTextFragment;
    doc.fragment_tree.fragments.clear();
    // Supply an independently specified producer stream. The second source
    // box starts above its column so its original lines 2 and 3 start at y=0.
    doc.nodes[root].ifc.as_mut().unwrap().multicol_fragments = Some(vec![
        MulticolTextFragment {
            line_start: 0,
            line_end: 2,
            fragmentainer: 0,
            x: 0.0,
            y: 0.0,
        },
        MulticolTextFragment {
            line_start: 2,
            line_end: 4,
            fragmentainer: 1,
            x: 60.0,
            y: 20.0,
        },
    ]);
    for column in 0..2 {
        doc.fragment_tree
            .try_push(LayoutFragment {
                node_id: root,
                parent: None,
                fragmentainer: column,
                rect: FragmentRect {
                    x: column as f32 * 60.0,
                    y: -(column as f32) * 20.0,
                    width: 40.0,
                    height: 40.0,
                },
                fragmentainer_clip: Some(FragmentRect {
                    x: column as f32 * 60.0,
                    y: 0.0,
                    width: 40.0,
                    height: 20.0,
                }),
                fragment_index: column,
                fragment_count: 2,
                line_start: Some(column * 2),
                line_end: Some(column * 2 + 2),
            })
            .unwrap();
    }
}

#[test]
fn projected_column_clips_follow_committed_fragment_coordinates() {
    use crate::{ClipKind, PaintEvent};
    use raikiri_traits::PaintRect;
    let (doc, cascade) = committed_column_projection("");
    let runs = doc.page_text_runs(&cascade, 0);
    assert_eq!(
        runs.iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>(),
        [
            ("A", (0.0, 8.0)),
            ("B", (0.0, 18.0)),
            ("C", (60.0, 8.0)),
            ("D", (60.0, 18.0))
        ]
    );
    let events = doc.page_paint_order_for_text_runs(&cascade, 0, &runs);
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    for event in &events {
        match event {
            PaintEvent::PushClip(clip, kind) => {
                assert_eq!(*kind, ClipKind::Fragmentainer);
                clips.push(*clip);
            }
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                line_clips.push((line.index, clips.last().map(|clip| clip.rect)))
            }
            _ => {}
        }
    }
    assert!(clips.is_empty());
    let left = Some(PaintRect::new(0.0, 0.0, 40.0, 20.0));
    let right = Some(PaintRect::new(60.0, 0.0, 40.0, 20.0));
    assert_eq!(line_clips, [(0, left), (1, left), (2, right), (3, right)]);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, PaintEvent::PushOpacity(_)))
            .count(),
        1
    );
}

#[test]
fn column_break_clips_intersect_the_explicit_ancestor_clip() {
    use crate::PaintEvent;
    use crate::fragment::{FragmentRect, LayoutFragment};
    use raikiri_traits::PaintRect;
    let (mut doc, cascade) = committed_column_projection("");
    let root = doc.page_text_runs(&cascade, 0)[0].line.root.0 as usize;
    let body = doc.parent_of(root).unwrap();
    let parent = doc
        .fragment_tree
        .try_push(LayoutFragment {
            node_id: body,
            parent: None,
            fragmentainer: 0,
            rect: FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            },
            fragmentainer_clip: Some(FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 15.0,
            }),
            fragment_index: 0,
            fragment_count: 1,
            line_start: None,
            line_end: None,
        })
        .unwrap();
    for fragment in &mut doc.fragment_tree.fragments[..2] {
        fragment.parent = Some(parent);
    }
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    let runs = doc.page_text_runs(&cascade, 0);
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    for event in doc.page_paint_order_for_text_runs(&cascade, 0, &runs) {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                line_clips.push((line.index, clips.last().map(|clip| clip.rect)))
            }
            _ => {}
        }
    }
    let left = Some(PaintRect::new(0.0, 0.0, 40.0, 15.0));
    let right = Some(PaintRect::new(60.0, 0.0, 40.0, 15.0));
    assert_eq!(line_clips, [(0, left), (1, left), (2, right), (3, right)]);
}

#[test]
fn an_empty_paragraph_keeps_box_clips_without_inventing_line_events() {
    use crate::PaintEvent;
    let (mut doc, cascade) = committed_column_projection("");
    let root = doc.page_text_runs(&cascade, 0)[0].line.root.0 as usize;
    doc.nodes[root].ifc.as_mut().unwrap().lines = None;
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    let runs = doc.page_text_runs(&cascade, 0);
    assert!(runs.is_empty());
    let events = doc.page_paint_order_for_text_runs(&cascade, 0, &runs);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PaintEvent::TextLine(_)))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                PaintEvent::PushClip(_, crate::ClipKind::Fragmentainer)
            ))
            .count(),
        2
    );
}

#[test]
fn duplicated_outside_marker_placements_preserve_an_explicit_omission() {
    let (mut doc, cascade) = committed_column_projection(
        "display:list-item;list-style-position:outside;list-style-type:decimal",
    );
    let root = doc.fragment_tree.fragments[0].node_id;
    // A repeated ancestor can carry a second copy of the same first marker.
    doc.fragment_tree.fragments[1].fragment_index = 0;
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    assert!(doc.page_text_runs(&cascade, 0).is_empty());
    assert_eq!(
        doc.omitted_text_run_roots(&cascade),
        [(
            raikiri_traits::NodeId::new(root as u64),
            "the same paragraph line has overlapping column placements"
        )]
    );
}

#[test]
fn repeated_header_columns_shift_shared_overflow_and_break_clips_together() {
    use crate::PaintEvent;
    use raikiri_traits::PaintRect;
    let (mut doc, _, root) =
        crate::layout::test_support::ahem_paragraph("A\nB\nC\nD", "width:40px;overflow:hidden");
    let body = doc.parent_of(root).unwrap();
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display:table;width:100px;border-collapse:collapse"),
    );
    let header = doc.append_element(
        Some(table),
        "thead",
        Style::default(),
        Some("display:table-header-group"),
    );
    let row = doc.append_element(
        Some(header),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display:table-cell;padding:0"),
    );
    doc.append_child(cell, root).unwrap();
    let body_row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    doc.append_element(
        Some(body_row),
        "td",
        Style::default(),
        Some("display:table-cell;height:200px;padding:0"),
    );
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    // Prepare real table ownership, then supply independently scheduled
    // repeated-header and column placements to the projection boundary.
    doc.table_objects.headers =
        crate::layout::table::headers::HeaderRepeats::prepare(&doc, &cascade, body, 600.0, |_| 0.0);
    let repeated = doc.table_objects.headers.headers.get_mut(&header).unwrap();
    repeated.placements.insert(0, (0.0, 0.0));
    repeated.placements.insert(1, (100.0, 107.5));
    doc.table_objects.headers.index();
    set_committed_column_fragments(&mut doc, root);
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[
            PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            },
            PageSlice {
                page_index: 1,
                content_origin_y: 100.0,
                page_name: None,
            },
        ],
        &[],
    )
    .unwrap();
    let overflow: Vec<_> = doc.page_overflow_clips(1).collect();
    assert_eq!(overflow.len(), 2);
    assert_eq!(
        overflow
            .iter()
            .map(|entry| entry.border_box)
            .collect::<Vec<_>>(),
        [
            PaintRect::new(0.0, 7.5, 40.0, 40.0),
            PaintRect::new(60.0, -12.5, 40.0, 40.0),
        ]
    );
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(
        runs.iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>(),
        [
            ("A", (0.0, 15.5)),
            ("B", (0.0, 25.5)),
            ("C", (60.0, 15.5)),
            ("D", (60.0, 25.5)),
        ]
    );
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    for event in doc.page_paint_order_for_text_runs(&cascade, 1, &runs) {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                line_clips.push((line.index, clips.last().map(|clip| clip.rect)))
            }
            _ => {}
        }
    }
    let left = Some(PaintRect::new(0.0, 7.5, 40.0, 20.0));
    let right = Some(PaintRect::new(60.0, 7.5, 40.0, 20.0));
    assert_eq!(line_clips, [(0, left), (1, left), (2, right), (3, right)]);
}

#[test]
fn an_outside_text_marker_keeps_its_explicit_column_clip() {
    use crate::PaintEvent;
    use raikiri_traits::PaintRect;
    let (doc, cascade) = committed_column_projection(
        "display:list-item;list-style-position:outside;list-style-type:decimal",
    );
    let runs = doc.page_text_runs(&cascade, 0);
    let marker = runs
        .iter()
        .find(|run| run.is_standalone_marker())
        .expect("outside marker");
    let events = doc.page_paint_order_for_text_runs(&cascade, 0, &runs);
    let mut clips = Vec::new();
    let mut marker_clips = Vec::new();
    for event in events {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) if line == marker.line => {
                marker_clips.push(clips.last().map(|clip| clip.rect))
            }
            _ => {}
        }
    }
    assert_eq!(marker_clips, [Some(PaintRect::new(0.0, 0.0, 40.0, 20.0))]);
}

#[test]
fn column_clips_and_lines_share_fractional_page_margins_and_slice_origin() {
    use crate::PaintEvent;
    use raikiri_traits::PaintRect;
    let (mut doc, cascade) = committed_column_projection("");
    let page = page_box_800x600();
    let margins = PageMargins {
        top: 7.25,
        left: 3.25,
        ..Default::default()
    };
    let insets = PageContentInsets {
        top: 2.5,
        left: 1.5,
        ..Default::default()
    };
    doc.project_pages(
        &cascade,
        page,
        &[
            PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            },
            PageSlice {
                page_index: 1,
                content_origin_y: 10.5,
                page_name: None,
            },
        ],
        &[(page, margins, insets); 2],
    )
    .unwrap();
    let runs = doc.page_text_runs(&cascade, 1);
    assert_eq!(
        runs.iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>(),
        [("B", (4.75, 17.25)), ("D", (64.75, 17.25)),]
    );
    let events = doc.page_paint_order_for_text_runs(&cascade, 1, &runs);
    let mut clips = Vec::new();
    let mut line_clips = Vec::new();
    for event in events {
        match event {
            PaintEvent::PushClip(clip, _) => clips.push(clip),
            PaintEvent::PopClip => {
                clips.pop().unwrap();
            }
            PaintEvent::TextLine(line) => {
                line_clips.push((line.index, clips.last().map(|clip| clip.rect)))
            }
            _ => {}
        }
    }
    assert_eq!(
        line_clips,
        [
            (1, Some(PaintRect::new(4.75, -0.75, 40.0, 20.0))),
            (3, Some(PaintRect::new(64.75, -0.75, 40.0, 20.0))),
        ]
    );
    assert!(clips.is_empty());
}

#[test]
fn overlapping_column_line_placements_keep_an_explicit_omission() {
    for scenario in 0..3 {
        let (mut doc, cascade) = committed_column_projection("");
        let root = doc.page_text_runs(&cascade, 0)[0].line.root.0 as usize;
        if scenario == 0 {
            doc.nodes[root]
                .ifc
                .as_mut()
                .unwrap()
                .multicol_fragments
                .as_mut()
                .unwrap()[0]
                .line_end = 3;
        } else if scenario == 1 {
            doc.nodes[root].ifc.as_mut().unwrap().multicol_fragments = None;
        } else {
            doc.fragment_tree.fragments[1].fragmentainer = 0;
        }
        doc.project_pages(
            &cascade,
            page_box_800x600(),
            &[PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            &[],
        )
        .unwrap();
        assert!(doc.page_text_runs(&cascade, 0).is_empty());
        assert_eq!(
            doc.omitted_text_run_roots(&cascade),
            [(
                raikiri_traits::NodeId::new(root as u64),
                "the same paragraph line has overlapping column placements"
            )]
        );
    }
}

#[test]
fn page_layout_control_cancels_candidate_collection() {
    use std::cell::Cell;

    let (mut document, cascade) = hello_world_doc();
    let polls = Cell::new(0);
    let abort_check = || {
        let next = polls.get() + 1;
        polls.set(next);
        next >= 4
    };
    let control = PageLayoutControl::new(Some(10)).with_abort_check(&abort_check);
    let result = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box_800x600(),
        &[],
        &[],
        &control,
    );

    assert!(matches!(result, Err(LayoutError::Aborted)));
    assert_eq!(polls.get(), 4, "pagination stops at the cancellation point");
}

#[test]
fn page_layout_control_allows_an_explicit_unbounded_page_count() {
    let (mut document, cascade) = hello_world_doc();
    let control = PageLayoutControl::new(None);
    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box_800x600(),
        &[],
        &[],
        &control,
    )
    .expect("unbounded pagination");

    assert_eq!(pages.len(), 1);
}

#[test]
fn page_origins_cover_fixed_and_scheduled_steps_without_a_page_ceiling() {
    let fixed = PageOrigins::new(&[], 100.0);
    assert_eq!(fixed.origin(5_000), 500_000.0);
    assert_eq!(fixed.page_index_for_y(500_000.0), 5_000);
    assert_eq!(fixed.page_index_for_end(500_000.0), 4_999);
    assert_eq!(fixed.correct_page_index_for_y(5_001, 500_000.0), 5_000);

    let scheduled = PageOrigins::new(&[100.0, 200.0], 50.0);
    assert_eq!(scheduled.origin(4), 400.0);
    assert_eq!(scheduled.page_index_for_y(100.0), 1);
    assert_eq!(scheduled.page_index_for_y(300.0), 2);
    assert_eq!(scheduled.page_index_for_y(350.0), 3);
    assert_eq!(scheduled.page_index_for_end(100.0), 0);
    assert_eq!(scheduled.page_index_for_end(300.0), 1);
    assert_eq!(scheduled.page_index_for_end(350.0), 2);
}

#[test]
fn page_origins_keep_fractional_end_boundaries_on_the_preceding_page() {
    let fixed = PageOrigins::new(&[], 0.1);
    assert_eq!(fixed.page_index_for_y(0.3), 3);
    assert_eq!(fixed.page_index_for_end(0.3), 2);

    for page_index in [7, 21] {
        let boundary = fixed.origin(page_index);
        assert_eq!(fixed.page_end(page_index - 1), boundary);
        assert_eq!(fixed.page_index_for_y(boundary), page_index);
        assert_eq!(fixed.page_index_for_end(boundary), page_index - 1);
    }

    let scheduled = PageOrigins::new(&[0.1], 0.1);
    assert_eq!(scheduled.page_index_for_y(0.3), 3);
    assert_eq!(scheduled.page_index_for_end(0.3), 2);
}

#[test]
fn page_origins_resolve_repeated_large_fixed_boundaries() {
    let fixed = PageOrigins::new(&[], 0.1);
    let coordinate = 1_677_722.0;
    let candidates = 16_777_200..16_777_240;
    let expected_y = candidates
        .clone()
        .find(|page_index| coordinate < fixed.page_end(*page_index))
        .expect("coordinate falls within the candidate page range");
    let expected_end = candidates
        .clone()
        .find(|page_index| coordinate <= fixed.page_end(*page_index))
        .expect("coordinate ends within the candidate page range");

    assert_eq!(fixed.page_index_for_y(coordinate), expected_y);
    assert_eq!(fixed.page_index_for_end(coordinate), expected_end);
}

#[test]
fn page_layout_control_accepts_a_box_ending_on_a_fractional_page_boundary() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:0.3px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let mut page_box = page_box_800x600();
    page_box.height = 0.1;
    let control = PageLayoutControl::new(Some(3));

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box,
        &[],
        &[],
        &control,
    )
    .expect("three-page boundary layout");

    assert_eq!(pages.len(), 3);
}

#[test]
fn page_layout_control_accepts_a_box_ending_on_a_repeated_fractional_boundary() {
    use raikiri_style::{build_rule_tree, cascade};

    let page_origins = PageOrigins::new(&[], 0.1);
    let boundary = page_origins.origin(21);
    let style = format!("display:block;height:{boundary}px");
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    document.append_element(Some(body), "div", Style::default(), Some(&style));
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let mut page_box = page_box_800x600();
    page_box.height = 0.1;
    let control = PageLayoutControl::new(Some(21));

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box,
        &[],
        &[],
        &control,
    )
    .expect("twenty-one-page boundary layout");

    assert_eq!(pages.len(), 21);
}

#[test]
fn page_layout_control_accepts_a_page_height_box_after_fractional_forced_breaks() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    for _ in 0..6 {
        document.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:0;break-after:page"),
        );
    }
    document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:0.1px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let mut page_box = page_box_800x600();
    page_box.height = 0.1;
    let control = PageLayoutControl::new(Some(7));

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box,
        &[],
        &[],
        &control,
    )
    .expect("seven-page fractional forced-break layout");

    assert_eq!(pages.len(), 7);
}

#[test]
fn page_layout_discovery_returns_a_bounded_prefix() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    for _ in 0..5 {
        document.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:10px;break-before:page"),
        );
    }
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let control = PageLayoutControl::for_geometry_discovery(Some(2));

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box_800x600(),
        &[],
        &[],
        &control,
    )
    .expect("bounded page discovery");

    assert_eq!(pages.len(), 2);
    assert!(control.page_limit_reached());
}

#[test]
fn page_layout_discovery_control_records_a_truncated_limit_check() {
    let control = PageLayoutControl::for_geometry_discovery(Some(1));

    assert!(control.check_discovery_page_index(1).is_ok());
    assert!(control.page_limit_reached());
}

#[test]
fn page_origins_saturate_indexes_that_exceed_u32() {
    let tiny = PageOrigins::new(&[], f32::MIN_POSITIVE);

    assert_eq!(tiny.page_index_for_y(f32::MAX), u32::MAX);
    assert_eq!(tiny.page_index_after_prefix(f64::INFINITY), u32::MAX);
}

#[test]
fn page_layout_control_rejects_natural_overflow_beyond_4096_pages() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:3000000px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let control = PageLayoutControl::new(Some(4_500));

    let result = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box_800x600(),
        &[],
        &[],
        &control,
    );

    assert!(matches!(
        result,
        Err(LayoutError::PageLimitExceeded {
            limit: 4_500,
            actual: 5_000,
        })
    ));
}

#[test]
fn page_layout_control_checks_percent_width_nodes() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let block = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;width:50%;height:1200px"),
    );
    document.append_text(block, "percentage width");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");
    let control = PageLayoutControl::new(None);

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box_800x600(),
        &[600.0, 300.0],
        &[800.0, 400.0],
        &control,
    )
    .expect("percentage-width pagination");

    assert!(pages.len() >= 2);
}

#[test]
fn candidate_collection_recurses_through_inline_subtrees() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let paragraph =
        document.append_element(Some(body), "p", Style::default(), Some("display:block"));
    let span = document.append_element(
        Some(paragraph),
        "span",
        Style::default(),
        Some("display:inline"),
    );
    document.append_text(span, "inline text");
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade");

    let pages = layout_pages_with_page_geometry_and_control(
        &mut document,
        &cascade,
        page_box_800x600(),
        &[],
        &[],
        &PageLayoutControl::new(None),
    )
    .expect("inline-subtree pagination");

    assert_eq!(pages.len(), 1);
    assert!(
        document.nodes[span]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
    );
}

#[test]
fn page_origins_use_direct_fixed_and_cumulative_scheduled_offsets() {
    let fixed = PageOrigins::new(&[], 100.0);
    assert_eq!(fixed.origin(10_000), 1_000_000.0);
    assert_eq!(fixed.page_index_for_y(300.0), 3);
    assert_eq!(fixed.page_index_for_end(300.0), 2);

    let scheduled = PageOrigins::new(&[100.0, 180.0], 100.0);
    assert_eq!(scheduled.origin(0), 0.0);
    assert_eq!(scheduled.origin(1), 100.0);
    assert_eq!(scheduled.origin(2), 280.0);
    assert_eq!(scheduled.origin(3), 380.0);
    assert_eq!(scheduled.page_index_for_y(150.0), 1);
    assert_eq!(scheduled.page_index_for_end(150.0), 1);
    assert_eq!(scheduled.page_index_for_y(280.0), 2);
    assert_eq!(scheduled.page_index_for_end(280.0), 1);

    let invalid_schedule = PageOrigins::new(&[0.0, -10.0, f32::NAN], 100.0);
    assert_eq!(invalid_schedule.origin(3), 300.0);
}

// ── layout_single_page driver (Task 7) ──────────────────────

fn hello_world_doc() -> (Document, raikiri_style::CascadeResult) {
    // <html><head></head><body><p style="color:red">Hi</p></body></html>
    // Equivalent to parsing (built manually instead; future umbrella integration handles raikiri-html).
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let p = doc.append_element(
        Some(body),
        "p",
        Style::default(),
        Some("display:block;color:red"),
    );
    let _text = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");
    (doc, cr)
}

#[test]
fn balanced_multicol_text_that_fits_uses_one_page() {
    use shodo::limits::Limits;

    let text = std::iter::repeat_n("x xx xxx xxxx xxxxx", 11)
        .collect::<Vec<_>>()
        .join(" ");
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:600px;column-count:6;column-gap:0;font:20px/1 Ahem",
        |doc, root| {
            doc.append_text(root, text);
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());

    let pages = layout_pages(&mut doc, &cascade, page_box_800x600()).expect("pagination");

    assert_eq!(
        pages.len(),
        1,
        "a balanced six-column paragraph fits one page"
    );
    assert_eq!(
        doc.nodes[root]
            .ifc_multicol_fragments()
            .expect("column fragments")
            .len(),
        6
    );

    let text_node = doc.nodes[root].children[0];
    let owned = doc.ifc_text_lines(text_node).expect("owned lines");
    let positioned = positioned_text_line_bounds(&doc, text_node).expect("positioned lines");
    let fragments = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|ifc| ifc.multicol_fragments.as_ref())
        .expect("column fragments");
    for fragment in fragments {
        let owned_index = owned
            .lines
            .iter()
            .position(|line| line.line == fragment.line_start)
            .expect("the text node owns each fragment's first line");
        assert!(
            (positioned[owned_index].0 - fragment.y).abs() < 0.001,
            "line {} starts at {:?}, fragment starts at {}",
            fragment.line_start,
            positioned[owned_index],
            fragment.y
        );
    }
}

#[test]
fn pagination_keeps_multicol_inline_text_in_its_column() {
    let text = "XXXX XXXX XXXX XXXX XXXX XXXX XXXX";
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:360px;column-count:3;column-gap:0;font:20px/1 Ahem",
        |doc, root| {
            for color in ["purple", "orange", "blue"] {
                let span = doc.append_element(
                    Some(root),
                    "span",
                    Style::default(),
                    Some(&format!("display:inline;color:{color}")),
                );
                doc.append_text(span, text);
            }
        },
    );
    doc.set_font_collection(ifc_ahem_fonts());

    let pages = layout_pages(&mut doc, &cascade, page_box_800x600()).expect("pagination");

    assert_eq!(pages.len(), 1);
    let lines = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|ifc| ifc.lines.as_ref())
        .expect("paragraph lines");
    assert!(
        lines.shifts.iter().all(|(_, delta)| *delta >= 0.0),
        "pagination must not move later column text above its source position: {:?}",
        lines.shifts
    );
}

#[test]
fn layout_pages_exercises_used_margin_fallbacks_and_page_insets() {
    use raikiri_style::{Origin, build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut direct_doc = Document::new();
    let html = direct_doc.append_element(
        Some(0),
        "html",
        Style::default(),
        Some("display:block;margin-top:auto"),
    );
    let body = direct_doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;background-color:red"),
    );
    direct_doc.append_text(body, "direct body text");
    let direct_rules = build_rule_tree(&direct_doc);
    let direct_cascade = cascade(&direct_doc, &direct_rules).expect("cascade Ok");
    let mut direct_page = PageBox::new();
    direct_page.width = 100.0;
    direct_page.height = 100.0;
    assert!(
        !layout_pages(&mut direct_doc, &direct_cascade, direct_page,)
            .expect("direct pagination Ok")
            .is_empty()
    );

    let mut block_doc = Document::new();
    let html = block_doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body =
        block_doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    block_doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:20px;margin-bottom:auto"),
    );
    let mut block_rules = build_rule_tree(&block_doc);
    block_rules.add_stylesheet("@page { padding:10px; }", Origin::Author);
    let block_cascade = cascade(&block_doc, &block_rules).expect("cascade Ok");
    let mut block_page = PageBox::new();
    block_page.width = 100.0;
    block_page.height = 100.0;
    assert!(
        !layout_pages(&mut block_doc, &block_cascade, block_page,)
            .expect("block pagination Ok")
            .is_empty()
    );
}

#[test]
fn relayout_nested_multicol_children_packs_block_children_into_columns() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let c = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let d = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // Each column balances to a 60px target (4 * 30px total / 2
    // columns); the third child overflows the first column's budget and
    // starts the second column.
    assert!(
        doc.fragment_tree
            .break_tokens
            .iter()
            .any(|token| token.node_id == container && token.child_index == 2),
        "a column break must be recorded before the third child (index 2)"
    );
    let fragmentainer_of = |node: usize| {
        doc.fragment_tree
            .fragments
            .iter()
            .rev()
            .find(|fragment| fragment.node_id == node && fragment.line_start.is_none())
            .map(|fragment| fragment.fragmentainer)
    };
    assert_eq!(fragmentainer_of(a), Some(0));
    assert_eq!(fragmentainer_of(b), Some(0));
    assert_eq!(fragmentainer_of(c), Some(1));
    assert_eq!(fragmentainer_of(d), Some(1));

    // width 200 / 2 columns with a 20px gap: (200 - 20) / 2 = 90; column
    // 1 starts at 90 + 20 = 110.
    assert!((doc.nodes[a].unrounded_layout.location.x - 0.0).abs() < 0.01);
    assert!((doc.nodes[c].unrounded_layout.location.x - 110.0).abs() < 0.01);
    assert!((doc.nodes[a].unrounded_layout.location.y - 0.0).abs() < 0.01);
    assert!((doc.nodes[b].unrounded_layout.location.y - 30.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 60.0).abs() < 0.01);
}

#[test]
fn auto_fill_keeps_auto_height_block_children_in_source_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px;column-fill:auto"),
    );
    let children: Vec<usize> = (0..4)
        .map(|_| {
            doc.append_element(
                Some(container),
                "div",
                Style::default(),
                Some("display:block;height:30px"),
            )
        })
        .collect();

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    let fragmentainer_of = |node: usize| {
        doc.fragment_tree
            .fragments
            .iter()
            .rev()
            .find(|fragment| fragment.node_id == node && fragment.line_start.is_none())
            .map(|fragment| fragment.fragmentainer)
    };
    assert_eq!(
        children
            .iter()
            .map(|&child| fragmentainer_of(child))
            .collect::<Vec<_>>(),
        [Some(0), Some(0), Some(0), Some(0)]
    );
    assert_eq!(
        children
            .iter()
            .map(|&child| doc.nodes[child].unrounded_layout.location.y)
            .collect::<Vec<_>>(),
        [0.0, 30.0, 60.0, 90.0]
    );
    assert!((doc.nodes[container].unrounded_layout.size.height - 120.0).abs() < 0.01);
}

#[test]
fn relayout_nested_multicol_children_honors_break_before_avoid() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;column-count:2;column-gap:20px;width:200px"),
    );
    let a = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let b = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );
    let c = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px;break-before:avoid"),
    );
    let d = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("display:block;height:30px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 300.0;
    page.height = 200.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // `break-before:avoid` on the third child forbids the break that
    // would otherwise land there, so it stays in column 0 and the
    // fourth child is pushed into column 1 instead.
    assert!(
        doc.fragment_tree
            .break_tokens
            .iter()
            .any(|token| token.node_id == container && token.child_index == 3),
        "the break must move to the fourth child (index 3)"
    );
    let fragmentainer_of = |node: usize| {
        doc.fragment_tree
            .fragments
            .iter()
            .rev()
            .find(|fragment| fragment.node_id == node && fragment.line_start.is_none())
            .map(|fragment| fragment.fragmentainer)
    };
    assert_eq!(fragmentainer_of(a), Some(0));
    assert_eq!(fragmentainer_of(b), Some(0));
    assert_eq!(fragmentainer_of(c), Some(0));
    assert_eq!(fragmentainer_of(d), Some(1));
    assert!((doc.nodes[c].unrounded_layout.location.y - 60.0).abs() < 0.01);
    assert!((doc.nodes[container].unrounded_layout.size.height - 90.0).abs() < 0.01);
}

#[test]
fn text_indent_offsets_empty_inline_block_by_content_box_percentage() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;box-sizing:border-box;width:120px;padding-right:10px;text-indent:50%"),
    );
    let inline_block = doc.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4).expect("layout Ok");

    assert!((doc.nodes[block].unrounded_layout.content_box_width() - 110.0).abs() < 0.01);
    assert!((doc.nodes[inline_block].unrounded_layout.location.x - 55.0).abs() < 0.01);
}

#[test]
fn ch_box_values_reach_taffy_before_percentage_children() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;
    use std::path::PathBuf;

    let fonts_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("wpt")
        .join("fonts");
    // cov:ignore: this integration fixture is intentionally skippable when shared WPT assets are absent.
    if !fonts_dir.join("Ahem.ttf").exists() {
        eprintln!(
            "skipping ch box test: Ahem.ttf is required under {}",
            fonts_dir.display()
        );
        return;
    }

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let parent = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;font-family:Ahem;font-size:16px;width:1ch;height:2ch;padding-top:1ch;padding-right:2ch;padding-bottom:1ch;padding-left:1ch;margin-top:1ch;margin-right:3ch;margin-bottom:1ch;margin-left:1ch"),
        );
    let child = doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("display:block;width:100%;height:1px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    doc.set_font_collection(
        crate::fonts::build_wpt_font_collection(&fonts_dir)
            .expect("bundled WPT fonts should register"),
    );
    layout_single_page(&mut doc, &cascade, PageBox::A4).expect("layout Ok");

    let parent_layout = doc.nodes[parent].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    assert!((parent_layout.size.width - 64.0).abs() < 0.01);
    assert!((parent_layout.size.height - 64.0).abs() < 0.01);
    let used_style = &doc.nodes[parent].style;
    assert_eq!(used_style.padding.top.into_raw().value(), 16.0);
    assert_eq!(used_style.padding.right.into_raw().value(), 32.0);
    assert_eq!(used_style.padding.bottom.into_raw().value(), 16.0);
    assert_eq!(used_style.padding.left.into_raw().value(), 16.0);
    assert_eq!(used_style.margin.top.into_raw().value(), 16.0);
    assert_eq!(used_style.margin.right.into_raw().value(), 48.0);
    assert_eq!(used_style.margin.bottom.into_raw().value(), 16.0);
    assert_eq!(used_style.margin.left.into_raw().value(), 16.0);
    assert!((parent_layout.location.x - 16.0).abs() < 0.01);
    assert!((child_layout.size.width - 16.0).abs() < 0.01);
}

#[test]
fn layout_single_page_without_body_returns_error() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::{LayoutError, PageBox};

    // Attach <p> directly (fragment equivalent).
    let mut doc = Document::new();
    let _p = doc.append_element(Some(0), "p", Style::default(), None::<&str>);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).unwrap();

    match layout_single_page(&mut doc, &cr, PageBox::A4) {
        Err(LayoutError::Internal { message }) => {
            assert!(
                message.contains("body"),
                "error message should mention <body>, got '{}'",
                message
            );
        }
        other => panic!("expected LayoutError::Internal, got {:?}", other),
    }
}

#[test]
fn layout_single_page_bridges_display_none() {
    // Check that the display bridge is active via layout_single_page:
    // assigning display:none to body must set taffy::Style.display
    // to Display::None.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:none"));
    let p = doc.append_element(Some(body), "p", Style::default(), Some("color:red"));
    let _text = doc.append_text(p, "Hi");
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");
    assert_eq!(doc.nodes[body].style.display, Display::None);
}

#[test]
fn inline_block_width_auto_shrink_wraps_to_content() {
    // An inline-block with width:auto shrinks to its content rather than
    // filling the containing block (shrink-to-fit). Check its geometry against
    // a plain block child that fills the available width:
    // an inline-block containing a 100px child is 100px wide, while an
    // inline-block with explicit width:300px stays 300px wide (neither fills the body).
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    // Block sibling first so body keeps mixed children (no minimal
    // line-box rerouting) and lays the inline-blocks out as block
    // children with a definite available width.
    let _p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let ib = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:inline-block"),
    );
    let _child = doc.append_element(
        Some(ib),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let ib_fixed = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:inline-block;width:300px"),
    );
    let _fixed_child = doc.append_element(
        Some(ib_fixed),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let auto_size = doc.nodes[ib].unrounded_layout.size;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (auto_size.width - 100.0).abs() < 0.5,
        "width:auto inline-block must shrink-wrap its 100px child, got width={}",
        auto_size.width
    );
    let fixed_size = doc.nodes[ib_fixed].unrounded_layout.size;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (fixed_size.width - 300.0).abs() < 0.5,
        "explicit-width inline-block must keep its specified width, got width={}",
        fixed_size.width
    );
}

#[test]
fn flex_items_follow_order_modified_source_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;width:160px;height:20px"),
    );
    let a = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:2;width:40px;height:20px"),
    );
    let absolute = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("position:absolute;order:-100;width:10px;height:10px"),
    );
    let b = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:-1;width:40px;height:20px"),
    );
    let c = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:2;width:40px;height:20px"),
    );
    let d = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("width:40px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    assert_eq!(doc.nodes[flex].children, vec![a, absolute, b, c, d]);
    assert_eq!(
        doc.nodes[flex].order_modified_children.as_ref(),
        &[b, absolute, d, a, c]
    );
    let x = |id: usize| doc.nodes[id].unrounded_layout.location.x;
    assert!((x(d) - x(b) - 40.0).abs() < 0.5);
    assert!((x(a) - x(d) - 40.0).abs() < 0.5);
    assert!((x(c) - x(a) - 40.0).abs() < 0.5);
}

#[test]
fn relayout_invalidates_cache_when_order_modified_view_returns_to_dom_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;width:60px;height:20px"),
    );
    let first = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;width:30px;height:20px"),
    );
    let second = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;width:30px;height:20px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade_result = cascade(&doc, &rules).expect("initial cascade Ok");
    layout_single_page(&mut doc, &cascade_result, PageBox::A4).expect("initial layout Ok");
    assert_eq!(
        doc.nodes[flex].order_modified_children.as_ref(),
        &[second, first]
    );
    assert!(
        doc.nodes[second].unrounded_layout.location.x
            < doc.nodes[first].unrounded_layout.location.x
    );

    // Keep the same Document and page size, but remove the nonzero order.
    // The derived child view becomes empty, which must invalidate cached
    // flex positions rather than reusing the prior reversed placement.
    doc.set_element_inline_style(first, Some("order:0;width:30px;height:20px".into()));
    doc.set_element_inline_style(second, Some("order:0;width:30px;height:20px".into()));
    let rules = build_rule_tree(&doc);
    let updated_cascade = cascade(&doc, &rules).expect("updated cascade Ok");
    layout_single_page(&mut doc, &updated_cascade, PageBox::A4).expect("updated layout Ok");

    assert!(doc.nodes[flex].order_modified_children.is_empty());
    assert!(
        doc.nodes[first].unrounded_layout.location.x
            < doc.nodes[second].unrounded_layout.location.x
    );
}

#[test]
fn order_is_ignored_for_non_flex_grid_children() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let a = doc.append_element(
        Some(block),
        "div",
        Style::default(),
        Some("order:2;height:20px"),
    );
    let b = doc.append_element(
        Some(block),
        "div",
        Style::default(),
        Some("order:-1;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    assert!(doc.nodes[block].order_modified_children.is_empty());
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        doc.nodes[b].unrounded_layout.location.y > doc.nodes[a].unrounded_layout.location.y,
        "ordinary block children must retain source order despite CSS order"
    );
}

#[test]
fn nested_normal_flow_descendant_clears_ancestor_level_float() {
    // `LayoutBlockContainer::compute_block_child_layout`'s override in
    // `taffy_impl` only matters when a float and content that reacts to
    // it are at *different* nesting depths — direct siblings share one
    // `compute_block_layout` invocation (and hence one taffy
    // `BlockFormattingContext`) regardless of the override. This
    // fixture puts the float and a `clear:left` box two levels apart
    // (`float_sibling` is `container`'s child, `probe` is `container`'s
    // grandchild via the intervening `wrapper`), so the assertion only
    // holds if `wrapper`'s own recursive layout call continues
    // `container`'s `BlockFormattingContext` rather than starting a
    // fresh, float-blind one for `wrapper`'s own children (CSS2 §9.5.2
    // <https://www.w3.org/TR/CSS2/visuren.html#propdef-clear>: "clear"
    // requires the box's top border edge be below any earlier float in
    // the same block formatting context — not just floats that are its
    // own direct siblings).
    //
    // `wrapper` is left with no explicit width so it stretch-fits to
    // `container`'s full inner width, matching the BFC root's width —
    // avoiding a taffy `block_layout` limitation (a same-BFC child
    // narrower than its BFC root can get float insets computed against
    // the root's width instead of its own) that is orthogonal to what
    // this test pins.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let container = doc.append_element(Some(body), "div", Style::default(), Some("width:200px"));
    let float_sibling = doc.append_element(
        Some(container),
        "div",
        Style::default(),
        Some("float:left;width:60px;height:40px"),
    );
    let wrapper = doc.append_element(Some(container), "div", Style::default(), None::<&str>);
    let probe = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("clear:left;width:50px;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let float_loc = doc.nodes[float_sibling].unrounded_layout.location;
    let float_size = doc.nodes[float_sibling].unrounded_layout.size;
    let wrapper_loc = doc.nodes[wrapper].unrounded_layout.location;
    let probe_loc = doc.nodes[probe].unrounded_layout.location;

    // `location` is parent-relative in taffy, so translate `probe`'s y
    // into `container`'s coordinate space by walking up one level.
    let probe_y_in_container = wrapper_loc.y + probe_loc.y;
    let float_bottom = float_loc.y + float_size.height;

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        probe_y_in_container + 0.5 >= float_bottom,
        "clear:left box two BFC levels below the float must sit at or below the float's bottom edge, got float_bottom={float_bottom}, probe_y_in_container={probe_y_in_container}"
    );
}

#[test]
fn layout_single_page_deterministic_across_10_runs() {
    // Require byte-identical results across ten consecutive runs.
    // Check determinism on the same machine (cross-machine consistency needs
    // future font pinning).
    use raikiri_traits::PageBox;

    fn one_run() -> Vec<taffy::Layout> {
        let (mut doc, cr) = hello_world_doc();
        layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");
        doc.nodes.iter().map(|n| n.unrounded_layout).collect()
    }

    let baseline = one_run();
    for i in 1..10 {
        let run = one_run();
        assert_eq!(
            baseline.len(),
            run.len(),
            "run {i}: layout node count changed"
        );
        for (j, (b, r)) in baseline.iter().zip(run.iter()).enumerate() {
            // Compare every taffy::Layout field byte for byte.
            // This catches floating-point subnormal / NaN drift first
            // (equivalent to the design doc §12.8 NonFiniteFloat check).
            assert_eq!(
                b.size.width, r.size.width,
                "run {i} node {j}: size.width differs (baseline={} run={})",
                b.size.width, r.size.width
            );
            assert_eq!(b.size.height, r.size.height);
            assert_eq!(b.location.x, r.location.x);
            assert_eq!(b.location.y, r.location.y);
        }
    }
}

#[test]
fn grid_auto_placement_uses_order_modified_source_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;width:200px;grid-template-columns:100px 100px;grid-auto-rows:20px"),
    );
    let a = doc.append_element(Some(grid), "div", Style::default(), Some("order:2"));
    let b = doc.append_element(Some(grid), "div", Style::default(), Some("order:-1"));
    let c = doc.append_element(Some(grid), "div", Style::default(), Some("order:2"));
    let d = doc.append_element(Some(grid), "div", Style::default(), None::<&str>);
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    assert_eq!(doc.nodes[grid].children, vec![a, b, c, d]);
    assert_eq!(
        doc.nodes[grid].order_modified_children.as_ref(),
        &[b, d, a, c]
    );
    let loc = |id: usize| doc.nodes[id].unrounded_layout.location;
    assert!((loc(d).x - loc(b).x - 100.0).abs() < 0.5);
    assert!((loc(a).y - loc(b).y - 20.0).abs() < 0.5);
    assert!((loc(c).x - loc(a).x - 100.0).abs() < 0.5);
    assert!((loc(c).y - loc(d).y - 20.0).abs() < 0.5);
}

#[test]
fn grid_template_columns_named_line_after_repeat_resolves_to_the_correct_taffy_grid_line() {
    // Regression test for the interleaving contract between
    // raikiri-style's `GridTrackList::line_names` (one entry per
    // `<line-names>?` production, i.e. `components.len() + 1` entries —
    // CSS Grid 1 §7.2.1 `<track-list>` grammar) and taffy's
    // `NamedLineResolver`, which advances an internal line counter once
    // per name-list entry and additionally steps across an unrolled
    // `repeat()`'s own tracks when it consumes one. A child placed with
    // `grid-column-start: z`, where `z` is named right after a
    // `repeat(2, [b] 50px)` block, must resolve to the grid line
    // following the 100px + 50px + 50px tracks that precede it — this
    // can only be told apart from an off-by-one in that bookkeeping by
    // checking where taffy's own grid algorithm actually places the
    // child, not by asserting on the bridged `taffy::Style` value.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let _head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid_container = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:[a] 100px repeat(2, [b] 50px) [z] 100px"),
    );
    let cell_first = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;height:20px"),
    );
    let cell_z = doc.append_element(
        Some(grid_container),
        "div",
        Style::default(),
        Some("grid-column:z;grid-row:1;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cr = cascade(&doc, &rules).expect("cascade Ok");

    layout_single_page(&mut doc, &cr, PageBox::A4).expect("layout Ok");

    let first_loc = doc.nodes[cell_first].unrounded_layout.location;
    let z_loc = doc.nodes[cell_z].unrounded_layout.location;
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert!(
        (z_loc.x - first_loc.x - 200.0).abs() < 0.5,
        "line `z` follows a 100px track and a repeat(2, 50px) block \
             (100px total), so it should sit 200px to the right of line 1, \
             got first.x={}, z.x={}",
        first_loc.x,
        z_loc.x
    );
}

#[test]
fn grid_auto_abspos_static_position_uses_all_parent_padding_sides() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;width:100px;height:100px;padding:10px"),
    );
    let child = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("position:absolute;width:20px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4).expect("layout Ok");
    let layout = doc.nodes[child].unrounded_layout;
    assert!((layout.location.x - 10.0).abs() < 0.01);
    assert!((layout.location.y - 10.0).abs() < 0.01);
}

#[test]
fn layout_page_fragments_emits_one_page_with_deterministic_items() {
    let (mut doc, cascade) = hello_world_doc();
    let pages = layout_page_fragments(&mut doc, &cascade, PageBox::A4)
        .expect("page fragment layout should succeed");

    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page_index, 0);
    assert_eq!(pages[0].page_box, PageBox::A4);
    assert!(!pages[0].is_empty());
    assert!(
        pages[0]
            .items
            .windows(2)
            .all(|items| items[0].node_id <= items[1].node_id)
    );
    assert!(
        pages[0]
            .items
            .iter()
            .any(|item| item.kind == PageFragmentKind::Text)
    );
}

#[test]
fn layout_page_fragments_preserves_forced_page_break() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let first = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;height:10px"),
    );
    doc.append_text(first, "first");
    let second = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;break-before:page;height:10px"),
    );
    doc.append_text(second, "second");
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 60.0;

    let pages = layout_page_fragments(&mut doc, &cascade, page)
        .expect("page fragment layout should succeed");
    assert!(pages.len() >= 2, "forced page break must create page 1+");
    assert!(
        pages[1]
            .items
            .iter()
            .any(|item| item.node_id.0 == second as u64)
    );
}

#[test]
fn page_fragment_projection_handles_empty_missing_body_and_invalid_slice_inputs() {
    use raikiri_style::{build_rule_tree, cascade};

    let document = Document::new();
    let rules = build_rule_tree(&document);
    let cascade_result = cascade(&document, &rules).expect("cascade Ok");
    assert!(page_fragments_from_slices(&document, &cascade_result, PageBox::A4, &[]).is_empty());

    let slice = PageSlice {
        page_index: 0,
        content_origin_y: 0.0,
        page_name: None,
    };
    let pages = page_fragments_from_slices(
        &document,
        &cascade_result,
        PageBox::A4,
        std::slice::from_ref(&slice),
    );
    assert_eq!(pages.len(), 1);
    assert!(pages[0].is_empty());

    let reversed_slices = [
        PageSlice {
            page_index: 1,
            content_origin_y: 100.0,
            page_name: None,
        },
        slice.clone(),
    ];
    let pages =
        page_fragments_from_slices(&document, &cascade_result, PageBox::A4, &reversed_slices);
    assert_eq!(
        pages.iter().map(|page| page.page_index).collect::<Vec<_>>(),
        vec![0, 1]
    );

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let comment = document.append_comment(Some(body), "comment");
    document.nodes[comment].set_in_document(true);
    document.nodes[body].children.push(usize::MAX);
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let pages = page_fragments_from_slices(
        &document,
        &cascade,
        PageBox::A4,
        std::slice::from_ref(&slice),
    );
    assert_eq!(pages.len(), 1);
    assert!(
        pages[0]
            .items
            .iter()
            .all(|item| item.node_id.0 != comment as u64)
    );

    let invalid_slice = PageSlice {
        page_index: 0,
        content_origin_y: f32::NAN,
        page_name: None,
    };
    let pages = page_fragments_from_slices(
        &document,
        &cascade,
        PageBox::A4,
        std::slice::from_ref(&invalid_slice),
    );
    assert_eq!(pages.len(), 1);
    assert!(pages[0].is_empty());

    document.nodes[body].unrounded_layout.location.x = f32::NAN;
    document.nodes[body].unrounded_layout.size.width = f32::NAN;
    document.nodes[body].unrounded_layout.size.height = f32::NAN;
    let pages = page_fragments_from_slices(
        &document,
        &cascade,
        PageBox::A4,
        std::slice::from_ref(&slice),
    );
    assert!(pages[0].is_empty());
}

#[test]
fn layout_page_fragments_classifies_replaced_and_skips_non_rendered_nodes() {
    use raikiri_style::{build_rule_tree, cascade};

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let template = document.append_element(Some(body), "template", Style::default(), None::<&str>);
    document.append_text(template, "not rendered");
    document.append_comment(Some(body), "comment");
    document.append_element(Some(body), "div", Style::default(), None::<&str>);
    let image = document.append_element(
        Some(body),
        "img",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");

    let pages = layout_page_fragments(&mut document, &cascade, PageBox::A4)
        .expect("page fragment layout should succeed");
    assert!(
        pages[0]
            .items
            .iter()
            .any(|item| item.node_id.0 == image as u64 && item.kind == PageFragmentKind::Replaced)
    );
    assert!(
        !pages[0]
            .items
            .iter()
            .any(|item| item.node_id.0 == template as u64)
    );
}

#[test]
fn page_fragment_geometry_table_groups_fragments_by_node_id() {
    let mut first_page = PageFragment {
        page_index: 0,
        ..Default::default()
    };
    first_page.items.push(PageFragmentItem::new(
        NodeId::new(9),
        PageFragmentRect::new(0.0, 0.0, 10.0, 4.0),
        PageFragmentKind::Box,
        0,
        2,
        false,
    ));
    let mut second_page = PageFragment {
        page_index: 1,
        ..Default::default()
    };
    second_page.items.push(PageFragmentItem::new(
        NodeId::new(9),
        PageFragmentRect::new(0.0, 0.0, 10.0, 4.0),
        PageFragmentKind::Box,
        1,
        2,
        false,
    ));
    let pages = vec![second_page, first_page];
    let table = page_fragment_geometry_table(&pages);
    let geometry = table.get(&NodeId::new(9)).expect("node geometry");
    assert_eq!(geometry.node_id, NodeId::new(9));
    assert!(geometry.is_split());
    assert_eq!(geometry.fragments.len(), 2);
    assert_eq!(geometry.fragments[0].page_index, 0);
    assert_eq!(geometry.fragments[1].page_index, 1);
    assert_eq!(geometry.fragments[1].fragment_index, 1);
}

#[test]
fn propagated_start_page_name_uses_resolved_grid_child_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:grid;grid-template-columns:100px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;order:1;page:a;height:10px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;order:0;page:b;height:10px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    layout_single_page(&mut doc, &cascade, PageBox::A4).expect("layout Ok");
    assert_eq!(
        propagated_start_page_name(&doc, &cascade, body, None),
        (true, Some("b".to_owned()))
    );
}

#[test]
fn layout_pages_uses_order_modified_flex_sequence_for_forced_breaks() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column;width:40px;height:20px"),
    );
    let break_later_in_visual_order = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;break-before:page;height:10px"),
    );
    let first_in_visual_order = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    let first_y = doc.nodes[first_in_visual_order].unrounded_layout.location.y;
    let later_y = doc.nodes[break_later_in_visual_order]
        .unrounded_layout
        .location
        .y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "the visual first item stays on page 0: y={first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        later_y > first_y + 50.0,
        "the forced-break item moves later: y={later_y}"
    );
    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[first_in_visual_order, break_later_in_visual_order]
    );
}

#[test]
fn layout_pages_uses_order_modified_flex_last_item_for_overflow() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column;width:40px"),
    );
    let visual_last = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;height:70px"),
    );
    let visual_first = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;height:40px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let _slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    let first_y = doc.nodes[visual_first].unrounded_layout.location.y;
    let last_y = doc.nodes[visual_last].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "the visual first item stays at y=0: {first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        last_y < 60.0,
        "the visual last item may continue across the page edge: y={last_y}"
    );
    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[visual_first, visual_last]
    );
}

#[test]
fn layout_pages_uses_column_reverse_visual_order_for_forced_breaks() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column-reverse;width:40px;height:20px"),
    );
    let visual_top = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;height:10px"),
    );
    let visual_bottom_forced_break = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;break-before:page;height:10px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[visual_bottom_forced_break, visual_top]
    );
    let top_y = doc.nodes[visual_top].unrounded_layout.location.y;
    let bottom_y = doc.nodes[visual_bottom_forced_break]
        .unrounded_layout
        .location
        .y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        top_y.abs() < 0.01,
        "the visual top stays on page 0: {top_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        bottom_y >= 100.0,
        "the lower item moves to page 1+: {bottom_y}"
    );
}

#[test]
fn layout_pages_keeps_column_reverse_visual_last_item_at_page_edge() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;flex-direction:column-reverse;width:40px"),
    );
    let visual_top = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:1;height:40px"),
    );
    let visual_bottom = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("order:0;height:70px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let _slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert_eq!(
        doc.nodes[flex].layout_children(),
        &[visual_bottom, visual_top]
    );
    let top_y = doc.nodes[visual_top].unrounded_layout.location.y;
    let bottom_y = doc.nodes[visual_bottom].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        top_y.abs() < 0.01,
        "visual first item starts at y=0: {top_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        bottom_y < 60.0 && bottom_y + 70.0 > 100.0,
        "visual last item may continue across the page edge: y={bottom_y}"
    );
}

#[test]
fn layout_pages_orders_grid_rows_by_resolved_placement_before_forced_break() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let row_two = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:0;break-before:page;height:60px"),
    );
    let _comment = doc.append_comment(Some(grid), "not a grid item");
    let row_one = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;order:1;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[row_two, _comment, row_one]
    );
    let row_one_y = doc.nodes[row_one].unrounded_layout.location.y;
    let row_two_y = doc.nodes[row_two].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_one_y.abs() < 0.01,
        "resolved row 1 stays on page 0: {row_one_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_two_y >= 100.0,
        "resolved row 2 moves after the break: {row_two_y}"
    );
}

#[test]
fn layout_pages_processes_single_column_grid_in_placed_row_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let first_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;order:1;height:60px"),
    );
    let second_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:0;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        slices.len() >= 2,
        "the second grid row should continue to page 1+"
    );
    assert_eq!(doc.nodes[grid].layout_children(), &[second_row, first_row]);
    let first_y = doc.nodes[first_row].unrounded_layout.location.y;
    let second_y = doc.nodes[second_row].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "explicit row 1 stays at y=0: {first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        second_y > first_y + 50.0,
        "explicit row 2 moves after row 1: y={second_y}"
    );
}

#[test]
fn layout_pages_coalesces_zero_height_named_boxes_at_same_position() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;page:first;height:0px"),
    );
    doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;page:last;height:0px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let slices = layout_pages(&mut doc, &cascade, PageBox::A4).expect("pagination Ok");

    assert_eq!(slices.len(), 1);
    assert_eq!(slices[0].page_name.as_deref(), Some("last"));
}

#[test]
fn layout_pages_processes_mixed_auto_and_explicit_grid_rows_by_resolved_placement() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let auto_row_one = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("order:1;height:60px"),
    );
    let explicit_row_two = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:0;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        slices.len() >= 2,
        "the second grid row should continue to page 1+"
    );
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[explicit_row_two, auto_row_one]
    );
    let row_one_y = doc.nodes[auto_row_one].unrounded_layout.location.y;
    let row_two_y = doc.nodes[explicit_row_two].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_one_y.abs() < 0.01,
        "auto row 1 stays at y=0: {row_one_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        row_two_y > row_one_y + 50.0,
        "explicit row 2 follows row 1: {row_two_y}"
    );
}

#[test]
fn layout_pages_orders_direct_grid_text_before_later_forced_break() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:100px;grid-template-rows:60px 60px"),
    );
    let first_row_text = doc.append_text(grid, "first row text");
    let later_row_forced_break = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;order:-1;break-before:page;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[later_row_forced_break, first_row_text]
    );
    let text_y = doc.nodes[first_row_text].unrounded_layout.location.y;
    let later_y = doc.nodes[later_row_forced_break]
        .unrounded_layout
        .location
        .y;
    assert!(text_y.abs() < 0.01, "row 1 text stays on page 0: {text_y}");
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        later_y >= 100.0,
        "row 2 moves after the forced break: {later_y}"
    );
}

#[test]
fn layout_pages_orders_negative_explicit_grid_rows_by_placement() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let first_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:-3;order:1;height:60px"),
    );
    let second_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:-2;order:0;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        slices.len() >= 2,
        "the second grid row should continue to page 1+"
    );
    assert_eq!(doc.nodes[grid].layout_children(), &[second_row, first_row]);
    let first_y = doc.nodes[first_row].unrounded_layout.location.y;
    let second_y = doc.nodes[second_row].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_y.abs() < 0.01,
        "negative row line -3 is first: {first_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        second_y > first_y + 50.0,
        "negative row line -2 follows: {second_y}"
    );
}

#[test]
fn layout_pages_uses_resolved_rows_for_reversed_grid_lines_and_order() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:60px 60px"),
    );
    let reversed_row_lines = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2 / 1;height:60px"),
    );
    let later_row_forced_break = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2 / 3;order:-1;break-before:page;height:60px"),
    );

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    assert_eq!(
        doc.nodes[grid].layout_children(),
        &[later_row_forced_break, reversed_row_lines]
    );
    let first_row_y = doc.nodes[reversed_row_lines].unrounded_layout.location.y;
    let second_row_y = doc.nodes[later_row_forced_break]
        .unrounded_layout
        .location
        .y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        first_row_y.abs() < 0.01,
        "reversed lines resolve to row 1 and stay before the forced break: {first_row_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        second_row_y >= 100.0,
        "row 2's break-before moves only that item to page 1+: {second_row_y}"
    );
}

#[test]
fn layout_pages_orders_relative_grid_items_by_resolved_placement() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let grid = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:grid;grid-template-columns:40px;grid-template-rows:20px 20px"),
    );
    let relative_first_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:1;position:relative;top:60px;height:20px"),
    );
    let page_break_after_second_row = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("grid-column:1;grid-row:2;break-after:page;height:20px"),
    );
    let following = doc.append_element(Some(body), "div", Style::default(), Some("height:10px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert!(slices.len() >= 2, "the forced break must create page 1+");
    let first_row_y = doc.nodes[relative_first_row].unrounded_layout.location.y;
    let second_row_y = doc.nodes[page_break_after_second_row]
        .unrounded_layout
        .location
        .y;
    let following_y = doc.nodes[following].unrounded_layout.location.y;
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        (first_row_y - 60.0).abs() < 0.01,
        "row 1 keeps its relative visual offset without being page-shifted: y={first_row_y}"
    );
    // cov:ignore: panic-message text is only executed when the assertion fails.
    assert!(
        (second_row_y - 20.0).abs() < 0.01,
        "row 2 remains in its placed row: y={second_row_y}"
    );
    assert!(following_y > second_row_y + 50.0);
}

#[test]
fn layout_pages_places_footnote_at_page_bottom_without_flow_footprint() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let lead = doc.append_element(Some(body), "div", Style::default(), Some("height:20px"));
    let note = doc.append_element(
        Some(body),
        "aside",
        Style::default(),
        Some("float:footnote;height:10px;width:50px"),
    );
    doc.append_text(note, "note");
    let following = doc.append_element(Some(body), "div", Style::default(), Some("height:10px"));

    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");
    assert!(matches!(cascade.computed[note].float, FloatValue::Footnote));
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 50.0;
    let slices = layout_pages(&mut doc, &cascade, page).expect("pagination Ok");

    assert_eq!(slices.len(), 1);
    let note_y = doc.nodes[note].unrounded_layout.location.y;
    let following_y = doc.nodes[following].unrounded_layout.location.y;
    assert!((note_y - 40.0).abs() < 0.01, "note y={note_y}");
    assert!(
        following_y < note_y,
        "following flow content should not be pushed below the footnote: following y={following_y}, note y={note_y}"
    );
    assert!(doc.nodes[lead].unrounded_layout.size.height > 0.0);
}

#[test]
fn direct_absolute_auto_width_shrink_to_fit_empty_with_margin() {
    // CSS 2.1 §10.3.7: an absolutely positioned box with `width:auto` and
    // `left:auto` / `right:auto` shrink-wraps its content instead of filling
    // the containing block. An empty box with 10px borders on each side has
    // zero content width, so its border box is 20px wide regardless of the
    // 20px right margin or the 100px containing width.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let abs = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute; margin-right:20px; border:10px solid black"),
    );
    let _auto_margin_abs = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute; margin-left:auto; margin-right:auto"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    // Empty content shrink-wraps to zero; the border box holds only borders.
    let layout = doc.nodes[abs].unrounded_layout;
    assert!((layout.size.width - 20.0).abs() < 0.001);
}

#[test]
fn direct_absolute_auto_width_shrink_to_fit_block_child() {
    // CSS 2.1 §10.3.7 shrink-to-fit with a definite-content child: a
    // direct-body absolute box containing a 100px block must be 100px wide
    // in an 800px containing block, not stretched to the viewport width.
    // This is the WPT line-break-anywhere green-square case (70k-pixel
    // mismatch when the old fill override applied).
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let abs_pos = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:absolute; height:100px"),
    );
    let _child = doc.append_element(
        Some(abs_pos),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    let layout = doc.nodes[abs_pos].unrounded_layout;
    assert!(
        (layout.size.width - 100.0).abs() < 0.5,
        "width:auto absolute must shrink-wrap its 100px child, got width={}",
        layout.size.width
    );
}

#[test]
fn nested_absolute_auto_width_matches_direct_body_shrink() {
    // The same 100px-child shrink must hold inside a relative wrapper: the
    // nested path always used taffy directly, so direct-body must match it.
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let outer = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative"),
    );
    let abs_pos = doc.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("position:absolute; height:100px"),
    );
    let _child = doc.append_element(
        Some(abs_pos),
        "div",
        Style::default(),
        Some("width:100px;height:20px"),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade Ok");

    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    layout_single_page(&mut doc, &cascade, page).expect("layout Ok");

    let layout = doc.nodes[abs_pos].unrounded_layout;
    assert!(
        (layout.size.width - 100.0).abs() < 0.5,
        "nested width:auto absolute must shrink-wrap its 100px child, got width={}",
        layout.size.width
    );
}

mod page_fragments_tests;

// ── ifc roots in taffy ───────────────────────────────────────

use crate::layout::test_support::{
    ahem_paragraph, ahem_paragraph_beside_float, ahem_paragraph_beside_floats,
    ahem_paragraph_in_block_wrapper, ahem_paragraph_with_atomic, ahem_paragraph_with_float,
    ifc_ahem_fonts, line_start_x, line_text, page_box_800x600,
};

fn lay_out(doc: &mut Document, cascade: &CascadeResult) {
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(doc), cascade, page_box_800x600()).expect("layout");
}

fn stored_lines(doc: &Document, root: usize) -> &crate::layout::ifc::root::IfcLines {
    doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .expect("the root holds performed lines")
}

#[test]
fn ifc_root_height_is_lines_times_line_height() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[root].unrounded_layout;
    assert_eq!(layout.size.width, 50.0);
    // Three 10px lines.
    assert_eq!(layout.size.height, 30.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
}

#[test]
fn a_definite_physical_height_sets_the_vertical_inline_extent() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, cascade, root) = ahem_paragraph(
            "aaaa bbbb cccc",
            &format!("writing-mode:{mode};width:100px;height:30px"),
        );
        lay_out(&mut doc, &cascade);

        assert_eq!(stored_lines(&doc, root).width, 30.0, "{mode}");
        assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{mode}");
        let layout = doc.nodes[root].unrounded_layout;
        assert_eq!(
            (layout.size.width, layout.size.height),
            (100.0, 30.0),
            "{mode}"
        );
    }
}

#[test]
fn padding_and_border_shrink_the_line_width() {
    let (mut doc, cascade, root) = ahem_paragraph(
        "aaaa bbbb",
        "box-sizing:border-box;width:70px;padding:0 10px;border:0 solid red;border-width:0 5px",
    );
    lay_out(&mut doc, &cascade);
    // Content box: 70 - 2*10 - 2*5 = 40px, so "aaaa" and "bbbb" stack.
    let lines = stored_lines(&doc, root);
    assert_eq!(lines.width, 40.0);
    assert_eq!(lines.lines.len(), 2);
}

#[test]
fn an_explicit_width_root_breaks_at_the_box_width() {
    // The width is stretched or explicit, so it must not be clamped to the
    // max-content width of the text (90px).
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "width:200px");
    lay_out(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, root).width, 200.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 200.0);
}

#[test]
fn a_floated_root_shrinks_to_its_max_content() {
    // A `width:auto` float is laid out with no known width and a definite
    // available width: the shrink-to-fit branch.
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "float:left");
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 90.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
}

#[test]
fn an_inline_block_shrinks_to_the_max_content_of_its_ifc_root() {
    // inline-block -> block wrapper -> root: the inline-block is measured at
    // max-content, which reaches the root as `AvailableSpace::MaxContent`.
    let (mut doc, cascade, wrapper, root) =
        ahem_paragraph_in_block_wrapper("aaaa bbbb", "display:inline-block");
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[wrapper].unrounded_layout.size.width, 90.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 90.0);
}

#[test]
fn the_last_performed_layout_decides_the_stored_lines() {
    // The inline-block probes its content at several widths before the final
    // pass; whichever pass ran last must be the one the root keeps.
    let (mut doc, cascade, _wrapper, root) =
        ahem_paragraph_in_block_wrapper("aaaa bbbb cccc", "display:inline-block;max-width:50px");
    lay_out(&mut doc, &cascade);
    assert_eq!(
        stored_lines(&doc, root).width,
        doc.nodes[root].unrounded_layout.size.width
    );
}

#[test]
fn lines_survive_a_second_layout_pass() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:50px");
    lay_out(&mut doc, &cascade);
    // Same cascade: no generation change, so only the engine's own cache drop
    // keeps the measure callback running.
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("second layout");
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
}

#[test]
fn first_baseline_includes_the_top_padding_and_border() {
    use taffy::{
        AvailableSpace, LayoutInput, LayoutPartialTree, Line, NodeId, RequestedAxis, RunMode, Size,
        SizingMode,
    };
    let (mut doc, cascade, root) = ahem_paragraph(
        "aa",
        "width:100px;padding-top:6px;border-top-width:3px;border-top-style:solid",
    );
    lay_out(&mut doc, &cascade);
    // Ask the root for its layout output again: the baselines are part of it.
    let output = doc.compute_child_layout(
        NodeId::from(root),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size {
                width: None,
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    // Ahem ascent 8px + 6px padding + 3px border, from the border-box top.
    assert_eq!(output.baselines.first, Some(17.0));
}

#[test]
fn calc_min_width_is_resolved_on_an_ifc_root() {
    // The bridge passes `calc()` through for sizes and min/max sizes, not for
    // padding. A floated root shrinks to 90px, so the calc minimum decides.
    let (mut doc, cascade, root) =
        ahem_paragraph("aaaa bbbb", "float:left;min-width:calc(20% + 10px)");
    lay_out(&mut doc, &cascade);
    // 20% of the 800px page plus 10px.
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 170.0);
    // The stored lines must be broken at the final content width too.
    assert_eq!(stored_lines(&doc, root).width, 170.0);
}

#[test]
fn a_floated_root_with_a_narrow_max_width_breaks_at_that_width() {
    // max-width is below the longest word, so the box shrinks to 40px and the
    // lines must be broken there: three lines, not the two that min-content
    // (60px) would give.
    let (mut doc, cascade, root) = ahem_paragraph("aaaaaa bb cc", "float:left;max-width:40px");
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 40.0);
    assert_eq!(stored_lines(&doc, root).width, 40.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
}

#[test]
fn an_inline_block_baseline_follows_the_last_ifc_line() {
    use taffy::{
        AvailableSpace, LayoutInput, LayoutPartialTree, Line, NodeId, RequestedAxis, RunMode, Size,
        SizingMode,
    };
    let (mut doc, cascade, wrapper, _root) =
        ahem_paragraph_in_block_wrapper("aaaa bbbb cccc", "display:inline-block;width:50px");
    lay_out(&mut doc, &cascade);
    let output = doc.compute_child_layout(
        NodeId::from(wrapper),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(800.0),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    // Three 10px lines: the last baseline is 20px + the 8px Ahem ascent.
    assert_eq!(output.baselines.first, Some(28.0));
}

// ── relayout for a page-specific width ───────────────────────

#[test]
fn relayout_keeps_an_authored_width_root_at_its_width() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:200px");
    lay_out(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    let height_before = doc.nodes[root].unrounded_layout.size.height;
    // The authored 200px is kept whatever the page width is.
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, height_before);
}

#[test]
fn relayout_follows_the_page_width_for_an_auto_width_root() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "");
    lay_out(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    let height_before = doc.nodes[root].unrounded_layout.size.height;
    // No authored width anywhere above: the lines break again at `max_advance`,
    // narrower or wider than the width laid out at.
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
    // Only the lines change; the taffy box is left as it was.
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, height_before);
}

#[test]
fn relayout_breaks_lines_at_the_authored_or_the_given_width() {
    use crate::layout::relayout_text_for_width;
    // "aaaa bbbb cccc" is 140px of Ahem: three lines at 50px, one line in an
    // authored 200px box whatever `max_advance` is, one line at 800px.
    for (css, max_advance, lines) in [("", 50.0_f32, 3), ("width:200px", 50.0, 1), ("", 800.0, 1)] {
        let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", css);
        lay_out(&mut doc, &cascade);
        relayout_text_for_width(&mut doc, &cascade, max_advance);
        assert_eq!(
            stored_lines(&doc, root).lines.len(),
            lines,
            "{css} / {max_advance}"
        );
    }
}

// ── ifc roots beside floats ──────────────────────────────────

#[test]
fn a_paragraph_beside_a_left_float_starts_after_it() {
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb cccc", "float:left;width:30px;height:20px", "");
    lay_out(&mut doc, &cascade);
    assert!(
        doc.nodes[root].is_ifc_root(),
        "the paragraph beside a float is an ifc root"
    );
    let lines = &stored_lines(&doc, root).lines;
    // Two lines fit beside the 20px float (70px wide), the third has the width.
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaaa", "bbbb", "cccc"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(30.0), Some(0.0)]
    );
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
    assert!(stored_lines(&doc, root).beside_floats);
}

#[test]
fn a_paragraph_beside_a_right_float_keeps_its_start_and_loses_width() {
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb cccc", "float:right;width:30px;height:20px", "");
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaaa", "bbbb", "cccc"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(0.0); 3]
    );
}

#[test]
fn a_line_that_reaches_a_later_float_segment_is_narrowed_by_it() {
    // Two floats: a 30x10 one on the left at the top, then a 30x30 one on the
    // right that clears it, so it starts at y=10. The root has a 20px line
    // height. The first attempt at line 1 (assumed height 0) sees only the
    // left float: "aaa bb" (60px) fits in 70px. Its real height, 20px, reaches
    // into the right float's segment (y=10..40), so the line has to be laid
    // out again in 100 - 30 - 30 = 40px, where only "aaa" fits. A loop that
    // reads one segment, or never re-checks the space for the real height,
    // keeps "aaa bb".
    let (mut doc, cascade, _floats, root) = ahem_paragraph_beside_floats(
        "aaa bb cc",
        &[
            "float:left;width:30px;height:10px",
            "float:right;clear:left;width:30px;height:30px",
        ],
        "line-height:20px",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaa", "bb cc"]
    );
    // Line 2 (y=20) is inside the right float's segment only: 70px wide from 0.
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(0.0)]
    );
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_beside_a_float() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb cccc", "float:left;width:30px;height:20px", "");
    lay_out(&mut doc, &cascade);
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(before, ["aaaa", "bbbb", "cccc"]);
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 800.0);
    // The shared float context is gone at this point; re-breaking at the full
    // width would ignore the float the lines were laid out beside.
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(after, before);
}

#[test]
fn a_paragraph_with_padding_beside_a_float_is_offset_inside_its_content_box() {
    let (mut doc, cascade, _float, root) = ahem_paragraph_beside_float(
        "aaaa bbbb",
        "float:left;width:30px;height:20px",
        "padding:0 10px;box-sizing:border-box",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    // The content box is 80px wide and starts 10px in; the float covers the
    // first 30px of the context, so 20px of the content box remain covered.
    assert_eq!(line_start_x(&lines[0]), Some(20.0));
}

#[test]
fn a_paragraph_with_top_padding_asks_for_space_below_its_border_top() {
    // The float is 20px tall and starts at the top of the wrapper. A root with
    // 15px of top padding has its content box at y=15, so the first line
    // (y=15..25) still overlaps the float (until y=20) and starts after it,
    // while the second line (y=25) is below it.
    let (mut doc, cascade, _float, root) = ahem_paragraph_beside_float(
        "aaaa bbbb",
        "float:left;width:30px;height:20px",
        "padding-top:15px",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(0.0)]
    );
}

// ── floats inside ifc roots ──────────────────────────────────

#[test]
fn a_float_inside_the_paragraph_shortens_the_line_it_is_anchored_in() {
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb cccc dddd",
        "",
    );
    lay_out(&mut doc, &cascade);
    assert!(
        doc.nodes[root].is_ifc_root(),
        "a paragraph with a float child is an ifc root"
    );
    let lines = &stored_lines(&doc, root).lines;
    // Line 1 holds "aa" and the anchor and is 70px wide beside the float:
    // "aa bbbb" (70px) fits exactly. Line 2 is still beside the float.
    // Line 3 (y=20) is below it.
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa bbbb", "cccc", "dddd"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(30.0), Some(30.0), Some(0.0)]
    );
    // The float sits at the top left of the content box.
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 20.0));
}

#[test]
fn a_right_float_inside_the_paragraph_sits_at_the_right_edge() {
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aa",
        "float:right;width:30px;height:20px",
        " bbbb cccc dddd",
        "",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (70.0, 0.0));
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(0.0); 3]
    );
}

#[test]
fn a_float_that_would_shorten_its_own_line_moves_to_the_next_line() {
    // "aaaa bbbb" is 90px. The float (60px) is anchored right after "bbbb":
    // placing it would leave 40px and push "bbbb" (and with it the anchor) to
    // the next line, so the float is withdrawn and placed on the next line.
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aaaa bbbb",
        "float:left;width:60px;height:10px",
        " cccc",
        "",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_text(&lines[0]), "aaaa bbbb");
    assert_eq!(line_start_x(&lines[0]), Some(0.0));
    // The float is placed at the top of the second line.
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!(line_text(&lines[1]), "cccc");
    assert_eq!(line_start_x(&lines[1]), Some(60.0));
}

#[test]
fn an_intrinsic_width_includes_the_float() {
    // A shrink-to-fit root is measured for its content before the final pass.
    // The float (30px) sits on the first line with "aa bbbb" (70px), so the
    // max-content width is 100px; if the float's width is left out of the
    // intrinsic sizes the root is only 70px wide.
    let (mut doc, cascade, float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb",
        "float:left;width:auto",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_start_x(&lines[0]), Some(30.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 100.0);
}

#[test]
fn a_float_inside_a_padded_paragraph_is_placed_inside_its_content_box() {
    // `apply_content_box_inset` is what keeps the paragraph's own floats out of
    // its padding: a left float sits at the content box's left edge.
    let (mut doc, cascade, float, _root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb",
        "padding:0 10px;box-sizing:border-box",
    );
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[float].unrounded_layout.location.x, 10.0);
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_that_has_boxes() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, _float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px",
        " bbbb cccc dddd",
        "",
    );
    lay_out(&mut doc, &cascade);
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(
        after, before,
        "a float fixes the positions the lines were broken for"
    );
}

#[test]
fn a_float_wider_than_the_rest_of_its_line_waits_for_the_next_line() {
    // Two 60px left floats anchored after "aa": the first takes 60px of the
    // 100px line, the second no longer fits beside it and is placed at the
    // start of the next line instead of shortening this one.
    let (mut doc, cascade, first, root) =
        ahem_paragraph_with_float("aa", "float:left;width:60px;height:10px", "", "");
    let second = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some("display:block;float:left;width:60px;height:10px"),
    );
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_2 = raikiri_style::cascade(&doc, &rules).expect("cascade");
    drop(cascade);
    lay_out(&mut doc, &cascade_2);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "bb"]
    );
    assert_eq!(
        lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(60.0), Some(60.0)]
    );
    let at = |id: usize| {
        let layout = doc.nodes[id].unrounded_layout;
        (layout.location.x, layout.location.y)
    };
    assert_eq!(at(first), (0.0, 0.0));
    assert_eq!(at(second), (0.0, 10.0));
}

#[test]
fn a_paragraph_inside_a_float_of_a_root_is_measured() {
    // The float holds a paragraph of its own. Laying the float out from the
    // outer root lays that paragraph out too, which needs the engine state:
    // the float gets the height of its two lines.
    let (mut doc, cascade, float, root) =
        ahem_paragraph_with_float("aa", "float:left;width:30px", " bbbb", "");
    let inner = doc.append_element(Some(float), "div", Style::default(), Some("display:block"));
    doc.append_text(inner, "ff gg");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_2 = raikiri_style::cascade(&doc, &rules).expect("cascade");
    drop(cascade);
    lay_out(&mut doc, &cascade_2);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(doc.nodes[inner].is_ifc_root());
    assert_eq!(stored_lines(&doc, inner).lines.len(), 2);
    assert_eq!(doc.nodes[float].unrounded_layout.size.height, 20.0);
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_whose_float_is_below_them() {
    // The float is withdrawn from the only line and placed below it, so no
    // line is beside a float; the lines still belong with the float's place.
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_with_float("aaaa bbbb", "float:left;width:60px;height:10px", "", "");
    lay_out(&mut doc, &cascade);
    assert!(!stored_lines(&doc, root).beside_floats);
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(before, ["aaaa bbbb"]);
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(after, before);
}

#[test]
fn only_a_root_that_is_its_own_formatting_context_grows_around_its_floats() {
    // One 10px line and a 20px float. An in-flow root leaves the float
    // hanging below its line for the parent to collect; a floated root is a
    // formatting context of its own and contains it. (A later pass grows
    // auto-height ancestors of every float on both paths, so the node height
    // is not what tells the two apart; the measured content height is.)
    for (root_css, height) in [("", 10.0), ("float:left", 20.0)] {
        let (mut doc, cascade, _float, root) =
            ahem_paragraph_with_float("aa", "float:left;width:30px;height:20px", " bb", root_css);
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{root_css}");
        let lines = stored_lines(&doc, root);
        assert_eq!(lines.lines.len(), 1, "{root_css}");
        assert_eq!(lines.height, height, "{root_css}");
    }
}

// ── committed boxes of ifc roots ─────────────────────────────

#[test]
fn a_paragraph_that_is_a_formatting_context_contains_its_floats() {
    // The root is itself a block formatting context (it floats), so its height
    // reaches the bottom of the tallest float even when the text is shorter.
    let (mut doc, cascade, _float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:50px",
        " bb",
        "float:left;width:100px",
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 50.0);
}

#[test]
fn an_in_flow_paragraph_does_not_grow_for_its_floats() {
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_with_float("aa", "float:left;width:30px;height:50px", " bb", "");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // One 10px line; the float hangs below the content, which is what a block
    // that is not a formatting context does.
    assert_eq!(stored_lines(&doc, root).height, 10.0);
    // `propagate_float_bottoms_to_auto_height_ancestors` later grows every
    // auto-height ancestor of a float to the float's bottom; the float starts at the
    // top of the paragraph, so that is 50px.
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 50.0);
}

#[test]
fn a_committed_float_layout_carries_its_box_model() {
    let (mut doc, cascade, float, _root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px;margin-left:5px;padding-top:2px;box-sizing:content-box",
        " bb",
        "",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    // The border box starts after the 5px margin and is 22px tall.
    assert_eq!((layout.location.x, layout.location.y), (5.0, 0.0));
    assert_eq!(layout.size.height, 22.0);
    assert_eq!(layout.padding.top, 2.0);
    assert_eq!(layout.margin.left, 5.0);
}

#[test]
fn a_percentage_margin_resolves_against_the_paragraph_width() {
    // The root is 100px wide under an 800px body: 10% is 10px, not 80px.
    let (mut doc, cascade, float, _root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:20px;margin-left:10%",
        " bb",
        "",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!(layout.margin.left, 10.0);
    assert_eq!(layout.location.x, 10.0);
}

#[test]
fn a_padded_formatting_context_does_not_count_its_top_edge_twice() {
    // The context measures floats from the border-box top, the box height from
    // the content-box top: a 15px top padding and a 50px float make a box that
    // is 15 + 50 = 65px tall, not 15 + 65.
    let (mut doc, cascade, _float, root) = ahem_paragraph_with_float(
        "aa",
        "float:left;width:30px;height:50px",
        " bb",
        "float:left;width:100px;padding-top:15px",
    );
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 65.0);
}

// ── float placement rules the shadow record has to follow ────

/// `ahem_paragraph_with_float` with more floats after the first one: each
/// entry of `more` is `(float_css, text_after)`. Returns the floats in order.
fn ahem_paragraph_with_floats(
    before: &str,
    first: (&str, &str),
    more: &[(&str, &str)],
    root_css: &str,
) -> (Document, CascadeResult, Vec<usize>, usize) {
    let (mut doc, _cascade, float, root) =
        ahem_paragraph_with_float(before, first.0, first.1, root_css);
    let mut floats = vec![float];
    for (css, after) in more {
        floats.push(doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;{css}")),
        ));
        if !after.is_empty() {
            doc.append_text(root, *after);
        }
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, floats, root)
}

fn origin(doc: &Document, id: usize) -> (f32, f32) {
    let layout = doc.nodes[id].unrounded_layout;
    (layout.location.x, layout.location.y)
}

#[test]
fn a_float_after_a_deferred_float_is_deferred_too() {
    // CSS 2.1 9.5.1 rule 5: a float is not placed above an earlier one. The
    // 60px float does not stay on "aaaa bbbb" and goes to the next line, so
    // the 5px float after it must follow it there instead of taking line 1.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aaaa bbbb",
        ("float:left;width:60px;height:10px", ""),
        &[("float:left;width:5px;height:10px", " cc")],
        "",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_text(&lines[0]), "aaaa bbbb");
    assert_eq!(line_start_x(&lines[0]), Some(0.0));
    assert_eq!(origin(&doc, floats[0]), (0.0, 10.0));
    assert_eq!(origin(&doc, floats[1]), (60.0, 10.0));
}

#[test]
fn a_float_that_clears_a_float_of_the_same_line_does_not_narrow_it() {
    // The second float clears the first one, so it sits below it and takes no
    // room from the line both are anchored in.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aa",
        ("float:left;width:20px;height:10px", ""),
        &[("float:left;clear:left;width:20px;height:10px", " bb")],
        "",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_start_x(&lines[0]), Some(20.0));
    assert_eq!(origin(&doc, floats[0]), (0.0, 0.0));
    assert_eq!(origin(&doc, floats[1]), (0.0, 10.0));
}

#[test]
fn a_float_is_not_placed_above_an_earlier_cleared_float() {
    // Line 1 holds a 30x30 float and one that clears it (placed at y=30).
    // The 10px float on line 2 (y=10) may not go above that one, so it does
    // not narrow line 2 either.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aa",
        ("float:left;width:30px;height:30px", ""),
        &[
            ("float:left;clear:left;width:30px;height:10px", " bbbb cc"),
            ("float:left;width:10px;height:10px", " dd"),
        ],
        "",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa bbbb", "cc dd"]
    );
    assert_eq!(line_start_x(&lines[1]), Some(30.0));
    assert_eq!(origin(&doc, floats[1]), (0.0, 30.0));
    assert_eq!(origin(&doc, floats[2]).1, 30.0);
}

#[test]
fn a_line_too_narrow_beside_a_float_moves_below_it() {
    // CSS 2.1 9.5: a line box that does not fit next to a float is shifted
    // down. Only 20px are left beside the 80px float, too little for "aaaa".
    let (mut doc, cascade, _float, root) =
        ahem_paragraph_beside_float("aaaa bbbb", "float:left;width:80px;height:20px", "");
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aaaa bbbb"]
    );
    assert_eq!(lines[0].block_offset(), 20.0);
    assert_eq!(line_start_x(&lines[0]), Some(0.0));
}

#[test]
fn a_float_wider_than_the_paragraph_stays_at_the_top_and_the_text_goes_below() {
    let (mut doc, cascade, float, root) =
        ahem_paragraph_with_float("", "float:left;width:150px;height:20px", "aa", "");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(origin(&doc, float), (0.0, 0.0));
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(line_text(&lines[0]), "aa");
    assert_eq!(lines[0].block_offset(), 20.0);
}

#[test]
fn a_cleared_float_does_not_shorten_the_line_above_its_clearance() {
    // The 30x30 float of line 1 still reaches line 2 (y=10). The float of
    // line 2 clears it, so it goes below it (y=30) and must not take room
    // from line 2, which starts after the first float only.
    let (mut doc, cascade, floats, root) = ahem_paragraph_with_floats(
        "aa",
        ("float:left;width:30px;height:30px", " bbbb cc"),
        &[("float:left;clear:left;width:20px;height:10px", " dd")],
        "",
    );
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa bbbb", "cc dd"]
    );
    assert_eq!(line_start_x(&lines[1]), Some(30.0));
    assert_eq!(origin(&doc, floats[1]), (0.0, 30.0));
}

// ── atomic inlines in ifc roots ──────────────────────────────

#[test]
fn an_inline_block_is_measured_before_the_lines_are_broken() {
    // "aa " (30px) + a 30x30 inline-block + " bb". The atomic's baseline is its
    // bottom edge (it has no text), so it rises 30px above the baseline and
    // the line is 30 + 2 (the strut's descent) = 32px tall.
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:30px;height:30px", " bb", "");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 32.0);
}

#[test]
fn a_wide_atomic_moves_to_the_next_line() {
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:90px;height:10px", " bb", "");
    lay_out(&mut doc, &cascade);
    // "aa " leaves 70px, the atomic needs 90: it starts the second line. After
    // it only 10px are left, so " bb" (30px) starts a third line. The line
    // with the atomic has no text (the placeholder is stripped).
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "", "bb"]
    );
}

#[test]
fn a_shrink_to_fit_paragraph_with_an_inline_block_is_as_wide_as_its_content() {
    // max-content: "aa " (30) + the 30px atomic + "bb" (20) = 80.
    let (mut doc, cascade, _atomic, root) = ahem_paragraph_with_atomic(
        "aa ",
        "width:30px;height:10px",
        "bb",
        "float:left;width:auto",
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 80.0);
}

#[test]
fn an_inline_block_holding_a_paragraph_is_measured() {
    // The atomic holds a block holding a block with text; the innermost block
    // is an ifc root of its own (its parent is a block container and its
    // siblings are blocks). The state of the engine must survive measuring it
    // from inside the outer paragraph.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic_box = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block"),
    );
    let middle = doc.append_element(
        Some(atomic_box),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    let inner = doc.append_element(
        Some(middle),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    doc.append_text(inner, "bb cc");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(
        doc.nodes[inner].is_ifc_root(),
        "the paragraph inside the atomic is a root of its own"
    );
    // The inner paragraph was laid out while the outer one was measured; with
    // the engine state taken it would store no lines at all.
    assert_eq!(stored_lines(&doc, inner).lines.len(), 1);
}

#[test]
fn an_inline_block_baseline_counts_its_top_margin() {
    // The atomic holds one line of "ii" (baseline 8px below its border-box
    // top) and has a 3px top margin, so its baseline is 11px below its margin
    // box top. The margin box (13px) rises 11px above the line's baseline and
    // reaches 2px below it: the line is 13px tall with the baseline at 11.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:20px;margin-top:3px"),
    );
    doc.append_text(atomic, "ii");
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].block_size(), 13.0);
    assert_eq!(
        lines[0].baseline(shodo::geometry::BaselineKind::Alphabetic),
        11.0
    );
}

#[test]
fn a_clipping_inline_block_sits_on_its_bottom_margin_edge() {
    // An inline-block that is a scroll container has no baseline (CSS 2.1
    // 10.8.1): its 10px box rises 10px above the line's baseline instead of
    // lining its text up with the surrounding text, so the line is 10 + 2
    // (the strut's descent) = 12px tall.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:20px;height:10px;overflow:hidden"),
    );
    doc.append_text(atomic, "ii");
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].block_size(), 12.0);
}

#[test]
fn an_inline_block_is_placed_at_its_fragment() {
    // The atomic follows "aa " (30px) and rises 30px above the baseline: its
    // top is the line's top (y=0).
    let (mut doc, cascade, atomic, _root) =
        ahem_paragraph_with_atomic("aa ", "width:30px;height:30px", " bb", "");
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 30.0));
}

#[test]
fn an_inline_block_with_text_sits_on_its_last_baseline() {
    // The atomic holds one line of text, so its baseline is the text's (8px
    // from its top) and it lines up with the surrounding text: same line
    // height, top at y=0.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:20px"),
    );
    doc.append_text(atomic, "ii");
    doc.append_text(root, " bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 10.0);
}

#[test]
fn an_atomic_margin_moves_its_border_box_inside_the_margin_box() {
    let (mut doc, cascade, atomic, _root) = ahem_paragraph_with_atomic(
        "aa ",
        "width:30px;height:10px;margin-left:5px;margin-top:3px",
        "",
        "",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[atomic].unrounded_layout;
    // The margin box starts after "aa " (30px); the border box is 5px in.
    assert_eq!(layout.location.x, 35.0);
    assert_eq!(layout.margin.left, 5.0);
    // The atomic has no text, so its baseline is the margin box's bottom edge
    // and the margin box (3 + 10 = 13px) rises 13px above the baseline: the
    // margin box starts at the line top and the border box 3px below it.
    assert_eq!(layout.location.y, 3.0);
}

#[test]
fn atomics_on_later_lines_are_placed_on_their_own_line() {
    let (mut doc, cascade, atomic, root) =
        ahem_paragraph_with_atomic("aaaaaaaa ", "width:30px;height:10px", "", "");
    lay_out(&mut doc, &cascade);
    // "aaaaaaaa " is 90px; the atomic (30px) starts the second line.
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!(stored_lines(&doc, root).lines.len(), 2);
}

#[test]
fn an_atomic_is_offset_by_the_content_box_of_its_paragraph() {
    let (mut doc, cascade, atomic, _root) = ahem_paragraph_with_atomic(
        "aa ",
        "width:30px;height:10px",
        "",
        "padding:4px 6px;box-sizing:border-box",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (36.0, 4.0));
}

#[test]
fn an_atomic_beside_a_float_starts_after_it() {
    // The float covers the first 30px of the line, so "aa " starts at 30 and
    // the atomic follows it at 60.
    let (mut doc, _cascade, _float, root) =
        ahem_paragraph_with_float("aa ", "float:left;width:30px;height:30px", "", "");
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some("display:inline-block;width:30px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[atomic].unrounded_layout;
    assert_eq!(layout.location.x, 60.0);
}

#[test]
fn an_image_is_placed_like_an_inline_block() {
    // A 10x10 image after "aa " sits on the baseline with its bottom edge: it
    // rises 10px, 2px above the strut's ascent, so the line is 12px tall and
    // the image's top is the line's top.
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "width:100px");
    let image = doc.append_element(
        Some(root),
        "img",
        taffy::Style::default(),
        Some("display:inline;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[image].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (30.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (10.0, 10.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 12.0);
}

// ── block children of ifc roots ──────────────────────────────

/// `root` holds `before`, a block div of the given css, then `after`.
fn paragraph_with_block(
    before: &str,
    block_css: &str,
    after: &str,
    root_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let (mut doc, _cascade, root) = ahem_paragraph(before, &format!("width:100px;{root_css}"));
    let block = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;{block_css}")),
    );
    doc.append_text(root, after);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, block, root)
}

#[test]
fn a_block_child_splits_the_lines_around_it() {
    let (mut doc, cascade, block, root) = paragraph_with_block("aa", "height:20px", "bb", "");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "bb"]
    );
    assert_eq!(
        lines.iter().map(|l| l.block_offset()).collect::<Vec<_>>(),
        [0.0, 30.0]
    );
    let layout = doc.nodes[block].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!((layout.size.width, layout.size.height), (100.0, 20.0));
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 40.0);
}

#[test]
fn a_leading_block_child_starts_the_paragraph() {
    let (mut doc, cascade, block, root) = paragraph_with_block("", "height:20px", "bb", "");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[block].unrounded_layout.location.y, 0.0);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
}

#[test]
fn a_block_child_is_as_wide_as_the_content_box_less_its_margins() {
    let (mut doc, cascade, block, _root) = paragraph_with_block(
        "aa",
        "height:10px;margin-left:10px;margin-right:20px",
        "bb",
        "",
    );
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[block].unrounded_layout;
    assert_eq!((layout.location.x, layout.size.width), (10.0, 70.0));
}

#[test]
fn a_block_child_is_offset_by_the_content_box_of_its_paragraph() {
    let (mut doc, cascade, block, root) = paragraph_with_block(
        "aa",
        "height:10px",
        "bb",
        "padding:4px 6px;box-sizing:border-box",
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[block].unrounded_layout;
    // The content box starts at (6, 4); the block follows the first line.
    // (Its width is the root's authored 100px, which a pre-layout pass copies
    // into every auto-width block under an authored-width ancestor on both
    // paths; the block child keeps the width taffy's style gives it.)
    assert_eq!((layout.location.x, layout.location.y), (6.0, 14.0));
    assert_eq!(layout.size.width, 100.0);
}

#[test]
fn vertical_block_children_follow_the_logical_block_direction() {
    for (mode, expected_x) in [("vertical-rl", 78.0), ("vertical-lr", 10.0)] {
        let (mut doc, cascade, block, root) = paragraph_with_block(
            "aa",
            "width:12px;height:8px",
            "bb",
            &format!("height:50px;writing-mode:{mode}"),
        );
        lay_out(&mut doc, &cascade);

        let child = doc.nodes[block].unrounded_layout;
        assert_eq!(
            (child.location.x, child.location.y),
            (expected_x, 0.0),
            "{mode}"
        );
        assert_eq!((child.size.width, child.size.height), (12.0, 8.0), "{mode}");
        assert_eq!(stored_lines(&doc, root).lines.len(), 2, "{mode}");
    }
}

#[test]
fn vertical_rtl_block_child_uses_the_logical_inline_start_once() {
    for (mode, expected_x) in [("vertical-rl", 80.0), ("vertical-lr", 0.0)] {
        let (mut doc, cascade, block, root) = paragraph_with_block(
            "",
            "width:20px;height:20px;margin-top:3px;margin-bottom:7px",
            "bb",
            &format!("height:100px;writing-mode:{mode};direction:rtl"),
        );
        lay_out(&mut doc, &cascade);

        assert!(doc.nodes[root].is_ifc_root(), "{mode}");
        let child = doc.nodes[block].unrounded_layout;
        assert_eq!(
            (child.location.x, child.location.y),
            (expected_x, 73.0),
            "{mode}"
        );
    }
}

#[test]
fn dimensioned_empty_vertical_block_children_follow_the_block_axis() {
    for (mode, expected_x) in [("vertical-rl", [80.0, 60.0]), ("vertical-lr", [0.0, 20.0])] {
        let (mut doc, _cascade, root) = ahem_paragraph(
            "bb",
            &format!("width:100px;height:30px;writing-mode:{mode};font-size:0;line-height:0"),
        );
        let first = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;width:20px;height:30px"),
        );
        let second = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;width:20px;height:30px"),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");

        lay_out(&mut doc, &cascade);

        assert!(doc.nodes[root].is_ifc_root(), "{mode}");
        assert_eq!(
            doc.nodes[first].unrounded_layout.location.x, expected_x[0],
            "{mode}"
        );
        assert_eq!(
            doc.nodes[second].unrounded_layout.location.x, expected_x[1],
            "{mode}"
        );
    }
}

#[test]
fn vertical_rl_keeps_multiple_block_children_in_logical_source_order() {
    let (mut doc, _, root) =
        ahem_paragraph("aa", "width:100px;height:50px;writing-mode:vertical-rl");
    let first = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some("display:block;width:12px;height:8px"),
    );
    doc.append_text(root, "bb");
    let second = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some("display:block;width:14px;height:8px"),
    );
    doc.append_text(root, "cc");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);

    assert_eq!(doc.nodes[first].unrounded_layout.location.x, 78.0);
    assert_eq!(doc.nodes[second].unrounded_layout.location.x, 54.0);
}

#[test]
fn vertical_block_children_collapse_adjoining_block_axis_margins() {
    for (mode, first_margin, second_margin, gap_direction, expected_gap) in [
        (
            "vertical-rl",
            "margin-left:10px",
            "margin-right:20px",
            -1.0,
            20.0,
        ),
        (
            "vertical-lr",
            "margin-right:10px",
            "margin-left:20px",
            1.0,
            20.0,
        ),
        (
            "vertical-rl",
            "margin-left:20px",
            "margin-right:-5px",
            -1.0,
            15.0,
        ),
        (
            "vertical-lr",
            "margin-right:20px",
            "margin-left:-5px",
            1.0,
            15.0,
        ),
        (
            "vertical-rl",
            "margin-left:-8px",
            "margin-right:-5px",
            -1.0,
            -8.0,
        ),
        (
            "vertical-lr",
            "margin-right:-8px",
            "margin-left:-5px",
            1.0,
            -8.0,
        ),
    ] {
        let (mut doc, _, root) =
            ahem_paragraph("", &format!("width:100px;height:100px;writing-mode:{mode}"));
        let first = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!(
                "display:block;width:10px;height:20px;{first_margin}"
            )),
        );
        doc.append_text(first, "a");
        let second = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!(
                "display:block;width:10px;height:20px;{second_margin}"
            )),
        );
        doc.append_text(second, "b");
        doc.append_text(root, "c");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);

        let first_layout = doc.nodes[first].unrounded_layout;
        let second_layout = doc.nodes[second].unrounded_layout;
        let gap = if gap_direction < 0.0 {
            first_layout.location.x - (second_layout.location.x + second_layout.size.width)
        } else {
            second_layout.location.x - (first_layout.location.x + first_layout.size.width)
        };
        assert_eq!(
            gap, expected_gap,
            "{mode}: {first_margin} / {second_margin}"
        );
    }
}

#[test]
fn vertical_block_child_margin_percentages_resolve_against_inline_size() {
    for (mode, block_start_margin, expected_x) in [
        ("vertical-rl", "margin-right:10%", 40.0),
        ("vertical-lr", "margin-left:10%", 10.0),
    ] {
        let (mut doc, _, root) =
            ahem_paragraph("", &format!("width:60px;height:100px;writing-mode:{mode}"));
        let block = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!(
                "display:block;width:10px;height:20px;{block_start_margin}"
            )),
        );
        doc.append_text(block, "a");
        doc.append_text(root, "b");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);

        assert_eq!(
            doc.nodes[block].unrounded_layout.location.x, expected_x,
            "{mode}"
        );
    }
}

#[test]
fn vertical_block_child_percentage_edges_use_parent_inline_size() {
    for (mode, block_start_side) in [
        ("vertical-rl", "margin-right"),
        ("vertical-lr", "margin-left"),
    ] {
        let (mut doc, _, root) =
            ahem_paragraph("", &format!("width:60px;height:100px;writing-mode:{mode}"));
        let block = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!(
                "display:block;width:30px;height:49px;padding:10%;{block_start_side}:10%"
            )),
        );
        doc.append_text(block, "a");
        doc.append_text(root, "b");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);

        let layout = doc.nodes[block].unrounded_layout;
        assert_eq!(stored_lines(&doc, block).width, 49.0, "{mode}");
        assert_eq!(
            (layout.size.width, layout.size.height),
            (50.0, 69.0),
            "{mode}"
        );
        assert_eq!(layout.padding.left, 10.0, "{mode}");
        assert_eq!(layout.padding.top, 10.0, "{mode}");
        assert_eq!(
            if mode == "vertical-rl" {
                layout.margin.right
            } else {
                layout.margin.left
            },
            10.0,
            "{mode}"
        );
    }
}

#[test]
fn vertical_block_children_distribute_auto_inline_margins() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, _, root) =
            ahem_paragraph("", &format!("width:100px;height:100px;writing-mode:{mode}"));
        let block = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:block;width:10px;height:20px;margin-top:auto;margin-bottom:auto"),
        );
        doc.append_text(block, "a");
        doc.append_text(root, "b");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);

        assert_eq!(doc.nodes[block].unrounded_layout.location.y, 40.0, "{mode}");
    }
}

#[test]
fn vertical_block_child_auto_inline_margin_is_zero_when_the_child_overflows() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, _, root) =
            ahem_paragraph("", &format!("width:100px;height:20px;writing-mode:{mode}"));
        let block = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:block;width:10px;height:40px;margin-top:auto"),
        );
        doc.append_text(block, "a");
        doc.append_text(root, "b");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);

        assert_eq!(doc.nodes[block].unrounded_layout.location.y, 0.0, "{mode}");
    }
}

#[test]
fn vertical_nested_block_children_match_flat_flow_positions() {
    let positions = |nested: bool| {
        let (mut doc, _, root) =
            ahem_paragraph("", "width:100px;height:50px;writing-mode:vertical-rl");
        let mut blocks = Vec::new();
        if nested {
            let first = doc.append_element(
                Some(root),
                "div",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(first, "aa");
            blocks.push(first);

            let wrapper = doc.append_element(
                Some(root),
                "if-zh",
                taffy::Style::default(),
                Some("display:block"),
            );
            for text in ["bb", "cc"] {
                let child = doc.append_element(
                    Some(wrapper),
                    "div",
                    taffy::Style::default(),
                    Some("display:block"),
                );
                doc.append_text(child, text);
                blocks.push(child);
            }

            let last = doc.append_element(
                Some(root),
                "div",
                taffy::Style::default(),
                Some("display:block"),
            );
            doc.append_text(last, "dd");
            blocks.push(last);
        } else {
            for text in ["aa", "bb", "cc", "dd"] {
                let block = doc.append_element(
                    Some(root),
                    "div",
                    taffy::Style::default(),
                    Some("display:block"),
                );
                doc.append_text(block, text);
                blocks.push(block);
            }
        }
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        blocks
            .into_iter()
            .map(|block| {
                let layout = doc.nodes[block].unrounded_layout;
                let mut absolute_x = 0.0;
                let mut absolute_y = 0.0;
                let mut ancestor = Some(block);
                while let Some(node) = ancestor {
                    let layout = doc.nodes[node].unrounded_layout;
                    absolute_x += layout.location.x;
                    absolute_y += layout.location.y;
                    ancestor = doc.parent_of(node);
                }
                (
                    absolute_x,
                    absolute_y,
                    layout.size.width,
                    layout.size.height,
                )
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(positions(true), positions(false));
}

#[test]
fn vertical_rl_block_child_placement_accounts_for_root_padding_and_border() {
    let root_css = "width:100px;height:40px;box-sizing:border-box;writing-mode:vertical-rl;padding:5px 4px 3px 6px;border:1px solid";
    let (mut doc, cascade, block, root) =
        paragraph_with_block("aa", "width:12px;height:8px", "bb", root_css);
    lay_out(&mut doc, &cascade);

    assert_eq!(stored_lines(&doc, root).width, 30.0);
    let child = doc.nodes[block].unrounded_layout;
    assert_eq!((child.location.x, child.location.y), (73.0, 6.0));
}

#[test]
fn text_inside_a_block_child_is_laid_out_inside_the_block() {
    // The block is a box of the outer root and the root of its own text: it
    // is one 10px line tall ("bb cc dd" is 80px in the 100px block).
    let (mut doc, _cascade, block, root) = paragraph_with_block("aa", "", "cc", "");
    doc.append_text(block, "bb cc dd");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(doc.nodes[block].is_ifc_root());
    let layout = doc.nodes[block].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 10.0));
    assert_eq!(layout.size.height, 10.0);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(
        lines.iter().map(line_text).collect::<Vec<_>>(),
        ["aa", "cc"]
    );
    assert_eq!(
        lines.iter().map(|l| l.block_offset()).collect::<Vec<_>>(),
        [0.0, 20.0]
    );
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0);
}

#[test]
fn a_float_anchored_right_before_a_block_child_is_placed_above_it() {
    // No text precedes the block, so the float's anchor ends no line: the
    // float is still placed, at the top, and the block starts there too.
    let (mut doc, _cascade, root) = ahem_paragraph("", "width:100px");
    let float = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some("display:block;float:left;width:30px;height:10px"),
    );
    let block = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some("display:block;height:20px"),
    );
    doc.append_text(root, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[float].unrounded_layout;
    assert_eq!((layout.location.x, layout.location.y), (0.0, 0.0));
    assert_eq!((layout.size.width, layout.size.height), (30.0, 10.0));
    assert_eq!(doc.nodes[block].unrounded_layout.location.y, 0.0);
}

#[test]
fn a_shrink_to_fit_paragraph_is_as_wide_as_its_widest_block_child() {
    // max-content: the lines are 20px wide, the block child 60px.
    let (mut doc, cascade, _block, root) = paragraph_with_block(
        "aa",
        "width:60px;height:10px",
        "bb",
        "float:left;width:auto",
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 60.0);
}

#[test]
fn a_paragraph_that_contains_its_floats_contains_those_of_its_block_children() {
    // The root floats, so it is a formatting context of its own. The block
    // child holds only a 30x50 float, which starts below "aa" (y=10) and ends
    // at 60; "bb" goes beside it. (The node height grows to the float in a
    // later pass on both paths; the measured content height is what counts.)
    let (mut doc, _cascade, block, root) = paragraph_with_block("aa", "", "bb", "float:left");
    doc.append_element(
        Some(block),
        "div",
        taffy::Style::default(),
        Some("display:block;float:left;width:30px;height:50px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = stored_lines(&doc, root);
    assert_eq!(
        lines.lines.iter().map(line_start_x).collect::<Vec<_>>(),
        [Some(0.0), Some(30.0)]
    );
    assert_eq!(lines.height, 60.0);
}

// ── re-break rules for roots with boxes ──────────────────────

/// `root` (no authored width, so the re-break follows the page width) holds
/// "aa ", a child of the given element css, then `after`.
fn unsized_paragraph_with_child(child_css: &str, after: &str) -> (Document, CascadeResult, usize) {
    let (mut doc, _cascade, root) = ahem_paragraph("aa ", "");
    doc.append_element(Some(root), "span", taffy::Style::default(), Some(child_css));
    doc.append_text(root, after);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_with_an_atomic() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) =
        unsized_paragraph_with_child("display:inline-block;width:30px;height:10px", " bbbb cccc");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(
        after, before,
        "an atomic's position is tied to the lines it was placed in"
    );
}

#[test]
fn relayout_keeps_the_lines_of_a_paragraph_with_a_block_child() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) =
        unsized_paragraph_with_child("display:block;height:20px", "bbbb cccc");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let before: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    let after: Vec<_> = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    assert_eq!(after, before);
}

#[test]
fn relayout_still_follows_the_page_width_for_a_plain_paragraph() {
    use crate::layout::relayout_text_for_width;
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "");
    lay_out(&mut doc, &cascade);
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 3);
}

#[test]
fn a_block_child_keeps_its_own_width() {
    for (css, width) in [
        ("width:40px;height:10px", 40.0),
        ("max-width:50%;height:10px", 50.0),
        ("width:10px;min-width:30px;height:10px", 30.0),
        (
            "width:40px;padding-left:5px;box-sizing:border-box;height:10px",
            40.0,
        ),
        ("width:40px;padding-left:5px;height:10px", 45.0),
    ] {
        let (mut doc, cascade, block, root) = paragraph_with_block("aa", css, "bb", "");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        let layout = doc.nodes[block].unrounded_layout;
        assert_eq!(
            (layout.location.x, layout.size.width),
            (0.0, width),
            "{css}"
        );
    }
}

#[test]
fn a_right_float_inside_a_narrow_block_child_sits_at_the_block_edge() {
    // Each block is 40px wide, so its 10px right float is 30px in.
    for css in ["width:40px", "max-width:40px", "width:10px;min-width:40px"] {
        let (mut doc, _cascade, block, root) = paragraph_with_block("aa", css, "bb", "");
        let float = doc.append_element(
            Some(block),
            "div",
            taffy::Style::default(),
            Some("display:block;float:right;width:10px;height:10px"),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        assert_eq!(doc.nodes[float].unrounded_layout.location.x, 30.0, "{css}");
    }
}

#[test]
fn a_float_root_with_its_own_width_breaks_a_long_word_at_that_width() {
    // `overflow-wrap: break-word` does not lower the min-content width, so a
    // shrink-to-fit clamp would lay the word out on one 240px line; the
    // float's own `width` fixes the line width instead (CSS 2.1 10.3.5 applies
    // shrink-to-fit only to an `auto` width).
    for css in [
        "float:left;width:100px;overflow-wrap:break-word",
        "float:left;width:100px;word-break:break-word",
        "float:left;width:12.5%;overflow-wrap:break-word",
        "float:left;width:calc(50px + 6.25%);overflow-wrap:break-word",
    ] {
        let (mut doc, cascade, root) = ahem_paragraph("FillerFillerFillerFiller", css);
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        assert_eq!(doc.nodes[root].unrounded_layout.size.width, 100.0, "{css}");
        assert_eq!(stored_lines(&doc, root).lines.len(), 3, "{css}");
        assert_eq!(doc.nodes[root].unrounded_layout.size.height, 30.0, "{css}");
    }
}

#[test]
fn a_float_root_without_a_width_still_shrinks_to_fit() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bb", "float:left");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 70.0);
    assert_eq!(stored_lines(&doc, root).lines.len(), 1);
}

// ── boxes of inline elements in ifc roots ────────────────────

const SPAN_EDGES: &str = "padding:0 3px;border-width:0 2px;border-style:solid;margin:0 4px";

#[test]
fn an_inline_element_has_its_pinned_border_box() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let make = || {
        let mut span_id = 0;
        let (doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
            doc.append_text(root, "aa");
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some(format!("display:inline;{SPAN_EDGES}").as_str()),
            );
            doc.append_text(inner, "bb");
            doc.append_text(root, "cc");
            span_id = inner;
        });
        (doc, cascade, span_id)
    };
    let (mut off_doc, off_cascade, off_span) = make();
    layout_single_page(with_ahem(&mut off_doc), &off_cascade, page_box_800x600()).expect("layout");
    let (mut on_doc, on_cascade, on_span) = make();
    lay_out(&mut on_doc, &on_cascade);
    assert!(on_doc.nodes[on_doc.parent_of(on_span).expect("root")].is_ifc_root());

    let off = absolute_rect(&off_doc, off_span);
    assert_eq!(off, (24.0, 0.0, 30.0, 10.0), "the span's border box");
    assert_eq!(absolute_rect(&on_doc, on_span), off);
}

#[test]
fn a_padded_root_places_the_inline_element_inside_its_content_box() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let make = || {
        let mut span_id = 0;
        let (doc, cascade, _root) = ahem_paragraph_with(
            "width:200px;padding:0 5px 0 7px;box-sizing:content-box",
            |doc, root| {
                doc.append_text(root, "aa");
                let inner = doc.append_element(
                    Some(root),
                    "span",
                    taffy::Style::default(),
                    Some("display:inline;padding:0 3px"),
                );
                doc.append_text(inner, "bb");
                span_id = inner;
            },
        );
        (doc, cascade, span_id)
    };
    let (mut off_doc, off_cascade, off_span) = make();
    layout_single_page(with_ahem(&mut off_doc), &off_cascade, page_box_800x600()).expect("layout");
    let (mut on_doc, on_cascade, on_span) = make();
    lay_out(&mut on_doc, &on_cascade);
    assert!(on_doc.nodes[on_doc.parent_of(on_span).expect("root")].is_ifc_root());
    // 7px left padding, "aa" 20px, then the span's box.
    assert_eq!(absolute_rect(&on_doc, on_span), (27.0, 0.0, 26.0, 10.0));
    assert_eq!(
        absolute_rect(&on_doc, on_span),
        absolute_rect(&off_doc, off_span)
    );
}

#[test]
fn a_root_with_a_top_border_places_the_inline_element_below_it() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:200px;border-width:6px 0 0 0;border-style:solid;padding-top:1px",
        |doc, root| {
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline"),
            );
            doc.append_text(inner, "bb");
            span_id = inner;
        },
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // The content box starts 7px below the root's border-box top.
    let (_, root_y, _, _) = absolute_rect(&doc, root);
    assert_eq!(
        absolute_rect(&doc, span_id),
        (0.0, root_y + 7.0, 20.0, 10.0)
    );
}

#[test]
fn a_wrapping_inline_element_has_the_bounding_box_of_its_pieces() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some(format!("display:inline;{SPAN_EDGES}").as_str()),
        );
        doc.append_text(inner, "aaaa bbbb");
        span_id = inner;
    });
    lay_out(&mut doc, &cascade);
    // The span wraps after "aaaa ": pieces x 4..49 (line 1) and x 0..45
    // (line 2); the bounding box is x 0..49, y 0..20.
    assert_eq!(absolute_rect(&doc, span_id), (0.0, 0.0, 49.0, 20.0));
}

#[test]
fn nested_inline_elements_accumulate_to_their_own_boxes() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let (mut outer_id, mut inner_id) = (0, 0);
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "a");
        let outer = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 1px"),
        );
        doc.append_text(outer, "b");
        let inner = doc.append_element(
            Some(outer),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 2px"),
        );
        doc.append_text(inner, "c");
        outer_id = outer;
        inner_id = inner;
    });
    lay_out(&mut doc, &cascade);
    assert_eq!(absolute_rect(&doc, outer_id), (10.0, 0.0, 26.0, 10.0));
    assert_eq!(absolute_rect(&doc, inner_id), (21.0, 0.0, 14.0, 10.0));
}

#[test]
fn an_empty_inline_element_has_a_zero_width_box_on_its_line() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    // An inline element without content still has a content area on the
    // line: 0 wide, ascent + descent (10px) tall, after "aa".
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        span_id = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline"),
        );
    });
    lay_out(&mut doc, &cascade);
    assert_eq!(absolute_rect(&doc, span_id), (20.0, 0.0, 0.0, 10.0));
}

#[test]
fn an_inline_element_without_a_piece_gets_an_empty_layout() {
    use crate::layout::test_support::ahem_paragraph_with;
    let mut span_id = 0;
    let (mut doc, cascade, root) =
        ahem_paragraph_with("width:200px;padding:5px 0 0 7px", |doc, root| {
            doc.append_text(root, "aa");
            span_id = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:none;padding:0 3px"),
            );
            doc.append_text(span_id, "bb");
        });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let layout = doc.nodes[span_id].unrounded_layout;
    assert_eq!(
        (
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height
        ),
        (0.0, 0.0, 0.0, 0.0)
    );
}

#[test]
fn a_second_layout_does_not_keep_the_stale_box_of_a_removed_element() {
    use crate::layout::test_support::ahem_paragraph_with;
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 3px"),
        );
        doc.append_text(inner, "bb");
        span_id = inner;
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[span_id].unrounded_layout.size.width > 0.0);
    // Hide the element and lay out again: it has no piece any more.
    doc.set_element_inline_style(span_id, Some("display:none".into()));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[span_id].unrounded_layout.size.width, 0.0);
}

#[test]
fn a_wide_padding_does_not_make_the_layout_check_zero_the_element() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    // The padding (2 x 40) is wider than the bounding box of the pieces,
    // which carry one edge each; the layout check must not read that as a
    // negative content box and zero the element.
    let mut span_id = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:60px", |doc, root| {
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 40px"),
        );
        doc.append_text(inner, "a b");
        span_id = inner;
    });
    lay_out(&mut doc, &cascade);
    assert!(
        absolute_rect(&doc, span_id).2 > 0.0,
        "the element kept its box"
    );
}

#[test]
fn relayout_records_the_boxes_again_for_the_new_lines() {
    use crate::layout::relayout_text_for_width;
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    // No authored width: relayout follows the page width. The content box
    // starts 7px right of and 3px below the root's border-box corner.
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "padding-left:7px;border-width:3px 0 0 0;border-style:solid",
        |doc, root| {
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline"),
            );
            doc.append_text(inner, "aaaa bbbb");
            span_id = inner;
        },
    );
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(
        absolute_rect(&doc, span_id),
        (7.0, 3.0, 90.0, 10.0),
        "one line"
    );
    relayout_text_for_width(with_ahem(&mut doc), &cascade, 50.0);
    assert_eq!(
        absolute_rect(&doc, span_id),
        (7.0, 3.0, 40.0, 20.0),
        "two lines at 50px"
    );
}

/// Pages of a paragraph whose inline element asks for a page break before it.
fn pages_with_a_breaking_inline() -> (usize, bool) {
    use crate::layout::test_support::ahem_paragraph_with;
    // At 30px the span lands on the third line, 20px down the page.
    let (mut doc, cascade, root) = ahem_paragraph_with("width:30px", |doc, root| {
        doc.append_text(root, "aa aa ");
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;break-before:page"),
        );
        doc.append_text(inner, "bb");
        doc.append_text(root, " cc");
    });
    let slices = layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("pages");
    (slices.len(), doc.nodes[root].is_ifc_root())
}

#[test]
fn an_inline_element_in_an_ifc_paragraph_is_not_a_page_break_candidate() {
    // `break-before` applies to block-level boxes (CSS Fragmentation 3 3.1);
    // the paragraph's lines are not split around an inline element.
    assert_eq!(pages_with_a_breaking_inline(), (1, true));
}

#[test]
fn a_relative_inline_element_is_located_with_its_offset() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let mut span_id = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let inner = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;position:relative;left:5px;top:2px"),
        );
        doc.append_text(inner, "bb");
        span_id = inner;
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // Unshifted "bb" box: x 20..40, y 0..10; shifted by (5, 2).
    assert_eq!(absolute_rect(&doc, span_id), (25.0, 2.0, 20.0, 10.0));
}

#[test]
fn a_relative_inline_is_located() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let make = || {
        let mut span_id = 0;
        let (doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
            doc.append_text(root, "aa");
            let inner = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:inline;position:relative;right:4px;bottom:3px"),
            );
            doc.append_text(inner, "bb");
            span_id = inner;
        });
        (doc, cascade, span_id)
    };
    let (mut off_doc, off_cascade, off_span) = make();
    layout_single_page(with_ahem(&mut off_doc), &off_cascade, page_box_800x600()).expect("layout");
    let (mut on_doc, on_cascade, on_span) = make();
    lay_out(&mut on_doc, &on_cascade);
    assert!(on_doc.nodes[on_doc.parent_of(on_span).expect("root")].is_ifc_root());
    let off = absolute_rect(&off_doc, off_span);
    assert_eq!(off, (16.0, -3.0, 20.0, 10.0), "the span is shifted");
    assert_eq!(absolute_rect(&on_doc, on_span), off);
}

#[test]
fn a_child_of_a_relative_inline_moves_with_it() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let (mut outer_id, mut inner_id) = (0, 0);
    let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
        let outer = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:inline;position:relative;left:5px"),
        );
        doc.append_text(outer, "a");
        let inner = doc.append_element(
            Some(outer),
            "span",
            taffy::Style::default(),
            Some("display:inline"),
        );
        doc.append_text(inner, "b");
        outer_id = outer;
        inner_id = inner;
    });
    lay_out(&mut doc, &cascade);
    assert_eq!(absolute_rect(&doc, outer_id), (5.0, 0.0, 20.0, 10.0));
    // The child is not itself relative, but sits inside the shifted parent.
    assert_eq!(absolute_rect(&doc, inner_id), (15.0, 0.0, 10.0, 10.0));
}

#[test]
fn an_element_inside_a_contents_element_is_located_from_the_nearest_box() {
    use crate::layout::test_support::{absolute_rect, ahem_paragraph_with};
    let (mut wrapper_id, mut inner_id) = (0, 0);
    let (mut doc, cascade, root) = ahem_paragraph_with("width:200px", |doc, root| {
        doc.append_text(root, "aa");
        let wrapper = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some("display:contents"),
        );
        let inner = doc.append_element(
            Some(wrapper),
            "span",
            taffy::Style::default(),
            Some("display:inline;padding:0 3px"),
        );
        doc.append_text(inner, "bb");
        wrapper_id = wrapper;
        inner_id = inner;
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // The contents element has no box; the span after "aa" is x 20..46.
    assert_eq!(absolute_rect(&doc, wrapper_id), (0.0, 0.0, 0.0, 0.0));
    assert_eq!(absolute_rect(&doc, inner_id), (20.0, 0.0, 26.0, 10.0));
}

#[test]
fn a_block_inside_a_contents_child_of_a_paragraph_still_breaks_the_page() {
    use crate::layout::test_support::ahem_paragraph_with;
    let pages = |switch: bool| {
        let (mut doc, cascade, _root) = ahem_paragraph_with("width:200px", |doc, root| {
            doc.append_text(root, "aa");
            let wrapper = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("display:contents"),
            );
            let block = doc.append_element(
                Some(wrapper),
                "div",
                taffy::Style::default(),
                Some("display:block;break-before:page"),
            );
            doc.append_text(block, "bb");
        });
        if switch {
            doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        }
        layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600())
            .expect("pages")
            .len()
    };
    assert_eq!(pages(true), 2);
}

// ── roots under other layout algorithms ─────────────────────

use crate::layout::test_support::{
    ahem_paragraph_in, body_with_text_and_block_child, flex_container_with_inline_item,
    paragraph_with_inline_block, paragraph_with_only_an_empty_span, paragraph_with_only_atomic,
    paragraph_with_only_br,
};

/// Lay `doc` out with Ahem. The inline engine is the only text layout path,
/// so `ifc` no longer changes anything; the callers' comparisons of the two
/// values compare a layout with itself.
/// Border-box size of the paragraph of [`ahem_paragraph_in`].
fn laid_out_sizes(parent_css: &str, css: &str, text: &str) -> (f32, f32) {
    let (mut doc, cascade, root) = ahem_paragraph_in(parent_css, css, text);
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[root].unrounded_layout;
    (layout.size.width, layout.size.height)
}

/// Offset of the outer line's baseline from the top of the inline-block of
/// [`paragraph_with_inline_block`]: where the inline-block's own baseline sits.
fn inline_block_baseline(text: &str, width: f32) -> f32 {
    let (mut doc, cascade, root, inline_block) = paragraph_with_inline_block(text, width);
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let line = &stored_lines(&doc, root).lines[0];
    let line_baseline =
        line.block_offset() + line.baseline(shodo::geometry::BaselineKind::Alphabetic);
    line_baseline - doc.nodes[inline_block].unrounded_layout.location.y
}

#[test]
fn a_flex_item_becomes_an_ifc_root() {
    let (mut doc, cascade, root) = ahem_paragraph_in("display:flex", "", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn an_inline_flex_item_becomes_an_ifc_root() {
    // The item is `display:inline` in the cascade; only taffy blockifies it.
    let (mut doc, cascade, span) = flex_container_with_inline_item("aaaa");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[span].is_ifc_root());
    assert_eq!(doc.nodes[span].unrounded_layout.size.width, 40.0); // 4 em of Ahem 10px
}

#[test]
fn a_block_child_of_a_paragraph_with_text_is_its_own_root() {
    // <body>aaaa <div>bbbb cccc</div></body>: the div is a box of body's paragraph
    // and the root of its own text.
    let (mut doc, cascade, body, div) = body_with_text_and_block_child("aaaa ", "bbbb cccc");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[body].is_ifc_root());
    assert!(doc.nodes[div].is_ifc_root());
}

#[test]
fn a_flex_item_is_laid_out() {
    // Ahem 10px: "aaaa bbbb" is 90px wide in one line, 40px per word when wrapped.
    let mut expected_3795 = [(90.0, 10.0), (50.0, 20.0), (800.0, 10.0), (90.0, 10.0)].into_iter();
    for (parent_css, css) in [
        ("display:flex", ""),
        ("display:flex", "width:50px"),
        ("display:flex;flex-direction:column", ""),
        ("display:flex;align-items:flex-start", ""),
    ] {
        assert_eq!(
            laid_out_sizes(parent_css, css, "aaaa bbbb"),
            expected_3795.next().expect("a value per case"),
            "{parent_css} / {css}"
        );
    }
    // Hand-computed, not an oracle: a flex row item shrinks to its max-content.
    assert_eq!(
        laid_out_sizes("display:flex", "", "aaaa bbbb"),
        (90.0, 10.0)
    );
}

#[test]
fn a_paragraph_of_only_an_atomic_becomes_a_root() {
    let (mut doc, cascade, root) =
        paragraph_with_only_atomic("display:inline-block;width:20px;height:10px");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn a_paragraph_of_only_br_elements_becomes_a_root() {
    let (mut doc, cascade, root) = paragraph_with_only_br(2);
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 20.0); // two 10px lines
}

#[test]
fn a_paragraph_of_only_an_empty_inline_with_edges_becomes_a_root() {
    let (mut doc, cascade, root) =
        paragraph_with_only_an_empty_span("padding:4px;border:1px solid");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn an_inline_block_baseline_is_its_last_line() {
    // Two lines of 10px text: the inline-block's baseline sits on line 2.
    let off = 18.0;
    let on = inline_block_baseline("aaaa bbbb", 40.0);
    assert_eq!(on, off);
    assert_eq!(on, 18.0); // 10 (line 1) + 8 (ascent of line 2)
}

#[test]
fn a_paragraph_of_only_an_atomic_has_the_strut_descent_below_it() {
    // A 20x10 inline-block without lines sits on the baseline with its bottom
    // margin edge (CSS 2.1 10.8.1); the strut adds Ahem's 2px descent below
    // the baseline, so the line is 12px tall.
    let css = "display:inline-block;width:20px;height:10px";
    let (mut doc, cascade, root) = paragraph_with_only_atomic(css);
    lay_out(&mut doc, &cascade);
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 12.0);
}

#[test]
fn a_paragraph_of_only_an_empty_inline_with_edges_is_one_line_tall() {
    {
        let (mut doc, cascade, root) =
            paragraph_with_only_an_empty_span("padding:4px;border:1px solid");
        lay_out(&mut doc, &cascade);
        assert_eq!(doc.nodes[root].unrounded_layout.size.height, 10.0);
    }
}

#[test]
fn a_grid_item_becomes_an_ifc_root() {
    let (mut doc, cascade, root) = ahem_paragraph_in("display:grid", "", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(
        laid_out_sizes("display:grid", "", "aaaa bbbb"),
        (800.0, 10.0)
    );
    // Hand-computed: the single grid column stretches over the 800px page.
    assert_eq!(
        laid_out_sizes("display:grid", "", "aaaa bbbb"),
        (800.0, 10.0)
    );
}

#[test]
fn an_inline_block_with_text_becomes_an_ifc_root() {
    let (mut doc, cascade, root) = ahem_paragraph_in("", "display:inline-block", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let on = laid_out_sizes("", "display:inline-block", "aaaa bbbb");
    assert_eq!(on, (90.0, 10.0));
    // Hand-computed: shrink-to-fit is the max-content width of "aaaa bbbb".
    assert_eq!(on, (90.0, 10.0));
}

#[test]
fn a_list_item_becomes_an_ifc_root() {
    let css = "display:list-item;list-style-type:none;width:50px";
    let (mut doc, cascade, root) = ahem_paragraph_in("", css, "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let on = laid_out_sizes("", css, "aaaa bbbb");
    assert_eq!(on, (50.0, 20.0));
    // Hand-computed: two 40px words do not fit 50px together.
    assert_eq!(on, (50.0, 20.0));
}

/// `location.y` of two flex items aligned by their baselines: Ahem at 10px and
/// at 20px, with a line height of 1.
fn flex_baseline_item_ys() -> (f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;align-items:baseline;font-family:Ahem;line-height:1"),
    );
    let small = doc.append_element(Some(flex), "div", Style::default(), Some("font-size:10px"));
    doc.append_text(small, "aa");
    let large = doc.append_element(Some(flex), "div", Style::default(), Some("font-size:20px"));
    doc.append_text(large, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[small].is_ifc_root() && doc.nodes[large].is_ifc_root());
    (
        doc.nodes[small].unrounded_layout.location.y,
        doc.nodes[large].unrounded_layout.location.y,
    )
}

#[test]
fn a_flex_item_baseline_matches() {
    let on = flex_baseline_item_ys();
    assert_eq!(on, (8.0, 0.0));
    // Hand-computed: the 20px item's ascent is 16 and the 10px item's is 8,
    // so the small item sits 8px lower.
    assert_eq!(on, (8.0, 0.0));
}

#[test]
fn an_empty_inline_block_keeps_its_margin_box_baseline() {
    // "x" then an empty 20x20 inline-block: without lines it sits on the
    // baseline with its bottom margin edge, so the line baseline is 20px down
    // and the strut descent makes the line 22px tall.
    {
        let (mut doc, cascade, root, inline_block) = paragraph_with_inline_block("", 20.0);
        doc.set_element_inline_style(
            inline_block,
            Some("display:inline-block;width:20px;height:20px".into()),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade_again = raikiri_style::cascade(&doc, &rules).expect("cascade");
        let _ = cascade;
        lay_out(&mut doc, &cascade_again);
        assert!(!doc.nodes[inline_block].is_ifc_root());
        assert_eq!(doc.nodes[inline_block].unrounded_layout.location.y, 0.0);
        assert_eq!(doc.nodes[root].unrounded_layout.size.height, 22.0);
    }
}

#[test]
fn bare_text_next_to_an_element_item_gets_its_own_root() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flex;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let bare = doc.append_text(flex, "aa");
    let item = doc.append_element(Some(flex), "div", Style::default(), Some("display:block"));
    doc.append_text(item, "bbbb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(!doc.nodes[flex].is_ifc_root());
    assert!(doc.nodes[item].is_ifc_root());
    // The bare text is an anonymous item and the root of its own paragraph.
    assert!(doc.nodes[bare].is_ifc_root());
    // Hand-computed: the item follows the 20px anonymous item.
    assert_eq!(doc.nodes[item].unrounded_layout.location.x, 20.0);
    assert_eq!(doc.nodes[item].unrounded_layout.size.width, 40.0);
}

// ── text node roots ─────────────────────────────────────────

use crate::layout::test_support::ahem_paragraph_in_text_only;

/// Border-box size of the bare text of [`ahem_paragraph_in_text_only`].
fn bare_text_size(parent_css: &str) -> (f32, f32) {
    let (mut doc, cascade, parent) = ahem_paragraph_in_text_only(parent_css, "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[parent].children[0];
    let layout = doc.nodes[text].unrounded_layout;
    (layout.size.width, layout.size.height)
}

#[test]
fn bare_text_in_a_flex_container_becomes_a_root() {
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    assert!(doc.nodes[text].is_ifc_root());
}

#[test]
fn bare_text_in_a_flex_container_has_its_pinned_size() {
    let mut expected_4025 = [(90.0, 10.0), (800.0, 10.0), (800.0, 10.0)].into_iter();
    for parent_css in [
        "display:flex",
        "display:flex;flex-direction:column",
        "display:grid",
    ] {
        assert_eq!(
            bare_text_size(parent_css),
            expected_4025.next().expect("a value per case"),
            "{parent_css}"
        );
    }
    // Hand-computed: a flex row item shrinks to "aaaa bbbb" (90px), one line;
    // in a 50px container the two words wrap.
    assert_eq!(bare_text_size("display:flex"), (90.0, 10.0));
    assert_eq!(bare_text_size("display:flex;width:50px"), (50.0, 20.0));
}

#[test]
fn a_text_node_root_answers_the_line_and_baseline_readers() {
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    let lines = doc.ifc_text_lines(text).expect("lines");
    assert_eq!(lines.root, text);
    assert_eq!(lines.lines.len(), 1);
    // The baseline reader is the root's own measurement, which a flex
    // container aligns by (see a_bare_text_root_is_aligned_by_its_baseline).
}

/// `location.y` of a bare-text anonymous item (Ahem 10px) and an element item
/// (Ahem 20px) of a flex row aligned by their baselines, line height 1.
fn bare_text_baseline_item_ys() -> (f32, f32) {
    let (mut doc, _, flex) =
        ahem_paragraph_in_text_only("display:flex;align-items:baseline;line-height:1", "aa");
    let large = doc.append_element(Some(flex), "div", Style::default(), Some("font-size:20px"));
    doc.append_text(large, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    assert!(doc.nodes[text].is_ifc_root() && doc.nodes[large].is_ifc_root());
    (
        doc.nodes[text].unrounded_layout.location.y,
        doc.nodes[large].unrounded_layout.location.y,
    )
}

#[test]
fn a_bare_text_root_is_aligned_by_its_baseline() {
    let on = bare_text_baseline_item_ys();
    assert_eq!(on, (8.0, 0.0));
    // Hand-computed: ascents 8 (10px) and 16 (20px), so the text sits 8px lower.
    assert_eq!(on, (8.0, 0.0));
}

#[test]
fn whitespace_only_text_in_a_flex_container_is_not_a_root() {
    // Collapsible white space makes no anonymous item (CSS Flexbox 1, 4).
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", " \n\t ");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    assert!(!doc.nodes[text].is_ifc_root());
}

#[test]
fn a_flex_row_places_bare_text_after_its_sibling_item() {
    let (mut doc, cascade, flex) = ahem_paragraph_in_text_only("display:flex", "");
    let item = doc.append_element(
        Some(flex),
        "div",
        Style::default(),
        Some("display:block;width:30px;height:10px"),
    );
    let text = doc.append_text(flex, "aaaa");
    let _ = item;
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_again = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let _ = cascade;
    lay_out(&mut doc, &cascade_again);
    assert!(doc.nodes[text].is_ifc_root());
    let layout = doc.nodes[text].unrounded_layout;
    // Hand-computed: the 30px item first, then the 40px anonymous item.
    assert_eq!((layout.location.x, layout.size.width), (30.0, 40.0));
}

#[test]
fn bare_text_takes_the_text_align_of_its_container() {
    // The anonymous item inherits `text-align` from the container: in a 50px
    // flex row, "aaaa" (40px) is centered in the 50px item.
    let (mut doc, cascade, flex) =
        ahem_paragraph_in_text_only("display:flex;width:50px;text-align:center", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    let text = doc.nodes[flex].children[0];
    let lines = &stored_lines(&doc, text).lines;
    assert_eq!(line_start_x(&lines[0]), Some(5.0));
}

#[test]
fn a_lone_empty_inline_block_is_indented_once() {
    // The paragraph's only content is an empty inline-block: the engine
    // places it after the 20px indent, and nothing moves it again.
    let (mut doc, cascade, root) = ahem_paragraph_in("", "width:200px;text-indent:20px", "");
    let span = doc.append_element(
        Some(root),
        "span",
        Style::default(),
        Some("display:inline-block;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade_again = raikiri_style::cascade(&doc, &rules).expect("cascade");
    let _ = cascade;
    lay_out(&mut doc, &cascade_again);
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[span].unrounded_layout.location.x, 20.0);
}

#[test]
fn an_inline_block_root_with_side_margins_shrinks_to_its_content() {
    // A 200px paragraph holding an inline-block with 20px side margins: the
    // margin box may take 200px, so "aaaa bbbb cccc" (140px) fits on one
    // line and the inline-block is 140px wide. The margins come off the
    // available width once.
    {
        let (mut doc, cascade, root) = ahem_paragraph_in("", "width:200px", "");
        let inline_block = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline-block;margin:0 20px"),
        );
        doc.append_text(inline_block, "aaaa bbbb cccc");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade_again = raikiri_style::cascade(&doc, &rules).expect("cascade");
        let _ = cascade;
        lay_out(&mut doc, &cascade_again);
        assert!(doc.nodes[inline_block].is_ifc_root());
        let size = doc.nodes[inline_block].unrounded_layout.size;
        assert_eq!((size.width, size.height), (140.0, 10.0));
    }
}

#[test]
fn an_inline_block_root_under_an_authored_width_shrinks_to_its_content() {
    // "aa" then an inline-block "bb" in a 200px paragraph. The inline-block's
    // width is auto, so it shrinks to fit (CSS 2.1 10.3.9): 20px, beside
    // "aa" on the first line. It does not take the paragraph's 200px.
    let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aa");
    let inline_block = doc.append_element(
        Some(root),
        "span",
        Style::default(),
        Some("display:inline-block"),
    );
    doc.append_text(inline_block, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[inline_block].is_ifc_root());
    let layout = doc.nodes[inline_block].unrounded_layout;
    assert_eq!(
        (
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height
        ),
        (20.0, 0.0, 20.0, 10.0)
    );
}

// ── table cells ──────────────────────────────────────────────

/// Border-box sizes of the table and the cell of [`ahem_table`].
#[derive(Debug, PartialEq)]
struct TableSizes {
    table: (f32, f32),
    cell: (f32, f32),
}

fn table_sizes(table_css: &str, cell_css: &str, text: &str) -> TableSizes {
    let (mut doc, cascade, [table, row, cell]) =
        crate::layout::test_support::ahem_table(table_css, cell_css, |doc, cell| {
            doc.append_text(cell, text);
        });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[cell].is_ifc_root(), "{cell_css}");
    let size = |id: usize| {
        let s = doc.nodes[id].unrounded_layout.size;
        (s.width, s.height)
    };
    // The table algorithm lays out cells, not rows: a row keeps no layout of
    // its own.
    let _ = row;
    TableSizes {
        table: size(table),
        cell: size(cell),
    }
}

#[test]
fn a_table_cell_becomes_an_ifc_root() {
    let (mut doc, cascade, [table, row, cell]) =
        crate::layout::test_support::ahem_table("", "", |doc, cell| {
            doc.append_text(cell, "aaaa bbbb");
        });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[cell].is_ifc_root());
    // The table and the row hold rows and cells, not inline content: they
    // are not candidates at all (not refused roots).
    assert!(!doc.nodes[table].is_ifc_root());
    assert!(!doc.nodes[row].is_ifc_root());
    assert!(!crate::layout::ifc::assign::can_be_ifc_root(
        &doc, &cascade, table
    ));
    assert!(!crate::layout::ifc::assign::can_be_ifc_root(
        &doc, &cascade, row
    ));
}

#[test]
fn a_table_cell_is_laid_out() {
    let mut expected_4254 = [
        TableSizes {
            table: (90.0, 10.0),
            cell: (90.0, 10.0),
        },
        TableSizes {
            table: (50.0, 20.0),
            cell: (50.0, 20.0),
        },
    ]
    .into_iter();
    for cell_css in ["", "width:50px"] {
        assert_eq!(
            table_sizes("", cell_css, "aaaa bbbb"),
            expected_4254.next().expect("a value per case"),
            "{cell_css}"
        );
    }
    // Hand-computed, not an oracle: a one-cell table is as wide as
    // "aaaa bbbb" (90px) and one 10px line tall.
    assert_eq!(table_sizes("", "", "aaaa bbbb").cell, (90.0, 10.0));
    // A 50px cell breaks it into two 40px words on two lines.
    assert_eq!(
        table_sizes("", "width:50px", "aaaa bbbb").cell,
        (50.0, 20.0)
    );
}

#[test]
fn an_all_inline_table_box_shapes_its_anonymous_cell() {
    // The table algorithm sizes its anonymous row/cell; the cell shapes text.
    let (mut doc, cascade, table) =
        crate::layout::test_support::ahem_table_with_only_text("", "aaaa bbbb");
    lay_out(&mut doc, &cascade);
    assert!(!doc.nodes[table].is_ifc_root());
    let cells: Vec<_> = doc.anonymous_table_cells(table).collect();
    assert_eq!(cells.len(), 1);
    assert!(cells[0].1.is_ifc_root());
    let size = doc.nodes[table].unrounded_layout.size;
    // Hand-computed: the table shrinks to "aaaa bbbb", one 10px line.
    assert_eq!((size.width, size.height), (90.0, 10.0));
}

#[test]
fn a_table_cell_keeps_its_padding_and_border() {
    // A 40px content box breaks "aaaa bbbb" into two 10px lines; 5px of
    // padding and a 3px border on each side make a 56x36 cell.
    let css = "width:40px;padding:5px;border:3px solid";
    let on = table_sizes("", css, "aaaa bbbb");
    assert_eq!(on.cell, (56.0, 36.0));
    assert_eq!(
        on,
        TableSizes {
            table: (56.0, 36.0),
            cell: (56.0, 36.0)
        }
    );
    let (mut doc, cascade, [_, _, cell]) =
        crate::layout::test_support::ahem_table("", css, |doc, cell| {
            doc.append_text(cell, "aaaa bbbb");
        });
    lay_out(&mut doc, &cascade);
    assert_eq!(stored_lines(&doc, cell).lines.len(), 2);
}

#[test]
fn a_table_cell_with_mixed_block_and_inline_content_is_laid_out() {
    // "aaaa" then a block of "bbbb": the cell is a root with the block as a
    // box of its paragraph, and the block is the root of its own text.
    let sizes = || {
        let (mut doc, cascade, [table, _, cell]) =
            crate::layout::test_support::ahem_table("", "", |doc, cell| {
                doc.append_text(cell, "aaaa");
                let block =
                    doc.append_element(Some(cell), "div", Style::default(), Some("display:block"));
                doc.append_text(block, "bbbb");
            });
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[cell].is_ifc_root());
        let size = |id: usize| {
            let s = doc.nodes[id].unrounded_layout.size;
            (s.width, s.height)
        };
        (size(table), size(cell))
    };
    let on = sizes();
    assert_eq!(on, ((40.0, 20.0), (40.0, 20.0)));
    // Hand-computed: two 40px lines, one above the block and one in it.
    assert_eq!(on.1, (40.0, 20.0));
}

#[test]
fn bare_text_directly_in_a_table_is_laid_out() {
    // Text straight in a table box (no row or cell): one anonymous cell's
    // content. The height is pinned.
    let mut expected_4339 = [10.0, 20.0].into_iter();
    for (text, lines) in [("aaaa bbbb", 1.0), ("aaaa\nbbbb", 2.0)] {
        let height = || {
            let (mut doc, cascade, table) = crate::layout::test_support::ahem_table_with_only_text(
                "white-space:pre-line",
                text,
            );
            lay_out(&mut doc, &cascade);
            assert!(!doc.nodes[table].is_ifc_root());
            let cells: Vec<_> = doc.anonymous_table_cells(table).collect();
            assert_eq!(cells.len(), 1);
            assert!(cells[0].1.is_ifc_root());
            doc.nodes[table].unrounded_layout.size.height
        };
        assert_eq!(height(), 10.0 * lines, "{text:?}");
        assert_eq!(
            height(),
            expected_4339.next().expect("a value per case"),
            "{text:?}"
        );
    }
}

// ── vertical margins of block children ───────────────────────

/// `root > ["aaaa", div(a_css) > "bb", div(b_css) > "cc", "dddd"]` in a
/// 200px Ahem paragraph. Returns the document, the cascade, the root and
/// the two blocks.
fn paragraph_with_block_children(
    a_css: &str,
    b_css: &str,
) -> (Document, CascadeResult, usize, [usize; 2]) {
    let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aaaa");
    let a = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some(&format!("display:block;{a_css}")),
    );
    doc.append_text(a, "bb");
    let b = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some(&format!("display:block;{b_css}")),
    );
    doc.append_text(b, "cc");
    doc.append_text(root, "dddd");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root, [a, b])
}

/// Distance from the bottom of the first block child to the top of the
/// second, in a paragraph of [`paragraph_with_block_children`].
fn block_child_gap(a_css: &str, b_css: &str) -> f32 {
    let (mut doc, cascade, root, [a, b]) = paragraph_with_block_children(a_css, b_css);
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{a_css} / {b_css}");
    let a = doc.nodes[a].unrounded_layout;
    let b = doc.nodes[b].unrounded_layout;
    b.location.y - (a.location.y + a.size.height)
}

#[test]
fn block_child_margins_keep_the_root_an_ifc_root() {
    let (mut doc, cascade, root, _) =
        paragraph_with_block_children("margin-bottom:20px", "margin-top:30px");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn adjoining_block_children_collapse_their_margins() {
    assert_eq!(
        block_child_gap("margin-bottom:20px", "margin-top:30px"),
        30.0
    );
    assert_eq!(30.0, 30.0);
}

#[test]
fn negative_and_positive_margins_collapse_by_summing_the_extremes() {
    // CSS 2.1 8.3.1: the largest positive (20) plus the most negative (-5).
    assert_eq!(
        block_child_gap("margin-bottom:20px", "margin-top:-5px"),
        15.0
    );
    assert_eq!(
        block_child_gap("margin-bottom:-8px", "margin-top:-5px"),
        -8.0
    );
}

#[test]
fn a_block_childs_inner_margin_collapses_through_it() {
    // A's last child "pp" has a 20px bottom margin. A has no padding or
    // border, so that margin leaves A and collapses with B's 30px top
    // margin: 30px between A's bottom and B's top.
    let gap = || {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aaaa");
        let a = doc.append_element(Some(root), "div", Style::default(), Some("display:block"));
        let p = doc.append_element(
            Some(a),
            "div",
            Style::default(),
            Some("display:block;margin-bottom:20px"),
        );
        doc.append_text(p, "pp");
        let b = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;margin-top:30px"),
        );
        doc.append_text(b, "cc");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        let a = doc.nodes[a].unrounded_layout;
        let b = doc.nodes[b].unrounded_layout;
        (a.size.height, b.location.y - (a.location.y + a.size.height))
    };
    assert_eq!(gap(), (10.0, 30.0));
    assert_eq!(gap(), (10.0, 30.0));
}

/// Distance from the bottom of the line "aaaa" to the top of a block child
/// with `css` that follows it.
fn gap_between_line_and_block(css: &str) -> f32 {
    let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aaaa");
    let block = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some(&format!("display:block;{css}")),
    );
    doc.append_text(block, "bb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{css}");
    doc.nodes[block].unrounded_layout.location.y - 10.0
}

#[test]
fn a_margin_does_not_collapse_with_a_line() {
    assert_eq!(gap_between_line_and_block("margin-top:12px"), 12.0);
    assert_eq!(gap_between_line_and_block("margin-top:12px"), 12.0);
}

/// `root > [lead, div(a_css) > "bb", div(e_css) (empty), div(b_css) > "cc"]`
/// in a 200px Ahem paragraph; `lead` is text before the first block (may be
/// empty). Returns the root, the three blocks and their border boxes as
/// `(y, height)`, and the root's `(y, height)`.
fn three_blocks(
    lead: &str,
    a_css: &str,
    e_css: &str,
    b_css: &str,
) -> ((f32, f32), [(f32, f32); 3]) {
    let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", lead);
    let mut blocks = [0; 3];
    for (slot, (css, text)) in [(a_css, "bb"), (e_css, ""), (b_css, "cc")]
        .into_iter()
        .enumerate()
    {
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;{css}")),
        );
        if !text.is_empty() {
            doc.append_text(block, text);
        }
        blocks[slot] = block;
    }
    doc.append_text(root, "dddd");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{a_css} / {e_css} / {b_css}");
    let geometry = |id: usize| {
        let l = doc.nodes[id].unrounded_layout;
        (l.location.y, l.size.height)
    };
    (geometry(root), blocks.map(geometry))
}

#[test]
fn an_empty_block_child_collapses_through() {
    // 20 (A's bottom), 10 and 25 (the empty block) and 5 (B's top) are one
    // collapsed margin: 25px from A's bottom to B's top.
    let run = || {
        three_blocks(
            "aaaa",
            "margin-bottom:20px",
            "margin-top:10px;margin-bottom:25px",
            "margin-top:5px",
        )
    };
    let (_, [a, _, b]) = run();
    assert_eq!(b.0 - (a.0 + a.1), 25.0);
    assert_eq!(
        run(),
        ((0.0, 65.0), [(10.0, 10.0), (40.0, 0.0), (45.0, 10.0)])
    );
}

#[test]
fn a_block_child_with_padding_does_not_collapse_through() {
    // The 1px padding separates the empty block's margins: 20 above it
    // (max of 20 and 10), 1px of padding, 25 below it (max of 25 and 5).
    let run = || {
        three_blocks(
            "aaaa",
            "margin-bottom:20px",
            "margin-top:10px;margin-bottom:25px;padding-top:1px",
            "margin-top:5px",
        )
    };
    let (_, [a, e, b]) = run();
    assert_eq!(e.0 - (a.0 + a.1), 20.0);
    assert_eq!(b.0 - (a.0 + a.1), 46.0);
    assert_eq!(
        run(),
        ((0.0, 86.0), [(10.0, 10.0), (40.0, 1.0), (66.0, 10.0)])
    );
}

#[test]
fn margins_are_included_in_the_roots_height() {
    // Line "aaaa" (10) + A (10) + 20 + B (10) + 8 (B's bottom margin, then
    // the line "dddd") + "dddd" (10): the lines keep the margins in the root.
    let run = || three_blocks("aaaa", "margin-bottom:20px", "", "margin-bottom:8px");
    let (root, _) = run();
    assert_eq!(root.1, 68.0);
    assert_eq!(
        run(),
        ((0.0, 68.0), [(10.0, 10.0), (40.0, 0.0), (40.0, 10.0)])
    );
}

#[test]
fn the_first_block_childs_top_margin_stays_inside_the_root() {
    // The paragraph contains its first block child's top margin: the root
    // stays where it is and the block starts 15px into it.
    let run = || three_blocks("", "margin-top:15px", "", "");
    let (root, [a, _, _]) = run();
    assert_eq!((root.0, a.0), (0.0, 15.0));
    assert_eq!(
        run(),
        ((0.0, 45.0), [(15.0, 10.0), (25.0, 0.0), (25.0, 10.0)])
    );
}

#[test]
fn the_last_block_childs_bottom_margin_collapses_through_the_root() {
    // "aaaa" then a block with a 12px bottom margin and nothing after it.
    // The paragraph is in its parent's formatting context and has no bottom
    // padding or border, so the margin collapses through its bottom edge
    // (CSS 2.1 8.3.1): the paragraph is 20px tall, and the margin is the
    // root's to collapse with whatever follows it.
    let height = || {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aaaa");
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;margin-bottom:12px"),
        );
        doc.append_text(block, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        doc.nodes[root].unrounded_layout.size.height
    };
    assert_eq!(height(), 20.0);
    assert_eq!(height(), 20.0);
}

#[test]
fn a_formatting_context_root_contains_its_last_block_childs_bottom_margin() {
    // A flex item establishes a formatting context of its own: the 12px
    // bottom margin of its last block child stays inside it, 10 + 10 + 12.
    let height = || {
        let (mut doc, _cascade, root) = ahem_paragraph_in("display:flex", "", "aaaa");
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;margin-bottom:12px"),
        );
        doc.append_text(block, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        doc.nodes[root].unrounded_layout.size.height
    };
    assert_eq!(height(), 32.0);
    assert_eq!(height(), 32.0);
}

// ── engine-only mode and limits ──────────────────────────────

/// A paragraph of 200 bytes of text with the engine limited to 10 bytes.
fn over_the_text_limit() -> (Document, CascadeResult) {
    let (mut doc, cascade, _) = ahem_paragraph(&"a ".repeat(100), "");
    doc.set_font_collection_with_limits(
        ifc_ahem_fonts(),
        shodo::limits::Limits {
            max_text_bytes: Some(10),
            ..Default::default()
        },
    );
    (doc, cascade)
}

#[test]
fn a_limit_overflow_is_reported_as_an_error() {
    let (mut doc, cascade) = over_the_text_limit();
    let result = layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600());
    assert!(
        matches!(result, Err(LayoutError::IfcLimitExceeded { .. })),
        "{result:?}"
    );
}

#[test]
fn a_limit_overflow_is_an_error_whatever_the_document() {
    // A limit is not a refusal, and there is no other layout path to leave
    // it to, so it is always reported.
    let (mut doc, cascade) = over_the_text_limit();
    let result = layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600());
    assert!(
        matches!(result, Err(LayoutError::IfcLimitExceeded { .. })),
        "{result:?}"
    );
}

#[test]
fn every_authored_shape_lays_out_without_an_error() {
    // The shapes that were refused last (a fixed root, a block with a forced
    // page break, generated content, a positioned box) are all laid out in
    // engine-only mode; a refusal is now an internal inconsistency only (see
    // the assignment's unit tests).
    let (mut doc, cascade, root) = crate::layout::test_support::ahem_paragraph_with(
        "position:fixed;width:100px",
        |doc, root| {
            doc.append_text(root, "aa ");
            doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;break-before:page"),
            );
            doc.append_element(
                Some(root),
                "span",
                Style::default(),
                Some("position:absolute;width:10px;height:10px"),
            );
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let result = layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600());
    assert!(result.is_ok(), "{result:?}");
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn a_shaping_limit_overflow_is_an_error_on_either_build_path() {
    // The glyph budget is checked when a paragraph is shaped, after it is
    // projected: the error has to come out of the build step, sequential or
    // parallel.
    for parallel in [false, true] {
        let (mut doc, cascade, root) = ahem_paragraph(&"a ".repeat(100), "");
        doc.set_font_collection_with_limits(
            ifc_ahem_fonts(),
            shodo::limits::Limits {
                max_shaped_glyphs: Some(5),
                ..Default::default()
            },
        );
        doc.set_ifc_parallel_build(parallel);
        doc.set_ifc_parallel_threshold(1);
        let result = layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600());
        assert!(
            matches!(result, Err(LayoutError::IfcLimitExceeded { node, .. }) if node == root),
            "{parallel}: {result:?}"
        );
    }
}

// ── degraded projections ─────────────────────────────────────

/// `root > span(css) > text` in an Ahem paragraph (or `root(css) > text`
/// when `on_root`); returns the root's border-box size, asserting the root
/// is a root exactly when the engine is on.
fn degraded_size(css: &str, text: &str, on_root: bool) -> (f32, f32) {
    let (mut doc, _cascade, root) = ahem_paragraph_in(
        "",
        if on_root { css } else { "" },
        if on_root { text } else { "" },
    );
    if !on_root {
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(&format!("display:inline;{css}")),
        );
        doc.append_text(span, text);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{css}");
    let size = doc.nodes[root].unrounded_layout.size;
    (size.width, size.height)
}

#[test]
fn vertical_writing_mode_uses_the_max_content_inline_extent_when_height_is_auto() {
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb", "writing-mode:vertical-rl");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    let lines = stored_lines(&doc, root);
    assert_eq!(lines.width, 90.0);
    assert_eq!(lines.lines.len(), 1);
    assert!(
        lines
            .lines
            .iter()
            .all(|line| line.writing_mode() == shodo::geometry::WritingMode::VerticalRl)
    );
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 90.0);
}

#[test]
fn vertical_auto_inline_extent_includes_block_child_physical_height() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let (mut doc, _cascade, root) =
            ahem_paragraph("", &format!("width:100px;height:auto;writing-mode:{mode}"));
        let child = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;width:20px;height:80px;margin-top:3px;margin-bottom:7px"),
        );
        doc.append_text(child, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");

        lay_out(&mut doc, &cascade);

        assert!(doc.nodes[root].is_ifc_root(), "{mode}");
        assert_eq!(doc.nodes[root].unrounded_layout.size.height, 90.0, "{mode}");
    }
}

#[test]
fn degraded_forms_have_their_pinned_sizes() {
    // Each property is on an inline child, where the degradation applies
    // (on the root, taffy resolves the percentage and the property is moot).
    let mut expected_4772 = [(800.0, 10.0), (800.0, 10.0), (800.0, 10.0)].into_iter();
    for (css, text) in [
        ("padding-left:10%", "aaaa"),
        ("text-autospace:auto", "漢字abc"),
        ("text-autospace:normal", "漢字abc"),
    ] {
        assert_eq!(
            degraded_size(css, text, false),
            expected_4772.next().expect("a value per case"),
            "{css}"
        );
    }
}

/// `root > ["aaaa", input(css)]`; returns the input's x and the root's
/// height.
fn paragraph_with_input(css: &str) -> (f32, f32) {
    let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aaaa");
    let input = doc.append_element(Some(root), "input", Style::default(), Some(css));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{css}");
    (
        doc.nodes[input].unrounded_layout.location.x,
        doc.nodes[root].unrounded_layout.size.height,
    )
}

#[test]
fn a_form_control_in_a_paragraph_is_an_atomic_box() {
    // Hand-computed: "aaaa" followed by a 30x10 atomic, so the input starts
    // at x = 40 on a single line. The input has no baseline, so its margin
    // box's bottom sits on the line's baseline (CSS 2.1 10.8.1) and the
    // strut's 2px descent hangs below it: the line is 12px tall.
    assert_eq!(paragraph_with_input("width:30px;height:10px"), (40.0, 12.0));
}

#[test]
fn vertical_align_middle_centres_the_box_on_half_the_parents_x_height() {
    // CSS 2.1 10.8.1: the box's midpoint (3px above its baseline: Ahem 10px
    // has an 8px ascent and a 2px descent) is aligned with the baseline plus
    // half the parent's x-height (Ahem's x-height is 8px, so 4px): the span
    // is raised 1px and the line is 11px tall.
    assert_eq!(
        degraded_size("vertical-align:middle", "aaaa", false).1,
        11.0
    );
}

#[test]
fn a_block_child_with_auto_side_margins_is_centred() {
    // CSS 2.1 10.3.3: a 100px block in a 200px paragraph with both side
    // margins auto takes 50px on each side; with only the left one auto it
    // goes to the right edge.
    let x = |css: &str| {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aaaa");
        let block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;width:100px;{css}")),
        );
        doc.append_text(block, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        doc.nodes[block].unrounded_layout.location.x
    };
    assert_eq!(x("margin:0 auto"), 50.0);
    assert_eq!(x("margin-left:auto"), 100.0);
    assert_eq!(x("margin-left:auto;margin-right:20px"), 80.0);
    let mut expected_4852 = [50.0, 100.0, 80.0].into_iter();
    for css in [
        "margin:0 auto",
        "margin-left:auto",
        "margin-left:auto;margin-right:20px",
    ] {
        assert_eq!(
            x(css),
            expected_4852.next().expect("a value per case"),
            "{css}"
        );
    }
}

#[test]
fn an_inline_table_sits_on_the_line_like_an_inline_block() {
    // "aa" then an inline-table holding a cell with "bb": a 20px atomic at
    // x = 20 on the first line (CSS 2.1 17.4: an inline-level table).
    let place = || {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aa");
        let table = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline-table"),
        );
        let row = doc.append_element(
            Some(table),
            "span",
            Style::default(),
            Some("display:table-row"),
        );
        let cell = doc.append_element(
            Some(row),
            "span",
            Style::default(),
            Some("display:table-cell"),
        );
        doc.append_text(cell, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        let l = doc.nodes[table].unrounded_layout;
        (l.location.x, l.size.width, l.size.height)
    };
    assert_eq!(place(), (20.0, 20.0, 10.0));
}

#[test]
fn a_relatively_positioned_inline_block_is_offset_from_its_place_on_the_line() {
    // "aa" then a 10x10 inline-block moved by left:5px and top:3px: it keeps
    // its place on the line (x = 20) and is drawn 5px right and 3px down
    // (CSS 2.1 9.4.3).
    let place = |css: &str| {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aa");
        let block = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(&format!(
                "display:inline-block;width:10px;height:10px;{css}"
            )),
        );
        doc.append_text(root, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{css}");
        let l = doc.nodes[block].unrounded_layout.location;
        (l.x, l.y)
    };
    let still = place("");
    assert_eq!(still.0, 20.0);
    let moved = place("position:relative;left:5px;top:3px");
    assert_eq!(moved, (still.0 + 5.0, still.1 + 3.0));
    assert_eq!(moved, (25.0, 3.0));
    let moved = place("position:relative;right:5px;bottom:3px");
    assert_eq!(moved, (still.0 - 5.0, still.1 - 3.0));
}

#[test]
fn a_relatively_positioned_float_is_placed_like_a_float() {
    // A 10x10 left float with left:5px: placed at the line start like any
    // float; the offset is applied by the painter.
    let place = || {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "aa");
        let float = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:block;float:left;width:10px;height:10px;position:relative;left:5px"),
        );
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        let l = doc.nodes[float].unrounded_layout.location;
        (l.x, l.y)
    };
    // CSS 2.1 9.5.1 lets the float share the line of "aa" when it fits.
    assert_eq!(place(), (0.0, 0.0));
}

#[test]
fn an_atomic_in_a_right_to_left_paragraph_is_placed_from_the_right_edge() {
    // A 200px right-to-left paragraph "aa", a 10x10 inline-block, "bb": the
    // line runs from the right edge, so "aa" takes 180..200 and the box sits
    // at 170..180.
    let place = || {
        let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px;direction:rtl", "aa");
        let block = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline-block;width:10px;height:10px"),
        );
        doc.append_text(root, "bb");
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        doc.nodes[block].unrounded_layout.location.x
    };
    assert_eq!(place(), 170.0);
    assert_eq!(place(), 170.0);
}

/// The root ("aaaa" then a block with a 12px bottom margin) with `root_css`,
/// and a sibling after it with a 3px top margin: the root's height and the
/// sibling's y.
fn root_then_sibling(root_css: &str) -> (f32, f32) {
    let (mut doc, _cascade, root) = ahem_paragraph_in("", root_css, "aaaa");
    let block = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some("display:block;margin-bottom:12px"),
    );
    doc.append_text(block, "bb");
    let parent = doc.parent_of(root).expect("parent");
    let sibling = doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("display:block;margin-top:3px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{root_css}");
    (
        doc.nodes[root].unrounded_layout.size.height,
        doc.nodes[sibling].unrounded_layout.location.y,
    )
}

#[test]
fn an_escaping_bottom_margin_collapses_with_the_next_sibling() {
    // The 12px margin leaves the 20px root and collapses with the sibling's
    // 3px: the sibling starts at 20 + 12.
    assert_eq!(root_then_sibling(""), (20.0, 32.0));
    assert_eq!(root_then_sibling(""), (20.0, 32.0));
}

#[test]
fn a_min_height_keeps_the_last_childs_margin_inside_the_root() {
    // CSS 2.1 8.3.1: when min-height sets the used height the last child's
    // bottom margin does not collapse through the root; only the sibling's
    // own 3px separates them.
    let mut expected_5029 = [(20.0, 23.0), (25.0, 28.0), (50.0, 53.0)].into_iter();
    for (css, height) in [
        ("min-height:20px", 20.0),
        ("min-height:25px", 25.0),
        ("min-height:50px", 50.0),
    ] {
        assert_eq!(root_then_sibling(css), (height, height + 3.0), "{css}");
        assert_eq!(
            root_then_sibling(css),
            expected_5029.next().expect("a value per case"),
            "{css}"
        );
    }
}

/// `aa <span>bb</span>` in an Ahem root with a 10px line height, `root_css`
/// on the root and `sheet` as the author style sheet.
fn generated_paragraph(sheet: &str, root_css: &str) -> (Document, CascadeResult, usize) {
    let crate::layout::ifc::test_support::Fixture { doc, cascade, root } =
        crate::layout::ifc::test_support::sheet_fixture(
            sheet,
            &format!("line-height:10px;{root_css}"),
            |doc, root| {
                doc.append_text(root, "aa ");
                let span = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
                doc.set_element_attribute(span, "data-k", "zz")
                    .expect("attribute");
                doc.append_text(span, "bb");
            },
        );
    (doc, cascade, root)
}

/// The text of every stored line of the engine root of `generated_paragraph`,
/// and the root's height.
fn generated_lines(sheet: &str, root_css: &str) -> (Vec<String>, f32) {
    let (mut doc, cascade, root) = generated_paragraph(sheet, root_css);
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{sheet}");
    let lines = stored_lines(&doc, root)
        .lines
        .iter()
        .map(line_text)
        .collect();
    (lines, doc.nodes[root].unrounded_layout.size.height)
}

#[test]
fn generated_text_of_an_inline_takes_room_on_its_line() {
    // Hand-computed: "aa " is 30px, then the span's "x" and "bb" make "xbb"
    // (30px), which does not fit the 20px left of a 50px line.
    assert_eq!(
        generated_lines(r#"span::before { content: "x" }"#, "width:50px"),
        (vec!["aa".to_owned(), "xbb".to_owned()], 20.0)
    );
    // `::after` closes the span's content.
    assert_eq!(
        generated_lines(r#"span::after { content: "y" }"#, "width:200px").0,
        ["aa bby"]
    );
}

#[test]
fn generated_text_of_the_root_starts_and_ends_its_lines() {
    assert_eq!(
        generated_lines(
            r#"div::before { content: "x" } div::after { content: "y" }"#,
            "width:200px"
        ),
        (vec!["xaa bby".to_owned()], 10.0)
    );
}

#[test]
fn counters_and_attributes_resolve_in_generated_text() {
    // `counter-reset: n 6` on the root; the span's `::before` reads it and the
    // span's `data-k` attribute.
    assert_eq!(
        generated_lines(
            r#"div { counter-reset: n 6 } span::before { content: counter(n) attr(data-k) }"#,
            "width:200px"
        )
        .0,
        ["aa 6zzbb"]
    );
}

#[test]
fn generated_images_and_out_of_flow_generated_boxes_are_not_in_the_lines() {
    for sheet in [
        r#"span::before { content: url(missing.png) }"#,
        r#"span::before { content: "x"; position: absolute }"#,
        r#"span::before { content: "x"; float: left }"#,
        r#"span::before { content: none }"#,
    ] {
        assert_eq!(
            generated_lines(sheet, "width:200px").0,
            ["aa bb"],
            "{sheet}"
        );
    }
}

#[test]
fn block_level_generated_text_gets_a_line_of_its_own() {
    assert_eq!(
        generated_lines(
            r#"div::before { content: "x"; display: block }"#,
            "width:200px"
        ),
        (vec!["x".to_owned(), "aa bb".to_owned()], 20.0)
    );
    assert_eq!(
        generated_lines(
            r#"div::after { content: "y"; display: block }"#,
            "width:200px"
        ),
        (vec!["aa bb".to_owned(), "y".to_owned()], 20.0)
    );
}

#[test]
fn an_element_with_only_generated_text_is_a_one_line_root() {
    let crate::layout::ifc::test_support::Fixture {
        mut doc,
        cascade,
        root,
    } = crate::layout::ifc::test_support::sheet_fixture(
        r#"div::before { content: "x" }"#,
        "line-height:10px",
        |_, _| {},
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 10.0);
}

#[test]
fn generated_content_is_laid_out_by_the_engine() {
    let (mut doc, cascade, root) = generated_paragraph(
        r#"div::before { content: "x" } span::before { content: url(missing.png) "w" }"#,
        "width:200px",
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(line_text(&stored_lines(&doc, root).lines[0]), "xaa wbb");
}

/// Where the lines of a paragraph split across columns are drawn.
#[derive(Debug, PartialEq)]
struct MulticolTextGeometry {
    /// Border-box height of the multicol container.
    container_height: f32,
    /// Start of each line, `(x, y)` from the paragraph's content-box corner,
    /// after the column offsets.
    lines: Vec<(f32, f32)>,
}

/// The engine side of [`multicol_text_geometry`]: the stored lines of `root`
/// moved by its column fragments.
fn engine_multicol_lines(doc: &Document, root: usize) -> Vec<(f32, f32)> {
    let ifc = doc.nodes[root].ifc.as_ref().expect("an ifc root");
    let lines = &ifc.lines.as_ref().expect("performed lines").lines;
    let fragments = ifc.multicol_fragments.clone().unwrap_or_else(|| {
        vec![crate::node::MulticolTextFragment {
            line_start: 0,
            line_end: lines.len(),
            fragmentainer: 0,
            x: 0.0,
            y: lines.first().map_or(0.0, |line| line.block_offset()),
        }]
    });
    let mut out = Vec::new();
    for fragment in fragments {
        let first = lines[fragment.line_start].block_offset();
        for line in &lines[fragment.line_start..fragment.line_end] {
            out.push((fragment.x, fragment.y - first + line.block_offset()));
        }
    }
    out
}

/// A paragraph under a multicol parent (`parent_css`) or, with `own`, a
/// multicol container that holds the text itself: its column geometry on
/// either path.
fn multicol_text_geometry_of(parent_css: &str, text: &str, own: bool) -> MulticolTextGeometry {
    let (mut doc, cascade, root) = if own {
        ahem_paragraph(text, parent_css)
    } else {
        ahem_paragraph_in(parent_css, "", text)
    };
    let container = if own {
        root
    } else {
        doc.parent_of(root).expect("parent")
    };
    lay_out(&mut doc, &cascade);
    assert!(
        doc.nodes[root].is_ifc_root(),
        "{parent_css}: not an ifc root"
    );
    let lines = engine_multicol_lines(&doc, root);
    MulticolTextGeometry {
        container_height: doc.nodes[container].unrounded_layout.size.height,
        lines,
    }
}

fn multicol_text_geometry(parent_css: &str, text: &str) -> MulticolTextGeometry {
    multicol_text_geometry_of(parent_css, text, false)
}

#[test]
fn balanced_multicol_text_fills_each_column_to_the_minimum_height() {
    let text = std::iter::repeat_n("x xx xxx xxxx xxxxx", 11)
        .collect::<Vec<_>>()
        .join(" ");
    let (mut doc, cascade, root) = ahem_paragraph(
        &text,
        "width:600px;column-count:6;column-gap:0;font:20px/1 Ahem",
    );
    lay_out(&mut doc, &cascade);
    let ifc = doc.nodes[root].ifc.as_ref().unwrap();
    let ranges = ifc
        .multicol_fragments
        .as_ref()
        .unwrap()
        .iter()
        .map(|fragment| (fragment.line_start, fragment.line_end))
        .collect::<Vec<_>>();
    assert_eq!(
        ranges,
        vec![(0, 8), (8, 16), (16, 24), (24, 32), (32, 40), (40, 44)]
    );
    assert_eq!(doc.nodes[root].unrounded_layout.size.height, 160.0);
}

#[test]
fn a_paragraph_in_a_multicol_container_becomes_a_root() {
    let (mut doc, cascade, p) =
        ahem_paragraph_in("column-count:2;width:100px", "", "aaaa bbbb cccc dddd");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[p].is_ifc_root());
}

#[test]
fn a_multicol_paragraph_is_split() {
    let mut expected_5265 = [
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![(0.0, 0.0), (0.0, 10.0), (55.0, 0.0), (55.0, 10.0)],
        },
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![
                (0.0, 0.0),
                (0.0, 10.0),
                (50.0, 0.0),
                (50.0, 10.0),
                (100.0, 0.0),
                (100.0, 10.0),
            ],
        },
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![(0.0, 0.0), (0.0, 10.0), (55.0, 0.0), (55.0, 10.0)],
        },
    ]
    .into_iter();
    for (parent_css, text) in [
        (
            "column-count:2;width:100px;column-gap:10px",
            "aaaa bbbb cccc dddd",
        ),
        (
            "column-count:3;width:150px;column-gap:0",
            "aaaa bbbb cccc dddd eeee ffff",
        ),
        (
            "column-width:40px;width:100px;column-gap:10px",
            "aaaa bbbb cccc dddd",
        ),
    ] {
        assert_eq!(
            multicol_text_geometry(parent_css, text),
            expected_5265.next().expect("a value per case"),
            "{parent_css}"
        );
    }
    // Hand-computed, not an oracle: two 45px columns with a 10px gap; each
    // 40px word takes a line of its own, two lines per column, and the
    // second column's lines start at (55, 0). The container is two lines tall.
    let geometry = multicol_text_geometry(
        "column-count:2;width:100px;column-gap:10px",
        "aaaa bbbb cccc dddd",
    );
    assert_eq!(geometry.lines[2].0, 55.0);
    assert_eq!(geometry.lines.len(), 4);
}

#[test]
fn multicol_container_with_direct_text_is_laid_out() {
    let mut expected_5299 = [
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![(0.0, 0.0), (0.0, 10.0), (55.0, 0.0), (55.0, 10.0)],
        },
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![
                (0.0, 0.0),
                (0.0, 10.0),
                (50.0, 0.0),
                (50.0, 10.0),
                (100.0, 0.0),
                (100.0, 10.0),
            ],
        },
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![(0.0, 0.0), (0.0, 10.0), (55.0, 0.0), (55.0, 10.0)],
        },
    ]
    .into_iter();
    for (css, text) in [
        (
            "column-count:2;width:100px;column-gap:10px",
            "aaaa bbbb cccc dddd",
        ),
        (
            "column-count:3;width:150px;column-gap:0",
            "aaaa bbbb cccc dddd eeee ffff",
        ),
        (
            "column-width:40px;width:100px;column-gap:10px",
            "aaaa bbbb cccc dddd",
        ),
    ] {
        assert_eq!(
            multicol_text_geometry_of(css, text, true),
            expected_5299.next().expect("a value per case"),
            "{css}"
        );
    }
    // Hand-computed: four one-word lines balanced over two 45px columns: the
    // container is two lines tall and the third line starts the second
    // column at (55, 0).
    let geometry = multicol_text_geometry_of(
        "column-count:2;width:100px;column-gap:10px",
        "aaaa bbbb cccc dddd",
        true,
    );
    assert_eq!(geometry.container_height, 20.0);
    assert_eq!(
        geometry.lines,
        [(0.0, 0.0), (0.0, 10.0), (55.0, 0.0), (55.0, 10.0)]
    );
}

#[test]
fn auto_fill_keeps_auto_height_direct_text_in_source_order() {
    let geometry = multicol_text_geometry_of(
        "column-count:2;width:100px;column-gap:10px;font:10px/10px Ahem;white-space:pre-line;column-fill:auto",
        "aa\nbb\ncc",
        true,
    );

    assert_eq!(
        geometry,
        MulticolTextGeometry {
            container_height: 30.0,
            lines: vec![(0.0, 0.0), (0.0, 10.0), (0.0, 20.0)],
        }
    );
}

#[test]
fn a_definite_height_multicol_fills_its_columns_in_turn_and_keeps_widows() {
    // Hand-computed: a 20px-tall content box holds two 10px lines per
    // column; five lines fill 2 + 2 + 1, and `widows: 2` moves one line of
    // the second column into the third.
    let geometry = multicol_text_geometry_of(
        "column-count:3;width:150px;column-gap:0;height:20px;widows:2;orphans:1",
        "aaaa bbbb cccc dddd eeee",
        true,
    );
    assert_eq!(
        geometry.lines,
        [
            (0.0, 0.0),
            (0.0, 10.0),
            (50.0, 0.0),
            (100.0, 0.0),
            (100.0, 10.0)
        ]
    );
}

#[test]
fn multicol_with_a_forced_break_is_laid_out() {
    // Under a multicol parent, one text node with preserved newlines.
    let css = "column-count:2;width:100px;column-gap:10px;white-space:pre-line";
    assert_eq!(
        multicol_text_geometry(css, "aa\nbb\ncc"),
        MulticolTextGeometry {
            container_height: 20.0,
            lines: vec![(0.0, 0.0), (0.0, 10.0), (55.0, 0.0)]
        },
    );
    // A container whose direct content is lines with `<br>` between them is
    // balanced across its auto-height columns in source order.
    let own = multicol_text_geometry_of(css, "aa\nbb\ncc", true);
    assert_eq!(own.lines, [(0.0, 0.0), (0.0, 10.0), (55.0, 0.0)]);
    assert_eq!(own.container_height, 20.0);
}

#[test]
fn a_definite_height_multicol_with_line_breaks_keeps_widows() {
    // Hand-computed: a 40px column holds four 10px lines; nine lines fill
    // 4 + 4 + 1, and `widows: 3` moves two lines of the second column into
    // the third. The columns are 120px wide with a 20px gap.
    let geometry = multicol_text_geometry_of(
        "width:400px;column-count:3;column-gap:20px;height:40px;widows:3;orphans:1",
        "1\n2\n3\n4\n5\n6\n7\n8\n9",
        true,
    );
    let xs: Vec<f32> = geometry.lines.iter().map(|line| line.0).collect();
    assert_eq!(xs, [0.0, 0.0, 0.0, 0.0, 140.0, 140.0, 280.0, 280.0, 280.0]);
    assert_eq!(geometry.container_height, 40.0);
}

// ── block-level boxes of every kind inside a paragraph ───────

/// A 200px Ahem paragraph whose children `build` appends after an empty text
/// node. Returns the document, its cascade and the root.
fn paragraph_of(build: impl FnOnce(&mut Document, usize)) -> (Document, CascadeResult, usize) {
    let (mut doc, _cascade, root) = ahem_paragraph_in("", "width:200px", "");
    build(&mut doc, root);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
}

/// `root > ["aa ", div(inner) > [div(margin-top:10px) > "bb"], " cc"]`.
/// Returns the document, the cascade, the root and the block child.
fn paragraph_with_block_child(inner: &str) -> (Document, CascadeResult, usize, usize) {
    let mut block = 0;
    let (doc, cascade, root) = paragraph_of(|doc, root| {
        doc.append_text(root, "aa ");
        block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;{inner}")),
        );
        let first = doc.append_element(
            Some(block),
            "div",
            Style::default(),
            Some("display:block;margin-top:10px"),
        );
        doc.append_text(first, "bb");
        doc.append_text(root, " cc");
    });
    (doc, cascade, root, block)
}

/// Border box of the block child of [`paragraph_with_block_child`] from the
/// root's border box, and the root's height.
fn block_child_geometry(inner: &str) -> ((f32, f32, f32, f32), f32) {
    let (mut doc, cascade, root, block) = paragraph_with_block_child(inner);
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{inner}");
    let layout = doc.nodes[block].unrounded_layout;
    (
        (
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height,
        ),
        doc.nodes[root].unrounded_layout.size.height,
    )
}

#[test]
fn block_level_boxes_of_every_inner_display_keep_the_root() {
    let mut expected_5448 = [
        ((0.0, 10.0, 200.0, 20.0), 40.0),
        ((0.0, 10.0, 200.0, 20.0), 40.0),
        ((0.0, 10.0, 200.0, 20.0), 40.0),
        ((0.0, 20.0, 200.0, 10.0), 40.0),
        ((0.0, 10.0, 200.0, 20.0), 40.0),
        ((0.0, 20.0, 200.0, 10.0), 40.0),
    ]
    .into_iter();
    for inner in [
        "display:flow-root",
        "display:flex",
        "display:grid",
        "display:list-item",
        "overflow:hidden",
        "clear:left",
    ] {
        let (mut doc, cascade, root, _) = paragraph_with_block_child(inner);
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root(), "{inner}");
        assert_eq!(
            block_child_geometry(inner),
            expected_5448.next().expect("a value per case"),
            "{inner}"
        );
    }
}

#[test]
fn a_formatting_context_block_child_does_not_collapse_with_its_children() {
    // Hand-computed: the "aa" line is 10px; a plain block lets its first
    // child's 10px top margin through (its border box starts at 20 and holds
    // one 10px line), a flow-root keeps it inside (it starts at 10 and is
    // 20px tall).
    assert_eq!(
        block_child_geometry("display:flow-root").0,
        (0.0, 10.0, 200.0, 20.0)
    );
    assert_eq!(block_child_geometry("").0, (0.0, 20.0, 200.0, 10.0));
    assert_eq!(
        block_child_geometry("display:flow-root;margin-top:10px"),
        ((0.0, 20.0, 200.0, 20.0), 50.0)
    );
}

/// `root > [div(float:left;width:20px;height:20px), "aa", div(css) > "bb"]`:
/// the block child's border box from the root.
fn block_after_float(float_css: &str, css: &str) -> (f32, f32, f32, f32) {
    let mut block = 0;
    let (mut doc, cascade, root) = paragraph_of(|doc, root| {
        doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;float:left;{float_css}")),
        );
        doc.append_text(root, "aa");
        block = doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some(&format!("display:block;{css}")),
        );
        doc.append_text(block, "bb");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{css}");
    let layout = doc.nodes[block].unrounded_layout;
    (
        layout.location.x,
        layout.location.y,
        layout.size.width,
        layout.size.height,
    )
}

#[test]
fn a_cleared_block_child_moves_below_the_float() {
    let float = "width:20px;height:20px";
    assert_eq!(
        block_after_float(float, "clear:left"),
        (0.0, 20.0, 200.0, 10.0)
    );
    // Hand-computed: a 20px-tall left float precedes a 10px line; the cleared
    // block starts at y = 20.
    assert_eq!(block_after_float(float, "clear:left").1, 20.0);
}

#[test]
fn an_overflow_hidden_block_child_sits_beside_a_float() {
    let float = "width:50px;height:30px";
    // With a width of its own, both paths place it the same.
    assert_eq!(
        block_after_float(float, "overflow:hidden;width:100px"),
        (50.0, 10.0, 100.0, 10.0)
    );
    // Hand-computed: below the 10px line, beside the 50px float, as wide as
    // the 150px left of it.
    assert_eq!(
        block_after_float(float, "overflow:hidden"),
        (50.0, 10.0, 150.0, 10.0)
    );
}

#[test]
fn a_flex_block_child_gets_its_own_height() {
    let mut expected_5539 = [
        ((0.0, 10.0, 200.0, 20.0), 40.0),
        ((0.0, 10.0, 200.0, 20.0), 40.0),
        ((0.0, 10.0, 200.0, 20.0), 40.0),
    ]
    .into_iter();
    for inner in [
        "display:flex",
        "display:grid",
        "display:flex;flex-direction:column",
    ] {
        assert_eq!(
            block_child_geometry(inner),
            expected_5539.next().expect("a value per case"),
            "{inner}"
        );
    }
    // Hand-computed: the flex item keeps its 10px top margin inside the flex
    // container, below the 10px "aa" line.
    assert_eq!(
        block_child_geometry("display:flex").0,
        (0.0, 10.0, 200.0, 20.0)
    );
}

#[test]
fn a_table_block_child_is_laid_out() {
    // A table holding only text: one anonymous cell's content.
    let geometry = || {
        let mut table = 0;
        let (mut doc, cascade, root) = paragraph_of(|doc, root| {
            doc.append_text(root, "aa ");
            table = doc.append_element(Some(root), "div", Style::default(), Some("display:table"));
            doc.append_text(table, "bb");
            doc.append_text(root, " cc");
        });
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        let layout = doc.nodes[table].unrounded_layout;
        (
            layout.location.y,
            layout.size.height,
            doc.nodes[root].unrounded_layout.size.height,
        )
    };
    assert_eq!(geometry(), (10.0, 10.0, 30.0));
    // Hand-computed: between the "aa" and "cc" lines, one 10px line tall.
    assert_eq!(geometry(), (10.0, 10.0, 30.0));
}

#[test]
fn a_list_item_block_child_collapses_like_a_block() {
    // A list item is a block container in its parent's formatting context.
    assert_eq!(
        block_child_geometry("display:list-item"),
        block_child_geometry("")
    );
    assert_eq!(
        block_child_geometry("display:list-item"),
        ((0.0, 20.0, 200.0, 10.0), 40.0)
    );
}

#[test]
fn a_cleared_line_break_moves_the_next_line_below_the_float() {
    let lines = || {
        let (mut doc, cascade, root) = paragraph_of(|doc, root| {
            doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;float:left;width:20px;height:30px"),
            );
            doc.append_text(root, "aa");
            doc.append_element(
                Some(root),
                "br",
                Style::default(),
                Some("display:inline;clear:left"),
            );
            doc.append_text(root, "bb");
        });
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        doc.nodes[root].unrounded_layout.size.height
    };
    // Hand-computed: "aa" beside the 30px float, then "bb" below it.
    assert_eq!(lines(), 40.0);
    assert_eq!(lines(), 40.0);
    let (mut doc, cascade, root) = paragraph_of(|doc, root| {
        doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;float:left;width:20px;height:30px"),
        );
        doc.append_text(root, "aa");
        doc.append_element(
            Some(root),
            "br",
            Style::default(),
            Some("display:inline;clear:left"),
        );
        doc.append_text(root, "bb");
    });
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    assert_eq!(lines[1].block_offset(), 30.0);
}

#[test]
fn a_cleared_line_break_inside_an_inline_element_moves_the_next_line() {
    let (mut doc, cascade, root) = paragraph_of(|doc, root| {
        doc.append_element(
            Some(root),
            "div",
            Style::default(),
            Some("display:block;float:left;width:20px;height:30px"),
        );
        let span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("display:inline;padding-right:2px;border-right:1px solid"),
        );
        doc.append_text(span, "aa");
        doc.append_element(
            Some(span),
            "br",
            Style::default(),
            Some("display:inline;clear:left"),
        );
        doc.append_text(span, "bb");
    });
    lay_out(&mut doc, &cascade);
    let lines = &stored_lines(&doc, root).lines;
    // The line the break ends reports it, so the next line clears the float.
    assert_eq!(lines[1].block_offset(), 30.0);
}

#[test]
fn logical_float_sides_and_clears_are_placed() {
    // The bridge maps the logical sides to none on both paths: such a box is
    // not floated and does not clear.
    let mut expected_5662 = [(0.0, 10.0, 40.0), (0.0, 10.0, 40.0)].into_iter();
    for css in ["float:inline-start", "float:inline-end"] {
        let geometry = || {
            let mut float = 0;
            let (mut doc, cascade, root) = paragraph_of(|doc, root| {
                doc.append_text(root, "aa");
                float = doc.append_element(
                    Some(root),
                    "div",
                    Style::default(),
                    Some(&format!("display:block;width:20px;height:20px;{css}")),
                );
                doc.append_text(root, "bb");
            });
            lay_out(&mut doc, &cascade);
            assert!(doc.nodes[root].is_ifc_root(), "{css}");
            let layout = doc.nodes[float].unrounded_layout;
            (
                layout.location.x,
                layout.location.y,
                doc.nodes[root].unrounded_layout.size.height,
            )
        };
        assert_eq!(
            geometry(),
            expected_5662.next().expect("a value per case"),
            "{css}"
        );
    }
    assert_eq!(
        block_after_float("width:20px;height:20px", "clear:inline-start"),
        (0.0, 10.0, 200.0, 10.0)
    );
}

// ── boxes inside inline elements ─────────────────────────────

/// `root > [span > ["aaaa", child(css)], " bbbb"]` in a 200px Ahem paragraph.
fn paragraph_with_span_child(css: &str) -> (Document, CascadeResult, usize) {
    paragraph_of(|doc, root| {
        let span = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
        doc.append_text(span, "aaaa");
        doc.append_element(
            Some(span),
            "span",
            Style::default(),
            Some(&format!("display:block;{css}")),
        );
        doc.append_text(root, " bbbb");
    })
}

/// The position of `node` from the page origin, adding up the locations of
/// its layout parents.
fn layout_position(doc: &Document, node: usize) -> (f32, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut current = Some(node);
    while let Some(id) = current {
        let layout = doc.nodes[id].unrounded_layout;
        x += layout.location.x;
        y += layout.location.y;
        current = doc.layout_parent_of(id);
    }
    (x, y)
}

#[test]
fn a_float_inside_a_span_keeps_the_root() {
    let (mut doc, cascade, root) = paragraph_with_span_child("float:left;width:20px;height:10px");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn layout_parent_of_skips_inline_elements_inside_a_root() {
    let (mut doc, cascade, root) =
        paragraph_with_span_child("display:inline-block;width:20px;height:10px");
    lay_out(&mut doc, &cascade);
    let span = doc.nodes[root].children[1];
    let atomic = doc.nodes[span].children[1];
    assert_eq!(doc.layout_parent_of(atomic), Some(root));
    assert_eq!(doc.layout_parent_of(span), doc.parent_of(span));
    assert!(!doc.contributes_layout_offset(span));
    assert!(doc.contributes_layout_offset(atomic));
}

#[test]
fn an_atomic_inside_a_span_has_the_same_absolute_position_as_without_the_span() {
    let position = |nested: bool| {
        let mut atomic = 0;
        let (mut doc, cascade, root) = paragraph_of(|doc, root| {
            let parent = if nested {
                doc.append_element(Some(root), "span", Style::default(), Some("padding-left:0"))
            } else {
                root
            };
            doc.append_text(parent, "aaaa");
            atomic = doc.append_element(
                Some(parent),
                "span",
                Style::default(),
                Some("display:inline-block;width:20px;height:10px"),
            );
            doc.append_text(root, " bbbb");
        });
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        let (root_x, root_y) = layout_position(&doc, root);
        let (x, y) = layout_position(&doc, atomic);
        (x - root_x, y - root_y)
    };
    assert_eq!(position(true), (40.0, 0.0));
    // Hand-computed: "aaaa" (40px) then the 20x10 atomic, which sits on the
    // baseline and is the tallest thing on the line: its top is the line's.
    assert_eq!(position(true), (40.0, 0.0));
}

#[test]
fn a_float_inside_a_nested_span_displaces_the_next_lines() {
    let (mut doc, cascade, root) = paragraph_of(|doc, root| {
        let outer = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
        let inner = doc.append_element(Some(outer), "span", Style::default(), None::<&str>);
        doc.append_element(
            Some(inner),
            "div",
            Style::default(),
            Some("display:block;float:left;width:120px;height:20px"),
        );
        doc.append_text(outer, "aaaa bbbb cccc");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    // Hand-computed: 80px beside the 120px float on the first two lines, one
    // 40px word each ("aaaa bbbb" is 90px); the third line is below it.
    let lines = &stored_lines(&doc, root).lines;
    let starts: Vec<Option<f32>> = lines.iter().map(line_start_x).collect();
    assert_eq!(starts, [Some(120.0), Some(120.0), Some(0.0)]);
}

#[test]
fn a_block_inside_a_span_splits_the_paragraph() {
    let geometry = || {
        let mut block = 0;
        let (mut doc, cascade, root) = paragraph_of(|doc, root| {
            doc.append_text(root, "aa ");
            let span = doc.append_element(Some(root), "span", Style::default(), None::<&str>);
            doc.append_text(span, "bb");
            block = doc.append_element(Some(span), "div", Style::default(), Some("display:block"));
            doc.append_text(block, "cc");
            doc.append_text(span, "dd");
            doc.append_text(root, " ee");
        });
        lay_out(&mut doc, &cascade);
        assert!(doc.nodes[root].is_ifc_root());
        let (root_x, root_y) = layout_position(&doc, root);
        let (x, y) = layout_position(&doc, block);
        (
            x - root_x,
            y - root_y,
            doc.nodes[block].unrounded_layout.size.height,
            doc.nodes[root].unrounded_layout.size.height,
        )
    };
    let (on, off) = (geometry(), (0.0, 10.0, 10.0, 30.0));
    assert_eq!((on.1, on.2, on.3), (off.1, off.2, off.3));
    // Hand-computed: "aa bb", the block's "cc" line, then "dd ee"; the block
    // starts at the content edge.
    assert_eq!(on, (0.0, 10.0, 10.0, 30.0));
}

// ── positioned boxes inside a paragraph ──────────────────────

/// `root(position:relative) > ["aaaa ", span(position:absolute;css), " bbbb"]`
/// in a 200px Ahem paragraph. Returns the document, cascade, root and span.
fn paragraph_with_absolute_child(css: &str) -> (Document, CascadeResult, usize, usize) {
    let mut span = 0;
    let (mut doc, _cascade, root) = paragraph_of(|doc, root| {
        doc.append_text(root, "aaaa ");
        span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some(&format!("position:absolute;width:10px;height:10px;{css}")),
        );
        doc.append_text(root, " bbbb");
    });
    doc.set_element_inline_style(
        root,
        Some("display:block;line-height:10px;width:200px;position:relative".into()),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root, span)
}

/// The border-box location of the positioned span from the root's border
/// box, and the root's line count (0 off the engine).
fn positioned_box_location(css: &str) -> (f32, f32) {
    let (mut doc, cascade, root, span) = paragraph_with_absolute_child(css);
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root(), "{css}");
    let layout = doc.nodes[span].unrounded_layout;
    (layout.location.x, layout.location.y)
}

#[test]
fn a_positioned_box_inside_a_paragraph_keeps_the_root() {
    let (mut doc, cascade, root, _) = paragraph_with_absolute_child("left:5px;top:7px");
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn a_positioned_box_with_auto_insets_takes_its_static_position() {
    // Hand-computed (CSS 2.1 10.3.7, 10.6.4): the span was inline-level, so
    // its static position is where it would have been on the line, after
    // "aaaa " (50px), at the top of that line.
    assert_eq!(positioned_box_location(""), (50.0, 0.0));
    // A box that was block-level would have started below the line.
    assert_eq!(positioned_box_location("display:block"), (0.0, 10.0));
}

#[test]
fn a_positioned_box_with_insets_is_placed_against_the_containing_block() {
    let mut expected_5872 = [(5.0, 7.0), (185.0, -7.0), (20.0, 0.0)].into_iter();
    for css in [
        "left:5px;top:7px",
        "right:5px;bottom:7px",
        "left:20px;top:0",
    ] {
        assert_eq!(
            positioned_box_location(css),
            expected_5872.next().expect("a value per case"),
            "{css}"
        );
    }
    // Hand-computed: against the root's padding box (no padding).
    assert_eq!(positioned_box_location("left:5px;top:7px"), (5.0, 7.0));
    // From the right edge of the 200px box: 200 - 5 - 10.
    assert_eq!(positioned_box_location("right:5px;top:0").0, 185.0);
}

#[test]
fn an_absolute_box_does_not_take_up_inline_space() {
    for css in [
        "left:0;top:0",
        "display:inline-block",
        "display:block;float:left",
    ] {
        let (mut doc, cascade, root, _) = paragraph_with_absolute_child(css);
        lay_out(&mut doc, &cascade);
        let lines = &stored_lines(&doc, root).lines;
        // Hand-computed: one 90px line, "aaaa bbbb" (the two spaces collapse).
        assert_eq!(lines.len(), 1, "{css}");
        assert_eq!(lines[0].inline_size(), 90.0, "{css}");
    }
}

#[test]
fn a_fixed_box_inside_a_paragraph_is_laid_out() {
    // With insets: against the root, as taffy lays a fixed child out.
    let css = "position:fixed;left:3px;top:4px";
    assert_eq!(positioned_box_location(css), (3.0, 4.0),);
    assert_eq!(positioned_box_location(css), (3.0, 4.0));
}

#[test]
fn a_relatively_positioned_block_child_is_offset() {
    let mut expected_5920 = [
        ((4.0, 23.0, 200.0, 10.0), 40.0),
        ((0.0, 23.0, 200.0, 10.0), 40.0),
    ]
    .into_iter();
    for css in [
        "position:relative;left:4px;top:3px",
        "position:sticky;top:3px",
    ] {
        assert_eq!(
            block_child_geometry(css),
            expected_5920.next().expect("a value per case"),
            "{css}"
        );
    }
}

// ── `ch` on box properties through the inline engine's fonts ──

/// The border box of an Ahem block styled `css` (10px font), on either path.
fn ch_box_size(css: &str) -> (f32, f32, f32, f32) {
    let (mut doc, cascade, root) = ahem_paragraph("aa", css);
    lay_out(&mut doc, &cascade);
    let layout = doc.nodes[root].unrounded_layout;
    (
        layout.size.width,
        layout.size.height,
        layout.padding.left,
        layout.margin.left,
    )
}

#[test]
fn ch_box_values_resolve_through_the_engine() {
    let mut expected_5949 = [
        (100.0, 10.0, 0.0, 0.0),
        (800.0, 30.0, 0.0, 0.0),
        (800.0, 10.0, 20.0, 0.0),
        (760.0, 10.0, 0.0, 40.0),
    ]
    .into_iter();
    for css in [
        "width:10ch",
        "height:3ch",
        "padding-left:2ch",
        "margin-left:4ch",
    ] {
        assert_eq!(
            ch_box_size(css),
            expected_5949.next().expect("a value per case"),
            "{css}"
        );
    }
    // Hand-computed: Ahem's "0" advance is 1 em, so 10ch at 10px is 100px.
    assert_eq!(ch_box_size("width:10ch").0, 100.0);
    assert_eq!(ch_box_size("padding-left:2ch").2, 20.0);
}

#[test]
fn ch_min_max_constraints_use_the_selected_ahem_zero_advance() {
    let widths = [
        "font-size:16px;width:100px;max-width:4ch",
        "font-size:16px;width:16px;min-width:4ch",
    ]
    .map(|css| ch_box_size(css).0);
    let heights = [
        "font-size:16px;height:100px;max-height:4ch",
        "font-size:16px;height:16px;min-height:4ch",
    ]
    .map(|css| ch_box_size(css).1);
    assert_eq!([widths[0], widths[1], heights[0], heights[1]], [64.0; 4]);
}

#[test]
fn ch_constraints_preserve_logical_minimums_and_ordinary_values() {
    assert_eq!(
        ch_box_size("font-size:16px;height:16px;min-block-size:4ch").1,
        64.0
    );
    assert_eq!(
        ch_box_size("font-size:8px;width:100px;max-width:4ch").0,
        32.0
    );
    assert_eq!(ch_box_size("width:100px;max-width:50px").0, 50.0);
    assert_eq!(ch_box_size("width:100px;max-width:50%").0, 100.0);
    assert_eq!(ch_box_size("height:10px;min-height:30px").1, 30.0);
}

#[test]
fn ch_constraints_keep_the_empty_font_collection_fallback() {
    let (mut doc, cascade, root) =
        ahem_paragraph_with("font-size:16px;width:100px;max-width:4ch", |_, _| {});
    let fonts = shodo::font::FontCollection::with_options(
        &shodo::limits::Limits::default(),
        shodo::font::FontOptions {
            system_fonts: false,
            ..Default::default()
        },
    );
    doc.set_font_collection(fonts);
    layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
    assert_eq!(doc.nodes[root].unrounded_layout.size.width, 32.0);
}

#[test]
fn logical_ch_minimums_follow_the_existing_normalized_bridge_axis() {
    for (css, expected_height) in [
        ("writing-mode:vertical-rl;min-block-size:4ch", 64.0),
        (
            "writing-mode:vertical-rl;min-block-size:4ch;min-height:20px",
            20.0,
        ),
    ] {
        let css = format!("font-size:16px;width:16px;height:16px;{css}");
        let (mut doc, cascade, root) = ahem_paragraph("aa", &css);
        lay_out(&mut doc, &cascade);
        assert_eq!(
            doc.nodes[root].style.min_size.width.into_raw().value(),
            64.0
        );
        assert_eq!(
            doc.nodes[root].style.min_size.height.into_raw().value(),
            expected_height
        );
    }
}

#[test]
fn max_width_ch_uses_the_inherited_font_before_laying_out_children() {
    let mut child = 0;
    let (mut doc, cascade, _) = ahem_paragraph_with("font-size:16px", |doc, parent| {
        child = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:block;max-width:4ch;white-space:nowrap;overflow:hidden;text-overflow:ellipsis"),
        );
        doc.append_text(child, "ABCABCABCABC");
    });
    lay_out(&mut doc, &cascade);
    assert_eq!(cascade.computed[child].font_size.0, 16.0);
    assert_eq!(doc.nodes[child].unrounded_layout.size.width, 64.0);
}

#[test]
fn measure_ch_advance_uses_the_engine_fonts() {
    let key = raikiri_style::ChFontKey {
        family: std::sync::Arc::new(vec![raikiri_style::FontFamilyName::named("Ahem")]),
        size: raikiri_style::ComputedLength(16.0),
        weight: 400.0,
        style: raikiri_style::property::FontStyle::Normal,
    };
    assert_eq!(
        crate::layout::measure_ch_advance(&ifc_ahem_fonts(), &key),
        16.0
    );
}

#[test]
fn a_fixed_root_and_a_paragraph_in_an_unsized_fixed_box_are_laid_out_by_the_engine() {
    // A fixed box shrinks against the page area; "aaaa bbbb cccc" is one
    // 140px line wherever the box sits, also under a zero-wide positioned
    // parent (CSS 2.1 10.1).
    for (parent_css, css) in [
        ("", "position:fixed"),
        ("width:0;position:relative", "position:fixed"),
    ] {
        let (mut doc, cascade, root) = ahem_paragraph_in(parent_css, css, "aaaa bbbb cccc");
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert!(doc.nodes[root].is_ifc_root(), "{parent_css} / {css}");
        let size = doc.nodes[root].unrounded_layout.size;
        assert_eq!(
            (size.width, size.height),
            (140.0, 10.0),
            "{parent_css} / {css}"
        );
    }
    // A block paragraph inside a fixed box: the box is laid out by taffy on
    // both paths, the paragraph by the engine.
    let (mut doc, cascade, root) = ahem_paragraph_in("position:fixed", "", "aaaa bbbb cccc");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
}

#[test]
fn a_fixed_paragraph_shrinks_against_the_page_not_its_parent() {
    // A fixed box's containing block is the viewport (the page area here),
    // not the narrow absolutely positioned box it sits in: its 140px line
    // "aaaa bbbb cccc" stays one line.
    let size = || {
        let mut fixed = 0;
        let (mut doc, cascade, _root) = paragraph_of(|doc, root| {
            let abs = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;position:absolute"),
            );
            doc.append_text(abs, "aa");
            fixed = doc.append_element(
                Some(abs),
                "div",
                Style::default(),
                Some("display:block;position:fixed;bottom:0"),
            );
            doc.append_text(fixed, "aaaa bbbb cccc");
        });
        lay_out(&mut doc, &cascade);
        doc.nodes[fixed].unrounded_layout.size
    };
    assert_eq!(
        size(),
        taffy::Size {
            width: 140.0,
            height: 10.0
        }
    );
    assert_eq!((size().width, size().height), (140.0, 10.0));
}

#[test]
fn the_text_of_an_absolutely_positioned_inline_is_its_own_paragraph() {
    // An absolutely positioned box is blockified (CSS 2.1 9.7): a `<span>`
    // with `position: absolute` is a block container whose text is a
    // paragraph of the engine. Hand-computed: "abspos" is one 60px line.
    let mut span = 0;
    let (mut doc, cascade, root) = paragraph_of(|doc, root| {
        doc.append_text(root, "aa");
        span = doc.append_element(
            Some(root),
            "span",
            Style::default(),
            Some("position:absolute;left:0;top:20px"),
        );
        doc.append_text(span, "abspos");
    });
    lay_out(&mut doc, &cascade);
    assert!(doc.nodes[root].is_ifc_root());
    assert!(doc.nodes[span].is_ifc_root());
    let layout = doc.nodes[span].unrounded_layout;
    assert_eq!(
        (layout.location.y, layout.size.width, layout.size.height),
        (20.0, 60.0, 10.0)
    );
}

/// `html > body > "aaaa bbbb"` with `html_css` / `body_css`, laid out with
/// Ahem: whether the body is a paragraph root, whether `html` is one, and
/// the number of lines of the text.
fn inline_body(html_css: Option<&str>, body_css: &str) -> (bool, bool, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), html_css);
    let css = format!("font-family:Ahem;font-size:10px;line-height:10px;{body_css}");
    let body = doc.append_element(Some(html), "body", Style::default(), Some(css.as_str()));
    let text = doc.append_text(body, "aaaa bbbb");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    lay_out(&mut doc, &cascade);
    (
        doc.nodes[body].is_ifc_root(),
        doc.nodes[html].is_ifc_root(),
        doc.ifc_text_lines(text)
            .map_or(0, |lines| lines.lines.len()),
    )
}

#[test]
fn an_inline_body_still_lays_its_text_out_as_a_paragraph() {
    // Layout starts at the body, which always generates a block box (CSS
    // Display 3 2.7 blockifies the root element): its text is a paragraph of
    // one line on the page, with or without a UA stylesheet, and `html` (not laid
    // out) is never the paragraph.
    assert_eq!(inline_body(None, ""), (true, false, 1));
    assert_eq!(
        inline_body(Some("display:block"), "display:inline"),
        (true, false, 1)
    );
    // `display: contents` on the root also computes to `block`.
    assert_eq!(
        inline_body(Some("display:block"), "display:contents"),
        (true, false, 1)
    );
}

fn page_sibling_positions(
    heights: &[u32],
    before: &str,
    after: &str,
    inside: &str,
) -> (Vec<f32>, usize) {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let mut boxes = Vec::new();
    for (index, height) in heights.iter().enumerate() {
        let edge = match index {
            1 => after,
            2 => before,
            _ => "",
        };
        boxes.push(doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(&format!(
                "display:block;width:100px;height:{height}px;{inside};{edge}"
            )),
        ));
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let pages = layout_pages(&mut doc, &cascade, page).unwrap();
    (
        boxes
            .into_iter()
            .map(|id| doc.get_node(id).unwrap().unrounded_layout.location.y)
            .collect(),
        pages.len(),
    )
}

fn avoiding_sibling_positions(before: &str, after: &str) -> (Vec<f32>, usize) {
    page_sibling_positions(&[60, 30, 30], before, after, "break-inside:avoid")
}

#[test]
fn automatic_overflow_keeps_each_avoiding_block_intact() {
    assert_eq!(
        avoiding_sibling_positions("", ""),
        (vec![0.0, 60.0, 100.0], 2)
    );
}

#[test]
fn before_avoid_moves_the_connected_sibling_run_to_the_next_page() {
    assert_eq!(
        avoiding_sibling_positions("break-before:avoid", ""),
        (vec![0.0, 100.0, 130.0], 2)
    );
}

#[test]
fn running_elements_are_removed_from_the_normal_flow() {
    use raikiri_style::{build_rule_tree, cascade};

    for position in ["static", "relative", "sticky", "absolute", "fixed"] {
        let mut doc = Document::new();
        let root = doc.append_element(
            Some(0),
            "div",
            Style::default(),
            Some(&format!(
                "display:block;position:{position};position:running(header);break-before:page"
            )),
        );
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        // The winning running() value replaces the preceding declaration
        // but records a template without changing the initial position.
        // CSS GCPM 3 §1.2.1 removes the element from the normal flow: it
        // generates no box, and its block display moves to the template.
        assert_eq!(cascade.computed[root].position, PositionValue::Static);
        assert_eq!(cascade.computed[root].display, DisplayValue::None);
        assert_eq!(cascade.computed[root].running_templates.len(), 1);
        assert_eq!(cascade.computed[root].running_templates[0].name, "header");
        assert_eq!(
            cascade.computed[root].running_templates[0].display,
            DisplayValue::Block
        );
    }

    // Neither their size nor their break properties reach the flow.
    for (before, after) in [("break-before:avoid", ""), ("break-before:page", "")] {
        assert_eq!(
            page_sibling_positions(&[60, 30, 30], before, after, "position:running(header)"),
            (vec![0.0, 0.0, 0.0], 1)
        );
    }
}

#[test]
fn a_parallel_float_preserves_the_connected_normal_flow_run() {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let body = doc.append_element(
        Some(0),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let mut normal = Vec::new();
    for css in [
        "height:60px",
        "height:30px",
        "float:left;height:20px;width:20px",
        "height:30px;break-before:avoid;break-inside:avoid",
    ] {
        let id = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(&format!("display:block;width:100px;{css}")),
        );
        if !css.starts_with("float") {
            normal.push(id);
        }
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let pages = layout_pages(&mut doc, &cascade, page).unwrap();
    let positions: Vec<_> = normal
        .into_iter()
        .map(|id| doc.nodes[id].unrounded_layout.location.y)
        .collect();
    assert_eq!(positions, vec![0.0, 100.0, 130.0]);
    assert_eq!(pages.len(), 2);
}

#[test]
fn after_avoid_moves_the_connected_sibling_run_to_the_next_page() {
    assert_eq!(
        avoiding_sibling_positions("", "break-after:avoid"),
        (vec![0.0, 100.0, 130.0], 2)
    );
}

#[test]
fn page_specific_avoid_moves_the_connected_sibling_run_to_the_next_page() {
    assert_eq!(
        avoiding_sibling_positions("break-before:avoid-page", ""),
        (vec![0.0, 100.0, 130.0], 2)
    );
}

#[test]
fn column_specific_avoid_does_not_forbid_a_page_boundary() {
    assert_eq!(
        avoiding_sibling_positions("break-before:avoid-column", ""),
        (vec![0.0, 60.0, 100.0], 2)
    );
}

#[test]
fn between_avoid_moves_siblings_without_an_inside_constraint() {
    assert_eq!(
        page_sibling_positions(&[60, 30, 30], "break-before:avoid", "", ""),
        (vec![0.0, 100.0, 130.0], 2)
    );
}

#[test]
fn oversized_avoided_sibling_run_relaxes_the_between_constraint() {
    assert_eq!(
        page_sibling_positions(
            &[60, 70, 70],
            "break-before:avoid",
            "",
            "break-inside:avoid"
        ),
        (vec![0.0, 100.0, 200.0], 3)
    );
}

#[test]
fn always_forces_a_page_boundary_outside_columns() {
    assert_eq!(
        page_sibling_positions(&[20, 20, 20], "break-before:always", "", ""),
        (vec![0.0, 20.0, 100.0], 2)
    );
}

#[test]
fn a_forced_page_boundary_overrides_avoidance_at_the_same_edge() {
    assert_eq!(
        page_sibling_positions(&[20, 20, 20], "break-before:page", "break-after:avoid", ""),
        (vec![0.0, 20.0, 100.0], 2)
    );
}

#[test]
fn column_breaks_do_not_force_a_page_boundary() {
    assert_eq!(
        page_sibling_positions(&[20, 20, 20], "break-before:column", "", ""),
        (vec![0.0, 20.0, 40.0], 1)
    );
}

#[test]
fn always_uses_the_nearest_column_context_but_page_stays_page_specific() {
    let mut doc = Document::new();
    let columns = doc.append_element(Some(0), "div", Style::default(), Some("columns:2"));
    let child = doc.append_element(Some(columns), "div", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    apply_computed_to_style(&mut doc, &cascade).unwrap();
    assert!(!page_break_is_forced(&doc, child, BreakBetween::Always));
    assert!(page_break_is_forced(&doc, child, BreakBetween::Page));
}

#[test]
fn review_avoided_sibling_runs_use_the_destination_page_height() {
    use raikiri_style::{build_rule_tree, cascade};
    for (steps, prefix, expected) in [
        ([50.0, 100.0], 20, [0.0, 50.0, 90.0]),
        ([100.0, 50.0], 60, [0.0, 60.0, 100.0]),
    ] {
        let mut doc = Document::new();
        let body = doc.append_element(
            Some(0),
            "body",
            Style::default(),
            Some("display:block;margin:0"),
        );
        let mut ids = Vec::new();
        for css in [
            format!("height:{prefix}px"),
            "height:40px".into(),
            "height:40px;break-before:avoid".into(),
        ] {
            ids.push(doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some(&format!("display:block;width:100px;{css}")),
            ));
        }
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        let mut page = PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_pages_with_page_steps(&mut doc, &cascade, page, &steps).unwrap();
        let actual: Vec<_> = ids
            .into_iter()
            .map(|id| doc.nodes[id].unrounded_layout.location.y)
            .collect();
        assert_eq!(actual, expected, "{steps:?}");
    }
}

#[test]
fn review_same_explicit_page_name_keeps_avoided_sibling_runs_connected() {
    for edge in ["break-before:avoid", "break-before:avoid-page"] {
        assert_eq!(
            page_sibling_positions(&[60, 30, 30], edge, "", "break-inside:avoid;page:chapter"),
            (vec![0.0, 100.0, 130.0], 2)
        );
    }
}

#[test]
fn review_flow_root_page_siblings_keep_avoided_runs_connected() {
    for edge in ["break-before:avoid", "break-before:avoid-page"] {
        assert_eq!(
            page_sibling_positions(
                &[60, 30, 30],
                edge,
                "",
                "display:flow-root;break-inside:avoid"
            ),
            (vec![0.0, 100.0, 130.0], 2)
        );
    }
}

#[test]
fn review_list_item_page_siblings_keep_avoided_runs_connected() {
    assert_eq!(
        page_sibling_positions(
            &[60, 30, 30],
            "break-before:avoid-page",
            "",
            "display:list-item;list-style:none;break-inside:avoid"
        ),
        (vec![0.0, 100.0, 130.0], 2)
    );
}

#[test]
fn review_block_level_context_page_siblings_keep_outer_avoidance() {
    for display in ["flex", "grid", "flow-root", "list-item"] {
        assert_eq!(
            page_sibling_positions(
                &[60, 30, 30],
                "",
                "break-after:avoid-page",
                &format!("display:{display};list-style:none;break-inside:avoid")
            ),
            (vec![0.0, 100.0, 130.0], 2)
        );
        assert_eq!(
            page_sibling_positions(
                &[60, 30, 30],
                "break-before:page",
                "break-after:avoid-page",
                &format!("display:{display};list-style:none;break-inside:avoid")
            ),
            (vec![0.0, 60.0, 100.0], 2)
        );
    }
}

#[test]
fn review_forced_page_edge_still_overrides_same_named_avoidance() {
    assert_eq!(
        page_sibling_positions(
            &[60, 30, 30],
            "break-before:page",
            "break-after:avoid",
            "break-inside:avoid;page:chapter"
        ),
        (vec![0.0, 60.0, 100.0], 2)
    );
}

#[test]
fn review_a_changed_explicit_page_name_still_breaks_the_avoided_run() {
    let mut doc = Document::new();
    let body = doc.append_element(
        Some(0),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let ids: Vec<_> = [
        "height:60px;page:first",
        "height:30px;page:first",
        "height:30px;page:second;break-before:avoid",
    ]
    .into_iter()
    .map(|css| {
        doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(&format!(
                "display:block;width:100px;break-inside:avoid;{css}"
            )),
        )
    })
    .collect();
    doc.mark_in_document_flags();
    let tree = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &tree).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let pages = layout_pages(&mut doc, &cascade, page).unwrap();
    let actual: Vec<_> = ids
        .into_iter()
        .map(|id| doc.nodes[id].unrounded_layout.location.y)
        .collect();
    assert_eq!(actual, vec![0.0, 60.0, 100.0]);
    assert_eq!(pages.len(), 2);
}

fn review_body_boundary_document(body_css: &str) -> (Document, usize, Vec<usize>) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some(&format!("display:block;margin:0;{body_css}")),
    );
    let first = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;flex-shrink:0;width:100px;height:60px"),
    );
    (doc, body, vec![first])
}

fn review_body_boundary_positions(mut doc: Document, ids: &[usize]) -> Vec<f32> {
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_pages(&mut doc, &cascade, page).unwrap();
    ids.iter()
        .map(|&id| crate::layout::test_support::absolute_rect(&doc, id).1)
        .collect()
}

#[test]
fn review_ignored_flex_item_float_keeps_page_avoidance_connected() {
    let (mut doc, body, mut ids) =
        review_body_boundary_document("display:flex;flex-direction:column");
    ids.push(doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some(
            "display:block;flex-shrink:0;float:left;width:100px;height:30px;break-after:avoid-page",
        ),
    ));
    ids.push(doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:block;flex-shrink:0;width:100px;height:30px;break-inside:avoid"),
    ));
    assert_eq!(
        review_body_boundary_positions(doc, &ids),
        vec![0.0, 100.0, 130.0]
    );
}

#[test]
fn review_contents_boxes_are_effective_body_siblings_for_avoidance() {
    let (mut doc, body, mut ids) = review_body_boundary_document("");
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:contents"),
    );
    ids.push(doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:block;width:100px;height:30px;break-inside:avoid"),
    ));
    ids.push(doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:block;width:100px;height:30px;break-before:avoid;break-inside:avoid"),
    ));
    assert_eq!(
        review_body_boundary_positions(doc, &ids),
        vec![0.0, 100.0, 130.0]
    );
}

#[test]
fn review_plain_child_edges_propagate_page_avoidance_to_body_siblings() {
    for (first_edge, last_edge) in [
        ("break-after:avoid-page", ""),
        ("", "break-before:avoid-page"),
    ] {
        let (mut doc, body, mut ids) = review_body_boundary_document("");
        for edge in [first_edge, last_edge] {
            let wrapper = doc.append_element(
                Some(body),
                "div",
                Style::default(),
                Some("display:block;width:100px;break-inside:avoid"),
            );
            doc.append_element(
                Some(wrapper),
                "div",
                Style::default(),
                Some(&format!("display:block;width:100px;height:30px;{edge}")),
            );
            ids.push(wrapper);
        }
        assert_eq!(
            review_body_boundary_positions(doc, &ids),
            vec![0.0, 100.0, 130.0],
            "first={first_edge} last={last_edge}"
        );
    }
}

#[test]
fn review_page_child_edges_respect_context_and_text_barriers() {
    for (wrapper_css, child_css, text, expected) in [
        (
            "display:block",
            "break-before:avoid-page;break-after:avoid-page",
            "",
            (false, true),
        ),
        (
            "display:block",
            "break-before:avoid-column;break-after:avoid-column",
            "",
            (false, false),
        ),
        (
            "display:flex",
            "break-before:avoid-page;break-after:avoid-page",
            "",
            (false, false),
        ),
        (
            "display:grid",
            "break-before:avoid-page;break-after:avoid-page",
            "",
            (false, false),
        ),
        (
            "display:inline-block",
            "break-before:avoid-page;break-after:avoid-page",
            "",
            (false, false),
        ),
        (
            "display:block",
            "break-before:avoid-page;break-after:avoid-page",
            "M",
            (false, false),
        ),
        (
            "display:block;break-before:avoid;break-after:avoid",
            "break-before:page;break-after:page",
            "",
            (true, true),
        ),
    ] {
        let (mut doc, body, _) = review_body_boundary_document("");
        let wrapper =
            doc.append_element(Some(body), "article", Style::default(), Some(wrapper_css));
        if !text.is_empty() {
            doc.append_text(wrapper, text);
        }
        doc.append_element(
            Some(wrapper),
            "div",
            Style::default(),
            Some(&format!("display:block;height:20px;{child_css}")),
        );
        if !text.is_empty() {
            doc.append_text(wrapper, text);
        }
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
        for before in [false, true] {
            assert_eq!(
                page_edge_constraint(&doc, &cascade, wrapper, before),
                expected,
                "wrapper={wrapper_css} child={child_css} text={text} before={before}"
            );
        }
    }
}

#[test]
fn review_page_child_edges_elide_contents_and_skip_parallel_children() {
    let (mut doc, body, _) = review_body_boundary_document("");
    let wrapper = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    for css in ["display:none", "position:absolute", "float:left"] {
        doc.append_element(
            Some(wrapper),
            "div",
            Style::default(),
            Some(&format!("height:1px;{css}")),
        );
    }
    let empty = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:contents;break-before:page;break-after:page"),
    );
    doc.append_text(empty, " \n ");
    doc.append_element(Some(empty), "div", Style::default(), Some("display:none"));
    let contents = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:contents"),
    );
    doc.append_text(contents, " \n ");
    doc.append_element(
        Some(contents),
        "div",
        Style::default(),
        Some("display:block;height:20px;break-before:avoid-page;break-after:avoid-page"),
    );
    doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:none;height:1px"),
    );
    let empty = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:contents;break-before:page;break-after:page"),
    );
    doc.append_text(empty, " \n ");
    doc.append_element(Some(empty), "div", Style::default(), Some("display:none"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
    for before in [false, true] {
        assert_eq!(
            page_edge_constraint(&doc, &cascade, wrapper, before),
            (false, true)
        );
    }
}

#[test]
fn review_page_generated_content_stops_child_edge_propagation() {
    for display in ["block", "contents"] {
        let (mut doc, body, _) = review_body_boundary_document("");
        let wrapper =
            doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
        let generated = doc.append_element(
            Some(wrapper),
            "article",
            Style::default(),
            Some(&format!(
                "display:{display};break-before:page;break-after:page"
            )),
        );
        doc.append_element(
            Some(generated),
            "div",
            Style::default(),
            Some("display:block;height:20px;break-before:avoid-page;break-after:avoid-page"),
        );
        doc.mark_in_document_flags();
        let mut rules = raikiri_style::build_rule_tree(&doc);
        rules.add_stylesheet(
            "article::before,article::after{content:'M'}",
            raikiri_style::Origin::Author,
        );
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
        assert!(doc.nodes[generated].has_before_or_after_content);
        for before in [false, true] {
            assert_eq!(
                page_edge_constraint(&doc, &cascade, wrapper, before),
                (display == "block", false),
                "display={display} before={before}"
            );
        }
    }
}

#[test]
fn review_page_contents_edge_depth_exhaustion_is_an_opaque_boundary() {
    let (mut doc, body, _) = review_body_boundary_document("");
    let wrapper = doc.append_element(Some(body), "div", Style::default(), Some("display:block"));
    let mut parent = wrapper;
    for _ in 0..129 {
        parent = doc.append_element(
            Some(parent),
            "div",
            Style::default(),
            Some("display:contents"),
        );
    }
    doc.append_element(
        Some(parent),
        "div",
        Style::default(),
        Some("display:block;height:1px"),
    );
    doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:block;height:1px;break-before:avoid-page"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
    layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
    assert_eq!(
        page_edge_constraint(&doc, &cascade, wrapper, true),
        (false, false)
    );
}

#[test]
fn review_contents_nonempty_text_interrupts_avoided_body_sibling_runs() {
    for text in ["M", " \n "] {
        let (mut doc, body, mut ids) =
            review_body_boundary_document("font-family:Ahem;font-size:20px;line-height:20px");
        doc.set_font_collection(crate::layout::test_support::ifc_ahem_fonts());
        ids.push(doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:30px;break-after:avoid-page;break-inside:avoid"),
        ));
        let contents = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:contents"),
        );
        doc.append_text(contents, text);
        ids.push(doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some("display:block;height:30px;break-before:avoid-page;break-inside:avoid"),
        ));
        let positions = review_body_boundary_positions(doc, &ids);
        assert_eq!(
            positions[1],
            if text == "M" { 60.0 } else { 100.0 },
            "text={text:?}"
        );
        assert!(positions[2] >= 100.0);
    }
}

fn many_column_fixture(count: usize) -> (Document, CascadeResult, usize) {
    use crate::fragment::{FragmentRect, LayoutFragment};
    use crate::node::MulticolTextFragment;
    let text = vec!["A"; count].join("\n");
    let (mut doc, cascade, root) = crate::layout::test_support::ahem_paragraph(&text, "width:20px");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    doc.fragment_tree.fragments.clear();
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
    for column in 0..count {
        doc.fragment_tree
            .try_push(LayoutFragment {
                node_id: root,
                parent: None,
                fragmentainer: column,
                rect: FragmentRect {
                    x: column as f32 * 20.0,
                    y: 0.0,
                    width: 20.0,
                    height: 10.0,
                },
                fragmentainer_clip: None,
                fragment_index: column,
                fragment_count: count,
                line_start: Some(column),
                line_end: Some(column + 1),
            })
            .unwrap();
    }
    (doc, cascade, root)
}

#[test]
fn many_columns_project_with_a_linear_work_budget() {
    use raikiri_traits::NodeId;
    for count in [128, 1000, 5000] {
        let (mut doc, cascade, root) = many_column_fixture(count);
        doc.fragment_tree.limit = (count * 16).min(doc.fragment_tree.limit);
        let started = std::time::Instant::now();
        doc.project_pages(
            &cascade,
            page_box_800x600(),
            &[PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None,
            }],
            &[],
        )
        .unwrap();
        eprintln!("column_projection {count} lines: {:?}", started.elapsed());
        let first_break = doc.nodes[root]
            .children
            .iter()
            .copied()
            .find(|&id| doc.nodes[id].tag_name() == Some("br"))
            .unwrap();
        assert_eq!(
            doc.page_fragments(0)
                .filter(|f| f.node() == NodeId::new(first_break as u64))
                .count(),
            0
        );
        assert_eq!(
            doc.page_fragments(0)
                .filter(|f| f.kind() == crate::FragmentKind::Text)
                .count(),
            count
        );
        if count == 128 {
            assert_eq!(doc.page_text_runs(&cascade, 0).len(), count);
        }
    }
}

#[test]
fn column_projection_cancellation_keeps_the_previous_snapshot() {
    let (mut doc, cascade) = committed_column_projection("");
    let before: Vec<_> = doc
        .page_fragments(0)
        .map(|f| (f.node(), f.rect()))
        .collect();
    let checks = Cell::new(0);
    let abort = || {
        checks.set(checks.get() + 1);
        checks.get() >= 10
    };
    let control = PageLayoutControl::default().with_abort_check(&abort);
    assert!(matches!(
        doc.project_pages_with_control(
            &cascade,
            page_box_800x600(),
            &[PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None
            }],
            &[],
            &control
        ),
        Err(LayoutError::Aborted)
    ));
    assert_eq!(checks.get(), 10);
    assert_eq!(
        doc.page_fragments(0)
            .map(|f| (f.node(), f.rect()))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn column_projection_work_limit_keeps_the_previous_snapshot() {
    let (mut doc, cascade) = committed_column_projection("");
    let before: Vec<_> = doc
        .page_fragments(0)
        .map(|f| (f.node(), f.rect()))
        .collect();
    doc.fragment_tree.limit = 1;
    assert!(matches!(
        doc.project_pages(
            &cascade,
            page_box_800x600(),
            &[PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None
            }],
            &[]
        ),
        Err(LayoutError::FragmentLimitExceeded { limit: 1 })
    ));
    assert_eq!(
        doc.page_fragments(0)
            .map(|f| (f.node(), f.rect()))
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn duplicate_column_placements_charge_their_selected_records_before_copying() {
    use crate::fragment::{FragmentRect, LayoutFragment};
    use crate::node::MulticolTextFragment;
    let count = 128;
    let (mut doc, cascade, root) =
        ahem_paragraph_with("width:20px;white-space:pre", |doc, root| {
            doc.append_text(root, vec!["A"; 128].join("\n"));
        });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    doc.fragment_tree.fragments.clear();
    doc.nodes[root].ifc.as_mut().unwrap().multicol_fragments = Some(vec![MulticolTextFragment {
        line_start: 0,
        line_end: count,
        fragmentainer: 0,
        x: 0.0,
        y: 0.0,
    }]);
    for index in 0..count {
        doc.fragment_tree
            .try_push(LayoutFragment {
                node_id: root,
                parent: None,
                fragmentainer: 0,
                rect: FragmentRect {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 1280.0,
                },
                fragmentainer_clip: None,
                fragment_index: index,
                fragment_count: count,
                line_start: Some(0),
                line_end: Some(count),
            })
            .unwrap();
    }
    doc.fragment_tree.limit = 1024;
    assert!(matches!(
        doc.project_pages(
            &cascade,
            page_box_800x600(),
            &[PageSlice {
                page_index: 0,
                content_origin_y: 0.0,
                page_name: None
            }],
            &[]
        ),
        Err(LayoutError::FragmentLimitExceeded { limit: 1024 })
    ));
    assert_eq!(doc.page_fragments(0).count(), 0);
}

fn projected_decoration_fixture(image_marker: bool) -> (Document, CascadeResult) {
    use raikiri_traits::{DecodedImage, ImagePixelSource};
    use std::sync::Arc;
    struct MarkerPixels;
    impl ImagePixelSource for MarkerPixels {
        fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
            Some(Arc::new(DecodedImage {
                width: 4,
                height: 4,
                rgba: [255, 0, 0, 255].repeat(16),
            }))
        }
    }
    let crate::layout::ifc::test_support::Fixture {
        mut doc,
        cascade,
        root,
    } = crate::layout::ifc::test_support::sheet_fixture(
        if image_marker {
            ""
        } else {
            "div::before{content:'X';background:red}"
        },
        if image_marker {
            "display:list-item;list-style:inside url(https://images.test/marker.png);width:40px;line-height:10px"
        } else {
            "width:40px;line-height:10px"
        },
        |doc, root| {
            for (index, text) in ["A", "B", "C", "D"].into_iter().enumerate() {
                if index > 0 {
                    doc.append_element(Some(root), "br", Style::default(), Some("display:inline"));
                }
                doc.append_text(root, text);
            }
        },
    );
    if image_marker {
        doc.prepare_list_marker_images(&cascade, &MarkerPixels, None);
    }
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    set_committed_column_fragments(&mut doc, root);
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    (doc, cascade)
}

#[test]
fn decoration_projection_uses_the_remaining_budget_and_preserves_the_snapshot() {
    for image_marker in [false, true] {
        let (mut doc, cascade) = projected_decoration_fixture(image_marker);
        let before: Vec<_> = doc
            .page_fragments(0)
            .map(|f| (f.node(), f.rect()))
            .collect();
        let slices = [PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }];
        let (_, _, _, _, _, remaining) = project_slices_with_control(
            &doc,
            &cascade,
            page_box_800x600(),
            &slices,
            &[],
            &PageLayoutControl::default(),
        )
        .unwrap();
        let limit = doc.fragment_tree.limit - remaining;
        doc.fragment_tree.limit = limit;
        assert!(matches!(
            doc.project_pages(&cascade, page_box_800x600(), &slices, &[]),
            Err(LayoutError::FragmentLimitExceeded { limit: actual }) if actual == limit
        ));
        assert_eq!(
            doc.page_fragments(0)
                .map(|f| (f.node(), f.rect()))
                .collect::<Vec<_>>(),
            before
        );
    }
}

#[test]
fn decoration_projection_polls_cancellation_after_geometry_preparation() {
    for image_marker in [false, true] {
        let (mut doc, cascade) = projected_decoration_fixture(image_marker);
        let before: Vec<_> = doc
            .page_fragments(0)
            .map(|f| (f.node(), f.rect()))
            .collect();
        let slices = [PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }];
        let checks = Cell::new(0);
        let probe = || {
            checks.set(checks.get() + 1);
            false
        };
        project_slices_with_control(
            &doc,
            &cascade,
            page_box_800x600(),
            &slices,
            &[],
            &PageLayoutControl::default().with_abort_check(&probe),
        )
        .unwrap();
        let geometry_checks = checks.replace(0);
        let abort = || {
            checks.set(checks.get() + 1);
            checks.get() > geometry_checks
        };
        assert!(matches!(
            doc.project_pages_with_control(
                &cascade,
                page_box_800x600(),
                &slices,
                &[],
                &PageLayoutControl::default().with_abort_check(&abort)
            ),
            Err(LayoutError::Aborted)
        ));
        assert_eq!(checks.get(), geometry_checks + 1);
        assert_eq!(
            doc.page_fragments(0)
                .map(|f| (f.node(), f.rect()))
                .collect::<Vec<_>>(),
            before
        );
    }
}

#[test]
fn column_generated_eligibility_charges_its_ancestor_walk_once() {
    let (mut doc, mut cascade) = projected_decoration_fixture(false);
    let root = doc.page_text_runs(&cascade, 0)[0].line.root.0 as usize;
    let parent = doc.parent_of(root).unwrap();
    let inherited = cascade.computed[parent].clone();
    doc.nodes[parent].children.retain(|&child| child != root);
    let depth = 128;
    let mut last = parent;
    for _ in 0..depth {
        last = doc.append_element(
            Some(last),
            "section",
            Style::default(),
            Some("display:block"),
        );
        cascade.computed.resize(doc.nodes.len(), inherited.clone());
    }
    doc.nodes[last].children.push(root);
    doc.nodes[root].parent = Some(last);
    doc.mark_in_document_flags();
    let slices = [PageSlice {
        page_index: 0,
        content_origin_y: 0.0,
        page_name: None,
    }];
    let (_, _, _, _, _, remaining) = project_slices_with_control(
        &doc,
        &cascade,
        page_box_800x600(),
        &slices,
        &[],
        &PageLayoutControl::default(),
    )
    .unwrap();
    let geometry_cost = doc.fragment_tree.limit - remaining;
    doc.fragment_tree.limit = geometry_cost + depth + 32;
    doc.project_pages(&cascade, page_box_800x600(), &slices, &[])
        .unwrap();
    let events = doc.page_paint_order(&cascade, 0, None);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, crate::PaintEvent::GeneratedBox(_)))
            .count(),
        1
    );
}

#[test]
fn unassigned_column_lines_have_no_glyphs_or_generated_decorations() {
    let (mut doc, cascade) = projected_decoration_fixture(false);
    let root = doc.page_text_runs(&cascade, 0)[0].line.root.0 as usize;
    doc.nodes[root]
        .ifc
        .as_mut()
        .unwrap()
        .multicol_fragments
        .as_mut()
        .unwrap()
        .retain(|range| range.fragmentainer == 1);
    doc.project_pages(
        &cascade,
        page_box_800x600(),
        &[PageSlice {
            page_index: 0,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    assert_eq!(
        doc.page_text_runs(&cascade, 0)
            .iter()
            .map(|run| (run.text, run.origin))
            .collect::<Vec<_>>(),
        [("C", (60.0, 8.0)), ("D", (60.0, 18.0))]
    );
    assert!(
        !doc.page_paint_order(&cascade, 0, None)
            .iter()
            .any(|event| matches!(event, crate::PaintEvent::GeneratedBox(_)))
    );
}

#[test]
fn marker_cache_preparation_obeys_each_work_budget_boundary() {
    for limit in 0..=64 {
        let (doc, cascade) = projected_decoration_fixture(true);
        let root = doc.page_text_runs(&cascade, 0)[0].line.root.0 as usize;
        let control = PageLayoutControl::default();
        let mut work = column_projection::ProjectionWork::new(&control, limit);
        let result =
            column_projection::ParagraphProjection::prepare(&doc, &cascade, root, 0, &mut work);
        if limit == 0 {
            assert!(matches!(
                result,
                Err(LayoutError::FragmentLimitExceeded { limit: 0 })
            ));
        } else if limit == 64 {
            assert!(result.is_ok());
        }
        match result {
            Err(LayoutError::FragmentLimitExceeded { limit: actual }) => assert_eq!(actual, limit),
            Ok(cache) => assert_eq!(cache.markers(Some(0)).len(), 1),
            Err(error) => panic!("unexpected marker cache error: {error:?}"),
        }
    }
}

#[test]
fn repeated_header_copies_are_charged_to_the_projection_budget() {
    let (mut doc, _, root) = crate::layout::test_support::ahem_paragraph("A", "");
    let body = doc.parent_of(root).unwrap();
    let table = doc.append_element(
        Some(body),
        "table",
        Style::default(),
        Some("display:table;width:100px;border-collapse:collapse"),
    );
    let header = doc.append_element(
        Some(table),
        "thead",
        Style::default(),
        Some("display:table-header-group"),
    );
    let row = doc.append_element(
        Some(header),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "td",
        Style::default(),
        Some("display:table-cell;padding:0"),
    );
    doc.append_child(cell, root).unwrap();
    let body_row = doc.append_element(
        Some(table),
        "tr",
        Style::default(),
        Some("display:table-row"),
    );
    doc.append_element(
        Some(body_row),
        "td",
        Style::default(),
        Some("display:table-cell;height:200px;padding:0"),
    );
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    doc.table_objects.headers =
        crate::layout::table::headers::HeaderRepeats::prepare(&doc, &cascade, body, 600.0, |_| 0.0);
    let repeated = doc.table_objects.headers.headers.get_mut(&header).unwrap();
    let pages = 64;
    let slices: Vec<_> = (0..pages)
        .map(|page| {
            repeated
                .placements
                .insert(page, (page as f32 * 100.0, page as f32 * 100.0));
            PageSlice {
                page_index: page,
                content_origin_y: page as f32 * 100.0,
                page_name: None,
            }
        })
        .collect();
    doc.table_objects.headers.index();
    doc.project_pages(&cascade, page_box_800x600(), &slices, &[])
        .unwrap();
    let before: Vec<_> = doc
        .page_fragments(pages - 1)
        .map(|f| (f.node(), f.rect()))
        .collect();
    assert!(before.iter().any(|&(node, _)| node.0 as usize == header));
    // Four header boxes on each of 64 pages exceed this limit; no ordinary
    // box needs a charge in this document.
    doc.fragment_tree.limit = 128;
    assert!(matches!(
        doc.project_pages(&cascade, page_box_800x600(), &slices, &[]),
        Err(LayoutError::FragmentLimitExceeded { limit: 128 })
    ));
    assert_eq!(
        doc.page_fragments(pages - 1)
            .map(|f| (f.node(), f.rect()))
            .collect::<Vec<_>>(),
        before
    );
}
