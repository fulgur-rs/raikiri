use crate::property::{CssColor, Direction, DisplayValue};
use crate::ruletree::build_rule_tree;
use crate::test_dom::TestDoc;
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
