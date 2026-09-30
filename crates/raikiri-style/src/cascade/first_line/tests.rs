use crate::property::{CssColor, Direction, DisplayValue};
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;
use crate::{MediaContext, StyleDom, cascade_with_first_line};

fn tree_doc(css: &str) -> (TestDoc, usize) {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", Some("font-size:20px;line-height:30px"));
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, css);
    let root = doc.push_element_with_attrs(html, "p", Some("display:block"), &[("id", "root")]);
    (doc, root)
}

#[test]
fn first_line_child_winners_preserve_provenance() {
    let (mut doc, root) = tree_doc(
        "#root{font-size:16px;color:blue} #root::first-line{font-size:32px;color:red;text-transform:uppercase}",
    );
    let inherited = doc.push_element(root, "span", None);
    let equal = doc.push_element(root, "span", Some("font-size:16px;color:blue"));
    let different = doc.push_element(root, "span", Some("font-size:20px"));
    let outer = doc.push_element(root, "span", Some("font-size:20px"));
    let nested = doc.push_element(outer, "span", Some("font-size:150%"));
    let relative = doc.push_element(root, "span", Some("font-size:150%"));
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let first = result.first_line.unwrap();
    let ids = [inherited, equal, different, nested, relative];
    assert_eq!(
        ids.map(|id| result.normal.computed[id].font_size.0),
        [16.0, 16.0, 20.0, 30.0, 24.0]
    );
    assert_eq!(
        ids.map(|id| first.computed[id].as_ref().unwrap().font_size.0),
        [32.0, 16.0, 20.0, 30.0, 48.0]
    );
    assert_eq!(
        first.computed[inherited].as_ref().unwrap().color,
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        first.computed[equal].as_ref().unwrap().color,
        CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255
        }
    );
    let normal = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert_eq!(result.normal.computed, normal.computed);
    assert_eq!(first.computed.len(), doc.node_count());
}

#[test]
fn first_line_preserves_normal_inheritance_channels() {
    let (mut doc, root) = tree_doc(
        "#root{color:blue;background-color:green;--size:16px;--ink:blue} #root::first-line{font-size:32px;color:red;background-color:red;--size:45px;--ink:red;direction:rtl} span{color:blue} #child{color:inherit;background-color:inherit;font-size:var(--size)}",
    );
    let child = doc.push_element_with_attrs(root, "span", None, &[("id", "child")]);
    let var = doc.push_element(root, "span", Some("color:var(--ink)"));
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let first = result.first_line.unwrap();
    let cv = first.computed[child].as_ref().unwrap();
    assert_eq!(cv.font_size.0, 16.0);
    assert_eq!(
        cv.color,
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        cv.background_color,
        result.normal.computed[root].background_color
    );
    assert_eq!(cv.direction, result.normal.computed[child].direction);
    assert_eq!(
        first.computed[var].as_ref().unwrap().color,
        result.normal.computed[var].color
    );
    assert_eq!(
        cv.custom_properties,
        result.normal.computed[child].custom_properties
    );
}

#[test]
fn first_line_preserves_document_relative_units() {
    let (mut doc, root) = tree_doc("#root::first-line{font-size:32px}");
    let child = doc.push_element(root, "span", Some("font-size:1rem;line-height:1rlh"));
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let first = result.first_line.unwrap();
    let cv = first.computed[child].as_ref().unwrap();
    assert_eq!(cv.font_size.0, 20.0);
    assert_eq!(cv.line_height, result.normal.computed[child].line_height);
}

#[test]
fn first_line_inherit_winner_and_var_substitution() {
    let (mut doc, root) = tree_doc(
        "#root{font-size:16px;color:blue;background-color:green;--inherit:inherit} #root::first-line{font-size:32px;color:red} span{font-size:20px;color:blue} #child{font-size:inherit;color:var(--inherit);background-color:var(--inherit)}",
    );
    let child = doc.push_element_with_attrs(root, "span", None, &[("id", "child")]);
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    assert_eq!(result.normal.computed[child].font_size.0, 16.0);
    let first = result.first_line.unwrap();
    let cv = first.computed[child].as_ref().unwrap();
    assert_eq!(cv.font_size.0, 32.0);
    assert_eq!(
        cv.color,
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(
        cv.background_color,
        result.normal.computed[root].background_color
    );
}

#[test]
fn first_line_rejects_unsupported_roots_and_subtrees() {
    let (mut doc, root) = tree_doc("#root::first-line{color:red}");
    let detached = doc.push_element(root, "span", None);
    doc.nodes[detached].in_document = false;
    let hidden = doc.push_element(root, "span", Some("display:none"));
    let hidden_child = doc.push_element(hidden, "span", None);
    let sibling = doc.push_element(1, "p", Some("display:block"));
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let first = result.first_line.unwrap();
    for id in [0, 1, detached, hidden, hidden_child, sibling] {
        assert!(first.computed[id].is_none());
    }
    assert!(
        cascade_with_first_line(
            &doc,
            &build_rule_tree(&doc),
            &MediaContext::screen(),
            StyleNodeId(999)
        )
        .is_err()
    );
    let inline = doc.push_element(root, "span", None);
    assert!(
        cascade_with_first_line(
            &doc,
            &build_rule_tree(&doc),
            &MediaContext::screen(),
            StyleNodeId(inline as u64)
        )
        .is_err()
    );
    for display in ["block", "inline-block"] {
        doc.nodes[inline].inline_style = Some(format!("display:{display}"));
        assert!(
            cascade_with_first_line(
                &doc,
                &build_rule_tree(&doc),
                &MediaContext::screen(),
                StyleNodeId(root as u64)
            )
            .is_err()
        );
    }
    let (doc, root) = tree_doc("");
    assert!(
        cascade_with_first_line(
            &doc,
            &build_rule_tree(&doc),
            &MediaContext::screen(),
            StyleNodeId(root as u64)
        )
        .unwrap()
        .first_line
        .is_none()
    );
}
use crate::{ComputedValues, PseudoElem, StyleNodeId, cascade};

fn pseudo(css: &str) -> (ComputedValues, ComputedValues) {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, css);
    let p = doc.push_element(0, "p", None);
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    (
        result.computed[p].clone(),
        result.pseudo[&(StyleNodeId(p as u64), PseudoElem::FirstLine)].clone(),
    )
}

#[test]
fn first_line_filters_box_and_excluded_properties() {
    let (normal, first) = pseudo(
        "p{display:block;direction:ltr} p::first-line{display:block;margin:19px;width:55px;direction:rtl;writing-mode:vertical-rl;text-orientation:upright}",
    );
    assert_eq!(normal.display, DisplayValue::Block);
    assert_eq!(first.display, DisplayValue::Inline);
    assert_eq!(first.direction, Direction::Ltr);
    assert_eq!(first.cssom_writing_mode, normal.cssom_writing_mode);
    assert_eq!(first.text_orientation, normal.text_orientation);
    assert_eq!(first.margin, ComputedValues::initial().margin);
    assert_eq!(first.width, ComputedValues::initial().width);
}

#[test]
fn first_line_keeps_inline_properties_and_vars() {
    let (_, first) = pseudo(
        "p{--size:27px} p::first-line{font-size:var(--size);color:red;opacity:.5;line-height:2;letter-spacing:3px;text-transform:uppercase;background-color:blue}",
    );
    assert_eq!(first.font_size.0, 27.0);
    assert_eq!(
        first.color,
        CssColor {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        }
    );
    assert_eq!(first.opacity, 0.5);
    assert_eq!(
        first.background_color,
        CssColor {
            r: 0,
            g: 0,
            b: 255,
            a: 255
        }
    );
    assert_eq!(
        first.text_transform,
        crate::property::TextTransform::Uppercase
    );
}

#[test]
fn first_line_filters_deferred_shorthand() {
    let (_, first) = pseudo(
        "p{--box:29px;--font:italic 24px serif} p::first-line{margin:var(--box);font:var(--font);direction:garbage;color:garbage}",
    );
    assert_eq!(first.margin, ComputedValues::initial().margin);
    assert_eq!(first.font_size.0, 24.0);
    assert_eq!(first.font_style, crate::property::FontStyle::Italic);
}
