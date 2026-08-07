use std::sync::Arc;

use domain::{InvokeRequest, InvokeResponse};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};

pub struct InvokeFunction {
    catalog: Arc<dyn FunctionCatalog>,
    artifacts: Arc<dyn ArtifactStore>,
    runner: Arc<dyn FunctionRunner>,
}

impl InvokeFunction {
    pub fn new(
        catalog: Arc<dyn FunctionCatalog>,
        artifacts: Arc<dyn ArtifactStore>,
        runner: Arc<dyn FunctionRunner>,
    ) -> Self {
        Self {
            catalog,
            artifacts,
            runner,
        }
    }

    #[instrument(skip(self, req), fields(function = %req.function, version = %req.version.as_str()))]
    pub async fn execute(&self, req: InvokeRequest) -> Result<InvokeResponse, AppError> {
        let version = self.catalog.resolve(&req.function, &req.version).await?;
        let wasm = self.artifacts.get(&version.content_hash).await?;

        let actual = domain::ContentHash::from_bytes(&wasm);
        if actual != version.content_hash {
            return Err(AppError::HashMismatch {
                expected: version.content_hash.to_hex(),
                actual: actual.to_hex(),
            });
        }

        let outcome = self
            .runner
            .run(&version.content_hash, &wasm, &req.payload)
            .await?;

        Ok(InvokeResponse {
            output: outcome.output,
            warm_module: outcome.warm_module,
        })
    }
}
