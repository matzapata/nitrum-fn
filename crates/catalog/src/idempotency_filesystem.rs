use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use application::error::AppError;
use application::ports::{
    IdempotencyClaim, IdempotencyRecord, IdempotencyStatus, PublishIdempotency,
};
use async_trait::async_trait;
use domain::{ContentHash, FunctionId, IdempotencyKey};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::idempotency::{claim_in_map, complete_in_map, unix_now, Stored};

/// Persists `{storage_key: {function, hash, status, expires_at}}` as JSON next to the catalog.
pub struct FilesystemPublishIdempotency {
    path: PathBuf,
    records: RwLock<HashMap<String, Stored>>,
}

impl FilesystemPublishIdempotency {
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, AppError> {
        let path = path.into();
        let records = match tokio::fs::read(&path).await {
            Ok(bytes) => parse_file(&bytes)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(err) => return Err(AppError::Storage(err.to_string())),
        };
        Ok(Self {
            path,
            records: RwLock::new(records),
        })
    }

    async fn persist(&self, snapshot: &HashMap<String, Stored>) -> Result<(), AppError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AppError::Storage(e.to_string()))?;
        }
        let json = serde_json::to_vec_pretty(&to_file(snapshot))
            .map_err(|e| AppError::Storage(e.to_string()))?;
        tokio::fs::write(&self.path, json)
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FileRecord {
    function: String,
    content_hash: String,
    wasm_bytes: u64,
    status: String,
    expires_at: u64,
}

type FileShape = BTreeMap<String, FileRecord>;

fn to_file(records: &HashMap<String, Stored>) -> FileShape {
    records
        .iter()
        .map(|(k, stored)| {
            (
                k.clone(),
                FileRecord {
                    function: stored.record.function.to_string(),
                    content_hash: stored.record.content_hash.to_hex(),
                    wasm_bytes: stored.record.wasm_bytes as u64,
                    status: match stored.record.status {
                        IdempotencyStatus::Pending => "pending".into(),
                        IdempotencyStatus::Completed => "completed".into(),
                    },
                    expires_at: stored.expires_at,
                },
            )
        })
        .collect()
}

fn parse_file(bytes: &[u8]) -> Result<HashMap<String, Stored>, AppError> {
    let file: FileShape =
        serde_json::from_slice(bytes).map_err(|e| AppError::Storage(e.to_string()))?;
    let mut records = HashMap::new();
    for (k, row) in file {
        let status = match row.status.as_str() {
            "pending" => IdempotencyStatus::Pending,
            "completed" => IdempotencyStatus::Completed,
            other => {
                return Err(AppError::Storage(format!(
                    "idempotency item has unknown status {other}"
                )))
            }
        };
        let record = IdempotencyRecord {
            function: FunctionId::new(row.function).map_err(AppError::from)?,
            content_hash: ContentHash::from_hex(&row.content_hash).map_err(AppError::from)?,
            wasm_bytes: row.wasm_bytes as usize,
            status,
        };
        records.insert(
            k,
            Stored {
                record,
                expires_at: row.expires_at,
            },
        );
    }
    Ok(records)
}

#[async_trait]
impl PublishIdempotency for FilesystemPublishIdempotency {
    async fn claim(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<IdempotencyClaim, AppError> {
        let mut records = self.records.write().await;
        let before = records.clone();
        let claim = claim_in_map(&mut records, unix_now(), key, record)?;
        if *records != before {
            self.persist(&records).await?;
        }
        Ok(claim)
    }

    async fn complete(
        &self,
        key: &IdempotencyKey,
        record: &IdempotencyRecord,
    ) -> Result<(), AppError> {
        let mut records = self.records.write().await;
        complete_in_map(&mut records, unix_now(), key, record)?;
        self.persist(&records).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::ContentHash;

    fn rec(wasm: &[u8]) -> IdempotencyRecord {
        IdempotencyRecord {
            function: FunctionId::new("echo").unwrap(),
            content_hash: ContentHash::from_bytes(wasm),
            wasm_bytes: wasm.len(),
            status: IdempotencyStatus::Pending,
        }
    }

    #[tokio::test]
    async fn survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("idempotency.json");
        let k = IdempotencyKey::new("retry-1").unwrap();
        let first = rec(b"one");

        let store = FilesystemPublishIdempotency::open(&path).await.unwrap();
        assert_eq!(
            store.claim(&k, &first).await.unwrap(),
            IdempotencyClaim::Proceed
        );
        store.complete(&k, &first).await.unwrap();
        drop(store);

        let store = FilesystemPublishIdempotency::open(&path).await.unwrap();
        assert!(matches!(
            store.claim(&k, &first).await.unwrap(),
            IdempotencyClaim::Replay(_)
        ));
    }
}
