use async_trait::async_trait;
use domain::{Account, AccountId, KeyId, KeyMeta, KeyRecord, SecretHash};

/// Failures from the account ledger. The use case turns admission failures into
/// [`crate::AppError`] and attaches the credit URL.
#[derive(Debug)]
pub enum StoreError {
    Unauthorized,
    NotFound,
    Insufficient {
        account_id: AccountId,
        balance: u64,
        cost: u64,
        spend_cap_remaining: Option<u64>,
    },
    Storage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebitReceipt {
    pub secret_hash: SecretHash,
    pub account_id: AccountId,
    pub cost: u64,
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreditOutcome {
    Applied {
        balance: u64,
    },
    /// The payment nonce was already credited. Balance is unchanged.
    Replay {
        balance: u64,
    },
}

/// Invoke-credit ledger. Implementations must make debit and credit conditional
/// so concurrent callers cannot overdraw or apply one payment twice.
#[async_trait]
pub trait AccountStore: Send + Sync {
    async fn create(&self, account: &Account, key: &KeyRecord) -> Result<(), StoreError>;

    async fn put_key(&self, key: &KeyRecord) -> Result<(), StoreError>;

    async fn revoke(&self, account_id: &AccountId, key_id: &KeyId) -> Result<(), StoreError>;

    async fn key_by_hash(&self, hash: &SecretHash) -> Result<Option<KeyRecord>, StoreError>;

    async fn account(&self, id: &AccountId) -> Result<Option<Account>, StoreError>;

    async fn list_keys(&self, account_id: &AccountId) -> Result<Vec<KeyMeta>, StoreError>;

    /// Subtract `cost` invoke credits from the account and, when the key is capped,
    /// from `cap_remaining`. One conditional write. Does not create credits.
    async fn debit(&self, secret_hash: &SecretHash, cost: u64) -> Result<DebitReceipt, StoreError>;

    /// Add `receipt.cost` back after a failure before the guest runs.
    async fn refund(&self, receipt: &DebitReceipt) -> Result<(), StoreError>;

    /// Add `invokes` credits once per `nonce`. A replay does not add them again
    /// and does not change any key cap.
    async fn credit(
        &self,
        account_id: &AccountId,
        invokes: u64,
        nonce: &str,
    ) -> Result<CreditOutcome, StoreError>;
}
