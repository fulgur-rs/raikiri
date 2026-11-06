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
    before_text_with("", value, content)
}

/// `before_text` with the style rules `style` added.
fn before_text_with(style: &str, value: i32, content: &str) -> String {
    generated_text(
        &format!("{style} #t {{ counter-reset: c {value} }} #t::before {{ content: {content} }}"),
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

/// The marker text of one list item with `value` for the `ol`'s
/// `list-style-type: x`, with the style rules `style`.
fn marker_at(style: &str, value: i32) -> String {
    generated_text(
        style,
        &format!("<ol style='list-style-type: x'><li value={value}>x</li></ol>"),
        GeneratedKind::Marker,
    )
}

/// The marker texts of the values 1 to 5 of the author style `x` in `style`.
fn first_five_markers(style: &str) -> Vec<String> {
    (1..=5).map(|value| marker_at(style, value)).collect()
}

#[test]
fn a_disclosure_marker_is_a_triangle_and_a_space() {
    // The marker texts Chromium shows, in every direction, position and
    // spelling of the name. The marker of a `<summary>` and the markers in a
    // vertical writing mode have no text runs, so they are not checked here.
    for (style, expected) in [
        ("list-style-type: disclosure-open", "\u{25be} "),
        ("list-style-type: disclosure-closed", "\u{25b8} "),
        ("list-style-type: Disclosure-Closed", "\u{25b8} "),
        (
            "list-style-type: disclosure-open; list-style-position: inside",
            "\u{25be} ",
        ),
        (
            "direction: rtl; list-style-type: disclosure-closed",
            "\u{25b8} ",
        ),
        (
            "direction: rtl; list-style-type: disclosure-open; list-style-position: inside",
            "\u{25be} ",
        ),
    ] {
        let text = generated_text(
            "",
            &format!("<ul style='{style}'><li>x</li></ul>"),
            GeneratedKind::Marker,
        );
        assert_eq!(text, expected, "{style}");
    }
    // Every item shows the triangle, not its number.
    let items = generated_text(
        "",
        "<ol style='list-style-type: disclosure-open'><li>1</li><li>2</li><li>3</li></ol>",
        GeneratedKind::Marker,
    );
    assert_eq!(items, "\u{25be} ".repeat(3));
}

#[test]
fn counter_shows_a_disclosure_triangle_without_a_space() {
    for (value, style, expected) in [
        (1, "disclosure-open", "\u{25be}"),
        (5, "disclosure-closed", "\u{25b8}"),
        (0, "disclosure-closed", "\u{25b8}"),
        (-3, "disclosure-open", "\u{25be}"),
        (5, "Disclosure-Open", "\u{25be}"),
    ] {
        assert_eq!(
            before_text(value, &format!("counter(c, {style})")),
            expected,
            "{style} at {value}"
        );
    }
    assert_eq!(
        before_text(5, "counters(c, '.', disclosure-closed)"),
        "\u{25b8}"
    );
}

#[test]
fn a_value_outside_the_range_of_an_author_style_is_shown_by_its_predefined_fallback() {
    // `x` shows 1 and 2 as a star; the other values go to `fallback`. The
    // marker keeps the prefix and suffix of `x`, whatever style shows the
    // value (the texts Chromium shows).
    let x = |fallback: &str| {
        format!(
            "@counter-style x {{ system: cyclic; symbols: '\u{2605}'; range: 1 2; \
             fallback: {fallback} }}"
        )
    };
    for (fallback, expected) in [
        (
            "lower-roman",
            ["\u{2605}. ", "\u{2605}. ", "iii. ", "iv. ", "v. "],
        ),
        (
            "upper-alpha",
            ["\u{2605}. ", "\u{2605}. ", "C. ", "D. ", "E. "],
        ),
        (
            "decimal-leading-zero",
            ["\u{2605}. ", "\u{2605}. ", "03. ", "04. ", "05. "],
        ),
        (
            "cjk-decimal",
            [
                "\u{2605}. ",
                "\u{2605}. ",
                "\u{4e09}. ",
                "\u{56db}. ",
                "\u{4e94}. ",
            ],
        ),
        (
            "hiragana",
            [
                "\u{2605}. ",
                "\u{2605}. ",
                "\u{3046}. ",
                "\u{3048}. ",
                "\u{304a}. ",
            ],
        ),
        (
            "disc",
            [
                "\u{2605}. ",
                "\u{2605}. ",
                "\u{2022}. ",
                "\u{2022}. ",
                "\u{2022}. ",
            ],
        ),
        (
            "disclosure-open",
            [
                "\u{2605}. ",
                "\u{2605}. ",
                "\u{25be}. ",
                "\u{25be}. ",
                "\u{25be}. ",
            ],
        ),
        // The predefined name matches without regard to case.
        (
            "Lower-Roman",
            ["\u{2605}. ", "\u{2605}. ", "iii. ", "iv. ", "v. "],
        ),
        // A name no style defines, and the default `fallback`, are decimal.
        (
            "nonexistent",
            ["\u{2605}. ", "\u{2605}. ", "3. ", "4. ", "5. "],
        ),
        ("decimal", ["\u{2605}. ", "\u{2605}. ", "3. ", "4. ", "5. "]),
    ] {
        assert_eq!(first_five_markers(&x(fallback)), expected, "{fallback}");
    }
    assert_eq!(
        first_five_markers("@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 2 }"),
        ["\u{2605}. ", "\u{2605}. ", "3. ", "4. ", "5. "]
    );
}

#[test]
fn a_fallback_chain_ends_at_a_predefined_style() {
    // `x` shows 1 and 2, `y` shows 3 only, and `y` falls back to lower-roman.
    let chain = "@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 2; fallback: y } \
                 @counter-style y { system: cyclic; symbols: '\u{2666}'; range: 3 3; \
                 fallback: lower-roman }";
    assert_eq!(
        first_five_markers(chain),
        ["\u{2605}. ", "\u{2605}. ", "\u{2666}. ", "iv. ", "v. "]
    );
    // A loop in the chain is decimal, as it is when the chain ends nowhere.
    let cycle = "@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 2; fallback: y } \
                 @counter-style y { system: cyclic; symbols: '\u{2666}'; range: 1 2; fallback: x }";
    assert_eq!(
        first_five_markers(cycle),
        ["\u{2605}. ", "\u{2605}. ", "3. ", "4. ", "5. "]
    );
    // An author style of a predefined name is the one the chain follows.
    let redefined = "@counter-style lower-roman { system: cyclic; symbols: 'R' } \
                     @counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 2; \
                     fallback: lower-roman }";
    assert_eq!(
        first_five_markers(redefined),
        ["\u{2605}. ", "\u{2605}. ", "R. ", "R. ", "R. "]
    );
    // It is found by any spelling of the predefined name (Chromium).
    assert_eq!(
        first_five_markers(&redefined.replace("fallback: lower-roman", "fallback: Lower-Roman")),
        ["\u{2605}. ", "\u{2605}. ", "R. ", "R. ", "R. "]
    );
}

#[test]
fn a_predefined_fallback_keeps_the_prefix_and_suffix_of_the_requested_style() {
    let style = "@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 2; \
                 prefix: '<'; suffix: '> '; fallback: lower-roman }";
    assert_eq!(
        first_five_markers(style),
        ["<\u{2605}> ", "<\u{2605}> ", "<iii> ", "<iv> ", "<v> "]
    );
}

#[test]
fn a_predefined_fallback_applies_its_own_range() {
    // The predefined style cannot show the value either: it uses decimal.
    let alpha = "@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 1; \
                 fallback: lower-alpha }";
    assert_eq!(
        [0, -3, 27].map(|value| marker_at(alpha, value)),
        ["0. ", "-3. ", "aa. "]
    );
    let roman = "@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 1; \
                 fallback: lower-roman }";
    assert_eq!(
        [4000, 0, 12].map(|value| marker_at(roman, value)),
        ["4000. ", "0. ", "xii. "]
    );
    // An additive style that cannot write the value falls back to the
    // predefined style, and that one writes it.
    let additive = "@counter-style x { system: additive; additive-symbols: 2 'B', 1 'A'; \
                    range: 1 3; fallback: upper-roman }";
    assert_eq!(
        first_five_markers(additive),
        ["A. ", "B. ", "BA. ", "IV. ", "V. "]
    );
    // A style with no symbol for zero hands zero to the predefined style.
    let alphabetic = "@counter-style x { system: alphabetic; symbols: 'a' 'b'; \
                      fallback: cjk-decimal }";
    assert_eq!(
        [0, 1, 3].map(|value| marker_at(alphabetic, value)),
        ["\u{3007}. ", "a. ", "aa. "]
    );
}

#[test]
fn counter_shows_a_value_the_author_style_cannot_show_in_the_predefined_fallback() {
    // `counter()` and `counters()` have no prefix and no suffix.
    let text = |style: &str, content: &str, value: i32| {
        before_text_with(
            &format!(
                "@counter-style x {{ system: cyclic; symbols: '\u{2605}'; range: 1 2; {style} }}"
            ),
            value,
            content,
        )
    };
    assert_eq!(text("fallback: lower-roman", "counter(c, x)", 4), "iv");
    assert_eq!(
        text("fallback: lower-roman", "counter(c, x)", 2),
        "\u{2605}"
    );
    assert_eq!(
        text(
            "prefix: '<'; suffix: '>'; fallback: upper-alpha",
            "counter(c, x)",
            4
        ),
        "D"
    );
    assert_eq!(
        text("fallback: disclosure-closed", "counter(c, x)", 4),
        "\u{25b8}"
    );
    assert_eq!(text("fallback: nonexistent", "counter(c, x)", 4), "4");
    let nested = generated_text(
        "@counter-style x { system: cyclic; symbols: '\u{2605}'; range: 1 2; \
         fallback: lower-roman } \
         #t { counter-reset: c 1 } #t div { counter-reset: c 4 } \
         #t div::before { content: counters(c, '-', x) }",
        "<div id=t><div></div></div>",
        GeneratedKind::Before,
    );
    assert_eq!(nested, "\u{2605}-iv");
}
