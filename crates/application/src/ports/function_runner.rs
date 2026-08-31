use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub output: Vec<u8>,
}

#[async_trait]
pub trait FunctionRunner: Send + Sync {
    /// Validate + compile wasm, return serialized AOT bytes.
    async fn compile(&self, hash: &ContentHash, wasm: &[u8]) -> Result<Vec<u8>, AppError>;

    /// Deserialize a serialized module and run one invoke.
    async fn run_precompiled(
        &self,
        hash: &ContentHash,
        compiled: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError>;

    /// Compile from raw wasm and run one invoke.
    async fn run(
        &self,
        hash: &ContentHash,
        wasm: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError>;
}
