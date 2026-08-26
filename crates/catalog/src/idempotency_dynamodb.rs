use std::collections::HashMap;

use application::error::AppError;
use application::ports::{
    evaluate_claim, IdempotencyClaim, IdempotencyRecord, IdempotencyStatus, PublishIdempotency,
};
use async_trait::async_trait;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
    ScalarAttributeType, TableStatus, TimeToLiveSpecification, TimeToLiveStatus,
};
use aws_sdk_dynamodb::Client;
use domain::{ContentHash, FunctionId, IdempotencyKey};

use crate::idempotency::{is_live, storage_key, unix_now, TTL_SECS};

const ATTR_KEY: &str = "idempotency_key";
const ATTR_FN_ID: &str = "function_id";
const ATTR_HASH: &str = "content_hash";
const ATTR_WASM_BYTES: &str = "wasm_bytes";
const ATTR_STATUS: &str = "status";
const ATTR_EXPIRES_AT: &str = "expires_at";
const STATUS_PENDING: &str = "pending";
const STATUS_COMPLETED: &str = "completed";

/// DynamoDB items: `{function}#{key}` → function + content hash + status (24h TTL).
pub struct DynamoDbPublishIdempotency {
    client: Client,
    table: String,
}

impl DynamoDbPublishIdempotency {
    pub fn new(client: Client, table: impl Into<String>) -> Self {
        Self {
            client,
            table: table.into(),
        }
    }

    pub async fn ensure_table(client: &Client, table: &str) -> Result<(), AppError> {
        if client
            .describe_table()
            .table_name(table)
            .send()
            .await
            .is_ok()
        {
            wait_table_active(client, table).await?;
            return enable_ttl(client, table).await;
        }

        client
            .create_table()
            .table_name(table)
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name(ATTR_KEY)
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .map_err(|e| AppError::Storage(e.to_string()))?,
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name(ATTR_KEY)
                    .key_type(KeyType::Hash)
                    .build()
                    .map_err(|e| AppError::Storage(e.to_string()))?,
            )
            .billing_mode(BillingMode::PayPerRequest)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;

        wait_table_active(client, table).await?;
        enable_ttl(client, table).await
    }

    async fn load(
        &self,
        function: &FunctionId,
        key: &IdempotencyKey,
    ) -> Result<Option<IdempotencyRecord>, AppError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table)
            .key(ATTR_KEY, AttributeValue::S(storage_key(function, key)))
            .consistent_read(true)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;

        let Some(item) = out.item() else {
            return Ok(None);
        };
        if let Ok(expires_at) = attr_n(item, ATTR_EXPIRES_AT) {
            if !is_live(expires_at, unix_now()) {
                return Ok(None);
            }
        }
        Ok(Some(record_from_item(item)?))
    }

    async fn insert_pending(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<bool, AppError> {
        let expires_at = unix_now().saturating_add(TTL_SECS).to_string();
        let now = unix_now().to_string();
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(
                ATTR_KEY,
                AttributeValue::S(storage_key(&record.function, key)),
            )
            .item(
                ATTR_FN_ID,
                AttributeValue::S(record.function.as_str().to_string()),
            )
            .item(ATTR_HASH, AttributeValue::S(record.content_hash.to_hex()))
            .item(
                ATTR_WASM_BYTES,
                AttributeValue::N(record.wasm_bytes.to_string()),
            )
            .item(ATTR_STATUS, AttributeValue::S(STATUS_PENDING.into()))
            .item(ATTR_EXPIRES_AT, AttributeValue::N(expires_at))
            .condition_expression("attribute_not_exists(#k) OR #ttl <= :now")
            .expression_attribute_names("#k", ATTR_KEY)
            .expression_attribute_names("#ttl", ATTR_EXPIRES_AT)
            .expression_attribute_values(":now", AttributeValue::N(now))
            .send()
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(err)
                if err
                    .as_service_error()
                    .map(|e| e.is_conditional_check_failed_exception())
                    .unwrap_or(false) =>
            {
                Ok(false)
            }
            Err(err) => Err(AppError::Storage(err.to_string())),
        }
    }
}

async fn wait_table_active(client: &Client, table: &str) -> Result<(), AppError> {
    for _ in 0..50 {
        let desc = client
            .describe_table()
            .table_name(table)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        if matches!(
            desc.table().and_then(|t| t.table_status()),
            Some(&TableStatus::Active)
        ) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Err(AppError::Storage(format!(
        "DynamoDB table {table} did not become ACTIVE"
    )))
}

async fn enable_ttl(client: &Client, table: &str) -> Result<(), AppError> {
    let desc = client
        .describe_time_to_live()
        .table_name(table)
        .send()
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?;
    if matches!(
        desc.time_to_live_description()
            .and_then(|d| d.time_to_live_status()),
        Some(&TimeToLiveStatus::Enabled) | Some(&TimeToLiveStatus::Enabling)
    ) {
        return Ok(());
    }

    client
        .update_time_to_live()
        .table_name(table)
        .time_to_live_specification(
            TimeToLiveSpecification::builder()
                .enabled(true)
                .attribute_name(ATTR_EXPIRES_AT)
                .build()
                .map_err(|e| AppError::Storage(e.to_string()))?,
        )
        .send()
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}

fn attr_s<'a>(item: &'a HashMap<String, AttributeValue>, key: &str) -> Result<&'a str, AppError> {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .map(String::as_str)
        .ok_or_else(|| AppError::Storage(format!("idempotency item missing {key}")))
}

fn attr_n(item: &HashMap<String, AttributeValue>, key: &str) -> Result<u64, AppError> {
    item.get(key)
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::Storage(format!("idempotency item missing {key}")))
}

fn record_from_item(item: &HashMap<String, AttributeValue>) -> Result<IdempotencyRecord, AppError> {
    let function = FunctionId::new(attr_s(item, ATTR_FN_ID)?).map_err(AppError::from)?;
    let content_hash = ContentHash::from_hex(attr_s(item, ATTR_HASH)?).map_err(AppError::from)?;
    let wasm_bytes = attr_n(item, ATTR_WASM_BYTES)? as usize;
    let status = match attr_s(item, ATTR_STATUS).unwrap_or(STATUS_COMPLETED) {
        STATUS_PENDING => IdempotencyStatus::Pending,
        _ => IdempotencyStatus::Completed,
    };
    Ok(IdempotencyRecord {
        function,
        content_hash,
        wasm_bytes,
        status,
    })
}

#[async_trait]
impl PublishIdempotency for DynamoDbPublishIdempotency {
    async fn claim(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<IdempotencyClaim, AppError> {
        for _ in 0..3 {
            if let Some(existing) = self.load(&record.function, key).await? {
                return evaluate_claim(&existing, record);
            }
            if self.insert_pending(key, record).await? {
                return Ok(IdempotencyClaim::Proceed);
            }
        }
        Err(AppError::Storage(
            "idempotency claim raced; retry the request".into(),
        ))
    }

    async fn complete(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<(), AppError> {
        let expires_at = unix_now().saturating_add(TTL_SECS).to_string();
        let now = unix_now().to_string();
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(
                ATTR_KEY,
                AttributeValue::S(storage_key(&record.function, key)),
            )
            .item(
                ATTR_FN_ID,
                AttributeValue::S(record.function.as_str().to_string()),
            )
            .item(ATTR_HASH, AttributeValue::S(record.content_hash.to_hex()))
            .item(
                ATTR_WASM_BYTES,
                AttributeValue::N(record.wasm_bytes.to_string()),
            )
            .item(ATTR_STATUS, AttributeValue::S(STATUS_COMPLETED.into()))
            .item(ATTR_EXPIRES_AT, AttributeValue::N(expires_at))
            .condition_expression(
                "attribute_not_exists(#k) OR #ttl <= :now OR (#h = :h AND #f = :f)",
            )
            .expression_attribute_names("#k", ATTR_KEY)
            .expression_attribute_names("#ttl", ATTR_EXPIRES_AT)
            .expression_attribute_names("#h", ATTR_HASH)
            .expression_attribute_names("#f", ATTR_FN_ID)
            .expression_attribute_values(":now", AttributeValue::N(now))
            .expression_attribute_values(":h", AttributeValue::S(record.content_hash.to_hex()))
            .expression_attribute_values(
                ":f",
                AttributeValue::S(record.function.as_str().to_string()),
            )
            .send()
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(err)
                if err
                    .as_service_error()
                    .map(|e| e.is_conditional_check_failed_exception())
                    .unwrap_or(false) =>
            {
                Err(AppError::Storage(
                    "idempotency complete payload mismatch".into(),
                ))
            }
            Err(err) => Err(AppError::Storage(err.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_dynamodb::config::Builder as DdbConfigBuilder;
    use domain::FunctionId;
    use std::time::{SystemTime, UNIX_EPOCH};

    async fn ddb_client() -> Option<Client> {
        let endpoint = std::env::var("NITRUM_FN_DDB_ENDPOINT").ok()?;
        let sdk = crate::load_test_aws_config().await;
        let conf = DdbConfigBuilder::from(&sdk).endpoint_url(endpoint).build();
        Some(Client::from_conf(conf))
    }

    async fn store() -> Option<(DynamoDbPublishIdempotency, Client, String)> {
        let client = ddb_client().await?;
        let table = format!(
            "nitrum-fn-idem-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        DynamoDbPublishIdempotency::ensure_table(&client, &table)
            .await
            .expect("table");
        Some((
            DynamoDbPublishIdempotency::new(client.clone(), table.clone()),
            client,
            table,
        ))
    }

    fn rec(name: &str, wasm: &[u8]) -> IdempotencyRecord {
        IdempotencyRecord {
            function: FunctionId::new(name).unwrap(),
            content_hash: ContentHash::from_bytes(wasm),
            wasm_bytes: wasm.len(),
            status: IdempotencyStatus::Pending,
        }
    }

    #[tokio::test]
    async fn claim_complete_replay_and_conflict() {
        let Some((store, _, _)) = store().await else {
            eprintln!("skip: NITRUM_FN_DDB_ENDPOINT not set");
            return;
        };
        let key = IdempotencyKey::new("retry-1").unwrap();
        let first = rec("echo", b"one");
        assert_eq!(
            store.claim(&key, &first).await.expect("claim"),
            IdempotencyClaim::Proceed
        );
        store.complete(&key, &first).await.expect("complete");
        assert!(matches!(
            store.claim(&key, &first).await.expect("replay"),
            IdempotencyClaim::Replay(_)
        ));

        let other = rec("echo", b"two");
        let err = store.claim(&key, &other).await.expect_err("conflict");
        assert!(matches!(err, AppError::Conflict(_)), "{err}");
    }

    #[tokio::test]
    async fn expired_row_can_be_overwritten() {
        let Some((store, client, table)) = store().await else {
            eprintln!("skip: NITRUM_FN_DDB_ENDPOINT not set");
            return;
        };
        let key = IdempotencyKey::new("retry-1").unwrap();
        let first = rec("echo", b"one");
        let sk = storage_key(&first.function, &key);
        client
            .put_item()
            .table_name(&table)
            .item(ATTR_KEY, AttributeValue::S(sk))
            .item(ATTR_FN_ID, AttributeValue::S("echo".into()))
            .item(ATTR_HASH, AttributeValue::S(first.content_hash.to_hex()))
            .item(ATTR_WASM_BYTES, AttributeValue::N("3".into()))
            .item(ATTR_STATUS, AttributeValue::S(STATUS_COMPLETED.into()))
            .item(ATTR_EXPIRES_AT, AttributeValue::N("1".into()))
            .send()
            .await
            .expect("seed expired");

        let second = rec("echo", b"two");
        assert_eq!(
            store.claim(&key, &second).await.expect("reuse"),
            IdempotencyClaim::Proceed
        );
    }

    #[tokio::test]
    async fn same_key_different_functions_do_not_collide() {
        let Some((store, _, _)) = store().await else {
            eprintln!("skip: NITRUM_FN_DDB_ENDPOINT not set");
            return;
        };
        let key = IdempotencyKey::new("retry-1").unwrap();
        assert_eq!(
            store.claim(&key, &rec("echo", b"one")).await.expect("echo"),
            IdempotencyClaim::Proceed
        );
        assert_eq!(
            store
                .claim(&key, &rec("other", b"one"))
                .await
                .expect("other"),
            IdempotencyClaim::Proceed
        );
    }
}
