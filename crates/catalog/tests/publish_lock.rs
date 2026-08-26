//! Floci DynamoDB publish-lock adapter. Requires `NITRUM_FN_CATALOG__ENDPOINT`.

mod common;

use application::error::AppError;
use application::ports::PublishLock;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
    ScalarAttributeType, TimeToLiveSpecification,
};
use aws_sdk_dynamodb::Client;
use catalog::DynamoDbPublishLock;
use domain::{ContentHash, FunctionId};

async fn ensure_table(client: &Client, table: &str) {
    if client
        .describe_table()
        .table_name(table)
        .send()
        .await
        .is_ok()
    {
        return;
    }
    let _ = client
        .create_table()
        .table_name(table)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("fn_id")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("fn_id attr"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("fn_id")
                .key_type(KeyType::Hash)
                .build()
                .expect("hash key"),
        )
        .billing_mode(BillingMode::PayPerRequest)
        .send()
        .await;
    for _ in 0..30 {
        if client
            .describe_table()
            .table_name(table)
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    client
        .update_time_to_live()
        .table_name(table)
        .time_to_live_specification(
            TimeToLiveSpecification::builder()
                .enabled(true)
                .attribute_name("expires_at")
                .build()
                .expect("ttl"),
        )
        .send()
        .await
        .ok();
}

async fn store() -> (DynamoDbPublishLock, Client, String) {
    let client = common::ddb_client().await;
    let table = common::unique("nitrum-fn-publish-lock");
    ensure_table(&client, &table).await;
    (
        DynamoDbPublishLock::new(client.clone(), table.clone()),
        client,
        table,
    )
}

#[tokio::test]
async fn acquire_twice_conflicts_until_release() {
    let (store, _, _) = store().await;
    let fn_id = FunctionId::new("echo").unwrap();
    let hash_a = ContentHash::from_bytes(b"one");
    let hash_b = ContentHash::from_bytes(b"two");

    store.acquire(&fn_id, &hash_a, 1).await.expect("first");
    let err = store
        .acquire(&fn_id, &hash_b, 2)
        .await
        .expect_err("conflict");
    assert!(matches!(err, AppError::Conflict(_)), "{err}");

    store.release(&fn_id, &hash_a).await.expect("release");
    store.acquire(&fn_id, &hash_b, 3).await.expect("second");
}

#[tokio::test]
async fn expired_row_can_be_stolen() {
    let (store, client, table) = store().await;
    let fn_id = FunctionId::new("echo").unwrap();
    let hash_a = ContentHash::from_bytes(b"one");
    let hash_b = ContentHash::from_bytes(b"two");

    client
        .put_item()
        .table_name(&table)
        .item("fn_id", AttributeValue::S("echo".into()))
        .item("content_hash", AttributeValue::S(hash_a.to_hex()))
        .item("queued_at_ms", AttributeValue::N("1".into()))
        .item("expires_at", AttributeValue::N("1".into()))
        .send()
        .await
        .expect("seed expired");

    store.acquire(&fn_id, &hash_b, 2).await.expect("steal");
}

#[tokio::test]
async fn release_wrong_hash_is_noop() {
    let (store, _, _) = store().await;
    let fn_id = FunctionId::new("echo").unwrap();
    let hash_a = ContentHash::from_bytes(b"one");
    let hash_b = ContentHash::from_bytes(b"two");

    store.acquire(&fn_id, &hash_a, 1).await.expect("acquire");
    store
        .release(&fn_id, &hash_b)
        .await
        .expect("wrong hash noop");
    let err = store
        .acquire(&fn_id, &hash_b, 2)
        .await
        .expect_err("still held");
    assert!(matches!(err, AppError::Conflict(_)), "{err}");
}

#[tokio::test]
async fn different_functions_do_not_collide() {
    let (store, _, _) = store().await;
    let echo = FunctionId::new("echo").unwrap();
    let other = FunctionId::new("other").unwrap();
    let hash = ContentHash::from_bytes(b"one");

    store.acquire(&echo, &hash, 1).await.expect("echo");
    store.acquire(&other, &hash, 1).await.expect("other");
}
