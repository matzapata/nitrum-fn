use crate::error::AppError;
use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, PublishLock};
use domain::{
    ContentHash, PublishRequest, PublishResponse, PublishStatus, VersionLabel, MAX_WASM_BYTES,
};
use std::sync::Arc;
use tracing::instrument;

pub struct PublishFunction {
    artifacts: Arc<dyn ArtifactStore>,
    catalog: Arc<dyn FunctionCatalog>,
    lock: Arc<dyn PublishLock>,
    runner: Arc<dyn FunctionRunner>,
}

impl PublishFunction {
    pub fn new(
        artifacts: Arc<dyn ArtifactStore>,
        catalog: Arc<dyn FunctionCatalog>,
        lock: Arc<dyn PublishLock>,
        runner: Arc<dyn FunctionRunner>,
    ) -> Self {
        Self {
            artifacts,
            catalog,
            lock,
            runner,
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

        self.runner.validate(&req.wasm).await?;

        let hash = ContentHash::from_bytes(&req.wasm);
        let queued_at_ms = unix_now_ms();
        self.lock
            .acquire(&req.function, &hash, queued_at_ms)
            .await?;

        let stored = match self.artifacts.put(&req.wasm).await {
            Ok(stored) => stored,
            Err(err) => {
                let _ = self.lock.release(&req.function, &hash).await;
                return Err(err);
            }
        };

        let applied = match self
            .catalog
            .upsert(
                &req.function,
                &VersionLabel::latest(),
                hash.clone(),
                queued_at_ms,
                &req.egress_allow,
            )
            .await
        {
            Ok(applied) => applied,
            Err(err) => {
                let _ = self.lock.release(&req.function, &hash).await;
                return Err(err);
            }
        };

        self.lock.release(&req.function, &hash).await?;

        if !applied {
            tracing::info!(
                function = %req.function,
                hash = %hash,
                queued_at_ms,
                "skipped stale catalog upsert"
            );
        }

        Ok(PublishResponse {
            function: req.function,
            version: VersionLabel::latest(),
            content_hash: stored,
            wasm_bytes: req.wasm.len(),
            egress_allow: req.egress_allow,
            status: if applied {
                PublishStatus::Ready
            } else {
                PublishStatus::Superseded
            },
        })
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, PublishLock, RunOutcome};
    use async_trait::async_trait;
    use domain::{FunctionId, FunctionVersion};
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
    }

    struct MemCatalog {
        hash: Mutex<Option<ContentHash>>,
        fail: bool,
        refuse_stale: bool,
    }

    impl MemCatalog {
        fn new() -> Self {
            Self {
                hash: Mutex::new(None),
                fail: false,
                refuse_stale: false,
            }
        }

        fn failing() -> Self {
            Self {
                hash: Mutex::new(None),
                fail: true,
                refuse_stale: false,
            }
        }

        fn stale() -> Self {
            Self {
                hash: Mutex::new(None),
                fail: false,
                refuse_stale: true,
            }
        }
    }

    #[async_trait]
    impl FunctionCatalog for MemCatalog {
        async fn upsert(
            &self,
            _id: &FunctionId,
            _label: &VersionLabel,
            hash: ContentHash,
            _queued_at_ms: u64,
            _egress_allow: &[domain::EgressOrigin],
        ) -> Result<bool, AppError> {
            if self.fail {
                return Err(AppError::Storage("ddb down".into()));
            }
            if self.refuse_stale {
                return Ok(false);
            }
            *self.hash.lock().unwrap() = Some(hash);
            Ok(true)
        }

        async fn resolve(
            &self,
            id: &FunctionId,
            label: &VersionLabel,
        ) -> Result<FunctionVersion, AppError> {
            let hash = self
                .hash
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| AppError::NotFound(id.to_string()))?;
            Ok(FunctionVersion {
                id: id.clone(),
                label: label.clone(),
                content_hash: hash,
                egress_allow: vec![],
            })
        }

        async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
            Ok(vec![])
        }
    }

    struct AcceptingRunner;

    #[async_trait]
    impl FunctionRunner for AcceptingRunner {
        async fn validate(&self, _wasm: &[u8]) -> Result<(), AppError> {
            Ok(())
        }

        async fn run(
            &self,
            _hash: &ContentHash,
            _wasm: &[u8],
            _input: &[u8],
            _egress_allow: &[domain::EgressOrigin],
        ) -> Result<RunOutcome, AppError> {
            unimplemented!()
        }
    }

    struct RejectingRunner;

    #[async_trait]
    impl FunctionRunner for RejectingRunner {
        async fn validate(&self, _wasm: &[u8]) -> Result<(), AppError> {
            Err(AppError::Compile("bad abi".into()))
        }

        async fn run(
            &self,
            _hash: &ContentHash,
            _wasm: &[u8],
            _input: &[u8],
            _egress_allow: &[domain::EgressOrigin],
        ) -> Result<RunOutcome, AppError> {
            unimplemented!()
        }
    }

    fn publish(
        artifacts: Arc<MemArtifacts>,
        catalog: Arc<MemCatalog>,
        lock: Arc<MemLock>,
    ) -> PublishFunction {
        PublishFunction::new(artifacts, catalog, lock, Arc::new(AcceptingRunner))
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
        let publish = publish(
            Arc::new(MemArtifacts::new()),
            Arc::new(MemCatalog::new()),
            Arc::new(MemLock::new()),
        );
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: vec![],
                egress_allow: vec![],
            })
            .await
            .expect_err("empty");
        assert!(matches!(err, AppError::Compile(_)), "{err}");
    }

    #[tokio::test]
    async fn rejects_invalid_wasm_without_put() {
        let artifacts = Arc::new(MemArtifacts::new());
        let catalog = Arc::new(MemCatalog::new());
        let lock = Arc::new(MemLock::new());
        let publish = PublishFunction::new(
            artifacts.clone(),
            catalog.clone(),
            lock.clone(),
            Arc::new(RejectingRunner),
        );
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm not a real module".to_vec(),
                egress_allow: vec![],
            })
            .await
            .expect_err("invalid");
        assert!(matches!(err, AppError::Compile(_)), "{err}");
        assert!(artifacts.wasm.lock().unwrap().is_empty());
        assert!(catalog.hash.lock().unwrap().is_none());
        assert!(lock.held.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rejects_oversize_wasm_without_put() {
        let artifacts = Arc::new(MemArtifacts::new());
        let catalog = Arc::new(MemCatalog::new());
        let lock = Arc::new(MemLock::new());
        let publish = publish(artifacts.clone(), catalog.clone(), lock.clone());
        let wasm = vec![0u8; domain::MAX_WASM_BYTES + 1];
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm,
                egress_allow: vec![],
            })
            .await
            .expect_err("too large");
        assert!(matches!(err, AppError::PayloadTooLarge(_)), "{err}");
        assert!(artifacts.wasm.lock().unwrap().is_empty());
        assert!(catalog.hash.lock().unwrap().is_none());
        assert!(lock.held.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn catalog_error_after_put_releases_lock() {
        let artifacts = Arc::new(MemArtifacts::new());
        let lock = Arc::new(MemLock::new());
        let publish = publish(
            artifacts.clone(),
            Arc::new(MemCatalog::failing()),
            lock.clone(),
        );
        let wasm = b"\0asm not empty".to_vec();
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: wasm.clone(),
                egress_allow: vec![],
            })
            .await
            .expect_err("catalog");
        assert!(matches!(err, AppError::Storage(_)), "{err}");
        let hash = ContentHash::from_bytes(&wasm);
        assert_eq!(artifacts.get(&hash).await.unwrap(), wasm);
        assert!(lock.held.lock().unwrap().is_empty());
        assert_eq!(*lock.releases.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn put_error_releases_lock() {
        let lock = Arc::new(MemLock::new());
        let publish = publish(
            Arc::new(MemArtifacts::failing()),
            Arc::new(MemCatalog::new()),
            lock.clone(),
        );
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm not empty".to_vec(),
                egress_allow: vec![],
            })
            .await
            .expect_err("put");
        assert!(matches!(err, AppError::Storage(_)), "{err}");
        assert!(lock.held.lock().unwrap().is_empty());
        assert_eq!(*lock.releases.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn success_upserts_catalog_and_returns_ready() {
        let catalog = Arc::new(MemCatalog::new());
        let lock = Arc::new(MemLock::new());
        let publish = publish(Arc::new(MemArtifacts::new()), catalog.clone(), lock.clone());
        let wasm = b"\0asm module".to_vec();
        let res = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: wasm.clone(),
                egress_allow: vec![],
            })
            .await
            .expect("publish");
        assert_eq!(res.status, PublishStatus::Ready);
        assert_eq!(
            catalog.hash.lock().unwrap().as_ref(),
            Some(&ContentHash::from_bytes(&wasm))
        );
        assert!(lock.held.lock().unwrap().is_empty());
        assert_eq!(*lock.releases.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn stale_upsert_returns_superseded_and_releases() {
        let catalog = Arc::new(MemCatalog::stale());
        let lock = Arc::new(MemLock::new());
        let publish = publish(Arc::new(MemArtifacts::new()), catalog.clone(), lock.clone());
        let res = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm module".to_vec(),
                egress_allow: vec![],
            })
            .await
            .expect("publish");
        assert_eq!(res.status, PublishStatus::Superseded);
        assert!(catalog.hash.lock().unwrap().is_none());
        assert_eq!(*lock.releases.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn second_publish_while_locked_conflicts() {
        let catalog = Arc::new(MemCatalog::new());
        let lock = Arc::new(MemLock::new());
        // Hold the lock as if another publish is in flight.
        lock.held
            .lock()
            .unwrap()
            .insert("echo".into(), "other".into());
        let publish = publish(Arc::new(MemArtifacts::new()), catalog, lock);
        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm two".to_vec(),
                egress_allow: vec![],
            })
            .await
            .expect_err("conflict");
        assert!(matches!(err, AppError::Conflict(_)), "{err}");
    }

    #[tokio::test]
    async fn overlapping_publishes_conflict_while_lock_held() {
        use tokio::sync::Notify;

        struct HoldLock {
            held: Mutex<HashMap<String, String>>,
            first_acquired: Notify,
            allow_release: Notify,
        }

        #[async_trait]
        impl PublishLock for HoldLock {
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
                drop(held);
                self.first_acquired.notify_one();
                Ok(())
            }

            async fn release(
                &self,
                function: &FunctionId,
                hash: &ContentHash,
            ) -> Result<(), AppError> {
                self.allow_release.notified().await;
                let mut held = self.held.lock().unwrap();
                if held
                    .get(function.as_str())
                    .is_some_and(|h| h == &hash.to_hex())
                {
                    held.remove(function.as_str());
                }
                Ok(())
            }
        }

        let lock = Arc::new(HoldLock {
            held: Mutex::new(HashMap::new()),
            first_acquired: Notify::new(),
            allow_release: Notify::new(),
        });
        let publish = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts::new()),
            Arc::new(MemCatalog::new()),
            lock.clone(),
            Arc::new(AcceptingRunner),
        ));

        let a = tokio::spawn({
            let publish = publish.clone();
            async move {
                publish
                    .execute(PublishRequest {
                        function: FunctionId::new("echo").unwrap(),
                        wasm: b"\0asm one".to_vec(),
                        egress_allow: vec![],
                    })
                    .await
            }
        });
        lock.first_acquired.notified().await;

        let err = publish
            .execute(PublishRequest {
                function: FunctionId::new("echo").unwrap(),
                wasm: b"\0asm two".to_vec(),
                egress_allow: vec![],
            })
            .await
            .expect_err("conflict while first still holds lock");
        assert!(matches!(err, AppError::Conflict(_)), "{err}");

        lock.allow_release.notify_one();
        let ra = a.await.expect("join").expect("first publish");
        assert_eq!(ra.status, PublishStatus::Ready);
    }
}
