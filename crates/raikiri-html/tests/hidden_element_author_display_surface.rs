//! The metadata and raw text elements that HTML hides with a user agent
//! `display: none` render when author style gives them a display, as in
//! browsers: the text of a `script`, `style` or `title`, and the box of a
//! `meta`, `link` or `base`. The contents of a `template` or `noscript` never
//! render. The page counts and line positions are the ones Chromium prints
//! for the same documents (`page.pdf` with the page size taken from `@page`).

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays `body` out on pages 100px square with 30px lines.
fn layout_body(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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

/// Each text run as `text@page:top`, sorted, with the page count.
fn lines(body: &str) -> (usize, Vec<String>) {
    let result = layout_body(body);
    let mut runs = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages += 1;
        for run in page.text_runs() {
            // The 20px Ahem baseline sits 21px below the top of its 30px line.
            let top = (run.origin.1 - 21.0).round() as i32;
            runs.push(format!("{}@{index}:{top}", run.text));
        }
    }
    runs.sort();
    (pages, runs)
}

fn expect(body: &str, pages: usize, runs: &[&str]) {
    let mut runs: Vec<String> = runs.iter().map(|run| run.to_string()).collect();
    runs.sort();
    assert_eq!(lines(body), (pages, runs), "{body}");
}

#[test]
fn hidden_elements_with_an_author_display_render() {
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><script style='display:block'>x</script>Z</div>",
            1,
            &["x@0:0", "Z@0:30"][..],
        ),
        (
            "<div style='width:20px'><script style='display:inline'>x</script>Z</div>",
            1,
            &["x@0:0", "Z@0:0"][..],
        ),
        (
            "<div style='width:20px'><style style='display:block'>.q{}</style>Z</div>",
            1,
            &[".q{}@0:0", "Z@0:30"][..],
        ),
        (
            "<div style='width:20px'><title style='display:block'>x</title>Z</div>",
            1,
            &["x@0:0", "Z@0:30"][..],
        ),
        (
            "<div style='width:20px'><meta style='display:block;height:30px'>Z</div>",
            1,
            &["Z@0:30"][..],
        ),
        (
            "<div style='width:20px'><link style='display:block;height:30px'>Z</div>",
            1,
            &["Z@0:30"][..],
        ),
        (
            "<div style='width:20px'><base style='display:block;height:30px'>Z</div>",
            1,
            &["Z@0:30"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}

#[test]
fn hidden_elements_keep_hidden_without_an_author_display() {
    // The user agent `display: none` holds, and the contents of a `template`
    // or `noscript` stay hidden whatever their display.
    for (body, pages, runs) in [
        (
            "<div style='width:20px'><script>x</script>Z</div>",
            1,
            &["Z@0:0"][..],
        ),
        (
            "<div style='width:20px'><noscript style='display:block'>x</noscript>Z</div>",
            1,
            &["Z@0:0"][..],
        ),
        (
            "<div style='width:20px'><template style='display:block'>x</template>Z</div>",
            1,
            &["Z@0:0"][..],
        ),
    ] {
        expect(body, pages, runs);
    }
}
