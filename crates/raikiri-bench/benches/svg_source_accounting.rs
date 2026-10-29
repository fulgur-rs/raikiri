//! Real SVG cache insertion accounting, with parsing outside the timed run.
//! Run cargo bench -p raikiri-bench --bench svg_source_accounting.
//! RAIKIRI_SVG_COUNTS selects counts (default100,1000,2000,4000).
//! Requested allocation counters cover run only; live heap covers the whole
//! prepared workload through drop. Timing uses another freshly prepared probe
//! with both allocation and work-counter updates disabled.

// cov:ignore: standalone benchmark target is not instrumented by workspace coverage.
mod accounting {
    use raikiri_html::{CacheAccountingCase, CacheAccountingProbe};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::collections::BTreeMap;
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

    fn ledger_layout_control(count: usize) {
        // This equivalent key/value layout isolates BTree allocation storage;
        // it is a toolchain estimate check, not a cache workload or std guarantee.
        LIVE.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        let mut ledger = BTreeMap::<usize, (usize, u64)>::new();
        for at in 0..count {
            ledger.insert(at, (1, 1));
        }
        let dense = LIVE.load(Relaxed);
        assert!(dense <= (512 * count) as isize);
        for at in 0..count - 1 {
            ledger.remove(&at);
        }
        let sparse = LIVE.load(Relaxed);
        assert!(sparse <= 512);
        ledger.remove(&(count - 1));
        drop(ledger);
        let empty = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(empty, 0);
        println!(
            "ledger-layout n={count} dense_heap={dense} one_identity_heap={sparse} after_drop={empty} charge_per_identity=512"
        );
    }

    fn measure(case: CacheAccountingCase, count: usize) {
        LIVE.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        let mut probe = CacheAccountingProbe::prepare(case, count);
        let before = probe.result();
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOC_CALLS.store(0, Relaxed);
        REALLOC_CALLS.store(0, Relaxed);
        COUNT_REQUESTS.store(true, Relaxed);
        probe.run(true);
        COUNT_REQUESTS.store(false, Relaxed);
        let result = probe.result();
        let retained_heap = LIVE.load(Relaxed);
        let alloc = ALLOC_BYTES.load(Relaxed);
        let realloc = REALLOC_BYTES.load(Relaxed);
        let calls = ALLOC_CALLS.load(Relaxed);
        let realloc_calls = REALLOC_CALLS.load(Relaxed);
        drop(probe);
        let after_drop = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(after_drop, 0, "prepared/cache allocations must be released");
        assert_eq!(result.accounted_bytes, result.process_bytes);
        if matches!(
            case,
            CacheAccountingCase::CacheDenied | CacheAccountingCase::ProcessDenied
        ) {
            assert_eq!(result.rejected, count);
            assert_eq!(result.accounted_bytes, before.accounted_bytes);
            assert_eq!(result.entries, before.entries);
        } else {
            assert_eq!(result.rejected, 0);
        }
        let mut timed = CacheAccountingProbe::prepare(case, count);
        let start = Instant::now();
        timed.run(false);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let timed_result = timed.result();
        assert_eq!(timed_result.accounted_bytes, result.accounted_bytes);
        assert_eq!(timed_result.entries, result.entries);
        assert_eq!(timed_result.rejected, result.rejected);
        assert_eq!(timed_result.work, 0);
        println!(
            "case={} n={count} attempts={} rejected={} entries={} svg_entries={} work={} allocated_bytes={alloc} realloc_bytes={realloc} allocations={calls} reallocations={realloc_calls} retained_heap={retained_heap} accounted_bytes={} process_bytes={} ledger_entries={} ledger_charge={} after_drop={after_drop} ms={ms:.6}",
            case.name(),
            result.attempts,
            result.rejected,
            result.entries,
            result.svg_entries,
            result.work,
            result.accounted_bytes,
            result.process_bytes,
            result.ledger_entries,
            result.ledger_accounted_bytes
        );
    }

    pub fn run() {
        let counts: Vec<usize> = std::env::var("RAIKIRI_SVG_COUNTS")
            .ok()
            .map(|list| {
                list.split(',')
                    .map(|n| n.trim().parse().expect("positive count"))
                    .collect()
            })
            .unwrap_or_else(|| vec![100, 1000, 2000, 4000]);
        // Warm parser/library globals before tracking owned allocations.
        for case in CacheAccountingCase::ALL {
            let mut warm = CacheAccountingProbe::prepare(case, 2);
            warm.run(false);
        }
        for count in counts {
            assert!(count > 0);
            ledger_layout_control(count);
            for case in CacheAccountingCase::ALL {
                measure(case, count);
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
