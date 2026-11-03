//! Layout-only measurements for many ordinary blocks crossing page edges.

#[path = "support/allocation_tests.rs"]
mod support;

use raikiri_dom::{Document, layout::layout_pages};
use raikiri_html::{FontCollectionBuilder, RenderResources};
use raikiri_traits::PageBox;

#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;

#[test]
fn normal_block_pagination_measurements_preserve_pages_positions_and_text() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    for count in [128, 256, 512, 1024] {
        for sample in 0..3 {
            let mut doc = Document::new();
            doc.set_font_collection(resources.inline_engine_fonts());
            doc.set_ifc_parallel_build(false);
            let html = doc.append_element(Some(0), "html", Some("display:block"));
            let body = doc.append_element(
                Some(html),
                "body",
                Some("display:block;margin:0;width:100px;font:20px/30px Ahem"),
            );
            let section = doc.append_element(Some(body), "section", Some("display:block"));
            let mut blocks = Vec::with_capacity(count);
            for _ in 0..count {
                let block = doc.append_element(
                    Some(section),
                    "div",
                    Some("display:block;height:60px;break-inside:avoid"),
                );
                doc.append_text(block, "x");
                blocks.push(block);
            }
            doc.mark_in_document_flags();
            let cascade =
                raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
            let mut page = PageBox::new();
            page.width = 100.0;
            page.height = 100.0;
            let ((pages, elapsed), allocation) = support::allocated_by(|| {
                let start = std::time::Instant::now();
                let pages = layout_pages(&mut doc, &cascade, page).unwrap();
                (pages, start.elapsed())
            });
            assert_eq!(pages.len(), count);
            doc.project_pages(&cascade, page, &pages, &[]).unwrap();
            for (index, &block) in blocks.iter().enumerate() {
                let layout = doc.get_node(block).unwrap().layout();
                assert_eq!(layout.location.y, index as f32 * 100.0);
                assert_eq!(layout.size.height, 60.0);
                let text: String = doc
                    .page_text_runs(&cascade, index as u32)
                    .iter()
                    .map(|run| run.text)
                    .collect();
                assert_eq!(text, "x");
            }
            eprintln!(
                "normal_block_pagination count={count} sample={sample} allocations={} allocated_bytes={} elapsed_ns={}",
                allocation.calls,
                allocation.bytes,
                elapsed.as_nanos()
            );
        }
    }
}
