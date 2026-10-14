//! Fresh single-page layout by formatting context; parse/cascade and drops are untimed.
//! A fresh DOM per iteration avoids timing a cached relayout. Fonts are bundled Ahem.

// cov:ignore: bench-only item; workspace tests do not instrument this target.
mod common {
    pub mod workloads;
}

use common::workloads::{self, Shape};
use criterion::{BatchSize, Criterion, Throughput};
use raikiri::PageBox;
use std::{cell::OnceCell, hint::black_box};

// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn bench_layout(c: &mut Criterion) {
    let resources = workloads::resources();
    let mut page = PageBox::new();
    page.width = 640.0;
    page.height = 480.0;
    let mut group = c.benchmark_group("layout_document");
    group.sample_size(20);
    for shape in [
        Shape::Text,
        Shape::Inline,
        Shape::TableFixed,
        Shape::TableAuto,
        Shape::TableSpans,
        Shape::Flex,
        Shape::Grid,
        Shape::Multicol,
    ] {
        for items in [100, 1000] {
            let slot = OnceCell::new();
            group.throughput(Throughput::Elements(items as u64));
            group.bench_function(format!("{}_{items}", shape.name()), |b| {
                let html = slot.get_or_init(|| {
                    let html = workloads::document(shape, items);
                    let (mut document, cascade) = workloads::single_page_input(&html, &resources);
                    workloads::check_shape(&document.dom, &cascade, shape);
                    raikiri_dom::layout_single_page(&mut document.dom, &cascade, page)
                        .expect("probe layout");
                    workloads::check_layout(&document.dom, items);
                    html
                });
                b.iter_batched_ref(
                    || workloads::single_page_input(html, &resources),
                    |(document, cascade)| {
                        raikiri_dom::layout_single_page(
                            &mut document.dom,
                            cascade,
                            black_box(page),
                        )
                        .expect("layout");
                        black_box(&document.dom);
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

// cov:ignore: bench-only item; workspace tests do not instrument this target.
fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_layout(&mut c);
    c.final_summary();
}
