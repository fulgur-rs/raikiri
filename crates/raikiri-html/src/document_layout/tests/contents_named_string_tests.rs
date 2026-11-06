use super::*;

#[test]
fn contents_named_strings_public_html_page_start() {
    for prefix in [
        "<span style='display:contents'></span>",
        "<div style='display:contents'><span style='display:contents'></span></div>",
    ] {
        let document = dom(&format!(
            "<style>@page{{size:300px 200px;margin:40px;@top-left{{content:'V:' string(title,start)}}}}body{{margin:0;font:10px/10px serif}}h2{{margin:0;string-set:title content(text)}}</style><body>{prefix}<h2>A1</h2><h2>A2</h2></body>"
        ));
        let out = completed(
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new(),
            )
            .unwrap(),
        );
        assert_eq!(out.page_count(), 1);
        let boxes = out.page(0).unwrap().margin_boxes();
        assert_eq!(boxes[0].content, "V:A1", "{prefix}");
    }
}

#[test]
fn contents_named_strings_loaded_generated_image_keeps_single_page_entry() {
    for pseudo in ["before", "after"] {
        let document = dom(&format!(
            "<style>@page{{size:300px 200px;margin:40px}}body{{margin:0}}h2{{margin:0;string-set:title content(text)}}.c::{pseudo}{{content:url(data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22/%3E)}}</style><body><span class=c style='display:contents'></span><h2>A1</h2></body>"
        ));
        let out = completed(
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new(),
            )
            .unwrap(),
        );
        let page = out.page(0).unwrap();
        assert!(
            page.paint_order_for_text_runs(&page.text_runs())
                .iter()
                .any(|event| matches!(event, crate::PaintEvent::GeneratedImage(_))),
            "{pseudo}"
        );
        let source = out.running_source();
        let index = raikiri_dom::RunningIndex::build_single_page(source.document, source.cascade);
        assert_eq!(
            index.select_string("title", raikiri_style::property::StringFetchMode::Start, 0),
            None,
            "{pseudo}"
        );
    }
}
