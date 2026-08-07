use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError>;
}
