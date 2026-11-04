//! Vertical table inline tracks fit between independently measured bounds.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

#[test]
fn vertical_table_heights_match_chromium() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let mut failures = Vec::new();
    for row in include_str!("data/vertical-table-height.tsv")
        .lines()
        .filter(|row| !row.starts_with('#'))
    {
        let fields = row.split('\t').collect::<Vec<_>>();
        let label = fields[0];
        let html = format!(
            "<!doctype html><style>@page{{size:800px 600px;margin:0}}body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}{}</style><div id=stage>{}</div>",
            fields[1], fields[2]
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
        for expected in fields[3].split(';') {
            let (id, values) = expected.split_once(':').unwrap();
            let expected = values
                .split(',')
                .map(|n| n.parse::<f32>().unwrap())
                .collect::<Vec<_>>();
            let rect = page
                .fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .unwrap_or_else(|| panic!("{label}: #{id}"))
                .rect();
            let actual = [rect.x, rect.y, rect.width, rect.height];
            if actual
                .iter()
                .zip(&expected)
                .any(|(a, e)| (a - e).abs() > 0.04)
            {
                failures.push(format!(
                    "{label} #{id}: actual {actual:?}, expected {expected:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
