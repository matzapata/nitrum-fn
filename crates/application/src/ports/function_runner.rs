use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub output: Vec<u8>,
}

#[async_trait]
pub trait FunctionRunner: Send + Sync {
    /// Validate guest wasm (parse + ABI) without instantiating.
    async fn validate(&self, wasm: &[u8]) -> Result<(), AppError>;

    /// Compile from raw wasm and run one invoke.
    async fn run(
        &self,
        hash: &ContentHash,
        wasm: &[u8],
        input: &[u8],
        egress_allow: &[domain::EgressOrigin],
    ) -> Result<RunOutcome, AppError>;
}
