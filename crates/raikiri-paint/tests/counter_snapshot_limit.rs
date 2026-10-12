//! Regression coverage for cumulative counter snapshot resource limits.

use anyrender::Scene;
use raikiri_dom::{
    Document, MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES, counter_snapshots, layout_single_page,
};
use raikiri_paint::{paint_single_page, paint_single_page_with_origin};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::{LayoutError, LimitKind, PageBox, RenderError};
use taffy::Style;

#[test]
fn counter_snapshot_budget_fails_closed_at_api_layout_and_paint_boundaries() {
    const COUNTER_COUNT: usize = 2048;
    const DESCENDANT_COUNT: usize = 1024;

    let mut document = Document::new();
    let html = document.append_element(
        Some(document.root_index()),
        "html",
        Style::default(),
        None::<&str>,
    );
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        "body, div { display: block } body::before { content: counter(c0) }",
    );

    let mut reset = String::from("display: block; counter-reset:");
    for index in 0..COUNTER_COUNT {
        use std::fmt::Write as _;
        write!(reset, " c{index} 0").expect("writing to String succeeds");
    }
    let body = document.append_element(Some(html), "body", Style::default(), Some(reset));
    for _ in 0..DESCENDANT_COUNT {
        document.append_element(Some(body), "div", Style::default(), None::<&str>);
    }
    document.mark_in_document_flags();

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade succeeds");

    let api_error = counter_snapshots(&document, &cascade)
        .expect_err("the public API must reject cumulative snapshot expansion");
    assert_eq!(api_error.limit, MAX_COUNTER_SNAPSHOT_ESTIMATED_BYTES);
    assert!(api_error.actual > api_error.limit);

    let layout_error = layout_single_page(&mut document, &cascade, PageBox::A4)
        .expect_err("IFC must propagate the counter snapshot limit");
    assert!(matches!(
        layout_error,
        LayoutError::CounterSnapshotLimitExceeded { .. }
    ));

    let mut scene = Scene::new();
    let paint_error = paint_single_page(&mut scene, &document, &cascade, PageBox::A4)
        .expect_err("paint must propagate the counter snapshot limit");
    assert!(matches!(
        paint_error,
        RenderError::LimitExceeded {
            kind: LimitKind::CounterSnapshots,
            ..
        }
    ));
    assert!(
        scene.commands.is_empty(),
        "paint fails before emitting commands"
    );

    struct EmptyPixels;
    impl raikiri_traits::ImagePixelSource for EmptyPixels {
        fn get_decoded(
            &self,
            _url: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            None
        }
    }

    let mut image_scene = Scene::new();
    let image_error = raikiri_paint::paint_single_page_with_images_and_warnings(
        &mut image_scene,
        &document,
        &cascade,
        PageBox::A4,
        &EmptyPixels,
    )
    .expect_err("image paint must propagate the counter snapshot limit");
    assert!(matches!(
        image_error,
        RenderError::LimitExceeded {
            kind: LimitKind::CounterSnapshots,
            ..
        }
    ));
    assert!(
        image_scene.commands.is_empty(),
        "image paint fails before emitting commands"
    );
}

/// Every page slice reads the snapshots built once for the document, so a
/// document whose snapshots fit the limit paints any number of pages. Each
/// page used to rebuild them and charge one shared budget, which this
/// document exceeds on its second page.
#[test]
fn multipage_paint_shares_one_counter_snapshot_build() {
    const COUNTER_COUNT: usize = 512;
    const DESCENDANT_COUNT: usize = 1024;

    let mut document = Document::new();
    let html = document.append_element(
        Some(document.root_index()),
        "html",
        Style::default(),
        None::<&str>,
    );
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        "body, div { display: block } body::before { content: counter(c0) }",
    );

    let mut reset = String::from("display: block; counter-reset:");
    for index in 0..COUNTER_COUNT {
        use std::fmt::Write as _;
        write!(reset, " c{index} 0").expect("writing to String succeeds");
    }
    let body = document.append_element(Some(html), "body", Style::default(), Some(reset));
    for _ in 0..DESCENDANT_COUNT {
        document.append_element(Some(body), "div", Style::default(), None::<&str>);
    }
    document.mark_in_document_flags();
    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade succeeds");

    let shared = document
        .shared_counter_snapshots(&cascade)
        .expect("one build stays within the snapshot limit");
    for page in 0..4 {
        let mut scene = Scene::new();
        paint_single_page_with_origin(
            &mut scene,
            &document,
            &cascade,
            PageBox::A4,
            page as f32 * PageBox::A4.height,
        )
        .expect("every page reuses the shared snapshots");
    }
    let again = document
        .shared_counter_snapshots(&cascade)
        .expect("the shared snapshots stay cached");
    assert!(
        std::sync::Arc::ptr_eq(&shared, &again),
        "painting must not rebuild the shared snapshots"
    );
}
