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
fn a_last_box_that_the_next_page_cannot_hold_with_the_edges_stays() {
    // 20px of box and 90px of padding make 110px, more than a page: moving
    // gains nothing.
    assert_eq!(closed_parent("padding-bottom:90px"), (70.0, 180.0, 180.0));
}

#[test]
fn a_box_with_no_box_before_it_stays_with_its_parent() {
    let (mut only, mut parent) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        block(doc, body, "display:block;height:60px");
        parent = block(doc, body, "display:block;padding-bottom:30px");
        only = block(doc, parent, "display:block;height:30px");
    });
    assert_eq!(page_flow_y(&doc, only), 60.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 60.0);
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

#[test]
fn an_avoided_break_before_the_last_box_keeps_it_with_the_box_before() {
    for css in ["break-before:avoid", "break-before:avoid-page"] {
        let (mut parent, mut last) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            parent = block(doc, body, "display:block;padding-bottom:30px");
            block(doc, parent, "display:block;height:70px");
            last = block(doc, parent, &format!("display:block;height:20px;{css}"));
        });
        assert_eq!(page_flow_y(&doc, last), 70.0, "{css}");
        assert_eq!(doc.nodes[parent].layout.size.height, 120.0, "{css}");
    }
    // So does an avoided break after the box before it.
    let (mut parent, mut last) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:70px;break-after:avoid");
        last = block(doc, parent, "display:block;height:20px");
    });
    assert_eq!(page_flow_y(&doc, last), 70.0);
    assert_eq!(doc.nodes[parent].layout.size.height, 120.0);
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
fn a_wrapper_that_has_something_to_break_at_stays() {
    for (css, inner_css) in [
        // Padding or a border separates the wrapper from its box.
        ("padding-top:5px", ""),
        ("border-bottom:1px solid", ""),
        // A float beside the box is a flow of its own.
        ("", "float:left;width:10px;height:5px"),
        // A wrapper that is not a block container of its own.
        ("display:flex;flex-direction:column", ""),
    ] {
        let (mut wrapper, mut parent) = (0, 0);
        let (doc, _) = paged_body(|doc, body| {
            parent = block(doc, body, "display:block;padding-bottom:30px");
            block(doc, parent, "display:block;height:40px");
            block(doc, parent, "display:block;height:30px");
            wrapper = block(doc, parent, &format!("display:block;{css}"));
            if !inner_css.is_empty() {
                block(doc, wrapper, inner_css);
            }
            block(doc, wrapper, "display:block;height:20px");
        });
        assert_eq!(page_flow_y(&doc, wrapper), 70.0, "{css} {inner_css}");
    }
    // A box that is not a block container.
    let (mut wrapper, mut grid) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        let parent = block(doc, body, "display:block;padding-bottom:30px");
        block(doc, parent, "display:block;height:40px");
        block(doc, parent, "display:block;height:30px");
        wrapper = block(doc, parent, "display:block");
        grid = block(doc, wrapper, "display:grid;height:20px");
    });
    assert_eq!(page_flow_y(&doc, wrapper), 70.0);
    assert_eq!(page_flow_y(&doc, grid), 70.0);
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
    for lines in [3, 2, 1] {
        assert_eq!(
            closed_paragraph(lines, ""),
            (100.0, 140.0 + 10.0 * lines as f32),
            "{lines} lines"
        );
    }
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
}

#[test]
fn the_padding_of_the_paragraph_itself_moves_with_its_last_lines() {
    // The paragraph has 20px of padding below its lines and the div 40px more:
    // the lines end at 70, and 60px of edges do not fit after them.
    assert_eq!(closed_paragraph(6, "padding-bottom:20px"), (10.0, 180.0));
}

#[test]
fn lines_that_the_next_page_cannot_hold_with_the_edges_stay() {
    // The last two lines and 90px of padding make 110px, more than a page.
    assert_eq!(closed_paragraph_with(6, 90, ""), (10.0, 160.0));
}

#[test]
fn a_paragraph_alone_in_its_parent_keeps_its_last_lines() {
    // Nothing before the paragraph in the div is a box to break from.
    let (mut parent, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        block(doc, body, "display:block;height:10px");
        parent = block(doc, body, "display:block;padding-bottom:40px");
        paragraph = block(
            doc,
            parent,
            "display:block;width:30px;font-family:Ahem;font-size:10px;line-height:10px",
        );
        doc.append_text(paragraph, ["aaa"; 6].join(" "));
    });
    assert_eq!(doc.nodes[parent].layout.size.height, 100.0);
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
    // or are moved with its edges.
    for css in ["float:left;width:20px", "position:sticky;top:0"] {
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
