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
}

#[async_trait]
impl FunctionCatalog for InMemoryCatalog {
    async fn upsert(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
        hash: ContentHash,
    ) -> Result<(), AppError> {
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert((id.as_str().to_string(), label.as_str().to_string()), hash);
        Ok(())
    }

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

    async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::with_capacity(entries.len());
        for ((id, label), hash) in entries.iter() {
            let id = FunctionId::new(id.clone()).map_err(AppError::from)?;
            let label = VersionLabel::new(label.clone()).map_err(AppError::from)?;
            out.push(FunctionVersion {
                id,
                label,
                content_hash: hash.clone(),
            });
        }
        Ok(out)
    }
}
