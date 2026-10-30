//! Fresh table layout with independently varied reservation rows and columns.
//! Run cargo bench -p raikiri-bench --bench rowspan_reservations.
//! Counters run separately; timing includes the reservation index construction.

// cov:ignore: standalone benchmark is not instrumented by workspace coverage.
mod accounting {
    use raikiri_dom::fonts::{BundledFace, build_bundled_font_collection};
    use raikiri_dom::{Document, layout_single_page, table_reservation_diagnostics as diagnostics};
    use raikiri_style::{CascadeResult, build_rule_tree, cascade};
    use raikiri_traits::PageBox;
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

    #[derive(Clone, Copy)]
    struct Case {
        rows: usize,
        prefix: usize,
        cells_per_row: usize,
        spanning: bool,
    }

    struct Prepared {
        document: Document,
        cascade: CascadeResult,
        page: PageBox,
        last: usize,
        case: Case,
    }

    impl Prepared {
        fn new(case: Case, install: &impl Fn(&mut Document)) -> Self {
            let mut document = Document::new();
            install(&mut document);
            let html = document.append_element(Some(0), "html", Some("display:block"));
            let body = document.append_element(Some(html), "body", Some("display:block"));
            let table = document.append_element(Some(body), "table", Some(format!(
                "display:table;table-layout:fixed;border-collapse:separate;border-spacing:0;width:{}px",
                case.prefix + case.cells_per_row
            )));
            let group =
                document.append_element(Some(table), "tbody", Some("display:table-row-group"));
            let first =
                document.append_element(Some(group), "tr", Some("display:table-row;height:1px"));
            let mut remaining = case.prefix;
            while remaining > 0 {
                let span = remaining.min(1000);
                remaining -= span;
                let cell = document.append_element(
                    Some(first),
                    "td",
                    Some("display:table-cell;height:1px"),
                );
                document
                    .set_element_attribute(cell, "colspan", span.to_string())
                    .unwrap();
                document
                    .set_element_attribute(
                        cell,
                        "rowspan",
                        &if case.spanning {
                            case.rows.to_string()
                        } else {
                            "1".into()
                        },
                    )
                    .unwrap();
            }
            let mut last = 0;
            for _ in 1..case.rows {
                let row = document.append_element(
                    Some(group),
                    "tr",
                    Some("display:table-row;height:1px"),
                );
                for column in 0..case.cells_per_row {
                    let cell = document.append_element(
                        Some(row),
                        "td",
                        Some("display:table-cell;height:1px"),
                    );
                    if column == 0 {
                        last = cell;
                    }
                }
            }
            document.mark_in_document_flags();
            let cascade = cascade(&document, &build_rule_tree(&document)).unwrap();
            let mut page = PageBox::new();
            page.width = (case.prefix + case.cells_per_row) as f32;
            page.height = 100000.0;
            Self {
                document,
                cascade,
                page,
                last,
                case,
            }
        }

        fn layout(&mut self) {
            layout_single_page(&mut self.document, &self.cascade, self.page).unwrap();
        }

        fn verify(&self) {
            let expected = if self.case.spanning {
                self.case.prefix as f32
            } else {
                0.0
            };
            let actual = self
                .document
                .get_node(self.last)
                .unwrap()
                .layout()
                .location
                .x;
            assert!(
                (actual - expected).abs() < 0.1,
                "actual={actual} expected={expected}"
            );
        }
    }

    fn measure(shape: &str, case: Case, install: &impl Fn(&mut Document)) {
        let mut warm = Prepared::new(case, install);
        warm.layout();
        warm.verify();
        drop(warm);
        LIVE.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        let mut prepared = Prepared::new(case, install);
        let before = LIVE.load(Relaxed);
        PEAK.store(before, Relaxed);
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOC_CALLS.store(0, Relaxed);
        REALLOC_CALLS.store(0, Relaxed);
        COUNT_REQUESTS.store(true, Relaxed);
        diagnostics::begin();
        prepared.layout();
        let work = diagnostics::finish();
        COUNT_REQUESTS.store(false, Relaxed);
        let peak = PEAK.load(Relaxed);
        let retained = LIVE.load(Relaxed);
        prepared.verify();
        drop(prepared);
        let after_drop = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(after_drop, 0);
        let mut times = Vec::new();
        for _ in 0..3 {
            let mut timed = Prepared::new(case, install);
            let start = Instant::now();
            timed.layout();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            timed.verify();
            assert_eq!(diagnostics::finish(), Default::default());
        }
        times.sort_by(f64::total_cmp);
        let cells = case.prefix.div_ceil(1000) + (case.rows - 1) * case.cells_per_row;
        println!(
            "shape={shape} rows={} prefix={} cells_per_row={} spanning={} cells={cells} passes={} occupied_slots={} slot_updates={} search_nodes={} update_nodes={} copied_nodes={} alloc_bytes={} realloc_bytes={} allocs={} reallocs={} before_layout_heap={before} peak_layout_heap={peak} retained_heap={retained} after_drop={after_drop} layout_ms={:.6}",
            case.rows,
            case.prefix,
            case.cells_per_row,
            case.spanning,
            work.passes,
            work.occupied_slots,
            work.slot_updates,
            work.search_nodes,
            work.update_nodes,
            work.copied_nodes,
            ALLOC_BYTES.load(Relaxed),
            REALLOC_BYTES.load(Relaxed),
            ALLOC_CALLS.load(Relaxed),
            REALLOC_CALLS.load(Relaxed),
            times[1]
        );
    }

    pub fn run() {
        let fonts = build_bundled_font_collection(
            vec![BundledFace {
                family: "Ahem".into(),
                bytes: include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf")
                    .to_vec(),
            }],
            false,
        )
        .unwrap();
        let install = |document: &mut Document| document.set_font_collection(fonts.clone());
        let counts: Vec<usize> = std::env::var("RAIKIRI_ROWSPAN_COUNTS")
            .ok()
            .map(|s| s.split(',').map(|n| n.parse().unwrap()).collect())
            .unwrap_or_else(|| vec![1000, 4000, 8000, 16000]);
        for count in counts {
            for (shape, rows, prefix) in [
                ("diagonal", count, count),
                ("rows", count, 128),
                ("prefix", 128, count),
            ] {
                for spanning in [false, true] {
                    measure(
                        shape,
                        Case {
                            rows,
                            prefix,
                            cells_per_row: 1,
                            spanning,
                        },
                        &install,
                    );
                }
            }
        }
        for spanning in [false, true] {
            measure(
                "cells",
                Case {
                    rows: 4000,
                    prefix: 4000,
                    cells_per_row: 4,
                    spanning,
                },
                &install,
            );
        }
    }
}

// cov:ignore: standalone benchmark entry point is not instrumented by workspace coverage.
fn main() {
    if std::env::args().any(|arg| arg == "--bench") {
        accounting::run();
    }
}
