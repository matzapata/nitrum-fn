use std::collections::BTreeMap;
use std::path::PathBuf;

use application::error::AppError;
use application::ports::FunctionCatalog;
use async_trait::async_trait;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct CatalogRow {
    hash: ContentHash,
    queued_at_ms: u64,
}

/// Persists `{ "name": { "latest": { "hash", "queued_at_ms" } } }` as JSON.
/// Legacy `{ "name": { "latest": "<hex>" } }` still reads (queued_at_ms = 0).
pub struct FilesystemCatalog {
    path: PathBuf,
    entries: RwLock<BTreeMap<(String, String), CatalogRow>>,
}

impl FilesystemCatalog {
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, AppError> {
        let path = path.into();
        let entries = match tokio::fs::read(&path).await {
            Ok(bytes) => parse_file(&bytes)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(err) => return Err(AppError::Storage(err.to_string())),
        };
        Ok(Self {
            path,
            entries: RwLock::new(entries),
        })
    }

    async fn persist(
        &self,
        snapshot: &BTreeMap<(String, String), CatalogRow>,
    ) -> Result<(), AppError> {
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
struct FileMeta {
    hash: String,
    queued_at_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum FileVersion {
    Legacy(String),
    Meta(FileMeta),
}

type FileShape = BTreeMap<String, BTreeMap<String, FileVersion>>;

fn to_file(entries: &BTreeMap<(String, String), CatalogRow>) -> FileShape {
    let mut out: FileShape = BTreeMap::new();
    for ((id, label), row) in entries {
        out.entry(id.clone()).or_default().insert(
            label.clone(),
            FileVersion::Meta(FileMeta {
                hash: row.hash.to_hex(),
                queued_at_ms: row.queued_at_ms,
            }),
        );
    }
    out
}

fn parse_file(bytes: &[u8]) -> Result<BTreeMap<(String, String), CatalogRow>, AppError> {
    let file: FileShape =
        serde_json::from_slice(bytes).map_err(|e| AppError::Storage(e.to_string()))?;
    let mut entries = BTreeMap::new();
    for (id, versions) in file {
        for (label, version) in versions {
            let (hex, queued_at_ms) = match version {
                FileVersion::Legacy(hex) => (hex, 0),
                FileVersion::Meta(meta) => (meta.hash, meta.queued_at_ms),
            };
            let hash = ContentHash::from_hex(&hex).map_err(AppError::from)?;
            entries.insert((id.clone(), label), CatalogRow { hash, queued_at_ms });
        }
    }
    Ok(entries)
}

fn version_from_entry(
    id: &str,
    label: &str,
    hash: ContentHash,
) -> Result<FunctionVersion, AppError> {
    Ok(FunctionVersion {
        id: FunctionId::new(id).map_err(AppError::from)?,
        label: VersionLabel::new(label).map_err(AppError::from)?,
        content_hash: hash,
    })
}

#[async_trait]
impl FunctionCatalog for FilesystemCatalog {
    async fn upsert(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
        hash: ContentHash,
        queued_at_ms: u64,
    ) -> Result<bool, AppError> {
        let key = (id.as_str().to_string(), label.as_str().to_string());
        let mut entries = self.entries.write().await;
        if let Some(existing) = entries.get(&key) {
            if existing.queued_at_ms > queued_at_ms {
                return Ok(false);
            }
        }
        entries.insert(key, CatalogRow { hash, queued_at_ms });
        self.persist(&entries).await?;
        Ok(true)
    }

    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError> {
        let entries = self.entries.read().await;
        let hash = entries
            .get(&(id.as_str().to_string(), label.as_str().to_string()))
            .map(|row| row.hash.clone())
            .ok_or_else(|| AppError::NotFound(format!("{id}@{label}")))?;
        Ok(FunctionVersion {
            id: id.clone(),
            label: label.clone(),
            content_hash: hash,
        })
    }

    async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
        let entries = self.entries.read().await;
        let mut out = Vec::with_capacity(entries.len());
        for ((id, label), row) in entries.iter() {
            out.push(version_from_entry(id, label, row.hash.clone())?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_hash_strings() {
        let raw = br#"{ "echo": { "latest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } }"#;
        let entries = parse_file(raw).expect("legacy");
        let row = entries.get(&("echo".into(), "latest".into())).unwrap();
        assert_eq!(row.queued_at_ms, 0);
    }

    #[tokio::test]
    async fn stale_upsert_does_not_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        let catalog = FilesystemCatalog::open(&path).await.unwrap();
        let id = FunctionId::new("echo").unwrap();
        let label = VersionLabel::latest();
        let old = ContentHash::from_bytes(b"old");
        let new = ContentHash::from_bytes(b"new");

        assert!(catalog.upsert(&id, &label, old.clone(), 100).await.unwrap());
        assert!(catalog.upsert(&id, &label, new.clone(), 200).await.unwrap());
        assert!(!catalog.upsert(&id, &label, old, 50).await.unwrap());

        let resolved = catalog.resolve(&id, &label).await.unwrap();
        assert_eq!(resolved.content_hash, new);
    }
}
