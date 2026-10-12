//! Cascade time and retained heap for repeated and divergent pseudo outputs.
//!
//! Run with `cargo bench -p raikiri-bench --bench pseudo_output_sharing`.
//! Parsing and rule-tree construction happen before each measurement. Each
//! fixture gets one untimed warmup cascade, five wall-time samples, and three
//! separate allocator-retained-heap samples; the median is reported.

// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace` never
// build this bench target, so nothing in this file has coverage instrumentation.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashSet;
use std::hint::black_box;
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use std::time::Instant;

use raikiri_html::{ParseOptions, UncascadedDocument};
use raikiri_style::{
    CascadeResult, ComputedValues, MediaContext, PseudoElem, StyleDom, StyleElement, StyleNode,
    StyleNodeId, StyleNodeKind,
};

const ELEMENT_COUNTS: [usize; 3] = [1_000, 10_000, 100_000];
const TIME_SAMPLES: usize = 5;
const HEAP_SAMPLES: usize = 3;

struct Counting;

thread_local! {
    static MEASURING: Cell<bool> = const { Cell::new(false) };
}

static LIVE: AtomicIsize = AtomicIsize::new(0);

fn measuring() -> bool {
    MEASURING.try_with(Cell::get).unwrap_or(false)
}

fn record_allocation(size: usize) {
    if measuring() {
        LIVE.fetch_add(isize::try_from(size).unwrap_or(isize::MAX), Relaxed);
    }
}

fn record_release(size: usize) {
    if measuring() {
        LIVE.fetch_sub(isize::try_from(size).unwrap_or(isize::MAX), Relaxed);
    }
}

// SAFETY: every method forwards its arguments unchanged to `System` and only
// updates counters on the side, so `System`'s guarantees carry over.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by this allocator for `layout`.
        unsafe { System.dealloc(ptr, layout) };
        record_release(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            record_release(layout.size());
            record_allocation(new_size);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

trait PseudoOutput {
    fn computed_values(&self) -> &ComputedValues;
}

impl PseudoOutput for ComputedValues {
    fn computed_values(&self) -> &ComputedValues {
        self
    }
}

impl PseudoOutput for std::sync::Arc<ComputedValues> {
    fn computed_values(&self) -> &ComputedValues {
        self.as_ref()
    }
}

impl PseudoOutput for triomphe::Arc<ComputedValues> {
    fn computed_values(&self) -> &ComputedValues {
        self.as_ref()
    }
}

struct Fixture {
    document: UncascadedDocument,
    tree: raikiri_style::RuleTree,
    span_ids: Vec<StyleNodeId>,
    element_nodes: usize,
}

#[derive(Clone, Copy)]
enum Shape {
    IdenticalInputs,
    InheritedColorVaries,
    OrdinaryWidthVaries,
}

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Self::IdenticalInputs => "identical_inputs",
            Self::InheritedColorVaries => "inherited_color_varies",
            Self::OrdinaryWidthVaries => "ordinary_width_varies",
        }
    }

    fn style(self, index: usize, html: &mut String) {
        match self {
            Self::IdenticalInputs => {}
            Self::InheritedColorVaries => {
                html.push_str(" style=\"color:#");
                html.push_str(&format!("{index:06x}"));
                html.push('"');
            }
            Self::OrdinaryWidthVaries => {
                html.push_str(" style=\"width:");
                html.push_str(&format!("{}px", index + 1));
                html.push('"');
            }
        }
    }
}

fn fixture(elements: usize, shape: Shape) -> Fixture {
    let mut html = String::with_capacity(elements.saturating_mul(36));
    html.push_str(
        "<!doctype html><html><head><style>span::before { content: 'x' }</style></head><body>",
    );
    for index in 0..elements {
        html.push_str("<span");
        shape.style(index, &mut html);
        html.push_str("></span>");
    }
    html.push_str("</body></html>");

    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let document = raikiri_html::parse(html.as_bytes(), &options).expect("the fixture parses");
    let tree = raikiri_html::build_rule_tree(&document);
    let mut span_ids = Vec::with_capacity(elements);
    let mut element_nodes = 0;
    for raw_id in 0..document.dom.node_count() {
        let id = StyleNodeId::new(raw_id as u64);
        let Some(node) = document.dom.node(id) else {
            continue;
        };
        if node.kind() == StyleNodeKind::Element {
            element_nodes += 1;
            if node
                .as_element()
                .is_some_and(|element| element.tag_name() == "span")
            {
                span_ids.push(id);
            }
        }
    }
    assert_eq!(span_ids.len(), elements);
    Fixture {
        document,
        tree,
        span_ids,
        element_nodes,
    }
}

fn measure_retained<T>(f: impl FnOnce() -> T) -> (T, usize) {
    LIVE.store(0, Relaxed);
    MEASURING.set(true);
    let value = f();
    MEASURING.set(false);
    (value, usize::try_from(LIVE.load(Relaxed)).unwrap_or(0))
}

fn median<T: Ord>(mut values: Vec<T>) -> T {
    values.sort_unstable();
    values.swap_remove(values.len() / 2)
}

fn pseudo_values(result: &CascadeResult) -> impl Iterator<Item = &ComputedValues> {
    result.pseudo.values().map(PseudoOutput::computed_values)
}

fn pseudo_value(result: &CascadeResult, id: StyleNodeId) -> &ComputedValues {
    result
        .pseudo
        .get(&(id, PseudoElem::Before))
        .expect("the span has a matched ::before output")
        .computed_values()
}

fn validate_shape(fixture: &Fixture, result: &CascadeResult, shape: Shape) -> usize {
    let elements = fixture.span_ids.len();
    assert_eq!(result.pseudo.len(), elements);
    let first = fixture.span_ids[0];
    let second = fixture.span_ids[1];
    match shape {
        Shape::IdenticalInputs => {
            assert_eq!(
                result.computed[first.0 as usize],
                result.computed[second.0 as usize]
            );
            assert_eq!(pseudo_value(result, first), pseudo_value(result, second));
        }
        Shape::InheritedColorVaries => {
            assert_ne!(
                result.computed[first.0 as usize].color,
                result.computed[second.0 as usize].color
            );
            assert_ne!(
                pseudo_value(result, first).color,
                pseudo_value(result, second).color
            );
        }
        Shape::OrdinaryWidthVaries => {
            assert_ne!(
                result.computed[first.0 as usize].width,
                result.computed[second.0 as usize].width
            );
            assert_eq!(pseudo_value(result, first), pseudo_value(result, second));
        }
    }
    let unique: HashSet<usize> = pseudo_values(result)
        .map(|values| values as *const ComputedValues as usize)
        .collect();
    unique.len()
}

fn benchmark_case(elements: usize, shape: Shape) {
    let fixture = fixture(elements, shape);
    let media = MediaContext::default();
    let cascade = || {
        raikiri_style::cascade_with_media_context(&fixture.document.dom, &fixture.tree, &media)
            .expect("the cascade succeeds")
    };

    let warmed = cascade();
    validate_shape(&fixture, &warmed, shape);
    drop(warmed);

    let mut elapsed = Vec::with_capacity(TIME_SAMPLES);
    for _ in 0..TIME_SAMPLES {
        let start = Instant::now();
        let result = cascade();
        black_box((result.computed.len(), result.pseudo.len()));
        elapsed.push(start.elapsed());
        drop(result);
    }

    let mut retained = Vec::with_capacity(HEAP_SAMPLES);
    let mut pseudo_entries = 0;
    let mut unique_outputs = 0;
    for _ in 0..HEAP_SAMPLES {
        let (result, bytes) = measure_retained(cascade);
        pseudo_entries = result.pseudo.len();
        unique_outputs = validate_shape(&fixture, &result, shape);
        retained.push(bytes);
        drop(result);
    }

    let elapsed = median(elapsed);
    let retained = median(retained);
    println!(
        "{},{},{},{},{},{},{:.3},{}",
        shape.name(),
        elements,
        fixture.element_nodes,
        fixture.document.dom.node_count(),
        pseudo_entries,
        unique_outputs,
        elapsed.as_secs_f64() * 1_000.0,
        retained
    );
}

fn main() {
    println!(
        "# profile=release; warmup=1 cascade per fixture; time_samples={TIME_SAMPLES}; heap_samples={HEAP_SAMPLES}; median; parse and rule-tree construction excluded"
    );
    println!(
        "shape,span_elements,all_element_nodes,dom_nodes,pseudo_entries,unique_pseudo_values,median_cascade_ms,median_retained_heap_bytes"
    );
    for shape in [
        Shape::IdenticalInputs,
        Shape::InheritedColorVaries,
        Shape::OrdinaryWidthVaries,
    ] {
        for elements in ELEMENT_COUNTS {
            benchmark_case(elements, shape);
        }
    }
}
