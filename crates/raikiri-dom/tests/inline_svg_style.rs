//! The declarations a page stylesheet gives to the descendants of an inline
//! SVG, listed by the element's preorder index inside the SVG.

use raikiri_dom::inline_svg_style::{
    InlineSvgElementDeclarations, inline_svg_element_declarations,
};
use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, NodeKind, PageDefaults};

/// Declarations of the first `<svg>` of `html`. An id past the arena has none.
fn declarations(html: &str) -> Vec<InlineSvgElementDeclarations> {
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let page = result.page(0).expect("page");
    let (document, cascade, _, _) = page.paint_inputs();
    let svg = (0..document.node_count())
        .find(|&id| {
            document
                .get_node(id)
                .is_some_and(|node| node.kind() == NodeKind::Element && node.is_inline_svg_root())
        })
        .expect("an inline svg");
    assert!(
        inline_svg_element_declarations(document, cascade, document.node_count())
            .expect("an id past the arena is not an error")
            .is_empty()
    );
    inline_svg_element_declarations(document, cascade, svg).expect("declarations")
}

#[test]
fn declarations_are_listed_by_preorder_index_without_the_root() {
    let elements = declarations(
        "<style>svg{color:red}.a{display:none}.b{color:blue}</style>\
         <svg width=20 height=20><g><rect class=a /></g><rect class=b /><rect /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            (2, "display:none!important;"),
            (3, "color:rgba(0,0,255,1.000000)!important;"),
        ]
    );
}

#[test]
fn painting_properties_are_listed_as_declarations() {
    let elements = declarations(
        "<style>rect{fill:blue;stroke:url(#g) red;fill-opacity:25%}.w{stroke-width:2px}</style>\
         <svg width=20 height=20><rect class=w /><g><rect /></g></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    let rect = "fill:rgba(0,0,255,1)!important;stroke:url(\"#g\") rgba(255,0,0,1)!important;fill-opacity:0.25!important;";
    assert_eq!(
        listed,
        [
            (1, format!("{rect}stroke-width:2px!important;").as_str()),
            (3, rect),
        ]
    );
}

#[test]
fn stroke_and_rule_properties_are_listed_as_declarations() {
    let elements = declarations(
        "<style>path{fill-rule:EvenOdd;stroke-linecap:round;stroke-linejoin:arcs;\
         stroke-miterlimit:2;stroke-dasharray:4px,2;stroke-dashoffset:-1px}\
         .k{clip-rule:evenodd}</style>\
         <svg width=20 height=20><path class=k /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    assert_eq!(
        listed,
        [(
            1,
            "fill-rule:evenodd!important;clip-rule:evenodd!important;stroke-linecap:round!important;stroke-linejoin:miter!important;stroke-miterlimit:2!important;stroke-dasharray:4px 2!important;stroke-dashoffset:-1px!important;"
        )]
    );
}

#[test]
fn css_wide_stroke_keywords_are_listed_as_declarations() {
    let elements = declarations(
        "<style>path{stroke-dasharray:initial;stroke-linecap:inherit;stroke-miterlimit:unset;\
         stroke-dashoffset:initial}</style><svg width=20 height=20><path /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    assert_eq!(
        listed,
        [(
            1,
            "stroke-linecap:inherit!important;stroke-miterlimit:inherit!important;stroke-dasharray:none!important;stroke-dashoffset:0!important;"
        )]
    );
}

#[test]
fn a_rule_on_the_root_lists_its_stroke_and_rule_properties() {
    let elements = declarations(
        "<style>svg{stroke-dasharray:3 1;fill-rule:evenodd;visibility:visible}</style>\
         <svg width=20 height=20><rect /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    assert_eq!(
        listed,
        [(
            0,
            "fill-rule:evenodd!important;stroke-dasharray:3 1!important;"
        )]
    );
}

#[test]
fn css_wide_painting_keywords_are_listed_as_declarations() {
    let elements = declarations(
        "<style>rect{fill:inherit;stroke:unset;stroke-width:initial}</style>\
         <svg width=20 height=20><rect /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    assert_eq!(
        listed,
        [(
            1,
            "fill:inherit!important;stroke:inherit!important;stroke-width:1!important;"
        )]
    );
}

#[test]
fn a_rule_on_the_root_lists_only_its_painting_properties() {
    let elements = declarations(
        "<style>svg{color:red;opacity:.5;fill:blue;stroke-width:3px}</style>\
         <svg width=20 height=20><rect /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    assert_eq!(
        listed,
        [(
            0,
            "fill:rgba(0,0,255,1)!important;stroke-width:3px!important;"
        )]
    );
}

#[test]
fn an_svg_without_matching_rules_lists_nothing() {
    let elements = declarations("<svg width=20 height=20><rect /></svg>");
    assert!(elements.is_empty());
}

#[test]
fn revert_layer_lists_the_paint_attribute_it_rolls_back_to() {
    let elements = declarations(
        "<style>@layer a, b; @layer b {rect{fill:revert-layer;stroke:revert-layer}}</style>\
         <svg width=20 height=20><rect fill=red /></svg>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    // The attribute is restored; with none, `stroke` falls back to inheriting.
    assert_eq!(
        listed,
        [(
            1,
            "fill:rgba(255,0,0,1)!important;stroke:inherit!important;"
        )]
    );
}

#[test]
fn a_painting_attribute_with_a_variable_is_listed_resolved() {
    let elements = declarations(
        "<div style='--paint:blue;--w:3'><svg width=20 height=20>\
         <rect fill='var(--paint)' stroke=red stroke-width='var(--w)' /></svg></div>",
    );
    let listed: Vec<_> = elements
        .iter()
        .map(|element| (element.element_index, element.declarations.as_str()))
        .collect();
    // `stroke` has no variable and stays in the SVG source.
    assert_eq!(
        listed,
        [(1, "fill:rgba(0,0,255,1)!important;stroke-width:3!important;")]
    );
}
