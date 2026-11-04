//! Vertical table items consume assigned height once across their column tracks.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

#[test]
fn vertical_table_items_match_chromium() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let mut failures = Vec::new();
    for row in include_str!("data/vertical-table-item-stretch.tsv")
        .lines()
        .filter(|row| !row.starts_with('#'))
    {
        let fields = row.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 5);
        let label = fields[0];
        let count = fields[1].parse::<usize>().unwrap();
        let expected = fields[4].split(';').collect::<Vec<_>>();
        assert_eq!(expected.len(), count + 3, "{label}: include every cell");
        let html = format!(
            "<!doctype html><style>@page{{size:800px 600px;margin:0}}body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}#w,#o{{overflow:visible}}{}</style><div id=stage>{}</div>",
            fields[2], fields[3]
        );
        let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        let LayoutStatus::Completed(document) = layout(
            &parsed,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap() else {
            panic!("complete {label}")
        };
        assert_eq!(document.page_count(), 1, "{label}");
        let page = document.page(0).unwrap();
        let dom = page.dom();
        for expected in expected {
            let (id, coordinates) = expected.split_once(':').unwrap();
            let expected = coordinates
                .split(',')
                .map(|value| value.parse::<f32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(expected.len(), 4, "{label}: rectangle coordinates");
            let rect = page
                .fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .unwrap_or_else(|| panic!("{label}: missing #{id}"))
                .rect();
            let actual = [rect.x, rect.y, rect.width, rect.height];
            // Match the existing tolerance for Chromium's 1/64px layout units.
            if !actual
                .iter()
                .zip(&expected)
                .all(|(actual, expected)| (actual - expected).abs() <= 0.04)
            {
                failures.push(format!(
                    "{label} #{id}: actual {actual:?}, expected {expected:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
