use super::*;
use crate::resolve::ComputedLengthPercentageOrAuto as Dimension;

fn check_cached_contexts(source: &str, css_expected: (Dimension, Dimension)) {
    const SVG: &str = "http://www.w3.org/2000/svg";
    for svg_first in [true, false] {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, "rect,div{width:30px;height:40px}");
        let mut expected = Vec::new();
        for _ in 0..3 {
            for is_svg in [svg_first, !svg_first] {
                let node = if is_svg {
                    let node = doc.push_element_with_namespace(0, "rect", SVG, &[]);
                    doc.nodes[node].inline_style = Some(source.into());
                    node
                } else {
                    doc.push_element(0, "div", Some(source))
                };
                expected.push((
                    node,
                    if is_svg {
                        (Dimension::Px(20.0), Dimension::Px(10.0))
                    } else {
                        css_expected.clone()
                    },
                ));
                let animated = doc.push_element_with_namespace(0, "rect", SVG, &[]);
                doc.nodes[animated].animation_style = Some(source.into());
                expected.push((animated, css_expected.clone()));
            }
        }
        let tree = build_rule_tree(&doc);
        let result = cascade(&doc, &tree).expect("cascade");
        for (node, (width, height)) in expected {
            assert_eq!(
                (&result.computed[node].width, &result.computed[node].height),
                (&width, &height),
                "svg_first={svg_first}, node={node}"
            );
        }
    }
}

#[test]
fn svg_unitless_inline_sizes_keep_their_grammar_across_cache_hits() {
    check_cached_contexts(
        "width:20;height:10",
        (Dimension::Px(30.0), Dimension::Px(40.0)),
    );
}

#[test]
fn svg_unitless_deferred_sizes_keep_their_context_across_cache_hits() {
    check_cached_contexts(
        "--w:20;--h:10;width:var(--w);height:var(--h)",
        (Dimension::Auto, Dimension::Auto),
    );
}

fn svg_sizes(source: &str, css: &str, tag: &str) -> (Dimension, Dimension) {
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, css);
    let node = doc.push_element_with_namespace(0, tag, "http://www.w3.org/2000/svg", &[]);
    doc.nodes[node].inline_style = Some(source.into());
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    (
        result.computed[node].width.clone(),
        result.computed[node].height.clone(),
    )
}

#[test]
fn svg_unitless_sizes_distinguish_parse_time_and_computed_time_invalidity() {
    let css = "rect,svg{width:30px;height:40px;--n:20}";
    for tag in ["rect", "svg"] {
        for (source, expected) in [
            (
                "WIDTH:20;Height:10",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:20px;height:10px",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:50%;height:25%",
                (Dimension::Percent(50.0), Dimension::Percent(25.0)),
            ),
            (
                "width:calc(20px);height:calc(10px)",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:20;height:10",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:20.5;height:10.5",
                (Dimension::Px(20.5), Dimension::Px(10.5)),
            ),
            (
                "width:2e1;height:1e1",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:/**/20/**/;height:/**/10/**/",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:var(--n);height:var(--n)",
                (Dimension::Px(20.0), Dimension::Px(20.0)),
            ),
            (
                "width:var(--missing,20);height:var(--missing,10)",
                (Dimension::Px(20.0), Dimension::Px(10.0)),
            ),
            (
                "width:-20;height:-10",
                (Dimension::Px(30.0), Dimension::Px(40.0)),
            ),
            (
                "width:-20 5px;height:-10 5px",
                (Dimension::Px(30.0), Dimension::Px(40.0)),
            ),
            (
                "width:20 junk;height:10 junk",
                (Dimension::Px(30.0), Dimension::Px(40.0)),
            ),
            (
                "width:calc(20);height:calc(10)",
                (Dimension::Px(30.0), Dimension::Px(40.0)),
            ),
            (
                "--n:-20;width:var(--n);height:var(--n)",
                (Dimension::Auto, Dimension::Auto),
            ),
            (
                "--n:20 junk;width:var(--n);height:var(--n)",
                (Dimension::Auto, Dimension::Auto),
            ),
            (
                "--n:calc(20);width:var(--n);height:var(--n)",
                (Dimension::Auto, Dimension::Auto),
            ),
            (
                "width:var(--missing);height:var(--missing)",
                (Dimension::Auto, Dimension::Auto),
            ),
        ] {
            assert_eq!(svg_sizes(source, css, tag), expected, "{tag}/{source}");
        }
    }
}

#[test]
fn svg_unitless_sizes_stabilize_numbers_before_saturating() {
    for (number, expected) in [
        ("1e40", f32::MAX),
        ("-0", 0.0),
        ("0e999", 0.0),
        ("-0e999", 0.0),
    ] {
        for source in [
            format!("width:{number};height:{number}"),
            format!("--n:{number};width:var(--n);height:var(--n)"),
        ] {
            assert_eq!(
                svg_sizes(&source, "rect{width:30px;height:40px}", "rect"),
                (Dimension::Px(expected), Dimension::Px(expected)),
                "{source}"
            );
        }
    }
    assert_eq!(
        svg_sizes(
            "width:-1e40;height:-1e40",
            "rect{width:30px;height:40px}",
            "rect"
        ),
        (Dimension::Px(30.0), Dimension::Px(40.0))
    );
    assert_eq!(
        svg_sizes(
            "--n:-1e40;width:var(--n);height:var(--n)",
            "rect{width:30px;height:40px}",
            "rect"
        ),
        (Dimension::Auto, Dimension::Auto)
    );
}

#[test]
fn svg_unitless_sizes_leave_stylesheets_and_other_properties_strict() {
    assert_eq!(
        svg_sizes("--n:20", "rect{width:var(--n);height:var(--n)}", "rect"),
        (Dimension::Auto, Dimension::Auto)
    );
    let mut doc = TestDoc::new();
    let style = doc.push_element(0, "style", None);
    doc.push_text(style, "rect{width:20;height:10;min-width:30px}");
    let node = doc.push_element_with_namespace(0, "rect", "http://www.w3.org/2000/svg", &[]);
    doc.nodes[node].inline_style = Some("min-width:20".into());
    let result = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    assert_eq!(result.computed[node].width, Dimension::Auto);
    assert_eq!(result.computed[node].height, Dimension::Auto);
    assert_eq!(result.computed[node].min_width, Dimension::Px(30.0));
}

#[test]
fn svg_unitless_size_importance_and_declaration_order_are_preserved() {
    assert_eq!(
        svg_sizes(
            "width:20;height:10",
            "rect{width:30px!important;height:40px!important}",
            "rect"
        ),
        (Dimension::Px(30.0), Dimension::Px(40.0))
    );
    assert_eq!(
        svg_sizes(
            "width:20!important;height:10!important",
            "rect{width:30px!important;height:40px!important}",
            "rect"
        ),
        (Dimension::Px(20.0), Dimension::Px(10.0))
    );
    assert_eq!(
        svg_sizes(
            "width:10px;width:20;height:5px;height:10",
            "rect{width:30px;height:40px}",
            "rect"
        ),
        (Dimension::Px(20.0), Dimension::Px(10.0))
    );
}
