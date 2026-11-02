use super::*;

fn span_peer() -> String {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("../data/table-cell-stretch-min-height.json")).unwrap();
    oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(oracle["known_preexisting_differences"].as_array().unwrap())
        .find(|case| case["label"] == "span-peer")
        .unwrap()["html"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_tall_page_preserves_the_full_rowspan_geometry() {
    check(
        &format!("<style>@page{{size:800px 1000px}}</style>{}", span_peer()),
        &[
            ("t", [8.0, 8.0, 102.0, 604.0]),
            ("c", [8.0, 8.0, 90.0, 604.0]),
            ("i", [9.0, 9.0, 240.796_88, 602.0]),
        ],
    );
}

#[test]
fn the_short_page_reports_clipped_fragments() {
    let result = layout_geometry_fixture(&span_peer(), None, [800.0, 600.0]);
    assert!(result.page_count() > 1);
    let boxes = first_page_border_boxes(&result);
    for (id, height) in [("t", 592.0), ("c", 592.0), ("i", 591.0)] {
        let rect = boxes.iter().find(|(name, _)| name == id).unwrap().1;
        assert_eq!(rect[3], height);
    }
}

#[test]
#[should_panic(expected = "unfragmented geometry oracle")]
fn full_geometry_oracles_reject_fragmented_layouts() {
    let _ = border_boxes(&span_peer());
}
