//! Selective URLSearchParams reads through public evaluate.
//! Run cargo bench -p raikiri-js --features url-params-diagnostics --bench url_params.

// cov:ignore: standalone benchmark is not built by cargo test.
mod measurement {
    use raikiri_dom::Document;
    use raikiri_js::runtime::{BoxGeometry, DocumentHost, DomRuntime, HostError};
    use raikiri_js::url_params_diagnostics as diagnostics;
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
        GetFirst,
        HasFirst,
        GetLast,
        HasLast,
        GetMissing,
        HasMissing,
        AllFirst,
        AllMissing,
        AllMatches,
    }
    fn prepare(n: usize, mode: Mode) -> (DomRuntime, String, usize) {
        let payload = "x".repeat(64);
        let mut query = String::from("first=v");
        for i in 0..n {
            if matches!(mode, Mode::AllMatches) {
                query.push_str(&format!("&dup={payload}"));
            } else {
                query.push_str(&format!("&k{i:06}={payload}"));
            }
        }
        let last = if n == 0 {
            "first".to_owned()
        } else {
            format!("k{:06}", n - 1)
        };
        let expected = if n == 0 { "v" } else { &payload };
        let reads = if matches!(mode, Mode::AllMatches) {
            10
        } else {
            100
        };
        let statement = match mode {
            Mode::GetFirst => "if(p.get('first')==='v')hits++;".into(),
            Mode::HasFirst => "if(p.has('first'))hits++;".into(),
            Mode::GetLast => format!("if(p.get({last:?})==={expected:?})hits++;"),
            Mode::HasLast => format!("if(p.has({last:?}))hits++;"),
            Mode::GetMissing => "if(p.get('missing')===null)hits++;".into(),
            Mode::HasMissing => "if(!p.has('missing'))hits++;".into(),
            Mode::AllFirst => {
                "const a=p.getAll('first');if(a.length===1&&a[0]==='v')hits++;".into()
            }
            Mode::AllMissing => "if(p.getAll('missing').length===0)hits++;".into(),
            Mode::AllMatches => format!(
                "const a=p.getAll('dup');if(a.length==={n}&&({n}===0||(a[0]==={payload:?}&&a[{n}-1]==={payload:?})))hits++;"
            ),
        };
        let script =
            format!("(()=>{{let hits=0;for(let i=0;i<{reads};++i){{{statement}}}return hits;}})()");
        let mut d = Document::new();
        d.append_element(Some(0), "html", None::<&str>);
        d.mark_in_document_flags();
        let mut rt = DomRuntime::new(Host(d)).unwrap();
        rt.evaluate(&format!("globalThis.p=new URLSearchParams({query:?});"))
            .unwrap();
        (rt, script, reads)
    }
    fn evaluate(rt: &mut DomRuntime, script: &str, reads: usize) {
        let result = rt.evaluate(script).unwrap();
        assert_eq!(result.as_number(), Some(reads as f64));
    }
    pub fn run() {
        for n in [0, 256, 512, 1024, 2048, 4096] {
            for mode in [
                Mode::GetFirst,
                Mode::HasFirst,
                Mode::GetLast,
                Mode::HasLast,
                Mode::GetMissing,
                Mode::HasMissing,
                Mode::AllFirst,
                Mode::AllMissing,
                Mode::AllMatches,
            ] {
                if n != 0 && n != 4096 && !matches!(mode, Mode::GetFirst | Mode::HasFirst) {
                    continue;
                }
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
                    "tail_pairs={n} mode={mode:?} reads={reads} raw_ms={times:?} alloc_requested_bytes={alloc_bytes} realloc_requested_bytes={realloc_bytes} alloc_calls={allocs} realloc_calls={reallocs} work={work:?}"
                );
            }
        }
    }
}
// cov:ignore: standalone benchmark entry point is not built by cargo test.
fn main() {
    measurement::run();
}
