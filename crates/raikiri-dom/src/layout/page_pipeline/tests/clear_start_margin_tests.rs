use super::*;

const AHEM_BODY_STYLE: &str =
    "display:block;margin:0;width:100px;font-family:Ahem;font-size:20px;line-height:30px";

#[test]
fn a_clear_empty_child_without_floats_keeps_the_collapsed_margin_after_a_forced_break() {
    for empty_styles in [
        vec!["clear:both;margin-top:10px"],
        vec!["margin-top:10px", "clear:both;margin-top:30px"],
    ] {
        let mut last = 0;
        let (doc, pages) = paged_body(|doc, body| {
            doc.append_element(
                Some(body),
                "div",
                Some("display:block;height:30px;break-after:page"),
            );
            let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
            for style in &empty_styles {
                doc.append_element(
                    Some(wrapper),
                    "div",
                    Some(format!("display:block;height:0;{style}")),
                );
            }
            last = doc.append_element(
                Some(wrapper),
                "div",
                Some("display:block;margin-top:50px;height:20px"),
            );
        });
        assert_eq!(page_flow_y(&doc, last), 150.0, "{empty_styles:?}");
        assert_eq!(pages, 2);
    }
}

#[test]
fn a_clear_empty_child_after_a_float_starts_beside_it_after_a_forced_break() {
    for (nested, preceding_empty, expected_y) in [
        (false, false, 100.0),
        (true, false, 100.0),
        (false, true, 100.0),
    ] {
        let (doc, pages, text) = float_before_empty_clear(nested, preceding_empty, true);
        let (x, y) = first_line_origin(&doc, text);
        assert_eq!(
            (x, y),
            (10.0, expected_y),
            "nested={nested}, preceding={preceding_empty}"
        );
        assert_eq!(pages, 2);
    }
}

struct FloatClearStyles<'a> {
    before: &'a str,
    float: &'a str,
    wrapper: &'a str,
    cleared: &'a str,
    prefix: Option<&'a str>,
    cleared_after: Option<&'a str>,
    outer: Option<&'a str>,
}

impl Default for FloatClearStyles<'_> {
    fn default() -> Self {
        Self {
            before: "break-after:page",
            float: "height:40px",
            wrapper: "",
            cleared: "height:0",
            prefix: None,
            cleared_after: None,
            outer: None,
        }
    }
}

fn float_before_empty_clear(
    nested: bool,
    preceding_empty: bool,
    forced_break: bool,
) -> (Document, usize, usize) {
    float_clear_fixture(
        usize::from(nested),
        preceding_empty,
        forced_break,
        FloatClearStyles {
            before: if forced_break { "break-after:page" } else { "" },
            ..Default::default()
        },
    )
}

fn float_clear_fixture(
    nested: usize,
    preceding_empty: bool,
    paged: bool,
    styles: FloatClearStyles<'_>,
) -> (Document, usize, usize) {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some(AHEM_BODY_STYLE));
    let flow_parent = if let Some(outer) = styles.outer {
        doc.append_element(Some(body), "section", Some(outer))
    } else {
        body
    };
    let before = doc.append_element(
        Some(flow_parent),
        "div",
        Some(format!("display:block;height:30px;{}", styles.before)),
    );
    doc.append_text(before, "a");
    doc.append_element(
        Some(flow_parent),
        "div",
        Some(format!(
            "display:block;float:left;width:10px;{}",
            styles.float
        )),
    );
    let wrapper = doc.append_element(
        Some(flow_parent),
        "div",
        Some(format!("display:block;{}", styles.wrapper)),
    );
    if let Some(prefix_style) = styles.prefix {
        let prefix = doc.append_element(Some(wrapper), "div", Some(prefix_style));
        doc.append_element(Some(prefix), "div", Some("display:block;height:0"));
    }
    if preceding_empty {
        doc.append_element(Some(wrapper), "div", Some("display:block;height:0"));
    }
    let mut parent = wrapper;
    for _ in 0..nested {
        parent = doc.append_element(Some(parent), "div", Some("display:block"));
    }
    doc.append_element(
        Some(parent),
        "div",
        Some(format!(
            "display:block;clear:both;margin-top:5px;{}",
            styles.cleared
        )),
    );
    if let Some(after_style) = styles.cleared_after {
        doc.append_element(Some(wrapper), "div", Some(after_style));
    }
    let last = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;height:20px;margin-top:10px"),
    );
    let text = doc.append_text(last, "X");
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    crate::layout::test_support::with_ahem(&mut doc);
    let mut page = raikiri_traits::PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let pages = if paged {
        layout_pages(&mut doc, &cascade, page).unwrap().len()
    } else {
        layout_single_page(&mut doc, &cascade, page).unwrap();
        1
    };
    (doc, pages, text)
}

#[test]
fn a_clear_empty_child_keeps_its_own_margin_strut_below_clearance() {
    let (doc, pages, text) = float_before_empty_clear(false, false, false);
    let (x, y, _, _) = crate::layout::test_support::absolute_rect(&doc, text);
    assert_eq!((x, y), (0.0, 75.0));
    assert_eq!(pages, 1);
}

#[test]
fn a_clear_empty_childs_trailing_margin_stays_inside_its_parent() {
    for (trailing, expected_height, expected_after) in
        [(None, 43.0, 83.0), (Some("margin-top:12px"), 47.0, 87.0)]
    {
        use raikiri_style::{build_rule_tree, cascade};
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Some("display:block"));
        let body = doc.append_element(Some(html), "body", Some(AHEM_BODY_STYLE));
        let before = doc.append_element(Some(body), "div", Some("display:block;height:30px"));
        doc.append_text(before, "a");
        doc.append_element(
            Some(body),
            "div",
            Some("display:block;float:left;width:10px;height:40px"),
        );
        let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
        let cleared = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;height:0;clear:both;margin-top:5px;margin-bottom:8px"),
        );
        if let Some(trailing) = trailing {
            doc.append_element(
                Some(wrapper),
                "div",
                Some(format!("display:block;height:0;{trailing}")),
            );
        }
        let after = doc.append_element(
            Some(body),
            "div",
            Some("display:block;margin-top:10px;height:20px"),
        );
        doc.append_text(after, "X");
        doc.mark_in_document_flags();
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        crate::layout::test_support::with_ahem(&mut doc);
        layout_single_page(
            &mut doc,
            &cascade,
            crate::layout::test_support::page_box_800x600(),
        )
        .unwrap();
        assert_eq!(page_flow_y(&doc, cleared), 70.0);
        assert_eq!(doc.nodes[wrapper].layout.size.height, expected_height);
        assert_eq!(page_flow_y(&doc, after), expected_after);
    }
}

fn first_line_origin(doc: &Document, text: usize) -> (f32, f32) {
    let parent = doc.layout_parent_of(text).unwrap();
    let lines = doc.nodes[parent]
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap();
    let line = &lines.lines[0];
    (
        crate::layout::test_support::absolute_rect(doc, parent).0
            + crate::layout::test_support::line_start_x(line).unwrap(),
        page_flow_y(doc, parent) + line.block_offset(),
    )
}

#[test]
fn resumption_requires_used_clearance() {
    for (float, cleared, expected) in [
        ("height:40px", "height:0;clear:right", (10.0, 110.0)),
        ("height:1px", "height:0", (0.0, 110.0)),
    ] {
        let (doc, _, text) = float_clear_fixture(
            0,
            false,
            true,
            FloatClearStyles {
                float,
                cleared,
                ..Default::default()
            },
        );
        assert_eq!(
            first_line_origin(&doc, text),
            expected,
            "{float}; {cleared}"
        );
    }
}

#[test]
fn completed_clearance_propagates_through_deep_empty_ancestors() {
    for depth in [3, 5] {
        let (doc, _, text) = float_clear_fixture(depth, false, true, Default::default());
        assert_eq!(
            first_line_origin(&doc, text),
            (10.0, 100.0),
            "depth={depth}"
        );
    }
}

#[test]
fn a_float_margin_is_not_a_fragment_boundary() {
    let (doc, _, text) = float_clear_fixture(
        0,
        false,
        true,
        FloatClearStyles {
            float: "height:40px;margin-top:10px",
            ..Default::default()
        },
    );
    assert_eq!(first_line_origin(&doc, text), (10.0, 100.0));
}

#[test]
fn positive_clear_box_extents_are_not_completed_at_the_previous_page_end() {
    for (cleared, y) in [
        ("height:1px", 151.0),
        ("height:0;padding-top:1px", 151.0),
        ("height:0;border-top:1px solid", 151.0),
        ("height:0;min-height:1px", 151.0),
    ] {
        let (doc, _, text) = float_clear_fixture(
            0,
            false,
            true,
            FloatClearStyles {
                cleared,
                ..Default::default()
            },
        );
        assert_eq!(first_line_origin(&doc, text), (0.0, y), "{cleared}");
    }
}

#[test]
fn a_nonempty_prefix_with_an_empty_child_keeps_clearance_after_the_break() {
    let (doc, _, text) = float_clear_fixture(
        0,
        false,
        true,
        FloatClearStyles {
            prefix: Some("display:block;height:20px"),
            ..Default::default()
        },
    );
    assert_eq!(first_line_origin(&doc, text), (0.0, 145.0));
}

#[test]
fn clearance_completion_uses_the_float_side_and_actual_height() {
    for (float, cleared, expected) in [
        ("height:0", "height:0", (0.0, 110.0)),
        ("height:4px", "height:0", (0.0, 110.0)),
        ("height:5px", "height:0", (0.0, 110.0)),
        ("height:6px", "height:0", (10.0, 100.0)),
        ("height:180px", "height:0", (10.0, 100.0)),
        ("height:250px", "height:0", (10.0, 100.0)),
        (
            "height:40px;float:right",
            "height:0;clear:left",
            (0.0, 110.0),
        ),
        (
            "height:40px;float:right",
            "height:0;clear:right",
            (0.0, 100.0),
        ),
        ("height:40px", "height:0;clear:left", (10.0, 100.0)),
    ] {
        let (doc, _, text) = float_clear_fixture(
            0,
            false,
            true,
            FloatClearStyles {
                float,
                cleared,
                ..Default::default()
            },
        );
        assert_eq!(first_line_origin(&doc, text), expected, "{float};{cleared}");
    }
}

#[test]
fn a_clearance_free_empty_prefix_does_not_hide_a_later_clearing_box() {
    let (doc, _, text) = float_clear_fixture(
        0,
        false,
        true,
        FloatClearStyles {
            prefix: Some("display:block;height:0;clear:right"),
            ..Default::default()
        },
    );
    assert_eq!(first_line_origin(&doc, text), (10.0, 100.0));
}

#[test]
fn consecutive_empty_clearing_boxes_complete_at_the_same_fragment_end() {
    for after in [
        "display:block;height:0;clear:both;margin-top:5px",
        "display:block;height:0;clear:both;margin-top:30px",
        "display:block;height:0;clear:both;margin-top:100px",
    ] {
        let (doc, _, text) = float_clear_fixture(
            0,
            false,
            true,
            FloatClearStyles {
                cleared_after: Some(after),
                ..Default::default()
            },
        );
        assert_eq!(first_line_origin(&doc, text), (10.0, 100.0), "{after}");
    }
}

#[test]
fn an_escaping_prefix_margin_controls_whether_clearance_can_resume_at_the_break() {
    let cases = [
        ("display:block;height:0;margin-top:5px", (10.0, 105.0)),
        ("display:block;height:0;margin-top:10px", (0.0, 145.0)),
        ("display:block;height:0;margin-top:25px", (0.0, 145.0)),
        ("display:block;height:0;margin-top:50px", (0.0, 150.0)),
    ];
    let actual: Vec<_> = cases
        .iter()
        .map(|(prefix, _)| {
            let (doc, _, text) = float_clear_fixture(
                0,
                false,
                true,
                FloatClearStyles {
                    prefix: Some(prefix),
                    ..Default::default()
                },
            );
            first_line_origin(&doc, text)
        })
        .collect();
    let expected: Vec<_> = cases.iter().map(|(_, expected)| *expected).collect();
    assert_eq!(actual, expected);
}

#[test]
fn resumption_keeps_the_inherited_relative_offset() {
    let (doc, _, text) = float_clear_fixture(
        0,
        false,
        true,
        FloatClearStyles {
            outer: Some("display:block;position:relative;top:17px"),
            ..Default::default()
        },
    );
    assert_eq!(first_line_origin(&doc, text), (10.0, 117.0));
}

#[test]
fn a_float_own_break_does_not_erase_a_preceding_normal_flow_break() {
    let (doc, _, text) = float_clear_fixture(
        0,
        false,
        true,
        FloatClearStyles {
            float: "height:40px;break-before:page",
            ..Default::default()
        },
    );
    assert_eq!(first_line_origin(&doc, text), (10.0, 100.0));
}

#[test]
fn resumption_uses_the_page_schedule_on_repeated_layout() {
    use raikiri_style::{build_rule_tree, cascade};
    let (mut doc, _, text) = float_clear_fixture(0, false, true, Default::default());
    for steps in [&[80.0, 140.0][..], &[120.0, 70.0][..]] {
        let rules = build_rule_tree(&doc);
        let cascade = cascade(&doc, &rules).unwrap();
        let mut page = raikiri_traits::PageBox::new();
        page.width = 100.0;
        page.height = 100.0;
        layout_pages_with_options(
            &mut doc,
            &cascade,
            page,
            PageLayoutOptions {
                page_steps: steps,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first_line_origin(&doc, text), (10.0, steps[0]));
    }
}

#[test]
fn resumption_keeps_a_named_page_in_the_same_flow() {
    use raikiri_style::{build_rule_tree, cascade};
    let (mut doc, _, text) = float_clear_fixture(0, false, true, Default::default());
    let body = doc.nodes[doc.root]
        .children
        .iter()
        .find_map(|&html| doc.nodes[html].children.first().copied())
        .unwrap();
    doc.set_element_inline_style(body, Some(format!("{AHEM_BODY_STYLE};page:content").into()));
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let pages = layout_pages(&mut doc, &cascade, page).unwrap();
    assert_eq!(first_line_origin(&doc, text), (10.0, 100.0));
    assert!(
        pages
            .iter()
            .all(|page| page.page_name.as_deref() == Some("content"))
    );
}

#[test]
fn several_forced_float_resumptions_keep_separate_boundaries() {
    use raikiri_style::{build_rule_tree, cascade};
    let (mut doc, _, first) = float_clear_fixture(0, false, true, Default::default());
    let body = doc.nodes[doc.root]
        .children
        .iter()
        .find_map(|&html| doc.nodes[html].children.first().copied())
        .unwrap();
    let mut texts = vec![first];
    for _ in 0..2 {
        let before = doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:30px;break-after:page"),
        );
        doc.append_text(before, "a");
        doc.append_element(
            Some(body),
            "div",
            Some("display:block;float:left;width:10px;height:40px"),
        );
        let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
        doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;height:0;clear:both;margin-top:5px"),
        );
        let last = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;height:20px;margin-top:10px"),
        );
        texts.push(doc.append_text(last, "X"));
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = raikiri_traits::PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let pages = layout_pages(&mut doc, &cascade, page).unwrap();
    let origins: Vec<_> = texts
        .iter()
        .map(|&text| first_line_origin(&doc, text))
        .collect();
    assert_eq!(origins, vec![(10.0, 100.0), (10.0, 200.0), (10.0, 300.0)]);
    assert_eq!(pages.len(), 4);
}

#[test]
fn completed_clearance_consumes_zero_and_negative_own_margins() {
    for cleared in ["height:0;margin-top:0", "height:0;margin-top:-5px"] {
        let (doc, _, text) = float_clear_fixture(
            0,
            false,
            true,
            FloatClearStyles {
                cleared,
                ..Default::default()
            },
        );
        assert_eq!(first_line_origin(&doc, text), (10.0, 100.0), "{cleared}");
    }
}
