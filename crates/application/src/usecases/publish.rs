use std::sync::Arc;

use domain::{PublishQueuedEvent, PublishRequest, PublishResponse, VersionLabel};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{ArtifactStore, PublishBus};

pub struct PublishFunction {
    artifacts: Arc<dyn ArtifactStore>,
    bus: Arc<dyn PublishBus>,
}

impl PublishFunction {
    pub fn new(artifacts: Arc<dyn ArtifactStore>, bus: Arc<dyn PublishBus>) -> Self {
        Self { artifacts, bus }
    }

    #[instrument(skip(self, req), fields(function = %req.function, wasm_len = req.wasm.len()))]
    pub async fn execute(&self, req: PublishRequest) -> Result<PublishResponse, AppError> {
        if req.wasm.is_empty() {
            return Err(AppError::Compile("empty wasm".into()));
        }

        let hash = self.artifacts.put(&req.wasm).await?;
        let version = VersionLabel::latest();
        let event =
            PublishQueuedEvent::new(req.function.to_string(), hash.to_hex(), req.wasm.len());
        self.bus.publish_queued(&event).await?;

        Ok(PublishResponse {
            function: req.function,
            version,
            content_hash: hash,
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
    use domain::{ContentHash, FunctionId};
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

    #[tokio::test]
    async fn rejects_empty_wasm() {
        let publish = PublishFunction::new(Arc::new(MemArtifacts::new()), Arc::new(MemBus::new()));
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
    async fn bus_error_after_put_still_stores_wasm() {
        let artifacts = Arc::new(MemArtifacts::new());
        let publish = PublishFunction::new(artifacts.clone(), Arc::new(MemBus::failing()));
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
    }

    #[tokio::test]
    async fn enqueue_records_hash_and_generation() {
        let bus = Arc::new(MemBus::new());
        let publish = PublishFunction::new(Arc::new(MemArtifacts::new()), bus.clone());
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
    }
}
