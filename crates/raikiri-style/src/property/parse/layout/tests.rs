use super::*;
use crate::grid_area_diagnostics as diagnostics;

fn build(strings: &[&str]) -> Option<GridTemplateAreas> {
    let rows = strings
        .iter()
        .map(|s| tokenize_grid_area_row(s))
        .collect::<Option<Vec<_>>>()?;
    build_grid_template_areas(rows, strings.iter().map(SmolStr::new).collect())
}

#[test]
fn named_area_lookup_work_is_bounded_by_cells() {
    for n in [1000, 2000, 4000] {
        let row = (0..n)
            .map(|i| format!("a{i:06}"))
            .collect::<Vec<_>>()
            .join(" ");
        diagnostics::begin();
        let result = build(&[&row]).unwrap();
        let work = diagnostics::finish();
        assert_eq!(result.areas.len(), n);
        assert!(
            work.linear_comparisons + work.indexed_lookups + work.indexed_insertions
                <= 8 * n as u64,
            "n={n}, work={work:?}"
        );
        assert_eq!(work.rectangle_cells, n as u64);
    }
}

#[test]
fn index_transition_preserves_first_seen_order_and_existing_bounds() {
    let names = ["z", "A", "a", "é", "y", "x", "w", "v", "u", "t"];
    let row = names.join(" ");
    for n in [1, 4, 8, 9, 10] {
        let row = names[..n].join(" ");
        let result = build(&[&row, &row]).unwrap();
        assert_eq!(result.row_strings, vec![SmolStr::new(&row); 2]);
        for (column, (area, name)) in result.areas.iter().zip(&names).enumerate() {
            assert_eq!(area.name, *name);
            assert_eq!((area.row_start, area.row_end), (1, 3));
            assert_eq!(
                (area.column_start, area.column_end),
                (column as u32 + 1, column as u32 + 2)
            );
        }
        assert_eq!(result.areas.len(), n);
    }
    assert_eq!(build(&[&row]).unwrap().areas.len(), names.len());
}

#[test]
fn large_index_rejects_holes_disconnected_names_and_uneven_rows() {
    let row = "z y x w v u t s r";
    for second in ["z y x w v u t s .", "z y x w v u t s r", "z y x w v u t s"] {
        let expected = second.split_whitespace().count() == 9;
        assert_eq!(build(&[row, second]).is_some(), expected);
    }
    assert!(build(&["z y x w v u t s r", "z y x w v u t r s"]).is_none());
    assert!(
        build(&[
            "z y x w v u t s r",
            "z y x w v u t . r",
            "z y x w v u t s r"
        ])
        .is_none()
    );
    assert!(build(&["..."]).unwrap().areas.is_empty());
    assert!(build(&[""]).is_none());
}

#[test]
fn escaped_unicode_names_and_shorthand_share_the_builder() {
    let text = r#""z A a é y x w v \75 " "z A a \e9  y x w v u""#;
    let mut source = cssparser::ParserInput::new(text);
    let longhand = parse_grid_template_areas(&mut Parser::new(&mut source)).unwrap();
    let mut source = cssparser::ParserInput::new(text);
    let shorthand = parse_grid_template_shorthand(&mut Parser::new(&mut source)).unwrap();
    assert_eq!(longhand, shorthand.areas);
    let GridTemplateAreasValue::Areas(areas) = longhand else {
        panic!("expected areas")
    };
    assert_eq!(areas.areas.len(), 9);
    assert_eq!(areas.areas[3].name, "é");
    assert_eq!(areas.areas[8].name, "u");
    assert!(areas.areas.iter().all(|a| a.row_end == 3));
}

#[test]
fn diagnostics_are_optional_resettable_and_cover_small_controls() {
    assert_eq!(diagnostics::finish(), Default::default());
    build(&["a a a a"]).unwrap();
    assert_eq!(diagnostics::finish(), Default::default());
    diagnostics::begin();
    build(&["a a a a"]).unwrap();
    let work = diagnostics::finish();
    assert_eq!(work.linear_comparisons, 3);
    assert_eq!(work.indexed_lookups + work.indexed_insertions, 0);
    assert_eq!(work.rectangle_cells, 4);
    diagnostics::begin();
    diagnostics::begin();
    assert_eq!(diagnostics::finish(), Default::default());
    diagnostics::begin();
    assert!(build(&["a a", "a ."]).is_none());
    assert!(diagnostics::finish().rectangle_cells > 0);
}
