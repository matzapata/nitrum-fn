use std::sync::Arc;

use domain::{
    authorize_additional_key, Account, AccountId, BearerSecret, KeyId, KeyMeta, KeyRecord, NewKey,
    SecretHash,
};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{AccountStore, CreditSettler, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedAccount {
    pub account_id: AccountId,
    pub key_id: KeyId,
    pub secret: BearerSecret,
    pub balance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedKey {
    pub key_id: KeyId,
    pub secret: BearerSecret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountView {
    pub account_id: AccountId,
    pub balance: u64,
    pub keys: Vec<KeyMeta>,
}

pub struct Accounts {
    store: Arc<dyn AccountStore>,
    settler: Arc<dyn CreditSettler>,
    operator_token: String,
    credit_base: String,
}

impl Accounts {
    pub fn new(
        store: Arc<dyn AccountStore>,
        settler: Arc<dyn CreditSettler>,
        operator_token: impl Into<String>,
        credit_base: impl Into<String>,
    ) -> Self {
        Self {
            store,
            settler,
            operator_token: operator_token.into(),
            credit_base: credit_base.into().trim_end_matches('/').to_string(),
        }
    }

    #[instrument(skip(self))]
    pub async fn create(&self) -> Result<CreatedAccount, AppError> {
        let account = Account::open()?;
        let issued = NewKey::issue(account.id.clone(), None)?;
        self.store.create(&account, &issued.record).await?;
        Ok(CreatedAccount {
            account_id: account.id,
            key_id: issued.record.key_id,
            secret: issued.secret,
            balance: 0,
        })
    }

    #[instrument(skip(self, bearer))]
    pub async fn issue_key(
        &self,
        account_id: &AccountId,
        bearer: Option<&BearerSecret>,
        spend_cap: Option<u64>,
    ) -> Result<IssuedKey, AppError> {
        let presented = self.presented_key(bearer).await?;
        authorize_additional_key(account_id, presented.as_ref())
            .map_err(|_| AppError::Unauthorized)?;
        let issued = NewKey::issue(account_id.clone(), spend_cap)?;
        self.store.put_key(&issued.record).await?;
        Ok(IssuedKey {
            key_id: issued.record.key_id,
            secret: issued.secret,
        })
    }

    #[instrument(skip(self, bearer))]
    pub async fn revoke(
        &self,
        account_id: &AccountId,
        key_id: &KeyId,
        bearer: Option<&BearerSecret>,
    ) -> Result<(), AppError> {
        let presented = self.presented_key(bearer).await?;
        authorize_additional_key(account_id, presented.as_ref())
            .map_err(|_| AppError::Unauthorized)?;
        let before = self.store.account(account_id).await?;
        self.store.revoke(account_id, key_id).await?;
        let after = self.store.account(account_id).await?;
        if before.as_ref().map(|a| a.balance) != after.as_ref().map(|a| a.balance) {
            return Err(AppError::Storage(
                "revoke changed the invoke-credit balance".into(),
            ));
        }
        Ok(())
    }

    #[instrument(skip(self, bearer))]
    pub async fn read(&self, bearer: &BearerSecret) -> Result<AccountView, AppError> {
        let key = self
            .presented_key(Some(bearer))
            .await?
            .ok_or(AppError::Unauthorized)?;
        if key.revoked {
            return Err(AppError::Unauthorized);
        }
        let account = self
            .store
            .account(&key.account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(key.account_id.to_string()))?;
        let keys = self.store.list_keys(&key.account_id).await?;
        Ok(AccountView {
            account_id: account.id,
            balance: account.balance,
            keys,
        })
    }

    /// Buy `invokes` credits. The settler must succeed before the ledger is credited.
    #[instrument(skip(self, payment_header))]
    pub async fn credit(
        &self,
        account_id: &AccountId,
        invokes: u64,
        payment_header: Option<&str>,
    ) -> Result<u64, AppError> {
        if invokes == 0 {
            return Err(AppError::BadRequest(
                "invokes must be a positive number".into(),
            ));
        }
        let Some(header) = payment_header.filter(|h| !h.is_empty()) else {
            let challenge = self.settler.challenge(account_id, invokes).await?;
            return Err(AppError::PaymentChallenge {
                invokes: challenge.invokes,
                amount: challenge.amount,
                asset: challenge.asset,
                pay_to: challenge.pay_to,
                resource: challenge.resource,
                network: challenge.network,
            });
        };
        let nonce = self.settler.settle(account_id, invokes, header).await?;
        let outcome = self.store.credit(account_id, invokes, &nonce).await?;
        Ok(outcome.balance())
    }

    #[instrument(skip(self, presented_token))]
    pub async fn admin_credit(
        &self,
        account_id: &AccountId,
        invokes: u64,
        presented_token: &str,
    ) -> Result<u64, AppError> {
        if !constant_time_eq(presented_token, &self.operator_token)
            || self.operator_token.is_empty()
        {
            return Err(AppError::Unauthorized);
        }
        if invokes == 0 {
            return Err(AppError::BadRequest(
                "invokes must be a positive number".into(),
            ));
        }
        Ok(self.store.admin_credit(account_id, invokes).await?)
    }

    pub fn credit_url(&self, account_id: &AccountId) -> String {
        format!("{}/accounts/{account_id}/credit", self.credit_base)
    }

    async fn presented_key(
        &self,
        bearer: Option<&BearerSecret>,
    ) -> Result<Option<KeyRecord>, AppError> {
        let Some(secret) = bearer else {
            return Ok(None);
        };
        let hash = SecretHash::of(secret);
        Ok(self.store.key_by_hash(&hash).await?)
    }
}

impl crate::ports::CreditOutcome {
    fn balance(&self) -> u64 {
        match self {
            Self::Applied { balance } | Self::Replay { balance } => *balance,
        }
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let a = left.as_bytes();
    let b = right.as_bytes();
    let mut diff = a.len() ^ b.len();
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

impl From<StoreError> for AppError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::Unauthorized => AppError::Unauthorized,
            StoreError::NotFound => AppError::NotFound("account".into()),
            StoreError::Insufficient {
                account_id,
                balance,
                cost,
                spend_cap_remaining,
            } => AppError::InsufficientCredits {
                credit_cost: cost,
                balance,
                credit_url: format!("/accounts/{account_id}/credit"),
                spend_cap_remaining,
            },
            StoreError::Storage(msg) => AppError::Storage(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{CreditOutcome, CreditSettler, PaymentChallenge};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MemStore {
        accounts: Mutex<HashMap<String, Account>>,
        keys: Mutex<HashMap<String, KeyRecord>>,
        receipts: Mutex<HashMap<String, u64>>,
        credits: Mutex<u32>,
    }

    impl MemStore {
        fn new() -> Self {
            Self {
                accounts: Mutex::new(HashMap::new()),
                keys: Mutex::new(HashMap::new()),
                receipts: Mutex::new(HashMap::new()),
                credits: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl AccountStore for MemStore {
        async fn create(&self, account: &Account, key: &KeyRecord) -> Result<(), StoreError> {
            self.accounts
                .lock()
                .unwrap()
                .insert(account.id.to_hex(), account.clone());
            self.keys
                .lock()
                .unwrap()
                .insert(key.secret_hash.to_hex(), key.clone());
            Ok(())
        }

        async fn put_key(&self, key: &KeyRecord) -> Result<(), StoreError> {
            self.keys
                .lock()
                .unwrap()
                .insert(key.secret_hash.to_hex(), key.clone());
            Ok(())
        }

        async fn revoke(&self, account_id: &AccountId, key_id: &KeyId) -> Result<(), StoreError> {
            let mut keys = self.keys.lock().unwrap();
            let key = keys
                .values_mut()
                .find(|k| &k.account_id == account_id && &k.key_id == key_id)
                .ok_or(StoreError::NotFound)?;
            key.revoked = true;
            Ok(())
        }

        async fn key_by_hash(&self, hash: &SecretHash) -> Result<Option<KeyRecord>, StoreError> {
            Ok(self.keys.lock().unwrap().get(&hash.to_hex()).cloned())
        }

        async fn account(&self, id: &AccountId) -> Result<Option<Account>, StoreError> {
            Ok(self.accounts.lock().unwrap().get(&id.to_hex()).cloned())
        }

        async fn list_keys(&self, account_id: &AccountId) -> Result<Vec<KeyMeta>, StoreError> {
            Ok(self
                .keys
                .lock()
                .unwrap()
                .values()
                .filter(|k| &k.account_id == account_id)
                .map(KeyMeta::from)
                .collect())
        }

        async fn debit(
            &self,
            _secret_hash: &SecretHash,
            _cost: u64,
        ) -> Result<crate::ports::DebitReceipt, StoreError> {
            unimplemented!()
        }

        async fn refund(&self, _receipt: &crate::ports::DebitReceipt) -> Result<(), StoreError> {
            unimplemented!()
        }

        async fn credit(
            &self,
            account_id: &AccountId,
            invokes: u64,
            nonce: &str,
        ) -> Result<CreditOutcome, StoreError> {
            if self.receipts.lock().unwrap().contains_key(nonce) {
                let balance = self
                    .account(account_id)
                    .await?
                    .ok_or(StoreError::NotFound)?
                    .balance;
                return Ok(CreditOutcome::Replay { balance });
            }
            let mut accounts = self.accounts.lock().unwrap();
            let account = accounts
                .get_mut(&account_id.to_hex())
                .ok_or(StoreError::NotFound)?;
            account.balance = account.balance.saturating_add(invokes);
            let balance = account.balance;
            drop(accounts);
            self.receipts
                .lock()
                .unwrap()
                .insert(nonce.to_string(), invokes);
            *self.credits.lock().unwrap() += 1;
            Ok(CreditOutcome::Applied { balance })
        }

        async fn admin_credit(
            &self,
            account_id: &AccountId,
            invokes: u64,
        ) -> Result<u64, StoreError> {
            let mut accounts = self.accounts.lock().unwrap();
            let account = accounts
                .get_mut(&account_id.to_hex())
                .ok_or(StoreError::NotFound)?;
            account.balance = account.balance.saturating_add(invokes);
            Ok(account.balance)
        }
    }

    struct Settler {
        fail: bool,
        calls: Mutex<u32>,
    }

    #[async_trait]
    impl CreditSettler for Settler {
        async fn challenge(
            &self,
            account_id: &AccountId,
            invokes: u64,
        ) -> Result<PaymentChallenge, AppError> {
            Ok(PaymentChallenge {
                invokes,
                amount: invokes * 10,
                asset: "usdc".into(),
                pay_to: "0xplatform".into(),
                resource: format!("/accounts/{account_id}/credit"),
                network: "base".into(),
            })
        }

        async fn settle(
            &self,
            _account_id: &AccountId,
            _invokes: u64,
            header: &str,
        ) -> Result<String, AppError> {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                return Err(AppError::BadRequest("payment rejected".into()));
            }
            Ok(header.to_string())
        }
    }

    fn service(store: Arc<MemStore>, settler: Arc<Settler>) -> Accounts {
        Accounts::new(store, settler, "operator-secret", "http://api.test")
    }

    #[tokio::test]
    async fn stranger_cannot_issue_key() {
        let store = Arc::new(MemStore::new());
        let accounts = service(
            store.clone(),
            Arc::new(Settler {
                fail: false,
                calls: Mutex::new(0),
            }),
        );
        let created = accounts.create().await.expect("create");
        let err = accounts
            .issue_key(&created.account_id, None, None)
            .await
            .expect_err("stranger");
        assert!(matches!(err, AppError::Unauthorized), "{err}");
        assert_eq!(store.list_keys(&created.account_id).await.unwrap().len(), 1);

        let other = accounts.create().await.expect("other");
        let err = accounts
            .issue_key(&created.account_id, Some(&other.secret), None)
            .await
            .expect_err("other account");
        assert!(matches!(err, AppError::Unauthorized), "{err}");
        assert_eq!(store.list_keys(&created.account_id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn secret_is_not_in_the_list() {
        let store = Arc::new(MemStore::new());
        let accounts = service(
            store,
            Arc::new(Settler {
                fail: false,
                calls: Mutex::new(0),
            }),
        );
        let created = accounts.create().await.expect("create");
        let view = accounts.read(&created.secret).await.expect("read");
        let rendered = format!("{view:?}");
        assert!(!rendered.contains(created.secret.as_str()));
        assert!(!rendered.contains("secret_hash"));
        assert!(!rendered.contains("SecretHash"));
        assert_eq!(view.balance, 0);
        assert_eq!(view.keys.len(), 1);
        assert_eq!(view.keys[0].key_id, created.key_id);
    }

    #[tokio::test]
    async fn credit_is_not_applied_when_settle_fails() {
        let store = Arc::new(MemStore::new());
        let settler = Arc::new(Settler {
            fail: true,
            calls: Mutex::new(0),
        });
        let accounts = service(store.clone(), settler);
        let created = accounts.create().await.expect("create");
        let err = accounts
            .credit(&created.account_id, 3, Some("nonce-1"))
            .await
            .expect_err("settle");
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
        assert_eq!(*store.credits.lock().unwrap(), 0);
        assert_eq!(
            store
                .account(&created.account_id)
                .await
                .unwrap()
                .unwrap()
                .balance,
            0
        );
    }
}
