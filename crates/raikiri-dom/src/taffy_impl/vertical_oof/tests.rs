use crate::Document;
use crate::LayoutStyle as Style;
use crate::layout::layout_single_page;
use crate::layout::test_support::style_css;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};
use taffy::{AvailableSpace, LayoutInput, Line, NodeId, RequestedAxis, RunMode, Size, SizingMode};

/// Rects of a column of two Ahem paragraphs in a positioned container under
/// a root with `root_css`. `column_css` styles the column.
fn column_rects(root_css: &str, column_css: &str) -> Vec<(f32, f32, f32, f32)> {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Some(format!("display:block;{root_css}").as_str()),
    );
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;position:relative"),
    );
    let column = doc.append_element(
        Some(container),
        "div",
        Some(format!("display:block;{column_css}").as_str()),
    );
    let mut paragraphs = Vec::new();
    for text in ["aa bb", "cc dd"] {
        let paragraph = doc.append_element(Some(column), "div", Some("display:block"));
        doc.append_text(paragraph, text);
        paragraphs.push(paragraph);
    }
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    std::iter::once(column)
        .chain(paragraphs)
        .map(|node| absolute_rect(&doc, node))
        .collect()
}

#[test]
fn vertical_rl_absolute_box_shrinks_to_its_containing_block_inline_size() {
    let in_flow = column_rects("writing-mode:vertical-rl", "");
    let absolute = column_rects("writing-mode:vertical-rl", "position:absolute");
    // In flow, the box fills the 600px inline size of the root. Out of
    // flow, it shrinks to fit: each paragraph keeps "aa bb" on one 50px
    // line instead of breaking at min-content, and the box sits on the
    // block-start (right) edge.
    assert_eq!(in_flow[0], (780.0, 0.0, 20.0, 600.0));
    assert_eq!(
        absolute,
        [
            (780.0, 0.0, 20.0, 50.0),
            (790.0, 0.0, 10.0, 50.0),
            (780.0, 0.0, 10.0, 50.0)
        ]
    );
}

#[test]
fn vertical_lr_absolute_box_starts_at_the_left_edge() {
    let in_flow = column_rects("writing-mode:vertical-lr", "");
    let absolute = column_rects("writing-mode:vertical-lr", "position:absolute");
    assert_eq!(in_flow[0], (0.0, 0.0, 20.0, 600.0));
    assert_eq!(
        absolute,
        [
            (0.0, 0.0, 20.0, 50.0),
            (0.0, 0.0, 10.0, 50.0),
            (10.0, 0.0, 10.0, 50.0)
        ]
    );
}

#[test]
fn vertical_absolute_box_keeps_authored_physical_offsets() {
    let rects = column_rects(
        "writing-mode:vertical-rl",
        "position:absolute;left:10px;top:30px",
    );
    // The containing block holds no in-flow content, so it is a zero-wide
    // box on the right edge of the 800px root.
    assert_eq!(rects[0], (810.0, 30.0, 20.0, 50.0));
}

#[test]
fn horizontal_absolute_box_is_unchanged() {
    let rects = column_rects("", "position:absolute");
    assert_eq!(rects[0], (0.0, 0.0, 50.0, 20.0));
}

#[test]
fn vertical_absolute_box_without_inline_content_starts_at_the_block_start_edge() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some("display:block;writing-mode:vertical-rl;width:30px;height:35px;position:relative"),
    );
    let absolute = doc.append_element(
        Some(parent),
        "div",
        Some("display:block;position:absolute;width:15px;height:20px"),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    // The static position is on the block-start edge, the right one.
    assert_eq!(absolute_rect(&doc, absolute), (15.0, 0.0, 15.0, 20.0));
}

#[test]
fn sideways_absolute_text_uses_the_vertical_block_start_edge() {
    // `sideways-lr` starts its inline axis at the bottom.
    for (mode, expected) in [("sideways-rl", (15.0, 0.0)), ("sideways-lr", (0.0, 15.0))] {
        let mut doc = Document::new();
        let body = doc.append_element(Some(0), "body", Some("display:block;margin:0"));
        let parent = doc.append_element(Some(body), "div", Some(format!("display:block;position:relative;writing-mode:{mode};width:30px;height:35px;font:10px Ahem")));
        let child = doc.append_element(
            Some(parent),
            "div",
            Some("display:block;position:absolute;width:15px;height:20px"),
        );
        doc.append_text(child, "x");
        doc.mark_in_document_flags();
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        assert_eq!(
            absolute_rect(&doc, child),
            (expected.0, expected.1, 15.0, 20.0)
        );
    }
}

#[test]
fn only_an_absolute_child_of_the_vertical_containing_block_gets_its_inline_size() {
    let mut doc = Document::new();
    let container = doc.append_element(Some(0), "div", None::<&str>);
    let absolute = doc.append_element(
        Some(container),
        "div",
        Some(style_css(&Style {
            position: taffy::Position::Absolute,
            ..Style::default()
        })),
    );
    let in_flow = doc.append_element(Some(container), "div", None::<&str>);
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let mut session = crate::layout::context::LayoutSession::bridged(&mut doc, &session_cascade);
    session
        .context
        .vertical_oof_containing_blocks
        .push((container, 120.0));
    let inputs = taffy::LayoutInput::HIDDEN;
    let resolved = super::absolute_child_inputs(&session, absolute, inputs);
    assert_eq!(resolved.parent_size.height, Some(120.0));
    assert_eq!(
        resolved.available_space.height,
        taffy::AvailableSpace::Definite(120.0)
    );
    let unchanged = super::absolute_child_inputs(&session, in_flow, inputs);
    assert_eq!(unchanged.parent_size.height, inputs.parent_size.height);
}

#[test]
fn automatic_vertical_containing_block_measurement_restores_its_stack_entry() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let container = doc.append_element(
        Some(html),
        "div",
        Some("display:block;position:relative;writing-mode:vertical-rl"),
    );
    let absolute = doc.append_element(
        Some(container),
        "div",
        Some("display:block;position:absolute;writing-mode:vertical-rl;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    doc.append_text(absolute, "aa bb cc dd ee ff gg hh ii jj kk ll");
    doc.mark_in_document_flags();
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    let mut session = crate::layout::context::LayoutSession::bridged(&mut doc, &cascade);
    // Model the vertical block as a descendant of a multicol container.
    session.nodes[html].multicol = Some(crate::fragment::MulticolStyle {
        rtl: false,
        count: Some(2),
        width: None,
        column_fill: raikiri_style::property::ColumnFillValue::Balance,
        gap: 0.0,
        gap_percent: None,
        height_definite: true,
        horizontal: true,
        orphans: 2,
        widows: 2,
    });
    assert_eq!(
        session.nodes[container].formatting_context,
        crate::node::FormattingContext::Block
    );
    assert_eq!(
        session.nodes[absolute].formatting_context,
        crate::node::FormattingContext::Inline
    );
    assert_eq!(
        session.nodes[absolute].ifc.as_ref().unwrap().writing_mode,
        shodo::geometry::WritingMode::VerticalRl
    );
    let inputs = LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::ContentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size {
            width: Some(100.0),
            height: None,
        },
        known_dimensions_are_definite: Size {
            width: true,
            height: false,
        },
        parent_size: Size {
            width: Some(300.0),
            height: Some(200.0),
        },
        available_space: Size {
            width: AvailableSpace::Definite(300.0),
            height: AvailableSpace::Definite(200.0),
        },
        vertical_margins_are_collapsible: Line::FALSE,
    };
    assert_eq!(
        super::containing_block_inline_size(&session, container, &inputs),
        Some(200.0)
    );
    session.compute_child_layout_with_block_ctx(NodeId::from(container), inputs, None);

    assert_eq!(session.nodes[absolute].unrounded_layout.size.height, 200.0);
    assert!(session.context.vertical_oof_containing_blocks.is_empty());
}

/// A document laid out with a 50px square absolutely positioned child styled
/// with `child_css`, in a 100px square positioned container styled with
/// `container_css`, at the origin of the page. Returns the document, the
/// container and the child.
fn aligned_document(container_css: &str, child_css: &str) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", Some("display:block;margin:0"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(format!(
            "display:block;position:relative;width:100px;height:100px;{container_css}"
        )),
    );
    let child = doc.append_element(
        Some(container),
        "div",
        Some(format!(
            "display:block;position:absolute;width:50px;height:50px;{child_css}"
        )),
    );
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    (doc, container, child)
}

/// The page rect of the child of [`aligned_document`].
fn aligned_rect(container_css: &str, child_css: &str) -> (f32, f32, f32, f32) {
    let (doc, _, child) = aligned_document(container_css, child_css);
    absolute_rect(&doc, child)
}

#[test]
fn align_self_places_the_box_along_the_block_axis_of_vertical_rl() {
    // The block axis runs from right to left, so `start` is the right edge.
    for (alignment, x) in [
        ("start", 50.0),
        ("flex-start", 50.0),
        ("end", 0.0),
        ("flex-end", 0.0),
        ("center", 25.0),
        ("normal", 50.0),
    ] {
        let rect = aligned_rect(
            "writing-mode:vertical-rl",
            &format!("left:0;right:0;align-self:{alignment}"),
        );
        assert_eq!((rect.0, rect.2), (x, 50.0), "align-self:{alignment}");
    }
}

#[test]
fn align_self_places_the_box_along_the_block_axis_of_vertical_lr() {
    for (alignment, x) in [
        ("start", 0.0),
        ("end", 50.0),
        ("center", 25.0),
        ("normal", 0.0),
    ] {
        let rect = aligned_rect(
            "writing-mode:vertical-lr",
            &format!("left:0;right:0;align-self:{alignment}"),
        );
        assert_eq!(rect.0, x, "align-self:{alignment}");
    }
}

#[test]
fn justify_self_places_the_box_along_the_inline_axis() {
    for (alignment, y) in [("start", 0.0), ("end", 50.0), ("center", 25.0)] {
        let rect = aligned_rect(
            "writing-mode:vertical-rl",
            &format!("top:0;bottom:0;justify-self:{alignment}"),
        );
        assert_eq!((rect.1, rect.3), (y, 50.0), "justify-self:{alignment}");
    }
    // `align-self` belongs to the other axis.
    let rect = aligned_rect(
        "writing-mode:vertical-rl",
        "top:0;bottom:0;justify-self:start;align-self:end",
    );
    assert_eq!(rect.1, 0.0);
}

#[test]
fn each_axis_takes_its_own_property() {
    let rect = aligned_rect(
        "writing-mode:vertical-rl",
        "left:0;right:0;top:0;bottom:0;justify-self:start;align-self:end",
    );
    assert_eq!((rect.0, rect.1), (0.0, 0.0));
    let rect = aligned_rect(
        "writing-mode:vertical-rl",
        "left:0;right:0;top:0;bottom:0;justify-self:end;align-self:start",
    );
    assert_eq!((rect.0, rect.1), (50.0, 50.0));
}

#[test]
fn the_inline_axis_starts_at_the_bottom_for_sideways_lr_and_rtl() {
    for (container_css, y) in [
        ("writing-mode:sideways-lr", 50.0),
        ("writing-mode:sideways-lr;direction:rtl", 0.0),
        ("writing-mode:vertical-rl;direction:rtl", 50.0),
        ("writing-mode:vertical-lr", 0.0),
    ] {
        let rect = aligned_rect(container_css, "top:0;bottom:0;justify-self:start");
        assert_eq!(rect.1, y, "{container_css}");
    }
}

#[test]
fn a_sized_box_with_normal_alignment_sits_at_the_block_start_edge() {
    let rect = |container_css: &str| aligned_rect(container_css, "left:0;right:0");
    assert_eq!(rect("writing-mode:vertical-rl").0, 50.0);
    assert_eq!(rect("writing-mode:vertical-lr").0, 0.0);
    // `horizontal-tb` is left to Taffy, whose inline axis starts at the left.
    assert_eq!(rect("").0, 0.0);
}

#[test]
fn over_constrained_insets_drop_the_block_end_inset() {
    let child_css = "left:60px;right:60px;width:20px";
    // The right inset is the block-start one in `vertical-rl`.
    assert_eq!(aligned_rect("writing-mode:vertical-rl", child_css).0, 20.0);
    assert_eq!(aligned_rect("writing-mode:vertical-lr", child_css).0, 60.0);
}

#[test]
fn last_baseline_falls_back_to_the_self_end_edge_of_the_box() {
    // An absolutely positioned box takes no part in baseline alignment: `last
    // baseline` falls back to `safe self-end`. A box in the writing mode of
    // its containing block ends at the left edge of `vertical-rl`.
    let rect = |child_css: &str| aligned_rect("writing-mode:vertical-rl", child_css).0;
    assert_eq!(rect("left:0;right:0;align-self:last baseline"), 0.0);
    // A horizontal box ends at its right edge, the block start of the
    // containing block.
    assert_eq!(
        rect("writing-mode:horizontal-tb;left:0;right:0;align-self:last baseline"),
        50.0
    );
}

#[test]
fn baseline_falls_back_to_safe_start_when_the_box_overflows_the_inset_area() {
    // The box fits the containing block but not the inset-modified one. `start`
    // keeps the default overflow alignment, which pulls the box back inside the
    // containing block; the `safe start` fallback of `baseline` stays at the
    // inset.
    let rect = |container_css: &str, child_css: &str| aligned_rect(container_css, child_css).0;
    assert_eq!(
        rect(
            "writing-mode:vertical-lr",
            "left:70px;right:0;align-self:start"
        ),
        50.0
    );
    assert_eq!(
        rect(
            "writing-mode:vertical-lr",
            "left:70px;right:0;align-self:baseline"
        ),
        70.0
    );
    assert_eq!(
        rect(
            "writing-mode:horizontal-tb",
            "left:70px;right:0;justify-self:start"
        ),
        50.0
    );
    assert_eq!(
        rect(
            "writing-mode:horizontal-tb",
            "left:70px;right:0;justify-self:baseline"
        ),
        70.0
    );
}

#[test]
fn auto_margins_take_the_free_space_before_alignment_does() {
    let rect = aligned_rect(
        "writing-mode:vertical-rl",
        "left:0;right:0;margin-left:auto;margin-right:auto;align-self:start",
    );
    assert_eq!(rect.0, 25.0);
}

#[test]
fn an_axis_with_an_auto_inset_keeps_the_static_position() {
    let rect = aligned_rect("writing-mode:vertical-rl", "left:0;align-self:end");
    assert_eq!(rect.0, 0.0);
    let rect = aligned_rect("writing-mode:vertical-rl", "right:0;align-self:start");
    assert_eq!(rect.0, 50.0);
}

#[test]
fn an_overflowing_box_keeps_the_block_start_edge() {
    let rect = |container_css: &str, alignment: &str| {
        aligned_rect(
            container_css,
            &format!("left:0;right:0;width:150px;align-self:{alignment}"),
        )
        .0
    };
    let vertical_rl = "writing-mode:vertical-rl";
    let vertical_lr = "writing-mode:vertical-lr";
    // The 150px box does not fit the 100px containing block: it hangs over
    // the block-end edge.
    assert_eq!(rect(vertical_rl, "end"), -50.0);
    assert_eq!(rect(vertical_lr, "end"), 0.0);
    // `last baseline` falls back to `safe self-end`.
    assert_eq!(rect(vertical_rl, "last baseline"), -50.0);
    assert_eq!(rect(vertical_lr, "last baseline"), 0.0);
    // A scroll container may scroll in the direction of the overflow.
    assert_eq!(
        rect("writing-mode:vertical-rl;overflow:hidden", "end"),
        -50.0
    );
    assert_eq!(rect("writing-mode:vertical-lr;overflow:hidden", "end"), 0.0);
}

#[test]
fn the_overflow_of_a_moved_box_is_that_of_its_new_position() {
    // The 150px box does not fit the 100px container, so it hangs over the
    // block-end edge: the left edge of `vertical-rl`, where Taffy aligned the
    // box to the left edge and let it overflow to the right.
    let overflow = |container_css: &str, child_css: &str| {
        let (doc, container, _) = aligned_document(container_css, child_css);
        let rect = doc.nodes[container]
            .unrounded_layout
            .scrollable_overflow_rect;
        (rect.left, rect.right, rect.top, rect.bottom)
    };
    let wide = "left:0;right:0;width:150px;align-self:end";
    assert_eq!(
        overflow("writing-mode:vertical-rl", wide),
        (-50.0, 100.0, 0.0, 50.0)
    );
    // In `vertical-lr` the box stays where Taffy put it.
    assert_eq!(
        overflow("writing-mode:vertical-lr", wide),
        (0.0, 150.0, 0.0, 50.0)
    );
    // The inline axis of a `direction: rtl` box ends at the top.
    assert_eq!(
        overflow(
            "writing-mode:vertical-lr;direction:rtl",
            "top:0;bottom:0;height:150px;justify-self:end"
        ),
        (0.0, 100.0, -50.0, 100.0)
    );
}

#[test]
fn the_overflow_of_a_box_moved_in_a_container_with_lines_is_that_of_its_new_position() {
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", Some("display:block;margin:0"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;position:relative;writing-mode:vertical-rl;width:100px;height:100px;font:10px Ahem"),
    );
    doc.append_text(container, "x");
    let child = doc.append_element(
        Some(container),
        "div",
        Some(
            "display:block;position:absolute;width:150px;height:50px;left:0;right:0;align-self:end",
        ),
    );
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
    assert_eq!(absolute_rect(&doc, child).0, -50.0);
    let rect = doc.nodes[container]
        .unrounded_layout
        .scrollable_overflow_rect;
    assert_eq!((rect.left, rect.right), (-50.0, 100.0));
}

#[test]
fn a_box_wider_than_its_inset_modified_block_stays_inside_the_containing_block() {
    let rect = |alignment: &str| {
        aligned_rect(
            "writing-mode:vertical-lr",
            &format!("left:25px;right:25px;width:60px;align-self:{alignment}"),
        )
        .0
    };
    // The inset-modified block is 50px wide, the box 60px.
    assert_eq!(rect("start"), 25.0);
    assert_eq!(rect("end"), 15.0);
    assert_eq!(rect("center"), 20.0);
}

#[test]
fn a_box_hoisted_from_a_descendant_is_aligned_in_its_containing_block() {
    let mut doc = Document::new();
    let body = doc.append_element(Some(0), "body", Some("display:block;margin:0"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some("display:block;position:relative;width:100px;height:100px;writing-mode:vertical-rl"),
    );
    let block = doc.append_element(
        Some(container),
        "div",
        Some("display:block;width:75px;height:50px"),
    );
    let child = doc.append_element(
        Some(block),
        "div",
        Some("display:block;position:absolute;width:50px;height:50px;left:0;right:0;align-self:start"),
    );
    doc.mark_in_document_flags();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, child).0, 50.0);
}

#[test]
fn percentage_insets_resolve_against_the_positioning_area() {
    let rect = |child_css: &str| aligned_rect("writing-mode:vertical-rl", child_css);
    // 10% of the 100px area leaves 10px on either side.
    assert_eq!(rect("left:10%;right:10%;width:20px;align-self:end").0, 10.0);
    assert_eq!(
        rect("left:10%;right:10%;width:20px;align-self:start").0,
        70.0
    );
    assert_eq!(
        rect("top:10%;bottom:10%;height:20px;justify-self:end").1,
        70.0
    );
}

#[test]
fn the_lines_of_a_vertical_container_do_not_change_how_its_boxes_are_aligned() {
    for (mode, expected) in [("vertical-rl", (50.0, 50.0)), ("vertical-lr", (0.0, 50.0))] {
        let mut doc = Document::new();
        let body = doc.append_element(Some(0), "body", Some("display:block;margin:0"));
        let container = doc.append_element(
            Some(body),
            "div",
            Some(format!(
                "display:block;position:relative;writing-mode:{mode};width:100px;height:100px;font:10px Ahem"
            )),
        );
        doc.append_text(container, "x");
        let child = doc.append_element(
            Some(container),
            "div",
            Some("display:block;position:absolute;width:50px;height:50px;left:0;right:0;top:0;bottom:0;align-self:start;justify-self:end"),
        );
        doc.mark_in_document_flags();
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).unwrap();
        let rect = absolute_rect(&doc, child);
        // `align-self` places the box along the block axis, `justify-self` along
        // the inline axis, which ends at the bottom.
        assert_eq!((rect.0, rect.1), expected, "{mode}");
    }
}
