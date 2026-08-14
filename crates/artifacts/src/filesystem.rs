use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use application::error::AppError;
use application::ports::ArtifactStore;
use async_trait::async_trait;
use domain::ContentHash;

/// Stores `{root}/{sha256hex}.wasm` and `{root}/{sha256hex}.cwasm`.
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

    pub fn compiled_path_for(&self, hash: &ContentHash) -> PathBuf {
        self.root.join(format!("{}.cwasm", hash.to_hex()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    async fn read_path(&self, path: &Path, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        match tokio::fs::read(path).await {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                Err(AppError::ArtifactMissing(hash.to_hex()))
            }
            Err(err) => Err(AppError::Storage(err.to_string())),
        }
    }
}

#[async_trait]
impl ArtifactStore for FilesystemArtifactStore {
    async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError> {
        let hash = ContentHash::from_bytes(wasm);
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        tokio::fs::write(self.path_for(&hash), wasm)
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(hash)
    }

    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        self.read_path(&self.path_for(hash), hash).await
    }

    async fn put_compiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        tokio::fs::write(self.compiled_path_for(hash), compiled)
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        self.read_path(&self.compiled_path_for(hash), hash).await
    }
}
