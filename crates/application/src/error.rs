use domain::DomainError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    /// Domain error.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// Function not found.
    #[error("function not found: {0}")]
    NotFound(String),

    /// Another publish of this function is already in progress.
    #[error("publish conflict: {0}")]
    Conflict(String),

    /// Artifact missing for hash.
    #[error("artifact missing for hash {0}")]
    ArtifactMissing(String),

    /// Artifact hash mismatch.
    #[error("artifact hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    /// Request header or other client input failed validation.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Compile failed.
    #[error("compile failed: {0}")]
    Compile(String),

    /// Storage operation failed.
    #[error("storage: {0}")]
    Storage(String),

    /// Invoke failed.
    #[error("invoke failed: {0}")]
    Invoke(String),

    /// Guest trapped while executing `invoke`.
    #[error("guest trap: {0}")]
    Trap(String),

    /// Guest hit the wall-clock invoke deadline (Wasmtime epoch interrupt).
    #[error("invoke timed out: {0}")]
    Timeout(String),

    /// Request body, wasm artifact, or guest output exceeded a product limit.
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    /// Enclave attestation (NSM / crypto API) failed.
    #[error("attestation: {0}")]
    Attestation(String),

    /// Missing, unknown, or revoked bearer.
    #[error("unauthorized")]
    Unauthorized,

    /// Invoke or deploy refused because the credit balance or spend cap is too low.
    #[error("insufficient invoke credits")]
    InsufficientCredits {
        credit_cost: u64,
        balance: u64,
        credit_url: String,
        spend_cap_remaining: Option<u64>,
    },

    /// Publish refused because the balance is under the deploy minimum. Balance is unchanged.
    #[error("balance below minimum deploy credits")]
    DeployMinimum {
        min_deploy_credits: u64,
        balance: u64,
        credit_url: String,
    },

    /// x402 challenge for buying invoke credits. Not used on the invoke host.
    #[error("payment required")]
    PaymentChallenge {
        invokes: u64,
        amount: u64,
        asset: String,
        pay_to: String,
        resource: String,
        network: String,
    },
}

impl AppError {
    /// True for failures whose Display may include driver or guest details.
    pub fn is_internal(&self) -> bool {
        matches!(
            self,
            Self::Invoke(_) | Self::Trap(_) | Self::Storage(_) | Self::Attestation(_)
        )
    }

    /// Stable client-facing message. Log [`Display`] separately.
    pub fn public_message(&self) -> String {
        if self.is_internal() {
            "internal error".into()
        } else {
            self.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn internal_errors_do_not_leak_in_public_message() {
        let err = AppError::Storage("table nitrum-fn AccessDeniedException".into());
        assert_eq!(err.public_message(), "internal error");
        assert!(err.to_string().contains("AccessDeniedException"));
    }

    #[test]
    fn client_errors_keep_their_display() {
        let err = AppError::NotFound("echo@latest".into());
        assert_eq!(err.public_message(), err.to_string());
        let err = AppError::BadRequest("invalid x-nitrum-fn-nonce (base64)".into());
        assert_eq!(err.public_message(), err.to_string());
    }
}
