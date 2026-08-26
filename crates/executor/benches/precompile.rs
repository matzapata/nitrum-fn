//! Representative benches for nitrum-fn host paths (minus HTTP).
//!
//! | Group | Maps to |
//! |---|---|
//! | `publish` | store `.wasm` + enqueue (in-memory bus) |
//! | `invoke/precompiled` | Invoke with in-memory `.cwasm` (deserialize each call) |
//! | `invoke/cranelift` | `FunctionRunner::run` from raw `.wasm` |
//!
//! ```text
//! cargo bench -p executor --bench precompile
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, PublishBus, PublishLock};
use application::AppError;
use application::{CompileQueuedFunction, InvokeFunction, PublishFunction};
use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use domain::{
    ContentHash, FunctionId, FunctionVersion, InvokeRequest, PublishQueuedEvent, PublishRequest,
    VersionLabel,
};
use executor::WasmtimeRunner;
use runtime::{encode_request, Request as FnRequest};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

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

/// Records queued events; drain via `take` for the bench compile step.
struct MemBus {
    events: Mutex<Vec<PublishQueuedEvent>>,
}

impl MemBus {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    async fn take(&self) -> Vec<PublishQueuedEvent> {
        std::mem::take(&mut *self.events.lock().await)
    }
}

#[async_trait]
impl PublishBus for MemBus {
    async fn publish_queued(&self, event: &PublishQueuedEvent) -> Result<(), AppError> {
        self.events.lock().await.push(event.clone());
        Ok(())
    }
}

struct MemArtifacts {
    wasm: StdMutex<HashMap<String, Vec<u8>>>,
    compiled: StdMutex<HashMap<String, Vec<u8>>>,
}

impl MemArtifacts {
    fn new() -> Self {
        Self {
            wasm: StdMutex::new(HashMap::new()),
            compiled: StdMutex::new(HashMap::new()),
        }
    }

    fn drop_compiled(&self, hash: &ContentHash) {
        self.compiled.lock().unwrap().remove(&hash.to_hex());
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
    bus: Arc<MemBus>,
    artifacts: Arc<MemArtifacts>,
    compile: Arc<CompileQueuedFunction>,
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
        let bus = Arc::new(MemBus::new());
        let compile = Arc::new(CompileQueuedFunction::new(
            catalog.clone() as Arc<dyn FunctionCatalog>,
            artifacts.clone() as Arc<dyn ArtifactStore>,
            runner.clone() as Arc<dyn FunctionRunner>,
            Arc::new(NoopLock) as Arc<dyn PublishLock>,
        ));
        let publish = Arc::new(PublishFunction::new(
            artifacts.clone() as Arc<dyn ArtifactStore>,
            bus.clone() as Arc<dyn PublishBus>,
            Arc::new(NoopLock) as Arc<dyn PublishLock>,
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
            }))
            .expect("seed publish");
        for event in rt.block_on(bus.take()) {
            rt.block_on(compile.execute(&event)).expect("seed compile");
        }

        Self {
            rt,
            runner,
            bus,
            artifacts,
            compile,
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
        g.bench_function("hello_world_store_and_enqueue", |b| {
            b.iter(|| {
                let res = env.rt.block_on(env.publish.execute(PublishRequest {
                    function: env.function.clone(),
                    wasm: env.wasm.clone(),
                }));
                let _ = env.rt.block_on(env.bus.take());
                black_box(res.expect("publish"));
            });
        });
        g.finish();
    }

    {
        let mut g = c.benchmark_group("compile");
        g.sample_size(20);
        g.bench_function("hello_world_aot", |b| {
            b.iter(|| {
                let event = PublishQueuedEvent::new(
                    env.function.to_string(),
                    env.hash.to_hex(),
                    env.wasm.len(),
                );
                env.artifacts.drop_compiled(&env.hash);
                let res = env.rt.block_on(env.compile.execute(&event));
                res.expect("compile");
                black_box(());
            });
        });
        g.finish();
    }

    {
        let mut g = c.benchmark_group("invoke");
        g.sample_size(20);

        g.bench_function("precompiled_hello_world", |b| {
            b.iter(|| {
                let res = env
                    .rt
                    .block_on(env.invoke.execute(env.invoke_req()))
                    .expect("invoke precompiled");
                black_box(res);
            });
        });

        g.bench_function("cranelift_hello_world", |b| {
            b.iter(|| {
                let res = env
                    .rt
                    .block_on(env.runner.run(&env.hash, &env.wasm, &env.payload))
                    .expect("runner.run");
                black_box(res);
            });
        });
        g.finish();
    }
}

criterion_group!(benches, host_path_benches);
criterion_main!(benches);
