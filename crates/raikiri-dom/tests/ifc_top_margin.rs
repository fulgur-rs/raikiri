//! Leading block margins escape ordinary IFC roots without enlarging their content boxes.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

fn fragments(html: &str, id: &str) -> Vec<(u32, f32, f32, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut out = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("page");
        let dom = page.dom();
        for fragment in page.fragments() {
            let node = fragment.node();
            let matches = if id == "#text" {
                dom.local_name(node).is_none()
            } else {
                dom.attr(node, "id") == Some(id)
            };
            if matches {
                let rect = fragment.rect();
                out.push((index, rect.x, rect.y, rect.height));
            }
        }
    }
    out
}

fn document(styles: &str, content: &str) -> String {
    format!(
        "<!doctype html><style>body{{margin:0;font:20px/20px Ahem}}{styles}</style>\
        <main style='display:flow-root'>{content}</main>"
    )
}

fn assert_box(html: &str, id: &str, top: f32, height: f32) {
    let boxes = fragments(html, id);
    assert_eq!(boxes.len(), 1, "{id}: {boxes:?}");
    let actual = boxes[0];
    assert!(
        (actual.2 - top).abs() < 0.01 && (actual.3 - height).abs() < 0.01,
        "{id}: expected y={top}, height={height}, got {actual:?}; {html}"
    );
}

#[test]
fn leading_source_block_margin_moves_the_ifc_root_without_adding_height() {
    let html = document(
        "",
        "<section id=root><div id=child style='margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 20.0, 40.0);
    assert_box(&html, "child", 20.0, 20.0);
    assert_box(&html, "after", 60.0, 20.0);
}

#[test]
fn leading_generated_block_margin_moves_the_ifc_root_without_adding_height() {
    let html = document(
        "section::before{content:'X';display:block;margin-top:20px}",
        "<section id=root>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 20.0, 40.0);
    assert_box(&html, "after", 60.0, 20.0);
}

#[test]
fn escaping_leading_margins_keep_their_signed_set_when_collapsing_with_the_roots() {
    for (own, expected) in [("0", 20.0), ("40px", 30.0), ("-15px", 15.0)] {
        let html = document(
            &format!("section{{margin-top:{own}}}"),
            "<section id=root><div style='margin-top:30px;margin-bottom:-10px'></div><div id=child>X</div>Y</section><div id=after>Z</div>",
        );
        assert_box(&html, "root", expected, 40.0);
        assert_box(&html, "child", expected, 20.0);
        assert_box(&html, "after", expected + 40.0, 20.0);
    }
}

#[test]
fn empty_generated_prefix_margins_escape_before_a_real_line() {
    let html = document(
        "section::before{content:'';display:block;margin-top:30px;margin-bottom:-10px}section{margin-top:40px}",
        "<section id=root>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 30.0, 20.0);
    assert_box(&html, "after", 50.0, 20.0);
}

#[test]
fn nested_mixed_roots_propagate_leading_margins_without_duplicate_offsets() {
    let html = document(
        "",
        "<article id=outer><section id=root><div id=child style='margin-top:20px'>X</div>Y</section>Z</article><div id=after>W</div>",
    );
    assert_box(&html, "outer", 20.0, 60.0);
    assert_box(&html, "root", 20.0, 40.0);
    assert_box(&html, "child", 20.0, 20.0);
    assert_box(&html, "after", 80.0, 20.0);
}

#[test]
fn borders_padding_and_formatting_contexts_keep_leading_margins_inside() {
    for (style, height, child) in [
        ("padding-top:1px", 61.0, 21.0),
        ("border-top:2px solid", 62.0, 22.0),
        ("display:flow-root", 60.0, 20.0),
        ("overflow:hidden", 60.0, 20.0),
        ("contain:layout", 60.0, 20.0),
        ("contain:paint", 60.0, 20.0),
    ] {
        let html = document(
            &format!("section{{{style}}}"),
            "<section id=root><div id=child style='margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
        );
        assert_box(&html, "root", 0.0, height);
        assert_box(&html, "child", child, 20.0);
        assert_box(&html, "after", height, 20.0);
    }
}

#[test]
fn a_float_in_a_collapsed_prefix_stays_above_its_clearing_sibling() {
    let html = document(
        "",
        "<section id=root><div id=empty style='margin-top:20px'><div id=float style='float:left;width:10px;height:10px'></div></div><div id=clear style='clear:both;height:10px'></div>X</section><div id=after>Y</div>",
    );
    assert_box(&html, "root", 20.0, 40.0);
    // Fragment projection currently grows auto-height float ancestors;
    // the collapse contract here is their origin and the following flow.
    assert!((fragments(&html, "empty")[0].2 - 20.0).abs() < 0.01);
    assert_box(&html, "float", 20.0, 10.0);
    assert_box(&html, "clear", 30.0, 10.0);
    assert_box(&html, "after", 60.0, 20.0);
}

#[test]
fn a_leading_float_does_not_separate_adjoining_top_margins() {
    let html = document(
        "",
        "<section id=root><div id=float style='float:left;width:10px;height:10px'></div><div id=child style='margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 20.0, 40.0);
    assert_box(&html, "float", 20.0, 10.0);
    assert_box(&html, "child", 20.0, 20.0);
    assert_box(&html, "after", 60.0, 20.0);
}

#[test]
fn external_float_clearance_keeps_the_cleared_margin_inside() {
    let html = document(
        "",
        "<div id=float style='float:left;width:10px;height:100px'></div><section id=root><div id=child style='clear:both;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 0.0, 140.0);
    assert_box(&html, "child", 100.0, 20.0);
    assert_box(&html, "after", 140.0, 20.0);
}

#[test]
fn clearance_preserves_an_earlier_escaping_empty_margin_set() {
    let html = document(
        "",
        "<div id=float style='float:left;width:10px;height:100px'></div><section id=root><div id=empty style='margin-top:30px'></div><div id=child style='clear:both;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 30.0, 110.0);
    assert_box(&html, "empty", 30.0, 0.0);
    assert_box(&html, "child", 100.0, 20.0);
    assert_box(&html, "after", 140.0, 20.0);
}

#[test]
fn a_leading_formatting_context_childs_own_margin_still_escapes() {
    let html = document(
        "",
        "<section id=root><div id=child style='display:flow-root;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 20.0, 40.0);
    assert_box(&html, "child", 20.0, 20.0);
    assert_box(&html, "after", 60.0, 20.0);
}

#[test]
fn clearance_separates_a_float_adjoining_the_roots_top_margin() {
    let html = document(
        "",
        "<section id=root><div id=float style='float:left;width:10px;height:10px'></div><div id=child style='clear:both;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 0.0, 50.0);
    assert_box(&html, "float", 0.0, 10.0);
    assert_box(&html, "child", 10.0, 20.0);
    assert_box(&html, "after", 50.0, 20.0);
}

#[test]
fn an_escaping_margin_keeps_a_nested_float_visible_to_following_clearance() {
    let html = document(
        "",
        "<section id=root><div id=float style='float:left;width:10px;height:100px'></div><div id=child style='margin-top:20px'>X</div>Y</section><div id=after style='clear:both'>Z</div>",
    );
    assert!((fragments(&html, "root")[0].2 - 20.0).abs() < 0.01);
    assert_box(&html, "float", 20.0, 100.0);
    assert_box(&html, "after", 120.0, 20.0);
}

#[test]
fn clear_without_floats_does_not_drop_a_leading_formatting_context_margin() {
    let html = document(
        "",
        "<section id=root><div id=child style='display:flow-root;clear:both;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 20.0, 40.0);
    assert_box(&html, "child", 20.0, 20.0);
    assert_box(&html, "after", 60.0, 20.0);
}

#[test]
fn generated_clearance_preserves_an_earlier_escaping_margin_set() {
    let html = document(
        "section::before{content:'';display:block;margin-top:30px}section::after{content:'X';display:block;clear:both;margin-top:20px}",
        "<div id=float style='float:left;width:10px;height:100px'></div><section id=root></section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 30.0, 90.0);
    assert_box(&html, "after", 120.0, 20.0);
}

#[test]
fn clearance_accounts_for_the_roots_own_margin_when_an_empty_margin_escapes() {
    let html = document(
        "section{margin-top:40px}",
        "<div id=float style='float:left;width:10px;height:100px'></div><section id=root><div style='margin-top:30px'></div><div id=child style='clear:both;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "child", 100.0, 20.0);
    assert_box(&html, "root", 40.0, 100.0);
    assert_box(&html, "after", 140.0, 20.0);
}

#[test]
fn escaping_past_an_external_float_remeasures_the_available_line_width() {
    let html = document(
        "main{width:100px}",
        "<div style='float:left;width:60px;height:30px'></div><section id=root><div id=child style='margin-top:40px'>XX XX</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 40.0, 40.0);
    assert_box(&html, "child", 40.0, 20.0);
    assert_box(&html, "after", 80.0, 20.0);
}

#[test]
fn a_later_mixed_child_commits_its_float_at_the_collapsed_margin_origin() {
    let html = document(
        "",
        "<section id=root>Y<article id=inner style='margin-top:10px'><div style='margin-top:40px'>X</div><div id=float style='float:left;width:10px;height:100px'></div>Z</article>W</section><div id=after style='clear:both'>Q</div>",
    );
    assert!((fragments(&html, "inner")[0].2 - 60.0).abs() < 0.01);
    assert_box(&html, "float", 80.0, 100.0);
    assert_box(&html, "after", 180.0, 20.0);
}

#[test]
fn nested_signed_margins_place_the_shared_float_only_once() {
    let html = document(
        "article{margin-top:40px}section{margin-top:-15px}",
        "<article><section id=root><div id=float style='float:left;width:10px;height:100px'></div><div style='margin-top:30px'>X</div>Y</section>Z</article><div id=after style='clear:both'>W</div>",
    );
    assert!((fragments(&html, "root")[0].2 - 25.0).abs() < 0.01);
    assert_box(&html, "float", 25.0, 100.0);
    assert_box(&html, "after", 125.0, 20.0);
}

#[test]
fn relative_offsets_do_not_change_the_shared_float_clearance_origin() {
    let html = document(
        "section{position:relative;top:50px}",
        "<section id=root><div id=float style='float:left;width:10px;height:100px'></div><div style='margin-top:20px'>X</div>Y</section><div id=after style='clear:both'>Z</div>",
    );
    assert!((fragments(&html, "root")[0].2 - 70.0).abs() < 0.01);
    assert_box(&html, "float", 70.0, 100.0);
    assert_box(&html, "after", 120.0, 20.0);
}

#[test]
fn a_deep_leading_strut_resolves_before_descendant_floats_are_committed() {
    let depth = 12;
    let mut content = "<section style='margin-top:40px'>".repeat(depth);
    content.push_str("<div id=float style='float:left;width:10px;height:100px'></div><div style='margin-top:30px'>X</div>");
    content.push_str(&"Y</section>".repeat(depth));
    content.push_str("<div id=after style='clear:both'>Z</div>");
    let html = document("", &content);
    assert_box(&html, "float", 40.0, 100.0);
    assert_box(&html, "after", 300.0, 20.0);
}

#[test]
fn an_empty_generated_box_with_height_ends_the_adjoining_float_strut() {
    let html = document(
        "section::before{content:'';display:block;height:20px}",
        "<article><div style='float:left;width:10px;height:10px'></div><section id=root><div id=child style='clear:both;margin-top:20px'>X</div>Y</section></article><div id=after>Z</div>",
    );
    assert_box(&html, "child", 40.0, 20.0);
    assert_box(&html, "root", 0.0, 80.0);
    assert_box(&html, "after", 80.0, 20.0);
}

#[test]
fn negative_leading_clearance_uses_the_hypothetical_ancestor_margin_set() {
    let html = document(
        "section{margin-top:100px}",
        "<div style='float:left;width:40px;height:90px'></div><section id=root><div id=child style='clear:left;margin-top:-80px;height:10px'></div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "child", 90.0, 10.0);
    assert_box(&html, "root", 100.0, 20.0);
    assert_box(&html, "after", 120.0, 20.0);
}

#[test]
fn negative_generated_clearance_uses_the_hypothetical_ancestor_margin_set() {
    let html = document(
        "section{margin-top:100px}section::before{content:'';display:block;clear:left;margin-top:-80px;height:10px}",
        "<div style='float:left;width:40px;height:90px'></div><section id=root>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 100.0, 20.0);
    assert_box(&html, "after", 120.0, 20.0);
}

#[test]
fn leading_formatting_context_margins_escape_beside_external_floats() {
    for style in [
        "display:flow-root",
        "overflow:hidden",
        "contain:layout",
        "contain:paint",
    ] {
        for (float_height, child_left) in [(10, 0.0), (100, 40.0)] {
            let html = document(
                "main{width:200px}",
                &format!(
                    "<div style='float:left;width:40px;height:{float_height}px'></div><section id=root><div id=child style='{style};width:100px;margin-top:20px'>X</div>Y</section><div id=after>Z</div>"
                ),
            );
            assert_box(&html, "root", 20.0, 40.0);
            assert_box(&html, "child", 20.0, 20.0);
            assert_eq!(fragments(&html, "child")[0].1, child_left);
            assert_box(&html, "after", 60.0, 20.0);
        }
    }
}

#[test]
fn float_avoidance_compares_the_hypothetical_collapsed_border_position() {
    for (float_height, root_top, root_height, child_top, after) in [
        (10, 20.0, 40.0, 20.0, 60.0),
        (100, 0.0, 140.0, 100.0, 140.0),
    ] {
        let html = document(
            "main{width:200px}",
            &format!(
                "<div style='float:left;width:40px;height:{float_height}px'></div><section id=root><div id=child style='display:flow-root;width:200px;margin-top:20px'>X</div>Y</section><div id=after>Z</div>"
            ),
        );
        assert_box(&html, "root", root_top, root_height);
        assert_box(&html, "child", child_top, 20.0);
        assert_box(&html, "after", after, 20.0);
    }
}

#[test]
fn negative_leading_clearance_also_resolves_for_a_formatting_context_child() {
    let html = document(
        "main{width:200px}section{margin-top:100px}",
        "<div style='float:left;width:40px;height:90px'></div><section id=root><div id=child style='display:flow-root;clear:left;margin-top:-80px;width:100px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "child", 90.0, 20.0);
    assert_box(&html, "root", 100.0, 30.0);
    assert_box(&html, "after", 130.0, 20.0);
}

#[test]
fn an_adjoining_float_forces_clearance_for_a_formatting_context_child() {
    let html = document(
        "",
        "<section id=root><div style='float:left;width:10px;height:10px'></div><div id=child style='display:flow-root;clear:both;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 0.0, 50.0);
    assert_box(&html, "child", 10.0, 20.0);
    assert_box(&html, "after", 50.0, 20.0);
}

#[test]
fn a_formatting_context_separates_from_an_adjoining_float_that_it_cannot_fit_beside() {
    let html = document(
        "main{width:200px}",
        "<section id=root><div style='float:left;width:200px;height:10px'></div><div id=child style='display:flow-root;width:200px;margin-top:20px'>X</div>Y</section><div id=after>Z</div>",
    );
    assert_box(&html, "root", 0.0, 50.0);
    assert_box(&html, "child", 10.0, 20.0);
    assert_box(&html, "after", 50.0, 20.0);
}
