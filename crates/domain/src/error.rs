use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid function id: {0}")]
    InvalidFunctionId(String),

    #[error("invalid version label: {0}")]
    InvalidVersionLabel(String),

    #[error("invalid content hash: {0}")]
    InvalidContentHash(String),

    #[error("invalid idempotency key: {0}")]
    InvalidIdempotencyKey(String),
}
