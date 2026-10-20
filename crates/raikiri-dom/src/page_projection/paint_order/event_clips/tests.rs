use super::*;
use crate::layout::ifc::test_support::block_fixture_with_ahem;
use crate::{PageSlice, layout_single_page};
use raikiri_traits::{PageBox, PaintRect};

#[test]
fn an_event_keeps_its_line_clip_outside_the_glyphs_own_page() {
    let fixture = block_fixture_with_ahem("width:10px;line-height:10px", |doc, root| {
        doc.append_text(root, "X X");
    });
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    let mut page_box = PageBox::new();
    page_box.width = 100.0;
    page_box.height = 10.0;
    layout_single_page(&mut doc, &cascade, page_box).unwrap();
    let slices: Vec<_> = (0..2)
        .map(|page_index| PageSlice {
            page_index,
            content_origin_y: page_index as f32 * 10.0,
            page_name: None,
        })
        .collect();
    doc.project_pages(&cascade, page_box, &slices, &[]).unwrap();
    let placement = doc
        .page_projection
        .text_roots
        .iter_mut()
        .find(|placement| placement.node == root)
        .unwrap();
    placement.fragment_clip = Some(PaintRect::new(0.0, 0.0, 10.0, 20.0));
    placement.overflow_chain = Some(17);
    rebuild_text_index(&mut doc, &cascade);
    let page = &doc.page_projection.pages[1];
    let items = PageItems {
        by_node: HashMap::new(),
        content_box: page.content_box,
        overflow_clips: Default::default(),
        generated_boxes: None,
        inline_boxes: None,
        generated_images: None,
        cascade: &cascade,
        document: &doc,
        page,

        source_lines: None,
        paragraph_presence: Default::default(),
        full_paragraph_listing: false,
        paragraph_listings: Default::default(),
        paragraph_nodes: Default::default(),
        paragraph_slots: Default::default(),
    };
    // A sliced block pseudo box can refer to the first line on later pages.
    let line = TextLineId {
        root: NodeId::new(root as u64),
        index: 0,
    };
    let clips = LineClips::new(&doc, &cascade, &items, &[PaintEvent::TextLine(line)]);
    assert_eq!(clips.overflow_chains.get(&line), Some(&17));
    assert!(clips.clips.contains_key(&line));
}

#[test]
fn a_paginated_block_pseudo_keeps_its_placement_fragmentainer_clip() {
    let fixture = crate::layout::ifc::test_support::sheet_fixture(
        "p{display:block;width:10px;margin:0} p::before{content:'B';display:block;height:250px;background:blue}",
        "width:10px;line-height:10px",
        |doc, root| {
            let wrapper = doc.append_element(
                Some(root),
                "section",
                crate::LayoutStyle::default(),
                Some("display:block;overflow:hidden"),
            );
            let paragraph = doc.append_element(
                Some(wrapper),
                "p",
                crate::LayoutStyle::default(),
                None::<&str>,
            );
            doc.append_text(paragraph, "X");
        },
    );
    let (mut doc, cascade) = (fixture.doc, fixture.cascade);
    doc.set_font_collection(crate::layout::ifc::test_support::ahem_fonts());
    let mut page_box = PageBox::new();
    page_box.width = 100.0;
    page_box.height = 100.0;
    let slices = crate::layout_pages(&mut doc, &cascade, page_box).unwrap();
    doc.project_pages(&cascade, page_box, &slices, &[]).unwrap();
    // Explicit fragmentainer metadata is independent of the block box's
    // slicing; assign it after layout to isolate that clip contract.
    for placement in &mut doc.page_projection.text_roots {
        placement.fragment_clip = Some(PaintRect::new(0.0, 0.0, 10.0, 300.0));
    }
    rebuild_text_index(&mut doc, &cascade);
    let mut sliced = 0;
    for (position, slice) in slices.iter().enumerate() {
        let runs = doc.page_text_runs(&cascade, slice.page_index);
        let events = doc.page_paint_order_for_text_runs(&cascade, slice.page_index, &runs);
        for &event in &events {
            let PaintEvent::GeneratedBox(generated) = event else {
                continue;
            };
            if runs.iter().any(|run| run.line == generated.line) {
                continue;
            }
            let placement = doc
                .page_projection
                .text_roots
                .iter()
                .find(|placement| {
                    placement.node as u64 == generated.line.root.0
                        && placement.fragment_clip.is_some()
                })
                .unwrap();
            let page = &doc.page_projection.pages[position];
            let items = PageItems {
                by_node: HashMap::new(),
                content_box: page.content_box,
                overflow_clips: Default::default(),
                generated_boxes: None,
                inline_boxes: None,
                generated_images: None,
                cascade: &cascade,
                document: &doc,
                page,
                source_lines: None,
                paragraph_presence: Default::default(),
                full_paragraph_listing: false,
                paragraph_listings: Default::default(),
                paragraph_nodes: Default::default(),
                paragraph_slots: Default::default(),
            };
            let clips = LineClips::new(&doc, &cascade, &items, &[event]);
            assert!(placement.fragment_clip.is_some());
            assert!(clips.clips.contains_key(&generated.line));
            sliced += 1;
        }
    }
    assert!(
        sliced > 0,
        "the block pseudo must survive beyond its glyph page"
    );
}

fn rebuild_text_index(doc: &mut Document, cascade: &CascadeResult) {
    let control = crate::PageLayoutControl::default();
    let mut work = crate::layout::ProjectionWork::new(&control, 1_000_000);
    let snapshot = &doc.page_projection;
    let (index, roots_by_page) = crate::page_projection::text_index::PageTextIndex::prepare(
        doc,
        cascade,
        &snapshot.text_roots,
        &snapshot.markers,
        &snapshot.pages,
        &mut work,
    )
    .unwrap();
    doc.page_projection.text_index = index;
    doc.page_projection.text_roots_by_page = roots_by_page;
}
