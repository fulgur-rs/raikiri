use super::*;
use crate::layout::context::LayoutSession;

#[test]
fn all_vertical_modes_use_the_containing_block_inline_percentage_basis() {
    use raikiri_style::property::WritingMode;
    for (mode, expected) in [
        (WritingMode::HorizontalTb, 60.0),
        (WritingMode::VerticalRl, 30.0),
        (WritingMode::VerticalLr, 30.0),
        (WritingMode::SidewaysRl, 30.0),
        (WritingMode::SidewaysLr, 30.0),
    ] {
        let mut tree = Document::new();
        let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
        let node = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
        tree.nodes[parent].used_writing_mode = mode;
        let session = LayoutSession::new(&mut tree);

        assert_eq!(
            ifc_parent_inline_size(
                &session,
                node,
                Size {
                    width: Some(60.0),
                    height: Some(30.0),
                },
            ),
            Some(expected),
            "{mode:?} uses its containing block inline basis"
        );
    }
}

#[test]
fn whitespace_only_text_is_not_a_child_of_a_block_container() {
    // CSS 2.1 9.2.1.1: a whitespace-only run between blocks generates no
    // anonymous block box, so taffy's block algorithm never sees it and it
    // cannot separate the margins of its siblings.
    let mut tree = Document::new();
    let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
    tree.set_display(parent, raikiri_style::property::DisplayValue::Block);
    tree.append_text(parent, "\n  ");
    let first = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
    tree.append_text(parent, "\t\r\u{000c} ");
    let text = tree.append_text(parent, " x ");
    let last = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
    tree.append_text(parent, "\n");
    let parent = NodeId::from(parent);
    let session = LayoutSession::new(&mut tree);

    let children: Vec<usize> = session.child_ids(parent).map(usize::from).collect();
    assert_eq!(children, vec![first, text, last]);
    assert_eq!(session.child_count(parent), 3);
    assert_eq!(usize::from(session.get_child_id(parent, 1)), text);
}

#[test]
fn children_walked_by_index_match_the_child_iterator() {
    // A container with 40 children, three of them inside a `display:
    // contents` span after a whitespace-only text, and a second container
    // with one child.
    let mut tree = Document::new();
    let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
    tree.set_display(parent, raikiri_style::property::DisplayValue::Block);
    let mut expected = Vec::new();
    for _ in 0..37 {
        expected.push(tree.append_element(Some(parent), "div", Style::default(), None::<&str>));
    }
    tree.append_text(parent, " ");
    let contents = tree.append_element(Some(parent), "span", Style::default(), None::<&str>);
    tree.set_display(contents, DisplayValue::Contents);
    for _ in 0..3 {
        expected.push(tree.append_element(Some(contents), "div", Style::default(), None::<&str>));
    }
    let other = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
    let e = tree.append_element(Some(other), "div", Style::default(), None::<&str>);
    let (parent, other) = (NodeId::from(parent), NodeId::from(other));
    let mut session = LayoutSession::new(&mut tree);
    let children: Vec<usize> = session.child_ids(parent).map(usize::from).collect();
    assert_eq!(children, expected);

    let count = session.child_count(parent);
    assert_eq!(count, 40);
    let mut walked = Vec::new();
    for index in 0..count {
        walked.push(usize::from(session.get_child_id(parent, index)));
        // A walk of another container in between leaves this one's list.
        assert_eq!(session.child_count(other), 1);
        assert_eq!(usize::from(session.get_child_id(other, 0)), e);
    }
    assert_eq!(walked, expected);
    // The list goes when the walk ends, so a later change of the children
    // is seen without counting them again.
    assert!(
        session
            .context
            .node_states
            .borrow()
            .values()
            .all(|state| state.taffy_child_walks.is_none())
    );
    session.nodes[expected[0]].set_display(DisplayValue::Contents);
    assert_eq!(usize::from(session.get_child_id(parent, 0)), expected[1]);
    // Indices out of order read the same children.
    session.nodes[expected[0]].set_display(DisplayValue::Block);
    session.child_count(parent);
    for index in [39, 2, 0, 38, 1] {
        assert_eq!(
            usize::from(session.get_child_id(parent, index)),
            expected[index]
        );
    }
}

#[test]
fn a_percentage_max_height_of_an_indefinite_basis_does_not_bound_the_height() {
    // CSS 2.1 10.7: a percentage `max-height` against an indefinite
    // containing block height is treated as `none`.
    let style = Style {
        size: Size {
            width: taffy::Dimension::auto(),
            height: taffy::Dimension::length(100.0),
        },
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::percent(0.5),
        },
        ..Style::default()
    };
    let bounds = ifc_content_height_bounds(&style, None, None);
    assert_eq!(bounds, (0.0, f32::INFINITY));
    assert_eq!(ifc_content_height_bounds(&style, Some(80.0), None).1, 40.0);

    // A calc() bound with a percentage needs the basis as well.
    let mut arena = Vec::new();
    let calc_ptr = push_calc(
        &mut arena,
        raikiri_style::property::CalcLengthPercentage {
            percent: 50.0,
            px: 2.0,
        },
        false,
    );
    let style = Style {
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::calc(calc_ptr),
        },
        ..style
    };
    assert_eq!(
        ifc_content_height_bounds(&style, None, None),
        (0.0, f32::INFINITY)
    );
    assert_eq!(ifc_content_height_bounds(&style, Some(80.0), None).1, 42.0);

    // A calc() bound without a percentage needs no basis.
    let length_ptr = push_calc(
        &mut arena,
        raikiri_style::property::CalcLengthPercentage {
            percent: 0.0,
            px: 30.0,
        },
        false,
    );
    let length = taffy::LengthPercentageAuto::calc(length_ptr);
    let style = Style {
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: length,
        },
        ..style
    };
    assert_eq!(ifc_content_height_bounds(&style, None, None), (0.0, 30.0));
    assert!(ifc_length_only(length).into_raw().is_plain_calc());
    assert!(
        ifc_length_only(taffy::LengthPercentageAuto::calc(calc_ptr))
            .into_raw()
            .tag()
            == taffy::CompactLength::AUTO_TAG
    );
}

#[test]
fn the_fit_content_limit_resolves_against_the_basis_it_needs() {
    let limit =
        |size: taffy::Dimension, basis: Option<f32>| fit_content_limit(size.expand(), basis);
    let mut arena = Vec::new();
    let mut calc = |percent: f32, px: f32| {
        let value = raikiri_style::property::CalcLengthPercentage { percent, px };
        taffy::Dimension::fit_content_calc(push_calc(&mut arena, value, true))
    };

    // A length needs no basis.
    assert_eq!(
        limit(taffy::Dimension::fit_content_px(120.0), None),
        Some(120.0)
    );
    // A percentage leaves no limit while the basis is indefinite.
    assert_eq!(
        limit(taffy::Dimension::fit_content_percent(0.25), None),
        None
    );
    assert_eq!(
        limit(taffy::Dimension::fit_content_percent(0.25), Some(400.0)),
        Some(100.0)
    );
    // So does a calc() with a percentage term, which a calc() of lengths does not.
    let mixed = calc(25.0, -50.0);
    assert_eq!(limit(mixed, None), None);
    assert_eq!(limit(mixed, Some(400.0)), Some(50.0));
    assert_eq!(limit(calc(0.0, 30.0), None), Some(30.0));
    // A limit never goes below zero.
    assert_eq!(limit(mixed, Some(100.0)), Some(0.0));
    // Other sizes have no limit.
    assert_eq!(limit(taffy::Dimension::length(10.0), Some(400.0)), None);
    assert_eq!(limit(taffy::Dimension::auto(), Some(400.0)), None);
}

#[test]
fn a_horizontal_override_keeps_the_vertical_ancestor_fragmentation_guard() {
    use crate::layout::ifc::boxes::IfcBoxKind;
    use crate::layout::test_support::ifc_ahem_fonts;
    for mode in [
        "horizontal-tb",
        "vertical-rl",
        "vertical-lr",
        "sideways-rl",
        "sideways-lr",
    ] {
        let mut doc = Document::new();
        let body = doc.append_element(Some(0), "body", Style::default(), Some("display:block"));
        let ancestor = doc.append_element(
            Some(body),
            "div",
            Style::default(),
            Some(format!("display:block;writing-mode:{mode}")),
        );
        let columns = doc.append_element(Some(ancestor), "div", Style::default(),
            Some("display:block;writing-mode:horizontal-tb;columns:2;column-gap:0;width:200px;height:100px"));
        let flex = doc.append_element(Some(columns), "div", Style::default(), Some("display:flex"));
        let paragraph = doc.append_element(
            Some(flex),
            "div",
            Style::default(),
            Some("display:block;font:10px Ahem"),
        );
        let float = doc.append_element(
            Some(paragraph),
            "div",
            Style::default(),
            Some("display:block;float:left;width:10px;height:10px"),
        );
        doc.append_text(float, "x");
        doc.append_text(paragraph, "text");
        doc.mark_in_document_flags();
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        crate::layout::apply_computed_to_style(&mut doc, &cascade).unwrap();
        let root = doc.nodes[paragraph].ifc.as_ref().unwrap().clone();
        assert!(root.boxes.iter().any(|box_| box_.kind == IfcBoxKind::Float));
        assert_eq!(
            doc.nodes[paragraph].used_writing_mode,
            raikiri_style::property::WritingMode::HorizontalTb
        );
        let context = crate::fragment::FragmentationContext::resolve(
            200.0,
            Some(100.0),
            doc.nodes[columns].multicol.unwrap(),
        )
        .unwrap();
        let mut session = LayoutSession::new(&mut doc);
        session.context.fragmentation_stack.push(context);
        let expected = (mode == "horizontal-tb").then_some(context);
        assert_eq!(
            fragmentation_context_for_ifc(&session, paragraph, &root),
            expected
        );
    }
}

#[test]
fn absolute_auto_width_respects_space_after_its_inset() {
    use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600};
    let mut doc = crate::Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let parent = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;position:relative;width:200px;height:100px"),
    );
    let child = doc.append_element(Some(parent), "div", taffy::Style::default(), Some("display:block;position:absolute;left:120px;font-family:Ahem;font-size:10px;line-height:10px"));
    doc.append_text(child, "aaaa bbbb cccc");
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    // CSS 2.2 10.3.7: min(max(40, 200 - 120), 140) = 80px, with three 10px lines.
    assert_eq!(absolute_rect(&doc, child), (120.0, 0.0, 80.0, 30.0));
}

#[test]
fn grid_abspos_static_alignment_uses_the_content_box_and_parent_percentage_basis() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(Some(html), "body", Style::default(), Some("display:block"));
    let containing_block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative;width:200px;height:200px"),
    );
    let grid = doc.append_element(
        Some(containing_block),
        "div",
        Style::default(),
        Some(
            "display:grid;width:100px;height:100px;border:2px solid;padding:0;padding-left:10%;padding-top:10%;justify-items:center;align-items:center",
        ),
    );
    let child = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some("position:absolute;width:20px;height:20px;margin:auto"),
    );
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &cascade, PageBox::A4).unwrap();

    let grid_layout = doc.nodes[grid].unrounded_layout;
    let child_layout = doc.nodes[child].unrounded_layout;
    let grid_origin = doc.layout_origin_of(grid).unwrap();
    let child_origin = doc.layout_origin_of(child).unwrap();
    assert_eq!(
        (grid_layout.padding.left, grid_layout.padding.top),
        (20.0, 20.0)
    );
    let expected_x = grid_origin.x
        + grid_layout.border.left
        + grid_layout.padding.left
        + (grid_layout.size.width
            - grid_layout.border.left
            - grid_layout.border.right
            - grid_layout.padding.left
            - grid_layout.padding.right
            - child_layout.size.width)
            / 2.0;
    let expected_y = grid_origin.y
        + grid_layout.border.top
        + grid_layout.padding.top
        + (grid_layout.size.height
            - grid_layout.border.top
            - grid_layout.border.bottom
            - grid_layout.padding.top
            - grid_layout.padding.bottom
            - child_layout.size.height)
            / 2.0;
    assert!((child_origin.x - expected_x).abs() < 1e-3);
    assert!((child_origin.y - expected_y).abs() < 1e-3);
}

#[test]
fn vertical_grid_abspos_keeps_positive_inline_space_centered() {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some("display:block;margin:0"),
    );
    let containing_block = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("position:relative;width:200px;height:200px"),
    );
    let grid = doc.append_element(
        Some(containing_block),
        "div",
        Style::default(),
        Some(
            "display:grid;position:relative;writing-mode:vertical-rl;width:100px;height:100px;grid-template-columns:100px;grid-template-rows:100px",
        ),
    );
    let child = doc.append_element(
        Some(grid),
        "div",
        Style::default(),
        Some(
            "position:absolute;grid-column:1;grid-row:1;top:10px;bottom:10px;height:20px;width:10px;margin-top:auto;margin-bottom:auto",
        ),
    );
    doc.mark_in_document_flags();
    let computed = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    crate::layout::layout_single_page(&mut doc, &computed, PageBox::A4).unwrap();

    assert_eq!(
        crate::layout::test_support::absolute_rect(&doc, child).1,
        40.0
    );
}

#[test]
fn grid_alignment_ignores_children_owned_by_another_grid() {
    let mut doc = Document::new();
    let grid = doc.append_element(Some(0), "div", Style::default(), Some("display:grid"));
    let other_parent = doc.append_element(Some(0), "div", Style::default(), Some("display:block"));
    let child = doc.append_element(
        Some(other_parent),
        "div",
        Style::default(),
        Some("position:absolute;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    let computed = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    crate::layout::apply_computed_to_style(&mut doc, &computed).unwrap();
    let session = LayoutSession::new(&mut doc);

    assert!(
        super::auto_grid_abspos_aligned_layout(
            &session,
            grid,
            child,
            Size::ZERO,
            taffy::Rect::ZERO,
            taffy::Rect::ZERO,
            taffy::Point::ZERO,
        )
        .is_none()
    );
}

/// Lays out a `container` that holds a block wrapping a `scroller` (with an
/// 80px-tall child) and a 20px-tall sibling in a 800x600 page, and returns the
/// top of the block and of the sibling.
fn baseline_aligned_tops(scroller_style: &str, container: &str) -> (f32, f32) {
    use crate::layout::test_support::{absolute_rect, page_box_800x600};
    let mut doc = crate::Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let parent = doc.append_element(Some(body), "div", taffy::Style::default(), Some(container));
    let wrapper = doc.append_element(
        Some(parent),
        "div",
        taffy::Style::default(),
        Some("display:block;width:100px"),
    );
    let scroller = doc.append_element(
        Some(wrapper),
        "div",
        taffy::Style::default(),
        Some(scroller_style),
    );
    doc.append_element(
        Some(scroller),
        "div",
        taffy::Style::default(),
        Some("width:20px;height:80px"),
    );
    let sibling = doc.append_element(
        Some(parent),
        "div",
        taffy::Style::default(),
        Some("width:10px;height:20px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    (
        absolute_rect(&doc, wrapper).1,
        absolute_rect(&doc, sibling).1,
    )
}

#[test]
fn a_flex_scroll_container_aligns_by_its_clamped_baseline() {
    // The scroller's content ends at 80px, outside its 20px border box. The
    // baseline the block exports is clamped to 20px, which is where the
    // sibling's synthesized baseline lies, so both items start at the same
    // height.
    let (scroller, sibling) = baseline_aligned_tops(
        "display:flex;overflow:hidden;width:20px;height:20px",
        "display:flex;align-items:baseline",
    );
    assert_eq!(scroller, sibling);
}

#[test]
fn a_flex_box_that_does_not_scroll_aligns_by_its_content_baseline() {
    // Without overflow, the baseline stays at the 80px content edge, so the
    // sibling sinks 60px below the block.
    let (scroller, sibling) = baseline_aligned_tops(
        "display:flex;width:20px;height:20px",
        "display:flex;align-items:baseline",
    );
    assert_eq!(sibling - scroller, 60.0);
}

/// Height of a line holding an inline-block that `fill` populates, in Ahem at
/// 10px with a 10px line height.
fn line_height_around_inline_block(fill: impl FnOnce(&mut crate::Document, usize)) -> f32 {
    use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600};
    let mut doc = crate::Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block;margin:0"),
    );
    let line = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let inline_block = doc.append_element(
        Some(line),
        "span",
        taffy::Style::default(),
        Some("display:inline-block"),
    );
    fill(&mut doc, inline_block);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, line).3
}

/// A block of `css` under `parent`, 40px wide so that its text breaks into
/// two 10px lines.
fn two_lines_in(doc: &mut crate::Document, parent: usize, css: &str) -> usize {
    let boxed = doc.append_element(
        Some(parent),
        "div",
        taffy::Style::default(),
        Some(format!("display:block;width:40px;{css}")),
    );
    doc.append_text(boxed, "aaaa bbbb");
    boxed
}

#[test]
fn an_inline_block_takes_the_clamped_baseline_of_a_scroll_container_inside_it() {
    // The second line lies below the 10px box, so the baseline the box
    // exports is its bottom edge. The inline-block sits on it, above the
    // 2px the strut reaches below the baseline.
    let height = line_height_around_inline_block(|doc, inline_block| {
        two_lines_in(doc, inline_block, "height:10px;overflow:hidden");
    });
    assert_eq!(height, 12.0);
}

#[test]
fn an_inline_block_takes_the_last_line_baseline_of_a_box_that_does_not_scroll() {
    // Without overflow the last line's baseline, 18px down, is the
    // inline-block's, so the line grows to hold it.
    let height = line_height_around_inline_block(|doc, inline_block| {
        two_lines_in(doc, inline_block, "height:10px");
    });
    assert_eq!(height, 20.0);
}

#[test]
fn a_scroll_container_inside_another_is_clamped_to_both() {
    // The inner box is 30px tall, so alone it would keep the 18px baseline;
    // the outer one is 10px tall and clamps it further.
    let height = line_height_around_inline_block(|doc, inline_block| {
        let outer = doc.append_element(
            Some(inline_block),
            "div",
            taffy::Style::default(),
            Some("display:block;height:10px;overflow:hidden"),
        );
        two_lines_in(doc, outer, "height:30px;overflow:hidden");
    });
    assert_eq!(height, 12.0);
}

#[test]
fn a_scroll_container_scrolled_out_of_another_is_clamped_to_the_outer_edge() {
    // The inner box lies entirely below the 10px outer one, so its own
    // clamp keeps a baseline the outer box then pulls up to its bottom edge.
    let height = line_height_around_inline_block(|doc, inline_block| {
        let outer = doc.append_element(
            Some(inline_block),
            "div",
            taffy::Style::default(),
            Some("display:block;height:10px;overflow:hidden"),
        );
        doc.append_element(
            Some(outer),
            "div",
            taffy::Style::default(),
            Some("display:block;height:20px"),
        );
        two_lines_in(doc, outer, "height:10px;overflow:hidden");
    });
    assert_eq!(height, 12.0);
}

#[test]
fn a_grid_scroll_container_aligns_by_its_clamped_baseline() {
    let (scroller, sibling) = baseline_aligned_tops(
        "display:flex;overflow:hidden;width:20px;height:20px",
        "display:grid;grid-auto-flow:column;align-items:baseline",
    );
    assert_eq!(scroller, sibling);
}
