use std::sync::Arc;

use domain::{InvokeRequest, InvokeResponse};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};

pub struct InvokeFunction {
    catalog: Arc<dyn FunctionCatalog>,
    artifacts: Arc<dyn ArtifactStore>,
    runner: Arc<dyn FunctionRunner>,
}

impl InvokeFunction {
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

    #[instrument(skip(self, req), fields(function = %req.function, version = %req.version.as_str()))]
    pub async fn execute(&self, req: InvokeRequest) -> Result<InvokeResponse, AppError> {
        let version = self.catalog.resolve(&req.function, &req.version).await?;
        let hash = version.content_hash.clone();

        // Trusted path: load `.wasm`, re-hash in the store, Cranelift in the host.
        let wasm = self.artifacts.get(&hash).await?;
        let outcome = self
            .runner
            .run(&hash, &wasm, &req.payload, &version.egress_allow)
            .await?;

        Ok(InvokeResponse {
            output: outcome.output,
            content_hash: hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, RunOutcome};
    use async_trait::async_trait;
    use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FixedCatalog {
        version: FunctionVersion,
    }

    #[async_trait]
    impl FunctionCatalog for FixedCatalog {
        async fn upsert(
            &self,
            _id: &FunctionId,
            _label: &VersionLabel,
            _hash: ContentHash,
            _queued_at_ms: u64,
            _egress_allow: &[domain::EgressOrigin],
        ) -> Result<bool, AppError> {
            Ok(true)
        }

        async fn resolve(
            &self,
            id: &FunctionId,
            label: &VersionLabel,
        ) -> Result<FunctionVersion, AppError> {
            if id != &self.version.id || label != &self.version.label {
                return Err(AppError::NotFound(id.to_string()));
            }
            Ok(self.version.clone())
        }

        async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
            Ok(vec![self.version.clone()])
        }
    }

    struct MemArtifacts {
        /// Catalog hash key → bytes (may intentionally mismatch for tests).
        wasm: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MemArtifacts {
        fn wasm_only(hash: &ContentHash, wasm: Vec<u8>) -> Self {
            let key = hash.to_hex();
            Self {
                wasm: Mutex::new(HashMap::from([(key, wasm)])),
            }
        }

        fn wrong_bytes(catalog_hash: &ContentHash, wrong: Vec<u8>) -> Self {
            Self {
                wasm: Mutex::new(HashMap::from([(catalog_hash.to_hex(), wrong)])),
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
            let bytes = self
                .wasm
                .lock()
                .unwrap()
                .get(&hash.to_hex())
                .cloned()
                .ok_or_else(|| AppError::ArtifactMissing(hash.to_hex()))?;
            let actual = ContentHash::from_bytes(&bytes);
            if actual != *hash {
                return Err(AppError::HashMismatch {
                    expected: hash.to_hex(),
                    actual: actual.to_hex(),
                });
            }
            Ok(bytes)
        }
    }

    struct Runner {
        from_wasm: Mutex<u32>,
        trap: bool,
    }

    impl Runner {
        fn ok() -> Self {
            Self {
                from_wasm: Mutex::new(0),
                trap: false,
            }
        }

        fn trapping() -> Self {
            Self {
                from_wasm: Mutex::new(0),
                trap: true,
            }
        }
    }

    #[async_trait]
    impl FunctionRunner for Runner {
        async fn validate(&self, _wasm: &[u8]) -> Result<(), AppError> {
            Ok(())
        }

        async fn run(
            &self,
            _hash: &ContentHash,
            _wasm: &[u8],
            input: &[u8],
            _egress_allow: &[domain::EgressOrigin],
        ) -> Result<RunOutcome, AppError> {
            *self.from_wasm.lock().unwrap() += 1;
            if self.trap {
                return Err(AppError::Trap("unreachable".into()));
            }
            let mut out = b"wasm:".to_vec();
            out.extend_from_slice(input);
            Ok(RunOutcome { output: out })
        }
    }

    fn harness(runner: Arc<Runner>) -> (InvokeFunction, Arc<Runner>, ContentHash) {
        let wasm = b"\0asm fake";
        let hash = ContentHash::from_bytes(wasm);
        let id = FunctionId::new("echo").unwrap();
        let catalog = Arc::new(FixedCatalog {
            version: FunctionVersion {
                id: id.clone(),
                label: VersionLabel::latest(),
                content_hash: hash.clone(),
                egress_allow: vec![],
            },
        });
        let artifacts = Arc::new(MemArtifacts::wasm_only(&hash, wasm.to_vec()));
        (
            InvokeFunction::new(catalog, artifacts, runner.clone()),
            runner,
            hash,
        )
    }

    #[tokio::test]
    async fn runs_from_verified_wasm() {
        let (invoke, runner, hash) = harness(Arc::new(Runner::ok()));
        let out = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect("invoke");
        assert_eq!(out.output, b"wasm:hi");
        assert_eq!(out.content_hash, hash);
        assert_eq!(*runner.from_wasm.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn missing_wasm_is_artifact_missing() {
        let wasm = b"\0asm fake";
        let hash = ContentHash::from_bytes(wasm);
        let id = FunctionId::new("echo").unwrap();
        let catalog = Arc::new(FixedCatalog {
            version: FunctionVersion {
                id: id.clone(),
                label: VersionLabel::latest(),
                content_hash: hash.clone(),
                egress_allow: vec![],
            },
        });
        let artifacts = Arc::new(MemArtifacts {
            wasm: Mutex::new(HashMap::new()),
        });
        let runner = Arc::new(Runner::ok());
        let invoke = InvokeFunction::new(catalog, artifacts, runner.clone());
        let err = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect_err("missing wasm");
        assert!(matches!(err, AppError::ArtifactMissing(_)), "{err}");
        assert_eq!(*runner.from_wasm.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn wrong_bytes_is_hash_mismatch() {
        let catalog_hash = ContentHash::from_bytes(b"\0asm expected");
        let id = FunctionId::new("echo").unwrap();
        let catalog = Arc::new(FixedCatalog {
            version: FunctionVersion {
                id: id.clone(),
                label: VersionLabel::latest(),
                content_hash: catalog_hash.clone(),
                egress_allow: vec![],
            },
        });
        let artifacts = Arc::new(MemArtifacts::wrong_bytes(
            &catalog_hash,
            b"\0asm wrong-bytes".to_vec(),
        ));
        let runner = Arc::new(Runner::ok());
        let invoke = InvokeFunction::new(catalog, artifacts, runner.clone());
        let err = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect_err("hash mismatch");
        assert!(matches!(err, AppError::HashMismatch { .. }), "{err}");
        assert_eq!(*runner.from_wasm.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn guest_trap_propagates() {
        let (invoke, runner, _) = harness(Arc::new(Runner::trapping()));
        let err = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect_err("trap");
        assert!(matches!(err, AppError::Trap(_)), "{err}");
        assert_eq!(*runner.from_wasm.lock().unwrap(), 1);
    }
}
