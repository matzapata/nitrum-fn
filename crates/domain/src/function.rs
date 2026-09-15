use sha2::{Digest, Sha256};
use std::fmt;

use crate::limits::MAX_EGRESS_ALLOW;
use crate::DomainError;

/// Normalized HTTPS origin allowed for guest outbound GET (`https://host[:port]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct EgressOrigin(String);

impl EgressOrigin {
    /// Parse a full URL or origin; stores normalized `https://host[:port]`.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidEgressOrigin(raw.to_string()));
        }
        let without_path = trimmed.split('/').nth(2).ok_or_else(|| {
            DomainError::InvalidEgressOrigin(format!("expected https URL, got {raw}"))
        })?;
        if !trimmed.starts_with("https://") {
            return Err(DomainError::InvalidEgressOrigin(format!(
                "only https origins allowed, got {raw}"
            )));
        }
        let authority = without_path.split('@').next_back().ok_or_else(|| {
            DomainError::InvalidEgressOrigin(format!("invalid authority in {raw}"))
        })?;
        if without_path.contains('@') {
            return Err(DomainError::InvalidEgressOrigin(format!(
                "userinfo not allowed in {raw}"
            )));
        }
        let host_port = authority.split(':').collect::<Vec<_>>();
        let host = match host_port.as_slice() {
            [h] => *h,
            [h, p] if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
                let port: u16 = p.parse().map_err(|_| {
                    DomainError::InvalidEgressOrigin(format!("invalid port in {raw}"))
                })?;
                if port == 0 {
                    return Err(DomainError::InvalidEgressOrigin(format!(
                        "invalid port in {raw}"
                    )));
                }
                let normalized = format!("https://{h}:{port}");
                validate_host(h)?;
                return Ok(Self(normalized));
            }
            _ => {
                return Err(DomainError::InvalidEgressOrigin(format!(
                    "invalid host:port in {raw}"
                )));
            }
        };
        validate_host(host)?;
        Ok(Self(format!("https://{host}")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EgressOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validate and dedupe a list of egress origins.
pub fn normalize_egress_allow(
    origins: impl IntoIterator<Item = EgressOrigin>,
) -> Result<Vec<EgressOrigin>, DomainError> {
    let mut out = Vec::new();
    for origin in origins {
        if out.len() >= MAX_EGRESS_ALLOW {
            return Err(DomainError::TooManyEgressOrigins {
                max: MAX_EGRESS_ALLOW,
            });
        }
        if !out.contains(&origin) {
            out.push(origin);
        }
    }
    Ok(out)
}

fn validate_host(host: &str) -> Result<(), DomainError> {
    if host.is_empty() || host.len() > 253 {
        return Err(DomainError::InvalidEgressOrigin(format!(
            "invalid host {host}"
        )));
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err(DomainError::InvalidEgressOrigin(format!(
            "localhost not allowed: {host}"
        )));
    }
    if host.ends_with(".local") {
        return Err(DomainError::InvalidEgressOrigin(format!(
            ".local host not allowed: {host}"
        )));
    }
    if is_ip_literal(host) {
        return Err(DomainError::InvalidEgressOrigin(format!(
            "IP literal not allowed: {host}"
        )));
    }
    if host.contains("..")
        || !host
            .chars()
            .all(|c| c.is_ascii() && (c.is_ascii_alphanumeric() || c == '-' || c == '.'))
    {
        return Err(DomainError::InvalidEgressOrigin(format!(
            "invalid hostname {host}"
        )));
    }
    Ok(())
}

fn is_ip_literal(host: &str) -> bool {
    if host.contains(':') {
        // IPv6 — reject any bracketless or bracketed form.
        return true;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u8>().is_ok())
}

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

/// Resolved catalog row: name + label + content hash + egress policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionVersion {
    pub id: FunctionId,
    pub label: VersionLabel,
    pub content_hash: ContentHash,
    pub egress_allow: Vec<EgressOrigin>,
}

#[cfg(test)]
mod egress_origin_tests {
    use super::*;
    use crate::DomainError;

    #[test]
    fn parses_origin_from_full_url() {
        let o = EgressOrigin::parse("https://api.coingecko.com/api/v3/simple/price").unwrap();
        assert_eq!(o.as_str(), "https://api.coingecko.com");
    }

    #[test]
    fn parses_origin_with_port() {
        let o = EgressOrigin::parse("https://example.com:8443/path").unwrap();
        assert_eq!(o.as_str(), "https://example.com:8443");
    }

    #[test]
    fn rejects_http() {
        assert!(matches!(
            EgressOrigin::parse("http://example.com"),
            Err(DomainError::InvalidEgressOrigin(_))
        ));
    }

    #[test]
    fn rejects_localhost() {
        assert!(matches!(
            EgressOrigin::parse("https://localhost"),
            Err(DomainError::InvalidEgressOrigin(_))
        ));
    }

    #[test]
    fn rejects_ip_literal() {
        assert!(matches!(
            EgressOrigin::parse("https://127.0.0.1"),
            Err(DomainError::InvalidEgressOrigin(_))
        ));
    }

    #[test]
    fn normalize_dedupes_and_caps() {
        let a = EgressOrigin::parse("https://a.example.com").unwrap();
        let b = EgressOrigin::parse("https://b.example.com").unwrap();
        let out = normalize_egress_allow([a.clone(), a, b]).unwrap();
        assert_eq!(out.len(), 2);
    }
}
