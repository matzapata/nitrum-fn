//! Pure domain types for nitrum-fn.

mod error;
mod function;
mod invoke;
mod limits;
mod publish;

pub use error::DomainError;
pub use function::{
    normalize_egress_allow, ContentHash, EgressOrigin, FunctionId, FunctionVersion, VersionLabel,
};
pub use invoke::{InvokeRequest, InvokeResponse};
pub use limits::{
    EPOCH_TICK, HTTP_TIMEOUT, INVOKE_TIMEOUT, MAX_EGRESS_ALLOW, MAX_GUEST_MEMORY_BYTES,
    MAX_GUEST_OUTPUT_BYTES, MAX_HTTP_BODY_BYTES, MAX_HTTP_URL_BYTES, MAX_INVOKE_BODY_BYTES,
    MAX_WASM_BYTES,
};
pub use publish::{PublishRequest, PublishResponse, PublishStatus};
