//! Child replacement and text-append throughput, excluding document
//! construction and drop.
//!
//! Run with `cargo bench -p raikiri-dom --bench mutation`.

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};
use raikiri_dom::Document;
use taffy::Style;

// Put the target after a populated sibling subtree so repeated parent lookup
// must traverse both preceding nodes and the target's shrinking child list.
fn target_document(children: usize) -> (Document, usize, usize) {
    let mut doc = Document::new();
    let preceding = doc.append_element(Some(0), "aside", Style::default(), None::<&str>);
    for _ in 0..children {
        doc.append_text(preceding, "unrelated");
    }
    let parent = doc.append_element(Some(0), "main", Style::default(), None::<&str>);
    let first_old = doc.append_text(parent, "old");
    for _ in 1..children {
        doc.append_text(parent, "old");
    }
    doc.mark_in_document_flags();
    (doc, parent, first_old)
}

fn bench_replace_children(c: &mut Criterion) {
    let mut group = c.benchmark_group("replace_children");
    for (name, children, replacement_children) in [
        ("clear", 500, 0),
        ("clear", 1000, 0),
        ("clear", 2000, 0),
        ("clear", 4000, 0),
        ("replace", 2000, 2000),
        ("replace", 4000, 4000), // cov:ignore: bench harness — new bench row, never instrumented
    ] {
        let (target, parent, first_old) = target_document(children);
        let mut source = Document::new();
        for _ in 0..replacement_children {
            source.append_text(0, "new");
        }

        let mut probe = target.clone();
        probe.replace_children_from(parent, &source, 0);
        assert_eq!(probe.parent_of(first_old), None);
        assert_eq!(probe.parent_of(first_old + children - 1), None);
        assert_eq!(
            probe.node_count(),
            target.node_count() + replacement_children
        );
        assert_eq!(
            probe.serialize_inner_html(parent).unwrap(),
            "new".repeat(replacement_children)
        );
        assert_eq!(
            source.serialize_inner_html(0).unwrap(),
            "new".repeat(replacement_children)
        );

        group.throughput(Throughput::Elements(children as u64));
        group.bench_with_input(BenchmarkId::new(name, children), &children, |b, _| {
            b.iter_batched_ref(
                || target.clone(),
                |doc| doc.replace_children_from(parent, &source, 0),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

// One empty `main` parent under the document root; appends below measure
// per-call text-node insertion, not tree construction.
// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn append_target() -> (Document, usize) {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(0), "main", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    (doc, parent)
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn bench_text_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_append");
    // `split` appends N one-character text nodes (one arena node per call);
    // `coalesced` merges each call into the trailing text node (one arena
    // node total, at the cost of re-copying the merged string per call).
    // The pair bounds the split-text-append optimization candidate from both
    // sides: per-call insertion cost vs retained node count.
    for name in ["split", "coalesced"] {
        for n in [500usize, 1000, 2000, 4000] {
            let (target, parent) = append_target();
            let base_nodes = target.node_count();

            let mut probe = target.clone();
            for _ in 0..n {
                if name == "split" {
                    probe.append_text(parent, "x");
                } else {
                    probe.append_text_coalesced(parent, "x");
                }
            }
            assert_eq!(
                probe.node_count(),
                base_nodes + if name == "split" { n } else { 1 }
            );
            assert_eq!(probe.serialize_inner_html(parent).unwrap(), "x".repeat(n));

            group.throughput(Throughput::Elements(n as u64));
            group.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
                b.iter_batched_ref(
                    || target.clone(),
                    |doc| {
                        for _ in 0..n {
                            if name == "split" {
                                doc.append_text(parent, "x");
                            } else {
                                doc.append_text_coalesced(parent, "x");
                            }
                        }
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_replace_children(&mut criterion);
    bench_text_append(&mut criterion);
    criterion.final_summary();
}
