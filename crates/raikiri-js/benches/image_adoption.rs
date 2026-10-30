//! Image adoption checks during unrelated DOM mutations through public evaluate.
//! Run cargo bench -p raikiri-js --features image-adoption-diagnostics --bench image_adoption.

// cov:ignore: standalone benchmark is not built by cargo test.
mod measurement {
    use raikiri_dom::Document;
    use raikiri_js::image_adoption_diagnostics as diagnostics;
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
    enum Shape {
        Images,
        Divs,
        Inert,
    }
    #[derive(Debug, Clone, Copy)]
    enum Mode {
        Attribute,
        Text,
        Create,
        Move,
    }
    #[derive(Debug, Clone, Copy)]
    enum Setup {
        Parsed,
        Script,
    }
    fn element(d: &mut Document, parent: usize, tag: &str) -> usize {
        let id = d.append_element(Some(parent), tag, None::<&str>);
        d.set_element_namespace(id, Some("http://www.w3.org/1999/xhtml".into()));
        id
    }
    fn prepare(n: usize, shape: Shape, mode: Mode, setup: Setup) -> (DomRuntime, String, usize) {
        let mut d = Document::new();
        let html = element(&mut d, 0, "html");
        let body = element(&mut d, html, "body");
        let target = element(&mut d, body, "div");
        d.set_element_attribute(target, "id", "target").unwrap();
        d.append_text(target, "start");
        let holder = element(&mut d, body, "div");
        d.set_element_attribute(holder, "id", "holder").unwrap();
        let parent = if matches!(shape, Shape::Inert) {
            let t = element(&mut d, body, "template");
            d.allocate_template_fragment_root(t)
        } else {
            body
        };
        if matches!(setup, Setup::Parsed) {
            for _ in 0..n {
                let id = element(
                    &mut d,
                    parent,
                    if matches!(shape, Shape::Divs) {
                        "div"
                    } else {
                        "img"
                    },
                );
                if !matches!(shape, Shape::Divs) {
                    d.set_element_attribute(id, "src", "").unwrap();
                }
            }
        }
        d.mark_in_document_flags();
        let mut rt = DomRuntime::new(Host(d)).unwrap();
        let report = rt.run_document();
        assert!(report.aborted.is_none());
        assert!(report.uncaught_errors.is_empty());
        assert!(report.host_failures.is_empty());
        if matches!(setup, Setup::Script) {
            rt.evaluate(&format!("for(let i=0;i<{n};i++){{const node=document.createElement('{}');{}document.body.appendChild(node);}}",if matches!(shape,Shape::Divs){"div"}else{"img"},if matches!(shape,Shape::Divs){""}else{"node.src='';"})).unwrap();
        }
        rt.evaluate("globalThis.el=document.getElementById('target');globalThis.holder=document.getElementById('holder');globalThis.text=el.firstChild;el.setAttribute('data-warm','ready');").unwrap();
        if !matches!(shape, Shape::Inert) {
            let count = rt
                .evaluate(if matches!(shape, Shape::Divs) {
                    "document.getElementsByTagName('div').length"
                } else {
                    "document.getElementsByTagName('img').length"
                })
                .unwrap();
            assert_eq!(
                count.as_number(),
                Some((n + if matches!(shape, Shape::Divs) { 2 } else { 0 }) as f64)
            );
        }
        let updates = 500;
        let (statement, check) = match mode {
            Mode::Attribute => (
                "el.setAttribute('data-x',String(i));hits++;",
                "el.getAttribute('data-x')==='499'",
            ),
            Mode::Text => ("text.data=String(i);hits++;", "text.data==='499'"),
            Mode::Create => (
                "const node=document.createElement('span');if(node.nodeType===1)hits++;",
                "true",
            ),
            Mode::Move => (
                "if((i%2===0?holder:document.body).appendChild(el)===el)hits++;",
                "el.parentNode===document.body",
            ),
        };
        let script = format!(
            "(()=>{{let hits=0;for(let i=0;i<{updates};++i){{{statement}}}return ({check})?hits:0;}})()"
        );
        (rt, script, updates)
    }
    fn evaluate(rt: &mut DomRuntime, script: &str, updates: usize) {
        assert_eq!(
            rt.evaluate(script).unwrap().as_number(),
            Some(updates as f64)
        );
    }
    fn measure(n: usize, shape: Shape, mode: Mode, setup: Setup) {
        let (mut rt, script, updates) = prepare(n, shape, mode, setup);
        let mut times = Vec::new();
        for sample in 0..4 {
            let start = Instant::now();
            let result = rt.evaluate(&script).unwrap();
            let ms = start.elapsed().as_secs_f64() * 1000.;
            assert_eq!(result.as_number(), Some(updates as f64));
            if sample > 0 {
                times.push(ms);
            }
        }
        drop(rt);
        let (mut rt, script, updates) = prepare(n, shape, mode, setup);
        evaluate(&mut rt, &script, updates);
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
        assert_eq!(result.as_number(), Some(updates as f64));
        drop(rt);
        let (mut rt, script, updates) = prepare(n, shape, mode, setup);
        evaluate(&mut rt, &script, updates);
        diagnostics::begin();
        evaluate(&mut rt, &script, updates);
        let work = diagnostics::finish();
        println!(
            "nodes={n} shape={shape:?} mode={mode:?} setup={setup:?} updates={updates} raw_ms={times:?} alloc_requested_bytes={alloc_bytes} realloc_requested_bytes={realloc_bytes} alloc_calls={allocs} realloc_calls={reallocs} work={work:?}"
        );
    }
    pub fn run() {
        for n in [0, 1000, 2000, 4000, 8000] {
            for shape in [Shape::Images, Shape::Divs] {
                measure(n, shape, Mode::Attribute, Setup::Parsed);
            }
        }
        for n in [0, 8000] {
            for shape in [Shape::Images, Shape::Divs] {
                for mode in [Mode::Text, Mode::Create, Mode::Move] {
                    measure(n, shape, mode, Setup::Parsed);
                }
            }
        }
        for shape in [Shape::Images, Shape::Divs] {
            measure(8000, shape, Mode::Attribute, Setup::Script);
        }
        measure(8000, Shape::Inert, Mode::Attribute, Setup::Parsed);
    }
}
// cov:ignore: standalone benchmark entry point is not built by cargo test.
fn main() {
    measurement::run();
}
