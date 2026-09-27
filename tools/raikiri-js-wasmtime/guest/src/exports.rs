use super::RpcDocumentHost;
use raikiri_dom::Document;
use raikiri_js::runtime::DomRuntime;
use raikiri_js_wasmtime_harness as harness;
use raikiri_js_wasmtime_protocol::*;
use std::{cell::RefCell, collections::HashMap};
thread_local! {
    static BUFFERS: RefCell<HashMap<u32, Box<[u8]>>> = RefCell::new(HashMap::new());
    static RUNTIME: RefCell<Option<DomRuntime>> = const { RefCell::new(None) };
}
#[link(wasm_import_module = "raikiri")]
unsafe extern "C" {
    fn host_request(ptr: u32, len: u32) -> i32;
    fn host_response_copy(ptr: u32, capacity: u32) -> i32;
}
fn keep(bytes: Vec<u8>) -> (u32, u32) {
    let buf = bytes.into_boxed_slice();
    let ptr = buf.as_ptr() as u32;
    let len = buf.len() as u32;
    BUFFERS.with(|b| {
        assert!(b.borrow_mut().insert(ptr, buf).is_none());
    });
    (ptr, len)
}
#[unsafe(no_mangle)]
pub extern "C" fn guest_alloc(len: u32) -> u32 {
    assert!(len > 0 && len as usize <= MAX_MESSAGE);
    keep(vec![0; len as usize]).0
}
#[unsafe(no_mangle)]
pub extern "C" fn guest_free(ptr: u32, len: u32) {
    BUFFERS.with(|b| {
        let mut buffers = b.borrow_mut();
        assert_eq!(buffers.get(&ptr).map(|b| b.len()), Some(len as usize));
        buffers.remove(&ptr);
    });
}
fn rpc(operation: HostOperation) -> Result<HostValue, String> {
    let request = encode(&Request {
        version: VERSION,
        operation,
    })?;
    // SAFETY: request storage is live and immutable during this synchronous import.
    let len = unsafe { host_request(request.as_ptr() as u32, request.len() as u32) };
    if len <= 0 || len as usize > MAX_MESSAGE {
        return Err("host request failed".into());
    }
    let mut bytes = vec![0; len as usize];
    // SAFETY: response buffer is writable for its full length; import copies once.
    if unsafe { host_response_copy(bytes.as_mut_ptr() as u32, len as u32) } != len {
        return Err("host response copy failed".into());
    }
    let response: Response = decode(&bytes)?;
    if response.version != VERSION {
        return Err("response version".into());
    }
    response.result
}
fn dispatch(request: GuestRequest) -> Result<GuestValue, String> {
    if request.version != VERSION {
        return Err("guest protocol version".into());
    }
    RUNTIME.with(|state| {
        let mut state = state.borrow_mut();
        if let GuestOperation::Create(c) = request.operation {
            if state.is_some() {
                return Err("page already created".into());
            }
            let max_nodes = c.limits.nodes;
            let document = Document::from_logical_snapshot(c.document, max_nodes)
                .map_err(|e| e.to_string())?;
            let host = RpcDocumentHost::new(document, c.document_url, max_nodes, Box::new(rpc));
            let mut runtime = DomRuntime::with_options(host, c.limits.into_options())
                .map_err(|e| e.to_string())?;
            harness::install_page_support(&mut runtime).map_err(|e| e.to_string())?;
            *state = Some(runtime);
            return Ok(GuestValue::Unit);
        }
        if matches!(request.operation, GuestOperation::TakeDocument) {
            let host = state.take().ok_or("page not created")?.into_host();
            return Ok(GuestValue::Document(host.document().logical_snapshot()));
        }
        let runtime = state.as_mut().ok_or("page not created")?;
        match request.operation {
            GuestOperation::Evaluate(s) => {
                runtime.evaluate(&s).map_err(|e| e.to_string())?;
                Ok(GuestValue::Unit)
            }
            GuestOperation::RunDocument => Ok(GuestValue::Report(runtime.run_document().into())),
            GuestOperation::ProbeTimeout(r) => {
                let mut report = r.into_native();
                // Preserve probe delivery for the following TakeResults operation.
                if let Some(delivery) = harness::probe_timeout(runtime, &mut report) {
                    runtime.context_mut().insert_data(delivery);
                }
                Ok(GuestValue::Report(report.into()))
            }
            GuestOperation::TakeResults => Ok(GuestValue::Delivery(
                harness::take_delivery(runtime).map(|d| DeliveryDto {
                    tests: d
                        .tests
                        .into_iter()
                        .map(|t| TestOutcomeDto {
                            name: t.name,
                            passed: t.passed,
                            message: t.message,
                        })
                        .collect(),
                    harness_status: d.harness_status,
                    harness_message: d.harness_message,
                }),
            )),
            _ => Err("operation state".into()),
        }
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn guest_call(ptr: u32, len: u32) -> u64 {
    let request = BUFFERS.with(|buffers| {
        let b = buffers.borrow();
        let bytes = b.get(&ptr).ok_or("unknown buffer")?;
        if bytes.len() != len as usize {
            return Err("buffer length".into());
        }
        decode::<GuestRequest>(bytes)
    });
    let result = request.and_then(dispatch);
    let response = GuestResponse {
        version: VERSION,
        result,
    };
    let bytes = encode(&response).unwrap_or_else(|error| {
        encode(&GuestResponse {
            version: VERSION,
            result: Err(error),
        })
        .expect("bounded error")
    });
    let (ptr, len) = keep(bytes);
    ((len as u64) << 32) | ptr as u64
}
