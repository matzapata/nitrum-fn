//! Fargate / local compile worker: long-poll SQS → AOT → catalog upsert.

mod config;

use std::sync::Arc;
use anyhow::{Context, Result};
use application::ports::{
    ArtifactStore, CompileQueue, FunctionCatalog, FunctionRunner, PublishLock,
};
use application::{AppError, CompileQueuedFunction};
use artifacts::S3ArtifactStore;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sqs::config::Builder as SqsConfigBuilder;
use aws_sdk_sqs::Client as SqsClient;
use catalog::{DynamoDbFunctionCatalog, DynamoDbPublishLock};
use domain::PublishQueuedEvent;
use executor::WasmtimeRunner;
use messaging::{SqsCompileConsumer, COMPILE_VISIBILITY_TIMEOUT_SECS};
use telemetry::{env, TelemetryConfig};
use tracing::{error, info, warn};
use crate::config::WorkerConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let telemetry = telemetry::init(
        TelemetryConfig::new("nitrum-fn-publish-worker")
            .with_otlp_endpoint(std::env::var(env::OTEL_EXPORTER_OTLP_ENDPOINT).ok()),
    );

    // Load configuration.
    let config = WorkerConfig::load().context("load worker config")?;

    // Build AWS clients.
    let sdk = load_aws_config().await;
    let s3 = build_s3_client(&sdk, config.artifacts.endpoint.as_deref())?;
    let ddb = build_ddb_client(&sdk, config.catalog.endpoint.as_deref())?;
    let sqs = build_sqs_client(&sdk, config.compile.endpoint.as_deref())?;

    // Build application services.
    let catalog: Arc<dyn FunctionCatalog> = Arc::new(DynamoDbFunctionCatalog::new(
        ddb.clone(),
        config.catalog.table.clone(),
    ));
    let lock: Arc<dyn PublishLock> = Arc::new(DynamoDbPublishLock::new(
        ddb,
        config.catalog.publish_lock_table.clone(),
    ));
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(S3ArtifactStore::new(
        s3,
        config.artifacts.bucket.clone(),
        config.artifacts.prefix.clone(),
    ));
    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);
    let compile = Arc::new(CompileQueuedFunction::new(catalog, artifacts, runner, lock));

    // Build compile queue consumer.
    let queue_url = config.compile.queue_url.clone();
    let queue: Arc<dyn CompileQueue> =
        Arc::new(SqsCompileConsumer::new(sqs, queue_url.clone()).with_wait_seconds(20));

    info!(
        %queue_url,
        bucket = %config.artifacts.bucket,
        artifacts_endpoint = ?config.artifacts.endpoint,
        table = %config.catalog.table,
        catalog_endpoint = ?config.catalog.endpoint,
        compile_endpoint = ?config.compile.endpoint,
        "nitrum-fn-publish-worker ready"
    );

    loop {
        tokio::select! {
            _ = shutdown_signal() => break,
            result = queue.receive() => {
                match result {
                    Ok(None) => {}
                    Ok(Some(msg)) => {
                        match compile_with_heartbeat(
                            queue.as_ref(),
                            &msg.receipt_handle,
                            compile.as_ref(),
                            &msg.event,
                        )
                        .await
                        {
                            Ok(()) => {
                                if let Err(err) = queue.delete(&msg.receipt_handle).await {
                                    error!(error = %err, "failed to delete SQS message after success");
                                } else {
                                    info!(
                                        function = %msg.event.function,
                                        hash = %msg.event.content_hash,
                                        "compiled and cataloged"
                                    );
                                }
                            }
                            Err(err) => {
                                // Leave message for retry / DLQ.
                                warn!(
                                    error = %err,
                                    function = %msg.event.function,
                                    hash = %msg.event.content_hash,
                                    "compile failed; message left for retry"
                                );
                            }
                        }
                    }
                    Err(err) => {
                        error!(error = %err, "SQS receive failed");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }

    telemetry.shutdown();
    Ok(())
}

async fn compile_with_heartbeat(
    queue: &dyn CompileQueue,
    receipt_handle: &str,
    compile: &CompileQueuedFunction,
    event: &PublishQueuedEvent,
) -> Result<(), AppError> {
    let heartbeat = async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(err) = queue
                .extend_visibility(receipt_handle, COMPILE_VISIBILITY_TIMEOUT_SECS)
                .await
            {
                warn!(error = %err, "failed to extend SQS visibility");
            }
        }
    };
    tokio::select! {
        result = compile.execute(event) => result,
        _ = heartbeat => unreachable!("heartbeat loop does not complete"),
    }
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

fn build_sqs_client(sdk: &aws_config::SdkConfig, endpoint: Option<&str>) -> Result<SqsClient> {
    let mut builder = SqsConfigBuilder::from(sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(SqsClient::from_conf(builder.build()))
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
