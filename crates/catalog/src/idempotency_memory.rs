use std::collections::HashMap;
use std::sync::RwLock;

use application::error::AppError;
use application::ports::{IdempotencyClaim, IdempotencyRecord, PublishIdempotency};
use async_trait::async_trait;
use domain::IdempotencyKey;

use crate::idempotency::{claim_in_map, complete_in_map, unix_now, Stored};

#[derive(Debug, Default)]
pub struct InMemoryPublishIdempotency {
    records: RwLock<HashMap<String, Stored>>,
}

impl InMemoryPublishIdempotency {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PublishIdempotency for InMemoryPublishIdempotency {
    async fn claim(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<IdempotencyClaim, AppError> {
        let mut records = self.records.write().unwrap_or_else(|e| e.into_inner());
        claim_in_map(&mut records, unix_now(), key, record)
    }

    async fn complete(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<(), AppError> {
        let mut records = self.records.write().unwrap_or_else(|e| e.into_inner());
        complete_in_map(&mut records, unix_now(), key, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::{IdempotencyClaim, IdempotencyStatus};
    use domain::{ContentHash, FunctionId};

    fn rec(wasm: &[u8]) -> IdempotencyRecord {
        IdempotencyRecord {
            function: FunctionId::new("echo").unwrap(),
            content_hash: ContentHash::from_bytes(wasm),
            wasm_bytes: wasm.len(),
            status: IdempotencyStatus::Pending,
        }
    }

    fn key(raw: &str) -> IdempotencyKey {
        IdempotencyKey::new(raw).unwrap()
    }

    #[tokio::test]
    async fn claim_complete_replay_and_conflict() {
        let store = InMemoryPublishIdempotency::new();
        let k = key("retry-1");
        let first = rec(b"one");
        assert_eq!(
            store.claim(&k, &first).await.unwrap(),
            IdempotencyClaim::Proceed
        );
        store.complete(&k, &first).await.unwrap();
        assert!(matches!(
            store.claim(&k, &first).await.unwrap(),
            IdempotencyClaim::Replay(_)
        ));

        let other = rec(b"two");
        let err = store.claim(&k, &other).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "{err}");
    }
}
