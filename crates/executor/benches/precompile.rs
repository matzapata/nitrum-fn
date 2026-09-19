//! Representative benches for nitrum-fn host paths (minus HTTP).
//!
//! | Group | Maps to |
//! |---|---|
//! | `publish` | validate wasm + store `.wasm` + catalog upsert |
//! | `invoke/catalog` | `InvokeFunction` (catalog resolve + re-hash wasm + Cranelift) |
//! | `invoke/cranelift` | `FunctionRunner::run` from raw `.wasm` |
//!
//! ```text
//! cargo bench -p executor --bench precompile
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, PublishLock};
use application::AppError;
use application::{InvokeFunction, PublishFunction};
use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use domain::{
    ContentHash, FunctionId, FunctionVersion, InvokeRequest, PublishRequest, VersionLabel,
};
use executor::WasmtimeRunner;
use runtime::{encode_request, Request as FnRequest};
use tokio::runtime::Runtime;

fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn load_hello_world() -> Vec<u8> {
    let path = fixtures_dir().join("hello_world.wasm");
    std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "missing fixture {}: {err} (see tests/fixtures/README.md)",
            path.display()
        )
    })
}

fn wire_payload() -> Vec<u8> {
    let req = FnRequest::new(
        "POST",
        "/invoke/hello-world",
        vec![("content-type".into(), "application/json".into())],
        b"{}".to_vec(),
    );
    encode_request(&req).expect("encode wire request")
}

struct MemArtifacts {
    wasm: StdMutex<HashMap<String, Vec<u8>>>,
}

impl MemArtifacts {
    fn new() -> Self {
        Self {
            wasm: StdMutex::new(HashMap::new()),
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

struct MemCatalog {
    entries: StdMutex<HashMap<(String, String), ContentHash>>,
}

impl MemCatalog {
    fn new() -> Self {
        Self {
            entries: StdMutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl FunctionCatalog for MemCatalog {
    async fn upsert(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
        hash: ContentHash,
        _queued_at_ms: u64,
        _egress_allow: &[domain::EgressOrigin],
    ) -> Result<bool, AppError> {
        self.entries
            .lock()
            .unwrap()
            .insert((id.as_str().to_string(), label.as_str().to_string()), hash);
        Ok(true)
    }

    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError> {
        let hash = self
            .entries
            .lock()
            .unwrap()
            .get(&(id.as_str().to_string(), label.as_str().to_string()))
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("{id}@{label}")))?;
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

struct NoopLock;

#[async_trait]
impl PublishLock for NoopLock {
    async fn acquire(
        &self,
        _function: &FunctionId,
        _hash: &ContentHash,
        _queued_at_ms: u64,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn release(&self, _function: &FunctionId, _hash: &ContentHash) -> Result<(), AppError> {
        Ok(())
    }
}

struct BenchEnv {
    rt: Runtime,
    runner: Arc<WasmtimeRunner>,
    publish: Arc<PublishFunction>,
    invoke: Arc<InvokeFunction>,
    wasm: Vec<u8>,
    hash: ContentHash,
    payload: Vec<u8>,
    function: FunctionId,
}

impl BenchEnv {
    fn new() -> Self {
        let rt = Runtime::new().expect("tokio runtime");
        let artifacts = Arc::new(MemArtifacts::new());
        let catalog = Arc::new(MemCatalog::new());
        let runner = Arc::new(WasmtimeRunner::new().expect("runner"));
        let publish = Arc::new(PublishFunction::new(
            artifacts.clone() as Arc<dyn ArtifactStore>,
            catalog.clone() as Arc<dyn FunctionCatalog>,
            Arc::new(NoopLock) as Arc<dyn PublishLock>,
            runner.clone() as Arc<dyn FunctionRunner>,
        ));
        let invoke = Arc::new(InvokeFunction::new(
            catalog as Arc<dyn FunctionCatalog>,
            artifacts.clone() as Arc<dyn ArtifactStore>,
            runner.clone() as Arc<dyn FunctionRunner>,
        ));
        let wasm = load_hello_world();
        let function = FunctionId::new("hello-world").expect("id");
        let payload = wire_payload();

        let published = rt
            .block_on(publish.execute(PublishRequest {
                function: function.clone(),
                wasm: wasm.clone(),
                egress_allow: vec![],
            }))
            .expect("seed publish");

        Self {
            rt,
            runner,
            publish,
            invoke,
            wasm,
            hash: published.content_hash.clone(),
            payload,
            function,
        }
    }

    fn invoke_req(&self) -> InvokeRequest {
        InvokeRequest {
            function: self.function.clone(),
            version: VersionLabel::latest(),
            payload: self.payload.clone(),
        }
    }
}

fn host_path_benches(c: &mut Criterion) {
    let env = BenchEnv::new();

    {
        let mut g = c.benchmark_group("publish");
        g.sample_size(20);
        g.bench_function("hello_world_validate_store_upsert", |b| {
            b.iter(|| {
                let res = env.rt.block_on(env.publish.execute(PublishRequest {
                    function: env.function.clone(),
                    wasm: env.wasm.clone(),
                    egress_allow: vec![],
                }));
                black_box(res.expect("publish"));
            });
        });
        g.finish();
    }

    {
        let mut g = c.benchmark_group("invoke");
        g.sample_size(20);

        g.bench_function("catalog_hello_world", |b| {
            b.iter(|| {
                let res = env
                    .rt
                    .block_on(env.invoke.execute(env.invoke_req()))
                    .expect("invoke");
                black_box(res);
            });
        });

        g.bench_function("cranelift_hello_world", |b| {
            b.iter(|| {
                let res = env
                    .rt
                    .block_on(env.runner.run(&env.hash, &env.wasm, &env.payload, &[]))
                    .expect("runner.run");
                black_box(res);
            });
        });
        g.finish();
    }
}

criterion_group!(benches, host_path_benches);
criterion_main!(benches);
