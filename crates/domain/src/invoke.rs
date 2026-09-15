use crate::{ContentHash, FunctionId, VersionLabel};

#[derive(Debug, Clone)]
pub struct InvokeRequest {
    pub function: FunctionId,
    pub version: VersionLabel,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct InvokeResponse {
    pub output: Vec<u8>,
    /// sha256 of the `.wasm` bytes the host compiled and ran.
    pub content_hash: ContentHash,
}
