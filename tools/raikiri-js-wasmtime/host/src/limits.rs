//! Track limiter failures separately from Wasmtime instruction traps.
pub(crate) struct PageLimits {
    inner: wasmtime::StoreLimits,
    pub exhausted: bool,
}
impl PageLimits {
    pub fn new(bytes: usize) -> Self {
        Self {
            inner: wasmtime::StoreLimitsBuilder::new()
                .memory_size(bytes)
                .trap_on_grow_failure(true)
                .build(),
            exhausted: false,
        }
    }
}
impl wasmtime::ResourceLimiter for PageLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let result = self.inner.memory_growing(current, desired, maximum);
        if !matches!(result, Ok(true)) {
            self.exhausted = true;
        }
        result
    }
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.exhausted = true;
        self.inner.memory_grow_failed(error)
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let result = self.inner.table_growing(current, desired, maximum);
        if !matches!(result, Ok(true)) {
            self.exhausted = true;
        }
        result
    }
    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.exhausted = true;
        self.inner.table_grow_failed(error)
    }
}
