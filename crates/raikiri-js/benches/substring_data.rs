//! UTF-16 CharacterData substring reads through public evaluate.
//! Run cargo bench -p raikiri-js --features substring-diagnostics --bench substring_data.

// cov:ignore: standalone benchmark is not built by cargo test.
mod measurement {
    use raikiri_dom::Document;
    use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
    use raikiri_js::substring_diagnostics as diagnostics;
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
                record(layout.size(), false);
            }
            ptr
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: the caller supplies a valid allocation layout.
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                record(layout.size(), false);
            }
            ptr
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: ptr/layout describe a live System allocation.
            unsafe { System.dealloc(ptr, layout) };
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller upholds System's reallocation contract.
            let new = unsafe { System.realloc(ptr, layout, size) };
            if !new.is_null() {
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
        Prefix,
        Empty,
        NearEnd,
        EndEmpty,
        Invalid,
        High,
        Low,
        Full,
    }
    fn prepare(n: usize, mode: Mode) -> (DomRuntime, String, usize) {
        let mut data = "x".repeat(n);
        if matches!(mode, Mode::High | Mode::Low) {
            data.push('😀');
        }
        let expected = if n == 0 { "" } else { "x" };
        let statement = match mode {
            Mode::Prefix => format!("if(p.substringData(0,1)==={expected:?})hits++;"),
            Mode::Empty => "if(p.substringData(0,0)==='')hits++;".into(),
            Mode::NearEnd => format!(
                "if(p.substringData({},1)==={expected:?})hits++;",
                n.saturating_sub(1)
            ),
            Mode::EndEmpty => format!("if(p.substringData({n},0)==='')hits++;"),
            Mode::Invalid => format!(
                "try{{p.substringData({},0);}}catch(e){{if(e.name==='IndexSizeError')hits++;}}",
                n + 1
            ),
            Mode::High => format!("if(p.substringData({n},1).charCodeAt(0)===0xd83d)hits++;"),
            Mode::Low => format!(
                "if(p.substringData({},1).charCodeAt(0)===0xde00)hits++;",
                n + 1
            ),
            Mode::Full => "if(p.substringData(0,4294967295)===expected)hits++;".into(),
        };
        let reads = 100;
        let script =
            format!("(()=>{{let hits=0;for(let i=0;i<{reads};++i){{{statement}}}return hits;}})()");
        let mut rt = DomRuntime::new(Host(Document::new())).unwrap();
        rt.evaluate(&format!(
            "globalThis.expected={data:?};globalThis.p=document.createTextNode(expected);"
        ))
        .unwrap();
        assert_eq!(
            rt.evaluate("p.length").unwrap().as_number(),
            Some(data.encode_utf16().count() as f64)
        );
        (rt, script, reads)
    }
    fn evaluate(rt: &mut DomRuntime, script: &str, reads: usize) {
        assert_eq!(rt.evaluate(script).unwrap().as_number(), Some(reads as f64));
    }
    fn measure(n: usize, mode: Mode) {
        let (mut rt, script, reads) = prepare(n, mode);
        let mut times = Vec::new();
        for sample in 0..4 {
            let start = Instant::now();
            let result = rt.evaluate(&script).unwrap();
            let ms = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(result.as_number(), Some(reads as f64));
            if sample > 0 {
                times.push(ms);
            }
        }
        drop(rt);
        let (mut rt, script, reads) = prepare(n, mode);
        evaluate(&mut rt, &script, reads);
        ALLOC_BYTES.store(0, Relaxed);
        REALLOC_BYTES.store(0, Relaxed);
        ALLOCS.store(0, Relaxed);
        REALLOCS.store(0, Relaxed);
        COUNT.store(true, Relaxed);
        let result = rt.evaluate(&script).unwrap();
        COUNT.store(false, Relaxed);
        let alloc_bytes = ALLOC_BYTES.load(Relaxed);
        let realloc_bytes = REALLOC_BYTES.load(Relaxed);
        let allocs = ALLOCS.load(Relaxed);
        let reallocs = REALLOCS.load(Relaxed);
        assert_eq!(result.as_number(), Some(reads as f64));
        drop(rt);
        let (mut rt, script, reads) = prepare(n, mode);
        evaluate(&mut rt, &script, reads);
        diagnostics::begin();
        evaluate(&mut rt, &script, reads);
        let work = diagnostics::finish();
        println!(
            "prefix_bytes={n} mode={mode:?} reads={reads} raw_ms={times:?} alloc_requested_bytes={alloc_bytes} realloc_requested_bytes={realloc_bytes} alloc_calls={allocs} realloc_calls={reallocs} work={work:?}"
        );
    }
    pub fn run() {
        for n in [0, 1, 16, 4096, 16384, 65536, 262144, 1048576] {
            for mode in [Mode::Prefix, Mode::Empty] {
                measure(n, mode);
            }
            if n == 0 || n == 4096 || n == 1048576 {
                for mode in [Mode::NearEnd, Mode::EndEmpty, Mode::Invalid] {
                    measure(n, mode);
                }
            }
            if n == 0 || n == 4096 {
                for mode in [Mode::High, Mode::Low, Mode::Full] {
                    measure(n, mode);
                }
            }
        }
    }
}
// cov:ignore: standalone benchmark entry point is not built by cargo test.
fn main() {
    measurement::run();
}
