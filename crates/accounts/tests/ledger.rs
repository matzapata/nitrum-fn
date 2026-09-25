//! Floci DynamoDB account ledger. Requires `NITRUM_FN_CATALOG__ENDPOINT`.

mod common;

use accounts::DynamoAccountStore;
use application::ports::{AccountStore, CreditOutcome, StoreError};
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, GlobalSecondaryIndex, KeySchemaElement, KeyType, Projection,
    ProjectionType, ScalarAttributeType,
};
use aws_sdk_dynamodb::Client;
use domain::{Account, KeyMeta, NewKey};

async fn ensure_hash_table(client: &Client, table: &str, hash_key: &str) {
    if client
        .describe_table()
        .table_name(table)
        .send()
        .await
        .is_ok()
    {
        return;
    }
    client
        .create_table()
        .table_name(table)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name(hash_key)
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("attr"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name(hash_key)
                .key_type(KeyType::Hash)
                .build()
                .expect("hash"),
        )
        .billing_mode(BillingMode::PayPerRequest)
        .send()
        .await
        .expect("create table");
}

async fn ensure_keys_table(client: &Client, table: &str) {
    if client
        .describe_table()
        .table_name(table)
        .send()
        .await
        .is_ok()
    {
        return;
    }
    client
        .create_table()
        .table_name(table)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("secret_hash")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("secret_hash"),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("account_id")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("account_id"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("secret_hash")
                .key_type(KeyType::Hash)
                .build()
                .expect("hash"),
        )
        .global_secondary_indexes(
            GlobalSecondaryIndex::builder()
                .index_name("account_id_index")
                .key_schema(
                    KeySchemaElement::builder()
                        .attribute_name("account_id")
                        .key_type(KeyType::Hash)
                        .build()
                        .expect("gsi hash"),
                )
                .projection(
                    Projection::builder()
                        .projection_type(ProjectionType::All)
                        .build(),
                )
                .build()
                .expect("gsi"),
        )
        .billing_mode(BillingMode::PayPerRequest)
        .send()
        .await
        .expect("create keys table");
}

async fn store() -> DynamoAccountStore {
    let client = common::ddb_client().await;
    let accounts = common::unique("nitrum-fn-accounts");
    let keys = common::unique("nitrum-fn-keys");
    let receipts = common::unique("nitrum-fn-credit-receipts");
    ensure_hash_table(&client, &accounts, "account_id").await;
    ensure_keys_table(&client, &keys).await;
    ensure_hash_table(&client, &receipts, "nonce").await;
    DynamoAccountStore::new(client, accounts, keys, receipts)
}

fn cap_of(keys: &[KeyMeta]) -> Option<u64> {
    keys.iter().find_map(|k| k.cap_remaining)
}

#[tokio::test]
async fn credit_does_not_refill_the_cap() {
    let store = store().await;
    let account = Account::open().unwrap();
    let issued = NewKey::issue(account.id.clone(), Some(5)).unwrap();
    store
        .create(&account, &issued.record)
        .await
        .expect("create");
    store
        .credit(&account.id, 5, &common::unique("fund"))
        .await
        .expect("fund");
    store
        .debit(&issued.record.secret_hash, 2)
        .await
        .expect("debit");

    let outcome = store
        .credit(&account.id, 10, "nonce-cap")
        .await
        .expect("credit");
    assert_eq!(outcome, CreditOutcome::Applied { balance: 13 });

    let keys = store.list_keys(&account.id).await.expect("list");
    assert_eq!(cap_of(&keys), Some(3));
    let rendered = format!("{keys:?}");
    assert!(!rendered.contains(issued.secret.as_str()));
    assert!(!rendered.contains("secret_hash"));
}

#[tokio::test]
async fn revoke_leaves_balance_unchanged() {
    let store = store().await;
    let account = Account::open().unwrap();
    let issued = NewKey::issue(account.id.clone(), None).unwrap();
    store
        .create(&account, &issued.record)
        .await
        .expect("create");
    store
        .credit(&account.id, 7, &common::unique("fund"))
        .await
        .expect("fund");
    store
        .revoke(&account.id, &issued.record.key_id)
        .await
        .expect("revoke");
    let after = store.account(&account.id).await.expect("account").unwrap();
    assert_eq!(after.balance, 7);
    let err = store
        .debit(&issued.record.secret_hash, 1)
        .await
        .expect_err("revoked");
    assert!(matches!(err, StoreError::Unauthorized), "{err:?}");
    let still = store.account(&account.id).await.expect("account").unwrap();
    assert_eq!(still.balance, 7);
}

#[tokio::test]
async fn replayed_receipt_does_not_add_invokes_again() {
    let store = store().await;
    let account = Account::open().unwrap();
    let issued = NewKey::issue(account.id.clone(), None).unwrap();
    store
        .create(&account, &issued.record)
        .await
        .expect("create");
    let first = store
        .credit(&account.id, 4, "nonce-once")
        .await
        .expect("credit");
    assert_eq!(first, CreditOutcome::Applied { balance: 4 });
    let second = store
        .credit(&account.id, 4, "nonce-once")
        .await
        .expect("replay");
    assert_eq!(second, CreditOutcome::Replay { balance: 4 });
    let account = store.account(&account.id).await.unwrap().unwrap();
    assert_eq!(account.balance, 4);
}

#[tokio::test]
async fn two_concurrent_one_credit_debits_when_balance_is_one() {
    let store = std::sync::Arc::new(store().await);
    let account = Account::open().unwrap();
    let issued = NewKey::issue(account.id.clone(), None).unwrap();
    store
        .create(&account, &issued.record)
        .await
        .expect("create");
    store
        .credit(&account.id, 1, &common::unique("fund"))
        .await
        .expect("fund");

    let hash = issued.record.secret_hash.clone();
    let a = {
        let store = store.clone();
        let hash = hash.clone();
        tokio::spawn(async move { store.debit(&hash, 1).await })
    };
    let b = {
        let store = store.clone();
        tokio::spawn(async move { store.debit(&hash, 1).await })
    };
    let ra = a.await.expect("join");
    let rb = b.await.expect("join");
    let oks = [&ra, &rb].iter().filter(|r| r.is_ok()).count();
    let denied = [&ra, &rb]
        .iter()
        .filter(|r| matches!(r, Err(StoreError::Insufficient { .. })))
        .count();
    assert_eq!(oks, 1, "{ra:?} {rb:?}");
    assert_eq!(denied, 1, "{ra:?} {rb:?}");
    let after = store.account(&account.id).await.unwrap().unwrap();
    assert_eq!(after.balance, 0);
}
