use super::*;
use crate::layout::layout_single_page;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use raikiri_style::property::CalcLengthPercentage;

fn block(css: &str) -> GeneratedBlock {
    let fixture = crate::layout::ifc::test_support::sheet_fixture(
        &format!("div::before {{ content: ''; display: block; {css} }}"),
        "",
        |_, _| {},
    );
    let cv = &fixture.cascade.pseudo[&(
        raikiri_style::StyleNodeId::new(fixture.root as u64),
        raikiri_style::PseudoElem::Before,
    )];
    GeneratedBlock::new(1, 2, cv)
}

#[test]
fn an_end_token_needs_a_free_bit() {
    assert_eq!(end_token_id(4), Some(4 | END_TOKEN_BIT));
    assert_eq!(end_token_id(END_TOKEN_BIT), None);
}

#[test]
fn lengths_resolve_against_a_basis_only_when_there_is_one() {
    let calc = ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
        percent: 50.0,
        px: 5.0,
    });
    assert_eq!(length_or_auto(calc, Some(100.0)), Some(55.0));
    assert_eq!(length_or_auto(calc, None), None);
    let absolute = ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
        percent: 0.0,
        px: 40.0,
    });
    assert_eq!(length_or_auto(absolute, None), Some(40.0));
    assert_eq!(
        length_or_auto(ComputedLengthPercentageOrAuto::Percent(10.0), Some(100.0)),
        Some(10.0)
    );
    assert_eq!(
        length_or_auto(ComputedLengthPercentageOrAuto::Percent(10.0), None),
        None
    );
    assert_eq!(
        length_or_auto(ComputedLengthPercentageOrAuto::Auto, None),
        None
    );
}

#[test]
fn the_used_height_is_clamped_by_min_and_max() {
    assert_eq!(block("").used_height(12.0, 0.0, None), 12.0);
    assert_eq!(block("max-height: 10px").used_height(12.0, 0.0, None), 10.0);
    assert_eq!(
        block("height: 40px; min-height: 50px").used_height(0.0, 0.0, None),
        50.0
    );
    // A border-box size includes the edges, never below zero.
    assert_eq!(
        block("box-sizing: border-box; height: 5px").used_height(0.0, 10.0, None),
        0.0
    );
    assert_eq!(block("height: 50%").used_height(3.0, 0.0, Some(40.0)), 20.0);
    assert_eq!(block("height: 50%").used_height(3.0, 0.0, None), 3.0);
}

fn insets(css: &str, rtl: bool) -> (f32, f32) {
    let block = block(css);
    block.inline_insets(100.0, rtl, &block.edges(100.0))
}

#[test]
fn an_auto_width_is_set_in_by_margins_border_and_padding() {
    assert_eq!(
        insets("margin: 0 4px 0 3px; padding: 0 2px 0 1px", false),
        (4.0, 6.0)
    );
    assert_eq!(insets("margin: 0 auto", false), (0.0, 0.0));
}

#[test]
fn a_definite_width_leaves_its_room_to_the_end_margin() {
    assert_eq!(
        insets(
            "width: 50px; min-width: 60px; max-width: 55px; padding-left: 10px",
            false
        ),
        (10.0, 30.0)
    );
    assert_eq!(
        insets("box-sizing: border-box; width: 50%; padding: 0 5px", false),
        (5.0, 55.0)
    );
    assert_eq!(insets("width: 50px; margin-left: 10px", true), (50.0, 0.0));
}

#[test]
fn auto_margins_share_the_room_a_definite_width_leaves() {
    assert_eq!(insets("width: 40px; margin: 0 auto", false), (30.0, 30.0));
    assert_eq!(
        insets("width: 40px; margin: 0 10px 0 auto", false),
        (50.0, 10.0)
    );
    assert_eq!(
        insets("width: 40px; margin: 0 auto 0 10px", false),
        (10.0, 50.0)
    );
    // Over-constrained, the auto margins are zero and the end margin gives.
    assert_eq!(insets("width: 120px; margin: 0 auto", false), (0.0, -20.0));
}

#[test]
fn a_definite_or_minimum_width_counts_for_intrinsic_widths() {
    assert_eq!(block("").intrinsic_width(), 0.0);
    assert_eq!(block("width: 50%").intrinsic_width(), 0.0);
    assert_eq!(
        block("width: 40px; padding: 0 5px; margin: 0 3px 0 auto").intrinsic_width(),
        53.0
    );
    assert_eq!(
        block("min-width: 60px; padding-left: 10px").intrinsic_width(),
        70.0
    );
    assert_eq!(block("margin-left: -10px").intrinsic_width(), 0.0);
}

#[test]
fn a_clamped_auto_width_is_placed_as_a_definite_one() {
    assert_eq!(
        insets("max-width: 40px; margin: 0 auto", false),
        (30.0, 30.0)
    );
    assert_eq!(insets("max-width: 40px", true), (60.0, 0.0));
    assert_eq!(insets("min-width: 120px", false), (0.0, -20.0));
    assert_eq!(insets("min-width: 10px; padding: 0 5px", false), (5.0, 5.0));
}

#[test]
fn a_negative_end_inset_moves_the_line_ends_out_past_the_line_space() {
    let narrowed = |css: &str, rtl: bool, (inline_start, inline_size)| {
        let space = super::super::flow::LineSpace {
            inline_start,
            inline_size,
        };
        let block = block(css);
        let mut bfc = taffy::BlockFormattingContext::new();
        let ctx = bfc.root_block_context();
        let (mut y, mut pending, mut leading) = (0.0, CollapsibleMarginSet::ZERO, None);
        let mut flow = flow(&mut y, &mut pending, &mut leading, &ctx);
        flow.rtl = rtl;
        let line = open(&block, flow).narrow(space, 100.0);
        (line.inline_start, line.inline_size)
    };
    let all = (0.0, 100.0);
    assert_eq!(
        narrowed("margin: 0 -20px 0 -10px", false, all),
        (0.0, 120.0)
    );
    assert_eq!(
        narrowed("margin: 0 -20px 0 -10px", true, all),
        (-10.0, 110.0)
    );
    assert_eq!(narrowed("margin: 0 15px 0 5px", false, all), (5.0, 80.0));
    // A float at the end or the start keeps its room.
    assert_eq!(narrowed("width: 120px", false, (0.0, 80.0)), (0.0, 80.0));
    assert_eq!(narrowed("width: 120px", true, (20.0, 80.0)), (20.0, 80.0));
}

#[test]
fn logical_clear_sides_are_not_mapped() {
    assert_eq!(block("clear: left").clear, taffy::Clear::Left);
    assert_eq!(block("clear: right").clear, taffy::Clear::Right);
    assert_eq!(block("clear: inline-start").clear, taffy::Clear::None);
}

#[test]
fn a_vertical_paragraph_breaks_its_lines_at_a_block_before_without_its_edges() {
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::before { content: 'X'; display: block; width: 30px; margin: 10px }",
        "writing-mode: vertical-rl; height: 60px; line-height: 10px",
        |doc, root| {
            doc.append_text(root, "AA");
        },
    );
    layout_single_page(
        with_ahem(&mut fixture.doc),
        &fixture.cascade,
        page_box_800x600(),
    )
    .expect("layout");
    let root = &fixture.doc.nodes[fixture.root];
    assert_eq!(root.ifc_lines().expect("lines").len(), 2);
    assert_eq!(root.unrounded_layout.size.width, 20.0);
}

#[test]
fn a_token_is_found_past_a_stale_hint() {
    let base = block("");
    let blocks = [
        GeneratedBlock {
            start: 3,
            end: 4,
            ..base
        },
        GeneratedBlock {
            start: 5,
            end: 6,
            ..base
        },
    ];
    assert!(matches!(token(&blocks, 6, 0), Some((1, Token::End(_)))));
    assert!(matches!(token(&blocks, 3, 0), Some((0, Token::Start(_)))));
    assert!(token(&blocks, 7, 1).is_none());
}

fn flow<'a, 'b>(
    y: &'a mut f32,
    pending: &'a mut CollapsibleMarginSet,
    leading: &'a mut Option<CollapsibleMarginSet>,
    ctx: &'a BlockContext<'b>,
) -> Flow<'a, 'b> {
    Flow {
        y,
        pending,
        leading_block_margin: leading,
        lines: &[],
        ctx,
        width: 100.0,
        top_edge: 0.0,
        definite_height: None,
        at_start: false,
        rtl: false,
    }
}

#[test]
fn a_continued_box_takes_the_rest_of_its_height_in_the_next_fragmentainer() {
    let close_after = |css: &str| {
        let block = block(&format!("padding-top: 10px; {css}"));
        let mut bfc = taffy::BlockFormattingContext::new();
        let ctx = bfc.root_block_context();
        let (mut y, mut pending, mut leading) = (0.0, CollapsibleMarginSet::ZERO, None);
        let open = open(&block, flow(&mut y, &mut pending, &mut leading, &ctx));
        // The box fills two 40px fragmentainers, 70px of content, then its
        // lines take 30px of the next.
        let open = open.continued(40.0).continued(40.0);
        y = 30.0;
        close(&block, open, flow(&mut y, &mut pending, &mut leading, &ctx));
        (y, pending.resolve())
    };
    assert_eq!(
        close_after("height: 140px; padding-bottom: 5px; margin-bottom: 3px"),
        (75.0, 3.0)
    );
    assert_eq!(close_after("max-height: 50px"), (0.0, 0.0));
    assert_eq!(close_after(""), (30.0, 0.0));
}

/// The paragraph `text` styled `root_css`, laid out with the generated
/// blocks of `sheet`: its lines and the boxes the line loop placed.
fn placed(sheet: &str, root_css: &str, text: &str) -> (Vec<shodo::Line>, Vec<PlacedBlock>) {
    let mut fixture =
        crate::layout::ifc::test_support::sheet_fixture(sheet, root_css, |doc, root| {
            doc.append_text(root, text);
        });
    layout_single_page(
        with_ahem(&mut fixture.doc),
        &fixture.cascade,
        page_box_800x600(),
    )
    .expect("layout");
    let lines = fixture.doc.nodes[fixture.root]
        .ifc
        .as_ref()
        .and_then(|root| root.lines.as_ref())
        .expect("lines");
    (lines.lines.to_vec(), lines.placed_blocks.clone())
}

const TEN_PX: &str = "font: 10px/10px Ahem; width: 100px";

#[test]
fn the_line_loop_records_the_border_box_of_a_block_before() {
    let (lines, blocks) = placed(
        "div::before { content: 'B B'; display: block; width: 10px; margin-left: 4px; \
         padding: 5px 1px 3px; border-top: 2px solid }",
        TEN_PX,
        "A",
    );
    assert_eq!(lines.len(), 3);
    let [block] = blocks.as_slice() else {
        panic!("one placed block: {blocks:?}")
    };
    assert_eq!(
        block.border_box,
        BoxRect {
            x: 4.0,
            y: 0.0,
            width: 12.0,
            height: 30.0
        }
    );
    assert_eq!(
        block.content_box,
        BoxRect {
            x: 5.0,
            y: 7.0,
            width: 10.0,
            height: 20.0
        }
    );
    assert_eq!((block.first_line, block.end_line), (0, 2));
}

#[test]
fn an_empty_block_after_is_placed_below_the_lines_and_one_collapsed_through_is_not() {
    let (_, blocks) = placed(
        "div::after { content: ''; display: block; height: 4px }",
        TEN_PX,
        "A",
    );
    let [block] = blocks.as_slice() else {
        panic!("one placed block: {blocks:?}")
    };
    assert_eq!(block.border_box.y, 10.0);
    assert_eq!(block.border_box.height, 4.0);
    assert_eq!((block.first_line, block.end_line), (1, 1));
    let (_, blocks) = placed(
        "div::after { content: ''; display: block; margin: 5px }",
        TEN_PX,
        "A",
    );
    assert!(blocks.is_empty());
}

#[test]
fn a_vertical_paragraph_places_no_block_box() {
    let (_, blocks) = placed(
        "div::before { content: 'X'; display: block; background: red }",
        "writing-mode: vertical-rl; height: 60px; font: 10px/10px Ahem",
        "AA",
    );
    assert!(blocks.is_empty());
}

#[test]
fn a_block_box_is_cut_where_pagination_moves_its_lines() {
    let (lines, blocks) = placed(
        "div::before { content: 'B B'; display: block; width: 10px; \
         padding: 5px 0 3px; border-top: 2px solid }",
        TEN_PX,
        "A",
    );
    let block = blocks[0];
    // Unmoved, the box is one piece with both edges.
    let [whole] = block.pieces(&lines, |_| 0.0)[..] else {
        panic!("one piece")
    };
    assert_eq!(whole.border_box, block.border_box);
    assert_eq!(whole.line, 0);
    assert!(whole.has_block_start_edge && whole.has_block_end_edge);
    // The box's second line, 17px down, goes 7px further down with the
    // rest of the box below it.
    let pieces = block.pieces(&lines, |index| if index >= 1 { 7.0 } else { 0.0 });
    let [top, bottom] = pieces[..] else {
        panic!("two pieces: {pieces:?}")
    };
    assert_eq!((top.border_box.y, top.border_box.height), (0.0, 17.0));
    assert_eq!((top.content_box.y, top.content_box.height), (7.0, 10.0));
    assert_eq!((top.line, top.shift), (0, 0.0));
    assert!(top.has_block_start_edge && !top.has_block_end_edge);
    assert_eq!(
        (bottom.border_box.y, bottom.border_box.height),
        (24.0, 13.0)
    );
    assert_eq!(
        (bottom.content_box.y, bottom.content_box.height),
        (24.0, 10.0)
    );
    assert_eq!((bottom.line, bottom.shift), (1, 7.0));
    assert!(!bottom.has_block_start_edge && bottom.has_block_end_edge);
    assert_eq!(bottom.joined_border_box.y, 7.0);
    assert_eq!(bottom.joined_content_box.y, 14.0);
    // A move of every line also moves the edges above the first one.
    let [moved] = block.pieces(&lines, |_| 5.0)[..] else {
        panic!("one piece")
    };
    assert_eq!(moved.border_box.y, 5.0);
}

#[test]
fn a_box_without_lines_moves_with_the_line_above_it() {
    let (lines, blocks) = placed(
        "div::after { content: ''; display: block; height: 4px }",
        TEN_PX,
        "A",
    );
    let pieces = blocks[0].pieces(&lines, |index| if index >= 1 { 9.0 } else { 3.0 });
    let [piece] = pieces[..] else {
        panic!("one piece: {pieces:?}")
    };
    assert_eq!(
        (piece.border_box.y, piece.line, piece.shift),
        (13.0, 0, 3.0)
    );
    assert!(piece.has_block_start_edge && piece.has_block_end_edge);
}

#[test]
fn the_top_of_a_block_box_moves_with_its_first_line() {
    let (lines, blocks) = placed(
        "div::after { content: 'B'; display: block; padding-top: 5px; border-top: 1px solid }",
        TEN_PX,
        "A",
    );
    let block = blocks[0];
    assert_eq!((block.first_line, block.end_line), (1, 2));
    // Pagination moves the box's line, not the line above it: the top edge
    // and padding go with the line.
    let pieces = block.pieces(&lines, |index| if index >= 1 { 9.0 } else { 0.0 });
    let [piece] = pieces[..] else {
        panic!("one piece: {pieces:?}")
    };
    assert_eq!((piece.border_box.y, piece.border_box.height), (19.0, 16.0));
    assert_eq!((piece.line, piece.shift), (1, 9.0));
    assert!(piece.has_block_start_edge && piece.has_block_end_edge);
}

#[test]
fn a_line_overflowing_below_the_box_does_not_cut_it() {
    let (lines, blocks) = placed(
        "div::before { content: 'B B'; display: block; width: 10px; height: 5px }",
        TEN_PX,
        "A",
    );
    let block = blocks[0];
    assert_eq!(block.border_box.height, 5.0);
    // The box's second line overflows below its 5px border box, so moving
    // it leaves the box whole.
    let pieces = block.pieces(&lines, |index| if index >= 1 { 7.0 } else { 0.0 });
    let [piece] = pieces[..] else {
        panic!("one piece: {pieces:?}")
    };
    assert_eq!((piece.border_box.y, piece.border_box.height), (0.0, 5.0));
    assert!(piece.has_block_start_edge && piece.has_block_end_edge);
}
