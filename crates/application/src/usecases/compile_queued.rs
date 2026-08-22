use std::sync::Arc;

use domain::{ContentHash, FunctionId, PublishQueuedEvent, VersionLabel};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};

/// Worker-side: AOT-compile a queued `.wasm` and publish it to the catalog.
pub struct CompileQueuedFunction {
    catalog: Arc<dyn FunctionCatalog>,
    artifacts: Arc<dyn ArtifactStore>,
    runner: Arc<dyn FunctionRunner>,
}

impl CompileQueuedFunction {
    pub fn new(
        catalog: Arc<dyn FunctionCatalog>,
        artifacts: Arc<dyn ArtifactStore>,
        runner: Arc<dyn FunctionRunner>,
    ) -> Self {
        Self {
            catalog,
            artifacts,
            runner,
        }
    }

    #[instrument(skip(self, event), fields(function = %event.function, hash = %event.content_hash))]
    pub async fn execute(&self, event: &PublishQueuedEvent) -> Result<(), AppError> {
        let function = FunctionId::new(&event.function).map_err(AppError::from)?;
        let hash = ContentHash::from_hex(&event.content_hash).map_err(AppError::from)?;

        match self.artifacts.get_compiled(&hash).await {
            Ok(_) => {
                tracing::info!(
                    hash = %event.content_hash,
                    "skipping compile; cwasm already present"
                );
            }
            Err(AppError::ArtifactMissing(_)) => {
                let wasm = self.artifacts.get(&hash).await?;
                let actual = ContentHash::from_bytes(&wasm);
                if actual != hash {
                    return Err(AppError::HashMismatch {
                        expected: hash.to_hex(),
                        actual: actual.to_hex(),
                    });
                }

                let compiled = self.runner.compile(&hash, &wasm).await?;
                self.artifacts.put_compiled(&hash, &compiled).await?;
            }
            Err(err) => return Err(err),
        }

        let applied = self
            .catalog
            .upsert(&function, &VersionLabel::latest(), hash, event.queued_at_ms)
            .await?;
        if !applied {
            tracing::info!(
                function = %event.function,
                hash = %event.content_hash,
                queued_at_ms = event.queued_at_ms,
                "skipped stale catalog upsert"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, RunOutcome};
    use async_trait::async_trait;
    use domain::{ContentHash, FunctionId, FunctionVersion, PublishQueuedEvent, VersionLabel};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MemCatalog {
        hash: Mutex<Option<ContentHash>>,
        refuse_stale: bool,
    }

    impl MemCatalog {
        fn new() -> Self {
            Self {
                hash: Mutex::new(None),
                refuse_stale: false,
            }
        }

        fn stale() -> Self {
            Self {
                hash: Mutex::new(None),
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
        ) -> Result<bool, AppError> {
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
            })
        }

        async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
            Ok(vec![])
        }
    }

    struct MemArtifacts {
        wasm: Mutex<HashMap<String, Vec<u8>>>,
        compiled: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MemArtifacts {
        fn with_wasm(hash: &ContentHash, wasm: Vec<u8>) -> Self {
            Self {
                wasm: Mutex::new(HashMap::from([(hash.to_hex(), wasm)])),
                compiled: Mutex::new(HashMap::new()),
            }
        }

        fn with_both(hash: &ContentHash, wasm: Vec<u8>, compiled: Vec<u8>) -> Self {
            let key = hash.to_hex();
            Self {
                wasm: Mutex::new(HashMap::from([(key.clone(), wasm)])),
                compiled: Mutex::new(HashMap::from([(key, compiled)])),
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

        async fn put_compiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError> {
            self.compiled
                .lock()
                .unwrap()
                .insert(hash.to_hex(), compiled.to_vec());
            Ok(())
        }

        async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
            self.compiled
                .lock()
                .unwrap()
                .get(&hash.to_hex())
                .cloned()
                .ok_or_else(|| AppError::ArtifactMissing(hash.to_hex()))
        }
    }

    struct Runner {
        compiles: Mutex<u32>,
    }

    impl Runner {
        fn new() -> Self {
            Self {
                compiles: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl FunctionRunner for Runner {
        async fn compile(&self, _hash: &ContentHash, _wasm: &[u8]) -> Result<Vec<u8>, AppError> {
            *self.compiles.lock().unwrap() += 1;
            Ok(b"cwasm".to_vec())
        }

        async fn run_precompiled(
            &self,
            _hash: &ContentHash,
            _compiled: &[u8],
            _input: &[u8],
        ) -> Result<RunOutcome, AppError> {
            unimplemented!()
        }

        async fn run(
            &self,
            _hash: &ContentHash,
            _wasm: &[u8],
            _input: &[u8],
        ) -> Result<RunOutcome, AppError> {
            unimplemented!()
        }
    }

    fn event(hash: &ContentHash, queued_at_ms: u64) -> PublishQueuedEvent {
        PublishQueuedEvent {
            function: "echo".into(),
            content_hash: hash.to_hex(),
            wasm_bytes: 4,
            queued_at_ms,
        }
    }

    #[tokio::test]
    async fn hash_mismatch_skips_catalog() {
        let wasm = b"wasm".to_vec();
        let claimed = ContentHash::from_bytes(b"other");
        let artifacts = Arc::new(MemArtifacts::with_wasm(&claimed, wasm));
        // store under claimed key but bytes hash to something else
        let catalog = Arc::new(MemCatalog::new());
        let compile =
            CompileQueuedFunction::new(catalog.clone(), artifacts, Arc::new(Runner::new()));
        let err = compile
            .execute(&event(&claimed, 1))
            .await
            .expect_err("mismatch");
        assert!(matches!(err, AppError::HashMismatch { .. }), "{err}");
        assert!(catalog.hash.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn success_compiles_and_upserts() {
        let wasm = b"wasm".to_vec();
        let hash = ContentHash::from_bytes(&wasm);
        let artifacts = Arc::new(MemArtifacts::with_wasm(&hash, wasm));
        let catalog = Arc::new(MemCatalog::new());
        let runner = Arc::new(Runner::new());
        let compile =
            CompileQueuedFunction::new(catalog.clone(), artifacts.clone(), runner.clone());
        compile.execute(&event(&hash, 10)).await.expect("compile");
        assert_eq!(catalog.hash.lock().unwrap().as_ref(), Some(&hash));
        assert_eq!(artifacts.get_compiled(&hash).await.unwrap(), b"cwasm");
        assert_eq!(*runner.compiles.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn existing_cwasm_skips_compile() {
        let wasm = b"wasm".to_vec();
        let hash = ContentHash::from_bytes(&wasm);
        let artifacts = Arc::new(MemArtifacts::with_both(
            &hash,
            wasm,
            b"already-cwasm".to_vec(),
        ));
        let catalog = Arc::new(MemCatalog::new());
        let runner = Arc::new(Runner::new());
        let compile =
            CompileQueuedFunction::new(catalog.clone(), artifacts.clone(), runner.clone());
        compile.execute(&event(&hash, 10)).await.expect("skip");
        assert_eq!(catalog.hash.lock().unwrap().as_ref(), Some(&hash));
        assert_eq!(
            artifacts.get_compiled(&hash).await.unwrap(),
            b"already-cwasm"
        );
        assert_eq!(*runner.compiles.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn stale_generation_still_writes_compiled() {
        let wasm = b"wasm".to_vec();
        let hash = ContentHash::from_bytes(&wasm);
        let artifacts = Arc::new(MemArtifacts::with_wasm(&hash, wasm));
        let catalog = Arc::new(MemCatalog::stale());
        let compile =
            CompileQueuedFunction::new(catalog.clone(), artifacts.clone(), Arc::new(Runner::new()));
        compile.execute(&event(&hash, 1)).await.expect("ok");
        assert!(catalog.hash.lock().unwrap().is_none());
        assert_eq!(artifacts.get_compiled(&hash).await.unwrap(), b"cwasm");
    }
}
