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

        // Load-only: deserialize the publish-time .cwasm. No Cranelift fallback —
        // the API must emit AOT for the same target as the enclave (musl).
        let compiled = self.artifacts.get_compiled(&version.content_hash).await?;
        let outcome = self
            .runner
            .run_precompiled(&version.content_hash, &compiled, &req.payload)
            .await?;

        Ok(InvokeResponse {
            output: outcome.output,
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
        ) -> Result<(), AppError> {
            Ok(())
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
        wasm: Mutex<HashMap<String, Vec<u8>>>,
        compiled: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MemArtifacts {
        fn with_both(hash: &ContentHash, wasm: Vec<u8>, compiled: Vec<u8>) -> Self {
            let key = hash.to_hex();
            Self {
                wasm: Mutex::new(HashMap::from([(key.clone(), wasm)])),
                compiled: Mutex::new(HashMap::from([(key, compiled)])),
            }
        }

        fn wasm_only(hash: &ContentHash, wasm: Vec<u8>) -> Self {
            let key = hash.to_hex();
            Self {
                wasm: Mutex::new(HashMap::from([(key, wasm)])),
                compiled: Mutex::new(HashMap::new()),
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
        precompiled: Mutex<u32>,
        from_wasm: Mutex<u32>,
        fail_precompiled: bool,
        trap_precompiled: bool,
    }

    impl Runner {
        fn new(fail_precompiled: bool) -> Self {
            Self {
                precompiled: Mutex::new(0),
                from_wasm: Mutex::new(0),
                fail_precompiled,
                trap_precompiled: false,
            }
        }

        fn trapping() -> Self {
            Self {
                precompiled: Mutex::new(0),
                from_wasm: Mutex::new(0),
                fail_precompiled: false,
                trap_precompiled: true,
            }
        }
    }

    #[async_trait]
    impl FunctionRunner for Runner {
        async fn compile(&self, _hash: &ContentHash, _wasm: &[u8]) -> Result<Vec<u8>, AppError> {
            Ok(b"compiled".to_vec())
        }

        async fn run_precompiled(
            &self,
            _hash: &ContentHash,
            _compiled: &[u8],
            input: &[u8],
        ) -> Result<RunOutcome, AppError> {
            *self.precompiled.lock().unwrap() += 1;
            if self.trap_precompiled {
                return Err(AppError::Trap("unreachable".into()));
            }
            if self.fail_precompiled {
                return Err(AppError::Invoke("deserialize compiled module: abi".into()));
            }
            Ok(RunOutcome {
                output: input.to_vec(),
            })
        }

        async fn run(
            &self,
            _hash: &ContentHash,
            _wasm: &[u8],
            input: &[u8],
        ) -> Result<RunOutcome, AppError> {
            *self.from_wasm.lock().unwrap() += 1;
            let mut out = b"wasm:".to_vec();
            out.extend_from_slice(input);
            Ok(RunOutcome { output: out })
        }
    }

    fn harness(runner: Arc<Runner>) -> (InvokeFunction, Arc<Runner>) {
        let wasm = b"\0asm fake";
        let hash = ContentHash::from_bytes(wasm);
        let id = FunctionId::new("echo").unwrap();
        let catalog = Arc::new(FixedCatalog {
            version: FunctionVersion {
                id: id.clone(),
                label: VersionLabel::latest(),
                content_hash: hash.clone(),
            },
        });
        let artifacts = Arc::new(MemArtifacts::with_both(
            &hash,
            wasm.to_vec(),
            b"cwasm".to_vec(),
        ));
        (
            InvokeFunction::new(catalog, artifacts, runner.clone()),
            runner,
        )
    }

    fn harness_missing_cwasm(runner: Arc<Runner>) -> (InvokeFunction, Arc<Runner>) {
        let wasm = b"\0asm fake";
        let hash = ContentHash::from_bytes(wasm);
        let id = FunctionId::new("echo").unwrap();
        let catalog = Arc::new(FixedCatalog {
            version: FunctionVersion {
                id: id.clone(),
                label: VersionLabel::latest(),
                content_hash: hash.clone(),
            },
        });
        let artifacts = Arc::new(MemArtifacts::wasm_only(&hash, wasm.to_vec()));
        (
            InvokeFunction::new(catalog, artifacts, runner.clone()),
            runner,
        )
    }

    #[tokio::test]
    async fn uses_precompiled_when_it_runs() {
        let (invoke, runner) = harness(Arc::new(Runner::new(false)));
        let out = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect("invoke");
        assert_eq!(out.output, b"hi");
        assert_eq!(*runner.precompiled.lock().unwrap(), 1);
        assert_eq!(*runner.from_wasm.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn deserialize_failure_does_not_compile_from_wasm() {
        let (invoke, runner) = harness(Arc::new(Runner::new(true)));
        let err = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect_err("deserialize");
        assert!(matches!(err, AppError::Invoke(_)), "{err}");
        assert_eq!(*runner.precompiled.lock().unwrap(), 1);
        assert_eq!(*runner.from_wasm.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn missing_cwasm_is_artifact_missing() {
        let (invoke, runner) = harness_missing_cwasm(Arc::new(Runner::new(false)));
        let err = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect_err("missing cwasm");
        assert!(matches!(err, AppError::ArtifactMissing(_)), "{err}");
        assert_eq!(*runner.precompiled.lock().unwrap(), 0);
        assert_eq!(*runner.from_wasm.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn guest_trap_on_precompiled_does_not_fall_back() {
        let (invoke, runner) = harness(Arc::new(Runner::trapping()));
        let err = invoke
            .execute(InvokeRequest {
                function: FunctionId::new("echo").unwrap(),
                version: VersionLabel::latest(),
                payload: b"hi".to_vec(),
            })
            .await
            .expect_err("trap");
        assert!(matches!(err, AppError::Trap(_)), "{err}");
        assert_eq!(*runner.from_wasm.lock().unwrap(), 0);
    }
}
