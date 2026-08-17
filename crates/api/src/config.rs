use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreBackend {
    Filesystem,
    Aws,
}

pub struct ApiConfig {
    pub port: u16,
    pub store: StoreBackend,
    pub artifact_dir: PathBuf,
    pub catalog_path: PathBuf,
    pub s3_bucket: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_create_bucket: bool,
    pub ddb_table: Option<String>,
    pub ddb_endpoint: Option<String>,
    pub ddb_create_table: bool,
}

impl ApiConfig {
    pub fn from_env() -> Self {
        let port = std::env::var("NITRUM_FN_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        let store = match std::env::var("NITRUM_FN_STORE")
            .unwrap_or_else(|_| "fs".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "aws" => StoreBackend::Aws,
            _ => StoreBackend::Filesystem,
        };
        let artifact_dir = std::env::var("NITRUM_FN_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.data/artifacts"));
        let catalog_path = std::env::var("NITRUM_FN_CATALOG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.data/catalog.json"));
        let s3_bucket = std::env::var("NITRUM_FN_S3_BUCKET")
            .ok()
            .filter(|s| !s.is_empty());
        let s3_endpoint = std::env::var("NITRUM_FN_S3_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty());
        let s3_create_bucket = env_flag("NITRUM_FN_S3_CREATE_BUCKET");
        let ddb_table = std::env::var("NITRUM_FN_DDB_TABLE")
            .ok()
            .filter(|s| !s.is_empty());
        let ddb_endpoint = std::env::var("NITRUM_FN_DDB_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty());
        let ddb_create_table = env_flag("NITRUM_FN_DDB_CREATE_TABLE");
        Self {
            port,
            store,
            artifact_dir,
            catalog_path,
            s3_bucket,
            s3_endpoint,
            s3_create_bucket,
            ddb_table,
            ddb_endpoint,
            ddb_create_table,
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
