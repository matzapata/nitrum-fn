//! Product limits for publish / invoke (functions 0.1).

/// Max raw `.wasm` upload size (and seed / S3 get).
pub const MAX_WASM_BYTES: usize = 2 * 1024 * 1024;

/// Max HTTP invoke request body (before wire encoding).
pub const MAX_INVOKE_BODY_BYTES: usize = 1024 * 1024;

/// Max guest `invoke` return buffer.
pub const MAX_GUEST_OUTPUT_BYTES: usize = 1024 * 1024;

/// Max guest linear memory (Wasmtime `StoreLimits`).
pub const MAX_GUEST_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Max AOT `.cwasm` artifact size (8× wasm).
pub const MAX_COMPILED_BYTES: usize = 8 * MAX_WASM_BYTES;

/// Wall-clock guest invoke deadline.
pub const INVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Epoch ticker interval used with [`INVOKE_TIMEOUT`].
pub const EPOCH_TICK: std::time::Duration = std::time::Duration::from_millis(10);
