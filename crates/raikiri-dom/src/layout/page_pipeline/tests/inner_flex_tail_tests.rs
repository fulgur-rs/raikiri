use super::*;

#[derive(Debug, Default)]
struct TailStyle<'a> {
    outer: &'a str,
    column: &'a str,
    item: &'a str,
    first: &'a str,
    last: &'a str,
    depth: usize,
    tied_third: bool,
    trailing_float: bool,
    contents: bool,
    float_in_contents: bool,
}

fn inner_tail(style: TailStyle<'_>) -> (Document, [usize; 6]) {
    let mut ids = [0; 6];
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        ids[0] = doc.append_element(
            Some(body),
            "div",
            Some(format!(
                "display:block;padding-bottom:30px;font:8px/10px Ahem;{}",
                style.outer
            )),
        );
        for (height, text) in [(40, "A"), (30, "B")] {
            let block = doc.append_element(
                Some(ids[0]),
                "div",
                Some(format!("display:block;height:{height}px")),
            );
            doc.append_text(block, text);
        }
        ids[1] = doc.append_element(
            Some(ids[0]),
            "div",
            Some(format!(
                "display:flex;flex-direction:column;{}",
                style.column
            )),
        );
        let first = doc.append_element(Some(ids[1]), "div", Some("display:block;height:10px"));
        doc.append_text(first, "c");
        ids[2] = doc.append_element(
            Some(ids[1]),
            "div",
            Some(format!("display:block;{}", style.item)),
        );
        let mut parent = ids[2];
        for _ in 0..style.depth {
            parent = doc.append_element(Some(parent), "div", Some("display:block"));
        }
        if style.contents {
            parent = doc.append_element(Some(parent), "div", Some("display:contents"));
        }
        for (slot, css, text) in [(3, style.first, "e"), (4, style.last, "f")] {
            ids[slot] = doc.append_element(
                Some(parent),
                "div",
                Some(format!("display:block;height:5px;{css}")),
            );
            doc.append_text(ids[slot], text);
        }
        if style.trailing_float {
            let parent = if style.float_in_contents {
                doc.append_element(Some(parent), "div", Some("display:contents"))
            } else {
                parent
            };
            let float = doc.append_element(
                Some(parent),
                "div",
                Some("display:block;float:left;width:5px;height:5px"),
            );
            doc.append_text(float, "g");
        }
        if style.tied_third {
            ids[4] = doc.append_element(
                Some(parent),
                "div",
                Some("display:block;height:5px;break-before:avoid"),
            );
            doc.append_text(ids[4], "g");
        }
        ids[5] = doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    });
    (doc, ids)
}

#[test]
fn closing_edges_follow_only_the_last_block_inside_a_breakable_column_item() {
    for depth in [0, 1, 64] {
        let (doc, ids) = inner_tail(TailStyle {
            depth,
            ..Default::default()
        });
        assert_eq!(page_flow_y(&doc, ids[3]), 80.0);
        assert_eq!(page_flow_y(&doc, ids[4]), 100.0);
        assert_eq!(page_flow_y(&doc, ids[5]), 135.0);
        assert_eq!(doc.nodes[ids[0]].layout.size.height, 135.0);
        assert_eq!(doc.nodes[ids[1]].layout.size.height, 35.0);
        assert_eq!(doc.nodes[ids[2]].layout.size.height, 25.0);
    }
}

#[test]
fn an_avoiding_or_fixed_last_column_item_keeps_its_inner_blocks_together() {
    for style in [
        TailStyle {
            item: "break-inside:avoid",
            ..Default::default()
        },
        TailStyle {
            item: "break-inside:avoid-page",
            ..Default::default()
        },
        TailStyle {
            item: "height:10px",
            ..Default::default()
        },
        TailStyle {
            first: "break-after:avoid",
            ..Default::default()
        },
        TailStyle {
            last: "break-before:avoid",
            ..Default::default()
        },
    ] {
        let (doc, ids) = inner_tail(style);
        assert_eq!(page_flow_y(&doc, ids[3]), 100.0);
        assert_eq!(page_flow_y(&doc, ids[4]), 105.0);
        assert_eq!(page_flow_y(&doc, ids[5]), 140.0);
    }
}

#[test]
fn inner_tail_stays_when_the_edges_fit() {
    for outer in ["padding-bottom:0", "padding-bottom:10px"] {
        let (doc, ids) = inner_tail(TailStyle {
            outer,
            ..Default::default()
        });
        assert_eq!(page_flow_y(&doc, ids[3]), 80.0);
        assert_eq!(page_flow_y(&doc, ids[4]), 85.0);
    }
}

#[test]
fn independent_or_bounded_inner_flows_do_not_gain_tail_candidates() {
    for style in [
        TailStyle {
            tied_third: true,
            ..Default::default()
        },
        TailStyle {
            trailing_float: true,
            ..Default::default()
        },
        TailStyle {
            last: "overflow:hidden",
            ..Default::default()
        },
        TailStyle {
            item: "min-height:1px",
            ..Default::default()
        },
        TailStyle {
            item: "max-height:10px",
            ..Default::default()
        },
        TailStyle {
            item: "column-count:2",
            ..Default::default()
        },
        TailStyle {
            item: "overflow:hidden",
            ..Default::default()
        },
        TailStyle {
            item: "margin-bottom:1px",
            ..Default::default()
        },
        TailStyle {
            last: "position:relative;top:1px",
            ..Default::default()
        },
        TailStyle {
            outer: "height:300px;overflow:hidden",
            ..Default::default()
        },
        TailStyle {
            column: "flex-wrap:wrap",
            ..Default::default()
        },
    ] {
        let label = format!("{style:?}");
        let (doc, ids) = inner_tail(style);
        assert!(page_flow_y(&doc, ids[4]) < 100.0, "{label}");
    }
}

#[test]
fn inner_item_padding_and_border_follow_the_selected_block() {
    for item in ["padding-bottom:5px", "border-bottom:5px solid"] {
        let (doc, ids) = inner_tail(TailStyle {
            item,
            ..Default::default()
        });
        assert_eq!(page_flow_y(&doc, ids[3]), 80.0);
        assert_eq!(page_flow_y(&doc, ids[4]), 100.0);
        assert_eq!(page_flow_y(&doc, ids[5]), 140.0);
        assert_eq!(doc.nodes[ids[2]].layout.size.height, 30.0);
    }
}

#[test]
fn a_contents_wrapper_preserves_the_selected_inner_tail() {
    let (doc, ids) = inner_tail(TailStyle {
        contents: true,
        ..Default::default()
    });
    assert_eq!(page_flow_y(&doc, ids[3]), 80.0);
    assert_eq!(page_flow_y(&doc, ids[4]), 100.0);
    assert_eq!(page_flow_y(&doc, ids[5]), 135.0);
}

#[test]
fn a_float_inside_contents_also_excludes_the_inner_tail() {
    let (doc, ids) = inner_tail(TailStyle {
        trailing_float: true,
        float_in_contents: true,
        ..Default::default()
    });
    assert_eq!(page_flow_y(&doc, ids[4]), 85.0);
}
