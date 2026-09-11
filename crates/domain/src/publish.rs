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
    /// Publish accept status (`"queued"` while AOT runs).
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
    /// HTTPS origins the function may fetch at invoke time.
    #[serde(default)]
    pub egress_allow: Vec<EgressOrigin>,
}

impl PublishQueuedEvent {
    pub fn new(
        function: impl Into<String>,
        content_hash: impl Into<String>,
        wasm_bytes: usize,
        egress_allow: Vec<EgressOrigin>,
    ) -> Self {
        Self {
            function: function.into(),
            content_hash: content_hash.into(),
            wasm_bytes,
            queued_at_ms: unix_now_ms(),
            egress_allow,
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
