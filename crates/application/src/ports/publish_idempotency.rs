use async_trait::async_trait;
use domain::{ContentHash, FunctionId, IdempotencyKey};

use crate::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyStatus {
    Pending,
    Completed,
}

/// Publish attempt keyed by `{function}#{Idempotency-Key}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyRecord {
    pub function: FunctionId,
    pub content_hash: ContentHash,
    pub wasm_bytes: usize,
    pub status: IdempotencyStatus,
}

/// Result of [`PublishIdempotency::claim`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyClaim {
    /// Missing, expired, or still pending for this function+hash. Enqueue, then [`PublishIdempotency::complete`].
    Proceed,
    /// Already completed for this function+hash. Do not enqueue.
    Replay(IdempotencyRecord),
}

/// Decide how to treat a live (non-expired) row for a new publish of `proposed`.
pub fn evaluate_claim(
    existing: &IdempotencyRecord,
    proposed: &IdempotencyRecord,
) -> Result<IdempotencyClaim, AppError> {
    if existing.function != proposed.function || existing.content_hash != proposed.content_hash {
        return Err(AppError::Conflict(format!(
            "key reused for a different publish (have {}@{})",
            existing.function,
            existing.content_hash.to_hex()
        )));
    }
    match existing.status {
        IdempotencyStatus::Completed => Ok(IdempotencyClaim::Replay(existing.clone())),
        IdempotencyStatus::Pending => Ok(IdempotencyClaim::Proceed),
    }
}

#[async_trait]
pub trait PublishIdempotency: Send + Sync {
    /// Reserve or inspect the key *before* artifact put / enqueue.
    ///
    /// Same key + different function or hash is [`AppError::Conflict`].
    async fn claim(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<IdempotencyClaim, AppError>;

    /// Mark the claim completed after a successful enqueue.
    async fn complete(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<(), AppError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::FunctionId;

    fn rec(wasm: &[u8], status: IdempotencyStatus) -> IdempotencyRecord {
        IdempotencyRecord {
            function: FunctionId::new("echo").unwrap(),
            content_hash: ContentHash::from_bytes(wasm),
            wasm_bytes: wasm.len(),
            status,
        }
    }

    #[test]
    fn completed_same_payload_replays() {
        let existing = rec(b"one", IdempotencyStatus::Completed);
        let proposed = rec(b"one", IdempotencyStatus::Pending);
        assert_eq!(
            evaluate_claim(&existing, &proposed).unwrap(),
            IdempotencyClaim::Replay(existing)
        );
    }

    #[test]
    fn pending_same_payload_proceeds() {
        let existing = rec(b"one", IdempotencyStatus::Pending);
        let proposed = rec(b"one", IdempotencyStatus::Pending);
        assert_eq!(
            evaluate_claim(&existing, &proposed).unwrap(),
            IdempotencyClaim::Proceed
        );
    }

    #[test]
    fn different_hash_conflicts() {
        let existing = rec(b"one", IdempotencyStatus::Pending);
        let proposed = rec(b"two", IdempotencyStatus::Pending);
        let err = evaluate_claim(&existing, &proposed).unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "{err}");
    }
}
