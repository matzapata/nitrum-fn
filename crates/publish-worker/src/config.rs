use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreBackend {
    Filesystem,
    Aws,
}

pub struct WorkerConfig {
    pub store: StoreBackend,
    pub artifact_dir: PathBuf,
    pub catalog_path: PathBuf,
    pub s3_bucket: Option<String>,
    pub s3_endpoint: Option<String>,
    pub ddb_table: Option<String>,
    pub ddb_endpoint: Option<String>,
    pub sqs_queue_url: Option<String>,
    pub sqs_endpoint: Option<String>,
    pub sqs_create_queue: bool,
}

impl WorkerConfig {
    pub fn from_env() -> Self {
        let store = match std::env::var("NITRUM_FN_STORE")
            .unwrap_or_else(|_| "fs".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "aws" => StoreBackend::Aws,
            _ => StoreBackend::Filesystem,
        };
        Self {
            store,
            artifact_dir: std::env::var("NITRUM_FN_ARTIFACT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./.data/artifacts")),
            catalog_path: std::env::var("NITRUM_FN_CATALOG_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./.data/catalog.json")),
            s3_bucket: std::env::var("NITRUM_FN_S3_BUCKET")
                .ok()
                .filter(|s| !s.is_empty()),
            s3_endpoint: std::env::var("NITRUM_FN_S3_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
            ddb_table: std::env::var("NITRUM_FN_DDB_TABLE")
                .ok()
                .filter(|s| !s.is_empty()),
            ddb_endpoint: std::env::var("NITRUM_FN_DDB_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
            sqs_queue_url: std::env::var("NITRUM_FN_SQS_QUEUE_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            sqs_endpoint: std::env::var("NITRUM_FN_SQS_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
            sqs_create_queue: env_flag("NITRUM_FN_SQS_CREATE_QUEUE"),
        }
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}
