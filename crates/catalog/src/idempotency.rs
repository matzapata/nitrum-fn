use domain::{FunctionId, IdempotencyKey};

pub(crate) const TTL_SECS: u64 = 24 * 60 * 60;

pub(crate) fn storage_key(function: &FunctionId, key: &IdempotencyKey) -> String {
    format!("{}#{}", function.as_str(), key.as_str())
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn is_live(expires_at: u64, now: u64) -> bool {
    expires_at > now
}
