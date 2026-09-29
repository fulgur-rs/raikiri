use super::*;
use raikiri_style::{ComputedValues, Origin, RuleTree, cascade};

/// Ordinary light-DOM children appended directly under a `<template>`
/// element (the DOM `appendChild` shape) are in the document tree: HTML
/// §4.12.3 stores template contents in a separate fragment node rather than
/// as the element's children, so these children stay in-document.
#[test]
fn template_real_children_stay_in_document() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    let text = doc.append_text(inner, "hi");
    let nested = doc.append_element(Some(inner), "span", Style::default(), None::<&str>);

    doc.mark_in_document_flags();

    assert!(
        doc.get_node(tmpl).unwrap().is_in_document(),
        "template element itself stays in-document"
    );
    assert!(
        doc.get_node(inner).unwrap().is_in_document(),
        "real <p> child of <template> stays in-document"
    );
    assert!(
        doc.get_node(text).unwrap().is_in_document(),
        "text under a real <template> child stays in-document"
    );
    assert!(
        doc.get_node(nested).unwrap().is_in_document(),
        "nested element under a real <template> child stays in-document"
    );
}

/// A `<template>` element's associated contents fragment (the parser's
/// shape, via [`Document::allocate_template_fragment_root`]) is detached
/// from the Document root, so the fragment root and everything under it
/// stays out-of-document while the template element itself stays in.
#[test]
fn template_contents_fragment_children_stay_out_of_document() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    let frag = doc.allocate_template_fragment_root(tmpl);
    let span = doc.append_element(Some(frag), "span", Style::default(), None::<&str>);
    let text = doc.append_text(span, "x");

    doc.mark_in_document_flags();

    assert!(
        doc.get_node(tmpl).unwrap().is_in_document(),
        "template element itself stays in-document"
    );
    assert!(
        !doc.get_node(frag).unwrap().is_in_document(),
        "contents fragment root stays out-of-document"
    );
    assert!(
        !doc.get_node(span).unwrap().is_in_document(),
        "<span> in the contents fragment stays out-of-document"
    );
    assert!(
        !doc.get_node(text).unwrap().is_in_document(),
        "text in the contents fragment stays out-of-document"
    );
}

/// Selector matching effect: a rule reaches a real `<template>` child
/// through the cascade but never reaches the inert contents fragment.
#[test]
fn cascade_styles_template_real_children_but_not_fragment_contents() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
    let real = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    let frag = doc.allocate_template_fragment_root(tmpl);
    let inert = doc.append_element(Some(frag), "p", Style::default(), None::<&str>);

    let mut rules = RuleTree::empty();
    rules.add_stylesheet("p { color: red }", Origin::Author);
    doc.mark_in_document_flags();
    let result = cascade(&doc, &rules).expect("cascade Ok");

    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_ne!(
        result.computed[real].color,
        ComputedValues::initial().color,
        "a rule must apply to a real <template> child"
    );
    assert_eq!(
        result.computed[inert].color,
        ComputedValues::initial().color,
        "a rule must not reach the inert contents fragment"
    );
}

/// Sibling-combinator effect: real `<template>` children take part in
/// flat-tree sibling matching, so `span + span` styles the second one.
#[test]
fn sibling_combinator_matches_template_real_children() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
    let first = doc.append_element(Some(tmpl), "span", Style::default(), None::<&str>);
    let second = doc.append_element(Some(tmpl), "span", Style::default(), None::<&str>);

    let mut rules = RuleTree::empty();
    rules.add_stylesheet("span + span { color: red }", Origin::Author);
    doc.mark_in_document_flags();
    let result = cascade(&doc, &rules).expect("cascade Ok");

    assert_eq!(
        result.computed[first].color,
        ComputedValues::initial().color,
        "the first sibling must not match `span + span`"
    );
    // cov:ignore: panic-message literal only executed on assertion
    // failure, which doesn't happen while this test passes.
    assert_ne!(
        result.computed[second].color,
        ComputedValues::initial().color,
        "the second real <template> child must match `span + span`"
    );
}
