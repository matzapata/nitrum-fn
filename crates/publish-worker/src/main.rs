//! Fargate / local compile worker: long-poll SQS → AOT → catalog upsert.

mod config;

use std::sync::Arc;

use anyhow::{Context, Result};
use application::ports::{ArtifactStore, CompileQueue, FunctionCatalog, FunctionRunner};
use application::{AppError, CompileQueuedFunction};
use artifacts::S3ArtifactStore;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sqs::config::Builder as SqsConfigBuilder;
use aws_sdk_sqs::Client as SqsClient;
use catalog::DynamoDbCatalog;
use domain::PublishQueuedEvent;
use executor::WasmtimeRunner;
use messaging::{SqsCompileConsumer, COMPILE_VISIBILITY_TIMEOUT_SECS};
use tracing::{error, info, warn};

use crate::config::WorkerConfig;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = WorkerConfig::from_env().context("load worker config")?;
    let queue_url = config.sqs_queue_url.clone();

    let s3 = build_s3_client(config.s3_endpoint.as_deref()).await?;
    let ddb = build_ddb_client(config.ddb_endpoint.as_deref()).await?;
    let catalog: Arc<dyn FunctionCatalog> =
        Arc::new(DynamoDbCatalog::new(ddb, config.ddb_table.clone()));
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(S3ArtifactStore::new(
        s3,
        config.s3_bucket.clone(),
        "artifacts",
    ));

    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);
    let compile = Arc::new(CompileQueuedFunction::new(catalog, artifacts, runner));

    let sqs = build_sqs_client(config.sqs_endpoint.as_deref()).await?;
    let queue: Arc<dyn CompileQueue> =
        Arc::new(SqsCompileConsumer::new(sqs, queue_url.clone()).with_wait_seconds(20));

    info!(
        %queue_url,
        endpoint = ?config.sqs_endpoint,
        "nitrum-fn-publish-worker listening"
    );

    loop {
        tokio::select! {
            _ = shutdown_signal() => {
                info!("shutdown signal received");
                break;
            }
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
}
