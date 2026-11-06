use super::*;

#[derive(Debug, Default)]
struct WrapStyle<'a> {
    outer: &'a str,
    flex: &'a str,
    item: &'a str,
    local: Option<(usize, &'a str)>,
    contents: bool,
    absolute: bool,
}

struct Wrapped {
    doc: Document,
    holder: usize,
    flex: usize,
    items: Vec<usize>,
    after: usize,
    absolute: Option<usize>,
}

fn wrapped(items: &[(u32, u32)], style: WrapStyle<'_>) -> Wrapped {
    let (mut holder, mut flex, mut after) = (0, 0, 0);
    let mut nodes = Vec::new();
    let mut absolute = None;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        holder = doc.append_element(
            Some(body),
            "div",
            Some(format!(
                "display:block;padding-bottom:30px;font:8px/10px Ahem;{}",
                style.outer
            )),
        );
        for height in [40, 30] {
            doc.append_element(
                Some(holder),
                "div",
                Some(format!("display:block;height:{height}px")),
            );
        }
        flex = doc.append_element(
            Some(holder),
            "div",
            Some(format!(
                "display:flex;flex-wrap:wrap;width:100px;{}",
                style.flex
            )),
        );
        let parent = if style.contents {
            doc.append_element(Some(flex), "div", Some("display:contents"))
        } else {
            flex
        };
        for (index, &(width, height)) in items.iter().enumerate() {
            let item = doc.append_element(
                Some(parent),
                "div",
                Some(format!(
                    "display:block;flex:none;width:{width}px;height:{height}px;background:green;{};{}",
                    style.item,
                    style.local.filter(|(at, _)| *at == index).map_or("", |(_, css)| css)
                )),
            );
            doc.append_text(item, ["cc", "dd", "ee", "ff"][index]);
            nodes.push(item);
        }
        if style.absolute {
            absolute = Some(doc.append_element(
                Some(parent),
                "div",
                Some("display:block;position:absolute;top:10px;width:5px;height:5px"),
            ));
        }
        after = doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    });
    Wrapped {
        doc,
        holder,
        flex,
        items: nodes,
        after,
        absolute,
    }
}

#[test]
fn the_final_wrapped_flex_line_moves_with_closing_edges() {
    for (items, expected) in [
        (vec![(60, 10), (60, 10)], vec![70.0, 100.0]),
        (vec![(60, 10), (60, 10), (60, 10)], vec![70.0, 80.0, 100.0]),
        (vec![(40, 10), (40, 10), (60, 10)], vec![70.0, 70.0, 100.0]),
        (vec![(60, 10), (60, 10), (40, 10)], vec![70.0, 100.0, 100.0]),
    ] {
        let r = wrapped(&items, WrapStyle::default());
        let actual: Vec<_> = r.items.iter().map(|&id| page_flow_y(&r.doc, id)).collect();
        assert_eq!(actual, expected);
        assert_eq!(page_flow_y(&r.doc, r.after), 140.0);
        assert_eq!(r.doc.nodes[r.holder].layout.size.height, 140.0);
        assert_eq!(r.doc.nodes[r.flex].layout.size.height, 40.0);
    }
}

#[test]
fn all_final_line_members_move_when_the_tallest_is_first_or_last() {
    for heights in [[20, 10], [10, 20]] {
        let r = wrapped(
            &[(60, 10), (40, 10), (40, heights[0]), (60, heights[1])],
            WrapStyle::default(),
        );
        let actual: Vec<_> = r.items.iter().map(|&id| page_flow_y(&r.doc, id)).collect();
        assert_eq!(actual, [70.0, 70.0, 100.0, 100.0]);
        assert_eq!(page_flow_y(&r.doc, r.after), 150.0);
        assert_eq!(r.doc.nodes[r.flex].layout.size.height, 50.0);
    }
}

#[test]
fn contents_peers_move_while_an_absolute_child_keeps_its_position() {
    let r = wrapped(
        &[(60, 10), (60, 10), (40, 10)],
        WrapStyle {
            flex: "position:relative",
            contents: true,
            absolute: true,
            ..Default::default()
        },
    );
    assert_eq!(
        r.items
            .iter()
            .map(|&id| page_flow_y(&r.doc, id))
            .collect::<Vec<_>>(),
        [70.0, 100.0, 100.0]
    );
    assert_eq!(page_flow_y(&r.doc, r.absolute.unwrap()), 80.0);
}

#[test]
fn wrapping_tails_keep_positive_gap_and_own_padding() {
    for (flex, after) in [
        ("row-gap:5px", 140.0),
        ("padding-bottom:5px", 145.0),
        ("align-items:flex-start", 140.0),
    ] {
        let r = wrapped(
            &[(60, 10), (60, 10)],
            WrapStyle {
                flex,
                ..Default::default()
            },
        );
        assert_eq!(page_flow_y(&r.doc, r.items[0]), 70.0);
        assert_eq!(page_flow_y(&r.doc, r.items[1]), 100.0);
        assert_eq!(page_flow_y(&r.doc, r.after), after);
        assert_eq!(r.doc.nodes[r.holder].layout.size.height, after);
    }
    for item in ["align-self:flex-start", "height:auto", "break-inside:avoid"] {
        let r = wrapped(
            &[(60, 10), (60, 10)],
            WrapStyle {
                item,
                ..Default::default()
            },
        );
        assert_eq!(page_flow_y(&r.doc, r.items[1]), 100.0);
    }
}

#[test]
fn wrapping_items_stay_when_the_closing_edges_fit() {
    for (outer, after) in [("padding-bottom:0", 90.0), ("padding-bottom:10px", 100.0)] {
        let r = wrapped(
            &[(60, 10), (60, 10)],
            WrapStyle {
                outer,
                ..Default::default()
            },
        );
        assert_eq!(
            r.items
                .iter()
                .map(|&id| page_flow_y(&r.doc, id))
                .collect::<Vec<_>>(),
            [70.0, 80.0]
        );
        assert_eq!(page_flow_y(&r.doc, r.after), after);
    }
}

#[test]
fn unsupported_alignment_and_avoided_boundaries_keep_the_existing_path() {
    for style in [
        WrapStyle {
            flex: "align-items:center",
            ..Default::default()
        },
        WrapStyle {
            flex: "align-items:baseline",
            ..Default::default()
        },
        WrapStyle {
            flex: "flex-direction:row-reverse",
            ..Default::default()
        },
        WrapStyle {
            flex: "flex-wrap:wrap-reverse",
            ..Default::default()
        },
        WrapStyle {
            item: "align-self:center",
            ..Default::default()
        },
        WrapStyle {
            item: "min-height:1px",
            ..Default::default()
        },
        WrapStyle {
            item: "max-height:10px",
            ..Default::default()
        },
        WrapStyle {
            item: "break-before:avoid",
            ..Default::default()
        },
        WrapStyle {
            item: "break-after:avoid",
            ..Default::default()
        },
        WrapStyle {
            item: "position:relative;top:1px",
            ..Default::default()
        },
        WrapStyle {
            item: "margin-top:1px",
            ..Default::default()
        },
    ] {
        let r = wrapped(&[(60, 10), (60, 10)], style);
        assert!(r.items.iter().all(|&id| page_flow_y(&r.doc, id) < 100.0));
    }
}

#[test]
fn a_single_line_keeps_its_existing_whole_container_path() {
    let r = wrapped(
        &[(40, 10), (40, 10)],
        WrapStyle {
            flex: "flex-wrap:nowrap",
            ..Default::default()
        },
    );
    assert_eq!(
        r.items
            .iter()
            .map(|&id| page_flow_y(&r.doc, id))
            .collect::<Vec<_>>(),
        [100.0, 100.0]
    );
    let mut r = wrapped(&[(40, 10), (40, 10)], WrapStyle::default());
    let cascade = crate::layout::test_support::cascaded(&r.doc);
    let tree = crate::layout::context::LayoutSession::resumed(&mut r.doc, &cascade);
    assert!(wrapping_row_item_tail(&tree, &cascade, r.flex, false).is_none());
}

#[test]
fn an_overlapping_line_report_is_not_treated_as_independent_rows() {
    let mut r = wrapped(
        &[(60, 10), (60, 10)],
        WrapStyle {
            outer: "padding-bottom:0",
            ..Default::default()
        },
    );
    let cascade = crate::layout::test_support::cascaded(&r.doc);
    let mut tree = crate::layout::context::LayoutSession::resumed(&mut r.doc, &cascade);
    assert!(wrapping_row_item_tail(&tree, &cascade, r.flex, false).is_some());
    tree.nodes[r.items[1]].layout.location.y = tree.nodes[r.items[0]].layout.location.y + 5.0;
    assert!(wrapping_row_item_tail(&tree, &cascade, r.flex, false).is_none());
}

#[test]
fn a_short_next_page_must_fit_the_entire_final_line() {
    let mut r = wrapped(
        &[(60, 10), (40, 10), (40, 10), (60, 20)],
        WrapStyle {
            outer: "padding-bottom:0",
            ..Default::default()
        },
    );
    let cascade = crate::layout::test_support::cascaded(&r.doc);
    let tree = crate::layout::context::LayoutSession::resumed(&mut r.doc, &cascade);
    let tallest = wrapping_row_item_tail(&tree, &cascade, r.flex, false).unwrap();
    assert_eq!(tallest, r.items[3]);
    let candidate = PageCandidate {
        node_id: tallest,
        raw_y: 80.0,
        height: tree.nodes[tallest].layout.size.height,
        context: element_candidate_context(candidates::ElementContext {
            item_tail_edges: 30.0,
            ..Default::default()
        }),
        page_name: None,
        named_flex_context: None,
        ifc_root: None,
    };
    let fit = BoxFit {
        effective_y: 80.0,
        flow_y: 80.0,
        margin_top: 0.0,
        trailing_margin: 0.0,
        holder_lead: 0.0,
        holder_margin_top: 0.0,
        avoided_lead: 0.0,
        avoided_margin_top: 0.0,
        page_start: 0.0,
        page_end: 100.0,
        next_page_step: 15.0,
    };
    assert!(!item_tail_move(&candidate, &fit));
    assert!(item_tail_move(
        &candidate,
        &BoxFit {
            next_page_step: 20.0,
            ..fit
        }
    ));
}

#[test]
fn every_line_member_can_avoid_the_boundary() {
    for (items, local) in [
        (
            [(60, 20), (40, 10), (40, 10), (60, 10)],
            (1, "break-after:avoid"),
        ),
        (
            [(60, 10), (40, 10), (40, 20), (60, 10)],
            (3, "break-before:avoid"),
        ),
    ] {
        let r = wrapped(
            &items,
            WrapStyle {
                local: Some(local),
                ..Default::default()
            },
        );
        assert!(r.items.iter().all(|&id| page_flow_y(&r.doc, id) < 100.0));
    }
}

#[test]
fn forced_row_breaks_preserve_direct_and_contents_peers_without_moving_oof() {
    for contents in [false, true] {
        let r = wrapped(
            &[(60, 10), (40, 10), (40, 10), (60, 10)],
            WrapStyle {
                outer: "padding-bottom:0",
                flex: "position:relative",
                local: Some((3, "break-before:page")),
                contents,
                absolute: true,
                ..Default::default()
            },
        );
        assert_eq!(
            r.items
                .iter()
                .map(|&id| page_flow_y(&r.doc, id))
                .collect::<Vec<_>>(),
            [70.0, 70.0, 100.0, 100.0]
        );
        assert_eq!(page_flow_y(&r.doc, r.absolute.unwrap()), 80.0);
        assert_eq!(page_flow_y(&r.doc, r.after), 110.0);
    }
}
