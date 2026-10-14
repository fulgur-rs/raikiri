//! Paged layout and consumer page-output extraction, excluding parsing and cascade.
//! Layout includes text construction and fragmentation, not a pagination-only pass.

// cov:ignore: bench-only item; workspace tests do not instrument this target.
mod common {
    pub mod workloads;
}

use common::workloads::{self, Shape};
use criterion::{Criterion, Throughput};
// cov:ignore: benchmark-only imports; this target is absent from workspace coverage.
use raikiri_html::{
    DocumentLayout, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources,
    layout,
};
use std::{cell::OnceCell, hint::black_box};

// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn paged(document: &raikiri::HtmlDocument, resources: &RenderResources<'_>) -> DocumentLayout {
    let LayoutStatus::Completed(result) = layout(
        document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .expect("paged layout") else {
        panic!("layout aborted without a signal");
    };
    result
}

// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn bench_pages(c: &mut Criterion) {
    let resources = workloads::resources();
    for stage in ["pagination_document", "page_output_document"] {
        let mut group = c.benchmark_group(stage);
        group.sample_size(10);
        for (name, shape) in [
            ("forced_pages", None),
            ("inline", Some(Shape::Inline)),
            ("table", Some(Shape::TableFixed)),
            ("multicol", Some(Shape::Multicol)),
        ] {
            for size in [10, 100] {
                let slot = OnceCell::new();
                group.throughput(Throughput::Elements(
                    if shape.is_none() { size } else { size * 10 } as u64,
                ));
                group.bench_function(format!("{name}_{size}"), |b| {
                    let (document, result) = slot.get_or_init(|| {
                        let html = shape.map_or_else(
                            || workloads::pages(size),
                            |shape| workloads::document(shape, size * 10),
                        );
                        let document = workloads::parse(&html, &resources);
                        if let Some(shape) = shape {
                            workloads::check_shape(document.dom(), document.cascade(), shape);
                        }
                        let result = paged(&document, &resources);
                        if shape.is_none() {
                            assert_eq!(result.page_count() as usize, size, "forced page count");
                        } else {
                            assert!(result.page_count() > 1, "the fixture must paginate");
                        }
                        let text_pages = result
                            .pages()
                            .filter(|page| !page.text_runs().is_empty())
                            .count();
                        assert_eq!(
                            text_pages,
                            result.page_count() as usize,
                            "every page carries text"
                        );
                        let mut text = Vec::new();
                        for page in result.pages() {
                            for run in page.text_runs() {
                                assert!(!run.glyphs.is_empty(), "text run has no glyphs");
                                text.extend(
                                    run.text.bytes().filter(|byte| !byte.is_ascii_whitespace()),
                                );
                            }
                        }
                        text.sort_unstable();
                        assert_eq!(
                            text,
                            workloads::text_signature(document.dom()),
                            "paged output preserves text"
                        );
                        (document, result)
                    });
                    if stage == "pagination_document" {
                        b.iter(|| black_box(paged(black_box(document), &resources)));
                    } else {
                        b.iter(|| {
                            for page in black_box(result).pages() {
                                for fragment in page.fragments() {
                                    black_box(fragment);
                                }
                                let runs = page.text_runs();
                                black_box(page.paint_order_for_text_runs(&runs));
                                black_box(runs);
                            }
                        });
                    }
                });
            }
        }
        group.finish();
    }
}

// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_pages(&mut c);
    c.final_summary();
}
