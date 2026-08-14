use async_trait::async_trait;
use domain::ContentHash;

use crate::AppError;

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub output: Vec<u8>,
    pub warm_module: bool,
}

#[async_trait]
pub trait FunctionRunner: Send + Sync {
    /// Validate + compile wasm, cache the Module, return serialized AOT bytes.
    async fn compile(&self, hash: &ContentHash, wasm: &[u8]) -> Result<Vec<u8>, AppError>;

    /// Deserialize a previously serialized module into the in-process cache.
    async fn load_precompiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError>;

    /// Run from a serialized module (cache hit or deserialize).
    async fn run_precompiled(
        &self,
        hash: &ContentHash,
        compiled: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError>;

    /// Compile (or reuse cached Module) from raw wasm and run one invoke.
    async fn run(
        &self,
        hash: &ContentHash,
        wasm: &[u8],
        input: &[u8],
    ) -> Result<RunOutcome, AppError>;
}
