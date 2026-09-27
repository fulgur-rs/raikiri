//! Guest owns the actual DOM runtime; native services use owned messages.
mod rpc_host;
pub use rpc_host::RpcDocumentHost;
#[cfg(target_arch = "wasm32")]
mod exports;
#[cfg(test)]
mod tests;
