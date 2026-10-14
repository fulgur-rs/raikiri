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
fn only_a_definite_width_counts_for_intrinsic_widths() {
    assert_eq!(block("").intrinsic_width(), None);
    assert_eq!(block("width: 50%").intrinsic_width(), None);
    assert_eq!(
        block("width: 40px; padding: 0 5px; margin: 0 3px 0 auto").intrinsic_width(),
        Some(53.0)
    );
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
