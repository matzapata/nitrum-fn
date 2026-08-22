use domain::DomainError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error("function not found: {0}")]
    NotFound(String),

    /// Same `Idempotency-Key` with a different function or wasm hash.
    #[error("idempotency conflict: {0}")]
    Conflict(String),

    #[error("artifact missing for hash {0}")]
    ArtifactMissing(String),

    #[error("artifact hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("compile failed: {0}")]
    Compile(String),

    #[error("storage: {0}")]
    Storage(String),

    #[error("invoke failed: {0}")]
    Invoke(String),

    /// Guest trapped while executing `invoke` (Wasmtime trap / panic / unreachable).
    #[error("guest trap: {0}")]
    Trap(String),
}
