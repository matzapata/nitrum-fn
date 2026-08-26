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

    /// Guest hit the wall-clock invoke deadline (Wasmtime epoch interrupt).
    #[error("invoke timed out: {0}")]
    Timeout(String),

    /// Request body, wasm artifact, or guest output exceeded a product limit.
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),
}

impl AppError {
    /// True for failures whose Display may include driver / guest internals.
    pub fn is_internal(&self) -> bool {
        matches!(self, Self::Invoke(_) | Self::Trap(_) | Self::Storage(_))
    }

    /// Stable client-facing message. Log [`Display`] separately for internals.
    pub fn public_message(&self) -> String {
        if self.is_internal() {
            "internal error".into()
        } else {
            self.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn internal_errors_do_not_leak_in_public_message() {
        let err = AppError::Storage("table nitrum-fn AccessDeniedException".into());
        assert_eq!(err.public_message(), "internal error");
        assert!(err.to_string().contains("AccessDeniedException"));
    }

    #[test]
    fn client_errors_keep_their_display() {
        let err = AppError::NotFound("echo@latest".into());
        assert_eq!(err.public_message(), err.to_string());
    }
}
