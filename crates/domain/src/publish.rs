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
    /// Always `"queued"` until a future ready-status API exists.
    pub status: &'static str,
}

/// Event published when a `.wasm` is stored and awaiting AOT compile.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublishQueuedEvent {
    pub function: String,
    pub content_hash: String,
    pub wasm_bytes: usize,
    /// Unix millis at enqueue time. Catalog upserts ignore older generations.
    #[serde(default)]
    pub queued_at_ms: u64,
}

impl PublishQueuedEvent {
    pub fn new(
        function: impl Into<String>,
        content_hash: impl Into<String>,
        wasm_bytes: usize,
    ) -> Self {
        Self {
            function: function.into(),
            content_hash: content_hash.into(),
            wasm_bytes,
            queued_at_ms: unix_now_ms(),
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
