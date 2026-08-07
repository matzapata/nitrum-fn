use crate::{FunctionId, VersionLabel};

#[derive(Debug, Clone)]
pub struct InvokeRequest {
    pub function: FunctionId,
    pub version: VersionLabel,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct InvokeResponse {
    pub output: Vec<u8>,
    /// True when the compiled Module was already in the in-process cache.
    pub warm_module: bool,
}
