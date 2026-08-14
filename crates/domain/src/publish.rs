use crate::{ContentHash, FunctionId, VersionLabel};

#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub function: FunctionId,
    pub wasm: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PublishResponse {
    pub function: FunctionId,
    pub version: VersionLabel,
    pub content_hash: ContentHash,
    pub wasm_bytes: usize,
    pub compiled_bytes: usize,
}
