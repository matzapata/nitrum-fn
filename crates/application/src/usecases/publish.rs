use crate::error::AppError;
use crate::ports::{ArtifactStore, PublishBus, PublishLock};
use domain::{
    ContentHash, PublishQueuedEvent, PublishRequest, PublishResponse, VersionLabel, MAX_WASM_BYTES,
};
use std::sync::Arc;
use tracing::instrument;

pub struct PublishFunction {
    artifacts: Arc<dyn ArtifactStore>,
    bus: Arc<dyn PublishBus>,
    lock: Arc<dyn PublishLock>,
}

impl PublishFunction {
    pub fn new(
        artifacts: Arc<dyn ArtifactStore>,
        bus: Arc<dyn PublishBus>,
        lock: Arc<dyn PublishLock>,
    ) -> Self {
        Self {
            artifacts,
            bus,
            lock,
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
        let event =
            PublishQueuedEvent::new(req.function.to_string(), hash.to_hex(), req.wasm.len());
        self.lock
            .acquire(&req.function, &hash, event.queued_at_ms)
            .await?;

        let stored = match self.artifacts.put(&req.wasm).await {
            Ok(stored) => stored,
            Err(err) => {
                let _ = self.lock.release(&req.function, &hash).await;
                return Err(err);
            }
        };

        if let Err(err) = self.bus.publish_queued(&event).await {
            let _ = self.lock.release(&req.function, &hash).await;
            return Err(err);
        }

        Ok(PublishResponse {
            function: req.function,
            version: VersionLabel::latest(),
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
    use domain::FunctionId;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MemArtifacts {
        wasm: Mutex<HashMap<String, Vec<u8>>>,
        fail: bool,
    }

    impl MemArtifacts {
        fn new() -> Self {
            Self {
                wasm: Mutex::new(HashMap::new()),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                wasm: Mutex::new(HashMap::new()),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl ArtifactStore for MemArtifacts {
        async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError> {
            if self.fail {
                return Err(AppError::Storage("s3 down".into()));
            }
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

    struct MemLock {
        /// function_id → content_hash hex of the holder
        held: Mutex<HashMap<String, String>>,
        releases: Mutex<u32>,
    }

    impl MemLock {
        fn new() -> Self {
            Self {
                held: Mutex::new(HashMap::new()),
                releases: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl PublishLock for MemLock {
        async fn acquire(
            &self,
            function: &FunctionId,
            hash: &ContentHash,
            _queued_at_ms: u64,
        ) -> Result<(), AppError> {
            let mut held = self.held.lock().unwrap();
            if held.contains_key(function.as_str()) {
                return Err(AppError::Conflict(format!(
                    "publish already in progress for {function}"
                )));
            }
            held.insert(function.as_str().to_string(), hash.to_hex());
            Ok(())
        }

        async fn release(&self, function: &FunctionId, hash: &ContentHash) -> Result<(), AppError> {
            let mut held = self.held.lock().unwrap();
            if held
                .get(function.as_str())
                .is_some_and(|h| h == &hash.to_hex())
            {
                held.remove(function.as_str());
                *self.releases.lock().unwrap() += 1;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn rejects_empty_wasm() {
        let publish = PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            Arc::new(MemBus::new()),
            Arc::new(MemLock::new()),
        );
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: vec![],
            })
            .await
            .expect_err("empty");
        assert!(matches!(err, AppError::Compile(_)), "{err}");
    }

    #[tokio::test]
    async fn rejects_oversize_wasm_without_put() {
        let artifacts = Arc::new(MemArtifacts::new());
        let bus = Arc::new(MemBus::new());
        let lock = Arc::new(MemLock::new());
        let publish = PublishFunction::new(artifacts.clone(), bus.clone(), lock.clone());
        let wasm = vec![0u8; domain::MAX_WASM_BYTES + 1];
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm,
            })
            .await
            .expect_err("too large");
        assert!(matches!(err, AppError::PayloadTooLarge(_)), "{err}");
        assert!(artifacts.wasm.lock().unwrap().is_empty());
        assert!(bus.events.lock().unwrap().is_empty());
        assert!(lock.held.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bus_error_after_put_releases_lock() {
        let artifacts = Arc::new(MemArtifacts::new());
        let lock = Arc::new(MemLock::new());
        let publish =
            PublishFunction::new(artifacts.clone(), Arc::new(MemBus::failing()), lock.clone());
        let wasm = b"\0asm not empty".to_vec();
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: wasm.clone(),
            })
            .await
            .expect_err("bus");
        assert!(matches!(err, AppError::Storage(_)), "{err}");
        let hash = ContentHash::from_bytes(&wasm);
        assert_eq!(artifacts.get(&hash).await.unwrap(), wasm);
        assert!(lock.held.lock().unwrap().is_empty());
        assert_eq!(*lock.releases.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn put_error_releases_lock() {
        let lock = Arc::new(MemLock::new());
        let publish = PublishFunction::new(
            Arc::new(MemArtifacts::failing()),
            Arc::new(MemBus::new()),
            lock.clone(),
        );
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm not empty".to_vec(),
            })
            .await
            .expect_err("put");
        assert!(matches!(err, AppError::Storage(_)), "{err}");
        assert!(lock.held.lock().unwrap().is_empty());
        assert_eq!(*lock.releases.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn enqueue_records_hash_and_generation() {
        let bus = Arc::new(MemBus::new());
        let lock = Arc::new(MemLock::new());
        let publish =
            PublishFunction::new(Arc::new(MemArtifacts::new()), bus.clone(), lock.clone());
        let wasm = b"\0asm module".to_vec();
        let res = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: wasm.clone(),
            })
            .await
            .expect("publish");
        assert_eq!(res.status, "queued");
        let events = bus.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content_hash, res.content_hash.to_hex());
        assert!(events[0].queued_at_ms > 0);
        assert!(lock.held.lock().unwrap().contains_key("echo"));
    }

    #[tokio::test]
    async fn second_publish_while_locked_conflicts() {
        let bus = Arc::new(MemBus::new());
        let lock = Arc::new(MemLock::new());
        let publish = PublishFunction::new(Arc::new(MemArtifacts::new()), bus.clone(), lock);
        publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm one".to_vec(),
            })
            .await
            .expect("first");
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm two".to_vec(),
            })
            .await
            .expect_err("conflict");
        assert!(matches!(err, AppError::Conflict(_)), "{err}");
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn concurrent_publishes_only_one_wins() {
        let bus = Arc::new(MemBus::new());
        let publish = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            bus.clone(),
            Arc::new(MemLock::new()),
        ));
        let a = publish.execute(PublishRequest {
            function: FunctionId::new("echo").unwrap(),
            wasm: b"\0asm one".to_vec(),
        });
        let b = publish.execute(PublishRequest {
            function: FunctionId::new("echo").unwrap(),
            wasm: b"\0asm two".to_vec(),
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
