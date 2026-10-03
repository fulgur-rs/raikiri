use super::*;
use raikiri_dom::Document;
use raikiri_js::runtime::{BoxGeometry, DocumentHost, HostError};
use raikiri_js_wasmtime_protocol::*;
use std::{any::Any, cell::Cell, rc::Rc};
struct TestHost {
    doc: Document,
    flushes: Rc<Cell<u32>>,
}
impl DocumentHost for TestHost {
    fn document(&self) -> &Document {
        &self.doc
    }
    fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }
    fn flush(&mut self) -> Result<(), HostError> {
        self.flushes.set(self.flushes.get() + 1);
        Ok(())
    }
    fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
        Ok(None)
    }
    fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
        Ok(Some("red".into()))
    }
    fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
        Ok(Document::new())
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
#[test]
fn response_copy_does_not_repeat_flush() {
    let flushes = Rc::new(Cell::new(0));
    let mut bridge = Bridge::new(
        Box::new(TestHost {
            doc: Document::new(),
            flushes: flushes.clone(),
        }),
        100,
    );
    let request = encode(&Request {
        version: VERSION,
        operation: HostOperation::Flush(Document::new().logical_snapshot()),
    })
    .unwrap();
    let length = bridge.request(&request).unwrap();
    assert!(bridge.request(&request).is_err());
    assert!(bridge.copy_response(&mut [0; 1]).is_err());
    assert_eq!(flushes.get(), 1);
    let mut buffer = vec![0; length];
    bridge.copy_response(&mut buffer).unwrap();
    assert_eq!(flushes.get(), 1);
    assert!(bridge.copy_response(&mut buffer).is_err());
}

fn page() -> WasmtimePage {
    page_with_document(Document::new())
}
fn page_with_document(doc: Document) -> WasmtimePage {
    WasmtimePage::new(
        Box::new(TestHost {
            doc,
            flushes: Rc::new(Cell::new(0)),
        }),
        Default::default(),
        Default::default(),
    )
    .unwrap()
}
fn document_with_canvas() -> (Document, usize) {
    let mut doc = Document::new();
    let html = doc.create_detached_element("html").unwrap();
    doc.attach_child(doc.root_index(), html);
    let body = doc.create_detached_element("body").unwrap();
    doc.append_child(html, body).unwrap();
    let canvas = doc.create_detached_element("canvas").unwrap();
    doc.append_child(body, canvas).unwrap();
    doc.set_element_attribute(canvas, "width", "2").unwrap();
    doc.set_element_attribute(canvas, "height", "2").unwrap();
    assert!(doc.canvas_fill_rect(canvas, 0, 0, 2, 2, [255, 0, 0, 255]));
    doc.mark_in_document_flags();
    (doc, canvas)
}
#[test]
fn active_page_takes_canvas_sidecar_by_moving_pixels() {
    let (doc, canvas) = document_with_canvas();
    let mut page = page_with_document(doc);

    let sidecar = page.take_canvases_in_tree_order();
    assert_eq!(sidecar.len(), 1);
    assert_eq!((sidecar[0].width, sidecar[0].height), (2, 2));
    assert_eq!(&sidecar[0].rgba[..4], &[255, 0, 0, 255]);
    assert!(page.document().canvas_bitmap(canvas).is_none());
}
#[test]
fn stopped_page_takes_canvas_sidecar_by_moving_pixels() {
    let (doc, canvas) = document_with_canvas();
    let mut page = page_with_document(doc);
    assert!(page.evaluate_preamble("throw new Error('stop')").is_err());

    let sidecar = page.take_canvases_in_tree_order();
    assert_eq!(sidecar.len(), 1);
    assert_eq!((sidecar[0].width, sidecar[0].height), (2, 2));
    assert_eq!(&sidecar[0].rgba[..4], &[255, 0, 0, 255]);
    assert!(page.document().canvas_bitmap(canvas).is_none());
}
#[test]
fn real_guest_runtime_and_independent_stores() {
    let mut first = page();
    let mut second = page();
    first
        .evaluate_preamble("document.title='first'; window.privateState=42")
        .unwrap();
    second.evaluate_preamble("if (typeof privateState !== 'undefined') throw Error('state leaked'); document.title='second'").unwrap();
    first.run_document().unwrap();
    second.run_document().unwrap();
    first.synchronize_final_document().unwrap();
    second.synchronize_final_document().unwrap();
    assert!(first.metrics().unwrap().consumed_fuel > 0);
    assert!(first.metrics().unwrap().guest_request_bytes > 0);
    assert!(first.metrics().unwrap().guest_response_bytes > 0);
}

#[test]
fn concurrent_store_trap_does_not_stop_another_store() {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let stop_barrier = barrier.clone();
    let stopped = std::thread::spawn(move || {
        let mut runtime = page();
        stop_barrier.wait();
        assert!(
            runtime
                .evaluate_preamble("Array.prototype.forEach.call({length:1e15},()=>{})")
                .unwrap_err()
                .aborted
        );
    });
    let alive = std::thread::spawn(move || {
        let mut runtime = page();
        barrier.wait();
        runtime
            .evaluate_preamble("document.title='concurrent survivor'")
            .unwrap();
        runtime.run_document().unwrap();
        runtime.synchronize_final_document().unwrap();
    });
    stopped.join().unwrap();
    alive.join().unwrap();
}

#[test]
fn mismatched_metering_configuration_rejects_trusted_artifact() {
    let mut config = super::config::make();
    config.consume_fuel(false);
    let engine = wasmtime::Engine::new(&config).unwrap();
    // SAFETY: the embedded bytes are trusted compiler output; this API copies
    // its input and owns its code pages, independent of the static publisher.
    let result = unsafe { wasmtime::Module::deserialize(&engine, super::loader::artifact()) };
    assert!(
        result.is_err(),
        "an incompatible fuel configuration must be rejected"
    );
}
#[test]
fn hostile_builtin_stops_and_next_document_runs() {
    let mut p = page();
    let error = p
        .evaluate_preamble("Array.prototype.forEach.call({length:1e15},function(){})")
        .unwrap_err();
    assert!(error.aborted, "{error}");
    assert!(p.run_document().is_err());
    page().evaluate_preamble("document.title='alive'").unwrap();
}
#[test]
fn invalid_pointer_ranges_are_rejected() {
    assert!(checked_range(u32::MAX, 16, 128 * 1024 * 1024).is_err());
    assert!(checked_range(0, (MAX_MESSAGE + 1) as u32, 128 * 1024 * 1024).is_err());
    assert!(checked_range(16, 16, 31).is_err());
    assert_eq!(checked_range(16, 16, 32).unwrap(), 16..32);
}
#[test]
fn allocation_cap_is_an_abort_and_next_document_runs() {
    let mut p = WasmtimePage::new(
        Box::new(TestHost {
            doc: Document::new(),
            flushes: Rc::new(Cell::new(0)),
        }),
        Default::default(),
        SandboxOptions {
            fuel: 10_000_000_000,
            memory_bytes: 8 * 1024 * 1024,
        },
    )
    .unwrap();
    let e = p
        .evaluate_preamble("new Array(2000000).fill('allocated')")
        .unwrap_err();
    assert!(e.aborted, "memory exhaustion must be an abort: {e}");
    assert!(p.metrics().is_some(), "retain metrics after stop");
    page().evaluate_preamble("1+1").unwrap();
}

#[test]
fn repeated_calls_after_stop_keep_native_checkpoint() {
    let mut p = page();
    let before = p.document().logical_snapshot();
    let _=p.evaluate_preamble("document.title='unsynchronized'; Array.prototype.forEach.call({length:1e15},function(){})").unwrap_err();
    assert_eq!(p.document().logical_snapshot(), before);
    assert!(p.run_document().is_err());
    assert_eq!(p.document().logical_snapshot(), before);
}

#[test]
fn initialization_memory_cap_is_an_abort() {
    let result = WasmtimePage::new(
        Box::new(TestHost {
            doc: Document::new(),
            flushes: Rc::new(Cell::new(0)),
        }),
        Default::default(),
        SandboxOptions {
            fuel: 10_000_000_000,
            memory_bytes: 65536,
        },
    );
    match result {
        Err(e) => assert!(e.aborted, "{e}"),
        Ok(_) => panic!("initial guest memory must exceed one page"),
    }
}

#[test]
fn hostile_regex_call_tree_parser_and_report_getter_are_contained() {
    let cases=vec![
        "/(a+)+$/.test('a'.repeat(100)+'!')".to_owned(),
        "function f(n){if(n){f(n-1);f(n-1);}} f(50)".to_owned(),
        format!("{}0{}", "[".repeat(20000), "]".repeat(20000)),
        "var sink=globalThis[Object.getOwnPropertySymbols(globalThis).find(s=>s.description==='raikiri testharness report sink')]; sink([{get name(){ Array.prototype.forEach.call({length:1e15},()=>{});},status:0}],{status:0})".to_owned(),
    ];
    for source in cases {
        let mut p = page();
        let error = p.evaluate_preamble(&source).unwrap_err();
        assert!(error.aborted, "{error}");
        assert!(p.run_document().is_err());
        page()
            .evaluate_preamble("document.title='survived'")
            .unwrap();
    }
}

struct FailHost {
    doc: Document,
}
impl DocumentHost for FailHost {
    fn document(&self) -> &Document {
        &self.doc
    }
    fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }
    fn flush(&mut self) -> Result<(), HostError> {
        Err(HostError("flush failed".into()))
    }
    fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
        Err(HostError("geometry failed".into()))
    }
    fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
        Err(HostError("computed failed".into()))
    }
    fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
        Err(HostError("fragment failed".into()))
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

fn bridge() -> Bridge {
    Bridge::new(
        Box::new(TestHost {
            doc: Document::new(),
            flushes: Rc::new(Cell::new(0)),
        }),
        100,
    )
}

fn roundtrip_response(bridge: &mut Bridge, op: HostOperation) -> Result<HostValue, String> {
    let bytes = encode(&Request {
        version: VERSION,
        operation: op,
    })
    .unwrap();
    let len = bridge.request(&bytes).unwrap();
    let mut out = vec![0; len];
    bridge.copy_response(&mut out).unwrap();
    let response: Response = decode(&out).unwrap();
    response.result
}

#[test]
fn hostile_request_rejects_bad_version_and_corrupt_bytes() {
    let mut b = bridge();
    assert!(b.request(b"not json").is_err());
    let bad_version = encode(&Request {
        version: VERSION + 1,
        operation: HostOperation::Fetch("x".into()),
    })
    .unwrap();
    assert!(b.request(&bad_version).is_err());

    // Unconsumed response blocks the next request.
    let mut b = bridge();
    let bytes = encode(&Request {
        version: VERSION,
        operation: HostOperation::Fetch("x".into()),
    })
    .unwrap();
    b.request(&bytes).unwrap();
    assert!(b.request(&bytes).is_err());
}

#[test]
fn hostile_flush_rejects_invalid_snapshot() {
    let mut b = bridge();
    let empty = raikiri_dom::snapshot::LogicalSnapshot {
        root: 0,
        nodes: Vec::new(),
        quirks: 0,
        stylesheets: Vec::new(),
    };
    assert!(roundtrip_response(&mut b, HostOperation::Flush(empty)).is_err());
}

#[test]
fn hostile_geometry_and_computed_enforce_bounds() {
    let mut b = bridge();
    // Document has one node, so index 99 is out of bounds.
    assert!(roundtrip_response(&mut b, HostOperation::Geometry(99)).is_err());
    assert!(roundtrip_response(&mut b, HostOperation::Computed(99, "color".into())).is_err());
    // In-bounds indices succeed through the test host.
    assert!(roundtrip_response(&mut b, HostOperation::Geometry(0)).is_ok());
    assert!(roundtrip_response(&mut b, HostOperation::Computed(0, "color".into())).is_ok());
}

#[test]
fn hostile_fragment_and_fetch_propagate_host_errors() {
    // Default test host fails fetches (unsupported) but succeeds fragments.
    let mut b = bridge();
    assert!(roundtrip_response(&mut b, HostOperation::Fetch("http://x/".into())).is_err());
    assert!(
        roundtrip_response(
            &mut b,
            HostOperation::Fragment {
                tag: "div".into(),
                namespace: "".into(),
                markup: "hi".into(),
            },
        )
        .is_ok()
    );

    // Failing host propagates flush/geometry/computed/fragment errors.
    let mut failing = Bridge::new(
        Box::new(FailHost {
            doc: Document::new(),
        }),
        100,
    );
    let valid = Document::new().logical_snapshot();
    assert!(roundtrip_response(&mut failing, HostOperation::Flush(valid)).is_err());
    assert!(roundtrip_response(&mut failing, HostOperation::Geometry(0)).is_err());
    assert!(roundtrip_response(&mut failing, HostOperation::Computed(0, "color".into())).is_err());
    assert!(
        roundtrip_response(
            &mut failing,
            HostOperation::Fragment {
                tag: "div".into(),
                namespace: "".into(),
                markup: "hi".into(),
            },
        )
        .is_err()
    );
}

#[test]
fn hostile_response_capacity_rules() {
    let mut b = bridge();
    assert!(b.take_response(16).is_err());
    let bytes = encode(&Request {
        version: VERSION,
        operation: HostOperation::Fetch("x".into()),
    })
    .unwrap();
    let len = b.request(&bytes).unwrap();
    assert!(b.take_response(len - 1).is_err());
    assert!(b.take_response(MAX_MESSAGE + 1).is_err());
    let taken = b.take_response(len).unwrap();
    assert_eq!(taken.len(), len);
    assert!(b.take_response(len).is_err());
}

#[test]
fn hostile_code_publisher_rejects_out_of_bounds_ranges() {
    use wasmtime::CustomCodeMemory;
    let publisher = super::loader::Publisher;
    assert_eq!(publisher.required_alignment(), 4096);
    let base = super::loader::artifact().as_ptr();
    // Zero-length publish inside the artifact succeeds without changing protections.
    assert!(publisher.publish_executable(base, 0).is_ok());
    assert!(publisher.unpublish_executable(base, 0).is_ok());
    // Unaligned address and length are rejected before any syscall.
    assert!(
        publisher
            .publish_executable(unsafe { base.add(1) }, 4096)
            .is_err()
    );
    assert!(publisher.publish_executable(base, 100).is_err());
    // Ranges outside the embedded artifact are rejected.
    assert!(publisher.publish_executable(base, usize::MAX).is_err());
    let outside = usize::MAX as *const u8;
    assert!(publisher.publish_executable(outside, 4096).is_err());
}
