use super::*;

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
        let mut text = 0;
        let (doc, pages) = paged_body_with_ifc(|doc, body| {
            doc.set_element_inline_style(body, Some("display:block;margin:0;width:100px;font-family:Ahem;font-size:20px;line-height:30px".into()));
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
            text = doc.append_text(last, "X");
        });
        let (x, y, _, _) = crate::layout::test_support::absolute_rect(&doc, text);
        assert_eq!(
            (x, y),
            (10.0, expected_y),
            "nested={nested}, preceding={preceding_empty}"
        );
        assert_eq!(pages, 2);
    }
}
