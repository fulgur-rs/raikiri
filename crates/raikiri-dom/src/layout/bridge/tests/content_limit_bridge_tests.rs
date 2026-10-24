use super::*;
use crate::layout::dispatch::content_limits::ContentLimits;
use crate::layout::test_support::bridged_style;
use raikiri_layout_style::values::StyleSize;
use raikiri_style::{build_rule_tree, cascade};
use taffy::{Dimension, LengthPercentageAuto};

#[test]
fn bridge_retains_raw_content_limits_for_table_parts_and_replaced_nodes() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let table = doc.append_element(
        Some(body),
        "table",
        Some(
            "display:table;min-width:min-content;max-width:max-content;\
             min-height:fit-content;max-height:min-content",
        ),
    );
    let caption = doc.append_element(
        Some(table),
        "caption",
        Some(
            "display:table-caption;min-width:max-content;max-width:min-content;\
             min-height:min-content;max-height:max-content",
        ),
    );
    let row = doc.append_element(Some(table), "tr", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "td",
        Some(
            "display:table-cell;min-width:fit-content;max-width:fit-content(17px);\
             min-height:max-content;max-height:fit-content",
        ),
    );
    let canvas = doc.append_element(
        Some(body),
        "canvas",
        Some(
            "display:inline;min-width:min-content;max-width:fit-content;\
             min-height:max-content;max-height:fit-content(13px)",
        ),
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade succeeds");

    apply_computed_to_style(&mut doc, &cascade).expect("bridge succeeds");

    let expected = [
        (
            table,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::min_content(),
                    height: Dimension::fit_content(),
                },
                max: StyleSize {
                    width: Dimension::max_content(),
                    height: Dimension::min_content(),
                },
            },
        ),
        (
            caption,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::max_content(),
                    height: Dimension::min_content(),
                },
                max: StyleSize {
                    width: Dimension::min_content(),
                    height: Dimension::max_content(),
                },
            },
        ),
        (
            cell,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::fit_content(),
                    height: Dimension::max_content(),
                },
                max: StyleSize {
                    width: Dimension::fit_content_px(17.0),
                    height: Dimension::fit_content(),
                },
            },
        ),
        (
            canvas,
            ContentLimits {
                min: StyleSize {
                    width: Dimension::min_content(),
                    height: Dimension::max_content(),
                },
                max: StyleSize {
                    width: Dimension::fit_content(),
                    height: Dimension::fit_content_px(13.0),
                },
            },
        ),
    ];

    for (node, limits) in expected {
        assert_eq!(doc.nodes[node].content_limits, limits);
        let style = &bridged_style(&mut doc, &cascade, node);
        assert_eq!(
            style.min_size,
            StyleSize::<LengthPercentageAuto>::auto(),
            "Taffy min_size for node {node}"
        );
        assert_eq!(
            style.max_size,
            StyleSize::<LengthPercentageAuto>::auto(),
            "Taffy max_size for node {node}"
        );
    }
}
