use std::fmt;

use crate::{ContentHash, EgressOrigin, FunctionId, VersionLabel};

#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub function: FunctionId,
    pub wasm: Vec<u8>,
    pub egress_allow: Vec<EgressOrigin>,
}

/// Outcome of a catalog upsert after wasm was stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishStatus {
    Ready,
    Superseded,
}

impl PublishStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Superseded => "superseded",
        }
    }
}

impl fmt::Display for PublishStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct PublishResponse {
    pub function: FunctionId,
    pub version: VersionLabel,
    pub content_hash: ContentHash,
    pub wasm_bytes: usize,
    pub egress_allow: Vec<EgressOrigin>,
    pub status: PublishStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_wire_strings() {
        assert_eq!(PublishStatus::Ready.as_str(), "ready");
        assert_eq!(PublishStatus::Superseded.as_str(), "superseded");
        assert_eq!(PublishStatus::Ready.to_string(), "ready");
    }
}
