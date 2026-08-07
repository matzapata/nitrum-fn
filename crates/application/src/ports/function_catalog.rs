use async_trait::async_trait;
use domain::{FunctionId, FunctionVersion, VersionLabel};

use crate::AppError;

#[async_trait]
pub trait FunctionCatalog: Send + Sync {
    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError>;
}
