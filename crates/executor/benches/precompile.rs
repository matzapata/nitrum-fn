//! Representative benches for nitrum-fn host paths (minus HTTP).
//!
//! | Group | Maps to |
//! |---|---|
//! | `publish` | store `.wasm` + enqueue (in-memory bus) |
//! | `invoke/precompiled` | Invoke with `.cwasm` on disk (deserialize each call) |
//! | `invoke/cranelift` | `FunctionRunner::run` from raw `.wasm` |
//!
//! ```text
//! cargo bench -p executor --bench precompile
//! ```

use std::sync::Arc;

use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, PublishBus};
use application::AppError;
use application::{CompileQueuedFunction, InvokeFunction, PublishFunction};
use artifacts::FilesystemArtifactStore;
use async_trait::async_trait;
use catalog::InMemoryCatalog;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use domain::{
    ContentHash, FunctionId, InvokeRequest, PublishQueuedEvent, PublishRequest, VersionLabel,
};
use executor::WasmtimeRunner;
use runtime::{encode_request, Request as FnRequest};
use tempfile::TempDir;
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

struct BenchEnv {
    _dir: TempDir,
    rt: Runtime,
    runner: Arc<WasmtimeRunner>,
    bus: Arc<MemBus>,
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
        let dir = TempDir::new().expect("tempdir");
        let rt = Runtime::new().expect("tokio runtime");
        let artifacts = Arc::new(FilesystemArtifactStore::new(dir.path().join("artifacts")));
        let catalog = Arc::new(InMemoryCatalog::new());
        let runner = Arc::new(WasmtimeRunner::new().expect("runner"));
        let bus = Arc::new(MemBus::new());
        let compile = Arc::new(CompileQueuedFunction::new(
            catalog.clone() as Arc<dyn FunctionCatalog>,
            artifacts.clone() as Arc<dyn ArtifactStore>,
            runner.clone() as Arc<dyn FunctionRunner>,
        ));
        let publish = Arc::new(PublishFunction::new(
            artifacts.clone() as Arc<dyn ArtifactStore>,
            bus.clone() as Arc<dyn PublishBus>,
        ));
        let invoke = Arc::new(InvokeFunction::new(
            catalog as Arc<dyn FunctionCatalog>,
            artifacts as Arc<dyn ArtifactStore>,
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
            _dir: dir,
            rt,
            runner,
            bus,
            compile,
            publish,
            invoke,
            wasm,
            hash: published.content_hash,
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
