use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Store raw `.wasm` bytes; returns their sha256.
    async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError>;

    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError>;

    /// Store a Wasmtime-serialized module keyed by the source wasm hash.
    async fn put_compiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError>;

    async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError>;
}
