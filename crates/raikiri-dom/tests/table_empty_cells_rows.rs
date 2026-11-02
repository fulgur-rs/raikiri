//! Empty-cell paint suppression leaves the row geometry intact, matching
//! Chromium rather than CSS 2.2's zero-height empty-row rule.

use raikiri_html::{LayoutOptions, LayoutStatus, layout, parse_html};
use raikiri_traits::{LayoutConfig, PageDefaults};

#[test]
fn hidden_empty_cells_keep_row_heights_spacing_and_height_distribution() {
    for (css, rows, expected) in [
        (
            "border-spacing:0",
            "<tr id=r1><td style='height:20px'></td></tr><tr id=r2><td><div style='height:10px'></div></td></tr>",
            [("t", 8.0, 30.0), ("r1", 8.0, 20.0), ("r2", 28.0, 10.0)],
        ),
        (
            "border-spacing:4px",
            "<tr id=r1><td style='height:20px'></td></tr><tr id=r2><td><div style='height:10px'></div></td></tr>",
            [("t", 8.0, 42.0), ("r1", 12.0, 20.0), ("r2", 36.0, 10.0)],
        ),
        (
            "border-spacing:0;height:100px",
            "<tr id=r1><td></td></tr><tr id=r2><td style='height:20px'></td></tr>",
            [("t", 8.0, 100.0), ("r1", 8.0, 80.0), ("r2", 88.0, 20.0)],
        ),
        (
            "border-spacing:0;height:100px",
            "<tr id=r1><td style='height:20px'></td></tr><tr style='visibility:collapse'><td style='height:20px'></td></tr><tr id=r2><td><div style='height:10px'></div></td></tr>",
            [("t", 8.0, 80.0), ("r1", 8.0, 20.0), ("r2", 28.0, 60.0)],
        ),
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:800px 600px;margin:0}}body{{margin:8px}}td{{padding:0}}table{{empty-cells:hide;{css}}}</style><table id=t>{rows}</table>"
        );
        let doc = parse_html(html.as_bytes(), &Default::default()).unwrap();
        let LayoutStatus::Completed(result) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .unwrap() else {
            panic!("complete layout");
        };
        let page = result.pages().next().unwrap();
        let dom = page.dom();
        for (id, top, height) in expected {
            let rect = page
                .fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .unwrap()
                .rect();
            assert_eq!((rect.y, rect.height), (top, height), "{css} #{id}");
        }
    }
}
