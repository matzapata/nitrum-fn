//! Wasmtime-backed `FunctionRunner`.
//!
//! Module ABI (v0):
//! - export `memory`
//! - export `invoke(ptr, len) -> len` — wire Request JSON in, wire Response JSON out
//!   (handler is registered lazily on first `invoke`)

mod http_host;
mod wasmtime_runner;

pub use http_host::{HttpClient, SharedHttpClient, StubHttpClient, UreqClient};
pub use wasmtime_runner::WasmtimeRunner;
