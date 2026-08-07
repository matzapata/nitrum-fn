use sha2::{Digest, Sha256};
use std::fmt;

use crate::DomainError;

/// Function name as used in `/invoke/{fn}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionId(String);

impl FunctionId {
    pub fn new(raw: impl Into<String>) -> Result<Self, DomainError> {
        let s = raw.into();
        if s.is_empty()
            || s.len() > 64
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DomainError::InvalidFunctionId(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Mutable label like `v1` or `latest` resolved via the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionLabel(String);

impl VersionLabel {
    pub fn new(raw: impl Into<String>) -> Result<Self, DomainError> {
        let s = raw.into();
        if s.is_empty() || s.len() > 64 {
            return Err(DomainError::InvalidVersionLabel(s));
        }
        Ok(Self(s))
    }

    pub fn latest() -> Self {
        Self("latest".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Immutable sha256 of the `.wasm` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&digest);
        Self(arr)
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, DomainError> {
        let bytes = hex::decode(hex_str)
            .map_err(|_| DomainError::InvalidContentHash(hex_str.to_string()))?;
        if bytes.len() != 32 {
            return Err(DomainError::InvalidContentHash(hex_str.to_string()));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Resolved catalog row: name + label + content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionVersion {
    pub id: FunctionId,
    pub label: VersionLabel,
    pub content_hash: ContentHash,
}
