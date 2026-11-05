//! A column flex item's first line carries its box to the next page.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults, PaintRect};

fn fragments(display: &str, height: &str, count: usize, tail: bool) -> PageBoxes {
    let items = (1..=count)
        .map(|i| format!("<div class=i id=i{i}>{i}</div>"))
        .collect::<String>();
    let tail = if tail {
        "<div id=tail style='height:20px'></div>"
    } else {
        ""
    };
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:200px 200px;margin:0}}\
         html,body{{margin:0;font:16px/50px Ahem}}\
         .f{{display:{display};flex-direction:column;background:#333}}\
         .i{{{height}background:#eee;margin:0 10px}}</style>\
         <div style='height:44px'></div><div class=f id=f>{items}</div>\
         {tail}"
    );
    render(&html)
}

fn render(html: &str) -> PageBoxes {
    render_with_text(html).0
}

type PageBoxes = Vec<Vec<(String, PaintRect)>>;

type TextCharacters = Vec<Vec<(String, f32, f32)>>;

fn render_with_text(html: &str) -> (PageBoxes, TextCharacters) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!("<style>@page{{size:200px 200px;margin:0}}</style>{html}");
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("incomplete layout")
    };
    let boxes = result
        .pages()
        .map(|page| {
            page.fragments()
                .filter_map(|fragment| {
                    let id = page.dom().attr(fragment.node(), "id")?;
                    Some((id.to_owned(), fragment.rect()))
                })
                .collect()
        })
        .collect();
    let characters = result
        .pages()
        .map(|page| {
            let mut characters = Vec::new();
            for run in page.text_runs() {
                let count = run.text.chars().count();
                for (index, character) in run.text.chars().enumerate() {
                    if matches!(character, '1'..='5') {
                        characters.push((
                            character.to_string(),
                            run.origin.0 + index as f32 * run.advance / count as f32,
                            run.origin.1,
                        ));
                    }
                }
            }
            characters.sort_by(|a, b| {
                a.2.total_cmp(&b.2)
                    .then(a.1.total_cmp(&b.1))
                    .then(a.0.cmp(&b.0))
            });
            characters
        })
        .collect();
    (boxes, characters)
}

fn assert_whole_fourth_item(display: &str, height: &str, count: usize, tail: bool) {
    let pages = fragments(display, height, count, tail);
    assert_eq!(pages.len(), 2, "{display}/{height}: {pages:?}");
    assert!(
        pages[0].iter().all(|(id, _)| id != "i4"),
        "{display}/{height}: {pages:?}"
    );
    for (id, top, expected_height) in [
        ("i4", 0.0, 50.0),
        ("i5", 50.0, 50.0),
        ("tail", (count - 3) as f32 * 50.0, 20.0),
    ] {
        if id == "i5" && count < 5 || id == "tail" && !tail {
            continue;
        }
        let (_, rect) = pages[1].iter().find(|(found, _)| found == id).expect(id);
        assert_eq!(
            (rect.y, rect.height),
            (top, expected_height),
            "{display}/{height}/{id}"
        );
    }
    let (_, container) = pages[1]
        .iter()
        .find(|(id, _)| id == "f")
        .expect("container");
    assert_eq!(
        (container.y, container.height),
        (0.0, (count - 3) as f32 * 50.0)
    );
}

#[test]
fn a_column_flex_item_moves_whole_when_its_first_line_does_not_fit() {
    for height in ["height:50px;", ""] {
        for count in [4, 5] {
            for tail in [false, true] {
                assert_whole_fourth_item("flex", height, count, tail);
            }
        }
    }
}

#[test]
fn the_corresponding_block_keeps_its_whole_item_transition() {
    for height in ["height:50px;", ""] {
        for count in [4, 5] {
            for tail in [false, true] {
                assert_whole_fourth_item("block", height, count, tail);
            }
        }
    }
}

#[test]
fn recorded_chromium_print_fragments_keep_the_first_piece_with_its_item() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/column-flex-first-line/chromium.json"))
            .expect("Chromium print fixture");
    let mut failures = Vec::new();
    for case in fixture["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("case name");
        let (actual, characters) = render_with_text(case["body"].as_str().expect("body"));
        let expected_chars = case["chars"].as_array().expect("character pages");
        if characters.len() != expected_chars.len() {
            failures.push(format!(
                "{name}: character pages {} vs {}",
                characters.len(),
                expected_chars.len()
            ));
        }
        for (page, (actual, expected)) in characters.iter().zip(expected_chars).enumerate() {
            let expected = expected.as_array().expect("characters");
            if actual.len() != expected.len() {
                failures.push(format!(
                    "{name} page{page}: characters {actual:?} vs {expected:?}"
                ));
                continue;
            }
            for ((text, x, y), wanted) in actual.iter().zip(expected) {
                if text != wanted[0].as_str().expect("character")
                    || (*x - wanted[1].as_f64().expect("x") as f32).abs() > 0.02
                    || (*y - wanted[2].as_f64().expect("y") as f32).abs() > 0.02
                {
                    failures.push(format!(
                        "{name} page{page}: character {:?} vs {wanted:?}",
                        (text, x, y)
                    ));
                }
            }
        }
        let expected = case["pages"].as_array().expect("pages");
        if actual.len() != expected.len() {
            failures.push(format!(
                "{name}: pages {} vs {}",
                actual.len(),
                expected.len()
            ));
            continue;
        }
        for (page, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            let expected = expected.as_array().expect("page boxes");
            let mut actual: Vec<_> = actual
                .iter()
                .filter(|(_, rect)| rect.width > 0.001 && rect.height > 0.001)
                .collect();
            actual.sort_by_key(|(id, _)| id);
            if actual.len() != expected.len() {
                failures.push(format!(
                    "{name} page{page}: boxes {actual:?} vs {expected:?}"
                ));
                continue;
            }
            for ((id, rect), expected) in actual.into_iter().zip(expected) {
                let wanted = expected["id"].as_str().expect("id");
                let values = [rect.x, rect.y, rect.width, rect.height];
                let bounds: Vec<_> = expected["rect"]
                    .as_array()
                    .expect("rect")
                    .iter()
                    .map(|v| v.as_f64().expect("coordinate") as f32)
                    .collect();
                if id != wanted
                    || values
                        .iter()
                        .zip(&bounds)
                        .any(|(a, b)| (a - b).abs() > 0.02)
                {
                    failures.push(format!(
                        "{name} page{page} #{id}: {values:?} vs #{wanted} {bounds:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
