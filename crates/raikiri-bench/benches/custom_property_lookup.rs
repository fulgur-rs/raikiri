//! Inherited custom-property lookup work, allocations and cascade-only time.
//! Run cargo bench -p raikiri-bench --bench custom_property_lookup.
//! RAIKIRI_PROPERTY_DEPTHS selects depths (default1024,2048,4096).
//! Preparation and result destruction are outside each timed cascade.

// cov:ignore: standalone benchmark target is not instrumented by workspace coverage.
mod accounting {
    use raikiri_dom::Document;
    use raikiri_style::{
        CascadeResult, ComputedLengthPercentageOrAuto, begin_custom_property_diagnostics,
        build_rule_tree, cascade, finish_custom_property_diagnostics,
    };
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::Relaxed};
    use std::time::Instant;

    static TRACK_HEAP: AtomicBool = AtomicBool::new(false);
    static COUNT_REQUESTS: AtomicBool = AtomicBool::new(false);
    static LIVE: AtomicIsize = AtomicIsize::new(0);
    static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    static REALLOC_CALLS: AtomicU64 = AtomicU64::new(0);

    struct Counting;

    fn allocation(size: usize) {
        if TRACK_HEAP.load(Relaxed) {
            LIVE.fetch_add(size as isize, Relaxed);
        }
        if COUNT_REQUESTS.load(Relaxed) {
            ALLOC_BYTES.fetch_add(size as u64, Relaxed);
            ALLOC_CALLS.fetch_add(1, Relaxed);
        }
    }

    // SAFETY: every method forwards System's arguments and results unchanged;
    // the side counters do not allocate or access the allocation's contents.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds GlobalAlloc's allocation contract.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                allocation(layout.size());
            }
            ptr
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller upholds GlobalAlloc's allocation contract.
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                allocation(layout.size());
            }
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: ptr and layout satisfy System's deallocation contract.
            unsafe { System.dealloc(ptr, layout) };
            if TRACK_HEAP.load(Relaxed) {
                LIVE.fetch_sub(layout.size() as isize, Relaxed);
            }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller upholds GlobalAlloc's reallocation contract.
            let new = unsafe { System.realloc(ptr, layout, size) };
            if !new.is_null() {
                if TRACK_HEAP.load(Relaxed) {
                    LIVE.fetch_add(size as isize - layout.size() as isize, Relaxed);
                }
                if COUNT_REQUESTS.load(Relaxed) {
                    REALLOC_BYTES.fetch_add(size as u64, Relaxed);
                    REALLOC_CALLS.fetch_add(1, Relaxed);
                }
            }
            new
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    fn document(depth: usize, unique: bool, mode: &str) -> Document {
        let mut doc = Document::new();
        let mut parent = doc.append_element(Some(doc.root_index()), "html", Some("--base:7px"));
        for index in 0..depth {
            let name = if unique {
                format!("--layer{index}")
            } else {
                "--layer".to_owned()
            };
            let width = match mode {
                "hit" => "var(--base)",
                "miss" => "var(--missing,7px)",
                _ => "7px",
            };
            parent = doc.append_element(
                Some(parent),
                "div",
                Some(format!("{name}:{index};width:{width}")),
            );
        }
        doc.mark_in_document_flags();
        doc
    }

    fn check(result: &CascadeResult, depth: usize) {
        assert_eq!(result.computed.len(), depth + 2);
        for value in &result.computed[2..] {
            assert_eq!(value.width, ComputedLengthPercentageOrAuto::Px(7.0));
        }
    }

    fn measure(depth: usize, unique: bool, mode: &str) {
        let doc = document(depth, unique, mode);
        let rules = build_rule_tree(&doc);
        LIVE.store(0, Relaxed);
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOC_CALLS.store(0, Relaxed);
        REALLOC_CALLS.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        COUNT_REQUESTS.store(true, Relaxed);
        begin_custom_property_diagnostics();
        let result = cascade(&doc, &rules).expect("cascade fits its budgets");
        let work = finish_custom_property_diagnostics();
        COUNT_REQUESTS.store(false, Relaxed);
        let retained = LIVE.load(Relaxed);
        let allocated = ALLOC_BYTES.load(Relaxed);
        let reallocated = REALLOC_BYTES.load(Relaxed);
        let calls = ALLOC_CALLS.load(Relaxed);
        let realloc_calls = REALLOC_CALLS.load(Relaxed);
        check(&result, depth);
        drop(result);
        let after_drop = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(
            after_drop, 0,
            "cascade-owned tracked allocations must be released"
        );
        let mut times = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            let result = cascade(&doc, &rules).expect("timed cascade fits");
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            check(&result, depth);
            assert_eq!(finish_custom_property_diagnostics(), Default::default());
            times.push(ms);
            drop(result);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "depth={depth} unique={unique} mode={mode} ancestor_visits={} index_visits={} created_nodes={} ownership_visits={} allocated_bytes={allocated} realloc_bytes={reallocated} allocations={calls} reallocations={realloc_calls} retained_heap={retained} after_drop={after_drop} median_ms={:.6}",
            work.ancestor_visits,
            work.index_visits,
            work.created_nodes,
            work.ownership_visits,
            times[1]
        );
    }

    pub fn run() {
        let depths: Vec<usize> = std::env::var("RAIKIRI_PROPERTY_DEPTHS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|n| n.trim().parse().expect("positive depth"))
                    .collect()
            })
            .unwrap_or_else(|| vec![1024, 2048, 4096]);
        for unique in [false, true] {
            for mode in ["direct", "hit", "miss"] {
                let doc = document(4, unique, mode);
                let rules = build_rule_tree(&doc);
                check(&cascade(&doc, &rules).expect("warm cascade"), 4);
            }
        }
        for depth in depths {
            assert!(depth > 0);
            for unique in [false, true] {
                for mode in ["direct", "hit", "miss"] {
                    measure(depth, unique, mode);
                }
            }
        }
    }
}

// cov:ignore: standalone benchmark entry point; workspace coverage does not run it.
fn main() {
    if std::env::args().any(|arg| arg == "--bench") {
        accounting::run();
    }
}
