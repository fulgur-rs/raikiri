//! Geometry discovery continues past the pages known by the preceding pass.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderError, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LimitKind, RenderLimits};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn completed(body: &str, css: &str) -> DocumentLayout {
    run(body, css, LayoutConfig::default()).expect("layout must settle")
}

fn run(body: &str, css: &str, config: LayoutConfig) -> Result<DocumentLayout, RenderError> {
    let html = format!(
        "<!doctype html><style>@page{{size:200px 30px;margin:0}}\
        @page:first{{size:200px 200px}}body{{margin:0;font:10px/10px Ahem}}{css}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let document = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        config,
        LayoutOptions::new().resources(&resources),
    )?
    else {
        panic!("layout must complete");
    };
    Ok(result)
}

fn row_boxes(count: usize, property: &str) -> String {
    (0..count)
        .map(|index| format!("<div id=r{index} style='{property}:10px'>r{index}</div>"))
        .collect()
}

fn horizontal_body() -> String {
    format!(
        "<div style='height:100px'>A</div><div style='height:60px'>B</div>{}",
        row_boxes(24, "height")
    )
}

fn assert_row_text_once(result: &DocumentLayout, count: usize) {
    let text: Vec<_> = result
        .pages()
        .flat_map(|page| page.text_runs().into_iter().map(|run| run.text.to_owned()))
        .collect();
    for index in 0..count {
        let expected = format!("r{index}");
        assert_eq!(
            text.iter().filter(|text| **text == expected).count(),
            1,
            "{expected}: {text:?}"
        );
    }
}

#[test]
fn a_tall_first_page_discovers_all_of_the_short_following_pages() {
    let result = completed(&horizontal_body(), "");
    let pages: Vec<_> = result.pages().collect();
    assert_eq!(pages.len(), 8);
    assert_eq!(pages[0].geometry().page_box.height, 200.0);
    assert!(
        pages[1..]
            .iter()
            .all(|page| page.geometry().page_box.height == 30.0)
    );
    for index in 0..24 {
        let expected = if index < 4 {
            (0, 160.0 + index as f32 * 10.0)
        } else {
            (1 + (index - 4) / 3, ((index - 4) % 3) as f32 * 10.0)
        };
        let id = format!("r{index}");
        let mut actual = Vec::new();
        for (page_index, page) in pages.iter().enumerate() {
            let dom = page.dom();
            for fragment in page.fragments() {
                if dom.attr(fragment.node(), "id") == Some(id.as_str()) {
                    actual.push((page_index, fragment.rect().y));
                }
            }
        }
        assert_eq!(actual, [expected], "{id}");
    }
}

#[test]
fn closing_padding_keeps_every_row_when_later_pages_are_short() {
    let result = completed(
        &format!(
            "<div style='padding-bottom:30px'><div style='height:100px'>A</div><div style='height:60px'>B</div>{}</div><div style='height:20px'>Z</div>",
            row_boxes(12, "height")
        ),
        "",
    );
    assert!(result.pages().count() >= 5);
    assert_row_text_once(&result, 12);
    let tail_count = result
        .pages()
        .flat_map(|page| page.text_runs().into_iter())
        .filter(|run| run.text == "Z")
        .count();
    assert_eq!(tail_count, 1);
}

#[test]
fn alternating_pages_are_resolved_after_discovering_the_short_tail() {
    let result = completed(&horizontal_body(), "@page:right { size:200px 50px }");
    let heights: Vec<_> = result
        .pages()
        .map(|page| page.geometry().page_box.height)
        .collect();
    assert_eq!(heights, [200.0, 30.0, 50.0, 30.0, 50.0, 30.0, 50.0]);
    assert_row_text_once(&result, 24);
    for (id, expected_page) in [
        ("r4", 1),
        ("r7", 2),
        ("r12", 3),
        ("r15", 4),
        ("r20", 5),
        ("r23", 6),
    ] {
        let mut actual = Vec::new();
        for (index, page) in result.pages().enumerate() {
            let dom = page.dom();
            for fragment in page.fragments() {
                if dom.attr(fragment.node(), "id") == Some(id) {
                    actual.push((index, fragment.rect().y));
                }
            }
        }
        assert_eq!(actual, [(expected_page, 0.0)], "{id}");
    }
}

#[test]
fn named_page_transitions_re_resolve_the_discovered_tail() {
    let result = completed(
        &format!(
            "<div style='height:160px'>A</div><section style='page:small'>{}</section><div style='height:10px'>Z</div>",
            row_boxes(4, "height")
        ),
        "@page small { size:200px 20px }",
    );
    let geometry: Vec<_> = result
        .pages()
        .map(|page| {
            (
                page.name().map(str::to_owned),
                page.geometry().page_box.height,
            )
        })
        .collect();
    assert_eq!(
        geometry,
        [
            (None, 200.0),
            (Some("small".to_owned()), 20.0),
            (Some("small".to_owned()), 20.0),
            (None, 30.0)
        ]
    );
    assert_row_text_once(&result, 4);
}

#[test]
fn vertical_pages_extend_the_last_known_block_axis_width() {
    let boxes: String = (0..24)
        .map(|index| {
            format!("<div id=r{index} style='width:10px;height:20px;background:lime'></div>")
        })
        .collect();
    let result = completed(
        &format!(
            "<div style='width:100px;height:20px'></div><div style='width:60px;height:20px'></div>{boxes}"
        ),
        "@page { size:30px 200px } @page:first { size:200px 200px } html { writing-mode:vertical-lr }",
    );
    let widths: Vec<_> = result
        .pages()
        .map(|page| page.geometry().page_box.width)
        .collect();
    assert_eq!(widths, [200.0, 30.0, 30.0, 30.0, 30.0, 30.0, 30.0, 30.0]);
    for index in 0..24 {
        let expected_page = if index < 4 { 0 } else { 1 + (index - 4) / 3 };
        let id = format!("r{index}");
        let mut actual = Vec::new();
        for (page_index, page) in result.pages().enumerate() {
            let dom = page.dom();
            for fragment in page.fragments() {
                if dom.attr(fragment.node(), "id") == Some(id.as_str()) {
                    actual.push(page_index);
                }
            }
        }
        assert_eq!(actual, [expected_page], "{id}");
    }
}

#[test]
fn page_limits_are_checked_after_the_short_page_schedule_settles() {
    for limit in [None, Some(8), Some(9)] {
        let config = LayoutConfig::builder()
            .limits(RenderLimits::builder().max_document_pages(limit).build())
            .build();
        let result = run(&horizontal_body(), "", config).expect("within the page limit");
        assert_eq!(result.pages().count(), 8);
        assert_row_text_once(&result, 24);
    }
    let config = LayoutConfig::builder()
        .limits(RenderLimits::builder().max_document_pages(Some(7)).build())
        .build();
    assert!(matches!(
        run(&horizontal_body(), "", config),
        Err(RenderError::LimitExceeded {
            kind: LimitKind::Pages,
            limit: 7,
            actual: 8
        })
    ));
}
