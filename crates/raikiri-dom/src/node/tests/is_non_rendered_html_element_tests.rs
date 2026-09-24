//! `<template>` 経路 test は
//! `is_in_document()` が先に発火するため、predicate 自体の direct
//! coverage が薄い。DOM predicate を builder + namespace mutation で
//! namespace 分岐まで含めて直接 check する。

use super::super::*;

fn html_element(tag: &str) -> Node {
    Node::new_element(SmolStr::new(tag), taffy::Style::default(), None)
}

fn set_ns(n: &mut Node, ns: &str) {
    if let NodeData::Element(e) = &mut n.data {
        e.namespace = Some(SmolStr::new(ns));
    }
}

/// d9y.5 の 9 element + s8w で追加した §15.3.1 完全化 4 element
/// (datalist / noembed / noframes / rp)。tag 列挙は
/// `Node::is_non_rendered_html_element` の match arms と 1:1 対応。
const SKIP_SET_TAGS: &[&str] = &[
    // d9y.5 original:
    "head", "title", "meta", "link", "base", "noscript", "script", "style", "template",
    // s8w additions (§15.3.1 完全化):
    "datalist", "noembed", "noframes", "rp",
];

#[test]
fn predicate_true_for_html_default_namespace_skip_set() {
    for tag in SKIP_SET_TAGS {
        let n = html_element(tag);
        assert!(
            n.is_non_rendered_html_element(),
            "{tag} in HTML default namespace (None) must be non-rendered"
        );
    }
}

#[test]
fn predicate_true_for_explicit_xhtml_namespace_skip_set() {
    for tag in SKIP_SET_TAGS {
        let mut n = html_element(tag);
        set_ns(&mut n, "http://www.w3.org/1999/xhtml");
        assert!(
            n.is_non_rendered_html_element(),
            "{tag} with explicit xhtml namespace must be non-rendered"
        );
    }
}

#[test]
fn predicate_false_for_svg_namespace_same_named_elements() {
    // SVG <title>, <style>, <script> は rendered / effective in SVG context。
    // predicate は HTML namespace のみ filter するのが契約 (paint 側は
    // SVG rendering を将来別 pipeline で扱う)。
    for tag in ["title", "style", "script"] {
        let mut n = html_element(tag);
        set_ns(&mut n, "http://www.w3.org/2000/svg");
        assert!(
            !n.is_non_rendered_html_element(),
            "SVG {tag} must NOT be filtered — SVG rendering owns these"
        );
    }
}

#[test]
fn predicate_false_for_mathml_namespace_same_named_elements() {
    for tag in ["style", "script"] {
        let mut n = html_element(tag);
        set_ns(&mut n, "http://www.w3.org/1998/Math/MathML");
        assert!(
            !n.is_non_rendered_html_element(),
            "MathML {tag} must NOT be filtered"
        );
    }
}

#[test]
fn predicate_false_for_normal_html_elements() {
    for tag in [
        "p", "div", "span", "h1", "a", "body", "html", "img", "table",
    ] {
        let n = html_element(tag);
        assert!(
            !n.is_non_rendered_html_element(),
            "{tag} is rendered content — predicate must return false"
        );
    }
}

#[test]
fn predicate_false_for_non_element_nodes() {
    // Text / Document node は Element でないので false。
    let text = Node::new_text(SmolStr::new("hi"));
    assert!(!text.is_non_rendered_html_element());
    let doc = Node::new_document();
    assert!(!doc.is_non_rendered_html_element());
}
