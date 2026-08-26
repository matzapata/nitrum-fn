use application::error::AppError;
use application::ports::PublishLock;
use async_trait::async_trait;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;
use domain::{ContentHash, FunctionId};

const ATTR_FN_ID: &str = "fn_id";
const ATTR_HASH: &str = "content_hash";
const ATTR_QUEUED_AT: &str = "queued_at_ms";
const ATTR_EXPIRES_AT: &str = "expires_at";

/// Safety TTL so a dead worker cannot block publishes forever (~15 min).
pub(crate) const TTL_SECS: u64 = 15 * 60;

/// DynamoDB items: `fn_id` → content hash + queued_at + expires_at (TTL).
pub struct DynamoDbPublishLock {
    client: Client,
    table: String,
}

impl DynamoDbPublishLock {
    pub fn new(client: Client, table: impl Into<String>) -> Self {
        Self {
            client,
            table: table.into(),
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
impl PublishLock for DynamoDbPublishLock {
    async fn acquire(
        &self,
        function: &FunctionId,
        hash: &ContentHash,
        queued_at_ms: u64,
    ) -> Result<(), AppError> {
        let now = unix_now();
        let expires_at = now.saturating_add(TTL_SECS).to_string();
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(ATTR_FN_ID, AttributeValue::S(function.as_str().to_string()))
            .item(ATTR_HASH, AttributeValue::S(hash.to_hex()))
            .item(ATTR_QUEUED_AT, AttributeValue::N(queued_at_ms.to_string()))
            .item(ATTR_EXPIRES_AT, AttributeValue::N(expires_at))
            .condition_expression("attribute_not_exists(#k) OR #ttl <= :now")
            .expression_attribute_names("#k", ATTR_FN_ID)
            .expression_attribute_names("#ttl", ATTR_EXPIRES_AT)
            .expression_attribute_values(":now", AttributeValue::N(now.to_string()))
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
                Err(AppError::Conflict(format!(
                    "publish already in progress for {function}"
                )))
            }
            Err(err) => Err(AppError::Storage(err.to_string())),
        }
    }

    async fn release(&self, function: &FunctionId, hash: &ContentHash) -> Result<(), AppError> {
        let result = self
            .client
            .delete_item()
            .table_name(&self.table)
            .key(ATTR_FN_ID, AttributeValue::S(function.as_str().to_string()))
            .condition_expression("#h = :h")
            .expression_attribute_names("#h", ATTR_HASH)
            .expression_attribute_values(":h", AttributeValue::S(hash.to_hex()))
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
                // Wrong hash or already gone — ignore.
                Ok(())
            }
            Err(err) => Err(AppError::Storage(err.to_string())),
        }
    }
}
