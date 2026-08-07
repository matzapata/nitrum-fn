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
    /// Compile (or reuse cached Module) and run one invoke.
    async fn run(&self, hash: &ContentHash, wasm: &[u8], input: &[u8]) -> Result<RunOutcome, AppError>;
}
