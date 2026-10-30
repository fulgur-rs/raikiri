//! Public DOM construction, with timing separate from work/allocation probes.
//! Run cargo bench -p raikiri-bench --bench insertion_cycle.

// cov:ignore: standalone benchmark is not built by workspace cargo test.
mod measurement {
    use raikiri_dom::{Document, begin_insertion_diagnostics, finish_insertion_diagnostics};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::hint::black_box;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::Relaxed};
    use std::time::Instant;

    static LIVE: AtomicIsize = AtomicIsize::new(0);
    static PEAK: AtomicIsize = AtomicIsize::new(0);
    static COUNT: AtomicBool = AtomicBool::new(false);
    static ALLOCS: AtomicU64 = AtomicU64::new(0);
    static REALLOCS: AtomicU64 = AtomicU64::new(0);
    static REQUESTED: AtomicU64 = AtomicU64::new(0);
    struct Counting;
    fn track(delta: isize, requested: usize, realloc: bool) {
        let live = LIVE.fetch_add(delta, Relaxed) + delta;
        if COUNT.load(Relaxed) {
            PEAK.fetch_max(live, Relaxed);
            REQUESTED.fetch_add(requested as u64, Relaxed);
            if realloc {
                REALLOCS.fetch_add(1, Relaxed);
            } else {
                ALLOCS.fetch_add(1, Relaxed);
            }
        }
    }
    // SAFETY: arguments and allocation results are forwarded unchanged to
    // System. Atomic accounting neither allocates nor accesses the payload.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds System's allocation contract.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                track(layout.size() as isize, layout.size(), false);
            }
            ptr
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds System's allocation contract.
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                track(layout.size() as isize, layout.size(), false);
            }
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: ptr/layout describe a live allocation owned by System.
            unsafe { System.dealloc(ptr, layout) };
            LIVE.fetch_sub(layout.size() as isize, Relaxed);
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller upholds System's reallocation contract.
            let new = unsafe { System.realloc(ptr, layout, size) };
            if !new.is_null() {
                track(size as isize - layout.size() as isize, size, true);
            }
            new
        }
    }
    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    #[derive(Clone, Copy, Debug)]
    enum Mode {
        Wrap,
        WideLeaf,
        DeepLeaf,
    }

    fn fill(d: &mut Document, original: usize, n: usize, mode: Mode) -> usize {
        let mut current = original;
        for _ in 0..n {
            let fresh = d.create_detached_element("div").unwrap();
            match mode {
                Mode::Wrap => {
                    d.append_child(fresh, current).unwrap();
                    current = fresh;
                }
                Mode::WideLeaf => {
                    d.append_child(original, fresh).unwrap();
                }
                Mode::DeepLeaf => {
                    d.append_child(current, fresh).unwrap();
                    current = fresh;
                }
            }
        }
        current
    }
    fn validate(d: &Document, original: usize, last: usize, n: usize, mode: Mode) {
        assert_eq!(d.node_count(), n + 2);
        let root = if matches!(mode, Mode::Wrap) {
            last
        } else {
            original
        };
        assert_eq!(d.preorder(root).count(), n + 1);
        if matches!(mode, Mode::WideLeaf) {
            assert_eq!(d.get_node(root).unwrap().children.len(), n);
        } else {
            let mut at = if matches!(mode, Mode::Wrap) {
                original
            } else {
                last
            };
            let mut depth = 0;
            while let Some(parent) = d.parent_of(at) {
                at = parent;
                depth += 1;
            }
            assert_eq!(depth, n);
            assert_eq!(at, root);
        }
    }
    pub fn run() {
        for n in [1000, 2000, 4000, 8000] {
            for mode in [Mode::Wrap, Mode::WideLeaf, Mode::DeepLeaf] {
                let mut timings = Vec::new();
                for sample in 0..4 {
                    let mut d = Document::new();
                    let original = d.create_detached_element("div").unwrap();
                    let start = Instant::now();
                    let last = fill(black_box(&mut d), original, black_box(n), mode);
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    validate(&d, original, last, n, mode);
                    if sample > 0 {
                        timings.push(ms);
                    }
                    black_box(d);
                }
                let mut d = Document::new();
                let original = d.create_detached_element("div").unwrap();
                let baseline = LIVE.load(Relaxed);
                PEAK.store(baseline, Relaxed);
                ALLOCS.store(0, Relaxed);
                REALLOCS.store(0, Relaxed);
                REQUESTED.store(0, Relaxed);
                COUNT.store(true, Relaxed);
                let last = fill(&mut d, original, n, mode);
                COUNT.store(false, Relaxed);
                let retained = LIVE.load(Relaxed) - baseline;
                let peak = PEAK.load(Relaxed) - baseline;
                let allocs = ALLOCS.load(Relaxed);
                let reallocs = REALLOCS.load(Relaxed);
                let requested = REQUESTED.load(Relaxed);
                validate(&d, original, last, n, mode);
                drop(d);

                let mut d = Document::new();
                let original = d.create_detached_element("div").unwrap();
                begin_insertion_diagnostics();
                let last = fill(&mut d, original, n, mode);
                let work = finish_insertion_diagnostics();
                validate(&d, original, last, n, mode);
                println!(
                    "n={n} mode={mode:?} raw_ms={timings:?} retained_requested_bytes={retained} peak_requested_bytes={peak} alloc_calls={allocs} realloc_calls={reallocs} cumulative_requested_bytes={requested} work={work:?}"
                );
            }
        }
    }
}

// cov:ignore: standalone benchmark entry point is not built by cargo test.
fn main() {
    measurement::run();
}
