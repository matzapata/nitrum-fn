mod config;
mod error;
mod http;
mod state;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use api::ApiState;
use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};
use application::{InvokeFunction, PublishFunction};
use artifacts::FilesystemArtifactStore;
use catalog::FilesystemCatalog;
use domain::{FunctionId, PublishRequest};
use executor::WasmtimeRunner;
use tracing::{info, warn};

use crate::config::HostConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = HostConfig::from_env();
    tokio::fs::create_dir_all(&config.artifact_dir)
        .await
        .with_context(|| format!("create artifact dir {}", config.artifact_dir.display()))?;

    let catalog: Arc<dyn FunctionCatalog> = Arc::new(
        FilesystemCatalog::open(&config.catalog_path)
            .await
            .with_context(|| format!("open catalog {}", config.catalog_path.display()))?,
    );
    let artifacts: Arc<dyn ArtifactStore> =
        Arc::new(FilesystemArtifactStore::new(config.artifact_dir.clone()));
    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);

    let publish = Arc::new(PublishFunction::new(
        catalog.clone(),
        artifacts.clone(),
        runner.clone(),
    ));
    seed_dir(&publish, &config.seed_dir).await?;

    let invoke = Arc::new(InvokeFunction::new(
        catalog.clone(),
        artifacts.clone(),
        runner,
    ));

    let app = http::router(AppState { invoke }).merge(api::router(ApiState { publish, catalog }));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!(
        %addr,
        artifact_dir = %config.artifact_dir.display(),
        catalog_path = %config.catalog_path.display(),
        seed_dir = %config.seed_dir.display(),
        "nitrum-fn host listening"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;

    Ok(())
}

async fn seed_dir(publish: &PublishFunction, seed_dir: &Path) -> Result<()> {
    if !seed_dir.exists() {
        info!(
            seed_dir = %seed_dir.display(),
            "no seed dir — publish with `nitrum-fn publish`"
        );
        return Ok(());
    }

    let mut entries = tokio::fs::read_dir(seed_dir)
        .await
        .with_context(|| format!("read seed dir {}", seed_dir.display()))?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid seed filename {}", path.display()))?;

        let wasm = tokio::fs::read(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;

        let function = match FunctionId::new(name) {
            Ok(id) => id,
            Err(err) => {
                warn!(%name, error = %err, "skipping seed wasm");
                continue;
            }
        };

        match publish.execute(PublishRequest { function, wasm }).await {
            Ok(res) => info!(
                name = %res.function,
                hash = %res.content_hash,
                wasm_bytes = res.wasm_bytes,
                compiled_bytes = res.compiled_bytes,
                "seeded function @latest"
            ),
            Err(err) => warn!(%name, error = %err, "skipping seed wasm"),
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown signal received");
}
