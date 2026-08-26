mod config;

use std::net::SocketAddr;
use std::sync::Arc;

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
use tracing::info;

use crate::config::ApiConfig;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ApiConfig::load().context("load api config")?;

    let s3 = build_s3_client(config.artifacts.endpoint.as_deref()).await?;
    let ddb = build_ddb_client(config.catalog.endpoint.as_deref()).await?;
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
    info!(
        bucket = %config.artifacts.bucket,
        table = %config.catalog.table,
        publish_lock_table = %config.catalog.publish_lock_table,
        "using S3 artifacts and DynamoDB catalog"
    );

    let bus = build_publish_bus(&config).await?;
    let publish = Arc::new(PublishFunction::new(artifacts, bus, lock));

    let app = api::router(ApiState { publish, catalog });

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!(
        %addr,
        bucket = %config.artifacts.bucket,
        artifacts_endpoint = ?config.artifacts.endpoint,
        table = %config.catalog.table,
        catalog_endpoint = ?config.catalog.endpoint,
        publish_topic_arn = %config.publish.topic_arn,
        publish_endpoint = ?config.publish.endpoint,
        "nitrum-fn api listening"
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

async fn build_publish_bus(config: &ApiConfig) -> Result<Arc<dyn PublishBus>> {
    let client = build_sns_client(config.publish.endpoint.as_deref()).await?;
    info!(
        topic_arn = %config.publish.topic_arn,
        endpoint = ?config.publish.endpoint,
        "publish bus: SNS"
    );
    Ok(Arc::new(SnsPublishBus::new(
        client,
        config.publish.topic_arn.clone(),
    )))
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

async fn build_sns_client(endpoint: Option<&str>) -> Result<SnsClient> {
    let sdk = load_aws_config().await;
    let mut builder = SnsConfigBuilder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(SnsClient::from_conf(builder.build()))
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
