//! A `<details>` element shows its summary with a disclosure marker, and
//! hides the rest of its contents while it is closed.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn lay_out(body: &str, extra: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 300px;margin:0}}\
         body{{margin:0;font:10px/20px Ahem}}{extra}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

/// Each run on the first page as its text and origin.
fn runs(body: &str, extra: &str) -> Vec<(String, (f32, f32))> {
    let document = lay_out(body, extra);
    let page = document.page(0).unwrap();
    page.text_runs()
        .into_iter()
        .filter(|run| !run.text.trim().is_empty())
        .map(|run| (run.text.to_string(), run.origin))
        .collect()
}

/// The origin of the first run whose text is `text`.
fn origin_of(runs: &[(String, (f32, f32))], text: &str) -> (f32, f32) {
    runs.iter()
        .find(|(run, _)| run == text)
        .unwrap_or_else(|| panic!("no run {text:?} in {runs:?}"))
        .1
}

fn texts(runs: &[(String, (f32, f32))]) -> Vec<&str> {
    runs.iter().map(|(text, _)| text.as_str()).collect()
}

// The expected positions are Chromium's (print media, Ahem 10px/20px). Runs
// report the alphabetic baseline, 13px below the top of a 20px line. The
// marker advance depends on the font that draws the triangle, so the text
// after a marker is only checked to share its line.

#[test]
fn a_closed_details_shows_its_summary_with_a_closed_marker() {
    let runs = runs(
        "<details><summary>AB</summary><p>CD</p>EF</details><div>GH</div>",
        "",
    );
    assert_eq!(texts(&runs), ["\u{25b8}", "AB", "GH"], "{runs:?}");
    assert_eq!(origin_of(&runs, "\u{25b8}"), (0.0, 13.0));
    assert_eq!(origin_of(&runs, "AB").1, 13.0);
    // The details is one line high: the next block starts at 20.
    assert_eq!(origin_of(&runs, "GH"), (0.0, 33.0));
}

#[test]
fn an_open_details_shows_an_open_marker_and_all_its_content() {
    let runs = runs(
        "<details open><summary>AB</summary><p>CD</p>EF</details><div>GH</div>",
        "",
    );
    assert_eq!(origin_of(&runs, "\u{25be}"), (0.0, 13.0));
    assert_eq!(origin_of(&runs, "AB").1, 13.0);
    assert_eq!(origin_of(&runs, "CD"), (0.0, 43.0));
    assert_eq!(origin_of(&runs, "EF"), (0.0, 73.0));
    assert_eq!(origin_of(&runs, "GH"), (0.0, 93.0));
    // `open=""` and any other value open it too.
    let runs = runs_of("<details open=''><summary>AB</summary>EF</details>");
    assert!(texts(&runs).contains(&"EF"), "{runs:?}");
}

fn runs_of(body: &str) -> Vec<(String, (f32, f32))> {
    runs(body, "")
}

#[test]
fn only_the_first_summary_of_a_details_has_a_marker() {
    let runs = runs_of("<details open><summary>AB</summary><summary>CD</summary></details>");
    assert_eq!(texts(&runs), ["\u{25be}", "AB", "CD"], "{runs:?}");
    assert_eq!(origin_of(&runs, "CD"), (0.0, 33.0));
    // A closed details hides its other summaries, and text before its
    // summary.
    let runs =
        runs_of("<details>EF<summary>AB</summary>IJ<summary>CD</summary></details><div>GH</div>");
    assert_eq!(texts(&runs), ["\u{25b8}", "AB", "GH"], "{runs:?}");
    assert_eq!(origin_of(&runs, "GH"), (0.0, 33.0));
    // A summary outside a details is an ordinary block.
    let runs = runs_of("<summary>AB</summary><div><summary>CD</summary></div>");
    assert_eq!(texts(&runs), ["AB", "CD"], "{runs:?}");
    assert_eq!(origin_of(&runs, "AB"), (0.0, 13.0));
}

#[test]
fn the_marker_of_a_right_to_left_summary_keeps_its_triangle() {
    for (open, marker) in [("", "\u{25b8}"), (" open", "\u{25be}")] {
        let runs = runs_of(&format!(
            "<details dir=rtl{open}><summary>AB</summary></details>"
        ));
        assert!(texts(&runs).contains(&marker), "{runs:?}");
    }
}

#[test]
fn author_styles_do_not_show_the_content_of_a_closed_details() {
    let runs = runs(
        "<details><summary>AB</summary><p>CD</p><span>EF</span></details><div>GH</div>",
        "details p, details span { display: block !important }",
    );
    assert_eq!(texts(&runs), ["\u{25b8}", "AB", "GH"], "{runs:?}");
    assert_eq!(origin_of(&runs, "GH"), (0.0, 33.0));
}

#[test]
fn the_hidden_text_takes_no_room_in_any_kind_of_details_box() {
    for (body, gh) in [
        (
            "<details style='display:flex'><summary>AB</summary>EF<b>IJ</b></details><div>GH</div>",
            (0.0, 33.0),
        ),
        (
            "<details style='display:grid'><summary>AB</summary>EF<b>IJ</b></details><div>GH</div>",
            (0.0, 33.0),
        ),
        (
            "<details style='columns:2'><summary>AB</summary>EF<b>IJ</b></details><div>GH</div>",
            (0.0, 33.0),
        ),
        (
            "<details style='display:inline'><summary>AB</summary>EF<b>IJ</b></details><i>GH</i>",
            (0.0, 33.0),
        ),
        (
            "<details style='display:contents'><summary>AB</summary>EF</details><i>GH</i>",
            (0.0, 33.0),
        ),
        (
            "<details><summary>AB</summary>EF<span style='float:left'>IJ</span><span style='position:absolute'>MN</span></details><div>GH</div>",
            (0.0, 33.0),
        ),
        (
            "<details style='white-space:pre'><summary>AB</summary>  EF  </details><div>GH</div>",
            (0.0, 33.0),
        ),
        (
            "<table><tr><td><details><summary>AB</summary>EF</details>GH</td></tr></table>",
            (3.0, 36.0),
        ),
    ] {
        let runs = runs_of(body);
        assert!(
            !texts(&runs).iter().any(|text| ["EF", "IJ", "MN"]
                .iter()
                .any(|hidden| text.contains(hidden))),
            "{body}: {runs:?}"
        );
        assert_eq!(origin_of(&runs, "GH"), gh, "{body}");
    }
}

#[test]
fn a_shrink_to_fit_closed_details_is_as_wide_as_its_summary() {
    // Chromium: the float is as wide as its summary line and MN follows it.
    let runs =
        runs_of("<details style='float:left'><summary>AB</summary>EFGHIJKL</details><i>MN</i>");
    let ab = origin_of(&runs, "AB");
    assert_eq!(origin_of(&runs, "MN"), (ab.0 + 20.0, 13.0), "{runs:?}");
}

#[test]
fn a_closed_details_does_not_hide_the_content_of_an_open_one() {
    // Identical children under a closed and an open details: the open one's
    // must not take the hidden style of the closed one's.
    let runs = runs_of(
        "<details><summary>AB</summary><p>CD</p>EF</details>\
         <details open><summary>AB</summary><p>CD</p>EF</details>",
    );
    assert_eq!(
        texts(&runs).iter().filter(|text| **text == "CD").count(),
        1,
        "{runs:?}"
    );
    assert_eq!(
        texts(&runs).iter().filter(|text| **text == "EF").count(),
        1,
        "{runs:?}"
    );
    assert_eq!(origin_of(&runs, "CD"), (0.0, 63.0));
}

/// Each run on the first page as its text, origin and font size.
fn sized_runs(body: &str, extra: &str) -> Vec<(String, (f32, f32), f32)> {
    let document = lay_out(body, extra);
    let page = document.page(0).unwrap();
    page.text_runs()
        .into_iter()
        .filter(|run| !run.text.trim().is_empty())
        .map(|run| (run.text.to_string(), run.origin, run.font_size))
        .collect()
}

#[test]
fn hidden_text_is_not_the_first_line_or_the_first_letter_of_a_details() {
    // Chromium styles the summary line, the first formatted line, even
    // when hidden text comes before the summary.
    let runs = sized_runs(
        "<details>EF<summary>AB</summary>IJ</details>",
        "details::first-line{font-size:20px}",
    );
    assert_eq!(
        runs.iter().map(|run| run.0.as_str()).collect::<Vec<_>>(),
        ["\u{25b8}", "AB"]
    );
    assert!(runs.iter().all(|run| run.2 == 20.0), "{runs:?}");
    // Chromium gives a lone punctuation mark no first letter: the hidden
    // letters after it do not join it.
    let runs = sized_runs(
        "<details><summary style='display:inline'>\"</summary>EF</details>",
        "details::first-letter{font-size:20px}",
    );
    assert_eq!(runs, [("\"".to_string(), (0.0, 13.0), 10.0)]);
}

#[test]
fn hidden_text_does_not_keep_a_details_around_a_spanner() {
    // A details whose only rendered child spans the columns lays out as one
    // with no other content.
    let with_hidden_text = sized_runs(
        "<div style='columns:2'><details><summary style='column-span:all'>AB</summary>EF</details><p>CD</p></div>",
        "",
    );
    let without = sized_runs(
        "<div style='columns:2'><details><summary style='column-span:all'>AB</summary></details><p>CD</p></div>",
        "",
    );
    assert_eq!(with_hidden_text, without);
}

// The cases below were measured in Chromium print with the same page, body
// and font. The legend text starts after the closed or open marker, so the
// marker and the text are checked to share the first line.

#[test]
fn a_details_without_a_summary_shows_the_default_legend() {
    // Closed: only the legend shows, one line high.
    let runs = runs_of("<details><p>CD</p>EF</details><div>GH</div>");
    assert_eq!(texts(&runs), ["\u{25b8}", " Details", "GH"], "{runs:?}");
    assert_eq!(origin_of(&runs, "\u{25b8}"), (0.0, 13.0));
    assert_eq!(origin_of(&runs, " Details").1, 13.0);
    assert_eq!(origin_of(&runs, "GH"), (0.0, 33.0));
    // An empty details shows the legend too.
    let runs = runs_of("<details></details><div>GH</div>");
    assert_eq!(texts(&runs), ["\u{25b8}", " Details", "GH"], "{runs:?}");
    // Open: the legend has the open marker and the contents follow it.
    let runs = runs_of("<details open><p>CD</p>EF</details><div>GH</div>");
    assert_eq!(origin_of(&runs, "\u{25be}"), (0.0, 13.0));
    assert_eq!(origin_of(&runs, "CD"), (0.0, 43.0));
    assert_eq!(origin_of(&runs, "EF"), (0.0, 73.0));
    assert_eq!(origin_of(&runs, "GH"), (0.0, 93.0));
}

#[test]
fn the_default_legend_belongs_only_to_a_details_without_a_summary() {
    // Two details side by side, one with a summary and one without: the
    // style of one is not shared with the other.
    let runs = runs_of("<details><summary>X</summary></details><details>EF</details><div>GH</div>");
    assert_eq!(
        texts(&runs),
        ["\u{25b8}", "X", "\u{25b8}", " Details", "GH"],
        "{runs:?}"
    );
    assert_eq!(origin_of(&runs, " Details").1, 33.0);
    assert_eq!(origin_of(&runs, "GH"), (0.0, 53.0));
    // A summary nested deeper is not the details' summary.
    let runs = runs_of("<details open><div><summary>X</summary></div></details>");
    assert_eq!(texts(&runs), ["\u{25be}", " Details", "X"], "{runs:?}");
    // An author `::before` without content keeps the legend, which stays a
    // block of its own.
    let runs = self::runs(
        "<details>CD</details>",
        "details::before { display: inline }",
    );
    assert_eq!(texts(&runs), ["\u{25b8}", " Details"], "{runs:?}");
    // An author `::before` with content takes the place of the legend.
    let runs = self::runs("<details>CD</details>", "details::before { content: 'Q' }");
    assert_eq!(texts(&runs), ["Q"], "{runs:?}");
}

#[test]
fn the_default_legend_sizes_a_shrink_to_fit_details() {
    let runs = runs_of("<div style='display:flex'><details>EF</details><div>GH</div></div>");
    let marker = origin_of(&runs, "\u{25b8}");
    let legend = origin_of(&runs, " Details");
    let gh = origin_of(&runs, "GH");
    assert_eq!(marker, (0.0, 13.0));
    // The item is as wide as the marker and the seven letters of the legend.
    assert_eq!(gh, (legend.0 + 80.0, 13.0), "{runs:?}");
}

#[test]
fn the_first_summary_is_laid_out_ahead_of_the_other_children() {
    let runs = runs_of("<details open><p>CD</p><summary>EF</summary></details><div>GH</div>");
    assert_eq!(origin_of(&runs, "\u{25be}"), (0.0, 13.0));
    assert_eq!(origin_of(&runs, "EF").1, 13.0);
    assert_eq!(origin_of(&runs, "CD"), (0.0, 43.0));
    // The bottom margin of the paragraph separates the next block.
    assert_eq!(origin_of(&runs, "GH"), (0.0, 73.0));
    // Text on both sides of the summary joins up in one line after it.
    let runs = runs_of("<details open>EF<summary>S</summary>GH</details><div>XY</div>");
    assert_eq!(origin_of(&runs, "S").1, 13.0);
    assert_eq!(origin_of(&runs, "EF"), (0.0, 33.0));
    assert_eq!(origin_of(&runs, "GH"), (20.0, 33.0));
    assert_eq!(origin_of(&runs, "XY"), (0.0, 53.0));
    // A comment and a hidden child before the summary change nothing.
    let runs = runs_of(
        "<details open><!-- c --><p style='display:none'>IJ</p><summary>EF</summary><p>CD</p></details>",
    );
    assert_eq!(texts(&runs), ["\u{25be}", "EF", "CD"], "{runs:?}");
    assert_eq!(origin_of(&runs, "CD"), (0.0, 43.0));
    // White space before the summary changes nothing.
    let runs = runs_of("<details open>\n  <summary>EF</summary>\n  <p>CD</p></details>");
    assert_eq!(origin_of(&runs, "EF").1, 13.0);
    assert_eq!(origin_of(&runs, "CD"), (0.0, 43.0));
}

#[test]
fn the_first_summary_leads_a_flex_details_and_a_paginated_one() {
    // The summary is the first flex item.
    let runs =
        runs_of("<details open style='display:flex'><div>XY</div><summary>EF</summary></details>");
    let ef = origin_of(&runs, "EF");
    let xy = origin_of(&runs, "XY");
    assert_eq!(ef.1, 13.0);
    assert_eq!(xy, (ef.0 + 20.0, 13.0), "{runs:?}");
    // `order` sorts the items after the summary has moved to the front.
    let runs = runs_of(
        "<details open style='display:flex'><div style='order:1'>XY</div><b>IJ</b><summary>EF</summary></details>",
    );
    let ef = origin_of(&runs, "EF");
    assert_eq!(origin_of(&runs, "IJ"), (ef.0 + 20.0, 13.0), "{runs:?}");
    assert_eq!(origin_of(&runs, "XY"), (ef.0 + 40.0, 13.0), "{runs:?}");
    // Content after the summary moves to the next page as its own box.
    let document = lay_out(
        "<details open><div style='height:290px'></div><div>XY</div><summary>EF</summary></details>",
        "",
    );
    assert_eq!(document.page_count(), 2);
    let runs = runs_of(
        "<details open><div style='height:290px'></div><div>XY</div><summary>EF</summary></details>",
    );
    assert_eq!(texts(&runs), ["\u{25be}", "EF"], "{runs:?}");
}

/// Each run on page `index` as its text and origin.
fn page_runs(body: &str, index: u32) -> Vec<(String, (f32, f32))> {
    let document = lay_out(body, "");
    let page = document.page(index).unwrap();
    page.text_runs()
        .into_iter()
        .filter(|run| !run.text.trim().is_empty())
        .map(|run| (run.text.to_string(), run.origin))
        .collect()
}

#[test]
fn pagination_follows_the_summary_first_order() {
    // A forced break before the paragraph comes after the summary.
    let body =
        "<details open><p style='break-before:page;margin:0'>CD</p><summary>S</summary></details>";
    assert_eq!(lay_out(body, "").page_count(), 2);
    assert_eq!(texts(&page_runs(body, 0)), ["\u{25be}", "S"]);
    assert_eq!(page_runs(body, 1), [("CD".to_owned(), (0.0, 13.0))]);
    // The tall block crosses the page end and the next one follows it.
    let body = "<details open><div style='height:290px'></div><div>XY</div><summary>EF</summary></details>";
    assert_eq!(page_runs(body, 1), [("XY".to_owned(), (0.0, 23.0))]);
}

#[test]
fn css_order_does_not_move_the_children_of_a_block_details() {
    let runs =
        runs_of("<details open><p style='order:1'>AB</p><p>CD</p><summary>S</summary></details>");
    assert_eq!(origin_of(&runs, "S").1, 13.0);
    assert_eq!(origin_of(&runs, "AB"), (0.0, 43.0));
    assert_eq!(origin_of(&runs, "CD"), (0.0, 73.0));
}

#[test]
fn preserved_white_space_before_the_summary_follows_it() {
    let runs =
        runs_of("<details open style='white-space:pre'>\n\n<summary>S</summary>XY</details>");
    assert_eq!(origin_of(&runs, "S").1, 13.0);
    assert_eq!(origin_of(&runs, "XY"), (0.0, 73.0));
}

#[test]
fn a_summary_or_details_without_a_box_still_puts_the_summary_first() {
    // A `display: contents` summary of a flex details: its content is the
    // first item, once.
    let runs = runs_of(
        "<details open style='display:flex'><div>X</div><summary style='display:contents'><span>S</span></summary></details>",
    );
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(origin_of(&runs, "S"), (0.0, 13.0));
    assert_eq!(origin_of(&runs, "X"), (10.0, 13.0));
    // A `display: contents` details in a flex container: its summary comes
    // before its other children among the container's items.
    let runs = runs_of(
        "<div style='display:flex'><i style='order:1'>Z</i><details open style='display:contents'><b>X</b><summary>S</summary></details></div>",
    );
    let s = origin_of(&runs, "S");
    assert_eq!(origin_of(&runs, "X"), (s.0 + 10.0, 13.0), "{runs:?}");
    assert_eq!(origin_of(&runs, "Z"), (s.0 + 20.0, 13.0), "{runs:?}");
}
