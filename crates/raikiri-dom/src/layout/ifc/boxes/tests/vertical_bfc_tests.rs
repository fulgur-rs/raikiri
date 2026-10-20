use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::ifc::geometry::IfcAxes;
use crate::layout::layout_single_page;
use crate::layout::test_support::{
    absolute_rect, ahem_paragraph_with, ahem_paragraph_with_atomic, ifc_ahem_fonts,
    page_box_800x600, with_ahem,
};
use crate::taffy_impl::block_flow::context::BlockFormattingContext;
use shodo::geometry::{Direction, PhysicalSize, WritingMode};
use taffy::{Clear, CollapsibleMarginSet, FloatDirection, Size, Style};

type BoxRect = (f32, f32, f32, f32);

fn layout_vertical_mixed_ifc(writing_mode: &str, clear: &str) -> (BoxRect, BoxRect, BoxRect) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Style::default(),
        Some(
            format!(
                "display:block;margin:0;writing-mode:{writing_mode};width:100px;height:40px;\
                 font-family:Ahem;font-size:10px;line-height:10px"
            )
            .as_str(),
        ),
    );
    let root = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:flow-root;width:100px;height:40px"),
    );
    let float = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some("display:block;float:left;width:10px;height:20px"),
    );
    let wrapper = doc.append_element(
        Some(root),
        "div",
        Style::default(),
        Some("display:block;width:100px;height:40px"),
    );
    doc.append_text(wrapper, "a");
    let cleared = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some(format!("display:block;clear:{clear};width:20px;height:20px").as_str()),
    );
    doc.append_text(wrapper, "b");
    let orthogonal = doc.append_element(
        Some(wrapper),
        "div",
        Style::default(),
        Some("display:block;writing-mode:horizontal-tb;width:10px;height:10px"),
    );

    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");

    (
        absolute_rect(&doc, float),
        absolute_rect(&doc, cleared),
        absolute_rect(&doc, orthogonal),
    )
}

fn vertical_block_child_fixture() -> (Document, usize, usize, usize) {
    let mut children = Vec::new();
    let (mut doc, cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:40px",
        |doc, root| {
            children.push(doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;width:0;height:0;margin-right:30px;margin-left:20px"),
            ));
            children.push(doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;clear:both;width:20px;height:20px;margin-right:5px"),
            ));
            let nested = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;width:40px;height:40px"),
            );
            doc.append_element(
                Some(nested),
                "div",
                Style::default(),
                Some("display:block;width:10px;height:10px;margin-right:30px"),
            );
            doc.append_text(nested, "x");
            children.push(nested);
            doc.append_text(root, "x");
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (doc, children[0], children[1], children[2])
}

fn vertical_geometry() -> FlowGeometry {
    FlowGeometry::new(
        IfcAxes::new(WritingMode::VerticalRl, Direction::Ltr),
        PhysicalSize {
            width: 100.0,
            height: 40.0,
        },
        taffy::Rect::ZERO,
    )
}

#[test]
fn vertical_mixed_ifc_block_children_use_their_bfc_context() {
    for writing_mode in ["vertical-rl", "vertical-lr"] {
        let (float, _uncleared, _) = layout_vertical_mixed_ifc(writing_mode, "none");
        let (same_float, cleared, orthogonal) = layout_vertical_mixed_ifc(writing_mode, "both");
        assert_eq!(same_float, float);
        if writing_mode == "vertical-rl" {
            assert!(
                cleared.0 + cleared.2 <= float.0,
                "{writing_mode}: float={float:?}, cleared={cleared:?}"
            );
        } else {
            assert!(
                float.0 + float.2 <= cleared.0,
                "{writing_mode}: float={float:?}, cleared={cleared:?}"
            );
        }
        assert_eq!((orthogonal.2, orthogonal.3), (10.0, 10.0), "{writing_mode}");
    }
}

#[test]
fn table_children_do_not_share_the_root_bfc() {
    let mut table = 0;
    let mut ordinary = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:40px",
        |doc, root| {
            table = doc.append_element(
                Some(root),
                "table",
                Style::default(),
                Some("display:table;width:20px;height:20px"),
            );
            ordinary = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;width:20px;height:20px"),
            );
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");

    let session = LayoutSession::new(&mut doc);
    assert!(super::super::in_the_roots_formatting_context(
        &session, ordinary
    ));
    assert!(!super::super::in_the_roots_formatting_context(
        &session, table
    ));
}

fn layout_vertical_aligned_block_ifc(
    writing_mode: &str,
    align_content: &str,
    with_ancestor_float: bool,
) -> (BoxRect, BoxRect, bool) {
    let mut wrapper = 0;
    let mut cleared = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with(
        &format!("writing-mode:{writing_mode};width:100px;height:40px"),
        |doc, root| {
            if with_ancestor_float {
                doc.append_element(
                    Some(root),
                    "div",
                    Style::default(),
                    Some("display:block;float:left;width:10px;height:20px"),
                );
            }
            wrapper = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some(&format!(
                    "display:block;align-content:{align_content};width:100px;height:40px"
                )),
            );
            doc.append_text(wrapper, "a");
            cleared = doc.append_element(
                Some(wrapper),
                "div",
                Style::default(),
                Some("display:block;clear:both;width:20px;height:20px"),
            );
            doc.append_text(wrapper, "b");
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");

    let beside_floats = doc.nodes[wrapper]
        .ifc
        .as_ref()
        .and_then(|ifc| ifc.lines.as_ref())
        .expect("aligned paragraph lines")
        .beside_floats;
    (
        absolute_rect(&doc, wrapper),
        absolute_rect(&doc, cleared),
        beside_floats,
    )
}

#[test]
fn vertical_aligned_block_ifc_does_not_share_ancestor_floats() {
    for (writing_mode, align_content) in [("vertical-rl", "center"), ("vertical-lr", "end")] {
        let (wrapper_without_float, cleared_without_float, _) =
            layout_vertical_aligned_block_ifc(writing_mode, align_content, false);
        let (wrapper_with_float, cleared_with_float, beside_floats) =
            layout_vertical_aligned_block_ifc(writing_mode, align_content, true);

        assert!(
            !beside_floats,
            "{writing_mode} align-content:{align_content} paragraph inherited an ancestor float"
        );
        assert_eq!(
            cleared_with_float.0 - wrapper_with_float.0,
            cleared_without_float.0 - wrapper_without_float.0,
            "{writing_mode} align-content:{align_content} clear inherited an ancestor float"
        );
    }
}

fn layout_vertical_auto_width_ifc_with_float() -> (BoxRect, BoxRect) {
    let mut float = 0;
    let mut cleared = 0;
    let (mut doc, cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:100px",
        |doc, root| {
            let wrapper = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;width:auto;height:auto"),
            );
            float = doc.append_element(
                Some(wrapper),
                "div",
                Style::default(),
                Some("display:block;float:left;width:10px;height:10px"),
            );
            doc.append_text(wrapper, "a");
            cleared = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;clear:both;width:10px;height:10px"),
            );
        },
    );
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (absolute_rect(&doc, float), absolute_rect(&doc, cleared))
}

#[test]
fn vertical_auto_width_ifc_probe_does_not_duplicate_its_float() {
    let (float, cleared) = layout_vertical_auto_width_ifc_with_float();

    assert_eq!(float, (90.0, 0.0, 10.0, 10.0));
    assert_eq!(cleared, (80.0, 0.0, 10.0, 10.0));
}

#[test]
fn vertical_independent_bfc_child_avoids_ancestor_float() {
    let mut child = 0;
    let (mut doc, _cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:40px",
        |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:flow-root;width:100px;height:40px"),
            );
        },
    );
    let geometry = vertical_geometry();
    let mut session = LayoutSession::new(&mut doc);
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.set_origin_resolved(true);
    ctx.place_floated_box(
        Size {
            width: 20.0,
            height: 10.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        false,
    );

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        child,
        0.0,
        CollapsibleMarginSet::ZERO,
        geometry,
        false,
        false,
    );

    assert_eq!(child.top, 10.0);
}

#[test]
fn vertical_independent_bfc_slot_probes_height_and_rtl_auto_margins() {
    let mut child = 0;
    let (mut doc, _cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:40px",
        |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:flow-root;width:5px;height:10px;direction:rtl;margin-bottom:auto"),
            );
        },
    );
    let geometry = FlowGeometry::new(
        IfcAxes::new(WritingMode::VerticalRl, Direction::Rtl),
        PhysicalSize {
            width: 100.0,
            height: 40.0,
        },
        taffy::Rect::ZERO,
    );
    let mut session = LayoutSession::new(&mut doc);
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.set_origin_resolved(true);
    ctx.place_floated_box(
        Size {
            width: 20.0,
            height: 10.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        true,
    );
    ctx.place_floated_box(
        Size {
            width: 30.0,
            height: 20.0,
        },
        10.0,
        FloatDirection::Right,
        Clear::None,
        false,
    );

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        child,
        0.0,
        CollapsibleMarginSet::ZERO,
        geometry,
        false,
        true,
    );

    assert_eq!(child.top, 0.0);
    assert_eq!(child.next_y, 5.0);
}

#[test]
fn vertical_shared_block_clearance_uses_the_pending_margin() {
    let mut child = 0;
    let (mut doc, _cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:40px",
        |doc, root| {
            child = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;width:100px;height:40px;clear:both"),
            );
        },
    );
    let geometry = vertical_geometry();
    let mut session = LayoutSession::new(&mut doc);
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.set_origin_resolved(false);
    ctx.place_floated_box(
        Size {
            width: 20.0,
            height: 10.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        true,
    );

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        child,
        0.0,
        CollapsibleMarginSet::from_margin(4.0),
        geometry,
        false,
        true,
    );

    assert_eq!(child.top, 6.0);
    assert_eq!(child.next_y, 106.0);
}

#[test]
fn vertical_inline_boxes_use_the_physical_top_inset() {
    for (writing_mode, block_start_inset) in [
        ("vertical-rl", "padding-right:11px"),
        ("vertical-lr", "padding-left:17px"),
    ] {
        let root_style = asymmetric_vertical_root_style(writing_mode, block_start_inset);
        let mut inline = 0;
        let (mut doc, cascade, root) = ahem_paragraph_with(&root_style, |doc, root| {
            inline =
                doc.append_element(Some(root), "span", Style::default(), Some("display:inline"));
            doc.append_text(inline, "a");
        });
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert_eq!(
            absolute_rect(&doc, inline).1,
            absolute_rect(&doc, root).1 + 3.0,
            "{writing_mode} inline box"
        );
    }
}

#[test]
fn vertical_inline_atomics_use_the_physical_top_inset() {
    for (writing_mode, block_start_inset) in [
        ("vertical-rl", "padding-right:11px"),
        ("vertical-lr", "padding-left:17px"),
    ] {
        let root_style = asymmetric_vertical_root_style(writing_mode, block_start_inset);
        let (mut doc, cascade, atomic, root) =
            ahem_paragraph_with_atomic("", "width:10px;height:10px", "", &root_style);
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert_eq!(
            absolute_rect(&doc, atomic).1,
            absolute_rect(&doc, root).1 + 3.0,
            "{writing_mode} inline atomic"
        );
    }
}

fn asymmetric_vertical_root_style(writing_mode: &str, block_start_inset: &str) -> String {
    format!("writing-mode:{writing_mode};height:40px;padding-top:3px;{block_start_inset}")
}

#[test]
fn vertical_ifc_float_uses_logical_extents_and_physical_placement() {
    for (writing_mode, block_start_inset, block_start_margin, expected) in [
        (
            "vertical-rl",
            "padding-right:11px",
            "margin-right:1px",
            (89.0, 20.0, 10.0, 20.0),
        ),
        (
            "vertical-lr",
            "padding-left:17px",
            "margin-left:1px",
            (18.0, 20.0, 10.0, 20.0),
        ),
    ] {
        let mut float = 0;
        let (mut doc, cascade, root) = ahem_paragraph_with(
            &format!(
                "writing-mode:{writing_mode};width:100px;height:37px;padding-top:3px;\
                 {block_start_inset}"
            ),
            |doc, root| {
                float = doc.append_element(
                    Some(root),
                    "div",
                    Style::default(),
                    Some(&format!(
                        "display:block;float:right;width:10px;height:20px;margin-top:2px;\
                         {block_start_margin}"
                    )),
                );
                doc.append_text(root, "a");
            },
        );
        doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
        layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
        assert_eq!(
            absolute_rect(&doc, float),
            expected,
            "{writing_mode} root={root}"
        );
        let lines = doc.nodes[root]
            .ifc
            .as_ref()
            .and_then(|root| root.lines.as_ref())
            .expect("paragraph lines");
        assert!(lines.beside_floats, "{writing_mode}");
        assert_eq!(
            lines.lines[0].remaining_inline_size(),
            5.0,
            "{writing_mode}"
        );
    }
}

#[test]
fn vertical_clear_does_not_use_a_descendant_float_as_its_clearance() {
    let mut clear = 0;
    let (mut doc, _cascade, _root) = ahem_paragraph_with(
        "writing-mode:vertical-rl;width:100px;height:40px",
        |doc, root| {
            clear = doc.append_element(
                Some(root),
                "div",
                Style::default(),
                Some("display:block;clear:both;width:20px;height:20px"),
            );
            doc.append_element(
                Some(clear),
                "div",
                Style::default(),
                Some("display:block;float:left;width:10px;height:10px"),
            );
        },
    );
    let geometry = vertical_geometry();
    let mut session = LayoutSession::new(&mut doc);
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.set_origin_resolved(true);

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        clear,
        0.0,
        CollapsibleMarginSet::ZERO,
        geometry,
        false,
        true,
    );

    assert_eq!(child.top, 0.0);
    assert_eq!(child.next_y, 20.0);
    assert!(child.top.is_finite());
    assert!(child.next_y.is_finite());
}

#[test]
fn vertical_empty_block_collapses_its_margins_through() {
    let (mut doc, empty, _, _) = vertical_block_child_fixture();
    let geometry = vertical_geometry();
    let mut session = LayoutSession::new(&mut doc);
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.set_origin_resolved(true);

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        empty,
        0.0,
        CollapsibleMarginSet::from_margin(15.0),
        geometry,
        false,
        true,
    );

    assert!(child.collapses_through);
    assert_eq!(child.top, 0.0);
    assert_eq!(child.next_y, 0.0);
    assert_eq!(child.pending.resolve(), 30.0);
    assert_eq!(child.top_margin.expect("leading margin").resolve(), 30.0);
}

#[test]
fn vertical_clearance_keeps_an_unresolved_parent_margin() {
    let (mut doc, _, clear, _) = vertical_block_child_fixture();
    let geometry = vertical_geometry();
    let mut session = LayoutSession::new(&mut doc);
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.place_floated_box(
        Size {
            width: 10.0,
            height: 10.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        true,
    );
    ctx.commit_strut();

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        clear,
        0.0,
        CollapsibleMarginSet::from_margin(4.0),
        geometry,
        false,
        true,
    );

    assert_eq!(child.top, 6.0);
    assert_eq!(child.next_y, 26.0);
    assert_eq!(child.top_margin, None);
    assert!(!child.collapses_through);

    ctx.set_origin_resolved(true);
    let resolved_child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        clear,
        0.0,
        CollapsibleMarginSet::from_margin(4.0),
        geometry,
        false,
        true,
    );

    assert_eq!(resolved_child.top, 10.0);
    assert_eq!(resolved_child.next_y, 30.0);
    assert_eq!(resolved_child.top_margin, None);
    assert!(!resolved_child.collapses_through);
}

#[test]
fn vertical_block_retries_when_a_collapsing_child_margin_moves_it() {
    let (mut doc, _, _, nested) = vertical_block_child_fixture();
    let geometry = vertical_geometry();
    let mut session = LayoutSession::new(&mut doc);
    assert!(super::super::in_the_roots_formatting_context(
        &session, nested
    ));
    let mut bfc = BlockFormattingContext::new();
    let mut ctx = bfc.root_block_context();
    ctx.set_width(geometry.width);
    ctx.set_origin_resolved(true);
    ctx.place_floated_box(
        Size {
            width: 10.0,
            height: 10.0,
        },
        0.0,
        FloatDirection::Left,
        Clear::None,
        false,
    );

    let child = super::super::layout_vertical_block_child(
        &mut session,
        &mut ctx,
        nested,
        0.0,
        CollapsibleMarginSet::ZERO,
        geometry,
        true,
        false,
    );

    assert_eq!(child.top, 30.0);
    assert_eq!(child.next_y, 70.0);
}
