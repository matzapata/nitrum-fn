use std::collections::HashMap;
use std::sync::RwLock;

use application::error::AppError;
use application::ports::FunctionCatalog;
use async_trait::async_trait;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};

#[derive(Debug, Default)]
pub struct InMemoryCatalog {
    // (id, label) -> content hash
    entries: RwLock<HashMap<(String, String), ContentHash>>,
}

impl InMemoryCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, id: &FunctionId, label: &VersionLabel, hash: ContentHash) {
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert((id.as_str().to_string(), label.as_str().to_string()), hash);
    }
}

#[async_trait]
impl FunctionCatalog for InMemoryCatalog {
    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError> {
        let key = (id.as_str().to_string(), label.as_str().to_string());
        let hash = self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("{id}@{label}")))?;

        Ok(FunctionVersion {
            id: id.clone(),
            label: label.clone(),
            content_hash: hash,
        })
    }
}
