use async_trait::async_trait;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};

use crate::AppError;

#[async_trait]
pub trait FunctionCatalog: Send + Sync {
    async fn upsert(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
        hash: ContentHash,
    ) -> Result<(), AppError>;

    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError>;

    async fn list(&self) -> Result<Vec<FunctionVersion>, AppError>;
}
