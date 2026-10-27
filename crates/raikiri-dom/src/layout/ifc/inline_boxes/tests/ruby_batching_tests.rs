use super::*;
use crate::layout::ifc::test_support::block_fixture_with_ahem;
use crate::layout::layout_single_page;
use crate::layout::test_support::page_box_800x600;

fn independent_ruby_fixture(count: usize) -> crate::layout::ifc::test_support::Fixture {
    block_fixture_with_ahem("white-space:nowrap;line-height:20px", |doc, root| {
        for _ in 0..count {
            let ruby = doc.append_element(Some(root), "ruby", Some("display:ruby"));
            doc.append_text(ruby, "a");
            let rt = doc.append_element(Some(ruby), "rt", Some("display:ruby-text;font-size:5px"));
            doc.append_text(rt, "x");
        }
    })
}

#[test]
fn independent_annotations_on_one_physical_line_have_linear_insertion_work() {
    let mut samples = Vec::new();
    for count in [32, 64, 128] {
        let fixture = independent_ruby_fixture(count);
        let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
        work::take();
        layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
        let work = work::take();
        let pieces = doc.nodes[root].ifc_inline_boxes().expect("recorded pieces");
        assert!(pieces.iter().all(|piece| piece.line == 0), "{count}");
        assert_eq!(
            pieces
                .iter()
                .filter(|piece| doc.nodes[piece.node].tag_name() == Some("rt"))
                .count(),
            count
        );
        eprintln!("ruby count={count} pieces={} work={work:?}", pieces.len());
        samples.push((count, pieces.len(), work));
    }
    for (count, pieces, work) in samples {
        assert!(
            work.parent_entries
                + work.moved_pieces
                + work.end_updates
                + work.inspected
                + work.copied
                <= 64 * pieces,
            "insertion must use linear work: count={count}, pieces={pieces}, work={work:?}"
        );
        assert_eq!(work.batch_lines, 1, "actual independent ruby must batch");
        assert_eq!(work.fallback_lines, 0);
    }
}

#[test]
#[ignore = "manual inspection of production ruby fragment metadata"]
fn inspect_independent_ruby_fragment_metadata() {
    let fixture = independent_ruby_fixture(3);
    let mut cx = LayoutContext::new();
    let projected = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut cx,
        &ahem_fonts(),
        &Limits::default(),
    )
    .expect("project");
    let root = IfcRoot::new(projected);
    let lines = break_lines(&root, &mut cx, 800.0).lines.to_vec();
    assert_eq!(lines.len(), 1);
    for fragment in lines[0].fragments() {
        match fragment {
            Fragment::InlineBox(piece) => {
                eprintln!("base node={} parent={:?}", piece.node.0, piece.parent)
            }
            Fragment::RubyAnnotation(annotation) => {
                eprintln!("annotation container={}", annotation.container().0);
                for fragment in annotation.line().fragments() {
                    if let Fragment::InlineBox(piece) = fragment {
                        eprintln!("  own node={} parent={:?}", piece.node.0, piece.parent);
                    }
                }
            }
            _ => {}
        }
    }
}

fn piece(node: usize, parent: Option<usize>, salt: usize) -> InlineBoxPiece {
    let rect = BoxRect {
        x: salt as f32,
        y: salt as f32 + 0.25,
        width: 3.0,
        height: 5.0,
    };
    InlineBoxPiece {
        node,
        line: salt % 3,
        border_box: rect,
        content_box: BoxRect { width: 2.0, ..rect },
        has_start_edge: salt.is_multiple_of(2),
        has_end_edge: salt.is_multiple_of(3),
        parent,
        source_container: Some(salt + 200),
        source_owner: Some(salt + 300),
    }
}

// Keep the former algorithm as an independent oracle, including the parent
// table rebuilt after each insertion and the shifting cached insertion ends.
fn legacy_insert(
    pieces: &mut Vec<InlineBoxPiece>,
    line_start: usize,
    annotations: Vec<(usize, Vec<InlineBoxPiece>)>,
) {
    let mut ends: HashMap<usize, usize> = HashMap::new();
    for (container, own) in annotations {
        let at = match ends.get(&container) {
            Some(&at) => at,
            None => {
                let line = &pieces[line_start..];
                let parents: HashMap<usize, Option<usize>> = line
                    .iter()
                    .map(|piece| (piece.node, piece.parent))
                    .collect();
                let inside = |mut node: Option<usize>| {
                    for _ in 0..=parents.len() {
                        let Some(id) = node else { return false };
                        if id == container {
                            return true;
                        }
                        node = parents.get(&id).copied().flatten();
                    }
                    false
                };
                match line.iter().position(|piece| piece.node == container) {
                    Some(start) => {
                        let mut end = line_start + start + 1;
                        while end < pieces.len() && inside(pieces[end].parent) {
                            end += 1;
                        }
                        end
                    }
                    None => pieces.len(),
                }
            }
        };
        let count = own.len();
        pieces.splice(at..at, own);
        for end in ends.values_mut() {
            if *end >= at {
                *end += count;
            }
        }
        ends.insert(container, at + count);
    }
}

#[test]
fn every_piece_field_matches_sequential_insertion_with_conflicting_metadata() {
    let fixtures = [
        // The first contiguous descendant run ends before the second child.
        (
            vec![(1, None), (2, Some(1)), (3, None), (4, Some(1))],
            vec![(1, vec![(8, None)]), (3, vec![(9, None)])],
        ),
        // Nested containers require the original interaction between inserts.
        (
            vec![(1, None), (2, Some(1)), (3, Some(2)), (4, None)],
            vec![(2, vec![(8, Some(2))]), (1, vec![(9, Some(1))])],
        ),
        // Last piece wins when the same node has different parents.
        (
            vec![(1, None), (2, Some(1)), (2, None), (3, Some(2))],
            vec![(1, vec![(8, None)]), (2, vec![(9, None)])],
        ),
        // The first insertion changes a base node's later ancestry.
        (
            vec![(1, None), (2, None), (3, Some(1)), (4, None)],
            vec![(1, vec![(3, Some(2))]), (2, vec![(9, None)])],
        ),
        // Self-parent annotation wrappers must retain their exact metadata.
        (
            vec![(1, None), (2, None)],
            vec![(1, vec![(8, None), (8, Some(8))]), (2, vec![(9, None)])],
        ),
        // Missing containers, forward parents and cycles use the old bounds.
        (
            vec![(1, Some(2)), (2, Some(1)), (3, None)],
            vec![(1, vec![(8, None)]), (7, vec![(9, None)])],
        ),
    ];
    for (base, annotations) in fixtures {
        for prefix in [0, 2] {
            let mut initial: Vec<_> = (0..prefix).map(|i| piece(100 + i, None, i)).collect();
            initial.extend(
                base.iter()
                    .enumerate()
                    .map(|(i, &(id, parent))| piece(id, parent, i + 20)),
            );
            let annotations: Vec<_> = annotations
                .iter()
                .enumerate()
                .map(|(i, (id, own))| {
                    (
                        *id,
                        own.iter()
                            .enumerate()
                            .map(|(j, &(node, parent))| piece(node, parent, 100 + i * 10 + j))
                            .collect(),
                    )
                })
                .collect();
            let mut expected = initial.clone();
            legacy_insert(&mut expected, prefix, annotations.clone());
            insert_annotation_pieces(&mut initial, prefix, annotations);
            assert_eq!(initial, expected);
        }
    }
}

#[test]
#[ignore = "manual public-layout and paint-box performance measurements"]
fn measure_public_ruby_insertion() {
    let count: usize = std::env::var("RAIKIRI_RUBY_COUNT")
        .unwrap_or_else(|_| "8".into())
        .parse()
        .expect("integer count");
    let repeats: usize = std::env::var("RAIKIRI_RUBY_REPEATS")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .expect("integer repeats");
    let mode = std::env::var("RAIKIRI_RUBY_MODE").unwrap_or_else(|_| "ruby".into());
    let fixture = if mode == "none" {
        block_fixture_with_ahem("white-space:nowrap;line-height:20px", |doc, root| {
            for _ in 0..count {
                let span = doc.append_element(Some(root), "span", Some("display:inline"));
                doc.append_text(span, "a");
            }
        })
    } else {
        independent_ruby_fixture(if mode == "shallow" { 1 } else { count })
    };
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    work::take();
    let start = std::time::Instant::now();
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    let layout_ns = start.elapsed().as_nanos();
    let layout_work = work::take();
    let mut times = Vec::with_capacity(repeats);
    let mut digest = 0_u64;
    let mut piece_count = 0;
    for _ in 0..repeats {
        let start = std::time::Instant::now();
        let pieces =
            std::hint::black_box(doc.nodes[root].ifc_inline_boxes().expect("recorded boxes"));
        times.push(start.elapsed().as_nanos());
        assert!(pieces.iter().all(|piece| piece.line == 0));
        piece_count = pieces.len();
        digest = 0;
        for piece in &pieces {
            for field in [
                piece.node as u64,
                piece.line as u64,
                piece.parent.unwrap_or(usize::MAX) as u64,
                piece.border_box.x.to_bits() as u64,
                piece.border_box.y.to_bits() as u64,
                piece.border_box.width.to_bits() as u64,
                piece.border_box.height.to_bits() as u64,
                piece.content_box.x.to_bits() as u64,
                piece.content_box.y.to_bits() as u64,
                piece.content_box.width.to_bits() as u64,
                piece.content_box.height.to_bits() as u64,
                u64::from(piece.has_start_edge),
                u64::from(piece.has_end_edge),
                piece.source_container.unwrap_or(usize::MAX) as u64,
                piece.source_owner.unwrap_or(usize::MAX) as u64,
            ] {
                digest = digest.wrapping_mul(1099511628211).wrapping_add(field);
            }
        }
    }
    let paint_work = work::take();
    times.sort_unstable();
    let status = std::fs::read_to_string("/proc/self/status").expect("process status");
    let rss = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .unwrap_or("unknown");
    eprintln!(
        "mode={mode} count={count} repeats={repeats} pieces={piece_count} digest={digest:016x} layout_ns={layout_ns} paint_min_ns={} paint_median_ns={} paint_max_ns={} layout_work={layout_work:?} paint_work={paint_work:?} rss={rss}",
        times[0],
        times[repeats / 2],
        times[repeats - 1]
    );
}

#[test]
fn seeded_piece_forests_match_every_field_of_sequential_insertion() {
    let mut state = 0x0097_6ace_1984_u64;
    let mut next = |limit: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 32) as usize % limit
    };
    for case in 0..2048 {
        let prefix = next(3);
        let count = 1 + next(12);
        let mut initial: Vec<_> = (0..prefix).map(|i| piece(1000 + i, None, i)).collect();
        for i in 0..count {
            let node = if next(4) == 0 { 1 + next(count) } else { i + 1 };
            let parent = if next(3) == 0 {
                None
            } else {
                Some(1 + next(count))
            };
            initial.push(piece(node, parent, 10 + i));
        }
        let annotation_count = next(8);
        let mut annotations = Vec::new();
        for i in 0..annotation_count {
            let container = 1 + next(count + 1);
            let mut own = Vec::new();
            for j in 0..next(4) {
                let node = if next(3) == 0 {
                    1 + next(count)
                } else {
                    100 + i * 4 + j
                };
                let parent = if next(2) == 0 {
                    None
                } else {
                    Some(1 + next(count))
                };
                own.push(piece(node, parent, 100 + i * 4 + j));
            }
            annotations.push((container, own));
        }
        let mut expected = initial.clone();
        legacy_insert(&mut expected, prefix, annotations.clone());
        insert_annotation_pieces(&mut initial, prefix, annotations);
        assert_eq!(initial, expected, "seeded case {case}");
    }
}

#[test]
fn flat_duplicate_container_wrappers_keep_multiple_annotation_order() {
    let initial = vec![
        piece(1, None, 1),
        piece(2, Some(1), 2),
        piece(2, Some(2), 3),
        piece(3, None, 4),
        piece(4, Some(3), 5),
        piece(4, Some(4), 6),
        piece(5, None, 7),
        piece(6, Some(5), 8),
        piece(6, Some(6), 9),
    ];
    for order in [[2, 4, 6], [6, 4, 2], [4, 2, 6], [6, 2, 4]] {
        let mut annotations = Vec::new();
        for (i, container) in order.into_iter().enumerate() {
            annotations.push((container, vec![piece(100 + i, None, 100 + i)]));
            annotations.push((container, vec![piece(200 + i, None, 200 + i)]));
        }
        let mut actual = initial.clone();
        let mut expected = initial.clone();
        legacy_insert(&mut expected, 0, annotations.clone());
        insert_annotation_pieces(&mut actual, 0, annotations);
        assert_eq!(actual, expected);
    }
}

fn public_fixture(kind: &str) -> crate::layout::ifc::test_support::Fixture {
    let css = match kind {
        "bidi" => "white-space:nowrap;direction:rtl;unicode-bidi:bidi-override",
        "vertical" => "white-space:nowrap;writing-mode:vertical-rl;width:100px;height:200px",
        "multiline" => "width:40px",
        _ => "white-space:nowrap",
    };
    block_fixture_with_ahem(&format!("line-height:20px;{css}"), |doc, root| {
        for _ in 0..12 {
            let ruby = doc.append_element(
                Some(root),
                "ruby",
                Some("display:ruby;background:blue;padding:0 1px"),
            );
            if kind == "nested-base" {
                doc.append_text(ruby, "b");
                let inner = doc.append_element(Some(ruby), "ruby", Some("display:ruby"));
                doc.append_text(inner, "a");
                let rt =
                    doc.append_element(Some(inner), "rt", Some("display:ruby-text;font-size:5px"));
                doc.append_text(rt, "x");
            } else {
                doc.append_text(ruby, "a");
            }
            for _ in 0..if kind == "multiple" { 2 } else { 1 } {
                let rt = doc.append_element(
                    Some(ruby),
                    "rt",
                    Some("display:ruby-text;font-size:5px;background:red"),
                );
                if kind == "nested-annotation" {
                    let inner = doc.append_element(Some(rt), "ruby", Some("display:ruby"));
                    doc.append_text(inner, "x");
                    let nested = doc.append_element(
                        Some(inner),
                        "rt",
                        Some("display:ruby-text;font-size:3px"),
                    );
                    doc.append_text(nested, "y");
                } else if kind != "empty" {
                    let span =
                        doc.append_element(Some(rt), "span", Some("display:inline;padding:0 1px"));
                    doc.append_text(span, "x");
                }
            }
            if kind == "multiline" {
                doc.append_text(root, " ");
            }
        }
    })
}

#[test]
fn public_layout_and_paint_geometry_match_legacy_in_every_writing_scenario() {
    for kind in [
        "flat",
        "nested-base",
        "nested-annotation",
        "multiple",
        "bidi",
        "vertical",
        "multiline",
        "empty",
    ] {
        let fixture = public_fixture(kind);
        let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
        work::take();
        layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("batch layout");
        let actual = doc.nodes[root]
            .ifc_inline_boxes()
            .expect("batch paint pieces");
        let actual_layouts: Vec<_> = doc.nodes.iter().map(|node| *node.layout()).collect();
        let measured = work::take();
        let (expected, expected_layouts) = work::with_legacy(|| {
            let fixture = public_fixture(kind);
            let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
            layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("legacy layout");
            let pieces = doc.nodes[root]
                .ifc_inline_boxes()
                .expect("legacy paint pieces");
            let layouts: Vec<_> = doc.nodes.iter().map(|node| *node.layout()).collect();
            (pieces, layouts)
        });
        assert_eq!(actual, expected, "all paint piece fields: {kind}");
        assert_eq!(
            actual_layouts, expected_layouts,
            "all recorded node layouts: {kind}"
        );
        assert!(!actual.is_empty(), "{kind}");
        if kind == "multiline" {
            assert!(
                actual.iter().any(|piece| piece.line > 0),
                "multiple physical lines"
            );
        } else {
            assert!(actual.iter().all(|piece| piece.line == 0), "{kind}");
        }
        if kind == "flat" {
            assert!(
                measured.batch_lines > 0,
                "public flat fixture must activate batch"
            );
            assert_eq!(measured.fallback_lines, 0);
        }
        if kind == "nested-base" {
            assert!(
                measured.fallback_lines > 0,
                "nested containers retain the legacy path"
            );
        }
        eprintln!("scenario={kind} pieces={} work={measured:?}", actual.len());
    }
}

#[test]
fn seeded_independent_batches_activate_and_preserve_noncontiguous_groups() {
    let mut state = 0x0976_5eed_u64;
    let mut next = |limit: usize| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state >> 32) as usize % limit
    };
    for case in 0..256 {
        let count = 2 + next(7);
        let prefix = next(3);
        let mut initial: Vec<_> = (0..prefix).map(|i| piece(900 + i, None, i)).collect();
        let mut containers = Vec::new();
        for i in 0..count {
            let root = 10 * i;
            let container = root + 1;
            containers.push(container);
            initial.push(piece(root, None, i));
            initial.push(piece(container, Some(root), i + 20));
            initial.push(piece(container, Some(container), i + 40));
            initial.push(piece(root + 2, Some(container), i + 60));
            if next(2) == 0 {
                initial.push(piece(root + 2, Some(container), i + 80));
            }
            initial.push(piece(root + 3, None, i + 100));
            // This later descendant must not move the first group's end.
            initial.push(piece(root + 4, Some(container), i + 120));
        }
        for i in (1..count).rev() {
            containers.swap(i, next(i + 1));
        }
        let mut annotations = Vec::new();
        for (i, &container) in containers.iter().enumerate() {
            for level in 0..1 + next(3) {
                // Repeated own IDs with different, foreign parent chains are
                // harmless because none of them shadows any base node.
                let own = vec![piece(
                    10_000 + level,
                    Some(containers[next(count)]),
                    i + level + 200,
                )];
                annotations.push((container, own));
            }
        }
        let mut expected = initial.clone();
        legacy_insert(&mut expected, prefix, annotations.clone());
        work::take();
        insert_annotation_pieces(&mut initial, prefix, annotations);
        let measured = work::take();
        assert_eq!(initial, expected, "independent case {case}");
        assert_eq!(measured.batch_lines, 1, "independent case {case}");
        assert_eq!(measured.fallback_lines, 0, "independent case {case}");
    }
}
