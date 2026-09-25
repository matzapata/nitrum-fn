use std::collections::HashMap;
use std::sync::Mutex;

use application::ports::{AccountStore, CreditOutcome, DebitReceipt, StoreError};
use async_trait::async_trait;
use domain::{Account, AccountId, KeyId, KeyMeta, KeyRecord, SecretHash};

#[derive(Default)]
struct Ledger {
    accounts: HashMap<String, Account>,
    keys: HashMap<String, KeyRecord>,
    receipts: HashMap<String, u64>,
}

pub struct MemLedger {
    inner: Mutex<Ledger>,
}

impl MemLedger {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Ledger::default()),
        }
    }
}

#[async_trait]
impl AccountStore for MemLedger {
    async fn create(&self, account: &Account, key: &KeyRecord) -> Result<(), StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.accounts.insert(account.id.to_hex(), account.clone());
        g.keys.insert(key.secret_hash.to_hex(), key.clone());
        Ok(())
    }

    async fn put_key(&self, key: &KeyRecord) -> Result<(), StoreError> {
        self.inner
            .lock()
            .unwrap()
            .keys
            .insert(key.secret_hash.to_hex(), key.clone());
        Ok(())
    }

    async fn revoke(&self, account_id: &AccountId, key_id: &KeyId) -> Result<(), StoreError> {
        let mut g = self.inner.lock().unwrap();
        let key = g
            .keys
            .values_mut()
            .find(|k| &k.account_id == account_id && &k.key_id == key_id)
            .ok_or(StoreError::NotFound)?;
        key.revoked = true;
        Ok(())
    }

    async fn key_by_hash(&self, hash: &SecretHash) -> Result<Option<KeyRecord>, StoreError> {
        Ok(self.inner.lock().unwrap().keys.get(&hash.to_hex()).cloned())
    }

    async fn account(&self, id: &AccountId) -> Result<Option<Account>, StoreError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .accounts
            .get(&id.to_hex())
            .cloned())
    }

    async fn list_keys(&self, account_id: &AccountId) -> Result<Vec<KeyMeta>, StoreError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .keys
            .values()
            .filter(|k| &k.account_id == account_id)
            .map(KeyMeta::from)
            .collect())
    }

    async fn debit(&self, secret_hash: &SecretHash, cost: u64) -> Result<DebitReceipt, StoreError> {
        let mut g = self.inner.lock().unwrap();
        let key = g
            .keys
            .get(&secret_hash.to_hex())
            .cloned()
            .ok_or(StoreError::Unauthorized)?;
        if key.revoked {
            return Err(StoreError::Unauthorized);
        }
        let account = g
            .accounts
            .get_mut(&key.account_id.to_hex())
            .ok_or(StoreError::NotFound)?;
        let cap_short = key.capped && key.cap_remaining.unwrap_or(0) < cost;
        if account.balance < cost || cap_short {
            return Err(StoreError::Insufficient {
                account_id: account.id.clone(),
                balance: account.balance,
                cost,
                spend_cap_remaining: key.capped.then_some(key.cap_remaining.unwrap_or(0)),
            });
        }
        account.balance -= cost;
        let account_id = account.id.clone();
        if key.capped {
            let stored = g.keys.get_mut(&secret_hash.to_hex()).expect("key");
            stored.cap_remaining = Some(stored.cap_remaining.unwrap_or(0) - cost);
        }
        Ok(DebitReceipt {
            secret_hash: secret_hash.clone(),
            account_id,
            cost,
            capped: key.capped,
        })
    }

    async fn refund(&self, receipt: &DebitReceipt) -> Result<(), StoreError> {
        let mut g = self.inner.lock().unwrap();
        let account = g
            .accounts
            .get_mut(&receipt.account_id.to_hex())
            .ok_or(StoreError::NotFound)?;
        account.balance += receipt.cost;
        if receipt.capped {
            let key = g
                .keys
                .get_mut(&receipt.secret_hash.to_hex())
                .ok_or(StoreError::NotFound)?;
            key.cap_remaining = Some(key.cap_remaining.unwrap_or(0) + receipt.cost);
        }
        Ok(())
    }

    async fn credit(
        &self,
        account_id: &AccountId,
        invokes: u64,
        nonce: &str,
    ) -> Result<CreditOutcome, StoreError> {
        let mut g = self.inner.lock().unwrap();
        if g.receipts.contains_key(nonce) {
            let balance = g
                .accounts
                .get(&account_id.to_hex())
                .ok_or(StoreError::NotFound)?
                .balance;
            return Ok(CreditOutcome::Replay { balance });
        }
        let account = g
            .accounts
            .get_mut(&account_id.to_hex())
            .ok_or(StoreError::NotFound)?;
        account.balance += invokes;
        let balance = account.balance;
        g.receipts.insert(nonce.to_string(), invokes);
        Ok(CreditOutcome::Applied { balance })
    }
}
