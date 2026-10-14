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
        let open = open.continued(&[], 40.0).continued(&[], 40.0);
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
