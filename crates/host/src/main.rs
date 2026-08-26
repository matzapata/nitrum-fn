mod config;
mod error;
mod http;
mod state;
mod telemetry;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};
use application::InvokeFunction;
use artifacts::S3ArtifactStore;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use catalog::DynamoDbCatalog;
use executor::WasmtimeRunner;
use tracing::info;

use crate::config::HostConfig;
use crate::state::AppState;
use crate::telemetry::Telemetry;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let telemetry = Telemetry::init();
    let config = HostConfig::load().context("load host config")?;

    let s3 = build_s3_client(config.artifacts.endpoint.as_deref()).await?;
    let ddb = build_ddb_client(config.catalog.endpoint.as_deref()).await?;
    let catalog: Arc<dyn FunctionCatalog> =
        Arc::new(DynamoDbCatalog::new(ddb, config.catalog.table.clone()));
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(S3ArtifactStore::new(
        s3,
        config.artifacts.bucket.clone(),
        config.artifacts.prefix.clone(),
    ));
    info!(
        bucket = %config.artifacts.bucket,
        table = %config.catalog.table,
        "using S3 artifacts and DynamoDB catalog"
    );

    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);

    let invoke = Arc::new(InvokeFunction::new(catalog, artifacts, runner));
    let app = http::router(AppState { invoke });

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!(
        %addr,
        bucket = %config.artifacts.bucket,
        artifacts_endpoint = ?config.artifacts.endpoint,
        table = %config.catalog.table,
        catalog_endpoint = ?config.catalog.endpoint,
        "nitrum-fn host listening"
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;

    telemetry.shutdown();
    Ok(())
}

async fn load_aws_config() -> aws_config::SdkConfig {
    let http_client = aws_smithy_http_client::Builder::new()
        .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
            aws_smithy_http_client::tls::rustls_provider::CryptoMode::Ring,
        ))
        .build_https();
    aws_config::defaults(BehaviorVersion::latest())
        .http_client(http_client)
        .load()
        .await
}

async fn build_s3_client(endpoint: Option<&str>) -> Result<S3Client> {
    let sdk = load_aws_config().await;
    let mut builder = S3ConfigBuilder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url).force_path_style(true);
    }
    Ok(S3Client::from_conf(builder.build()))
}

async fn build_ddb_client(endpoint: Option<&str>) -> Result<DdbClient> {
    let sdk = load_aws_config().await;
    let mut builder = aws_sdk_dynamodb::config::Builder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(DdbClient::from_conf(builder.build()))
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = term.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
    info!("shutdown signal received");
}
