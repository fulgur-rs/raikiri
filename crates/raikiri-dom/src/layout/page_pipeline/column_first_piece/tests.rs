use super::*;
use crate::layout::test_support::page_box_800x600;

fn measured_piece(column_css: &str, item_css: &str, earlier_css: &str) -> Option<f32> {
    let mut item = 0;
    let mut fixture = crate::layout::ifc::test_support::block_fixture_with_ahem(
        &format!("display:flex;flex-direction:column;line-height:50px;{column_css}"),
        |doc, column| {
            doc.append_element(Some(column), "div", Some(earlier_css));
            item = doc.append_element(
                Some(column),
                "div",
                Some(format!("display:block;height:50px;{item_css}")),
            );
            doc.append_text(item, "4");
        },
    );
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    layout_single_page(&mut fixture.doc, &fixture.cascade, page_box_800x600()).expect("layout");
    let session = LayoutSession::resumed(&mut fixture.doc, &fixture.cascade);
    piece(&session, &fixture.cascade, body, item)
}

#[test]
fn ordinary_column_items_keep_the_first_line_as_their_first_piece() {
    assert_eq!(measured_piece("", "", "display:block"), Some(50.0));
    for alignment in ["normal", "start", "flex-start"] {
        assert_eq!(
            measured_piece(&format!("justify-content:{alignment}"), "", "display:block"),
            Some(50.0)
        );
    }
}

#[test]
fn other_layout_and_height_contracts_keep_their_existing_pagination() {
    for css in [
        "display:block",
        "flex-direction:row",
        "flex-direction:column-reverse",
        "flex-wrap:wrap",
        "writing-mode:vertical-rl",
        "justify-content:center",
        "height:200px",
        "min-height:200px",
        "max-height:200px",
        "aspect-ratio:1/1",
        "contain:size",
        "position:absolute",
    ] {
        assert_eq!(measured_piece(css, "", "display:block"), None, "{css}");
    }
    for css in ["writing-mode:vertical-rl", "position:relative", "order:1"] {
        assert_eq!(measured_piece("", css, "display:block"), None, "{css}");
    }
    assert_eq!(measured_piece("", "", "display:block;order:1"), None);
}
