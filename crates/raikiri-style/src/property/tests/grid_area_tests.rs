use super::*;
use GridLineValue::{Auto, Line, Named, NamedLine, Span, SpanNamed};

fn area_edges(area: &GridAreaShorthand) -> [GridLineValue; 4] {
    [
        area.row_start.clone(),
        area.column_start.clone(),
        area.row_end.clone(),
        area.column_end.clone(),
    ]
}

#[test]
fn grid_area_shorthand_omitted_edges_follow_the_corresponding_start() {
    let name = |s: &str| Named(s.into());
    for (css, expected) in [
        (
            "header",
            [
                name("header"),
                name("header"),
                name("header"),
                name("header"),
            ],
        ),
        ("a / b", [name("a"), name("b"), name("a"), name("b")]),
        ("a / b / c", [name("a"), name("b"), name("c"), name("b")]),
        ("1 / 2 / 3 / 4", [Line(1), Line(2), Line(3), Line(4)]),
        ("1", [Line(1), Auto, Auto, Auto]),
        ("1 / b", [Line(1), name("b"), Auto, name("b")]),
        ("a / 2", [name("a"), Line(2), name("a"), Auto]),
        ("auto / b / span 3", [Auto, name("b"), Span(3), name("b")]),
        ("a / auto", [name("a"), Auto, name("a"), Auto]),
        ("a / b / auto", [name("a"), name("b"), Auto, name("b")]),
        ("auto", [Auto, Auto, Auto, Auto]),
        ("1 a", [NamedLine("a".into(), 1), Auto, Auto, Auto]),
        ("a 1", [NamedLine("a".into(), 1), Auto, Auto, Auto]),
        ("span a", [SpanNamed("a".into(), 1), Auto, Auto, Auto]),
        ("span 2", [Span(2), Auto, Auto, Auto]),
        (
            "a / span b",
            [name("a"), SpanNamed("b".into(), 1), name("a"), Auto],
        ),
        (
            "a / b 1 / 3",
            [name("a"), NamedLine("b".into(), 1), Line(3), Auto],
        ),
        (
            "a / b / span 2 / c",
            [name("a"), name("b"), Span(2), name("c")],
        ),
        (
            r"h\65 ader/**/ / Other",
            [name("header"), name("Other"), name("header"), name("Other")],
        ),
    ] {
        let Some(PropertyValue::GridArea(area)) = parse_entire(css, "grid-area") else {
            panic!("valid grid-area rejected: {css}");
        };
        assert_eq!(area_edges(&area), expected, "{css}");
    }
}

#[test]
fn grid_area_shorthand_rejects_missing_components_and_invalid_lines() {
    for css in [
        "",
        "/",
        "a /",
        "a / b /",
        "a / b / c /",
        "a // b",
        "a / / c",
        "a / b / / d",
        "a / b / c / d / e",
        "a / b / c / d !",
        "0",
        "a / 0",
        "a / b / 0",
        "a / b / c / 0",
        "span",
        "span 0",
        "a / span",
        "a / b / span",
        "a / b / c / span",
    ] {
        assert!(
            parse_entire(css, "grid-area").is_none(),
            "invalid grid-area accepted: {css}"
        );
    }
}

#[test]
fn grid_area_shorthand_invalid_suffix_drops_the_whole_declaration() {
    for css in [
        "a /",
        "a / b / c / d / e",
        "a / b / c / d !",
        "a / b / c junk",
    ] {
        let source = format!("grid-row-start:7; grid-area:{css}; color:red");
        let mut input = ParserInput::new(&source);
        let mut parser = Parser::new(&mut input);
        let declarations = crate::rule::parse_declaration_block(&mut parser);
        assert_eq!(declarations.len(), 2, "{css}");
        assert_eq!(declarations[0].value, PropertyValue::GridRowStart(Line(7)));
        assert_eq!(declarations[1].value.key(), PropertyKey::Color);
    }
}

#[test]
fn grid_area_shorthand_preserves_css_wide_defaulting_markers() {
    assert_css_wide_keywords_deferred("grid-area");
}
