mod config;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use api::ApiState;
use application::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};
use application::PublishFunction;
use artifacts::{FilesystemArtifactStore, S3ArtifactStore};
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType, TableStatus,
};
use aws_sdk_dynamodb::Client as DdbClient;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client as S3Client;
use catalog::{DynamoDbCatalog, FilesystemCatalog};
use executor::WasmtimeRunner;
use tracing::info;

use crate::config::{ApiConfig, StoreBackend};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = ApiConfig::from_env();

    let (catalog, artifacts) = match config.store {
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
            (catalog, artifacts)
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
            let s3 = build_s3_client(config.s3_endpoint.as_deref()).await?;
            let ddb = build_ddb_client(config.ddb_endpoint.as_deref()).await?;
            if config.s3_create_bucket {
                ensure_bucket(&s3, &bucket).await?;
            }
            if config.ddb_create_table {
                ensure_table(&ddb, &table).await?;
            }
            let catalog: Arc<dyn FunctionCatalog> =
                Arc::new(DynamoDbCatalog::new(ddb, table.clone()));
            let artifacts: Arc<dyn ArtifactStore> =
                Arc::new(S3ArtifactStore::new(s3, bucket.clone(), "artifacts"));
            info!(%bucket, %table, "using S3 artifacts and DynamoDB catalog");
            (catalog, artifacts)
        }
    };

    let runner: Arc<dyn FunctionRunner> =
        Arc::new(WasmtimeRunner::new().context("create wasmtime runner")?);

    let publish = Arc::new(PublishFunction::new(catalog.clone(), artifacts, runner));

    let app = api::router(ApiState { publish, catalog });

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    match config.store {
        StoreBackend::Filesystem => info!(
            %addr,
            artifact_dir = %config.artifact_dir.display(),
            catalog_path = %config.catalog_path.display(),
            store = "fs",
            "nitrum-fn api listening"
        ),
        StoreBackend::Aws => info!(
            %addr,
            bucket = ?config.s3_bucket,
            s3_endpoint = ?config.s3_endpoint,
            table = ?config.ddb_table,
            ddb_endpoint = ?config.ddb_endpoint,
            store = "aws",
            "nitrum-fn api listening"
        ),
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;

    Ok(())
}

async fn build_s3_client(endpoint: Option<&str>) -> Result<S3Client> {
    let sdk = aws_config::defaults(BehaviorVersion::latest()).load().await;
    let mut builder = S3ConfigBuilder::from(&sdk);
    if let Some(url) = endpoint {
        builder = builder.endpoint_url(url).force_path_style(true);
    }
    Ok(S3Client::from_conf(builder.build()))
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

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown signal received");
}
