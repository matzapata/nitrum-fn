use std::sync::Arc;

use domain::{PublishRequest, PublishResponse, VersionLabel};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{ArtifactStore, FunctionCatalog, FunctionRunner};

pub struct PublishFunction {
    catalog: Arc<dyn FunctionCatalog>,
    artifacts: Arc<dyn ArtifactStore>,
    runner: Arc<dyn FunctionRunner>,
}

impl PublishFunction {
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

    #[instrument(skip(self, req), fields(function = %req.function, wasm_len = req.wasm.len()))]
    pub async fn execute(&self, req: PublishRequest) -> Result<PublishResponse, AppError> {
        if req.wasm.is_empty() {
            return Err(AppError::Compile("empty wasm".into()));
        }

        let hash = self.artifacts.put(&req.wasm).await?;
        let compiled = self.runner.compile(&hash, &req.wasm).await?;
        self.artifacts.put_compiled(&hash, &compiled).await?;

        let version = VersionLabel::latest();
        self.catalog
            .upsert(&req.function, &version, hash.clone())
            .await?;

        Ok(PublishResponse {
            function: req.function,
            version,
            content_hash: hash,
            wasm_bytes: req.wasm.len(),
            compiled_bytes: compiled.len(),
        })
    }
}
