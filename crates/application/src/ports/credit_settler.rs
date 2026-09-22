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
    async fn challenge(
        &self,
        account_id: &AccountId,
        invokes: u64,
    ) -> Result<PaymentChallenge, AppError>;

    async fn settle(
        &self,
        account_id: &AccountId,
        invokes: u64,
        payment_header: &str,
    ) -> Result<String, AppError>;
}
