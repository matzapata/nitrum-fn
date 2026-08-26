use std::collections::HashMap;

use application::error::AppError;
use application::ports::FunctionCatalog;
use async_trait::async_trait;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};

const ATTR_FN_ID: &str = "fn_id";
const ATTR_LABEL: &str = "label";
const ATTR_HASH: &str = "content_hash";
const ATTR_QUEUED_AT: &str = "queued_at_ms";

/// Persists catalog rows as DynamoDB items: `fn_id` (hash) + `label` (range) → `content_hash`.
pub struct DynamoDbFunctionCatalog {
    client: Client,
    table: String,
}

impl DynamoDbFunctionCatalog {
    pub fn new(client: Client, table: impl Into<String>) -> Self {
        Self {
            client,
            table: table.into(),
        }
    }
}

#[async_trait]
impl FunctionCatalog for DynamoDbFunctionCatalog {
    async fn upsert(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
        hash: ContentHash,
        queued_at_ms: u64,
    ) -> Result<bool, AppError> {
        let incoming = queued_at_ms.to_string();
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(ATTR_FN_ID, AttributeValue::S(id.as_str().to_string()))
            .item(ATTR_LABEL, AttributeValue::S(label.as_str().to_string()))
            .item(ATTR_HASH, AttributeValue::S(hash.to_hex()))
            .item(ATTR_QUEUED_AT, AttributeValue::N(incoming.clone()))
            .condition_expression("attribute_not_exists(#q) OR #q <= :incoming")
            .expression_attribute_names("#q", ATTR_QUEUED_AT)
            .expression_attribute_values(":incoming", AttributeValue::N(incoming))
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

    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table)
            .key(ATTR_FN_ID, AttributeValue::S(id.as_str().to_string()))
            .key(ATTR_LABEL, AttributeValue::S(label.as_str().to_string()))
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;

        let Some(item) = out.item() else {
            return Err(AppError::NotFound(format!("{id}@{label}")));
        };
        version_from_item(item)
    }

    async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
        let mut out = Vec::new();
        let mut start_key = None;

        loop {
            let mut req = self.client.scan().table_name(&self.table);
            if let Some(key) = start_key {
                req = req.set_exclusive_start_key(Some(key));
            }
            let page = req
                .send()
                .await
                .map_err(|e| AppError::Storage(e.to_string()))?;

            for item in page.items() {
                out.push(version_from_item(item)?);
            }

            match page.last_evaluated_key() {
                Some(k) if !k.is_empty() => start_key = Some(k.clone()),
                _ => break,
            }
        }

        Ok(out)
    }
}

fn attr_s<'a>(item: &'a HashMap<String, AttributeValue>, key: &str) -> Result<&'a str, AppError> {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .map(String::as_str)
        .ok_or_else(|| AppError::Storage(format!("catalog item missing {key}")))
}

fn version_from_item(item: &HashMap<String, AttributeValue>) -> Result<FunctionVersion, AppError> {
    let id = FunctionId::new(attr_s(item, ATTR_FN_ID)?).map_err(AppError::from)?;
    let label = VersionLabel::new(attr_s(item, ATTR_LABEL)?).map_err(AppError::from)?;
    let content_hash = ContentHash::from_hex(attr_s(item, ATTR_HASH)?).map_err(AppError::from)?;
    Ok(FunctionVersion {
        id,
        label,
        content_hash,
    })
}
