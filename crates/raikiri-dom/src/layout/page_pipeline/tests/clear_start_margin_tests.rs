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
        let (x, y, _, _) = crate::layout::test_support::absolute_rect(&doc, text);
        assert_eq!(
            (x, y),
            (10.0, expected_y),
            "nested={nested}, preceding={preceding_empty}"
        );
        assert_eq!(pages, 2);
    }
}

fn float_before_empty_clear(
    nested: bool,
    preceding_empty: bool,
    forced_break: bool,
) -> (Document, usize, usize) {
    use raikiri_style::{build_rule_tree, cascade};
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some(AHEM_BODY_STYLE));
    let before = doc.append_element(
        Some(body),
        "div",
        Some(if forced_break {
            "display:block;height:30px;break-after:page"
        } else {
            "display:block;height:30px"
        }),
    );
    doc.append_text(before, "a");
    doc.append_element(
        Some(body),
        "div",
        Some("display:block;float:left;width:10px;height:40px"),
    );
    let wrapper = doc.append_element(Some(body), "div", Some("display:block"));
    if preceding_empty {
        doc.append_element(Some(wrapper), "div", Some("display:block;height:0"));
    }
    let parent = if nested {
        doc.append_element(Some(wrapper), "div", Some("display:block"))
    } else {
        wrapper
    };
    doc.append_element(
        Some(parent),
        "div",
        Some("display:block;height:0;clear:both;margin-top:5px"),
    );
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
    let pages = if forced_break {
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
