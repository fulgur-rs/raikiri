pub fn make() -> wasmtime::Config {
    let mut config = wasmtime::Config::default();
    config.consume_fuel(true);
    config.max_wasm_stack(512 * 1024);
    config.memory_reservation(128 * 1024 * 1024);
    config.memory_guard_size(64 * 1024);
    config
}
