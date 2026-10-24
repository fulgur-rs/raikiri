//! The root element takes the used `writing-mode`, `direction` and
//! `text-orientation` of the principal element, and keeps its computed values
//! (CSS Writing Modes 4 §8).

use super::*;
use crate::page::cascade_page_with_media_context;
use crate::property::{Direction, DisplayValue, TextOrientation, WritingMode};
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;

const SVG: &str = "http://www.w3.org/2000/svg";

struct Page {
    html: usize,
    body: usize,
    result: CascadeResult,
}

fn page(html_style: Option<&str>, body_style: Option<&str>) -> Page {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", html_style);
    let body = doc.push_element(html, "body", body_style);
    Page {
        html,
        body,
        result: cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok"),
    }
}

fn flow(values: &ComputedValues) -> (WritingMode, Direction, TextOrientation) {
    (
        values.cssom_writing_mode,
        values.direction,
        values.text_orientation,
    )
}

#[test]
fn the_body_flow_becomes_the_used_flow_of_the_root() {
    let Page { html, body, result } = page(
        None,
        Some("writing-mode: vertical-rl; direction: rtl; text-orientation: upright"),
    );
    let used = (
        WritingMode::VerticalRl,
        Direction::Rtl,
        TextOrientation::Upright,
    );
    assert_eq!(result.principal_element_index(), body);
    assert_eq!(flow(&result.computed[html]), used);
    assert_eq!(flow(&result.computed[body]), used);
    assert_eq!(flow(result.root_element_computed()), used);
    // The layout fallback of the keyword follows too.
    assert_eq!(
        result.computed[html].writing_mode,
        result.computed[body].writing_mode
    );
    assert_eq!(
        result.authored_writing_modes[html],
        Some(WritingMode::VerticalRl)
    );

    // CSSOM reports what the root computed to.
    assert_eq!(
        flow(&result.cssom_computed(html)),
        (
            WritingMode::HorizontalTb,
            Direction::Ltr,
            TextOrientation::Mixed
        )
    );
    assert_eq!(*result.cssom_computed(body), result.computed[body]);
    let own = result.cssom_computed(html).into_owned();
    assert_eq!(*result.root_element_cssom_computed(), own);
    let all = result.into_cssom_computed();
    assert_eq!(all[html], own);
    assert_eq!(all[body].cssom_writing_mode, WritingMode::VerticalRl);
}

#[test]
fn the_body_flow_replaces_one_the_root_declares() {
    let Page { html, body, result } = page(
        Some("writing-mode: vertical-lr; direction: rtl"),
        Some("writing-mode: horizontal-tb; direction: ltr"),
    );
    let used = (
        WritingMode::HorizontalTb,
        Direction::Ltr,
        TextOrientation::Mixed,
    );
    assert_eq!(flow(&result.computed[html]), used);
    assert_eq!(
        result.authored_writing_modes[html],
        Some(WritingMode::HorizontalTb)
    );
    assert_eq!(
        flow(&result.cssom_computed(html)),
        (
            WritingMode::VerticalLr,
            Direction::Rtl,
            TextOrientation::Mixed
        )
    );
    assert_ne!(
        result.computed[html].cssom_writing_mode,
        result.cssom_computed(html).cssom_writing_mode
    );
    assert_eq!(result.principal_element_index(), body);
}

#[test]
fn a_body_without_its_own_flow_leaves_the_root_unchanged() {
    let Page { html, body, result } = page(Some("writing-mode: vertical-rl"), None);
    // The body inherits the root's flow, so the two agree already.
    assert_eq!(result.principal_element_index(), body);
    assert_eq!(flow(&result.computed[html]), flow(&result.computed[body]));
    assert_eq!(
        result.authored_writing_modes[html],
        Some(WritingMode::VerticalRl)
    );
    assert!(matches!(
        result.cssom_computed(html),
        std::borrow::Cow::Borrowed(_)
    ));
    assert_eq!(result.clone().into_cssom_computed(), result.computed);
}

#[test]
fn the_html_and_body_tag_names_match_in_any_case() {
    // Type selectors match HTML tag names ignoring ASCII case, and so do the
    // elements that take part in the propagation.
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "HTML", None);
    let body = doc.push_element(html, "Body", Some("writing-mode: vertical-rl"));
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok");
    assert_eq!(result.principal_element_index(), body);
    assert_eq!(
        result.computed[html].cssom_writing_mode,
        WritingMode::VerticalRl
    );
}

#[test]
fn only_the_direction_can_differ() {
    let Page { html, result, .. } = page(None, Some("direction: rtl"));
    assert_eq!(result.computed[html].direction, Direction::Rtl);
    assert_eq!(result.cssom_computed(html).direction, Direction::Ltr);
    // No `writing-mode` was authored, so the root has none of its own either.
    assert_eq!(result.authored_writing_modes[html], None);
}

#[test]
fn containment_on_the_root_or_the_body_stops_the_propagation() {
    for (html_style, body_style) in [
        (Some("contain: layout"), Some("writing-mode: vertical-rl")),
        (None, Some("writing-mode: vertical-rl; contain: paint")),
        (Some("contain: style"), Some("direction: rtl")),
    ] {
        let Page { html, result, .. } = page(html_style, body_style);
        assert_eq!(result.principal_element_index(), html);
        assert_eq!(
            flow(&result.computed[html]),
            flow(&result.cssom_computed(html))
        );
        assert_eq!(
            result.computed[html].cssom_writing_mode,
            WritingMode::HorizontalTb
        );
        assert_eq!(result.computed[html].direction, Direction::Ltr);
        assert_eq!(result.authored_writing_modes[html], None);
    }
}

#[test]
fn a_body_that_generates_no_box_does_not_propagate() {
    let Page { html, result, .. } = page(
        None,
        Some("display: none; writing-mode: vertical-rl; direction: rtl"),
    );
    assert_eq!(result.principal_element_index(), html);
    assert_eq!(
        result.computed[html].cssom_writing_mode,
        WritingMode::HorizontalTb
    );
    assert_eq!(result.computed[html].direction, Direction::Ltr);
}

#[test]
fn a_body_lookalike_or_a_root_of_another_kind_does_not_propagate() {
    let css = "body, html { writing-mode: vertical-rl }";
    // A `body` element of another namespace under an HTML root.
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, "body { writing-mode: vertical-rl }");
    doc.push_element_with_namespace(html, "body", SVG, &[]);
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok");
    assert_eq!(result.principal_element_index(), html);
    assert_eq!(
        result.computed[html].cssom_writing_mode,
        WritingMode::HorizontalTb
    );

    // An `html` element of another namespace at the root.
    let mut doc = TestDoc::new();
    let root = doc.push_element_with_namespace(0, "html", SVG, &[]);
    let style = doc.push_element(root, "style", None);
    doc.push_text(style, "body { direction: rtl }");
    doc.push_element(root, "body", None);
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok");
    assert_eq!(result.principal_element_index(), root);
    assert_eq!(result.computed[root].direction, Direction::Ltr);

    // A root that is not `html`, and a tree without an element.
    let mut doc = TestDoc::new();
    let root = doc.push_element(0, "div", None);
    let style = doc.push_element(root, "style", None);
    doc.push_text(style, css);
    doc.push_element(root, "body", None);
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok");
    assert_eq!(result.principal_element_index(), root);
    let mut doc = TestDoc::new();
    doc.push_text(0, "text only");
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade Ok");
    assert_eq!(result.principal_element_index(), 0);
}

#[test]
fn what_inherits_from_the_root_inherits_its_own_flow() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(
        style,
        "html::after { content: \"x\" } body { writing-mode: vertical-rl; direction: rtl }",
    );
    let body = doc.push_element(html, "body", None);
    let sibling = doc.push_element(html, "div", None);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");

    let used = (
        WritingMode::VerticalRl,
        Direction::Rtl,
        TextOrientation::Mixed,
    );
    let own = (
        WritingMode::HorizontalTb,
        Direction::Ltr,
        TextOrientation::Mixed,
    );
    assert_eq!(flow(&result.computed[html]), used);
    assert_eq!(flow(&result.computed[body]), used);
    assert_eq!(flow(&result.computed[sibling]), own);
    let after = &result.pseudo[&(StyleNodeId(html as u64), PseudoElem::After)];
    assert_eq!(flow(after), own);

    // The `@page` context inherits the root's own values too.
    for query in [
        PageContextQuery::default(),
        PageContextQuery {
            is_first: true,
            ..PageContextQuery::default()
        },
    ] {
        let page = cascade_page_with_media_context(
            &tree,
            &query,
            PageInheritance::FromRoot(&result.cssom_computed(html)),
            &MediaContext::default(),
        );
        assert_eq!(
            page,
            result.cascade_page_from_root(&tree, &query, &MediaContext::default())
        );
        assert_eq!(page, result.page);
    }
}

#[test]
fn extract_keeps_the_roots_own_values_with_the_root() {
    let Page { html, body, result } = page(None, Some("writing-mode: vertical-lr"));
    assert_ne!(
        result.computed[html].cssom_writing_mode,
        result.cssom_computed(html).cssom_writing_mode
    );

    let kept = result.extract(&[0, html, body]);
    assert_eq!(kept.principal_element_index(), 2);
    assert_eq!(kept.computed[1].cssom_writing_mode, WritingMode::VerticalLr);
    assert_eq!(
        kept.cssom_computed(1).cssom_writing_mode,
        WritingMode::HorizontalTb
    );

    // Without the body the principal element is the root; the root's entry
    // keeps the values it was given, so its own are still the ones to report.
    let rootless_body = result.extract(&[0, html]);
    assert_eq!(rootless_body.principal_element_index(), 1);
    assert_eq!(
        rootless_body.cssom_computed(1).cssom_writing_mode,
        WritingMode::HorizontalTb
    );

    // Without the root nothing is the root element's.
    let without_root = result.extract(&[0, body]);
    assert_eq!(without_root.principal_element_index(), 0);
    assert!(matches!(
        without_root.cssom_computed(0),
        std::borrow::Cow::Borrowed(_)
    ));
    assert_eq!(
        without_root.computed[1].cssom_writing_mode,
        WritingMode::VerticalLr
    );
}

#[test]
fn the_root_keeps_the_physical_sizes_its_own_writing_mode_mapped() {
    use crate::resolve::ComputedLengthPercentageOrAuto as L;
    // `inline-size` is the height of a vertical root, the width of a
    // horizontal one, whatever the used writing mode is.
    let Page { html, result, .. } = page(
        Some("writing-mode: vertical-lr; inline-size: 300px"),
        Some("writing-mode: horizontal-tb"),
    );
    let used = &result.computed[html];
    assert_eq!((used.width, used.height), (L::Auto, L::Px(300.0)));
    assert_eq!(used.vertical_logical_size, None);
    assert!(result.cssom_computed(html).vertical_logical_size.is_some());

    let Page { html, result, .. } = page(
        Some("inline-size: 300px"),
        Some("writing-mode: vertical-lr"),
    );
    let used = &result.computed[html];
    assert_eq!((used.width, used.height), (L::Px(300.0), L::Auto));
    assert_eq!(used.vertical_logical_size, None);
}

#[test]
fn what_the_root_took_is_put_back_over_its_current_values() {
    // Consumers rewrite the entries of `computed` in place after the cascade;
    // CSSOM reports those, with the root's own flow.
    let Page {
        html, mut result, ..
    } = page(None, Some("writing-mode: vertical-rl"));
    result.computed[html].display = DisplayValue::None;
    let own = result.cssom_computed(html);
    assert_eq!(own.display, DisplayValue::None);
    assert_eq!(own.cssom_writing_mode, WritingMode::HorizontalTb);
    let all = result.into_cssom_computed();
    assert_eq!(all[html].display, DisplayValue::None);
    assert_eq!(all[html].cssom_writing_mode, WritingMode::HorizontalTb);
}
