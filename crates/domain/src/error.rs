use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid function id: {0}")]
    InvalidFunctionId(String),

    #[error("invalid version label: {0}")]
    InvalidVersionLabel(String),

    #[error("invalid content hash: {0}")]
    InvalidContentHash(String),

    #[error("invalid egress origin: {0}")]
    InvalidEgressOrigin(String),

    #[error("too many egress origins (max {max})")]
    TooManyEgressOrigins { max: usize },

    #[error("invalid account id")]
    InvalidAccountId,

    #[error("invalid key id")]
    InvalidKeyId,

    #[error("invalid bearer")]
    InvalidBearer,

    #[error("bearer is not a non-revoked key for this account")]
    NotKeyHolder,

    #[error("entropy unavailable")]
    Entropy,
}
