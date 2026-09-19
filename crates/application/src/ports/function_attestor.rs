use async_trait::async_trait;

use crate::AppError;

/// Mint a Nitro attestation document over opaque `user_data` + client `nonce`.
///
/// Returns `None` when attestation is not available (local / noop). When the
/// attestor is configured, a failure is an error — do not return an unverified body
/// as if it were attested.
#[async_trait]
pub trait FunctionAttestor: Send + Sync {
    async fn attest(&self, user_data: &[u8], nonce: &[u8]) -> Result<Option<Vec<u8>>, AppError>;
}
