use async_trait::async_trait;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};

use crate::AppError;

#[async_trait]
pub trait FunctionCatalog: Send + Sync {
    /// Point `id`@`label` at `hash` if `queued_at_ms` is at least as new as the
    /// generation already stored. Returns `false` when a newer generation won.
    async fn upsert(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
        hash: ContentHash,
        queued_at_ms: u64,
    ) -> Result<bool, AppError>;

    /// Resolve `id`@`label` to the latest version record, or `None` if not found.
    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError>;

    /// List all version records.
    async fn list(&self) -> Result<Vec<FunctionVersion>, AppError>;
}
