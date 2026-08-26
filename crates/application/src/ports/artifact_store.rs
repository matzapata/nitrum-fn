use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Store raw `.wasm` bytes and return their sha256 content hash.
    async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError>;

    /// Retrieve raw `.wasm` bytes by content hash.
    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError>;

    /// Store a Wasmtime-serialized compiled module, keyed by the source wasm hash.
    async fn put_compiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError>;

    /// Retrieve a Wasmtime-serialized compiled module by wasm content hash.
    async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError>;
}
