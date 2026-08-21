use std::collections::HashMap;
use std::sync::RwLock;

use application::error::AppError;
use application::ports::FunctionCatalog;
use async_trait::async_trait;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};

#[derive(Debug, Clone)]
struct CatalogRow {
    hash: ContentHash,
    queued_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct InMemoryCatalog {
    // (id, label) -> row
    entries: RwLock<HashMap<(String, String), CatalogRow>>,
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
        queued_at_ms: u64,
    ) -> Result<bool, AppError> {
        let key = (id.as_str().to_string(), label.as_str().to_string());
        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = entries.get(&key) {
            if existing.queued_at_ms > queued_at_ms {
                return Ok(false);
            }
        }
        entries.insert(key, CatalogRow { hash, queued_at_ms });
        Ok(true)
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
            .map(|row| row.hash.clone())
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
        for ((id, label), row) in entries.iter() {
            let id = FunctionId::new(id.clone()).map_err(AppError::from)?;
            let label = VersionLabel::new(label.clone()).map_err(AppError::from)?;
            out.push(FunctionVersion {
                id,
                label,
                content_hash: row.hash.clone(),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_upsert_does_not_clobber() {
        let catalog = InMemoryCatalog::new();
        let id = FunctionId::new("echo").unwrap();
        let label = VersionLabel::latest();
        let old = ContentHash::from_bytes(b"old");
        let new = ContentHash::from_bytes(b"new");

        assert!(catalog.upsert(&id, &label, old.clone(), 100).await.unwrap());
        assert!(catalog.upsert(&id, &label, new.clone(), 200).await.unwrap());
        assert!(!catalog.upsert(&id, &label, old.clone(), 150).await.unwrap());

        let resolved = catalog.resolve(&id, &label).await.unwrap();
        assert_eq!(resolved.content_hash, new);
    }
}
