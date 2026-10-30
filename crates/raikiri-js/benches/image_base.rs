//! Live base URL discovery while initializing real runtime image requests.
//! Run cargo bench -p raikiri-js --features image-base-diagnostics --bench image_base.

// cov:ignore: standalone benchmark is not built by workspace cargo test.
mod measurement {
    use raikiri_dom::{Document, begin_base_url_diagnostics, finish_base_url_diagnostics};
    use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError, RunReport};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::any::Any;
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

    struct Host {
        document: Document,
        loads: usize,
        expected: &'static str,
    }
    impl DocumentHost for Host {
        fn document(&self) -> &Document {
            &self.document
        }
        fn document_mut(&mut self) -> &mut Document {
            &mut self.document
        }
        fn flush(&mut self) -> Result<(), HostError> {
            Ok(())
        }
        fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
            Ok(None)
        }
        fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
            Ok(None)
        }
        fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
            Err(HostError("unexpected fragment parse".into()))
        }
        fn document_url(&self) -> Option<String> {
            Some("https://document.test/folder/page".into())
        }
        fn load_image(&mut self, url: &str) -> Result<(), HostError> {
            assert_eq!(url, self.expected);
            self.loads += 1;
            Ok(())
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn into_any(self: Box<Self>) -> Box<dyn Any> {
            self
        }
    }
    #[derive(Debug, Clone, Copy)]
    enum Base {
        Absent,
        Head,
        Tail,
    }
    fn element(d: &mut Document, parent: usize, name: &str) -> usize {
        let node = d.append_element(Some(parent), name, None::<&str>);
        d.set_element_namespace(node, Some("http://www.w3.org/1999/xhtml".into()));
        node
    }
    fn base_element(d: &mut Document, parent: usize) {
        let base = element(d, parent, "base");
        d.set_element_attribute(base, "href", "https://base.test/assets/")
            .unwrap();
    }
    fn runtime(n: usize, base: Base, relative: bool) -> DomRuntime {
        let mut d = Document::new();
        let html = element(&mut d, 0, "html");
        let head = element(&mut d, html, "head");
        if matches!(base, Base::Head) {
            base_element(&mut d, head);
        }
        let body = element(&mut d, html, "body");
        for _ in 0..n {
            let image = element(&mut d, body, "img");
            d.set_element_attribute(
                image,
                "src",
                if relative {
                    "a.png"
                } else {
                    "https://image.test/a.png"
                },
            )
            .unwrap();
        }
        if matches!(base, Base::Tail) {
            base_element(&mut d, body);
        }
        d.mark_in_document_flags();
        let expected = if !relative {
            "https://image.test/a.png"
        } else if matches!(base, Base::Absent) {
            "https://document.test/folder/a.png"
        } else {
            "https://base.test/assets/a.png"
        };
        DomRuntime::new(Host {
            document: d,
            loads: 0,
            expected,
        })
        .unwrap()
    }
    fn validate_report(report: &RunReport) {
        assert!(report.aborted.is_none());
        assert!(report.uncaught_errors.is_empty());
        assert!(report.host_failures.is_empty());
        assert_eq!(report.scripts_run, 0);
    }
    fn validate(rt: DomRuntime, n: usize) {
        let host = rt.try_into_host::<Host>().ok().unwrap();
        assert_eq!(host.loads, n);
    }
    pub fn measure() {
        for n in [500, 1000, 2000, 4000] {
            for base in [Base::Absent, Base::Head, Base::Tail] {
                for relative in [false, true] {
                    let mut timings = Vec::new();
                    for sample in 0..4 {
                        let mut rt = runtime(n, base, relative);
                        let start = Instant::now();
                        let report = black_box(&mut rt).run_document();
                        let ms = start.elapsed().as_secs_f64() * 1000.0;
                        validate_report(&report);
                        validate(rt, n);
                        if sample > 0 {
                            timings.push(ms);
                        }
                    }
                    let mut rt = runtime(n, base, relative);
                    let initial = LIVE.load(Relaxed);
                    PEAK.store(initial, Relaxed);
                    ALLOCS.store(0, Relaxed);
                    REALLOCS.store(0, Relaxed);
                    REQUESTED.store(0, Relaxed);
                    COUNT.store(true, Relaxed);
                    let report = rt.run_document();
                    COUNT.store(false, Relaxed);
                    let retained = LIVE.load(Relaxed) - initial;
                    let peak = PEAK.load(Relaxed) - initial;
                    let allocs = ALLOCS.load(Relaxed);
                    let reallocs = REALLOCS.load(Relaxed);
                    let requested = REQUESTED.load(Relaxed);
                    validate_report(&report);
                    validate(rt, n);
                    let mut rt = runtime(n, base, relative);
                    begin_base_url_diagnostics();
                    let report = rt.run_document();
                    let work = finish_base_url_diagnostics();
                    validate_report(&report);
                    validate(rt, n);
                    println!(
                        "images={n} base={base:?} relative={relative} raw_ms={timings:?} retained_requested_bytes={retained} peak_requested_bytes={peak} alloc_calls={allocs} realloc_calls={reallocs} cumulative_requested_bytes={requested} work={work:?}"
                    );
                }
            }
        }
    }
}

// cov:ignore: standalone benchmark entry point is not built by cargo test.
fn main() {
    measurement::measure();
}
