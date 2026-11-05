//! The canvas applies the HTML root's CSS transform once, outside all
//! paint events, text-run transforms and link paint coordinates.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, Page, PaintEvent,
    RenderResources, RunSource, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, NodeId, PageDefaults, PaintTransform};

fn lay_out(root_style: &str, child_style: &str) -> DocumentLayout {
    lay_out_with_body(root_style, child_style, "")
}

fn lay_out_with_body(root_style: &str, child_style: &str, body_style: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page {{size:200px 100px;margin:0}} \
         html {{width:200px;height:80px;transform-origin:0 0;{root_style}}} \
         body {{margin:0;font:10px/10px Ahem;{body_style}}} \
         #probe {{display:block;width:40px;height:20px;background:red;\
         transform-origin:0 0;{child_style}}}</style>\
         <a id=probe href='https://example.test/'>AB</a>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("expected complete layout")
    };
    result
}

#[track_caller]
fn close(actual: PaintTransform, expected: PaintTransform) {
    for (actual, expected) in [actual.a, actual.b, actual.c, actual.d, actual.e, actual.f]
        .into_iter()
        .zip([
            expected.a, expected.b, expected.c, expected.d, expected.e, expected.f,
        ])
    {
        assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
    }
}

#[track_caller]
fn close_point(actual: (f32, f32), expected: (f32, f32)) {
    assert!(
        (actual.0 - expected.0).abs() < 0.02 && (actual.1 - expected.1).abs() < 0.02,
        "{actual:?} != {expected:?}"
    );
}

fn event_transform(page: &Page<'_>, node: NodeId) -> PaintTransform {
    let mut stack = vec![PaintTransform::IDENTITY];
    let mut found = None;
    for event in page.paint_order() {
        match event {
            PaintEvent::PushTransform(transform) => stack.push(*stack.last().unwrap() * transform),
            PaintEvent::PopTransform => {
                assert!(stack.len() > 1);
                stack.pop();
            }
            PaintEvent::Box(fragment) if fragment.node() == node => {
                assert!(found.is_none());
                found = stack.last().copied();
            }
            _ => {}
        }
    }
    assert_eq!(stack.len(), 1);
    found.expect("the probe box must have an event")
}

fn check_inner(page: &Page<'_>, expected: PaintTransform) -> (NodeId, PaintTransform) {
    let probe = page
        .fragments()
        .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("probe"))
        .unwrap()
        .node();
    let event = event_transform(page, probe);
    close(event, expected);
    close(page.paint_transform(probe).unwrap(), expected);
    let runs = page.text_runs();
    let run = runs
        .iter()
        .find(|run| run.text == "AB")
        .expect("probe text");
    close(run.transform.unwrap_or(PaintTransform::IDENTITY), expected);
    let RunSource::Text(text) = run.source else {
        panic!("probe has ordinary source text")
    };
    close(page.paint_transform(text).unwrap(), expected);
    let canvas = page
        .root_box()
        .unwrap()
        .transform
        .unwrap_or(PaintTransform::IDENTITY);
    let links = page
        .links()
        .filter(|link| link.owner == probe)
        .collect::<Vec<_>>();
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert!(!link.quads.is_empty());
    assert_eq!(link.quads.len(), link.paint_quads.len());
    for (rect, painted) in link.quads.iter().zip(&link.paint_quads) {
        let corners = [
            (rect.x, rect.y),
            (rect.x + rect.width, rect.y),
            (rect.x + rect.width, rect.y + rect.height),
            (rect.x, rect.y + rect.height),
        ];
        for (corner, painted) in corners.into_iter().zip(painted) {
            close_point(*painted, expected.apply(corner));
            close_point(canvas.apply(*painted), (canvas * event).apply(corner));
        }
    }
    (probe, canvas)
}

#[test]
fn the_canvas_owns_root_translate_scale_rotate_and_singular_transforms() {
    for (style, expected_canvas) in [
        (
            "transform:translate(40px,30px)",
            PaintTransform::translate(40.0, 30.0),
        ),
        (
            "transform:scale(2,3)",
            PaintTransform::new(2.0, 0.0, 0.0, 3.0, 0.0, 0.0),
        ),
        (
            "transform:rotate(90deg)",
            PaintTransform::new(0.0, 1.0, -1.0, 0.0, 0.0, 0.0),
        ),
        (
            "transform:scale(0)",
            PaintTransform::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        ),
        ("", PaintTransform::IDENTITY),
    ] {
        let result = lay_out(style, "");
        assert_eq!(result.pages().count(), 1, "{style}");
        let page = result.page(0).unwrap();
        let (_, canvas) = check_inner(&page, PaintTransform::IDENTITY);
        close(canvas, expected_canvas);
    }
}

#[test]
fn root_relative_layout_and_nested_rotation_are_kept_inside_the_canvas() {
    let result = lay_out(
        "position:relative;left:11px;top:13px;transform:translate(40px,30px)",
        "transform:rotate(90deg)",
    );
    let page = result.page(0).unwrap();
    let (probe, canvas) = check_inner(&page, PaintTransform::new(0.0, 1.0, -1.0, 0.0, 24.0, 2.0));
    let rect = page
        .fragments()
        .find(|fragment| fragment.node() == probe)
        .unwrap()
        .rect();
    close_point((rect.x, rect.y), (11.0, 13.0));
    close_point(
        (canvas * event_transform(&page, probe)).apply((21.0, 13.0)),
        (51.0, 53.0),
    );
}

#[test]
fn a_singular_descendant_still_hides_its_inner_content() {
    let result = lay_out("transform:translate(40px,30px)", "transform:scale(0)");
    let page = result.page(0).unwrap();
    let probe = page
        .fragments()
        .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("probe"))
        .unwrap()
        .node();
    assert!(page.paint_transform(probe).is_none());
    assert!(
        !page
            .paint_order()
            .iter()
            .any(|event| matches!(event, PaintEvent::Box(fragment) if fragment.node() == probe))
    );
    assert!(!page.text_runs().iter().any(|run| run.text == "AB"));
    assert!(page.links().all(|link| link.paint_quads.is_empty()));
}

#[test]
fn a_non_html_document_keeps_its_root_transform_inside_the_events() {
    let mut document = raikiri_dom::Document::new();
    let root = document.append_element(
        Some(document.root_index()),
        "article",
        Some("display:block;width:100px;height:60px;transform:translate(7px,9px)"),
    );
    let body = document.append_element(Some(root), "body", Some("margin:0"));
    let child = document.append_element(
        Some(body),
        "div",
        Some("display:block;width:40px;height:20px;transform:translate(3px,4px)"),
    );
    let cascade =
        raikiri_style::cascade(&document, &raikiri_style::build_rule_tree(&document)).unwrap();
    let page = raikiri_traits::PageBox::A4;
    let slices = raikiri_dom::layout_pages(&mut document, &cascade, page).unwrap();
    document
        .project_pages(&cascade, page, &slices, &[])
        .unwrap();
    assert!(raikiri_dom::root_box(&document, &cascade, page, 0.0, 1).is_none());
    let mut stack = vec![PaintTransform::IDENTITY];
    let mut painted = None;
    for event in document.page_paint_order(&cascade, 0, None) {
        match event {
            PaintEvent::PushTransform(transform) => stack.push(*stack.last().unwrap() * transform),
            PaintEvent::PopTransform => {
                stack.pop();
            }
            PaintEvent::Box(fragment) if fragment.node() == NodeId::new(child as u64) => {
                painted = stack.last().copied();
            }
            _ => {}
        }
    }
    let expected = PaintTransform::translate(10.0, 13.0);
    close(painted.expect("child event"), expected);
    // This control preserves the inner wrapper when no canvas root exists.
}

#[test]
fn root_opacity_and_inner_overflow_clips_are_kept() {
    let baseline = lay_out("opacity:.5;overflow:hidden", "overflow:hidden");
    let result = lay_out(
        "opacity:.5;overflow:hidden;transform:translate(40px,30px)",
        "overflow:hidden",
    );
    let before = baseline.page(0).unwrap();
    let page = result.page(0).unwrap();
    check_inner(&page, PaintTransform::IDENTITY);
    let clips = |page: &Page<'_>| {
        page.paint_order()
            .into_iter()
            .filter_map(|event| {
                if let PaintEvent::PushClip(clip, kind) = event {
                    Some((clip, kind))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert!(!clips(&before).is_empty());
    assert_eq!(clips(&page), clips(&before));
    let mut opacity = 1.0;
    let mut saw_probe = false;
    for event in page.paint_order() {
        match event {
            PaintEvent::PushOpacity(value) => opacity *= value,
            PaintEvent::PopOpacity => opacity = 1.0,
            PaintEvent::Box(fragment)
                if page.dom().attr(fragment.node(), "id") == Some("probe") =>
            {
                assert!((opacity - 0.5).abs() < 0.001);
                saw_probe = true;
            }
            _ => {}
        }
    }
    assert!(saw_probe);
}

#[test]
fn root_transform_still_contains_fixed_descendants() {
    for (root, expected_y) in [("transform:translate(40px,30px)", 60.0), ("", 80.0)] {
        let result = lay_out(root, "position:fixed;right:0;bottom:0");
        let page = result.page(0).unwrap();
        let (probe, _) = check_inner(&page, PaintTransform::IDENTITY);
        let rect = page
            .fragments()
            .find(|fragment| fragment.node() == probe)
            .unwrap()
            .rect();
        close_point((rect.x, rect.y), (160.0, expected_y));
    }
}

#[test]
fn multipage_percentage_and_origin_use_the_external_whole_root_box() {
    for scale in [1.0, 2.0] {
        let result = lay_out(
            &format!("height:250px;transform:translate(50%,50%) scale({scale})"),
            "height:250px",
        );
        assert_eq!(result.pages().count(), 3);
        for page in result.pages() {
            let root = page.root_box().unwrap();
            assert!((root.border_box.height - 250.0).abs() < 0.02);
            let canvas = root.transform.unwrap();
            close(
                canvas,
                PaintTransform::new(
                    scale,
                    0.0,
                    0.0,
                    scale,
                    100.0,
                    125.0 + (scale - 1.0) * 100.0 * page.index() as f32,
                ),
            );
            let probe = page
                .fragments()
                .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("probe"))
                .unwrap()
                .node();
            close(event_transform(&page, probe), PaintTransform::IDENTITY);
            close(
                page.paint_transform(probe).unwrap(),
                PaintTransform::IDENTITY,
            );
            close_point(
                (canvas * event_transform(&page, probe)).apply((0.0, 0.0)),
                canvas.apply((0.0, 0.0)),
            );
        }
    }
}

#[test]
fn root_column_style_keeps_the_existing_internal_motion_policy() {
    let baseline = lay_out("columns:2;column-gap:0", "transform:translate(7px,9px)");
    let result = lay_out(
        "columns:2;column-gap:0;transform:translate(40px,30px)",
        "transform:translate(7px,9px)",
    );
    let before = baseline.page(0).unwrap();
    let page = result.page(0).unwrap();
    check_inner(&before, PaintTransform::translate(7.0, 9.0));
    let (_, canvas) = check_inner(&page, PaintTransform::translate(7.0, 9.0));
    close(canvas, PaintTransform::translate(40.0, 30.0));
    let clips = |page: &Page<'_>| {
        page.paint_order()
            .into_iter()
            .filter_map(|event| {
                if let PaintEvent::PushClip(clip, kind) = event {
                    Some((clip, kind))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(clips(&page), clips(&before));
    let initial = before.text_runs();
    let moved = page.text_runs();
    close_point(
        initial.iter().find(|run| run.text == "AB").unwrap().origin,
        moved.iter().find(|run| run.text == "AB").unwrap().origin,
    );
}

#[test]
fn root_relative_placement_is_preserved_for_float_table_vertical_and_columns() {
    for extra in [
        "",
        "float:left",
        "display:table",
        "display:table-row",
        "writing-mode:vertical-rl",
        "columns:2;column-gap:0",
    ] {
        let style = format!("position:relative;left:11px;top:13px;{extra}");
        let baseline = lay_out(&style, "transform:rotate(90deg)");
        let result = lay_out(
            &format!("{style};transform:translate(40px,30px)"),
            "transform:rotate(90deg)",
        );
        let before = baseline.page(0).unwrap();
        let page = result.page(0).unwrap();
        let probe = page
            .fragments()
            .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("probe"))
            .unwrap()
            .node();
        let initial_probe = before
            .fragments()
            .find(|fragment| before.dom().attr(fragment.node(), "id") == Some("probe"))
            .unwrap()
            .node();
        let initial = event_transform(&before, initial_probe);
        let current = event_transform(&page, probe);
        close(current, initial);
        let initial_query = before.paint_transform(initial_probe);
        let current_query = page.paint_transform(probe);
        match (initial_query, current_query) {
            (Some(initial), Some(current)) => close(current, initial),
            (None, None) => {}
            other => panic!("{extra}: {other:?}"),
        }
        let initial_runs = before.text_runs();
        let current_runs = page.text_runs();
        assert_eq!(initial_runs.len(), current_runs.len(), "{extra}");
        for (a, b) in initial_runs.iter().zip(&current_runs) {
            assert_eq!(a.text, b.text);
            close_point(a.origin, b.origin);
            close(
                a.transform.unwrap_or(PaintTransform::IDENTITY),
                b.transform.unwrap_or(PaintTransform::IDENTITY),
            );
        }
        let canvas = page.root_box().unwrap().transform.unwrap();
        close(canvas, PaintTransform::translate(40.0, 30.0));
        close_point(
            (canvas * current).apply((20.0, 10.0)),
            PaintTransform::translate(40.0, 30.0).apply(initial.apply((20.0, 10.0))),
        );
    }
}

#[test]
fn body_motion_and_fixed_omission_keep_their_existing_inner_policy() {
    for body_style in [
        "position:relative;left:5px;top:7px",
        "transform:translate(5px,7px)",
    ] {
        for child_style in ["", "position:fixed;right:0;bottom:0"] {
            let baseline = lay_out_with_body(
                "position:relative;left:11px;top:13px;transform:translate(0px,0px)",
                child_style,
                body_style,
            );
            let result = lay_out_with_body(
                "position:relative;left:11px;top:13px;transform:translate(40px,30px)",
                child_style,
                body_style,
            );
            let before = baseline.page(0).unwrap();
            let page = result.page(0).unwrap();
            let probe = page
                .fragments()
                .find(|fragment| page.dom().attr(fragment.node(), "id") == Some("probe"))
                .unwrap()
                .node();
            let initial_probe = before
                .fragments()
                .find(|fragment| before.dom().attr(fragment.node(), "id") == Some("probe"))
                .unwrap()
                .node();
            let initial = event_transform(&before, initial_probe);
            let current = event_transform(&page, probe);
            close(current, initial);
            close(
                page.paint_transform(probe).unwrap(),
                before.paint_transform(initial_probe).unwrap(),
            );
            let initial_runs = before.text_runs();
            let current_runs = page.text_runs();
            assert_eq!(initial_runs.len(), current_runs.len());
            if child_style.starts_with("position:fixed") && body_style.starts_with("transform") {
                assert!(
                    current_runs.is_empty(),
                    "the existing nested-CB omission stays in effect"
                );
            }
            for (a, b) in initial_runs.iter().zip(&current_runs) {
                assert_eq!(a.text, b.text);
                close_point(a.origin, b.origin);
                close(
                    a.transform.unwrap_or(PaintTransform::IDENTITY),
                    b.transform.unwrap_or(PaintTransform::IDENTITY),
                );
            }
            let initial_links = before
                .links()
                .map(|link| link.paint_quads)
                .collect::<Vec<_>>();
            let current_links = page
                .links()
                .map(|link| link.paint_quads)
                .collect::<Vec<_>>();
            assert_eq!(initial_links, current_links);
            let canvas = page.root_box().unwrap().transform.unwrap();
            close(canvas, PaintTransform::translate(40.0, 30.0));
            close_point(
                (canvas * current).apply((20.0, 10.0)),
                PaintTransform::translate(40.0, 30.0).apply(initial.apply((20.0, 10.0))),
            );
        }
    }
}
