use std::path::{Path, PathBuf};

use application::error::AppError;
use application::ports::ArtifactStore;
use async_trait::async_trait;
use domain::ContentHash;

/// Stores artifacts as `{root}/{sha256hex}.wasm`.
pub struct FilesystemArtifactStore {
    root: PathBuf,
}

impl FilesystemArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn path_for(&self, hash: &ContentHash) -> PathBuf {
        self.root.join(format!("{}.wasm", hash.to_hex()))
    }

    pub async fn put(&self, bytes: &[u8]) -> Result<ContentHash, AppError> {
        let hash = ContentHash::from_bytes(bytes);
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| AppError::Invoke(e.to_string()))?;
        let path = self.path_for(&hash);
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| AppError::Invoke(e.to_string()))?;
        Ok(hash)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[async_trait]
impl ArtifactStore for FilesystemArtifactStore {
    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        let path = self.path_for(hash);
        tokio::fs::read(&path)
            .await
            .map_err(|_| AppError::ArtifactMissing(hash.to_hex()))
    }
}
