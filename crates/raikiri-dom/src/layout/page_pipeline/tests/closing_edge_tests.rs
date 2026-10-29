use super::*;

fn block(doc: &mut Document, parent: usize, css: &str) -> usize {
    doc.append_element(Some(parent), "div", Some(css))
}

/// `body > [div(parent_css) > [div(40px), div(30px), div(20px)], div(10px)]`
/// on 100px pages. Returns the page-flow y of the last child of the parent,
/// the parent's height and the page-flow y of the block after it.
fn closed_parent(parent_css: &str) -> (f32, f32, f32) {
    let (mut parent, mut last, mut after) = (0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, &format!("display:block;{parent_css}"));
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        last = block(doc, parent, "display:block;height:20px");
        after = block(doc, body, "display:block;height:10px");
    });
    (
        page_flow_y(&doc, last),
        doc.nodes[parent].layout.size.height,
        page_flow_y(&doc, after),
    )
}

#[test]
fn the_last_box_moves_with_the_padding_that_would_break_away_from_it() {
    // The last box ends at 90 and fits the page, but the 30px of padding
    // below it would not. The page breaks before the box, not between the
    // box and the padding, so the padding is not left alone on the next page.
    assert_eq!(closed_parent("padding-bottom:30px"), (100.0, 150.0, 150.0));
    // The same for a border, and for both together.
    assert_eq!(
        closed_parent("border-bottom:30px solid"),
        (100.0, 150.0, 150.0)
    );
    assert_eq!(
        closed_parent("padding-bottom:15px;border-bottom:15px solid"),
        (100.0, 150.0, 150.0)
    );
}

#[test]
fn the_last_box_stays_when_its_parents_closing_edges_fit_the_page() {
    assert_eq!(closed_parent("padding-bottom:10px"), (70.0, 100.0, 100.0));
    assert_eq!(closed_parent(""), (70.0, 90.0, 90.0));
}

#[test]
fn the_last_box_stays_when_its_parent_has_a_height_of_its_own() {
    // The parent's height does not follow its content, so its edges do not
    // follow the last box either.
    assert_eq!(
        closed_parent("height:90px;padding-bottom:30px"),
        (70.0, 120.0, 120.0)
    );
}

#[test]
fn a_last_box_moves_when_the_next_page_cannot_hold_it_with_the_edges() {
    // 20px of box and 90px of padding make 110px, more than a page. A break
    // before the box is still better than one between it and the padding, so
    // the box moves and the padding runs on past the page edge.
    assert_eq!(closed_parent("padding-bottom:90px"), (100.0, 210.0, 210.0));
}

#[test]
fn a_last_box_that_fills_the_rest_of_its_page_moves_with_the_edges() {
    // The box ends at the page end and the padding does not fit after it, so
    // the box moves to the next page whatever the padding is: the padding is
    // not left alone at the top of it.
    for padding in [15, 100, 250] {
        let (ys, height) = tied_boxes(
            &format!("padding-bottom:{padding}px"),
            &[(10, ""), (90, "")],
        );
        assert_eq!(ys, vec![0.0, 100.0], "{padding}px");
        assert_eq!(height, 100.0 + 90.0 + padding as f32, "{padding}px");
    }
}

#[test]
fn a_last_box_that_runs_past_its_page_end_stays() {
    // The page breaks inside the box, as it would inside an empty block
    // taller than what is left of the page, and the padding follows it: the
    // break before the box is not the one that avoids a break next to the
    // padding.
    let (ys, height) = tied_boxes("padding-bottom:15px", &[(10, ""), (95, "")]);
    assert_eq!(ys, vec![0.0, 10.0]);
    assert_eq!(height, 120.0);
}

#[test]
fn a_box_moves_only_when_the_next_page_can_hold_it_without_the_edges() {
    // `height` of block size at 40 on a page from 0 to 100 with 30 of edges
    // below the box, 30 of which follow the box inside the block itself when
    // `inside` is set, and `span` of block size below the box that a run of
    // tied boxes takes.
    let moves = |height: f32, inside: f32, span: f32, next_page_step: f32| {
        let candidate = PageCandidate {
            node_id: 0,
            raw_y: 40.0,
            height,
            context: element_candidate_context(candidates::ElementContext {
                closing_edges: 30.0 - inside,
                closing_edges_inside: inside,
                closing_edges_span: span,
                ..Default::default()
            }),
            page_name: None,
            named_flex_context: None,
            ifc_root: None,
        };
        let fit = BoxFit {
            effective_y: 40.0,
            flow_y: 40.0,
            margin_top: 0.0,
            trailing_margin: 0.0,
            page_start: 0.0,
            page_end: 100.0,
            next_page_step,
        };
        closing_edges_move(&candidate, &fit)
    };
    // A box of 60 that fills the rest of its page: the next page holds it
    // from 60 up, whatever the edges that run on after it.
    assert!(moves(60.0, 0.0, 0.0, 100.0));
    assert!(moves(60.0, 0.0, 0.0, 60.0));
    assert!(!moves(60.0, 0.0, 0.0, 59.0));
    // A run: the box of 30 moves with the 30 that the rest of it takes.
    assert!(moves(30.0, 0.0, 30.0, 60.0));
    assert!(!moves(30.0, 0.0, 30.0, 59.0));
    // A block of 90 whose last 30 follow its one box: the box is 60.
    assert!(moves(90.0, 30.0, 0.0, 60.0));
    assert!(!moves(90.0, 30.0, 0.0, 59.0));
}

/// `body > [div(60px), div(parent_css) > div(30px)]` on 100px pages. Returns
/// the page-flow y of the parent and of its one box.
fn lone_box_parent(parent_css: &str) -> (f32, f32) {
    let (mut parent, mut only) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, &format!("display:block;{parent_css}"));
        only = block(doc, parent, "display:block;height:30px");
    });
    (page_flow_y(&doc, parent), page_flow_y(&doc, only))
}

#[test]
fn a_block_that_holds_one_box_moves_whole_with_the_padding_below_it() {
    // The box ends at 90 and fits the page, the 30px of padding below it do
    // not, and no box before the one in the block is there to break from. The
    // page breaks before the block: the padding is not left alone on the next
    // page, nor the box left alone on this one.
    assert_eq!(lone_box_parent("padding-bottom:30px"), (100.0, 100.0));
    assert_eq!(lone_box_parent("border-bottom:30px solid"), (100.0, 100.0));
    // The padding and border above the box move with it.
    assert_eq!(
        lone_box_parent("padding-top:10px;padding-bottom:30px"),
        (100.0, 110.0)
    );
    assert_eq!(
        lone_box_parent("border-top:5px solid;border-bottom:30px solid"),
        (100.0, 105.0)
    );
}

#[test]
fn a_block_that_holds_one_box_stays_when_its_edges_fit_the_page() {
    assert_eq!(lone_box_parent("padding-bottom:10px"), (60.0, 60.0));
    assert_eq!(lone_box_parent(""), (60.0, 60.0));
    // The padding above the box counts as part of the content that fits.
    assert_eq!(
        lone_box_parent("padding-top:5px;padding-bottom:5px"),
        (60.0, 65.0)
    );
}

#[test]
fn a_block_that_holds_one_box_stays_when_the_box_does_not_fit_the_page() {
    // The box runs past the page end on its own, so the page breaks in it, or
    // not at all, and the padding follows it.
    let (mut parent, mut only) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block;padding-bottom:30px");
        only = block(doc, parent, "display:block;height:50px");
    });
    assert_eq!(page_flow_y(&doc, parent), 60.0);
    assert_eq!(page_flow_y(&doc, only), 60.0);
}

#[test]
fn a_block_that_holds_one_box_moves_when_the_next_page_cannot_hold_it_with_the_edges() {
    // 30px of box and 80px of padding make 110px, more than a page. The page
    // still breaks before the block rather than between its box and the
    // padding, which runs on past the page edge.
    assert_eq!(lone_box_parent("padding-bottom:80px"), (100.0, 100.0));
}

#[test]
fn a_block_that_starts_its_page_stays() {
    // Nothing before it on the page is a box to break from.
    let (mut parent, mut only) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:30px");
        only = block(doc, parent, "display:block;height:80px");
    });
    assert_eq!(page_flow_y(&doc, parent), 0.0);
    assert_eq!(page_flow_y(&doc, only), 0.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 110.0);
}

#[test]
fn the_margin_below_the_one_box_of_a_block_counts_with_its_edges() {
    // The box ends at 70 and the 30px of padding below it end at 100, which
    // fits, but not with 10px of margin between them.
    for (margin, expected) in [(0, 40.0), (10, 100.0)] {
        let mut parent = 0;
        let (doc, _) = paged_body(|doc, body| {
            block(doc, body, "display:block;height:40px");
            parent = block(doc, body, "display:block;padding-bottom:30px");
            block(
                doc,
                parent,
                &format!("display:block;height:30px;margin-bottom:{margin}px"),
            );
        });
        assert_eq!(page_flow_y(&doc, parent), expected, "{margin}");
    }
}

#[test]
fn a_block_that_holds_one_box_through_wrappers_moves_whole() {
    // The wrappers have nothing of their own that the page breaks at, with
    // edges below the box or none.
    for (inner_css, outer_css) in [
        ("", "padding-bottom:30px"),
        ("padding-bottom:30px", ""),
        ("padding-bottom:15px", "padding-bottom:15px"),
    ] {
        let (mut outer, mut only) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            block(doc, body, "display:block;height:60px");
            outer = block(doc, body, &format!("display:block;{outer_css}"));
            let inner = block(doc, outer, &format!("display:block;{inner_css}"));
            only = block(doc, inner, "display:block;height:30px");
        });
        assert_eq!(page_flow_y(&doc, outer), 100.0, "{inner_css} {outer_css}");
        assert_eq!(page_flow_y(&doc, only), 100.0, "{inner_css} {outer_css}");
    }
}

#[test]
fn a_block_that_ends_the_blocks_around_it_moves_with_their_edges() {
    // The inner block holds one box and is after a box in the outer block,
    // which ends in 20px of padding: the box ends at 70 and fits, and the two
    // paddings after it, which end at 110, do not.
    let (mut inner, mut only, mut outer) = (0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        outer = block(doc, body, "display:block;padding-bottom:20px");
        block(doc, outer, "display:block;height:50px");
        inner = block(doc, outer, "display:block;padding-bottom:20px");
        only = block(doc, inner, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, inner), 100.0);
    assert_eq!(page_flow_y(&doc, only), 100.0);
    assert_eq!(doc.nodes[outer].layout.size.height, 160.0);
}

#[test]
fn a_block_that_holds_one_box_in_a_container_moves_after_the_box_before_it() {
    // The container has no edges and starts the page; the box before the block
    // in it is the one to break from.
    let (mut container, mut parent, mut only) = (0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        container = block(doc, body, "display:block");
        block(doc, container, "display:block;height:60px");
        parent = block(doc, container, "display:block;padding-bottom:30px");
        only = block(doc, parent, "display:block;height:30px");
    });
    assert_eq!(page_flow_y(&doc, container), 0.0);
    assert_eq!(page_flow_y(&doc, parent), 100.0);
    assert_eq!(page_flow_y(&doc, only), 100.0);
}

#[test]
fn a_block_that_holds_one_box_stays_when_a_break_before_it_is_avoided() {
    for (before_css, parent_css, only_css) in [
        ("break-after:avoid", "", ""),
        ("", "break-before:avoid", ""),
        ("", "", "break-before:avoid"),
    ] {
        let (mut parent, mut only) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            block(
                doc,
                body,
                &format!("display:block;height:60px;{before_css}"),
            );
            parent = block(
                doc,
                body,
                &format!("display:block;padding-bottom:30px;{parent_css}"),
            );
            only = block(
                doc,
                parent,
                &format!("display:block;height:30px;{only_css}"),
            );
        });
        assert_eq!(
            page_flow_y(&doc, parent),
            60.0,
            "{before_css} {parent_css} {only_css}"
        );
        assert_eq!(
            page_flow_y(&doc, only),
            60.0,
            "{before_css} {parent_css} {only_css}"
        );
    }
}

/// The page count of a 60px block, a block with `padding` below the one box
/// styled with `only_css`, and a 10px block.
fn pages_around_a_block_that_holds_one_box(only_css: &str, padding: u32) -> usize {
    paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        let parent = block(
            doc,
            body,
            &format!("display:block;padding-bottom:{padding}px"),
        );
        block(
            doc,
            parent,
            &format!("display:block;height:30px;{only_css}"),
        );
        block(doc, body, "display:block;height:10px");
    })
    .1
}

#[test]
fn a_forced_break_or_a_page_name_in_a_block_that_holds_one_box_is_left_to_its_own_rules() {
    // The break already puts the block on the page it asks for; moving the
    // block as well for its padding would add a page.
    for only_css in ["break-before:always", "page:foo"] {
        assert_eq!(
            pages_around_a_block_that_holds_one_box(only_css, 30),
            pages_around_a_block_that_holds_one_box(only_css, 0),
            "{only_css}"
        );
    }
}

#[test]
fn a_forced_break_after_the_box_of_a_block_whose_padding_runs_over_a_page_edge_follows_the_padding()
{
    // The padding below the box runs from 90 to 120, over the edge at 100. The
    // break after the box sits after the padding, so the next block opens the
    // page after the one the padding ends on.
    assert_eq!(
        pages_around_a_block_that_holds_one_box("break-after:always", 0),
        2
    );
    assert_eq!(
        pages_around_a_block_that_holds_one_box("break-after:always", 30),
        3
    );
}

/// The page-flow y of the div with 30px of padding below it, after a 60px
/// block, when `build` fills it.
fn parent_with(build: impl FnOnce(&mut Document, usize)) -> f32 {
    let mut parent = 0;
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block;padding-bottom:30px");
        build(doc, parent);
    });
    page_flow_y(&doc, parent)
}

#[test]
fn a_block_that_holds_one_box_with_something_to_break_at_stays() {
    // A box with boxes of its own: the last of them moves with the edges, and
    // the block stays where it starts.
    assert_eq!(
        parent_with(|doc, parent| {
            let inner = block(doc, parent, "display:block");
            block(doc, inner, "display:block;height:10px");
            block(doc, inner, "display:block;height:20px");
        }),
        60.0
    );
    // A multicol container breaks into its columns on its own.
    assert_eq!(
        parent_with(|doc, parent| {
            let columns = block(doc, parent, "display:block;columns:2;column-gap:0");
            block(doc, columns, "display:block;height:30px");
        }),
        60.0
    );
}

/// A flex container that holds `items` blocks of 30px by 40px, one line.
fn flex_line(doc: &mut Document, parent: usize, items: usize) -> Vec<usize> {
    let line = block(doc, parent, "display:flex");
    let mut ids = vec![line];
    for _ in 0..items {
        ids.push(block(doc, line, "display:block;width:40px;height:30px"));
    }
    ids
}

#[test]
fn a_block_that_holds_one_box_beside_a_float_moves_whole() {
    // The float lies within the content of the block, so it goes with it and
    // is no place to break at: before the box, or inside a wrapper around it.
    assert_eq!(
        parent_with(|doc, parent| {
            block(doc, parent, "float:left;width:10px;height:5px");
            block(doc, parent, "display:block;height:30px");
        }),
        100.0
    );
    assert_eq!(
        parent_with(|doc, parent| {
            let wrapper = block(doc, parent, "display:block");
            block(doc, wrapper, "float:left;width:10px;height:5px");
            block(doc, wrapper, "display:block;height:30px");
        }),
        100.0
    );
}

#[test]
fn a_block_that_holds_one_box_stays_when_a_float_reaches_past_its_content() {
    // The float runs into the padding below the box or out of the block, to
    // where the page ends, and has a break to take of its own.
    for (float_height, float_first) in [(70, true), (70, false), (40, true)] {
        assert_eq!(
            parent_with(|doc, parent| {
                let float = format!("float:left;width:10px;height:{float_height}px");
                if float_first {
                    block(doc, parent, &float);
                }
                block(doc, parent, "display:block;height:30px");
                if !float_first {
                    block(doc, parent, &float);
                }
            }),
            60.0,
            "{float_height} {float_first}"
        );
    }
}

#[test]
fn a_block_that_holds_one_row_of_boxes_moves_whole() {
    // A flex container, a grid and a table that hold one row of boxes that
    // the page cannot break inside are the one box of the block, as Chromium
    // sends them.
    assert_eq!(
        parent_with(|doc, parent| {
            block(doc, parent, "display:grid;height:30px");
        }),
        100.0
    );
    assert_eq!(
        parent_with(|doc, parent| {
            block(doc, parent, "display:flex;height:30px");
        }),
        100.0
    );
    assert_eq!(
        parent_with(|doc, parent| {
            block(doc, parent, "display:table;height:30px");
        }),
        100.0
    );
    assert_eq!(
        parent_with(|doc, parent| {
            let wrapper = block(doc, parent, "display:block");
            block(doc, wrapper, "display:grid;height:30px");
        }),
        100.0
    );
    assert_eq!(
        parent_with(|doc, parent| {
            flex_line(doc, parent, 2);
        }),
        100.0
    );
}

#[test]
fn a_box_after_the_last_one_keeps_the_edges_from_following_it() {
    // A float after the box is in the parent, whose edges stay below it.
    let (mut parent, mut middle) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, "display:flow-root;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        middle = block(doc, parent, "display:block;height:50px");
        block(doc, parent, "float:left;width:20px;height:5px");
    });
    assert_eq!(page_flow_y(&doc, middle), 40.0);
    // The float ends 5px below the box and the flow root holds it.
    assert_eq!(doc.nodes[parent].layout.size.height, 125.0);
}

#[test]
fn a_float_that_the_parent_encloses_below_the_last_box_ends_the_parent() {
    // The padding of the flow root follows the 150px float, not the last box:
    // it is not stranded at the page end below the box.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:flow-root;padding-bottom:30px");
        block(doc, parent, "float:left;width:20px;height:150px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
}

#[test]
fn a_relative_offset_of_the_last_box_does_not_hide_a_float_below_it() {
    // The float ends the flow root at 150. The last box ends at 90 in the flow,
    // and at 190 with its offset, which is not where the padding follows it.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:flow-root;padding-bottom:30px");
        block(doc, parent, "float:left;width:20px;height:150px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        last = block(
            doc,
            parent,
            "display:block;height:20px;position:relative;top:100px",
        );
    });
    assert_eq!(page_flow_y(&doc, last), 170.0);
}

/// `body > div(parent_css) > boxes`, each a block of the given height and
/// style, on 100px pages. Returns the page-flow y of each box and the height of
/// the parent.
fn tied_boxes(parent_css: &str, boxes: &[(u32, &str)]) -> (Vec<f32>, f32) {
    let (mut parent, mut ids) = (0, Vec::new());
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, &format!("display:block;{parent_css}"));
        for &(height, css) in boxes {
            ids.push(block(
                doc,
                parent,
                &format!("display:block;height:{height}px;{css}"),
            ));
        }
    });
    (
        ids.iter().map(|&id| page_flow_y(&doc, id)).collect(),
        doc.nodes[parent].layout.size.height,
    )
}

#[test]
fn a_run_of_boxes_tied_by_avoided_breaks_moves_with_the_edges_below_it() {
    // The boxes end at 90 and the 30px of padding below them do not fit. The
    // last box is tied to the one before it, so the break falls before that
    // one: both move, and the padding with them.
    for (second, third) in [
        ("break-after:avoid", ""),
        ("break-after:avoid-page", ""),
        ("", "break-before:avoid"),
        ("", "break-before:avoid-page"),
        ("break-after:avoid", "break-before:avoid"),
    ] {
        assert_eq!(
            tied_boxes(
                "padding-bottom:30px",
                &[(40, ""), (30, second), (20, third)]
            ),
            (vec![0.0, 100.0, 130.0], 180.0),
            "{second} {third}"
        );
    }
    // A border closes the block the same way.
    assert_eq!(
        tied_boxes(
            "border-bottom:30px solid",
            &[(40, ""), (30, "break-after:avoid"), (20, "")]
        ),
        (vec![0.0, 100.0, 130.0], 180.0)
    );
}

#[test]
fn a_run_of_more_boxes_moves_from_the_first_break_that_is_not_avoided() {
    // The last box is tied to the one before it, which is tied to the one
    // before: the run starts at the second box, after a break that is free.
    assert_eq!(
        tied_boxes(
            "padding-bottom:30px",
            &[
                (30, ""),
                (30, "break-after:avoid"),
                (20, "break-after:avoid"),
                (10, "")
            ]
        ),
        (vec![0.0, 100.0, 130.0, 150.0], 190.0)
    );
    // Avoided breaks further up do not count when the break before the last
    // box is free: it alone moves, as it does with no avoided break at all.
    assert_eq!(
        tied_boxes(
            "padding-bottom:30px",
            &[(30, ""), (30, "break-after:avoid"), (10, ""), (20, "")]
        ),
        (vec![0.0, 30.0, 60.0, 100.0], 150.0)
    );
}

#[test]
fn a_run_of_tied_boxes_stays_when_the_edges_fit_and_moves_when_the_next_page_cannot_hold_them() {
    let tied = [(40, ""), (30, "break-after:avoid"), (20, "")];
    assert_eq!(
        tied_boxes("padding-bottom:10px", &tied),
        (vec![0.0, 40.0, 70.0], 100.0)
    );
    // 50px of boxes and 60px of padding make 110px, more than a page. The run
    // still moves: a break before it beats one between its last box and the
    // padding, which runs on past the page edge.
    assert_eq!(
        tied_boxes("padding-bottom:60px", &tied),
        (vec![0.0, 100.0, 130.0], 210.0)
    );
}

#[test]
fn a_run_of_tied_boxes_stays_when_the_page_breaks_better_inside_one_of_them() {
    // The tied paragraph has lines enough for `orphans` and `widows` to break
    // between them, which comes later than breaking before the run.
    let (mut parent, mut last) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:30px");
        let paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;\
             line-height:10px;break-after:avoid",
        );
        doc.append_text(paragraph, ["aaa"; 4].join(" "));
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 120.0);
}

#[test]
fn an_avoided_break_before_the_last_box_gives_way_when_it_ties_it_to_the_first_box() {
    // No break between boxes is left to take: the last box moves alone with
    // the padding, the avoided break giving way, as browsers do.
    for (first, second) in [
        ("break-after:avoid", ""),
        ("", "break-before:avoid"),
        ("", "break-before:avoid-page"),
    ] {
        assert_eq!(
            tied_boxes("padding-bottom:30px", &[(40, first), (50, second)]),
            (vec![0.0, 100.0], 180.0),
            "{first} {second}"
        );
    }
    // Tied boxes in a row do the same.
    assert_eq!(
        tied_boxes(
            "padding-bottom:30px",
            &[
                (30, "break-after:avoid"),
                (30, "break-after:avoid"),
                (30, "")
            ]
        ),
        (vec![0.0, 30.0, 100.0], 160.0)
    );
}

#[test]
fn a_run_that_starts_the_block_stays_when_the_page_breaks_better_inside_its_first_box() {
    // The paragraph has lines enough for `orphans` and `widows` to break
    // between them: the last box is not moved away from it alone.
    let (mut parent, mut last) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:30px");
        let paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;\
             line-height:10px;break-after:avoid",
        );
        doc.append_text(paragraph, ["aaa"; 4].join(" "));
        last = block(doc, parent, "display:block;height:40px");
    });
    assert_eq!(page_flow_y(&doc, last), 40.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 110.0);
}

#[test]
fn a_block_of_tied_boxes_that_ends_its_parent_moves_whole_with_the_parents_edges() {
    // The run is all the inner block holds, and the padding of the parent
    // below it does not fit: the page breaks before the inner block, which is
    // a better place than the avoided break inside it.
    let (mut inner, mut last) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        inner = block(doc, parent, "display:block");
        block(doc, inner, "display:block;height:30px;break-after:avoid");
        last = block(doc, inner, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, inner), 100.0);
    assert_eq!(page_flow_y(&doc, last), 130.0);
}

#[test]
fn a_block_of_tied_boxes_with_edges_of_its_own_moves_whole_with_the_parents_edges() {
    // The padding of the outer block below the inner one does not fit, and a
    // break before the inner block is a better one than the avoided break
    // before its last box.
    let (mut inner, mut last) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let outer = block(doc, body, "display:block;padding-bottom:20px");
        block(doc, outer, "display:block;height:20px");
        inner = block(doc, outer, "display:block;padding-bottom:20px");
        block(doc, inner, "display:block;height:40px;break-after:avoid");
        last = block(doc, inner, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, inner), 100.0);
    assert_eq!(page_flow_y(&doc, last), 140.0);
}

#[test]
fn a_run_of_tied_boxes_in_an_inner_block_moves_with_the_edges_of_both_blocks() {
    // The inner block ends the outer one: 30px of padding each, and the tie is
    // between boxes of the inner block, after a free break.
    let (mut third, mut after) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let outer = block(doc, body, "display:block;padding-bottom:20px");
        block(doc, outer, "display:block;height:10px");
        let inner = block(doc, outer, "display:block;padding-bottom:20px");
        block(doc, inner, "display:block;height:30px");
        block(doc, inner, "display:block;height:30px;break-after:avoid");
        third = block(doc, inner, "display:block;height:10px");
        after = block(doc, body, "display:block;height:10px");
    });
    assert_eq!(page_flow_y(&doc, third), 130.0);
    assert_eq!(page_flow_y(&doc, after), 180.0);
}

#[test]
fn an_avoided_break_ties_a_last_box_that_holds_one_row_of_boxes_to_the_box_before_it() {
    // A flex container, a grid or a table that the page cannot break inside
    // is tied to the box before it as a block is: both go to the next page
    // with the padding.
    for third in [
        "display:flex",
        "display:table",
        "display:grid",
        "display:block",
    ] {
        let (mut second, mut last) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, "display:block;padding-bottom:30px");
            block(doc, parent, "display:block;height:40px");
            second = block(doc, parent, "display:block;height:30px;break-after:avoid");
            last = block(doc, parent, &format!("height:20px;{third}"));
        });
        assert_eq!(
            (page_flow_y(&doc, second), page_flow_y(&doc, last)),
            (100.0, 130.0),
            "{third}"
        );
    }
}

#[test]
fn an_avoided_break_before_a_float_leaves_the_box_before_it_as_the_last_one() {
    // The float is not a box of the flow: the second box ends the parent, and
    // its padding fits the page.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px;break-after:avoid");
        last = block(doc, parent, "display:block;height:20px;float:left");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
}

#[test]
fn a_float_between_the_tied_boxes_leaves_them_alone() {
    // The float is a flow of its own and has rules of its own.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px;break-after:avoid");
        block(
            doc,
            parent,
            "display:block;float:left;width:10px;height:10px",
        );
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
}

#[test]
fn a_page_name_change_between_the_tied_boxes_leaves_them_alone() {
    // The named page already starts the last box on a page of its own, so the
    // run does not move ahead of it: the second box stays and the last box
    // takes the padding to the next page.
    assert_eq!(
        tied_boxes(
            "padding-bottom:30px",
            &[(40, ""), (30, "break-after:avoid"), (20, "page:foo")]
        ),
        (vec![0.0, 40.0, 100.0], 150.0)
    );
    // The same when the named page is on the box before the last.
    assert_eq!(
        tied_boxes(
            "padding-bottom:30px",
            &[(40, ""), (30, "break-after:avoid;page:foo"), (20, "")]
        ),
        (vec![0.0, 100.0, 130.0], 180.0)
    );
}

#[test]
fn a_box_around_which_a_better_break_exists_stays() {
    // The last box has boxes of its own, and the last of them is the one that
    // moves: breaking inside the box comes later than breaking before it.
    let (mut last, mut inner) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        last = block(doc, parent, "display:block");
        block(doc, last, "display:block;height:20px");
        inner = block(doc, last, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 40.0);
    assert_eq!(page_flow_y(&doc, inner), 100.0);
}

#[test]
fn a_wrapper_around_one_unbreakable_box_moves_with_the_edges() {
    // The wrapper has nothing of its own that a page breaks at, so it moves as
    // the box in it does.
    let (mut wrapper, mut inner, mut parent) = (0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        wrapper = block(doc, parent, "display:block");
        inner = block(doc, wrapper, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, wrapper), 100.0);
    assert_eq!(page_flow_y(&doc, inner), 100.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 150.0);
}

#[test]
fn a_multicol_container_breaks_into_its_columns_and_stays() {
    // Its columns are fragmented on their own, which a move of the whole
    // container would pre-empt.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:70px");
        last = block(doc, parent, "display:block;columns:2;column-gap:0");
        block(doc, last, "display:block;height:20px");
    });
    assert!(doc.nodes[last].multicol.is_some());
    assert_eq!(page_flow_y(&doc, last), 70.0);
}

#[test]
fn a_wrapper_with_a_float_beside_its_box_moves_with_the_edges() {
    // The float sits within the wrapper, so it goes with it; it is no place
    // to break at.
    let (mut wrapper, mut float) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        wrapper = block(doc, parent, "display:block");
        float = block(doc, wrapper, "float:left;width:10px;height:5px");
        block(doc, wrapper, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, wrapper), 100.0);
    assert_eq!(page_flow_y(&doc, float), 100.0);
}

#[test]
fn a_wrapper_with_a_float_that_runs_past_it_stays() {
    // The float reaches below the wrapper, to where the page ends.
    let (mut wrapper, mut float) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        wrapper = block(doc, parent, "display:block");
        float = block(doc, wrapper, "float:left;width:10px;height:60px");
        block(doc, wrapper, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, wrapper), 70.0);
    assert_eq!(page_flow_y(&doc, float), 70.0);
}

#[test]
fn a_wrapper_with_padding_or_a_border_around_its_one_box_moves_with_the_edges() {
    // The wrapper's own edges move with it as one box, like a wrapper without
    // any.
    for css in ["padding-top:5px", "border-bottom:1px solid"] {
        let (mut wrapper, mut inner) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, "display:block;padding-bottom:30px");
            block(doc, parent, "display:block;height:40px");
            block(doc, parent, "display:block;height:30px");
            wrapper = block(doc, parent, &format!("display:block;{css}"));
            inner = block(doc, wrapper, "display:block;height:15px");
        });
        assert_eq!(page_flow_y(&doc, wrapper), 100.0, "{css}");
        assert!(page_flow_y(&doc, inner) >= 100.0, "{css}");
    }
}

#[test]
fn boxes_that_take_no_space_after_the_last_box_do_not_keep_it_from_ending_the_parent() {
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        last = block(doc, parent, "display:block;height:20px");
        block(doc, parent, "display:none");
        block(doc, parent, "position:absolute;width:10px;height:10px");
        doc.append_text(parent, " ");
    });
    assert_eq!(page_flow_y(&doc, last), 100.0);
}

#[test]
fn a_box_inside_a_contents_element_before_the_box_is_a_box_to_break_from() {
    // The boxes inside a contents element are the parent's own, so the last of
    // them is the box before the last box of the parent.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        let contents = block(doc, parent, "display:contents");
        block(doc, contents, "display:block;height:40px");
        block(doc, contents, "display:block;height:30px");
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 100.0);
}

#[test]
fn a_box_that_takes_no_space_before_the_box_is_no_box_to_break_from() {
    // Only an empty block precedes the last box, so a break before the last
    // box would be one inside the parent, at its start.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:70px");
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block");
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
}

#[test]
fn the_margin_below_the_last_box_counts_with_the_edges() {
    // The box ends at 85. The 10px of padding fit below it with a margin of
    // 4px, but not with one of 20px.
    for (margin, expected) in [(4, 70.0), (20, 100.0)] {
        let mut last = 0;
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, "display:block;padding-bottom:10px");
            block(doc, parent, "display:block;height:40px");
            block(doc, parent, "display:block;height:30px");
            last = block(
                doc,
                parent,
                &format!("display:block;height:15px;margin-bottom:{margin}px"),
            );
        });
        assert_eq!(page_flow_y(&doc, last), expected, "{margin}");
    }
}

#[test]
fn a_negative_margin_below_the_last_box_pulls_the_edges_up_under_it() {
    // The box ends at 90 and its -20px margin leaves 10px of the 30px of
    // padding below it, which fit; -40px leaves none.
    for margin in [-20, -40] {
        let mut last = 0;
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, "display:block;padding-bottom:30px");
            block(doc, parent, "display:block;height:70px");
            last = block(
                doc,
                parent,
                &format!("display:block;height:20px;margin-bottom:{margin}px"),
            );
        });
        assert_eq!(page_flow_y(&doc, last), 70.0, "{margin}");
    }
}

#[test]
fn the_margin_below_an_inner_block_counts_with_the_edges_of_the_outer_one() {
    // The inner block ends at 80 with 5px of padding, and the outer block has
    // 5px more below the margin of the inner one.
    for (margin, expected) in [(0, 40.0), (15, 100.0)] {
        let mut last = 0;
        let (doc, _) = paged_body(|doc, body| {
            let outer = block(doc, body, "display:block;padding-bottom:5px");
            let inner = block(
                doc,
                outer,
                &format!("display:block;padding-bottom:5px;margin-bottom:{margin}px"),
            );
            block(doc, inner, "display:block;height:40px");
            last = block(doc, inner, "display:block;height:40px");
        });
        assert_eq!(page_flow_y(&doc, last), expected, "{margin}");
    }
}

#[test]
fn a_relatively_positioned_last_box_moves_and_keeps_its_offset() {
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:70px");
        last = block(
            doc,
            parent,
            "display:block;height:20px;position:relative;top:7px",
        );
    });
    assert_eq!(page_flow_y(&doc, last), 107.0);
}

#[test]
fn every_closing_edge_of_the_nested_blocks_the_box_ends_counts() {
    // The box ends the inner block, which ends the outer one: 20px of
    // padding each. The outer padding alone would fit beside the box, both
    // do not.
    let (mut last, mut outer) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        outer = block(doc, body, "display:block;padding-bottom:20px");
        block(doc, outer, "display:block;height:40px");
        let inner = block(doc, outer, "display:block;padding-bottom:20px");
        block(doc, inner, "display:block;height:20px");
        last = block(doc, inner, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 100.0);
    assert_eq!(doc.nodes[outer].layout.size.height, 160.0);
}

#[test]
fn an_inner_block_that_the_box_does_not_end_stops_the_edges_there() {
    // Another box follows the inner block, so the padding of the outer block
    // below the inner one is not carried by the last box of the inner
    // block: 5px of padding fit. The box after the inner block carries the
    // outer padding and moves.
    let (mut last, mut after, mut outer) = (0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        outer = block(doc, body, "display:block;padding-bottom:60px");
        let inner = block(doc, outer, "display:block;padding-bottom:5px");
        block(doc, inner, "display:block;height:40px");
        last = block(doc, inner, "display:block;height:20px");
        after = block(doc, outer, "display:block;height:10px");
    });
    assert_eq!(page_flow_y(&doc, last), 40.0);
    assert_eq!(page_flow_y(&doc, after), 100.0);
    assert_eq!(doc.nodes[outer].layout.size.height, 170.0);
}

/// `body > div(padding-bottom:40px) > [div, paragraph]` where the paragraph is
/// `lines` lines of 10px Ahem text, 30px wide, and ends at 70px. Returns the
/// page-flow y of the paragraph and the height of the div.
fn closed_paragraph(lines: usize, paragraph_css: &str) -> (f32, f32) {
    closed_paragraph_with(lines, 40, paragraph_css)
}

/// [`closed_paragraph`] with `padding` pixels of padding below the div.
fn closed_paragraph_with(lines: usize, padding: u32, paragraph_css: &str) -> (f32, f32) {
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        parent = block(
            doc,
            body,
            &format!("display:block;padding-bottom:{padding}px"),
        );
        block(
            doc,
            parent,
            &format!("display:block;height:{}px", 70 - 10 * lines),
        );
        paragraph = block(
            doc,
            parent,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
    });
    assert!(doc.nodes[paragraph].is_ifc_root());
    (
        page_flow_y(&doc, paragraph),
        doc.nodes[parent].layout.size.height,
    )
}

#[test]
fn lines_that_orphans_and_widows_keep_together_move_with_the_edges() {
    // The lines end at 70 on the page and the padding below them does not.
    // The paragraph moves whole to the top of the next page, and the padding
    // follows it.
    for lines in [2, 1] {
        assert_eq!(
            closed_paragraph(lines, ""),
            (100.0, 140.0 + 10.0 * lines as f32),
            "{lines} lines"
        );
    }
}

#[test]
fn a_paragraph_too_short_for_both_constraints_keeps_its_orphans_before_the_edges() {
    // Three lines cannot have two orphans and two widows. The first two stay,
    // and the last one moves with the padding, as Chromium prints it.
    assert_eq!(closed_paragraph(3, ""), (40.0, 150.0));
    // Orphans of three leave no line to move, so the paragraph moves whole.
    assert_eq!(closed_paragraph(3, "orphans:3"), (100.0, 170.0));
}

#[test]
fn the_last_lines_that_widows_keep_together_move_with_the_edges() {
    // Lines that fit apart stay where the paragraph starts, and only the last
    // `widows` lines move with the padding. Four lines make a break between
    // the second and third.
    assert_eq!(closed_paragraph(4, ""), (30.0, 160.0));
    // Six lines, the last two moving from 50 to the top of the next page.
    assert_eq!(closed_paragraph(6, ""), (10.0, 160.0));
    // Fewer widows move fewer lines, and the break is the one that the
    // widows allow, not the earliest one.
    assert_eq!(closed_paragraph(6, "orphans:1;widows:1"), (10.0, 150.0));
    assert_eq!(closed_paragraph(6, "orphans:1;widows:3"), (10.0, 170.0));
    assert_eq!(closed_paragraph(3, "orphans:1;widows:1"), (40.0, 150.0));
    assert_eq!(closed_paragraph(2, "orphans:1;widows:1"), (50.0, 150.0));
    // A paragraph that avoids breaks inside keeps all its lines.
    for avoid in ["break-inside:avoid", "break-inside:avoid-page"] {
        assert_eq!(closed_paragraph(6, avoid), (10.0, 110.0), "{avoid}");
    }
}

#[test]
fn the_padding_of_the_paragraph_itself_moves_with_its_last_lines() {
    // The paragraph has 20px of padding below its lines and the div 40px more:
    // the lines end at 70, and 60px of edges do not fit after them.
    assert_eq!(closed_paragraph(6, "padding-bottom:20px"), (10.0, 180.0));
}

#[test]
fn lines_move_when_the_next_page_cannot_hold_them_with_the_edges() {
    // The last two lines and 90px of padding make 110px, more than a page.
    // The lines still go to the next page: a break between lines beats one
    // between the last line and the padding, which runs on past the page edge.
    assert_eq!(closed_paragraph_with(6, 90, ""), (10.0, 210.0));
}

/// `body > [div(10px), div(padding-bottom:40px) > paragraph]` where the
/// paragraph is `lines` lines of 10px Ahem text, 30px wide. Returns the height
/// of the div and the page-flow y of the paragraph's first line.
fn lone_paragraph(lines: usize) -> (f32, f32) {
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:10px");
        parent = block(doc, body, "display:block;padding-bottom:40px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
    });
    (
        doc.nodes[parent].layout.size.height,
        page_flow_y(&doc, paragraph),
    )
}

#[test]
fn a_paragraph_alone_in_its_parent_moves_its_last_lines_with_the_edges() {
    // No box before the paragraph in the div is there to break from, but its
    // lines are: the last two move to the next page with the padding, which
    // would otherwise be left alone there. Six lines end at 70.
    assert_eq!(lone_paragraph(6), (150.0, 10.0));
    // Four lines end at 50 and fit with the padding.
    assert_eq!(lone_paragraph(4), (80.0, 10.0));
}

#[test]
fn a_paragraph_of_one_line_alone_in_its_parent_moves_whole_with_the_edges() {
    // The line ends at 100 and the padding below it does not fit. There is no
    // place to break in the line, and the page breaks before the div.
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:90px");
        parent = block(doc, body, "display:block;padding-bottom:40px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "aaa");
    });
    assert_eq!(page_flow_y(&doc, parent), 100.0);
    assert_eq!(page_flow_y(&doc, paragraph), 100.0);
}

#[test]
fn the_lines_of_a_paragraph_in_a_float_keep_their_place() {
    // A float is a flow of its own: the padding that closes a block in it does
    // not move the paragraph's lines.
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let float = block(doc, body, "float:left;width:30px");
        parent = block(doc, float, "display:block;padding-bottom:40px");
        block(doc, parent, "display:block;height:10px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, ["aaa"; 6].join(" "));
    });
    assert_eq!(doc.nodes[parent].layout.size.height, 110.0);
}

#[test]
fn a_paragraph_with_a_block_among_its_lines_keeps_its_last_lines() {
    // The lines run in two runs around a block, which the page breaks at.
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:40px");
        block(doc, parent, "display:block;height:10px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "aaa bbb");
        block(doc, paragraph, "display:block;height:10px");
        doc.append_text(paragraph, "ccc ddd eee");
    });
    assert_eq!(doc.nodes[parent].layout.size.height, 110.0);
}

#[test]
fn a_paragraph_with_a_block_between_its_lines_breaks_at_the_block_and_stays() {
    // The paragraph is two lines with a block between them, which the page
    // breaks at, so it does not move whole with the edges.
    let mut paragraph = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:40px");
        block(doc, parent, "display:block;height:40px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "a");
        block(doc, paragraph, "display:block;height:10px");
        doc.append_text(paragraph, "b");
    });
    let lines = doc.nodes[paragraph]
        .ifc
        .as_ref()
        .and_then(|ifc| ifc.lines.as_ref())
        .unwrap();
    assert!(!lines.block_line_starts.is_empty());
    assert_eq!(page_flow_y(&doc, paragraph), 40.0);
}

#[test]
fn boxes_that_are_not_in_the_block_flow_of_the_parent_stay() {
    for css in ["display:flex;flex-direction:column", "display:grid"] {
        let mut last = 0;
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, &format!("padding-bottom:30px;{css}"));
            block(doc, parent, "display:block;height:70px");
            last = block(doc, parent, "display:block;height:20px");
        });
        assert_eq!(page_flow_y(&doc, last), 70.0, "{css}");
    }
    // A float and a sticky box are not boxes of the flow that end the parent
    // or are moved with its edges, nor is a sticky flex container or grid.
    for css in [
        "float:left;width:20px",
        "position:sticky;top:0",
        "display:flex;position:sticky;top:0",
        "display:grid;position:sticky;top:0",
    ] {
        let (mut first, mut last) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, "display:block;padding-bottom:30px");
            first = block(doc, parent, "display:block;height:70px");
            last = block(doc, parent, &format!("display:block;height:20px;{css}"));
        });
        assert_eq!(page_flow_y(&doc, first), 0.0, "{css}");
        assert_eq!(page_flow_y(&doc, last), 70.0, "{css}");
    }
}

#[test]
fn the_edges_of_the_body_are_not_closing_edges() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px;padding-bottom:30px"),
    );
    block(&mut doc, body, "display:block;height:70px");
    let last = block(&mut doc, body, "display:block;height:20px");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_pages(&mut doc, &cascade, page).unwrap();
    assert_eq!(page_flow_y(&doc, last), 70.0);
}

/// `body > [div(parent_css) > [div(30px), div(inner_css) > boxes], div(10px)]`
/// on 100px pages, each of the `boxes` being a div of the given height and
/// style. Returns the page-flow y of each of them and of the block after the
/// parent.
fn inner_block_of_boxes(parent_css: &str, inner_css: &str, boxes: &[(u32, &str)]) -> Vec<f32> {
    let (mut ids, mut after) = (Vec::new(), 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, &format!("display:block;{parent_css}"));
        block(doc, parent, "display:block;height:30px");
        let inner = block(doc, parent, &format!("display:block;{inner_css}"));
        for &(height, css) in boxes {
            ids.push(block(
                doc,
                inner,
                &format!("display:block;height:{height}px;{css}"),
            ));
        }
        after = block(doc, body, "display:block;height:10px");
    });
    ids.push(after);
    ids.into_iter().map(|id| page_flow_y(&doc, id)).collect()
}

#[test]
fn a_last_box_that_holds_only_tied_boxes_moves_whole_with_the_closing_edges() {
    let tied_pair = [(30, "break-after:avoid"), (35, "")];
    // The last box of the parent holds two boxes that an avoided break ties.
    // It ends at 95 and fits the page, the 15px of padding below it do not,
    // and the page cannot break inside it at a place better than before it:
    // it moves whole, as the two boxes would if they were the parent's own.
    assert_eq!(
        inner_block_of_boxes("padding-bottom:15px", "", &tied_pair),
        [100.0, 130.0, 180.0]
    );
    // The avoided break may come from the box after it.
    assert_eq!(
        inner_block_of_boxes(
            "padding-bottom:15px",
            "",
            &[(30, ""), (35, "break-before:avoid")]
        ),
        [100.0, 130.0, 180.0]
    );
    // The last of the tied boxes may be a flex container, a grid or a table
    // that moves whole.
    for last in ["display:flex", "display:grid", "display:table"] {
        assert_eq!(
            inner_block_of_boxes(
                "padding-bottom:15px",
                "",
                &[(30, "break-after:avoid"), (35, last)]
            ),
            [100.0, 130.0, 180.0],
            "{last}"
        );
    }
    // A chain of three.
    assert_eq!(
        inner_block_of_boxes(
            "padding-bottom:15px",
            "",
            &[
                (20, "break-after:avoid"),
                (20, "break-after:avoid"),
                (25, "")
            ]
        ),
        [100.0, 120.0, 140.0, 180.0]
    );
    // The padding and border of the block itself move with it.
    assert_eq!(
        inner_block_of_boxes(
            "padding-bottom:15px",
            "padding-bottom:5px",
            &[(30, "break-after:avoid"), (30, "")]
        ),
        [100.0, 130.0, 180.0]
    );
    assert_eq!(
        inner_block_of_boxes(
            "padding-bottom:15px",
            "padding-top:5px",
            &[(30, "break-after:avoid"), (30, "")]
        ),
        [105.0, 135.0, 180.0]
    );
    // The edges fit the page: nothing moves.
    assert_eq!(
        inner_block_of_boxes("padding-bottom:5px", "", &tied_pair),
        [30.0, 60.0, 100.0]
    );
}

#[test]
fn a_last_box_that_holds_boxes_with_a_break_to_take_between_them_keeps_its_place() {
    // The last box of the parent can break after its second box, where the
    // third box follows with the edges: that one moves, as the last box of the
    // parent would.
    assert_eq!(
        inner_block_of_boxes(
            "padding-bottom:15px",
            "",
            &[(30, "break-after:avoid"), (30, ""), (10, "")]
        ),
        [30.0, 60.0, 100.0, 125.0]
    );
}

/// `body > [div(parent_css) > [div(30px), div > <built boxes>], div(10px)]`
/// on 100px pages. `build_inner` adds the boxes to the inner block and
/// returns them. Returns the page-flow y of each of them and of the block
/// after the parent, which has `tail_css` as well.
fn inner_block_built(
    parent_css: &str,
    build_inner: impl FnOnce(&mut Document, usize) -> Vec<usize>,
    tail_css: &str,
) -> Vec<f32> {
    let (mut ids, mut after) = (Vec::new(), 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, &format!("display:block;{parent_css}"));
        block(doc, parent, "display:block;height:30px");
        let inner = block(doc, parent, "display:block");
        ids = build_inner(doc, inner);
        after = block(doc, body, &format!("display:block;height:10px;{tail_css}"));
    });
    ids.push(after);
    ids.into_iter().map(|id| page_flow_y(&doc, id)).collect()
}

fn tied_pair(doc: &mut Document, inner: usize) -> Vec<usize> {
    vec![
        block(doc, inner, "display:block;height:30px;break-after:avoid"),
        block(doc, inner, "display:block;height:35px"),
    ]
}

#[test]
fn boxes_tied_through_a_contents_wrapper_move_whole_in_their_order() {
    // The wrapper generates no box: its two boxes and the one after it make a
    // chain of three, which the closing edges of the parent take along. The
    // second break is avoided by the box after it, so the order of the boxes
    // matters.
    assert_eq!(
        inner_block_built(
            "padding-bottom:15px",
            |doc, inner| {
                let wrapper = block(doc, inner, "display:contents");
                vec![
                    block(doc, wrapper, "display:block;height:20px;break-after:avoid"),
                    block(doc, wrapper, "display:block;height:20px"),
                    block(doc, inner, "display:block;height:20px;break-before:avoid"),
                ]
            },
            ""
        ),
        [100.0, 120.0, 140.0, 175.0]
    );
}

#[test]
fn a_last_box_whose_boxes_are_not_all_tied_by_avoided_breaks_keeps_its_place() {
    let keeps_place = |tied: &dyn Fn(&mut Document, usize) -> Vec<usize>, tail_css: &str| {
        let ys = inner_block_built(
            "padding-bottom:15px",
            |doc, inner| tied(doc, inner),
            tail_css,
        );
        ys[0] < 100.0
    };
    // A forced break between the boxes starts a page there whatever tied them.
    assert!(keeps_place(
        &|doc, inner| vec![
            block(doc, inner, "display:block;height:30px;break-after:page"),
            block(doc, inner, "display:block;height:35px"),
        ],
        ""
    ));
    // Neither does a flex container that stacks its items: the page breaks
    // between them.
    assert!(keeps_place(
        &|doc, inner| {
            let first = block(doc, inner, "display:block;height:30px;break-after:avoid");
            let column = block(doc, inner, "display:flex;flex-direction:column");
            block(doc, column, "display:block;height:15px");
            block(doc, column, "display:block;height:20px");
            vec![first, column]
        },
        ""
    ));
    // Nor does a box that has a break to take inside it.
    assert!(keeps_place(
        &|doc, inner| {
            let first = block(doc, inner, "display:block;break-after:avoid");
            block(doc, first, "display:block;height:15px");
            block(doc, first, "display:block;height:15px");
            vec![first, block(doc, inner, "display:block;height:35px")]
        },
        ""
    ));
    // A named page may start between the boxes whatever ties them.
    assert!(keeps_place(&tied_pair, "page:chapter"));
    // The same boxes without any of that move whole.
    assert!(!keeps_place(&tied_pair, ""));
}

#[test]
fn a_last_box_that_holds_a_box_with_generated_boxes_keeps_its_place() {
    // The generated box is not among the children of its block, which looks
    // like one that holds one box, but the page can break between the two.
    let ys = |generated: bool| {
        let (mut pair, mut after) = (Vec::new(), 0);
        let (doc, _) = paged_body(|doc, body| {
            let parent = block(doc, body, "display:block;padding-bottom:15px");
            block(doc, parent, "display:block;height:30px");
            let inner = block(doc, parent, "display:block");
            let first = doc.append_element(
                Some(inner),
                "section",
                Some("display:block;break-after:avoid"),
            );
            block(doc, first, "display:block;height:25px");
            pair = vec![first, block(doc, inner, "display:block;height:35px")];
            after = block(doc, body, "display:block;height:10px");
            if generated {
                let style = doc.append_element(Some(0), "style", None::<&str>);
                doc.append_text(
                    style,
                    "section::before { content: \"x\"; display: block; height: 10px }",
                );
            }
        });
        [pair[0], pair[1], after].map(|id| page_flow_y(&doc, id))
    };
    // Without it the two boxes move whole with the edges of the parent.
    assert_eq!(ys(false), [100.0, 125.0, 175.0]);
    assert!(ys(true)[0] < 100.0);
}

/// `body > [div(prefix px), div(padding-bottom:40px) > [wrapper >] paragraph,
/// div(10px)]` where the paragraph is `lines` lines of 10px Ahem text, 30px
/// wide, and the pages are 100px. The wrapper has 10px of padding below it.
/// Returns the page-flow y of the paragraph, the height of the div and the
/// page-flow y of the block after it.
fn card_of_a_paragraph(
    prefix: u32,
    lines: usize,
    paragraph_css: &str,
    wrapped: bool,
) -> (f32, f32, f32) {
    let (mut card, mut paragraph, mut after) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{prefix}px"));
        card = block(doc, body, "display:block;padding-bottom:40px");
        let parent = if wrapped {
            block(doc, card, "display:block;padding-bottom:10px")
        } else {
            card
        };
        paragraph = block(
            doc,
            parent,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
        after = block(doc, body, "display:block;height:10px");
    });
    assert!(doc.nodes[paragraph].is_ifc_root());
    (
        page_flow_y(&doc, paragraph),
        doc.nodes[card].layout.size.height,
        page_flow_y(&doc, after),
    )
}

#[test]
fn a_card_of_two_lines_sends_its_last_line_with_the_edges() {
    // The lines end at 70 and the 40px of padding below them do not fit. The
    // paragraph is the only box in the card, so no break before it is better
    // than the one between its lines, whatever the orphans and widows say:
    // the first line stays and the second goes to the next page with the
    // padding, as Chromium prints it.
    for css in [
        "",
        "orphans:1",
        "widows:1",
        "orphans:1;widows:1",
        "orphans:3;widows:3",
    ] {
        assert_eq!(
            card_of_a_paragraph(50, 2, css, false),
            (50.0, 100.0, 150.0),
            "{css}"
        );
    }
    // The edges of a wrapper around the paragraph move with it.
    assert_eq!(card_of_a_paragraph(50, 2, "", true), (50.0, 110.0, 160.0));
}

#[test]
fn a_card_of_a_few_lines_sends_the_lines_that_the_orphans_leave() {
    // Orphans of three cannot be kept with a last line to move: the first two
    // lines stay and the last one goes.
    assert_eq!(
        card_of_a_paragraph(40, 3, "orphans:3", false),
        (40.0, 110.0, 150.0)
    );
    // Fewer orphans leave room for the widows.
    assert_eq!(
        card_of_a_paragraph(40, 3, "orphans:1", false),
        (40.0, 120.0, 160.0)
    );
}

#[test]
fn a_card_of_two_lines_stays_when_its_edges_fit() {
    // The padding fits below the lines.
    assert_eq!(card_of_a_paragraph(20, 2, "", false), (20.0, 60.0, 80.0));
}

#[test]
fn a_card_of_two_lines_sends_its_last_line_when_the_next_page_cannot_hold_it_with_the_edges() {
    // Two lines and 95px of padding make more than a page. The last line
    // still goes to the next page, with the padding running on past its end.
    for padding in [95, 250] {
        let (mut card, mut after) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, "display:block;height:50px");
            card = block(
                doc,
                body,
                &format!("display:block;padding-bottom:{padding}px"),
            );
            let paragraph = block(
                doc,
                card,
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
            );
            doc.append_text(paragraph, "aaa aaa");
            after = block(doc, body, "display:block;height:10px");
        });
        // The first line stays at 50, the second starts the next page at 100
        // and the padding follows it.
        assert_eq!(
            doc.nodes[card].layout.size.height,
            60.0 + padding as f32,
            "{padding}px"
        );
        assert_eq!(
            page_flow_y(&doc, after),
            100.0 + 10.0 + padding as f32,
            "{padding}px"
        );
    }
}

#[test]
fn a_card_of_two_lines_after_a_box_in_the_card_moves_whole() {
    // With a box before the paragraph in the card, a break between the two is
    // a better one: the paragraph moves whole, and orphans of two keep it so.
    assert_eq!(closed_paragraph(2, ""), (100.0, 160.0));
    // When the next page cannot hold the paragraph with the edges, it moves
    // all the same: a break before it beats one between it and the edges.
    assert_eq!(closed_paragraph_with(2, 90, ""), (100.0, 210.0));
}

/// `body > [div(20px), outer > [div(30px), wrapper > paragraph], div(10px)]`
/// where the paragraph is two lines of 10px Ahem text, 30px wide, on 100px
/// pages. Returns the height of the paragraph, of the wrapper and the
/// page-flow y of the block after `outer`.
fn wrapper_after_a_box(outer_css: &str, wrapper_css: &str) -> (f32, f32, f32) {
    let (mut paragraph, mut wrapper, mut after) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:20px");
        let outer = block(doc, body, &format!("display:block;{outer_css}"));
        block(doc, outer, "display:block;height:30px");
        wrapper = block(doc, outer, &format!("display:block;{wrapper_css}"));
        paragraph = block(
            doc,
            wrapper,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "aaa aaa");
        after = block(doc, body, "display:block;height:10px");
    });
    (
        doc.nodes[paragraph].layout.size.height,
        doc.nodes[wrapper].layout.size.height,
        page_flow_y(&doc, after),
    )
}

#[test]
fn a_paragraph_after_a_box_inside_the_block_that_the_edges_close_is_not_split() {
    // The wrapper comes after a box in the card whose padding closes it: a
    // break before the wrapper is left to the rules for it, and the two lines
    // are not split for orphans that they cannot keep.
    assert_eq!(
        wrapper_after_a_box("padding-bottom:40px", "padding-bottom:10px"),
        (20.0, 30.0, 120.0)
    );
}

#[test]
fn a_paragraph_after_a_box_outside_the_block_that_the_edges_close_is_split() {
    // The card has no edges of its own; the wrapper's padding closes it, and
    // no box comes before the paragraph inside the wrapper. The last line goes
    // to the next page with the padding, whatever comes before the wrapper.
    assert_eq!(
        wrapper_after_a_box("", "padding-bottom:40px"),
        (60.0, 100.0, 150.0)
    );
}

/// `body > [div(50px), div(padding-bottom:40px) > [div(lead_css),
/// paragraph]]` where the paragraph is two lines of 10px Ahem text, 30px wide,
/// on 100px pages. Returns the height of the paragraph and the page-flow y of
/// the paragraph.
fn card_after_a_lead(lead_css: &str) -> (f32, f32) {
    let mut paragraph = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        let card = block(doc, body, "display:block;padding-bottom:40px");
        block(doc, card, &format!("display:block;{lead_css}"));
        paragraph = block(
            doc,
            card,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "aaa aaa");
    });
    (
        doc.nodes[paragraph].layout.size.height,
        page_flow_y(&doc, paragraph),
    )
}

#[test]
fn a_paragraph_after_an_empty_block_in_the_card_is_not_split() {
    // An empty block before the paragraph is a box to break from, as it is in
    // Chromium: a break between the two is a better one than a break between
    // the lines, so the two lines are not split for orphans that they cannot
    // keep.
    assert_eq!(card_after_a_lead(""), (20.0, 50.0));
}

#[test]
fn a_paragraph_after_an_out_of_flow_box_in_the_card_is_split() {
    // An absolutely positioned box is no box in the flow of the card: the
    // paragraph is still the only one in it, and its last line goes to the
    // next page with the padding.
    assert_eq!(
        card_after_a_lead("position:absolute;width:10px;height:10px"),
        (60.0, 50.0)
    );
}

#[test]
fn a_card_that_avoids_a_break_inside_keeps_its_paragraph_whole() {
    // The card cannot break inside: it moves to the next page with the
    // padding rather than sending a line of the paragraph.
    let (mut card, mut paragraph, mut after) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        card = block(
            doc,
            body,
            "display:block;padding-bottom:40px;break-inside:avoid",
        );
        paragraph = block(
            doc,
            card,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, "aaa aaa");
        after = block(doc, body, "display:block;height:10px");
    });
    assert_eq!(
        (
            doc.nodes[paragraph].layout.size.height,
            page_flow_y(&doc, card),
            page_flow_y(&doc, after)
        ),
        (20.0, 100.0, 160.0)
    );
}

/// `body > div(padding-bottom:30px) > [div(40px), div(30px), ...]` on 100px
/// pages, where `build` adds the last box to the div and returns the boxes to
/// look at. Returns their page-flow y and the height of the div.
fn card_ending_with(build: impl FnOnce(&mut Document, usize) -> Vec<usize>) -> (Vec<f32>, f32) {
    let (mut ids, mut card) = (Vec::new(), 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        card = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, card, "display:block;height:40px");
        block(doc, card, "display:block;height:30px");
        ids = build(doc, card);
    });
    (
        ids.iter().map(|&id| page_flow_y(&doc, id)).collect(),
        doc.nodes[card].layout.size.height,
    )
}

/// A table of `rows` rows with one cell of `height` each, in a row group or
/// not. Returns the table, then each row and its cell.
fn table_of_rows(
    doc: &mut Document,
    parent: usize,
    grouped: bool,
    rows: usize,
    height: u32,
) -> Vec<usize> {
    let table = block(doc, parent, "display:table;border-spacing:0");
    let holder = if grouped {
        block(doc, table, "display:table-row-group")
    } else {
        table
    };
    let mut ids = vec![table];
    for _ in 0..rows {
        let row = block(doc, holder, "display:table-row");
        let cell = block(
            doc,
            row,
            &format!("display:table-cell;padding:0;height:{height}px"),
        );
        ids.extend([row, cell]);
    }
    ids
}

#[test]
fn a_last_box_that_holds_one_row_of_boxes_moves_with_the_edges() {
    // The flex container, the grid and the table have one row of boxes that
    // the page cannot break inside, so they move whole as a block does:
    // Chromium sends them to the next page with the padding.
    type Shape = fn(&mut Document, usize) -> Vec<usize>;
    let shapes: [(&str, Shape); 14] = [
        ("grid", |doc, card| {
            vec![block(doc, card, "display:grid;height:20px")]
        }),
        ("flex container", |doc, card| {
            vec![block(doc, card, "display:flex;height:20px")]
        }),
        ("table", |doc, card| {
            vec![block(doc, card, "display:table;height:20px")]
        }),
        ("grid in a wrapper", |doc, card| {
            let wrapper = block(doc, card, "display:block");
            vec![wrapper, block(doc, wrapper, "display:grid;height:20px")]
        }),
        ("flex line of two items", |doc, card| {
            let line = block(doc, card, "display:flex");
            vec![
                line,
                block(doc, line, "display:block;width:40px;height:20px"),
                block(doc, line, "display:block;width:40px;height:20px"),
            ]
        }),
        ("flex column of one item", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            vec![column, block(doc, column, "display:block;height:20px")]
        }),
        ("grid of two rows", |doc, card| {
            let grid = block(doc, card, "display:grid;grid-template-columns:50px");
            vec![
                grid,
                block(doc, grid, "display:block;height:10px"),
                block(doc, grid, "display:block;height:10px"),
            ]
        }),
        (
            "flex line with an item that has boxes to break between",
            |doc, card| {
                let line = block(doc, card, "display:flex");
                let item = block(doc, line, "display:block;width:40px");
                block(doc, item, "display:block;height:10px");
                block(doc, item, "display:block;height:10px");
                vec![line, item]
            },
        ),
        ("grid row of two items", |doc, card| {
            let grid = block(doc, card, "display:grid;grid-template-columns:40px 40px");
            vec![
                grid,
                block(doc, grid, "display:block;height:20px"),
                block(doc, grid, "display:block;height:20px"),
            ]
        }),
        ("table row", |doc, card| {
            table_of_rows(doc, card, false, 1, 20)
        }),
        ("table row in a row group", |doc, card| {
            table_of_rows(doc, card, true, 1, 20)
        }),
        ("flex item that holds a grid", |doc, card| {
            let line = block(doc, card, "display:flex");
            let item = block(doc, line, "display:block;width:40px");
            vec![line, item, block(doc, item, "display:grid;height:20px")]
        }),
        ("grid of a wrapper with a float", |doc, card| {
            let wrapper = block(doc, card, "display:block");
            block(doc, wrapper, "float:left;width:10px;height:5px");
            vec![wrapper, block(doc, wrapper, "display:grid;height:20px")]
        }),
        // A box that is not rendered forces no break.
        ("grid with a hidden box that has a break", |doc, card| {
            let grid = block(doc, card, "display:grid;height:20px");
            block(doc, grid, "display:none;break-before:page");
            vec![grid]
        }),
    ];
    for (label, build) in shapes {
        let (ys, height) = card_ending_with(build);
        // The container starts the next page and what it holds follows it.
        assert_eq!(ys[0], 100.0, "{label}: {ys:?}");
        assert!(ys.iter().all(|&y| y >= 100.0), "{label}: {ys:?}");
        assert_eq!(height, 150.0, "{label}");
    }
}

#[test]
fn a_last_box_with_more_than_one_row_or_a_break_to_take_inside_stays() {
    // The container starts on its page: the page breaks between its rows
    // (the last of which Chromium sends to the next page with the padding),
    // inside a caption's table, or in a writing mode left to other rules.
    type Shape = fn(&mut Document, usize) -> Vec<usize>;
    let shapes: [(&str, Shape); 8] = [
        ("flex column of two items", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            block(doc, column, "display:block;height:10px");
            block(doc, column, "display:block;height:10px");
            vec![column]
        }),
        (
            "flex column of one item that has boxes to break between",
            |doc, card| {
                let column = block(doc, card, "display:flex;flex-direction:column");
                let item = block(doc, column, "display:block");
                block(doc, item, "display:block;height:10px");
                block(doc, item, "display:block;height:10px");
                vec![column]
            },
        ),
        ("flex row that wraps", |doc, card| {
            let lines = block(doc, card, "display:flex;flex-wrap:wrap");
            block(doc, lines, "display:block;width:60px;height:10px");
            block(doc, lines, "display:block;width:60px;height:10px");
            vec![lines]
        }),
        ("table of two rows", |doc, card| {
            let ids = table_of_rows(doc, card, false, 2, 10);
            vec![ids[0]]
        }),
        ("table of two rows in a row group", |doc, card| {
            let ids = table_of_rows(doc, card, true, 2, 10);
            vec![ids[0]]
        }),
        ("table with a caption", |doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            block(doc, table, "display:table-caption;height:10px");
            let row = block(doc, table, "display:table-row");
            block(doc, row, "display:table-cell;padding:0;height:10px");
            vec![table]
        }),
        ("table row group with a box that is no row", |doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            let group = block(doc, table, "display:table-row-group");
            block(doc, group, "display:block;height:20px");
            vec![table]
        }),
        ("flex container in a vertical writing mode", |doc, card| {
            let flex = block(
                doc,
                card,
                "display:flex;writing-mode:vertical-rl;height:20px",
            );
            block(doc, flex, "display:block;height:20px;width:10px");
            vec![flex]
        }),
    ];
    for (label, build) in shapes {
        let (ys, height) = card_ending_with(build);
        assert_eq!(ys, vec![70.0], "{label}");
        assert!(height < 150.0, "{label}: {height}");
    }
}

#[test]
fn a_last_box_with_a_forced_break_or_a_page_name_inside_keeps_its_place() {
    // The page breaks inside the container where the break or the page name
    // asks it to, so the container starts where it is and its last part opens
    // the next page, with the padding below it. Moving the container whole
    // would only add a page.
    type Shape = fn(&mut Document, usize) -> Vec<usize>;
    let shapes: [(&str, Shape); 7] = [
        ("grid rows with a break before the second", |doc, card| {
            let grid = block(doc, card, "display:grid;grid-template-columns:50px");
            vec![
                grid,
                block(doc, grid, "display:block;height:10px"),
                block(doc, grid, "display:block;height:10px;break-before:page"),
            ]
        }),
        ("grid rows with a break after the first", |doc, card| {
            let grid = block(doc, card, "display:grid;grid-template-columns:50px");
            vec![
                grid,
                block(doc, grid, "display:block;height:10px;break-after:page"),
                block(doc, grid, "display:block;height:10px"),
            ]
        }),
        ("grid rows with another page name", |doc, card| {
            let grid = block(doc, card, "display:grid;grid-template-columns:50px");
            vec![
                grid,
                block(doc, grid, "display:block;height:10px"),
                block(doc, grid, "display:block;height:10px;page:foo"),
            ]
        }),
        ("grid item with a break between its boxes", |doc, card| {
            let grid = block(doc, card, "display:grid");
            let item = block(doc, grid, "display:block");
            vec![
                grid,
                block(doc, item, "display:block;height:10px"),
                block(doc, item, "display:block;height:10px;break-before:page"),
            ]
        }),
        ("flex item with a break between its boxes", |doc, card| {
            let line = block(doc, card, "display:flex");
            let item = block(doc, line, "display:block;width:40px");
            vec![
                line,
                block(doc, item, "display:block;height:10px"),
                block(doc, item, "display:block;height:10px;break-before:always"),
            ]
        }),
        ("table cell with a break between its boxes", |doc, card| {
            let ids = table_of_rows(doc, card, false, 1, 10);
            let cell = ids[2];
            vec![
                ids[0],
                block(doc, cell, "display:block;height:10px"),
                block(doc, cell, "display:block;height:10px;break-before:page"),
            ]
        }),
        ("flex column item holding a break", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            let item = block(doc, column, "display:block");
            vec![
                column,
                block(doc, item, "display:block;height:10px"),
                block(doc, item, "display:block;height:10px;break-after:page"),
            ]
        }),
    ];
    for (label, build) in shapes {
        let (ys, _) = card_ending_with(build);
        assert_eq!(&ys[..2], &[70.0, 70.0], "{label}: {ys:?}");
    }
}
