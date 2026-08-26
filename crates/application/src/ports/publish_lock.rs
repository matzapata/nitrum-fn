use async_trait::async_trait;
use domain::{ContentHash, FunctionId};

use crate::AppError;

/// Per-function lock for concurrent publish serialization (Lambda-style).
///
/// One live lock per [`FunctionId`]. Held from publish accept until compile
/// succeeds (or TTL expires). A second acquire while live is [`AppError::Conflict`].
#[async_trait]
pub trait PublishLock: Send + Sync {
    /// Reserve `function` for this hash generation. Fails with conflict if a
    /// non-expired lock already exists.
    async fn acquire(
        &self,
        function: &FunctionId,
        hash: &ContentHash,
        queued_at_ms: u64,
    ) -> Result<(), AppError>;

    /// Drop the lock only if it still points at `hash`. Wrong-hash deletes are
    /// ignored so a late worker cannot unlock a newer publish.
    async fn release(&self, function: &FunctionId, hash: &ContentHash) -> Result<(), AppError>;
}
