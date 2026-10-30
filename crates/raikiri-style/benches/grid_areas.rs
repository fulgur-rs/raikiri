//! Public RuleTree named-area parsing benchmark, with separate work/heap counts.
//! Run cargo bench -p raikiri-style --features grid-area-diagnostics --bench grid_areas.

// cov:ignore: standalone benchmark is not run by workspace coverage.
mod accounting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::Relaxed};
    use std::time::Instant;

    static TRACK_HEAP: AtomicBool = AtomicBool::new(false);
    static COUNT_REQUESTS: AtomicBool = AtomicBool::new(false);
    static PEAK: AtomicIsize = AtomicIsize::new(0);
    static LIVE: AtomicIsize = AtomicIsize::new(0);
    static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    static REALLOC_CALLS: AtomicU64 = AtomicU64::new(0);

    struct Counting;

    fn allocation(size: usize) {
        if TRACK_HEAP.load(Relaxed) {
            let live = LIVE.fetch_add(size as isize, Relaxed) + size as isize;
            PEAK.fetch_max(live, Relaxed);
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
                    let live = LIVE.fetch_add(size as isize - layout.size() as isize, Relaxed)
                        + size as isize
                        - layout.size() as isize;
                    PEAK.fetch_max(live, Relaxed);
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

    use raikiri_style::property::{GridTemplateAreasValue, PropertyValue};
    use raikiri_style::{Origin, RuleTree, grid_area_diagnostics as diagnostics};

    fn verify(tree: &RuleTree, n: usize, names: usize) {
        assert_eq!(tree.style_rules().len(), 1);
        let declarations = tree.style_rules()[0].declarations();
        assert_eq!(declarations.len(), 1);
        let PropertyValue::GridTemplateAreas(GridTemplateAreasValue::Areas(areas)) =
            declarations[0].value()
        else {
            panic!("valid area declaration was discarded");
        };
        assert_eq!(areas.row_count, 1);
        assert_eq!(areas.column_count, n as u32);
        assert_eq!(areas.areas.len(), names);
        for (i, area) in areas.areas.iter().enumerate() {
            assert_eq!(area.name, format!("a{i:06}"));
            assert_eq!((area.row_start, area.row_end), (1, 2));
            let start = (i * n).div_ceil(names);
            let end = ((i + 1) * n).div_ceil(names);
            assert_eq!(
                (area.column_start, area.column_end),
                (start as u32 + 1, end as u32 + 1)
            );
        }
    }

    fn measure(n: usize, names: usize) {
        let row = (0..n)
            .map(|i| format!("a{:06}", i * names / n))
            .collect::<Vec<_>>()
            .join(" ");
        let css = format!("div{{grid-template-areas:\"{row}\";}}");
        let mut warm = RuleTree::empty();
        warm.add_stylesheet(&css, Origin::Author);
        verify(&warm, n, names);
        drop(warm);

        // Work and allocator accounting run separately from elapsed-time samples.
        LIVE.store(0, Relaxed);
        PEAK.store(0, Relaxed);
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOC_CALLS.store(0, Relaxed);
        REALLOC_CALLS.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        let mut measured = RuleTree::empty();
        let before = LIVE.load(Relaxed);
        PEAK.store(before, Relaxed);
        COUNT_REQUESTS.store(true, Relaxed);
        diagnostics::begin();
        measured.add_stylesheet(&css, Origin::Author);
        let work = diagnostics::finish();
        COUNT_REQUESTS.store(false, Relaxed);
        let peak = PEAK.load(Relaxed);
        let retained = LIVE.load(Relaxed);
        verify(&measured, n, names);
        drop(measured);
        let after_drop = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(after_drop, 0);

        let mut times = [0.0; 5];
        for time in &mut times {
            let mut timed = RuleTree::empty();
            let start = Instant::now();
            timed.add_stylesheet(&css, Origin::Author);
            *time = start.elapsed().as_secs_f64() * 1000.0;
            verify(&timed, n, names);
            assert_eq!(diagnostics::finish(), Default::default());
            std::hint::black_box(timed);
        }
        let mut sorted = times;
        sorted.sort_by(f64::total_cmp);
        println!(
            "n={n},names={names},input_bytes={},linear_comparisons={},indexed_lookups={},indexed_insertions={},rectangle_cells={},alloc_bytes={},realloc_bytes={},alloc_calls={},realloc_calls={},before={},peak={},retained={},after_drop={},median_ms={:.6},samples_ms={:.6};{:.6};{:.6};{:.6};{:.6}",
            css.len(),
            work.linear_comparisons,
            work.indexed_lookups,
            work.indexed_insertions,
            work.rectangle_cells,
            ALLOC_BYTES.load(Relaxed),
            REALLOC_BYTES.load(Relaxed),
            ALLOC_CALLS.load(Relaxed),
            REALLOC_CALLS.load(Relaxed),
            before,
            peak,
            retained,
            after_drop,
            sorted[2],
            times[0],
            times[1],
            times[2],
            times[3],
            times[4]
        );
    }

    pub fn run() {
        for n in [1000, 2000, 4000, 8000] {
            measure(n, 1);
            measure(n, n);
        }
        for names in [4, 8, 9, 16] {
            measure(8000, names);
            measure(names, names);
        }
    }
}
// cov:ignore: standalone benchmark entry point.
fn main() {
    accounting::run();
}
