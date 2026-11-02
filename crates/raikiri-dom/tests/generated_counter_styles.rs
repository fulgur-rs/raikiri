//! `counter()` and `counters()` in generated content show a value in the
//! counter style they name, whether it is a predefined style of CSS Counter
//! Styles 3 or an `@counter-style` of the author. The expected texts are the
//! ones Chromium shows for the same markup.

use raikiri_html::{
    FontCollectionBuilder, GeneratedKind, LayoutOptions, LayoutStatus, RenderResources, RunSource,
    layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The text the generated content of the given `kind` shows on the first
/// page of `body` with the style rules `style`.
fn generated_text(style: &str, body: &str, kind: GeneratedKind) -> String {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} {style}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    page.text_runs()
        .iter()
        .filter(|run| matches!(run.source, RunSource::Generated(_, found) if found == kind))
        .map(|run| run.text)
        .collect()
}

/// The text of the `::before` of `#t` when it shows `content` and the counter
/// `c` is at `value`.
fn before_text(value: i32, content: &str) -> String {
    generated_text(
        &format!("#t {{ counter-reset: c {value} }} #t::before {{ content: {content} }}"),
        "<div id=t></div>",
        GeneratedKind::Before,
    )
}

#[test]
fn counter_shows_the_value_in_the_predefined_style() {
    for (value, content, expected) in [
        (5, "counter(c)", "5"),
        (5, "counter(c, decimal-leading-zero)", "05"),
        (12, "counter(c, decimal-leading-zero)", "12"),
        (123, "counter(c, decimal-leading-zero)", "123"),
        (-3, "counter(c, decimal-leading-zero)", "-3"),
        (5, "counter(c, lower-alpha)", "e"),
        (27, "counter(c, lower-alpha)", "aa"),
        (703, "counter(c, upper-latin)", "AAA"),
        (12, "counter(c, lower-roman)", "xii"),
        (1999, "counter(c, upper-roman)", "MCMXCIX"),
        (4000, "counter(c, upper-roman)", "4000"),
        (0, "counter(c, lower-roman)", "0"),
        (5, "counter(c, Decimal-Leading-Zero)", "05"),
        (5, "counter(c, no-such-style)", "5"),
    ] {
        assert_eq!(
            before_text(value, content),
            expected,
            "{content} at {value}"
        );
    }
}

#[test]
fn counter_shows_the_value_in_the_predefined_style_of_other_scripts() {
    for (value, style, expected) in [
        (5, "disc", "\u{2022}"),
        (5, "circle", "\u{25e6}"),
        (5, "cjk-decimal", "\u{4e94}"),
        (12, "cjk-decimal", "\u{4e00}\u{4e8c}"),
        (5, "lower-greek", "\u{3b5}"),
        (5, "hiragana", "\u{304a}"),
        (5, "thai", "\u{e55}"),
        (5, "arabic-indic", "\u{665}"),
        (13, "cjk-earthly-branch", "\u{4e00}\u{4e09}"),
        (12, "japanese-informal", "\u{5341}\u{4e8c}"),
    ] {
        assert_eq!(
            before_text(value, &format!("counter(c, {style})")),
            expected,
            "{style} at {value}"
        );
    }
}

#[test]
fn counters_shows_every_value_in_the_predefined_style() {
    let nested = |content: &str| {
        generated_text(
            &format!(
                "div {{ counter-reset: c 2 }} #t {{ counter-reset: c 5 }} \
                 #t::before {{ content: {content} }}"
            ),
            "<div><div id=t></div></div>",
            GeneratedKind::Before,
        )
    };
    assert_eq!(nested("counters(c, '.')"), "2.5");
    assert_eq!(nested("counters(c, '.', decimal-leading-zero)"), "02.05");
    assert_eq!(nested("counters(c, '-', lower-alpha)"), "b-e");
    assert_eq!(nested("counters(c, '', upper-roman)"), "IIV");
    assert_eq!(nested("counters(c, '.', cjk-decimal)"), "\u{4e8c}.\u{4e94}");
}

#[test]
fn an_author_counter_style_wins_over_the_predefined_style_of_its_name() {
    // A predefined name matches without regard to case, the name of an author
    // style exactly (WPT name-case-sensitivity).
    let text = |content: &str| {
        generated_text(
            &format!(
                "@counter-style lower-alpha {{ system: cyclic; symbols: '*'; }} \
                 @counter-style custom-style {{ system: cyclic; symbols: '#'; }} \
                 #t {{ counter-reset: c 5 }} #t::before {{ content: {content} }}"
            ),
            "<div id=t></div>",
            GeneratedKind::Before,
        )
    };
    assert_eq!(text("counter(c, lower-alpha)"), "*");
    assert_eq!(text("counter(c, Lower-Alpha)"), "*");
    assert_eq!(text("counter(c, upper-alpha)"), "E");
    assert_eq!(text("counter(c, custom-style)"), "#");
    assert_eq!(text("counter(c, Custom-Style)"), "5");
}

#[test]
fn a_marker_shows_a_predefined_style_in_its_counters() {
    // css-pseudo marker-content-005: the item counter, then another counter in
    // a predefined style, which runs on from it without a separator.
    let text = generated_text(
        "li { counter-increment: foo 5 } \
         li::marker { content: counters(list-item, '.') counter(foo, decimal-leading-zero) }",
        "<ol><li id=t>B</li></ol>",
        GeneratedKind::Marker,
    );
    assert_eq!(text, "105");
}

#[test]
fn a_list_marker_shows_the_predefined_style_with_its_suffix() {
    // The marker texts Chromium shows for the fifth item of a list.
    for (style, expected) in [
        ("decimal", "5. "),
        ("lower-greek", "\u{3b5}. "),
        ("thai", "\u{e55}. "),
        ("cjk-decimal", "\u{4e94}\u{3001}"),
        ("hiragana", "\u{304a}\u{3001}"),
        ("cjk-earthly-branch", "\u{8fb0}\u{3001}"),
        ("korean-hangul-formal", "\u{c624}, "),
        ("ethiopic-numeric", "\u{136d}/ "),
    ] {
        let text = generated_text(
            "",
            &format!("<ol start=5><li style='list-style: {style} inside'>x</li></ol>"),
            GeneratedKind::Marker,
        );
        assert_eq!(text, expected, "{style}");
    }
}
