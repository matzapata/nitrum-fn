//! Pure domain types for nitrum-fn. No I/O, no frameworks.

mod error;
mod function;
mod invoke;

pub use error::DomainError;
pub use function::{ContentHash, FunctionId, FunctionVersion, VersionLabel};
pub use invoke::{InvokeRequest, InvokeResponse};
