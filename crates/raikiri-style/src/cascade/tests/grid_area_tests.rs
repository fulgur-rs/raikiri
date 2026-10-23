use super::*;
use crate::property::GridLineValue::{self, Auto, Line, Named};

#[test]
fn grid_area_shorthand_resets_all_four_longhands_when_values_are_omitted() {
    for (css, expected) in [
        (
            "a",
            [
                Named("a".into()),
                Named("a".into()),
                Named("a".into()),
                Named("a".into()),
            ],
        ),
        (
            "a / 2",
            [Named("a".into()), Line(2), Named("a".into()), Auto],
        ),
        (
            "1 / b / 3",
            [Line(1), Named("b".into()), Line(3), Named("b".into())],
        ),
        ("1 / 2 / 3 / 4", [Line(1), Line(2), Line(3), Line(4)]),
    ] {
        let values = cascade_doc(
            "p{grid-row-start:9;grid-column-start:8;grid-row-end:7;grid-column-end:6}",
            "p",
            Some(&format!("grid-area:{css}")),
        );
        let actual: [GridLineValue; 4] = [
            values.grid_row_start,
            values.grid_column_start,
            values.grid_row_end,
            values.grid_column_end,
        ];
        assert_eq!(actual, expected, "{css}");
    }
}

#[test]
fn grid_area_shorthand_respects_declaration_order_and_importance() {
    for (css, expected) in [
        (
            "grid-row-start:9;grid-area:1/2/3/4",
            [Line(1), Line(2), Line(3), Line(4)],
        ),
        (
            "grid-area:1/2/3/4;grid-row-start:9",
            [Line(9), Line(2), Line(3), Line(4)],
        ),
        (
            "grid-row-start:9!important;grid-area:a",
            [
                Line(9),
                Named("a".into()),
                Named("a".into()),
                Named("a".into()),
            ],
        ),
        (
            "grid-area:a!important;grid-row-start:9",
            [
                Named("a".into()),
                Named("a".into()),
                Named("a".into()),
                Named("a".into()),
            ],
        ),
    ] {
        let values = cascade_doc("", "p", Some(css));
        assert_eq!(
            [
                values.grid_row_start,
                values.grid_column_start,
                values.grid_row_end,
                values.grid_column_end
            ],
            expected,
            "{css}"
        );
    }
}

#[test]
fn grid_area_shorthand_variable_values_expand_into_longhand_candidates() {
    for (css, expected) in [
        (
            "--area:a / b / c;grid-area:var(--area)",
            [
                Named("a".into()),
                Named("b".into()),
                Named("c".into()),
                Named("b".into()),
            ],
        ),
        (
            "--area:1/2/3/4;grid-area:var(--area)",
            [Line(1), Line(2), Line(3), Line(4)],
        ),
        (
            "grid-area:var(--missing, a / b)",
            [
                Named("a".into()),
                Named("b".into()),
                Named("a".into()),
                Named("b".into()),
            ],
        ),
        (
            "grid-row-start:9;grid-area:var(--area);--area:a/2",
            [Named("a".into()), Line(2), Named("a".into()), Auto],
        ),
    ] {
        let values = cascade_doc("", "p", Some(css));
        assert_eq!(
            [
                values.grid_row_start,
                values.grid_column_start,
                values.grid_row_end,
                values.grid_column_end
            ],
            expected,
            "{css}"
        );
    }
}

#[test]
fn grid_area_shorthand_inherit_uses_all_four_parent_longhands() {
    let mut doc = TestDoc::new();
    let parent = doc.push_element(0, "div", Some("grid-area:1/2/3/4"));
    let child = doc.push_element(parent, "p", Some("grid-area:inherit"));
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).unwrap();
    let values = &result.computed[child];
    assert_eq!(
        [
            values.grid_row_start.clone(),
            values.grid_column_start.clone(),
            values.grid_row_end.clone(),
            values.grid_column_end.clone()
        ],
        [Line(1), Line(2), Line(3), Line(4)]
    );
}
