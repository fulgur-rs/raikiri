use super::*;
use crate::Document;
use crate::layout::page_pipeline::layout_single_page;
use raikiri_style::{build_rule_tree, cascade};

fn with_fixture(
    float_style: &str,
    prefix: impl FnOnce(&mut Document, usize),
    check: impl FnOnce(&mut LayoutSession<'_>, &CascadeResult, usize, usize, usize),
) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let float = doc.append_element(
        Some(body),
        "div",
        Some(format!(
            "display:block;float:left;width:10px;height:40px;{float_style}"
        )),
    );
    let flow = doc.append_element(Some(body), "div", Some("display:block"));
    prefix(&mut doc, flow);
    let cleared = doc.append_element(
        Some(flow),
        "div",
        Some("display:block;clear:both;height:0;margin-top:5px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    layout_single_page(
        &mut doc,
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session
        .context
        .float_resumption_candidates
        .push((float, flow));
    session.context.float_resumption_boundaries.insert(
        float,
        Boundary {
            page_origin: 100.0,
            relative_offset: 0.0,
        },
    );
    session.context.block_clearance.insert(cleared, true);
    session
        .context
        .clearance_resumption_offsets
        .insert(cleared, 0.0);
    check(&mut session, &cascade, float, flow, cleared);
}

#[test]
fn resumption_requires_a_static_float_and_a_measured_page_boundary() {
    with_fixture(
        "position:relative",
        |_, _| {},
        |session, cascade, _, _, _| {
            assert!(Plan::collect(session, cascade).is_empty());
        },
    );
    with_fixture(
        "",
        |_, _| {},
        |session, cascade, _, _, _| {
            session.context.float_resumption_boundaries.clear();
            assert!(Plan::collect(session, cascade).is_empty());
        },
    );
}

#[test]
fn incomplete_clearance_measurements_do_not_complete_a_box() {
    with_fixture(
        "",
        |_, _| {},
        |session, cascade, _, _, cleared| {
            session.context.block_clearance.clear();
            assert!(Plan::collect(session, cascade).is_empty());
            session.context.block_clearance.insert(cleared, true);
            session.context.clearance_resumption_offsets.clear();
            assert!(Plan::collect(session, cascade).is_empty());
        },
    );
}

#[test]
fn the_resumed_flow_can_itself_be_the_empty_clearing_box() {
    with_fixture(
        "",
        |_, _| {},
        |session, cascade, float, flow, cleared| {
            session.context.float_resumption_candidates = vec![(float, cleared)];
            let plan = Plan::collect(session, cascade);
            let local = plan.completed_at(cleared, 0.0).unwrap();
            assert_eq!(local + current_abs_y(session, flow), 100.0);
        },
    );
}

#[test]
fn nonfinite_page_or_float_positions_cannot_become_resumption_placements() {
    for (page_origin, float_y) in [(f32::INFINITY, 0.0), (100.0, f32::NAN)] {
        with_fixture(
            "",
            |_, _| {},
            |session, cascade, float, _, _| {
                session
                    .context
                    .float_resumption_boundaries
                    .get_mut(&float)
                    .unwrap()
                    .page_origin = page_origin;
                session.nodes[float].layout.location.y = float_y;
                assert!(Plan::collect(session, cascade).is_empty());
            },
        );
    }
}

#[test]
fn invisible_and_positioned_prefixes_do_not_hide_a_later_clearing_box() {
    with_fixture(
        "",
        |doc, flow| {
            for css in [
                "display:none",
                "display:block;position:absolute",
                "display:block;position:fixed",
            ] {
                let child = doc.append_element(Some(flow), "div", Some(css));
                doc.append_text(child, "outside normal flow");
            }
            let contents = doc.append_element(Some(flow), "div", Some("display:contents"));
            doc.append_text(contents, " \t\n\r\x0c");
        },
        |session, cascade, _, _, cleared| {
            assert_eq!(
                Plan::collect(session, cascade).completed_at(cleared, 0.0),
                Some(0.0)
            );
        },
    );
}

#[test]
fn non_collapsible_text_stops_the_leading_clearance_proof() {
    for text in ["X", "\u{a0}"] {
        with_fixture(
            "",
            |doc, flow| {
                let contents = doc.append_element(Some(flow), "div", Some("display:contents"));
                doc.append_text(contents, text);
            },
            |session, cascade, _, _, _| {
                assert!(Plan::collect(session, cascade).is_empty());
            },
        );
    }
}

#[test]
fn a_container_only_completes_when_all_normal_flow_children_are_empty() {
    for (css, text, completes) in [
        ("display:none", "X", true),
        ("display:block;position:absolute", "X", true),
        ("display:block;float:right", "X", true),
        ("display:contents", " \t\n", true),
        ("display:contents", "X", false),
        ("display:block;height:0", "", true),
        ("display:block;height:20px", "", false),
    ] {
        with_fixture(
            "",
            |doc, flow| {
                let child = doc.append_element(Some(flow), "div", Some(css));
                if !text.is_empty() {
                    doc.append_text(child, text);
                }
            },
            |session, cascade, _, flow, cleared| {
                let mut plan = Plan::default();
                plan.completed.insert(cleared, 100.0);
                assert_eq!(
                    plan.can_complete_container(session, cascade, flow, 100.0),
                    completes,
                    "{css} {text:?}"
                );
            },
        );
    }
}

#[test]
fn a_completed_child_must_share_its_containers_boundary() {
    with_fixture(
        "",
        |_, _| {},
        |session, cascade, _, flow, cleared| {
            let mut plan = Plan::default();
            plan.completed.insert(cleared, 100.0);
            assert!(plan.can_complete_container(session, cascade, flow, 100.0));
            assert!(!plan.can_complete_container(session, cascade, flow, 200.0));
        },
    );
}

#[test]
fn a_container_with_nonzero_edges_cannot_complete_at_a_childs_boundary() {
    with_fixture(
        "",
        |doc, flow| {
            doc.set_element_attribute(flow, "style", "display:block;padding-top:1px")
                .unwrap();
        },
        |session, cascade, _, flow, cleared| {
            let mut plan = Plan::default();
            plan.completed.insert(cleared, 100.0);
            assert!(!plan.can_complete_container(session, cascade, flow, 100.0));
        },
    );
}

#[test]
fn an_empty_aspect_ratio_box_stops_the_leading_run_at_its_used_height() {
    with_fixture(
        "",
        |doc, flow| {
            doc.append_element(
                Some(flow),
                "div",
                Some("display:block;width:20px;aspect-ratio:1/1"),
            );
        },
        |session, cascade, _, _, _| {
            assert!(Plan::collect(session, cascade).is_empty());
        },
    );
}

#[test]
fn a_later_nonempty_clearing_box_ends_the_completed_run() {
    with_fixture(
        "",
        |doc, flow| {
            doc.append_element(Some(flow), "div", Some("display:block;clear:both;height:0"));
            let nonempty = doc.append_element(
                Some(flow),
                "div",
                Some("display:block;clear:both;margin-top:-10px"),
            );
            doc.append_element(Some(nonempty), "div", Some("display:block;height:20px"));
        },
        |session, cascade, _, flow, cleared| {
            let first = session.nodes[flow].children[0];
            let nonempty = session.nodes[flow].children[1];
            session.context.block_clearance.insert(first, true);
            session
                .context
                .clearance_resumption_offsets
                .insert(first, 0.0);
            session.context.block_clearance.insert(nonempty, true);
            let plan = Plan::collect(session, cascade);
            assert_eq!(plan.completed_at(first, 0.0), Some(0.0));
            assert_eq!(plan.completed_at(nonempty, 0.0), None);
            assert_eq!(plan.completed_at(cleared, 0.0), None);
        },
    );
}

#[test]
fn mixed_height_and_generated_content_cannot_be_completed_as_empty_prefixes() {
    for css in [
        "div { height:calc(1px + 1%) }",
        "body > div + div::before { content:'X' }",
        "body > div + div::before { content:'';display:block;height:1px }",
    ] {
        with_fixture(
            "",
            |doc, _| {
                crate::layout::test_support::with_ahem(doc);
                let sheet = doc.append_element(Some(0), "style", None::<&str>);
                doc.append_text(sheet, css);
            },
            |session, cascade, _, _, _| {
                assert!(Plan::collect(session, cascade).is_empty(), "{css}");
            },
        );
    }
}

fn with_margin_fixture(
    style: &str,
    check: impl FnOnce(&mut LayoutSession<'_>, &CascadeResult, usize, usize),
) {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let float = doc.append_element(
        Some(body),
        "div",
        Some("display:block;float:left;width:40px;height:5px;margin-top:4px"),
    );
    let flow = doc.append_element(
        Some(body),
        "div",
        Some(format!("display:block;font:20px/30px Ahem;{style}")),
    );
    doc.append_text(flow, "XX XX XX XX");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    layout_single_page(
        &mut doc,
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let mut session = LayoutSession::with_cascade(&mut doc, &cascade);
    session
        .context
        .float_resumption_candidates
        .push((float, flow));
    session.context.float_resumption_boundaries.insert(
        float,
        Boundary {
            page_origin: 100.0,
            relative_offset: 3.0,
        },
    );
    check(&mut session, &cascade, float, flow);
}

#[test]
fn margin_resumption_retains_a_relationship_without_freezing_page_coordinates() {
    with_margin_fixture("margin-top:10%", |session, cascade, float, flow| {
        session.nodes[flow].layout.margin.top = 10.0;
        let plan = Plan::collect(session, cascade);
        assert_eq!(plan.margin_float(flow), Some(float));
        assert!(plan.has_margin_flows());
        assert!(!plan.is_empty());
        assert_eq!(plan.float_top(float), None);
        assert_eq!(plan.flow_top(flow), None);
        assert!(plan.completed.is_empty());
        assert!(plan.completed_containers.is_empty());
        session.context.float_resumptions = plan.clone();
        session.nodes[flow].layout.margin.top = 20.0;
        session.nodes[flow].layout.location.y += 500.0;
        session
            .context
            .float_resumption_boundaries
            .get_mut(&float)
            .unwrap()
            .page_origin += 100.0;
        assert_eq!(Plan::collect(session, cascade), plan);
        session.nodes[flow].layout.margin.top = 0.0;
        assert!(Plan::collect(session, cascade).is_empty());
    });
}

#[test]
fn margin_resumption_requires_an_ordinary_shared_context_and_positive_margin() {
    for style in [
        "margin-top:0",
        "margin-top:-10px",
        "margin-top:10px;clear:both",
        "margin-top:10px;position:relative",
        "margin-top:10px;display:flow-root",
        "margin-top:10px;writing-mode:vertical-rl",
    ] {
        with_margin_fixture(style, |session, cascade, _, _| {
            assert!(Plan::collect(session, cascade).is_empty(), "{style}");
        });
    }
    with_margin_fixture("margin-top:10px", |session, cascade, float, flow| {
        let boundary = session
            .context
            .float_resumption_boundaries
            .remove(&float)
            .unwrap();
        assert!(Plan::collect(session, cascade).is_empty());
        session
            .context
            .float_resumption_boundaries
            .insert(float, boundary);
        session.nodes[flow].layout.margin.top = f32::NAN;
        assert!(Plan::collect(session, cascade).is_empty());
    });
}
