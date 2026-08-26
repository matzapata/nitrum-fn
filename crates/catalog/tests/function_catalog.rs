//! Floci DynamoDB function-catalog adapter. Requires `NITRUM_FN_CATALOG__ENDPOINT`.

mod common;

use application::error::AppError;
use application::ports::FunctionCatalog;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType,
};
use aws_sdk_dynamodb::Client;
use catalog::DynamoDbFunctionCatalog;
use domain::{ContentHash, FunctionId, VersionLabel};

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
    client
        .create_table()
        .table_name(table)
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("fn_id")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("fn_id attr"),
        )
        .attribute_definitions(
            AttributeDefinition::builder()
                .attribute_name("label")
                .attribute_type(ScalarAttributeType::S)
                .build()
                .expect("label attr"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("fn_id")
                .key_type(KeyType::Hash)
                .build()
                .expect("hash key"),
        )
        .key_schema(
            KeySchemaElement::builder()
                .attribute_name("label")
                .key_type(KeyType::Range)
                .build()
                .expect("range key"),
        )
        .billing_mode(BillingMode::PayPerRequest)
        .send()
        .await
        .expect("create table");
}

#[tokio::test]
async fn upsert_resolve_list() {
    let client = common::ddb_client().await;
    let table = common::unique("nitrum-fn-catalog");
    ensure_table(&client, &table).await;

    let catalog = DynamoDbFunctionCatalog::new(client, table);
    let id = FunctionId::new("echo").unwrap();
    let label = VersionLabel::latest();
    let hash = ContentHash::from_bytes(b"hello");

    catalog
        .upsert(&id, &label, hash.clone(), 1)
        .await
        .expect("upsert");
    let resolved = catalog.resolve(&id, &label).await.expect("resolve");
    assert_eq!(resolved.content_hash, hash);

    let listed = catalog.list().await.expect("list");
    assert!(listed.iter().any(|v| v.id == id && v.content_hash == hash));
}

#[tokio::test]
async fn missing_is_not_found() {
    let client = common::ddb_client().await;
    let table = common::unique("nitrum-fn-catalog-miss");
    ensure_table(&client, &table).await;

    let catalog = DynamoDbFunctionCatalog::new(client, table);
    let err = catalog
        .resolve(
            &FunctionId::new("missing").unwrap(),
            &VersionLabel::latest(),
        )
        .await
        .expect_err("missing");
    assert!(matches!(err, AppError::NotFound(_)), "{err}");
}

#[tokio::test]
async fn stale_upsert_does_not_clobber() {
    let client = common::ddb_client().await;
    let table = common::unique("nitrum-fn-catalog-stale");
    ensure_table(&client, &table).await;

    let catalog = DynamoDbFunctionCatalog::new(client, table);
    let id = FunctionId::new("echo").unwrap();
    let label = VersionLabel::latest();
    let old = ContentHash::from_bytes(b"old");
    let new = ContentHash::from_bytes(b"new");

    assert!(catalog
        .upsert(&id, &label, old.clone(), 100)
        .await
        .expect("first"));
    assert!(catalog
        .upsert(&id, &label, new.clone(), 200)
        .await
        .expect("newer"));
    assert!(!catalog
        .upsert(&id, &label, old.clone(), 150)
        .await
        .expect("stale"));

    let resolved = catalog.resolve(&id, &label).await.expect("resolve");
    assert_eq!(resolved.content_hash, new);
}
