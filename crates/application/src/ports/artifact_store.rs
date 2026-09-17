use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Store raw `.wasm` bytes and return their sha256 content hash.
    async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError>;

    /// Retrieve raw `.wasm` bytes by content hash (re-hash and reject mismatch).
    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError>;
}
