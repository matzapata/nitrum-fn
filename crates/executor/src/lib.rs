//! Wasmtime-backed `FunctionRunner`.
//!
//! Module ABI (v0):
//! - export `memory`
//! - export `invoke(ptr, len) -> len` — wire Request JSON in, wire Response JSON out
//!   (handler is registered lazily on first `invoke`)

mod wasmtime_runner;

pub use wasmtime_runner::WasmtimeRunner;
