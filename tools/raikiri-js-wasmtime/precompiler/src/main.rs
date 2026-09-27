#[path = "../../host/src/config.rs"]
mod config;
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: precompiler guest.wasm output.cwasm"
    );
    let engine = wasmtime::Engine::new(&config::make())?;
    let wasm = std::fs::read(&args[1])?;
    let module = wasmtime::Module::new(&engine, &wasm)?;
    for import in module.imports() {
        println!(
            "import {}::{} {:?}",
            import.module(),
            import.name(),
            import.ty()
        );
    }
    let artifact = engine.precompile_module(&wasm)?;
    std::fs::write(&args[2], &artifact)?;
    println!("guest_bytes={} aot_bytes={}", wasm.len(), artifact.len());
    Ok(())
}
