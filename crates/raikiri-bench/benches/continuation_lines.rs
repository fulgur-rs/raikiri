//! Continuation queries with construction and query work measured separately.
//! Run cargo bench -p raikiri-bench --bench continuation_lines.
//! RAIKIRI_CONTINUATION_LINES selects counts (default512,1024,2048).
//! Time uses a separate probe with counter updates disabled, but the allocator
//! wrapper remains installed. Query timing includes result assertions.

// cov:ignore: standalone benchmark target is not instrumented by workspace coverage.
mod accounting {
    use raikiri_dom::fonts::{BundledFace, build_bundled_font_collection};
    use raikiri_dom::{
        Document, PageContentInsets, PageMargins, PageSlice, PageStartToken,
        begin_continuation_diagnostics, finish_continuation_diagnostics, layout_pages,
    };
    use raikiri_style::{CascadeResult, build_rule_tree, cascade};
    use raikiri_traits::PageBox;
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

    struct Prepared {
        doc: Document,
        cascade: CascadeResult,
        slices: Vec<PageSlice>,
        geometries: Vec<(PageBox, PageMargins, PageContentInsets)>,
        page: PageBox,
        text: usize,
        count: usize,
    }

    impl Prepared {
        fn new(count: usize, install: &impl Fn(&mut Document)) -> Self {
            let mut doc = Document::new();
            install(&mut doc);
            let html = doc.append_element(Some(0), "html", Some("display:block"));
            let body = doc.append_element(Some(html), "body", Some("display:block"));
            let root=doc.append_element(Some(body),"div",Some("display:block;font-family:Ahem;font-size:10px;line-height:10px;white-space:pre;orphans:1;widows:1"));
            let text = doc.append_text(root, vec!["a"; count].join("\n"));
            doc.mark_in_document_flags();
            let tree = build_rule_tree(&doc);
            let cascade = cascade(&doc, &tree).expect("cascade");
            let mut page = PageBox::new();
            page.width = 100.0;
            page.height = 10.0;
            let slices = layout_pages(&mut doc, &cascade, page).expect("layout");
            assert_eq!(doc.node_count(), 5);
            assert_eq!(slices.len(), count);
            let geometries = slices
                .iter()
                .map(|_| (page, PageMargins::default(), PageContentInsets::default()))
                .collect();
            Self {
                doc,
                cascade,
                slices,
                geometries,
                page,
                text,
                count,
            }
        }

        fn project(&mut self) {
            self.doc
                .project_pages(&self.cascade, self.page, &self.slices, &self.geometries)
                .expect("projection");
        }

        fn query(&self, mode: &str) {
            for index in 1..self.count {
                let token = PageStartToken::Line {
                    text: self.text,
                    offset: (2 * index) as u32,
                };
                match mode {
                    "ownership" => assert_eq!(
                        self.doc
                            .ifc_text_lines(self.text)
                            .expect("owner")
                            .lines
                            .len(),
                        self.count
                    ),
                    "start" => {
                        let start = self
                            .doc
                            .page_start(&self.cascade, index as u32)
                            .expect("start");
                        assert_eq!(start.token, token);
                        assert_eq!(start.offset, 0.0);
                        std::hint::black_box(start);
                    }
                    _ => assert_eq!(self.doc.page_token_offset(index as u32, token), Some(0.0)),
                }
            }
        }
    }

    fn reset_requests() {
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOC_CALLS.store(0, Relaxed);
        REALLOC_CALLS.store(0, Relaxed);
    }

    fn measure(count: usize, mode: &str, install: &impl Fn(&mut Document)) {
        LIVE.store(0, Relaxed);
        TRACK_HEAP.store(true, Relaxed);
        let mut data = Prepared::new(count, install);
        let before_projection = LIVE.load(Relaxed);
        reset_requests();
        COUNT_REQUESTS.store(true, Relaxed);
        begin_continuation_diagnostics();
        data.project();
        let construction = finish_continuation_diagnostics();
        COUNT_REQUESTS.store(false, Relaxed);
        let construction_alloc = ALLOC_BYTES.load(Relaxed);
        let construction_realloc = REALLOC_BYTES.load(Relaxed);
        let construction_calls = ALLOC_CALLS.load(Relaxed);
        let construction_reallocs = REALLOC_CALLS.load(Relaxed);
        let projected_heap = LIVE.load(Relaxed);
        reset_requests();
        COUNT_REQUESTS.store(true, Relaxed);
        begin_continuation_diagnostics();
        data.query(mode);
        let query = finish_continuation_diagnostics();
        COUNT_REQUESTS.store(false, Relaxed);
        let query_alloc = ALLOC_BYTES.load(Relaxed);
        let query_realloc = REALLOC_BYTES.load(Relaxed);
        let query_calls = ALLOC_CALLS.load(Relaxed);
        let query_reallocs = REALLOC_CALLS.load(Relaxed);
        let retained = LIVE.load(Relaxed);
        assert_eq!(retained, projected_heap);
        drop(data);
        let after_drop = LIVE.load(Relaxed);
        TRACK_HEAP.store(false, Relaxed);
        assert_eq!(
            after_drop, 0,
            "prepared layout/projection allocations must be released"
        );
        let mut construction_times = Vec::new();
        let mut query_times = Vec::new();
        for _ in 0..3 {
            let mut timed = Prepared::new(count, install);
            let start = Instant::now();
            timed.project();
            let project_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            timed.query(mode);
            let query_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(finish_continuation_diagnostics(), Default::default());
            construction_times.push(project_ms);
            query_times.push(query_ms);
        }
        construction_times.sort_by(f64::total_cmp);
        query_times.sort_by(f64::total_cmp);
        println!(
            "mode={mode} lines={count} nodes=5 pages={count} construction_line_visits={} construction_glyph_calls={} construction_alloc_bytes={construction_alloc} construction_realloc_bytes={construction_realloc} construction_allocs={construction_calls} construction_reallocs={construction_reallocs} query_line_visits={} query_glyph_calls={} query_alloc_bytes={query_alloc} query_realloc_bytes={query_realloc} query_allocs={query_calls} query_reallocs={query_reallocs} before_projection_heap={before_projection} projected_heap={projected_heap} retained_heap={retained} after_drop={after_drop} project_ms={:.6} query_ms={:.6}",
            construction.text_line_visits,
            construction.glyph_owner_calls,
            query.text_line_visits,
            query.glyph_owner_calls,
            construction_times[1],
            query_times[1]
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
        .expect("Ahem fonts");
        let install = |doc: &mut Document| doc.set_font_collection(fonts.clone());
        for mode in ["ownership", "start", "offset"] {
            let mut warm = Prepared::new(2, &install);
            warm.project();
            warm.query(mode);
        }
        let counts: Vec<usize> = std::env::var("RAIKIRI_CONTINUATION_LINES")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|n| n.trim().parse().expect("count"))
                    .collect()
            })
            .unwrap_or_else(|| vec![512, 1024, 2048]);
        for count in counts {
            assert!(count > 1);
            for mode in ["ownership", "start", "offset"] {
                measure(count, mode, &install);
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
