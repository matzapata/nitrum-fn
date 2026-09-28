use async_trait::async_trait;
use domain::AccountId;

use crate::AppError;

/// x402 challenge for `POST /accounts/{id}/credit`. Amount is atomic USDC (6 decimals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentChallenge {
    pub invokes: u64,
    pub amount: u64,
    pub asset: String,
    pub pay_to: String,
    pub resource: String,
    pub network: String,
}

/// Local exact-check plus facilitator settle. Returns the payment nonce only
/// after settle succeeds. Does not credit the ledger.
#[async_trait]
pub trait CreditSettler: Send + Sync {
    /// Build the x402 challenge (price, asset, payee, resource, network) for
    /// crediting `invokes` invokes to the account. Performs no payment check
    /// and no I/O against the facilitator.
    async fn challenge(
        &self,
        account_id: &AccountId,
        invokes: u64,
    ) -> Result<PaymentChallenge, AppError>;

    /// Run the local exact-check and return the payment nonce without calling
    /// the facilitator, so callers can spot an already-credited payment before
    /// settling it a second time.
    async fn payment_nonce(&self, invokes: u64, payment_header: &str) -> Result<String, AppError>;

    /// Run the local exact-check, then settle the payment through the
    /// facilitator. Returns the payment nonce only once settle has succeeded;
    /// the caller is responsible for crediting the ledger afterwards.
    async fn settle(
        &self,
        account_id: &AccountId,
        invokes: u64,
        payment_header: &str,
    ) -> Result<String, AppError>;
}
