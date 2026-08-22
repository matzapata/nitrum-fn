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

/// Client-supplied key for retrying `PUT /functions/{name}` without a second enqueue.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(raw: impl Into<String>) -> Result<Self, DomainError> {
        let s = raw.into();
        if s.is_empty()
            || s.len() > 128
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DomainError::InvalidIdempotencyKey(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_illegal_idempotency_keys() {
        assert!(matches!(
            IdempotencyKey::new(""),
            Err(DomainError::InvalidIdempotencyKey(_))
        ));
        assert!(matches!(
            IdempotencyKey::new("retry/1"),
            Err(DomainError::InvalidIdempotencyKey(_))
        ));
        assert!(IdempotencyKey::new("a".repeat(129)).is_err());
    }

    #[test]
    fn accepts_uuid_shaped_idempotency_keys() {
        let key = IdempotencyKey::new("550e8400-e29b-41d4-a716-446655440000").expect("uuid");
        assert_eq!(key.as_str(), "550e8400-e29b-41d4-a716-446655440000");
    }
}
