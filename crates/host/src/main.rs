mod config;
mod error;
mod http;
mod state;
mod telemetry;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use api::{catalog_router, publish_router};
use application::ports::{
    ArtifactStore, FunctionCatalog, FunctionRunner, PublishBus, PublishIdempotency,
};
use application::{CompileQueuedFunction, InvokeFunction, PublishFunction};
use artifacts::{FilesystemArtifactStore, S3ArtifactStore};
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType, TableStatus,
};
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sns::Client as SnsClient;
use aws_sdk_sqs::config::Builder as SqsConfigBuilder;
use aws_sdk_sqs::Client as SqsClient;
use catalog::{
    DynamoDbCatalog, DynamoDbPublishIdempotency, FilesystemCatalog, FilesystemPublishIdempotency,
};
use domain::{FunctionId, PublishQueuedEvent};
use executor::WasmtimeRunner;
use messaging::{ensure_queue, SnsPublishBus, SqsPublishBus};
use tracing::{info, warn};

use crate::config::{HostConfig, StoreBackend};
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

    let telemetry = Telemetry::init().context("init telemetry")?;
    let config = HostConfig::from_env();

    let (catalog, artifacts, idempotency) = match config.store {
        StoreBackend::Filesystem => {
            tokio::fs::create_dir_all(&config.artifact_dir)
                .await
                .with_context(|| {
                    format!("create artifact dir {}", config.artifact_dir.display())
                })?;
            let catalog: Arc<dyn FunctionCatalog> = Arc::new(
                FilesystemCatalog::open(&config.catalog_path)
                    .await
                    .with_context(|| format!("open catalog {}", config.catalog_path.display()))?,
            );
            let artifacts: Arc<dyn ArtifactStore> =
                Arc::new(FilesystemArtifactStore::new(config.artifact_dir.clone()));
            let idempotency_path = config.catalog_path.with_file_name("idempotency.json");
            let idempotency: Arc<dyn PublishIdempotency> = Arc::new(
                FilesystemPublishIdempotency::open(&idempotency_path)
                    .await
                    .with_context(|| {
                        format!("open idempotency store {}", idempotency_path.display())
                    })?,
            );
            (catalog, artifacts, idempotency)
        }
        StoreBackend::Aws => {
            let bucket = config
                .s3_bucket
                .clone()
                .context("NITRUM_FN_S3_BUCKET is required when NITRUM_FN_STORE=aws")?;
            let table = config
                .ddb_table
                .clone()
                .context("NITRUM_FN_DDB_TABLE is required when NITRUM_FN_STORE=aws")?;
            let idem_table = config
                .ddb_idempotency_table
                .clone()
                .context("NITRUM_FN_DDB_IDEMPOTENCY_TABLE is required when NITRUM_FN_STORE=aws")?;
            let s3 = build_s3_client(config.s3_endpoint.as_deref()).await?;
            let ddb = build_ddb_client(config.ddb_endpoint.as_deref()).await?;
            if config.s3_create_bucket {
                ensure_bucket(&s3, &bucket).await?;
            }
            if config.ddb_create_table {
                ensure_table(&ddb, &table).await?;
                DynamoDbPublishIdempotency::ensure_table(&ddb, &idem_table)
                    .await
                    .context("ensure idempotency table")?;
            }
            let catalog: Arc<dyn FunctionCatalog> =
                Arc::new(DynamoDbCatalog::new(ddb.clone(), table.clone()));
            let artifacts: Arc<dyn ArtifactStore> =
                Arc::new(S3ArtifactStore::new(s3, bucket.clone(), "artifacts"));
            let idempotency: Arc<dyn PublishIdempotency> =
                Arc::new(DynamoDbPublishIdempotency::new(ddb, idem_table.clone()));
            info!(
                %bucket,
                %table,
                %idem_table,
                "using S3 artifacts and DynamoDB catalog"
            );
            (catalog, artifacts, idempotency)
        }
    };

    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);

    let compile = CompileQueuedFunction::new(catalog.clone(), artifacts.clone(), runner.clone());
    seed_dir(&compile, &artifacts, &config.seed_dir).await?;

    let invoke = Arc::new(InvokeFunction::new(
        catalog.clone(),
        artifacts.clone(),
        runner,
    ));

    let mut app = http::router(AppState { invoke }).merge(catalog_router(catalog));
    match (config.store, build_publish_bus(&config).await?) {
        (StoreBackend::Aws, Some(bus)) => {
            let publish = Arc::new(PublishFunction::new(artifacts, bus, idempotency));
            app = app.merge(publish_router(publish));
        }
        (StoreBackend::Filesystem, Some(_)) => {
            drop(idempotency);
            warn!(
                "HTTP publish is disabled when NITRUM_FN_STORE=fs (seed + invoke only); \
                 use NITRUM_FN_STORE=aws with publish-worker"
            );
        }
        (_, None) => {
            drop(idempotency);
            info!("publish disabled (no NITRUM_FN_SNS_TOPIC_ARN or NITRUM_FN_SQS_QUEUE_URL)");
        }
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    match config.store {
        StoreBackend::Filesystem => info!(
            %addr,
            artifact_dir = %config.artifact_dir.display(),
            catalog_path = %config.catalog_path.display(),
            seed_dir = %config.seed_dir.display(),
            store = "fs",
            "nitrum-fn host listening"
        ),
        StoreBackend::Aws => info!(
            %addr,
            bucket = ?config.s3_bucket,
            s3_endpoint = ?config.s3_endpoint,
            table = ?config.ddb_table,
            ddb_endpoint = ?config.ddb_endpoint,
            sns_topic_arn = ?config.sns_topic_arn,
            sqs_queue_url = ?config.sqs_queue_url,
            seed_dir = %config.seed_dir.display(),
            store = "aws",
            "nitrum-fn host listening"
        ),
    }

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
        let sdk = aws_config::defaults(BehaviorVersion::latest()).load().await;
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
    if config.sqs_create_queue {
        ensure_queue(&client, &queue_url)
            .await
            .context("ensure SQS queue")?;
        info!(%queue_url, "SQS queue ready");
    }
    info!(%queue_url, endpoint = ?config.sqs_endpoint, "publish bus: SQS direct");
    Ok(Some(Arc::new(SqsPublishBus::new(client, queue_url))))
}

async fn build_s3_client(endpoint: Option<&str>) -> Result<S3Client> {
    let sdk = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let mut builder = S3ConfigBuilder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url).force_path_style(true);
    }
    Ok(S3Client::from_conf(builder.build()))
}

async fn build_sqs_client(endpoint: Option<&str>) -> Result<SqsClient> {
    let sdk = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let mut builder = SqsConfigBuilder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(SqsClient::from_conf(builder.build()))
}

async fn build_ddb_client(endpoint: Option<&str>) -> Result<DdbClient> {
    let sdk = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let mut builder = aws_sdk_dynamodb::config::Builder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url);
    }
    Ok(DdbClient::from_conf(builder.build()))
}

async fn ensure_bucket(client: &S3Client, bucket: &str) -> Result<()> {
    match client.head_bucket().bucket(bucket).send().await {
        Ok(_) => {
            info!(%bucket, "S3 bucket already exists");
            Ok(())
        }
        Err(_) => {
            client
                .create_bucket()
                .bucket(bucket)
                .send()
                .await
                .with_context(|| format!("create S3 bucket {bucket}"))?;
            info!(%bucket, "created S3 bucket");
            Ok(())
        }
    }
}

async fn ensure_table(client: &DdbClient, table: &str) -> Result<()> {
    if client
        .describe_table()
        .table_name(table)
        .send()
        .await
        .is_ok()
    {
        info!(%table, "DynamoDB table already exists");
        return wait_table_active(client, table).await;
    }

    client
        .create_table()
        .table_name(table)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("fn_id")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .context("fn_id attribute definition")?,
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("label")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .context("label attribute definition")?,
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("fn_id")
                .key_type(KeyType::Hash)
                .build()
                .context("fn_id key schema")?,
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("label")
                .key_type(KeyType::Range)
                .build()
                .context("label key schema")?,
        )
        .billing_mode(BillingMode::PayPerRequest)
        .send()
        .await
        .with_context(|| format!("create DynamoDB table {table}"))?;
    info!(%table, "created DynamoDB table");
    wait_table_active(client, table).await
}

async fn wait_table_active(client: &DdbClient, table: &str) -> Result<()> {
    for _ in 0..50 {
        let desc = client
            .describe_table()
            .table_name(table)
            .send()
            .await
            .with_context(|| format!("describe DynamoDB table {table}"))?;
        if matches!(
            desc.table().and_then(|t| t.table_status()),
            Some(&TableStatus::Active)
        ) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    anyhow::bail!("DynamoDB table {table} did not become ACTIVE")
}

/// Boot-time fixtures: compile inline so seed works without a running worker.
async fn seed_dir(
    compile: &CompileQueuedFunction,
    artifacts: &Arc<dyn ArtifactStore>,
    seed_dir: &Path,
) -> Result<()> {
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

        let hash = match artifacts.put(&wasm).await {
            Ok(h) => h,
            Err(err) => {
                warn!(%name, error = %err, "skipping seed wasm");
                continue;
            }
        };
        let event = PublishQueuedEvent::new(function.to_string(), hash.to_hex(), wasm.len());
        match compile.execute(&event).await {
            Ok(()) => info!(
                name = %function,
                hash = %hash,
                wasm_bytes = wasm.len(),
                "seeded function @latest"
            ),
            Err(err) => warn!(%name, error = %err, "skipping seed wasm"),
        }
    }

    Ok(())
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
