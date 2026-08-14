use std::collections::BTreeMap;
use std::path::PathBuf;

use application::error::AppError;
use application::ports::FunctionCatalog;
use async_trait::async_trait;
use domain::{ContentHash, FunctionId, FunctionVersion, VersionLabel};
use tokio::sync::RwLock;

/// Persists `{ "name": { "latest": "<sha256 hex>" } }` as JSON.
pub struct FilesystemCatalog {
    path: PathBuf,
    entries: RwLock<BTreeMap<(String, String), ContentHash>>,
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
        snapshot: &BTreeMap<(String, String), ContentHash>,
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

type FileShape = BTreeMap<String, BTreeMap<String, String>>;

fn to_file(entries: &BTreeMap<(String, String), ContentHash>) -> FileShape {
    let mut out: FileShape = BTreeMap::new();
    for ((id, label), hash) in entries {
        out.entry(id.clone())
            .or_default()
            .insert(label.clone(), hash.to_hex());
    }
    out
}

fn parse_file(bytes: &[u8]) -> Result<BTreeMap<(String, String), ContentHash>, AppError> {
    let file: FileShape =
        serde_json::from_slice(bytes).map_err(|e| AppError::Storage(e.to_string()))?;
    let mut entries = BTreeMap::new();
    for (id, versions) in file {
        for (label, hex) in versions {
            let hash = ContentHash::from_hex(&hex).map_err(AppError::from)?;
            entries.insert((id.clone(), label), hash);
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
    ) -> Result<(), AppError> {
        let mut entries = self.entries.write().await;
        entries.insert((id.as_str().to_string(), label.as_str().to_string()), hash);
        self.persist(&entries).await
    }

    async fn resolve(
        &self,
        id: &FunctionId,
        label: &VersionLabel,
    ) -> Result<FunctionVersion, AppError> {
        let entries = self.entries.read().await;
        let hash = entries
            .get(&(id.as_str().to_string(), label.as_str().to_string()))
            .cloned()
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
        for ((id, label), hash) in entries.iter() {
            out.push(version_from_entry(id, label, hash.clone())?);
        }
        Ok(out)
    }
}
