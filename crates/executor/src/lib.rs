//! Wasmtime-backed `FunctionRunner`.
//!
//! Module ABI (v0):
//! - export `memory`
//! - optional export `main()` — called once per instance to run `runtime::run(...)`
//! - export `invoke(ptr, len) -> len` — wire Request JSON in, wire Response JSON out

mod module_cache;
mod wasmtime_runner;

pub use wasmtime_runner::WasmtimeRunner;
