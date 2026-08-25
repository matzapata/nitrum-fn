use std::sync::Arc;

use domain::{
    ContentHash, PublishQueuedEvent, PublishRequest, PublishResponse, VersionLabel, MAX_WASM_BYTES,
};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{
    ArtifactStore, IdempotencyClaim, IdempotencyRecord, IdempotencyStatus, PublishBus,
    PublishIdempotency,
};

pub struct PublishFunction {
    artifacts: Arc<dyn ArtifactStore>,
    bus: Arc<dyn PublishBus>,
    idempotency: Arc<dyn PublishIdempotency>,
}

impl PublishFunction {
    pub fn new(
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn PublishBus>,
        idempotency: Arc<dyn PublishIdempotency>,
    ) -> Self {
        Self {
            artifacts,
            bus,
            idempotency,
        }
    }

    #[instrument(skip(self, req), fields(function = %req.function, wasm_len = req.wasm.len()))]
    pub async fn execute(&self, req: PublishRequest) -> Result<PublishResponse, AppError> {
        if req.wasm.is_empty() {
            return Err(AppError::Compile("empty wasm".into()));
        }
        if req.wasm.len() > MAX_WASM_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "wasm {} bytes exceeds max {MAX_WASM_BYTES}",
                req.wasm.len()
            )));
        }

        let hash = ContentHash::from_bytes(&req.wasm);
        let record = IdempotencyRecord {
            function: req.function.clone(),
            content_hash: hash,
            wasm_bytes: req.wasm.len(),
            status: IdempotencyStatus::Pending,
        };
        if let Some(key) = &req.idempotency_key {
            match self.idempotency.claim(key, &record).await? {
                IdempotencyClaim::Replay(existing) => {
                    return Ok(PublishResponse {
                        function: req.function,
                        version: VersionLabel::latest(),
                        content_hash: existing.content_hash,
                        wasm_bytes: existing.wasm_bytes,
                        status: "queued",
                    });
                }
                IdempotencyClaim::Proceed => {}
            }
        }

        let stored = self.artifacts.put(&req.wasm).await?;
        let version = VersionLabel::latest();
        let event =
            PublishQueuedEvent::new(req.function.to_string(), stored.to_hex(), req.wasm.len());
        self.bus.publish_queued(&event).await?;

        if let Some(key) = &req.idempotency_key {
            self.idempotency
                .complete(
                    key,
                    &IdempotencyRecord {
                        content_hash: stored.clone(),
                        status: IdempotencyStatus::Completed,
                        ..record
                    },
                )
                .await?;
        }

        Ok(PublishResponse {
            function: req.function,
            version,
            content_hash: stored,
            wasm_bytes: req.wasm.len(),
            status: "queued",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::ports::ArtifactStore;
    use async_trait::async_trait;
    use domain::{FunctionId, IdempotencyKey};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MemArtifacts {
        wasm: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MemArtifacts {
        fn new() -> Self {
            Self {
                wasm: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl ArtifactStore for MemArtifacts {
        async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError> {
            let hash = ContentHash::from_bytes(wasm);
            self.wasm
                .lock()
                .unwrap()
                .insert(hash.to_hex(), wasm.to_vec());
            Ok(hash)
        }

        async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
            self.wasm
                .lock()
                .unwrap()
                .get(&hash.to_hex())
                .cloned()
                .ok_or_else(|| AppError::ArtifactMissing(hash.to_hex()))
        }

        async fn put_compiled(
            &self,
            _hash: &ContentHash,
            _compiled: &[u8],
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
            Err(AppError::ArtifactMissing(hash.to_hex()))
        }
    }

    struct MemBus {
        events: Mutex<Vec<PublishQueuedEvent>>,
        fail: bool,
    }

    impl MemBus {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl PublishBus for MemBus {
        async fn publish_queued(&self, event: &PublishQueuedEvent) -> Result<(), AppError> {
            if self.fail {
                return Err(AppError::Storage("bus down".into()));
            }
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    struct MemIdempotency {
        records: Mutex<HashMap<String, IdempotencyRecord>>,
    }

    impl MemIdempotency {
        fn new() -> Self {
            Self {
                records: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl PublishIdempotency for MemIdempotency {
        async fn claim(
            &self,
            key: &IdempotencyKey,
            record: &IdempotencyRecord,
        ) -> Result<IdempotencyClaim, AppError> {
            catalog_claim(&mut self.records.lock().unwrap(), key, record)
        }

        async fn complete(
            &self,
            key: &IdempotencyKey,
            record: &IdempotencyRecord,
        ) -> Result<(), AppError> {
            let mut records = self.records.lock().unwrap();
            let sk = format!("{}#{}", record.function.as_str(), key.as_str());
            records.insert(
                sk,
                IdempotencyRecord {
                    status: IdempotencyStatus::Completed,
                    ..record.clone()
                },
            );
            Ok(())
        }
    }

    fn catalog_claim(
        records: &mut HashMap<String, IdempotencyRecord>,
        key: &IdempotencyKey,
        proposed: &IdempotencyRecord,
    ) -> Result<IdempotencyClaim, AppError> {
        let sk = format!("{}#{}", proposed.function.as_str(), key.as_str());
        if let Some(existing) = records.get(&sk) {
            return crate::ports::evaluate_claim(existing, proposed);
        }
        records.insert(
            sk,
            IdempotencyRecord {
                status: IdempotencyStatus::Pending,
                ..proposed.clone()
            },
        );
        Ok(IdempotencyClaim::Proceed)
    }

    fn key(raw: &str) -> IdempotencyKey {
        IdempotencyKey::new(raw).unwrap()
    }

    #[tokio::test]
    async fn rejects_empty_wasm() {
        let publish = PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            Arc::new(MemBus::new()),
            Arc::new(MemIdempotency::new()),
        );
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: vec![],
                idempotency_key: None,
            })
            .await
            .expect_err("empty");
        assert!(matches!(err, AppError::Compile(_)), "{err}");
    }

    #[tokio::test]
    async fn rejects_oversize_wasm_without_put() {
        let artifacts = Arc::new(MemArtifacts::new());
        let bus = Arc::new(MemBus::new());
        let publish = PublishFunction::new(
            artifacts.clone(),
            bus.clone(),
            Arc::new(MemIdempotency::new()),
        );
        let wasm = vec![0u8; domain::MAX_WASM_BYTES + 1];
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm,
                idempotency_key: None,
            })
            .await
            .expect_err("too large");
        assert!(matches!(err, AppError::PayloadTooLarge(_)), "{err}");
        assert!(artifacts.wasm.lock().unwrap().is_empty());
        assert!(bus.events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bus_error_after_put_still_stores_wasm() {
        let artifacts = Arc::new(MemArtifacts::new());
        let publish = PublishFunction::new(
            artifacts.clone(),
            Arc::new(MemBus::failing()),
            Arc::new(MemIdempotency::new()),
        );
        let wasm = b"\0asm not empty".to_vec();
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: wasm.clone(),
                idempotency_key: None,
            })
            .await
            .expect_err("bus");
        assert!(matches!(err, AppError::Storage(_)), "{err}");
        let hash = ContentHash::from_bytes(&wasm);
        assert_eq!(artifacts.get(&hash).await.unwrap(), wasm);
    }

    #[tokio::test]
    async fn enqueue_records_hash_and_generation() {
        let bus = Arc::new(MemBus::new());
        let publish = PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            bus.clone(),
            Arc::new(MemIdempotency::new()),
        );
        let wasm = b"\0asm module".to_vec();
        let res = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: wasm.clone(),
                idempotency_key: None,
            })
            .await
            .expect("publish");
        assert_eq!(res.status, "queued");
        let events = bus.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content_hash, res.content_hash.to_hex());
        assert!(events[0].queued_at_ms > 0);
    }

    #[tokio::test]
    async fn same_key_same_body_does_not_enqueue_twice() {
        let bus = Arc::new(MemBus::new());
        let publish = PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            bus.clone(),
            Arc::new(MemIdempotency::new()),
        );
        let wasm = b"\0asm module".to_vec();
        let req = PublishRequest {
            function: FunctionId::new("echo").unwrap(),
            wasm,
            idempotency_key: Some(key("retry-1")),
        };
        let first = publish.execute(req.clone()).await.expect("first");
        let second = publish.execute(req).await.expect("second");
        assert_eq!(first.content_hash, second.content_hash);
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn same_key_different_body_conflicts_without_second_enqueue() {
        let bus = Arc::new(MemBus::new());
        let publish = PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            bus.clone(),
            Arc::new(MemIdempotency::new()),
        );
        let k = Some(key("retry-1"));
        publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm one".to_vec(),
                idempotency_key: k.clone(),
            })
            .await
            .expect("first");
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm two".to_vec(),
                idempotency_key: k,
            })
            .await
            .expect_err("conflict");
        assert!(matches!(err, AppError::Conflict(_)), "{err}");
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pending_claim_retries_enqueue() {
        let bus = Arc::new(MemBus::new());
        let idem = Arc::new(MemIdempotency::new());
        let wasm = b"\0asm module".to_vec();
        let function = FunctionId::new("echo").unwrap();
        let k = key("retry-1");
        let record = IdempotencyRecord {
            function: function.clone(),
            content_hash: ContentHash::from_bytes(&wasm),
            wasm_bytes: wasm.len(),
            status: IdempotencyStatus::Pending,
        };
        idem.claim(&k, &record).await.expect("seed pending");
        let publish = PublishFunction::new(Arc::new(MemArtifacts::new()), bus.clone(), idem);
        publish
            .execute(PublishRequest {
                function,
                wasm,
                idempotency_key: Some(k),
            })
            .await
            .expect("recover");
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn concurrent_different_body_only_winner_enqueues() {
        let bus = Arc::new(MemBus::new());
        let publish = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            bus.clone(),
            Arc::new(MemIdempotency::new()),
        ));
        let k = key("retry-1");
        let a = publish.execute(PublishRequest {
            function: FunctionId::new("echo").unwrap(),
            wasm: b"\0asm one".to_vec(),
            idempotency_key: Some(k.clone()),
        });
        let b = publish.execute(PublishRequest {
            function: FunctionId::new("echo").unwrap(),
            wasm: b"\0asm two".to_vec(),
            idempotency_key: Some(k),
        });
        let (ra, rb) = tokio::join!(a, b);
        let oks = [&ra, &rb].iter().filter(|r| r.is_ok()).count();
        let conflicts = [&ra, &rb]
            .iter()
            .filter(|r| matches!(r, Err(AppError::Conflict(_))))
            .count();
        assert_eq!(oks, 1, "ra={ra:?} rb={rb:?}");
        assert_eq!(conflicts, 1, "ra={ra:?} rb={rb:?}");
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }
}
