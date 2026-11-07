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
    // the box moves. The padding does not fit after it on the next page
    // either, and starts the page after that: 200 + 90.
    assert_eq!(closed_parent("padding-bottom:90px"), (100.0, 290.0, 290.0));
}

#[test]
fn a_last_box_that_fills_the_rest_of_its_page_moves_with_the_edges() {
    // The box ends at the page end and the padding does not fit after it, so
    // the box moves to the next page whatever the padding is: the padding is
    // not left alone at the top of it. The box ends at 190 there, where the
    // padding does not fit either, and it starts the page after that.
    for padding in [15, 100, 250] {
        let (ys, height) = tied_boxes(
            &format!("padding-bottom:{padding}px"),
            &[(10, ""), (90, "")],
        );
        assert_eq!(ys, vec![0.0, 100.0], "{padding}px");
        assert_eq!(height, 200.0 + padding as f32, "{padding}px");
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
            holder_lead: 0.0,
            holder_margin_top: 0.0,
            avoided_lead: 0.0,
            avoided_margin_top: 0.0,
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
    // padding, which starts the page after it when it does not fit there.
    assert_eq!(lone_box_parent("padding-bottom:80px"), (100.0, 100.0));
}

#[test]
fn a_block_that_starts_its_page_stays() {
    // Nothing before it on the page is a box to break from, so the box stays
    // and the padding that does not fit below it starts the next page.
    let (mut parent, mut only) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:30px");
        only = block(doc, parent, "display:block;height:80px");
    });
    assert_eq!(page_flow_y(&doc, parent), 0.0);
    assert_eq!(page_flow_y(&doc, only), 0.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 130.0);
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

/// `body > [div(h, css)..., div(card_css; padding-bottom:{pad}px) > div(30px;
/// only_css), div(10px)]` on 100px pages, with one lead box per entry of `lead`.
/// Returns the page-flow y of each lead box, of the card, of its box and of the
/// block after it.
fn card_after_leads_tied(
    lead: &[(u32, &str)],
    pad: u32,
    card_css: &str,
    only_css: &str,
) -> Vec<f32> {
    let (mut ids, mut only, mut after) = (Vec::new(), 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        for &(height, css) in lead {
            ids.push(block(
                doc,
                body,
                &format!("display:block;height:{height}px;{css}"),
            ));
        }
        let card = block(
            doc,
            body,
            &format!("display:block;padding-bottom:{pad}px;{card_css}"),
        );
        ids.push(card);
        only = block(doc, card, &format!("display:block;height:30px;{only_css}"));
        after = block(doc, body, "display:block;height:10px");
    });
    ids.push(only);
    ids.push(after);
    ids.iter().map(|&id| page_flow_y(&doc, id)).collect()
}

#[test]
fn a_block_that_holds_one_box_tied_to_the_box_before_it_goes_on_when_that_box_starts_its_page() {
    // The box before it starts the page, so the avoided break cannot keep the
    // two together: the box ends at 90, the padding below it does not fit, and
    // the page breaks before the block, as browsers do.
    for (lead_css, card_css, only_css) in [
        ("break-after:avoid", "", ""),
        ("", "break-before:avoid", ""),
        ("", "", "break-before:avoid"),
    ] {
        assert_eq!(
            card_after_leads_tied(&[(60, lead_css)], 30, card_css, only_css),
            [0.0, 100.0, 100.0, 160.0],
            "{lead_css} {card_css} {only_css}"
        );
    }
}

#[test]
fn a_block_tied_through_a_chain_of_boxes_to_the_box_that_starts_the_page_goes_on() {
    // The chain of avoided breaks reaches back to the first box of the page.
    assert_eq!(
        card_after_leads_tied(
            &[(10, "break-after:avoid"), (50, "break-after:avoid")],
            30,
            "",
            ""
        ),
        [0.0, 10.0, 100.0, 100.0, 160.0]
    );
}

#[test]
fn a_block_tied_to_a_box_that_starts_its_page_after_a_break_goes_on() {
    // The run starts the second page, below a box that fills the first.
    assert_eq!(
        card_after_leads_tied(&[(100, ""), (60, "break-after:avoid")], 30, "", ""),
        [0.0, 100.0, 200.0, 200.0, 260.0]
    );
}

#[test]
fn a_block_tied_to_a_box_below_the_top_of_its_page_stays_with_the_run() {
    // The run of the two boxes does not start the page, so it moves as a
    // whole to the next page, where the block fits after the box.
    assert_eq!(
        card_after_leads_tied(&[(30, ""), (30, "break-after:avoid")], 30, "", ""),
        [0.0, 100.0, 130.0, 130.0, 190.0]
    );
}

#[test]
fn a_run_too_tall_only_because_of_closing_edges_moves_before_its_card_moves_alone() {
    // The content fits the current page, but the card's closing edges do
    // not. The run first moves to the next page; its head then starts that
    // page, so the card can break away and start the following one.
    assert_eq!(
        card_after_leads_tied(&[(10, ""), (50, "break-after:avoid")], 30, "", ""),
        [0.0, 100.0, 200.0, 200.0, 260.0]
    );
}

#[test]
fn the_margin_above_the_first_box_of_the_run_counts_as_part_of_the_page_top() {
    // The margin of the first box of the first page is kept, which leaves the
    // box below the page top but its margin box at it.
    assert_eq!(
        card_after_leads_tied(&[(50, "margin-top:10px;break-after:avoid")], 30, "", ""),
        [10.0, 100.0, 100.0, 160.0]
    );
}

#[test]
fn a_block_tied_to_the_box_before_it_in_a_wrapper_goes_on_with_the_wrapper_edges() {
    // Both boxes are the children of a wrapper with padding of its own that
    // the block ends: the wrapper starts the page and so does the box.
    let (mut lead, mut card, mut only, mut after) = (0, 0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let wrapper = block(doc, body, "display:block;padding-bottom:10px");
        lead = block(doc, wrapper, "display:block;height:60px;break-after:avoid");
        card = block(doc, wrapper, "display:block;padding-bottom:30px");
        only = block(doc, card, "display:block;height:30px");
        after = block(doc, body, "display:block;height:10px");
    });
    assert_eq!(page_flow_y(&doc, lead), 0.0);
    assert_eq!(page_flow_y(&doc, card), 100.0);
    assert_eq!(page_flow_y(&doc, only), 100.0);
    assert_eq!(page_flow_y(&doc, after), 170.0);
}

#[test]
fn a_block_ending_a_wrapper_of_tied_boxes_goes_on_alone_below_the_page_top() {
    // Every box of the wrapper is tied to its first, so the avoided break
    // before the block gives way for the wrapper's own padding, wherever the
    // wrapper starts. The first box is not at the top of the page here, and
    // the block goes on without it. (Browsers move that box on to the next
    // page too, which is not done.)
    let (mut lead, mut card, mut only, mut after) = (0, 0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:20px");
        let wrapper = block(doc, body, "display:block;padding-bottom:10px");
        lead = block(doc, wrapper, "display:block;height:50px;break-after:avoid");
        card = block(doc, wrapper, "display:block;padding-bottom:30px");
        only = block(doc, card, "display:block;height:30px");
        after = block(doc, body, "display:block;height:10px");
    });
    assert_eq!(page_flow_y(&doc, lead), 20.0);
    assert_eq!(page_flow_y(&doc, card), 100.0);
    assert_eq!(page_flow_y(&doc, only), 100.0);
    assert_eq!(page_flow_y(&doc, after), 170.0);
}

#[test]
fn a_block_tied_to_the_box_before_it_stays_when_it_would_gain_nothing() {
    // The edges fit after the box.
    assert_eq!(
        card_after_leads_tied(&[(60, "break-after:avoid")], 10, "", ""),
        [0.0, 60.0, 60.0, 100.0]
    );
    // The box does not fit the page: the page breaks inside it.
    let (mut card, mut only) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px;break-after:avoid");
        card = block(doc, body, "display:block;padding-bottom:30px");
        only = block(doc, card, "display:block;height:50px");
    });
    assert_eq!(page_flow_y(&doc, card), 60.0);
    assert_eq!(page_flow_y(&doc, only), 60.0);
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
    // The padding below the box would run from 90 to 120, over the edge at
    // 100, so the block moves whole to the next page. The break after the box
    // sits after the padding, so the next block opens the page after the one
    // the padding ends on.
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
fn a_block_that_holds_one_box_stays_when_a_float_crosses_the_page_end() {
    // The float runs into the padding below the box or out of the block, past
    // the page end, and has a break to take of its own.
    for (float_height, float_first) in [(70, true), (70, false), (41, true), (11, false)] {
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
fn a_block_that_holds_one_box_moves_when_a_float_reaches_into_its_padding_within_the_page() {
    // The float reaches below the box, into the padding or past it, but ends
    // at the page end at the latest: it goes with the block.
    for (float_height, float_first) in [(40, true), (35, true), (10, false), (5, false)] {
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
            100.0,
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
fn the_last_box_moves_with_the_enclosed_float_and_the_edges_below_it() {
    // The float ends within the page, but the padding after it does not.
    // The last flow box moves with both, to the break before that box.
    let (mut parent, mut middle) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, "display:flow-root;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        middle = block(doc, parent, "display:block;height:50px");
        block(doc, parent, "float:left;width:20px;height:5px");
    });
    assert_eq!(page_flow_y(&doc, middle), 100.0);
    // The float follows the moved box, and the flow root holds its tail.
    assert_eq!(doc.nodes[parent].layout.size.height, 185.0);
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
    // still moves, as a break before it beats one between its last box and the
    // padding; the page it moves to starts with the run, and the last box goes
    // on to the page after with the padding, the avoided break giving way.
    assert_eq!(
        tied_boxes("padding-bottom:60px", &tied),
        (vec![0.0, 100.0, 200.0], 280.0)
    );
}

#[test]
fn the_last_box_of_a_run_at_a_page_top_moves_on_with_the_edges_that_do_not_fit() {
    // The boxes tied to each other and the padding fill the second page
    // exactly with 50px of padding, and do not with one pixel more.
    let tied = [(10, ""), (30, "break-after:avoid"), (20, "")];
    assert_eq!(
        tied_boxes("padding-bottom:50px", &tied),
        (vec![0.0, 100.0, 130.0], 200.0)
    );
    assert_eq!(
        tied_boxes("padding-bottom:51px", &tied),
        (vec![0.0, 100.0, 200.0], 271.0)
    );
    // A border closes the block the same way, and so does an avoided break
    // before the last box instead of one after the box before it.
    for (second, third) in [
        ("break-after:avoid", ""),
        ("", "break-before:avoid"),
        ("", "break-before:avoid-page"),
    ] {
        assert_eq!(
            tied_boxes(
                "border-bottom:51px solid",
                &[(10, ""), (30, second), (20, third)]
            ),
            (vec![0.0, 100.0, 200.0], 271.0),
            "{second} {third}"
        );
    }
}

#[test]
fn the_last_box_of_a_run_that_starts_its_page_in_the_flow_moves_on_with_the_edges() {
    // The first box of the run starts the second page where the box before it
    // ends the first, so the run has nowhere further to go.
    let tied = [(100, ""), (50, "break-after:avoid"), (40, "")];
    assert_eq!(
        tied_boxes("padding-bottom:10px", &tied),
        (vec![0.0, 100.0, 150.0], 200.0)
    );
    assert_eq!(
        tied_boxes("padding-bottom:11px", &tied),
        (vec![0.0, 100.0, 200.0], 251.0)
    );
}

#[test]
fn only_the_last_box_of_a_longer_run_moves_on_with_the_edges() {
    // The other boxes of the run stay together on the page the run moved to.
    assert_eq!(
        tied_boxes(
            "padding-bottom:60px",
            &[
                (10, ""),
                (30, "break-after:avoid"),
                (30, "break-after:avoid"),
                (30, "")
            ]
        ),
        (vec![0.0, 100.0, 130.0, 200.0], 290.0)
    );
}

#[test]
fn the_last_box_of_a_run_stays_when_it_crosses_the_end_of_its_page() {
    // A last box taller than a page breaks inside, wherever it starts.
    let (positions, _) = tied_boxes(
        "padding-bottom:30px",
        &[(10, ""), (30, "break-after:avoid"), (130, "")],
    );
    assert_eq!(positions, vec![0.0, 10.0, 40.0]);
    // So does one that fits a page but starts the second one with the run
    // and runs past its end: moving it would not take it out of the way.
    let (positions, _) = tied_boxes(
        "padding-bottom:30px",
        &[(100, ""), (50, "break-after:avoid"), (60, "")],
    );
    assert_eq!(positions, vec![0.0, 100.0, 150.0]);
}

#[test]
fn the_last_box_of_a_run_stays_where_a_box_cannot_move_on_its_own() {
    // The same boxes as above, 100px, 50px and 40px, and the padding that
    // sends the last one to the third page when nothing around them holds
    // them: a clipped parent of a set height, a flex container and a
    // multicol container keep the page positions the flow gave them.
    for wrapper in [
        "display:block;overflow:hidden;height:300px",
        "display:flex",
        "display:block;columns:1",
    ] {
        let (mut tied, mut last) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            let outer = block(doc, body, wrapper);
            let card = block(doc, outer, "display:block;padding-bottom:15px");
            block(doc, card, "display:block;height:100px");
            tied = block(doc, card, "display:block;height:50px;break-after:avoid");
            last = block(doc, card, "display:block;height:40px");
        });
        assert_eq!(page_flow_y(&doc, tied), 100.0, "{wrapper}");
        assert_eq!(page_flow_y(&doc, last), 150.0, "{wrapper}");
    }
}

#[test]
fn the_last_box_of_a_run_in_nested_blocks_moves_on_with_the_edges_of_both() {
    let (mut tied, mut last) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let outer = block(doc, body, "display:block;padding-bottom:15px");
        let inner = block(doc, outer, "display:block;padding-bottom:15px");
        block(doc, inner, "display:block;height:10px");
        tied = block(doc, inner, "display:block;height:50px;break-after:avoid");
        last = block(doc, inner, "display:block;height:40px");
    });
    // The run of two boxes moves to the second page and ends at 190, where
    // 30px of padding do not fit: the second box goes to the third page.
    assert_eq!(page_flow_y(&doc, tied), 100.0);
    assert_eq!(page_flow_y(&doc, last), 200.0);
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
    // The padding that does not fit under the last box starts the next page.
    // Chromium breaks between the lines of the paragraph for it and moves the
    // last box too, which is not done for a paragraph that avoided breaks tie
    // to the box after it.
    assert_eq!(doc.nodes[parent].layout.size.height, 130.0);
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
    // The padding that does not fit under the last box starts the next page.
    // Chromium breaks between the lines of the paragraph for it and moves the
    // last box too, which is not done for a paragraph that avoided breaks tie
    // to the box after it.
    assert_eq!(doc.nodes[parent].layout.size.height, 130.0);
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
fn a_float_between_the_tied_boxes_goes_with_them() {
    // The float lies in a flow of its own and does not cut the run the boxes
    // make: the second and last box move to the next page together, and the
    // padding of the parent with them.
    let (mut second, mut last) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        second = block(doc, parent, "display:block;height:30px;break-after:avoid");
        block(
            doc,
            parent,
            "display:block;float:left;width:10px;height:10px",
        );
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, second), 100.0);
    assert_eq!(page_flow_y(&doc, last), 130.0);
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
fn a_box_that_takes_no_space_before_the_box_is_a_box_to_break_from() {
    // The empty block is a box in the parent: the page breaks after it, so the
    // last box moves to the next page with the padding, as browsers do. A
    // break before the empty block would not be one between boxes.
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:70px");
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block");
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 100.0);
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
fn a_paragraph_too_short_for_both_constraints_moves_whole_with_the_edges() {
    // Three lines cannot have two orphans and two widows. The break before
    // the paragraph is the one between boxes, so all three lines move with
    // the padding, as Chromium prints it.
    assert_eq!(closed_paragraph(3, ""), (100.0, 170.0));
    // So do lines that one of the two leaves no break for: orphans of three,
    // widows of three, and four lines with three of either.
    for css in ["orphans:3", "orphans:2;widows:3", "orphans:1;widows:3"] {
        assert_eq!(closed_paragraph(3, css), (100.0, 170.0), "{css}");
    }
    for css in ["orphans:3", "widows:3", "orphans:2;widows:3"] {
        assert_eq!(closed_paragraph(4, css), (100.0, 180.0), "{css}");
    }
}

#[test]
fn a_paragraph_whose_own_padding_runs_past_the_page_is_split_between_its_lines() {
    // The lines end at 70, and 40px of padding of the paragraph itself end at
    // 110: its box does not fit the page, so it cannot move whole to a better
    // break. The last line goes to the next page with that padding and the
    // padding of the div, as Chromium prints three lines and 80px of padding
    // of their own on a page of 200px. Before the last line moves, the
    // paragraph spans its lines and the 40px of padding; moving the line adds
    // the 40px between the page end and the top of the next page.
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:40px");
        block(doc, parent, "display:block;height:40px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;\
             padding-bottom:40px",
        );
        doc.append_text(paragraph, "aaa aaa aaa");
    });
    assert_eq!(page_flow_y(&doc, paragraph), 40.0);
    assert_eq!(doc.nodes[paragraph].layout.size.height, 110.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 190.0);
}

#[test]
fn lines_enough_for_both_constraints_are_split_between_them_after_a_box() {
    // Orphans plus widows is as many lines as there are: one break is left, and
    // it keeps its lines apart.
    assert_eq!(closed_paragraph(3, "orphans:2;widows:1"), (40.0, 150.0));
    assert_eq!(closed_paragraph(3, "orphans:1;widows:2"), (40.0, 160.0));
    assert_eq!(closed_paragraph(5, "orphans:3;widows:2"), (20.0, 160.0));
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
    // A paragraph that avoids breaks inside keeps all its lines, so the box
    // before it is the one the page breaks from: it moves whole with the
    // padding, which fit a page together (60 + 40).
    for avoid in ["break-inside:avoid", "break-inside:avoid-page"] {
        assert_eq!(closed_paragraph(6, avoid), (100.0, 200.0), "{avoid}");
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
    // between the last line and the padding, which starts the page after them
    // when it does not fit there either.
    assert_eq!(closed_paragraph_with(6, 90, ""), (10.0, 290.0));
}

#[test]
fn a_paragraph_moved_whole_to_a_page_still_splits_when_the_edges_run_past_its_end() {
    // The lines end at 70 and the padding below them does not fit, so the
    // paragraph moves to the top of the next page: 100 to 120 for two lines.
    // The page break before it is then gone, and 81px of padding after the
    // lines no longer fit either: the last line goes on to the page after
    // with the padding, as Chromium prints it.
    assert_eq!(closed_paragraph_with(2, 80, ""), (100.0, 200.0));
    assert_eq!(closed_paragraph_with(2, 81, ""), (100.0, 291.0));
    assert_eq!(closed_paragraph_with(2, 90, ""), (100.0, 300.0));
    // Three lines cannot have two orphans and two widows, so the last line is
    // what goes: Chromium leaves two lines on the page it moved the paragraph
    // to.
    assert_eq!(closed_paragraph_with(3, 70, ""), (100.0, 200.0));
    assert_eq!(closed_paragraph_with(3, 71, ""), (100.0, 281.0));
    // So do lines that one of the two leaves no break for.
    for css in ["orphans:3", "widows:3", "orphans:1;widows:3"] {
        assert_eq!(closed_paragraph_with(3, 71, css), (100.0, 281.0), "{css}");
    }
    assert_eq!(closed_paragraph_with(4, 60, "orphans:3"), (100.0, 200.0));
    assert_eq!(closed_paragraph_with(4, 61, "orphans:3"), (100.0, 271.0));
    // Orphans that a break cannot keep do not hold the lines together either.
    assert_eq!(closed_paragraph_with(2, 90, "orphans:5"), (100.0, 300.0));
    assert_eq!(closed_paragraph_with(2, 90, "widows:5"), (100.0, 300.0));
}

#[test]
fn the_padding_of_the_paragraph_itself_counts_when_it_splits_after_moving_whole() {
    // 5px of padding or border below the lines of the paragraph are edges that
    // follow its last line like those of the div: 20 + 5 + 75 fill the page
    // and one pixel more does not.
    for css in ["padding-bottom:5px", "border-bottom:5px solid"] {
        assert_eq!(closed_paragraph_with(2, 75, css), (100.0, 200.0), "{css}");
        assert_eq!(closed_paragraph_with(2, 76, css), (100.0, 291.0), "{css}");
        assert_eq!(closed_paragraph_with(3, 65, css), (100.0, 200.0), "{css}");
        assert_eq!(closed_paragraph_with(3, 66, css), (100.0, 281.0), "{css}");
    }
    // The paragraph moves whole when its box fits the page it moves to,
    // whatever the edges are: breaking before it beats breaking in it.
    assert_eq!(
        closed_paragraph_with(3, 30, "padding-bottom:5px"),
        (100.0, 165.0)
    );
}

#[test]
fn a_paragraph_of_one_line_moves_whole_whatever_the_edges_below_it() {
    // A single line has no break to take: it keeps its edges. The 90px of
    // padding end at the page end below it, and 5px more of padding on the
    // paragraph push them to the next page: the paragraph's own 5px still fit
    // under the line, the 90px of the div do not.
    assert_eq!(closed_paragraph_with(1, 90, ""), (100.0, 200.0));
    assert_eq!(
        closed_paragraph_with(1, 90, "padding-bottom:5px"),
        (100.0, 290.0)
    );
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
fn the_lines_after_a_block_move_whole_to_the_next_page_with_the_edges_below_them() {
    // The lines run in two runs around a block, which the page breaks at.
    // The three lines after the block fit the page, but the padding below
    // them does not. Giving up the two widows would leave one line, fewer
    // than the orphans, so the run goes on whole to the next page and the
    // parent grows by the 60px that are left on the first page.
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
    assert_eq!(doc.nodes[parent].layout.size.height, 170.0);
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
    // The rows of a grid are no boxes to break between for the edges of the
    // grid; the items of a flex column are (see
    // `the_last_item_of_a_container_that_stacks_its_items_moves_with_the_edges`).
    let mut last = 0;
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "padding-bottom:30px;display:grid");
        block(doc, parent, "display:block;height:70px");
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
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
fn the_edges_of_the_body_are_closing_edges_like_those_of_any_block() {
    // The 20px box would end 10px short of the padding below it, so it goes
    // on the next page with the padding, as in a block of the body.
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
    assert_eq!(page_flow_y(&doc, last), 100.0);
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
    // still goes to the next page, and the padding starts the page after it
    // when it does not fit below the line, and the block after the card
    // follows the padding.
    for (padding, after_y) in [(95, 295.0), (250, 450.0)] {
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
        // and ends at 110, and the padding starts the page after it at 200.
        assert_eq!(
            doc.nodes[card].layout.size.height,
            150.0 + padding as f32,
            "{padding}px"
        );
        assert_eq!(page_flow_y(&doc, after), after_y, "{padding}px");
    }
}

#[test]
fn a_card_of_two_lines_after_a_box_in_the_card_moves_whole() {
    // With a box before the paragraph in the card, a break between the two is
    // a better one: the paragraph moves whole, and orphans of two keep it so.
    assert_eq!(closed_paragraph(2, ""), (100.0, 160.0));
}

/// `body > [div(20px), outer > [div(30px), wrapper > paragraph], div(10px)]`
/// where the paragraph is two lines of 10px Ahem text, 30px wide, on 100px
/// pages. Returns the height of the paragraph, of the wrapper and the
/// page-flow y of the block after `outer`.
fn wrapper_after_a_box(outer_css: &str, wrapper_css: &str) -> (f32, f32, f32) {
    wrapper_after_a_box_with(outer_css, wrapper_css, "", "aaa aaa")
}

/// [`wrapper_after_a_box`] with `paragraph_css` on the paragraph and `text` in
/// it, which is one line for each word.
fn wrapper_after_a_box_with(
    outer_css: &str,
    wrapper_css: &str,
    paragraph_css: &str,
    text: &str,
) -> (f32, f32, f32) {
    wrapper_after_a_lead(outer_css, "", wrapper_css, paragraph_css, text)
}

/// [`wrapper_after_a_box_with`] with `lead_css` on the box before the wrapper.
fn wrapper_after_a_lead(
    outer_css: &str,
    lead_css: &str,
    wrapper_css: &str,
    paragraph_css: &str,
    text: &str,
) -> (f32, f32, f32) {
    let (mut paragraph, mut wrapper, mut after) = (0, 0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:20px");
        let outer = block(doc, body, &format!("display:block;{outer_css}"));
        block(doc, outer, &format!("display:block;height:30px;{lead_css}"));
        wrapper = block(doc, outer, &format!("display:block;{wrapper_css}"));
        paragraph = block(
            doc,
            wrapper,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, text);
        after = block(doc, body, "display:block;height:10px");
    });
    (
        doc.nodes[paragraph].layout.size.height,
        doc.nodes[wrapper].layout.size.height,
        page_flow_y(&doc, after),
    )
}

#[test]
fn a_wrapper_of_a_paragraph_after_a_box_moves_whole_with_the_edges_that_close_the_card() {
    // The wrapper comes after a box in the card whose padding closes it. The
    // two lines of the paragraph cannot be split for orphans, so the page
    // breaks before the wrapper, which moves to the next page with its own
    // padding and that of the card.
    assert_eq!(
        wrapper_after_a_box("padding-bottom:40px", "padding-bottom:10px"),
        (20.0, 30.0, 170.0)
    );
}

#[test]
fn a_wrapper_of_lines_that_orphans_and_widows_keep_together_moves_whole_with_the_edges() {
    // Three lines cannot have two orphans and two widows, and a break between
    // two lines of fewer than the two together leaves too few at one end. The
    // page breaks before the wrapper in each case, as it does in browsers.
    // The wrapper is 20px of lines and 10px of padding, or 30px of lines and
    // 10px of padding, and the 40px of padding of the card follow it to the
    // next page: the block after the card starts at 100, plus the two.
    for (paragraph_css, text, wrapper, after) in [
        ("", "aaa aaa aaa", 40.0, 180.0),
        ("orphans:1", "aaa aaa", 30.0, 170.0),
        ("widows:1", "aaa aaa", 30.0, 170.0),
    ] {
        assert_eq!(
            wrapper_after_a_box_with(
                "padding-bottom:40px",
                "padding-bottom:10px",
                paragraph_css,
                text
            ),
            (wrapper - 10.0, wrapper, after),
            "{paragraph_css} {text}"
        );
    }
    // The block around the paragraph needs no padding of its own.
    assert_eq!(
        wrapper_after_a_box("padding-bottom:40px", ""),
        (20.0, 20.0, 160.0)
    );
}

#[test]
fn a_wrapper_of_lines_that_can_be_split_keeps_its_place() {
    // Four lines break between the second and third, so the wrapper stays and
    // the last two go to the next page with the padding: the paragraph spans
    // its 40px of lines and the 30px that the page end cuts out.
    let (paragraph, _, _) = wrapper_after_a_box_with(
        "padding-bottom:40px",
        "padding-bottom:10px",
        "",
        "aaa aaa aaa aaa",
    );
    assert_eq!(paragraph, 70.0);
}

#[test]
fn a_wrapper_of_lines_after_a_box_that_ties_it_is_split_between_the_lines() {
    // The box before the wrapper avoids a break after it, so the break between
    // the two is not there to take. The break falls between the lines, though
    // three of them have too few for two orphans and two widows: the last one
    // goes to the next page with the padding, as browsers print it. The
    // paragraph spans its 30px of lines and the 30px that the page end cuts
    // out.
    assert_eq!(
        wrapper_after_a_lead(
            "padding-bottom:40px",
            "break-after:avoid",
            "padding-bottom:10px",
            "",
            "aaa aaa aaa"
        ),
        (60.0, 70.0, 160.0)
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
    card_after_leads(50, &[(0, lead_css)], 2, "")
}

/// `body > [div(above), div(padding-bottom:40px) > [leads..., paragraph]]`
/// where each lead is a div of the height and style given, and the paragraph is
/// `lines` lines of 10px Ahem text, 30px wide, with `paragraph_css`, on 100px
/// pages. No div comes first when `above` is 0. Returns the height and the
/// page-flow y of the paragraph.
fn card_after_leads(
    above: u32,
    leads: &[(u32, &str)],
    lines: usize,
    paragraph_css: &str,
) -> (f32, f32) {
    let mut paragraph = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        if above > 0 {
            block(doc, body, &format!("display:block;height:{above}px"));
        }
        let card = block(doc, body, "display:block;padding-bottom:40px");
        for &(height, css) in leads {
            block(doc, card, &format!("display:block;height:{height}px;{css}"));
        }
        paragraph = block(
            doc,
            card,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
    });
    (
        doc.nodes[paragraph].layout.size.height,
        page_flow_y(&doc, paragraph),
    )
}

#[test]
fn a_paragraph_after_an_empty_block_in_the_card_moves_whole_with_the_edges() {
    // An empty block before the paragraph is a box to break from, as it is in
    // Chromium: a break between the two is a better one than a break between
    // the lines, so the two lines are not split for orphans that they cannot
    // keep, and the padding that does not fit after them goes with them.
    assert_eq!(card_after_a_lead(""), (20.0, 100.0));
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
fn a_paragraph_tied_to_an_empty_block_that_starts_the_card_is_split_between_its_lines() {
    // The empty block avoids a break after it, so the paragraph is tied to the
    // first box of the card and no break between boxes is left to take inside
    // the card. The break falls between the lines, though two lines have too
    // few for two orphans and two widows: the last one goes to the next page
    // with the padding, as Chromium prints it.
    assert_eq!(card_after_a_lead("break-after:avoid"), (60.0, 50.0));
}

#[test]
fn a_paragraph_tied_to_the_boxes_before_it_back_to_the_start_is_split_between_its_lines() {
    // The lines end at 70 on the page and the 40px of padding do not fit. In
    // each card an avoided break ties the paragraph to every box before it, so
    // the break falls between the lines whatever their number: the paragraph
    // spans its lines up to the page end and those that follow, as Chromium
    // prints two lines as one and one, three as two and one, and four as two
    // and two.
    for (lines, above) in [(2, 50), (3, 40), (4, 30)] {
        let split = match lines {
            2 => (60.0, 50.0),
            3 => (70.0, 40.0),
            _ => (90.0, 30.0),
        };
        // An empty block with a break avoided after it or before the next.
        assert_eq!(
            card_after_leads(above, &[(0, "break-after:avoid")], lines, ""),
            split,
            "{lines} lines after an empty block"
        );
        assert_eq!(
            card_after_leads(above, &[(0, "")], lines, "break-before:avoid"),
            split,
            "{lines} lines that avoid a break before them"
        );
        // Two empty blocks, each tied to the next.
        assert_eq!(
            card_after_leads(
                above,
                &[(0, "break-after:avoid"), (0, "break-after:avoid")],
                lines,
                ""
            ),
            split,
            "{lines} lines after two empty blocks"
        );
        // A box with space, with and without a box above the card.
        let height = 70 - 10 * lines as u32;
        assert_eq!(
            card_after_leads(0, &[(height, "break-after:avoid")], lines, ""),
            split,
            "{lines} lines after a box"
        );
        assert_eq!(
            card_after_leads(
                0,
                &[
                    (height - 10, "break-after:avoid"),
                    (10, "break-after:avoid")
                ],
                lines,
                ""
            ),
            split,
            "{lines} lines after two boxes tied to each other"
        );
    }
}

#[test]
fn a_float_between_the_boxes_that_tie_a_paragraph_does_not_untie_it() {
    // The float is no box in the flow of the card: the paragraph is tied to the
    // box before the float, and its lines are split as without the float.
    assert_eq!(
        card_after_leads(
            0,
            &[(50, "break-after:avoid"), (10, "float:left;width:0px")],
            2,
            ""
        ),
        (60.0, 50.0)
    );
}

#[test]
fn a_page_name_change_before_a_tied_paragraph_starts_a_page_for_it() {
    // The paragraph names a page, and the page break that this forces before it
    // is the one that the avoided break gave way to: the paragraph starts the
    // next page whole, with the padding after it.
    assert_eq!(
        card_after_leads(50, &[(0, "break-after:avoid")], 2, "page:foo"),
        (20.0, 100.0)
    );
}

#[test]
fn a_paragraph_tied_to_a_box_that_a_break_can_follow_moves_with_it() {
    // The first box of the card ends where a break is not avoided, so the page
    // breaks there: the box tied to the paragraph and the paragraph move to the
    // next page together, as Chromium prints them, and the lines are not
    // split.
    assert_eq!(
        card_after_leads(0, &[(40, ""), (10, "break-after:avoid")], 2, ""),
        (20.0, 110.0)
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

/// `body > [div(lead), div(padding-bottom:40px) > paragraph]` where the
/// paragraph is `lines` lines of 10px Ahem text, 30px wide, styled with
/// `paragraph_css`. Returns the page-flow y of the div and of the paragraph.
fn paragraph_alone_after(lead: u32, lines: usize, paragraph_css: &str) -> (f32, f32) {
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, &format!("display:block;height:{lead}px"));
        parent = block(doc, body, "display:block;padding-bottom:40px");
        paragraph = block(
            doc,
            parent,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
    });
    (page_flow_y(&doc, parent), page_flow_y(&doc, paragraph))
}

#[test]
fn a_paragraph_that_avoids_breaks_alone_in_its_parent_moves_whole_with_the_edges() {
    // The lines end at 70 and the padding below them does not fit. The
    // paragraph cannot move some of its lines, and no box before it in the div
    // is there to break from: the page breaks before the div.
    for (lead, lines) in [(50, 2), (30, 4), (10, 6)] {
        for avoid in ["break-inside:avoid", "break-inside:avoid-page"] {
            assert_eq!(
                paragraph_alone_after(lead, lines, avoid),
                (100.0, 100.0),
                "{lead} {lines} {avoid}"
            );
        }
    }
}

#[test]
fn a_paragraph_that_avoids_breaks_alone_in_its_parent_stays_when_the_edges_fit() {
    assert_eq!(
        paragraph_alone_after(30, 2, "break-inside:avoid"),
        (30.0, 30.0)
    );
}

#[test]
fn a_block_that_avoids_breaks_alone_in_its_parent_moves_whole_with_the_edges() {
    // The boxes end at 90 and the padding below them does not fit. The block
    // that holds them cannot be broken, so the page breaks before the div, even
    // with a wrapper between the two.
    assert_eq!(
        parent_with(|doc, parent| {
            let avoided = block(doc, parent, "display:block;break-inside:avoid");
            block(doc, avoided, "display:block;height:10px");
            block(doc, avoided, "display:block;height:20px");
        }),
        100.0
    );
    assert_eq!(
        parent_with(|doc, parent| {
            let wrapper = block(doc, parent, "display:block");
            let avoided = block(doc, wrapper, "display:block;break-inside:avoid");
            block(doc, avoided, "display:block;height:10px");
            block(doc, avoided, "display:block;height:20px");
        }),
        100.0
    );
}

#[test]
fn a_block_that_avoids_breaks_stays_with_its_parent_when_the_edges_fit() {
    assert_eq!(
        parent_with(|doc, parent| {
            let avoided = block(doc, parent, "display:block;break-inside:avoid");
            block(doc, avoided, "display:block;height:5px");
            block(doc, avoided, "display:block;height:5px");
        }),
        60.0
    );
}

#[test]
fn a_block_that_avoids_breaks_and_begins_its_parent_takes_the_parent_along() {
    // The avoiding block runs past the page end on its own, and no page breaks
    // between the start of its parent and the block, so the parent starts the
    // next page with it, and the padding below follows.
    let (mut parent, mut avoided) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block;padding-bottom:30px");
        avoided = block(doc, parent, "display:block;break-inside:avoid");
        block(doc, avoided, "display:block;height:30px");
        block(doc, avoided, "display:block;height:40px");
    });
    assert_eq!(
        (page_flow_y(&doc, parent), page_flow_y(&doc, avoided)),
        (100.0, 100.0)
    );
}

#[test]
fn a_block_that_avoids_breaks_leaves_a_parent_that_something_precedes_it_in_place() {
    // A box before it keeps the parent's start on this page: only the
    // avoiding block moves, and the padding below it follows it.
    let (mut parent, mut avoided) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:0");
        avoided = block(doc, parent, "display:block;break-inside:avoid");
        block(doc, avoided, "display:block;height:30px");
        block(doc, avoided, "display:block;height:40px");
    });
    assert_eq!(
        (page_flow_y(&doc, parent), page_flow_y(&doc, avoided)),
        (60.0, 100.0)
    );
}

#[test]
fn a_block_that_avoids_breaks_but_holds_a_forced_break_leaves_its_parent_in_place() {
    // A forced break goes through `break-inside:avoid`: the page breaks inside
    // the block where the break asks it to, whichever page the block starts. The
    // last part of the block opens the next page with the padding below it, so
    // moving the parent whole would only add a page.
    type Shape = fn(&mut Document, usize);
    let shapes: [(&str, Shape); 4] = [
        ("break before the second box", |doc, avoided| {
            block(doc, avoided, "display:block;height:20px");
            block(doc, avoided, "display:block;height:20px;break-before:page");
        }),
        ("break after the first box", |doc, avoided| {
            block(doc, avoided, "display:block;height:20px;break-after:always");
            block(doc, avoided, "display:block;height:20px");
        }),
        ("break deeper down", |doc, avoided| {
            block(doc, avoided, "display:block;height:20px");
            let inner = block(doc, avoided, "display:block");
            block(doc, inner, "display:block;height:20px;break-before:page");
        }),
        ("another page name", |doc, avoided| {
            block(doc, avoided, "display:block;height:20px");
            block(doc, avoided, "display:block;height:20px;page:foo");
        }),
    ];
    for (label, build) in shapes {
        assert_eq!(
            parent_with(|doc, parent| {
                let avoided = block(doc, parent, "display:block;break-inside:avoid");
                build(doc, avoided);
            }),
            60.0,
            "{label}"
        );
    }
}

/// `body > [div(50px), div(padding-bottom:40px) > [lead, paragraph]]` where the
/// paragraph is `lines` lines of 10px Ahem text, 30px wide, and `lead` fills
/// the div before it. The lines end at `50 + 10 * lines`. Returns the page-flow
/// y of the div and of the paragraph.
fn paragraph_after_lead(
    lead: impl FnOnce(&mut Document, usize),
    lines: usize,
    paragraph_css: &str,
) -> (f32, f32) {
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:50px");
        parent = block(
            doc,
            body,
            "display:block;padding-bottom:40px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        lead(doc, parent);
        paragraph = block(
            doc,
            parent,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, ["aaa"; 8][..lines].join(" "));
    });
    (page_flow_y(&doc, parent), page_flow_y(&doc, paragraph))
}

#[test]
fn an_empty_block_before_a_paragraph_is_a_box_to_break_from() {
    // The lines end at 70 and the padding below them does not fit. The empty
    // block is a box in the div, so the page breaks after it and the paragraph
    // moves to the next page with the padding. Browsers do the same.
    let empty = |doc: &mut Document, parent: usize| {
        block(doc, parent, "display:block");
    };
    assert_eq!(paragraph_after_lead(empty, 2, ""), (50.0, 100.0));
    // Four lines end at 90: the last two move with the padding, as they do
    // after a box that takes up space.
    assert_eq!(paragraph_after_lead(empty, 4, ""), (50.0, 50.0));
    // Margins around the empty block do not make it any less a box. The one
    // below it collapses through it and the div, which starts a pixel lower.
    assert_eq!(
        paragraph_after_lead(
            |doc, parent| {
                block(doc, parent, "display:block;margin-bottom:1px");
            },
            2,
            ""
        ),
        (51.0, 100.0)
    );
}

#[test]
fn an_empty_block_tied_to_the_paragraph_by_an_avoided_break_leaves_its_lines_to_break() {
    // The break between the empty block and the paragraph is avoided, so the
    // paragraph is left with the breaks between its own lines, as when it comes
    // first: its last two lines go to the next page with the padding.
    for (lead_css, paragraph_css) in [
        ("break-after:avoid", ""),
        ("", "break-before:avoid"),
        ("break-after:avoid-page", ""),
    ] {
        assert_eq!(
            paragraph_after_lead(
                |doc, parent| {
                    block(doc, parent, &format!("display:block;{lead_css}"));
                },
                4,
                paragraph_css
            ),
            (50.0, 50.0),
            "{lead_css} {paragraph_css}"
        );
    }
}

/// `body > div(padding-bottom:{pad}px) > [div(40px), div(30px), ...]` on 100px
/// pages, where `build` adds a container to the div and returns the boxes to
/// look at. Returns their page-flow y and the height of the div.
fn card_with_edges(
    pad: u32,
    build: impl FnOnce(&mut Document, usize) -> Vec<usize>,
) -> (Vec<f32>, f32) {
    let (mut ids, mut card) = (Vec::new(), 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        card = block(doc, body, &format!("display:block;padding-bottom:{pad}px"));
        block(doc, card, "display:block;height:40px");
        block(doc, card, "display:block;height:30px");
        ids = build(doc, card);
    });
    (
        ids.iter().map(|&id| page_flow_y(&doc, id)).collect(),
        doc.nodes[card].layout.size.height,
    )
}

/// A flex column of `items` blocks of 10px, styled with `css`. Returns the
/// column, then each item.
fn flex_column_of(doc: &mut Document, parent: usize, items: usize, css: &str) -> Vec<usize> {
    let column = block(
        doc,
        parent,
        &format!("display:flex;flex-direction:column;{css}"),
    );
    let mut ids = vec![column];
    for _ in 0..items {
        ids.push(block(doc, column, "display:block;height:10px"));
    }
    ids
}

/// A table with one row of 10px per group in `groups`, styled by the CSS of
/// that group. Returns the table, then each row.
fn table_of_row_groups(doc: &mut Document, parent: usize, groups: &[&str]) -> Vec<usize> {
    let table = block(doc, parent, "display:table;border-spacing:0");
    let mut ids = vec![table];
    for css in groups {
        let group = block(doc, table, &format!("display:table-row-group;{css}"));
        let row = block(doc, group, "display:table-row");
        block(doc, row, "display:table-cell;padding:0;height:10px");
        ids.push(row);
    }
    ids
}

#[test]
fn the_last_item_of_a_container_that_stacks_its_items_moves_with_the_edges() {
    // The items end at 90 and the padding below them does not fit. The page
    // breaks between the items, which beats a break between the last item and
    // the padding: Chromium sends the last item to the next page with it, and
    // the container stays where it is.
    type Shape = fn(&mut Document, usize) -> Vec<usize>;
    let shapes: [(&str, Shape); 13] = [
        ("flex column of two items", |doc, card| {
            let ids = flex_column_of(doc, card, 2, "");
            vec![ids[0], ids[1], ids[2]]
        }),
        ("table of two rows", |doc, card| {
            let ids = table_of_rows(doc, card, false, 2, 10);
            vec![ids[0], ids[1], ids[3]]
        }),
        ("table of two rows in a row group", |doc, card| {
            let ids = table_of_rows(doc, card, true, 2, 10);
            vec![ids[0], ids[2], ids[4]]
        }),
        ("flex column in a wrapper", |doc, card| {
            let wrapper = block(doc, card, "display:block");
            let ids = flex_column_of(doc, wrapper, 2, "");
            vec![wrapper, ids[1], ids[2]]
        }),
        ("flex column of items in a contents box", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            let contents = block(doc, column, "display:contents");
            vec![
                column,
                block(doc, contents, "display:block;height:10px"),
                block(doc, contents, "display:block;height:10px"),
            ]
        }),
        ("flex column with a gap", |doc, card| {
            let ids = flex_column_of(doc, card, 2, "row-gap:5px");
            vec![ids[0], ids[1], ids[2]]
        }),
        (
            "flex column reversed, whose last item comes first in the source",
            |doc, card| {
                let ids = flex_column_of(doc, card, 2, "flex-direction:column-reverse");
                vec![ids[0], ids[2], ids[1]]
            },
        ),
        (
            "table whose last row is followed by a hidden one",
            |doc, card| {
                let ids = table_of_rows(doc, card, false, 2, 10);
                block(doc, ids[0], "display:none");
                vec![ids[0], ids[1], ids[3]]
            },
        ),
        ("table with a caption above its one row", |doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            let caption = block(doc, table, "display:table-caption;height:10px");
            let row = block(doc, table, "display:table-row");
            block(doc, row, "display:table-cell;padding:0;height:10px");
            // The caption stays; the row goes with the padding.
            vec![table, caption, row]
        }),
        ("table of two row groups of one row", |doc, card| {
            let ids = table_of_row_groups(doc, card, &["", ""]);
            vec![ids[0], ids[1], ids[2]]
        }),
        (
            "table whose one row group avoids breaks around it, not between its rows",
            |doc, card| {
                let table = block(doc, card, "display:table;border-spacing:0");
                let group = block(
                    doc,
                    table,
                    "display:table-row-group;break-before:avoid;break-after:avoid",
                );
                let first = block(doc, group, "display:table-row");
                block(doc, first, "display:table-cell;padding:0;height:10px");
                let row = block(doc, group, "display:table-row");
                block(doc, row, "display:table-cell;padding:0;height:10px");
                vec![table, first, row]
            },
        ),
        (
            "table whose last row group avoids breaks but holds one row",
            |doc, card| {
                let ids = table_of_row_groups(doc, card, &["", "break-inside:avoid"]);
                vec![ids[0], ids[1], ids[2]]
            },
        ),
        ("table with a footer group", |doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            let group = block(doc, table, "display:table-footer-group");
            let foot = block(doc, group, "display:table-row");
            block(doc, foot, "display:table-cell;padding:0;height:10px");
            let row = block(doc, table, "display:table-row");
            block(doc, row, "display:table-cell;padding:0;height:10px");
            vec![table, row, foot]
        }),
    ];
    for (label, build) in shapes {
        let (ys, height) = card_with_edges(30, build);
        assert_eq!(ys[0], 70.0, "{label}: the container stays: {ys:?}");
        assert_eq!(ys[1], 70.0, "{label}: the box before it stays: {ys:?}");
        assert_eq!(ys[2], 100.0, "{label}: the last goes: {ys:?}");
        assert_eq!(height, 140.0, "{label}");
    }
}

#[test]
fn the_last_item_of_a_container_stays_when_the_edges_fit_the_page() {
    // The items end at 90: 10px of padding fit below them, 11px do not.
    for (shape, build) in [
        (
            "flex column",
            (|doc: &mut Document, card: usize| flex_column_of(doc, card, 2, "")[1..].to_vec())
                as fn(&mut Document, usize) -> Vec<usize>,
        ),
        ("table", |doc: &mut Document, card: usize| {
            let ids = table_of_rows(doc, card, false, 2, 10);
            vec![ids[1], ids[3]]
        }),
    ] {
        let (fits, _) = card_with_edges(10, build);
        assert_eq!(fits, vec![70.0, 80.0], "{shape}");
        let (does_not_fit, _) = card_with_edges(11, build);
        assert_eq!(does_not_fit, vec![70.0, 100.0], "{shape}");
    }
}

#[test]
fn the_padding_of_the_container_itself_counts_with_that_of_the_blocks_it_ends() {
    // The container's own padding and border follow its last item whatever
    // follows the container: Chromium sends the item to the next page with
    // them. With the padding of the block around it they add up.
    let body_child = |css: &str| {
        let mut ids = Vec::new();
        let (doc, _) = paged_body(|doc, body| {
            let column = flex_column_of(doc, body, 0, css)[0];
            block(doc, column, "display:block;height:70px");
            let last = block(doc, column, "display:block;height:20px");
            block(doc, body, "display:block;height:5px");
            ids = vec![column, last];
        });
        ids.iter()
            .map(|&id| page_flow_y(&doc, id))
            .collect::<Vec<_>>()
    };
    assert_eq!(body_child("padding-bottom:30px"), vec![0.0, 100.0]);
    assert_eq!(body_child("border-bottom:30px solid"), vec![0.0, 100.0]);
    // 10px fit after the item.
    assert_eq!(body_child("padding-bottom:10px"), vec![0.0, 70.0]);
    assert_eq!(body_child("padding-bottom:11px"), vec![0.0, 100.0]);
    // Five of the container's own, and five of the card's.
    let sum = |own: u32, card: u32| {
        let (ys, _) = card_with_edges(card, |doc, parent| {
            flex_column_of(doc, parent, 2, &format!("padding-bottom:{own}px"))[1..].to_vec()
        });
        ys
    };
    assert_eq!(sum(5, 5), vec![70.0, 80.0]);
    assert_eq!(sum(5, 6), vec![70.0, 100.0]);
    assert_eq!(sum(6, 5), vec![70.0, 100.0]);
}

#[test]
fn the_last_item_of_a_container_whose_break_is_not_a_better_one_keeps_its_place() {
    // The page breaks between the items only at a better place than before
    // the edges when nothing keeps the items together or makes the last one
    // something else than a box to move.
    type Shape = fn(&mut Document, usize) -> Vec<usize>;
    let shapes: [(&str, Shape); 15] = [
        ("avoided break before the last item", |doc, card| {
            let ids = flex_column_of(doc, card, 1, "");
            vec![
                ids[0],
                ids[1],
                block(doc, ids[0], "display:block;height:10px;break-before:avoid"),
            ]
        }),
        ("avoided break after the item before", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            let first = block(doc, column, "display:block;height:10px;break-after:avoid");
            vec![
                column,
                first,
                block(doc, column, "display:block;height:10px"),
            ]
        }),
        ("column that wraps", |doc, card| {
            let ids = flex_column_of(doc, card, 2, "flex-wrap:wrap");
            vec![ids[0], ids[1], ids[2]]
        }),
        ("last item with boxes to break between", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            let first = block(doc, column, "display:block;height:10px");
            let last = block(doc, column, "display:block");
            block(doc, last, "display:block;height:5px");
            block(doc, last, "display:block;height:5px");
            vec![column, first, last]
        }),
        ("last item moved by a relative offset", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            let first = block(doc, column, "display:block;height:10px");
            vec![
                column,
                first,
                block(
                    doc,
                    column,
                    "display:block;height:10px;position:relative;top:1px",
                ),
            ]
        }),
        ("last item in a vertical writing mode", |doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            let first = block(doc, column, "display:block;height:10px");
            vec![
                column,
                first,
                block(
                    doc,
                    column,
                    "display:block;height:10px;writing-mode:vertical-rl",
                ),
            ]
        }),
        ("table with a cell that spans rows", |doc, card| {
            let ids = table_of_rows(doc, card, false, 2, 10);
            doc.set_element_attribute(ids[2], "rowspan", "2").unwrap();
            vec![ids[0], ids[1], ids[3]]
        }),
        ("table with text outside its rows", |doc, card| {
            let ids = table_of_rows(doc, card, false, 2, 10);
            doc.append_text(ids[0], "x");
            vec![ids[0], ids[1], ids[3]]
        }),
        ("table row group with a row group inside it", |doc, card| {
            let ids = table_of_rows(doc, card, true, 2, 10);
            block(doc, ids[0] + 1, "display:table-row-group");
            vec![ids[0], ids[2], ids[4]]
        }),
        (
            "table with text outside the rows of its group",
            |doc, card| {
                let ids = table_of_rows(doc, card, true, 2, 10);
                doc.append_text(ids[0] + 1, "x");
                vec![ids[0], ids[2], ids[4]]
            },
        ),
        ("column of a set height", |doc, card| {
            let ids = flex_column_of(doc, card, 2, "height:20px");
            vec![ids[0], ids[1], ids[2]]
        }),
        ("table with a caption below its rows", |doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            let first = block(doc, table, "display:table-row");
            block(doc, first, "display:table-cell;padding:0;height:10px");
            let row = block(doc, table, "display:table-row");
            block(doc, row, "display:table-cell;padding:0;height:10px");
            block(
                doc,
                table,
                "display:table-caption;caption-side:bottom;height:10px",
            );
            vec![table, first, row]
        }),
        ("table row with an avoided break before it", |doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            let first = block(doc, table, "display:table-row");
            block(doc, first, "display:table-cell;padding:0;height:10px");
            let row = block(doc, table, "display:table-row;break-before:avoid");
            block(doc, row, "display:table-cell;padding:0;height:10px");
            vec![table, first, row]
        }),
        (
            "table whose last row group has an avoided break before it",
            |doc, card| {
                let ids = table_of_row_groups(doc, card, &["", "break-before:avoid"]);
                vec![ids[0], ids[1], ids[2]]
            },
        ),
        (
            "table whose row group before the last has an avoided break after it",
            |doc, card| {
                let ids = table_of_row_groups(doc, card, &["break-after:avoid", ""]);
                vec![ids[0], ids[1], ids[2]]
            },
        ),
    ];
    for (label, build) in shapes {
        let (ys, _) = card_with_edges(30, build);
        assert!(ys[2] < 100.0, "{label}: {ys:?}");
    }
}

#[test]
fn a_container_that_avoids_breaks_inside_moves_whole_with_the_edges() {
    // Nothing may break between its items, so the page breaks before the
    // container, and the items go on with it rather than one of them alone.
    let (ys, height) = card_with_edges(30, |doc, card| {
        let ids = flex_column_of(doc, card, 2, "break-inside:avoid");
        vec![ids[0], ids[1], ids[2]]
    });
    assert_eq!(ys, [100.0, 100.0, 110.0]);
    assert_eq!(height, 150.0);
    let (ys, height) = card_with_edges(30, |doc, card| {
        let table = block(
            doc,
            card,
            "display:table;border-spacing:0;break-inside:avoid",
        );
        let first = block(doc, table, "display:table-row");
        block(doc, first, "display:table-cell;padding:0;height:10px");
        let row = block(doc, table, "display:table-row");
        block(doc, row, "display:table-cell;padding:0;height:10px");
        vec![table, first, row]
    });
    assert_eq!(ys, [100.0, 100.0, 110.0]);
    assert_eq!(height, 150.0);
}

#[test]
fn an_avoiding_box_with_a_forced_break_inside_stays_where_the_break_splits_it() {
    // `break-inside` does not stop a forced break, so the box is split there
    // wherever it sits: moving it whole first would only add a page.
    for column in [true, false] {
        let (ys, _) = card_with_edges(30, |doc, card| {
            let display = if column {
                "display:flex;flex-direction:column;"
            } else {
                "display:block;"
            };
            let outer = block(doc, card, &format!("{display}break-inside:avoid"));
            let first = block(doc, outer, "display:block;height:10px");
            let second = block(doc, outer, "display:block;height:10px;break-before:page");
            vec![outer, first, second]
        });
        assert_eq!(ys, [70.0, 70.0, 100.0], "column: {column}");
    }
}

#[test]
fn the_last_item_of_a_container_stays_where_a_box_cannot_move_on_its_own() {
    // The same card as above: a clipped parent of a set height, a flex
    // container and a multicol container keep the page positions the flow gave
    // them.
    for wrapper in [
        "display:block;overflow:hidden;height:300px",
        "display:flex",
        "display:block;columns:1",
    ] {
        let mut last = 0;
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            let outer = block(doc, body, wrapper);
            let card = block(doc, outer, "display:block;padding-bottom:30px");
            block(doc, card, "display:block;height:40px");
            block(doc, card, "display:block;height:30px");
            last = flex_column_of(doc, card, 2, "")[2];
        });
        assert_eq!(page_flow_y(&doc, last), 80.0, "{wrapper}");
    }
}

#[test]
fn the_margin_below_the_last_item_is_no_edge_without_padding_or_border() {
    // Nothing closes the container but a margin below its last item, which is
    // no edge that breaks away from it: the item keeps its place.
    let (ys, _) = card_with_edges(0, |doc, card| {
        let column = block(doc, card, "display:flex;flex-direction:column");
        let first = block(doc, column, "display:block;height:10px");
        vec![
            column,
            first,
            block(doc, column, "display:block;height:10px;margin-bottom:30px"),
        ]
    });
    assert_eq!(ys[1..], [70.0, 80.0]);
}

/// A grid of `rows` rows of 10px in the card of [`card_with_edges`], whose
/// content ends at 70. Returns the grid, then each row.
fn grid_of_rows(doc: &mut Document, card: usize, rows: usize) -> Vec<usize> {
    let grid = block(doc, card, "display:grid;grid-template-columns:50px");
    let mut ids = vec![grid];
    ids.extend((0..rows).map(|_| block(doc, grid, "display:block;height:10px")));
    ids
}

#[test]
fn a_grid_that_ends_at_the_page_end_moves_whole_with_the_edges() {
    // Three rows fill the page to its end and the padding below them does not
    // fit. Chromium sends the whole grid to the next page, as it does one that
    // ends earlier, and does not break before its last row.
    let (ys, height) = card_with_edges(30, |doc, card| grid_of_rows(doc, card, 3));
    assert_eq!(ys, vec![100.0, 100.0, 110.0, 120.0]);
    assert_eq!(height, 160.0);
    let (ys, _) = card_with_edges(30, |doc, card| grid_of_rows(doc, card, 2));
    assert_eq!(ys, vec![100.0, 100.0, 110.0]);
}

#[test]
fn a_grid_that_runs_past_the_page_end_breaks_between_its_rows() {
    // The last of four rows does not fit the page, so the page breaks before
    // it and the padding follows it.
    let (ys, height) = card_with_edges(30, |doc, card| grid_of_rows(doc, card, 4));
    assert_eq!(ys, vec![70.0, 70.0, 80.0, 90.0, 100.0]);
    assert_eq!(height, 140.0);
}

#[test]
fn a_container_alone_in_its_card_moves_whole_with_the_edges() {
    // No box in the card is there to break from before the container: the
    // page breaks before the card, not between the container and the padding
    // nor between the card's items.
    for (label, build) in [
        (
            "flex column of one item",
            (|doc: &mut Document, card: usize| flex_column_of(doc, card, 1, "")[1..].to_vec())
                as fn(&mut Document, usize) -> Vec<usize>,
        ),
        ("table of one row", |doc: &mut Document, card: usize| {
            let ids = table_of_rows(doc, card, false, 1, 10);
            vec![ids[1]]
        }),
    ] {
        let (mut card, mut item) = (0, Vec::new());
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            block(doc, body, "display:block;height:70px");
            card = block(doc, body, "display:block;padding-bottom:30px");
            item = build(doc, card);
        });
        assert_eq!(page_flow_y(&doc, card), 100.0, "{label}");
        assert_eq!(page_flow_y(&doc, item[0]), 100.0, "{label}");
    }
}

/// `body > div(padding-bottom:30px) > [div(40px), div(30px), ...]` on 100px
/// pages, where `build` adds boxes to the card. Returns the page-flow y of
/// the boxes it returns.
fn card_boxes(build: impl FnOnce(&mut Document, usize) -> Vec<usize>) -> Vec<f32> {
    card_with_edges(30, build).0
}

#[test]
fn a_float_after_the_last_box_moves_with_it_when_it_ends_within_the_page() {
    // The last box ends at 90 and the padding below it does not fit; the float
    // after it ends at the page end at the latest, so it goes along.
    for (float_height, moves) in [(10, true), (5, true), (11, false), (30, false)] {
        let ys = card_boxes(|doc, card| {
            let last = block(doc, card, "display:block;height:20px");
            let float = block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            vec![last, float]
        });
        let expected = if moves { [100.0, 120.0] } else { [70.0, 90.0] };
        assert_eq!(ys, expected, "{float_height}");
    }
}

#[test]
fn a_float_after_the_last_box_of_an_inner_block_moves_with_it_unless_it_crosses() {
    // The float is a child of the wrapper the last box sits in, or of the card
    // after the wrapper, whose padding the box ends.
    for (float_in_wrapper, float_height, moves) in [
        (true, 10, true),
        (false, 10, true),
        (true, 11, false),
        (false, 11, false),
    ] {
        let ys = card_boxes(|doc, card| {
            let wrapper = block(doc, card, "display:block");
            let last = block(doc, wrapper, "display:block;height:20px");
            let float = block(
                doc,
                if float_in_wrapper { wrapper } else { card },
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            vec![last, float]
        });
        let expected = if moves { [100.0, 120.0] } else { [70.0, 90.0] };
        assert_eq!(ys, expected, "{float_in_wrapper} {float_height}");
    }
}

#[test]
fn the_margin_below_a_float_counts_in_where_it_ends() {
    // The float and its margin end at the page end at the latest to go along.
    for (float_height, moves) in [(9, true), (10, false)] {
        let ys = card_boxes(|doc, card| {
            let last = block(doc, card, "display:block;height:20px");
            let float = block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px;margin-bottom:1px"),
            );
            vec![last, float]
        });
        let expected = if moves { [100.0, 120.0] } else { [70.0, 90.0] };
        assert_eq!(ys, expected, "{float_height}");
    }
    // The same holds for a float beside the one box of a block.
    for (margin, moves) in [(5, true), (6, false)] {
        assert_eq!(
            parent_with(|doc, parent| {
                block(
                    doc,
                    parent,
                    &format!("float:left;width:10px;height:35px;margin-bottom:{margin}px"),
                );
                block(doc, parent, "display:block;height:30px");
            }),
            if moves { 100.0 } else { 60.0 },
            "{margin}"
        );
    }
}

#[test]
fn a_float_after_the_block_in_its_parent_does_not_keep_the_block_from_moving() {
    // The float belongs to the body, whose edges the block does not end: it is
    // no float of the block, and no edge it crosses stops the block.
    let (mut parent, mut only) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block;padding-bottom:30px");
        only = block(doc, parent, "display:block;height:30px");
        block(doc, body, "float:left;width:10px;height:80px");
    });
    assert_eq!(page_flow_y(&doc, parent), 100.0);
    assert_eq!(page_flow_y(&doc, only), 100.0);
}

#[test]
fn a_float_before_the_last_box_stays_behind_unless_it_crosses_the_page_end() {
    // The float sits at the end of the box before, and the page breaks
    // between it and the last box, which leaves the float behind. One that
    // runs past the page end keeps the box with it.
    for (float_height, moves) in [(20, true), (30, true), (31, false), (60, false)] {
        let ys = card_boxes(|doc, card| {
            let float = block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            let last = block(doc, card, "display:block;height:20px");
            vec![float, last]
        });
        let expected = if moves { [70.0, 100.0] } else { [70.0, 70.0] };
        assert_eq!(ys, expected, "{float_height}");
    }
}

#[test]
fn a_relative_offset_of_the_last_box_does_not_change_how_far_a_float_reaches() {
    // The box moves by its normal-flow position, 70 to 90, whatever its
    // offset: a float that ends at the page end at the latest goes along, a
    // float 1px over keeps it, offset up or down.
    for (top, float_height, moves) in [
        (-7, 30, true),
        (7, 30, true),
        (-7, 31, false),
        (7, 31, false),
    ] {
        let ys = card_boxes(|doc, card| {
            let float = block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            let last = block(
                doc,
                card,
                &format!("display:block;height:20px;position:relative;top:{top}px"),
            );
            vec![float, last]
        });
        let start = if moves { 100.0 } else { 70.0 };
        assert_eq!(ys, [70.0, start + top as f32], "{top} {float_height}");
    }
}

#[test]
fn a_float_after_the_last_box_of_an_offset_block_counts_from_the_normal_flow_position() {
    // The wrapper that holds the last box is offset, the float is not (it is
    // the card's): both ends are measured without the offset.
    for (top, float_height, moves) in [
        (-7, 10, true),
        (7, 10, true),
        (-7, 11, false),
        (7, 11, false),
    ] {
        let ys = card_boxes(|doc, card| {
            let wrapper = block(
                doc,
                card,
                &format!("display:block;position:relative;top:{top}px"),
            );
            let last = block(doc, wrapper, "display:block;height:20px");
            let float = block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            vec![last, float]
        });
        // The float in the card follows the box only to the position the
        // card's own flow gives it, which is not under test here.
        let start = if moves { 100.0 } else { 70.0 };
        assert_eq!(ys[0], start + top as f32, "{top} {float_height}");
    }
}

#[test]
fn a_float_in_an_offset_block_that_holds_the_one_box_counts_from_the_normal_flow_position() {
    // The block that holds the box, and the float beside it, are offset; the
    // block they sit in is not. The box ends at 90 and the float at 100 in the
    // flow, or at 101.
    for (top, float_height, moves) in [
        (-7, 10, true),
        (7, 10, true),
        (-7, 11, false),
        (7, 11, false),
    ] {
        assert_eq!(
            parent_with(|doc, parent| {
                let inner = block(
                    doc,
                    parent,
                    &format!("display:block;position:relative;top:{top}px"),
                );
                block(doc, inner, "display:block;height:30px");
                block(
                    doc,
                    inner,
                    &format!("float:left;width:10px;height:{float_height}px"),
                );
            }),
            if moves { 100.0 } else { 60.0 },
            "{top} {float_height}"
        );
    }
}

#[test]
fn a_float_of_an_offset_card_counts_from_the_normal_flow_position_of_the_box_in_it() {
    // The card and the wrapper that holds its last box are offset by different
    // amounts. The float is the card's, so it carries the card's offset only.
    for (card_top, wrapper_top, float_height, moves) in [
        (5, -7, 10, true),
        (-5, 7, 10, true),
        (5, -7, 11, false),
        (-5, 7, 11, false),
    ] {
        let mut last = 0;
        let (doc, _) = paged_body(|doc, body| {
            let card = block(
                doc,
                body,
                &format!("display:block;padding-bottom:30px;position:relative;top:{card_top}px"),
            );
            block(doc, card, "display:block;height:40px");
            block(doc, card, "display:block;height:30px");
            let wrapper = block(
                doc,
                card,
                &format!("display:block;position:relative;top:{wrapper_top}px"),
            );
            last = block(doc, wrapper, "display:block;height:20px");
            block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
        });
        let start = if moves { 100.0 } else { 70.0 };
        assert_eq!(
            page_flow_y(&doc, last),
            start + (card_top + wrapper_top) as f32,
            "{card_top} {wrapper_top} {float_height}"
        );
    }
}

#[test]
fn a_float_between_the_boxes_of_a_tied_run_moves_with_it() {
    // The float between the second box and the card does not cut the run the
    // boxes make with the card, which moves as a whole.
    let (mut second, mut card, mut only) = (0, 0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:50px");
        second = block(doc, body, "display:block;height:30px;break-after:avoid");
        block(doc, body, "float:left;width:10px;height:5px");
        card = block(doc, body, "display:block;padding-bottom:15px");
        only = block(doc, card, "display:block;height:10px");
    });
    assert_eq!(page_flow_y(&doc, second), 100.0);
    assert_eq!(page_flow_y(&doc, card), 130.0);
    assert_eq!(page_flow_y(&doc, only), 130.0);
}

#[test]
fn a_float_after_a_tied_run_that_ends_the_card_moves_with_the_run_unless_it_crosses() {
    // The run of the two boxes moves with the padding below it; the float
    // after the run counts from where the run ends, not from its first box.
    for (float_height, moves) in [(10, true), (11, false)] {
        let ys = card_boxes(|doc, card| {
            let first = block(doc, card, "display:block;height:10px;break-after:avoid");
            let second = block(doc, card, "display:block;height:10px");
            let float = block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            vec![first, second, float]
        });
        let expected = if moves {
            [100.0, 110.0, 120.0]
        } else {
            [70.0, 80.0, 90.0]
        };
        assert_eq!(ys, expected, "{float_height}");
    }
}

#[test]
fn a_float_after_an_inner_block_of_boxes_counts_for_the_last_box_in_it() {
    // The float is in the card after a wrapper of two boxes; the last of them
    // ends the card and moves with its padding unless the float crosses.
    for (float_height, moves) in [(10, true), (11, false)] {
        let ys = card_boxes(|doc, card| {
            let wrapper = block(doc, card, "display:block");
            block(doc, wrapper, "display:block;height:10px");
            let last = block(doc, wrapper, "display:block;height:10px");
            block(
                doc,
                card,
                &format!("float:left;width:10px;height:{float_height}px"),
            );
            vec![last]
        });
        assert_eq!(ys, [if moves { 100.0 } else { 80.0 }], "{float_height}");
    }
}

/// `body > div(padding-bottom:15px) > [A 30, B 30, middle, C 35]` on 100px
/// pages, where `ties` is the CSS of B and C that avoids the break between
/// them, and `middle` adds what sits between B and C. Returns the page-flow y
/// of A, B and C.
fn chain_with_float(
    (b_css, c_css): (&str, &str),
    middle: impl FnOnce(&mut Document, usize),
) -> [f32; 3] {
    let (mut ids, mut card) = ([0; 3], 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        card = block(doc, body, "display:block;padding-bottom:15px");
        ids[0] = block(doc, card, "display:block;height:30px");
        ids[1] = block(doc, card, &format!("display:block;height:30px;{b_css}"));
        middle(doc, card);
        ids[2] = block(doc, card, &format!("display:block;height:35px;{c_css}"));
    });
    ids.map(|id| page_flow_y(&doc, id))
}

const TIED_AFTER: (&str, &str) = ("break-after:avoid", "");
const TIED_BEFORE: (&str, &str) = ("", "break-before:avoid");

#[test]
fn a_float_between_the_boxes_of_a_tied_chain_does_not_cut_it() {
    // The chain of B and C moves with the padding below it, and the floats
    // with it, however the break between them is avoided. A float that ends at
    // the page end at the latest goes along; one that crosses it keeps the
    // chain where it is.
    for ties in [TIED_AFTER, TIED_BEFORE] {
        for (floats, moves) in [
            (&[10][..], true),
            (&[10, 12][..], true),
            (&[40][..], true),
            (&[41][..], false),
            (&[10, 60][..], false),
        ] {
            let ys = chain_with_float(ties, |doc, card| {
                for height in floats {
                    block(
                        doc,
                        card,
                        &format!("float:left;width:10px;height:{height}px"),
                    );
                }
            });
            let expected = if moves {
                [0.0, 100.0, 130.0]
            } else {
                [0.0, 30.0, 60.0]
            };
            assert_eq!(ys, expected, "{ties:?} {floats:?}");
        }
    }
}

#[test]
fn a_float_of_several_lines_between_the_boxes_of_a_chain_does_not_cut_it() {
    // The float has places to break at inside it, which the boxes of the run
    // do not: it is no box of the run and is not asked to move whole.
    let mut ys = [0; 3];
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let card = block(doc, body, "display:block;padding-bottom:15px");
        ys[0] = block(doc, card, "display:block;height:30px");
        ys[1] = block(doc, card, "display:block;height:30px;break-after:avoid");
        let float = block(
            doc,
            card,
            "float:left;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(float, ["aaa"; 3].join(" "));
        ys[2] = block(doc, card, "display:block;height:35px");
    });
    assert_eq!(ys.map(|id| page_flow_y(&doc, id)), [0.0, 100.0, 130.0]);
}

#[test]
fn a_float_before_a_chain_that_starts_the_block_leaves_it_as_it_was() {
    // The first box is tied to the one after it, and starts the block: the
    // page breaks before the second box, whether a float comes first or not.
    for leading_float in [false, true] {
        let (mut first, mut second) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            let card = block(doc, body, "display:block;padding-bottom:15px");
            if leading_float {
                block(doc, card, "float:left;width:10px;height:10px");
            }
            first = block(doc, card, "display:block;height:50px;break-after:avoid");
            second = block(doc, card, "display:block;height:40px");
        });
        assert_eq!(
            [page_flow_y(&doc, first), page_flow_y(&doc, second)],
            [0.0, 100.0],
            "{leading_float}"
        );
    }
}

#[test]
fn a_float_among_the_boxes_of_a_chain_in_an_inner_block_does_not_cut_it() {
    // The wrapper that holds the chain ends the card: the chain moves with the
    // padding of the card below the wrapper.
    let mut ys = [0; 3];
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let card = block(doc, body, "display:block;padding-bottom:15px");
        let wrapper = block(doc, card, "display:block");
        ys[0] = block(doc, wrapper, "display:block;height:30px");
        ys[1] = block(doc, wrapper, "display:block;height:30px;break-after:avoid");
        block(doc, wrapper, "float:left;width:10px;height:10px");
        ys[2] = block(doc, wrapper, "display:block;height:35px");
    });
    assert_eq!(ys.map(|id| page_flow_y(&doc, id)), [0.0, 100.0, 130.0]);
}

#[test]
fn floats_in_every_gap_of_a_chain_of_three_do_not_cut_it() {
    let mut ys = [0; 4];
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let card = block(doc, body, "display:block;padding-bottom:15px");
        ys[0] = block(doc, card, "display:block;height:20px");
        ys[1] = block(doc, card, "display:block;height:30px;break-after:avoid");
        block(doc, card, "float:left;width:10px;height:5px");
        ys[2] = block(doc, card, "display:block;height:25px;break-after:avoid");
        block(doc, card, "float:left;width:10px;height:5px");
        ys[3] = block(doc, card, "display:block;height:20px");
    });
    assert_eq!(
        ys.map(|id| page_flow_y(&doc, id)),
        [0.0, 100.0, 130.0, 155.0]
    );
}

/// `body > div(padding-bottom:15px) > [A 30, B 30 (break-after:avoid), C]` on
/// 100px pages, where `build` makes C and returns the boxes to look at.
/// Returns the page-flow y of A, B and those boxes.
fn chain_ending_in(build: impl FnOnce(&mut Document, usize) -> Vec<usize>) -> Vec<f32> {
    let (mut ids, mut card) = (Vec::new(), 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        card = block(doc, body, "display:block;padding-bottom:15px");
        ids.push(block(doc, card, "display:block;height:30px"));
        ids.push(block(
            doc,
            card,
            "display:block;height:30px;break-after:avoid",
        ));
        ids.extend(build(doc, card));
    });
    ids.iter().map(|&id| page_flow_y(&doc, id)).collect()
}

#[test]
fn a_chain_that_ends_in_one_row_of_a_flex_grid_or_table_moves_with_the_edges() {
    // A container that holds one row has nowhere to break at: it moves with
    // the boxes it is tied to, and the padding of the parent with them.
    for css in ["display:flex", "display:grid", "display:table"] {
        assert_eq!(
            chain_ending_in(|doc, card| vec![block(doc, card, &format!("height:35px;{css}"))]),
            [0.0, 100.0, 130.0],
            "{css}"
        );
    }
}

#[test]
fn a_chain_that_ends_in_a_grid_of_two_rows_moves_whole_with_the_edges() {
    // A grid moves whole when it ends at the page end, rows or not.
    assert_eq!(
        chain_ending_in(|doc, card| {
            let grid = block(doc, card, "display:grid;grid-template-rows:17px 18px");
            vec![
                grid,
                block(doc, grid, "display:block"),
                block(doc, grid, "display:block"),
            ]
        }),
        [0.0, 100.0, 130.0, 130.0, 147.0]
    );
}

#[test]
fn a_chain_that_ends_in_a_stacked_flex_or_table_breaks_between_its_last_items() {
    // The page breaks between the items or rows, so the chain stays and the
    // last one moves with the padding.
    assert_eq!(
        chain_ending_in(|doc, card| {
            let column = block(doc, card, "display:flex;flex-direction:column");
            vec![
                block(doc, column, "display:block;height:17px"),
                block(doc, column, "display:block;height:18px"),
            ]
        }),
        [0.0, 30.0, 60.0, 100.0]
    );
    assert_eq!(
        chain_ending_in(|doc, card| {
            let table = block(doc, card, "display:table;border-spacing:0");
            [17, 18]
                .map(|height| {
                    let group = block(doc, table, "display:table-row-group");
                    let row = block(doc, group, "display:table-row");
                    block(
                        doc,
                        row,
                        &format!("display:table-cell;padding:0;height:{height}px"),
                    );
                    row
                })
                .to_vec()
        }),
        [0.0, 30.0, 60.0, 100.0]
    );
}

/// `body > div(parent_css) > [div(before px), p(paragraph_css)]` where the
/// paragraph is `lines` lines of 10px Ahem text, 30px wide, on 100px pages.
/// Returns the page-flow y of the paragraph, its height and the height of the
/// div.
fn paragraph_in(
    parent_css: &str,
    before: u32,
    paragraph_css: &str,
    lines: usize,
) -> (f32, f32, f32) {
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        parent = block(doc, body, &format!("display:block;{parent_css}"));
        block(doc, parent, &format!("display:block;height:{before}px"));
        paragraph = block(
            doc,
            parent,
            &format!(
                "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;{paragraph_css}"
            ),
        );
        doc.append_text(paragraph, vec!["aaa"; lines].join(" "));
    });
    assert!(doc.nodes[paragraph].is_ifc_root());
    (
        page_flow_y(&doc, paragraph),
        doc.nodes[paragraph].layout.size.height,
        doc.nodes[parent].layout.size.height,
    )
}

#[test]
fn an_avoiding_paragraph_after_a_box_moves_whole_with_the_edges_that_would_break_away() {
    // The paragraph ends at 80 and the 30px of padding below it run past the
    // page end. Nothing may break inside the paragraph, so the page breaks
    // before it, and the padding is not left alone.
    for avoid in ["break-inside:avoid", "break-inside:avoid-page"] {
        assert_eq!(
            paragraph_in("padding-bottom:30px", 10, avoid, 7),
            (100.0, 70.0, 200.0),
            "{avoid}"
        );
    }
    // The edges of a block between it and the parent count too: 20px of
    // padding below the paragraph still fit the page, the 10px of the parent
    // do not.
    let mut paragraph = 0;
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:10px");
        block(doc, parent, "display:block;height:10px");
        let wrapper = block(doc, parent, "display:block;padding-bottom:20px");
        paragraph = block(
            doc,
            wrapper,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px;\
             break-inside:avoid",
        );
        doc.append_text(paragraph, ["aaa"; 7].join(" "));
    });
    assert_eq!(page_flow_y(&doc, paragraph), 100.0);
    // Without `break-inside` the page breaks between its lines instead.
    assert_eq!(
        paragraph_in("padding-bottom:30px", 10, "", 7),
        (10.0, 110.0, 150.0)
    );
    // Without edges below it, it stays where it is.
    assert_eq!(
        paragraph_in("", 10, "break-inside:avoid", 9),
        (10.0, 90.0, 100.0)
    );
}

#[test]
fn an_avoiding_paragraph_that_cannot_fit_a_page_with_the_edges_gives_up_its_last_lines() {
    // The paragraph and 30px of padding are more than a page together, so no
    // page holds both. The paragraph still goes to the next page, where it
    // fits, and its last two lines go on with the padding: a break between
    // lines beats one between the last line and the padding.
    for lines in [8, 9] {
        assert_eq!(
            paragraph_in("padding-bottom:30px", 10, "break-inside:avoid", lines),
            (100.0, 120.0, 250.0),
            "{lines} lines"
        );
    }
    // Where the paragraph starts its page there is no better break.
    assert_eq!(
        paragraph_in("padding-bottom:30px", 0, "break-inside:avoid", 9),
        (0.0, 120.0, 150.0)
    );
    // The same with the edges of the paragraph itself.
    assert_eq!(
        paragraph_in("", 0, "break-inside:avoid;padding-bottom:30px", 8),
        (0.0, 150.0, 150.0)
    );
}

#[test]
fn an_avoiding_paragraph_with_a_height_of_its_own_moves_whole_with_the_edges() {
    // The edges follow the box, not its lines: its 80px end at 90, and the
    // padding below does not fit. No page break goes between the lines, so
    // the box goes to the next page whole, as Chromium prints it.
    for height in ["height:80px", "min-height:80px"] {
        let (y, box_height, _) = paragraph_in(
            "padding-bottom:30px",
            10,
            &format!("break-inside:avoid;{height}"),
            3,
        );
        assert_eq!((y, box_height), (100.0, 80.0), "{height}");
    }
    // Where box and edges fit a page, nothing moves.
    let (y, box_height, parent_height) = paragraph_in(
        "padding-bottom:30px",
        10,
        "break-inside:avoid;height:60px",
        3,
    );
    assert_eq!((y, box_height, parent_height), (10.0, 60.0, 100.0));
}

#[test]
fn the_edges_of_a_block_in_a_grid_column_leave_the_items_beside_it_where_they_are() {
    // The items of a grid of two columns start the page together. The padding
    // below the 80px box of the first one does not fit under it, but the flow
    // does not advance for it: the second item, with the same top as the first,
    // is not pushed down by it. A grid of one column stacks its items, so the
    // block in its one item still pushes the flow after it.
    for (columns, beside, after) in [("50px 50px", 0.0, 110.0), ("50px", 130.0, 150.0)] {
        let (mut second, mut last) = (0, 0);
        let (doc, _) = paged_body_with_ifc(|doc, body| {
            let grid = block(
                doc,
                body,
                &format!("display:grid;grid-template-columns:{columns};align-items:start"),
            );
            let first = block(doc, grid, "display:block;padding-bottom:30px");
            block(doc, first, "display:block;height:80px");
            second = block(doc, grid, "display:block;height:20px");
            last = block(doc, body, "display:block;height:10px");
        });
        assert_eq!(
            (page_flow_y(&doc, second), page_flow_y(&doc, last)),
            (beside, after),
            "{columns}"
        );
    }
}

#[test]
fn a_final_avoided_row_group_moves_whole_with_closing_edges() {
    let (ys, height) = card_with_edges(30, |doc, card| {
        let table = block(doc, card, "display:table;border-spacing:0");
        let group = block(doc, table, "display:table-row-group;break-inside:avoid");
        let first = block(doc, group, "display:table-row");
        block(doc, first, "display:table-cell;padding:0;height:10px");
        let last = block(doc, group, "display:table-row");
        block(doc, last, "display:table-cell;padding:0;height:10px");
        vec![table, first, last]
    });
    assert_eq!(ys, [100.0, 100.0, 110.0]);
    assert_eq!(height, 150.0);
}

#[test]
fn a_final_avoided_group_can_split_its_tail_after_moving_to_a_shorter_page() {
    for next_step in [15.0, 30.0] {
        let steps = [
            200.0, next_step, next_step, next_step, next_step, next_step, next_step,
        ];
        let ys = paginated_flow_ys(
            |doc, body| {
                let card = block(doc, body, "display:block;padding-bottom:30px");
                block(doc, card, "display:block;height:160px");
                let table = block(doc, card, "display:table;border-spacing:0");
                let group = block(doc, table, "display:table-row-group;break-inside:avoid");
                let first = block(doc, group, "display:table-row");
                block(doc, first, "display:table-cell;padding:0;height:10px");
                let last = block(doc, group, "display:table-row");
                block(doc, last, "display:table-cell;padding:0;height:10px");
                vec![first, last]
            },
            &steps,
        );
        assert_eq!(ys, [200.0, 200.0 + next_step]);
    }
}

#[test]
fn moved_rows_check_their_table_for_spanning_cells_once() {
    use crate::layout::table::row_moves::ROW_MOVE_WORK;

    // Rows of 30px on 100px pages: every row after the first page break
    // moves, and all of them belong to one table.
    ROW_MOVE_WORK.with(|work| work.set((0, 0)));
    let mut ids = Vec::new();
    let (doc, pages) = paged_body(|doc, body| {
        ids = table_of_rows(doc, body, true, 12, 30);
    });
    let (moved_rows, table_checks) = ROW_MOVE_WORK.with(|work| work.get());
    assert!(pages > 3, "{pages} pages");
    assert!(moved_rows > 3, "{moved_rows} moved rows");
    assert_eq!(table_checks, 1);
    // Three rows fit a page, so the twelfth row is the third on the fourth
    // page.
    let last_row = ids[ids.len() - 2];
    assert_eq!(page_flow_y(&doc, last_row), 360.0);
}
