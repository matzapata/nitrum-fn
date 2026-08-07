use domain::DomainError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error("function not found: {0}")]
    NotFound(String),

    #[error("artifact missing for hash {0}")]
    ArtifactMissing(String),

    #[error("artifact hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("invoke failed: {0}")]
    Invoke(String),
}
