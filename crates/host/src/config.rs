use std::path::PathBuf;

pub struct HostConfig {
    pub port: u16,
    pub artifact_dir: PathBuf,
    pub catalog_path: PathBuf,
    /// Local-only: each `name.wasm` here is published as `name@latest` on boot.
    pub seed_dir: PathBuf,
}

impl HostConfig {
    pub fn from_env() -> Self {
        let port = std::env::var("NITRUM_FN_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        let artifact_dir = std::env::var("NITRUM_FN_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.data/artifacts"));
        let catalog_path = std::env::var("NITRUM_FN_CATALOG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.data/catalog.json"));
        let seed_dir = std::env::var("NITRUM_FN_SEED_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.data/seed"));
        Self {
            port,
            artifact_dir,
            catalog_path,
            seed_dir,
        }
    }
}
