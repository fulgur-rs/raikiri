use super::*;

#[test]
fn many_inline_roots_share_xml_and_picture_beyond_the_prepared_index_limit() {
    let icon = format!(
        "<svg width='1' height='1' data-pad='{}'><rect width='1' height='1' fill='red'/></svg>",
        "x".repeat(1024)
    );
    let html = format!(
        "<style>@page{{size:400px 400px;margin:0}}body{{margin:0;font-size:1px;line-height:1px}}</style>{}",
        icon.repeat(300)
    );
    with_first_page(&html, LayoutOptions::new(), |page| {
        let fragments: Vec<_> = page
            .fragments()
            .filter(|fragment| page.dom().local_name(fragment.node()) == Some("svg"))
            .collect();
        assert_eq!(fragments.len(), 300);
        let (_, first) = page.prepared_inline_svg(&fragments[0]).unwrap().unwrap();
        let ImageContent::Vector(scene) = page.inline_svg(&fragments[0]).unwrap().unwrap().content
        else {
            panic!("scene")
        };
        let first_source_bytes = page.svg_sources.charges().1;
        for fragment in &fragments {
            let (_, prepared) = page.prepared_inline_svg(fragment).unwrap().unwrap();
            assert!(std::sync::Arc::ptr_eq(&first.source, &prepared.source));
            let ImageContent::Vector(same) = page.inline_svg(fragment).unwrap().unwrap().content
            else {
                panic!("scene")
            };
            assert!(std::sync::Arc::ptr_eq(&scene, &same));
        }
        assert_eq!(page.svg_sources.len(), 256);
        assert_eq!(page.svg_sources.kept_pictures(), 1);
        assert_eq!(page.svg_sources.charges().1, first_source_bytes);
        assert_eq!(page.svg_sources.charges().2, scene.estimated_heap_bytes());
    });
}
