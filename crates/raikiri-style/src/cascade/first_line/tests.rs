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
fn first_line_children_inherit_the_color_syntax_of_the_first_line() {
    let (mut doc, root) =
        tree_doc("#root{color:#e53935} #root::first-line{color:oklch(0.6 0.2 25)}");
    let child = doc.push_element(root, "span", None);
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let first = result.first_line.unwrap();
    // The first line is in a modern syntax, the rest of the paragraph is not.
    assert!(first.computed[child].as_ref().unwrap().color_modern);
    assert!(!result.normal.computed[child].color_modern);
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
    let detached_sibling = doc.push_element(1, "span", None);
    doc.nodes[detached_sibling].in_document = false;
    let text = doc.push_text(root, "actual inline text");
    let result = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let first = result.first_line.unwrap();
    assert_eq!(first.computed[text].as_ref().unwrap().color.r, 255);
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

#[test]
fn first_line_applies_direct_and_deferred_inline_wrapping() {
    for rule in [
        "text-wrap-mode:nowrap;hanging-punctuation:first",
        "text-wrap:var(--wrap);hanging-punctuation:var(--hang)",
    ] {
        let (mut doc, root) = tree_doc(&format!(
            "#root{{--wrap:nowrap;--hang:first}}#root::first-line{{{rule}}}"
        ));
        let text = doc.push_text(root, "actual text");
        let result = cascade_with_first_line(
            &doc,
            &build_rule_tree(&doc),
            &MediaContext::screen(),
            StyleNodeId(root as u64),
        )
        .unwrap();
        let first = result.first_line.unwrap();
        for id in [root, text] {
            let cv = first.computed[id].as_ref().unwrap();
            assert_eq!(
                cv.text_wrap,
                crate::property::TextWrapMode::Nowrap,
                "{rule}"
            );
            assert_ne!(
                cv.hanging_punctuation,
                ComputedValues::initial().hanging_punctuation,
                "{rule}"
            );
        }
        assert_eq!(
            result.normal.computed[root].text_wrap,
            crate::property::TextWrapMode::Wrap
        );
    }
}

#[test]
fn first_line_rejects_out_of_flow_descendants() {
    for css in ["position:absolute", "position:fixed", "float:left"] {
        let (mut doc, root) = tree_doc("#root::first-line{color:red}");
        doc.push_element(root, "span", Some(css));
        assert!(
            cascade_with_first_line(
                &doc,
                &build_rule_tree(&doc),
                &MediaContext::screen(),
                StyleNodeId(root as u64)
            )
            .is_err(),
            "{css}"
        );
    }
}

fn pseudo(css: &str) -> (ComputedValues, ComputedValues) {
    let mut doc = TestDoc::new();
    let s = doc.push_element(0, "style", None);
    doc.push_text(s, css);
    let p = doc.push_element(0, "p", None);
    let result = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    (
        result.computed[p].clone(),
        result.pseudo[&(StyleNodeId(p as u64), PseudoElem::FirstLine)]
            .as_ref()
            .clone(),
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

/// Measures `line-height: normal` as 1.25 times the font size.
struct ScaledNormal;

impl crate::resolve::NormalLineHeight for ScaledNormal {
    fn normal_line_height(&self, font: &crate::ChFontKey) -> Option<crate::ComputedLength> {
        Some(crate::ComputedLength(font.size.0 * 1.25))
    }
}

#[test]
fn first_line_options_measure_normal_and_take_the_viewport() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", Some("font-size:20px"));
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, "#root::first-line{font-size:40px}");
    let root = doc.push_element_with_attrs(html, "p", Some("display:block"), &[("id", "root")]);
    let span = doc.push_element(root, "span", Some("padding-left:1lh;padding-right:50vw"));
    let options = crate::CascadeOptions {
        viewport: Some((200.0, 100.0)),
        normal_line_height: Some(crate::NormalLineHeightSource::new(ScaledNormal)),
        ..crate::CascadeOptions::default()
    };
    let result = crate::cascade_with_first_line_options(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
        &options,
    )
    .unwrap();
    let px = crate::resolve::ComputedLengthPercentageOrCalc::Px;
    let normal = &result.normal.computed[span];
    assert_eq!(normal.padding.left, px(25.0));
    assert_eq!(normal.padding.right, px(100.0));
    let first = result.first_line.unwrap().computed[span].clone().unwrap();
    assert_eq!(first.padding.left, px(50.0));
    assert_eq!(first.padding.right, px(100.0));
}

fn color(r: u8, g: u8, b: u8) -> CssColor {
    CssColor { r, g, b, a: 255 }
}

#[test]
fn first_line_styles_reach_descendants_without_any_first_letter() {
    let (mut doc, root) =
        tree_doc("#root{color:red} #root::first-line{color:blue} .own{color:green}");
    let inherits = doc.push_element(root, "span", None);
    let own = doc.push_element_with_attrs(root, "span", None, &[("class", "own")]);
    let nested = doc.push_element(own, "span", None);
    let text = doc.push_text(inherits, "text");
    let result = crate::cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert!(result.has_first_line_styles());
    assert!(!result.has_first_letter_styles());
    let origins = [StyleNodeId(root as u64)];
    let style = |id: usize| {
        result
            .first_line_style(
                &doc,
                &origins,
                StyleNodeId(root as u64),
                StyleNodeId(id as u64),
                None,
            )
            .unwrap()
    };
    assert_eq!(result.computed[inherits].color, color(255, 0, 0));
    assert_eq!(style(root).color, color(0, 0, 255));
    assert_eq!(style(inherits).color, color(0, 0, 255));
    assert_eq!(style(own).color, color(0, 128, 0));
    assert_eq!(style(nested).color, color(0, 128, 0));
    let text = result.first_line_text_style(StyleNodeId(text as u64), &style(inherits));
    assert_eq!(text.color, color(0, 0, 255));
    assert_eq!(result.computed[inherits].color, color(255, 0, 0));
}

#[test]
fn first_line_styles_agree_with_the_strict_first_line_cascade() {
    let (mut doc, root) = tree_doc(
        "#root{font-size:16px;color:blue} #root::first-line{font-size:32px;color:red;text-transform:uppercase}",
    );
    let inherited = doc.push_element(root, "span", None);
    let equal = doc.push_element(root, "span", Some("font-size:16px;color:blue"));
    let outer = doc.push_element(root, "span", Some("font-size:20px"));
    let nested = doc.push_element(outer, "span", Some("font-size:150%"));
    let relative = doc.push_element(root, "span", Some("font-size:150%"));
    let strict = cascade_with_first_line(
        &doc,
        &build_rule_tree(&doc),
        &MediaContext::screen(),
        StyleNodeId(root as u64),
    )
    .unwrap();
    let expected = strict.first_line.unwrap();
    let result = crate::cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let origins = [StyleNodeId(root as u64)];
    for id in [root, inherited, equal, outer, nested, relative] {
        let got = result
            .first_line_style(
                &doc,
                &origins,
                StyleNodeId(root as u64),
                StyleNodeId(id as u64),
                None,
            )
            .unwrap();
        let want = expected.computed[id].as_ref().unwrap();
        assert_eq!(got.font_size, want.font_size, "font-size of {id}");
        assert_eq!(got.color, want.color, "color of {id}");
        assert_eq!(got.text_transform, want.text_transform, "transform of {id}");
    }
}

#[test]
fn blocks_around_the_paragraph_pass_only_their_first_line_through() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, "#outer::first-line{color:green}");
    let outer = doc.push_element_with_attrs(html, "div", Some("display:block"), &[("id", "outer")]);
    let blue = doc.push_element(outer, "div", Some("display:block;color:blue"));
    let paragraph = doc.push_element(blue, "p", Some("display:block;color:purple"));
    let span = doc.push_element(paragraph, "span", None);
    let own = doc.push_element(paragraph, "span", Some("color:red"));
    let result = crate::cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let origins = [StyleNodeId(outer as u64)];
    let style = |id: usize| {
        result
            .first_line_style(
                &doc,
                &origins,
                StyleNodeId(paragraph as u64),
                StyleNodeId(id as u64),
                None,
            )
            .unwrap()
    };
    // The paragraph's own `color` is for its box, not its first line.
    assert_eq!(result.computed[paragraph].color, color(128, 0, 128));
    assert_eq!(style(paragraph).color, color(0, 128, 0));
    assert_eq!(style(span).color, color(0, 128, 0));
    assert_eq!(style(own).color, color(255, 0, 0));
}

#[test]
fn first_line_style_is_none_without_a_first_line_style_in_reach() {
    let (mut doc, root) = tree_doc("#root{color:red}");
    let span = doc.push_element(root, "span", None);
    let result = crate::cascade(&doc, &build_rule_tree(&doc)).unwrap();
    assert!(!result.has_first_line_styles());
    assert!(result.typographic_inheritance.is_empty());
    let origins = [StyleNodeId(root as u64)];
    let ask = |origins: &[StyleNodeId]| {
        result.first_line_style(
            &doc,
            origins,
            StyleNodeId(root as u64),
            StyleNodeId(span as u64),
            None,
        )
    };
    assert!(ask(&origins).is_none());
    assert!(ask(&[]).is_none());
}

#[test]
fn first_line_styles_survive_sibling_sharing_and_extraction() {
    let mut doc = TestDoc::new();
    let html = doc.push_element(0, "html", None);
    let style = doc.push_element(html, "style", None);
    doc.push_text(style, "p::first-line{color:blue}");
    let body = doc.push_element(html, "body", None);
    let first = doc.push_element(body, "p", None);
    let second = doc.push_element(body, "p", None);
    let inner = doc.push_element(second, "span", None);
    let result = crate::cascade(&doc, &build_rule_tree(&doc)).unwrap();
    for (paragraph, element) in [(first, first), (second, inner)] {
        let origins = [StyleNodeId(paragraph as u64)];
        let values = result
            .first_line_style(
                &doc,
                &origins,
                StyleNodeId(paragraph as u64),
                StyleNodeId(element as u64),
                None,
            )
            .unwrap();
        assert_eq!(values.color, color(0, 0, 255));
    }
    let part = result.extract(&[0, html, body, second, inner]);
    assert!(part.has_first_line_styles());
    assert!(!result.extract(&[0, html, body]).has_first_line_styles());
}
