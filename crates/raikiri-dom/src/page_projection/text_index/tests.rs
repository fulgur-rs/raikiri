use super::*;
use crate::layout::ifc::test_support::block_fixture_with_ahem;
use crate::{PageLayoutControl, PageSlice, layout_single_page};
use raikiri_traits::PageBox;
use std::cell::Cell;

fn fixture() -> (Document, CascadeResult, PageBox, usize) {
    let fixture = block_fixture_with_ahem("width:10px;line-height:10px", |doc, root| {
        doc.append_text(root, "X X");
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_single_page(&mut doc, &cascade, page).unwrap();
    doc.project_pages(
        &cascade,
        page,
        &[PageSlice {
            page_index: 7,
            content_origin_y: 0.0,
            page_name: None,
        }],
        &[],
    )
    .unwrap();
    (doc, cascade, page, root)
}

#[test]
fn nonfinite_line_placement_is_not_associated_with_a_page() {
    let (doc, cascade, _, root) = fixture();
    let snapshot = &doc.page_projection;
    let mut roots: Vec<_> = snapshot
        .text_roots
        .iter()
        .filter(|placement| placement.node == root)
        .cloned()
        .collect();
    assert!(!roots.is_empty());
    for root in &mut roots {
        root.y = f32::NAN;
    }
    let control = PageLayoutControl::default();
    let mut work = ProjectionWork::new(&control, 100_000);
    let (index, by_page) = PageTextIndex::prepare(
        &doc,
        &cascade,
        &roots,
        &snapshot.markers,
        &snapshot.pages,
        &mut work,
    )
    .unwrap();
    assert!(index.lines.iter().all(HashMap::is_empty));
    assert!(by_page.iter().all(Vec::is_empty));
}

#[test]
fn cancellation_at_every_preparation_poll_preserves_the_published_snapshot() {
    let (doc, cascade, page, _) = fixture();
    let slices = [PageSlice {
        page_index: 9,
        content_origin_y: 0.0,
        page_name: None,
    }];
    let checks = Cell::new(0);
    let probe = || {
        checks.set(checks.get() + 1);
        false
    };
    let mut completed = doc.clone();
    completed
        .project_pages_with_control(
            &cascade,
            page,
            &slices,
            &[],
            &PageLayoutControl::default().with_abort_check(&probe),
        )
        .unwrap();
    assert!(completed.page_fragments(9).next().is_some());
    let total = checks.get();
    assert!(total > 0);
    let expected: Vec<_> = doc
        .page_text_runs(&cascade, 7)
        .iter()
        .map(|run| run.text.to_owned())
        .collect();
    for stop in 1..=total {
        let mut interrupted = doc.clone();
        let checks = Cell::new(0);
        let abort = || {
            checks.set(checks.get() + 1);
            checks.get() >= stop
        };
        let result = interrupted.project_pages_with_control(
            &cascade,
            page,
            &slices,
            &[],
            &PageLayoutControl::default().with_abort_check(&abort),
        );
        assert!(
            matches!(result, Err(LayoutError::Aborted)),
            "poll {stop} of {total}"
        );
        assert!(interrupted.page_fragments(9).next().is_none());
        let retained: Vec<_> = interrupted
            .page_text_runs(&cascade, 7)
            .iter()
            .map(|run| run.text.to_owned())
            .collect();
        assert_eq!(retained, expected);
    }
}
