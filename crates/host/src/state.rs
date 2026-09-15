use std::sync::Arc;

use application::ports::FunctionAttestor;
use application::InvokeFunction;
use domain::ContentHash;

use crate::http::invoke_user_data;

#[derive(Clone)]
pub struct AppState {
    pub invoke: Arc<InvokeFunction>,
    pub attestor: Arc<dyn FunctionAttestor>,
}

impl AppState {
    /// When the attestor is active, mint a document over `H || sha256(body)`.
    /// Returns `None` for noop / local.
    pub async fn attest_invoke(
        &self,
        hash: &ContentHash,
        body: &[u8],
        nonce: &[u8],
    ) -> Result<Option<Vec<u8>>, application::AppError> {
        let user_data = invoke_user_data(hash, body);
        self.attestor.attest(&user_data, nonce).await
    }
}
