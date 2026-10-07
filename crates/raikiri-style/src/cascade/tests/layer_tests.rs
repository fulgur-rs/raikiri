use super::*;
use crate::property::PropertyValue;
use cssparser::ToCss;
const GREEN: CssColor = CssColor {
    r: 0,
    g: 128,
    b: 0,
    a: 255,
};
use crate::cascade::test_support::{BLUE, RED};
use crate::counter_style::parse_counter_style_rules;
use crate::font_face::parse_font_face_rules;
use crate::media::MediaType;
use crate::page::{PageInheritance, cascade_page_with_media_context};
use crate::ruletree::Origin;
use crate::test_dom::TestDoc;

fn author_tree(sheets: &[&str]) -> RuleTree {
    let mut tree = RuleTree::empty();
    for sheet in sheets {
        tree.add_stylesheet(sheet, Origin::Author);
    }
    tree
}

fn result(tree: &RuleTree, media: &MediaContext, inline: Option<&str>) -> (usize, CascadeResult) {
    let mut doc = TestDoc::new();
    let id = doc.push_element_with_attrs(0, "p", inline, &[("id", "target")]);
    (id, cascade_with_media_context(&doc, tree, media).unwrap())
}

fn color(tree: &RuleTree, media: &MediaContext) -> CssColor {
    let (id, result) = result(tree, media, None);
    result.computed[id].color
}

fn page_color(tree: &RuleTree, media: &MediaContext) -> CssColor {
    let result = cascade_page_with_media_context(
        tree,
        &PageContextQuery {
            is_first: true,
            ..PageContextQuery::default()
        },
        PageInheritance::LegacyInitialValues,
        media,
    );
    match result.declarations().get(&PropertyKey::Color) {
        Some(PropertyValue::Color(color)) => *color,
        other => panic!("expected page color, got {other:?}"),
    }
}

#[test]
fn unlayered_rules_beat_later_layered_sheets() {
    let tree = author_tree(&["p {color:red}", "@layer a {#target {color:blue}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn named_layers_keep_their_first_order_across_sheets() {
    let tree = author_tree(&[
        "@layer a,b; @layer b {p{color:blue}}",
        "@layer a {#target{color:red}}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
}

#[test]
fn blocks_preceding_statements_establish_order() {
    let tree = author_tree(&["@layer b {p{color:blue}} @layer a,b; @layer a {p{color:red}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn same_layer_keeps_source_order_across_sheets() {
    let tree = author_tree(&["@layer a{p{color:blue}}", "@layer a{p{color:red}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn nested_and_dotted_layers_share_identity() {
    let tree = author_tree(&[
        "@layer a.b,a.c; @layer a { @layer c {p{color:blue}} }",
        "@layer a.b {#target{color:red}}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
}

#[test]
fn parent_implicit_layer_comes_after_its_children() {
    let tree = author_tree(&["@layer a {p{color:red} @layer b {#target{color:blue}}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn anonymous_layers_are_distinct_and_have_distinct_children() {
    let tree = author_tree(&[
        "@layer {@layer x{#target{color:red}}}",
        "@layer {@layer x{p{color:blue}}}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
}

#[test]
fn layer_identifiers_are_case_sensitive_and_escaped_names_reopen_layers() {
    let tree =
        author_tree(&[r"@layer a,A; @layer A{p{color:blue}} @layer \61 {#target{color:red}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
}

#[test]
fn important_reverses_layer_order_and_beats_unlayered_important() {
    let tree = author_tree(&[
        "@layer a,b; @layer a{p{color:red!important}}",
        "@layer b{#target{color:blue!important}} #target{color:green!important}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn important_reverses_nested_layers_including_parent_implicit_layer() {
    let tree =
        author_tree(&["@layer a {p{color:blue!important} @layer b{p{color:red!important}}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn inline_important_keeps_priority_above_stylesheet_layers() {
    let tree = author_tree(&["@layer a{#target{color:red!important}}"]);
    let (id, result) = result(&tree, &MediaContext::print(), Some("color:blue!important"));
    assert_eq!(result.computed[id].color, BLUE);
}

#[test]
fn layer_order_is_independent_for_each_origin() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet("@layer b,a; @layer a{p{color:green}}", Origin::User);
    tree.add_stylesheet(
        "@layer a,b; @layer a{p{color:red}} @layer b{p{color:blue}}",
        Origin::Author,
    );
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
    tree.add_stylesheet(
        "@layer b{p{color:red!important}} @layer a{p{color:green!important}}",
        Origin::User,
    );
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn conditional_first_mentions_change_order_per_media_context() {
    for first in ["@layer a {}", "@layer a;"] {
        let css = format!(
            "@media print {{{first}}} @layer b,a; @layer a{{p{{color:red}}}} @layer b{{p{{color:blue}}}}"
        );
        let tree = author_tree(&[&css]);
        assert_eq!(color(&tree, &MediaContext::print()), BLUE, "{first}");
        assert_eq!(color(&tree, &MediaContext::screen()), RED, "{first}");
    }
}

#[test]
fn sheet_conditions_guard_layer_first_mentions() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet_with_media("@layer a;", Origin::Author, Some("print"));
    tree.add_stylesheet(
        "@layer b,a; @layer a{p{color:red}} @layer b{p{color:blue}}",
        Origin::Author,
    );
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
    assert_eq!(color(&tree, &MediaContext::screen()), RED);
}

#[test]
fn viewport_conditions_guard_nested_layer_first_mentions() {
    let tree = author_tree(&[
        "@layer outer { @media (min-width:900px){@layer a;} @layer b,a; @layer a{p{color:red}} @layer b{p{color:blue}} }",
    ]);
    let narrow = MediaContext::with_viewport(MediaType::Screen, 400, 600);
    let wide = MediaContext::with_viewport(MediaType::Screen, 1000, 600);
    assert_eq!(color(&tree, &narrow), RED);
    assert_eq!(color(&tree, &wide), BLUE);
}

#[test]
fn false_supports_and_unknown_wrappers_do_not_establish_layers() {
    let tree = author_tree(&[
        "@supports (unknown:yes){@layer a;} @unknown {@layer a;} @layer b,a; @layer a{p{color:red}} @layer b{p{color:blue}}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn namespace_applies_inside_nested_layers() {
    let tree = author_tree(&[
        "@namespace svg url(http://www.w3.org/2000/svg); @layer a{@layer b{svg|p{color:red}}}",
    ]);
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(tree.style_rules()[0].selectors.to_css_string(), "svg|p");
}

#[test]
fn custom_properties_and_pseudo_elements_share_layer_priority() {
    let tree = author_tree(&[
        "@layer a,b; @layer b{p{--c:blue} p::before{color:blue}}",
        "@layer a{#target{--c:red} #target::before{color:red}} p{color:var(--c)}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(result.computed[id].color, BLUE);
    assert_eq!(
        result
            .pseudo
            .get(&(StyleNodeId(id as u64), PseudoElem::Before))
            .unwrap()
            .color,
        BLUE
    );
}

#[test]
fn custom_properties_share_important_layer_reversal() {
    let tree = author_tree(&[
        "@layer a,b; @layer a{p{--c:red!important}} @layer b{p{--c:blue!important}} p{color:var(--c)}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn page_layers_persist_across_sheets_and_reverse_for_important() {
    let normal = author_tree(&[
        "@layer a,b; @layer b{@page{color:blue}}",
        "@layer a{@page:first{color:red}}",
    ]);
    assert_eq!(page_color(&normal, &MediaContext::print()), BLUE);
    let important = author_tree(&[
        "@layer a,b; @layer a{@page{color:red!important}}",
        "@layer b{@page{color:blue!important}} @page{color:green!important}",
    ]);
    assert_eq!(page_color(&important, &MediaContext::print()), RED);
}

#[test]
fn page_layers_follow_conditional_first_mentions() {
    let tree = author_tree(&[
        "@media print{@layer a;} @layer b,a; @layer a{@page{color:red}} @layer b{@page{color:blue}}",
    ]);
    assert_eq!(page_color(&tree, &MediaContext::print()), BLUE);
    assert_eq!(page_color(&tree, &MediaContext::screen()), RED);
}

fn descriptors(marker: &str) -> String {
    format!(
        "@font-face{{font-family:demo;src:url({marker}.woff)}} @counter-style demo{{system:cyclic;symbols:\"{marker}\"}}"
    )
}

fn assert_descriptors(tree: &RuleTree, media: &MediaContext, marker: &str) {
    let css = descriptors(marker);
    assert_eq!(
        tree.font_faces_for(media).get("demo"),
        parse_font_face_rules(&css).first()
    );
    assert_eq!(
        tree.counter_styles_for(media).get("demo"),
        parse_counter_style_rules(&css).first()
    );
}

#[test]
fn descriptor_layers_persist_and_unlayered_rules_win_across_sheets() {
    let a = format!("@layer a,b; @layer b{{{}}}", descriptors("blue"));
    let b = format!("@layer a{{{}}}", descriptors("red"));
    let mut tree = author_tree(&[&a, &b]);
    assert_descriptors(&tree, &MediaContext::print(), "blue");
    tree.add_stylesheet(&descriptors("green"), Origin::Author);
    tree.add_stylesheet(&b, Origin::Author);
    assert_descriptors(&tree, &MediaContext::print(), "green");
    assert_eq!(
        tree.font_faces().get("demo"),
        parse_font_face_rules(&descriptors("green")).first()
    );
    assert_eq!(
        tree.counter_styles().get("demo"),
        parse_counter_style_rules(&descriptors("green")).first()
    );
}

#[test]
fn descriptor_layers_share_conditional_first_mentions() {
    let css = format!(
        "@media print{{@layer a;}} @layer b,a; @layer a{{{}}} @layer b{{{}}}",
        descriptors("red"),
        descriptors("blue")
    );
    let tree = author_tree(&[&css]);
    assert_descriptors(&tree, &MediaContext::print(), "blue");
    assert_descriptors(&tree, &MediaContext::screen(), "red");
}

#[test]
fn invalid_layer_preludes_do_not_execute_contents() {
    for source in [
        "@layer a,b {p{color:red}}",
        "@layer a..b {p{color:red}}",
        "@layer \"a\" {p{color:red}}",
    ] {
        let tree = author_tree(&[&format!("p{{color:blue}} {source}")]);
        assert_eq!(color(&tree, &MediaContext::print()), BLUE, "{source}");
    }
}

#[test]
fn layer_blocks_retain_raw_inspection_structure_and_source_order() {
    let tree = author_tree(&["@layer a {p{color:red} @layer b{p{color:blue}}} p{color:green}"]);
    let record = tree
        .opaque_at_rules()
        .iter()
        .find(|r| r.name == "layer")
        .expect("raw layer record");
    assert_eq!(
        record.to_css(),
        "@layer a {p{color:red} @layer b{p{color:blue}}}"
    );
    let orders: Vec<_> = tree.style_rules().iter().map(|r| r.source_order).collect();
    assert_eq!(orders, vec![0, 1, 2]);
    assert_eq!(color(&tree, &MediaContext::print()), GREEN);
}

#[test]
fn highlight_colors_preserve_layer_order_across_sheets() {
    let tree = author_tree(&[
        "@layer a,b; @layer b{::highlight(mark){background-color:blue}}",
        "@layer a{::highlight(mark){background-color:red}}",
    ]);
    assert_eq!(tree.custom_highlight_styles().get("mark"), Some(&BLUE));
}

#[test]
fn layer_name_grammar_rejects_whitespace_and_css_wide_keywords() {
    for name in [
        "a .b",
        "a. b",
        "a..b",
        "initial",
        "InHeRiT",
        "a.unset",
        "revert",
        "revert-layer",
    ] {
        let tree = author_tree(&[&format!(
            "p{{color:blue}} @layer {name}{{p{{color:red!important}}}}"
        )]);
        assert_eq!(color(&tree, &MediaContext::print()), BLUE, "{name}");
    }
    let tree = author_tree(&["@layer default {p{color:red!important}}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn border_revert_layer_uses_previous_layer_without_rolling_back_origin() {
    let tree = author_tree(&[
        "@layer a,b; @layer a{p{border-top:7px solid red}} @layer b{p{border-top:revert-layer}}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(result.computed[id].border.top.width().px(), 7.0);
    assert_eq!(
        result.computed[id].border.top.color,
        crate::property::BorderColor::Resolved(RED)
    );
}

#[test]
fn preferred_sizes_and_white_space_share_layer_precedence() {
    let tree = author_tree(&[
        "@layer a,b; @layer b{p{inline-size:17px;white-space:pre}} @layer a{#target{width:3px;white-space-collapse:collapse}}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(
        result.computed[id].width,
        crate::resolve::ComputedLengthPercentageOrAuto::Px(17.0)
    );
    assert_eq!(
        result.computed[id].effective_white_space_collapse,
        crate::property::WhiteSpaceCollapse::Preserve
    );
}

#[test]
fn border_revert_layer_handles_important_order_and_repeated_markers() {
    for (css, width, color) in [
        (
            "@layer a,b,c; @layer a{p{border-top:7px solid red}} @layer b{p{border-top:revert-layer!important}} @layer c{p{border-top:9px solid blue!important}}",
            7.0,
            RED,
        ),
        (
            "@layer a,b,c; @layer a{p{border-top:7px solid red}} @layer b{p{border-top:revert-layer}} @layer c{p{border-top:revert-layer}}",
            7.0,
            RED,
        ),
        (
            "@layer a{p{border-top:7px solid red}} p{border-top:revert-layer}",
            7.0,
            RED,
        ),
    ] {
        let tree = author_tree(&[css]);
        let (id, result) = result(&tree, &MediaContext::print(), None);
        assert_eq!(result.computed[id].border.top.width().px(), width);
        assert_eq!(
            result.computed[id].border.top.color,
            crate::property::BorderColor::Resolved(color)
        );
    }
}

#[test]
fn page_descriptors_and_custom_properties_share_layer_order() {
    for (suffix, expected_color, expected_size) in [("", BLUE, 20.0), ("!important", RED, 10.0)] {
        let css = format!(
            "@layer a,b; @layer a{{@page{{--c:red{suffix};size:10px{suffix};marks:crop{suffix};bleed:1px{suffix}}}}} @layer b{{@page{{--c:blue{suffix};size:20px{suffix};marks:cross{suffix};bleed:2px{suffix}}}}} @page{{color:var(--c)}}"
        );
        let tree = author_tree(&[&css]);
        let page = cascade_page_with_media_context(
            &tree,
            &PageContextQuery::default(),
            PageInheritance::LegacyInitialValues,
            &MediaContext::print(),
        );
        assert_eq!(
            page.declarations().get(&PropertyKey::Color),
            Some(&PropertyValue::Color(expected_color))
        );
        assert_eq!(
            page.size(),
            Some(crate::page::PageSize::Lengths {
                width: crate::property::Length::Px(expected_size),
                height: crate::property::Length::Px(expected_size)
            })
        );
        assert_eq!(
            page.marks(),
            Some(crate::page::PageMarks::Marks {
                crop: !suffix.is_empty(),
                cross: suffix.is_empty()
            })
        );
        assert_eq!(
            page.bleed(),
            Some(crate::page::PageBleed::Length(crate::property::Length::Px(
                expected_size / 10.0
            )))
        );
    }
}

#[test]
fn descriptor_origin_policy_and_same_layer_source_order_are_preserved() {
    let mut tree = RuleTree::empty();
    tree.add_stylesheet(
        &format!("@layer user{{{}}}", descriptors("user")),
        Origin::User,
    );
    tree.add_stylesheet(
        &format!("@layer author{{{}}}", descriptors("first")),
        Origin::Author,
    );
    tree.add_stylesheet(
        &format!("@layer author{{{}}}", descriptors("last")),
        Origin::Author,
    );
    assert_descriptors(&tree, &MediaContext::print(), "last");
    tree.add_stylesheet(&descriptors("ua"), Origin::UserAgent);
    assert_descriptors(&tree, &MediaContext::print(), "last");
}

#[test]
fn highlight_important_declarations_reverse_layers_and_keep_same_rule_importance() {
    let tree = author_tree(&[
        "@layer a,b; @layer a{::highlight(mark){background-color:red!important; background-color:green}} @layer b{::highlight(mark){background-color:blue!important}}",
    ]);
    assert_eq!(tree.custom_highlight_styles().get("mark"), Some(&RED));
}

#[test]
fn layer_block_error_recovery_and_depth_limits_preserve_following_rules() {
    for source in [
        "@layer; p{color:blue}",
        "@layer a{p{color:red}} p{color:blue}",
        "@layer a,b{p{color:red}} p{color:blue}",
    ] {
        assert_eq!(color(&author_tree(&[source]), &MediaContext::print()), BLUE);
    }
    let source = format!(
        "{}p{{color:red}}{}p{{color:blue}}",
        "@layer a{".repeat(132),
        "}".repeat(132)
    );
    let tree = author_tree(&[&source]);
    assert_eq!(tree.style_rules().len(), 1);
    assert_eq!(color(&tree, &MediaContext::print()), BLUE);
    let tree = author_tree(&["@layer a{p{color:red}"]);
    assert!(tree.style_rules().is_empty());
}

#[test]
fn all_revert_layer_restores_previous_layer_and_keeps_its_exclusions() {
    let tree = author_tree(&[
        "@layer{#target{width:100px;height:100px;background-color:green;--c:red;direction:ltr}} @layer{#target{width:200px;height:200px;background-color:red;--c:blue;direction:rtl} #target{all:revert-layer}} p{color:var(--c)}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(
        result.computed[id].width,
        crate::resolve::ComputedLengthPercentageOrAuto::Px(100.0)
    );
    assert_eq!(result.computed[id].background_color, GREEN);
    assert_eq!(result.computed[id].color, BLUE);
    assert_eq!(
        result.computed[id].direction,
        crate::property::Direction::Rtl
    );
}

#[test]
fn transplanted_page_rule_does_not_use_another_trees_layer_identity() {
    let mut source = author_tree(&["@layer a{} @layer b{@page{color:red}}"]);
    let mut target = author_tree(&["@layer c{@page{color:blue}}"]);
    target.page_rules.push(source.page_rules.pop().unwrap());
    assert_eq!(page_color(&target, &MediaContext::print()), RED);
}

#[test]
fn all_revert_layer_handles_important_pseudo_and_repeated_layers() {
    let tree = author_tree(&[
        "@layer a,b,c; @layer a{p{color:red} p::before{color:red}} @layer b{p,p::before{color:blue!important;all:revert-layer!important}} @layer c{p,p::before{color:blue!important}}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(result.computed[id].color, RED);
    assert_eq!(
        result.pseudo[&(StyleNodeId(id as u64), PseudoElem::Before)].color,
        RED
    );
    let tree = author_tree(&[
        "@layer a,b,c; @layer a{p{color:red}} @layer b{p{all:revert-layer}} @layer c{p{color:blue; all:revert-layer}}",
    ]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn all_revert_layer_applies_to_page_and_margin_contexts() {
    let tree = author_tree(&[
        "@layer a{@page{color:red; @top-left{color:red}}} @layer b{@page{color:blue; all:revert-layer; @top-left{color:blue;all:revert-layer}}}",
    ]);
    assert_eq!(page_color(&tree, &MediaContext::print()), RED);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    assert!(!page.declarations().contains_key(&PropertyKey::All));
    let margin = crate::page::PageMarginBoxCascadeResult::cascade_matching(
        page.margin_boxes(),
        crate::page::PageMarginBoxSlot::TopLeft,
    )
    .unwrap();
    assert_eq!(
        margin
            .declarations
            .iter()
            .find(|d| d.value().key() == PropertyKey::Color)
            .unwrap()
            .value(),
        &PropertyValue::Color(RED)
    );
    assert!(
        margin
            .declarations
            .iter()
            .all(|d| d.value().key() != PropertyKey::All)
    );
}

#[test]
fn important_inline_layer_rollback_retains_stylesheet_important_declarations() {
    let tree = author_tree(&["@layer a{p{color:red!important}} p{color:blue!important}"]);
    let (id, result) = result(
        &tree,
        &MediaContext::print(),
        Some("color:blue!important;all:revert-layer!important"),
    );
    assert_eq!(result.computed[id].color, RED);
}

#[test]
fn important_layer_rollback_excludes_intervening_animation_origin() {
    let mut tree = author_tree(&[
        "@layer a,b; @layer a{p{color:red}} @layer b{p{all:revert-layer!important}}",
    ]);
    tree.add_stylesheet("p{color:blue}", Origin::Animation);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
}

#[test]
fn all_layer_rollback_removes_page_and_margin_important_intervals() {
    let tree = author_tree(&[
        "@layer a,b,c; @layer a{@page{color:red;@top-left{color:red}}} @layer b{@page{color:blue!important;all:revert-layer!important;@top-left{color:blue!important;all:revert-layer!important}}} @layer c{@page{color:blue!important;@top-left{color:blue!important}}}",
    ]);
    assert_eq!(page_color(&tree, &MediaContext::print()), RED);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    let margin = crate::page::PageMarginBoxCascadeResult::cascade_matching(
        page.margin_boxes(),
        crate::page::PageMarginBoxSlot::TopLeft,
    )
    .unwrap();
    assert_eq!(
        margin
            .declarations
            .iter()
            .find(|d| d.value().key() == PropertyKey::Color)
            .unwrap()
            .value(),
        &PropertyValue::Color(RED)
    );
}

#[test]
fn all_layer_rollback_can_leave_no_page_or_margin_property() {
    let tree =
        author_tree(&["@page{color:blue;all:revert-layer;@top-left{color:blue;all:revert-layer}}"]);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    assert!(!page.declarations().contains_key(&PropertyKey::Color));
    let margin = crate::page::PageMarginBoxCascadeResult::cascade_matching(
        page.margin_boxes(),
        crate::page::PageMarginBoxSlot::TopLeft,
    )
    .unwrap();
    assert!(margin.declarations.is_empty());
}

#[test]
fn all_layer_rollback_preserves_page_and_margin_excluded_properties() {
    let tree = author_tree(&[
        "@page{direction:rtl;unicode-bidi:bidi-override;--c:blue;all:revert-layer;@top-left{direction:rtl;unicode-bidi:bidi-override;--c:blue;all:revert-layer}}",
    ]);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    assert_eq!(
        page.declarations().get(&PropertyKey::Direction),
        Some(&PropertyValue::Direction(crate::property::Direction::Rtl))
    );
    assert!(page.declarations().contains_key(&PropertyKey::UnicodeBidi));
    let margin = crate::page::PageMarginBoxCascadeResult::cascade_matching(
        page.margin_boxes(),
        crate::page::PageMarginBoxSlot::TopLeft,
    )
    .unwrap();
    assert_eq!(margin.declarations.len(), 3);
    assert!(
        margin
            .declarations
            .iter()
            .any(|d| matches!(d.value(), PropertyValue::CustomProperty(_)))
    );
}

#[test]
fn border_and_all_layer_rollback_share_one_exclusion_chain() {
    let tree = author_tree(&[
        "@layer a,b,c; @layer a{p{border-top:7px solid red}} @layer b{p{border-top:revert-layer}} @layer c{p{border-top:9px solid blue;all:revert-layer}}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(result.computed[id].border.top.width().px(), 7.0);
    assert_eq!(
        result.computed[id].border.top.color,
        crate::property::BorderColor::Resolved(RED)
    );
}

#[test]
fn deferred_border_layer_rollback_uses_the_resolved_keyword() {
    let tree = author_tree(&[
        "@layer a,b; @layer a{p{border-top:7px solid red}} @layer b{p{--rollback:revert-layer;border-top-style:solid;border-top-width:var(--rollback)}}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(result.computed[id].border.top.width().px(), 7.0);
}

#[test]
fn layers_order_within_ua_hint_and_animation_origins() {
    for origin in [
        Origin::UserAgent,
        Origin::AuthorPresentationalHint,
        Origin::Animation,
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@layer a,b; @layer a{#target{color:red}} @layer b{p{color:blue}}",
            origin,
        );
        assert_eq!(color(&tree, &MediaContext::print()), BLUE);
    }
}

#[test]
fn all_layer_rollback_selects_each_border_side_and_can_chain_origin_rollback() {
    for (origin, width) in [
        (Origin::Author, 7.0),
        (Origin::User, 7.0),
        (Origin::UserAgent, 0.0),
    ] {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p{border:7px solid red}", Origin::UserAgent);
        tree.add_stylesheet("@layer a,b; @layer a{p{border:revert}} @layer b{p{border:9px solid blue;all:revert-layer!important}}",origin);
        let (id, result) = result(&tree, &MediaContext::print(), None);
        let border = &result.computed[id].border;
        for side in [&border.top, &border.right, &border.bottom, &border.left] {
            assert_eq!(side.width().px(), width);
        }
    }
    let tree = author_tree(&[
        "@layer a,b,c; @layer a{p{border:7px solid red}} @layer b{p{border:revert-layer}} @layer c{p{border:9px solid blue;all:revert-layer}}",
    ]);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    let border = &result.computed[id].border;
    for side in [&border.top, &border.right, &border.bottom, &border.left] {
        assert_eq!(side.width().px(), 7.0);
        assert_eq!(side.color, crate::property::BorderColor::Resolved(RED));
    }
}

#[test]
fn page_all_rollback_keeps_each_properties_candidates_separate() {
    let tree = author_tree(&[
        "@layer a{@page{color:red;width:17px;@top-left{color:red;width:17px}}} @layer b{@page{color:blue;width:3px;all:revert-layer;@top-left{color:blue;width:3px;all:revert-layer}}}",
    ]);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    assert_eq!(
        page.declarations().get(&PropertyKey::Width),
        Some(&PropertyValue::Width(
            crate::property::LengthOrAuto::Length(crate::property::Length::Px(17.0))
        ))
    );
    let margin = crate::page::PageMarginBoxCascadeResult::cascade_matching(
        page.margin_boxes(),
        crate::page::PageMarginBoxSlot::TopLeft,
    )
    .unwrap();
    assert_eq!(margin.declarations.len(), 2);
    assert_eq!(
        margin
            .declarations
            .iter()
            .find(|d| d.value().key() == PropertyKey::Color)
            .unwrap()
            .value(),
        &PropertyValue::Color(RED)
    );
}

#[test]
fn invalid_layer_name_tokens_are_rejected_and_all_marker_serializes() {
    let tree = author_tree(&["@layer a:b{p{color:blue}} @layer a!{p{color:blue}} p{color:red}"]);
    assert_eq!(color(&tree, &MediaContext::print()), RED);
    assert_eq!(
        crate::property::serialize_value(&PropertyValue::AllRevertLayer).as_deref(),
        Some("revert-layer")
    );
}

#[test]
fn border_rollback_continues_through_deferred_fallback_markers() {
    for final_rule in [
        "border-top-width:revert-layer",
        "--y:revert-layer;border-top-width:var(--y)",
    ] {
        let tree = author_tree(&[&format!(
            "@layer a,b,c; @layer a{{p{{border-top:7px solid red}}}} @layer b{{p{{--x:revert-layer;border-top-width:var(--x)}}}} @layer c{{p{{{final_rule}}}}}"
        )]);
        let (id, result) = result(&tree, &MediaContext::print(), None);
        assert_eq!(result.computed[id].border.top.width().px(), 7.0);
    }
    let mut tree = author_tree(&[
        "@layer a,b; @layer a{p{--x:revert;border-top-width:var(--x)}} @layer b{p{border-top-width:revert-layer}} p{border-top-style:solid}",
    ]);
    tree.add_stylesheet("p{border-top:7px solid red}", Origin::User);
    let (id, result) = result(&tree, &MediaContext::print(), None);
    assert_eq!(result.computed[id].border.top.width().px(), 7.0);
}

#[test]
fn static_highlight_all_rollback_restores_previous_layer() {
    for css in [
        "@layer a{::highlight(mark){background-color:blue}} @layer b{::highlight(mark){background-color:red;all:revert-layer}}",
        "@layer a{::highlight(mark){background-color:blue}} @layer b{::highlight(mark){background-color:red} ::highlight(mark){all:revert-layer}}",
        "@layer a,b,c; @layer a{::highlight(mark){background-color:blue}} @layer b{::highlight(mark){background-color:red!important;all:revert-layer!important}} @layer c{::highlight(mark){background-color:red!important}}",
    ] {
        let tree = author_tree(&[css]);
        assert_eq!(tree.custom_highlight_styles().get("mark"), Some(&BLUE));
    }
    let tree = author_tree(&["::highlight(mark){background-color:red;all:revert-layer}"]);
    assert!(!tree.custom_highlight_styles().contains_key("mark"));
}

#[test]
fn public_origin_changes_do_not_reuse_another_origins_layer_rank() {
    let mut tree =
        author_tree(&["@layer x{p{color:red} @page{color:red;size:A4;@top-left{color:red}}}"]);
    tree.add_stylesheet(
        "@layer a,b; @layer b{p{color:blue} @page{color:blue;size:letter;@top-left{color:blue}}}",
        Origin::User,
    );
    let expected_size = tree.page_rules[0].size_declarations[0].value();
    tree.page_rules[0].origin = Origin::User;
    tree.style_rules[0].origin = Origin::User;
    assert_eq!(color(&tree, &MediaContext::print()), RED);
    assert_eq!(page_color(&tree, &MediaContext::print()), RED);
    let page = cascade_page_with_media_context(
        &tree,
        &PageContextQuery::default(),
        PageInheritance::LegacyInitialValues,
        &MediaContext::print(),
    );
    assert_eq!(page.size(), Some(expected_size));
    let margin = crate::page::PageMarginBoxCascadeResult::cascade_matching(
        page.margin_boxes(),
        crate::page::PageMarginBoxSlot::TopLeft,
    )
    .unwrap();
    assert_eq!(
        margin
            .declarations
            .iter()
            .find(|d| d.value().key() == PropertyKey::Color)
            .unwrap()
            .value(),
        &PropertyValue::Color(RED)
    );
}
