use crate::{ContentHash, EgressOrigin, FunctionId, VersionLabel};

#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub function: FunctionId,
    pub wasm: Vec<u8>,
    pub egress_allow: Vec<EgressOrigin>,
}

#[derive(Debug, Clone)]
pub struct PublishResponse {
    pub function: FunctionId,
    pub version: VersionLabel,
    pub content_hash: ContentHash,
    pub wasm_bytes: usize,
    pub egress_allow: Vec<EgressOrigin>,
    /// `"ready"` when the catalog row was applied; `"superseded"` if a newer generation won.
    pub status: &'static str,
}
