//! Scene extraction, paint command recording, and paint/raster/PNG after layout.
//! Parse/cascade/layout are untimed; timed outputs include their destruction.

mod common {
    pub mod workloads;
}

use anyrender::{Scene, recording::RenderCommand};
use common::workloads::{self, Shape};
use criterion::{Criterion, Throughput};
use raikiri::{PageBox, build_page_scene};
use std::{cell::OnceCell, hint::black_box};

fn record(dom: &raikiri::Document, cascade: &raikiri::CascadeResult, page: PageBox) -> Scene {
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, dom, cascade, page).expect("paint");
    scene
}

fn bench_paint(c: &mut Criterion) {
    let resources = workloads::resources();
    let mut page = PageBox::new();
    page.width = 640.0;
    page.height = 800.0;
    for stage in ["scene_document", "paint_document", "raster_png_document"] {
        let mut group = c.benchmark_group(stage);
        group.sample_size(10);
        // Both sizes fit on the first page, so raster work cannot shrink through clipping.
        for items in [5, 20] {
            let slot = OnceCell::new();
            group.throughput(Throughput::Elements(items as u64));
            group.bench_function(format!("text_{items}"), |b| {
                let (document, cascade, scene) = slot.get_or_init(|| {
                    let html = workloads::document(Shape::Text, items);
                    let (mut document, cascade) = workloads::single_page_input(&html, &resources);
                    raikiri_dom::layout_single_page(&mut document.dom, &cascade, page)
                        .expect("layout");
                    workloads::check_layout(&document.dom, items);
                    for id in 0..document.dom.node_count() {
                        let node = document.dom.get_node(id).unwrap();
                        if node.tag_name() == Some("p") {
                            // The fixture has 20px page margins and one unshifted container.
                            let bounds = &node.unrounded_layout;
                            assert!(
                                bounds.location.y + bounds.size.height <= page.height - 40.0,
                                "paint paragraph {id} overflows the content box"
                            );
                        }
                    }
                    let recorded = record(&document.dom, &cascade, page);
                    let runs = recorded
                        .commands
                        .iter()
                        .filter(|command| matches!(command, RenderCommand::GlyphRun(_)))
                        .count();
                    assert!(runs >= items, "paint lost paragraphs");
                    let glyphs: usize = recorded
                        .commands
                        .iter()
                        .filter_map(|command| {
                            if let RenderCommand::GlyphRun(run) = command {
                                Some(run.glyphs.len())
                            } else {
                                None
                            }
                        })
                        .sum();
                    assert!(
                        glyphs >= workloads::text_signature(&document.dom).len(),
                        "paint lost glyphs"
                    );
                    let scene = build_page_scene(&document.dom, &cascade, page);
                    assert!(
                        scene.root_id.is_some() && scene.body_id.is_some(),
                        "scene has roots"
                    );
                    for id in 0..document.dom.node_count() {
                        if document.dom.get_node(id).unwrap().tag_name() == Some("p") {
                            let node_id = raikiri::NodeId::new(id as u64);
                            assert!(
                                scene.node_ids.contains(&node_id),
                                "scene lost paragraph {id}"
                            );
                            assert!(
                                scene
                                    .fragments
                                    .get(&node_id)
                                    .is_some_and(|fragments| !fragments.is_empty()),
                                "scene lost paragraph geometry {id}"
                            );
                        }
                    }
                    let png = scene
                        .rasterize(&document.dom, &cascade, page)
                        .expect("raster probe");
                    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
                    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 640);
                    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 800);
                    (document, cascade, scene)
                });
                match stage {
                    "scene_document" => {
                        b.iter(|| black_box(build_page_scene(&document.dom, cascade, page)))
                    }
                    "paint_document" => b.iter(|| black_box(record(&document.dom, cascade, page))),
                    _ => b.iter(|| {
                        black_box(
                            scene
                                .rasterize(&document.dom, cascade, page)
                                .expect("raster and encode"),
                        )
                    }),
                }
            });
        }
        group.finish();
    }
}

fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_paint(&mut c);
    c.final_summary();
}
