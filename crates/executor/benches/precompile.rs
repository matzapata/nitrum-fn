//! Representative benches for nitrum-fn host paths (minus HTTP).
//!
//! Exercises the same stack as production: `FilesystemArtifactStore`,
//! `InMemoryCatalog`, `WasmtimeRunner`, `PublishFunction` / `InvokeFunction`,
//! and the hello-world guest with host wire encoding (`runtime::encode_request`).
//!
//! | Group | Maps to |
//! |---|---|
//! | `publish` | `PUT /functions/{name}` body of work (compile + store + catalog) |
//! | `cold_invoke/wasm_fallback` | First invoke with `.wasm` only (no `.cwasm`) |
//! | `cold_invoke/precompiled` | First invoke with `.cwasm` on disk, empty Module cache |
//! | `warm_invoke` | Steady-state after publish / preload (cache hit) |
//! | `restart_preload` | Host boot: deserialize `.cwasm` into cache |
//!
//! Not included: axum, TLS, enclave boot, S3. Those dominate later; this isolates
//! the wasm load/invoke delta that compile-on-deploy is meant to buy.
//!
//! ```text
//! cargo bench -p executor --bench precompile
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};
use application::{InvokeFunction, PublishFunction};
use artifacts::FilesystemArtifactStore;
use catalog::InMemoryCatalog;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use domain::{ContentHash, FunctionId, InvokeRequest, PublishRequest, VersionLabel};
use executor::WasmtimeRunner;
use runtime::{encode_request, Request as FnRequest};
use tempfile::TempDir;
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

/// Temporarily rename `{hash}.cwasm` so InvokeFunction takes the wasm fallback path.
struct HiddenCwasm {
    original: PathBuf,
    backup: PathBuf,
}

impl HiddenCwasm {
    fn hide(path: PathBuf) -> Self {
        let backup = path.with_extension("cwasm.bak");
        if path.exists() {
            std::fs::rename(&path, &backup).expect("hide cwasm");
        }
        Self {
            original: path,
            backup,
        }
    }
}

impl Drop for HiddenCwasm {
    fn drop(&mut self) {
        if self.backup.exists() {
            let _ = std::fs::rename(&self.backup, &self.original);
        }
    }
}

struct BenchEnv {
    _dir: TempDir,
    rt: Runtime,
    artifacts: Arc<FilesystemArtifactStore>,
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
        let dir = TempDir::new().expect("tempdir");
        let rt = Runtime::new().expect("tokio runtime");
        let artifacts = Arc::new(FilesystemArtifactStore::new(dir.path().join("artifacts")));
        let catalog = Arc::new(InMemoryCatalog::new());
        let runner = Arc::new(WasmtimeRunner::new().expect("runner"));
        let publish = Arc::new(PublishFunction::new(
            catalog.clone() as Arc<dyn FunctionCatalog>,
            artifacts.clone() as Arc<dyn ArtifactStore>,
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

        // Seed catalog + both artifact forms so cold/warm paths share one setup.
        let published = rt
            .block_on(publish.execute(PublishRequest {
                function: function.clone(),
                wasm: wasm.clone(),
            }))
            .expect("seed publish");

        Self {
            _dir: dir,
            rt,
            artifacts,
            runner,
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

    // --- publish: what deploy waits on ---
    {
        let mut g = c.benchmark_group("publish");
        g.sample_size(20);
        g.bench_function("hello_world_compile_and_store", |b| {
            b.iter(|| {
                env.runner.clear_cache();
                let res = env.rt.block_on(env.publish.execute(PublishRequest {
                    function: env.function.clone(),
                    wasm: env.wasm.clone(),
                }));
                black_box(res.expect("publish"));
            });
        });
        g.finish();
    }

    // --- cold invoke: empty Module cache, read artifacts from disk ---
    {
        let mut g = c.benchmark_group("cold_invoke");
        g.sample_size(20);

        // Before precompile: only .wasm on disk (remove .cwasm for this arm).
        g.bench_function("wasm_fallback_hello_world", |b| {
            b.iter(|| {
                env.runner.clear_cache();
                let _hidden = HiddenCwasm::hide(env.artifacts.compiled_path_for(&env.hash));
                let res = env
                    .rt
                    .block_on(env.invoke.execute(env.invoke_req()))
                    .expect("invoke wasm fallback");
                assert!(!res.warm_module, "expected cold compile");
                black_box(res);
            });
        });

        // After precompile: .cwasm present, cache empty (miss preload).
        g.bench_function("precompiled_hello_world", |b| {
            b.iter(|| {
                env.runner.clear_cache();
                let res = env
                    .rt
                    .block_on(env.invoke.execute(env.invoke_req()))
                    .expect("invoke precompiled");
                assert!(!res.warm_module, "expected cold deserialize");
                black_box(res);
            });
        });
        g.finish();
    }

    // --- warm: cache already filled (post-publish or post-preload) ---
    {
        env.runner.clear_cache();
        env.rt
            .block_on(env.invoke.execute(env.invoke_req()))
            .expect("warm-up invoke");

        let mut g = c.benchmark_group("warm_invoke");
        g.sample_size(50);
        g.bench_function("hello_world", |b| {
            b.iter(|| {
                let res = env
                    .rt
                    .block_on(env.invoke.execute(env.invoke_req()))
                    .expect("warm invoke");
                assert!(res.warm_module, "expected warm cache hit");
                black_box(res);
            });
        });
        g.finish();
    }

    // --- restart: deserialize .cwasm into cache (host preload_compiled) ---
    {
        let mut g = c.benchmark_group("restart_preload");
        g.sample_size(30);
        g.bench_function("load_hello_world_cwasm", |b| {
            b.iter(|| {
                env.runner.clear_cache();
                let compiled = env
                    .rt
                    .block_on(env.artifacts.get_compiled(&env.hash))
                    .expect("read cwasm");
                env.rt
                    .block_on(env.runner.load_precompiled(&env.hash, &compiled))
                    .expect("preload");
                black_box(());
            });
        });
        g.finish();
    }
}

criterion_group!(benches, host_path_benches);
criterion_main!(benches);
