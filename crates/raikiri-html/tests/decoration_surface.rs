//! Used text decoration geometry available to downstream PDF painters.

use raikiri_html::computed::CssColor;
use raikiri_html::{
    DecorationKind, DecorationStyle, DocumentLayout, FontCollectionBuilder, LayoutConfig,
    LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout, parse_html_with_resources,
};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn document(body: &str, css: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .system_fonts(false)
        .font_bytes("Ahem", AHEM.to_vec())
        .font_bytes(
            "Noto Sans Test",
            include_bytes!("../../raikiri-dom/tests/data/noto-sans-test/NotoSansTest-Regular.ttf")
                .to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page {{size:200px 200px;margin:0}} \
         body {{margin:0;font:20px/20px Ahem}} p {{margin:0}} {css}</style>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout aborted")
    };
    result
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

#[test]
fn all_decoration_kinds_and_styles_are_exposed() {
    for (css, style) in [
        ("solid", DecorationStyle::Solid),
        ("double", DecorationStyle::Double),
        ("dotted", DecorationStyle::Dotted),
        ("dashed", DecorationStyle::Dashed),
        ("wavy", DecorationStyle::Wavy),
    ] {
        let doc = document(
            "<p id='origin'>ab</p>",
            &format!(
                "p {{text-decoration-line:underline overline line-through; \
                     text-decoration-style:{css};text-decoration-color:red}}"
            ),
        );
        let page = doc.page(0).unwrap();
        let runs = page.text_runs();
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.text, "ab");
        assert_eq!(run.decorations.len(), 3);
        for (line, kind, delta) in [
            (&run.decorations[0], DecorationKind::Underline, 2.0),
            (&run.decorations[1], DecorationKind::Overline, -15.375),
            (&run.decorations[2], DecorationKind::LineThrough, -5.6),
        ] {
            assert_eq!(line.kind, kind);
            assert_eq!(line.style, style);
            assert_eq!(
                line.color,
                CssColor {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255
                }
            );
            assert_eq!(page.dom().attr(line.origin, "id"), Some("origin"));
            close(line.x_start, 0.0);
            close(line.x_end, 40.0);
            close(line.pattern_origin_x, 0.0);
            close(line.thickness, 1.25);
            close(line.y - run.origin.1, delta);
        }
    }
}

#[test]
fn ancestor_decoration_keeps_origin_metrics_and_color() {
    let doc = document(
        "<p id='origin'>a<span style='font-size:10px;color:blue;text-decoration:none'>b</span>c</p>",
        "p {text-decoration:underline red}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(
        runs.iter().map(|r| r.text).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    let y = runs[0].decorations[0].y;
    for run in &runs {
        let line = &run.decorations[0];
        assert_eq!(page.dom().attr(line.origin, "id"), Some("origin"));
        assert_eq!(line.color.r, 255);
        assert_eq!(line.color.b, 0);
        close(line.thickness, 1.25);
        close(line.y, y);
    }
}

#[test]
fn direct_flex_and_grid_text_has_one_ancestor_decoration() {
    for display in ["flex", "grid"] {
        let doc = document(
            "<p id='origin'>ab</p>",
            &format!("p {{display:{display};text-decoration:underline rgba(255,0,0,0.5)}}"),
        );
        let page = doc.page(0).unwrap();
        let runs = page.text_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[0].decorations.len(), 1, "{display}");
        assert_eq!(
            page.dom().attr(runs[0].decorations[0].origin, "id"),
            Some("origin")
        );
        assert_eq!(runs[0].decorations[0].color.a, 128);
    }
}

#[test]
fn atomic_and_out_of_flow_boxes_stop_propagation() {
    for css in ["display:inline-block", "float:left", "position:absolute"] {
        let doc = document(
            &format!("<p id='outer'>a<span style='{css}'>b</span>c</p>"),
            "p {text-decoration:underline}",
        );
        let runs = doc.page(0).unwrap().text_runs();
        assert!(runs.iter().any(|r| r.text == "b"));
        for run in runs {
            assert_eq!(
                run.decorations.is_empty(),
                run.text == "b",
                "{css}: {}",
                run.text
            );
        }
    }
}

#[test]
fn display_contents_passes_ancestor_decoration() {
    let doc = document(
        "<p id='origin'><span style='display:contents;text-decoration:overline blue'>ab</span></p>",
        "p {text-decoration:underline red}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs[0].decorations.len(), 1);
    assert_eq!(runs[0].decorations[0].kind, DecorationKind::Underline);
    assert_eq!(
        page.dom().attr(runs[0].decorations[0].origin, "id"),
        Some("origin")
    );
}

#[test]
fn split_runs_keep_the_origin_of_the_unsplit_pattern() {
    let doc = document(
        "<p>ab<span style='color:blue'>cd</span>ef</p>",
        "p {text-decoration:underline;text-decoration-style:dashed}",
    );
    let runs = doc.page(0).unwrap().text_runs();
    assert_eq!(runs.len(), 3);
    for (run, x) in runs.iter().zip([0.0, 40.0, 80.0]) {
        let line = &run.decorations[0];
        close(line.x_start, x);
        close(line.x_end, x + 40.0);
        close(line.pattern_origin_x, 0.0);
    }
}

#[test]
fn underline_offset_and_inset_are_used_values() {
    let doc = document(
        "<p>abcd</p>",
        "p {text-decoration:underline; text-underline-offset:25%;text-decoration-inset:2px 3px}",
    );
    let runs = doc.page(0).unwrap().text_runs();
    let line = &runs[0].decorations[0];
    close(line.x_start, 2.0);
    close(line.x_end, 77.0);
    close(line.y - runs[0].origin.1, 7.0);
}

#[test]
fn automatic_insets_keep_the_full_decorated_text_extent() {
    let doc = document(
        "<p>ab</p>",
        "p {text-decoration:underline;text-decoration-inset:auto}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].decorations.len(), 1);
    close(runs[0].decorations[0].x_start, 0.0);
    close(runs[0].decorations[0].x_end, 40.0);
}

#[test]
fn wrapped_runs_stop_before_trailing_collapsible_space() {
    for direction in ["ltr", "rtl"] {
        let doc = document(
            "<p>ab cd</p>",
            &format!("p {{width:50px;direction:{direction};text-decoration:underline}}"),
        );
        let runs = doc.page(0).unwrap().text_runs();
        assert!(runs.len() >= 2);
        for run in runs {
            for line in run.decorations {
                assert!(line.x_end - line.x_start <= 40.001, "{direction}: {line:?}");
            }
        }
    }
}

#[test]
fn decorations_are_page_local_without_duplicates() {
    let doc = document(
        "<p>ab</p><p style='break-before:page'>cd</p>",
        "p {text-decoration:underline}",
    );
    assert_eq!(doc.pages().count(), 2);
    for (page, expected) in doc.pages().zip(["ab", "cd"]) {
        let runs = page.text_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, expected);
        assert_eq!(runs[0].decorations.len(), 1);
        close(runs[0].decorations[0].y - runs[0].origin.1, 2.0);
    }
}

#[test]
fn superscript_keeps_ancestor_line_but_moves_its_own_line() {
    let doc = document(
        "<p id='parent'>a<span id='sup' style='vertical-align:super;font-size:10px;text-decoration:overline blue'>b</span>c</p>",
        "p {text-decoration:underline red}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    let normal = runs.iter().find(|r| r.text == "a").unwrap();
    let sup = runs.iter().find(|r| r.text == "b").unwrap();
    assert!(sup.origin.1 < normal.origin.1);
    let parent = sup
        .decorations
        .iter()
        .find(|l| page.dom().attr(l.origin, "id") == Some("parent"))
        .unwrap();
    close(parent.y, normal.decorations[0].y);
    let own = sup
        .decorations
        .iter()
        .find(|l| page.dom().attr(l.origin, "id") == Some("sup"))
        .unwrap();
    close(own.y, sup.origin.1 - 8.0 + 0.5);
    close(own.thickness, 1.0);
}

#[test]
fn fallback_font_runs_retain_the_origin_metrics_and_pattern() {
    let doc = document(
        "<p>a\u{e70}b</p>",
        "p {font-family:Ahem,'Noto Sans Test';text-decoration:underline dashed red}",
    );
    let page = doc.page(0).unwrap();
    let runs = page.text_runs();
    assert_eq!(runs.iter().map(|r| r.text).collect::<String>(), "a\u{e70}b");
    assert!(
        runs.iter().any(|r| r.font.id != runs[0].font.id),
        "fixture must actually use font fallback"
    );
    let y = runs[0].decorations[0].y;
    for run in runs {
        assert!(!run.glyphs.is_empty());
        let line = &run.decorations[0];
        close(line.y, y);
        close(line.thickness, 1.25);
        close(line.pattern_origin_x, 0.0);
    }
}

#[test]
fn insets_apply_to_the_unsplit_line_before_run_slicing() {
    for (insets, endpoints) in [("4px", (4.0, 116.0)), ("-4px -6px", (-4.0, 126.0))] {
        let doc = document(
            "<p>ab<span style='color:blue'>cd</span>ef</p>",
            &format!("p {{text-decoration:underline red;text-decoration-inset:{insets}}}"),
        );
        let runs = doc.page(0).unwrap().text_runs();
        assert_eq!(runs.len(), 3);
        let lines: Vec<_> = runs.iter().map(|r| &r.decorations[0]).collect();
        close(lines[0].x_start, endpoints.0);
        close(lines[0].x_end, 40.0);
        close(lines[1].x_start, 40.0);
        close(lines[1].x_end, 80.0);
        close(lines[2].x_start, 80.0);
        close(lines[2].x_end, endpoints.1);
    }
}

#[test]
fn pattern_extent_is_shared_before_run_slicing() {
    let doc = document(
        "<p>ab<span style='color:blue'>cd</span>ef</p>",
        "p {text-decoration:underline wavy;text-decoration-inset:4px}",
    );
    let runs = doc.page(0).unwrap().text_runs();
    assert_eq!(runs.len(), 3);
    for run in runs {
        let line = &run.decorations[0];
        close(line.pattern_origin_x, 4.0);
        close(line.pattern_end_x, 116.0);
    }
}

#[test]
fn collapsed_insets_leave_glyphs_without_decoration_segments() {
    let doc = document(
        "<p>ab</p>",
        "p {text-decoration:underline;text-decoration-inset:50px}",
    );
    let runs = doc.page(0).unwrap().text_runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "ab");
    assert!(runs[0].decorations.is_empty());
}

#[test]
fn zero_width_formatting_text_does_not_decorate_neighboring_glyphs() {
    for text in ["\u{200b}", "\u{200c}", "\u{200d}"] {
        let doc = document(
            &format!("<p><span style='text-decoration:underline'>{text}</span>ab</p>"),
            "",
        );
        let runs = doc.page(0).unwrap().text_runs();
        assert!(runs.iter().any(|run| run.text.contains("ab")));
        assert!(runs.iter().all(|run| run.decorations.is_empty()));
    }
}

#[test]
fn word_break_placeholders_do_not_open_decoration_gaps() {
    for body in ["<p>ab<wbr>cd</p>", "<p><wbr>abcd</p>"] {
        let doc = document(body, "p {text-decoration:underline}");
        let runs = doc.page(0).unwrap().text_runs();
        assert_eq!(
            runs.iter()
                .map(|run| run.text)
                .collect::<String>()
                .replace('\u{200b}', ""),
            "abcd"
        );
        close(runs.iter().map(|run| run.advance).sum(), 80.0);
        for line in runs.iter().flat_map(|run| &run.decorations) {
            close(line.pattern_origin_x, 0.0);
            close(line.pattern_end_x, 80.0);
        }
    }
}

#[test]
fn zero_size_text_does_not_emit_collapsed_decoration_spans() {
    let doc = document("<p>ab</p>", "p {font-size:0;text-decoration:underline}");
    let runs = doc.page(0).unwrap().text_runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "ab");
    close(runs[0].advance, 0.0);
    assert!(runs[0].decorations.is_empty());
}
