//! Product limits for publish / invoke (functions 0.1).

/// Max raw `.wasm` upload size (and seed / S3 get).
pub const MAX_WASM_BYTES: usize = 2 * 1024 * 1024;

/// Max HTTP invoke request body (before wire encoding).
pub const MAX_INVOKE_BODY_BYTES: usize = 1024 * 1024;

/// Max guest `invoke` return buffer.
pub const MAX_GUEST_OUTPUT_BYTES: usize = 1024 * 1024;

/// Max guest linear memory (Wasmtime `StoreLimits`).
pub const MAX_GUEST_MEMORY_BYTES: usize = 64 * 1024 * 1024;

/// Wall-clock guest invoke deadline.
pub const INVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Epoch ticker interval used with [`INVOKE_TIMEOUT`].
pub const EPOCH_TICK: std::time::Duration = std::time::Duration::from_millis(10);

/// Per-function HTTPS origin allowlist size.
pub const MAX_EGRESS_ALLOW: usize = 8;

/// Guest outbound GET URL length cap.
pub const MAX_HTTP_URL_BYTES: usize = 2048;

/// Guest outbound GET response body cap.
pub const MAX_HTTP_BODY_BYTES: usize = 256 * 1024;

/// Guest outbound GET wall-clock timeout.
pub const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
