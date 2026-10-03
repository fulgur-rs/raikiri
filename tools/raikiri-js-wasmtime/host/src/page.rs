use super::{Bridge, BridgeMetrics, State, checked_range};
use raikiri_js::runtime::{DocumentHost, RunOptions};
use raikiri_js_wasmtime_protocol::*;
use std::sync::OnceLock;
use wasmtime::{Caller, Engine, Linker, Memory, Module, Store, TypedFunc};
#[derive(Debug, Clone)]
pub struct SandboxOptions {
    pub fuel: u64,
    pub memory_bytes: usize,
}
impl Default for SandboxOptions {
    fn default() -> Self {
        Self {
            fuel: 10_000_000_000,
            memory_bytes: 128 * 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub struct EngineError {
    pub message: String,
    pub aborted: bool,
}
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for EngineError {}
impl From<wasmtime::Error> for EngineError {
    fn from(e: wasmtime::Error) -> Self {
        let aborted = e.downcast_ref::<wasmtime::Trap>().is_some();
        Self {
            message: format!("{e:#}"),
            aborted,
        }
    }
}
fn error(message: impl Into<String>) -> EngineError {
    EngineError {
        message: message.into(),
        aborted: false,
    }
}
fn shared_module() -> Result<&'static (Engine, Module), EngineError> {
    static MODULE: OnceLock<Result<(Engine, Module), String>> = OnceLock::new();
    MODULE
        .get_or_init(|| {
            (|| -> anyhow::Result<_> {
                anyhow::ensure!(
                    rustix::param::page_size() == 4096,
                    "zero-copy trial requires 4KiB pages"
                );
                let mut config = super::config::make();
                config.with_custom_code_memory(Some(std::sync::Arc::new(super::loader::Publisher)));
                let engine = Engine::new(&config)?;
                let module = super::loader::load(&engine)?;
                Ok((engine, module))
            })()
            .map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(|e| error(e.clone()))
}
fn memory(caller: &mut Caller<'_, State>) -> wasmtime::Result<Memory> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("missing memory"))
}
fn linker(engine: &Engine) -> wasmtime::Result<Linker<State>> {
    let mut linker = Linker::new(engine);
    super::wasi::imports(&mut linker).map_err(|e| wasmtime::Error::msg(e.to_string()))?;
    linker.func_wrap(
        "raikiri",
        "host_request",
        |mut c: Caller<'_, State>, ptr: u32, len: u32| -> wasmtime::Result<i32> {
            let mem = memory(&mut c)?;
            let range = checked_range(ptr, len, mem.data_size(&c)).map_err(wasmtime::Error::msg)?;
            let bytes = mem.data(&c)[range].to_vec();
            let size = c
                .data_mut()
                .bridge
                .request(&bytes)
                .map_err(wasmtime::Error::msg)?;
            Ok(size as i32)
        },
    )?;
    linker.func_wrap(
        "raikiri",
        "host_response_copy",
        |mut c: Caller<'_, State>, ptr: u32, cap: u32| -> wasmtime::Result<i32> {
            let mem = memory(&mut c)?;
            checked_range(ptr, cap, mem.data_size(&c)).map_err(wasmtime::Error::msg)?;
            let bytes = match c.data_mut().bridge.take_response(cap as usize) {
                Ok(b) => b,
                Err(_) => return Ok(-1),
            };
            mem.write(&mut c, ptr as usize, &bytes)?;
            Ok(bytes.len() as i32)
        },
    )?;
    Ok(linker)
}
#[derive(Debug, Clone)]
pub struct PageMetrics {
    pub bridge: BridgeMetrics,
    pub consumed_fuel: u64,
    pub memory_bytes: usize,
    pub guest_operations: u64,
    pub guest_request_bytes: u64,
    pub guest_response_bytes: u64,
}
pub struct WasmtimePage {
    store: Option<Store<State>>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    free: TypedFunc<(u32, u32), ()>,
    call: TypedFunc<(u32, u32), u64>,
    fuel: u64,
    last_metrics: Option<PageMetrics>,
    stopped_state: Option<State>,
    guest_operations: u64,
    guest_request_bytes: u64,
    guest_response_bytes: u64,
}
impl WasmtimePage {
    pub fn new(
        host: Box<dyn DocumentHost>,
        options: RunOptions,
        sandbox: SandboxOptions,
    ) -> Result<Self, EngineError> {
        let (engine, module) = shared_module()?;
        let create = CreatePage {
            document: host.document().logical_snapshot(),
            document_url: host.document_url(),
            limits: options.clone().into(),
        };
        let state = State {
            limits: super::limits::PageLimits::new(sandbox.memory_bytes),
            bridge: Bridge::new(host, options.limits.max_nodes),
        };
        let mut store = Store::new(engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(sandbox.fuel)?;
        let instance = linker(engine)?
            .instantiate(&mut store, module)
            .map_err(|e| {
                let mut error = EngineError::from(e);
                error.aborted |= store.data().limits.exhausted;
                error
            })?;
        if let Some(init) = instance.get_func(&mut store, "_initialize") {
            init.typed::<(), ()>(&store)?.call(&mut store, ())?;
        }
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| error("missing guest memory"))?;
        let alloc = instance.get_typed_func(&mut store, "guest_alloc")?;
        let free = instance.get_typed_func(&mut store, "guest_free")?;
        let call = instance.get_typed_func(&mut store, "guest_call")?;
        let mut page = Self {
            store: Some(store),
            memory,
            alloc,
            free,
            call,
            fuel: sandbox.fuel,
            last_metrics: None,
            stopped_state: None,
            guest_operations: 0,
            guest_request_bytes: 0,
            guest_response_bytes: 0,
        };
        match page.operation(GuestOperation::Create(create))? {
            GuestValue::Unit => Ok(page),
            _ => Err(error("create response kind")),
        }
    }
    fn operation(&mut self, operation: GuestOperation) -> Result<GuestValue, EngineError> {
        if self.store.is_none() {
            return Err(error("Store discarded"));
        }
        let mut result = (|| -> Result<GuestValue, EngineError> {
            let bytes = encode(&GuestRequest {
                version: VERSION,
                operation,
            })
            .map_err(error)?;
            self.guest_operations += 1;
            self.guest_request_bytes += bytes.len() as u64;
            let store = self
                .store
                .as_mut()
                .ok_or_else(|| error("Store discarded"))?;
            let ptr = self.alloc.call(&mut *store, bytes.len() as u32)?;
            checked_range(ptr, bytes.len() as u32, self.memory.data_size(&*store))
                .map_err(error)?;
            self.memory
                .write(&mut *store, ptr as usize, &bytes)
                .map_err(|e| error(e.to_string()))?;
            let packed = self.call.call(&mut *store, (ptr, bytes.len() as u32))?;
            self.free.call(&mut *store, (ptr, bytes.len() as u32))?;
            let response_ptr = packed as u32;
            let response_len = (packed >> 32) as u32;
            self.guest_response_bytes += response_len as u64;
            let range = checked_range(response_ptr, response_len, self.memory.data_size(&*store))
                .map_err(error)?;
            let response: GuestResponse =
                decode(&self.memory.data(&*store)[range]).map_err(error)?;
            self.free.call(&mut *store, (response_ptr, response_len))?;
            if response.version != VERSION {
                return Err(error("guest response version"));
            }
            response.result.map_err(error)
        })();
        // Any failed export or protocol exchange invalidates the Store. Never re-enter it.
        if let Err(e) = &mut result {
            if let Some(store) = &self.store {
                e.aborted |= store.data().limits.exhausted;
            }
            self.last_metrics = self.metrics();
            self.stopped_state = self.store.take().map(Store::into_data);
        }
        result
    }
    pub fn evaluate_preamble(&mut self, source: &str) -> Result<(), EngineError> {
        match self.operation(GuestOperation::Evaluate(source.into()))? {
            GuestValue::Unit => Ok(()),
            _ => Err(error("evaluate response kind")),
        }
    }
    pub fn run_document(&mut self) -> Result<ReportDto, EngineError> {
        match self.operation(GuestOperation::RunDocument)? {
            GuestValue::Report(r) => Ok(r),
            _ => Err(error("run response kind")),
        }
    }
    pub fn probe_timeout(&mut self, report: ReportDto) -> Result<ReportDto, EngineError> {
        match self.operation(GuestOperation::ProbeTimeout(report))? {
            GuestValue::Report(r) => Ok(r),
            _ => Err(error("probe response kind")),
        }
    }
    pub fn take_results(&mut self) -> Result<Option<DeliveryDto>, EngineError> {
        match self.operation(GuestOperation::TakeResults)? {
            GuestValue::Delivery(d) => Ok(d),
            _ => Err(error("delivery response kind")),
        }
    }
    pub fn synchronize_final_document(&mut self) -> Result<(), EngineError> {
        match self.operation(GuestOperation::TakeDocument)? {
            GuestValue::Document(s) => {
                let store = self
                    .store
                    .as_mut()
                    .ok_or_else(|| error("Store discarded"))?;
                let max = store.data().bridge.max_nodes;
                let document = raikiri_dom::Document::from_logical_snapshot(s, max)
                    .map_err(|e| error(e.to_string()))?;
                *store.data_mut().bridge.host.document_mut() = document;
                Ok(())
            }
            _ => Err(error("document response kind")),
        }
    }
    pub fn document(&self) -> &raikiri_dom::Document {
        let state = if let Some(s) = &self.store {
            s.data()
        } else {
            self.stopped_state.as_ref().expect("initialized page state")
        };
        state.bridge.host.document()
    }
    /// Remove and return host canvas bitmaps in tree order.
    pub fn take_canvases_in_tree_order(&mut self) -> Vec<raikiri_dom::CanvasBitmap> {
        if let Some(store) = self.store.as_mut() {
            store
                .data_mut()
                .bridge
                .host
                .document_mut()
                .take_canvases_in_tree_order()
        } else if let Some(state) = self.stopped_state.as_mut() {
            state
                .bridge
                .host
                .document_mut()
                .take_canvases_in_tree_order()
        } else {
            Vec::new()
        }
    }
    pub fn metrics(&self) -> Option<PageMetrics> {
        let Some(s) = self.store.as_ref() else {
            return self.last_metrics.clone();
        };
        Some(PageMetrics {
            bridge: s.data().bridge.metrics.clone(),
            consumed_fuel: self.fuel - s.get_fuel().ok()?,
            memory_bytes: self.memory.data_size(s),
            guest_operations: self.guest_operations,
            guest_request_bytes: self.guest_request_bytes,
            guest_response_bytes: self.guest_response_bytes,
        })
    }
}
