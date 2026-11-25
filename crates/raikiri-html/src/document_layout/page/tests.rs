use super::*;

#[test]
fn margin_text_keeps_its_slot_when_mapping_line_roots() {
    let font = include_bytes!("../../../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");
    let fonts = crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", font.to_vec())
        .build()
        .unwrap();
    let resources = crate::RenderResources::new().fonts(fonts);
    let html: &[u8] = b"<style>@page { margin: 40px; @top-center { content: 'X'; font: 10px Ahem } }</style><p>A</p>";
    let doc = crate::parse_html_with_resources(html, &resources).unwrap();
    let crate::LayoutStatus::Completed(layout) = crate::layout(
        &doc,
        Default::default(),
        Default::default(),
        crate::LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("expected complete layout");
    };
    let page = layout.pages().next().unwrap();
    let margin = page
        .margin_boxes()
        .into_iter()
        .find(|b| b.slot == crate::PageMarginBoxSlot::TopCenter)
        .unwrap();
    let mut run = margin.text_runs().pop().unwrap();
    Page::map_run_ids(&mut run, |_| NodeId(123));
    assert_eq!(run.line.root, NodeId(123));
    assert_eq!(run.source, crate::RunSource::MarginBox(margin.slot));
    assert!(page.text_run_opacity_group(&run).is_none());
}
