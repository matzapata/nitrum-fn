mod config;

use anyhow::{Context, Result};
use api::ApiState;
use application::ports::{ArtifactStore, FunctionCatalog, PublishBus};
use application::PublishFunction;
use artifacts::S3ArtifactStore;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sns::config::Builder as SnsConfigBuilder;
use aws_sdk_sns::Client as SnsClient;
use catalog::{DynamoDbFunctionCatalog, DynamoDbPublishLock};
use messaging::SnsPublishBus;
use std::net::SocketAddr;
use std::sync::Arc;
use telemetry::{env, TelemetryConfig};
use tracing::info;

use crate::config::ApiConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let telemetry = telemetry::init(
        TelemetryConfig::new("nitrum-fn-api")
            .with_otlp_endpoint(std::env::var(env::OTEL_EXPORTER_OTLP_ENDPOINT).ok()),
    );

    // Load configuration.
    let config = ApiConfig::load().context("load api config")?;

    // Build AWS clients.
    let sdk = load_aws_config().await;
    let s3 = build_s3_client(&sdk, config.artifacts.endpoint.as_deref())?;
    let ddb = build_ddb_client(&sdk, config.catalog.endpoint.as_deref())?;
    let sns = build_sns_client(&sdk, config.publish.endpoint.as_deref())?;

    // Build application services.
    let catalog: Arc<dyn FunctionCatalog> = Arc::new(DynamoDbFunctionCatalog::new(
        ddb.clone(),
        config.catalog.table.clone(),
    ));
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(S3ArtifactStore::new(
        s3,
        config.artifacts.bucket.clone(),
        config.artifacts.prefix.clone(),
    ));
    let lock = Arc::new(DynamoDbPublishLock::new(
        ddb,
        config.catalog.publish_lock_table.clone(),
    ));
    let bus: Arc<dyn PublishBus> =
        Arc::new(SnsPublishBus::new(sns, config.publish.topic_arn.clone()));

    // Build publish usecase.
    let publish = Arc::new(PublishFunction::new(artifacts, bus, lock));

    // Build HTTP router.
    let app = api::router(ApiState { publish, catalog });
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    info!(
        %addr,
        bucket = %config.artifacts.bucket,
        artifacts_endpoint = ?config.artifacts.endpoint,
        table = %config.catalog.table,
        catalog_endpoint = ?config.catalog.endpoint,
        publish_topic_arn = %config.publish.topic_arn,
        publish_endpoint = ?config.publish.endpoint,
        "nitrum-fn api ready"
    );

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

fn build_s3_client(sdk: &aws_config::SdkConfig, endpoint: Option<&str>) -> Result<S3Client> {
    let mut builder = S3ConfigBuilder::from(sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url).force_path_style(true);
    }
    Ok(S3Client::from_conf(builder.build()))
}

fn build_sns_client(sdk: &aws_config::SdkConfig, endpoint: Option<&str>) -> Result<SnsClient> {
    let mut builder = SnsConfigBuilder::from(sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(SnsClient::from_conf(builder.build()))
}

fn build_ddb_client(sdk: &aws_config::SdkConfig, endpoint: Option<&str>) -> Result<DdbClient> {
    let mut builder = aws_sdk_dynamodb::config::Builder::from(sdk);
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
