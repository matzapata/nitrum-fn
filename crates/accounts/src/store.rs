use std::collections::HashMap;

use application::ports::{AccountStore, CreditOutcome, DebitReceipt, StoreError};
use async_trait::async_trait;
use aws_sdk_dynamodb::error::SdkError;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::types::{AttributeValue, Put, TransactWriteItem, Update};
use aws_sdk_dynamodb::Client;
use domain::{Account, AccountId, KeyId, KeyMeta, KeyRecord, SecretHash};

const ACCOUNT_ID: &str = "account_id";
const BALANCE: &str = "balance";
const SECRET_HASH: &str = "secret_hash";
const KEY_ID: &str = "key_id";
const REVOKED: &str = "revoked";
const CAPPED: &str = "capped";
const CAP_REMAINING: &str = "cap_remaining";
const NONCE: &str = "nonce";
const INVOKES: &str = "invokes";
const KEYS_GSI: &str = "account_id_index";

/// Accounts (`account_id`), keys (`secret_hash` + `account_id` GSI), and
/// credit receipts (`nonce`). Debit and paid credit are conditional transactions.
pub struct DynamoAccountStore {
    client: Client,
    accounts: String,
    keys: String,
    receipts: String,
}

impl DynamoAccountStore {
    pub fn new(
        client: Client,
        accounts: impl Into<String>,
        keys: impl Into<String>,
        receipts: impl Into<String>,
    ) -> Self {
        Self {
            client,
            accounts: accounts.into(),
            keys: keys.into(),
            receipts: receipts.into(),
        }
    }
}

#[async_trait]
impl AccountStore for DynamoAccountStore {
    async fn create(&self, account: &Account, key: &KeyRecord) -> Result<(), StoreError> {
        let account_put = Put::builder()
            .table_name(&self.accounts)
            .item(ACCOUNT_ID, AttributeValue::S(account.id.to_hex()))
            .item(BALANCE, n(account.balance))
            .condition_expression("attribute_not_exists(#id)")
            .expression_attribute_names("#id", ACCOUNT_ID)
            .build()
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        let key_put = put_key(&self.keys, key)?;
        self.transact([
            TransactWriteItem::builder().put(account_put).build(),
            TransactWriteItem::builder().put(key_put).build(),
        ])
        .await
        .map_err(|err| {
            if transaction_cancelled(&err) {
                StoreError::Storage("account or key already exists".into())
            } else {
                StoreError::Storage(err.to_string())
            }
        })
    }

    async fn put_key(&self, key: &KeyRecord) -> Result<(), StoreError> {
        self.client
            .put_item()
            .table_name(&self.keys)
            .set_item(Some(key_item(key)))
            .condition_expression("attribute_not_exists(#h)")
            .expression_attribute_names("#h", SECRET_HASH)
            .send()
            .await
            .map_err(|err| {
                if err
                    .as_service_error()
                    .map(|e| e.is_conditional_check_failed_exception())
                    .unwrap_or(false)
                {
                    StoreError::Storage("key already exists".into())
                } else {
                    StoreError::Storage(err.to_string())
                }
            })?;
        Ok(())
    }

    async fn revoke(&self, account_id: &AccountId, key_id: &KeyId) -> Result<(), StoreError> {
        let Some(hash) = self.find_hash(account_id, key_id).await? else {
            return Err(StoreError::NotFound);
        };
        self.client
            .update_item()
            .table_name(&self.keys)
            .key(SECRET_HASH, AttributeValue::S(hash))
            .update_expression("SET #r = :t")
            .condition_expression("#a = :a")
            .expression_attribute_names("#r", REVOKED)
            .expression_attribute_names("#a", ACCOUNT_ID)
            .expression_attribute_values(":t", AttributeValue::Bool(true))
            .expression_attribute_values(":a", AttributeValue::S(account_id.to_hex()))
            .send()
            .await
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn key_by_hash(&self, hash: &SecretHash) -> Result<Option<KeyRecord>, StoreError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.keys)
            .key(SECRET_HASH, AttributeValue::S(hash.to_hex()))
            .send()
            .await
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        match out.item() {
            Some(item) => Ok(Some(key_from_item(item)?)),
            None => Ok(None),
        }
    }

    async fn account(&self, id: &AccountId) -> Result<Option<Account>, StoreError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.accounts)
            .key(ACCOUNT_ID, AttributeValue::S(id.to_hex()))
            .send()
            .await
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        match out.item() {
            Some(item) => Ok(Some(account_from_item(item)?)),
            None => Ok(None),
        }
    }

    async fn list_keys(&self, account_id: &AccountId) -> Result<Vec<KeyMeta>, StoreError> {
        let out = self
            .client
            .query()
            .table_name(&self.keys)
            .index_name(KEYS_GSI)
            .key_condition_expression("#a = :a")
            .expression_attribute_names("#a", ACCOUNT_ID)
            .expression_attribute_values(":a", AttributeValue::S(account_id.to_hex()))
            .send()
            .await
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        out.items()
            .iter()
            .map(|item| Ok(KeyMeta::from(&key_from_item(item)?)))
            .collect()
    }

    async fn debit(&self, secret_hash: &SecretHash, cost: u64) -> Result<DebitReceipt, StoreError> {
        if cost == 0 {
            return Err(StoreError::Storage(
                "debit cost must be a positive number of invoke credits".into(),
            ));
        }
        let Some(key) = self.key_by_hash(secret_hash).await? else {
            return Err(StoreError::Unauthorized);
        };
        if key.revoked {
            return Err(StoreError::Unauthorized);
        }
        let Some(account) = self.account(&key.account_id).await? else {
            return Err(StoreError::Storage("key has no account".into()));
        };
        if let Some(denied) = denial(&account, &key, cost) {
            return Err(denied);
        }

        let account_update = Update::builder()
            .table_name(&self.accounts)
            .key(ACCOUNT_ID, AttributeValue::S(account.id.to_hex()))
            .update_expression("SET balance = balance - :cost")
            .condition_expression("balance >= :cost")
            .expression_attribute_values(":cost", n(cost))
            .build()
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        let key_update = if key.capped {
            Update::builder()
                .table_name(&self.keys)
                .key(SECRET_HASH, AttributeValue::S(secret_hash.to_hex()))
                .update_expression("SET cap_remaining = cap_remaining - :cost")
                .condition_expression("#r = :f AND #c = :t AND cap_remaining >= :cost")
                .expression_attribute_names("#r", REVOKED)
                .expression_attribute_names("#c", CAPPED)
                .expression_attribute_values(":f", AttributeValue::Bool(false))
                .expression_attribute_values(":t", AttributeValue::Bool(true))
                .expression_attribute_values(":cost", n(cost))
                .build()
                .map_err(|e| StoreError::Storage(e.to_string()))?
        } else {
            Update::builder()
                .table_name(&self.keys)
                .key(SECRET_HASH, AttributeValue::S(secret_hash.to_hex()))
                .update_expression("SET #r = :f")
                .condition_expression("#r = :f AND #c = :f")
                .expression_attribute_names("#r", REVOKED)
                .expression_attribute_names("#c", CAPPED)
                .expression_attribute_values(":f", AttributeValue::Bool(false))
                .build()
                .map_err(|e| StoreError::Storage(e.to_string()))?
        };

        if let Err(err) = self
            .transact([
                TransactWriteItem::builder().update(account_update).build(),
                TransactWriteItem::builder().update(key_update).build(),
            ])
            .await
        {
            if !transaction_cancelled(&err) {
                return Err(StoreError::Storage(err.to_string()));
            }
            let key = self
                .key_by_hash(secret_hash)
                .await?
                .ok_or(StoreError::Unauthorized)?;
            if key.revoked {
                return Err(StoreError::Unauthorized);
            }
            let account = self
                .account(&key.account_id)
                .await?
                .ok_or_else(|| StoreError::Storage("key has no account".into()))?;
            if let Some(denied) = denial(&account, &key, cost) {
                return Err(denied);
            }
            return Err(StoreError::Storage(
                "debit condition failed without an admission reason".into(),
            ));
        }

        Ok(DebitReceipt {
            secret_hash: secret_hash.clone(),
            account_id: account.id,
            cost,
            capped: key.capped,
        })
    }

    async fn refund(&self, receipt: &DebitReceipt) -> Result<(), StoreError> {
        let account_update = Update::builder()
            .table_name(&self.accounts)
            .key(ACCOUNT_ID, AttributeValue::S(receipt.account_id.to_hex()))
            .update_expression("SET balance = balance + :cost")
            .condition_expression("attribute_exists(#id)")
            .expression_attribute_names("#id", ACCOUNT_ID)
            .expression_attribute_values(":cost", n(receipt.cost))
            .build()
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        if !receipt.capped {
            self.transact([TransactWriteItem::builder().update(account_update).build()])
                .await
                .map_err(|e| StoreError::Storage(e.to_string()))?;
            return Ok(());
        }
        let key_update = Update::builder()
            .table_name(&self.keys)
            .key(SECRET_HASH, AttributeValue::S(receipt.secret_hash.to_hex()))
            .update_expression("SET cap_remaining = cap_remaining + :cost")
            .condition_expression("#c = :t")
            .expression_attribute_names("#c", CAPPED)
            .expression_attribute_values(":t", AttributeValue::Bool(true))
            .expression_attribute_values(":cost", n(receipt.cost))
            .build()
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        self.transact([
            TransactWriteItem::builder().update(account_update).build(),
            TransactWriteItem::builder().update(key_update).build(),
        ])
        .await
        .map_err(|e| StoreError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn credit(
        &self,
        account_id: &AccountId,
        invokes: u64,
        nonce: &str,
    ) -> Result<CreditOutcome, StoreError> {
        if invokes == 0 || nonce.is_empty() {
            return Err(StoreError::Storage(
                "credit requires a positive invoke count and a nonce".into(),
            ));
        }
        let account_update = Update::builder()
            .table_name(&self.accounts)
            .key(ACCOUNT_ID, AttributeValue::S(account_id.to_hex()))
            .update_expression("SET balance = balance + :n")
            .condition_expression("attribute_exists(#id)")
            .expression_attribute_names("#id", ACCOUNT_ID)
            .expression_attribute_values(":n", n(invokes))
            .build()
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        let receipt_put = Put::builder()
            .table_name(&self.receipts)
            .item(NONCE, AttributeValue::S(nonce.to_string()))
            .item(ACCOUNT_ID, AttributeValue::S(account_id.to_hex()))
            .item(INVOKES, n(invokes))
            .condition_expression("attribute_not_exists(#n)")
            .expression_attribute_names("#n", NONCE)
            .build()
            .map_err(|e| StoreError::Storage(e.to_string()))?;

        match self
            .transact([
                TransactWriteItem::builder().update(account_update).build(),
                TransactWriteItem::builder().put(receipt_put).build(),
            ])
            .await
        {
            Ok(()) => {
                let balance = self.balance_of(account_id).await?;
                Ok(CreditOutcome::Applied { balance })
            }
            Err(err) if transaction_cancelled(&err) => {
                if self.receipt_exists(nonce).await? {
                    let balance = self.balance_of(account_id).await?;
                    return Ok(CreditOutcome::Replay { balance });
                }
                Err(StoreError::NotFound)
            }
            Err(err) => Err(StoreError::Storage(err.to_string())),
        }
    }
}

impl DynamoAccountStore {
    async fn transact<const N: usize>(
        &self,
        items: [TransactWriteItem; N],
    ) -> Result<(), SdkError<TransactWriteItemsError>> {
        let mut req = self.client.transact_write_items();
        for item in items {
            req = req.transact_items(item);
        }
        req.send().await.map(|_| ())
    }

    async fn find_hash(
        &self,
        account_id: &AccountId,
        key_id: &KeyId,
    ) -> Result<Option<String>, StoreError> {
        let out = self
            .client
            .query()
            .table_name(&self.keys)
            .index_name(KEYS_GSI)
            .key_condition_expression("#a = :a")
            .filter_expression("#k = :k")
            .expression_attribute_names("#a", ACCOUNT_ID)
            .expression_attribute_names("#k", KEY_ID)
            .expression_attribute_values(":a", AttributeValue::S(account_id.to_hex()))
            .expression_attribute_values(":k", AttributeValue::S(key_id.to_hex()))
            .send()
            .await
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        Ok(out
            .items()
            .iter()
            .find_map(|item| item.get(SECRET_HASH).and_then(|v| v.as_s().ok()))
            .map(|s| s.to_string()))
    }

    async fn balance_of(&self, account_id: &AccountId) -> Result<u64, StoreError> {
        self.account(account_id)
            .await?
            .map(|a| a.balance)
            .ok_or(StoreError::NotFound)
    }

    async fn receipt_exists(&self, nonce: &str) -> Result<bool, StoreError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.receipts)
            .key(NONCE, AttributeValue::S(nonce.to_string()))
            .send()
            .await
            .map_err(|e| StoreError::Storage(e.to_string()))?;
        Ok(out.item().is_some())
    }
}

fn denial(account: &Account, key: &KeyRecord, cost: u64) -> Option<StoreError> {
    let cap_short = key.capped && key.cap_remaining.unwrap_or(0) < cost;
    if account.balance < cost || cap_short {
        Some(StoreError::Insufficient {
            account_id: account.id.clone(),
            balance: account.balance,
            cost,
            spend_cap_remaining: key.capped.then_some(key.cap_remaining.unwrap_or(0)),
        })
    } else {
        None
    }
}

fn transaction_cancelled(err: &SdkError<TransactWriteItemsError>) -> bool {
    err.as_service_error()
        .map(|e| e.is_transaction_canceled_exception())
        .unwrap_or(false)
}

fn n(value: u64) -> AttributeValue {
    AttributeValue::N(value.to_string())
}

fn put_key(table: &str, key: &KeyRecord) -> Result<Put, StoreError> {
    Put::builder()
        .table_name(table)
        .set_item(Some(key_item(key)))
        .condition_expression("attribute_not_exists(#h)")
        .expression_attribute_names("#h", SECRET_HASH)
        .build()
        .map_err(|e| StoreError::Storage(e.to_string()))
}

fn key_item(key: &KeyRecord) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();
    item.insert(
        SECRET_HASH.to_string(),
        AttributeValue::S(key.secret_hash.to_hex()),
    );
    item.insert(
        ACCOUNT_ID.to_string(),
        AttributeValue::S(key.account_id.to_hex()),
    );
    item.insert(KEY_ID.to_string(), AttributeValue::S(key.key_id.to_hex()));
    item.insert(REVOKED.to_string(), AttributeValue::Bool(key.revoked));
    item.insert(CAPPED.to_string(), AttributeValue::Bool(key.capped));
    if let Some(remaining) = key.cap_remaining {
        item.insert(CAP_REMAINING.to_string(), n(remaining));
    }
    item
}

fn key_from_item(item: &HashMap<String, AttributeValue>) -> Result<KeyRecord, StoreError> {
    let capped = attr_bool(item, CAPPED)?;
    let cap_remaining = if capped {
        Some(attr_u64(item, CAP_REMAINING).unwrap_or(0))
    } else {
        None
    };
    Ok(KeyRecord {
        secret_hash: SecretHash::from_hex(&attr_s(item, SECRET_HASH)?)
            .map_err(|e| StoreError::Storage(e.to_string()))?,
        account_id: AccountId::from_hex(&attr_s(item, ACCOUNT_ID)?)
            .map_err(|e| StoreError::Storage(e.to_string()))?,
        key_id: KeyId::from_hex(&attr_s(item, KEY_ID)?)
            .map_err(|e| StoreError::Storage(e.to_string()))?,
        revoked: attr_bool(item, REVOKED)?,
        capped,
        cap_remaining,
    })
}

fn account_from_item(item: &HashMap<String, AttributeValue>) -> Result<Account, StoreError> {
    Ok(Account {
        id: AccountId::from_hex(&attr_s(item, ACCOUNT_ID)?)
            .map_err(|e| StoreError::Storage(e.to_string()))?,
        balance: attr_u64(item, BALANCE)?,
    })
}

fn attr_s(item: &HashMap<String, AttributeValue>, key: &str) -> Result<String, StoreError> {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .cloned()
        .ok_or_else(|| StoreError::Storage(format!("item missing {key}")))
}

fn attr_bool(item: &HashMap<String, AttributeValue>, key: &str) -> Result<bool, StoreError> {
    item.get(key)
        .and_then(|v| v.as_bool().ok())
        .copied()
        .ok_or_else(|| StoreError::Storage(format!("item missing {key}")))
}

fn attr_u64(item: &HashMap<String, AttributeValue>, key: &str) -> Result<u64, StoreError> {
    item.get(key)
        .and_then(|v| v.as_n().ok())
        .ok_or_else(|| StoreError::Storage(format!("item missing {key}")))?
        .parse()
        .map_err(|_| StoreError::Storage(format!("item {key} is not an integer")))
}
