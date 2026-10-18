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
    GeneratedBlock::new(
        1,
        2,
        cv,
        (
            shodo::style::LineOptions::default(),
            raikiri_style::ComputedTextIndent::Px(5.0),
        ),
        false,
    )
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
        let mut bfc = crate::taffy_impl::block_flow::context::BlockFormattingContext::new();
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
        content_left: 0.0,
        top_edge: 0.0,
        definite_height: None,
        at_start: false,
        top_margin_escapes: false,
        rtl: false,
    }
}

#[test]
fn a_continued_box_takes_the_rest_of_its_height_in_the_next_fragmentainer() {
    let close_after = |css: &str| {
        let block = block(&format!("padding-top: 10px; {css}"));
        let mut bfc = crate::taffy_impl::block_flow::context::BlockFormattingContext::new();
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

/// The inline start of the first glyph run of each line.
fn line_starts(lines: &[shodo::Line]) -> Vec<f32> {
    lines
        .iter()
        .map(|line| {
            line.fragments()
                .find_map(|fragment| match fragment {
                    shodo::Fragment::GlyphRun(run) => Some(run.inline_start()),
                    _ => None,
                })
                .expect("a glyph run")
        })
        .collect()
}

#[test]
fn the_lines_of_a_block_box_take_its_own_alignment_and_indent() {
    let starts = |before: &str| {
        let (lines, _) = placed(
            &format!("div::before {{ content: 'B B B'; display: block; {before} }}"),
            &format!("{TEN_PX}; text-indent: 7px"),
            "A",
        );
        line_starts(&lines)
    };
    // The box inherits the paragraph's indent, which it may override; the
    // line after the box is not the paragraph's first.
    assert_eq!(starts(""), [7.0, 0.0]);
    assert_eq!(starts("text-indent: 20px"), [20.0, 0.0]);
    // A percentage indent is of the box's own width.
    assert_eq!(starts("text-indent: 10%; margin-left: 20px"), [28.0, 0.0]);
    assert_eq!(starts("text-align: right; text-indent: 0"), [50.0, 0.0]);
    // Only the box's first line is indented, unless the indent hangs.
    assert_eq!(starts("width: 30px; text-indent: 10px"), [10.0, 0.0, 0.0]);
    assert_eq!(
        starts("width: 30px; text-indent: 20px hanging"),
        [0.0, 20.0, 0.0]
    );
    // A box of the opposite direction aligns to its own start and end.
    assert_eq!(starts("direction: rtl; text-indent: 0"), [50.0, 0.0]);
    assert_eq!(
        starts("direction: rtl; text-indent: 0; text-align: end"),
        [0.0, 0.0]
    );
}

#[test]
fn start_and_end_alignments_resolve_to_the_box_sides() {
    use shodo::style::{LineOptions, TextAlign, TextAlignLast};
    let aligned = |text_align, text_align_last, rtl| {
        let mut options = LineOptions {
            text_align,
            text_align_last,
            ..LineOptions::default()
        };
        align_physically(&mut options, rtl);
        (options.text_align, options.text_align_last)
    };
    assert_eq!(
        aligned(TextAlign::Start, TextAlignLast::End, true),
        (TextAlign::Right, TextAlignLast::Left)
    );
    assert_eq!(
        aligned(TextAlign::End, TextAlignLast::Start, false),
        (TextAlign::Right, TextAlignLast::Left)
    );
    // The last line of justified text goes to the box's start.
    assert_eq!(
        aligned(TextAlign::Justify, TextAlignLast::Auto, true),
        (TextAlign::Justify, TextAlignLast::Right)
    );
    assert_eq!(
        aligned(TextAlign::Center, TextAlignLast::Auto, true),
        (TextAlign::Center, TextAlignLast::Auto)
    );
}

#[test]
fn a_box_whose_first_line_moves_on_starts_its_lines_in_the_next_fragmentainer() {
    let block = block("");
    let mut bfc = crate::taffy_impl::block_flow::context::BlockFormattingContext::new();
    let ctx = bfc.root_block_context();
    let (mut y, mut pending, mut leading) = (0.0, CollapsibleMarginSet::ZERO, None);
    let open = open(&block, flow(&mut y, &mut pending, &mut leading, &ctx));
    let first = |open: &OpenBlock, lines| {
        let (options, first) = open.line_options(lines);
        assert_eq!(options.text_indent.length, 5.0);
        first
    };
    assert!(first(&open, 0));
    assert!(!first(&open, 1));
    // No line of the box fits the first fragmentainer: its first line is
    // still to come.
    let open = open.after(0).continued(40.0);
    assert!(first(&open, 0));
    // A line of the box was laid out before the break.
    assert!(!first(&open.after(1).continued(40.0), 0));
}

#[test]
fn a_block_box_split_across_fragmentainers_indents_only_its_first_line() {
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture(
        "div::before { content: 'B B B B'; display: block; text-indent: 5px; width: 20px }",
        "font: 10px/10px Ahem",
        |_, _| {},
    );
    layout_single_page(
        with_ahem(&mut fixture.doc),
        &fixture.cascade,
        page_box_800x600(),
    )
    .expect("layout");
    // Fragmentainers two lines tall: the box's first two lines fill the
    // first, its last two go on in the second.
    let lines = {
        let mut session = crate::layout::context::LayoutSession::new(&mut fixture.doc);
        crate::layout::ifc::boxes::layout_with_boxes_in_fragmentainers(
            &mut session,
            fixture.root,
            crate::layout::ifc::flow::FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0),
            crate::fragment::FragmentationContext {
                available_width: 200.0,
                available_height: Some(20.0),
                column_fill: raikiri_style::property::ColumnFillValue::Auto,
                column_width: 100.0,
                column_count: 2,
                column_gap: 0.0,
                column_index: 0,
                origin_x: 0.0,
                origin_y: 0.0,
                orphans: 1,
                widows: 1,
            },
            false,
        )
    };
    assert_eq!(line_starts(&lines.lines), [5.0, 0.0, 0.0, 0.0]);
}

/// Where each line of a `white-space: pre-wrap` paragraph starts when its
/// fragmentainers are two lines tall: two short lines, then a line of eight
/// glyphs that opens the second fragmentainer beside a 20px float.
fn line_starts_beside_a_float_in_the_second_fragmentainer(text_indent: &str) -> Vec<f32> {
    let style = format!(
        "font: 10px/10px Ahem; white-space: pre-wrap; width: 100px; text-indent: {text_indent}"
    );
    let mut fixture = crate::layout::ifc::test_support::sheet_fixture("", &style, |doc, root| {
        doc.append_text(root, "A\nA\n");
        crate::layout::ifc::test_support::span(doc, root, "float: left; width: 20px; height: 10px");
        doc.append_text(root, "AAAAAAAA");
    });
    layout_single_page(
        with_ahem(&mut fixture.doc),
        &fixture.cascade,
        page_box_800x600(),
    )
    .expect("layout");
    let lines = {
        let mut session = crate::layout::context::LayoutSession::new(&mut fixture.doc);
        crate::layout::ifc::boxes::layout_with_boxes_in_fragmentainers(
            &mut session,
            fixture.root,
            crate::layout::ifc::flow::FlowGeometry::horizontal(100.0, (0.0, 0.0), 0.0),
            crate::fragment::FragmentationContext {
                available_width: 200.0,
                available_height: Some(20.0),
                column_fill: raikiri_style::property::ColumnFillValue::Auto,
                column_width: 100.0,
                column_count: 2,
                column_gap: 0.0,
                column_index: 0,
                origin_x: 0.0,
                origin_y: 0.0,
                orphans: 1,
                widows: 1,
            },
            false,
        )
    };
    line_starts(&lines.lines)
}

#[test]
fn a_line_opening_a_later_fragmentainer_is_not_the_paragraphs_first_line() {
    // Unindented, the 80px line fits beside the float.
    assert_eq!(
        line_starts_beside_a_float_in_the_second_fragmentainer("10px"),
        [10.0, 0.0, 20.0]
    );
}

#[test]
fn a_line_after_a_forced_break_in_an_earlier_fragmentainer_keeps_its_indent() {
    // With its each-line indent the line no longer fits and moves below the
    // float.
    assert_eq!(
        line_starts_beside_a_float_in_the_second_fragmentainer("10px each-line"),
        [10.0, 10.0, 10.0]
    );
}

/// The paragraph `text` styled `root_css`, with floats styled `floats` before
/// it, laid out with the generated blocks of `sheet`: its lines and the boxes
/// the line loop placed.
fn placed_after_floats(
    sheet: &str,
    root_css: &str,
    floats: &[&str],
    text: &str,
) -> (Vec<shodo::Line>, Vec<PlacedBlock>) {
    let mut fixture =
        crate::layout::ifc::test_support::sheet_fixture(sheet, root_css, |doc, root| {
            for float_css in floats {
                crate::layout::ifc::test_support::span(doc, root, float_css);
            }
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

const FLOAT_20X30: &str = "float: left; width: 20px; height: 30px";

/// The block `::after` of `display` styled `css` after a 20x30 left float in
/// a 100px paragraph: the offsets of its line and the border box it was
/// placed in.
fn block_after_float(display: &str, css: &str, root_css: &str) -> (Vec<(f32, f32)>, BoxRect) {
    let (lines, blocks) = placed_after_floats(
        &format!("div::after {{ content: 'XXX'; display: {display}; {css} }}"),
        root_css,
        &[FLOAT_20X30],
        "",
    );
    let [block] = blocks.as_slice() else {
        panic!("one placed block: {blocks:?}")
    };
    let tops = lines.iter().map(|line| line.block_offset());
    (tops.zip(line_starts(&lines)).collect(), block.border_box)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> BoxRect {
    BoxRect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn a_formatting_root_block_goes_beside_a_float_with_its_lines_at_full_width() {
    // The 30px word fits the 40px box, which fits beside the 20px float.
    assert_eq!(
        block_after_float("flow-root", "width: 40px", TEN_PX),
        (vec![(0.0, 20.0)], rect(20.0, 0.0, 40.0, 10.0))
    );
    // An ordinary block of the same size lies under the float, so its line is
    // left with 20px and moves below it.
    assert_eq!(
        block_after_float("block", "width: 40px", TEN_PX),
        (vec![(30.0, 0.0)], rect(0.0, 0.0, 40.0, 40.0))
    );
    // An `auto` width takes the room the float leaves.
    assert_eq!(
        block_after_float("flow-root", "", TEN_PX),
        (vec![(0.0, 20.0)], rect(20.0, 0.0, 80.0, 10.0))
    );
    // A scroll container is one too.
    assert_eq!(
        block_after_float("block", "width: 40px; overflow: hidden", TEN_PX),
        (vec![(0.0, 20.0)], rect(20.0, 0.0, 40.0, 10.0))
    );
}

#[test]
fn a_formatting_root_block_wider_than_the_room_beside_a_float_goes_below_it() {
    let below = (vec![(30.0, 0.0)], rect(0.0, 30.0, 90.0, 10.0));
    assert_eq!(block_after_float("flow-root", "width: 90px", TEN_PX), below);
    // Its top margin does not collapse with the margins above it, and
    // clearance puts it there as well.
    assert_eq!(
        block_after_float("flow-root", "width: 90px; margin-top: 5px", TEN_PX),
        below
    );
    assert_eq!(
        block_after_float("flow-root", "width: 90px; clear: left", TEN_PX),
        below
    );
    assert_eq!(
        block_after_float("flow-root", "width: 40px; clear: left", TEN_PX),
        (vec![(30.0, 0.0)], rect(0.0, 30.0, 40.0, 10.0))
    );
}

#[test]
fn a_formatting_root_block_takes_the_first_gap_among_floats_that_fits_it() {
    // The 60px and 30px floats leave 10px between them for 10px, then 70px.
    let (lines, blocks) = placed_after_floats(
        "div::after { content: 'XXX'; display: flow-root; width: 40px }",
        TEN_PX,
        &[
            "float: left; width: 60px; height: 10px",
            "float: right; width: 30px; height: 20px",
        ],
        "",
    );
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.border_box)
            .collect::<Vec<_>>(),
        [rect(0.0, 10.0, 40.0, 10.0)]
    );
    assert_eq!(line_starts(&lines), [0.0]);
}

#[test]
fn the_margins_of_a_formatting_root_block_beside_a_float_are_placed_by_the_slot() {
    let left = |css: &str| block_after_float("flow-root", css, TEN_PX).1.x;
    // A margin the float already takes up moves nothing.
    assert_eq!(left("width: 40px; margin-left: 5px"), 20.0);
    assert_eq!(left("width: 40px; margin-left: 30px"), 30.0);
    // `auto` margins share the room beside the float.
    assert_eq!(left("width: 40px; margin: 0 auto"), 40.0);
    assert_eq!(left("width: 40px; margin-left: auto"), 60.0);
    assert_eq!(left("width: 40px; margin-right: auto"), 20.0);
    // Padding and border are inside the width the box takes.
    let (lines, border_box) = block_after_float(
        "flow-root",
        "width: 40px; padding: 0 5px; border-left: 2px solid",
        TEN_PX,
    );
    assert_eq!(border_box, rect(20.0, 0.0, 52.0, 10.0));
    assert_eq!(lines, [(0.0, 27.0)]);
    // A narrow box in a right-to-left paragraph is at its end side.
    assert_eq!(
        block_after_float(
            "flow-root",
            "width: 40px",
            &format!("{TEN_PX}; direction: rtl")
        )
        .1,
        rect(60.0, 0.0, 40.0, 10.0)
    );
}

#[test]
fn the_lines_of_a_formatting_root_block_beside_a_float_are_not_moved_below_it() {
    // A word wider than the box overflows it where the box is, instead of
    // moving below the float.
    assert_eq!(
        block_after_float("flow-root", "width: 20px", TEN_PX),
        (vec![(0.0, 20.0)], rect(20.0, 0.0, 20.0, 10.0))
    );
    // Further lines stay in the box once the float ends.
    let (lines, blocks) = placed_after_floats(
        "div::after { content: 'XXX XXX'; display: flow-root; width: 40px }",
        TEN_PX,
        &["float: left; width: 20px; height: 10px"],
        "",
    );
    assert_eq!(line_starts(&lines), [20.0, 20.0]);
    assert_eq!(blocks[0].border_box, rect(20.0, 0.0, 40.0, 20.0));
}
