//! Pure domain types for nitrum-fn. No I/O, no frameworks.

mod error;
mod function;
mod invoke;
mod limits;
mod publish;

pub use error::DomainError;
pub use function::{ContentHash, FunctionId, FunctionVersion, VersionLabel};
pub use invoke::{InvokeRequest, InvokeResponse};
pub use limits::{
    EPOCH_TICK, INVOKE_TIMEOUT, MAX_COMPILED_BYTES, MAX_GUEST_MEMORY_BYTES, MAX_GUEST_OUTPUT_BYTES,
    MAX_INVOKE_BODY_BYTES, MAX_WASM_BYTES,
};
pub use publish::{PublishQueuedEvent, PublishRequest, PublishResponse};
