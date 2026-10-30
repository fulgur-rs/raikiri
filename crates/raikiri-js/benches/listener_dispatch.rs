//! Listener dispatch through public evaluate.
//! Run cargo bench -p raikiri-js --features listener-diagnostics --bench listener_dispatch.

// cov:ignore: standalone benchmark is not built by cargo test.
mod measurement {
    use raikiri_dom::Document;
    use raikiri_js::listener_diagnostics as diagnostics;
    use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        any::Any,
        sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        time::Instant,
    };
    static COUNT: AtomicBool = AtomicBool::new(false);
    static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
    static ALLOCS: AtomicU64 = AtomicU64::new(0);
    static REALLOCS: AtomicU64 = AtomicU64::new(0);
    static LIVE: AtomicU64 = AtomicU64::new(0);
    struct Counting;
    fn record(size: usize, realloc: bool) {
        if COUNT.load(Relaxed) {
            if realloc {
                REALLOC_BYTES.fetch_add(size as u64, Relaxed);
                REALLOCS.fetch_add(1, Relaxed);
            } else {
                ALLOC_BYTES.fetch_add(size as u64, Relaxed);
                ALLOCS.fetch_add(1, Relaxed);
            }
        }
    }
    // SAFETY: allocation arguments/results pass through System unchanged;
    // atomic counters neither allocate nor access allocation contents.
    #[allow(unsafe_code)]
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller supplies a valid allocation layout.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                LIVE.fetch_add(layout.size() as u64, Relaxed);
                record(layout.size(), false);
            }
            ptr
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller supplies a valid allocation layout.
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                LIVE.fetch_add(layout.size() as u64, Relaxed);
                record(layout.size(), false);
            }
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: ptr/layout describe a live System allocation.
            LIVE.fetch_sub(layout.size() as u64, Relaxed);
            unsafe { System.dealloc(ptr, layout) };
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller upholds System's reallocation contract.
            let new = unsafe { System.realloc(ptr, layout, size) };
            if !new.is_null() {
                LIVE.fetch_add(size as u64, Relaxed);
                LIVE.fetch_sub(layout.size() as u64, Relaxed);
                record(size, true);
            }
            new
        }
    }
    #[global_allocator]
    static ALLOCATOR: Counting = Counting;
    struct Host(Document);
    impl DocumentHost for Host {
        fn document(&self) -> &Document {
            &self.0
        }
        fn document_mut(&mut self) -> &mut Document {
            &mut self.0
        }
        fn flush(&mut self) -> Result<(), HostError> {
            panic!("unexpected flush")
        }
        fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
            panic!("unexpected host read")
        }
        fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
            panic!("unexpected host read")
        }
        fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
            Err(HostError("unexpected fragment parse".into()))
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
    enum Mode {
        Ordinary,
        Once,
        Second,
        Foreign,
        Nested,
    }
    fn registration(n: usize, mode: Mode) -> String {
        let kind = if matches!(mode, Mode::Foreign) {
            "'z'.repeat(65536)+i"
        } else {
            "'x'"
        };
        let body = if matches!(mode, Mode::Nested) {
            "calls++;target.dispatchEvent(nested);"
        } else {
            "calls++;"
        };
        let once = matches!(mode, Mode::Once | Mode::Second | Mode::Nested);
        format!(
            "for(let i=0;i<{n};i++)target.addEventListener({kind},()=>{{{body}}},{{once:{once}}});"
        )
    }
    fn collect(rt: &mut DomRuntime) {
        rt.context_mut().clear_kept_objects();
        boa_gc::force_collect();
    }
    fn prepare(n: usize, mode: Mode) -> DomRuntime {
        let mut rt = DomRuntime::new(Host(Document::new())).unwrap();
        rt.evaluate("globalThis.target=document.createElement('div');globalThis.calls=0;globalThis.event=new Event('x');globalThis.nested=new Event('y');").unwrap();
        rt.evaluate(&registration(n, mode)).unwrap();
        if matches!(mode, Mode::Second) {
            rt.evaluate("target.dispatchEvent(event)").unwrap();
        }
        collect(&mut rt);
        rt
    }
    fn query(rt: &mut DomRuntime, expected: usize) {
        assert_eq!(
            rt.evaluate("calls=0;target.dispatchEvent(event);calls")
                .unwrap()
                .as_number(),
            Some(expected as f64)
        );
    }
    fn reset() {
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOCS.store(0, Relaxed);
        REALLOCS.store(0, Relaxed);
    }
    fn measure(n: usize, mode: Mode) {
        let expected = if matches!(mode, Mode::Foreign | Mode::Second) {
            0
        } else {
            n
        };
        let mut times = Vec::new();
        for sample in 0..4 {
            let mut rt = prepare(n, mode);
            let start = Instant::now();
            query(&mut rt, expected);
            let ms = start.elapsed().as_secs_f64() * 1000.;
            if sample > 0 {
                times.push(ms);
            }
        }
        let mut rt = prepare(n, mode);
        reset();
        diagnostics::begin();
        COUNT.store(true, Relaxed);
        query(&mut rt, expected);
        COUNT.store(false, Relaxed);
        let work = diagnostics::finish();
        println!(
            "listeners={n} mode={mode:?} raw_ms={times:?} alloc_bytes={} realloc_bytes={} alloc_calls={} realloc_calls={} work={work:?}",
            ALLOC_BYTES.load(Relaxed),
            REALLOC_BYTES.load(Relaxed),
            ALLOCS.load(Relaxed),
            REALLOCS.load(Relaxed)
        );
    }
    fn memory(n: usize, shape: &str) {
        let mut rt = DomRuntime::new(Host(Document::new())).unwrap();
        rt.evaluate("globalThis.targets=[];globalThis.calls=0;globalThis.target=document.createElement('div');").unwrap();
        collect(&mut rt);
        let before = LIVE.load(Relaxed);
        let script = match shape {
            "Empty" => {
                format!("for(let i=0;i<{n};i++)targets.push(document.createElement('div'));")
            }
            "Many" => format!(
                "for(let i=0;i<{n};i++){{let t=document.createElement('div');t.addEventListener('x',()=>{{}});targets.push(t);}}"
            ),
            _ => registration(n, Mode::Ordinary),
        };
        reset();
        COUNT.store(true, Relaxed);
        let start = Instant::now();
        rt.evaluate(&script).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1000.;
        COUNT.store(false, Relaxed);
        let allocated = ALLOC_BYTES.load(Relaxed);
        let reallocated = REALLOC_BYTES.load(Relaxed);
        collect(&mut rt);
        let delta = LIVE.load(Relaxed) as i128 - before as i128;
        println!(
            "memory_nodes={n} shape={shape} registration_ms={ms} alloc_bytes={allocated} realloc_bytes={reallocated} live_requested_delta={delta}"
        );
    }
    pub fn run() {
        println!("storage={:?}", diagnostics::storage_sizes());
        for n in [0, 1, 16, 1000, 2000, 4000, 8000] {
            measure(n, Mode::Ordinary);
            measure(n, Mode::Once);
        }
        for n in [0, 1, 16, 8000] {
            measure(n, Mode::Second);
        }
        for n in [0, 16, 32, 64] {
            measure(n, Mode::Foreign);
        }
        for n in [0, 16, 64] {
            measure(n, Mode::Nested);
        }
        for n in [0, 1, 16, 1000, 8000] {
            for shape in ["Empty", "One", "Many"] {
                memory(n, shape);
            }
        }
    }
}
// cov:ignore: standalone benchmark entry point is not built by cargo test.
fn main() {
    measurement::run();
}
