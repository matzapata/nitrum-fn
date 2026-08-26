mod config;
mod error;
mod http;
mod state;
mod telemetry;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use api::{catalog_router, publish_router};
use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner, PublishBus};
use application::{InvokeFunction, PublishFunction};
use artifacts::S3ArtifactStore;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sns::Client as SnsClient;
use aws_sdk_sqs::config::Builder as SqsConfigBuilder;
use aws_sdk_sqs::Client as SqsClient;
use catalog::{DynamoDbCatalog, DynamoDbPublishIdempotency};
use executor::WasmtimeRunner;
use messaging::{SnsPublishBus, SqsPublishBus};
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
    let config = HostConfig::from_env().context("load host config")?;

    let s3 = build_s3_client(config.s3_endpoint.as_deref()).await?;
    let ddb = build_ddb_client(config.ddb_endpoint.as_deref()).await?;
    let catalog: Arc<dyn FunctionCatalog> =
        Arc::new(DynamoDbCatalog::new(ddb.clone(), config.ddb_table.clone()));
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(S3ArtifactStore::new(
        s3,
        config.s3_bucket.clone(),
        "artifacts",
    ));
    let idempotency = Arc::new(DynamoDbPublishIdempotency::new(
        ddb,
        config.ddb_idempotency_table.clone(),
    ));
    info!(
        bucket = %config.s3_bucket,
        table = %config.ddb_table,
        idem_table = %config.ddb_idempotency_table,
        "using S3 artifacts and DynamoDB catalog"
    );

    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);

    let invoke = Arc::new(InvokeFunction::new(
        catalog.clone(),
        artifacts.clone(),
        runner,
    ));

    let mut app = http::router(AppState { invoke }).merge(catalog_router(catalog));
    match build_publish_bus(&config).await? {
        Some(bus) => {
            let publish = Arc::new(PublishFunction::new(artifacts, bus, idempotency));
            app = app.merge(publish_router(publish));
        }
        None => {
            info!("publish disabled (no NITRUM_FN_SNS_TOPIC_ARN or NITRUM_FN_SQS_QUEUE_URL)");
        }
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!(
        %addr,
        bucket = %config.s3_bucket,
        s3_endpoint = ?config.s3_endpoint,
        table = %config.ddb_table,
        ddb_endpoint = ?config.ddb_endpoint,
        sns_topic_arn = ?config.sns_topic_arn,
        sqs_queue_url = ?config.sqs_queue_url,
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

async fn build_publish_bus(config: &HostConfig) -> Result<Option<Arc<dyn PublishBus>>> {
    if let Some(topic_arn) = &config.sns_topic_arn {
        let sdk = load_aws_config().await;
        let client = SnsClient::new(&sdk);
        info!(%topic_arn, "publish bus: SNS");
        return Ok(Some(Arc::new(SnsPublishBus::new(
            client,
            topic_arn.clone(),
        ))));
    }
    let Some(queue_url) = config.sqs_queue_url.clone() else {
        return Ok(None);
    };
    let client = build_sqs_client(config.sqs_endpoint.as_deref()).await?;
    info!(%queue_url, endpoint = ?config.sqs_endpoint, "publish bus: SQS direct");
    Ok(Some(Arc::new(SqsPublishBus::new(client, queue_url))))
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

async fn build_sqs_client(endpoint: Option<&str>) -> Result<SqsClient> {
    let sdk = load_aws_config().await;
    let mut builder = SqsConfigBuilder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(SqsClient::from_conf(builder.build()))
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
